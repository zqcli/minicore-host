#!/usr/bin/env python3
"""Paired actual-Host checks for Task107 output ownership and Turn binding.

Only the local HTTP model replies are scripted. Tool execution, ownership,
output retention and cleanup use the production Host. No transcripts are saved.
"""
import argparse
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import platform
import queue
import socket
import statistics
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[3]
HELPER = ROOT / "tasks/106-host-runtime/performance/measure.py"
spec = importlib.util.spec_from_file_location("task106", HELPER)
previous = importlib.util.module_from_spec(spec)
spec.loader.exec_module(previous)
host = previous.host
TOOL_ID = "task107-bash-output"
ANSWER = "Task107 deterministic final answer"
STREAM_BYTES = 8 * 1024 * 1024
TOOL_BYTES = 2 * 1024 * 1024
BINDING_TURNS = 32


def checksum(value):
    return hashlib.sha256(value).hexdigest()


def text_frames(text, chunk_bytes=16384):
    return b"".join(host.sse({"content": text[index:index + chunk_bytes]})
                    for index in range(0, len(text), chunk_bytes)) + host.sse({}, "stop") + b"data: [DONE]\n\n"


def canonical_request(request, replacements):
    value = json.loads(host.encoded(request))
    for message in value["messages"]:
        if message["role"] == "tool":
            # The actual PID/exit/cleanup are checked before normalization.
            native = json.loads(message["content"])
            native["pid"] = 0
            message["content"] = json.dumps(native, sort_keys=True, separators=(",", ":"))
    text = json.dumps(value, sort_keys=True, separators=(",", ":"))
    for actual, logical in replacements:
        text = text.replace(actual, logical)
    return text.encode()


class Plan:
    def __init__(self, frames, check, replacements):
        self.frames, self.check, self.replacements = frames, check, replacements
        self.entered, self.release, self.done = (threading.Event() for _ in range(3))
        self.request_ns = self.response_write_ns = self.request_bytes = None
        self.tools_sha256 = self.error = None
        self.canonical_bytes = self.request_sha256 = None


class Fixture:
    def __init__(self):
        self.pending, self.plans, self.errors = queue.Queue(), [], []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                plan = None
                try:
                    self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    raw = self.rfile.read(int(self.headers["Content-Length"]))
                    received = host.now()
                    plan = owner.pending.get_nowait()
                    request = json.loads(raw)
                    assert self.path == "/v1/chat/completions"
                    assert self.headers.get("Authorization") == "Bearer " + host.KEY
                    assert request["model"] == "gated-loopback-sse-fixture"
                    plan.check(request)
                    plan.request_ns, plan.request_bytes = received, len(raw)
                    plan.tools_sha256 = checksum(host.encoded(request.get("tools", [])))
                    canonical = canonical_request(request, plan.replacements)
                    plan.canonical_bytes, plan.request_sha256 = len(canonical), checksum(canonical)
                    plan.entered.set()
                    assert plan.release.wait(host.TIMEOUT), "model fixture gate timed out"
                    plan.response_write_ns = host.now()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(plan.frames)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.wfile.write(plan.frames)
                    self.wfile.flush()
                except Exception as error:
                    if plan:
                        plan.error = repr(error)
                        plan.entered.set()
                    else:
                        owner.errors.append(repr(error))
                    self.send_error(500)
                finally:
                    if plan:
                        plan.done.set()
                    self.close_connection = True

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.endpoint = f"http://127.0.0.1:{self.server.server_port}/v1/chat/completions"

    def add(self, frames, check, replacements):
        plan = Plan(frames, check, replacements)
        self.plans.append(plan)
        self.pending.put(plan)
        return plan

    def close(self):
        for plan in self.plans:
            plan.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


def finish_event(process, subscription, turn_id, identity):
    event, at = process.event(lambda value:
        value.get("method") == "events.push"
        and value["params"]["subscription_id"] == subscription
        and value["params"]["event"]["kind"] == "turn.state"
        and value["params"]["event"]["turn_id"] == turn_id
        and value["params"]["event"]["data"]["state"] == "finished")
    event = event["params"]["event"]
    host.assert_identity(event, identity)
    state = event["data"]
    assert state["outcome"] == "completed" and state["persistence"] == "not_applicable"
    assert state["fault"] is None and state["persistence_fault"] is None and not state["cleanup_faults"]
    return at


