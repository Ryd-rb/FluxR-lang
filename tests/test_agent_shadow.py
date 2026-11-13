"""Independent full/selected run observations and conservative shadow failures."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import impact, shadow
from agent_pipeline.identity import content_digest, canonical_bytes


def h(value):
    return content_digest(canonical_bytes(value))


def context():
    return {key: h(key) for key in shadow.CONTEXT_FIELDS}


def graph(snapshot, changed=False, missing_edge=False):
    return {"schema": "fluxr-test-graph-v1", "snapshot": snapshot,
            "analyzer": context()["analyzer"], "policy": context()["policy"], "complete": True,
            "nodes": {
                "input:a": {"kind": "input", "fingerprint": h("new" if changed else "old"), "dependencies": []},
                "input:b": {"kind": "input", "fingerprint": h("stable"), "dependencies": []},
                "test:a": {"kind": "test", "fingerprint": h("test:a"), "dependencies": ["input:a"]},
                "test:b": {"kind": "test", "fingerprint": h("test:b"), "dependencies": [] if missing_edge else ["input:b"]}}}


def run(plan, attempt, *, changes=None, terminals=None):
    changes, terminals = changes or {}, terminals or {}
    observations, results = {}, []
    for shard, tests in enumerate(plan["shards"]):
        for test in tests:
            terminal = terminals.get(test, "passed")
            observations[test] = h(changes.get(test, "observed:" + test))
            results.append({"schema": "fluxr-test-result-v1", "plan_key": plan["plan_key"],
                            "candidate_snapshot": plan["candidate_snapshot"], "shard": shard,
                            "test": test, "action_key": plan["catalog"][test], "terminal": terminal,
                            "exit_code": 0 if terminal == "passed" else 1,
                            "evidence": [h({"attempt": attempt, "test": test, "terminal": terminal})]})
    return {"schema": "fluxr-test-shadow-run-v1", "attempt": attempt, "context": context(),
            "results": results, "observations": observations}


def fixture():
    before, after = graph(h("base")), graph(h("candidate"), True)
    catalog = {test: after["nodes"][test]["fingerprint"] for test in ("test:a", "test:b")}
    select = lambda old, new: impact.select_tests(catalog, old, new,
                   baseline_snapshot=h("base"), candidate_snapshot=h("candidate"),
                   analyzer=context()["analyzer"], policy=context()["policy"])
    selected = select(before, after)
    full = select(None, after)
    base_selection = impact.select_tests(catalog, None, before,
                   baseline_snapshot=h("base"), candidate_snapshot=h("base"),
                   analyzer=context()["analyzer"], policy=context()["policy"])
    plans = {"baseline": impact.plan_shards(base_selection, catalog, 1, test_order=sorted(catalog)),
             "full": impact.plan_shards(full, catalog, 1, test_order=sorted(catalog)),
             "selected": impact.plan_shards(selected, catalog, 1, test_order=sorted(catalog))}
    policy = {"schema": "fluxr-test-shadow-policy-v1", "baseline_snapshot": h("base"),
              "candidate_snapshot": h("candidate"), "baseline_catalog": catalog,
              "candidate_catalog": catalog, "baseline_order": sorted(catalog),
              "candidate_order": sorted(catalog), "context": context(),
              "attempts": {"baseline": ["base-1"], "full": ["full-1"], "selected": ["selected-1"]}}
    trial = {"schema": "fluxr-test-shadow-trial-v1", "selection": selected, "plans": plans,
             "runs": {lane: [run(plans[lane], policy["attempts"][lane][0])]
                      for lane in shadow.LANES}}
    return policy, trial


class ShadowTests(unittest.TestCase):
    def test_agreement_keeps_full_suite_required_and_does_not_authorize_omission(self):
        policy, trial = fixture()
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "agreement")
        self.assertTrue(report["candidate_validation_passed"])
        self.assertFalse(report["activation_authorized"])
        self.assertTrue(report["full_suite_required"])
        self.assertEqual(report["unsafe_omissions"], [])
        self.assertEqual(report["ledgers"]["full"]["records"][0]["ledger"]["observed"], 2)

    def test_omitted_changed_observation_is_detected_even_when_tests_pass(self):
        policy, trial = fixture()
        trial["runs"]["full"] = [run(trial["plans"]["full"], "full-1", changes={"test:b": "changed output"})]
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "divergent")
        self.assertEqual(report["unsafe_omissions"], ["test:b"])
        self.assertTrue(report["candidate_validation_passed"])

    def test_omitted_new_failure_cannot_be_hidden_by_selected_success(self):
        policy, trial = fixture()
        trial["runs"]["full"] = [run(trial["plans"]["full"], "full-1", terminals={"test:b": "failed"})]
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "divergent")
        self.assertEqual(report["unsafe_omissions"], ["test:b"])
        self.assertFalse(report["candidate_validation_passed"])
        self.assertEqual(report["ledgers"]["full"]["records"][0]["ledger"]["failed"], ["test:b"])

    def test_matching_failure_is_selection_agreement_but_never_candidate_pass(self):
        policy, trial = fixture()
        for lane in ("full", "selected"):
            trial["runs"][lane] = [run(trial["plans"][lane], lane + "-1", terminals={"test:a": "failed"})]
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "agreement")
        self.assertFalse(report["candidate_validation_passed"])
        self.assertEqual(report["changed_tests"], ["test:a"])
        self.assertFalse(report["activation_authorized"])

    def test_order_or_fixture_state_divergence_is_not_hidden_by_pass_status(self):
        policy, trial = fixture()
        trial["runs"]["selected"] = [run(trial["plans"]["selected"], "selected-1",
                                                changes={"test:a": "missing fixture state"})]
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "divergent")
        self.assertEqual(report["divergent_selected_tests"], ["test:a"])

    def test_missing_attempt_result_or_observation_is_inconclusive(self):
        for fault in ("attempt", "result", "observation"):
            policy, trial = fixture()
            if fault == "attempt":
                trial["runs"]["full"] = []
            elif fault == "result":
                trial["runs"]["full"][0]["results"].pop()
            else:
                trial["runs"]["full"][0]["observations"].pop("test:b")
            with self.subTest(fault=fault):
                report = shadow.compare_trial(policy, trial)
                self.assertEqual(report["status"], "inconclusive")
                self.assertFalse(report["candidate_validation_passed"])

    def test_skipped_interrupted_and_unknown_terminals_never_establish_agreement(self):
        for terminal in ("skipped", "timeout", "cancelled", "infrastructure_error", "not_run", "unsupported"):
            policy, trial = fixture()
            trial["runs"]["full"] = [run(trial["plans"]["full"], "full-1", terminals={"test:b": terminal})]
            with self.subTest(terminal=terminal):
                report = shadow.compare_trial(policy, trial)
                self.assertEqual(report["status"], "inconclusive")
                self.assertFalse(report["candidate_validation_passed"])

    def test_duplicate_or_undeclared_attempts_are_retained_as_invalid(self):
        for unexpected in (False, True):
            policy, trial = fixture()
            repeat = deepcopy(trial["runs"]["full"][0])
            if unexpected:
                repeat["attempt"] = "hidden-retry"
            trial["runs"]["full"].append(repeat)
            report = shadow.compare_trial(policy, trial)
            self.assertEqual(report["status"], "inconclusive")
            self.assertEqual(report["ledgers"]["full"]["received_attempts"], 2)
            self.assertTrue(report["ledgers"]["full"]["invalid"])

    def test_failed_retry_then_pass_is_unstable_and_cannot_erase_failure(self):
        policy, trial = fixture()
        policy["attempts"]["full"].append("full-2")
        trial["runs"]["full"] = [run(trial["plans"]["full"], "full-1", terminals={"test:a": "failed"}),
                                       run(trial["plans"]["full"], "full-2")]
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "inconclusive")
        self.assertEqual(report["ledgers"]["full"]["unstable_tests"], ["test:a"])
        self.assertEqual(len(report["ledgers"]["full"]["records"]), 2)
        self.assertEqual(report["ledgers"]["full"]["records"][0]["ledger"]["status"], "failed")

    def test_declared_missing_retry_is_not_silently_dropped(self):
        policy, trial = fixture()
        policy["attempts"]["selected"].append("selected-2")
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "inconclusive")
        self.assertEqual(report["ledgers"]["selected"]["missing_attempts"], ["selected-2"])

    def test_independent_catalog_cannot_be_truncated_by_a_valid_self_hashed_plan(self):
        policy, trial = fixture()
        small = {"test:a": policy["candidate_catalog"]["test:a"]}
        selection = impact.select_tests(small, None, None, baseline_snapshot=h("base"),
                                       candidate_snapshot=h("candidate"), analyzer=context()["analyzer"],
                                       policy=context()["policy"])
        trial["plans"]["full"] = impact.plan_shards(selection, small, 1, test_order=["test:a"])
        with self.assertRaisesRegex(ValueError, "independent full catalog"):
            shadow.compare_trial(policy, trial)

    def test_wrong_snapshot_selection_config_or_run_context_refuses(self):
        for fault in ("snapshot", "context", "selection"):
            policy, trial = fixture()
            if fault == "snapshot":
                policy["candidate_snapshot"] = h("stale")
            elif fault == "context":
                trial["runs"]["full"][0]["context"]["target_profile"] = h("different flags")
            else:
                trial["selection"]["selected"] = ["test:b"]
            if fault == "context":
                self.assertEqual(shadow.compare_trial(policy, trial)["status"], "inconclusive")
            else:
                with self.assertRaises(ValueError):
                    shadow.compare_trial(policy, trial)

    def test_no_tests_selected_is_not_credited_as_executed_agreement(self):
        policy, trial = fixture()
        unchanged = graph(h("candidate"))
        selected = impact.select_tests(policy["candidate_catalog"], graph(h("base")), unchanged,
                     baseline_snapshot=h("base"), candidate_snapshot=h("candidate"),
                     analyzer=context()["analyzer"], policy=context()["policy"])
        self.assertEqual(selected["selected"], [])
        trial["selection"] = selected
        trial["plans"]["selected"] = impact.plan_shards(selected, policy["candidate_catalog"], 1,
                                                         test_order=policy["candidate_order"])
        trial["runs"]["selected"] = [run(trial["plans"]["selected"], "selected-1")]
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "inconclusive")
        self.assertEqual(report["ledgers"]["selected"]["records"][0]["ledger"]["status"], "no_tests_selected")

    def test_all_dependency_classes_select_or_fall_back_conservatively(self):
        for name in ("negative_lookup", "configuration", "generated_input", "schema", "effect", "external_resource", "test_discovery"):
            before, after = graph(h("base")), graph(h("candidate"), True)
            for g in (before, after):
                g["nodes"][name] = {"kind": "input", "fingerprint": h("old"), "dependencies": []}
                g["nodes"]["test:b"]["dependencies"] = [name]
            after["nodes"][name]["fingerprint"] = h("changed")
            catalog = {test: after["nodes"][test]["fingerprint"] for test in ("test:a", "test:b")}
            args = dict(baseline_snapshot=h("base"), candidate_snapshot=h("candidate"),
                        analyzer=context()["analyzer"], policy=context()["policy"])
            selected = impact.select_tests(catalog, before, after, **args)
            self.assertEqual(selected["selected"], ["test:a", "test:b"], name)
            after["complete"] = False
            fallback = impact.select_tests(catalog, before, after, **args)
            self.assertEqual(fallback["mode"], "full")
            self.assertEqual(fallback["reason"], "incomplete_graph")

    def test_new_test_discovery_requires_independent_catalog_coverage(self):
        policy, trial = fixture()
        catalog = {**policy["candidate_catalog"], "test:new": h("new-action")}
        after = graph(h("candidate"), True)
        selection = impact.select_tests(catalog, graph(h("base")), after,
                     baseline_snapshot=h("base"), candidate_snapshot=h("candidate"),
                     analyzer=context()["analyzer"], policy=context()["policy"])
        self.assertEqual(selection["mode"], "full")
        self.assertEqual(selection["reason"], "catalog_mismatch")
        self.assertIn("test:new", selection["selected"])

    def test_failed_baseline_is_preserved_and_cannot_qualify_trial(self):
        policy, trial = fixture()
        trial["runs"]["baseline"] = [run(trial["plans"]["baseline"], "base-1", terminals={"test:a": "failed"})]
        report = shadow.compare_trial(policy, trial)
        self.assertEqual(report["status"], "inconclusive")
        self.assertEqual(report["baseline_failed"], ["test:a"])

    def test_bounds_duplicate_policy_ids_and_unknown_fields_refuse(self):
        policy, trial = fixture()
        cases = []
        changed = deepcopy(policy); changed["attempts"]["full"] = ["base-1"]; cases.append(changed)
        changed = deepcopy(policy); changed["context"]["extra"] = h("extra"); cases.append(changed)
        changed = deepcopy(policy); changed["baseline_catalog"] = {}; cases.append(changed)
        changed = deepcopy(policy); changed["attempts"]["full"] = []; cases.append(changed)
        for item in cases:
            with self.assertRaises(ValueError):
                shadow.compare_trial(item, trial)
        with patch.object(shadow, "MAX_ATTEMPTS", 0), self.assertRaises(ValueError):
            shadow.compare_trial(policy, trial)

    def test_input_mutation_cannot_rewrite_returned_report(self):
        policy, trial = fixture()
        report = shadow.compare_trial(policy, trial)
        encoded = canonical_bytes(report)
        policy["attempts"]["full"].clear()
        trial["runs"]["full"].clear()
        self.assertEqual(canonical_bytes(report), encoded)


if __name__ == "__main__":
    unittest.main()
