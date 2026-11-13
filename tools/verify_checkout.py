#!/usr/bin/env python3
"""Run named trusted-checkout checks and retain exact source/evidence identities.

This driver is not a sandbox, hermetic build system, signature, merge authority,
or proof that every target-specific execution path ran. It inherits the host
environment and records endpoint source drift; use isolated trusted checkouts.
"""
from __future__ import annotations

import argparse
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import threading
import time

PROFILES = ("debug", "release", "all-features", "python", "full")
MAX_LOG_BYTES = 16 * 1024 * 1024


def timestamp():
    return datetime.now(timezone.utc).isoformat()


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(",", ":")).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def hash_file(path):
    value = hashlib.sha256()
    size = 0
    with Path(path).open("rb") as stream:
        while block := stream.read(128 * 1024):
            value.update(block)
            size += len(block)
    return {"sha256": value.hexdigest(), "bytes": size}


def write_json(path, value):
    # Every run owns a new output directory; evidence is never overwritten.
    with Path(path).open("x", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, ensure_ascii=True)
        stream.write("\n")


def git(repository, *arguments):
    return subprocess.run(
        ["git", "-C", str(repository), *arguments], stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=True,
    ).stdout


def source_snapshot(repository):
    """Record Git identity and hash tracked checkout bytes independently of stat cache.

    Raw checkout bytes may differ from Git blobs because of line-ending/filter
    rules. Both identities are retained. Ignored files and external tool inputs
    are not a complete hermetic dependency closure. Gitlinks require a future
    explicit recursive snapshot contract and fail closed here.
    """
    repository = Path(repository).resolve(strict=True)
    head = git(repository, "rev-parse", "--verify", "HEAD").decode().strip()
    tree = git(repository, "rev-parse", f"{head}^{{tree}}").decode().strip()
    for entry in git(repository, "ls-files", "-v", "-z").split(b"\0"):
        if entry and (entry[:1].islower() or entry[:1] == b"S"):
            raise ValueError("assume-unchanged and skip-worktree flags hide initial tracked changes")
    status_bytes = git(repository, "status", "--porcelain=v1", "-z", "--untracked-files=all")
    files = []
    for entry in git(repository, "ls-files", "--stage", "-z").split(b"\0"):
        if not entry:
            continue
        metadata, encoded_path = entry.split(b"\t", 1)
        mode, blob, stage = metadata.decode("ascii").split()
        if stage != "0" or mode == "160000":
            raise ValueError("unmerged index entries and submodules require explicit verification support")
        name = os.fsdecode(encoded_path)
        path = repository / name
        record = {"path": name, "index_mode": mode, "index_blob": blob}
        try:
            info = path.lstat()
            if stat.S_ISLNK(info.st_mode):
                raw = os.fsencode(os.readlink(path))
                record.update(kind="symlink", sha256=digest(raw), bytes=len(raw))
            elif stat.S_ISREG(info.st_mode):
                record.update(kind="file", executable=bool(info.st_mode & 0o111), **hash_file(path))
            else:
                raise ValueError(f"tracked path is not a regular file or symlink: {name}")
        except FileNotFoundError:
            record.update(kind="missing")
        files.append(record)
    files.sort(key=lambda item: item["path"])
    # Recheck HEAD so a concurrent ref change cannot silently label another tree.
    if git(repository, "rev-parse", "--verify", "HEAD").decode().strip() != head:
        raise ValueError("HEAD moved while capturing source identity")
    return {
        "head": head, "git_tree": tree, "git_clean": not status_bytes,
        "git_status_porcelain_v1_z": status_bytes.decode("utf-8", errors="surrogateescape"),
        "tracked_checkout_sha256": digest(canonical(files)), "tracked_files": files,
    }


def same_source(before, after):
    return (before["head"], before["git_tree"], before["tracked_checkout_sha256"]) == (
        after["head"], after["git_tree"], after["tracked_checkout_sha256"]
    ) and before["git_clean"] and after["git_clean"]


@dataclass(frozen=True)
class Check:
    name: str
    argv: tuple[str, ...]
    scope: str
    resources: tuple[str, ...] = ()


