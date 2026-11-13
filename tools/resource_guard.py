#!/usr/bin/env python3
"""FluxR resource coordinator: one admitted process tree, bounded logs, watchdog, ledger.

This is an operational resource guard, not a hostile-code sandbox. All participants
must route expensive work through this coordinator. Process containment assumes
Linux and ordinary child processes; user namespaces/cgroups remain separate gates.
"""
from __future__ import annotations

import argparse
import ctypes
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import re
import selectors
import stat
import shutil
import signal
import subprocess
import sys
import threading
import time
import uuid

GIB = 1024 ** 3
SCHEMA = "fluxr-resource-guard-v1"
IMPLEMENTATION_SHA256 = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
SAFE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,100}$")
DEFAULTS = {"disk_reserve_bytes": 8 * GIB, "memory_reserve_bytes": int(1.5 * GIB),
            "inode_reserve": 100000, "emergency_disk_bytes": 4 * GIB,
            "emergency_memory_bytes": int(.75 * GIB), "emergency_inodes": 50000,
            "poll_seconds": .5, "termination_grace_seconds": 2,
            "max_log_bytes": 32 * 1024 ** 2, "heartbeat_seconds": 10}


def utc():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def atomic_json(path, data):
    path = Path(path)
    temp = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    with temp.open("x") as stream:
        json.dump(data, stream, sort_keys=True, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temp, path)


def read_json(path):
    return json.loads(Path(path).read_text())


def cgroup_memory_observations(cgroup_root=Path("/sys/fs/cgroup"), hierarchy=Path("/proc/self/cgroup")):
    """Read every accessible v2 ancestor; a root 'max' never masks a child limit."""
    candidates = {cgroup_root}
    try:
        for line in hierarchy.read_text().splitlines():
            ident, controllers, relative = line.split(":", 2)
            if ident == "0" and not controllers:
                current = Path(os.path.normpath(str(cgroup_root / relative.lstrip("/"))))
                while current.is_relative_to(cgroup_root):
                    candidates.add(current)
                    if current == cgroup_root:
                        break
                    current = current.parent
    except (OSError, ValueError):
        pass
    observations = []
    for base in sorted(candidates, key=lambda p: (-len(p.parts), str(p))):
        try:
            maximum = (base / "memory.max").read_text().strip()
            current = int((base / "memory.current").read_text())
            observation = {"path": str(base), "memory_max": maximum, "memory_current": current}
            if maximum != "max":
                observation["available_bytes"] = max(0, int(maximum) - current)
            observations.append(observation)
        except (OSError, ValueError):
            continue
    return {"status": "read" if observations else "not_accessible", "observations": observations,
            "scope": "accessible cgroup v2 ancestors; inaccessible limits remain unknown"}


def metrics(root):
    disk = os.statvfs(root)
    mem = {}
    for line in Path("/proc/meminfo").read_text().splitlines():
        key, value = line.split(":", 1)
        mem[key] = int(value.strip().split()[0]) * 1024
    available = mem.get("MemAvailable", mem["MemFree"])
    cg = cgroup_memory_observations()
    for observation in cg["observations"]:
        if "available_bytes" in observation:
            available = min(available, observation["available_bytes"])
    return {"time": utc(), "monotonic": time.monotonic(),
            "disk_available_bytes": disk.f_bavail * disk.f_frsize,
            "inodes_available": disk.f_favail,
            "memory_available_bytes": available,
            "memory_total_bytes": mem["MemTotal"], "cgroup": cg,
            "quota_observation": "statvfs_available; separate_project_quota_not_exposed"}


def reserve_violation(sample, policy, budget=None):
    budget = budget or {}
    checks = [("disk_available_bytes", "disk_reserve_bytes", "disk_bytes"),
              ("memory_available_bytes", "memory_reserve_bytes", "memory_bytes"),
              ("inodes_available", "inode_reserve", "inodes")]
    return [key for key, reserve, growth in checks
            if sample[key] - budget.get(growth, 0) <= policy[reserve]]


def emergency_violation(sample, policy):
    return any(sample[key] <= policy[reserve] for key, reserve in [
        ("disk_available_bytes", "emergency_disk_bytes"),
        ("memory_available_bytes", "emergency_memory_bytes"),
        ("inodes_available", "emergency_inodes")])


