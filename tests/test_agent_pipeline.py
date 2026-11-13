"""Behavioral and fault tests for the trusted-local pipeline prototype."""
from __future__ import annotations

from copy import deepcopy
import json
import multiprocessing
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import ContentStore, WorkQueue, action_key, canonical_bytes, content_digest
from agent_pipeline.identity import MAX_JSON_BYTES, load_json_bytes
from agent_pipeline.planning import plan_batches, overlaps


def spec():
    return {"schema": "fluxr-action-v1", "argv": ["cargo", "test", "--locked"],
            "working_directory": ".",
            "environment": {"LANG": "C", "TZ": "UTC"}, "inputs": {"Cargo.lock": "a" * 64},
            "outputs": ["test.log"], "source_snapshot": "b" * 64, "toolchain": "c" * 64,
            "runner": "d" * 64, "platform": "linux-x86_64", "target": "x86_64-unknown-linux-gnu",
            "profile": "debug", "policy": "e" * 64}


def race_claim(database, objects, owner, barrier, results):
    queue = WorkQueue(database, ContentStore(objects))
    try:
        barrier.wait(timeout=15)
        try:
            lease = queue.claim("job", owner)
            results.put(("claimed", lease.owner, lease.token))
        except ValueError:
            results.put(("rejected", owner, 0))
    finally:
        queue.close()


