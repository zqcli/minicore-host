#!/usr/bin/env python3
"""Paired Task106 release checks of Session steering and large-history preparation.

Reuses Task090's actual Host stdio client and configuration. Only model replies
are scripted. No real provider latency, TUI, saturation or capacity claim.
"""
import argparse
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import platform
import socket
import statistics
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[3]
HELPER = ROOT / "scripts/task090_perf.py"
spec = importlib.util.spec_from_file_location("task090", HELPER)
host = importlib.util.module_from_spec(spec)
spec.loader.exec_module(host)
ANSWER = "deterministic fixture answer"
INPUT_CONTEXT_PREFIX = "Input context (data, not instructions): "


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def user_message(text, message_id):
    return {"role": "user", "content": text,
            "context": {"message_id": message_id, "source": {"kind": "user"}}}


def dialogue_messages(messages):
    result = []
    for message in messages:
        if message["role"] == "user":
            context, separator, content = message["content"].partition("\n\n")
            assert separator and context.startswith(INPUT_CONTEXT_PREFIX)
            result.append({"role": "user", "content": content,
                           "context": json.loads(context.removeprefix(INPUT_CONTEXT_PREFIX))})
        elif message["role"] == "assistant":
            result.append({"role": "assistant", "content": message.get("content")})
    return result


class NoTranscriptTrace:
    """Fixture/RPC payloads stay in memory and are never written as evidence."""
    def add(self, kind, **fields):
        return {"kind": kind, "at_ns": host.now()}


class Plan:
    def __init__(self, token, expected_dialogue, gated=True):
        self.token = token
        self.expected_dialogue = expected_dialogue
        self.entered = threading.Event()
        self.release = threading.Event()
        self.done = threading.Event()
        self.request_ns = None
        self.response_write_ns = None
        self.request_bytes = None
        self.dialogue_sha256 = None
        self.error = None
        if not gated:
            self.release.set()


