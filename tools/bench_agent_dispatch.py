#!/usr/bin/env python3
"""Measure bounded local dispatch with real active scopes and large packets.

Run this SAME script with PYTHONPATH pointing at each checkout's tools directory
to compare implementations. Fixture setup, result storage and completion are
outside the dispatch timing. Results are local microbenchmarks, not distributed
throughput or a claim about unlimited agents.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import statistics
import sys
import tempfile
import time
import tracemalloc

# Explicit PYTHONPATH comes first so an archived baseline can run this script.
_tools_directory = Path(__file__).resolve().parent
sys.path[:] = [entry for entry in sys.path if Path(entry).resolve() != _tools_directory]
sys.path.append(str(_tools_directory))
from agent_pipeline import ContentStore, WorkQueue


def spec(packet_bytes):
    return {"schema": "fluxr-action-v1", "argv": ["fixture"], "working_directory": ".",
            "environment": {"FIXTURE_DATA": "x" * packet_bytes}, "inputs": {}, "outputs": [],
            "source_snapshot": "1" * 64, "toolchain": "2" * 64, "runner": "3" * 64,
            "platform": sys.platform, "target": "fixture", "profile": "benchmark", "policy": "4" * 64}


def measure(active, queued, packet_bytes, repeats):
    with tempfile.TemporaryDirectory(prefix="fluxr-dispatch-bench-") as directory:
        root = Path(directory)
        store = ContentStore(root / "objects")
        queue = WorkQueue(root / "queue.sqlite", store, lambda: 1000)
        try:
            for index in range(active):
                task = f"active-{index}"
                queue.enqueue(task, spec(0), exclusive_scopes=[f"path:active/{index}"])
                queue.claim(task, task)
            large = spec(packet_bytes)
            for index in range(queued):
                queue.enqueue(f"ready-{index}", large, exclusive_scopes=[f"path:ready/{index}"])
            artifact = store.put(b"benchmark fixture; not language execution evidence")
            elapsed, peaks = [], []
            for index in range(repeats):
                tracemalloc.start()
                start = time.perf_counter_ns()
                dispatch = queue.claim_next("probe", scan_limit=queued)
                duration = time.perf_counter_ns() - start
                _, peak = tracemalloc.get_traced_memory()
                tracemalloc.stop()
                lease = dispatch.lease
                if lease is None or lease.task != f"ready-{index}" or dispatch.examined != 1:
                    raise AssertionError("dispatch violated the independent FIFO fixture oracle")
                queue.complete(lease, {"schema": "fluxr-job-receipt-v1", "action_key": lease.action_key,
                    "source_snapshot": lease.source_snapshot, "outcome": "succeeded", "exit_code": 0,
                    "checks_complete": True, "evidence": [artifact]})
                elapsed.append(duration)
                peaks.append(peak)
            return {"active_scopes": active, "queued_at_start": queued,
                    "packet_environment_bytes": packet_bytes, "repeats": repeats,
                    "dispatch_ns": elapsed, "median_dispatch_ns": int(statistics.median(elapsed)),
                    "peak_python_allocation_bytes": peaks, "max_peak_python_allocation_bytes": max(peaks),
                    "oracle": "exact FIFO lease, one examined job, successful fenced completion"}
        finally:
            queue.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--active", default="0,100,1000")
    parser.add_argument("--queued", type=int, default=128)
    parser.add_argument("--packet-bytes", type=int, default=65536)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    active = [int(value) for value in args.active.split(",")]
    if (not active or any(n < 0 or n > 10000 for n in active)
            or not 1 <= args.repeats <= args.queued <= 4096 or not 0 <= args.packet_bytes <= 1024 * 1024):
        parser.error("bounded active (0..10000), repeats <= queued (1..4096), packet bytes (0..1 MiB) required")
    import agent_pipeline
    result = {"schema": "fluxr-dispatch-benchmark-v1", "python": sys.version,
              "implementation": str(Path(agent_pipeline.__file__).resolve()),
              "instrumentation": "perf_counter_ns with tracemalloc enabled; fixture setup excluded",
              "samples": [measure(n, args.queued, args.packet_bytes, args.repeats) for n in active]}
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
