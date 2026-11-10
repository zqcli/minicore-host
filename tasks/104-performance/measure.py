#!/usr/bin/env python3
"""Task104 fixed-workload paired release probes; keeps every sample and failure."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import subprocess
import sys
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def workloads():
    cases = [("protocol", ["67108864"])]
    for size, iterations in [(64, 20000), (4096, 1024), (65536, 64), (1048576, 8)]:
        cases.append(("provider", [str(size), str(iterations)]))
    for groups in [64, 256, 1024]:
        cases.append(("compaction", [str(groups), "1024", "3"]))
    for mode in ["metadata", "journal"]:
        for size in [1024, 32768]:
            cases.append(("io", [mode, str(size), "9"]))
    for mode, sizes in [("diff-lines", [120, 12000]), ("diff-long", [1024, 131072])]:
        for size in sizes:
            for position in ["first", "middle", "last", "full"]:
                cases.append(("io", [mode, str(size), "9", position]))
    for size in [1000, 100000]:
        cases.append(("io", ["edits", str(size), "101"]))
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=4)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("--rounds must be positive")
    args.out.mkdir(parents=True, exist_ok=False)
    metadata = {}
    for variant in ["baseline", "candidate"]:
        record = json.loads((args.artifacts / f"{variant}-probes-build.json").read_text())
        if record["exit_code"] != 0:
            raise RuntimeError(f"{variant} probe build did not pass")
        for probe, entry in record["probes"].items():
            binary = Path(entry["binary"])
            if digest(binary) != entry["binary_sha256"]:
                raise RuntimeError(f"{variant}/{probe} binary hash mismatch")
        metadata[variant] = record
    for probe in metadata["baseline"]["probes"]:
        if (metadata["baseline"]["probes"][probe]["harness_sha256"] !=
                metadata["candidate"]["probes"][probe]["harness_sha256"]):
            raise RuntimeError(f"{probe} harness differs across builds")

    cases = workloads()
    method = {
        "runner_sha256": digest(Path(__file__)),
        "rounds": args.rounds,
        "workloads": [{"probe": probe, "arguments": argv} for probe, argv in cases],
        "pair_order": "baseline/candidate on even rounds; reverse on odd rounds",
        "case_order": "forward on even rounds; reverse on odd rounds",
        "environment": {
            "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "python": sys.version,
            "platform": platform.platform(),
            "logical_cpus": os.cpu_count(),
            "affinity_cpus": len(os.sched_getaffinity(0)),
            "loadavg": Path("/proc/loadavg").read_text(),
            "meminfo": Path("/proc/meminfo").read_text(),
            "cpu_model": next((line.split(":", 1)[1].strip()
                               for line in Path("/proc/cpuinfo").read_text().splitlines()
                               if line.startswith("model name")), None),
        },
        "builds": metadata,
        "limits": (
            "Coordinated no-project-build window, not machine isolation; no CPU pinning "
            "or cache flushing. Required production validation stays inside timed operations. "
            "Fixture construction and post-run correctness checks are outside each probe's timer. "
            "Warmup is excluded only where stated; compaction keeps every repetition. "
            "Whole-process CPU includes setup and warmup. Only provider "
            "reports HWM before validation; it includes its immutable fixture and warmup. "
            "Diff setup includes full-diff queries and cannot isolate page peak RSS. "
            "No live-provider, network, TUI, saturation or capacity conclusion."
        ),
    }
    (args.out / "method.json").write_text(json.dumps(method, indent=2) + "\n")
    completed = 0
    with (args.out / "commands.jsonl").open("w") as raw:
        for round_index in range(args.rounds):
            ordered = cases if round_index % 2 == 0 else list(reversed(cases))
            variants = ["baseline", "candidate"] if round_index % 2 == 0 else ["candidate", "baseline"]
            for probe, argv in ordered:
                for variant in variants:
                    binary = metadata[variant]["probes"][probe]["binary"]
                    command = [binary, *argv]
                    usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    started = time.perf_counter_ns()
                    result = subprocess.run(command, text=True, capture_output=True)
                    finished = time.perf_counter_ns()
                    usage_after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    item = {
                        "round": round_index, "variant": variant, "probe": probe,
                        "arguments": argv, "command": command,
                        "started_unix": time.time() - (finished - started) / 1e9,
                        "wall_ns": finished - started, "exit_code": result.returncode,
                        "process_cpu_seconds": {
                            "user": usage_after.ru_utime - usage_before.ru_utime,
                            "system": usage_after.ru_stime - usage_before.ru_stime,
                        },
                        "stdout": result.stdout, "stderr": result.stderr,
                    }
                    raw.write(json.dumps(item) + "\n")
                    raw.flush()
                    if result.returncode:
                        raise RuntimeError(f"probe failed: {command}; all output retained")
                    rows = [json.loads(line) for line in result.stdout.splitlines() if line]
                    if not rows:
                        raise RuntimeError(f"probe produced no data: {command}")
                    completed += 1
            print(f"round {round_index + 1}/{args.rounds}: {completed} probe commands complete", flush=True)
    (args.out / "completion.json").write_text(json.dumps({
        "completed_commands": completed,
        "raw_sha256": digest(args.out / "commands.jsonl"),
        "all_commands_succeeded": True,
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
