#!/usr/bin/env python3
"""Opt-in Linux local Host measurements. No fake Runtime, external API or packages.

Build the actual minicore-host executable first, then run --help. This is a
bounded first-version workload, not a load generator or product capacity test.
The harness uses the current public query projections only. Historical Task058
measurements retain their frozen harness and must never be overwritten.
"""
import argparse
import collections
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"
MODEL = "fixture/performance"
KEY = "task058-loopback-fixture-only"
TIMEOUT = 20
TICK_HZ = os.sysconf("SC_CLK_TCK")
PAGE_BYTES = os.sysconf("SC_PAGE_SIZE")


def now():
    return time.monotonic_ns()


def ident():
    return "".join(secrets.choice(ALPHABET) for _ in range(20))


def encoded(value):
    return json.dumps(value, separators=(",", ":")).encode()


def run_command(*args):
    return subprocess.check_output(args, text=True).strip()


class Trace:
    def __init__(self, path):
        self.file = path.open("w")
        self.lock = threading.Lock()

    def add(self, kind, **fields):
        record = {"kind": kind, "at_ns": now(), **fields}
        with self.lock:
            self.file.write(json.dumps(record, separators=(",", ":")) + "\n")
        return record

    def close(self):
        self.file.close()


def proc_sample(pid):
    """Linux /proc counters, not estimates based on workload sizes."""
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    status = Path(f"/proc/{pid}/status").read_text().splitlines()
    memory = {
        key: int(line.split()[1])
        for key in ("VmRSS", "VmHWM")
        for line in status if line.startswith(key + ":")
    }
    return {
        "pid": pid, "at_ns": now(), "user_ticks": int(fields[11]),
        "system_ticks": int(fields[12]), "rss_kib": memory.get("VmRSS"),
        "high_water_rss_kib": memory.get("VmHWM"),
        "threads": int(fields[17]), "state": fields[0], "start_ticks": int(fields[19]),
    }


def resource_snapshots(value, path="run"):
    """Find actual retained resource boundaries, not inferred memory values."""
    if isinstance(value, dict):
        if {"pid", "at_ns", "rss_kib", "high_water_rss_kib"} <= value.keys():
            yield path, value
        else:
            for key, item in value.items():
                yield from resource_snapshots(item, f"{path}.{key}")
    elif isinstance(value, list):
        for index, item in enumerate(value):
            yield from resource_snapshots(item, f"{path}[{index}]")


def aggregate_rss(samples):
    """Combine periodic and explicit samples once per actual PID/timestamp."""
    unique = {(sample["pid"], sample["at_ns"]): sample for sample in samples}
    rss = [sample["rss_kib"] for sample in unique.values() if sample["rss_kib"] is not None]
    high_water = [sample["high_water_rss_kib"] for sample in unique.values()
                  if sample["high_water_rss_kib"] is not None]
    return {"host_rss_peak_sampled_kib": max(rss, default=None),
            "host_rss_high_water_kib": max(high_water, default=None),
            "rss_sample_scope": "periodic_and_explicit_resource_boundaries",
            "rss_samples_present": len(rss), "rss_samples_missing": len(unique) - len(rss),
            "high_water_samples_present": len(high_water),
            "high_water_samples_missing": len(unique) - len(high_water)}


