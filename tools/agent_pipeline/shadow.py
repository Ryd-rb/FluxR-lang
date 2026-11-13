"""Bounded full-suite versus selected-suite shadow observations, no omission grant.

The caller owns the independent catalogs, context and preregistered attempt IDs.
This comparator neither executes candidate code nor authenticates supplied logs.
Finite agreement can support a qualification campaign; it never authorizes test
omission or proves that a dependency producer is complete.
"""
from __future__ import annotations

from . import impact
from .identity import canonical_bytes, load_json_bytes, require_digest

CONTEXT_FIELDS = {"runner", "toolchain", "target_profile", "oracle", "analyzer", "selector", "policy"}
LANES = ("baseline", "full", "selected")
MAX_ATTEMPTS = 64


def _owned(value):
    return load_json_bytes(canonical_bytes(value))


def _policy(value):
    value = _owned(value)
    impact._fields(value, {"schema", "baseline_snapshot", "candidate_snapshot", "baseline_catalog",
                           "candidate_catalog", "baseline_order", "candidate_order", "context", "attempts"})
    if value["schema"] != "fluxr-test-shadow-policy-v1":
        raise ValueError("unsupported shadow policy")
    for key in ("baseline_snapshot", "candidate_snapshot"):
        require_digest(value[key])
    impact._fields(value["context"], CONTEXT_FIELDS)
    for digest in value["context"].values():
        require_digest(digest)
    for prefix in ("baseline", "candidate"):
        catalog = impact._catalog(value[prefix + "_catalog"])
        order = value[prefix + "_order"]
        if not catalog:
            raise ValueError("shadow qualification requires a nonempty independent full catalog")
        if (type(order) is not list or any(type(name) is not str for name in order)
                or len(order) != len(catalog) or set(order) != set(catalog)):
            raise ValueError("shadow policy needs an exact full-suite order")
        value[prefix + "_catalog"] = catalog
    impact._fields(value["attempts"], LANES)
    all_attempts = set()
    for lane in LANES:
        attempts = value["attempts"][lane]
        if type(attempts) is not list or not 1 <= len(attempts) <= MAX_ATTEMPTS:
            raise ValueError("shadow policy requires bounded nonempty attempt lists")
        for attempt in attempts:
            impact._name(attempt)
            if attempt in all_attempts:
                raise ValueError("shadow attempt IDs must be globally unique")
            all_attempts.add(attempt)
    return value


def _plans(policy, trial):
    plans = trial["plans"]
    impact._fields(plans, LANES)
    context = policy["context"]
    selection = trial["selection"]
    # Reuse the released selection validator through plan_shards. This validates
    # all fields, canonical identity and the complete selected/omitted partition.
    impact.plan_shards(selection, policy["candidate_catalog"], 1,
                       test_order=policy["candidate_order"])
    if (selection["baseline_snapshot"] != policy["baseline_snapshot"]
            or selection["candidate_snapshot"] != policy["candidate_snapshot"]
            or selection["analyzer"] != context["analyzer"]
            or selection["policy"] != context["policy"]):
        raise ValueError("selection does not match trusted shadow context")
    for lane in LANES:
        plan = plans[lane]
        impact._validate_plan(plan)
        prefix = "baseline" if lane == "baseline" else "candidate"
        if (plan["candidate_snapshot"] != policy[prefix + "_snapshot"]
                or plan["catalog"] != policy[prefix + "_catalog"]
                or plan["test_order"] != policy[prefix + "_order"]):
            raise ValueError(f"{lane} plan differs from independent full catalog, snapshot or order")
        if lane != "selected":
            if plan["omitted"] or plan["selected"] != sorted(plan["catalog"]):
                raise ValueError(f"{lane} plan must execute the entire independent catalog")
            if plan["isolation"] is not None:
                raise ValueError("full shadow oracle must use its qualified single-shard order")
        else:
            if (plan["selection_key"] != selection["selection_key"]
                    or plan["selected"] != selection["selected"]
                    or plan["omitted"] != selection["omitted"]):
                raise ValueError("selected plan differs from the shadow selection")
        isolation = plan["isolation"]
        if isolation is not None:
            if isolation["runner"] != context["runner"] or isolation["policy"] != context["policy"]:
                raise ValueError("selected isolation context differs from trusted runner/policy")
    return plans


