#!/usr/bin/env python3
"""Bounded process tiering for reviewed WK1 programs, not a hostile-code sandbox.

The same pinned Rust executable runs both workers. The portable process remains
available while a second process expands/checks/prepares native code. Only future
calls switch after exact source/dictionary/artifact/configuration agreement.
No executable page, pointer, VM workspace, or compiler thread crosses processes.
"""
from __future__ import annotations

from dataclasses import asdict, dataclass
import copy
import ctypes
import fcntl
import hashlib
import json
import math
import os
import platform
from pathlib import Path
import resource
import selectors
import signal
import struct
import subprocess
import sys
import threading
import time

MAX_FRAME = 512 * 1024
MAX_SOURCE = 256 * 1024
MAX_DIAGNOSTIC = 64 * 1024
# Linux UAPI values; some Python builds omit these names despite kernel support.
# This module refuses other architectures/platforms before using the constants.
F_ADD_SEALS = getattr(fcntl, "F_ADD_SEALS", 1033)
F_GET_SEALS = getattr(fcntl, "F_GET_SEALS", 1034)
REQUIRED_SEALS = 0x0001 | 0x0002 | 0x0004 | 0x0008  # SEAL, SHRINK, GROW, WRITE


class TierError(RuntimeError):
    pass


def _object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise TierError("duplicate protocol field")
        result[key] = value
    return result


def _decode(data):
    return json.loads(data, object_pairs_hook=_object,
                      parse_constant=lambda _v: (_ for _ in ()).throw(TierError("nonfinite protocol value")))


def _encode(value):
    """Bound a host graph before JSON serialization; count aliases every visit."""
    pending = [(value, 0)]
    nodes = byte_budget = 0
    while pending:
        item, depth = pending.pop()
        nodes += 1
        if nodes > 32_768 or depth > 48:
            raise TierError("request node/depth bound")
        if type(item) is str:
            # UTF-8/JSON escaping is at most twelve bytes per code point.
            byte_budget += 12 * len(item) + 2
        elif item is None or type(item) is bool:
            byte_budget += 5
        elif type(item) is int:
            if item.bit_length() > 64:
                raise TierError("request integer bound")
            byte_budget += 22
        elif type(item) is dict:
            if len(item) > 32_768 or any(type(key) is not str for key in item):
                raise TierError("request object bound")
            pending.extend((key, depth + 1) for key in item)
            pending.extend((child, depth + 1) for child in item.values())
            byte_budget += 2 + 2 * len(item)
        elif type(item) is list:
            if len(item) > 32_768:
                raise TierError("request array bound")
            pending.extend((child, depth + 1) for child in item)
            byte_budget += 2 + len(item)
        else:
            raise TierError("unsupported request value")
        # The initial WK1 source is independently bounded at 256 KiB; this
        # conservative preflight caps allocation before precise encoding.
        if byte_budget > 16 * MAX_FRAME or len(pending) > 65_536:
            raise TierError("request allocation bound")
    try:
        data = json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode()
    except (ValueError, TypeError, RecursionError, UnicodeError) as error:
        raise TierError("request cannot be encoded") from error
    if not 0 < len(data) <= MAX_FRAME:
        raise TierError("request frame bound")
    return struct.pack(">I", len(data)) + data


@dataclass(frozen=True, slots=True)
class Bounds:
    compile_seconds: float = 10.0
    call_seconds: float = 2.0
    session_seconds: float = 120.0
    address_bytes: int = 1024 ** 3
    cpu_seconds: int = 20
    max_calls: int = 65_536

    def validate(self):
        for name in ("compile_seconds", "call_seconds", "session_seconds"):
            value = getattr(self, name)
            if not _duration(value) or value == 0:
                raise TierError("invalid wall bound")
        if type(self.address_bytes) is not int or not 128 * 1024 ** 2 <= self.address_bytes <= 4 * 1024 ** 3:
            raise TierError("invalid worker memory bound")
        if type(self.cpu_seconds) is not int or not 1 <= self.cpu_seconds <= 3600:
            raise TierError("invalid CPU bound")
        if type(self.max_calls) is not int or not 1 <= self.max_calls <= 65_536:
            raise TierError("invalid call bound")


@dataclass(frozen=True, slots=True)
class Ticket:
    generation: int
    source_sha256: str
    dictionary_sha256: str
    binary_sha256: str
    configuration_sha256: str


