#!/usr/bin/env python3
"""Audit Task104 paired evidence, then write or check its deterministic summary."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics


BASELINE = "4ee5e819798c2b165f9b03cf69dcdfb4a82e2cb4"
PROBES = {"protocol", "provider", "compaction", "io"}
VARIANTS = ("baseline", "candidate")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def reject_constant(value):
    raise ValueError(f"nonfinite JSON constant: {value}")


def parse(text):
    return json.loads(text, object_pairs_hook=unique_object, parse_constant=reject_constant)


def rows(text):
    require(isinstance(text, str), "JSONL must be retained text")
    lines = text.splitlines()
    require(lines and all(line.strip() for line in lines), "missing or blank JSONL row")
    result = [parse(line) for line in lines]
    require(all(isinstance(row, dict) for row in result), "JSONL rows must be objects")
    return result


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def integer(value, label, minimum=1):
    require(type(value) is int and value >= minimum, f"invalid integer {label}: {value!r}")
    return value


def finite(value, label, minimum=0):
    require(type(value) in (int, float) and math.isfinite(value) and value >= minimum,
            f"invalid finite number {label}: {value!r}")
    return value


def hash_value(value, length, label):
    require(isinstance(value, str) and re.fullmatch(f"[0-9a-f]{{{length}}}", value),
            f"invalid {label}")


def workloads():
    # Fixed fixture list from measure.py; a changed list requires a fresh audit.
    cases = [("protocol", ["67108864"])]
    for size, count in [(64, 20000), (4096, 1024), (65536, 64), (1048576, 8)]:
        cases.append(("provider", [str(size), str(count)]))
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


def audit_builds(method, task):
    require(set(method["builds"]) == set(VARIANTS), "expected exactly two build variants")
    builds = method["builds"]
    require(builds["baseline"]["source"] == BASELINE, "wrong fixed baseline source")
    require(builds["candidate"]["source"] != BASELINE, "candidate is the unchanged baseline")
    for variant in VARIANTS:
        build = builds[variant]
        hash_value(build["source"], 40, f"{variant} source")
        require(type(build["exit_code"]) is int and build["exit_code"] == 0,
                f"{variant} build failed")
        require(set(build["probes"]) == PROBES, f"{variant} must contain all four probes")
        require(build["profile"] == "release" and build["incremental"] is False,
                f"{variant} is not the recorded nonincremental release build")
        integer(build["jobs"], f"{variant} build jobs")
        require(build["command"] == ["cargo", "build", "--release", "--workspace",
                                     "--examples", "--locked", "--offline"],
                f"unexpected {variant} probe build command")
        for probe, entry in build["probes"].items():
            hash_value(entry["binary_sha256"], 64, f"{variant}/{probe} binary digest")
            hash_value(entry["harness_sha256"], 64, f"{variant}/{probe} harness digest")
            require(isinstance(entry["binary"], str) and entry["binary"], "missing binary path")
            require(entry["harness_sha256"] == digest(task / "probes" / f"{probe}.rs"),
                    f"retained {probe} source differs from {variant} harness")
            require(entry["harness_sha256"] == builds["baseline"]["probes"][probe]["harness_sha256"],
                    f"{probe} harness differs between builds")
    require(builds["baseline"]["jobs"] == builds["candidate"]["jobs"], "build jobs differ")
    require(method["runner_sha256"] == digest(task / "measure.py"), "runner digest mismatch")


def audit_protocol(data, argv):
    expected = []
    for size in [64, 4096, 65536, 1048576]:
        expected += [f"output_delta_{size}", f"text_range_{size}"]
        for count in [0, 8, 64]:
            expected += [f"session_config_{size}_{count}_tools", f"agent_config_{size}_{count}_tools"]
    require([row["case"] for row in data] == expected, "protocol case set/order mismatch")
    result = []
    timed = 0
    for row in data:
        require(row["schema"] == "task104.protocol.v1", "wrong protocol schema")
        encoded = integer(row["encoded_bytes"], "protocol encoded_bytes")
        target = integer(row["target_bytes"], "protocol target_bytes")
        require(target == int(argv[0]), "protocol target differs from arguments")
        count = integer(row["iterations"], "protocol iterations")
        require(count == min(20000, max(16, target // encoded)), "protocol iteration count mismatch")
        total = integer(row["total_bytes"], "protocol total_bytes")
        require(total == count * encoded, "protocol total bytes mismatch")
        elapsed = integer(row["elapsed_ns"], "protocol elapsed_ns")
        per_call = finite(row["ns_per_iteration"], "protocol ns_per_iteration")
        require(math.isclose(per_call, elapsed / count, rel_tol=1e-12), "protocol rate mismatch")
        dimensions = {key: row[key] for key in ("encoded_bytes", "target_bytes", "iterations", "total_bytes")}
        result.append((row["case"], dimensions, {"operation_ns": [elapsed / count]}))
        timed += elapsed
    return result, timed


def audit_provider(data, argv):
    require(len(data) == 1, "provider must emit one row")
    row = data[0]
    require(row["probe"] == "provider_request_encoding", "wrong provider probe")
    size, count = map(int, argv)
    dimensions = {key: integer(row[key], key) for key in
                  ("message_payload_bytes", "message_count", "iterations", "encoded_bytes")}
    require(dimensions["message_payload_bytes"] == size and dimensions["message_count"] == 17,
            "provider input dimensions mismatch")
    require(dimensions["iterations"] == count, "provider iteration count mismatch")
    elapsed = integer(row["elapsed_ns"], "provider elapsed_ns")
    rss = integer(row["peak_rss_kib_before_validation"], "provider HWM")
    return [("request_encoding", dimensions, {
        "operation_ns": [elapsed / count],
        "whole_process_peak_rss_kib_before_validation": [rss],
    })], elapsed


def audit_compaction(data, argv):
    groups, body, count = map(int, argv)
    require(len(data) == count, "compaction repetition count mismatch")
    for row in data:
        integer(row["repetition"], "compaction repetition", minimum=0)
    require([row["repetition"] for row in data] == list(range(count)), "compaction repetition order mismatch")
    keys = ("historical_groups", "body_bytes", "source_text_bytes", "covered_items",
            "utility_calls", "before_tokens", "after_tokens")
    dimensions = None
    samples = {key: [] for key in ("prepare_ns", "generate_ns", "total_ns")}
    for row in data:
        require(row["fixture"] == "DeterministicSummaryFixture", "wrong compaction fixture")
        current = {key: integer(row[key], key) for key in keys}
        require(current["historical_groups"] == groups and current["body_bytes"] == body,
                "compaction input dimensions mismatch")
        source = sum(len(f"source-{index:08}: ") + body for index in range(groups))
        require(current["source_text_bytes"] == source, "compaction source byte mismatch")
        require(current["covered_items"] <= groups, "compaction covered protected current input")
        require(dimensions is None or current == dimensions, "compaction semantics changed between repetitions")
        dimensions = current
        for key in samples:
            samples[key].append(integer(row[key], key))
        require(row["prepare_ns"] + row["generate_ns"] == row["total_ns"], "compaction timing sum mismatch")
    return [("prepare_and_generate", dimensions, samples)], sum(samples["total_ns"])


def audit_io(data, argv):
    require(len(data) == 1, "IO must emit one row")
    row = data[0]
    mode, size, count = argv[0], int(argv[1]), int(argv[2])
    position = argv[3] if len(argv) == 4 else "first"
    require(row["fixture"] == "task104_public_io_api", "wrong IO fixture")
    require(row["mode"] == mode and row["position"] == position, "IO mode/page mismatch")
    require(integer(row["size"], "IO size") == size and
            integer(row["repetitions"], "IO repetitions") == count, "IO argument dimensions mismatch")
    measured = row["measurement"]
    samples = measured["samples_ns"]
    require(isinstance(samples, list) and len(samples) == count, "IO sample count mismatch")
    for value in samples:
        integer(value, "IO sample_ns")
    if mode == "metadata":
        keys = ("file_bytes",)
        require(integer(measured["file_bytes"], "metadata file bytes") > size * 8, "metadata payload missing")
    elif mode == "journal":
        keys = ("input_bytes",)
        require(measured["input_bytes"] == size * 8, "journal payload byte mismatch")
    elif mode == "edits":
        keys = ("record_count", "after_index", "returned_records")
        require([measured[key] for key in keys] == [size, size - 2, 1], "edit-page dimensions mismatch")
    else:
        keys = ("source_bytes", "hunk_count", "output_bytes")
        if mode == "diff-long":
            source_bytes, hunks = 2 * (4 + size * 8 + 1), 1
        else:
            source_bytes = 2 * sum(len(f"{'old' if index % 12 == 0 else 'same'} {index}: x中🙂\n".encode())
                                   for index in range(size))
            hunks = (size + 11) // 12
        require(measured["source_bytes"] == source_bytes and measured["hunk_count"] == hunks,
                "diff source/hunk dimensions mismatch")
        output = integer(measured["output_bytes"], "diff output bytes")
        require(output <= (source_bytes if position == "full" else 128), "diff page exceeds requested range")
        if mode == "diff-long" and position == "full":
            require(output == source_bytes, "complete long diff omitted source lines")
    require(set(measured) == {*keys, "samples_ns"}, "unexpected IO measurement fields")
    dimensions = {key: integer(measured[key], key) for key in keys}
    dimensions.update(mode=mode, size=size, repetitions=count, position=position)
    return [(mode, dimensions, {"operation_ns": samples})], sum(samples)


def metric_summary(round_data, metric):
    pairs = []
    for round_index, entries in enumerate(round_data):
        before = entries["baseline"]["metrics"][metric]
        after = entries["candidate"]["metrics"][metric]
        baseline, candidate = statistics.median(before), statistics.median(after)
        pairs.append({
            "round": round_index,
            "baseline_samples": before, "candidate_samples": after,
            "baseline_round_median": baseline, "candidate_round_median": candidate,
            "candidate_over_baseline": candidate / baseline,
        })
    ratios = [pair["candidate_over_baseline"] for pair in pairs]
    return {
        "rounds": pairs,
        "baseline_median": statistics.median(pair["baseline_round_median"] for pair in pairs),
        "candidate_median": statistics.median(pair["candidate_round_median"] for pair in pairs),
        "baseline_round_median_range": [min(pair["baseline_round_median"] for pair in pairs),
                                        max(pair["baseline_round_median"] for pair in pairs)],
        "candidate_round_median_range": [min(pair["candidate_round_median"] for pair in pairs),
                                         max(pair["candidate_round_median"] for pair in pairs)],
        "median_paired_ratio": statistics.median(ratios),
        "paired_ratio_range": [min(ratios), max(ratios)],
        "median_percent_change": (statistics.median(ratios) - 1) * 100,
        "percent_change_range": [(min(ratios) - 1) * 100, (max(ratios) - 1) * 100],
    }


def summarize(directory):
    task = Path(__file__).resolve().parent
    method_path, raw_path, completion_path = [directory / name for name in
                                             ("method.json", "commands.jsonl", "completion.json")]
    method, completion = parse(method_path.read_text()), parse(completion_path.read_text())
    rounds = integer(method["rounds"], "rounds")
    cases = workloads()
    require(method["workloads"] == [{"probe": probe, "arguments": argv} for probe, argv in cases],
            "workload list differs from audited fixtures")
    require(method["pair_order"] == "baseline/candidate on even rounds; reverse on odd rounds" and
            method["case_order"] == "forward on even rounds; reverse on odd rounds", "unexpected ordering method")
    audit_builds(method, task)
    commands = rows(raw_path.read_text())
    expected_count = rounds * len(cases) * 2
    integer(completion["completed_commands"], "completed_commands")
    require(len(commands) == expected_count and completion["completed_commands"] == expected_count,
            "incomplete or extra command records")
    require(completion["all_commands_succeeded"] is True, "measurement did not complete successfully")
    require(completion["raw_sha256"] == digest(raw_path), "completion raw digest mismatch")
    expected = []
    for round_index in range(rounds):
        ordered = cases if round_index % 2 == 0 else list(reversed(cases))
        variants = VARIANTS if round_index % 2 == 0 else tuple(reversed(VARIANTS))
        expected.extend((round_index, variant, probe, argv) for probe, argv in ordered for variant in variants)
    observations = {}
    audit = {"protocol": audit_protocol, "provider": audit_provider,
             "compaction": audit_compaction, "io": audit_io}
    for line, (record, expected_record) in enumerate(zip(commands, expected), 1):
        round_index, variant, probe, argv = expected_record
        integer(record["round"], "command round", minimum=0)
        require((record["round"], record["variant"], record["probe"], record["arguments"]) == expected_record,
                f"command identity/order mismatch at line {line}")
        binary = method["builds"][variant]["probes"][probe]["binary"]
        require(record["command"] == [binary, *argv], f"command/build mismatch at line {line}")
        require(type(record["exit_code"]) is int and record["exit_code"] == 0, f"failed command at line {line}")
        wall = integer(record["wall_ns"], "command wall_ns")
        finite(record["started_unix"], "command start")
        for kind in ("user", "system"):
            finite(record["process_cpu_seconds"][kind], f"process CPU {kind}")
        require(isinstance(record["stderr"], str), "stderr is not retained text")
        parsed, timed = audit[probe](rows(record["stdout"]), argv)
        require(timed <= wall, f"timed operations exceed process wall time at line {line}")
        for case, dimensions, metrics in parsed:
            key = (probe, tuple(argv), case)
            entry = observations.setdefault(key, {"dimensions": dimensions, "rounds": [{} for _ in range(rounds)]})
            # Audit every pair and every repetition before computing any ratio.
            require(dimensions == entry["dimensions"], f"semantic workload mismatch: {key}")
            require(variant not in entry["rounds"][round_index], f"duplicate case/round/variant: {key}")
            entry["rounds"][round_index][variant] = {"raw_command_line": line, "metrics": metrics}
    require(len(observations) == 61, "expected all 61 cases across the 30 command workloads")
    for key, entry in observations.items():
        metric_keys = set(entry["rounds"][0]["baseline"]["metrics"])
        for pair in entry["rounds"]:
            require(set(pair) == set(VARIANTS), f"unpaired result: {key}")
            require(all(set(pair[variant]["metrics"]) == metric_keys for variant in VARIANTS),
                    f"metric set mismatch: {key}")
    summarized = []
    for (probe, argv, case), entry in sorted(observations.items()):
        summarized.append({
            "probe": probe, "arguments": list(argv), "case": case, "dimensions": entry["dimensions"],
            "raw_command_lines": [{variant: pair[variant]["raw_command_line"] for variant in VARIANTS}
                                  for pair in entry["rounds"]],
            "metrics": {metric: metric_summary(entry["rounds"], metric)
                        for metric in sorted(entry["rounds"][0]["baseline"]["metrics"])},
        })
    return {
        "schema": "task104.paired-summary.v1",
        "auditor_sha256": digest(Path(__file__)),
        "inputs": {path.name: digest(path) for path in (method_path, raw_path, completion_path)},
        "sources": {variant: method["builds"][variant]["source"] for variant in VARIANTS},
        "rounds": rounds, "workload_count": len(cases), "command_count": len(commands),
        "case_count": len(summarized), "all_semantic_dimensions_matched": True,
        "aggregation": "Median within each command, then median of paired candidate/baseline round ratios; all emitted samples retained, none discarded.",
        "percent_change_definition": "100 * (candidate / baseline - 1); negative means a lower measured value.",
        "limits": [
            "Process rounds are the comparison units; nested repetitions are correlated and are not additional independent runs.",
            "Protocol/provider operation_ns values normalize one aggregate timer; they are not per-operation latency samples or percentiles.",
            "Compaction retains its first repetition; no outlier or cold-sample removal is performed.",
            "Provider HWM is a separate whole-process peak observation including immutable fixture and warmup, not allocated bytes.",
            "IO diff setup includes a full diff and does not isolate small-page peak memory; whole-process wall/CPU remain in raw commands.",
            "Runner verified executable binary digests before measurement; this archive audit checks recorded provenance and retained harness hashes without requiring temporary binaries.",
            "No machine isolation, live-provider, network, TUI, saturation, capacity, or statistical significance claim.",
        ],
        "cases": summarized,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paired_dir", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()
    summary = summarize(args.paired_dir)
    output = args.paired_dir / "summary.json"
    if args.write:
        output.write_text(json.dumps(summary, indent=2, sort_keys=True, allow_nan=False) + "\n")
        print(f"Wrote audited summary: {summary['command_count']} commands, {summary['case_count']} cases")
    else:
        require(parse(output.read_text()) == summary, "summary differs from recomputed audited evidence")
        print(f"Verified summary: {summary['command_count']} commands, {summary['case_count']} cases")


if __name__ == "__main__":
    main()