def profile_checks(repository, profile):
    if profile not in PROFILES:
        raise ValueError("unknown verification profile")
    checks, skipped = [], []
    if profile in ("debug", "full"):
        checks.append(Check("cargo-debug", ("cargo", "test", "--locked", "--all-targets"), "runtime_tests", ("cargo-target",)))
    if profile in ("release", "full"):
        checks.append(Check("cargo-release", ("cargo", "test", "--locked", "--release", "--all-targets",
                                               "--features", "typed-jit,host-transport"), "runtime_tests", ("cargo-target",)))
    if profile in ("all-features", "full"):
        checks.append(Check("cargo-all-features", ("cargo", "check", "--locked", "--all-targets", "--all-features"), "compile_check_only", ("cargo-target",)))
    if profile in ("debug", "python", "full"):
        suites = [
            ("verification-driver", "tests", "test_verify_checkout.py", False),
            ("release-pack", "tests", "test_release_pack.py", False),
            ("resource-guard", "tests", "test_resource_guard.py", True),
            ("context-token-accounting", "tools", "test_context_token_accounting.py", False),
            ("jit-performance-contract", "tests", "test_jit_performance_contract.py", False),
            ("fragment-performance", "tests", "test_fragment_performance.py", False),
            ("source-performance", "tests", "test_source_performance.py", False),
            ("semantic-model", "tests", "test_semantic_campaign.py", False),
            ("semantic-campaign", "tests", "test_semantic_campaign_cli.py", True),
            ("semantic-native", "tests", "test_semantic_campaign_native.py", True),
            ("wordcode-token-accounting", "tools", "test_wordcode_token_accounting.py", False),
            ("canonical-frontend", "tools", "test_fluxr_native.py", True),
            ("standalone-exporter", "tools", "test_export_multicore_submission.py", True),
            ("multicore-qualification", "tools", "test_qualify_multicore.py", True),
            ("agent-pipeline", "tests", "test_agent_pipeline.py", True),
            ("agent-dispatch", "tests", "test_agent_dispatch.py", True),
            ("agent-ready", "tests", "test_agent_ready.py", True),
            ("agent-fair", "tests", "test_agent_fair.py", True),
            ("agent-snapshot", "tests", "test_agent_snapshot.py", True),
            ("agent-contracts", "tests", "test_agent_contracts.py", False),
            ("agent-contracts-v2", "tests", "test_agent_contracts_v2.py", False),
            ("agent-shadow", "tests", "test_agent_shadow.py", False),
            ("vulkan-registry", "tests", "test_vulkan_registry.py", False),
            ("compute-verification", "tests", "test_compute_verification.py", False),
            ("native-qualification", "tests", "test_native_qualification.py", False),
            ("native-stream-qualification", "tests", "test_native_stream_qualification.py", True),
            ("wordcode-tiering", "tests", "test_wordcode_tiering.py", True),
            ("agent-planning", "tests", "test_agent_planning_index.py", True),
            ("agent-store-faults", "tests", "test_agent_store_faults.py", True),
            ("test-impact", "tests", "test_agent_impact.py", False),
            ("evidence-policy", "tests", "test_agent_evidence.py", False),
            ("evaluation-control", "tools/eval/tests", "test_*.py", False),
        ]
        for name, directory, pattern, linux_only in suites:
            if not list((repository / directory).glob(pattern)):
                skipped.append({"name": name, "reason": "files_not_present", "required": True})
            elif linux_only and platform.system() != "Linux":
                skipped.append({"name": name, "reason": "prototype_supported_on_linux_only"})
            elif name == "semantic-native" and not os.environ.get("FLUXR_SEMANTIC_DRIVER"):
                # This file contains actual-driver checks only. Missing files
                # still fail above; an unconfigured native helper is an
                # explicit coverage gap, not an all-skipped suite success.
                skipped.append({"name": name, "reason": "semantic_driver_not_configured"})
            else:
                checks.append(Check(name, (sys.executable, str(repository / "tools/run_python_suite.py"),
                                           directory, pattern), "runtime_tests_with_platform_skips_recorded_in_log"))
    return checks, skipped


