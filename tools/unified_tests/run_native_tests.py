#!/usr/bin/env python3
"""Qualify real Cargo/libtest execution through the FluxR application builder.

Run under the shared resource coordinator with external work, target and result
directories. This compiles and executes trusted native fixture code; it is not
an arbitrary-code sandbox. Results and generated projects never enter source.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import sys
import time

TOOLS = Path(__file__).resolve().parents[1]
SOURCE = TOOLS.parent
sys.path.insert(0, str(TOOLS))
import fluxr


def identity(path):
    data = path.read_bytes()
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def external(path):
    path = path.resolve()
    if path.is_relative_to(SOURCE) or SOURCE.is_relative_to(path):
        raise ValueError("qualification paths must be outside the source tree")
    return path


def qualify(args):
    work, output, target = map(external, (args.work, args.output, args.target_dir))
    if work.exists() or output.exists():
        raise ValueError("work and output must be new directories")
    for left, right in [(work, output), (work, target), (output, target)]:
        if left.is_relative_to(right) or right.is_relative_to(left):
            raise ValueError("work, results and target directories must be disjoint")
    if not math.isfinite(args.timeout) or not 0 < args.timeout <= 3600:
        raise ValueError("timeout must be finite and in 0..3600 seconds")
    cli = args.cli.resolve()
    if not cli.is_file() or not os.access(cli, os.X_OK):
        raise ValueError("--cli must name an executable FluxR compiler")
    work.mkdir(parents=True)
    output.mkdir(parents=True)
    fixture = Path(__file__).with_name("native_tests.rs")
    report = {"schema": "fluxr-native-test-qualification-v1", "passed": False,
              "scope": "real emitted Cargo/libtest filters, ignored tests, panic/Result tests and failure exit",
              "fixture": identity(fixture), "frontend": identity(TOOLS / "fluxr.py"),
              "compiler": identity(cli), "commands": [], "checks": [],
              "started_at_unix": time.time()}

    def save():
        (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")

    def run(name, command, *, cwd=work, success=True):
        logs = output / name
        command = [str(item) for item in command]
        result = fluxr.run_logged(command, cwd, logs, args.timeout)
        result["name"] = name
        result["expected_success"] = success
        report["commands"].append(result)
        save()
        if result["timed_out"] or (result["exit_code"] == 0) != success:
            raise RuntimeError(f"{name}: unexpected exit {result['exit_code']} or timeout; see {logs}")
        return (logs / "stdout.log").read_text()

    def checked(name, condition):
        report["checks"].append({"name": name, "passed": bool(condition)})
        save()
        if not condition:
            raise AssertionError(name)

    try:
        authored = work / "authored"
        authored.mkdir()
        native = fixture.read_bytes()
        source = (b"fluxr 1\npub embedded_marker 0 int ret 314159\n"
                  + b"rust " + str(len(native)).encode() + b"\n" + native + b"\n")
        (authored / "main.fluxr").write_bytes(source)
        package = "fluxr-native-test-options"
        (authored / "Cargo.toml").write_text(
            f'[package]\nname = "{package}"\nversion = "0.1.0"\nedition = "2024"\n')
        (authored / "Cargo.lock").write_text(
            f'version = 4\n\n[[package]]\nname = "{package}"\nversion = "0.1.0"\n')
        manifest = authored / "fluxr.json"
        manifest.write_text(json.dumps({"format": "fluxr-project-v1", "rustflags": [], "files": [
            {"source": name, "output": name, "mode": "copy", "executable": False}
            for name in ["Cargo.toml", "Cargo.lock"]
        ] + [{"source": "main.fluxr", "output": "src/main.rs", "mode": "source", "executable": False}]}, indent=2) + "\n")
        emitted = work / "emitted"
        build_logs = output / "builder"
        harness_args = ["--include-ignored", "--skip", "intentional_failure", "--test-threads", "1", "--nocapture"]
        command = [cli, "build", manifest, emitted,
                   "--target-dir", target, "--logs", build_logs, "--test",
                   "--message-format", "json", "--profile", args.profile,
                   "--jobs", "1", "--timeout", str(args.timeout), "--", *harness_args]
        run("frontend-test", command)
        build = json.loads((build_logs / "build.json").read_text())
        checked("forwarded arguments are an exact Cargo-test suffix",
                build["argv"][1] == "test" and build["argv"][-len(harness_args)-1:] == ["--", *harness_args])
        stdout = (build_logs / "stdout.log").read_text()
        checked("real checked, Result, panic, ignored and admission tests executed",
                "5 passed; 0 failed; 0 ignored" in stdout and "1 filtered out" in stdout
                and "NATIVE_CHECKED_TEST_EXECUTED" in stdout and "NATIVE_IGNORED_TEST_EXECUTED" in stdout)
        executables = []
        for line in stdout.splitlines():
            if not line.startswith("{"):
                continue
            row = json.loads(line)
            if (row.get("reason") == "compiler-artifact" and row.get("profile", {}).get("test")
                    and row.get("target", {}).get("name") == package and row.get("executable")):
                executables.append(Path(row["executable"]))
        checked("one real native test binary produced", len(set(executables)) == 1)
        binary = executables[0]
        report["test_binary"] = identity(binary)
        listed = run("list-tests", [binary, "--list"])
        expected = {"__fluxr_unified_runtime_is_embedded", "checked_runtime_isolated", "result_success",
                    "expected_panic", "ignored_success", "intentional_failure"}
        actual = {line.removesuffix(": test") for line in listed.splitlines() if line.endswith(": test")}
        checked("list reports exact test inventory without execution", actual == expected and "TEST_EXECUTED" not in listed)
        exact = run("exact-checked", [binary, "checked_runtime_isolated", "--exact", "--test-threads", "1", "--nocapture"])
        checked("exact checked test is independent of runtime admission ordering",
                "1 passed; 0 failed" in exact and "5 filtered out" in exact and "NATIVE_CHECKED_TEST_EXECUTED" in exact)
        ignored = run("ignored-only", [binary, "ignored_success", "--exact", "--ignored", "--nocapture"])
        checked("ignored-only executes requested test", "1 passed; 0 failed" in ignored and "NATIVE_IGNORED_TEST_EXECUTED" in ignored)
        failure = run("failure-exit", [binary, "intentional_failure", "--exact", "--ignored", "--nocapture"], success=False)
        checked("native failure is reported as a failure", "0 passed; 1 failed" in failure)
        checked("native panic diagnostic is retained", "EXPECTED_NATIVE_TEST_FAILURE" in (output / "failure-exit/stderr.log").read_text())
        report["passed"] = True
    except BaseException as error:
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        report["finished_at_unix"] = time.time()
        save()
    print(json.dumps({"passed": True, "checks": len(report["checks"]), "report": str(output / "report.json")}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--profile", choices=["dev", "release"], default="release")
    parser.add_argument("--timeout", type=float, default=600)
    args = parser.parse_args()
    try:
        qualify(args)
    except (AssertionError, OSError, RuntimeError, ValueError) as error:
        print(f"native test qualification: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
