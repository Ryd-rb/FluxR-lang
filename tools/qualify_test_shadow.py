#!/usr/bin/env python3
"""Execute fixed synthetic full/selected suites and retain shadow trial ledgers.

No model-proposed candidate or arbitrary command is accepted. Input dependencies
are deliberately hand-built fixtures: success does not qualify a real compiler
extractor, hostile execution isolation, or production test omission.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import itertools
import json
from pathlib import Path
import platform
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_pipeline import impact, shadow
from agent_pipeline.identity import canonical_bytes, content_digest

KINDS = ("negative_lookup", "configuration", "generated_input", "schema", "effect", "external_resource", "test_discovery")


def h(value):
    return content_digest(canonical_bytes(value))


def state():
    return {"anchor": 0, "negative_lookup": {}, "configuration": {"flag": False},
            "generated_input": [0], "schema": ["value"], "effect": [],
            "external_resource": "retained-fixture-v1", "test_discovery": ["existing"],
            "a": 0, "b": 0, "c": 0}


def mutate(value, kind):
    replacements = {"negative_lookup": {"optional": 1}, "configuration": {"flag": True},
                    "generated_input": [1], "schema": ["value", "extra"], "effect": ["emit"],
                    "external_resource": "retained-fixture-v2", "test_discovery": ["existing", "new"]}
    value[kind] = replacements[kind]


def observed(test, value, shared):
    name = test.removeprefix("test:")
    if name == "anchor":
        return {"value": value["anchor"]}, True
    if name == "negative_lookup":
        answer = value[name].get("optional", "absent")
        return {"lookup": answer}, answer == "absent"
    if name == "configuration":
        answer = 1 if value[name]["flag"] else 0
        return {"branch": answer}, answer == 0
    if name == "generated_input":
        answer = sum(value[name])
        return {"sum": answer}, answer == 0
    if name == "schema":
        answer = len(value[name])
        return {"fields": answer}, answer == 1
    if name == "effect":
        events = []
        for event in value[name]:
            events.append(event)
        return {"events": events}, not events
    if name == "external_resource":
        answer = h(value[name])
        return {"resource_digest": answer}, answer == h("retained-fixture-v1")
    if name == "test_discovery":
        answer = sorted(value[name])
        return {"discovered": answer}, answer == ["existing"]
    if name == "invariant":
        total = value["a"] + value["b"] + value["c"]
        return {"sum": total, "maximum": 2}, total <= 2
    if name == "setup":
        shared["ready"] = True
        return {"initialized": True}, True
    if name == "uses_setup":
        ready = shared.get("ready", False)
        return {"fixture_ready": ready}, ready
    if name == "new":
        return {"new_test_executed": True}, True
    raise ValueError("unknown fixed fixture test")


def execute(plan, value, context, attempt):
    results, observations, raw, shared = [], {}, {}, {}
    for index, shard in enumerate(plan["shards"]):
        for test in shard:
            observation, passed = observed(test, value, shared)
            observations[test] = h(observation)
            raw[test] = observation
            terminal = "passed" if passed else "failed"
            results.append({"schema": "fluxr-test-result-v1", "plan_key": plan["plan_key"],
                            "candidate_snapshot": plan["candidate_snapshot"], "shard": index,
                            "test": test, "action_key": plan["catalog"][test], "terminal": terminal,
                            "exit_code": 0 if passed else 1,
                            "evidence": [h({"observation": observation, "terminal": terminal})]})
    return {"schema": "fluxr-test-shadow-run-v1", "attempt": attempt, "context": context,
            "results": results, "observations": observations}, raw


def make_graph(value, tests, context, *, omitted_dependency=None):
    nodes = {"input:" + key: {"kind": "input", "fingerprint": h(item), "dependencies": []}
             for key, item in value.items()}
    for test in tests:
        name = test.removeprefix("test:")
        dependencies = (["input:a", "input:b", "input:c"] if name == "invariant"
                        else ["input:anchor"] if name == "uses_setup"
                        else ["input:" + name] if name in value else [])
        if name == omitted_dependency:
            dependencies = []
        nodes[test] = {"kind": "test", "fingerprint": h({"fixed_fixture_action": test}),
                       "dependencies": sorted(dependencies)}
    return {"schema": "fluxr-test-graph-v1", "snapshot": h(value), "analyzer": context["analyzer"],
            "policy": context["policy"], "complete": True, "nodes": nodes}


def trial(label, old, new, *, tests=None, omitted_dependency=None, incomplete=False,
          add_test=False, missing_discovery=False):
    root = Path(__file__).resolve().parents[1]
    fixture_hash = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    context = {
        "runner": fixture_hash,
        "toolchain": h({"python": platform.python_version(), "implementation": platform.python_implementation()}),
        "target_profile": h({"platform": platform.platform(), "mode": "fixed-local-python-fixtures"}),
        "oracle": fixture_hash,
        "analyzer": h({"source": fixture_hash, "kind": "hand-built-dependency-fixture"}),
        "selector": hashlib.sha256((root / "tools/agent_pipeline/impact.py").read_bytes()).hexdigest(),
        "policy": h({"schema": "fixed-shadow-demo-policy-v1", "full_suite_required": True}),
    }
    baseline_tests = tests or ["test:anchor"] + ["test:" + kind for kind in KINDS]
    candidate_tests = baseline_tests + (["test:new"] if add_test else [])
    before = make_graph(old, baseline_tests, context, omitted_dependency=omitted_dependency)
    after = make_graph(new, candidate_tests, context, omitted_dependency=omitted_dependency)
    if incomplete:
        after["complete"] = False
    baseline_catalog = {test: before["nodes"][test]["fingerprint"] for test in baseline_tests}
    candidate_catalog = {test: after["nodes"][test]["fingerprint"] for test in candidate_tests}
    if missing_discovery:
        del after["nodes"]["test:new"]
    selection_args = {"baseline_snapshot": h(old), "candidate_snapshot": h(new),
                      "analyzer": context["analyzer"], "policy": context["policy"]}
    selection = impact.select_tests(candidate_catalog, before, after, **selection_args)
    complete = impact.select_tests(candidate_catalog, None, None, **selection_args)
    initial = impact.select_tests(baseline_catalog, None, None,
                                  **{**selection_args, "candidate_snapshot": h(old)})
    plans = {"baseline": impact.plan_shards(initial, baseline_catalog, 1, test_order=baseline_tests),
             "full": impact.plan_shards(complete, candidate_catalog, 1, test_order=candidate_tests),
             "selected": impact.plan_shards(selection, candidate_catalog, 1, test_order=candidate_tests)}
    policy = {"schema": "fluxr-test-shadow-policy-v1", "baseline_snapshot": h(old),
              "candidate_snapshot": h(new), "baseline_catalog": baseline_catalog,
              "candidate_catalog": candidate_catalog, "baseline_order": baseline_tests,
              "candidate_order": candidate_tests, "context": context,
              "attempts": {lane: [label + "/" + lane + "/1"] for lane in shadow.LANES}}
    runs, raw = {}, {}
    for lane in shadow.LANES:
        executed, raw[lane] = execute(plans[lane], old if lane == "baseline" else new,
                                     context, policy["attempts"][lane][0])
        runs[lane] = [executed]
    value = {"schema": "fluxr-test-shadow-trial-v1", "selection": selection, "plans": plans, "runs": runs}
    start = time.perf_counter_ns()
    result = shadow.compare_trial(policy, value)
    elapsed = time.perf_counter_ns() - start
    return {"label": label, "before_inputs": old, "candidate_inputs": new, "policy": policy,
            "trial": value, "raw_observations": raw, "comparison": result, "compare_elapsed_ns": elapsed}


def campaign():
    rows = []
    for kind in KINDS:
        old, new = state(), state()
        new["anchor"] = 1
        mutate(new, kind)
        good = trial(kind + "-complete", old, new)
        assert good["comparison"]["status"] == "agreement"
        assert not good["comparison"]["candidate_validation_passed"]
        bad = trial(kind + "-omitted", old, new, omitted_dependency=kind)
        assert bad["comparison"]["status"] == "divergent"
        assert bad["comparison"]["unsafe_omissions"] == ["test:" + kind]
        rows.extend([good, bad])
    old, new = state(), state()
    new["anchor"] = 1
    mutate(new, "configuration")
    unknown = trial("unknown-coverage-full-fallback", old, new, incomplete=True)
    assert unknown["trial"]["selection"]["mode"] == "full"
    assert unknown["comparison"]["status"] == "agreement"
    rows.append(unknown)
    discovery = trial("missing-discovered-test-full-fallback", old, new, add_test=True, missing_discovery=True)
    assert discovery["trial"]["selection"]["reason"] == "catalog_mismatch"
    assert "test:new" in discovery["trial"]["selection"]["selected"]
    rows.append(discovery)
    old, new = state(), state()
    new["anchor"] = 1
    order = trial("hidden-ordered-fixture", old, new,
                  tests=["test:anchor", "test:setup", "test:uses_setup"])
    assert order["comparison"]["status"] == "divergent"
    assert order["comparison"]["divergent_selected_tests"] == ["test:uses_setup"]
    rows.append(order)
    pairs = []
    for keys in itertools.combinations(("a", "b", "c"), 2):
        inputs = state()
        for key in keys:
            inputs[key] = 1
        observation, passed = observed("test:invariant", inputs, {})
        pairs.append({"edits": list(keys), "observation": observation, "passed": passed})
        assert passed
    triple = state()
    for key in ("a", "b", "c"):
        triple[key] = 1
    triple["anchor"] = 1
    higher = trial("three-edit-higher-order-conflict", state(), triple,
                   tests=["test:anchor", "test:invariant"], omitted_dependency="invariant")
    assert higher["comparison"]["unsafe_omissions"] == ["test:invariant"]
    higher["all_pairwise_checks"] = pairs
    rows.append(higher)
    root = Path(__file__).resolve().parents[1]
    def git(*args):
        result = subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)
        return result.stdout.strip() if result.returncode == 0 else None
    return {"schema": "fluxr-shadow-fixture-campaign-v1", "python": platform.python_version(),
            "platform": platform.platform(), "head": git("rev-parse", "HEAD"),
            "tree": git("rev-parse", "HEAD^{tree}"), "working_tree_status": git("status", "--porcelain"),
            "source_sha256": {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in
                              ["tools/qualify_test_shadow.py", "tools/agent_pipeline/shadow.py",
                               "tools/agent_pipeline/impact.py"]},
            "trials": rows, "trial_count": len(rows),
            "comparison_counts": {status: sum(row["comparison"]["status"] == status for row in rows)
                                  for status in ("agreement", "divergent", "inconclusive")},
            "limitations": ["Fixed local Python fixtures, no generated or arbitrary candidate execution",
                            "Hand-built dependency graphs are not trusted compiler extraction",
                            "Semantic/backend/source implementation tests still run as full suites",
                            "No producer authentication or hostile-process isolation is demonstrated",
                            "Agreement includes intentionally failing candidates; never a release pass",
                            "Dependency classes model observations, not real plugin/filesystem capture"]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    output = json.dumps(campaign(), sort_keys=True, indent=2) + "\n"
    if args.output:
        args.output.write_text(output)
    else:
        sys.stdout.write(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
