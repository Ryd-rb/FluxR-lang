"""Conservative test selection and deterministic, complete selected-test ledgers.

The graph and candidate catalog must come from a trusted, complete dependency
producer. This module checks their contracts; it cannot discover omitted edges.
Negative lookups, directory membership, generators, options, environment and
invariant dependencies must be graph inputs. Missing/incompatible analysis falls
back to the entire trusted candidate catalog. No command or candidate is run.
"""
from __future__ import annotations

from collections import defaultdict, deque
from dataclasses import dataclass
import heapq
from types import MappingProxyType

from .identity import canonical_bytes, content_digest, load_json_bytes, require_digest

MAX_NODES = 100_000
MAX_EDGES = 1_000_000
MAX_SHARDS = 1024
TERMINALS = frozenset({"passed", "failed", "timeout", "cancelled", "infrastructure_error"})


def _name(value):
    if (type(value) is not str or not value or len(value.encode("utf-8")) > 1024
            or any(ord(c) < 32 or ord(c) == 127 for c in value)):
        raise ValueError("invalid dependency/test identifier")
    return value


def _fields(value, expected):
    if type(value) is not dict or set(value) != set(expected):
        raise ValueError("unexpected or missing fields")


def _identity(domain, value):
    return content_digest(domain.encode("ascii") + b"\0" + canonical_bytes(value))


def _catalog(catalog):
    if type(catalog) is not dict or len(catalog) > MAX_NODES:
        raise ValueError("invalid trusted candidate test catalog")
    catalog = load_json_bytes(canonical_bytes(catalog))
    for test, action in catalog.items():
        _name(test)
        require_digest(action)
    canonical_bytes(catalog)
    return dict(sorted(catalog.items()))


@dataclass(frozen=True)
class Node:
    kind: str
    fingerprint: str
    dependencies: tuple[str, ...]


@dataclass(frozen=True)
class Graph:
    snapshot: str
    analyzer: str
    policy: str
    complete: bool
    nodes: object

    @classmethod
    def parse(cls, value):
        value = load_json_bytes(canonical_bytes(value))
        _fields(value, {"schema", "snapshot", "analyzer", "policy", "complete", "nodes"})
        if value["schema"] != "fluxr-test-graph-v1" or type(value["complete"]) is not bool:
            raise ValueError("invalid graph schema or completeness flag")
        for key in ("snapshot", "analyzer", "policy"):
            require_digest(value[key])
        entries = value["nodes"]
        if type(entries) is not dict or len(entries) > MAX_NODES:
            raise ValueError("invalid or oversized graph")
        canonical_bytes(value)
        nodes, edges = {}, 0
        for name, entry in entries.items():
            _name(name)
            _fields(entry, {"kind", "fingerprint", "dependencies"})
            if entry["kind"] not in ("input", "query", "test"):
                raise ValueError("invalid graph node kind")
            require_digest(entry["fingerprint"])
            dependencies = entry["dependencies"]
            if type(dependencies) is not list:
                raise ValueError("dependencies must be an ordered list")
            for dependency in dependencies:
                _name(dependency)
                if dependency not in entries:
                    raise ValueError("unresolved dependency; producer must declare negative inputs")
            if dependencies != sorted(set(dependencies)):
                raise ValueError("dependencies must be unique and sorted")
            edges += len(dependencies)
            if edges > MAX_EDGES:
                raise ValueError("too many dependency edges")
            nodes[name] = Node(entry["kind"], entry["fingerprint"], tuple(dependencies))
        return cls(value["snapshot"], value["analyzer"], value["policy"],
                   value["complete"], MappingProxyType(nodes))


