"""Native Win32 Job accounting versus retained process-handle exit witnesses.

Investigation only. Uses the standard Python installed on the Windows CI image.
The subprocesses communicate solely over a gated loopback TCP connection.
"""

import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import subprocess
import sys
import threading
import time


def child(role, port, memory_mb):
    # Deliberately leave the leaf for the actual Job owner; its stdio is closed.
    descendant = None
    if role == "root":
        descendant = subprocess.Popen(
            [sys.executable, __file__, "--child", "leaf", "--port", str(port),
             "--memory-mb", str(memory_mb)],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    memory = bytearray(memory_mb * 1024 * 1024) if role == "leaf" else bytearray()
    for offset in range(0, len(memory), 4096):
        memory[offset] = 1
    with socket.create_connection(("127.0.0.1", port), timeout=20) as peer:
        peer.sendall(f"{role} {os.getpid()}\n".encode("ascii"))
        if peer.recv(1) != b"x":
            raise RuntimeError("probe gate closed without release")
    # Keep the actual allocation and descendant handle alive through the gate.
    assert not memory or memory[0] == 1
    _ = descendant
    if role == "leaf":
        # Keep the allocation present for the native natural-exit comparison.
        os._exit(0)


class BasicLimits(c.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", c.c_int64), ("PerJobUserTimeLimit", c.c_int64),
        ("LimitFlags", w.DWORD), ("MinimumWorkingSetSize", c.c_size_t),
        ("MaximumWorkingSetSize", c.c_size_t), ("ActiveProcessLimit", w.DWORD),
        ("Affinity", c.c_size_t), ("PriorityClass", w.DWORD), ("SchedulingClass", w.DWORD),
    ]


class IoCounters(c.Structure):
    _fields_ = [(name, c.c_uint64) for name in (
        "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
        "ReadTransferCount", "WriteTransferCount", "OtherTransferCount",
    )]


class ExtendedLimits(c.Structure):
    _fields_ = [
        ("BasicLimitInformation", BasicLimits), ("IoInfo", IoCounters),
        ("ProcessMemoryLimit", c.c_size_t), ("JobMemoryLimit", c.c_size_t),
        ("PeakProcessMemoryUsed", c.c_size_t), ("PeakJobMemoryUsed", c.c_size_t),
    ]


class CompletionAssociation(c.Structure):
    _fields_ = [("CompletionKey", c.c_void_p), ("CompletionPort", w.HANDLE)]


class Accounting(c.Structure):
    _fields_ = [
        ("TotalUserTime", c.c_int64), ("TotalKernelTime", c.c_int64),
        ("ThisPeriodTotalUserTime", c.c_int64), ("ThisPeriodTotalKernelTime", c.c_int64),
        ("TotalPageFaultCount", w.DWORD), ("TotalProcesses", w.DWORD),
        ("ActiveProcesses", w.DWORD), ("TotalTerminatedProcesses", w.DWORD),
    ]


class Startup(c.Structure):
    _fields_ = [
        ("cb", w.DWORD), ("lpReserved", w.LPWSTR), ("lpDesktop", w.LPWSTR),
        ("lpTitle", w.LPWSTR), ("dwX", w.DWORD), ("dwY", w.DWORD),
        ("dwXSize", w.DWORD), ("dwYSize", w.DWORD), ("dwXCountChars", w.DWORD),
        ("dwYCountChars", w.DWORD), ("dwFillAttribute", w.DWORD), ("dwFlags", w.DWORD),
        ("wShowWindow", w.WORD), ("cbReserved2", w.WORD), ("lpReserved2", c.c_void_p),
        ("hStdInput", w.HANDLE), ("hStdOutput", w.HANDLE), ("hStdError", w.HANDLE),
    ]


class ProcessInfo(c.Structure):
    _fields_ = [("hProcess", w.HANDLE), ("hThread", w.HANDLE),
                ("dwProcessId", w.DWORD), ("dwThreadId", w.DWORD)]


class Win32:
    def __init__(self):
        self.dll = c.WinDLL("kernel32", use_last_error=True)
        signatures = {
            "CreateJobObjectW": ([c.c_void_p, w.LPCWSTR], w.HANDLE),
            "SetInformationJobObject": ([w.HANDLE, c.c_int, c.c_void_p, w.DWORD], w.BOOL),
            "QueryInformationJobObject": ([w.HANDLE, c.c_int, c.c_void_p, w.DWORD,
                                            c.POINTER(w.DWORD)], w.BOOL),
            "AssignProcessToJobObject": ([w.HANDLE, w.HANDLE], w.BOOL),
            "TerminateJobObject": ([w.HANDLE, w.UINT], w.BOOL),
            "TerminateProcess": ([w.HANDLE, w.UINT], w.BOOL),
            "CreateProcessW": ([w.LPCWSTR, w.LPWSTR, c.c_void_p, c.c_void_p, w.BOOL,
                                w.DWORD, c.c_void_p, w.LPCWSTR, c.POINTER(Startup),
                                c.POINTER(ProcessInfo)], w.BOOL),
            "ResumeThread": ([w.HANDLE], w.DWORD),
            "WaitForSingleObject": ([w.HANDLE, w.DWORD], w.DWORD),
            "OpenProcess": ([w.DWORD, w.BOOL, w.DWORD], w.HANDLE),
            "IsProcessInJob": ([w.HANDLE, w.HANDLE, c.POINTER(w.BOOL)], w.BOOL),
            "CreateIoCompletionPort": ([w.HANDLE, w.HANDLE, c.c_size_t, w.DWORD], w.HANDLE),
            "GetQueuedCompletionStatus": ([w.HANDLE, c.POINTER(w.DWORD),
                                           c.POINTER(c.c_size_t), c.POINTER(c.c_void_p),
                                           w.DWORD], w.BOOL),
            "CloseHandle": ([w.HANDLE], w.BOOL),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.dll, name)
            function.argtypes, function.restype = arguments, result
            setattr(self, name, function)

    @staticmethod
    def checked(value, operation):
        if not value:
            raise c.WinError(c.get_last_error(), operation)
        return value

    def wait(self, handle, milliseconds=0):
        result = self.WaitForSingleObject(handle, milliseconds)
        if result not in (0, 258):
            raise c.WinError(c.get_last_error(), f"WaitForSingleObject={result}")
        return result

    def in_job(self, process, job):
        result = w.BOOL()
        self.checked(self.IsProcessInJob(process, job, c.byref(result)), "IsProcessInJob")
        return bool(result.value)

    def pids(self, job):
        capacity = 16
        while True:
            buffer = c.create_string_buffer(8 + c.sizeof(c.c_size_t) * capacity)
            returned = w.DWORD()
            if self.QueryInformationJobObject(job, 3, buffer, len(buffer), c.byref(returned)):
                assigned, count = (w.DWORD * 2).from_buffer(buffer)
                if count > capacity or count != assigned:
                    raise RuntimeError(f"incomplete job PID list: {assigned}/{count}/{capacity}")
                return list((c.c_size_t * count).from_buffer(buffer, 8))
            error = c.get_last_error()
            if error != 234:  # ERROR_MORE_DATA: resize without imposing a process cap.
                raise c.WinError(error, "QueryInformationJobObject(ProcessIdList)")
            capacity = max(capacity * 2, (w.DWORD * 2).from_buffer(buffer)[0])

    def snapshot(self, job, held):
        # These reads are explicitly ordered, not claimed to form an atomic snapshot.
        before = {role: self.wait(handle) for role, (_, handle) in held.items()}
        info = Accounting()
        self.checked(self.QueryInformationJobObject(job, 1, c.byref(info), c.sizeof(info), None),
                     "QueryInformationJobObject(Accounting)")
        pids = self.pids(job)
        membership = {role: self.in_job(handle, job) for role, (_, handle) in held.items()}
        after = {role: self.wait(handle) for role, (_, handle) in held.items()}
        return {"wait_before": before, "active": info.ActiveProcesses,
                "total": info.TotalProcesses, "pids": pids,
                "membership": membership, "wait_after": after}


class CompletionLog:
    """Observe notifications; no notification is accepted as exit evidence."""

    def __init__(self, api, job, held):
        self.api, self.job, self.held = api, job, held
        self.events = []
        self.error = None
        self.started_ns = time.perf_counter_ns()
        self.stop = threading.Event()
        self.ready = threading.Event()
        self.thread = None
        self.port = api.checked(api.CreateIoCompletionPort(w.HANDLE(-1), None, 0, 1),
                                "CreateIoCompletionPort")
        try:
            association = CompletionAssociation(job, self.port)
            api.checked(api.SetInformationJobObject(job, 7, c.byref(association),
                                                    c.sizeof(association)),
                        "SetInformationJobObject(AssociateCompletionPort)")
            self.thread = threading.Thread(target=self._collect, daemon=False)
            self.thread.start()
            if not self.ready.wait(5):
                raise RuntimeError("completion collector did not start")
        except BaseException:
            self.close()
            raise

    def _collect(self):
        self.ready.set()
        try:
            while not self.stop.is_set():
                code, key, pid = w.DWORD(), c.c_size_t(), c.c_void_p()
                if not self.api.GetQueuedCompletionStatus(
                    self.port, c.byref(code), c.byref(key), c.byref(pid), 100
                ):
                    error = c.get_last_error()
                    if error == 258 and pid.value is None:
                        continue
                    raise c.WinError(error, "GetQueuedCompletionStatus")
                if key.value != self.job:
                    raise RuntimeError("notification belongs to a different Job")
                # The dictionary copy is made while holding Python's GIL; no
                # native API call uses a handle after the collector is joined.
                held = self.held.copy()
                target = next(((role, handle) for role, (number, handle) in held.items()
                               if number == pid.value), None)
                if code.value == 4 and "leaf" in held:
                    # ACTIVE_PROCESS_ZERO has no PID: observe the retained leaf
                    # immediately, while keeping the packet PID itself null.
                    target = ("leaf", held["leaf"][1])
                # Check the exact reported process first, before any accounting
                # or PID-list queries can consume its termination window.
                target_wait = self.api.wait(target[1], 0) if target else None
                self.events.append({
                    "kind": {4: "ACTIVE_PROCESS_ZERO", 6: "NEW_PROCESS",
                             7: "EXIT_PROCESS", 8: "ABNORMAL_EXIT_PROCESS"}.get(code.value, "OTHER"),
                    "code": code.value, "pid": pid.value, "key_matches": True,
                    "target": target[0] if target else None, "target_wait": target_wait,
                    "at_us": (time.perf_counter_ns() - self.started_ns) / 1000,
                    "snapshot": self.api.snapshot(self.job, held),
                })
        except BaseException as error:
            self.error = repr(error)

    def finish(self, leaf_pid):
        # This bounded tail collects possibly delayed packets after actual
        # process exit. Missing notifications are reported, never fabricated.
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline:
            events = self.events[:]
            if self.error or (
                any(event["code"] in (7, 8) and event["pid"] == leaf_pid for event in events)
                and any(event["code"] == 4 for event in events)
            ):
                break
            time.sleep(0.005)
        self.close()
        if self.error:
            raise RuntimeError(f"completion collector failed: {self.error}")
        return self.events[:]

    def close(self):
        self.stop.set()
        if self.thread is not None:
            self.thread.join(2)
            if self.thread.is_alive():
                raise RuntimeError("completion collector did not terminate")
        if self.port is not None:
            self.api.checked(self.api.CloseHandle(self.port), "CloseHandle(completion port)")
            self.port = None


def bash_path():
    candidates = [shutil.which("bash.exe")]
    for name in ("ProgramFiles", "ProgramFiles(x86)", "LocalAppData"):
        if root := os.environ.get(name):
            candidates += [str(Path(root) / "Git/bin/bash.exe"),
                           str(Path(root) / "Programs/Git/bin/bash.exe")]
    for candidate in candidates:
        if not candidate:
            continue
        path = Path(candidate)
        if path.is_file() and any((directory / "msys-2.0.dll").is_file()
                                  for directory in (path.parent, path.parent / "../usr/bin")):
            return str(path.resolve())
    raise RuntimeError("native Git Bash was not found")


def bash_quote(path):
    return "'" + str(path).replace("\\", "/").replace("'", "'\\''") + "'"


def one_round(api, bash, mode, memory_mb):
    job = api.checked(api.CreateJobObjectW(None, None), "CreateJobObjectW")
    process = ProcessInfo()
    held = {}
    peers = {}
    completions = None
    try:
        limits = ExtendedLimits()
        limits.BasicLimitInformation.LimitFlags = 0x2000  # KILL_ON_JOB_CLOSE, no breakaway.
        api.checked(api.SetInformationJobObject(job, 9, c.byref(limits), c.sizeof(limits)),
                    "SetInformationJobObject(ExtendedLimitInformation)")
        # Associate and start the collector while the Job is still inactive.
        completions = CompletionLog(api, job, held)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(2)
            listener.settimeout(20)
            script = (f"{bash_quote(sys.executable)} {bash_quote(Path(__file__).resolve())} "
                      f"--child root --port {listener.getsockname()[1]} --memory-mb {memory_mb} "
                      ">/dev/null 2>&1; exit 0")
            command = c.create_unicode_buffer(subprocess.list2cmdline(
                [bash, "--noprofile", "--norc", "-c", script]))
            startup = Startup()
            startup.cb = c.sizeof(startup)
            api.checked(api.CreateProcessW(bash, command, None, None, False, 4, None,
                                          str(Path(__file__).resolve().parent),
                                          c.byref(startup), c.byref(process)), "CreateProcessW")
            api.checked(api.AssignProcessToJobObject(job, process.hProcess),
                        "AssignProcessToJobObject")
            if api.ResumeThread(process.hThread) != 1:
                raise RuntimeError("the suspended Bash primary thread was not resumed exactly once")
            for _ in range(2):
                peer, _ = listener.accept()
                peer.settimeout(20)
                identity = bytearray()
                while not identity.endswith(b"\n"):
                    value = peer.recv(1)
                    if not value:
                        raise RuntimeError("probe disconnected before identifying itself")
                    identity += value
                role, pid = identity.decode("ascii").split()
                if role not in ("root", "leaf") or role in peers:
                    raise RuntimeError(f"unexpected probe identity: {identity!r}")
                handle = api.checked(api.OpenProcess(0x100001 | 0x1000, False, int(pid)),
                                     "OpenProcess(SYNCHRONIZE|TERMINATE|QUERY_LIMITED_INFORMATION)")
                held[role] = (int(pid), handle)
                peers[role] = peer
            before = api.snapshot(job, held)
            assert all(before["membership"].values()), before
            assert all(state == 258 for state in before["wait_after"].values()), before
            assert all(pid in before["pids"] for pid, _ in held.values()), before
            peers["root"].sendall(b"x")
            assert api.wait(process.hProcess, 10000) == 0, "Bash did not naturally exit"
            assert api.wait(held["root"][1], 0) == 0, "root is not actually terminated"
            pre_stop = api.snapshot(job, held)
            assert pre_stop["membership"]["leaf"] and pre_stop["wait_after"]["leaf"] == 258, pre_stop
            start = time.perf_counter_ns()
            if mode == "job":
                api.checked(api.TerminateJobObject(job, 1), "TerminateJobObject")
            elif mode in ("process", "process-abnormal"):
                # An abnormal status code tests the notification classification;
                # this is not an actual memory access violation in the child.
                code = 0xC0000005 if mode == "process-abnormal" else 1
                api.checked(api.TerminateProcess(held["leaf"][1], code), "TerminateProcess")
            else:
                peers["leaf"].sendall(b"x")
            stop_return_us = (time.perf_counter_ns() - start) / 1000
            after = api.snapshot(job, held)
            first_empty_unsignaled = None
            observation = after
            while observation["wait_after"]["leaf"] != 0:
                if observation["active"] == 0 and observation["wait_after"]["leaf"] == 258:
                    first_empty_unsignaled = first_empty_unsignaled or observation
                if time.perf_counter_ns() - start > 10_000_000_000:
                    raise RuntimeError(f"leaf did not really exit: {observation}")
                time.sleep(0.001)
                observation = api.snapshot(job, held)
            signaled_us = (time.perf_counter_ns() - start) / 1000
            events = completions.finish(held["leaf"][0])
            return {"mode": mode, "memory_mb": memory_mb, "root_pid": held["root"][0],
                    "leaf_pid": held["leaf"][0], "before": before, "pre_stop": pre_stop,
                    "stop_return_us": stop_return_us, "post_stop": after,
                    "first_empty_unsignaled": first_empty_unsignaled,
                    "signaled_us": signaled_us, "final": observation,
                    "notifications": events,
                    "leaf_exit_notification_seen": any(
                        event["code"] in (7, 8) and event["pid"] == held["leaf"][0]
                        for event in events),
                    "zero_notification_seen": any(event["code"] == 4 for event in events)}
    finally:
        # The probe never leaves a workload behind, including on failed assertions.
        api.checked(api.TerminateJobObject(job, 1), "cleanup TerminateJobObject")
        handles = [handle for _, handle in held.values()]
        if process.hProcess:
            # Also cover a CreateProcess success followed by failed Job attachment.
            if api.wait(process.hProcess, 0) == 258:
                if not api.TerminateProcess(process.hProcess, 1) and api.wait(process.hProcess, 0) != 0:
                    raise c.WinError(c.get_last_error(), "cleanup direct process termination")
            handles.append(process.hProcess)
        try:
            for handle in handles:
                if api.wait(handle, 10000) != 0:
                    raise RuntimeError("probe cleanup did not confirm a real process exit")
        finally:
            # The reader may still inspect held handles on an earlier failure.
            # Join it before closing those handles or the exact Job.
            if completions is not None:
                completions.close()
            for peer in peers.values():
                peer.close()
            for handle in handles + [process.hThread, job]:
                if handle:
                    api.checked(api.CloseHandle(handle), "CloseHandle")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--child", choices=("root", "leaf"))
    parser.add_argument("--port", type=int)
    parser.add_argument("--memory-mb", type=int, default=0)
    parser.add_argument("--rounds", type=int, default=30)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("this investigation requires native Windows")
    if args.child:
        child(args.child, args.port, args.memory_mb)
        return
    api, bash = Win32(), bash_path()
    print(json.dumps({"platform": platform.platform(), "python": sys.version,
                      "bash": bash, "structures": {"startup": c.sizeof(Startup),
                      "limits": c.sizeof(ExtendedLimits), "accounting": c.sizeof(Accounting)}}), flush=True)
    counts = {}
    for mode in ("job", "process", "natural", "process-abnormal"):
        for memory_mb in (0, 64):
            found = 0
            for index in range(args.rounds):
                result = one_round(api, bash, mode, memory_mb)
                found += result["first_empty_unsignaled"] is not None
                result["round"] = index
                print(json.dumps(result), flush=True)
            counts[f"{mode}/{memory_mb}MiB"] = found
    print(json.dumps({"empty_before_signal_observations": counts,
                      "rounds_per_configuration": args.rounds}), flush=True)


if __name__ == "__main__":
    main()
