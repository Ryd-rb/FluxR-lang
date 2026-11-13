"""Impact selection against a forward oracle and adversarial ledger cases."""
from copy import deepcopy
from pathlib import Path
import random
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline.impact import Graph, collect_results, plan_shards, select_tests
from agent_pipeline.identity import content_digest


def h(value):
    return content_digest(str(value).encode())


def node(name, dependencies=(), kind="input"):
    return {"kind": kind, "fingerprint": h(name), "dependencies": sorted(dependencies)}


def graph():
    return {"schema": "fluxr-test-graph-v1", "snapshot": h("base"), "analyzer": h("analyzer"),
            "policy": h("policy"), "complete": True,
            "nodes": {"src:a": node("a"), "src:b": node("b"),
                      "lookup:optional": node("absent"),
                      "query:a": node("query:a", ["src:a", "lookup:optional"], "query"),
                      "test:a": node("test:a", ["query:a"], "test"),
                      "test:b": node("test:b", ["src:b"], "test")}}


def select(before, after, catalog=None):
    if catalog is None:
        catalog = {name: value["fingerprint"] for name, value in after["nodes"].items()
                   if value["kind"] == "test"}
    return select_tests(catalog, before, after, baseline_snapshot=h("base"),
                        candidate_snapshot=h("candidate"), analyzer=h("analyzer"), policy=h("policy"))


def candidate(before):
    after = deepcopy(before)
    after["snapshot"] = h("candidate")
    return after


def results(plan):
    return [{"schema": "fluxr-test-result-v1", "plan_key": plan["plan_key"],
             "candidate_snapshot": plan["candidate_snapshot"], "shard": shard,
             "test": test, "action_key": plan["catalog"][test], "terminal": "passed", "exit_code": 0,
             "evidence": [h("log:" + test)]}
            for shard, tests in enumerate(plan["shards"]) for test in tests]


def isolation():
    return {"schema": "fluxr-isolated-tests-v1", "runner": h("runner"),
            "policy": h("policy"), "qualification": h("trusted isolation tests")}


class SelectionTests(unittest.TestCase):
    def test_transitive_and_negative_lookup_inputs(self):
        before = graph()
        for changed in ("src:a", "lookup:optional"):
            after = candidate(before)
            after["nodes"][changed]["fingerprint"] = h("new")
            selected = select(before, after)
            self.assertEqual(["test:a"], selected["selected"])
            self.assertEqual(["test:b"], selected["omitted"])
            self.assertEqual("incremental", selected["mode"])

    def test_deleted_edges_and_nodes_invalidate_old_dependents(self):
        before = graph()
        after = candidate(before)
        del after["nodes"]["src:a"]
        after["nodes"]["query:a"]["dependencies"].remove("src:a")
        self.assertEqual(["test:a"], select(before, after)["selected"])

    def test_new_tests_and_actions_are_always_selected(self):
        before = graph()
        after = candidate(before)
        after["nodes"]["test:new"] = node("new", kind="test")
        after["nodes"]["test:b"]["fingerprint"] = h("new compiler flags")
        self.assertEqual(["test:b", "test:new"], select(before, after)["selected"])

    def test_recursion_terminates_and_invalidates_cycle_users(self):
        before = graph()
        before["nodes"]["query:recursive"] = node("r", ["query:a"], "query")
        before["nodes"]["query:a"]["dependencies"].append("query:recursive")
        before["nodes"]["query:a"]["dependencies"].sort()
        after = candidate(before)
        after["nodes"]["src:a"]["fingerprint"] = h("changed")
        self.assertEqual(["test:a"], select(before, after)["selected"])

    def test_unchanged_test_prerequisite_is_included(self):
        before = graph()
        before["nodes"]["test:b"]["dependencies"].append("test:a")
        after = candidate(before)
        after["nodes"]["test:b"]["fingerprint"] = h("changed test")
        self.assertEqual(["test:a", "test:b"], select(before, after)["selected"])

    def test_incomplete_untrusted_or_mismatched_graphs_fall_back(self):
        before = graph()
        original = candidate(before)
        catalog = {x: original["nodes"][x]["fingerprint"] for x in ("test:a", "test:b")}
        changes = [("complete", False, "incomplete_graph"), ("complete", 1, "invalid_graph"),
                   ("snapshot", h("wrong"), "snapshot_mismatch"),
                   ("analyzer", h("unknown"), "analysis_identity_mismatch"),
                   ("policy", h("old"), "analysis_identity_mismatch")]
        for key, value, reason in changes:
            after = deepcopy(original)
            after[key] = value
            with self.subTest(key=key, value=value):
                chosen = select(before, after, catalog)
                self.assertEqual(reason, chosen["reason"])
                self.assertEqual(sorted(catalog), chosen["selected"])
        self.assertEqual("missing_graph", select(None, original, catalog)["reason"])
        after = deepcopy(original)
        del after["nodes"]["test:b"]
        self.assertEqual("catalog_mismatch", select(before, after, catalog)["reason"])
        after = deepcopy(original)
        after["nodes"]["query:a"]["dependencies"].append("unknown")
        self.assertEqual("invalid_graph", select(before, after, catalog)["reason"])

    def test_parsed_graph_is_detached_from_mutable_input(self):
        raw = graph()
        parsed = Graph.parse(raw)
        raw["nodes"]["query:a"]["dependencies"].clear()
        self.assertTrue(parsed.nodes["query:a"].dependencies)
        with self.assertRaises(TypeError):
            parsed.nodes["new"] = parsed.nodes["src:a"]

    def test_property_reverse_selection_matches_independent_forward_search(self):
        rng = random.Random(47971)
        for case in range(150):
            before = graph()
            names = [f"n:{n:02}" for n in range(20)] + [f"test:{n}" for n in range(5)]
            before["nodes"] = {name: node(name, sorted(rng.sample(names[:20], rng.randrange(5))),
                                          "test" if name.startswith("test:") else "query") for name in names}
            after = candidate(before)
            for name in rng.sample(names, 3):
                after["nodes"][name]["fingerprint"] = h(f"change:{case}:{name}")
            after["nodes"][rng.choice(names)]["dependencies"] = sorted(rng.sample(names[:20], 2))
            changed = {n for n in names if before["nodes"][n] != after["nodes"][n]}
            expected = []
            for test in names[-5:]:
                visited, stack = set(), [test]
                while stack:
                    current = stack.pop()
                    if current in visited:
                        continue
                    visited.add(current)
                    for g in (before, after):
                        stack.extend(g["nodes"][current]["dependencies"])
                if changed & visited:
                    expected.append(test)
            self.assertEqual(sorted(expected), select(before, after)["selected"], case)


