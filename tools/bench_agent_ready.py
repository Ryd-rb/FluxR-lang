#!/usr/bin/env python3
"""Compare legacy and indexed dependency-ready dispatch on identical queue states.

This is a local scheduler microbenchmark, not agent throughput. Both policies
must choose the same first eligible task. Setup, SQLite backup and independent
VM-step instrumentation are excluded from wall time. No worker code executes.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import platform
import sqlite3
import statistics
import sys
import tempfile
import time

from agent_pipeline import ContentStore, WorkQueue


def spec():
    return {"schema": "fluxr-action-v1", "argv": ["fixture"], "working_directory": ".",
            "environment": {}, "inputs": {}, "outputs": [], "source_snapshot": "1" * 64,
            "toolchain": "2" * 64, "runner": "3" * 64, "platform": "fixture",
            "target": "fixture", "profile": "fixture", "policy": "4" * 64}


def fixture(path, store, blocked):
    queue = WorkQueue(path, store, lambda: 1000)
    queue.enqueue("parent", spec())
    queue.claim("parent", "waiting")
    for index in range(blocked):
        queue.enqueue(f"blocked-{index:06}", spec(), dependencies=["parent"])
    queue.enqueue("eligible", spec())
    queue.close()


def observe(source, destination, store, method, instrument=False):
    with sqlite3.connect(source) as original, sqlite3.connect(destination) as clone:
        original.backup(clone)
    queue = WorkQueue(destination, store, lambda: 1000)
    try:
        instructions = 0
        if instrument:
            def progress():
                nonlocal instructions
                instructions += 1
                return 0
            queue.connection.set_progress_handler(progress, 1)
        after, examined, polls = 0, 0, 0
        started = time.perf_counter_ns()
        while True:
            result = getattr(queue, method)("benchmark", after_sequence=after, scan_limit=128)
            examined += result.examined
            polls += 1
            if result.lease is not None:
                break
            if result.next_after is None:
                raise AssertionError("eligible work was lost")
            after = result.next_after
        elapsed = time.perf_counter_ns() - started
        queue.connection.set_progress_handler(None, 0)
        if result.lease.task != "eligible" or result.lease.token != 1:
            raise AssertionError("dispatch violated the independent first-eligible oracle")
        if queue.connection.execute("SELECT 1 FROM tasks WHERE id LIKE 'blocked-%' AND state!='queued'").fetchone():
            raise AssertionError("a dependency-blocked task was claimed")
        return {"elapsed_ns": elapsed, "examined": examined, "polls": polls,
                "sqlite_vm_steps": instructions if instrument else None}
    finally:
        queue.close()


def run(sizes, repeats, temp_root=None):
    rows = []
    with tempfile.TemporaryDirectory(prefix="fluxr-ready-bench-", dir=temp_root) as directory:
        root = Path(directory)
        store = ContentStore(root / "objects")
        for blocked in sizes:
            source = root / f"source-{blocked}.sqlite"
            fixture(source, store, blocked)
            data = {name: [] for name in ("claim_next", "claim_ready")}
            for repeat in range(repeats):
                # Alternate order to reduce systematic warm-cache/order bias.
                for name in list(data)[::1 if repeat % 2 == 0 else -1]:
                    data[name].append(observe(source, root / f"{blocked}-{repeat}-{name}.sqlite", store, name))
            measured = {}
            for name, samples in data.items():
                counters = observe(source, root / f"{blocked}-{name}-steps.sqlite", store, name, True)
                measured[name] = {"median_ns": statistics.median(item["elapsed_ns"] for item in samples),
                                  "samples_ns": [item["elapsed_ns"] for item in samples],
                                  "examined": counters["examined"], "polls": counters["polls"],
                                  "sqlite_vm_steps_separate_instrumented_pass": counters["sqlite_vm_steps"]}
            rows.append({"blocked_jobs": blocked, "eligible_jobs": 1, "policies": measured,
                         "legacy_over_indexed_median": measured["claim_next"]["median_ns"] / measured["claim_ready"]["median_ns"]})
    return {"schema": "fluxr-dependency-ready-benchmark-v1", "python": sys.version,
            "sqlite": sqlite3.sqlite_version, "platform": platform.platform(), "repeats": repeats,
            "checks": "identical source states choose eligible exactly once; all children remain queued",
            "measurement": "dispatch only; setup/backup/open and separate VM-step pass excluded from wall time",
            "scope": "single-host blocked-prefix scheduler microbenchmark; no agent-scale throughput claim",
            "rows": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--blocked", type=int, nargs="+", default=[256, 2048, 8192])
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--temp-root", type=Path)
    args = parser.parse_args()
    if not 1 <= args.repeats <= 100 or any(not 0 <= n <= 100_000 for n in args.blocked):
        parser.error("use 1–100 repeats and 0–100000 blocked jobs")
    print(json.dumps(run(args.blocked, args.repeats, args.temp_root), indent=2))


if __name__ == "__main__":
    main()
