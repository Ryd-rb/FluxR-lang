"""Real-process outcome/cleanup probes for the reviewed-fixture runner.

The fixtures only write a readiness PID then wait or terminate. No shell/native
candidate compilation is performed. Temporary files live outside the checkout.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
RUNNER = ROOT / "tools/qualify_native_stream.py"


class NativeStreamQualificationTests(unittest.TestCase):
    def setUp(self):
        if not hasattr(os, "wait4"):
            self.fail("this runner qualification requires POSIX wait4; no skipped pass")
        self.temporary = tempfile.TemporaryDirectory(prefix="fluxr-stream-runner-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.assertFalse(self.directory.resolve().is_relative_to(ROOT))
        self.binary = self.directory / "fixture.py"
        self.ready = self.directory / "child.pid"
        self.output = self.directory / "evidence"
        self.environment = os.environ.copy()
        self.environment["FLUXR_TEST_CHILD_READY"] = str(self.ready)

    def fixture(self, body):
        self.binary.write_text(
            f"#!{sys.executable}\nimport os, signal, time\nfrom pathlib import Path\n"
            "Path(os.environ['FLUXR_TEST_CHILD_READY']).write_text(str(os.getpid()))\n"
            + body + "\n"
        )
        self.binary.chmod(0o700)

    def command(self, timeout="10"):
        return [sys.executable, str(RUNNER), "--binary", str(self.binary),
                "--output", str(self.output), "--timeout-seconds", timeout]

    def report(self):
        report = json.loads((self.output / "report.json").read_text())
        self.assertFalse(report["passed"])
        self.assertEqual(len(report["commands"]), 1)
        row = report["commands"][0]
        self.assertFalse(row["passed"])
        self.assertEqual(row["expected_exit"], 1)
        self.assertTrue((self.output / row["stdout"]).is_file())
        self.assertTrue((self.output / row["stderr"]).is_file())
        return row

    def assert_child_reaped(self):
        child = int(self.ready.read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(child, 0)

    def cleanup_child_if_needed(self):
        if self.ready.exists():
            try:
                os.killpg(int(self.ready.read_text()), signal.SIGKILL)
            except ProcessLookupError:
                pass

    def test_timeout_kills_reaps_and_retains_failure(self):
        self.fixture("time.sleep(30)")
        try:
            result = subprocess.run(self.command("0.15"), env=self.environment,
                                    capture_output=True, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            row = self.report()
            self.assertTrue(row["timed_out"])
            self.assertEqual(row["exit_status"], -signal.SIGKILL)
            self.assert_child_reaped()
        finally:
            self.cleanup_child_if_needed()

    def test_interrupt_kills_reaps_and_never_marks_cancelled_work_passed(self):
        self.fixture("time.sleep(30)")
        process = subprocess.Popen(self.command(), env=self.environment,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   start_new_session=True)
        try:
            deadline = time.monotonic() + 3
            while not self.ready.exists() and time.monotonic() < deadline:
                if process.poll() is not None:
                    break
                time.sleep(0.01)
            self.assertTrue(self.ready.exists(), "fixture did not reach running state")
            process.send_signal(signal.SIGINT)
            process.communicate(timeout=5)
            self.assertNotEqual(process.returncode, 0)
            row = self.report()
            self.assertTrue(row["interrupted_cleanup"])
            self.assertFalse(row["timed_out"])
            self.assertEqual(row["exit_status"], -signal.SIGKILL)
            self.assertIn("KeyboardInterrupt", row["error"])
            self.assert_child_reaped()
        finally:
            self.cleanup_child_if_needed()
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            process.communicate(timeout=5)

    def test_signal_exit_cannot_satisfy_expected_package_refusal(self):
        self.fixture("os.kill(os.getpid(), signal.SIGTERM)")
        result = subprocess.run(self.command(), env=self.environment,
                                capture_output=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        row = self.report()
        self.assertEqual(row["exit_status"], -signal.SIGTERM)
        self.assertFalse(row["timed_out"])
        self.assert_child_reaped()

    def test_launch_failure_is_retained_without_an_invented_exit(self):
        self.binary.write_text("not executable\n")
        self.binary.chmod(0o600)
        result = subprocess.run(self.command(), env=self.environment,
                                capture_output=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        row = self.report()
        self.assertIsNone(row["exit_status"])
        self.assertFalse(row["timed_out"])
        self.assertIn("PermissionError", row["error"])
        self.assertFalse(self.ready.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