def signal_group(process, signum):
    deadline = time.monotonic() + 0.5
    while True:
        # Reap an exited leader before signaling its remaining group.
        process.poll()
        try:
            os.killpg(process.pid, signum)
        except ProcessLookupError:
            return True
        except PermissionError:
            # Darwin may return EPERM while exited members are being reaped.
            # Retry until signaling succeeds or the group disappears.
            if sys.platform != "darwin" or time.monotonic() >= deadline:
                return False
            time.sleep(0.02)
        else:
            return True


def terminate(process):
    """Best-effort descendant cleanup, explicitly not hostile-code containment."""
    group_signaled = True
    if os.name == "posix":
        signal_group(process, signal.SIGTERM)
        try:
            process.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            pass
        group_signaled = signal_group(process, signal.SIGKILL)
        if not group_signaled and process.poll() is None:
            process.kill()
    else:
        # Windows process groups do not enforce a job-object lifetime. taskkill
        # is best effort; a hosted CI job timeout is an additional outer bound.
        taskkill = shutil.which("taskkill")
        if taskkill:
            try:
                subprocess.run([taskkill, "/PID", str(process.pid), "/T", "/F"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
            except (OSError, subprocess.TimeoutExpired):
                pass
        if process.poll() is None:
            process.kill()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        return False
    return group_signaled


def run_check(check, repository, log_path, timeout_seconds, max_log_bytes, cancel_event=None):
    if (type(timeout_seconds) not in (int, float) or not math.isfinite(timeout_seconds) or timeout_seconds <= 0
            or type(max_log_bytes) is not int or max_log_bytes < 1):
        raise ValueError("timeout and output cap must be positive")
    started, started_ns = timestamp(), time.monotonic_ns()
    state = {"observed_bytes": 0, "written_bytes": 0, "read_error": None}
    overflow, done, stop = threading.Event(), threading.Event(), threading.Event()
    lock = threading.Lock()
    process, worker = None, None
    terminal, launch_error, cleanup_complete = "launch_error", None, True
    # Python tempfile otherwise falls back to the current checkout when the
    # host's temporary paths are unavailable. Concurrent source checks would
    # then observe another suite's fixtures as untracked source drift. Give
    # every command a separate writable directory next to its evidence log.
    # This is lifecycle management for trusted checks, not filesystem isolation.
    with tempfile.TemporaryDirectory(prefix=f".tmp-{check.name}-", dir=Path(log_path).parent) as temporary, \
            Path(log_path).open("xb") as log:
        child_environment = {**os.environ, "TMPDIR": temporary, "TMP": temporary, "TEMP": temporary}
        try:
            kwargs = {"start_new_session": True} if os.name == "posix" else {
                "creationflags": getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
            }
            process = subprocess.Popen(check.argv, cwd=repository, stdin=subprocess.DEVNULL,
                                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=child_environment, **kwargs)

            def drain():
                try:
                    while not stop.is_set():
                        block = process.stdout.read1(65536)
                        if not block:
                            break
                        with lock:
                            if stop.is_set():
                                break
                            state["observed_bytes"] += len(block)
                            keep = block[:max(0, max_log_bytes - state["written_bytes"])]
                            log.write(keep)
                            state["written_bytes"] += len(keep)
                            if len(keep) != len(block):
                                overflow.set()
                except (OSError, ValueError) as error:
                    state["read_error"] = str(error)
                finally:
                    process.stdout.close()
                    done.set()

            worker = threading.Thread(target=drain, name=f"verify-{check.name}", daemon=True)
            worker.start()
            deadline = time.monotonic() + timeout_seconds
            while True:
                if cancel_event is not None and cancel_event.is_set():
                    terminal = "cancelled"
                    break
                if overflow.is_set():
                    terminal = "output_limit"
                    break
                if process.poll() is not None and done.is_set():
                    terminal = "passed" if process.returncode == 0 and state["read_error"] is None else "failed"
                    break
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    terminal = "timeout"
                    break
                stop.wait(min(0.02, remaining))
        except OSError as error:
            launch_error = f"{type(error).__name__}: {error}"
        except BaseException:
            if process is not None:
                terminate(process)
            raise
        finally:
            # An exited leader with closed pipes may still leave a descendant
            # alive. Clean its ordinary process group on success as well. This
            # remains best effort and cannot contain hostile session escape.
            if process is not None:
                cleanup_complete = terminate(process)
            if worker is not None:
                worker.join(timeout=0.5)
            # A descendant retaining a pipe cannot keep a log mutable after the
            # result is hashed. A surviving daemon reader is recorded as an
            # incomplete drain and never interpreted as a successful check.
            with lock:
                stop.set()
                log.flush()
    if terminal == "passed" and (not done.is_set() or not cleanup_complete):
        terminal = "failed"
    return {
        "name": check.name, "argv": list(check.argv), "cwd": str(repository), "scope": check.scope,
        "temporary_directory": temporary, "temporary_directory_removed": not Path(temporary).exists(),
        "started_at": started, "completed_at": timestamp(),
        "duration_seconds": (time.monotonic_ns() - started_ns) / 1_000_000_000,
        "terminal": terminal, "exit_code": None if process is None else process.returncode,
        "launch_error": launch_error, "timeout_seconds": timeout_seconds, "max_log_bytes": max_log_bytes,
        "output_truncated": overflow.is_set(), "output_observed_bytes": state["observed_bytes"],
        "output_drain_complete": done.is_set(), "output_read_error": state["read_error"],
        "termination_observed_complete": cleanup_complete,
        "termination_policy": "process_group_best_effort" if os.name == "posix" else "taskkill_tree_best_effort",
        "log": Path(log_path).name, **hash_file(log_path),
    }


def collect_toolchain(repository, output):
    probes = [Check("git-version", ("git", "--version"), "tool_metadata"),
              Check("rustc-version", ("rustc", "-vV"), "tool_metadata"),
              Check("cargo-version", ("cargo", "-vV"), "tool_metadata")]
    return {
        "python_executable": sys.executable, "python_version": sys.version,
        "os": platform.system(), "os_release": platform.release(), "architecture": platform.machine(),
        "environment_is_hermetic": False,
        "temporary_directory_policy": "private-per-command-directory-beside-evidence-log",
        "selected_environment": {key: os.environ[key] for key in
                                 ("RUSTUP_TOOLCHAIN", "CARGO_HOME", "TMPDIR", "CARGO_BUILD_TARGET", "CARGO_TARGET_DIR", "RUSTFLAGS",
                                  "CARGO_ENCODED_RUSTFLAGS", "CARGO_TERM_COLOR") if key in os.environ},
        "probes": [run_check(check, repository, output / f"{check.name}.log", 30, 65536) for check in probes],
    }


def execute_checks(checks, repository, output, before, report, jobs, timeout_seconds, max_log_bytes):
    """Bound admission, serialize shared build roots, and retain every started result.

    A source drift stops new admission and cancels ordinary child process groups.
    Results are ordered by the declared plan, never by completion timing. This
    scheduler is for trusted independent checks, not hostile-code isolation.
    """
    pending = list(enumerate(checks))
    active, busy, outcomes = {}, set(), {}
    cancel = threading.Event()
    pool = ThreadPoolExecutor(max_workers=jobs, thread_name_prefix="fluxr-check")
    try:
        while pending or active:
            if not cancel.is_set():
                for index, check in list(pending):
                    if len(active) >= jobs:
                        break
                    if busy.intersection(check.resources):
                        continue
                    future = pool.submit(run_check, check, repository,
                                         output / f"{index + 1:02d}-{check.name}.log",
                                         timeout_seconds, max_log_bytes, cancel)
                    active[future] = (index, check)
                    busy.update(check.resources)
                    pending.remove((index, check))
            if not active:
                break
            finished, _ = wait(active, return_when=FIRST_COMPLETED)
            for future in sorted(finished, key=lambda value: active[value][0]):
                index, check = active.pop(future)
                busy.difference_update(check.resources)
                outcomes[index] = future.result()
                report["checks"] = [outcomes[i] for i in sorted(outcomes)]
            after = source_snapshot(repository)
            if not same_source(before, after):
                report["status"] = "source_drift"
                cancel.set()
        if report["status"] != "source_drift":
            report["status"] = "passed" if all(c["terminal"] == "passed" for c in outcomes.values()) else "failed"
    finally:
        cancel.set()
        # No active task may continue writing evidence after report finalization.
        pool.shutdown(wait=True, cancel_futures=True)
        for future, (index, check) in active.items():
            if not future.cancelled() and future.exception() is None:
                outcomes[index] = future.result()
        report["checks"] = [outcomes[i] for i in sorted(outcomes)]


def verify(repository, output, profile="debug", timeout_seconds=1800, max_log_bytes=MAX_LOG_BYTES, jobs=1):
    if type(jobs) is not int or not 1 <= jobs <= 32:
        raise ValueError("jobs must be an integer from 1 through 32")
    repository = Path(git(repository, "rev-parse", "--show-toplevel").decode().strip()).resolve(strict=True)
    output = Path(output).resolve()
    if output.is_relative_to(repository):
        raise ValueError("evidence directory must be outside the checkout")
    output.mkdir(parents=True, exist_ok=False)
    checks, skipped = profile_checks(repository, profile)
    if len({check.name for check in checks}) != len(checks) or any(
            not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,127}", check.name) for check in checks):
        raise ValueError("check names must be unique safe log names")
    report = {"schema": "fluxr-checkout-evidence-v1", "repository_path": str(repository), "profile": profile,
              "started_at": timestamp(), "completed_at": None, "status": "running", "checks": [],
              "skipped": skipped, "errors": [], "hermetic": False, "sandbox": False,
              "jobs": jobs, "scheduling_policy": "bounded-independent-checks-exclusive-resource-groups-v1",
              "verification_plan_sha256": digest(canonical({"checks": [check.__dict__ for check in checks], "skipped": skipped})),
              "scope": "Recorded commands and endpoint tracked-source stability; no release or backend coverage certification."}
    write_json(output / "run.json", {**report, "planned_checks": [check.__dict__ for check in checks]})
    before, after = None, None
    try:
        before = source_snapshot(repository)
        write_json(output / "source-before.json", before)
        if not before["git_clean"]:
            report["status"] = "refused_dirty_checkout"
        elif not checks:
            report["status"] = "no_checks_available"
        elif any(item.get("required") for item in skipped):
            report["status"] = "incomplete_profile"
            report["errors"].append("required verification suites are missing; refusing a partial success")
        else:
            report["toolchain"] = collect_toolchain(repository, output)
            execute_checks(checks, repository, output, before, report, jobs, timeout_seconds, max_log_bytes)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        report["status"] = "driver_error"
        report["errors"].append(f"{type(error).__name__}: {error}")
    except KeyboardInterrupt:
        report["status"] = "interrupted"
    finally:
        try:
            after = source_snapshot(repository)
            write_json(output / "source-after.json", after)
            if before is not None and report["status"] not in {"refused_dirty_checkout", "driver_error", "interrupted"} and not same_source(before, after):
                report["status"] = "source_drift"
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            report["status"] = "driver_error"
            report["errors"].append(f"final source capture: {type(error).__name__}: {error}")
        report["completed_at"] = timestamp()
        report["source_before"] = None if before is None else {k: v for k, v in before.items() if k != "tracked_files"}
        report["source_after"] = None if after is None else {k: v for k, v in after.items() if k != "tracked_files"}
        completed = {check["name"] for check in report["checks"]}
        report["checks_not_run"] = [check.name for check in checks if check.name not in completed]
        report["evidence_files"] = {path.name: hash_file(path) for path in sorted(output.iterdir()) if path.is_file()}
        write_json(output / "report.json", report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, required=True, help="new directory outside the checkout")
    parser.add_argument("--profile", choices=PROFILES, default="debug")
    parser.add_argument("--timeout-seconds", type=int, default=1800, help="per-command wall-time bound")
    parser.add_argument("--max-log-bytes", type=int, default=MAX_LOG_BYTES, help="per-command retained output cap")
    parser.add_argument("--jobs", type=int, default=1, help="1–32 independent checks; shared Cargo target remains exclusive")
    args = parser.parse_args()
    if args.timeout_seconds <= 0 or args.max_log_bytes < 1:
        parser.error("timeout and output cap must be positive")
    try:
        report = verify(args.repository, args.output, args.profile, args.timeout_seconds, args.max_log_bytes, args.jobs)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(2, f"verify-checkout: {error}\n")
    print(json.dumps({"status": report["status"], "report": str(args.output.resolve() / "report.json")}, indent=2))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