def select_tests(catalog, before, after, *, baseline_snapshot, candidate_snapshot, analyzer, policy):
    """Select the reverse closure of changes across BOTH dependency graphs.

    Cycles are supported without recursion. Test-node fingerprints must be their
    exact action keys. The suite catalog is independent of the optimization so
    incomplete extraction cannot silently reduce the fallback test denominator.
    Invalid graphs are an optimization miss; an invalid trusted catalog/context
    is an input error. Caller-provided completeness is not a proof of soundness.
    """
    catalog = _catalog(catalog)
    for identity in (baseline_snapshot, candidate_snapshot, analyzer, policy):
        require_digest(identity)
    selection = {"schema": "fluxr-test-selection-v1", "baseline_snapshot": baseline_snapshot,
                 "candidate_snapshot": candidate_snapshot, "analyzer": analyzer, "policy": policy,
                 "catalog_digest": _identity("fluxr/test-catalog/v1", catalog),
                 "mode": "full", "reason": "missing_graph", "changed_nodes": [],
                 "selected": list(catalog), "omitted": []}

    def finish():
        return {**selection, "selection_key": _identity("fluxr/test-selection/v1", selection)}

    if before is None or after is None:
        return finish()
    try:
        old, new = Graph.parse(before), Graph.parse(after)
    except (ValueError, TypeError, UnicodeError, RecursionError):
        selection["reason"] = "invalid_graph"
        return finish()
    if old.snapshot != baseline_snapshot or new.snapshot != candidate_snapshot:
        selection["reason"] = "snapshot_mismatch"
    elif any(g.analyzer != analyzer or g.policy != policy for g in (old, new)):
        selection["reason"] = "analysis_identity_mismatch"
    elif not old.complete or not new.complete:
        selection["reason"] = "incomplete_graph"
    elif {name: node.fingerprint for name, node in new.nodes.items() if node.kind == "test"} != catalog:
        selection["reason"] = "catalog_mismatch"
    else:
        changed = {name for name in old.nodes.keys() | new.nodes.keys()
                   if old.nodes.get(name) != new.nodes.get(name)}
        reverse = defaultdict(set)
        for graph in (old, new):
            for name, node in graph.nodes.items():
                for dependency in node.dependencies:
                    reverse[dependency].add(name)
        impacted = set(changed)
        pending = deque(sorted(changed))
        while pending:
            for dependent in reverse[pending.popleft()]:
                if dependent not in impacted:
                    impacted.add(dependent)
                    pending.append(dependent)
        # A test can require another test's fixture/setup outcome. Include its
        # current prerequisites too, even when the prerequisite itself did not
        # change. A producer that omits such effects cannot claim completeness.
        needed = set(impacted & catalog.keys())
        visited, prerequisites = set(), list(needed)
        while prerequisites:
            name = prerequisites.pop()
            if name in visited:
                continue
            visited.add(name)
            if name in catalog:
                needed.add(name)
            prerequisites.extend(new.nodes[name].dependencies)
        chosen = sorted(needed)
        selection.update(mode="incremental", reason="complete_compatible_graphs",
                         changed_nodes=sorted(changed), selected=chosen,
                         omitted=sorted(catalog.keys() - set(chosen)))
    return finish()


