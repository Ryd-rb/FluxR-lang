"""Fair-ready dispatch, independent sweep oracle and transaction/crash faults."""
from __future__ import annotations

import json
import multiprocessing
import os
from pathlib import Path
import random
import sqlite3
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import ContentStore, WorkQueue
from agent_pipeline.queue import MAX_COUNTER
from test_agent_pipeline import spec


def claim_process(database, objects, mode, barrier=None, output=None):
    queue = WorkQueue(database, ContentStore(objects), lambda: 1000)
    if mode in {"before-commit", "after-commit"}:
        connection = queue.connection

        class CrashCommit:
            def __getattr__(self, name):
                return getattr(connection, name)

            def execute(self, sql, *args):
                if sql == "COMMIT":
                    if mode == "after-commit":
                        connection.execute(sql, *args)
                    os._exit(77)
                return connection.execute(sql, *args)

        queue.connection = CrashCommit()
        queue.claim_fair("lost-response", scan_limit=1)
    else:
        barrier.wait(timeout=15)
        claimed = []
        for _ in range(120):
            result = queue.claim_fair(f"worker-{os.getpid()}", scan_limit=3)
            if result.lease is not None:
                claimed.append((result.lease.task, result.lease.token))
            if result.examined == 0:
                break
        output.put(claimed)
    queue.close()


class FairTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.database = self.root / "queue.sqlite"
        self.store = ContentStore(self.root / "objects")
        self.now = 1000
        self.queue = WorkQueue(self.database, self.store, lambda: self.now)
        self.artifact = self.store.put(b"independent reviewed fixture evidence")

    def tearDown(self):
        self.queue.close()
        self.directory.cleanup()

    def finish(self, lease, success=True):
        return self.queue.complete(lease, {
            "schema": "fluxr-job-receipt-v1", "action_key": lease.action_key,
            "source_snapshot": lease.source_snapshot, "outcome": "succeeded" if success else "failed",
            "exit_code": 0 if success else 7, "checks_complete": True, "evidence": [self.artifact]})

    def state(self):
        return dict(self.queue.connection.execute("SELECT key,value FROM metadata WHERE key GLOB 'fair_*'"))

    def test_explicit_migration_preserves_existing_inputs_and_legacy_policies(self):
        self.queue.enqueue("old", spec(), exclusive_scopes=["path:src"])
        holder = self.queue.claim("old", "worker")
        self.queue.enqueue("next", spec())
        before = self.queue.inspect("old")
        with self.assertRaisesRegex(ValueError, "explicitly activate"):
            self.queue.claim_fair("not-enabled")
        self.assertEqual(4, self.queue.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])
        self.queue.enable_fair_dispatch()
        self.assertEqual(before, self.queue.inspect("old"))
        self.assertEqual({"fair_epoch": 0, "fair_after": 0, "fair_upper": 0}, self.state())
        self.assertEqual("next", self.queue.claim_ready("legacy").lease.task)
        self.queue.renew(holder)
        self.queue.enable_fair_dispatch()
        self.assertEqual(5, self.queue.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])

    def test_hot_prefix_and_continuous_arrivals_cannot_extend_active_sweep(self):
        self.queue.enqueue("holder", spec(), exclusive_scopes=["path:hot"])
        self.queue.claim("holder", "long-running")
        for i in range(9):
            self.queue.enqueue(f"blocked-{i}", spec(), exclusive_scopes=["path:hot/child"])
        self.queue.enqueue("independent", spec(), exclusive_scopes=["path:cold"])
        self.queue.enable_fair_dispatch()
        upper = self.queue.inspect("independent")["enqueue_sequence"]
        examined = 0
        for i in range(5):
            result = self.queue.claim_fair(f"controller-{i}", scan_limit=2)
            examined += result.examined
            self.assertEqual(1, result.epoch)
            self.assertEqual(upper, result.upper_sequence)
            self.assertLessEqual(result.examined, 2)
            self.queue.enqueue(f"arrival-{i}", spec(), exclusive_scopes=["path:hot"])
            if i < 4:
                self.assertIsNone(result.lease)
                self.assertEqual(2, result.deferred_scopes)
                self.assertFalse(result.sweep_complete)
            else:
                self.assertEqual("independent", result.lease.task)
                self.assertEqual(1, result.deferred_scopes)
                self.assertTrue(result.sweep_complete)
        self.assertEqual(10, examined)
        self.assertEqual("queued", self.queue.inspect("arrival-0")["state"])

    def test_restart_needs_no_client_cursor_to_reach_independent_job(self):
        self.queue.enqueue("holder", spec(), exclusive_scopes=["path:hot"])
        self.queue.claim("holder", "worker")
        for i in range(8):
            self.queue.enqueue(f"blocked-{i}", spec(), exclusive_scopes=["path:hot"])
        self.queue.enqueue("free", spec())
        # An intentionally stateless caller repeats the legacy first window.
        for _ in range(4):
            self.assertIsNone(self.queue.claim_ready("stateless", scan_limit=2).lease)
        self.queue.enable_fair_dispatch()
        for call in range(5):
            reopened = WorkQueue(self.database, self.store, lambda: self.now)
            try:
                result = reopened.claim_fair("replacement-controller", scan_limit=2)
            finally:
                reopened.close()
            if call < 4:
                self.assertIsNone(result.lease)
            else:
                self.assertEqual("free", result.lease.task)

    def test_retried_prefix_waits_until_following_sweep(self):
        for name in ("first", "second", "third"):
            self.queue.enqueue(name, spec())
        self.queue.enable_fair_dispatch()
        first = self.queue.claim_fair("one", scan_limit=1)
        failed = self.finish(first.lease, False)
        self.queue.retry_failed("first", first.lease.token, failed, self.artifact, request_id="retry")
        second = self.queue.claim_fair("two", scan_limit=1)
        third = self.queue.claim_fair("three", scan_limit=1)
        self.assertEqual(["second", "third"], [second.lease.task, third.lease.task])
        self.assertEqual(1, third.epoch)
        retry = self.queue.claim_fair("retry", scan_limit=1)
        self.assertEqual("first", retry.lease.task)
        self.assertEqual(2, retry.epoch)
        self.assertGreater(retry.lease.token, first.lease.token)
        with self.assertRaisesRegex(ValueError, "stale"):
            self.finish(first.lease)

    def test_dependency_ready_behind_cursor_returns_next_sweep(self):
        self.queue.enqueue("parent", spec())
        parent = self.queue.claim("parent", "worker")
        self.queue.enqueue("child", spec(), dependencies=["parent"])
        self.queue.enqueue("a", spec())
        self.queue.enqueue("b", spec())
        self.queue.enable_fair_dispatch()
        a = self.queue.claim_fair("a", scan_limit=1)
        self.finish(parent)
        b = self.queue.claim_fair("b", scan_limit=1)
        child = self.queue.claim_fair("child", scan_limit=1)
        self.assertEqual(["a", "b", "child"], [a.lease.task, b.lease.task, child.lease.task])
        self.assertEqual([1, 1, 2], [a.epoch, b.epoch, child.epoch])

    def test_zero_pending_count_never_overrides_dependencies(self):
        self.queue.enqueue("parent", spec())
        self.queue.claim("parent", "worker")
        self.queue.enqueue("child", spec(), dependencies=["parent"])
        self.queue.connection.execute("UPDATE tasks SET pending_dependencies=0 WHERE id='child'")
        self.queue.enable_fair_dispatch()
        result = self.queue.claim_fair("wrong-counter")
        self.assertIsNone(result.lease)
        self.assertEqual(1, result.deferred_dependencies)
        self.assertTrue(result.sweep_complete)
        self.assertEqual("queued", self.queue.inspect("child")["state"])

    def test_cursor_and_fence_roll_back_on_cursor_or_commit_failure(self):
        for point in ("cursor", "commit"):
            with self.subTest(point=point):
                self.queue.enqueue(f"job-{point}", spec(), exclusive_scopes=[f"path:{point}"])
                self.queue.enable_fair_dispatch()
                before = self.state()
                connection = self.queue.connection

                class FailPoint:
                    def __getattr__(self, name):
                        return getattr(connection, name)

                    def executemany(self, sql, rows):
                        if point == "cursor" and sql.startswith("UPDATE metadata SET value=?"):
                            raise sqlite3.OperationalError("injected cursor failure")
                        return connection.executemany(sql, rows)

                    def execute(self, sql, *args):
                        if point == "commit" and sql == "COMMIT":
                            raise sqlite3.OperationalError("injected commit failure")
                        return connection.execute(sql, *args)

                self.queue.connection = FailPoint()
                try:
                    with self.assertRaisesRegex(sqlite3.OperationalError, "injected"):
                        self.queue.claim_fair("worker", scan_limit=1)
                finally:
                    self.queue.connection = connection
                self.assertFalse(connection.in_transaction)
                self.assertEqual(before, self.state())
                self.assertEqual(("queued", 0), tuple(connection.execute(
                    "SELECT state,token FROM tasks WHERE id=?", (f"job-{point}",)).fetchone()))
                self.assertFalse(connection.execute("SELECT 1 FROM active_scopes").fetchone())
                result = self.queue.claim_fair("worker", scan_limit=1)
                self.finish(result.lease)

    def test_clock_rollback_keeps_scope_and_logical_cursor_authority(self):
        self.queue.enqueue("held", spec(), exclusive_scopes=["path:hot"])
        held = self.queue.claim("held", "old", 1)
        self.queue.enqueue("blocked", spec(), exclusive_scopes=["path:hot"])
        self.queue.enqueue("free", spec())
        self.queue.enable_fair_dispatch()
        self.now = held.expires_ms + 1
        blocked = self.queue.claim_fair("first", scan_limit=1)
        self.now = 0
        free = self.queue.claim_fair("second", scan_limit=1)
        self.assertEqual(1, blocked.deferred_scopes)
        self.assertEqual("free", free.lease.task)
        self.assertEqual(blocked.epoch, free.epoch)
        self.assertGreater(free.after_sequence, blocked.after_sequence)
        with self.assertRaisesRegex(ValueError, "stale"):
            self.queue.renew(held)
        self.assertEqual("queued", self.queue.inspect("blocked")["state"])

    def test_crash_before_or_after_commit_preserves_exact_transition(self):
        self.queue.enqueue("first", spec(), exclusive_scopes=["path:a"])
        self.queue.enqueue("second", spec(), exclusive_scopes=["path:b"])
        self.queue.enable_fair_dispatch()
        context = multiprocessing.get_context("spawn")
        for mode in ("before-commit", "after-commit"):
            process = context.Process(target=claim_process,
                args=(str(self.database), str(self.store.root), mode))
            process.start()
            process.join(timeout=20)
            self.assertEqual(77, process.exitcode)
            first = self.queue.inspect("first")
            if mode == "before-commit":
                self.assertEqual(("queued", 0), (first["state"], first["token"]))
                self.assertEqual(0, self.state()["fair_epoch"])
            else:
                self.assertEqual(("running", 1), (first["state"], first["token"]))
                self.assertEqual(first["enqueue_sequence"], self.state()["fair_after"])
                self.assertEqual("second", self.queue.claim_fair("replacement", scan_limit=1).lease.task)

    def test_four_processes_share_cursor_without_duplicate_claims(self):
        self.queue.enqueue("holder", spec(), exclusive_scopes=["path:hot"])
        self.queue.claim("holder", "long-running")
        for i in range(24):
            self.queue.enqueue(f"blocked-{i}", spec(), exclusive_scopes=["path:hot/nested"])
            self.queue.enqueue(f"ready-{i}", spec())
        self.queue.enable_fair_dispatch()
        context = multiprocessing.get_context("spawn")
        barrier, output = context.Barrier(4), context.Queue()
        processes = [context.Process(target=claim_process, args=(str(self.database), str(self.store.root),
            "drain", barrier, output)) for _ in range(4)]
        for process in processes:
            process.start()
        batches = [output.get(timeout=30) for _ in processes]
        for process in processes:
            process.join(timeout=30)
            self.assertEqual(0, process.exitcode)
        jobs = [job for batch in batches for job, token in batch]
        self.assertEqual(24, len(jobs))
        self.assertEqual({f"ready-{i}" for i in range(24)}, set(jobs))
        self.assertTrue(all(token == 1 for batch in batches for _, token in batch))
        self.assertEqual(24, self.queue.connection.execute("SELECT count(*) FROM tasks WHERE state='queued'").fetchone()[0])

    def test_independent_finite_sweep_oracle_under_scope_changes(self):
        rng = random.Random(0xF8A1)
        scopes = {}
        for i in range(80):
            task = f"job-{i:02}"
            scopes[task] = f"path:group-{rng.randrange(9)}"
            self.queue.enqueue(task, spec(), exclusive_scopes=[scopes[task]])
        self.queue.enable_fair_dispatch()
        pending, running, cursor, epoch = list(scopes), {}, 0, 1
        for attempt in range(400):
            if running and rng.random() < 0.7:
                name = rng.choice(list(running))
                self.finish(running.pop(name))
            if not pending:
                break
            scan = rng.randrange(1, 8)
            active = {scopes[name] for name in running}
            indexes = [int(name.split("-")[1]) for name in pending if int(name.split("-")[1]) >= cursor]
            expected, examined = None, 0
            for index in indexes[:scan]:
                examined += 1
                cursor = index + 1
                if scopes[f"job-{index:02}"] not in active:
                    expected = f"job-{index:02}"
                    break
            complete = examined == len(indexes)
            result = self.queue.claim_fair("oracle", scan_limit=scan)
            self.assertEqual(expected, None if result.lease is None else result.lease.task)
            self.assertEqual(examined, result.examined)
            self.assertEqual(complete, result.sweep_complete)
            self.assertEqual(epoch, result.epoch)
            if expected:
                pending.remove(expected)
                running[expected] = result.lease
            if complete:
                cursor = 0
                epoch += 1
        self.assertFalse(pending, "finite workload must drain under completed holders")

    def test_no_packet_reads_and_bounded_control_row_scan(self):
        for i in range(8):
            self.queue.enqueue(f"task-{i}", spec())
        self.queue.enable_fair_dispatch()
        trace = []

        def authorizer(operation, table, column, database, trigger):
            if operation == sqlite3.SQLITE_READ and table == "tasks" and column == "packet":
                return sqlite3.SQLITE_DENY
            return sqlite3.SQLITE_OK

        self.queue.connection.set_authorizer(authorizer)
        self.queue.connection.set_trace_callback(trace.append)
        result = self.queue.claim_fair("worker", scan_limit=2)
        self.queue.connection.set_trace_callback(None)
        self.queue.connection.set_authorizer(None)
        self.assertEqual(1, result.examined)
        self.assertTrue(any("enqueue_sequence<=8" in sql and "LIMIT 3" in sql for sql in trace))

    def test_empty_polls_do_not_consume_epochs_or_claim_idleness(self):
        self.queue.enable_fair_dispatch()
        before = self.state()
        for _ in range(3):
            result = self.queue.claim_fair("worker")
            self.assertEqual(0, result.examined)
            self.assertTrue(result.sweep_complete)
            self.assertNotIn("queue_empty", result.to_dict())
        self.assertEqual(before, self.state())

    def test_invalid_inputs_and_epoch_exhaustion_reject_without_transition(self):
        self.queue.enqueue("task", spec())
        self.queue.enable_fair_dispatch()
        for scan in (0, 4097, True, 1.5):
            with self.subTest(scan=scan), self.assertRaises(ValueError):
                self.queue.claim_fair("worker", scan_limit=scan)
        for owner in ("", "x\n", None):
            with self.subTest(owner=owner), self.assertRaises(ValueError):
                self.queue.claim_fair(owner)
        self.queue.claim_fair("first")
        self.queue.enqueue("new", spec())
        self.queue.connection.execute("UPDATE metadata SET value=? WHERE key='fair_epoch'", (MAX_COUNTER,))
        with self.assertRaisesRegex(ValueError, "epoch exhausted"):
            self.queue.claim_fair("worker")
        self.assertEqual("queued", self.queue.inspect("new")["state"])

    def test_cli_activates_and_claims_explicit_policy(self):
        self.queue.enqueue("cli", spec())
        prefix = [sys.executable, "-m", "agent_pipeline", "--database", str(self.database),
                  "--store", str(self.store.root)]
        env = dict(os.environ, PYTHONPATH=str(Path(__file__).resolve().parents[1] / "tools"))
        enabled = subprocess.run(prefix + ["enable-fair"], env=env, text=True, capture_output=True, timeout=10)
        self.assertEqual(0, enabled.returncode, enabled.stderr)
        self.assertEqual("fluxr-fair-ready-v1", json.loads(enabled.stdout)["policy"])
        claimed = subprocess.run(prefix + ["claim-fair", "cli-owner", "--scan-limit", "1"],
                                 env=env, text=True, capture_output=True, timeout=10)
        self.assertEqual(0, claimed.returncode, claimed.stderr)
        self.assertEqual("cli", json.loads(claimed.stdout)["lease"]["task"])


