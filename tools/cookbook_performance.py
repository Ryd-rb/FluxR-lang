#!/usr/bin/env python3
"""Capture and compare the finite Rust/FluxR cookbook benchmark.

Only an explicitly selected, prebuilt local binary is executed. Run capture
under the resource coordinator; its bounded POSIX collector is not a sandbox
for hostile native code. Failed attempts retain their bounded outputs. Offline
validation checks consistency, not authenticity or population-tail guarantees.
"""
from __future__ import annotations

import argparse
from dataclasses import asdict
import json
import os
from pathlib import Path
import re
import shlex
import sys
import time

import fragment_performance as fp
from eval.runner import file_digest, run_bounded

ROOT = Path(__file__).resolve().parents[1]
MAX_STDOUT = 64 * 1024 * 1024
MAX_STDERR = 64 * 1024
MAX_FILE = 16 * 1024 * 1024
MAX_NS = 3_600_000_000_000
STARTUPS = 7
LIMITS = {"instructions": 50_000, "cells": 16_384, "frames": 64}
FLAGS = fp.FLAGS + ("CARGO_BUILD_TARGET", "CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS")
BASE_LANES = ["rust_kernel", "rust_value_adapter", "portable", "named"]
WITNESSES = {"Portable", "TypedJit", "NativeScalar", "PrecompiledScan", "AheadOfTime"}
INDIVIDUAL = "ns per invocation; timer and harness dispatch included; post-call checking/conversion/destruction excluded"
BATCH = "ns per call averaged over a batch; result destruction included"
ROW_ORDER = "individual: sample modulo rows; batches: (round * calls_per_batch + call) modulo rows; identical for every lane"
PHASES = {
    "source_compile": "ns per source parse/check/compile; source format identified separately",
    "artifact_decode": "ns per artifact decode/check",
    "cold_prepare": "ns per cold preparation; code publication included when admitted",
    "first_call": "ns per first call with fresh workspace; excludes preparation",
}
CALIBRATION = {
    "empty_timer": "ns per empty timing observation; not subtracted",
    "dispatch": "ns per dynamic identity closure; not subtracted",
    "batch_loop": "ns per empty loop/selection/dispatch; not subtracted",
}
STARTUP_UNIT = "ns per bounded startup-only process spawn, collect and reap"
KERNEL_SCOPE = ("Exact imported Rust reference, preconverted typed input, opaque inputs, dynamic closure and typed result enum. "
                "Value conversion and logical metering excluded.")
ADAPTER_SCOPE = "Same Rust reference with per-call Value input/output adapters; logical metering excluded."
LAUNCH = ("import os,sys; os.environ.update(zip(('FLUXR_COOKBOOK_SAMPLES',"
          "'FLUXR_COOKBOOK_ROUNDS','FLUXR_COOKBOOK_BATCH_CALLS',"
          "'FLUXR_COOKBOOK_COMMON_ROUTES'),sys.argv[1:5])); os.execv(sys.argv[5],[sys.argv[5]])")


