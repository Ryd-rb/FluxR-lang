#!/usr/bin/env python3
"""Qualify reviewed FluxR stream fixtures and unchanged-binary effect addition.

This is a bounded command/evidence runner, not arbitrary-candidate OS isolation.
All attempted commands, including the expected missing-package refusal, retain
exact exit statuses, output and Linux process resource observations.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=60)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    if output.is_relative_to(ROOT) or not math.isfinite(args.timeout_seconds) or args.timeout_seconds <= 0:
        parser.error("output must be outside source; timeout must be finite and positive")
    output.mkdir(parents=True, exist_ok=False)
    runtime_hash = digest(binary)
    commands: list[dict] = []
    summary = {
        "schema": "fluxr-native-stream-qualification-v1",
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "runtime_binary": str(binary), "runtime_sha256": runtime_hash,
        "platform": platform.platform(), "python": platform.python_version(),
        "scope": "reviewed fixtures only; not a hostile-code isolation boundary",
        "passed": False,
    }

    def run(name: str, mode: str, root: Path, expected_exit: int = 0) -> dict | None:
        command = [str(binary), mode, str(root), str(output / f"{name}.json")]
        row = {"name": name, "command": command, "cwd": str(ROOT),
               "expected_exit": expected_exit, "exit_status": None,
               "timed_out": False, "passed": False,
               "stdout": f"{name}.stdout.log", "stderr": f"{name}.stderr.log"}
        started = time.monotonic()
        commands.append(row)
        process = None
        reaped = False
        try:
            if digest(binary) != runtime_hash:
                raise RuntimeError("runtime changed before execution")
            with (output / row["stdout"]).open("wb") as stdout, (output / row["stderr"]).open("wb") as stderr:
                process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr, start_new_session=True)
                while True:
                    pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                    if pid:
                        reaped = True
                        break
                    if time.monotonic() - started >= args.timeout_seconds:
                        row["timed_out"] = True
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        _, status, usage = os.wait4(process.pid, 0)
                        reaped = True
                        break
                    time.sleep(0.01)
                process.returncode = os.waitstatus_to_exitcode(status)
            row.update({"exit_status": process.returncode, "user_cpu_seconds": usage.ru_utime,
                        "system_cpu_seconds": usage.ru_stime, "peak_rss_kib": usage.ru_maxrss,
                        "rss_unit_assumption": "Linux wait4 KiB", "runtime_sha256_after": digest(binary)})
            if row["timed_out"] or process.returncode != expected_exit or row["runtime_sha256_after"] != runtime_hash:
                raise RuntimeError(f"{name}: command outcome differs from exact expected exit {expected_exit}")
            if expected_exit:
                if (output / f"{name}.json").exists():
                    raise RuntimeError("failed package admission unexpectedly published a report")
                report = None
            else:
                report = json.loads((output / f"{name}.json").read_text())
            row["passed"] = True
            return report
        except BaseException as error:
            if process is not None and not reaped:
                # A caller interruption or output/read error must not detach a
                # still-running fixture. This is process cleanup, not successful
                # cancellation evidence or an arbitrary-code sandbox.
                row["interrupted_cleanup"] = True
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                _, status, usage = os.wait4(process.pid, 0)
                process.returncode = os.waitstatus_to_exitcode(status)
                row.update({"exit_status": process.returncode,
                            "user_cpu_seconds": usage.ru_utime,
                            "system_cpu_seconds": usage.ru_stime,
                            "peak_rss_kib": usage.ru_maxrss,
                            "rss_unit_assumption": "Linux wait4 KiB"})
            row["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            row["wall_seconds"] = time.monotonic() - started
            (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")

    try:
        with tempfile.TemporaryDirectory(prefix="fluxr-effect-add-", dir=output) as temporary:
            stage = Path(temporary)
            for name in ("triangle", "bell"):
                shutil.copytree(ROOT / "examples/native/instruments" / name, stage / "instruments" / name)
            run("effect-before-addition", "effects", stage, expected_exit=1)
            shutil.copytree(ROOT / "examples/native/effects/softclip", stage / "effects/softclip")
            extension = run("effect-after-addition", "effects", stage)
            summary["unchanged_binary_extension"] = {
                "passed": True, "added_package": "fluxr.effects.softclip",
                "native_rebuilds_between_runs": 0, "runtime_sha256": runtime_hash,
                "before": "exact exit 1; package absent; no output publication",
                "after_graph_identity": extension["graph_identity"],
                "dry_sample_bits_sha256": extension["dry_sample_bits_sha256"],
                "wet_sample_bits_sha256": extension["wet_sample_bits_sha256"],
            }
        ml = [run(f"ml-{index}", "ml", ROOT / "examples/native") for index in range(2)]
        effects = [run(f"effects-{index}", "effects", ROOT / "examples/native") for index in range(2)]
        if ml[0]["graph_identity"] != ml[1]["graph_identity"] or ml[0]["reference_value_bits_sha256"] != ml[1]["reference_value_bits_sha256"]:
            raise RuntimeError("repeated stream ML identity or value bits differ")
        for observation in ml[0]["stream_observations"] + ml[1]["stream_observations"]:
            if observation["output_value_bits_sha256"] != ml[0]["reference_value_bits_sha256"]:
                raise RuntimeError("chunk-dependent model output")
            statistics = observation["statistics"]
            if statistics["published_rows"] != ml[0]["rows"] or statistics["failed_chunks"] or statistics["rejected_chunks"]:
                raise RuntimeError("stream reported incomplete work")
        for field in ("graph_identity", "dry_sample_bits_sha256", "wet_sample_bits_sha256"):
            if effects[0][field] != effects[1][field] or effects[0][field] != extension[field]:
                raise RuntimeError("repeated/independently added effects differ")
        summary.update({"passed": True, "repeated_ml_and_effect_bits_exact": True,
                        "source_fixtures": {str(path.relative_to(ROOT)): digest(path) for path in sorted((ROOT / "examples/native").rglob("*")) if path.is_file()},
                        "limitations": ["Unchanged binary identity is not package authentication",
                                        "No physical GPU, audio-device or real-time callback execution",
                                        "Process RSS includes preparation, VM, graph, ledgers and host fixtures",
                                        "Retained historical training overhead is not replaced by this inference experiment"]})
    except BaseException as error:
        summary["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        summary["finished_at_utc"] = datetime.now(timezone.utc).isoformat()
        summary["commands"] = commands
        (output / "report.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