def _hash(data):
    return hashlib.sha256(data).hexdigest()


class _Worker:
    def __init__(self, binary_fd, init, bounds):
        packet = _encode(init)
        self.binding = {name: init[name] for name in ("mode", "generation", "source_sha256", "dictionary_sha256", "export")}
        self.bounds = bounds
        self.started = time.monotonic()
        self.deadline = self.started + bounds.compile_seconds
        self.session_deadline = self.started + bounds.session_seconds
        self.reason = None
        self.closed = False
        self.ready = None
        self.sequence = 0
        self.buffer = bytearray()
        self.diagnostics = bytearray()
        self.stop = threading.Event()
        self.mutex = threading.RLock()
        self.selector = selectors.DefaultSelector()
        self.monitor = None
        # exec launcher avoids preexec_fn in a multithreaded Python host.
        launcher = [sys.executable, str(Path(__file__).resolve()), "--limited-exec",
                    str(bounds.address_bytes), str(bounds.cpu_seconds), str(binary_fd)]
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8",
               "FLUXR_TELEMETRY": "off"}
        try:
            self.process = subprocess.Popen(launcher, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                            stderr=subprocess.PIPE, start_new_session=True, env=env,
                                            pass_fds=(binary_fd,))
        except BaseException:
            self.selector.close()
            raise
        try:
            for stream, role in ((self.process.stdout, "stdout"), (self.process.stderr, "stderr")):
                os.set_blocking(stream.fileno(), False)
                self.selector.register(stream, selectors.EVENT_READ, role)
            os.set_blocking(self.process.stdin.fileno(), False)
            self.monitor = threading.Thread(target=self._watch, name="wk1-process-watchdog", daemon=True)
            self.monitor.start()  # running before source admission or compiler work
            self._send_packet(packet)
        except BaseException:
            self.close()
            raise

    def _watch(self):
        while not self.stop.wait(.01):
            with self.mutex:
                deadline = min(self.deadline or self.session_deadline, self.session_deadline)
            if time.monotonic() >= deadline:
                self._kill("wall deadline")
                return

    def _kill(self, reason):
        with self.mutex:
            if self.reason is None:
                self.reason = reason
            # No poll/reap before signaling the still-owned process group.
            if self.process.returncode is None:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            try:
                self.process.wait(timeout=1)
            except subprocess.TimeoutExpired as error:
                raise TierError("worker could not be reaped") from error

    def _send_packet(self, packet):
        packet = memoryview(packet)
        with selectors.DefaultSelector() as writable:
            writable.register(self.process.stdin, selectors.EVENT_WRITE)
            while packet:
                if self.reason or self.closed:
                    raise TierError(self.reason or "worker closed")
                if not writable.select(.02):
                    continue
                try:
                    count = os.write(self.process.stdin.fileno(), packet)
                    packet = packet[count:]
                except BrokenPipeError as error:
                    self._kill("worker input closed")
                    raise TierError("worker input closed") from error

    def _read(self, block):
        while True:
            if self.reason or self.closed:
                raise TierError(self.reason or "worker closed")
            if len(self.buffer) >= 4:
                length = struct.unpack(">I", self.buffer[:4])[0]
                if not 0 < length <= MAX_FRAME:
                    self._kill("response frame bound")
                    raise TierError("response frame bound")
                if len(self.buffer) >= length + 4:
                    data = bytes(self.buffer[4:length + 4])
                    del self.buffer[:length + 4]
                    try:
                        value = _decode(data)
                        if not isinstance(value, dict) or type(value.get("protocol")) is not int or value["protocol"] != 1:
                            raise TierError("worker protocol mismatch")
                        if self.buffer:
                            raise TierError("unsolicited response bytes")
                        return value
                    except (ValueError, TypeError, RecursionError, TierError) as error:
                        self._kill("malformed response")
                        raise TierError("malformed response") from error
            events = self.selector.select(.02 if block else 0)
            if not events:
                return None if not block else self._continue_read()
            for key, _mask in events:
                chunk = os.read(key.fileobj.fileno(), 8192)
                if not chunk:
                    self.selector.unregister(key.fileobj)
                    if key.data == "stdout":
                        self._kill("worker output closed")
                        raise TierError("worker output closed")
                    continue
                target = self.buffer if key.data == "stdout" else self.diagnostics
                limit = MAX_FRAME + 4 if key.data == "stdout" else MAX_DIAGNOSTIC
                if len(target) + len(chunk) > limit:
                    self._kill("worker output bound")
                    raise TierError("worker output bound")
                target.extend(chunk)

    def _continue_read(self):
        # Iteration, not recursively accumulating frames or polling stack.
        while True:
            result = self._read(False)
            if result is not None:
                return result
            self.stop.wait(.005)

    def receive_ready(self, block=False):
        if self.ready is None:
            result = self._read(block)
            if result is None:
                return None
            if result.get("kind") != "ready":
                self._kill("expected ready")
                raise TierError("expected ready")
            self.ready = result
            with self.mutex:
                self.deadline = None
        return self.ready

    def call(self, inputs, arguments, limits):
        if self.ready is None or self.sequence >= self.bounds.max_calls:
            raise TierError("worker not ready or call bound")
        packet = _encode({"op": "call", "sequence": self.sequence + 1, "inputs": inputs,
                          "arguments": arguments, "limits": limits})
        idle = self._read(False)
        if self.buffer or idle is not None:
            self._kill("unsolicited response")
            raise TierError("unsolicited response")
        self.sequence += 1
        with self.mutex:
            self.deadline = time.monotonic() + self.bounds.call_seconds
        self._send_packet(packet)
        result = self._read(True)
        with self.mutex:
            self.deadline = None
        expected = {"protocol", "kind", "generation", "sequence", "module_sha256", "route",
                    "value", "error", "execution_seconds"}
        if (set(result) != expected or result.get("kind") != "result"
                or type(result.get("sequence")) is not int or result["sequence"] != self.sequence
                or type(result.get("generation")) is not int
                or result.get("generation") != self.ready["generation"]
                or result.get("module_sha256") != self.ready["module_sha256"]
                or result.get("route") not in ("Portable", "TypedJit", "NativeScalar", "PrecompiledScan")
                or not _duration(result.get("execution_seconds"))
                or not ((result.get("error") is None and _valid_value(result.get("value")))
                        or (type(result.get("error")) is str and result["error"] and result.get("value") is None))):
            self._kill("call binding mismatch")
            raise TierError("call binding mismatch")
        return result

    def close(self):
        if self.closed:
            return
        self._kill("closed")
        self.closed = True
        self.stop.set()
        if self.monitor is not None and self.monitor.ident is not None and threading.current_thread() is not self.monitor:
            self.monitor.join(timeout=1)
        self.selector.close()
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            stream.close()

    def receipt(self):
        return {"pid": self.process.pid, "binding": dict(self.binding), "closed": self.closed,
                "exit_code": self.process.returncode, "termination_reason": self.reason,
                "stderr_bytes": len(self.diagnostics), "stderr_sha256": _hash(self.diagnostics),
                "stderr_utf8": bytes(self.diagnostics).decode("utf-8", errors="replace"),
                "elapsed_seconds": time.monotonic() - self.started,
                "watchdog_started_before_source": self.monitor is not None and self.monitor.ident is not None}


