#!/usr/bin/env python3
"""Read-only consistency audit of this task's retained raw measurements.

Run from the repository root. This never launches a Host or fabricates samples.
It is not a substitute for the live harness's identity/body/cleanup assertions.
"""
import collections
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path("scripts").resolve()))
from task090_perf import aggregate_rss, resource_snapshots
from task090_summarize import fresh_summary


ROOT = Path("tasks/090-fresh-performance/evidence")
for name, count in [("smoke-aa64dc7", 1), ("measured-aa64dc7", 3)]:
    directory = ROOT / name
    data = json.loads((directory / "results.json").read_text())
    assert "failure" not in data and len(data["runs"]) == count
    environment = data["environment"]
    assert environment["source_revision"] == "f36761a164c6f3ae092c5e0bbf7ab08bbcd58891"
    assert environment["harness_revision"] == "aa64dc732b52dd63f2813cb67103311973a67714"
    assert environment["harness_sha256"] == hashlib.sha256(
        Path("scripts/task090_perf.py").read_bytes()).hexdigest()
    assert json.loads((directory / "summary.json").read_text()) == fresh_summary(data)
    for run in data["runs"]:
        records = [json.loads(line) for line in
                   (directory / f"run-{run['run']:02d}.jsonl").read_text().splitlines()]
        kinds = collections.Counter(record["kind"] for record in records)
        assert kinds["fixture_error"] == 0 and kinds["model_request"] == 14
        assert kinds["model_emission"] == 104 and kinds["actual_cleanup_witness"] == 1
        requests = {record["id"]: record for record in records if record["kind"] == "rpc_send"}
        responses = {record["value"]["id"]: record for record in records
                     if record["kind"] == "stdout" and "id" in record["value"]}
        assert requests.keys() == responses.keys()
        assert all("error" not in response["value"] for response in responses.values())
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