class ShardTests(unittest.TestCase):
    def setUp(self):
        self.before = graph()
        self.after = candidate(self.before)
        self.catalog = {name: value["fingerprint"] for name, value in self.after["nodes"].items()
                        if value["kind"] == "test"}
        self.selection = select(None, self.after, self.catalog)
        self.plan = plan_shards(self.selection, self.catalog, 2, isolation=isolation())

    def test_every_selected_test_appears_once_and_order_is_stable(self):
        reverse_catalog = dict(reversed(list(self.catalog.items())))
        self.assertEqual(self.plan, plan_shards(self.selection, reverse_catalog, 2, isolation=isolation()))
        flattened = sum(self.plan["shards"], [])
        self.assertEqual(sorted(self.catalog), sorted(flattened))
        self.assertEqual(len(flattened), len(set(flattened)))
        self.assertEqual(2, len(plan_shards(self.selection, self.catalog, 100, isolation=isolation())["shards"]))

    def test_parallel_shards_require_explicit_isolation_qualification(self):
        with self.assertRaisesRegex(ValueError, "explicit qualified"):
            plan_shards(self.selection, self.catalog, 2)
        plan = plan_shards(self.selection, self.catalog, 100, test_order=["test:b", "test:a"])
        self.assertEqual([["test:b", "test:a"]], plan["shards"])
        self.assertEqual("passed", collect_results(plan, results(plan))["status"])
        wrong = isolation()
        wrong["policy"] = h("wrong")
        with self.assertRaises(ValueError):
            plan_shards(self.selection, self.catalog, 2, isolation=wrong)

    def test_estimates_only_change_packing_not_test_coverage(self):
        plan = plan_shards(self.selection, self.catalog, 1, estimated_ms={"test:b": 100}, test_order=list(self.catalog))
        self.assertEqual(sorted(self.catalog), sum(plan["shards"], []))
        for value in (0, -1, True, 1.5, 2**63):
            with self.subTest(value=value), self.assertRaises(ValueError):
                plan_shards(self.selection, self.catalog, 2, estimated_ms={"test:a": value})

    def test_tampering_catalog_and_partition_rejected(self):
        selection = deepcopy(self.selection)
        selection["selected"] = []
        with self.assertRaises(ValueError):
            plan_shards(selection, self.catalog, 2)
        changed = {**self.catalog, "test:new": h("new")}
        with self.assertRaises(ValueError):
            plan_shards(self.selection, changed, 2)
        broken = deepcopy(self.plan)
        broken["shards"][0] = []
        with self.assertRaises(ValueError):
            collect_results(broken, [])

    def test_pass_requires_every_expected_result(self):
        records = results(self.plan)
        self.assertEqual("passed", collect_results(self.plan, records)["status"])
        result = collect_results(self.plan, records[:1])
        self.assertEqual("incomplete", result["status"])
        self.assertEqual(1, len(result["missing"]))

    def test_duplicate_extra_wrong_subject_or_action_never_pass(self):
        records = results(self.plan)
        self.assertEqual("invalid", collect_results(self.plan, records + records[:1])["status"])
        for key, value in (("candidate_snapshot", h("wrong")), ("plan_key", h("old")),
                           ("action_key", h("old")), ("shard", True), ("test", "unexpected"),
                           ("exit_code", True), ("exit_code", 1), ("evidence", []),
                           ("terminal", "skipped")):
            changed = deepcopy(records)
            changed[0][key] = value
            with self.subTest(key=key, value=value):
                self.assertEqual("invalid", collect_results(self.plan, changed)["status"])

    def test_failure_timeout_cancel_and_infrastructure_are_distinct_rows(self):
        for terminal in ("failed", "timeout", "cancelled", "infrastructure_error"):
            records = results(self.plan)
            records[0].update(terminal=terminal, exit_code=None)
            with self.subTest(terminal=terminal):
                outcome = collect_results(self.plan, records)
                self.assertEqual("failed", outcome["status"])
                self.assertEqual(1, len(outcome["failed"]))

    def test_unchanged_selection_is_not_executed_test_evidence(self):
        selection = select(self.before, self.after)
        plan = plan_shards(selection, self.catalog, 4, test_order=list(self.catalog))
        self.assertEqual([], plan["shards"])
        outcome = collect_results(plan, [])
        self.assertEqual("no_tests_selected", outcome["status"])
        self.assertEqual(sorted(self.catalog), outcome["omitted"])


