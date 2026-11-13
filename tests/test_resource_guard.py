#!/usr/bin/env python3
"""Deterministic unit tests and low-resource Linux process-tree safety probes."""
import concurrent.futures
import json
import os
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import threading
import unittest
from unittest import mock

sys.dont_write_bytecode = True
BASE = Path(__file__).resolve().parents[1]
TOOLS = BASE / "tools"
sys.path.insert(0, str(TOOLS))
import resource_guard as guard

SCRIPT = TOOLS / "resource_guard.py"
MEASUREMENTS = []


class GuardTests(unittest.TestCase):
    def setUp(self):
        temporary_root = Path(os.environ.get("TMPDIR") or tempfile.gettempdir()).resolve(strict=True)
        if temporary_root.is_relative_to(BASE):
            raise RuntimeError("resource guard tests require a writable TMPDIR outside the checkout")
        self.temp = tempfile.TemporaryDirectory(prefix="fluxr-guard-test-", dir=temporary_root)
        self.root = Path(self.temp.name)
        self.state = self.root / "state"
        self.policy = guard.init_state(self.state, self.root,
            {"poll_seconds": .025, "termination_grace_seconds": .1,
             "heartbeat_seconds": .1, "max_log_bytes": 16384})
        self.server = None
        self.server_log = None

    def tearDown(self):
        if self.server is not None:
            guard.atomic_json(self.state / "stop.request", {"test": True})
            try:
                self.server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.server.kill()
                self.server.wait()
            self.server_log.close()
        self.temp.cleanup()

    def start(self, injected=False, audit_failure=False, progressive=False,
              final_audit_failure=False):
        self.server_log = (self.root / "server.log").open("wb")
        if final_audit_failure:
            code = ('import sys; from pathlib import Path; import resource_guard as g; '
                    'state=Path(sys.argv[1]); root=state.parent; c=g.Coordinator(state); original=c.audit\n'
                    'def audit(event,**fields):\n'
                    ' if event == "job_finished":\n'
                    '  published=(state/"results"/(fields["job"]+".json")).exists()\n'
                    '  (root/"audit-observed-publication").write_text(str(published))\n'
                    '  raise OSError("injected final audit storage failure")\n'
                    ' original(event,**fields)\n'
                    'c.audit=audit; c.serve()')
            args = [sys.executable, "-B", "-c", code, str(self.state)]
        elif progressive:
            code = ('import sys; from pathlib import Path; import resource_guard as g; '
                    'state=Path(sys.argv[1]); root=state.parent; '
                    'sampler=lambda: dict(g.metrics(root), memory_available_bytes='
                    '0 if (root/"stage2").exists() else g.GIB) '
                    'if (root/"trip").exists() else g.metrics(root); '
                    'g.Coordinator(state,sampler).serve()')
            args = [sys.executable, "-B", "-c", code, str(self.state)]
        elif audit_failure:
            code = ('import sys; from pathlib import Path; import resource_guard as g; '
                    'state=Path(sys.argv[1]); root=state.parent; c=g.Coordinator(state); original=c.audit\n'
                    'def audit(event,**fields):\n'
                    ' if (root/"trip").exists() and event in ("heartbeat","watchdog_error"):\n'
                    '  raise OSError("injected audit storage failure")\n'
                    ' original(event,**fields)\n'
                    'c.audit=audit; c.serve()')
            args = [sys.executable, "-B", "-c", code, str(self.state)]
        elif injected:
            code = ('import sys; from pathlib import Path; import resource_guard as g; '
                    'state=Path(sys.argv[1]); root=state.parent; '
                    'sampler=lambda: dict(g.metrics(root), memory_available_bytes=0) '
                    'if (root/"trip").exists() else g.metrics(root); '
                    'g.Coordinator(state,sampler).serve()')
            args = [sys.executable, "-B", "-c", code, str(self.state)]
        else:
            args = [sys.executable, "-B", str(SCRIPT), "--state", str(self.state), "serve"]
        self.server = subprocess.Popen(args, cwd=TOOLS, stdout=self.server_log,
                                       stderr=subprocess.STDOUT, start_new_session=True)
        self.wait_for(lambda: (self.state / "status.json").exists()
                      and guard.read_json(self.state / "status.json").get("state") == "running"
                      and guard.read_json(self.state / "status.json").get("pid") == self.server.pid)
        self.assertEqual(guard.read_json(self.state / "status.json")["state"], "running")

    def wait_for(self, predicate, timeout=8):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            if self.server and self.server.poll() is not None:
                self.fail("server exited: " + (self.root / "server.log").read_text())
            time.sleep(.02)
        self.fail("timed out")

    def submit(self, script, job="job", budget=None, timeout=3):
        return guard.submit(self.state, [sys.executable, "-c", script], self.root,
                            budget or {"disk_bytes": 1024 ** 2, "memory_bytes": 32 * 1024 ** 2,
                                       "inodes": 10}, timeout, job)

    def result(self, job="job"):
        target = self.state / "results" / (job + ".json")
        self.wait_for(target.exists)
        value = guard.read_json(target)
        MEASUREMENTS.append({"test": self.id(), "result": value})
        return value

    def test_budget_admission_math(self):
        sample = {"disk_available_bytes": 20 * guard.GIB,
                  "memory_available_bytes": 8 * guard.GIB, "inodes_available": 200000}
        self.assertEqual(guard.reserve_violation(sample, self.policy,
                         {"disk_bytes": 10 * guard.GIB, "memory_bytes": 4 * guard.GIB, "inodes": 90000}), [])
        self.assertEqual(guard.reserve_violation(sample, self.policy,
                         {"disk_bytes": 12 * guard.GIB}), ["disk_available_bytes"])

    def test_nonfinite_and_invalid_requests_rejected(self):
        for bad in [float("nan"), float("inf"), -1]:
            with self.assertRaises(ValueError):
                self.submit("pass", timeout=bad)
            with self.assertRaises(ValueError):
                self.submit("pass", budget={"disk_bytes": bad, "memory_bytes": 1, "inodes": 1})
        for bad in [float("nan"), float("inf"), -1, 0]:
            with self.assertRaises(ValueError):
                guard.init_state(self.root / ("policy-" + str(bad)), self.root, {"poll_seconds": bad})
        with self.assertRaises(ValueError):
            self.submit("pass", job="../outside")
        with self.assertRaises(ValueError):
            guard.init_state(self.root / "unknown-policy", self.root, {"invented": 1})

    def test_concurrent_duplicate_id_has_single_winner(self):
        def attempt(_):
            try:
                self.submit("pass", "same")
                return True
            except ValueError:
                return False
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            results = list(pool.map(attempt, range(16)))
        self.assertEqual(sum(results), 1)
        self.assertEqual(len(list((self.state / "queue").glob("*.json"))), 1)

    def test_ledger_ownership_and_protected_paths(self):
        disposable = self.root / "cache"
        guard.ledger_operation(self.state, "register", disposable, "test cache")
        (disposable / "output").write_text("temporary")
        protected = self.root / "source"
        protected.mkdir()
        (protected / "keep").write_text("source")
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "prune", protected)
        for invalid in [self.root, self.root.parent, self.root / "upload" / "x", self.state / "x"]:
            with self.assertRaises(ValueError):
                guard.ledger_operation(self.state, "register", invalid, "bad")
        symlink = self.root / "symlink"
        symlink.symlink_to(disposable, target_is_directory=True)
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "register", symlink / "x", "bad")
        guard.ledger_operation(self.state, "prune", disposable)
        self.assertFalse(disposable.exists())
        self.assertEqual((protected / "keep").read_text(), "source")

    def test_success_lineage_and_prior_watchdog(self):
        self.start()
        self.submit('print("guard hello")')
        result = self.result()
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(result["implementation_sha256"], guard.IMPLEMENTATION_SHA256)
        self.assertTrue(result["watchdog_started_before_spawn"])
        self.assertEqual(Path(result["log"]).read_text(), "guard hello\n")
        self.assertEqual(result["survivors"], [])
        events = [json.loads(x) for x in (self.state / "audit.jsonl").read_text().splitlines()]
        order = [x["event"] for x in events]
        self.assertLess(order.index("heartbeat"), order.index("admitted"))

    def test_reject_before_spawn(self):
        self.start()
        self.submit('from pathlib import Path; Path("SHOULD_NOT_EXIST").touch()',
                    budget={"disk_bytes": 10 ** 15, "memory_bytes": 0, "inodes": 0})
        result = self.result()
        self.assertEqual(result["status"], "admission_rejected")
        self.assertFalse((self.root / "SHOULD_NOT_EXIST").exists())

    def test_final_audit_failure_cannot_publish_pass_or_admit_next_job(self):
        self.start(final_audit_failure=True)
        self.submit('import time; time.sleep(.1); print("completed command")')
        self.submit('from pathlib import Path; Path("SHOULD_NOT_RUN").touch()', job="later")
        result = self.result()
        self.server.wait(timeout=3)
        self.assertEqual(result["status"], "audit_error")
        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(result["command_status_before_audit_error"], "passed")
        self.assertIn("injected final audit storage failure", result["audit_error"])
        self.assertEqual((self.root / "audit-observed-publication").read_text(), "False")
        self.assertEqual(Path(result["log"]).read_text(), "completed command\n")
        self.assertEqual(result["survivors"], [])
        self.assertFalse((self.root / "SHOULD_NOT_RUN").exists())
        self.assertFalse((self.state / "results/later.json").exists())
        self.assertTrue((self.state / "queue/later.json").exists())
        self.assertEqual(guard.read_json(self.state / "status.json")["state"], "stopped")

    def test_serial_execution(self):
        self.start()
        script = ('from pathlib import Path; import time; '
                  'p=Path("exclusive"); f=p.open("x"); time.sleep(.1); f.close(); p.unlink()')
        for n in range(4):
            self.submit(script, "serial-" + str(n))
        results = [self.result("serial-" + str(n)) for n in range(4)]
        self.assertEqual([x["status"] for x in results], ["passed"] * 4)
        events = [json.loads(x) for x in (self.state / "audit.jsonl").read_text().splitlines()]
        active = 0
        for event in events:
            if event["event"] == "admitted":
                active += 1
                self.assertEqual(active, 1)
            if event["event"] == "job_finished":
                active -= 1
                self.assertEqual(active, 0)

    def test_timeout_terminates_tree(self):
        self.start()
        self.submit('import time; time.sleep(60)', timeout=.15)
        result = self.result()
        self.assertEqual(result["status"], "terminated")
        self.assertEqual(result["termination_reason"], "timeout")
        self.assertLess(result["elapsed_seconds"], 2)
        self.assertEqual(result["survivors"], [])

    def test_forced_reserve_stops_reaps_detached_descendant(self):
        self.start(injected=True)
        child = 'import time; time.sleep(60)'
        script = ('import os,subprocess,sys,time; from pathlib import Path; '
                  f'p=subprocess.Popen([sys.executable,"-c",{child!r}],start_new_session=True); '
                  'Path("child.pid").write_text(str(p.pid)); time.sleep(.1); '
                  'Path("trip").touch(); time.sleep(60)')
        self.submit(script)
        result = self.result()
        self.assertEqual(result["status"], "terminated")
        self.assertEqual(result["termination_reason"], "reserve:memory_available_bytes")
        self.assertEqual(result["survivors"], [])
        self.assertGreaterEqual(len(result["tracked_processes"]), 2)
        self.assertLess(result["elapsed_seconds"], 2)
        pid = int((self.root / "child.pid").read_text())
        self.assertIsNone(guard.proc_identity(pid))

    def test_bounded_logs_not_pass(self):
        self.start()
        self.submit('import sys; sys.stdout.write("x"*1000000)')
        result = self.result()
        self.assertEqual(result["status"], "terminated")
        self.assertEqual(result["termination_reason"], "log_limit")
        self.assertEqual(result["log_bytes"], self.policy["max_log_bytes"])
        self.assertEqual(Path(result["log"]).stat().st_size, self.policy["max_log_bytes"])

    def test_live_server_forbids_prune(self):
        disposable = self.root / "cache"
        guard.ledger_operation(self.state, "register", disposable, "test")
        self.start()
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "prune", disposable)

    def test_duplicate_coordinator_refused(self):
        self.start()
        second = subprocess.run([sys.executable, "-B", str(SCRIPT), "--state", str(self.state), "serve"],
                                capture_output=True, timeout=3)
        self.assertEqual(second.returncode, 2)
        self.assertIn(b"another coordinator", second.stderr)

    def test_stop_terminates_active_writer(self):
        self.start()
        self.submit('from pathlib import Path; import time; Path("started").touch(); time.sleep(60)')
        self.wait_for(lambda: (self.root / "started").exists())
        guard.atomic_json(self.state / "stop.request", {"test": True})
        result = self.result()
        self.assertEqual(result["termination_reason"], "coordinator_stop")
        self.assertEqual(result["survivors"], [])

    def test_background_writer_cannot_receive_pass(self):
        self.start()
        child = 'import time; time.sleep(60)'
        script = ('import subprocess,sys; '
                  f'subprocess.Popen([sys.executable,"-c",{child!r}],start_new_session=True, '
                  'stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)')
        self.submit(script)
        result = self.result()
        self.assertEqual(result["status"], "terminated")
        self.assertEqual(result["termination_reason"], "background_descendants")
        self.assertEqual(result["survivors"], [])

    def test_sigterm_stops_and_reaps_active_job(self):
        self.start()
        self.submit('from pathlib import Path; import time; Path("started").touch(); time.sleep(60)')
        self.wait_for(lambda: (self.root / "started").exists())
        self.server.send_signal(signal.SIGTERM)
        result = self.result()
        self.assertEqual(result["termination_reason"], "coordinator_stop")
        self.assertEqual(result["survivors"], [])

    def test_exact_request_and_policy_identity_retained(self):
        self.start()
        guard.submit(self.state, [sys.executable, "-c", 'import os; print(os.environ["FLUXR_TEST"])'],
                     self.root, {"disk_bytes": 1, "memory_bytes": 1, "inodes": 1}, 3,
                     "job", "retention", {"FLUXR_TEST": "exact-value"})
        result = self.result()
        raw = Path(result["retained_request"]).read_bytes()
        self.assertEqual(guard.hashlib.sha256(raw).hexdigest(), result["request_sha256"])
        self.assertEqual(json.loads(raw)["env"], {"FLUXR_TEST": "exact-value"})
        self.assertEqual(json.loads(raw)["timeout_seconds"], 3)
        self.assertEqual(result["policy_sha256"],
                         guard.hashlib.sha256((self.state / "policy.json").read_bytes()).hexdigest())

    def test_final_low_resource_sample_cannot_receive_pass(self):
        # Freeze the poll loop out of the race: the sampler used by this unit
        # probe returns high for admission, then low at completion. A harmless
        # immediate-exit child tests completion; no acquisition/build occurs.
        other = self.root / "final-check-state"
        guard.init_state(other, self.root)
        # Earlier multiprocessing tests can leave a live resource-tracker child
        # in this test process. Bounded reaping then consumes the default grace
        # plus one second (3s); let this probe reach its final telemetry gate.
        # Timeout enforcement remains covered by its separate test.
        guard.submit(other, [sys.executable, "-c", "pass"], self.root,
                     {"disk_bytes": 0, "memory_bytes": 0, "inodes": 0}, 10, "final")
        high = guard.metrics(self.root)
        low = dict(high, memory_available_bytes=0)
        samples = iter([high, low])
        coordinator = guard.Coordinator(other, lambda: high if threading.current_thread().name == "resource-watchdog"
                                        else next(samples))
        coordinator.watchdog.start()
        self.assertTrue(coordinator.ready.wait(2))
        try:
            with mock.patch.object(guard, "descendants", return_value=[]):
                coordinator.run_job(other / "queue" / "final.json")
        finally:
            coordinator.stop_event.set()
            coordinator.watchdog.join(2)
        result = guard.read_json(other / "results" / "final.json")
        self.assertEqual(result["exit_code"], 0)
        self.assertEqual(result["status"], "terminated")
        self.assertEqual(result["termination_reason"], "final_reserve:memory_available_bytes")
        MEASUREMENTS.append({"test": self.id(), "result": result})

    def test_audit_failure_kills_before_failed_error_logging(self):
        self.start(audit_failure=True)
        self.submit('from pathlib import Path; import time; Path("trip").touch(); time.sleep(60)')
        result = self.result()
        self.assertEqual(result["status"], "terminated")
        self.assertEqual(result["termination_reason"], "watchdog_error")
        self.assertEqual(result["survivors"], [])
        self.assertLess(result["elapsed_seconds"], 2)

    def test_emergency_bypasses_termination_grace(self):
        policy = guard.read_json(self.state / "policy.json")
        policy["termination_grace_seconds"] = 2
        guard.atomic_json(self.state / "policy.json", policy)
        self.start(injected=True)
        self.submit('from pathlib import Path; import signal,time; '
                    'signal.signal(signal.SIGTERM,signal.SIG_IGN); Path("trip").touch(); time.sleep(60)')
        result = self.result()
        self.assertEqual(result["termination_reason"], "reserve:memory_available_bytes")
        self.assertLess(result["elapsed_seconds"], 1)
        events = [json.loads(x) for x in (self.state / "audit.jsonl").read_text().splitlines()]
        self.assertTrue(next(x for x in events if x["event"] == "watchdog_trigger")["immediate_kill"])

    def test_cgroup_child_and_parent_limits_apply_even_with_unlimited_root(self):
        root = self.root / "cgroup"
        child = root / "group" / "child"
        child.mkdir(parents=True)
        for where, maximum, current in [(root, "max", 100), (root / "group", "900", 600),
                                        (child, "500", 450)]:
            (where / "memory.max").write_text(str(maximum))
            (where / "memory.current").write_text(str(current))
        hierarchy = self.root / "cgroup-hierarchy"
        hierarchy.write_text("0::/group/child\n")
        observations = guard.cgroup_memory_observations(root, hierarchy)
        self.assertEqual(len(observations["observations"]), 3)
        self.assertEqual(min(x["available_bytes"] for x in observations["observations"]
                             if "available_bytes" in x), 50)

    def test_nested_protected_paths_and_overlapping_registrations_are_refused(self):
        disposable = self.root / "cache"
        guard.ledger_operation(self.state, "register", disposable, "test")
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "register", disposable / "nested", "bad")
        (disposable / ".git").mkdir()
        (disposable / ".git" / "keep").write_text("history")
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "prune", disposable)
        self.assertEqual((disposable / ".git" / "keep").read_text(), "history")

    def test_disposable_directory_replacement_cannot_reuse_ownership_marker(self):
        disposable = self.root / "cache"
        guard.ledger_operation(self.state, "register", disposable, "test")
        original = self.root / "original-cache"
        disposable.rename(original)
        disposable.mkdir()
        (disposable / ".fluxr-disposable-owner").write_bytes((original / ".fluxr-disposable-owner").read_bytes())
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "prune", disposable)
        self.assertTrue(original.exists())
        self.assertTrue(disposable.exists())

    def test_generation_mismatch_never_signals_reused_process_or_group(self):
        coordinator = guard.Coordinator(self.state)
        leader = {"pid": 98765, "starttime": 1, "pgrp": 98765, "state": "S"}
        coordinator.observed = {98765: leader}
        coordinator.active = {"process": type("Process", (), {"pid": 98765})(), "leader_identity": leader}
        reused = dict(leader, starttime=2)
        with mock.patch.object(guard, "descendants", return_value=[]), \
             mock.patch.object(guard, "proc_identity", return_value=reused), \
             mock.patch.object(guard.os, "killpg") as group, mock.patch.object(guard.os, "kill") as process:
            coordinator.signal_owned(signal.SIGKILL)
            group.assert_not_called()
            process.assert_not_called()


    def test_emergency_crossing_during_grace_escalates_immediately(self):
        policy = guard.read_json(self.state / "policy.json")
        policy["termination_grace_seconds"] = 2
        guard.atomic_json(self.state / "policy.json", policy)
        self.start(progressive=True)
        self.submit('from pathlib import Path; import signal,time; '
                    'signal.signal(signal.SIGTERM,signal.SIG_IGN); Path("trip").touch(); '
                    'time.sleep(.2); Path("stage2").touch(); time.sleep(60)')
        result = self.result()
        self.assertLess(result["elapsed_seconds"], 1)
        events = [json.loads(x) for x in (self.state / "audit.jsonl").read_text().splitlines()]
        self.assertFalse(next(x for x in events if x["event"] == "watchdog_trigger")["immediate_kill"])
        self.assertIn("emergency_escalation", [x["event"] for x in events])
        self.assertEqual(result["survivors"], [])

    def test_stop_between_admission_and_spawn_refuses_launch(self):
        coordinator = guard.Coordinator(self.state)
        coordinator.watchdog.start()
        self.assertTrue(coordinator.ready.wait(2))
        original = coordinator.audit
        def stop_on_admission(event, **fields):
            original(event, **fields)
            if event == "admitted":
                coordinator.stop_event.set()
        coordinator.audit = stop_on_admission
        self.submit('from pathlib import Path; Path("SHOULD_NOT_EXIST").touch()')
        try:
            with mock.patch.object(guard.subprocess, "Popen", side_effect=AssertionError("must not spawn")):
                coordinator.run_job(self.state / "queue" / "job.json")
        finally:
            coordinator.stop_event.set()
            coordinator.watchdog.join(2)
        result = guard.read_json(self.state / "results" / "job.json")
        self.assertEqual(result["status"], "launch_error")
        self.assertIn("live healthy watchdog", result["error"])
        self.assertFalse((self.root / "SHOULD_NOT_EXIST").exists())

    def test_missing_watchdog_refuses_direct_run(self):
        coordinator = guard.Coordinator(self.state)
        self.submit("pass")
        with mock.patch.object(guard.subprocess, "Popen", side_effect=AssertionError("must not spawn")):
            coordinator.run_job(self.state / "queue" / "job.json")
        result = guard.read_json(self.state / "results" / "job.json")
        self.assertFalse(result["watchdog_started_before_spawn"])
        self.assertIn("live healthy watchdog", result["error"])

    def test_live_policy_edit_does_not_relabel_loaded_policy(self):
        original = (self.state / "policy.json").read_bytes()
        self.start()
        changed = json.loads(original)
        changed["disk_reserve_bytes"] += 1
        guard.atomic_json(self.state / "policy.json", changed)
        self.submit("pass")
        result = self.result()
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["policy_sha256"], guard.hashlib.sha256(original).hexdigest())
        self.assertEqual(Path(result["retained_policy"]).read_bytes(), original)


    def make_legacy(self):
        disposable = self.root / "legacy-cache"
        guard.ledger_operation(self.state, "register", disposable, "legacy test cache")
        ledger = guard.read_json(self.state / "ledger.json")
        for entry in ledger["entries"]:
            entry.pop("device", None)
            entry.pop("inode", None)
        # Deliberately noncanonical whitespace: migration must retain the actual bytes.
        raw = json.dumps(ledger, indent=1).encode()
        (self.state / "ledger.json").write_bytes(raw)
        guard.atomic_json(self.state / "status.json", {"state": "stopped"})
        return disposable, raw, ledger

    def test_legacy_migration_preserves_exact_history_and_binds_only_current_identity(self):
        disposable, raw, original = self.make_legacy()
        (disposable / "output").write_text("temporary")
        entry = guard.ledger_operation(self.state, "migrate-legacy", disposable)
        self.assertEqual(Path(entry["prior_ledger_path"]).read_bytes(), raw)
        self.assertEqual(entry["prior_ledger_sha256"], guard.hashlib.sha256(raw).hexdigest())
        self.assertEqual(entry["legacy_registration"], original["entries"][0])
        self.assertEqual(entry["historical_directory_identity"], "unknown_not_asserted")
        self.assertEqual(entry["identity_binding"], "current_at_explicit_legacy_migration")
        self.assertEqual((entry["device"], entry["inode"]), (disposable.stat().st_dev, disposable.stat().st_ino))
        guard.ledger_operation(self.state, "prune", disposable)
        self.assertFalse(disposable.exists())
        self.assertEqual(Path(entry["prior_ledger_path"]).read_bytes(), raw)

    def test_legacy_migration_refuses_active_or_unresolved_writers(self):
        disposable, raw, _ = self.make_legacy()
        guard.atomic_json(self.state / "running" / "unresolved.json", {"interrupted": True})
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)
        (self.state / "running" / "unresolved.json").unlink()
        self.start()
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)
        self.assertEqual((self.state / "ledger.json").read_bytes(), raw)

    def test_legacy_migration_refuses_wrong_marker_protected_contents_and_symlink(self):
        disposable, raw, _ = self.make_legacy()
        marker = disposable / ".fluxr-disposable-owner"
        original_marker = marker.read_bytes()
        marker.write_text("wrong-token")
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)
        marker.write_bytes(original_marker)
        (disposable / ".git").mkdir()
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)
        (disposable / ".git").rmdir()
        moved = self.root / "moved-cache"
        disposable.rename(moved)
        disposable.symlink_to(moved, target_is_directory=True)
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)
        self.assertEqual((self.state / "ledger.json").read_bytes(), raw)
        self.assertFalse((self.state / "ledger-history").exists())

    def test_legacy_migration_refuses_overlap_and_unstopped_state(self):
        disposable, raw, ledger = self.make_legacy()
        guard.atomic_json(self.state / "status.json", {"state": "running"})
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)
        guard.atomic_json(self.state / "status.json", {"state": "stopped"})
        ledger["entries"].append(dict(ledger["entries"][0], path=str(disposable / "nested")))
        guard.atomic_json(self.state / "ledger.json", ledger)
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)
        self.assertFalse((self.state / "ledger-history").exists())

    def test_bound_registration_cannot_be_rebound_as_legacy(self):
        disposable = self.root / "bound-cache"
        guard.ledger_operation(self.state, "register", disposable, "test")
        guard.atomic_json(self.state / "status.json", {"state": "stopped"})
        with self.assertRaises(ValueError):
            guard.ledger_operation(self.state, "migrate-legacy", disposable)



if __name__ == "__main__":
    began = time.monotonic()
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(GuardTests)
    outcome = unittest.TextTestRunner(verbosity=2).run(suite)
    evidence_path = os.environ.get("FLUXR_GUARD_TEST_EVIDENCE")
    evidence = {
        "schema": "fluxr-resource-guard-test-evidence-v1",
        "implementation_sha256": guard.IMPLEMENTATION_SHA256,
        "test_sha256": guard.hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "tests_run": outcome.testsRun, "failures": len(outcome.failures),
        "errors": len(outcome.errors), "elapsed_seconds": time.monotonic() - began,
        "measurements": MEASUREMENTS,
        "limitations": ["Injected low-resource values test kill paths; no actual resource exhaustion attempted",
                        "Small trusted Python process trees only; not hostile-code sandbox qualification",
                        "Temporary test state directories removed only after server/writers stopped"]}
    if evidence_path:
        target = Path(evidence_path).resolve()
        if target.is_relative_to(BASE):
            raise RuntimeError("guard evidence must remain outside the checkout")
        if target.exists():
            raise FileExistsError("guard evidence output must be new")
        guard.atomic_json(target, evidence)
    raise SystemExit(0 if outcome.wasSuccessful() else 1)