class FairMigrationTests(unittest.TestCase):
    def test_incomplete_migration_rolls_back_schema_and_all_cursor_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            queue = WorkQueue(root / "queue.sqlite", ContentStore(root / "objects"), lambda: 1000)
            connection = queue.connection

            class FailSchema:
                def __getattr__(self, name):
                    return getattr(connection, name)

                def execute(self, sql, *args):
                    if sql.startswith("UPDATE metadata SET value=5"):
                        raise sqlite3.OperationalError("injected migration failure")
                    return connection.execute(sql, *args)

            queue.connection = FailSchema()
            with self.assertRaises(sqlite3.OperationalError):
                queue.enable_fair_dispatch()
            queue.connection = connection
            self.assertEqual(4, connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])
            self.assertFalse(connection.execute("SELECT 1 FROM metadata WHERE key GLOB 'fair_*'").fetchone())
            self.assertFalse(connection.in_transaction)
            queue.close()

    def test_missing_or_malformed_state_refuses_without_reconstruction(self):
        faults = [("DELETE FROM metadata WHERE key='fair_after'", ()),
                  ("UPDATE metadata SET value=-1 WHERE key='fair_after'", ()),
                  ("UPDATE metadata SET value=9 WHERE key='fair_after'", ()),
                  ("UPDATE metadata SET value=1.5 WHERE key='fair_epoch'", ()),
                  ("UPDATE metadata SET value=1 WHERE key='fair_epoch'", ()),
                  ("UPDATE metadata SET value=1 WHERE key IN ('fair_epoch','fair_upper')", ())]
        for statement, arguments in faults:
            with self.subTest(statement=statement), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                database, store = root / "queue.sqlite", ContentStore(root / "objects")
                queue = WorkQueue(database, store, lambda: 1000)
                queue.enable_fair_dispatch()
                queue.connection.execute(statement, arguments)
                before = list(queue.connection.execute("SELECT * FROM metadata ORDER BY key"))
                with self.assertRaises(ValueError):
                    queue.claim_fair("already-open-controller")
                queue.close()
                with self.assertRaises(ValueError):
                    WorkQueue(database, store, lambda: 2000)
                with sqlite3.connect(database) as connection:
                    self.assertEqual([tuple(row) for row in before], connection.execute("SELECT * FROM metadata ORDER BY key").fetchall())

    def test_v4_unexpected_partial_state_refuses_activation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            queue = WorkQueue(root / "queue.sqlite", ContentStore(root / "objects"))
            queue.connection.execute("INSERT INTO metadata VALUES ('fair_after',0)")
            with self.assertRaisesRegex(ValueError, "unexpected"):
                queue.enable_fair_dispatch()
            self.assertEqual(4, queue.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])
            queue.close()


if __name__ == "__main__":
    unittest.main()