class TieredSession:
    """One immutable module, at most two workers, one pending compile, no queue.

    Calls and promotion are fenced by one short host lock. This reference host is
    serialized and process-IPC based; it is neither a real-time callback path nor
    an accepted multi-agent scaling/latency claim. Host values are copied over IPC.
    """
    def __init__(self, binary, source, dictionary_sha256, export, bounds=Bounds(), dictionary_json=None):
        bounds.validate()
        if sys.platform != "linux" or platform.machine() != "x86_64":
            raise TierError("process qualification is Linux x86_64 only")
        if not isinstance(source, str) or len(source.encode()) > MAX_SOURCE:
            raise TierError("source bound")
        if not isinstance(export, str) or not 1 <= len(export.encode()) <= 128:
            raise TierError("export bound")
        if not isinstance(dictionary_sha256, str) or len(dictionary_sha256) != 64 or any(c not in "0123456789abcdef" for c in dictionary_sha256):
            raise TierError("dictionary identity format")
        if dictionary_json is not None and (type(dictionary_json) is not str or len(dictionary_json.encode()) > 65_536):
            raise TierError("dictionary document bound")
        self.dictionary_json = dictionary_json
        self.binary = Path(binary).resolve(strict=True)
        if not self.binary.is_file():
            raise TierError("worker executable required")
        with self.binary.open("rb") as executable:
            binary_bytes = executable.read(64 * 1024 ** 2 + 1)
        if not 0 < len(binary_bytes) <= 64 * 1024 ** 2:
            raise TierError("worker binary size bound")
        self.binary_sha256 = _hash(binary_bytes)
        self.binary_fd = os.memfd_create("fluxr-wk1-worker", os.MFD_CLOEXEC | os.MFD_ALLOW_SEALING)
        try:
            with os.fdopen(os.dup(self.binary_fd), "wb") as snapshot:
                snapshot.write(binary_bytes)
                snapshot.flush()
            fcntl.fcntl(self.binary_fd, F_ADD_SEALS, REQUIRED_SEALS)
            if fcntl.fcntl(self.binary_fd, F_GET_SEALS) & REQUIRED_SEALS != REQUIRED_SEALS:
                raise TierError("executable snapshot is not fully sealed")
        except BaseException:
            os.close(self.binary_fd)
            raise
        self.bounds = bounds
        self.configuration_sha256 = _hash(json.dumps(asdict(bounds), sort_keys=True).encode())
        self.source = source
        self.dictionary = dictionary_sha256
        self.source_sha256 = _hash(source.encode())
        self.export = export
        self.generation = 0
        self.pending = None
        self.ticket = None
        self.native = None
        self.closed = False
        self.lock = threading.RLock()
        self.events = []
        try:
            self.portable = self._spawn("portable", 0)
            self._validate_ready(self.portable.receive_ready(True), "portable", 0)
        except BaseException:
            if hasattr(self, "portable"):
                self.portable.close()
            os.close(self.binary_fd)
            raise
        self._event("portable_ready", ready=self.portable.ready)

    def _event(self, event, **fields):
        if len(self.events) >= 4096:
            raise TierError("event retention bound; close and retain receipt")
        self.events.append({"event": event, "monotonic": time.monotonic(), **fields})

    def _admit_events(self, count=1, reserve_pending=True):
        # One final close receipt is always reserved; admission precedes mutation.
        terminal = int(reserve_pending and self.pending is not None)
        if len(self.events) + count + terminal >= 4096:
            raise TierError("event retention bound; close and retain receipt")

    def _spawn(self, mode, generation):
        return _Worker(self.binary_fd, {"protocol": 1, "source": self.source,
                       "source_sha256": self.source_sha256, "dictionary_sha256": self.dictionary,
                       "dictionary_json": self.dictionary_json,
                       "export": self.export, "generation": generation, "mode": mode}, self.bounds)

    def _validate_ready(self, ready, mode, generation):
        expected = {"protocol", "kind", "generation", "source_sha256", "dictionary_sha256",
                    "module_sha256", "expanded_sha256", "export", "mode", "typed_jit_feature",
                    "dictionary_seconds", "expansion_seconds", "preparation_seconds", "max_calls", "max_frame_bytes",
                    "initialization_started_unix_ns", "preparation_started_unix_ns", "ready_unix_ns"}
        if (set(ready) != expected or type(ready.get("generation")) is not int
                or type(ready.get("max_calls")) is not int or ready["max_calls"] != 65_536
                or type(ready.get("max_frame_bytes")) is not int or ready["max_frame_bytes"] != MAX_FRAME
                or type(ready.get("typed_jit_feature")) is not bool
                or not _duration(ready.get("dictionary_seconds"))
                or not _duration(ready.get("expansion_seconds"))
                or not _duration(ready.get("preparation_seconds"))
                or any(ready.get(key) != value for key, value in {
            "kind": "ready", "protocol": 1, "generation": generation, "mode": mode,
            "source_sha256": self.source_sha256, "dictionary_sha256": self.dictionary,
            "export": self.export}.items())):
            raise TierError("ready binding mismatch")
        times = [ready[name] for name in ("initialization_started_unix_ns", "preparation_started_unix_ns", "ready_unix_ns")]
        if any(type(value) is not int or not 0 <= value <= 2 ** 64 - 1 for value in times) or times != sorted(times):
            raise TierError("ready phase timestamp mismatch")
        for key in ("module_sha256", "expanded_sha256"):
            value = ready.get(key)
            if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
                raise TierError("ready artifact identity format")

    def begin_compile(self):
        with self.lock:
            if self.closed or self.pending is not None or self.native is not None:
                raise TierError("closed, compile busy, or already promoted")
            self._admit_events(2)
            self.generation += 1
            self.ticket = Ticket(self.generation, self.source_sha256, self.dictionary,
                                 self.binary_sha256, self.configuration_sha256)
            try:
                self.pending = self._spawn("prepared", self.generation)
            except BaseException as error:
                self.ticket = None
                self._event("compile_spawn_failed", error=str(error))
                if isinstance(error, Exception):
                    raise TierError("compile worker could not start") from error
                raise
            self._event("compile_started", ticket=asdict(self.ticket), pid=self.pending.process.pid)
            return self.ticket

    def poll(self, ticket):
        with self.lock:
            if (self.closed or self.pending is None or ticket is not self.ticket
                    or ticket.generation != self.generation):
                raise TierError("stale or absent compile ticket")
            self._admit_events(reserve_pending=False)
            try:
                ready = self.pending.receive_ready(False)
                if ready is None:
                    return False
                self._validate_ready(ready, "prepared", ticket.generation)
                if any(ready[key] != self.portable.ready[key] for key in ("module_sha256", "expanded_sha256")):
                    raise TierError("candidate artifact differs from portable revision")
            except TierError as error:
                self.pending.close()
                worker_receipt = self.pending.receipt()
                self.pending = None
                self.ticket = None
                self._event("compile_rejected", error=str(error), worker=worker_receipt)
                raise
            self.native = self.pending
            self.pending = None
            self.ticket = None
            self._event("promoted", generation=ticket.generation, ready=ready,
                        scope="future calls; no on-stack replacement")
            return True

    def cancel(self, ticket):
        with self.lock:
            if self.closed or self.pending is None or ticket is not self.ticket:
                raise TierError("stale or absent compile ticket")
            self._admit_events(reserve_pending=False)
            pid = self.pending.process.pid
            self.pending.close()
            worker_receipt = self.pending.receipt()
            self.pending = None
            self.ticket = None
            self.generation += 1
            self._event("compile_cancelled_and_reaped", pid=pid, worker=worker_receipt)

    def call(self, arguments, inputs=None, limits=None):
        with self.lock:
            if self.closed:
                raise TierError("session closed")
            self._admit_events()
            inputs = [] if inputs is None else inputs
            limits = {"instructions": 50_000, "cells": 16_384, "frames": 64} if limits is None else limits
            selected = self.native or self.portable
            start = time.monotonic()
            try:
                result = selected.call(inputs, arguments, limits)
            except TierError as error:
                if selected is self.native:
                    selected.close()
                    self.native = None
                    self._event("native_failed", error=str(error), worker=selected.receipt(), fallback="portable retained; failed call is not retried")
                raise
            result["host_elapsed_seconds"] = time.monotonic() - start
            result["tier"] = "prepared_process" if selected is self.native else "portable_process"
            return result

    def receipt(self):
        with self.lock:
            return {"schema": "fluxr-wk1-tier-session-v1", "source_sha256": self.source_sha256,
                "dictionary_sha256": self.dictionary, "binary_sha256": self.binary_sha256,
                "dictionary_document_sha256": _hash(self.dictionary_json.encode()) if self.dictionary_json is not None else None,
                "configuration_sha256": self.configuration_sha256, "bounds": asdict(self.bounds),
                "events": copy.deepcopy(self.events), "max_live_workers": 2, "pending_capacity": 1,
                "executable_snapshot": "sealed memfd; exact bytes for both workers",
                "process_creation": "seccomp denies clone/clone3/fork/vfork, including extra threads",
                "closed": self.closed, "scope": "reviewed programs; RLIMIT/wall/process bounds are not filesystem/network isolation or grading authority"}

    def close(self):
        with self.lock:
            if not self.closed:
                failures = []
                disposition = {"pending_ticket": asdict(self.ticket) if self.ticket else None,
                               "worker_pids": [w.process.pid for w in (self.pending, self.native, self.portable) if w is not None]}
                workers = []
                for worker in (self.pending, self.native, self.portable):
                    if worker is not None:
                        try:
                            worker.close()
                        except (OSError, TierError) as error:
                            failures.append(str(error))
                        workers.append(worker.receipt())
                self.pending = self.native = None
                self.ticket = None
                os.close(self.binary_fd)
                self.closed = True
                self._event("close_failed" if failures else "closed_and_reaped", failures=failures,
                            disposition=disposition, workers=workers)
                if failures:
                    raise TierError("worker cleanup incomplete: " + "; ".join(failures))

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        self.close()


