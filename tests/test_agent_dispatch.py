"""Fault, persistence and multi-process tests of bounded local dispatch."""
from __future__ import annotations

import json
import multiprocessing
import random
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import ContentStore, WorkQueue, action_key, canonical_bytes
from agent_pipeline.queue import Lease, MAX_COUNTER
from agent_pipeline.planning import scope_conflicts
from test_agent_pipeline import spec


def drain_claims(database, objects, owner, barrier, output):
    queue = WorkQueue(database, ContentStore(objects))
    try:
        barrier.wait(timeout=15)
        after, claimed = 0, []
        while True:
            result = queue.claim_next(owner, after_sequence=after, scan_limit=3)
            if result.lease is not None:
                claimed.append((result.lease.task, result.lease.token))
            if result.next_after is None:
                break
            after = result.next_after
        output.put(claimed)
    finally:
        queue.close()


class DispatchTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.store = ContentStore(self.root / "objects")
        self.now = 100_000
        self.queue = WorkQueue(self.root / "queue.sqlite", self.store, lambda: self.now)

    def tearDown(self):
        self.queue.close()
        self.directory.cleanup()

    def finish(self, lease, success=True):
        return self.queue.complete(lease, {
            "schema": "fluxr-job-receipt-v1", "action_key": lease.action_key,
            "source_snapshot": lease.source_snapshot, "outcome": "succeeded" if success else "failed",
            "exit_code": 0 if success else 9, "checks_complete": True,
            "evidence": [self.store.put(b"trusted supervision fixture")]})

    def test_fifo_uses_enqueue_order_and_preserves_dependencies(self):
        self.queue.enqueue("z-first", spec())
        self.queue.enqueue("a-child", spec(), dependencies=["z-first"])
        self.queue.enqueue("m-independent", spec())
        first = self.queue.claim_next("worker")
        self.assertEqual("z-first", first.lease.task)
        second = self.queue.claim_next("worker", after_sequence=first.next_after)
        self.assertEqual("m-independent", second.lease.task)
        self.assertEqual(2, second.examined)
        self.assertIsNone(second.next_after)
        self.assertIsNone(self.queue.claim_next("worker").lease)
        self.finish(first.lease)
        self.assertEqual("a-child", self.queue.claim_next("worker").lease.task)

    def test_bounded_cursor_reaches_work_behind_many_blocked_jobs(self):
        self.queue.enqueue("scope-holder", spec(), exclusive_scopes=["path:src"])
        holder = self.queue.claim("scope-holder", "one", 1)
        for i in range(7):
            self.queue.enqueue(f"blocked-{i}", spec(), exclusive_scopes=["path:src/mod"])
        self.queue.enqueue("free", spec())
        self.now += 2000  # Expiration must not implicitly release exclusive ownership.
        after, examined = 0, 0
        while True:
            result = self.queue.claim_next("worker", after_sequence=after, scan_limit=2)
            self.assertLessEqual(result.examined, 2)
            examined += result.examined
            if result.lease:
                self.assertEqual("free", result.lease.task)
                break
            self.assertIsNotNone(result.next_after)
            after = result.next_after
        self.assertEqual(8, examined)
        note = self.store.put(b"retained old worktree, isolated new attempt")
        self.queue.reconcile_expired("scope-holder", holder.token, note)
        # Restarting a pass is required to revisit earlier blocked jobs.
        self.assertEqual("scope-holder", self.queue.claim_next("worker").lease.task)

    def test_four_processes_claim_each_of_48_jobs_once(self):
        for i in range(48):
            self.queue.enqueue(f"job-{i:03}", spec())
        context = multiprocessing.get_context("spawn")
        barrier, output = context.Barrier(4), context.Queue()
        processes = [context.Process(target=drain_claims, args=(str(self.root / "queue.sqlite"),
            str(self.root / "objects"), f"worker-{i}", barrier, output)) for i in range(4)]
        for process in processes:
            process.start()
        batches = [output.get(timeout=30) for _ in processes]
        for process in processes:
            process.join(timeout=30)
            self.assertEqual(0, process.exitcode)
        jobs = [job for batch in batches for job, token in batch]
        self.assertEqual(48, len(jobs))
        self.assertEqual(48, len(set(jobs)))
        self.assertTrue(all(token == 1 for batch in batches for _, token in batch))
        self.assertIsNone(self.queue.claim_next("late").lease)

    def test_four_processes_preserve_exclusive_scopes_during_dispatch(self):
        for i in range(48):
            self.queue.enqueue(f"scoped-{i:03}", spec(), exclusive_scopes=[f"path:group/{i % 3}"])
        context = multiprocessing.get_context("spawn")
        barrier, output = context.Barrier(4), context.Queue()
        processes = [context.Process(target=drain_claims, args=(str(self.root / "queue.sqlite"),
            str(self.root / "objects"), f"worker-{i}", barrier, output)) for i in range(4)]
        for process in processes:
            process.start()
        batches = [output.get(timeout=30) for _ in processes]
        for process in processes:
            process.join(timeout=30)
            self.assertEqual(0, process.exitcode)
        jobs = [job for batch in batches for job, _ in batch]
        self.assertEqual(3, len(jobs))
        self.assertEqual({0, 1, 2}, {int(job.split("-")[1]) % 3 for job in jobs})
        self.assertEqual(3, self.queue.connection.execute("SELECT count(*) FROM active_scopes").fetchone()[0])
        self.assertIsNone(self.queue.claim_next("late").lease)

    def test_retry_retains_receipt_fences_old_worker_and_is_durably_idempotent(self):
        self.queue.enqueue("build", spec())
        self.queue.enqueue("test", spec(), dependencies=["build"])
        first = self.queue.claim("build", "worker")
        receipt = self.finish(first, False)
        note = self.store.put(b"transient infrastructure failure; attempt retained")
        token = self.queue.retry_failed("build", first.token, receipt, note, request_id="retry-001")
        self.assertGreater(token, first.token)
        self.assertIsNone(self.queue.inspect("build")["result_digest"])
        self.assertEqual("queued", self.queue.inspect("build")["state"])
        second = self.queue.claim("build", "worker")
        self.assertGreater(second.token, token)
        with self.assertRaises(ValueError):
            self.finish(first)
        reopened = WorkQueue(self.root / "queue.sqlite", self.store, lambda: self.now)
        try:
            self.assertEqual(token, reopened.retry_failed("build", first.token, receipt, note,
                                                         request_id="retry-001"))
            self.assertEqual(second.token, reopened.inspect("build")["token"])
            with self.assertRaises(ValueError):
                reopened.retry_failed("build", first.token, receipt, self.store.put(b"other note"),
                                      request_id="retry-001")
        finally:
            reopened.close()
        self.finish(second)
        self.assertEqual("test", self.queue.claim_next("worker").lease.task)
        self.assertEqual("failed", json.loads(self.store.get(receipt))["outcome"])
        retries = [e for e in self.queue.events("build") if e["kind"] == "retry_requested"]
        self.assertEqual(1, len(retries))
        self.assertEqual(receipt, retries[0]["details"]["previous_receipt"])

    def test_retry_cannot_reopen_success_or_change_inputs(self):
        self.queue.enqueue("job", spec())
        lease = self.queue.claim("job", "worker")
        receipt = self.finish(lease)
        note = self.store.put(b"operator note")
        with self.assertRaises(ValueError):
            self.queue.retry_failed("job", lease.token, receipt, note, request_id="retry")
        changed = spec()
        changed["source_snapshot"] = "f" * 64
        with self.assertRaises(ValueError):
            self.queue.enqueue("job", changed)
        self.assertEqual("succeeded", self.queue.inspect("job")["state"])

    def test_retry_refuses_missing_previous_receipt(self):
        self.queue.enqueue("job", spec())
        lease = self.queue.claim("job", "worker")
        receipt = self.finish(lease, False)
        note = self.store.put(b"retry review")
        (self.store.root / receipt).unlink()
        with self.assertRaises(OSError):
            self.queue.retry_failed("job", lease.token, receipt, note, request_id="retry")
        row = self.queue.connection.execute("SELECT state,token FROM tasks WHERE id='job'").fetchone()
        self.assertEqual(("failed", lease.token), tuple(row))

    def test_retry_refuses_missing_failure_artifact_even_with_retained_receipt(self):
        self.queue.enqueue("job", spec())
        lease = self.queue.claim("job", "worker")
        receipt = self.finish(lease, False)
        previous = json.loads(self.store.get(receipt))
        (self.store.root / previous["evidence"][0]).unlink()
        note = self.store.put(b"retry review")
        with self.assertRaises(OSError):
            self.queue.retry_failed("job", lease.token, receipt, note, request_id="retry")
        self.assertEqual("failed", self.queue.inspect("job")["state"])
        self.assertEqual(lease.token, self.queue.inspect("job")["token"])

    def test_duplicate_receipt_references_do_not_rehash_same_blob_repeatedly(self):
        self.queue.enqueue("job", spec())
        lease = self.queue.claim("job", "worker")
        artifact = self.store.put(b"one shared log")
        with patch.object(self.store, "verify", wraps=self.store.verify) as verify:
            self.queue.complete(lease, {"schema": "fluxr-job-receipt-v1", "action_key": lease.action_key,
                "source_snapshot": lease.source_snapshot, "outcome": "succeeded", "exit_code": 0,
                "checks_complete": True, "evidence": [artifact] * 128})
        self.assertEqual(1, verify.call_count)

    def test_enqueue_copies_action_before_hashing_and_storing(self):
        original = spec()
        expected_key = action_key(original)
        def mutate_after_key(owned):
            key = action_key(owned)
            original["profile"] = "changed-by-other-client"
            original["inputs"].clear()
            return key
        with patch("agent_pipeline.queue.action_key", side_effect=mutate_after_key):
            self.queue.enqueue("job", original)
        row = self.queue.inspect("job")
        self.assertEqual(expected_key, row["action_key"])
        self.assertEqual(expected_key, action_key(row["packet"]["action"]))
        self.assertEqual("debug", row["packet"]["action"]["profile"])

    def test_receipt_alias_cannot_turn_failed_bytes_into_success_state(self):
        self.queue.enqueue("job", spec())
        self.queue.enqueue("child", spec(), dependencies=["job"])
        lease = self.queue.claim("job", "worker")
        receipt = {"schema": "fluxr-job-receipt-v1", "action_key": lease.action_key,
                   "source_snapshot": lease.source_snapshot, "outcome": "failed", "exit_code": 9,
                   "checks_complete": True, "evidence": [self.store.put(b"failing output")]}
        real_put = self.store.put
        def mutate_while_storing(data):
            digest = real_put(data)
            receipt["outcome"] = "succeeded"
            receipt["exit_code"] = 0
            return digest
        with patch.object(self.store, "put", side_effect=mutate_while_storing):
            digest = self.queue.complete(lease, receipt)
        self.assertEqual("failed", self.queue.inspect("job")["state"])
        self.assertEqual("failed", json.loads(self.store.get(digest))["outcome"])
        self.assertIsNone(self.queue.claim_next("worker").lease)

    def test_failed_commit_does_not_keep_writer_lock_or_partial_claim(self):
        self.queue.enqueue("job", spec())
        connection = self.queue.connection
        class FailCommit:
            failed = False
            def __getattr__(self, name):
                return getattr(connection, name)
            def execute(self, sql, *args):
                if sql == "COMMIT" and not self.failed:
                    self.failed = True
                    raise sqlite3.OperationalError("injected commit failure")
                return connection.execute(sql, *args)
        self.queue.connection = FailCommit()
        with self.assertRaises(sqlite3.OperationalError):
            self.queue.claim("job", "worker")
        self.assertFalse(connection.in_transaction)
        self.assertEqual("queued", self.queue.inspect("job")["state"])
        self.assertEqual(1, self.queue.claim("job", "next").token)

    def test_counter_and_clock_limits_refuse_without_wrapping(self):
        self.queue.enqueue("job", spec())
        self.queue.connection.execute("UPDATE tasks SET token=? WHERE id='job'", (MAX_COUNTER,))
        with self.assertRaisesRegex(ValueError, "exhausted"):
            self.queue.claim("job", "worker")
        self.assertEqual(MAX_COUNTER, self.queue.inspect("job")["token"])
        self.queue.enqueue("clock-job", spec())
        self.now = MAX_COUNTER - 1
        with self.assertRaisesRegex(ValueError, "deadline"):
            self.queue.claim("clock-job", "worker", 1)
        self.assertEqual("queued", self.queue.inspect("clock-job")["state"])
        self.now = MAX_COUNTER + 1
        with self.assertRaisesRegex(ValueError, "clock"):
            self.queue.claim_next("worker")
        self.assertFalse(self.queue.connection.in_transaction)

    def test_invalid_poll_and_fence_fields_reject(self):
        for kwargs in [{"scan_limit": 0}, {"scan_limit": True}, {"scan_limit": 4097},
                       {"after_sequence": -1}, {"after_sequence": True}]:
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                self.queue.claim_next("worker", **kwargs)
        for owner in ["", "a\nforged-log", "\0", "x" * 257]:
            with self.subTest(owner=owner), self.assertRaises(ValueError):
                self.queue.claim_next(owner)
        for token in [True, 0, MAX_COUNTER + 1]:
            with self.subTest(token=token), self.assertRaises(ValueError):
                Lease("task", "owner", token, 1, "a" * 64, "b" * 64)

    def test_dispatch_never_reads_large_immutable_packets(self):
        large = spec()
        large["environment"]["LARGE_INPUT"] = "x" * (1024 * 1024)
        for i in range(4):
            self.queue.enqueue(f"large-{i}", large, exclusive_scopes=[f"path:module/{i}"])
        def authorize(operation, table, column, database, trigger):
            if operation == sqlite3.SQLITE_READ and table == "tasks" and column == "packet":
                return sqlite3.SQLITE_DENY
            return sqlite3.SQLITE_OK
        self.queue.connection.set_authorizer(authorize)
        try:
            for i in range(4):
                lease = self.queue.claim_next("worker").lease
                self.assertEqual(f"large-{i}", lease.task)
                self.queue.renew(lease)
                self.finish(lease)
        finally:
            self.queue.connection.set_authorizer(None)
        self.assertEqual("succeeded", self.queue.inspect("large-3")["state"])

    def test_indexed_scope_queries_match_pairwise_reference(self):
        rng = random.Random(0xF105)
        names = ["path:src", "path:src/a", "path:src/a/b", "path:src2", "path:src_",
                 "path:src%/a", "path:src%/b", "path:src%", "path:é/🙂", "path:é",
                 "symbol:a", "symbol:a/b", "contract:api", "contract:api/a", "*"]
        for trial in range(80):
            scopes = rng.sample(names, rng.randrange(1, 5))
            self.queue.enqueue(f"holder-{trial}", spec(), exclusive_scopes=scopes)
            lease = self.queue.claim(f"holder-{trial}", "worker")
            for _ in range(15):
                candidates = rng.sample(names, rng.randrange(5))
                self.assertEqual(scope_conflicts(scopes, candidates),
                                 self.queue._active_scope_conflict(candidates))
            self.finish(lease, success=trial % 2 == 0)
            self.assertEqual(0, self.queue.connection.execute("SELECT count(*) FROM active_scopes").fetchone()[0])

    def test_failed_claim_commit_rolls_back_scope_acquisition(self):
        self.queue.enqueue("scoped", spec(), exclusive_scopes=["path:src"])
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
            self.queue.claim_next("worker")
        self.queue.connection = connection
        self.assertEqual(0, connection.execute("SELECT count(*) FROM active_scopes").fetchone()[0])
        self.assertEqual("scoped", self.queue.claim_next("recovery").lease.task)

    def test_reopen_keeps_expired_scopes_until_reconciled(self):
        self.queue.enqueue("old", spec(), exclusive_scopes=["path:src"])
        lease = self.queue.claim("old", "one", 1)
        self.queue.enqueue("new", spec(), exclusive_scopes=["path:src/nested"])
        self.now += 2000
        reopened = WorkQueue(self.root / "queue.sqlite", self.store, lambda: self.now)
        try:
            with self.assertRaisesRegex(ValueError, "exclusive"):
                reopened.claim("new", "two")
            reopened.reconcile_expired("old", lease.token, self.store.put(b"old worker fenced externally"))
            self.assertEqual("new", reopened.claim("new", "two").task)
        finally:
            reopened.close()


