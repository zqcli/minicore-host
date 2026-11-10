#!/usr/bin/env python3
"""Read-only Task098 audit of exact gzip-retained raw measurements.

Run from the repository root. This never launches a Host or fabricates samples.
It is not a substitute for the live harness's identity/body/cleanup assertions.
"""
import collections
import hashlib
import gzip
import json
from pathlib import Path
import sys

REPOSITORY = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPOSITORY / "scripts"))
from task090_perf import aggregate_rss, resource_snapshots
from task090_summarize import fresh_summary


ROOT = Path(__file__).resolve().parent / "evidence"
BUILD = json.loads((ROOT / "build-provenance.json").read_text())
for path, expected in BUILD["unchanged_harness_sha256"].items():
    assert hashlib.sha256((REPOSITORY / path).read_bytes()).hexdigest() == expected, path
COMPRESSION = json.loads((ROOT / "raw-compression.json").read_text())
assert {entry["gzip_file"] for entry in COMPRESSION["files"]} == {
    "smoke-c22d8c8/run-01.jsonl.gz", "measured-c22d8c8/run-01.jsonl.gz",
    "measured-c22d8c8/run-02.jsonl.gz", "measured-c22d8c8/run-03.jsonl.gz",
}
assert len(COMPRESSION["files"]) == 4
for entry in COMPRESSION["files"]:
    compressed = (ROOT / entry["gzip_file"]).read_bytes()
    assert len(compressed) == entry["gzip_bytes"]
    assert hashlib.sha256(compressed).hexdigest() == entry["gzip_sha256"]
    raw = gzip.decompress(compressed)
    assert len(raw) == entry["raw_bytes"]
    assert hashlib.sha256(raw).hexdigest() == entry["raw_sha256"]