def plan_shards(selection, catalog, shard_count, *, estimated_ms=None, test_order=None, isolation=None):
    """Deterministic longest-first packing; estimates affect scheduling only.

    Shard identity commits to the candidate, full suite, selection and exact test
    actions. Changing worker count creates a different plan, never missing rows.
    Empty selections create zero shards and cannot be counted as executed tests.
    Without an isolation qualification, require and preserve the trusted
    runner's declared full-suite order in a SINGLE shard. Parallel packing
    requires a runner/policy/qualification
    identity from trusted infrastructure; a self-assertion does not prove it.
    """
    selection = load_json_bytes(canonical_bytes(selection))
    catalog = _catalog(catalog)
    _fields(selection, {"schema", "baseline_snapshot", "candidate_snapshot", "analyzer", "policy",
                        "catalog_digest", "mode", "reason", "changed_nodes", "selected", "omitted",
                        "selection_key"})
    raw = {k: v for k, v in selection.items() if k != "selection_key"}
    if (selection["schema"] != "fluxr-test-selection-v1"
            or selection["selection_key"] != _identity("fluxr/test-selection/v1", raw)
            or selection["catalog_digest"] != _identity("fluxr/test-catalog/v1", catalog)):
        raise ValueError("selection identity mismatch")
    for key in ("baseline_snapshot", "candidate_snapshot", "analyzer", "policy"):
        require_digest(selection[key])
    for key in ("selected", "omitted", "changed_nodes"):
        items = selection[key]
        if type(items) is not list:
            raise ValueError("invalid selection partition")
        for item in items:
            _name(item)
        if items != sorted(set(items)):
            raise ValueError("selection lists must be unique and sorted")
    selected = selection["selected"]
    selected_set = set(selected)
    if (set(selected) & set(selection["omitted"])
            or set(selected) | set(selection["omitted"]) != set(catalog)):
        raise ValueError("selection must partition the full catalog")
    if selection["mode"] not in ("full", "incremental"):
        raise ValueError("invalid selection mode")
    if selection["mode"] == "full" and selection["omitted"]:
        raise ValueError("full fallback cannot omit tests")
    if type(shard_count) is not int or not 1 <= shard_count <= MAX_SHARDS:
        raise ValueError("invalid shard count")
    if test_order is None:
        if isolation is None:
            raise ValueError("ordered execution requires an explicit qualified full-suite test_order")
        test_order = list(catalog)
    else:
        test_order = load_json_bytes(canonical_bytes(test_order))
    if (type(test_order) is not list or any(type(x) is not str for x in test_order)
            or len(test_order) != len(catalog) or set(test_order) != set(catalog)):
        raise ValueError("test order must contain the full catalog exactly once")
    if isolation is not None:
        isolation = load_json_bytes(canonical_bytes(isolation))
        _fields(isolation, {"schema", "runner", "policy", "qualification"})
        if isolation["schema"] != "fluxr-isolated-tests-v1":
            raise ValueError("unknown test isolation contract")
        for key in ("runner", "policy", "qualification"):
            require_digest(isolation[key])
        if isolation["policy"] != selection["policy"]:
            raise ValueError("isolation policy differs from selection policy")
    if estimated_ms is None:
        estimated_ms = {}
    else:
        estimated_ms = load_json_bytes(canonical_bytes(estimated_ms))
    if type(estimated_ms) is not dict or set(estimated_ms) - set(catalog):
        raise ValueError("estimates refer to unknown tests")
    for cost in estimated_ms.values():
        if type(cost) is not int or not 1 <= cost < 2**63:
            raise ValueError("duration estimates must be positive int64 milliseconds")
    groups = [[] for _ in range(min(shard_count if isolation is not None else 1, len(selected)))]
    heap = [(0, index) for index in range(len(groups))]
    if isolation is None:
        if groups:
            groups[0] = [test for test in test_order if test in selected_set]
    else:
        for test in sorted(selected, key=lambda name: (-estimated_ms.get(name, 1), name)):
            load, index = heapq.heappop(heap)
            groups[index].append(test)
            heapq.heappush(heap, (load + estimated_ms.get(test, 1), index))
        groups = [sorted(group) for group in groups]
    plan = {"schema": "fluxr-test-shards-v1", "candidate_snapshot": selection["candidate_snapshot"],
            "selection_key": selection["selection_key"], "catalog_digest": selection["catalog_digest"],
            "catalog": catalog, "selected": selected[:], "omitted": selection["omitted"][:],
            "test_order": test_order[:], "isolation": None if isolation is None else dict(isolation),
            "shards": groups}
    return {**plan, "plan_key": _identity("fluxr/test-shards/v1", plan)}


