#!/usr/bin/env python3
"""Run trusted FluxR native-domain fixtures with one unchanged runtime binary.

Output must be outside the source checkout. Each invocation is bounded, has its
own stdout/stderr/exit ledger and process resource accounting. This runner is not
an OS sandbox and does not authorize arbitrary evaluated native candidates.
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
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=180)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    if output.is_relative_to(ROOT) or not math.isfinite(args.timeout_seconds) or args.timeout_seconds <= 0:
        parser.error("output must be outside source, and timeout must be finite and positive")
    output.mkdir(parents=True, exist_ok=False)
    binary_hash = digest(binary)
    ledger: list[dict] = []
    started = datetime.now(timezone.utc).isoformat()

    def run(name: str, arguments: list[str]) -> dict:
        if digest(binary) != binary_hash:
            raise RuntimeError("runtime binary changed before fixture execution")
        stdout_path, stderr_path = output / f"{name}.stdout.log", output / f"{name}.stderr.log"
        command = [str(binary), *arguments]
        start = time.monotonic()
        interrupted = False
        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr, start_new_session=True)
            while True:
                pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                if pid:
                    break
                if time.monotonic() - start >= args.timeout_seconds:
                    interrupted = True
                    os.killpg(process.pid, signal.SIGKILL)
                    _, status, usage = os.wait4(process.pid, 0)
                    break
                time.sleep(0.01)
            process.returncode = os.waitstatus_to_exitcode(status)
        row = {"name": name, "command": command, "cwd": str(ROOT), "exit_status": process.returncode,
               "timed_out": interrupted, "wall_seconds": time.monotonic()-start,
               "user_cpu_seconds": usage.ru_utime, "system_cpu_seconds": usage.ru_stime,
               "peak_rss_kib": usage.ru_maxrss, "rss_unit_assumption": "Linux wait4 reports KiB",
               "stdout": stdout_path.name, "stderr": stderr_path.name,
               "runtime_sha256_before_and_after": binary_hash}
        if digest(binary) != binary_hash:
            row["binary_changed"] = True
        ledger.append(row)
        (output / "commands.json").write_text(json.dumps(ledger, indent=2)+"\n")
        if process.returncode or row.get("binary_changed"):
            raise RuntimeError(f"fixture {name} failed; retained {stdout_path} and {stderr_path}")
        return json.loads((output / f"{name}.json").read_text())

    summary = {"schema": "fluxr-native-domain-qualification-v1", "started_at_utc": started,
               "platform": platform.platform(), "python": platform.python_version(),
               "runtime_binary": str(binary), "runtime_sha256": binary_hash,
               "scope": "trusted fixture execution, not arbitrary candidate isolation"}
    try:
        with tempfile.TemporaryDirectory(prefix="fluxr-library-add-", dir=output) as temporary:
            stage = Path(temporary)
            shutil.copytree(ROOT / "examples/native/instruments/triangle", stage / "triangle")
            before = run("plugin-before", ["probe", str(stage / "triangle"), str(output / "plugin-before.json")])
            shutil.copytree(ROOT / "examples/native/instruments/bell", stage / "bell")
            after = run("plugin-after", ["probe", str(stage / "bell"), str(output / "plugin-after.json")])
            if before["program_identity"] == after["program_identity"] or before["result_bits"] == after["result_bits"]:
                raise RuntimeError("independent instrument addition did not change selected behavior")
            summary["unchanged_binary_extension"] = {"passed": True, "added_package": "fluxr.demo.bell", "native_rebuilds_between_runs": 0,
                                                      "before": before, "after": after, "abi": "fluxr-scalar-library-v1"}
        ml = [run(f"ml-{index}", ["ml", str(ROOT / "examples/native/ml"), str(output / f"ml-{index}.json")]) for index in range(2)]
        keys = ["program_identity", "checkpoint", "non_neural", "sparse", "attempted_calls", "completed_calls"]
        same_ml = all(ml[0][key] == ml[1][key] for key in keys) and ml[0]["training"]["parameters"] == ml[1]["training"]["parameters"] and ml[0]["concurrent_cpu"]["results"] == ml[1]["concurrent_cpu"]["results"]
        if not same_ml:
            raise RuntimeError("repeated ML behavior differed")
        audio = [run(f"audio-{index}", ["audio", str(ROOT / "examples/native"), str(output / f"audio-{index}.json")]) for index in range(2)]
        if audio[0]["tensor_sha256"] != audio[1]["tensor_sha256"] or audio[0]["composition_sha256"] != audio[1]["composition_sha256"]:
            raise RuntimeError("repeated offline audio bytes differed")
        summary.update({"passed": True, "repeated_ml_behavior_exact": same_ml,
                        "repeated_audio_tensor_sha256": audio[0]["tensor_sha256"],
                        "source_fixtures": {str(p.relative_to(ROOT)): digest(p) for p in sorted((ROOT / "examples/native").rglob("*")) if p.is_file()},
                        "limitations": ["Unchanged binary is executable byte identity, not source authorization", "No physical audio device or real-time scheduling qualification", "Peak RSS covers each full process including compiler preparation", "Concurrent lanes are CPU only; no linear scaling inference"]})
    except BaseException as error:
        summary.update({"passed": False, "error": f"{type(error).__name__}: {error}"})
        raise
    finally:
        summary["finished_at_utc"] = datetime.now(timezone.utc).isoformat()
        summary["commands"] = ledger
        (output / "report.json").write_text(json.dumps(summary, indent=2)+"\n")

if __name__ == "__main__":
    main()
