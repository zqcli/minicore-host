#!/usr/bin/env python3
"""Check retained evidence integrity and recompute summaries; never rerun workloads."""
import argparse
import gzip
import hashlib
from pathlib import Path, PurePosixPath
import re
import sys

sys.dont_write_bytecode = True
from summarize import parse, require, summarize

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from task090_summarize import fresh_summary


def fingerprint(data):
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def check_fingerprint(data, record, label):
    actual = fingerprint(data)
    require(type(record["bytes"]) is int and record["bytes"] == actual["bytes"] and
            record["sha256"] == actual["sha256"], f"content mismatch: {label}")
    return actual


def verify(directory):
    directory = directory.resolve(strict=True)
    paths = list(directory.rglob("*"))
    require(all(not path.is_symlink() and (path.is_file() or path.is_dir()) for path in paths),
            "archive must contain only regular files and directories, without symlinks")
    actual = {path.relative_to(directory).as_posix() for path in paths if path.is_file()}
    actual.remove("manifest.json")
    read = lambda name: parse((directory / name).read_text())
    manifest = read("manifest.json")
    require(manifest["schema"] == "task104.evidence.v1", "wrong archive schema")
    listed, uncompressed = set(), {}
    for entry in manifest["files"]:
        name = entry["path"]
        require(isinstance(name, str) and name and "\\" not in name, "invalid archive path")
        relative = PurePosixPath(name)
        require(not relative.is_absolute() and ".." not in relative.parts and
                relative.as_posix() == name and name != "." and name != "manifest.json",
                f"unsafe or noncanonical archive path: {name}")
        require(name not in listed, f"duplicate archive path: {name}")
        listed.add(name)
        require(name in actual, f"missing archived file: {name}")
        data = (directory / name).read_bytes()
        check_fingerprint(data, entry, name)
        if name.endswith(".gz"):
            data = gzip.decompress(data)
            uncompressed[name] = fingerprint(data)
        if "uncompressed" in entry:
            require(name.endswith(".gz"), f"decompressed hash on non-gzip file: {name}")
            check_fingerprint(data, entry["uncompressed"], f"decompressed {name}")
    require(actual == listed, "archive inventory differs from manifest")

    paired = summarize(directory / "paired")
    require(paired == read("paired/summary.json"), "paired summary does not recompute")
    require(paired["sources"] == manifest["sources"], "paired/manifest source mismatch")
    method = read("paired/method.json")
    for variant, source in manifest["sources"].items():
        prefix = f"host-{variant}"
        results = read(f"{prefix}/results.json")
        require(results["environment"]["source_revision"] == source, f"{prefix} source mismatch")
        require([run["run"] for run in results["runs"]] == [1, 2, 3], f"{prefix} run count/order mismatch")
        require(fresh_summary(results) == read(f"{prefix}/summary.json"), f"{prefix} summary mismatch")
        require(all(f"{prefix}/run-{run:02d}.jsonl.gz" in listed for run in (1, 2, 3)),
                f"{prefix} raw run logs missing")
        build = read(f"builds/{variant}-build.json")
        require(build["source"] == source and type(build["exit_code"]) is int and
                build["exit_code"] == 0 and
                build["binary_sha256"] == results["environment"]["binary_sha256"],
                f"{prefix} build provenance mismatch")
        require(read(f"builds/{variant}-probes-build.json") == method["builds"][variant],
                f"{variant} probe build provenance mismatch")

    for stem, log in (("full-gate", "full-gate.log.gz"), ("test-inventory", "test-list.log.gz")):
        record = read(f"validation/{stem}.json")
        retained = uncompressed[f"validation/{log}"]
        require(record["source"] == manifest["sources"]["candidate"] and
                type(record["exit_code"]) is int and record["exit_code"] == 0,
                f"{stem} source or status mismatch")
        require(record["log_sha256"] == retained["sha256"], f"{stem} log hash mismatch")
        if "log_bytes" in record:
            require(record["log_bytes"] == retained["bytes"], f"{stem} log size mismatch")
    inventory = read("validation/test-inventory.json")
    expected = {"unique_cases": 964, "targets": 126, "nonempty_targets": 124}
    require(all(type(inventory[key]) is int and inventory[key] == value for key, value in expected.items()),
            "unexpected authoritative test inventory")
    log = gzip.decompress((directory / "validation/test-list.log.gz").read_bytes()).decode()
    sections = re.split(r"(?m)^     Running (.+)$", log)
    headers, bodies = sections[1::2], sections[2::2]
    require(len(headers) == len(set(headers)) == 126, "test-list target count/uniqueness mismatch")
    counts = []
    for body in bodies:
        names = re.findall(r"(?m)^(.+): test$", body)
        totals = re.findall(r"(?m)^(\d+) tests?, 0 benchmarks$", body)
        require(len(names) == len(set(names)) and totals == [str(len(names))],
                "test-list target contains duplicate tests or incorrect totals")
        counts.append(len(names))
    require(sum(counts) == 964 and sum(count > 0 for count in counts) == 124,
            "test-list contents disagree with inventory")
    require({"validation/historical-090.log", "validation/historical-098.log"} <= listed,
            "historical verification logs missing")
    return len(listed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path, nargs="?", default=Path(__file__).resolve().parent / "evidence")
    args = parser.parse_args()
    count = verify(args.evidence)
    print(f"Verified {count} archived files, paired evidence, Host summaries and candidate validation records")
    print("Integrity and deterministic recomputation only; workloads, builds and tests were not rerun.")


if __name__ == "__main__":
    main()
