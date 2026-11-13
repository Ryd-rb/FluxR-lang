#!/usr/bin/env python3
"""Measure committed-source capture/materialization, not accepted coding work."""
from __future__ import annotations

import argparse
from concurrent.futures import ProcessPoolExecutor
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import resource
import subprocess
import time

from agent_pipeline import ContentStore
from agent_pipeline.snapshot import capture_git, materialize, verify_materialization, _cleanup


def quantiles(values):
    values = sorted(values)
    if not values:
        return {"n": 0}
    return {"n": len(values), **{name: values[max(0, math.ceil(q * len(values)) - 1)]
             for name, q in (("p50", .50), ("p95", .95), ("p99", .99))}}


def work(payload):
    repository, commit, workdir, store_path, slot, cpus, enqueued_ns = payload
    started = time.perf_counter_ns()
    os.sched_setaffinity(0, set(cpus))
    row = {"task": slot, "pid": os.getpid(), "cpus": sorted(os.sched_getaffinity(0)),
           "wait_ns": started - enqueued_ns, "terminal": "failed"}
    destination = Path(workdir) / f"worker-{slot}"
    try:
        store = ContentStore(store_path)
        capture_start = time.perf_counter_ns()
        captured = capture_git(repository, commit, store)
        captured_ns = time.perf_counter_ns()
        copied = materialize(store, captured["snapshot"], destination)
        materialized_ns = time.perf_counter_ns()
        checked = verify_materialization(store, captured["snapshot"], destination)
        verified_ns = time.perf_counter_ns()
        row.update(terminal="verified_source_copy", capture=captured,
                   materialization=copied, verification=checked,
                   capture_ns=captured_ns - capture_start,
                   materialize_ns=materialized_ns - captured_ns,
                   independent_reread_ns=verified_ns - materialized_ns,
                   service_ns=verified_ns - started)
    except Exception as error:
        row.update(error_type=type(error).__name__, error=str(error), service_ns=time.perf_counter_ns() - started)
    finally:
        if destination.exists():
            _cleanup(destination)
        row["process_peak_rss_kib"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--work", required=True, help="new external temporary work directory")
    parser.add_argument("--output", required=True, help="new external JSON report")
    parser.add_argument("--tasks", type=int, default=12)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--workers", type=int, nargs="+", default=[1, 2, 4])
    args = parser.parse_args()
    repository, workdir, output = Path(args.repository).resolve(), Path(args.work).resolve(), Path(args.output).resolve()
    if workdir.is_relative_to(repository) or output.is_relative_to(repository):
        parser.error("work and report must be outside the source repository")
    if not 1 <= args.tasks <= 1000 or not 1 <= args.repeats <= 20 or any(not 1 <= n <= 64 for n in args.workers):
        parser.error("experiment execution bounds exceeded")
    workdir.mkdir()
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        parser.error("report already exists")
    available = sorted(os.sched_getaffinity(0))
    if len(available) < 2:
        parser.error("two CPU affinity slots are required for the fixed-total design")
    report = {"schema": "fluxr-source-preparation-benchmark-v1", "repository": str(repository),
              "commit": args.commit, "python": platform.python_version(), "platform": platform.platform(),
              "git": subprocess.check_output(["git", "--version"], text=True).strip(),
              "available_cpus": available, "tasks_per_trial": args.tasks, "repeats": args.repeats,
              "definition": "One accepted unit is one full independently reverified source copy, never an accepted coding task or cache hit.",
              "limitations": ["Source preparation only; no builds/tests/review/publication/tokens/provider episodes included.",
                "CPU affinity is enforced; memory and storage are shared and not physically partitioned.",
                "Other host activity can affect timings; no calibrated fleet or linear-scaling claim.",
                "p95/p99 are empirical nearest-rank order statistics, not confidence bounds.",
                "CAS is warmed before each trial; every warm object is rehashed and synchronized.",
                "RSS is process high-water mark across its assigned tasks, not per-task allocation."], "trials": []}
    store = ContentStore(workdir / "objects")
    cold_start = time.perf_counter_ns()
    report["cold_capture"] = capture_git(repository, args.commit, store)
    report["cold_capture_ns"] = time.perf_counter_ns() - cold_start
    for resource_mode in ("fixed_total", "expanding"):
        for workers in args.workers:
            needed = 2 if resource_mode == "fixed_total" else 2 * workers
            if needed > len(available):
                report["trials"].append({"mode": resource_mode, "workers": workers, "terminal": "unsupported_cpu_capacity"})
                continue
            for repeat in range(args.repeats):
                trial_path = workdir / f"{resource_mode}-{workers}-{repeat}"
                trial_path.mkdir()
                started = time.perf_counter_ns()
                payloads = [(str(repository), args.commit, str(trial_path), str(store.root), slot,
                             available[:2] if resource_mode == "fixed_total" else available[(slot % workers)*2:(slot % workers+1)*2],
                             started) for slot in range(args.tasks)]
                with ProcessPoolExecutor(max_workers=workers) as executor:
                    rows = list(executor.map(work, payloads))
                finished = time.perf_counter_ns()
                accepted = sum(row["terminal"] == "verified_source_copy" for row in rows)
                expected = report["cold_capture"]["snapshot"]
                identities_ok = all(row.get("capture", {}).get("snapshot") == expected for row in rows)
                trial = {"mode": resource_mode, "workers": workers, "repeat": repeat, "cpu_slots": needed,
                         "terminal": "passed" if accepted == args.tasks and identities_ok else "failed",
                         "accepted_source_copies": accepted, "wall_ns": finished - started,
                         "accepted_source_copies_per_second": accepted * 1e9 / (finished - started),
                         "service_ns": quantiles([r["service_ns"] for r in rows]),
                         "wait_ns": quantiles([r["wait_ns"] for r in rows]), "rows": rows}
                report["trials"].append(trial)
                output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    report["all_executed_trials_passed"] = all(t["terminal"] == "passed" for t in report["trials"])
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"report": str(output), "trials": len(report["trials"]), "passed": report["all_executed_trials_passed"]}))
    return 0 if report["all_executed_trials_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