def _validate_plan(plan):
    _fields(plan, {"schema", "candidate_snapshot", "selection_key", "catalog_digest", "catalog",
                   "selected", "omitted", "test_order", "isolation", "shards", "plan_key"})
    if plan["schema"] != "fluxr-test-shards-v1":
        raise ValueError("invalid shard schema")
    raw = {k: v for k, v in plan.items() if k != "plan_key"}
    if plan["plan_key"] != _identity("fluxr/test-shards/v1", raw):
        raise ValueError("shard plan identity mismatch")
    for key in ("candidate_snapshot", "selection_key", "catalog_digest"):
        require_digest(plan[key])
    catalog = _catalog(plan["catalog"])
    if plan["catalog_digest"] != _identity("fluxr/test-catalog/v1", catalog):
        raise ValueError("catalog identity mismatch")
    for key in ("selected", "omitted"):
        if type(plan[key]) is not list or any(type(x) is not str for x in plan[key]):
            raise ValueError("invalid plan test partition")
        if plan[key] != sorted(set(plan[key])):
            raise ValueError("invalid plan test partition")
    if (set(plan["selected"]) & set(plan["omitted"])
            or set(plan["selected"]) | set(plan["omitted"]) != set(catalog)):
        raise ValueError("plan does not partition catalog")
    order = plan["test_order"]
    if (type(order) is not list or any(type(x) is not str for x in order)
            or len(order) != len(catalog) or set(order) != set(catalog)):
        raise ValueError("invalid declared test order")
    isolation = plan["isolation"]
    if isolation is not None:
        _fields(isolation, {"schema", "runner", "policy", "qualification"})
        if isolation["schema"] != "fluxr-isolated-tests-v1":
            raise ValueError("invalid isolation contract")
        for key in ("runner", "policy", "qualification"):
            require_digest(isolation[key])
    shards = plan["shards"]
    if type(shards) is not list or len(shards) > MAX_SHARDS:
        raise ValueError("invalid shards")
    rows = []
    for shard in shards:
        if type(shard) is not list or not shard or any(type(x) is not str for x in shard):
            raise ValueError("invalid or empty shard")
        if len(shard) != len(set(shard)):
            raise ValueError("invalid shard rows")
        rows.extend(shard)
    if sorted(rows) != plan["selected"]:
        raise ValueError("shards must cover every selected test exactly once")
    if isolation is None:
        selected_set = set(plan["selected"])
        expected = [test for test in order if test in selected_set]
        if len(shards) > 1 or rows != expected:
            raise ValueError("unqualified tests must preserve order in one shard")


def collect_results(plan, results):
    """Check a full selected-test ledger, including failures and missing results.

    Duplicate delivery is rejected rather than silently hiding a conflicting
    result. Retry attempts need a new supervisor ledger. Records are not signed;
    this is protocol validation, not authentication or release authorization.
    Omitted tests still require separately justified evidence for full acceptance.
    """
    plan = load_json_bytes(canonical_bytes(plan))
    if type(results) is not list or len(results) > MAX_NODES:
        raise ValueError("invalid result collection")
    results = load_json_bytes(canonical_bytes(results))
    _validate_plan(plan)
    expected = {test: index for index, shard in enumerate(plan["shards"]) for test in shard}
    rows, errors = {}, []
    for result in results:
        try:
            _fields(result, {"schema", "plan_key", "candidate_snapshot", "shard", "test", "action_key",
                             "terminal", "exit_code", "evidence"})
            test = _name(result["test"])
            if (result["schema"] != "fluxr-test-result-v1" or result["plan_key"] != plan["plan_key"]
                    or result["candidate_snapshot"] != plan["candidate_snapshot"]
                    or test not in expected or type(result["shard"]) is not int
                    or result["shard"] != expected[test]
                    or result["action_key"] != plan["catalog"][test]
                    or type(result["terminal"]) is not str or result["terminal"] not in TERMINALS):
                raise ValueError("result does not match plan")
            code = result["exit_code"]
            if code is not None and (type(code) is not int or not -(2**31) <= code < 2**31):
                raise ValueError("invalid exit code")
            if result["terminal"] == "passed" and code != 0:
                raise ValueError("passing result needs zero exit code")
            if type(result["evidence"]) is not list or not result["evidence"]:
                raise ValueError("result needs evidence references")
            for item in result["evidence"]:
                require_digest(item)
            if result["evidence"] != sorted(set(result["evidence"])):
                raise ValueError("evidence references must be unique and sorted")
            if test in rows:
                raise ValueError("duplicate result")
            rows[test] = result["terminal"]
        except (ValueError, TypeError, UnicodeError) as error:
            errors.append(str(error))
    missing = sorted(set(expected) - set(rows))
    failed = sorted(test for test, terminal in rows.items() if terminal != "passed")
    status = ("invalid" if errors else "incomplete" if missing else "failed" if failed
              else "passed" if expected else "no_tests_selected")
    return {"schema": "fluxr-test-ledger-v1", "plan_key": plan["plan_key"], "status": status,
            "expected": len(expected), "observed": len(rows), "missing": missing,
            "failed": failed, "errors": errors, "omitted": plan["omitted"][:],
            "results_digest": _identity("fluxr/test-results/v1", results),
            "scope": "Selected tests only; producer authentication and omitted-test evidence are separate."}