class Process:
    def __init__(self, binary, directory, endpoint, trace):
        self.trace = trace
        config = {
            "name": "task058 actual Host", "framework_instructions": "",
            "models": [{"model": MODEL, "endpoint": endpoint,
                        "provider_model": "gated-loopback-sse-fixture",
                        "api_key_env": "TASK058_MODEL_KEY", "context_window": 128000,
                        "supported_reasoning": ["auto"], "supports_tools": True,
                        "request_timeout_ms": 20000}],
            "tools": [{"name": "bash"}],
            "network": {"listen": None, "access_key_env": None},
            "logging": {"level": "info", "output": {"kind": "stderr"}},
        }
        config_path = directory / "config.json"
        config_path.write_bytes(encoded(config))
        self.child = subprocess.Popen(
            [str(binary), "--data-root", str(directory / "data"),
             "--config", str(config_path), "--stdio"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env={**os.environ, "TASK058_MODEL_KEY": KEY}, start_new_session=True,
        )
        self.condition = threading.Condition()
        self.responses = {}
        self.events = []
        self.diagnostics = []
        self.errors = []
        self.next_id = 0
        self.owned_tool = None
        self.sample_stop = threading.Event()
        self.samples = []
        self.readers = [threading.Thread(target=self.read, args=(stream, name), daemon=True)
                        for stream, name in [(self.child.stdout, "stdout"),
                                             (self.child.stderr, "stderr")]]
        for reader in self.readers:
            reader.start()
        self.sampler = threading.Thread(target=self.sample, daemon=True)
        self.sampler.start()
        initialized = self.call("initialize", {})
        self.host_identity = {key: initialized[key] for key in ("host_id", "host_incarnation")}
        assert all(isinstance(value, str) and value for value in self.host_identity.values())

    def read(self, stream, name):
        try:
            for line in stream:
                at = now()  # Timestamp complete line arrival before JSON parsing.
                value = json.loads(line)
                self.trace.add(name, received_ns=at, value=value)
                if name == "stdout":
                    with self.condition:
                        if "id" in value:
                            self.responses[value["id"]] = (value, at)
                        else:
                            self.events.append((value, at))
                        self.condition.notify_all()
                else:
                    self.diagnostics.append((value, at))
        except Exception as error:
            with self.condition:
                self.errors.append(repr(error))
                self.condition.notify_all()

    def sample(self):
        while not self.sample_stop.is_set():
            try:
                sample = proc_sample(self.child.pid)
            except (FileNotFoundError, ProcessLookupError):
                break
            self.samples.append(sample)
            self.trace.add("process_sample", **sample)
            self.sample_stop.wait(0.02)

    def rpc(self, method, params):
        self.next_id += 1
        request_id = self.next_id
        frame = encoded({"jsonrpc": "2.0", "id": request_id,
                         "method": method, "params": params}) + b"\n"
        started = now()  # Serialization is excluded; pipe write and flush included.
        self.child.stdin.write(frame)
        self.child.stdin.flush()
        self.trace.add("rpc_send", id=request_id, method=method, started_ns=started,
                       params=params)
        with self.condition:
            if not self.condition.wait_for(
                    lambda: request_id in self.responses or self.errors,
                    timeout=TIMEOUT):
                raise TimeoutError(f"RPC {method}; child status {self.child.poll()}")
            if self.errors:
                raise RuntimeError(self.errors)
            value, ended = self.responses.pop(request_id)
        if "error" in value:
            raise RuntimeError(f"{method}: {value['error']}")
        return value["result"], started, ended

    def call(self, method, params):
        return self.rpc(method, params)[0]

    def event(self, predicate, after=0):
        deadline = time.monotonic() + TIMEOUT
        with self.condition:
            while True:
                for value, at in self.events:
                    if at >= after and predicate(value):
                        return value, at
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("expected output/completion notification")
                self.condition.wait(remaining)

    def phase(self, name, duration=0.4):
        start = proc_sample(self.child.pid)
        self.trace.add("phase_begin", name=name, sample=start)
        time.sleep(duration)
        end = proc_sample(self.child.pid)
        self.trace.add("phase_end", name=name, sample=end)
        return resource_delta(start, end)

    def finish(self):
        self.child.wait(timeout=TIMEOUT)
        finished = now()
        self.child.stdin.close()
        self.sample_stop.set()
        self.sampler.join()
        for reader in self.readers:
            reader.join(timeout=2)
        if self.child.returncode != 0 or self.errors:
            raise RuntimeError(f"Host exit {self.child.returncode}; {self.errors}")
        return finished

    def abort(self):
        self.sample_stop.set()
        if self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait()
        for reader in self.readers:
            reader.join(timeout=2)
        self.sampler.join(timeout=2)


class Plan:
    def __init__(self, token, chunks=None, command=None):
        self.token = token
        self.chunks = chunks or []
        self.command = command
        self.entered = threading.Event()
        self.release = threading.Event()
        self.done = threading.Event()
        self.received_ns = None
        self.emissions = []
        self.error = None


class ModelFixture:
    """Only model replies are scripted. Real HTTP, Runtime and tools execute."""
    def __init__(self, trace):
        self.trace = trace
        self.plans = {}
        self.lock = threading.Lock()
        self.requests = 0
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                assert self.path == "/v1/chat/completions"
                assert self.headers.get("Authorization") == "Bearer " + KEY
                text = json.dumps(request["messages"][-1])
                with fixture.lock:
                    plans = [plan for token, plan in fixture.plans.items() if token in text]
                    fixture.requests += 1
                if len(plans) != 1:
                    fixture.trace.add("fixture_error", message="unmatched model request", request=request)
                    self.send_error(500)
                    return
                plan = plans[0]
                try:
                    plan.received_ns = now()
                    fixture.trace.add("model_request", token=plan.token,
                                      received_ns=plan.received_ns, request=request)
                    plan.entered.set()
                    if not plan.release.wait(TIMEOUT):
                        raise TimeoutError("fixture release")
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Connection", "close")
                    self.end_headers()
                    if plan.command:
                        delta = {"tool_calls": [{"index": 0, "id": plan.token,
                                 "type": "function", "function": {"name": "bash",
                                 "arguments": json.dumps({"command": plan.command})}}]}
                        self.wfile.write(sse(delta))
                        self.wfile.write(sse({}, "tool_calls") + b"data: [DONE]\n\n")
                    else:
                        offset = 0
                        for index, chunk in enumerate(plan.chunks):
                            at = now()
                            self.wfile.write(sse({"content": chunk}))
                            self.wfile.flush()
                            end = offset + len(chunk.encode())
                            emission = {"index": index, "start": offset, "end": end,
                                        "before_write_ns": at, "after_write_ns": now()}
                            plan.emissions.append(emission)
                            fixture.trace.add("model_emission", token=plan.token, **emission)
                            offset = end
                            if index + 1 < len(plan.chunks):
                                time.sleep(0.01)  # Deliberate fixture pacing, not framework work.
                        self.wfile.write(sse({}, "stop") + b"data: [DONE]\n\n")
                    self.wfile.flush()
                except Exception as error:
                    plan.error = repr(error)
                    fixture.trace.add("fixture_error", token=plan.token, message=plan.error)
                finally:
                    plan.done.set()
                self.close_connection = True

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


def sse(delta, finish=None):
    return b"data: " + encoded({"choices": [{"index": 0, "delta": delta,
                                             "finish_reason": finish}]}) + b"\n\n"


def resource_delta(start, end):
    ticks = (end["user_ticks"] + end["system_ticks"]
             - start["user_ticks"] - start["system_ticks"])
    seconds = (end["at_ns"] - start["at_ns"]) / 1e9
    return {"start": start, "end": end, "wall_ms": seconds * 1000,
            "cpu_ms": ticks * 1000 / TICK_HZ,
            "cpu_one_core_percent": ticks / TICK_HZ / seconds * 100}


def request_config(workspace):
    return {
        "role": {"name": "Fixture", "description": "", "instructions": ""},
        "session_defaults": {"model": MODEL, "reasoning": "auto", "tools": [{"name": "bash"}],
            "workspace": {"path": str(workspace)},
            "execution": {"approval": "auto", "prompt_timeout_ms": 30000, "model_timeout_ms": 600000,
                "policy_timeout_ms": 30000, "tool_timeout_ms": 1800000,
                "model_retry_attempts": 1, "model_retry_base_delay_ms": 100,
                "compaction": {"enabled": False, "trigger_percent": 95, "target_percent": 50}}},
    }


def control(process, method, target, **fields):
    return process.rpc(method, {"target": target, "operation_id": ident(), **fields})


def assert_identity(snapshot, expected):
    """Compare actual public source IDs, never infer them from list position."""
    for key, value in expected.items():
        assert isinstance(value, str) and value, (key, value)
        assert snapshot[key] == value, (key, snapshot[key], value)


def query_data(snapshot, identity):
    assert_identity(snapshot, identity)
    data = snapshot["data"]
    assert isinstance(data, dict), data
    return data


def agent_items(page, host_identity):
    """Read the concrete discovery page, without a snapshot-envelope fallback."""
    assert isinstance(page, dict), page
    items = page["items"]
    assert isinstance(items, list), items
    assert page["next_cursor"] is None, "unpaged inventory must reach the current tail"
    for item in items:
        assert_identity(item, {"host_id": host_identity["host_id"]})
    ids = [item["id"] for item in items]
    assert all(isinstance(value, str) and value for value in ids), ids
    assert ids == sorted(set(ids)), "agent.list IDs must be unique and lexical"
    return items


def bound_message_turn(snapshot, identity, message_id, text):
    data = query_data(snapshot, {**identity, "message_id": message_id})
    assert data["text"].encode() == text.encode(), data
    assert data["mode"] == "follow_up" and data["source"] == {"kind": "user"}, data
    binding = data["binding"]
    assert binding["state"] == "bound", binding
    turn_id = binding["turn_id"]
    assert isinstance(turn_id, str) and turn_id, binding
    return turn_id


def finished_turn_data(snapshot, identity, outcome, persistence):
    """Read TurnResultSnapshot, not the removed native Runtime serialization."""
    data = query_data(snapshot, identity)
    info = data["info"]
    assert info["state"] == "finished" and info["outcome"] == outcome, info
    assert info["persistence"] == persistence, info
    assert not info["cleanup_faults"] and info["fault"] is None, info
    assert info["persistence_fault"] is None, info
    assert data["config"]["model"] == MODEL, data["config"]
    assert isinstance(data["history"], list), data
    # These fixtures finish their single model response before completion or
    # tool cancellation. An unfinished response must not count as a full body.
    assert data["partials"] == [], data["partials"]
    return data


def stream_body(data, text):
    history = data["history"]
    assert [item["kind"] for item in history] == ["user", "assistant"], history
    assert history[0]["text"].encode() == text.encode(), history[0]
    assert "response_index" not in history[0], history[0]
    assert history[1]["response_index"] == 0, history[1]
    response = history[1]["response"]
    assert response["finish_reason"] == "stop", response
    assert all(part["type"] == "text" for part in response["content"]), response
    return b"".join(part["data"].encode() for part in response["content"])


def send(process, target, text):
    message_id = ident()
    response = process.rpc("message.send", {"target": {"kind": "session", **target},
                           "message_id": message_id, "mode": "follow_up", "text": text})
    assert_identity(response[0], {**process.host_identity, **target, "message_id": message_id})
    return response


def stock(process, mode, workspace):
    targets = []
    for index in range(4):
        receipt = process.call("agent.create", {"operation_id": ident(),
            "name": f"task058-{mode}-{index}", "mode": mode, "config": request_config(workspace)})
        assert receipt["result"]["steps"]["agent"]["status"] == "succeeded", receipt
        assert receipt["result"]["steps"]["session"]["status"] == "succeeded", receipt
        targets.append({key: receipt[key] for key in ("agent_id", "session_id")})
        for _ in range(3):
            receipt = process.call("session.create", {"operation_id": ident(),
                "target": {"host_id": receipt["host_id"], "agent_id": receipt["agent_id"]}})
            assert receipt["result"]["steps"]["session"]["status"] == "succeeded", receipt
            targets.append({key: receipt[key] for key in ("agent_id", "session_id")})
    return targets


def inventory(process, targets):
    states = [query_data(process.call("session.state", {"target": target}),
                         {**process.host_identity, **target}) for target in targets]
    agents = agent_items(process.call("agent.list", {}), process.host_identity)
    assert {target["agent_id"] for target in targets} <= {agent["id"] for agent in agents}
    result = {
        "agents": len(agents), "sessions": len(states),
        "modes": dict(collections.Counter(state["mode"] for state in states)),
        "residencies": dict(collections.Counter(state["residency"] for state in states)),
        "active_turns": sum(state["active_turn_id"] is not None for state in states),
        "queued_inputs": sum(len(state["next_step"]) + len(state["next_turn"]) for state in states),
    }
    process.trace.add("inventory", **result)
    return result


def wait_until(predicate):
    deadline = time.monotonic() + TIMEOUT
    while not predicate():
        if time.monotonic() > deadline:
            raise TimeoutError("actual state did not converge")
        time.sleep(0.001)


def observe(process, target, categories):
    return process.call("events.subscribe", {"scope": {"kind": "session", **target},
                                            "categories": categories})["subscription_id"]


def stream_workload(process, fixture, targets, observers, label):
    subscriptions = {}
    plans = []
    sends = []
    resources_start = proc_sample(process.child.pid)
    for target in targets:
        subscriptions[target["session_id"]] = [
            observe(process, target, ["output", "completion"]) for _ in range(observers)]
        token = "stream-" + ident()
        chunks = [f"{index:02d}|" + "x" * 253 for index in range(8)]
        plans.append(fixture.add(Plan(token, chunks)))
        receipt, start, end = send(process, target, token)
        sends.append({"target": target, "receipt": receipt,
                      "started_ns": start, "accepted_ns": end, "accept_ms": (end-start)/1e6})
    for plan in plans:
        if not plan.entered.wait(TIMEOUT):
            raise TimeoutError("actual concurrent model request")
    concurrent = inventory(process, targets)
    assert concurrent["active_turns"] == len(targets), concurrent
    # All requested Turns are actually at the local provider before release.
    time.sleep(0.05)
    for plan in plans:
        plan.release.set()
    samples = []
    for target, plan, sent in zip(targets, plans, sends):
        if not plan.done.wait(TIMEOUT) or plan.error:
            raise RuntimeError(f"fixture {plan.token}: {plan.error}")
        # A fast Turn can already be finished; use the bound message's actual Turn.
        message = process.call("message.get", {"target": target,
                                              "message_id": sent["receipt"]["message_id"]})
        identity = {**process.host_identity, **target}
        turn_id = bound_message_turn(message, identity, sent["receipt"]["message_id"], plan.token)
        identity.update(turn_id=turn_id, load_id=sent["receipt"]["result"]["load_id"])
        result = process.call("turn.wait", {"target": target, "turn_id": turn_id})
        data = finished_turn_data(result, identity, "completed", "not_applicable")
        assert stream_body(data, plan.token) == "".join(plan.chunks).encode(), \
            "same actual body even without observers"
        observed = []
        for subscription in subscriptions[target["session_id"]]:
            process.event(lambda value:
                value.get("method") == "events.push"
                and value["params"]["subscription_id"] == subscription
                and value["params"]["event"]["kind"] == "model.output.delta"
                and value["params"]["event"]["turn_id"] == turn_id
                and value["params"]["event"]["data"]["end"] == 2048)
            with process.condition:
                events = [(value["params"]["event"], at)
                          for value, at in process.events
                          if value.get("method") == "events.push"
                          and value["params"]["subscription_id"] == subscription
                          and value["params"]["event"]["kind"] == "model.output.delta"
                          and value["params"]["event"]["turn_id"] == turn_id]
            for event, _ in events:
                assert_identity(event, identity)
                assert event["response_index"] == 0 and event["data"]["channel"] == "text", event
            deltas = [(event["data"], at) for event, at in events]
            body = b""
            for delta, at in deltas:
                assert delta["start"] == len(body), delta
                body += delta["text"].encode()
                assert delta["end"] == len(body), delta
            assert body == "".join(plan.chunks).encode()
            for emission in plan.emissions:
                at = min(at for delta, at in deltas if delta["end"] >= emission["end"])
                observed.append({"subscription_id": subscription, "chunk": emission["index"],
                    "observed_ns": at, "body_ms": (at-emission["before_write_ns"])/1e6,
                    "after_write_ms": (at-emission["after_write_ns"])/1e6})
        samples.append({**sent, "turn_id": turn_id, "model_received_ns": plan.received_ns,
            "fixture_wait_ms": (plan.emissions[0]["before_write_ns"]-plan.received_ns)/1e6,
            "first_write_from_send_ms": (plan.emissions[0]["before_write_ns"]-sent["started_ns"])/1e6,
            "first_observed_from_send_ms": min(
                ((item["observed_ns"]-sent["started_ns"])/1e6 for item in observed), default=None),
            "observations": observed, "emissions": plan.emissions})
    for group in subscriptions.values():
        for subscription in group:
            process.call("events.unsubscribe", {"subscription_id": subscription})
    return {"label": label, "parallel_turns": len(targets), "observers_per_session": observers,
            "concurrent": concurrent, "samples": samples,
            "resources": resource_delta(resources_start, proc_sample(process.child.pid))}


def cancel_tool(process, fixture, target, workspace):
    token = "cancel-" + ident()
    fifo = workspace / "never"
    os.mkfifo(fifo)
    command = ("printf '%s' \"$BASHPID\" > leader.pid; "
               "(printf '%s' \"$BASHPID\" > descendant.pid; IFS= read -r _ < never) & "
               "printf ready; IFS= read -r _ < never")
    plan = fixture.add(Plan(token, command=command))
    subscription = observe(process, target, ["output", "completion"])
    receipt, send_at, accept_at = send(process, target, token)
    assert plan.entered.wait(TIMEOUT)
    time.sleep(0.05)  # Named model gate, separately reported below.
    release_at = now()
    plan.release.set()
    output, ready_at = process.event(lambda value:
        value.get("method") == "events.push"
        and value["params"]["subscription_id"] == subscription
        and value["params"]["event"]["kind"] == "toolcall.output.delta"
        and value["params"]["event"]["tool_call_id"] == token)
    turn_id = output["params"]["event"]["turn_id"]
    identity = {**process.host_identity, **target, "turn_id": turn_id,
                "load_id": receipt["result"]["load_id"]}
    assert_identity(output["params"]["event"], identity)
    wait_until(lambda: (workspace / "descendant.pid").exists())
    pids = [int((workspace / name).read_text()) for name in ("leader.pid", "descendant.pid")]
    assert all(Path(f"/proc/{pid}").exists() for pid in pids)
    process.owned_tool = (pids[0], proc_sample(pids[0])["start_ticks"])
    hold_start = proc_sample(process.child.pid)
    time.sleep(0.05)  # Deliberate live tool/FIFO wait, excluded from cancel latency.
    before = proc_sample(process.child.pid)
    receipt, started, accepted = control(process, "turn.cancel", target,
                                        turn_id=turn_id, keep_inbox=False)
    result, _, waited = process.rpc("turn.wait", {"target": target, "turn_id": turn_id})
    data = finished_turn_data(result, identity, "cancelled", "persisted")
    completion, finished = process.event(lambda value:
        value.get("method") == "events.push"
        and value["params"]["subscription_id"] == subscription
        and value["params"]["event"]["kind"] == "turn.finished"
        and value["params"]["event"]["turn_id"] == turn_id)
    assert_identity(completion["params"]["event"], identity)
    assert completion["params"]["event"]["data"] == data["info"], completion
    # This timestamp is an upper bound sampled after owner completion, not death time.
    assert all(not Path(f"/proc/{pid}").exists() for pid in pids), pids
    try:
        os.killpg(pids[0], 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError("owned bash process group remains")
    gone_at = now()
    process.owned_tool = None
    tool = process.call("toolcall.get", {"target": target,
                       "tool": {"turn_id": turn_id, "tool_call_id": token}})
    native = query_data(tool, {**identity, "tool_call_id": token})["result"]
    assert native["outcome"] == "cancelled", native
    retained_tools = [item for item in data["history"] if item["kind"] == "tool_result"]
    assert len(retained_tools) == 1, retained_tools
    retained = retained_tools[0]
    assert retained["tool_call_id"] == token and retained["name"] == "bash", retained
    assert retained["response_index"] == 0 and retained["result"] == native, retained
    process.trace.add("actual_cleanup_witness", pids=pids, verified_ns=gone_at, native_result=native)
    process.call("events.unsubscribe", {"subscription_id": subscription})
    return {"message_accept_ms": (accept_at-send_at)/1e6,
            "fixture_wait_ms": (release_at-plan.received_ns)/1e6,
            "tool_ready_from_release_ms": (ready_at-release_at)/1e6,
            "tool_ready_to_cancel_ms": (started-ready_at)/1e6,
            "tool_wait_resources": resource_delta(hold_start, before),
            "receipt": receipt, "control_accept_ms": (accepted-started)/1e6,
            "turn_finished_observed_ms": (finished-started)/1e6,
            "turn_wait_returned_ms": (waited-started)/1e6,
            "pid_group_gone_verified_ms": (gone_at-started)/1e6,
            "tool_result": native, "resources": resource_delta(before, proc_sample(process.child.pid))}


def one_run(binary, out, index):
    trace = Trace(out / f"run-{index:02d}.jsonl")
    fixture = ModelFixture(trace)
    process = None
    with tempfile.TemporaryDirectory(prefix="task058-") as temporary:
        directory = Path(temporary)
        workspace = directory / "workspace"
        workspace.mkdir()
        try:
            process = Process(binary, directory, fixture.endpoint, trace)
            result = {"run": index, "load_begin": list(os.getloadavg()), "stock": [],
                      "system_cpu_begin": Path("/proc/stat").read_text().splitlines()[0]}
            result["stock"].append({"label": "empty_host", "inventory": inventory(process, []),
                                    "resources": process.phase("empty_host")})
            persistent = stock(process, "persistent", workspace)
            for target in persistent:
                control(process, "session.unload", target)
            wait_until(lambda: all(process.call("session.state", {"target": t})["data"]["residency"]
                                   == "unloaded" for t in persistent))
            result["stock"].append({"label": "persistent_unloaded", "inventory": inventory(process, persistent),
                                    "resources": process.phase("persistent_unloaded")})
            for target in persistent:
                control(process, "session.load", target)
            wait_until(lambda: all(process.call("session.state", {"target": t})["data"]["residency"]
                                   == "loaded" for t in persistent))
            result["stock"].append({"label": "persistent_loaded", "inventory": inventory(process, persistent),
                                    "resources": process.phase("persistent_loaded")})
            memory = stock(process, "memory", workspace)
            all_targets = persistent + memory
            result["stock"].append({"label": "plus_memory_unloaded", "inventory": inventory(process, all_targets),
                                    "resources": process.phase("plus_memory_unloaded")})
            for target in memory:
                control(process, "session.load", target)
            wait_until(lambda: all(process.call("session.state", {"target": t})["data"]["residency"]
                                   == "loaded" for t in memory))
            result["stock"].append({"label": "plus_memory_loaded", "inventory": inventory(process, all_targets),
                                    "resources": process.phase("plus_memory_loaded")})
            queued = []
            requests_before = fixture.requests
            for target in memory[:4]:
                control(process, "session.pause", target)
                for _ in range(8):
                    receipt, start, end = send(process, target, "q"*256)
                    queued.append({"receipt": receipt, "started_ns": start, "accepted_ns": end,
                                   "accept_ms": (end-start)/1e6})
            result["paused_queue"] = {"samples": queued, "inventory": inventory(process, all_targets),
                                       "resources": process.phase("paused_queue")}
            assert fixture.requests == requests_before, "paused input must not request a model"
            assert result["paused_queue"]["inventory"]["queued_inputs"] == 32
            for target in memory[:4]:
                control(process, "session.cancel", target, keep_inbox=False)
                control(process, "session.resume", target)
            # Alternate repeat order to expose order/history effects, not hide them.
            workloads = [(1, 1), (4, 0), (4, 1), (4, 2)]
            if index % 2 == 0:
                workloads.reverse()
            groups = {(1, 1): memory[:1], (4, 0): memory[4:8],
                      (4, 1): memory[8:12], (4, 2): memory[12:16]}
            result["stream"] = [stream_workload(process, fixture, groups[(parallel, observers)], observers,
                                f"turns{parallel}_observers{observers}")
                                for parallel, observers in workloads]
            result["cancel_tool"] = cancel_tool(process, fixture, persistent[0], workspace)
            receipt, start, accepted = process.rpc("host.shutdown", {"operation_id": ident(),
                                                                    "strategy": "graceful"})
            exited = process.finish()
            stopped = [at for diagnostic, at in process.diagnostics if diagnostic.get("event") == "host.stopped"]
            assert len(stopped) == 1, stopped
            result["shutdown"] = {"receipt": receipt, "accept_ms": (accepted-start)/1e6,
                                   "stopped_diagnostic_observed_ms": (stopped[0]-start)/1e6,
                                   "process_exit_observed_ms": (exited-start)/1e6}
            result["actual_model_requests"] = fixture.requests
            assert fixture.requests == 14, fixture.requests
            result["load_end"] = list(os.getloadavg())
            result["system_cpu_end"] = Path("/proc/stat").read_text().splitlines()[0]
            result["host_lifetime_sampled_resources"] = resource_delta(process.samples[0], process.samples[-1])
            result["periodic_rss_samples"] = len(process.samples)
            result.update(aggregate_rss(process.samples + [
                sample for _, sample in resource_snapshots(result)]))
            return result
        finally:
            if process:
                process.abort()
            # Failure cleanup only; do not signal a stale PID after proven cleanup.
            if process and process.owned_tool:
                pid, started = process.owned_tool
                try:
                    if proc_sample(pid)["start_ticks"] == started:
                        os.killpg(pid, signal.SIGKILL)
                except (FileNotFoundError, ProcessLookupError):
                    pass
            fixture.close()
            trace.close()


def environment(binary, source_revision):
    root = Path(__file__).resolve().parent.parent
    sha = run_command("git", "-C", str(root), "rev-parse", source_revision)
    diff = run_command("git", "-C", str(root), "diff", sha, "--", "crates", "Cargo.toml", "Cargo.lock")
    if diff:
        raise RuntimeError("production source differs from --source-revision; rebuild and give exact revision")
    info = {"source_revision": sha, "harness_revision": run_command("git", "-C", str(root), "rev-parse", "HEAD"),
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "python": sys.version, "platform": platform.platform(), "machine": platform.machine(),
        "logical_cpus": os.cpu_count(), "affinity_cpus": len(os.sched_getaffinity(0)),
        "clock_ticks_per_second": TICK_HZ, "page_bytes": PAGE_BYTES,
        "monotonic_clock": str(time.get_clock_info("monotonic")),
        "cargo_lock_sha256": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
        "cpu_model": next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                           if line.startswith("model name")), None),
        "meminfo": Path("/proc/meminfo").read_text(), "loadavg": Path("/proc/loadavg").read_text(),
        "process_snapshot": run_command("ps", "-eo", "pid,ppid,stat,comm,pcpu,pmem"),
        "cgroup": {name: Path("/sys/fs/cgroup", name).read_text().strip()
                   for name in ["cpu.max", "memory.max", "memory.current", "cpu.stat"]
                   if Path("/sys/fs/cgroup", name).exists()},
        "note": "No machine isolation promised. CPU is Host process only, not fixture/client/tool descendants."
    }
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="already-built actual Host executable")
    parser.add_argument("--source-revision", required=True, help="exact production source commit used for binary")
    parser.add_argument("--out", type=Path, required=True, help="new directory for raw JSONL and results.json")
    parser.add_argument("--runs", type=int, default=3, help="small independent fresh-Host repetitions (default 3)")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be positive")
    args.out.mkdir(parents=True, exist_ok=False)
    result = {"environment": environment(args.binary.resolve(), args.source_revision), "runs": []}
    try:
        for index in range(1, args.runs + 1):
            print(f"running {index}/{args.runs}", flush=True)
            result["runs"].append(one_run(args.binary.resolve(), args.out, index))
    except BaseException as error:
        result["failure"] = repr(error)
        raise
    finally:
        (args.out / "results.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"Raw evidence: {args.out}")


if __name__ == "__main__":
    main()