def canonical(value):
    """Match serde_json's ordered object encoding for row-contract digests."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode()


def read_bytes(path, maximum=MAX_FILE):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError("measurement input must be a regular non-symlink file: " + str(path))
    with path.open("rb") as stream:
        value = stream.read(maximum + 1)
    if len(value) > maximum:
        raise ValueError("measurement input exceeds its byte bound")
    return value


def source_manifest(source):
    """Resolve the actual shipped catalog, including every source/reference pair."""
    source = Path(source).resolve()
    root = source / "examples/cookbook"
    raw = read_bytes(root / "catalog.json")
    catalog = fp.decode(raw)
    entries = catalog.get("cases") if isinstance(catalog, dict) else None
    if (not isinstance(catalog, dict) or catalog.get("format") != "fluxr-cookbook-v1" or not isinstance(entries, list)
            or not 1 <= len(entries) <= 256):
        raise ValueError("invalid actual cookbook catalog")
    cases, names = [], set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise ValueError("catalog cases must be objects")
        name = entry.get("name")
        if not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,127}", name) or name in names:
            raise ValueError("duplicate or invalid cookbook name")
        names.add(name)
        row = {"name": name, "topics": entry.get("topics")}
        for field, extension, digest in (("fluxr", ".fluxr", "source_sha256"),
                                         ("rust", ".rs", "rust_sha256"),
                                         ("compact", ".compact", "compact_sha256")):
            if entry.get(field) != name + extension:
                raise ValueError("catalog source paths must name their exact paired files")
            row[digest] = fp.sha(read_bytes(root / entry[field]))
        cases.append(row)
    if {path.name for path in root.glob("*.fluxr")} != {name + ".fluxr" for name in names}:
        raise ValueError("catalog does not cover every cookbook FluxR source")
    result = {"format": "fluxr-cookbook-measurement-inputs-v1", "catalog_sha256": fp.sha(raw),
              "probe_sha256": fp.sha(read_bytes(source / "examples/bench_cookbook.rs")),
              "support_sha256": fp.sha(read_bytes(root / "support.rs")), "cases": cases}
    validate_manifest(result)
    return result


def validate_manifest(value):
    if not isinstance(value, dict) or value.get("format") != "fluxr-cookbook-measurement-inputs-v1":
        raise ValueError("missing actual cookbook source manifest")
    for key in ("catalog_sha256", "probe_sha256", "support_sha256"):
        if not fp.digest_valid(value.get(key)):
            raise ValueError("invalid cookbook manifest digest: " + key)
    rows = value.get("cases")
    if not isinstance(rows, list) or not 1 <= len(rows) <= 256:
        raise ValueError("invalid cookbook manifest case count")
    names = set()
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("invalid cookbook manifest case")
        name = row.get("name")
        if not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,127}", name) or name in names:
            raise ValueError("duplicate or invalid cookbook manifest case")
        names.add(name)
        topics = row.get("topics")
        if not isinstance(topics, list) or not topics or not all(isinstance(t, str) and 0 < len(t) <= 128 for t in topics):
            raise ValueError("missing cookbook coverage topics")
        for key in ("source_sha256", "rust_sha256", "compact_sha256"):
            if not fp.digest_valid(row.get(key)):
                raise ValueError("invalid cookbook case source digest")
    return value


def tree_identity(source):
    source = Path(source).resolve()
    rows = []
    total = 0
    for directory, dirs, files in os.walk(source, followlinks=False):
        if any((Path(directory) / name).is_symlink() for name in dirs):
            raise ValueError("source snapshot contains a directory symlink")
        dirs[:] = sorted(name for name in dirs if name not in {".git", "target", "__pycache__", ".pytest_cache"})
        for name in sorted(files):
            if name == ".git":
                continue  # A worktree's Git routing file is not product source.
            path = Path(directory) / name
            data = read_bytes(path)
            total += len(data)
            if total > 256 * 1024 * 1024 or len(rows) >= 20_000:
                raise ValueError("source snapshot exceeds the bounded source inventory")
            rows.append([path.relative_to(source).as_posix(), len(data), fp.sha(data)])
    return {"files": len(rows), "bytes": total, "sha256": fp.sha(canonical(rows)),
            "cargo_lock_sha256": fp.sha(read_bytes(source / "Cargo.lock"))}


def validate_value(value):
    pending, cells = [(value, 0)], 0
    while pending:
        value, depth = pending.pop()
        cells += 1
        if depth > 32 or cells > 262_144:
            raise ValueError("row value exceeds depth/cell bounds")
        if value == "unit":
            continue
        if not isinstance(value, dict) or len(value) != 1:
            raise ValueError("row values require exact tagged Value encodings")
        kind, item = next(iter(value.items()))
        if kind == "bool" and type(item) is bool:
            continue
        if kind == "i64" and fp.integer(item, -(2**63), 2**63-1):
            continue
        if kind == "u64" and fp.integer(item, 0, 2**64-1):
            continue
        if kind == "f32" and fp.integer(item, 0, 2**32-1):
            continue
        if kind in ("record", "list") and isinstance(item, list):
            pending.extend((child, depth+1) for child in item)
            continue
        if (kind == "variant" and isinstance(item, list) and len(item) == 2
                and fp.integer(item[0], 0, 2**32-1)):
            if item[1] is not None:
                pending.append((item[1], depth+1))
            continue
        raise ValueError("invalid typed row value")


def timing(value, count, unit):
    fp.validate_statistics(value, count)
    if value["observation_unit"] != unit or any(sample > MAX_NS for sample in value["samples_ns"]):
        raise ValueError("timing unit or bounded duration differs")


def exact_limits(value):
    return (isinstance(value, dict) and value == LIMITS
            and all(type(limit) is int for limit in value.values()))


def validate_report(report, manifest):
    validate_manifest(manifest)
    if (not isinstance(report, dict) or report.get("schema") != "fluxr-cookbook-performance-v1"
            or report.get("catalog") != "fluxr-cookbook-v1" or report.get("telemetry") != "off"
            or report.get("debug_assertions") is not False
            or report.get("failed_cases") != 0 or type(report.get("failed_cases")) is not int
            or not exact_limits(report.get("limits"))):
        raise ValueError("incomplete or incompatible cookbook performance report")
    for key in ("probe_sha256", "support_sha256", "catalog_sha256"):
        if report.get(key) != manifest[key]:
            raise ValueError("report does not match actual cookbook source: " + key)
    for key in ("typed_jit_feature", "public_baseline_adapter"):
        if type(report.get(key)) is not bool:
            raise ValueError("invalid observed build attribute: " + key)
    if not all(isinstance(report.get(key), str) and report[key] for key in ("os", "arch", "target_env")):
        raise ValueError("missing target identity")
    public = report["public_baseline_adapter"]
    if report.get("compiled_source_format") != ("compact-v2" if public else "fluxr-1"):
        raise ValueError("source parser identity differs from the compiled adapter")
    if report.get("rust_kernel_scope") != KERNEL_SCOPE or report.get("rust_value_adapter_scope") != ADAPTER_SCOPE:
        raise ValueError("Rust boundary accounting differs")
    if (report.get("route_order") != "rotate first lane each individual sample and batch round"
            or report.get("row_order") != ROW_ORDER):
        raise ValueError("benchmark scheduling differs")
    settings = report.get("settings")
    if (not isinstance(settings, dict) or not fp.integer(settings.get("individual_samples"), 64, 16_384)
            or not fp.integer(settings.get("batch_rounds"), 3, 101)
            or not fp.integer(settings.get("calls_per_batch"), 8, 4096)
            or settings.get("warmup_calls_per_lane") != 128 or type(settings.get("warmup_calls_per_lane")) is not int
            or settings.get("lifecycle_samples") != 7 or type(settings.get("lifecycle_samples")) is not int
            or type(settings.get("common_routes")) is not bool):
        raise ValueError("invalid cookbook sampling settings")
    lanes = BASE_LANES + ([] if settings["common_routes"] else ["prepared"])
    if settings.get("timed_lanes") != lanes or public and not settings["common_routes"]:
        raise ValueError("timed lanes do not match the public/candidate schedule")
    rows = report.get("cases")
    expected = manifest["cases"]
    if (not isinstance(rows, list) or len(rows) != len(expected)
            or any(not isinstance(row, dict) for row in rows)
            or [row.get("name") for row in rows] != [row["name"] for row in expected]):
        raise ValueError("missing, duplicate, reordered or unknown cookbook example")
    for row, source in zip(rows, expected):
        name = source["name"]
        if row.get("status") != "passed":
            raise ValueError("cookbook case did not pass: " + name)
        for key in ("topics", "source_sha256", "rust_sha256", "compact_sha256"):
            if row.get(key) != source[key]:
                raise ValueError("cookbook case source/reference mismatch: " + name + "/" + key)
        if not fp.digest_valid(row.get("artifact_sha256")) or not fp.integer(row.get("artifact_bytes"), 1, MAX_FILE):
            raise ValueError("missing artifact identity: " + name)
        contract = row.get("row_contract")
        if (not isinstance(contract, dict) or set(contract) != {"name", "rows", "limits"}
                or contract["name"] != name or not exact_limits(contract["limits"])
                or not isinstance(contract["rows"], list) or not 1 <= len(contract["rows"]) <= 4096
                or row.get("rows_sha256") != fp.sha(canonical(contract))):
            raise ValueError("row/resource/oracle identity mismatch: " + name)
        successes = 0
        for fixture in contract["rows"]:
            if (not isinstance(fixture, dict) or set(fixture) != {"row", "expected"}
                    or not isinstance(fixture["row"], dict) or set(fixture["row"]) != {"inputs", "arguments"}):
                raise ValueError("invalid row contract fixture")
            for key in ("inputs", "arguments"):
                values = fixture["row"][key]
                if not isinstance(values, list) or len(values) > 64:
                    raise ValueError("invalid row argument/input count")
                for value in values:
                    validate_value(value)
            outcome = fixture["expected"]
            if not isinstance(outcome, dict) or len(outcome) != 1:
                raise ValueError("invalid Rust oracle outcome")
            if "Ok" in outcome:
                validate_value(outcome["Ok"])
                successes += 1
            elif "Err" not in outcome or not isinstance(outcome["Err"], str) or not 0 < len(outcome["Err"]) <= 4096:
                raise ValueError("invalid Rust oracle error")
        count = len(contract["rows"])
        for key, actual in (("validation_rows", count), ("expected_successes", successes), ("expected_traps", count-successes)):
            if type(row.get(key)) is not int or row[key] != actual:
                raise ValueError("validation outcome count differs: " + name)
        fp.validate_counts(row.get("validation_route_counts"), count, {"Unattributed"} if public else WITNESSES)
        if not report["typed_jit_feature"] and "TypedJit" in row["validation_route_counts"]:
            raise ValueError("JIT attribution requires the observed typed-jit feature")
        times = row.get("timings")
        if not isinstance(times, dict) or set(times) != set(lanes):
            raise ValueError("missing or unselected timing lane: " + name)
        for lane, value in times.items():
            if not isinstance(value, dict):
                raise ValueError("timing lane must be an object")
            timing(value.get("individual_call"), settings["individual_samples"], INDIVIDUAL)
            timing(value.get("batch_mean"), settings["batch_rounds"], BATCH)
            allowed = {"rust_kernel": {"RustKernel"}, "rust_value_adapter": {"RustValueAdapter"},
                       "portable": {"Portable"}, "named": {"Unattributed"}, "prepared": WITNESSES}[lane]
            fp.validate_counts(value.get("timed_individual_route_counts"), settings["individual_samples"], allowed)
            if not report["typed_jit_feature"] and "TypedJit" in value["timed_individual_route_counts"]:
                raise ValueError("timed JIT attribution requires the observed typed-jit feature")
        lifecycle = row.get("lifecycle")
        if not isinstance(lifecycle, dict) or set(lifecycle) != set(PHASES):
            raise ValueError("missing cookbook lifecycle phase")
        for phase, unit in PHASES.items():
            timing(lifecycle[phase], settings["lifecycle_samples"], unit)
    calibration = report.get("calibration")
    if not isinstance(calibration, dict) or set(calibration) != set(CALIBRATION):
        raise ValueError("missing timer/dispatch/loop calibration")
    for phase, unit in CALIBRATION.items():
        count = settings["batch_rounds"] if phase == "batch_loop" else settings["individual_samples"]
        timing(calibration[phase], count, unit)
    if not fp.integer(report.get("elapsed_ns"), 1, MAX_NS):
        raise ValueError("invalid whole-probe duration")
    return report


def normalized_flags(flags, public):
    """Permit only the explicitly observed public API adapter cfg difference."""
    if (not isinstance(flags, dict) or set(flags) != set(FLAGS)
            or not all(value is None or isinstance(value, str) for value in flags.values())):
        raise ValueError("incomplete compiler environment flags")
    common = dict(flags)
    adapter_count = 0
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
        value = flags[key]
        words = ([] if not value else value.split("\x1f") if key == "CARGO_ENCODED_RUSTFLAGS" else shlex.split(value))
        kept, index = [], 0
        while index < len(words):
            if words[index:index+2] == ["--cfg", "fluxr_public_baseline"]:
                adapter_count += 1
                index += 2
            elif words[index] == "--cfg=fluxr_public_baseline":
                adapter_count += 1
                index += 1
            else:
                kept.append(words[index])
                index += 1
        common[key] = kept
    if bool(adapter_count) != public or adapter_count > 1:
        raise ValueError("actual public adapter cfg and captured build flags disagree")
    return common


def validate_capture(value):
    if (not isinstance(value, dict) or value.get("schema") != "fluxr-cookbook-capture-v1"
            or value.get("source_and_binary_unchanged") is not True
            or value.get("bounded_collection") != "posix-process-groups-v1"):
        raise ValueError("comparison requires a completed bounded immutable-source capture")
    manifest = validate_manifest(value.get("inputs"))
    report = validate_report(value.get("report"), manifest)
    for key in ("source_identity", "build_id"):
        if not isinstance(value.get(key), str) or not value[key].strip():
            raise ValueError("missing source/build identity")
    for key in ("source", "binary", "compiler", "hardware", "configuration"):
        if not isinstance(value.get(key), dict):
            raise ValueError("missing comparison metadata: " + key)
    source, binary = value["source"], value["binary"]
    if (not fp.integer(source.get("files"), 1, 20_000) or not fp.integer(source.get("bytes"), 1, 256*1024*1024)
            or not all(fp.digest_valid(source.get(key)) for key in ("sha256", "cargo_lock_sha256"))
            or not fp.digest_valid(binary.get("sha256")) or not fp.integer(binary.get("bytes"), 1, 2**40)):
        raise ValueError("invalid source tree or executable identity")
    compiler = value["compiler"]
    if (set(compiler) != {"rustc", "cargo"} or not isinstance(compiler["rustc"], str)
            or not compiler["rustc"].startswith("rustc ") or not isinstance(compiler["cargo"], str)
            or not compiler["cargo"].startswith("cargo ")):
        raise ValueError("actual rustc/Cargo metadata is required")
    configuration = value["configuration"]
    features = configuration.get("features")
    if (configuration.get("profile") != "release" or not isinstance(features, list)
            or any(not isinstance(name, str) or not name or "," in name for name in features)
            or features != sorted(set(features)) or ("typed-jit" in features) != report["typed_jit_feature"]):
        raise ValueError("release profile or observed typed-jit feature differs")
    normalized_flags(configuration.get("environment_flags"), report["public_baseline_adapter"])
    host = value["hardware"]
    if (not all(isinstance(host.get(key), str) and host[key] for key in ("system", "machine", "release"))
            or not fp.digest_valid(host.get("host_fingerprint")) or not isinstance(host.get("cpu_models"), list)
            or any(not isinstance(model, str) or not model for model in host["cpu_models"])):
        raise ValueError("incomplete hardware identity")
    affinity = host.get("cpu_affinity")
    if affinity is not None and (not isinstance(affinity, list) or not affinity
                                or any(not fp.integer(cpu, 0, 1_000_000) for cpu in affinity)
                                or affinity != sorted(set(affinity))):
        raise ValueError("invalid CPU affinity")
    system = {"Linux": "linux", "Windows": "windows", "Darwin": "macos"}.get(host["system"])
    machine = {"AMD64": "x86_64", "arm64": "aarch64"}.get(host["machine"], host["machine"])
    if (system, machine) != (report["os"], report["arch"]):
        raise ValueError("reported target does not match the executing host")
    fp.validate_execution(value.get("execution"), MAX_STDOUT)
    timing(value.get("process_startup"), STARTUPS, STARTUP_UNIT)
    startups = value.get("startup_execution")
    if not isinstance(startups, list) or len(startups) != STARTUPS:
        raise ValueError("missing actual startup collections")
    for result in startups:
        fp.validate_execution(result, 1024)
    return value


def tool_version(command, source, logs, label):
    stdout, stderr = logs / (label + ".stdout"), logs / (label + ".stderr")
    outcome = run_bounded(command, b"", source, stdout, stderr, timeout_seconds=20,
                          max_stdout_bytes=16_384, max_stderr_bytes=16_384)
    fp.write_new(logs / (label + ".json"), asdict(outcome))
    if outcome.terminal != "exited" or outcome.exit_code != 0:
        raise ValueError("compiler version observation failed: " + label)
    return read_bytes(stdout, 16_384).decode().strip()


def capture(args):
    source, binary = args.source.resolve(), args.binary.resolve()
    output = fp.outside(fp.outside(args.output, source), ROOT)
    if output.exists() or output.is_symlink():
        raise ValueError("capture output must be a new path")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("benchmark binary must already exist and be executable")
    if binary.is_relative_to(source):
        raise ValueError("benchmark binaries must be outside the source tree")
    if not args.identity.strip() or not args.build_id.strip() or args.profile != "release":
        raise ValueError("source/build identities and release profile are required")
    for value, low, high in ((args.samples, 64, 16_384), (args.rounds, 3, 101), (args.batch_calls, 8, 4096), (args.timeout, 1, 3600)):
        if not fp.integer(value, low, high):
            raise ValueError("capture sampling or timeout setting outside benchmark bounds")
    before, manifest = tree_identity(source), source_manifest(source)
    binary_before = file_digest(binary)
    features = sorted(set(filter(None, (part.strip() for part in args.features.split(",")))))
    flags, host = {key: os.environ.get(key) for key in FLAGS}, fp.hardware()
    output.parent.mkdir(parents=True, exist_ok=True)
    logs = output.with_name(output.name + ".logs")
    logs.mkdir(mode=0o700)
    phase = "compiler_identity"
    started = time.perf_counter_ns()
    try:
        compiler = {"rustc": tool_version(["rustc", "-vV"], source, logs, "rustc"),
                    "cargo": tool_version(["cargo", "--version"], source, logs, "cargo")}
        phase = "benchmark"
        stdout, stderr = logs / "stdout", logs / "stderr"
        command = [sys.executable, "-c", LAUNCH, str(args.samples), str(args.rounds), str(args.batch_calls),
                   "1" if args.common_routes else "0", str(binary)]
        outcome = run_bounded(command, b"", source, stdout, stderr, timeout_seconds=args.timeout,
                              max_stdout_bytes=MAX_STDOUT, max_stderr_bytes=MAX_STDERR)
        execution = {**asdict(outcome), "stdout": file_digest(stdout), "stderr": file_digest(stderr)}
        fp.write_new(logs / "execution.json", execution)
        if outcome.terminal != "exited" or outcome.exit_code != 0:
            raise ValueError("benchmark failed: " + outcome.terminal + "; bounded outputs retained")
        phase = "report_validation"
        report = validate_report(fp.read(stdout), manifest)
        for key, value in {"individual_samples": args.samples, "batch_rounds": args.rounds,
                           "calls_per_batch": args.batch_calls, "common_routes": args.common_routes}.items():
            if report["settings"][key] != value:
                raise ValueError("binary experiment setting differs from request: " + key)
        if report["typed_jit_feature"] != ("typed-jit" in features):
            raise ValueError("declared features differ from observed binary cfg")
        normalized_flags(flags, report["public_baseline_adapter"])
        phase = "process_startup"
        startup, startup_execution = [], []
        for index in range(STARTUPS):
            out, err = logs / f"startup-{index}.stdout", logs / f"startup-{index}.stderr"
            begin = time.perf_counter_ns()
            result = run_bounded([str(binary), "--startup"], b"", source, out, err,
                                 timeout_seconds=20, max_stdout_bytes=1024, max_stderr_bytes=MAX_STDERR)
            startup.append(time.perf_counter_ns() - begin)
            record = {**asdict(result), "stdout": file_digest(out), "stderr": file_digest(err)}
            startup_execution.append(record)
            fp.write_new(logs / f"startup-{index}.json", record)
            if (result.terminal != "exited" or result.exit_code != 0
                    or read_bytes(out, 1024) != b"fluxr-cookbook-ready\n" or err.stat().st_size):
                raise ValueError("startup-only process failed or output differed")
        phase = "final_identity"
        if (tree_identity(source) != before or source_manifest(source) != manifest or file_digest(binary) != binary_before
                or fp.hardware() != host or {key: os.environ.get(key) for key in FLAGS} != flags):
            raise ValueError("source, binary, hardware/affinity or build flags changed during capture")
        value = {"schema": "fluxr-cookbook-capture-v1", "source_identity": args.identity, "build_id": args.build_id,
                 "source": before, "inputs": manifest, "binary": binary_before, "compiler": compiler, "hardware": host,
                 "configuration": {"profile": args.profile, "features": features, "environment_flags": flags,
                                   "feature_observation": "typed-jit and public adapter observed from compiled cfg; remaining flags/features declared"},
                 "execution": execution, "startup_execution": startup_execution,
                 "process_startup": fp.statistics(startup, STARTUP_UNIT), "duration_ns": time.perf_counter_ns()-started,
                 "bounded_collection": "posix-process-groups-v1", "logs_directory": logs.name,
                 "source_and_binary_unchanged": True, "report": report,
                 "qualification": "one identified build/process; finite empirical observations, no authentication or population-tail claim"}
        validate_capture(value)
        fp.write_new(output, value)
        return {"status": "consistent", "capture": str(output), "cases": len(report["cases"]),
                "individual_samples_per_lane": args.samples, "qualification": value["qualification"]}
    except BaseException as error:
        fp.write_new(logs / "failure.json", {"phase": phase, "error": f"{type(error).__name__}: {error}",
                     "source_identity": args.identity, "build_id": args.build_id,
                     "source_before": before, "binary_before": binary_before, "inputs": manifest})
        raise


def compare(baseline, candidate, lane_pair=None):
    baseline, candidate = validate_capture(baseline), validate_capture(candidate)
    for key in ("compiler", "hardware", "inputs"):
        if baseline[key] != candidate[key]:
            raise ValueError("unmatched experiment " + key)
    left, right = baseline["report"], candidate["report"]
    for key in ("profile", "features"):
        if baseline["configuration"][key] != candidate["configuration"][key]:
            raise ValueError("unmatched build " + key)
    old_flags = normalized_flags(baseline["configuration"]["environment_flags"], left["public_baseline_adapter"])
    new_flags = normalized_flags(candidate["configuration"]["environment_flags"], right["public_baseline_adapter"])
    if old_flags != new_flags:
        raise ValueError("compiler flags differ beyond the explicit public API adapter cfg")
    for key in ("settings", "limits", "os", "arch", "target_env", "typed_jit_feature", "debug_assertions",
                "telemetry", "route_order", "row_order", "rust_kernel_scope", "rust_value_adapter_scope"):
        if left[key] != right[key]:
            raise ValueError("unmatched report setting: " + key)
    rows = []
    for before, after in zip(left["cases"], right["cases"]):
        for key in ("name", "topics", "source_sha256", "rust_sha256", "compact_sha256", "rows_sha256",
                    "row_contract", "validation_rows", "expected_successes", "expected_traps",
                    "artifact_sha256", "artifact_bytes"):
            if before[key] != after[key]:
                raise ValueError("unmatched semantic/input/resource contract: " + before["name"] + "/" + key)
        common = sorted(set(before["timings"]) & set(after["timings"]))
        pairs = [lane_pair] if lane_pair else [(lane, lane) for lane in common]
        measured = []
        for old, new in pairs:
            if old not in before["timings"] or new not in after["timings"]:
                raise ValueError("requested comparison lane is unavailable")
            for observation in ("individual_call", "batch_mean"):
                measured.append({"baseline_lane": old, "candidate_lane": new, "measurement": observation,
                                 **fp.compare_statistics(before["timings"][old][observation], after["timings"][new][observation])})
        lifecycle, excluded = [], []
        for phase in PHASES:
            if left["public_baseline_adapter"] != right["public_baseline_adapter"] and phase != "artifact_decode":
                excluded.append({"phase": phase, "reason": "public parser/preparation/call API boundary differs"})
            else:
                lifecycle.append({"phase": phase, **fp.compare_statistics(before["lifecycle"][phase], after["lifecycle"][phase])})
        rows.append({"name": before["name"], "warm": measured, "lifecycle": lifecycle, "lifecycle_not_compared": excluded,
                     "baseline_validation_routes": before["validation_route_counts"], "candidate_validation_routes": after["validation_route_counts"],
                     "baseline_only_lanes": sorted(set(before["timings"])-set(after["timings"])),
                     "candidate_only_lanes": sorted(set(after["timings"])-set(before["timings"]))})
    return {"schema": "fluxr-cookbook-comparison-v1", "baseline_identity": baseline["source_identity"],
            "candidate_identity": candidate["source_identity"], "baseline_build_id": baseline["build_id"],
            "candidate_build_id": candidate["build_id"], "lane_pair": lane_pair,
            "api_boundary_changed": bool(lane_pair and lane_pair[0] != lane_pair[1]),
            "public_api_adapter_changed": left["public_baseline_adapter"] != right["public_baseline_adapter"],
            "prepared_lane_status": {
                arm: "unavailable_public_api" if report["public_baseline_adapter"] else
                     "not_timed_common_schedule" if report["settings"]["common_routes"] else "measured"
                for arm, report in (("baseline", left), ("candidate", right))},
            "dependency_lock_equal": baseline["source"]["cargo_lock_sha256"] == candidate["source"]["cargo_lock_sha256"],
            "cases": rows, "process_startup": fp.compare_statistics(baseline["process_startup"], candidate["process_startup"]),
            "calibration": {key: fp.compare_statistics(left["calibration"][key], right["calibration"][key]) for key in CALIBRATION},
            "qualification": "all empirical ranks and regression points retained; no universal speed or population-tail improvement claim"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    collect = sub.add_parser("capture")
    for key in ("binary", "source", "output"):
        collect.add_argument("--" + key, required=True, type=Path)
    for key in ("identity", "features", "build-id"):
        collect.add_argument("--" + key, required=True)
    collect.add_argument("--profile", choices=["release"], default="release")
    collect.add_argument("--samples", type=int, default=1024)
    collect.add_argument("--rounds", type=int, default=11)
    collect.add_argument("--batch-calls", type=int, default=64)
    collect.add_argument("--timeout", type=int, default=600)
    collect.add_argument("--common-routes", action="store_true", help="four-lane schedule required for the public API baseline")
    checked = sub.add_parser("check")
    checked.add_argument("capture", type=Path)
    paired = sub.add_parser("compare")
    paired.add_argument("baseline", type=Path)
    paired.add_argument("candidate", type=Path)
    paired.add_argument("--output", type=Path, required=True)
    paired.add_argument("--lanes", help="explicit BASELINE_LANE:CANDIDATE_LANE; changed boundaries are reported")
    args = parser.parse_args(argv)
    try:
        if args.command == "capture":
            result = capture(args)
        elif args.command == "check":
            value = validate_capture(fp.read(args.capture))
            result = {"status": "consistent", "cases": len(value["report"]["cases"]), "scope": "offline consistency only"}
        else:
            lane_pair = None
            if args.lanes:
                lane_pair = args.lanes.split(":")
                if len(lane_pair) != 2 or not all(lane_pair):
                    raise ValueError("--lanes requires BASELINE_LANE:CANDIDATE_LANE")
            result = compare(fp.read(args.baseline), fp.read(args.candidate), lane_pair)
            output = fp.outside(args.output, ROOT)
            output.parent.mkdir(parents=True, exist_ok=True)
            fp.write_new(output, result)
            result = {"status": "consistent", "comparison": str(output), "cases": len(result["cases"]),
                      "scope": "finite empirical comparisons; every regression retained"}
        print(json.dumps(result, sort_keys=True, indent=2, allow_nan=False))
        return 0
    except (ValueError, TypeError, KeyError, IndexError, AttributeError, OverflowError, RecursionError, OSError) as error:
        print(json.dumps({"status": "failed", "error": str(error)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
