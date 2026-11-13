"""Fault and algebraic cases for independently authored design fragments."""
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
from agent_pipeline.contracts import Catalog, SCHEMA, fragment_digest
from agent_pipeline import contracts


def fragment(key, requires=None):
    return {"schema": SCHEMA, "key": key, "requires": requires or {},
            "exports": {key + "/Value": {"kind": "opaque", "profile": "proposal-v1"}},
            "invariants": {"value-stable": "The descriptor is not a semantic proof."}}


def edit(value, expected=None):
    return {"key": value["key"], "expected": expected, "replacement": value}


class ContractTests(unittest.TestCase):
    def test_permutation_identity_and_independent_edits_commute(self):
        items = [fragment(key) for key in ("a", "b", "c", "d")]
        expected = Catalog(items).manifest()
        for permutation in itertools.permutations(items):
            self.assertEqual(Catalog(permutation).manifest(), expected)
        empty = Catalog([])
        a, b = items[:2]
        together = empty.compose([edit(a), edit(b)])
        self.assertEqual(together.revision, empty.compose([edit(a)]).compose([edit(b)]).revision)
        self.assertEqual(together.revision, empty.compose([edit(b)]).compose([edit(a)]).revision)
        self.assertEqual(empty.manifest()["fragments"], {})

    def test_mutable_inputs_and_outputs_cannot_change_frozen_identity(self):
        value = fragment("a")
        catalog = Catalog([value])
        before = catalog.revision
        value["exports"]["a/Value"]["profile"] = "mutated"
        catalog.fragment("a")["invariants"].clear()
        catalog.manifest()["fragments"].clear()
        self.assertEqual(catalog.revision, before)
        self.assertEqual(catalog.fragment("a")["exports"]["a/Value"]["profile"], "proposal-v1")

    def test_provider_edit_requires_all_consumers_to_repin_explicitly(self):
        provider = fragment("provider")
        consumer = fragment("consumer", {"provider": fragment_digest(provider)})
        initial = Catalog([provider, consumer])
        replacement = copy.deepcopy(provider)
        replacement["invariants"]["new-rule"] = "Different requirement."
        with self.assertRaisesRegex(ValueError, "stale contract pin"):
            initial.compose([edit(replacement, fragment_digest(provider))])
        revised_consumer = copy.deepcopy(consumer)
        revised_consumer["requires"]["provider"] = fragment_digest(replacement)
        revised = initial.compose([edit(revised_consumer, fragment_digest(consumer)),
                                   edit(replacement, fragment_digest(provider))])
        self.assertNotEqual(initial.revision, revised.revision)
        self.assertEqual(initial.fragment("provider"), provider)

    def test_deletion_rejects_dangling_pin_and_batch_deletion_is_atomic(self):
        a = fragment("a")
        b = fragment("b", {"a": fragment_digest(a)})
        current = Catalog([a, b])
        remove = lambda value: {"key": value["key"], "expected": fragment_digest(value), "replacement": None}
        with self.assertRaisesRegex(ValueError, "stale contract pin"):
            current.compose([remove(a)])
        self.assertEqual(current.compose([remove(a), remove(b)]).revision, Catalog([]).revision)
        self.assertEqual(len(current.manifest()["fragments"]), 2)

    def test_stale_edit_cannot_overwrite_concurrent_work(self):
        a = fragment("a")
        base = Catalog([a])
        newer = copy.deepcopy(a)
        newer["invariants"]["changed"] = "A concurrent edit."
        current = base.compose([edit(newer, fragment_digest(a))])
        with self.assertRaisesRegex(ValueError, "stale contract edit"):
            current.compose([edit(a, fragment_digest(a))])
        self.assertEqual(current.fragment("a"), newer)

    def test_collision_duplicate_target_and_key_mismatch_refuse(self):
        a, b = fragment("a"), fragment("b")
        b["exports"] = copy.deepcopy(a["exports"])
        with self.assertRaisesRegex(ValueError, "export collision"):
            Catalog([a, b])
        with self.assertRaisesRegex(ValueError, "duplicate contract key"):
            Catalog([a, a])
        with self.assertRaisesRegex(ValueError, "duplicate contract edit"):
            Catalog([]).compose([edit(a), edit(a)])
        with self.assertRaisesRegex(ValueError, "replacement key"):
            Catalog([]).compose([{**edit(a), "key": "b"}])
        with self.assertRaisesRegex(ValueError, "absent"):
            Catalog([]).compose([{"key": "a", "expected": None, "replacement": None}])

    def test_missing_imports_and_self_pins_refuse(self):
        with self.assertRaisesRegex(ValueError, "missing or stale"):
            Catalog([fragment("a", {"b": "0" * 64})])
        with self.assertRaisesRegex(ValueError, "recursive"):
            Catalog([fragment("a", {"a": "0" * 64})])

    def test_total_input_and_graph_work_are_bounded(self):
        with patch.object(contracts, "MAX_FRAGMENTS", 3):
            with self.assertRaisesRegex(ValueError, "too many"):
                Catalog(fragment(f"item{i}") for i in itertools.count())
        with patch.object(contracts, "MAX_TOTAL_BYTES", 1):
            with self.assertRaisesRegex(ValueError, "aggregate"):
                Catalog([fragment("a")])
        with patch.object(contracts, "MAX_FRAGMENT_BYTES", 1):
            with self.assertRaisesRegex(ValueError, "byte budget"):
                fragment_digest(fragment("a"))
        with patch.object(contracts, "MAX_EXPORTS", 1):
            with self.assertRaisesRegex(ValueError, "aggregate"):
                Catalog([fragment("a"), fragment("b")])

    def test_unknown_fields_wrong_types_and_bad_names_refuse(self):
        bad = [None, [], {**fragment("a"), "approval": True}, fragment("../a"),
               {**fragment("a"), "exports": {}}, {**fragment("a"), "requires": []},
               {**fragment("a"), "invariants": {"r": False}}]
        for value in bad:
            with self.subTest(value=value), self.assertRaises(ValueError):
                Catalog([value])

    def test_cli_detaches_bytes_rejects_duplicate_json_and_does_not_execute_descriptors(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "a.json"
            source.write_text(json.dumps(fragment("a")))
            environment = {**os.environ, "PYTHONPATH": str(Path(contracts.__file__).parents[1])}
            command = [sys.executable, "-m", "agent_pipeline.contracts", str(source)]
            result = subprocess.run(command, env=environment, capture_output=True, check=True)
            self.assertEqual(json.loads(result.stdout), Catalog([fragment("a")]).manifest())
            source.write_text('{"schema":"x","schema":"y"}')
            result = subprocess.run(command, env=environment, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"duplicate JSON", result.stderr)


if __name__ == "__main__":
    unittest.main()