print("All four retained gzip files reproduce the exact raw byte counts and SHA-256 values.")
for name, count in [("smoke-c22d8c8", 1), ("measured-c22d8c8", 3)]:
    directory = ROOT / name
    data = json.loads((directory / "results.json").read_text())
    assert "failure" not in data and len(data["runs"]) == count
    environment = data["environment"]
    assert environment["source_revision"] == "c22d8c8af058b756d5f37bc6d1a01f1dc7749934"
    assert environment["harness_revision"] == "c22d8c8af058b756d5f37bc6d1a01f1dc7749934"
    assert environment["harness_sha256"] == hashlib.sha256(
        (REPOSITORY / "scripts/task090_perf.py").read_bytes()).hexdigest()
    assert environment["binary_sha256"] == BUILD["binary_sha256"]
    assert environment["cargo_lock_sha256"] == BUILD["cargo_lock_sha256"]
    assert json.loads((directory / "summary.json").read_text()) == fresh_summary(data)
    for run in data["runs"]:
        raw_path = directory / f"run-{run['run']:02d}.jsonl.gz"
        with gzip.open(raw_path, "rt") as raw:
            records = [json.loads(line) for line in raw]

        kinds = collections.Counter(record["kind"] for record in records)
        assert kinds["fixture_error"] == 0 and kinds["model_request"] == 14
        assert kinds["model_emission"] == 104 and kinds["actual_cleanup_witness"] == 1
        requests = {record["id"]: record for record in records if record["kind"] == "rpc_send"}
        responses = {record["value"]["id"]: record for record in records
                     if record["kind"] == "stdout" and "id" in record["value"]}
        assert requests.keys() == responses.keys()
        assert all("error" not in response["value"] for response in responses.values())
        cancel = next(request for request in requests.values() if request["method"] == "turn.cancel")
        start = cancel["started_ns"]
        turn_id = cancel["params"]["turn_id"]
        cancel_result = responses[cancel["id"]]
        assert run["cancel_tool"]["receipt"] == cancel_result["value"]["result"]
        assert run["cancel_tool"]["control_accept_ms"] == (cancel_result["received_ns"] - start) / 1e6
        wait = next(request for request in requests.values()
                    if request["method"] == "turn.wait" and request["params"]["turn_id"] == turn_id)
        assert run["cancel_tool"]["turn_wait_returned_ms"] == (responses[wait["id"]]["received_ns"] - start) / 1e6
        finished = next(record for record in records if record["kind"] == "stdout"
                        and record["value"].get("method") == "events.push"
                        and record["value"]["params"]["event"]["kind"] == "turn.finished"
                        and record["value"]["params"]["event"]["turn_id"] == turn_id)
        assert run["cancel_tool"]["turn_finished_observed_ms"] == (finished["received_ns"] - start) / 1e6
        cleanup = next(record for record in records if record["kind"] == "actual_cleanup_witness")
        assert run["cancel_tool"]["pid_group_gone_verified_ms"] == (cleanup["verified_ns"] - start) / 1e6
        assert run["cancel_tool"]["tool_result"] == cleanup["native_result"]
        shutdown = next(request for request in requests.values() if request["method"] == "host.shutdown")
        assert run["shutdown"]["accept_ms"] == (responses[shutdown["id"]]["received_ns"] - shutdown["started_ns"]) / 1e6
        for component, metric in [("host.owner", "owner_stopped_diagnostic_observed_ms"),
                                  ("host.process", "stopped_diagnostic_observed_ms")]:
            record = next(record for record in records if record["kind"] == "stderr"
                          and record["value"].get("event") == "host.stopped"
                          and record["value"]["component"] == component)
            assert run["shutdown"][metric] == (record["received_ns"] - shutdown["started_ns"]) / 1e6
        messages = {record["params"]["message_id"]: record for record in requests.values()
                    if record["method"] == "message.send"}
        assert len(messages) == 46
        samples = run["paused_queue"]["samples"] + [
            sample for work in run["stream"] for sample in work["samples"]]
        assert len(samples) == 45
        for sample in samples:
            request = messages[sample["receipt"]["message_id"]]
            response = responses[request["id"]]
            assert sample["receipt"] == response["value"]["result"]
            assert sample["started_ns"] == request["started_ns"]
            assert sample["accepted_ns"] == response["received_ns"]
            assert sample["accept_ms"] == (response["received_ns"] - request["started_ns"]) / 1e6
        emissions = {(record["token"], record["index"]): record for record in records
                     if record["kind"] == "model_emission"}
        observations = 0
        for work in run["stream"]:
            assert work["concurrent"]["active_turns"] == work["parallel_turns"]
            for sample in work["samples"]:
                token = messages[sample["receipt"]["message_id"]]["params"]["text"]
                for observation in sample["observations"]:
                    emission = emissions[token, observation["chunk"]]
                    matching = [record for record in records if record["kind"] == "stdout"
                        and record["value"].get("method") == "events.push"
                        and record["value"]["params"]["subscription_id"] == observation["subscription_id"]
                        and record["value"]["params"]["event"]["kind"] == "model.output.delta"
                        and record["value"]["params"]["event"]["turn_id"] == sample["turn_id"]
                        and record["value"]["params"]["event"]["data"]["end"] >= emission["end"]]
                    at = min(record["received_ns"] for record in matching)
                    assert observation["observed_ns"] == at
                    assert observation["body_ms"] == (at - emission["before_write_ns"]) / 1e6
                    observations += 1
        assert observations == 104
        periodic = [{key: value for key, value in record.items() if key != "kind"}
                    for record in records if record["kind"] == "process_sample"]
        assert len(periodic) == run["periodic_rss_samples"]
        rss = aggregate_rss(periodic + [sample for _, sample in resource_snapshots(run)])
        assert all(run[key] == value for key, value in rss.items())
        resources = run["host_lifetime_sampled_resources"]
        assert resources["start"] == periodic[0] and resources["end"] == periodic[-1]
        stopped = [record["value"]["component"] for record in records if record["kind"] == "stderr"
                   and record["value"].get("event") == "host.stopped"]
        assert sorted(stopped) == ["host.owner", "host.process"]
        print(f"{name}/run-{run['run']:02d}: {len(records)} JSONL records; "
              f"{len(requests)} RPCs matched without errors; 46 sends; 14 real HTTP requests; "
              "104 emitted chunks; 104 matching observer/chunk timings; cleanup witness "
              "and both stopped components; resource union verified")
    print(f"{name}: environment/source/harness hashes and regenerated summary match")
print("No failed run was reclassified as successful. Smoke results remain separate "
      "from the 3-run reported sample.")
