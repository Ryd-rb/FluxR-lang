"""Real-process adversarial outcomes for trusted SPIR-V verification evidence.

Every child is fixed test-owned Python code, running in temporary storage outside
this checkout. No arbitrary candidate is admitted by this test suite.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from verify_compute import CommandRecorder
import verify_checkout


class ComputeCommandVerification(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fluxr-compute-verification-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.work = self.root / "work"
        self.work.mkdir()
        self.output = self.root / "evidence"
        self.output.mkdir()
        self.recorder = CommandRecorder(self.work, self.output, timeout_seconds=5)
        source_root = Path(__file__).resolve().parents[1]
        self.assertFalse(self.root.is_relative_to(source_root))

    def child(self, name, code, expected=0):
        return self.recorder.run(name, [sys.executable, "-c", code], expected=expected)

    def receipt(self):
        records = json.loads((self.output / "commands.json").read_text())
        self.assertEqual(records, self.recorder.commands)
        for record in records:
            data = (self.output / record["log"]).read_bytes()
            self.assertEqual(record["sha256"], hashlib.sha256(data).hexdigest())
        return records[-1]

    def test_exact_success_and_exact_validator_refusal_preserve_output_receipts(self):
        for status in (0, 1):
            with self.subTest(expected=status):
                code = ("import os,sys;assert os.getcwd()==" + repr(str(self.work)) + ";"
                        "sys.stdout.buffer.write(b'raw\\x00\\xff\\n');"
                        "sys.stderr.buffer.write(b'diagnostic\\n');" + f"sys.exit({status})")
                self.child(f"expected-{status}", code, expected=status)
                record = self.receipt()
                self.assertEqual(record["exit_status"], status)
                self.assertEqual(record["expected_exit"], status)
                self.assertTrue(record["matched_expected_exit"])
                self.assertEqual(record["terminal"], "matched_expected_exit")
                self.assertEqual((self.output / record["log"]).read_bytes(), b"raw\x00\xff\ndiagnostic\n")
                self.assertIsNone(record["launch_error"])
                self.assertFalse(record["timed_out"])
        self.assertEqual(len(self.recorder.commands), 2)

    def test_other_failures_and_inverted_success_do_not_count_as_refusals(self):
        for status, expected in [(2, 1), (7, 1), (0, 1), (1, 0)]:
            with self.subTest(actual=status, expected=expected):
                with self.assertRaisesRegex(RuntimeError, "expected exit"):
                    self.child(f"unexpected-{status}-{expected}",
                               f"import sys;print('attempt retained');sys.exit({status})", expected)
                record = self.receipt()
                self.assertEqual(record["exit_status"], status)
                self.assertEqual(record["expected_exit"], expected)
                self.assertFalse(record["matched_expected_exit"])
                self.assertEqual(record["terminal"], "unexpected_exit")
                self.assertIn(b"attempt retained", (self.output / record["log"]).read_bytes())
        self.assertEqual(len(self.recorder.commands), 4)

    def test_signal_termination_where_supported_cannot_be_a_validator_refusal(self):
        if os.name == "posix":
            # SIGKILL cannot create a core dump and affects only this child.
            code = ("import os,signal;print('before signal',flush=True);"
                    "os.kill(os.getpid(),signal.SIGKILL)")
            expected_status = -signal.SIGKILL
        else:
            # Non-POSIX hosts exercise an abnormal normal-code outcome instead;
            # no skipped test or unsupported POSIX coverage is reported as proof.
            code = "import os;print('non-POSIX abnormal exit',flush=True);os._exit(7)"
            expected_status = 7
        with self.assertRaises(RuntimeError):
            self.child("terminated", code, expected=1)
        record = self.receipt()
        self.assertEqual(record["exit_status"], expected_status)
        self.assertEqual(record["terminated_by_signal"], os.name == "posix")
        self.assertFalse(record["matched_expected_exit"])
        self.assertEqual(record["terminal"], "unexpected_exit")
        self.assertFalse(list(self.work.glob("core*")))

    def test_launch_failure_has_a_retained_log_and_never_matches_expected_refusal(self):
        with self.assertRaises(RuntimeError):
            self.recorder.run("missing-tool", [str(self.work / "does-not-exist")], expected=1)
        record = self.receipt()
        self.assertIsNone(record["exit_status"])
        self.assertIn("FileNotFoundError", record["launch_error"])
        self.assertEqual(record["terminal"], "launch_error")
        self.assertFalse(record["matched_expected_exit"])
        self.assertFalse(record["timed_out"])
        self.assertIn(b"does-not-exist", (self.output / record["log"]).read_bytes())

    def test_timeout_retains_partial_output_without_becoming_a_pass(self):
        self.recorder = CommandRecorder(self.work, self.output, timeout_seconds=0.5)
        with self.assertRaises(RuntimeError):
            self.child("timeout", "import time;print('started',flush=True);time.sleep(30)", expected=1)
        record = self.receipt()
        self.assertIsNone(record["exit_status"])
        self.assertTrue(record["timed_out"])
        self.assertEqual(record["terminal"], "timeout")
        self.assertFalse(record["matched_expected_exit"])
        self.assertIn("TimeoutExpired", record["launch_error"])
        self.assertIn(b"started", (self.output / record["log"]).read_bytes())

    def test_deadlines_and_expected_statuses_refuse_invalid_admission(self):
        for timeout in (0, -1, float("nan"), float("inf"), -float("inf"), True, "1"):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, "finite and positive"):
                CommandRecorder(self.work, self.output, timeout_seconds=timeout)
        for expected in (-1, 256, True, 1.0, "1"):
            with self.subTest(expected=expected), self.assertRaisesRegex(ValueError, "normal integer exit"):
                self.child("never-started", "raise SystemExit(0)", expected)
        self.assertEqual(list(self.output.iterdir()), [])

    def test_cli_invalid_deadlines_refuse_before_creating_evidence(self):
        script = Path(__file__).resolve().parents[1] / "tools/verify_compute.py"
        for index, deadline in enumerate(("nan", "inf", "-inf", "0", "-1")):
            output = self.root / f"invalid-deadline-{index}"
            with self.subTest(deadline=deadline):
                result = subprocess.run([sys.executable, str(script), "--output", str(output),
                                         "--target-dir", str(self.root / "target"),
                                         "--spirv-tools", str(self.root / "missing-sdk"),
                                         "--timeout-seconds=" + deadline],
                                        capture_output=True, text=True, timeout=5, cwd=self.work)
                self.assertEqual(result.returncode, 2)
                self.assertIn("finite and positive", result.stderr)
                self.assertFalse(output.exists())

    def test_existing_evidence_and_duplicate_names_cannot_be_overwritten(self):
        self.child("original", "print('retained')")
        before = (self.output / "original.log").read_bytes()
        marker = self.work / "must-not-execute"
        code = "from pathlib import Path;Path(" + repr(str(marker)) + ").touch()"
        with self.assertRaisesRegex(ValueError, "unique safe"):
            self.child("original", code)
        with self.assertRaisesRegex(ValueError, "unique safe"):
            self.child("../escape", code)
        with self.assertRaises(FileExistsError):
            CommandRecorder(self.work, self.output)
        self.assertFalse(marker.exists())
        self.assertEqual((self.output / "original.log").read_bytes(), before)
        self.assertEqual(len(self.recorder.commands), 1)

    def test_compute_verification_is_mandatory_in_complete_profiles(self):
        root = Path(__file__).resolve().parents[1]
        for profile in ("debug", "python", "full"):
            with self.subTest(profile=profile):
                checks, skipped = verify_checkout.profile_checks(root, profile)
                match = [check for check in checks if check.name == "compute-verification"]
                self.assertEqual(len(match), 1)
                self.assertEqual(match[0].argv[-1], "test_compute_verification.py")
                self.assertNotIn("compute-verification", [check["name"] for check in skipped])


if __name__ == "__main__":
    unittest.main()
