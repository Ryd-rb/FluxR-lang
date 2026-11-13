"""Compare the indexed planner with a deliberately simple pairwise reference."""
from __future__ import annotations

from pathlib import Path
import random
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline.planning import (MAX_RESOURCE_BYTES, MAX_RESOURCE_COMPONENTS,
                                     _ResourceIndex, change_conflicts, plan_batches, resource)


def reference(changes):
    batches = []
    for change in sorted(changes, key=lambda value: value["id"]):
        if not change["analysis_complete"]:
            batches.append([change])
            continue
        for batch in batches:
            if all(peer["analysis_complete"] and not change_conflicts(change, peer) for peer in batch):
                batch.append(change)
                break
        else:
            batches.append([change])
    return [[change["id"] for change in batch] for batch in batches]


class IndexedPlanningTests(unittest.TestCase):
    def test_seeded_differential_against_pairwise_algorithm(self):
        rng = random.Random(0xF104)
        names = ["path:src", "path:src/a", "path:src/a/deep", "path:src/b", "path:src2/a",
                 "path:tests", "symbol:a", "symbol:b", "contract:api", "contract:db", "*"]
        for trial in range(250):
            changes = [{"id": f"change-{i:03}", "reads": rng.sample(names, rng.randrange(4)),
                        "writes": rng.sample(names, rng.randrange(4)), "analysis_complete": rng.random() > .1}
                       for i in range(rng.randrange(40))]
            with self.subTest(trial=trial):
                expected = reference(changes)
                self.assertEqual(expected, plan_batches(changes))
                rng.shuffle(changes)
                self.assertEqual(expected, plan_batches(changes))

    def test_4096_independent_changes_share_one_batch(self):
        changes = [{"id": f"module-{i:05}", "reads": ["path:shared/api"],
                    "writes": [f"path:src/mod-{i}/body"], "analysis_complete": True} for i in range(4096)]
        self.assertEqual([[change["id"] for change in changes]], plan_batches(changes))

    def test_later_parent_and_wildcard_conflicts_remain_visible(self):
        changes = [
            {"id": "a", "reads": [], "writes": ["path:src/a/deep"], "analysis_complete": True},
            {"id": "b", "reads": [], "writes": ["path:src"], "analysis_complete": True},
            {"id": "c", "reads": ["*"], "writes": [], "analysis_complete": True},
            {"id": "d", "reads": [], "writes": ["symbol:elsewhere"], "analysis_complete": True},
        ]
        self.assertEqual([["a"], ["b"], ["c"], ["d"]], plan_batches(changes))

    def test_bad_outer_shapes_are_explicit_errors(self):
        for changes in [None, {}, "changes", [None], ["bad"]]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                plan_batches(changes)

    def test_resource_depth_and_encoded_bytes_are_bounded(self):
        invalid = ["path:" + "x" * MAX_RESOURCE_BYTES,
                   "symbol:" + "🙂" * (MAX_RESOURCE_BYTES // 4),
                   "path:" + "/".join(["a"] * (MAX_RESOURCE_COMPONENTS + 1))]
        for value in invalid:
            with self.subTest(value=value[:40]), self.assertRaisesRegex(ValueError, "limit"):
                resource(value)
        self.assertEqual("path:" + "x" * (MAX_RESOURCE_BYTES - 5),
                         resource("path:" + "x" * (MAX_RESOURCE_BYTES - 5)))

    def test_deep_paths_use_component_trie_and_ancestor_pruning(self):
        index = _ResourceIndex()
        deep = "path:" + "/".join(f"component-{i}" for i in range(MAX_RESOURCE_COMPONENTS))
        resource(deep)
        index.add([deep])
        self.assertTrue(index.conflicts(["path:component-0"]))
        self.assertTrue(index.conflicts([deep]))
        self.assertFalse(index.conflicts(["path:component-0/other"]))
        index.add(["path:component-0"])
        self.assertTrue(index.conflicts([deep]))
        self.assertEqual({None: True}, index.path_tree["component-0"])

    def test_caller_mutation_cannot_change_validated_or_indexed_resources(self):
        changes = [
            {"id": "a", "reads": [], "writes": ["path:a"], "analysis_complete": True},
            {"id": "b", "reads": [], "writes": ["path:b"], "analysis_complete": True},
        ]
        def mutate_external(value):
            result = resource(value)
            if value == "path:b":
                changes[0]["writes"] = ["path:../invalid-after-validation"]
                changes[1]["analysis_complete"] = False
            return result
        with patch("agent_pipeline.planning.resource", side_effect=mutate_external):
            self.assertEqual([["a", "b"]], plan_batches(changes))


if __name__ == "__main__":
    unittest.main()