class IdentityTests(unittest.TestCase):
    def test_action_validation_and_hash_use_one_detached_snapshot(self):
        import agent_pipeline.identity as identity
        value = spec()
        expected = action_key(value)
        real_path = identity.relative_path

        def mutate_original_during_later_field_validation(path):
            # Deterministically exercise the same mutation window as another
            # controller thread reusing the caller-owned action dictionary.
            value["argv"].clear()
            return real_path(path)

        with patch.object(identity, "relative_path", side_effect=mutate_original_during_later_field_validation):
            self.assertEqual(action_key(value), expected)
        self.assertEqual(value["argv"], [])
        with self.assertRaisesRegex(ValueError, "argv"):
            action_key(value)

    def test_stable_mapping_order_but_all_execution_inputs_matter(self):
        value = spec()
        baseline = action_key(value)
        self.assertEqual(baseline, action_key(dict(reversed(list(value.items())))))
        changes = {"argv": ["cargo", "test", "--release"], "environment": {"LANG": "C"},
                   "working_directory": "nested-crate",
                   "inputs": {"Cargo.lock": "f" * 64}, "outputs": ["another.log"],
                   "source_snapshot": "1" * 64, "toolchain": "2" * 64, "runner": "3" * 64,
                   "platform": "windows", "target": "aarch64-unknown-linux-gnu", "profile": "release",
                   "policy": "4" * 64}
        for field, replacement in changes.items():
            with self.subTest(field=field):
                changed = deepcopy(value)
                changed[field] = replacement
                self.assertNotEqual(baseline, action_key(changed))

    def test_rejects_ambiguous_json(self):
        for data in [b'{"a":1,"a":2}', b'{"x":NaN}', b'{"x":Infinity}', b'{"x":1.0}', b'{"x":9223372036854775808}']:
            with self.subTest(data=data), self.assertRaises(ValueError):
                load_json_bytes(data)
        with self.assertRaises((ValueError, UnicodeError)):
            canonical_bytes({"x": "\ud800"})
        nested = []
        for _ in range(34):
            nested = [nested]
        with self.assertRaises(ValueError):
            canonical_bytes(nested)

    def test_rejects_missing_extra_bad_paths_and_digest_fields(self):
        for path in ["../secret", "/root/file", "a//b", "a/./b", "a\\b", "a\0b", "C:/outside", "c:outside"]:
            changed = spec()
            changed["inputs"] = {path: "a" * 64}
            with self.subTest(path=path), self.assertRaises(ValueError):
                action_key(changed)
        changed = spec()
        changed["extra"] = True
        with self.assertRaises(ValueError):
            action_key(changed)
        changed = spec()
        changed.pop("policy")
        with self.assertRaises(ValueError):
            action_key(changed)
        changed = spec()
        changed["toolchain"] = "A" * 64
        with self.assertRaises(ValueError):
            action_key(changed)

    def test_oversize_values_reject_before_serializing(self):
        values = ["x" * MAX_JSON_BYTES, "\x01" * (MAX_JSON_BYTES // 6 + 1),
                  "🙂" * (MAX_JSON_BYTES // 4), ["x" * 4096] * 1024,
                  {"left": "x" * (MAX_JSON_BYTES // 2), "right": "y" * (MAX_JSON_BYTES // 2)}]
        for value in values:
            with self.subTest(kind=type(value).__name__):
                with patch("agent_pipeline.identity.json.dumps", side_effect=AssertionError("serialized before limit")):
                    with self.assertRaisesRegex(ValueError, "byte limit"):
                        canonical_bytes(value)

    def test_exact_json_budget_preserves_unicode_escape_and_container_encoding(self):
        values = [None, True, False, -(2**63), 2**63 - 1, [], {},
                  "".join(chr(c) for c in range(128)) + "é漢🙂\u2028\u2029",
                  {"z": [None, -10, True, "quote\"\\\n"], "a": {"é": "🙂"}}]
        for value in values:
            expected = json.dumps(value, ensure_ascii=False, sort_keys=True,
                                  separators=(",", ":"), allow_nan=False).encode("utf-8")
            with self.subTest(value=repr(value)):
                with patch("agent_pipeline.identity.MAX_JSON_BYTES", len(expected)):
                    self.assertEqual(expected, canonical_bytes(value))
                with patch("agent_pipeline.identity.MAX_JSON_BYTES", len(expected) - 1):
                    with self.assertRaisesRegex(ValueError, "byte limit"):
                        canonical_bytes(value)

    def test_large_admitted_string_has_exact_existing_identity(self):
        text = "x" * (MAX_JSON_BYTES - 2)
        self.assertEqual(b'"' + text.encode() + b'"', canonical_bytes(text))

    def test_serialization_uses_the_owned_budgeted_snapshot(self):
        original = {"values": ["small"]}
        expected = canonical_bytes(original)
        real_dumps = json.dumps
        def mutate_before_encoding(owned, **kwargs):
            original["values"][0] = "x" * (MAX_JSON_BYTES + 1)
            original["added"] = ["caller mutation"]
            return real_dumps(owned, **kwargs)
        with patch("agent_pipeline.identity.json.dumps", side_effect=mutate_before_encoding):
            self.assertEqual(expected, canonical_bytes(original))


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.store = ContentStore(self.root / "objects")

    def tearDown(self):
        self.directory.cleanup()

    def test_atomic_idempotent_and_reopens(self):
        value = b"exact output\x00\xff"
        digest = self.store.put(value)
        self.assertEqual(digest, self.store.put(value))
        self.assertEqual(value, ContentStore(self.root / "objects").get(digest))
        self.assertEqual([digest], sorted(p.name for p in (self.root / "objects").iterdir()))

    def test_corruption_never_repaired_silently(self):
        digest = self.store.put(b"good")
        (self.root / "objects" / digest).write_bytes(b"bad")
        with self.assertRaises(ValueError):
            self.store.get(digest)
        with self.assertRaises(ValueError):
            self.store.put(b"good")
        self.assertEqual(b"bad", (self.root / "objects" / digest).read_bytes())

    def test_symlink_and_traversal_rejected(self):
        source = self.root / "external"
        source.write_bytes(b"outside")
        digest = content_digest(b"outside")
        (self.root / "objects" / digest).symlink_to(source)
        with self.assertRaises(ValueError):
            self.store.get(digest)
        with self.assertRaises(ValueError):
            self.store.get("../external")
        with self.assertRaises(ValueError):
            self.store.put(b"outside")

    def test_parallel_same_content_does_not_overwrite(self):
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(max_workers=8) as pool:
            digests = list(pool.map(self.store.put, [b"same"] * 32))
        self.assertEqual(1, len(set(digests)))
        self.assertEqual(b"same", self.store.get(digests[0]))

    @unittest.skipUnless(hasattr(os, "mkfifo"), "POSIX FIFO rejection")
    def test_nonregular_object_cannot_block_a_reader(self):
        digest = "1" * 64
        os.mkfifo(self.root / "objects" / digest)
        with self.assertRaises(ValueError):
            self.store.get(digest)


class QueueTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.store = ContentStore(self.root / "objects")
        self.now = 100_000
        self.queue = WorkQueue(self.root / "work.sqlite", self.store, lambda: self.now)

    def tearDown(self):
        self.queue.close()
        self.directory.cleanup()

    def receipt(self, lease, **kwargs):
        value = {"schema": "fluxr-job-receipt-v1", "action_key": lease.action_key,
                 "source_snapshot": lease.source_snapshot, "outcome": "succeeded", "exit_code": 0,
                 "checks_complete": True, "evidence": [self.store.put(b"fixture: all expected cases completed") ]}
        value.update(kwargs)
        return value

    def test_dependency_and_success_are_not_lease_assignment(self):
        self.queue.enqueue("compile", spec())
        self.queue.enqueue("test", spec(), dependencies=["compile"])
        with self.assertRaises(ValueError):
            self.queue.claim("test", "b")
        first = self.queue.claim("compile", "a")
        with self.assertRaises(ValueError):
            self.queue.claim("test", "b")
        self.queue.complete(first, self.receipt(first))
        second = self.queue.claim("test", "b")
        self.assertEqual("b", second.owner)

    def test_immutable_enqueue_and_cycles(self):
        self.assertEqual(self.queue.enqueue("a", spec()), self.queue.enqueue("a", spec()))
        changed = spec()
        changed["profile"] = "release"
        with self.assertRaises(ValueError):
            self.queue.enqueue("a", changed)
        with self.assertRaises(ValueError):
            self.queue.enqueue("b", spec(), dependencies=["missing"])
        with self.assertRaises(ValueError):
            self.queue.enqueue("b", spec(), dependencies=["b"])
        self.queue.enqueue("b", spec(), dependencies=["a"])
        with self.assertRaises(ValueError):
            self.queue.enqueue("a", spec(), dependencies=["b"])

    def test_scoped_exclusivity_and_expiry_needs_reconciliation(self):
        self.queue.enqueue("a", spec(), exclusive_scopes=["path:src"])
        self.queue.enqueue("b", spec(), exclusive_scopes=["path:src/lib.rs"])
        self.queue.enqueue("c", spec(), exclusive_scopes=["path:docs"])
        first = self.queue.claim("a", "one", 1)
        self.queue.claim("c", "three", 1)
        with self.assertRaises(ValueError):
            self.queue.claim("b", "two")
        self.now += 1001
        with self.assertRaises(ValueError):
            self.queue.claim("b", "two")
        note = self.store.put(b"preserved abandoned branch and dirty worktree; replacement isolated")
        self.queue.reconcile_expired("a", first.token, note)
        self.queue.claim("b", "two")

    def test_fences_resumed_workers_and_lost_response_is_idempotent(self):
        self.queue.enqueue("job", spec())
        stale = self.queue.claim("job", "worker", 1)
        self.now += 1000
        with self.assertRaises(ValueError):
            self.queue.complete(stale, self.receipt(stale))
        with self.assertRaises(ValueError):
            self.queue.renew(stale)
        note = self.store.put(b"old result retained for review")
        self.queue.reconcile_expired("job", stale.token, note)
        fresh = self.queue.claim("job", "worker", 2)
        self.assertGreater(fresh.token, stale.token)
        with self.assertRaises(ValueError):
            self.queue.complete(stale, self.receipt(stale))
        receipt = self.receipt(fresh)
        digest = self.queue.complete(fresh, receipt)
        self.now += 10_000
        self.assertEqual(digest, self.queue.complete(fresh, receipt))
        self.assertEqual(1, sum(e["kind"] == "completed" for e in self.queue.events("job")))
        with self.assertRaises(ValueError):
            self.queue.complete(fresh, self.receipt(fresh, outcome="failed"))

    def test_receipts_bind_exact_candidate_and_require_complete_checks(self):
        self.queue.enqueue("job", spec())
        lease = self.queue.claim("job", "a")
        for update in [{"source_snapshot": "0" * 64}, {"action_key": "0" * 64},
                       {"checks_complete": False}, {"exit_code": 1}, {"evidence": []},
                       {"evidence": ["9" * 64]}, {"exit_code": True}]:
            with self.subTest(update=update), self.assertRaises((ValueError, OSError)):
                self.queue.complete(lease, self.receipt(lease, **update))
        self.assertEqual("running", self.queue.inspect("job")["state"])
        self.queue.complete(lease, self.receipt(lease))
        self.assertEqual("succeeded", self.queue.inspect("job")["state"])

    def test_failure_does_not_release_dependents(self):
        self.queue.enqueue("a", spec())
        self.queue.enqueue("b", spec(), dependencies=["a"])
        lease = self.queue.claim("a", "owner")
        self.queue.complete(lease, self.receipt(lease, outcome="failed", exit_code=0, checks_complete=False))
        with self.assertRaises(ValueError):
            self.queue.claim("b", "owner")

    def test_clock_regression_does_not_revive_an_expired_lease(self):
        self.queue.enqueue("a", spec())
        self.queue.enqueue("b", spec())
        lease = self.queue.claim("a", "a", 1)
        self.now += 2000
        self.queue.claim("b", "b")  # Commits the clock floor.
        self.now -= 5000
        with self.assertRaises(ValueError):
            self.queue.complete(lease, self.receipt(lease))

    def test_rejected_expiry_check_preserves_clock_floor_across_reopen(self):
        self.queue.enqueue("a", spec())
        lease = self.queue.claim("a", "a", 1)
        self.now += 2000
        with self.assertRaises(ValueError):
            self.queue.complete(lease, self.receipt(lease))
        self.now -= 5000
        other = WorkQueue(self.root / "work.sqlite", self.store, lambda: self.now)
        try:
            with self.assertRaises(ValueError):
                other.complete(lease, self.receipt(lease))
            with self.assertRaises(ValueError):
                other.renew(lease)
            self.assertEqual("running", other.inspect("a")["state"])
        finally:
            other.close()

    def test_renew_does_not_shorten_and_reopen_preserves_fence(self):
        self.queue.enqueue("a", spec())
        lease = self.queue.claim("a", "a", 20)
        renewed = self.queue.renew(lease, 1)
        self.assertEqual(lease.expires_ms, renewed.expires_ms)
        other = WorkQueue(self.root / "work.sqlite", self.store, lambda: self.now)
        try:
            with self.assertRaises(ValueError):
                other.claim("a", "other")
            other.complete(renewed, self.receipt(renewed))
        finally:
            other.close()
        self.assertEqual("succeeded", self.queue.inspect("a")["state"])

    def test_two_processes_cannot_claim_one_job(self):
        self.queue.enqueue("job", spec())
        context = multiprocessing.get_context("spawn")
        barrier, results = context.Barrier(2), context.Queue()
        processes = [context.Process(target=race_claim, args=(str(self.root / "work.sqlite"), str(self.root / "objects"), str(i), barrier, results)) for i in range(2)]
        for process in processes:
            process.start()
        records = [results.get(timeout=20) for _ in processes]
        for process in processes:
            process.join(timeout=20)
            self.assertEqual(0, process.exitcode)
        self.assertEqual(["claimed", "rejected"], sorted(r[0] for r in records))


class PlanningTests(unittest.TestCase):
    def test_reads_share_writes_conflict_unknown_serializes(self):
        changes = [
            {"id": "a", "reads": ["symbol:helper"], "writes": ["symbol:a"], "analysis_complete": True},
            {"id": "b", "reads": ["symbol:helper"], "writes": ["symbol:b"], "analysis_complete": True},
            {"id": "c", "reads": [], "writes": ["symbol:helper"], "analysis_complete": True},
            {"id": "d", "reads": [], "writes": [], "analysis_complete": False},
        ]
        self.assertEqual([["a", "b"], ["c"], ["d"]], plan_batches(changes))
        self.assertEqual(plan_batches(changes), plan_batches(list(reversed(changes))))

    def test_hierarchical_paths_and_cross_namespace_unknown_are_conservative(self):
        self.assertTrue(overlaps("path:src", "path:src/lib.rs"))
        self.assertFalse(overlaps("path:src", "path:src2/lib.rs"))
        self.assertTrue(overlaps("path:src/lib.rs", "symbol:foo"))
        self.assertTrue(overlaps("*", "symbol:foo"))
        with self.assertRaises(ValueError):
            overlaps("path:../src", "path:src")


if __name__ == "__main__":
    unittest.main()