def expected_dialogue(dialogue):
    def check(request):
        assert previous.dialogue_messages(request["messages"]) == dialogue, "dialogue or provenance changed"
        assert [tool["function"]["name"] for tool in request.get("tools", [])] == ["bash"]
    return check


def read_output(process, method, params, identity, channel, expected):
    value = process.call(method, {**params, "request": {
        "channel": channel, "start": 0, "end": len(expected), "max_bytes": max(len(expected), 1)}})
    for key, expected_identity in identity.items():
        assert value[key] == expected_identity, f"output identity differs: {key}"
    data = value["data"]
    assert data == {"channel": channel, "start": 0, "end": len(expected),
                    "text": expected.decode(), "has_more": False}, "retained output differs"


def check_native(result, stdout, stderr):
    assert result["status"] == "exited" and result["exit_code"] == 0 and result["signal"] is None
    assert result["process_waited"] and result["cleanup_confirmed"] and not result["cleanup_errors"]
    for name, expected in (("stdout", stdout), ("stderr", stderr)):
        assert result[name] == {"output": {"encoding": "utf8", "data": expected}, "complete": True}


def run_case(binary, case):
    fixture, process = Fixture(), None
    with tempfile.TemporaryDirectory(prefix="task107-perf-") as temp:
        directory = Path(temp)
        workspace = directory / "workspace"
        workspace.mkdir()
        try:
            process = host.Process(binary, directory, fixture.endpoint, previous.NoTranscriptTrace())
            created = process.call("agent.create", {"operation_id": host.ident(),
                "name": "Task107 performance fixture", "mode": "memory",
                "config": host.request_config(workspace)})
            assert all(created["result"]["steps"][step]["status"] == "succeeded"
                       for step in ("agent", "session"))
            target = {key: created[key] for key in ("agent_id", "session_id")}
            subscription = host.observe(process, target, ["state"])
            dialogue, request_times, records = [], [], []
            expected_bytes = b""
            before = host.proc_sample(process.child.pid)
            turns = BINDING_TURNS if case == "binding" else 1
            for index in range(turns):
                text, message_id = f"task107-{case}-{index:04d}", host.ident()
                dialogue = dialogue + [previous.user_message(text, message_id)]
                replacements = [(str(workspace), "/fixture/workspace")]
                replacements.extend((message["context"]["message_id"], f"m{index:019d}")
                                    for index, message in enumerate(dialogue) if message["role"] == "user")
                answer = "x" * STREAM_BYTES if case == "model_output" else ANSWER
                if case == "tool_output":
                    # Below Linux's per-argument exec limit. Both streams are
                    # actual bash child output, not injected observation data.
                    command = ("python3 -c 'import os; "
                               f"os.write(1, b\"o\" * {TOOL_BYTES}); "
                               f"os.write(2, b\"e\" * {TOOL_BYTES})'\n#" + "p" * 65536)
                    arguments = {"command": command}
                    frames = host.sse({"tool_calls": [{"index": 0, "id": TOOL_ID,
                        "type": "function", "function": {"name": "bash", "arguments": json.dumps(arguments)}}]})
                    frames += host.sse({}, "tool_calls") + b"data: [DONE]\n\n"
                    stdout, stderr = "o" * TOOL_BYTES, "e" * TOOL_BYTES

                    def after_tool(request):
                        messages = request["messages"]
                        assert previous.dialogue_messages(messages[:-2]) == dialogue
                        assert messages[-2]["tool_calls"][0]["id"] == TOOL_ID
                        assert json.loads(messages[-2]["tool_calls"][0]["function"]["arguments"]) == arguments
                        assert messages[-1]["role"] == "tool" and messages[-1]["tool_call_id"] == TOOL_ID
                        check_native(json.loads(messages[-1]["content"]), stdout, stderr)

                    plan = fixture.add(frames, expected_dialogue(dialogue), replacements)
                    next_plan = fixture.add(text_frames(answer), after_tool, replacements)
                else:
                    plan = fixture.add(text_frames(answer), expected_dialogue(dialogue), replacements)
                receipt, sent, _ = process.rpc("message.send", {
                    "target": {"kind": "session", **target}, "message_id": message_id,
                    "mode": "follow_up", "text": text})
                host.assert_identity(receipt, {**process.host_identity, **target, "message_id": message_id})
                assert receipt["result"]["delivery"] == "accepted"
                previous.entered(plan)
                turn_id = previous.bound_turn(process, target, receipt["message_id"])
                turn = process.call("turn.get", {"target": target, "turn_id": turn_id})
                identity = {**process.host_identity, **target, "turn_id": turn_id,
                            "load_id": turn["load_id"]}
                host.assert_identity(turn, identity)
                request_times.append(plan.request_ns - sent)
                plan.release.set()
                if case == "tool_output":
                    previous.entered(next_plan)
                    next_plan.release.set()
                finished = finish_event(process, subscription, turn_id, identity)
                after = host.proc_sample(process.child.pid)
                # All large reads are deliberately after the measured memory
                # boundary, so serialization does not create the measured peak.
                data, _ = previous.wait_result(process, target, turn_id)
                assert data["info"]["requests"] == (2 if case == "tool_output" else 1)
                assert [item["text"] for item in data["history"] if item["kind"] == "user"] == [text]
                expected_bytes = answer.encode()
                final = [item for item in data["history"] if item["kind"] == "assistant"][-1]
                assert b"".join(part["data"].encode() for part in final["response"]["content"]
                                if part["type"] == "text") == expected_bytes
                read_output(process, "turn.output.read", {"target": target, "turn_id": turn_id,
                    "response_index": 1 if case == "tool_output" else 0},
                    {**identity, "response_index": 1 if case == "tool_output" else 0}, "text", expected_bytes)
                if case == "tool_output":
                    tool_ref = {"turn_id": turn_id, "tool_call_id": TOOL_ID}
                    native = host.query_data(process.call("toolcall.get", {"target": target, "tool": tool_ref}),
                                             {**identity, "tool_call_id": TOOL_ID})["result"]
                    assert native["outcome"] == "success"
                    native_command = json.loads(native["output"]["content"])
                    check_native(native_command, stdout, stderr)
                    for channel, expected in (("stdout", stdout), ("stderr", stderr)):
                        read_output(process, "toolcall.output.read", {"target": target, "tool": tool_ref},
                                    {**identity, "tool_call_id": TOOL_ID}, channel, expected.encode())
                    assert not Path(f"/proc/{native_command['pid']}").exists(), "tool process remains"
                    records.append({"first_response_write_to_next_request_ns": next_plan.request_ns - plan.response_write_ns,
                                    "command_bytes": len(command), "stdout_bytes": len(stdout), "stderr_bytes": len(stderr),
                                    "stdout_sha256": checksum(stdout.encode()), "stderr_sha256": checksum(stderr.encode())})
                records.append({"response_write_to_finished_state_ns": finished - plan.response_write_ns,
                                "resources_after_finished_before_result_read": after})
                dialogue = dialogue + [{"role": "assistant", "content": answer}]
            assert fixture.pending.empty() and not fixture.errors
            for plan in fixture.plans:
                assert plan.done.wait(host.TIMEOUT) and not plan.error
            result = {"case": case, "turns": turns, "fixture_requests": len(fixture.plans),
                "first_request_from_send_ns": request_times, "samples": records,
                "answer_bytes": len(expected_bytes), "answer_sha256": checksum(expected_bytes),
                "tools_sha256": [plan.tools_sha256 for plan in fixture.plans],
                "request_body_bytes": [plan.request_bytes for plan in fixture.plans],
                "canonical_request_body_bytes": [plan.canonical_bytes for plan in fixture.plans],
                "canonical_request_sha256": [plan.request_sha256 for plan in fixture.plans],
                "all_dialogue_provenance_output_cleanup_checks_passed": True,
                "host_resources": host.resource_delta(before, after)}
            process.call("host.shutdown", {"operation_id": host.ident(), "strategy": "graceful"})
            process.finish()
            return result
        finally:
            if process:
                process.abort()
            fixture.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for variant in ("baseline", "candidate"):
        parser.add_argument(f"--{variant}-root", type=Path, required=True)
        parser.add_argument(f"--{variant}-revision", required=True)
        parser.add_argument(f"--{variant}-binary", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=4)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    assert args.rounds > 0
    sources = {variant: previous.source_record(getattr(args, variant + "_root"),
        getattr(args, variant + "_revision"), getattr(args, variant + "_binary"))
        for variant in ("baseline", "candidate")}
    args.out.mkdir(parents=True, exist_ok=False)
    method = {"sources": sources, "rounds": args.rounds,
        "runner_sha256": previous.digest(__file__), "helper_sha256": previous.digest(HELPER),
        "transport_helper_sha256": previous.digest(previous.HELPER),
        "pair_order": "AB/BA, repeated; all measured rounds retained",
        "workload_checks": "Every full request is compared by canonical byte count and SHA-256, including system "
                           "instructions, tool declarations and top-level parameters. Only verified message IDs, "
                           "the temporary workspace path and checked native process PID are normalized. Raw request "
                           "byte counts are also retained (PID digit count may differ in the tool result).",
        "stream_bytes": STREAM_BYTES, "tool_bytes_per_stream": TOOL_BYTES, "binding_turns": BINDING_TURNS,
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "platform": platform.platform(), "affinity_cpus": len(os.sched_getaffinity(0)),
        "load_begin": list(os.getloadavg()),
        "limits": "No builds during measurement; shared machine, no isolation promise. Local deterministic HTTP/SSE model, "
                  "real Host and bash child. Finished-state timestamp is client arrival, not an internal owner clock. "
                  "Model/Tool output queries happen after memory/time boundary. RSS is Linux process memory, including allocator retention. "
                  "Binding starts at RPC send and includes HTTP encoding/receipt. No live provider, TUI or saturation claim."}
    (args.out / "method.json").write_text(json.dumps(method, indent=2) + "\n")
    rows = []
    with (args.out / "samples.jsonl").open("w") as output:
        for index in range(args.rounds):
            order = ("baseline", "candidate") if index % 2 == 0 else ("candidate", "baseline")
            for case in ("binding", "tool_output", "model_output"):
                pair = {}
                for variant in order:
                    value = run_case(Path(sources[variant]["binary"]), case)
                    pair[variant] = value
                    row = {"round": index, "variant": variant, **value}
                    rows.append(row)
                    output.write(json.dumps(row) + "\n")
                    output.flush()
                for key in ("turns", "fixture_requests", "answer_bytes", "answer_sha256", "tools_sha256",
                            "canonical_request_body_bytes", "canonical_request_sha256"):
                    assert pair["baseline"][key] == pair["candidate"][key], f"paired workload differs: {key}"
            print(f"output round {index + 1}/{args.rounds} complete", flush=True)
    summaries = []
    for case, metric in (("binding", "request_ns"), ("tool_output", "tool_transition_ns"),
                         ("model_output", "finished_ns"), ("model_output", "retained_rss_kib")):
        values = {}
        for variant in ("baseline", "candidate"):
            selected = [row for row in rows if row["case"] == case and row["variant"] == variant]
            if metric == "request_ns":
                samples = [value for row in selected for value in row["first_request_from_send_ns"]]
            elif metric == "tool_transition_ns":
                samples = [row["samples"][0]["first_response_write_to_next_request_ns"] for row in selected]
            elif metric == "finished_ns":
                samples = [row["samples"][-1]["response_write_to_finished_state_ns"] for row in selected]
            else:
                samples = [row["samples"][-1]["resources_after_finished_before_result_read"]["rss_kib"]
                           for row in selected]
            values[variant] = {"samples": len(samples), "median": statistics.median(samples),
                               "min": min(samples), "max": max(samples)}
        summaries.append({"case": case, "metric": metric, "variants": values,
            "candidate_to_baseline_median_ratio": values["candidate"]["median"] / values["baseline"]["median"]})
    (args.out / "summary.json").write_text(json.dumps({"cases": summaries,
        "paired_workload_equivalence_verified": True, "load_end": list(os.getloadavg()),
        "samples_sha256": previous.digest(args.out / "samples.jsonl")}, indent=2) + "\n")


if __name__ == "__main__":
    main()
