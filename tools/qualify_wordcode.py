#!/usr/bin/env python3
"""Execute reviewed WK1 fixtures through one unchanged bounded worker binary.

All output is retained outside the checkout. This is a local integration and
overhead experiment, not generated-candidate admission or a live model episode.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import time
import tomllib

from wordcode_tiering import TieredSession, TierError

ROOT = Path(__file__).resolve().parents[1]


def identity(path):
    data = Path(path).read_bytes()
    return {"path": str(Path(path).resolve()), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def write(path, data):
    path.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n")


def memory(pid):
    observations = {}
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in ("VmRSS", "VmHWM", "VmPeak", "Threads"):
            words = value.split()
            observations[key] = int(words[0]) * (1024 if words[-1] == "kB" else 1)
    raw = Path(f"/proc/{pid}/stat").read_text()
    fields = raw[raw.rfind(")") + 2:].split()
    ticks = {"user": int(fields[11]), "system": int(fields[12]), "ticks_per_second": os.sysconf("SC_CLK_TCK")}
    return {"pid": pid, "observations": observations, "cumulative_cpu_ticks": ticks,
            "scope": "Linux process resident/high-water/virtual and cumulative CPU tick observations before shutdown, including launcher; not allocator accounting or continuous CPU utilization"}


def latency(rows):
    values = sorted(row["host_elapsed_seconds"] for row in rows)
    if not values:
        raise ValueError("empty latency population")
    return {"calls": len(values), "median_seconds": statistics.median(values),
            "p95_seconds": values[math.ceil(.95 * len(values)) - 1],
            "p99_seconds": values[math.ceil(.99 * len(values)) - 1], "max_seconds": values[-1],
            "total_seconds": sum(values), "calls_per_measured_second": len(values) / sum(values),
            "scope": "Individual serialized process-IPC calls on this fixed fixture; no real-time qualification"}


def command(argv, output, name):
    start = time.monotonic()
    try:
        result = subprocess.run(list(map(str, argv)), cwd=ROOT, capture_output=True, timeout=120)
        stdout, stderr, code, status = result.stdout, result.stderr, result.returncode, "finished"
    except subprocess.TimeoutExpired as error:
        stdout, stderr, code, status = error.stdout or b"", error.stderr or b"", None, "timeout"
    (output / (name + ".stdout")).write_bytes(stdout)
    (output / (name + ".stderr")).write_bytes(stderr)
    record = {"argv": list(map(str, argv)), "exit_code": code, "status": status,
              "elapsed_seconds": time.monotonic() - start,
              "stdout": identity(output / (name + ".stdout")),
              "stderr": identity(output / (name + ".stderr"))}
    write(output / (name + ".command.json"), record)
    if code != 0:
        raise RuntimeError(str(record))
    return json.loads(stdout), record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--bench", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.binary, args.bench, args.output = (p.resolve() for p in (args.binary, args.bench, args.output))
    if args.output.exists() or args.output.is_relative_to(ROOT):
        raise ValueError("new evidence directory must be outside checkout")
    args.output.mkdir(parents=True)
    report = {"schema": "fluxr-wordcode-qualification-v1", "status": "running",
              "worker": identity(args.binary), "benchmark": identity(args.bench),
              "commands": [], "cases": [], "failures": [], "live_agent_trials": 0,
              "heldout_tasks": 0, "runtime_binary_unchanged": False,
              "scope": "Reviewed authored fixtures; bounded compiler/VM process execution; not filesystem/network isolation, evaluator authority, real-time callbacks or general Rust parity"}
    try:
        core, record = command([args.binary, "dictionary"], args.output, "dictionary")
        report["commands"].append(record)
        write(args.output / "dictionary.json", core)
        bench, record = command([args.bench], args.output, "benchmark")
        report["commands"].append(record)
        expected_names = ["affine", "polynomial", "repeat", "walk", "forest"]
        if (bench.get("status") != "passed" or [row.get("name") for row in bench.get("cases", [])] != expected_names
                or bench.get("binary_sha256") != report["benchmark"]["sha256"]):
            raise ValueError("in-process benchmark omitted cases")
        write(args.output / "benchmark.json", bench)
        catalog = tomllib.loads((ROOT / "benchmarks/wordcode-v14.toml").read_text())
        if [case.get("id") for case in catalog["cases"]] != ["WK14-" + name for name in expected_names]:
            raise ValueError("catalog population differs from all five required cases")
        for case in catalog["cases"]:
            name = case["id"].removeprefix("WK14-")
            source = (ROOT / case["wordcode"]).read_text()
            dictionary_json = (ROOT / case["dictionary"]).read_text() if "dictionary" in case else None
            dictionary = case.get("dictionary_sha256", core["dictionary_sha256"])
            expanded, record = command([args.binary, "expand", ROOT / case["wordcode"],
                                        *([ROOT / case["dictionary"]] if dictionary_json else [])], args.output, name + "-expand")
            report["commands"].append(record)
            matched = next(row for row in bench["cases"] if row["name"] == name)
            if (expanded["module_sha256"] != matched["module_sha256"]
                    or expanded["source_sha256"] != matched["source_sha256"]
                    or dictionary != matched["dictionary_sha256"]):
                raise ValueError("independent process and in-process artifact identities differ")
            session = None
            rows, native, error_rows = [], [], []
            row = {"name": name, "class": case["class"], "source": identity(ROOT / case["wordcode"]),
                   "expected": case["expected"], "status": "running",
                   "canonical_artifact_matches_in_process": True, "failures": []}
            report["cases"].append(row)
            try:
                start = time.monotonic()
                session = TieredSession(args.binary, source, dictionary, "calc", dictionary_json=dictionary_json)
                row["setup_seconds"] = time.monotonic() - start
                arguments = [{"i64": value} for value in case["arguments"]]
                baseline = session.call(arguments)
                if baseline["value"] != {"i64": case["expected"]} or baseline["route"] != "Portable":
                    raise ValueError("portable arithmetic mismatch")
                # Exercise cancellation and stale admission using real child processes.
                if name == "affine":
                    cancelled = session.begin_compile()
                    session.cancel(cancelled)
                    try:
                        session.poll(cancelled)
                    except TierError:
                        row["cancelled_ticket_refused"] = True
                    else:
                        raise ValueError("cancelled ticket accepted")
                ticket = session.begin_compile()
                try:
                    session.begin_compile()
                except TierError:
                    row["backpressure_refused_second_compile"] = True
                else:
                    raise ValueError("second pending compiler admitted")
                for _ in range(1500):
                    before = time.time_ns()
                    result = session.call(arguments)
                    after = time.time_ns()
                    result["host_started_unix_ns"] = before
                    result["host_finished_unix_ns"] = after
                    rows.append(result)
                    if result["error"] is not None or result["value"] != baseline["value"] or result["route"] != "Portable":
                        raise ValueError("pending portable execution differed")
                    if session.poll(ticket):
                        break
                else:
                    raise ValueError("no tier completion within admitted call population")
                native_ready = dict(session.native.ready)
                native = [session.call(arguments) for _ in range(100)]
                if any(result["error"] is not None or result["value"] != baseline["value"] for result in native):
                    raise ValueError("prepared worker changed the arithmetic result")
                row["pending_portable_calls"] = len(rows)
                row["pending_portable_latency"] = latency(rows)
                row["prepared_latency"] = latency(native)
                row["actual_routes"] = sorted({result["route"] for result in native})
                row["native_ready"] = native_ready
                # Wall timestamps are observed on this one host; never infer
                # overlap merely from an unconsumed readiness message.
                row["initialization_overlap_calls"] = sum(
                    result["host_started_unix_ns"] < native_ready["ready_unix_ns"]
                    and result["host_finished_unix_ns"] > native_ready["initialization_started_unix_ns"] for result in rows)
                row["host_calls_overlapping_preparation_to_ready_interval"] = sum(
                    result["host_started_unix_ns"] < native_ready["ready_unix_ns"]
                    and result["host_finished_unix_ns"] > native_ready["preparation_started_unix_ns"] for result in rows)
                row["clock_scope"] = "same-host observed wall intervals; preparation-to-ready includes readiness construction; host IPC call overlap is not proof of simultaneous CPU/VM execution or a cross-host clock guarantee"
                row["process_memory"] = [memory(session.portable.process.pid), memory(session.native.process.pid)]
                # Budget failures must retain exact portable errors and never be
                # treated as an executable native success.
                for fuel in range(15):
                    limits = {"instructions": fuel, "cells": 1000, "frames": 8}
                    portable = session.portable.call([], arguments, limits)
                    prepared = session.call(arguments, limits=limits)
                    if (portable["value"], portable["error"]) != (prepared["value"], prepared["error"]):
                        raise ValueError("first-trap/budget result changed across workers")
                    error_rows.append({"fuel": fuel, "portable": portable, "prepared": prepared})
                row["status"] = "passed"
            except BaseException as error:
                row["status"] = "incomplete"
                row["failures"].append(type(error).__name__ + ": " + str(error))
                raise
            finally:
                if session is not None:
                    try:
                        session.close()
                    except BaseException as error:
                        row["status"] = "incomplete"
                        row["failures"].append("cleanup: " + type(error).__name__ + ": " + str(error))
                    row["session"] = session.receipt()
                write(args.output / (name + "-calls.json"), {"pending": rows, "prepared": native, "budget_differential": error_rows})
                row["call_evidence"] = identity(args.output / (name + "-calls.json"))
                write(args.output / (name + "-case.json"), row)
        report["runtime_binary_unchanged"] = identity(args.binary) == report["worker"]
        report["benchmark_binary_unchanged"] = identity(args.bench) == report["benchmark"]
        if not report["runtime_binary_unchanged"] or not report["benchmark_binary_unchanged"]:
            raise ValueError("worker executable changed")
        if any(row["status"] != "passed" for row in report["cases"]):
            raise ValueError("incomplete case or cleanup")
        if not any("TypedJit" in row["actual_routes"] for row in report["cases"]):
            raise ValueError("no actual typed JIT invocation observed")
        report["status"] = "passed"
    except BaseException as error:
        report["status"] = "incomplete"
        report["failures"].append(type(error).__name__ + ": " + str(error))
    write(args.output / "report.json", report)
    print(json.dumps({"status": report["status"], "cases": len(report["cases"]), "failures": report["failures"]}))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