def _limited_exec(argv):
    if len(argv) != 3:
        raise SystemExit("invalid resource launcher")
    address, cpu = map(int, argv[:2])
    Bounds(address_bytes=address, cpu_seconds=cpu).validate()
    resource.setrlimit(resource.RLIMIT_AS, (address, address))
    resource.setrlimit(resource.RLIMIT_CPU, (cpu, cpu))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_FSIZE, (0, 0))
    resource.setrlimit(resource.RLIMIT_NOFILE, (32, 32))
    _deny_process_creation()
    # fd remains inherited so script fixtures can open their descriptor through
    # the interpreter; production workers are ELF executables, not scripts.
    binary_fd = int(argv[2])
    if fcntl.fcntl(binary_fd, F_GET_SEALS) & REQUIRED_SEALS != REQUIRED_SEALS:
        raise TierError("unsealed executable refused")
    os.set_inheritable(binary_fd, True)
    os.execve(binary_fd, ["wordcode_worker", "worker"], dict(os.environ))


def _duration(value):
    if type(value) is int:
        return 0 <= value <= 3600
    return type(value) is float and math.isfinite(value) and 0 <= value <= 3600


def _valid_value(value):
    pending = [(value, 0)]
    nodes = 0
    while pending:
        item, depth = pending.pop()
        nodes += 1
        if nodes > 32_768 or depth > 32:
            return False
        if item == "unit":
            continue
        if type(item) is not dict or len(item) != 1:
            return False
        key, part = next(iter(item.items()))
        if key == "bool":
            if type(part) is not bool:
                return False
        elif key in ("i64", "u64", "f32"):
            low, high = {"i64": (-(2 ** 63), 2 ** 63 - 1), "u64": (0, 2 ** 64 - 1),
                         "f32": (0, 2 ** 32 - 1)}[key]
            if type(part) is not int or not low <= part <= high:
                return False
        elif key in ("record", "list"):
            if type(part) is not list or len(part) > 32_768:
                return False
            pending.extend((child, depth + 1) for child in part)
        elif key == "variant":
            if (type(part) is not list or len(part) != 2 or type(part[0]) is not int
                    or not 0 <= part[0] <= 2 ** 64 - 1):
                return False
            if part[1] is not None:
                pending.append((part[1], depth + 1))
        else:
            return False
    return True