def init_state(state, task_root, overrides=None):
    state = Path(state).resolve()
    root = Path(task_root).resolve(strict=True)
    if not state.is_relative_to(root):
        raise ValueError("state must be inside task root")
    state.mkdir(parents=True, exist_ok=True)
    if (state / "policy.json").exists():
        raise ValueError("state already initialized")
    policy = dict(DEFAULTS)
    policy.update(overrides or {})
    for key, value in policy.items():
        if key not in DEFAULTS:
            raise ValueError("unknown policy key: " + key)
        if type(value) not in (int, float) or not math.isfinite(value) or value <= 0:
            raise ValueError("policy values must be positive finite numbers: " + key)
    if policy["poll_seconds"] > 1 or policy["termination_grace_seconds"] > 5:
        raise ValueError("poll/grace exceeds bounded response limits")
    for normal, emergency in [("disk_reserve_bytes", "emergency_disk_bytes"),
                              ("memory_reserve_bytes", "emergency_memory_bytes"),
                              ("inode_reserve", "emergency_inodes")]:
        if policy[emergency] >= policy[normal]:
            raise ValueError("emergency reserve must be below stop reserve")
    policy.update({"schema": SCHEMA, "task_root": str(root), "state": str(state),
                   "created": utc(), "max_concurrent_jobs": 1})
    for folder in ["queue", "running", "results", "logs", "requests"]:
        (state / folder).mkdir()
    atomic_json(state / "policy.json", policy)
    atomic_json(state / "ledger.json", {"schema": SCHEMA, "entries": []})
    atomic_json(state / "initial_resources.json", metrics(root))
    return policy


def checked_path(raw, root):
    path = Path(raw)
    if not path.is_absolute():
        path = root / path
    absolute = Path(os.path.abspath(path))
    if absolute == root or not absolute.is_relative_to(root):
        raise ValueError("disposable path must be strictly inside task root")
    relative = absolute.relative_to(root)
    cursor = root
    for part in relative.parts:
        cursor = cursor / part
        if cursor.is_symlink():
            raise ValueError("symlinks are not disposable paths")
    return absolute


