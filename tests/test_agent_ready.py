"""Dependency-ready dispatch: independent state oracle, faults and contention."""
from __future__ import annotations

import multiprocessing
from pathlib import Path
import random
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import ContentStore, WorkQueue
from test_agent_pipeline import spec


def claim_ready_jobs(database, objects, owner, barrier, output):
    queue = WorkQueue(database, ContentStore(objects))
    try:
        barrier.wait(timeout=15)
        after, claimed = 0, []
        while True:
            result = queue.claim_ready(owner, after_sequence=after, scan_limit=3)
            if result.lease is not None:
                claimed.append((result.lease.task, result.lease.token))
            if result.next_after is None:
                break
            after = result.next_after
        output.put(claimed)
    finally:
        queue.close()


class ReadyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.store = ContentStore(self.root / "objects")
        self.now = 100_000
        self.queue = WorkQueue(self.root / "queue.sqlite", self.store, lambda: self.now)
        self.artifact = self.store.put(b"retained supervision output")

    def tearDown(self):
        self.queue.close()
        self.directory.cleanup()

    def receipt(self, lease, success=True):
        return {"schema": "fluxr-job-receipt-v1", "action_key": lease.action_key,
                "source_snapshot": lease.source_snapshot, "outcome": "succeeded" if success else "failed",
                "exit_code": 0 if success else 9, "checks_complete": True, "evidence": [self.artifact]}

    def finish(self, lease, success=True):
        return self.queue.complete(lease, self.receipt(lease, success))

    def pending(self, task):
        return self.queue.inspect(task)["pending_dependencies"]

    def test_ready_index_skips_large_blocked_prefix(self):
        self.queue.enqueue("parent", spec())
        self.queue.claim("parent", "waiting")
        for index in range(300):
            self.queue.enqueue(f"blocked-{index:03}", spec(), dependencies=["parent"])
        self.queue.enqueue("free", spec())
        legacy = self.queue.claim_next("legacy", scan_limit=8)
        self.assertIsNone(legacy.lease)
        self.assertEqual(8, legacy.examined)
        trace = []
        self.queue.connection.set_trace_callback(trace.append)
        result = self.queue.claim_ready("indexed", scan_limit=1)
        self.queue.connection.set_trace_callback(None)
        self.assertEqual("free", result.lease.task)
        self.assertEqual(1, result.examined)
        self.assertIsNone(result.next_after)
        self.assertTrue(any("INDEXED BY tasks_dependency_ready_by_sequence" in sql for sql in trace))
        self.assertIsNone(self.queue.claim_ready("no-more").lease)

    def test_all_parents_release_once_only_after_success(self):
        for name in ("left", "right"):
            self.queue.enqueue(name, spec())
        self.queue.enqueue("join", spec(), dependencies=["left", "right"])
        self.assertEqual(2, self.pending("join"))
        left = self.queue.claim_ready("worker").lease
        right = self.queue.claim_ready("worker").lease
        self.finish(left)
        self.assertEqual(1, self.pending("join"))
        self.finish(left)  # An acknowledged completion's response was lost.
        self.assertEqual(1, self.pending("join"))
        self.assertIsNone(self.queue.claim_ready("blocked").lease)
        self.finish(right)
        self.assertEqual(0, self.pending("join"))
        self.assertEqual("join", self.queue.claim_ready("join-worker").lease.task)

    def test_enqueue_after_success_is_ready_and_failed_parent_is_not(self):
        self.queue.enqueue("old", spec())
        old = self.queue.claim("old", "worker")
        self.finish(old)
        self.queue.enqueue("new", spec(), dependencies=["old"])
        self.assertEqual(0, self.pending("new"))
        new = self.queue.claim_ready("worker").lease
        self.queue.enqueue("child", spec(), dependencies=["new"])
        failed = self.finish(new, False)
        self.assertEqual(1, self.pending("child"))
        self.assertIsNone(self.queue.claim_ready("worker").lease)
        self.queue.retry_failed("new", new.token, failed, self.artifact, request_id="retry")
        retry = self.queue.claim_ready("retry-worker").lease
        self.assertEqual("new", retry.task)
        self.assertEqual(1, self.pending("child"))
        self.finish(retry)
        self.assertEqual("child", self.queue.claim_ready("worker").lease.task)

    def test_failed_commit_rolls_back_parent_and_every_edge_release(self):
        self.queue.enqueue("parent", spec())
        for name in ("a", "b"):
            self.queue.enqueue(name, spec(), dependencies=["parent"])
        lease = self.queue.claim_ready("worker").lease
        connection = self.queue.connection

        class FailCommit:
            def __getattr__(self, name):
                return getattr(connection, name)

            def execute(self, sql, *args):
                if sql == "COMMIT":
                    raise sqlite3.OperationalError("injected commit failure")
                return connection.execute(sql, *args)

        self.queue.connection = FailCommit()
        with self.assertRaises(sqlite3.OperationalError):
            self.finish(lease)
        self.queue.connection = connection
        self.assertFalse(connection.in_transaction)
        self.assertEqual("running", self.queue.inspect("parent")["state"])
        self.assertEqual([1, 1], [self.pending(name) for name in ("a", "b")])
        self.assertIsNone(self.queue.claim_ready("blocked").lease)
        self.finish(lease)
        self.assertEqual([0, 0], [self.pending(name) for name in ("a", "b")])

    def test_expiry_cancellation_retry_and_stale_fence_do_not_release_edges(self):
        self.queue.enqueue("parent", spec())
        self.queue.enqueue("child", spec(), dependencies=["parent"])
        expired = self.queue.claim_ready("worker", 1).lease
        self.now += 1000
        with self.assertRaisesRegex(ValueError, "stale"):
            self.finish(expired)
        self.assertEqual(1, self.pending("child"))
        self.queue.reconcile_expired("parent", expired.token, self.artifact)
        self.assertEqual(1, self.pending("child"))
        cancelled = self.queue.claim_ready("worker").lease
        receipt = self.receipt(cancelled, False)
        receipt.update(exit_code=130, checks_complete=False)
        failed = self.queue.complete(cancelled, receipt)
        self.assertEqual(1, self.pending("child"))
        self.queue.retry_failed("parent", cancelled.token, failed, self.artifact, request_id="cancel-retry")
        with self.assertRaisesRegex(ValueError, "stale"):
            self.finish(cancelled)
        self.assertEqual(1, self.pending("child"))
        current = self.queue.claim_ready("worker").lease
        with self.assertRaisesRegex(ValueError, "stale"):
            self.finish(expired)
        self.finish(current)
        self.assertEqual(0, self.pending("child"))

    def test_scope_conflicts_remain_fenced_and_bounded(self):
        self.queue.enqueue("holder", spec(), exclusive_scopes=["path:src"])
        holder = self.queue.claim_ready("worker", 1).lease
        self.queue.enqueue("scoped", spec(), exclusive_scopes=["path:src/nested"])
        self.queue.enqueue("free", spec())
        self.now += 2000
        blocked = self.queue.claim_ready("worker", scan_limit=1)
        self.assertEqual(1, blocked.examined)
        self.assertIsNone(blocked.lease)
        self.assertIsNotNone(blocked.next_after)
        self.assertEqual("free", self.queue.claim_ready("worker", after_sequence=blocked.next_after).lease.task)
        self.queue.reconcile_expired("holder", holder.token, self.artifact)
        self.assertEqual("holder", self.queue.claim_ready("recovery").lease.task)

    def test_a_count_cannot_bypass_the_authoritative_dependency_check(self):
        self.queue.enqueue("parent", spec())
        self.queue.claim("parent", "worker")
        self.queue.enqueue("child", spec(), dependencies=["parent"])
        self.queue.connection.execute("UPDATE tasks SET pending_dependencies=0 WHERE id='child'")
        self.assertIsNone(self.queue.claim_ready("wrong-counter").lease)
        self.assertEqual("queued", self.queue.inspect("child")["state"])

    def test_reopen_preserves_dependency_counts_without_packet_reads(self):
        self.queue.enqueue("parent", spec())
        self.queue.enqueue("child", spec(), dependencies=["parent"])
        reopened = WorkQueue(self.root / "queue.sqlite", self.store, lambda: self.now)
        try:
            def authorize(operation, table, column, database, trigger):
                if operation == sqlite3.SQLITE_READ and table == "tasks" and column == "packet":
                    return sqlite3.SQLITE_DENY
                return sqlite3.SQLITE_OK
            reopened.connection.set_authorizer(authorize)
            lease = reopened.claim_ready("worker").lease
            self.finish(lease)
            self.assertEqual("child", reopened.claim_ready("worker").lease.task)
        finally:
            reopened.close()

    def test_random_dags_match_independent_recomputation(self):
        rng = random.Random(0xF1A6)
        graph, succeeded = {}, set()
        for index in range(80):
            name = f"job-{index:03}"
            parents = sorted(rng.sample(list(graph), rng.randrange(min(6, len(graph)) + 1)))
            graph[name] = set(parents)
            self.queue.enqueue(name, spec(), dependencies=parents)
        remaining = list(graph)
        while remaining:
            expected = next(name for name in remaining if graph[name] <= succeeded)
            actual = self.queue.claim_ready("worker", scan_limit=1).lease
            self.assertEqual(expected, actual.task)
            self.finish(actual)
            succeeded.add(actual.task)
            remaining.remove(actual.task)
            for name in remaining:
                self.assertEqual(len(graph[name] - succeeded), self.pending(name))
        self.assertIsNone(self.queue.claim_ready("done").lease)

    def test_four_processes_claim_each_ready_job_once(self):
        self.queue.enqueue("parent", spec())
        self.queue.claim("parent", "blocked")
        for i in range(40):
            self.queue.enqueue(f"blocked-{i:03}", spec(), dependencies=["parent"])
            self.queue.enqueue(f"ready-{i:03}", spec())
        context = multiprocessing.get_context("spawn")
        barrier, output = context.Barrier(4), context.Queue()
        processes = [context.Process(target=claim_ready_jobs, args=(str(self.root / "queue.sqlite"),
            str(self.root / "objects"), f"worker-{i}", barrier, output)) for i in range(4)]
        for process in processes:
            process.start()
        batches = [output.get(timeout=30) for _ in processes]
        for process in processes:
            process.join(timeout=30)
            self.assertEqual(0, process.exitcode)
        jobs = [job for batch in batches for job, token in batch]
        self.assertEqual(40, len(jobs))
        self.assertEqual({f"ready-{i:03}" for i in range(40)}, set(jobs))
        self.assertTrue(all(token == 1 for batch in batches for _, token in batch))


