#!/usr/bin/env python3
"""Measure the local fair queue under fixed and expanding CPU affinity budgets.

Reviewed deterministic hash fixtures execute outside queue transactions. These
are synthetic controller jobs, not coding agents or accepted user requirements.
Every attempted trial is retained; raw calls and transaction times are reported.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import multiprocessing
import os
from pathlib import Path
import platform
import resource
import sqlite3
import statistics
import sys
import tempfile
import time
import tracemalloc

from agent_pipeline import ContentStore, WorkQueue
from bench_agent_ready import spec


def quantiles(values):
    ordered = sorted(values)
    if not ordered:
        return {"n": 0, "p50": None, "p95": None, "p99": None, "max": None}
    return {"n": len(ordered), "p50": statistics.median(ordered),
            "p95": ordered[(95 * len(ordered) - 1) // 100],
            "p99": ordered[(99 * len(ordered) - 1) // 100], "max": ordered[-1]}


def fixture_result(index, rounds):
    value = index.to_bytes(8, "little") * 4096
    for _ in range(rounds):
        value = hashlib.sha256(value).digest() * 1024
    return hashlib.sha256(value).hexdigest()


def policy_overhead(root, count, repeats):
    """Same ready-set/order, explicit cursor versus durable cursor overhead.

    Wall-time observations exclude fixture creation, backup and queue opening.
    VM steps and Python heap peaks are a separate instrumented pass.
    """
    store = ContentStore(root / "objects")
    seed = root / "seed.sqlite"
    queue = WorkQueue(seed, store, lambda: 1000)
    for index in range(count):
        queue.enqueue(f"job-{index:04}", spec())
    queue.enable_fair_dispatch()
    queue.close()
    observations = []
    for repeat in range(repeats + 1):
        for policy in ("claim_ready", "claim_fair")[::1 if repeat % 2 == 0 else -1]:
            path = root / f"{repeat}-{policy}.sqlite"
            with sqlite3.connect(seed) as source, sqlite3.connect(path) as target:
                source.backup(target)
            queue = WorkQueue(path, store, lambda: 1000)
            instrument = repeat == repeats
            steps = 0

            def step():
                nonlocal steps
                steps += 1
                return 0

            if instrument:
                queue.connection.set_progress_handler(step, 1)
                tracemalloc.start()
            after, names, examined = 0, [], 0
            start = time.perf_counter_ns()
            for _ in range(count):
                if policy == "claim_fair":
                    result = queue.claim_fair("overhead", scan_limit=8)
                else:
                    result = queue.claim_ready("overhead", scan_limit=8, after_sequence=after)
                    after = result.next_after or 0
                if result.lease is None:
                    raise AssertionError("identical ready fixture lost a runnable task")
                names.append(result.lease.task)
                examined += result.examined
            elapsed = time.perf_counter_ns() - start
            peak = tracemalloc.get_traced_memory()[1] if instrument else None
            if instrument:
                tracemalloc.stop()
                queue.connection.set_progress_handler(None, 0)
            queue.close()
            if names != [f"job-{index:04}" for index in range(count)]:
                raise AssertionError("same-state policy order changed")
            observations.append({"policy": policy, "repeat": repeat, "instrumented": instrument,
                "jobs": count, "examined": examined, "elapsed_ns": elapsed,
                "sqlite_vm_steps": steps if instrument else None,
                "python_tracemalloc_peak_bytes": peak})
    return {"scope": "same-state uncontended claim-only overhead; packet/setup/backup/open excluded; native SQLite memory not measured",
            "observations": observations}


class TimedConnection:
    def __init__(self, connection):
        self.connection = connection
        self.transactions = []
        self.begin = None
        self.wait = None

    def __getattr__(self, name):
        return getattr(self.connection, name)

    def execute(self, sql, *args):
        start = time.perf_counter_ns()
        result = self.connection.execute(sql, *args)
        end = time.perf_counter_ns()
        if sql == "BEGIN IMMEDIATE":
            self.wait, self.begin = end - start, end
        elif sql in {"COMMIT", "ROLLBACK"} and self.begin is not None:
            self.transactions.append({"begin_wait_ns": self.wait,
                "writer_held_ns": end - self.begin, "commit_or_rollback_ns": end - start,
                "ending": sql})
            self.begin = None
        return result


def worker(database, objects, affinity, count, rounds, barrier, output):
    queue = None
    report = {"pid": os.getpid(), "affinity": affinity, "calls": [], "jobs": [], "error": None}
    try:
        os.sched_setaffinity(0, set(affinity))
        store = ContentStore(objects)
        queue = WorkQueue(database, store)
        connection = TimedConnection(queue.connection)
        queue.connection = connection
        barrier.wait(timeout=20)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            start = time.perf_counter_ns()
            result = queue.claim_fair(f"worker-{os.getpid()}", ttl_seconds=60, scan_limit=8)
            dispatch_ns = time.perf_counter_ns() - start
            report["calls"].append({"dispatch_ns": dispatch_ns, "epoch": result.epoch,
                "examined": result.examined, "deferred_scopes": result.deferred_scopes,
                "deferred_dependencies": result.deferred_dependencies,
                "sweep_complete": result.sweep_complete,
                "task": result.lease.task if result.lease else None})
            if result.lease:
                index = int(result.lease.task.split("-")[1])
                run_start = time.perf_counter_ns()
                digest = fixture_result(index, rounds)
                run_ns = time.perf_counter_ns() - run_start
                artifact_bytes = json.dumps({"index": index, "sha256": digest}, sort_keys=True).encode()
                evidence = store.put(artifact_bytes)
                receipt = queue.complete(result.lease, {"schema": "fluxr-job-receipt-v1",
                    "action_key": result.lease.action_key, "source_snapshot": result.lease.source_snapshot,
                    "outcome": "succeeded", "exit_code": 0, "checks_complete": True, "evidence": [evidence]})
                completed_ns = time.perf_counter_ns() - start
                report["jobs"].append({"task": result.lease.task, "token": result.lease.token,
                    "digest": digest, "work_ns": run_ns, "receipt": receipt,
                    "claim_through_complete_ns": completed_ns,
                    "receipt_utf8": store.get(receipt).decode("utf-8"),
                    "evidence_object": {"digest": evidence, "utf8": artifact_bytes.decode("utf-8")}})
            else:
                completed = queue.connection.execute("SELECT count(*) FROM tasks WHERE state='succeeded'").fetchone()[0]
                if completed == count:
                    break
                if result.sweep_complete:
                    time.sleep(0.0005)  # Bounded overload backoff, outside every writer transaction.
        else:
            raise TimeoutError("bounded fixture campaign exceeded 20 seconds")
        report["transactions"] = connection.transactions
        report["max_rss_kib"] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    except BaseException as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        if queue is not None:
            queue.close()
        output.put(report)


def trial(root, workers, count, rounds, cpus, mode):
    database, store = root / "queue.sqlite", ContentStore(root / "objects")
    queue = WorkQueue(database, store)
    queue.enqueue("scope-holder", spec(), exclusive_scopes=["path:hot"])
    queue.claim("scope-holder", "fixture-holder", 60)
    for i in range(16):
        queue.enqueue(f"blocked-{i}", spec(), exclusive_scopes=["path:hot/nested"])
    for index in range(count):
        task_spec = spec()
        task_spec["argv"] = ["reviewed-sha256-fixture", str(index), str(rounds)]
        queue.enqueue(f"job-{index:04}", task_spec,
                      exclusive_scopes=["path:shared"] if index % 4 == 0 else [])
    queue.enable_fair_dispatch()
    context = multiprocessing.get_context("spawn")
    barrier, output = context.Barrier(workers + 1), context.Queue()
    affinities = [[cpus[0]]] * workers if mode == "fixed_total_cpu" else [[cpus[i]] for i in range(workers)]
    processes = [context.Process(target=worker, args=(str(database), str(store.root), affinities[i],
        count, rounds, barrier, output)) for i in range(workers)]
    start = time.perf_counter_ns()
    reports, controller_error = [], None
    dispatch_start = start
    try:
        for process in processes:
            process.start()
        barrier.wait(timeout=20)
        dispatch_start = time.perf_counter_ns()
        for _ in processes:
            reports.append(output.get(timeout=30))
        for process in processes:
            process.join(timeout=10)
    except BaseException as error:
        controller_error = f"{type(error).__name__}: {error}"
    finally:
        for process in processes:
            if process.pid is not None and process.is_alive():
                process.terminate()
                process.join(timeout=10)
    finish = time.perf_counter_ns()
    errors = [item["error"] for item in reports if item["error"]]
    if controller_error:
        errors.append(controller_error)
    if any(process.exitcode != 0 for process in processes):
        errors.append("missing, interrupted or failed fixture worker")
    jobs = [job for item in reports for job in item["jobs"]]
    names = [job["task"] for job in jobs]
    if len(jobs) != count or len(set(names)) != count:
        errors.append("fixture job count/uniqueness mismatch")
    for index in range(count):
        name = f"job-{index:04}"
        expected = fixture_result(index, rounds)
        row = queue.inspect(name)
        if row["state"] != "succeeded" or row["token"] != 1:
            errors.append(f"wrong queue terminal state: {name}")
            continue
        receipt = json.loads(store.get(row["result_digest"]))
        actual = json.loads(store.get(receipt["evidence"][0]))
        if actual != {"index": index, "sha256": expected}:
            errors.append(f"independent fixture output mismatch: {name}")
    for job in jobs:
        if (hashlib.sha256(job["receipt_utf8"].encode()).hexdigest() != job["receipt"]
                or hashlib.sha256(job["evidence_object"]["utf8"].encode()).hexdigest()
                != job["evidence_object"]["digest"]):
            errors.append(f"retained receipt/artifact bytes mismatch: {job['task']}")
    if queue.connection.execute("SELECT count(*) FROM tasks WHERE id LIKE 'blocked-%' AND state='queued'").fetchone()[0] != 16:
        errors.append("hot-scope blocked work was wrongly claimed")
    calls = [call for item in reports for call in item["calls"]]
    transactions = [tx for item in reports for tx in item.get("transactions", [])]
    result = {"workers": workers, "resource_mode": mode, "cpu_affinities": affinities,
        "fixture_jobs": count, "fixture_rounds": rounds, "held_scope_jobs": 16,
        "validated_fixture_completions": count if not errors else None,
        "worker_exit_codes": [p.exitcode for p in processes], "errors": errors,
        "spawn_through_collect_ns": finish - start, "barrier_through_collect_ns": finish - dispatch_start,
        "validated_fixture_jobs_per_second": count * 1e9 / (finish - dispatch_start) if not errors else None,
        "claim_through_complete_ns": quantiles([job["claim_through_complete_ns"] for job in jobs]),
        "dispatch_ns": quantiles([call["dispatch_ns"] for call in calls]),
        "writer_held_ns": quantiles([tx["writer_held_ns"] for tx in transactions]),
        "begin_wait_ns": quantiles([tx["begin_wait_ns"] for tx in transactions]),
        "examined": sum(call["examined"] for call in calls),
        "deferred_scopes": sum(call["deferred_scopes"] for call in calls),
        "raw_workers": reports}
    queue.close()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workers", type=int, nargs="+", default=[1, 2, 4])
    parser.add_argument("--jobs", type=int, default=96)
    parser.add_argument("--rounds", type=int, default=64)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--temp-root", type=Path)
    args = parser.parse_args()
    if not hasattr(os, "sched_setaffinity"):
        parser.error("this resource-control benchmark requires Linux CPU affinity")
    cpus = sorted(os.sched_getaffinity(0))
    if (any(not 1 <= n <= min(16, len(cpus)) for n in args.workers)
            or not 1 <= args.jobs <= 4096 or not 1 <= args.rounds <= 4096 or not 1 <= args.repeats <= 20):
        parser.error("bounded workers/jobs/rounds/repeats exceeded")
    reports = []
    with tempfile.TemporaryDirectory(prefix="fluxr-fair-bench-", dir=args.temp_root) as directory:
        overhead_root = Path(directory) / "overhead"
        overhead_root.mkdir()
        overhead = policy_overhead(overhead_root, args.jobs, args.repeats)
        for repeat in range(args.repeats):
            treatments = [(mode, n) for mode in ("fixed_total_cpu", "expanding_cpu") for n in args.workers]
            for mode, n in treatments[::1 if repeat % 2 == 0 else -1]:
                root = Path(directory) / f"{repeat}-{mode}-{n}"
                root.mkdir()
                try:
                    report = trial(root, n, args.jobs, args.rounds, cpus, mode)
                except BaseException as error:
                    report = {"workers": n, "resource_mode": mode, "errors": [f"{type(error).__name__}: {error}"]}
                reports.append(dict(report, repeat=repeat))
    result = {"schema": "fluxr-fair-ready-benchmark-v1", "python": sys.version,
        "sqlite": sqlite3.sqlite_version, "platform": platform.platform(), "affinity_available": cpus,
        "measurements": "raw local fixture jobs; all worker attempts retained; empirical quantiles, no confidence claim",
        "scope": "single-host scheduler/synthetic fixture throughput; no autonomous coding, model tokens, certified requirements or fleet-scaling claim",
        "resource_bounds": "fixed one CPU shared by all workers versus one distinct allowed CPU per worker; same job count; process RSS observed, not a hard memory quota; filesystem and neighboring workloads uncontrolled",
        "clock_and_authority": "monotonic durations; wall-clock lease floor; one SQLite writer; queue transaction is not source publication",
        "same_state_policy_overhead": overhead,
        "trials": reports}
    print(json.dumps(result, indent=2))
    return 1 if any(report["errors"] for report in reports) else 0


if __name__ == "__main__":
    raise SystemExit(main())