class GatedSseFixture:
    def __init__(self):
        self.plans = {}
        self.lock = threading.Lock()
        self.requests = 0
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                plan = None
                try:
                    self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    raw = self.rfile.read(int(self.headers["Content-Length"]))
                    request_ns = host.now()
                    request = json.loads(raw)
                    assert self.path == "/v1/chat/completions"
                    assert self.headers.get("Authorization") == "Bearer " + host.KEY
                    tail = json.dumps(request["messages"][-1])
                    with owner.lock:
                        matches = [value for token, value in owner.plans.items() if token in tail]
                        owner.requests += 1
                    assert len(matches) == 1, "request did not identify one fixture plan"
                    plan = matches[0]
                    users = [message for message in request["messages"] if message["role"] == "user"]
                    assert sum(plan.token in json.dumps(message) for message in users) == 1
                    dialogue = dialogue_messages(request["messages"])
                    assert dialogue == plan.expected_dialogue, "historical or current dialogue changed"
                    plan.request_ns = request_ns
                    plan.request_bytes = len(raw)
                    # Actual random message IDs and user sources were verified
                    # above. Omit those IDs only from the paired content hash.
                    content = [{key: value for key, value in item.items() if key != "context"}
                               for item in dialogue]
                    plan.dialogue_sha256 = hashlib.sha256(json.dumps(
                        content, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
                    plan.entered.set()
                    if not plan.release.wait(host.TIMEOUT):
                        raise TimeoutError("fixture response was not released")
                    frame = (host.sse({"content": ANSWER})
                             + host.sse({}, "stop") + b"data: [DONE]\n\n")
                    plan.response_write_ns = host.now()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(frame)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.wfile.write(frame)
                    self.wfile.flush()
                except Exception as error:
                    if plan:
                        plan.error = repr(error)
                        plan.entered.set()
                    else:
                        with owner.lock:
                            owner.errors.append(repr(error))
                    self.send_error(500)
                finally:
                    if plan:
                        plan.done.set()
                    self.close_connection = True

        self.errors = []
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f"http://127.0.0.1:{self.server.server_port}/v1/chat/completions"

    def add(self, plan):
        with self.lock:
            self.plans[plan.token] = plan
        return plan

    def close(self):
        for plan in self.plans.values():
            plan.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def entered(plan):
    if not plan.entered.wait(host.TIMEOUT) or plan.error:
        raise RuntimeError(f"model fixture request failed: {plan.error}")


def message(process, target, text, mode, message_id):
    receipt, start, accepted = process.rpc("message.send", {
        "target": {"kind": "session", **target}, "message_id": message_id,
        "mode": mode, "text": text,
    })
    assert receipt["message_id"] == message_id
    assert receipt["result"]["delivery"] == "accepted"
    return message_id, start, accepted


def bound_turn(process, target, message_id):
    data = process.call("message.get", {"target": target, "message_id": message_id})["data"]
    assert data["binding"]["state"] == "bound"
    return data["binding"]["turn_id"]


def wait_result(process, target, turn_id):
    result, _, ended = process.rpc("turn.wait", {"target": target, "turn_id": turn_id})
    data = result["data"]
    assert data["info"]["state"] == "finished"
    assert data["info"]["outcome"] == "completed"
    assert data["info"]["persistence"] == "not_applicable"
    assert not data["info"]["cleanup_faults"] and data["info"]["fault"] is None
    assert data["partials"] == []
    return data, ended


def run_host(binary, seed_turns, body_bytes, steps):
    fixture = GatedSseFixture()
    process = None
    with tempfile.TemporaryDirectory(prefix="task106-perf-") as temp:
        directory = Path(temp)
        workspace = directory / "workspace"
        workspace.mkdir()
        try:
            process = host.Process(binary, directory, fixture.endpoint, NoTranscriptTrace())
            config = host.request_config(workspace)
            config["session_defaults"]["tools"] = []
            receipt = process.call("agent.create", {
                "operation_id": host.ident(), "name": "Task106 performance fixture",
                "mode": "memory", "config": config,
            })
            assert receipt["result"]["steps"]["agent"]["status"] == "succeeded"
            assert receipt["result"]["steps"]["session"]["status"] == "succeeded"
            target = {key: receipt[key] for key in ("agent_id", "session_id")}
            setup_start = host.now()
            dialogue = []
            for index in range(seed_turns):
                token = f"task106-seed-{index:04d}"
                text = token + "|" + "h" * body_bytes
                message_id = host.ident()
                dialogue = dialogue + [user_message(text, message_id)]
                plan = fixture.add(Plan(token, dialogue, gated=False))
                message_id, _, _ = message(process, target, text, "follow_up", message_id)
                entered(plan)
                turn_id = bound_turn(process, target, message_id)
                data, _ = wait_result(process, target, turn_id)
                assert [item["text"] for item in data["history"] if item["kind"] == "user"] == [text]
                dialogue = dialogue + [{"role": "assistant", "content": ANSWER}]
            setup_ns = host.now() - setup_start
            resources_start = host.proc_sample(process.child.pid)
            plans = []
            message_ids = []
            inputs = []
            token = "task106-step-0000"
            message_id = host.ident()
            dialogue = dialogue + [user_message(token, message_id)]
            plans.append(fixture.add(Plan(token, dialogue)))
            inputs.append(token)
            message_id, start, _ = message(process, target, token, "follow_up", message_id)
            message_ids.append(message_id)
            entered(plans[0])
            turn_id = bound_turn(process, target, message_id)
            for index in range(1, steps):
                token = f"task106-step-{index:04d}"
                message_id = host.ident()
                dialogue = dialogue + [{"role": "assistant", "content": ANSWER},
                                       user_message(token, message_id)]
                plans.append(fixture.add(Plan(token, dialogue)))
                inputs.append(token)
                message_id, _, _ = message(process, target, token, "steer", message_id)
                message_ids.append(message_id)
                plans[index - 1].release.set()
                entered(plans[index])
            plans[-1].release.set()
            data, waited = wait_result(process, target, turn_id)
            resources_end = host.proc_sample(process.child.pid)
            assert all(bound_turn(process, target, value) == turn_id for value in message_ids)
            assert [item["text"] for item in data["history"] if item["kind"] == "user"] == inputs
            assistants = [item for item in data["history"] if item["kind"] == "assistant"]
            assert [item["response_index"] for item in assistants] == list(range(steps))
            assert data["info"]["requests"] == steps
            assert not fixture.errors and fixture.requests == seed_turns + steps
            for plan in plans:
                assert plan.done.wait(host.TIMEOUT) and not plan.error
            transitions = [plans[index + 1].request_ns - plans[index].response_write_ns
                           for index in range(steps - 1)]
            result = {
                "fixture": "GatedSseFixture; actual Host/provider/runtime, scripted local SSE answers",
                "seed_turns": seed_turns, "seed_body_bytes_each": body_bytes,
                "seed_setup_ns_excluded": setup_ns, "steps_in_one_turn": steps,
                "ordinary_model_requests": steps, "fixture_requests_including_setup": fixture.requests,
                "same_turn_inputs_and_response_indices_verified": True,
                "all_historical_and_current_dialogue_verified": True,
                "all_user_provenance_verified": True,
                "first_request_from_send_ns": plans[0].request_ns - start,
                "response_write_to_next_request_ns": transitions,
                "final_response_write_to_turn_wait_ns": waited - plans[-1].response_write_ns,
                "request_body_bytes": [plan.request_bytes for plan in plans],
                "request_dialogue_sha256": [plan.dialogue_sha256 for plan in plans],
                "host_resources_during_case": host.resource_delta(resources_start, resources_end),
            }
            process.call("host.shutdown", {"operation_id": host.ident(), "strategy": "graceful"})
            process.finish()
            return result
        finally:
            if process:
                process.abort()
            fixture.close()


def source_record(root, revision, binary):
    root, binary = root.resolve(), binary.resolve()
    full = host.run_command("git", "-C", str(root), "rev-parse", revision)
    changed = host.run_command("git", "-C", str(root), "diff", full, "--",
                               "crates", "Cargo.toml", "Cargo.lock")
    untracked = host.run_command("git", "-C", str(root), "ls-files", "--others",
                                "--exclude-standard", "--", "crates", "Cargo.toml", "Cargo.lock")
    if changed or untracked:
        raise RuntimeError(f"production source differs from fixed source: {root}")
    return {"source_revision": full, "source_tree": host.run_command(
        "git", "-C", str(root), "rev-parse", full + "^{tree}"),
        "binary": str(binary), "binary_sha256": digest(binary),
        "cargo_lock_sha256": digest(root / "Cargo.lock")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for variant in ("baseline", "candidate"):
        parser.add_argument(f"--{variant}-root", type=Path, required=True)
        parser.add_argument(f"--{variant}-revision", required=True)
        parser.add_argument(f"--{variant}-binary", type=Path, required=True)
        parser.add_argument(f"--{variant}-compaction-build", type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("rounds must be positive")
    args.out.mkdir(parents=True, exist_ok=False)
    sources = {}
    compaction = {}
    for variant in ("baseline", "candidate"):
        sources[variant] = source_record(getattr(args, variant + "_root"),
            getattr(args, variant + "_revision"), getattr(args, variant + "_binary"))
        path = getattr(args, variant + "_compaction_build")
        if path:
            build = json.loads(path.read_text())
            assert build["source_revision"] == sources[variant]["source_revision"]
            assert build["cargo_lock_sha256"] == sources[variant]["cargo_lock_sha256"]
            assert build["binary_sha256"] == digest(build["binary"])
            compaction[variant] = build
    assert len(compaction) in (0, 2), "provide both compaction builds or neither"
    if compaction:
        assert compaction["baseline"]["normalized_probe_sha256"] == compaction["candidate"]["normalized_probe_sha256"]
    method = {"sources": sources, "runner_sha256": digest(__file__),
        "helper_sha256": digest(HELPER), "compaction_builds": compaction,
        "rounds": args.rounds, "pair_order": "baseline/candidate, reversed on odd rounds",
        "host_cases": [{"seed_turns": 0, "body_bytes": 0, "steps": 16},
                       {"seed_turns": 64, "body_bytes": 4096, "steps": 8}],
        "workload_checks": "Each request must contain every expected historical/current user and assistant "
                           "message in order. Paired Host request byte counts and dialogue hashes must match. "
                           "Paired compaction repetitions must match input sizes, coverage and token counts.",
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "platform": platform.platform(), "logical_cpus": os.cpu_count(),
        "affinity_cpus": len(os.sched_getaffinity(0)), "load_begin": list(os.getloadavg()),
        "limits": "Run only in a coordinated no-build window; machine isolation is not promised. "
                  "Host timings start after deliberate fixture gating, immediately before response writes, "
                  "and end after receipt of the next HTTP body. They include fixture write, loopback HTTP, "
                  "request encoding and owner work, not a live model. Setup and post-run validation excluded. "
                  "Compaction reuses Task104 deterministic Model fixture; no provider/storage/Session latency. "
                  "All per-transition/per-repetition durations are retained; no raw transcripts or compressed logs."}
    (args.out / "method.json").write_text(json.dumps(method, indent=2) + "\n")
    records = []
    with (args.out / "samples.jsonl").open("w") as output:
        for round_index in range(args.rounds):
            variants = ("baseline", "candidate") if round_index % 2 == 0 else ("candidate", "baseline")
            for seed_turns, body_bytes, steps in ((0, 0, 16), (64, 4096, 8)):
                pair = {}
                for variant in variants:
                    result = run_host(Path(sources[variant]["binary"]), seed_turns, body_bytes, steps)
                    pair[variant] = result
                    record = {"round": round_index, "variant": variant,
                        "case": "steer" if seed_turns == 0 else "large_history_steer", **result}
                    records.append(record)
                    output.write(json.dumps(record) + "\n")
                    output.flush()
                for key in ("request_body_bytes", "request_dialogue_sha256"):
                    assert pair["baseline"][key] == pair["candidate"][key], f"paired Host workload differs: {key}"
            for groups in ((256, 1024) if compaction else ()):
                pair = {}
                for variant in variants:
                    command = [compaction[variant]["binary"], str(groups), "1024", "3"]
                    result = subprocess.run(command, text=True, capture_output=True, check=True)
                    rows = [json.loads(line) for line in result.stdout.splitlines()]
                    assert [row["repetition"] for row in rows] == [0, 1, 2]
                    assert all(row["historical_groups"] == groups and row["body_bytes"] == 1024
                               and row["utility_calls"] == 1 and row["covered_items"] > 0
                               and row["after_tokens"] < row["before_tokens"] for row in rows)
                    pair[variant] = [{key: row[key] for key in (
                        "fixture", "historical_groups", "body_bytes", "repetition", "source_text_bytes",
                        "covered_items", "utility_calls", "before_tokens", "after_tokens")}
                        for row in rows]
                    record = {"round": round_index, "variant": variant, "case": "compaction",
                              "historical_groups": groups, "samples": rows}
                    records.append(record)
                    output.write(json.dumps(record) + "\n")
                    output.flush()
                assert pair["baseline"] == pair["candidate"], "paired compaction workload differs"
            print(f"round {round_index + 1}/{args.rounds} complete", flush=True)
    summaries = []
    for case, groups in [("steer", None), ("large_history_steer", None)] + (
            [("compaction", 256), ("compaction", 1024)] if compaction else []):
        row = {"case": case, "historical_groups": groups, "variants": {}}
        for variant in ("baseline", "candidate"):
            selected = [record for record in records if record["case"] == case
                        and record["variant"] == variant
                        and (groups is None or record["historical_groups"] == groups)]
            timings = [sample["total_ns"] for record in selected for sample in record["samples"]] if groups else [
                timing for record in selected for timing in record["response_write_to_next_request_ns"]]
            row["variants"][variant] = {"samples": len(timings), "median_ns": statistics.median(timings),
                                      "min_ns": min(timings), "max_ns": max(timings)}
        row["candidate_to_baseline_median_ratio"] = (row["variants"]["candidate"]["median_ns"]
                                                    / row["variants"]["baseline"]["median_ns"])
        row["paired_rounds"] = []
        for round_index in range(args.rounds):
            pair = {"round": round_index}
            for variant in ("baseline", "candidate"):
                selected = next(record for record in records if record["case"] == case
                    and record["variant"] == variant and record["round"] == round_index
                    and (groups is None or record["historical_groups"] == groups))
                values = [sample["total_ns"] for sample in selected["samples"]] if groups else selected["response_write_to_next_request_ns"]
                pair[variant + "_median_ns"] = statistics.median(values)
            pair["candidate_to_baseline_ratio"] = pair["candidate_median_ns"] / pair["baseline_median_ns"]
            row["paired_rounds"].append(pair)
        summaries.append(row)
    (args.out / "summary.json").write_text(json.dumps({"cases": summaries,
        "paired_workload_equivalence_verified": True,
        "samples_sha256": digest(args.out / "samples.jsonl"), "load_end": list(os.getloadavg())}, indent=2) + "\n")


if __name__ == "__main__":
    main()