class BoundaryIsolationTests(unittest.TestCase):
    def test_caller_mutation_cannot_shrink_a_validated_ledger_denominator(self):
        import agent_pipeline.impact as impact
        before = graph()
        after = candidate(before)
        catalog = {name: value["fingerprint"] for name, value in after["nodes"].items()
                   if value["kind"] == "test"}
        selection = select(None, after, catalog)
        plan = plan_shards(selection, catalog, 1, test_order=list(catalog))
        records = results(plan)[:1]
        original_key = plan["plan_key"]
        serialize = impact.canonical_bytes
        def mutate_after_serialization(value):
            encoded = serialize(value)
            if value is records:
                plan["shards"] = [["test:a"]]
                plan["selected"] = ["test:a"]
                plan["omitted"] = ["test:b"]
            return encoded
        with patch.object(impact, "canonical_bytes", side_effect=mutate_after_serialization):
            outcome = collect_results(plan, records)
        self.assertEqual("incomplete", outcome["status"])
        self.assertEqual(2, outcome["expected"])
        self.assertEqual(["test:b"], outcome["missing"])
        self.assertEqual([], outcome["omitted"])
        self.assertEqual(original_key, outcome["plan_key"])

    def test_selection_mutation_during_snapshot_cannot_change_shards(self):
        import agent_pipeline.impact as impact
        before = graph()
        after = candidate(before)
        catalog = {name: value["fingerprint"] for name, value in after["nodes"].items()
                   if value["kind"] == "test"}
        selection = select(None, after, catalog)
        serialize = impact.canonical_bytes
        def mutate_after_serialization(value):
            encoded = serialize(value)
            if value is selection:
                selection["selected"].clear()
                selection["omitted"] = sorted(catalog)
            return encoded
        with patch.object(impact, "canonical_bytes", side_effect=mutate_after_serialization):
            plan = plan_shards(selection, catalog, 1, test_order=list(catalog))
        self.assertEqual([["test:a", "test:b"]], plan["shards"])
        self.assertEqual([], plan["omitted"])


if __name__ == "__main__":
    unittest.main()