def _deny_process_creation():
    if sys.platform != "linux" or platform.machine() != "x86_64":
        raise TierError("unsupported seccomp architecture")
    class Filter(ctypes.Structure):
        _fields_ = [("code", ctypes.c_ushort), ("jt", ctypes.c_ubyte),
                    ("jf", ctypes.c_ubyte), ("k", ctypes.c_uint)]
    class Program(ctypes.Structure):
        _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.POINTER(Filter))]
    # Check AUDIT_ARCH_X86_64, reject x32 ABI, then deny all process/thread
    # creation syscalls. Everything else retains the ordinary host sandbox.
    entries = [(0x20, 0, 0, 4), (0x15, 1, 0, 0xC000003E), (0x06, 0, 0, 0x80000000),
               (0x20, 0, 0, 0), (0x35, 0, 1, 0x40000000), (0x06, 0, 0, 0x80000000)]
    for syscall in (56, 57, 58, 435):
        entries.extend(((0x15, 0, 1, syscall), (0x06, 0, 0, 0x00050001)))
    entries.append((0x06, 0, 0, 0x7FFF0000))
    array = (Filter * len(entries))(*(Filter(*entry) for entry in entries))
    program = Program(len(entries), array)
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(38, 1, 0, 0, 0) != 0 or libc.prctl(22, 2, ctypes.byref(program), 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "worker process filter unavailable")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--limited-exec":
        _limited_exec(sys.argv[2:])
    else:
        raise SystemExit("Import TieredSession; worker entry is private to the host supervisor.")
