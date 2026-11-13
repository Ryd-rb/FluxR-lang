#!/usr/bin/env python3
"""Measure complete v1/v2 contract preparations for a docs-only edit workload.

This is a local composition microbenchmark, not model/team throughput. The v1
fixture stores its designated documentation under an export descriptor; v2 is a
separately authored profile with that text in its explicit documentation map.
Automatic migration does not move that legacy field or weaken its old meaning.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time
import tracemalloc

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_pipeline import contracts as v1
from agent_pipeline import contracts_v2 as v2
from agent_pipeline.identity import canonical_bytes


TEXT = "Return the checked scalar value."
NEW_TEXT = "Return the checked scalar value; includes a clarified usage example."


def make_catalog(count, shape, version):
    api = v1 if version == 1 else v2
    values, digest = {}, None
    for index in range(count + 1):
        key = "n" + str(index)
        provider = "n0" if shape == "fanout" else "n" + str(index - 1)
        requires = {} if index == 0 else {provider: digest if shape == "chain" else root_digest}
        value = {"schema": api.SCHEMA, "key": key, "requires": requires,
                 "exports": {key + "/Value": {"kind": "scalar", "dtype": "i32"}},
                 "invariants": {"overflow": "Checked signed overflow traps."}}
        if version == 1:
            value["exports"][key + "/Value"]["documentation"] = TEXT
        else:
            value["documentation"] = {"summary": TEXT}
        digest = api.fragment_digest(value) if version == 1 else api.interface_digest(value)
        if index == 0:
            root_digest = digest
        values[key] = value
    return api.Catalog(values.values()), values


def prepare(catalog, values, shape, version):
    if version == 2:
        root = copy.deepcopy(values["n0"])
        root["documentation"]["summary"] = NEW_TEXT
        changes = [{"key": "n0", "expected": v2.fragment_digest(values["n0"]), "replacement": root}]
    else:
        changes, revised = [], {}
        for key, value in values.items():
            updated = copy.deepcopy(value)
            if key == "n0":
                updated["exports"][key + "/Value"]["documentation"] = NEW_TEXT
            else:
                updated["requires"] = {provider: revised[provider] for provider in value["requires"]}
            revised[key] = v1.fragment_digest(updated)
            changes.append({"key": key, "expected": v1.fragment_digest(value), "replacement": updated})
    result = catalog.compose(changes)
    return result, changes


def semantic_projection(value, version):
    # Independent workload-specific observation; not a generic compatibility
    # checker. All requirements, keys, kinds and dtypes must stay unchanged.
    value = copy.deepcopy(value)
    value.pop("requires")  # Required keys are checked separately below.
    value.pop("schema")
    if version == 1:
        for descriptor in value["exports"].values():
            descriptor.pop("documentation")
    else:
        value.pop("documentation")
    return value


def verify(before, values, after, changes, version):
    for key, original in values.items():
        updated = after.fragment(key)
        assert semantic_projection(original, version) == semantic_projection(updated, version)
        assert set(original["requires"]) == set(updated["requires"])
        text = (updated["exports"][key + "/Value"]["documentation"] if version == 1
                else updated["documentation"]["summary"])
        assert text == (NEW_TEXT if key == "n0" else TEXT)
    assert len(changes) == (1 if version == 2 else len(values))
    assert before.revision != after.revision
    if version == 2:
        assert before.interface_revision == after.interface_revision
        assert before.delta(after)["documentation_only"] == ["n0"]
        assert not before.delta(after)["interface_changed"]


def nearest(values, probability):
    return sorted(values)[max(0, math.ceil(len(values) * probability) - 1)]


def git(*args):
    result = subprocess.run(["git", *args], cwd=Path(__file__).resolve().parents[1],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return result.stdout.strip() if result.returncode == 0 else None


def run(counts, repeats):
    rows = []
    for shape in ("fanout", "chain"):
        for count in counts:
            inputs = {version: make_catalog(count, shape, version) for version in (1, 2)}
            timings = {1: [], 2: []}
            results = {}
            # Alternate measurement order to reduce systematic warm/order bias.
            for repeat in range(repeats):
                for version in ((1, 2) if repeat % 2 == 0 else (2, 1)):
                    catalog, values = inputs[version]
                    start = time.perf_counter_ns()
                    output, changes = prepare(catalog, values, shape, version)
                    elapsed = time.perf_counter_ns() - start
                    verify(catalog, values, output, changes, version)
                    timings[version].append(elapsed)
                    results[version] = (output, changes)
            versions = {}
            for version in (1, 2):
                catalog, values = inputs[version]
                tracemalloc.start()
                measured, measured_changes = prepare(catalog, values, shape, version)
                _, peak = tracemalloc.get_traced_memory()
                tracemalloc.stop()
                verify(catalog, values, measured, measured_changes, version)
                result, changes = results[version]
                assert measured.revision == result.revision
                times = timings[version]
                versions[str(version)] = {
                    "baseline_revision": catalog.revision, "result_revision": result.revision,
                    "fragments": len(values), "edited_fragments": len(changes),
                    "input_canonical_bytes": sum(len(canonical_bytes(value)) for value in values.values()),
                    "edit_transport_bytes": sum(len(canonical_bytes(change)) for change in changes),
                    "result_manifest_bytes": len(canonical_bytes(result.manifest())),
                    "elapsed_ns": times, "median_ns": statistics.median(times),
                    "p95_ns_nearest_rank": nearest(times, 0.95),
                    "peak_python_allocated_bytes_separate_run": peak,
                    "behavior_checked_every_run": True}
            rows.append({"shape": shape, "dependent_fragments": count, "versions": versions,
                         "median_v1_over_v2": versions["1"]["median_ns"] / versions["2"]["median_ns"]})
    root = Path(__file__).resolve().parents[1]
    paths = ["tools/agent_pipeline/contracts.py", "tools/agent_pipeline/contracts_v2.py",
             "tools/bench_contract_interfaces.py"]
    return {"schema": "fluxr-contract-interface-benchmark-v1",
            "python": platform.python_version(), "platform": platform.platform(),
            "head": git("rev-parse", "HEAD"), "head_tree": git("rev-parse", "HEAD^{tree}"),
            "working_tree_status": git("status", "--porcelain"),
            "source_sha256": {path: hashlib.sha256((root / path).read_bytes()).hexdigest() for path in paths},
            "repetitions": repeats, "rows": rows,
            "timing_scope": "owned edit construction, digests and full catalog compose; verification outside timer",
            "exclusions": ["initial catalog construction", "live agents", "publication", "type/effect checking", "network"],
            "limitations": ["v2 still validates whole catalogs; pin independence is not incremental compilation",
                            "Python tracemalloc is not total resident memory",
                            "small samples do not qualify production tail latency",
                            "profiles authored separately; migration never strips legacy normative descriptor fields"]}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--counts", nargs="+", type=int, default=[32, 256, 1024])
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if any(count < 1 or count >= v1.MAX_FRAGMENTS for count in args.counts) or not 1 <= args.repeats <= 100:
        parser.error("counts must fit catalog bounds; repeats must be 1..100")
    payload = json.dumps(run(args.counts, args.repeats), indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(payload)
    else:
        sys.stdout.write(payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
