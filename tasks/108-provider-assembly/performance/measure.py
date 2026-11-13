#!/usr/bin/env python3
"""After dev publication, run fixed Task108 CPU/RSS fixtures in balanced pairs.

Use build records from prepare.py. Every command, failure, warmup, and measured
repetition is retained outside the repository. Does not build or change source.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import statistics
import subprocess
import time


CASES = [
    ("text_short_frames", ["text", "16384", "16", "0", "8"]),
    ("tools_small", ["tools", "4", "128", "17", "512"]),
    ("tools_large_interleaved", ["tools", "16", "1048576", "1024", "2"]),
    ("tools_many_interleaved", ["tools", "128", "4096", "128", "8"]),
]
IDENTITY_KEYS = (
    "probe", "mode", "count", "payload_bytes", "fragment_bytes", "chunk_bytes", "iterations",
    "warmup_iterations", "sse_frames", "sse_bytes", "sse_fnv1a64", "request_bytes",
    "request_fnv1a64", "arguments_bytes", "arguments_fingerprint", "expected_response_bytes",
    "expected_response_fnv1a64", "all_exact_request_response_and_progress_checks_passed",
)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def build_record(path):
    record = json.loads(path.read_text())
    assert record["build_exit_code"] == 0 and record["production_source_unchanged"]
    assert record["probe_sha256"] == record["compiled_probe_sha256"]
    assert digest(record["binary"]) == record["binary_sha256"]
    return record


def distribution(values):
    return {"samples": len(values), "median": statistics.median(values),
            "min": min(values), "max": max(values), "values": values}


def summarize(rows):
    summaries = []
    for case, _ in CASES:
        selected = {variant: [row for row in rows if row["case"] == case and row["variant"] == variant]
                    for variant in ("baseline", "candidate")}
        for metric in ("ns_per_call", "peak_rss_kib"):
            values = {}
            for variant, samples in selected.items():
                values[variant] = [
                    (row["sample"]["elapsed_ns"] / row["sample"]["iterations"] if metric == "ns_per_call"
                     else row["sample"]["memory_after_measurement"]["peak_rss_kib"])
                    for row in samples]
            paired_ratios = [candidate / baseline
                             for baseline, candidate in zip(values["baseline"], values["candidate"])]
            summaries.append({
                "case": case, "metric": metric,
                "variants": {variant: distribution(value) for variant, value in values.items()},
                "candidate_to_baseline_paired_ratios": distribution(paired_ratios),
                "candidate_to_baseline_median_ratio": (
                    statistics.median(values["candidate"]) / statistics.median(values["baseline"])),
            })
    return summaries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-build", type=Path, required=True)
    parser.add_argument("--candidate-build", type=Path, required=True)
    parser.add_argument("--published-revision", required=True,
                        help="exact dev commit already published, confirmed by the operator")
    parser.add_argument("--rounds", type=int, default=6)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.rounds < 2 or args.rounds % 2:
        parser.error("--rounds must be positive and even, at least 2, for balanced AB/BA")
    builds = {"baseline": build_record(args.baseline_build), "candidate": build_record(args.candidate_build)}
    assert builds["candidate"]["source_revision"] == args.published_revision
    for key in ("probe_sha256", "cargo_lock_sha256", "rustc", "build_environment"):
        assert builds["baseline"][key] == builds["candidate"][key], f"unequal builds: {key}"
    assert builds["baseline"]["source_revision"] != builds["candidate"]["source_revision"]
    args.out.mkdir(parents=True, exist_ok=False)
    method = {
        "runner_sha256": digest(__file__), "builds": builds,
        "published_revision_operator_confirmation": args.published_revision,
        "rounds": args.rounds, "workloads": [{"case": case, "arguments": argv} for case, argv in CASES],
        "pair_order": "baseline/candidate on even rounds; candidate/baseline on odd rounds",
        "case_order": "forward on even rounds; reversed on odd rounds",
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "platform": platform.platform(), "affinity_cpus": len(os.sched_getaffinity(0)),
        "cpu_model": next((line.split(":", 1)[1].strip()
                           for line in Path("/proc/cpuinfo").read_text().splitlines()
                           if line.startswith("model name")), None),
        "load_begin": list(os.getloadavg()),
        "limits": (
            "Coordinated no-build window, shared machine without isolation or cache-flush promise. "
            "Real provider SSE/decoder and Runtime ModelDriver; deterministic memory stream with 8192-byte chunks. "
            "No HTTP, external model wait, tools executed, Host lifecycle, TUI, saturation or capacity claim. "
            "Per-call timing includes actual safety checks, request check and small observation check; excludes fixture "
            "creation and exact response comparison. One warmup per process is recorded and excluded from timing. "
            "Whole-process CPU/wall also include setup, warmup and validation. Linux VmHWM includes fixture, expected "
            "response, runtime, warmup and allocator retention. It is not an allocation count or isolated operation peak. "
            "Reported HWM precedes final expected-response serialization. Fingerprints are noncryptographic FNV-1a; "
            "source/harness/binary SHA-256 and each full exact response equality establish reproducibility."
        ),
    }
    (args.out / "method.json").write_text(json.dumps(method, indent=2) + "\n")
    rows = []
    with (args.out / "samples.jsonl").open("w") as raw:
        for round_index in range(args.rounds):
            cases = CASES if round_index % 2 == 0 else list(reversed(CASES))
            variants = ("baseline", "candidate") if round_index % 2 == 0 else ("candidate", "baseline")
            for case, argv in cases:
                pair = {}
                for variant in variants:
                    command = [builds[variant]["binary"], *argv]
                    before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    started_utc = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
                    started = time.perf_counter_ns()
                    result = subprocess.run(command, text=True, capture_output=True)
                    wall_ns = time.perf_counter_ns() - started
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    row = {
                        "round": round_index, "case": case, "variant": variant, "command": command,
                        "started_utc": started_utc, "wall_ns": wall_ns, "exit_code": result.returncode,
                        "process_cpu_seconds": {"user": after.ru_utime - before.ru_utime,
                                                "system": after.ru_stime - before.ru_stime},
                        "stdout": result.stdout, "stderr": result.stderr,
                    }
                    if result.returncode == 0:
                        try:
                            sample = json.loads(result.stdout)
                            assert sample["all_exact_request_response_and_progress_checks_passed"] is True
                            assert len(sample["iteration_elapsed_ns"]) == sample["iterations"]
                            assert sum(sample["iteration_elapsed_ns"]) == sample["elapsed_ns"]
                            row["sample"] = sample
                        except (ValueError, KeyError, TypeError, AssertionError) as failure:
                            row["validation_failure"] = repr(failure)
                    raw.write(json.dumps(row) + "\n")
                    raw.flush()
                    if result.returncode or "validation_failure" in row:
                        raise RuntimeError(f"probe failed; all raw output retained: {command}")
                    rows.append(row)
                    pair[variant] = row["sample"]
                for key in IDENTITY_KEYS:
                    assert pair["baseline"][key] == pair["candidate"][key], f"different fixture/output: {case}/{key}"
            print(f"round {round_index + 1}/{args.rounds}: {len(rows)} commands complete", flush=True)
    summary = {
        "cases": summarize(rows), "all_paired_workloads_and_exact_responses_verified": True,
        "completed_commands": len(rows), "load_end": list(os.getloadavg()),
        "samples_sha256": digest(args.out / "samples.jsonl"),
    }
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