class MigrationTests(unittest.TestCase):
    def test_missing_authority_is_never_reinitialized_by_ordinary_open(self):
        for missing in ("metadata", "dependencies", "scopes", "events", "requests", "clock_floor_ms"):
            with self.subTest(missing=missing), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                database, store = root / "damaged.sqlite", ContentStore(root / "objects")
                original = WorkQueue(database, store, lambda: 1000)
                original.enqueue("parent", spec(), exclusive_scopes=["path:src"])
                original.enqueue("child", spec(), dependencies=["parent"])
                original.close()
                with sqlite3.connect(database) as connection:
                    if missing == "clock_floor_ms":
                        connection.execute("DELETE FROM metadata WHERE key='clock_floor_ms'")
                    else:
                        connection.execute("DROP TABLE " + missing)
                with self.assertRaisesRegex(ValueError, "authority recovery"):
                    WorkQueue(database, store, lambda: 0)
                with sqlite3.connect(database) as connection:
                    if missing == "clock_floor_ms":
                        self.assertIsNone(connection.execute("SELECT value FROM metadata WHERE key='clock_floor_ms'").fetchone())
                    else:
                        self.assertIsNone(connection.execute("SELECT name FROM sqlite_master WHERE name=?", (missing,)).fetchone())
                    self.assertEqual(("queued", 0), connection.execute("SELECT state,token FROM tasks WHERE id='child'").fetchone())

    def test_invalid_persisted_clock_floor_cannot_revive_leases(self):
        for invalid in (-1, "not-a-clock", 1.5):
            with self.subTest(invalid=invalid), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                database, store = root / "clock.sqlite", ContentStore(root / "objects")
                original = WorkQueue(database, store, lambda: 1000)
                original.enqueue("held", spec())
                lease = original.claim("held", "worker", 1)
                original.close()
                with sqlite3.connect(database) as connection:
                    connection.execute("UPDATE metadata SET value=? WHERE key='clock_floor_ms'", (invalid,))
                with self.assertRaisesRegex(ValueError, "persisted clock floor"):
                    WorkQueue(database, store, lambda: 0)
                with sqlite3.connect(database) as connection:
                    self.assertEqual(("running", lease.token), connection.execute("SELECT state,token FROM tasks").fetchone())

    def test_missing_v3_active_table_rejects_instead_of_releasing_scopes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database, store = root / "damaged-v3.sqlite", ContentStore(root / "objects")
            original = WorkQueue(database, store, lambda: 1000)
            original.enqueue("held", spec(), exclusive_scopes=["path:src"])
            lease = original.claim("held", "worker")
            original.close()
            with sqlite3.connect(database) as connection:
                connection.execute("DROP TABLE active_scopes")
            with self.assertRaisesRegex(ValueError, "authority recovery"):
                WorkQueue(database, store, lambda: 2000)
            with sqlite3.connect(database) as connection:
                self.assertIsNone(connection.execute("SELECT name FROM sqlite_master WHERE name='active_scopes'").fetchone())
                self.assertEqual(("running", lease.token), connection.execute("SELECT state,token FROM tasks").fetchone())

    def test_v1_migrates_live_scope_and_fence_atomically(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database = root / "v1.sqlite"
            connection = sqlite3.connect(database)
            connection.executescript("""
                CREATE TABLE metadata (key TEXT PRIMARY KEY,value INTEGER NOT NULL);
                INSERT INTO metadata VALUES ('schema',1),('clock_floor_ms',1000);
                CREATE TABLE tasks (id TEXT PRIMARY KEY,action_key TEXT NOT NULL,source_snapshot TEXT NOT NULL,
                    packet TEXT NOT NULL,state TEXT NOT NULL,owner TEXT,token INTEGER NOT NULL DEFAULT 0,
                    expires_ms INTEGER,result_digest TEXT);
                CREATE TABLE dependencies (task TEXT NOT NULL REFERENCES tasks(id),
                    dependency TEXT NOT NULL REFERENCES tasks(id),PRIMARY KEY(task,dependency));
                CREATE TABLE events (sequence INTEGER PRIMARY KEY AUTOINCREMENT,at_ms INTEGER NOT NULL,
                    task TEXT NOT NULL,token INTEGER NOT NULL,kind TEXT NOT NULL,details TEXT NOT NULL);
            """)
            packet = canonical_bytes({"action": spec(), "dependencies": [], "exclusive_scopes": ["path:src"]}).decode()
            connection.execute("INSERT INTO tasks VALUES (?,?,?,?,?,?,?,?,?)", (
                "legacy", action_key(spec()), spec()["source_snapshot"], packet, "running", "old", 7, 5000, None))
            connection.execute("INSERT INTO events VALUES (1,1000,'legacy',0,'enqueued','{}')")
            connection.commit()
            connection.close()
            queue = WorkQueue(database, ContentStore(root / "objects"), lambda: 2000)
            try:
                self.assertEqual(7, queue.inspect("legacy")["token"])
                self.assertEqual(1, queue.inspect("legacy")["enqueue_sequence"])
                self.assertEqual(4, queue.connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])
                queue.enqueue("new", spec(), exclusive_scopes=["path:src/mod"])
                self.assertIsNone(queue.claim_next("worker").lease)
                queue.renew(Lease("legacy", "old", 7, 5000, action_key(spec()), spec()["source_snapshot"]))
            finally:
                queue.close()

    def test_v2_migration_builds_index_without_releasing_expired_attempt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database, store = root / "v2.sqlite", ContentStore(root / "objects")
            previous = WorkQueue(database, store, lambda: 1000)
            previous.enqueue("old", spec(), exclusive_scopes=["path:src"])
            lease = previous.claim("old", "worker", 1)
            previous.enqueue("queued", spec(), exclusive_scopes=["path:src/mod"])
            previous.close()
            with sqlite3.connect(database) as connection:
                connection.execute("DROP TABLE active_scopes")
                connection.execute("DROP INDEX tasks_dependency_ready_by_sequence")
                connection.execute("ALTER TABLE tasks DROP COLUMN pending_dependencies")
                connection.execute("UPDATE metadata SET value=2 WHERE key='schema'")
            upgraded = WorkQueue(database, store, lambda: 5000)
            try:
                self.assertEqual(lease.token, upgraded.inspect("old")["token"])
                self.assertEqual([("old", "path:src")], [tuple(r) for r in
                    upgraded.connection.execute("SELECT task,scope FROM active_scopes")])
                self.assertIsNone(upgraded.claim_next("new").lease)
                upgraded.reconcile_expired("old", lease.token, store.put(b"v2 migration recovery"))
                self.assertEqual("queued", upgraded.claim("queued", "new").task)
            finally:
                upgraded.close()

    def test_v2_migration_failure_keeps_old_schema_and_fences(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database, store = root / "invalid-v2.sqlite", ContentStore(root / "objects")
            previous = WorkQueue(database, store, lambda: 1000)
            previous.enqueue("old", spec(), exclusive_scopes=["path:src"])
            lease = previous.claim("old", "worker", 1)
            previous.close()
            with sqlite3.connect(database) as connection:
                connection.execute("DROP TABLE active_scopes")
                connection.execute("DROP INDEX tasks_dependency_ready_by_sequence")
                connection.execute("ALTER TABLE tasks DROP COLUMN pending_dependencies")
                connection.execute("UPDATE metadata SET value=2 WHERE key='schema'")
                connection.execute("UPDATE scopes SET scope=?", ("path:" + "/".join(["x"] * 129),))
            with self.assertRaisesRegex(ValueError, "limit"):
                WorkQueue(database, store, lambda: 5000)
            with sqlite3.connect(database) as connection:
                self.assertEqual(2, connection.execute("SELECT value FROM metadata WHERE key='schema'").fetchone()[0])
                self.assertEqual(("running", lease.token), connection.execute("SELECT state,token FROM tasks").fetchone())
                self.assertIsNone(connection.execute("SELECT name FROM sqlite_master WHERE name='active_scopes'").fetchone())

    def test_unknown_schema_is_rejected_before_adding_tables(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            database = root / "future.sqlite"
            with sqlite3.connect(database) as connection:
                connection.execute("CREATE TABLE metadata (key TEXT PRIMARY KEY,value INTEGER NOT NULL)")
                connection.execute("INSERT INTO metadata VALUES ('schema',99)")
            with self.assertRaisesRegex(ValueError, "unsupported"):
                WorkQueue(database, ContentStore(root / "objects"))
            with sqlite3.connect(database) as connection:
                names = [r[0] for r in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")]
            self.assertEqual(["metadata"], names)




if __name__ == "__main__":
    unittest.main()