def ledger_operation(state, action, raw=None, purpose=None):
    state = Path(state).resolve()
    policy = read_json(state / "policy.json")
    root = Path(policy["task_root"])
    protected = {".git", "bundle", "bundles", "patches", "evidence", "deliverables", "upload"}
    with (state / "ledger.lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        ledger_bytes = (state / "ledger.json").read_bytes()
        ledger = json.loads(ledger_bytes)
        path = checked_path(raw, root)
        if state == path or state.is_relative_to(path) or path.is_relative_to(state):
            raise ValueError("coordinator state cannot be disposable")
        if protected.intersection(path.relative_to(root).parts):
            raise ValueError("protected path cannot be disposable")
        if action == "register":
            if any(x["state"] == "registered" and
                   (path == Path(x["path"]) or path.is_relative_to(x["path"])
                    or Path(x["path"]).is_relative_to(path)) for x in ledger["entries"]):
                raise ValueError("disposable registrations cannot overlap")
            path.mkdir(parents=True, exist_ok=True)
            if any(path.iterdir()):
                raise ValueError("register a new empty directory, before producing outputs")
            token = uuid.uuid4().hex
            identity = path.lstat()
            (path / ".fluxr-disposable-owner").write_text(token + "\n")
            ledger["entries"].append({"path": str(path), "purpose": purpose,
                                       "ownership_token": token, "registered": utc(),
                                       "device": identity.st_dev, "inode": identity.st_ino,
                                       "state": "registered"})
            atomic_json(state / "ledger.json", ledger)
            return ledger["entries"][-1]
        if action not in {"prune", "migrate-legacy"}:
            raise ValueError(action)
        # The server owns this lock for its entire lifetime. Neither operation
        # can race an admitted writer, and unresolved prior writers fail closed.
        with (state / "server.lock").open("a+") as server_lock:
            try:
                fcntl.flock(server_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as exc:
                raise ValueError("stop coordinator/writers before ledger migration or pruning") from exc
            if list((state / "running").glob("*.json")):
                raise ValueError("unresolved running jobs prohibit ledger migration or pruning")
            matches = [x for x in ledger["entries"] if x["path"] == str(path)
                       and x["state"] == "registered"]
            if len(matches) != 1:
                raise ValueError("path is not an active explicitly registered disposable")
            entry = matches[0]
            for other in ledger["entries"]:
                if other is not entry and other["state"] == "registered" and (
                        path.is_relative_to(other["path"]) or Path(other["path"]).is_relative_to(path)):
                    raise ValueError("overlapping legacy registration requires separate review")
            identity = path.lstat()
            if not stat.S_ISDIR(identity.st_mode):
                raise ValueError("disposable path is not a directory")
            def traversal_error(error):
                raise error
            for _directory, dirs, files in os.walk(path, followlinks=False, onerror=traversal_error):
                if protected.intersection(dirs + files):
                    raise ValueError("protected nested content prohibits ledger migration or pruning")
            marker = path / ".fluxr-disposable-owner"
            if marker.is_symlink() or marker.read_text().strip() != entry["ownership_token"]:
                raise ValueError("ownership marker mismatch")
            if action == "migrate-legacy":
                if "device" in entry or "inode" in entry:
                    raise ValueError("only fully unbound legacy registrations can be migrated")
                status_path = state / "status.json"
                if not status_path.exists() or read_json(status_path).get("state") != "stopped":
                    raise ValueError("explicit stopped status required for legacy migration")
                prior_hash = hashlib.sha256(ledger_bytes).hexdigest()
                history = state / "ledger-history"
                history.mkdir(exist_ok=True)
                prior = history / (prior_hash + ".json")
                try:
                    with prior.open("xb") as stream:
                        stream.write(ledger_bytes)
                        stream.flush()
                        os.fsync(stream.fileno())
                except FileExistsError:
                    if prior.read_bytes() != ledger_bytes:
                        raise ValueError("retained ledger identity collision")
                original = json.loads(json.dumps(entry))
                entry.update({"device": identity.st_dev, "inode": identity.st_ino,
                              "identity_bound_at": utc(),
                              "identity_binding": "current_at_explicit_legacy_migration",
                              "historical_directory_identity": "unknown_not_asserted",
                              "legacy_registration": original,
                              "prior_ledger_sha256": prior_hash, "prior_ledger_path": str(prior)})
                atomic_json(state / "ledger.json", ledger)
                return entry
            if ("device" not in entry or "inode" not in entry
                    or (identity.st_dev, identity.st_ino) != (entry["device"], entry["inode"])):
                raise ValueError("directory identity mismatch or legacy unbound registration")
            shutil.rmtree(path)
            entry.update({"state": "pruned", "pruned": utc()})
            atomic_json(state / "ledger.json", ledger)
            return entry


def proc_identity(pid):
    try:
        raw = Path(f"/proc/{pid}/stat").read_text()
        fields = raw[raw.rfind(")") + 2:].split()
        return {"pid": int(pid), "ppid": int(fields[1]), "pgrp": int(fields[2]),
                "state": fields[0], "starttime": int(fields[19])}
    except (OSError, ValueError, IndexError):
        return None


def descendants(parent):
    all_procs = {}
    for candidate in Path("/proc").iterdir():
        if candidate.name.isdigit():
            identity = proc_identity(int(candidate.name))
            if identity:
                all_procs[identity["pid"]] = identity
    found = set()
    changed = True
    while changed:
        changed = False
        for pid, identity in all_procs.items():
            if pid not in found and (identity["ppid"] == parent or identity["ppid"] in found):
                found.add(pid)
                changed = True
    return [all_procs[pid] for pid in sorted(found)]


class Coordinator:
    def __init__(self, state, sampler=None):
        self.state = Path(state).resolve()
        self.policy_bytes = (self.state / "policy.json").read_bytes()
        self.policy_sha256 = hashlib.sha256(self.policy_bytes).hexdigest()
        self.policy = json.loads(self.policy_bytes)
        policies = self.state / "policies"
        policies.mkdir(exist_ok=True)
        self.retained_policy = policies / (self.policy_sha256 + ".json")
        try:
            with self.retained_policy.open("xb") as stream:
                stream.write(self.policy_bytes)
        except FileExistsError:
            if self.retained_policy.read_bytes() != self.policy_bytes:
                raise ValueError("retained policy identity collision")
        self.root = Path(self.policy["task_root"])
        self.sampler = sampler or (lambda: metrics(self.root))
        self.active = None
        self.mutex = threading.RLock()
        self.log_mutex = threading.Lock()
        self.stop_event = threading.Event()
        self.ready = threading.Event()
        self.latest = None
        self.trigger = None
        self.external_stop_signal = None
        self.observed = {}
        self.watchdog = threading.Thread(target=self.watch, name="resource-watchdog", daemon=True)

    def audit(self, event, **fields):
        with self.log_mutex:
            with (self.state / "audit.jsonl").open("a") as stream:
                stream.write(json.dumps({"time": utc(), "event": event, "implementation_sha256": IMPLEMENTATION_SHA256, **fields}, sort_keys=True) + "\n")
                stream.flush()

    def watch(self):
        last_heartbeat = 0
        while not self.stop_event.is_set():
            try:
                sample = self.sampler()
                self.latest = sample
                violation = reserve_violation(sample, self.policy)
                with self.mutex:
                    active = self.active
                    if active:
                        for identity in descendants(os.getpid()):
                            self.observed[identity["pid"]] = identity
                        reason = None
                        if violation:
                            reason = "reserve:" + ",".join(violation)
                        elif time.monotonic() >= active["deadline"]:
                            reason = "timeout"
                        elif active.get("log_limit"):
                            reason = "log_limit"
                        elif (self.state / "stop.request").exists() or self.external_stop_signal is not None:
                            reason = "coordinator_stop"
                        if reason and self.trigger is None:
                            self.trigger = reason
                            urgent = emergency_violation(sample, self.policy)
                            self.signal_owned(signal.SIGKILL if urgent else signal.SIGTERM)
                            active["kill_deadline"] = time.monotonic() + (0 if urgent else self.policy["termination_grace_seconds"])
                            self.audit("watchdog_trigger", job=active["id"], reason=reason, metrics=sample,
                                       immediate_kill=urgent)
                        if self.trigger and emergency_violation(sample, self.policy):
                            escalating = active.get("kill_deadline", 0) > time.monotonic()
                            self.signal_owned(signal.SIGKILL)
                            active["kill_deadline"] = time.monotonic()
                            if escalating:
                                self.audit("emergency_escalation", job=active["id"], metrics=sample)
                        elif self.trigger and time.monotonic() >= active.get("kill_deadline", float("inf")):
                            self.signal_owned(signal.SIGKILL)
                if time.monotonic() - last_heartbeat >= self.policy["heartbeat_seconds"]:
                    self.audit("heartbeat", metrics=sample, active_job=active["id"] if active else None)
                    atomic_json(self.state / "status.json", {"schema": SCHEMA, "pid": os.getpid(),
                        "state": "running", "metrics": sample, "active_job": active["id"] if active else None,
                        "watchdog_started": True})
                    last_heartbeat = time.monotonic()
                self.ready.set()
            except Exception as exc:
                # Logging may itself be the failing operation: terminate first.
                with self.mutex:
                    self.stop_event.set()
                    self.trigger = "watchdog_error"
                    if self.active:
                        self.signal_owned(signal.SIGKILL)
                self.stop_event.set()
                self.ready.set()
                try:
                    self.audit("watchdog_error", error=repr(exc))
                except Exception:
                    pass
            self.stop_event.wait(self.policy["poll_seconds"])

    def current_owned(self):
        result = []
        for saved in list(self.observed.values()):
            current = proc_identity(saved["pid"])
            if current and current["starttime"] == saved["starttime"]:
                result.append(current)
        return result

    def signal_owned(self, sig):
        if not self.active:
            return
        for identity in descendants(os.getpid()):
            self.observed[identity["pid"]] = identity
        process = self.active.get("process")
        leader = self.active.get("leader_identity")
        current = proc_identity(process.pid) if process else None
        # Never send killpg to a numeric group whose leader generation vanished.
        if (current and leader and current["starttime"] == leader["starttime"]
                and current["pgrp"] == process.pid):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                pass
        for identity in self.current_owned():
            try:
                os.kill(identity["pid"], sig)
            except ProcessLookupError:
                pass

    def run_job(self, request_path):
        request_bytes = request_path.read_bytes()
        request = json.loads(request_bytes)
        job = request["id"]
        requests = self.state / "requests"
        requests.mkdir(exist_ok=True)
        retained_request = requests / request_path.name
        with retained_request.open("xb") as stream:
            stream.write(request_bytes)
        running = self.state / "running" / request_path.name
        os.replace(request_path, running)
        started = utc()
        sample = self.sampler()
        issues = reserve_violation(sample, self.policy, request["budget"])
        base = {"schema": SCHEMA, "id": job, "label": request.get("label"),
                "argv": request["argv"], "cwd": request["cwd"],
                "budget": request["budget"], "started": started, "admission_metrics": sample,
                "request_sha256": hashlib.sha256(request_bytes).hexdigest(),
                "retained_request": str(retained_request),
                "timeout_seconds": request["timeout_seconds"],
                "policy_sha256": self.policy_sha256,
                "retained_policy": str(self.retained_policy),
                "log": str(self.state / "logs" / (job + ".log")),
                "watchdog_started_before_spawn": self.ready.is_set(),
                "implementation_sha256": IMPLEMENTATION_SHA256}
        if issues:
            base.update({"status": "admission_rejected", "reason": issues, "exit_code": None,
                         "finished": utc()})
            atomic_json(self.state / "results" / (job + ".json"), base)
            running.unlink()
            self.audit("admission_rejected", job=job, reason=issues)
            return
        self.audit("admitted", job=job, budget=request["budget"], metrics=sample)
        begin = time.monotonic()
        log_bytes = 0
        env = dict(os.environ)
        env.update(request.get("env", {}))
        try:
            with self.mutex:
                if (self.stop_event.is_set() or not self.ready.is_set() or not self.watchdog.is_alive()
                        or self.external_stop_signal is not None or (self.state / "stop.request").exists()):
                    raise RuntimeError("live healthy watchdog required at spawn")
                self.trigger = None
                self.observed = {}
                process = subprocess.Popen(request["argv"], cwd=request["cwd"], env=env,
                                           stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                           stderr=subprocess.STDOUT, start_new_session=True)
                self.active = {"id": job, "process": process, "leader_identity": proc_identity(process.pid),
                               "deadline": begin + request["timeout_seconds"]}
                self.audit("spawned", job=job, leader_identity=self.active["leader_identity"])
            with Path(base["log"]).open("wb") as log, selectors.DefaultSelector() as ready:
                os.set_blocking(process.stdout.fileno(), False)
                ready.register(process.stdout, selectors.EVENT_READ)
                while True:
                    events = ready.select(.1)
                    if not events:
                        with self.mutex:
                            output_deadline = min(self.active["deadline"] + self.policy["termination_grace_seconds"] + 3,
                                self.active.get("kill_deadline", float("inf")) + 3)
                        if time.monotonic() > output_deadline:
                            raise TimeoutError("bounded output drain expired")
                        continue
                    try:
                        chunk = os.read(process.stdout.fileno(), 65536)
                    except BlockingIOError:
                        continue
                    if not chunk:
                        break
                    room = max(0, self.policy["max_log_bytes"] - log_bytes)
                    log.write(chunk[:room])
                    log_bytes += min(room, len(chunk))
                    if len(chunk) > room:
                        with self.mutex:
                            self.active["log_limit"] = True
                            if self.trigger is None:
                                self.trigger = "log_limit"
                                self.signal_owned(signal.SIGTERM)
                                self.active["kill_deadline"] = time.monotonic() + self.policy["termination_grace_seconds"]
                                self.audit("watchdog_trigger", job=job, reason="log_limit")
                log.flush()
            process.stdout.close()
            code = process.wait(timeout=max(.1, begin + request["timeout_seconds"] - time.monotonic())
                                + self.policy["termination_grace_seconds"] + 3)
            # Successful leaders may leave descendants. No background writer survives a phase.
            with self.mutex:
                for identity in descendants(os.getpid()):
                    self.observed[identity["pid"]] = identity
                live = self.current_owned()
                if live:
                    self.audit("reaping_descendants", job=job, processes=live)
                    if any(x["state"] != "Z" for x in live) and self.trigger is None:
                        self.trigger = "background_descendants"
                    self.signal_owned(signal.SIGKILL)
            deadline = time.monotonic() + self.policy["termination_grace_seconds"] + 1
            while time.monotonic() < deadline:
                try:
                    child, _ = os.waitpid(-1, os.WNOHANG)
                except ChildProcessError:
                    break
                if child == 0:
                    time.sleep(.02)
            with self.mutex:
                trigger = self.trigger
                survivors = self.current_owned()
                self.active = None
                if survivors:
                    trigger = "unreaped_survivors"
                    self.stop_event.set()
            base.update({"status": "terminated" if trigger else ("passed" if code == 0 else "failed"),
                         "exit_code": code, "termination_reason": trigger,
                         "tracked_processes": list(self.observed.values()), "survivors": survivors})
        except Exception as exc:
            cleanup_complete = True
            with self.mutex:
                if self.active:
                    self.signal_owned(signal.SIGKILL)
                    try:
                        self.active["process"].wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        cleanup_complete = False
                    self.active = None
                survivors = self.current_owned()
                if survivors or not cleanup_complete:
                    self.stop_event.set()
            base.update({"status": "launch_error" if cleanup_complete and not survivors else "cleanup_incomplete",
                         "exit_code": None, "error": repr(exc), "survivors": survivors,
                         "cleanup_complete": cleanup_complete})
        try:
            final_sample = self.sampler()
            final_violation = reserve_violation(final_sample, self.policy)
            base["final_metrics"] = final_sample
            if final_violation and base["status"] == "passed":
                base.update(status="terminated", termination_reason="final_reserve:" + ",".join(final_violation))
            if final_violation:
                self.stop_event.set()
        except Exception as exc:
            base.update(status="telemetry_error", telemetry_error=repr(exc))
            self.stop_event.set()
        base.update({"finished": utc(), "elapsed_seconds": time.monotonic() - begin,
                     "log_bytes": log_bytes})
        if Path(base["log"]).exists():
            base["log_sha256"] = hashlib.sha256(Path(base["log"]).read_bytes()).hexdigest()
        # The terminal audit is part of a successful supervision result. Publish
        # it first: if storage fails here, no consumer can observe a transient
        # passed receipt. An audit line alone is not result authority; a crash
        # before atomic result publication leaves an incomplete running attempt.
        try:
            self.audit("job_finished", job=job, status=base["status"],
                       exit_code=base["exit_code"], result_publication="pending")
        except Exception as exc:
            base.update(status="audit_error", command_status_before_audit_error=base["status"],
                        audit_error=repr(exc))
            self.stop_event.set()
        atomic_json(self.state / "results" / (job + ".json"), base)
        if not base.get("survivors") and base.get("cleanup_complete", True):
            running.unlink()

    def serve(self):
        with (self.state / "server.lock").open("a+") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as exc:
                raise ValueError("another coordinator owns this state") from exc
            if list((self.state / "running").glob("*.json")):
                raise ValueError("unresolved prior running jobs: inspect/reconcile before resume")
            try:
                rc = ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0)  # PR_SET_CHILD_SUBREAPER
                if rc != 0:
                    raise OSError(ctypes.get_errno(), "prctl")
                self.audit("subreaper_enabled")
            except (AttributeError, OSError) as exc:
                raise RuntimeError("Linux subreaper is required") from exc
            (self.state / "stop.request").unlink(missing_ok=True)
            def signal_stop(signum, _frame):
                self.external_stop_signal = signum
            old_handlers = {sig: signal.signal(sig, signal_stop) for sig in (signal.SIGINT, signal.SIGTERM)}
            self.watchdog.start()
            if not self.ready.wait(5) or self.stop_event.is_set():
                raise RuntimeError("watchdog failed before first admission")
            self.audit("coordinator_ready", pid=os.getpid(), max_concurrent_jobs=1,
                       scope="trusted_task_commands; not arbitrary-candidate isolation")
            print(json.dumps({"ready": True, "state": str(self.state), "pid": os.getpid(),
                              "implementation_sha256": IMPLEMENTATION_SHA256}), flush=True)
            try:
                while (not self.stop_event.is_set() and not (self.state / "stop.request").exists()
                       and self.external_stop_signal is None):
                    queued = sorted((self.state / "queue").glob("*.json"), key=lambda p: (p.stat().st_mtime_ns, p.name))
                    if queued:
                        self.run_job(queued[0])
                    else:
                        self.stop_event.wait(.1)
            finally:
                self.stop_event.set()
                self.watchdog.join(5)
                for sig, handler in old_handlers.items():
                    signal.signal(sig, handler)
                atomic_json(self.state / "status.json", {"state": "stopped", "time": utc(),
                                                         "pid": os.getpid(), "watchdog_started": True})
                self.audit("coordinator_stopped")


def submit(state, argv, cwd, budget, timeout, job=None, label=None, env=None):
    state = Path(state).resolve()
    job = job or time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:8]
    if not SAFE_ID.fullmatch(job):
        raise ValueError("unsafe job id")
    if (not argv or any(not isinstance(x, str) or "\0" in x for x in argv)
            or type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0
            or set(budget) != {"disk_bytes", "memory_bytes", "inodes"}
            or any(type(x) not in (int, float) or not math.isfinite(x) or x < 0
                   for x in budget.values())):
        raise ValueError("invalid command, timeout, or budget")
    if env is not None and (not isinstance(env, dict) or any(
            not isinstance(k, str) or not isinstance(v, str) or "=" in k or "\0" in k or "\0" in v
            for k, v in env.items())):
        raise ValueError("environment must map valid string keys to string values")
    request = {"schema": SCHEMA, "id": job, "argv": argv, "cwd": str(Path(cwd).resolve(strict=True)),
               "budget": budget, "timeout_seconds": timeout, "label": label, "env": env or {},
               "submitted": utc()}
    with (state / "submit.lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if any((state / folder / (job + ".json")).exists() for folder in ["queue", "running", "results"]):
            raise ValueError("job id already exists")
        atomic_json(state / "queue" / (job + ".json"), request)
    return {"submitted": job, "result": str(state / "results" / (job + ".json"))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True)
    sub = parser.add_subparsers(dest="action", required=True)
    init = sub.add_parser("init")
    init.add_argument("--root", required=True)
    init.add_argument("--policy-json")
    sub.add_parser("serve")
    sub.add_parser("status")
    sub.add_parser("stop")
    reg = sub.add_parser("register")
    reg.add_argument("path")
    reg.add_argument("--purpose", required=True)
    prune = sub.add_parser("prune")
    prune.add_argument("path")
    migrate = sub.add_parser("migrate-legacy")
    migrate.add_argument("path")
    cmd = sub.add_parser("submit")
    cmd.add_argument("--id")
    cmd.add_argument("--label")
    cmd.add_argument("--cwd", default=os.getcwd())
    cmd.add_argument("--disk-gib", type=float, required=True)
    cmd.add_argument("--memory-gib", type=float, required=True)
    cmd.add_argument("--inodes", type=int, required=True)
    cmd.add_argument("--timeout", type=float, default=1800)
    cmd.add_argument("--env-json")
    cmd.add_argument("--wait", action="store_true")
    cmd.add_argument("argv", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    state = Path(args.state).resolve()
    try:
        if args.action == "init":
            result = init_state(state, args.root, read_json(args.policy_json) if args.policy_json else None)
        elif args.action == "serve":
            Coordinator(state).serve()
            return 0
        elif args.action == "status":
            result = read_json(state / "status.json") if (state / "status.json").exists() else {"state": "not_started"}
            result.update({"queued": [p.stem for p in (state / "queue").glob("*.json")],
                           "running": [p.stem for p in (state / "running").glob("*.json")]})
        elif args.action == "stop":
            atomic_json(state / "stop.request", {"requested": utc()})
            result = {"stop_requested": True}
        elif args.action in {"register", "prune", "migrate-legacy"}:
            result = ledger_operation(state, args.action, args.path, getattr(args, "purpose", None))
        else:
            argv = args.argv[1:] if args.argv and args.argv[0] == "--" else args.argv
            result = submit(state, argv, args.cwd,
                {"disk_bytes": int(args.disk_gib * GIB), "memory_bytes": int(args.memory_gib * GIB),
                 "inodes": args.inodes}, args.timeout, args.id, args.label,
                 read_json(args.env_json) if args.env_json else None)
            print(json.dumps(result), flush=True)
            if not args.wait:
                return 0
            result_path = Path(result["result"])
            wait_deadline = time.monotonic() + args.timeout + 60
            while not result_path.exists():
                if time.monotonic() >= wait_deadline:
                    raise TimeoutError("bounded submit wait expired; inspect coordinator queue/status")
                time.sleep(.2)
            result = read_json(result_path)
        print(json.dumps(result, sort_keys=True, indent=2))
        return 0 if result.get("status") in {None, "passed"} else 1
    except Exception as exc:
        print(json.dumps({"error": repr(exc)}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