class ReadyMigrationTests(unittest.TestCase):
    @staticmethod
    def downgrade_fixture(database):
        with sqlite3.connect(database) as connection:
            connection.execute("DROP INDEX tasks_dependency_ready_by_sequence")
            connection.execute("ALTER TABLE tasks DROP COLUMN pending_dependencies")
            connection.execute("UPDATE metadata SET value=3 WHERE key='schema'")

    def test_v3_migration_computes_pending_counts_from_existing_edges(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database, store = root / "queue.sqlite", ContentStore(root / "objects")
            original = WorkQueue(database, store, lambda: 1000)
            original.enqueue("parent", spec())
            original.enqueue("child", spec(), dependencies=["parent"])
            original.enqueue("free", spec())
            original.claim("parent", "worker")
            original.close()
            self.downgrade_fixture(database)
            upgraded = WorkQueue(database, store, lambda: 1000)
            try:
                self.assertEqual(1, upgraded.inspect("child")["pending_dependencies"])
                self.assertEqual("free", upgraded.claim_ready("new").lease.task)
                self.assertEqual(4, upgraded.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])
            finally:
                upgraded.close()

    def test_missing_v4_count_refuses_instead_of_claiming_false_ready_jobs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database, store = root / "queue.sqlite", ContentStore(root / "objects")
            WorkQueue(database, store).close()
            self.downgrade_fixture(database)
            with sqlite3.connect(database) as connection:
                connection.execute("UPDATE metadata SET value=4 WHERE key='schema'")
            with self.assertRaisesRegex(ValueError, "authority recovery"):
                WorkQueue(database, store)

    def test_dangling_edge_migration_rolls_back_without_creating_zero_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database, store = root / "queue.sqlite", ContentStore(root / "objects")
            original = WorkQueue(database, store, lambda: 1000)
            original.enqueue("parent", spec())
            original.enqueue("child", spec(), dependencies=["parent"])
            original.close()
            self.downgrade_fixture(database)
            with sqlite3.connect(database) as connection:
                connection.execute("DELETE FROM tasks WHERE id='parent'")
            with self.assertRaisesRegex(ValueError, "dangling"):
                WorkQueue(database, store)
            with sqlite3.connect(database) as connection:
                self.assertEqual(3, connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])
                self.assertNotIn("pending_dependencies", [row[1] for row in connection.execute("PRAGMA table_info(tasks)")])


if __name__ == "__main__":
    unittest.main()
