"""Adversarial interface/document identity, migration and composition coverage."""
from pathlib import Path
import copy
import itertools
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import contracts as v1
from agent_pipeline import contracts_v2 as v2
from agent_pipeline.identity import canonical_bytes, load_json_bytes


def fragment(key, requires=None):
    return {"schema": v2.SCHEMA, "key": key, "requires": requires or {},
            "exports": {key + "/Value": {"kind": "opaque", "layout": "abi-v1"}},
            "invariants": {"stable": "Keep ownership and first-trap order."},
            "documentation": {"summary": "Initial documentation."}}


def edit(old, new):
    return {"key": (new or old)["key"],
            "expected": v2.fragment_digest(old) if old else None, "replacement": new}


def legacy(key, requires=None):
    value = fragment(key, requires)
    del value["documentation"]
    value["schema"] = v1.SCHEMA
    return value


class ContractInterfaceTests(unittest.TestCase):
    def test_released_v1_identity_vectors_and_v2_domain_separation(self):
        original = {"schema": v1.SCHEMA, "key": "core", "requires": {},
                    "exports": {"core/Value": {"kind": "opaque", "description": "normative"}},
                    "invariants": {"stable": "Stable exact representation."}}
        self.assertEqual(v1.fragment_digest(original),
                         "debc1c288ecf549e8fc0c518d41ad7d930653d17de38164ba88f3d5a577828ff")
        self.assertEqual(v1.Catalog([original]).revision,
                         "9b83e76044f7643040e95046d49a8f7fd93812ea0aef4727580b22358bd1d538")
        current = {**original, "schema": v2.SCHEMA,
                   "documentation": {"summary": "Agent documentation."}}
        ids = v2.fragment_identities(current)
        self.assertEqual(ids, {
            "fragment": "d6e5b9cbb8f7aa6eb1465e494e550301446381e5d81bb0e963fbb3f283e3ab2e",
            "interface": "8d55bd24210eee765d0a7585c469f7add45feab649d781c8f1edf77b54d90c2c",
            "documentation": "216a25bc9ed9e4892b53423f33126465c5f9f3cbf6aa7f42772bb0f67af6cfcc"})
        self.assertEqual(len(set(ids.values()) | {v1.fragment_digest(original)}), 4)
        with self.assertRaises(ValueError):
            v1.Catalog([current])
        with self.assertRaises(ValueError):
            v2.Catalog([original])

    def test_documentation_edit_preserves_all_transitive_interface_pins(self):
        a = fragment("a")
        b = fragment("b", {"a": v2.interface_digest(a)})
        c = fragment("c", {"b": v2.interface_digest(b)})
        before = v2.Catalog([a, b, c])
        amended = copy.deepcopy(a)
        amended["documentation"]["summary"] = "New documentation with Ω and \n escapes."
        after = before.compose([edit(a, amended)])
        self.assertEqual(before.interface_revision, after.interface_revision)
        self.assertNotEqual(before.revision, after.revision)
        self.assertEqual(after.fragment("b"), b)
        self.assertEqual(after.fragment("c"), c)
        self.assertEqual(before.delta(after)["documentation_only"], ["a"])
        self.assertEqual(before.delta(after)["interface_changed"], [])

    def test_stale_document_guard_cannot_overwrite_a_concurrent_document_edit(self):
        original = fragment("a")
        a, b = copy.deepcopy(original), copy.deepcopy(original)
        a["documentation"]["summary"] = "Agent one"
        b["documentation"]["summary"] = "Agent two"
        current = v2.Catalog([original]).compose([edit(original, a)])
        with self.assertRaisesRegex(ValueError, "stale contract edit"):
            current.compose([edit(original, b)])
        guard = edit(a, b)
        guard["expected"] = v2.interface_digest(a)
        with self.assertRaisesRegex(ValueError, "stale contract edit"):
            current.compose([guard])
        self.assertEqual(current.fragment("a"), a)

    def test_every_interface_field_is_normative_including_description_defaults_and_effects(self):
        original = fragment("a")
        old = v2.interface_digest(original)
        for field, value in (("description", "Narrative may be normative"), ("default", 0),
                             ("effects", ["file-read"]), ("layout", "abi-v2"),
                             ("ownership", "shared"), ("numeric_policy", "fast")):
            changed = copy.deepcopy(original)
            changed["exports"]["a/Value"][field] = value
            self.assertNotEqual(v2.interface_digest(changed), old, field)
        changed = copy.deepcopy(original)
        changed["invariants"]["stable"] = "Remove a constraint"
        self.assertNotEqual(v2.interface_digest(changed), old)

    def test_semantic_edits_require_explicit_transitive_repin_in_one_candidate(self):
        a = fragment("a")
        b = fragment("b", {"a": v2.interface_digest(a)})
        c = fragment("c", {"b": v2.interface_digest(b)})
        base = v2.Catalog([a, b, c])
        a2, b2, c2 = copy.deepcopy(a), copy.deepcopy(b), copy.deepcopy(c)
        a2["invariants"]["stable"] += " New required behavior."
        b2["requires"]["a"] = v2.interface_digest(a2)
        c2["requires"]["b"] = v2.interface_digest(b2)
        for changes in ([edit(a, a2)], [edit(a, a2), edit(b, b2)]):
            with self.assertRaisesRegex(ValueError, "stale interface pin"):
                base.compose(changes)
        together = base.compose([edit(a, a2), edit(b, b2), edit(c, c2)])
        self.assertEqual(base.delta(together)["interface_changed"], ["a", "b", "c"])
        self.assertNotEqual(base.interface_revision, together.interface_revision)
        self.assertEqual(base.fragment("a"), a)

    def test_all_edit_orders_and_input_orders_produce_same_candidate(self):
        values = [fragment(key) for key in ["a", "b", "c"]]
        revisions = {v2.Catalog(items).revision for items in itertools.permutations(values)}
        self.assertEqual(len(revisions), 1)
        base = v2.Catalog(values)
        changes = []
        for value in values:
            newer = copy.deepcopy(value)
            newer["documentation"]["summary"] = "Revised " + value["key"]
            changes.append(edit(value, newer))
        expected = base.compose(changes).revision
        for ordered in itertools.permutations(changes):
            self.assertEqual(base.compose(ordered).revision, expected)
            current = base
            for change in ordered:
                current = current.compose([change])
            self.assertEqual(current.revision, expected)

    def test_whole_candidate_export_collisions_and_dangling_deletions_fail(self):
        a = fragment("a")
        b = fragment("b", {"a": v2.interface_digest(a)})
        base = v2.Catalog([a, b])
        with self.assertRaisesRegex(ValueError, "stale interface pin"):
            base.compose([edit(a, None)])
        c = fragment("c")
        c["exports"] = copy.deepcopy(a["exports"])
        with self.assertRaisesRegex(ValueError, "export collision"):
            base.compose([edit(None, c)])
        self.assertEqual(base.compose([edit(a, None), edit(b, None)]).manifest()["fragments"], {})

    def test_exact_repeated_targets_missing_guards_and_changed_target_refuse(self):
        a = fragment("a")
        base = v2.Catalog([a])
        cases = [[edit(a, a), edit(a, a)], [edit(None, a)],
                 [{"key": "a", "expected": v2.fragment_digest(a), "replacement": fragment("b")}],
                 [{"key": "missing", "expected": None, "replacement": None}],
                 [{"key": "a", "expected": v2.fragment_digest(a), "replacement": a, "force": True}]]
        for changes in cases:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                base.compose(changes)

    def test_unknown_fields_and_invalid_document_values_refuse(self):
        original = fragment("a")
        cases = []
        for key in original:
            changed = copy.deepcopy(original)
            del changed[key]
            cases.append(changed)
        cases.extend([{**original, "semantic_override": {}},
                      {**original, "schema": "fluxr-contract-fragment-v3"},
                      {**original, "documentation": []},
                      {**original, "documentation": {"summary": {"must": "obey"}}},
                      {**original, "documentation": {"../bad": "text"}}])
        for value in cases:
            with self.subTest(value=value), self.assertRaises(ValueError):
                v2.Catalog([value])

    def test_input_and_output_aliases_cannot_rewrite_retained_content(self):
        a = fragment("a")
        catalog = v2.Catalog([a])
        saved = catalog.manifest()
        a["documentation"].clear()
        catalog.fragment("a")["exports"].clear()
        catalog.manifest()["fragments"].clear()
        self.assertEqual(catalog.manifest(), saved)
        changes = []
        b = fragment("b")
        def stream():
            yield edit(None, b)
            b["documentation"]["summary"] = "Mutated after yield"
            yield edit(None, fragment("c"))
        result = catalog.compose(stream())
        self.assertEqual(result.fragment("b")["documentation"]["summary"], "Initial documentation.")

    def test_fragment_catalog_and_edit_budgets_reject_before_acceptance(self):
        a = fragment("a")
        with patch.object(v2, "MAX_FRAGMENT_BYTES", 10):
            with self.assertRaisesRegex(ValueError, "byte budget"):
                v2.Catalog([a])
        with patch.object(v2, "MAX_TOTAL_BYTES", 10):
            with self.assertRaisesRegex(ValueError, "aggregate budget"):
                v2.Catalog([a])
        with patch.object(v2, "MAX_FRAGMENTS", 1):
            with self.assertRaisesRegex(ValueError, "too many"):
                v2.Catalog([a, fragment("b")])
            base = v2.Catalog([a])
            with self.assertRaisesRegex(ValueError, "too many"):
                base.compose([edit(None, fragment("b")), edit(None, fragment("c"))])
        base = v2.Catalog([a])
        with patch.object(v2, "MAX_TOTAL_BYTES", 10):
            with self.assertRaisesRegex(ValueError, "aggregate budget"):
                base.compose([edit(a, a)])
        b = fragment("b", {"a": v2.interface_digest(a)})
        with patch.object(v2, "MAX_EDGES", 0):
            with self.assertRaisesRegex(ValueError, "aggregate budget"):
                v2.Catalog([a, b])
        with patch.object(v2, "MAX_EXPORTS", 0):
            with self.assertRaisesRegex(ValueError, "aggregate budget"):
                v2.Catalog([a])

    def test_bounded_generator_consumption(self):
        consumed = []
        def stream():
            for key in ("a", "b", "c", "d"):
                consumed.append(key)
                yield fragment(key)
        with patch.object(v2, "MAX_FRAGMENTS", 2):
            with self.assertRaisesRegex(ValueError, "too many"):
                v2.Catalog(stream())
        self.assertEqual(consumed, ["a", "b", "c"])

    def test_migration_retains_exact_v1_payloads_and_restores_original_catalog(self):
        a = legacy("a")
        a["exports"]["a/Value"]["description"] = "Never silently discard this"
        b = legacy("b", {"a": v1.fragment_digest(a)})
        c = legacy("c", {"b": v1.fragment_digest(b)})
        old = v1.Catalog([c, b, a])
        migration = v2.migrate_v1(old)
        new = migration.catalog
        self.assertEqual(migration.restore().manifest(), old.manifest())
        self.assertEqual(new.fragment("a")["exports"], a["exports"])
        self.assertEqual(new.fragment("a")["invariants"], a["invariants"])
        self.assertEqual(new.fragment("a")["documentation"], {})
        self.assertEqual(new.fragment("b")["requires"]["a"], v2.interface_digest(new.fragment("a")))
        for value in (a, b, c):
            self.assertEqual(canonical_bytes(migration.original_fragment(value["key"])), canonical_bytes(value))
        receipt = migration.receipt()
        self.assertEqual(receipt["source_catalog"], old.revision)
        self.assertEqual(receipt["result_catalog"], new.revision)
        receipt["mapping"].clear()
        migration.original_fragment("a")["exports"].clear()
        self.assertEqual(migration.restore().manifest(), old.manifest())
        self.assertEqual(len(migration.receipt()["mapping"]), 3)

    def test_migration_is_explicit_and_order_deterministic(self):
        a = legacy("a")
        b = legacy("b", {"a": v1.fragment_digest(a)})
        c = legacy("c", {"a": v1.fragment_digest(a)})
        expected = v2.migrate_v1(v1.Catalog([a, b, c])).receipt()
        for values in itertools.permutations([a, b, c]):
            self.assertEqual(v2.migrate_v1(v1.Catalog(values)).receipt(), expected)
        with self.assertRaisesRegex(ValueError, "validated v1"):
            v2.migrate_v1([a, b, c])
        with self.assertRaisesRegex(ValueError, "validated v1"):
            v2.migrate_v1(v2.Catalog([]))
        self.assertEqual(v2.migrate_v1(v1.Catalog([])).restore().revision, v1.Catalog([]).revision)

    def test_long_migration_chain_uses_bounded_iterative_dependencies(self):
        values, previous = [], None
        for index in range(150):
            value = legacy("n" + str(index), {} if previous is None else
                           {previous["key"]: v1.fragment_digest(previous)})
            values.append(value)
            previous = value
        old = v1.Catalog(reversed(values))
        migration = v2.migrate_v1(old)
        self.assertEqual(len(migration.catalog.manifest()["fragments"]), 150)
        self.assertEqual(migration.restore().revision, old.revision)

    def test_cli_executes_validates_and_rejects_duplicate_keys_and_links(self):
        root = Path(__file__).resolve().parents[1]
        environment = {**os.environ, "PYTHONPATH": str(root / "tools")}
        with tempfile.TemporaryDirectory(prefix="fluxr-contract-v2-") as raw:
            path = Path(raw) / "contract.json"
            value = fragment("a")
            path.write_bytes(canonical_bytes(value))
            command = [sys.executable, "-m", "agent_pipeline.contracts_v2", str(path)]
            result = subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), v2.Catalog([value]).manifest())
            path.write_text('{"schema":"first","schema":"second"}')
            result = subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("duplicate JSON key", result.stderr)
            path.unlink()
            try:
                path.symlink_to(root / "Cargo.toml")
            except (OSError, NotImplementedError):
                return  # Symlink creation is optional; CLI checks above still ran.
            result = subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("symlink", result.stderr)


if __name__ == "__main__":
    unittest.main()