def _runs(policy, plans, lane, supplied):
    expected = policy["attempts"][lane]
    if type(supplied) is not list or len(supplied) > MAX_ATTEMPTS:
        raise ValueError("shadow run collection must be bounded")
    attempts, invalid, records = {}, [], []
    for run in supplied:
        attempt = None
        try:
            impact._fields(run, {"schema", "attempt", "context", "results", "observations"})
            if run["schema"] != "fluxr-test-shadow-run-v1":
                raise ValueError("unsupported shadow run schema")
            attempt = impact._name(run["attempt"])
            if attempt not in expected or attempt in attempts:
                raise ValueError("unexpected or duplicate shadow attempt")
            # Reserve an observed attempt even if its content is invalid.
            attempts[attempt] = None
            if run["context"] != policy["context"]:
                raise ValueError("run context differs from trusted shadow context")
            ledger = impact.collect_results(plans[lane], run["results"])
            observations = run["observations"]
            if type(observations) is not dict or len(observations) > impact.MAX_NODES:
                raise ValueError("observations require a bounded test-to-digest map")
            for test, digest in observations.items():
                impact._name(test)
                require_digest(digest)
            # Require an observation for every expected case, including failed
            # assertions, and forbid undeclared observations. Missing execution
            # cannot be supplied by an observation-only row.
            if set(observations) != set(plans[lane]["selected"]):
                raise ValueError("observations do not cover the complete planned test set")
            record = {"attempt": attempt, "run_digest": impact._identity("fluxr/test-shadow-run/v1", run),
                      "ledger": ledger}
            records.append(record)
            if ledger["status"] not in ("passed", "failed"):
                invalid.append({"attempt": attempt, "reason": "execution_" + ledger["status"]})
                continue
            outcomes = {}
            for result in run["results"]:
                if result["terminal"] not in ("passed", "failed"):
                    raise ValueError("interrupted or unsupported execution is not a comparison outcome")
                outcomes[result["test"]] = (result["terminal"], result["exit_code"],
                                             observations[result["test"]])
            attempts[attempt] = outcomes
        except (ValueError, TypeError, UnicodeError) as error:
            invalid.append({"attempt": attempt, "reason": str(error)})
    missing = [attempt for attempt in expected if attempt not in attempts]
    successful = [attempts[attempt] for attempt in expected if attempts.get(attempt) is not None]
    unstable = sorted({test for observed in successful[1:] for test in observed
                       if observed[test] != successful[0][test]}) if successful else []
    comparable = (not invalid and not missing and not unstable and len(successful) == len(expected))
    report = {"expected_attempts": expected[:], "received_attempts": len(supplied),
              "missing_attempts": missing, "invalid": invalid, "unstable_tests": unstable,
              "records": records, "comparable": comparable}
    return report, successful[0] if comparable else None


def compare_trial(policy, trial):
    """Compare every preregistered attempt against full-suite observations.

    Policy belongs to trusted orchestration, independent of a proposing agent.
    Empty/missing/skipped/interrupted runs cannot establish agreement. Different
    retry outcomes remain unstable; a later success never erases an earlier
    failure. Matching assertion failures can establish *selection agreement* but
    leave candidate_validation_passed false. Authentication and required
    deployment-specific qualification remain separate and full suites mandatory.
    """
    policy = _policy(policy)
    trial = _owned(trial)
    impact._fields(trial, {"schema", "selection", "plans", "runs"})
    if trial["schema"] != "fluxr-test-shadow-trial-v1":
        raise ValueError("unsupported shadow trial schema")
    plans = _plans(policy, trial)
    impact._fields(trial["runs"], LANES)
    ledgers, outcomes = {}, {}
    for lane in LANES:
        ledgers[lane], outcomes[lane] = _runs(policy, plans, lane, trial["runs"][lane])
    report = {"schema": "fluxr-test-shadow-comparison-v1",
              "policy_digest": impact._identity("fluxr/test-shadow-policy/v1", policy),
              "trial_digest": impact._identity("fluxr/test-shadow-trial/v1", trial),
              "baseline_snapshot": policy["baseline_snapshot"],
              "candidate_snapshot": policy["candidate_snapshot"],
              "selection_key": trial["selection"]["selection_key"],
              "status": "inconclusive", "candidate_validation_passed": False,
              "activation_authorized": False, "full_suite_required": True,
              "ledgers": ledgers, "changed_tests": [], "unsafe_omissions": [],
              "divergent_selected_tests": [], "removed_tests": sorted(
                  policy["baseline_catalog"].keys() - policy["candidate_catalog"].keys()),
              "scope": "Finite supplied observations only; no authentication, dependency completeness proof or omission grant."}
    if any(outcomes[lane] is None for lane in LANES):
        return report
    baseline, full, selected = (outcomes[lane] for lane in LANES)
    if any(value[0] != "passed" for value in baseline.values()):
        report["baseline_failed"] = sorted(test for test, value in baseline.items() if value[0] != "passed")
        return report
    changed = sorted(test for test in full
                     if test not in baseline or full[test] != baseline[test]
                     or policy["baseline_catalog"].get(test) != policy["candidate_catalog"][test])
    omitted = sorted(set(changed) - set(selected))
    divergent = sorted(test for test in selected if selected[test] != full[test])
    report.update(status="divergent" if omitted or divergent else "agreement",
                  candidate_validation_passed=all(value[0] == "passed" for value in full.values()),
                  changed_tests=changed, unsafe_omissions=omitted,
                  divergent_selected_tests=divergent)
    return report
