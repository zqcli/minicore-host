#!/usr/bin/env python3
"""Build the unchanged Task104 compaction fixture against an exact source archive.

All build inputs and output live outside the tracked repository. The candidate
has exactly one import-name substitution; no production or workload edit.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import time


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--candidate", action="store_true")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    root = args.root.resolve()
    revision = subprocess.check_output(["git", "-C", str(root), "rev-parse", args.revision], text=True).strip()
    archive = subprocess.check_output(["git", "-C", str(root), "archive", revision])
    source = args.out / "source"
    source.mkdir()
    with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
        bundle.extractall(source, filter="data")
    original = source / "tasks/104-performance/probes/compaction.rs"
    canonical = original.read_text()
    probe = source / "crates/host-compaction/examples/task106_compaction.rs"
    probe.parent.mkdir(parents=True, exist_ok=True)
    rendered = canonical.replace("use minicore_runtime::{", "use minicore_host_runtime::{") if args.candidate else canonical
    probe.write_text(rendered)
    target = args.out / "target"
    command = ["cargo", "build", "--release", "--locked", "--offline", "--manifest-path",
               str(source / "Cargo.toml"), "--target-dir", str(target), "-p",
               "minicore-host-compaction", "--example", "task106_compaction"]
    started = time.perf_counter_ns()
    result = subprocess.run(command, text=True, capture_output=True)
    elapsed = time.perf_counter_ns() - started
    if result.returncode:
        print(result.stderr)
        raise SystemExit(result.returncode)
    binary = target / "release/examples/task106_compaction"
    record = {"source_revision": revision, "source_archive_sha256": hashlib.sha256(archive).hexdigest(),
        "normalized_probe_sha256": hashlib.sha256(canonical.encode()).hexdigest(),
        "compiled_probe_sha256": digest(probe), "candidate_import_only": args.candidate,
        "cargo_lock_sha256": digest(source / "Cargo.lock"), "binary": str(binary.resolve()),
        "binary_sha256": digest(binary), "build_command": command, "build_duration_ns": elapsed,
        "rustc": subprocess.check_output(["rustc", "--version", "--verbose"], text=True),
        "fixture": "Task104 DeterministicSummaryFixture; no live provider or Session owners"}
    (args.out / "build.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({"build_record": str(args.out / "build.json"), "build_duration_ns": elapsed}))


if __name__ == "__main__":
    main()
