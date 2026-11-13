"""Fault tests for exact-checkout evidence, using only temporary local repositories."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import verify_checkout as driver


class VerificationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fluxr-verification-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repository = self.root / "repository"
        self.repository.mkdir()
        self.git("init", "--quiet")
        self.git("config", "user.name", "FluxR verification fixture")
        self.git("config", "user.email", "fixture@localhost")
        (self.repository / "tracked.txt").write_text("original\n")
        self.git("add", "tracked.txt")
        self.git("commit", "--quiet", "-m", "fixture baseline")
        self.output = self.root / "evidence"

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.repository), *args], check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

    def check(self, name, source):
        return driver.Check(name, (sys.executable, "-c", source), "fixture_runtime_check")

    def verify(self, checks, **kwargs):
        with mock.patch.object(driver, "profile_checks", return_value=(checks, [])), \
             mock.patch.object(driver, "collect_toolchain", return_value={"fixture": True}):
            return driver.verify(self.repository, self.output, timeout_seconds=5, max_log_bytes=4096, **kwargs)

    def test_complete_run_binds_exact_head_tree_bytes_and_log_hashes(self):
        expected = b"exact\x00\xff\n"
        report = self.verify([self.check("output", "import sys;sys.stdout.buffer.write(b'exact\\x00\\xff\\n')")])
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["source_before"]["head"], self.git("rev-parse", "HEAD").decode().strip())
        self.assertEqual(report["source_before"]["git_tree"], self.git("rev-parse", "HEAD^{tree}").decode().strip())
        self.assertEqual(report["source_before"], report["source_after"])
        record = report["checks"][0]
        self.assertEqual(record["sha256"], hashlib.sha256(expected).hexdigest())
        self.assertEqual(record["bytes"], len(expected))
        self.assertEqual((self.output / record["log"]).read_bytes(), expected)
        self.assertEqual(record["argv"][0], sys.executable)
        self.assertFalse(report["hermetic"])
        self.assertFalse(report["sandbox"])
        self.assertEqual(json.loads((self.output / "report.json").read_text()), report)
        for name, identity in report["evidence_files"].items():
            self.assertEqual(driver.hash_file(self.output / name), identity)

    def test_private_child_temporary_directory_overrides_unavailable_host_paths(self):
        missing = str(self.root / "missing-host-temporary")
        code = ("import os,tempfile;from pathlib import Path;"
                "p=Path(tempfile.mkdtemp());(p/'fixture').write_text('private');"
                "assert not p.is_relative_to(Path.cwd());"
                "assert os.environ['TMPDIR']==os.environ['TMP']==os.environ['TEMP'];"
                "print(p)")
        with mock.patch.dict(os.environ, {"TMPDIR": missing, "TMP": missing, "TEMP": missing}):
            report = self.verify([self.check("temp", code)])
        self.assertEqual(report["status"], "passed")
        result = report["checks"][0]
        temporary = Path(result["temporary_directory"])
        self.assertTrue(temporary.is_relative_to(self.output))
        self.assertTrue(result["temporary_directory_removed"])
        self.assertFalse(temporary.exists())
        self.assertEqual(self.git("status", "--porcelain"), b"")

    def test_parallel_temporary_files_do_not_alias_or_look_like_source_drift(self):
        code = ("from pathlib import Path;import tempfile,time;"
                "p=Path(tempfile.gettempdir())/'same-filename';"
                "p.open('x').write('fixture');time.sleep(0.08)")
        report = self.verify([self.check("a", code), self.check("b", code)], jobs=2)
        self.assertEqual(report["status"], "passed")
        self.assertEqual(len({c["temporary_directory"] for c in report["checks"]}), 2)
        self.assertTrue(all(c["temporary_directory_removed"] for c in report["checks"]))
        self.assertEqual(self.git("status", "--porcelain"), b"")

    def test_failed_command_cleans_private_temporary_directory(self):
        code = ("import tempfile;from pathlib import Path;"
                "(Path(tempfile.gettempdir())/'failure').write_text('retained only in log');"
                "raise SystemExit(9)")
        report = self.verify([self.check("fails-with-temp", code)])
        self.assertEqual(report["status"], "failed")
        self.assertTrue(report["checks"][0]["temporary_directory_removed"])
        self.assertFalse(list(self.output.glob(".tmp-*")))

    def test_failed_command_keeps_following_evidence_without_becoming_success(self):
        checks = [self.check("fails", "import sys;print('failed');sys.exit(7)"),
                  self.check("continues", "print('independent check')")]
        report = self.verify(checks)
        self.assertEqual(report["status"], "failed")
        self.assertEqual([c["terminal"] for c in report["checks"]], ["failed", "passed"])
        self.assertEqual(report["checks"][0]["exit_code"], 7)
        self.assertEqual(report["checks_not_run"], [])

    def test_dirty_checkout_is_refused_before_any_check_runs(self):
        (self.repository / "untracked.txt").write_text("not admitted")
        with mock.patch.object(driver, "run_check", side_effect=AssertionError("must not run")):
            report = self.verify([self.check("never", "raise SystemExit(0)")])
        self.assertEqual(report["status"], "refused_dirty_checkout")
        self.assertFalse(report["source_before"]["git_clean"])
        self.assertEqual(report["checks"], [])
        self.assertEqual(report["checks_not_run"], ["never"])

    def test_source_mutation_rejects_successful_exit_and_stops_next_check(self):
        checks = [self.check("mutates", "from pathlib import Path;Path('tracked.txt').write_text('changed')"),
                  self.check("never", "raise SystemExit(99)")]
        report = self.verify(checks)
        self.assertEqual(report["checks"][0]["exit_code"], 0)
        self.assertEqual(report["status"], "source_drift")
        self.assertEqual(report["checks_not_run"], ["never"])
        self.assertNotEqual(report["source_before"]["tracked_checkout_sha256"], report["source_after"]["tracked_checkout_sha256"])

    def test_hidden_index_flags_refuse_initial_checkout_before_execution(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            self.git("update-index", flag, "tracked.txt")
            (self.repository / "tracked.txt").write_text("hidden initial change")
            with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, "hide initial"):
                driver.source_snapshot(self.repository)
            self.git("update-index", "--no-assume-unchanged", "--no-skip-worktree", "tracked.txt")
        self.git("update-index", "--assume-unchanged", "tracked.txt")
        with mock.patch.object(driver, "run_check", side_effect=AssertionError("must not run")):
            report = self.verify([self.check("never", "raise SystemExit(0)")])
        self.assertEqual(report["status"], "driver_error")
        self.assertEqual(report["checks"], [])

    @unittest.skipUnless(os.name == "posix", "POSIX process group cleanup")
    def test_successful_parent_does_not_leave_detached_output_child_running(self):
        marker = self.root / "orphan-marker"
        child = f"import time;from pathlib import Path;time.sleep(0.4);Path({str(marker)!r}).write_text('leaked')"
        parent = ("import subprocess,sys;subprocess.Popen([sys.executable,'-c'," + repr(child)
                  + "],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)")
        report = self.verify([self.check("parent", parent)])
        self.assertEqual(report["status"], "passed")
        time.sleep(0.5)
        self.assertFalse(marker.exists())

    def test_head_movement_with_unchanged_tree_invalidates_result(self):
        source = "import subprocess;subprocess.run(['git','commit','--quiet','--allow-empty','-m','moved'],check=True)"
        report = self.verify([self.check("moves-head", source)])
        self.assertEqual(report["status"], "source_drift")
        self.assertEqual(report["source_before"]["git_tree"], report["source_after"]["git_tree"])
        self.assertNotEqual(report["source_before"]["head"], report["source_after"]["head"])

    def test_output_directory_is_new_and_outside_the_checkout(self):
        self.output.mkdir()
        existing = self.output / "keep.txt"
        existing.write_text("keep")
        with self.assertRaises(FileExistsError):
            driver.verify(self.repository, self.output)
        self.assertEqual(existing.read_text(), "keep")
        with self.assertRaisesRegex(ValueError, "outside"):
            driver.verify(self.repository, self.repository / "evidence")
        self.assertFalse((self.repository / "evidence").exists())

    def test_missing_optional_suites_are_explicit_and_no_checks_is_not_success(self):
        checks, skipped = driver.profile_checks(self.repository, "python")
        self.assertEqual(checks, [])
        self.assertEqual({s["name"] for s in skipped}, {"verification-driver", "release-pack", "resource-guard", "context-token-accounting", "jit-performance-contract", "fragment-performance", "source-performance", "wordcode-token-accounting", "canonical-frontend", "standalone-exporter", "multicore-qualification", "wordcode-tiering", "agent-pipeline", "agent-dispatch",
                         "agent-planning", "agent-store-faults", "test-impact", "evidence-policy", "evaluation-control",
                         "agent-ready", "agent-fair", "agent-snapshot", "agent-contracts", "agent-contracts-v2", "agent-shadow", "vulkan-registry", "compute-verification", "native-qualification", "native-stream-qualification", "semantic-model", "semantic-campaign", "semantic-native"})
        report = driver.verify(self.repository, self.output, "python")
        self.assertEqual(report["status"], "no_checks_available")

    def test_known_profiles_keep_compile_checks_distinct_from_runtime_checks(self):
        checks, _ = driver.profile_checks(self.repository, "full")
        rust = {c.name: c for c in checks}
        self.assertIn("--locked", rust["cargo-debug"].argv)
        self.assertIn("typed-jit,host-transport", rust["cargo-release"].argv)
        self.assertEqual(rust["cargo-all-features"].scope, "compile_check_only")
        self.assertEqual(rust["cargo-debug"].scope, "runtime_tests")
        with self.assertRaises(ValueError):
            driver.profile_checks(self.repository, "arbitrary-command")

    def test_semantic_suites_are_required_in_each_python_profile(self):
        for profile in ("debug", "python", "full"):
            with self.subTest(profile=profile):
                _, skipped = driver.profile_checks(self.repository, profile)
                missing = {item["name"]: item for item in skipped}
                for name in ("semantic-model", "semantic-campaign", "semantic-native"):
                    self.assertEqual(missing[name]["reason"], "files_not_present")
                    self.assertIs(missing[name]["required"], True)

    def test_semantic_model_is_portable_and_process_campaign_is_linux_only(self):
        tests = self.repository / "tests"
        tests.mkdir()
        suites = {
            "semantic-model": "test_semantic_campaign.py",
            "semantic-campaign": "test_semantic_campaign_cli.py",
            "semantic-native": "test_semantic_campaign_native.py",
        }
        for filename in suites.values():
            (tests / filename).write_text("# Profile-discovery fixture, not execution evidence.\n")
        for profile in ("debug", "python", "full"):
            for system in ("Linux", "Windows", "Darwin"):
                with self.subTest(profile=profile, system=system), \
                     mock.patch.object(driver.platform, "system", return_value=system), \
                     mock.patch.dict(os.environ, {"FLUXR_SEMANTIC_DRIVER": "profile-discovery-fixture-only"}):
                    checks, skipped = driver.profile_checks(self.repository, profile)
                    named = {check.name: check for check in checks}
                    self.assertEqual(named["semantic-model"].argv[-2:], ("tests", suites["semantic-model"]))
                    self.assertTrue(named["semantic-model"].argv[1].endswith("run_python_suite.py"))
                    for name in ("semantic-campaign", "semantic-native"):
                        if system == "Linux":
                            self.assertEqual(named[name].argv[-2:], ("tests", suites[name]))
                        else:
                            self.assertNotIn(name, named)
                            excluded = [item for item in skipped if item["name"] == name]
                            self.assertEqual(excluded, [{
                                "name": name,
                                "reason": "prototype_supported_on_linux_only",
                            }])

    def test_missing_native_driver_is_an_explicit_coverage_gap(self):
        tests = self.repository / "tests"
        tests.mkdir()
        (tests / "test_semantic_campaign_native.py").write_text("# Native profile-discovery fixture.\n")
        with mock.patch.object(driver.platform, "system", return_value="Linux"), \
             mock.patch.dict(os.environ, {"FLUXR_SEMANTIC_DRIVER": ""}):
            for profile in ("debug", "python", "full"):
                with self.subTest(profile=profile):
                    checks, skipped = driver.profile_checks(self.repository, profile)
                    self.assertNotIn("semantic-native", {check.name for check in checks})
                    self.assertEqual(
                        [item for item in skipped if item["name"] == "semantic-native"],
                        [{"name": "semantic-native", "reason": "semantic_driver_not_configured"}],
                    )

    def test_required_missing_suite_cannot_turn_partial_profile_green(self):
        checks = [self.check("present", "raise SystemExit(0)")]
        skipped = [{"name": "missing", "reason": "files_not_present", "required": True}]
        with mock.patch.object(driver, "profile_checks", return_value=(checks, skipped)), \
             mock.patch.object(driver, "run_check", side_effect=AssertionError("must refuse before running")):
            report = driver.verify(self.repository, self.output)
        self.assertEqual(report["status"], "incomplete_profile")
        self.assertEqual(report["checks_not_run"], ["present"])
        self.assertEqual(report["checks"], [])

    def test_independent_checks_really_overlap_and_keep_declared_result_order(self):
        # Both commands must rendezvous; a serial implementation cannot pass.
        first, second = self.root / "first", self.root / "second"
        def source(own, peer):
            return ("from pathlib import Path;import time;"
                    f"Path({str(own)!r}).write_text('started');deadline=time.monotonic()+3\n"
                    f"while not Path({str(peer)!r}).exists():\n"
                    " if time.monotonic()>deadline: raise SystemExit(7)\n"
                    " time.sleep(0.01)\n")
        report = self.verify([self.check("first", source(first, second)),
                              self.check("second", source(second, first))], jobs=2)
        self.assertEqual(report["status"], "passed")
        self.assertEqual([check["name"] for check in report["checks"]], ["first", "second"])
        self.assertEqual(report["jobs"], 2)
        self.assertEqual(len(report["verification_plan_sha256"]), 64)

    def test_shared_resource_exclusion_with_parallel_capacity(self):
        resource = self.root / "resource"
        code = ("from pathlib import Path;import time;"
                f"p=Path({str(resource)!r});p.mkdir();time.sleep(0.12);p.rmdir()")
        checks = [driver.Check(name, (sys.executable, "-c", code), "fixture", ("cargo-target",))
                  for name in ("first", "second", "third")]
        report = self.verify(checks, jobs=3)
        self.assertEqual(report["status"], "passed")
        for left, right in zip(report["checks"], report["checks"][1:]):
            self.assertLessEqual(left["completed_at"], right["started_at"])

    def test_parallel_drift_cancels_running_checks_and_refuses_new_admission(self):
        started, forbidden = self.root / "started", self.root / "forbidden"
        mutator = ("from pathlib import Path;import time;deadline=time.monotonic()+3\n"
                   f"while not Path({str(started)!r}).exists():\n"
                   " if time.monotonic()>deadline: raise SystemExit(7)\n"
                   " time.sleep(0.01)\n"
                   "Path('tracked.txt').write_text('mutated')\n")
        waiting = f"from pathlib import Path;import time;Path({str(started)!r}).touch();time.sleep(30)"
        never = f"from pathlib import Path;Path({str(forbidden)!r}).touch()"
        report = self.verify([self.check("mutator", mutator), self.check("waiting", waiting),
                              self.check("never", never)], jobs=2)
        self.assertEqual(report["status"], "source_drift")
        self.assertEqual(report["checks_not_run"], ["never"])
        self.assertEqual(report["checks"][1]["terminal"], "cancelled")
        self.assertFalse(forbidden.exists())
        self.assertTrue(all(check["output_drain_complete"] for check in report["checks"]))

    def test_parallelism_is_bounded_and_log_names_cannot_alias(self):
        for jobs in (0, -1, 33, True, 1.5):
            with self.subTest(jobs=jobs), self.assertRaisesRegex(ValueError, "jobs"):
                driver.verify(self.repository, self.output, jobs=jobs)
        self.assertFalse(self.output.exists())
        for names in (("duplicate", "duplicate"), ("../escaped",)):
            with self.subTest(names=names), mock.patch.object(driver, "profile_checks",
                    return_value=([self.check(name, "pass") for name in names], [])):
                with self.assertRaisesRegex(ValueError, "unique safe"):
                    driver.verify(self.repository, self.output)
                self.output.rmdir()

    def test_required_python_suite_rejects_empty_and_all_skipped_runs(self):
        directory = self.root / "suite"
        directory.mkdir()
        script = Path(driver.__file__).with_name("run_python_suite.py")
        source = directory / "test_fixture.py"
        fixtures = [
            ("# A present filename alone is not an executed test.\n", 1, b"discovered no tests"),
            ("import unittest\n@unittest.skip('unavailable')\nclass Case(unittest.TestCase):\n def test_one(self): pass\n",
             1, b"no successful test or subtest"),
            ("import unittest\nclass Case(unittest.TestCase):\n def test_one(self): self.assertTrue(True)\n",
             0, b"OK"),
            ("import unittest\nclass Case(unittest.TestCase):\n def test_one(self): self.fail('real failure')\n",
             1, b"FAILED"),
        ]
        for code, expected, marker in fixtures:
            source.write_text(code)
            outcome = subprocess.run([sys.executable, "-B", str(script), str(directory), "test_*.py"],
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10)
            with self.subTest(expected=expected, marker=marker):
                self.assertEqual(outcome.returncode, expected, outcome.stdout)
                self.assertIn(marker, outcome.stdout)

    def test_required_python_suite_counts_subtest_outcomes_without_skip_arithmetic(self):
        directory = self.root / "subtests"
        directory.mkdir()
        script = Path(driver.__file__).with_name("run_python_suite.py")
        fixtures = [
            ("import unittest\nclass Case(unittest.TestCase):\n def test_one(self):\n"
             "  for i in range(2):\n   with self.subTest(i=i): self.skipTest('unavailable')\n", 1),
            ("import unittest\nclass Case(unittest.TestCase):\n def test_one(self):\n"
             "  with self.subTest(i=1): self.assertTrue(True)\n"
             "  with self.subTest(i=2): self.skipTest('unavailable')\n", 0),
            ("import unittest\nclass Case(unittest.TestCase):\n @unittest.expectedFailure\n"
             " def test_one(self): self.fail('known issue')\n", 1),
            ("import unittest\nclass Case(unittest.TestCase):\n def test_one(self):\n"
             "  with self.subTest(i=1): self.assertTrue(True)\n"
             "  with self.subTest(i=2): self.fail('real failure')\n", 1),
        ]
        for source, expected in fixtures:
            (directory / "test_fixture.py").write_text(source)
            outcome = subprocess.run([sys.executable, "-B", str(script), str(directory), "test_*.py"],
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10)
            self.assertEqual(outcome.returncode, expected, outcome.stdout)


class ProcessEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fluxr-process-evidence-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def run_fixture(self, source, *, timeout=2, cap=1024):
        check = driver.Check("fixture", (sys.executable, "-c", source), "fixture_runtime_check")
        return driver.run_check(check, self.root, self.root / "fixture.log", timeout, cap)

    def test_output_limit_is_bounded_and_never_passes_after_zero_exit(self):
        result = self.run_fixture("import os;os.write(1,b'x'*8192)")
        self.assertEqual(result["terminal"], "output_limit")
        self.assertTrue(result["output_truncated"])
        self.assertEqual(result["bytes"], 1024)
        self.assertEqual(result["sha256"], hashlib.sha256(b"x" * 1024).hexdigest())

    def test_timeout_produces_terminal_failure_and_stable_log(self):
        start = time.monotonic()
        result = self.run_fixture("import time;time.sleep(20)", timeout=0.1)
        self.assertEqual(result["terminal"], "timeout")
        self.assertLess(time.monotonic() - start, 8)
        self.assertIsNotNone(result["exit_code"])
        self.assertEqual(result["sha256"], driver.hash_file(self.root / "fixture.log")["sha256"])

    @unittest.skipUnless(os.name == "posix", "POSIX process-group cleanup")
    def test_descendant_holding_output_pipe_is_not_successful_completion(self):
        source = "import subprocess,sys;subprocess.Popen([sys.executable,'-c','import time;time.sleep(20)'])"
        result = self.run_fixture(source, timeout=0.1)
        self.assertEqual(result["terminal"], "timeout")
        self.assertTrue(result["output_drain_complete"])

    def test_launch_failure_is_recorded_with_empty_log_identity(self):
        check = driver.Check("missing", (str(self.root / "absent-program"),), "fixture")
        result = driver.run_check(check, self.root, self.root / "launch.log", 1, 1024)
        self.assertEqual(result["terminal"], "launch_error")
        self.assertIsNone(result["exit_code"])
        self.assertIn("FileNotFoundError", result["launch_error"])
        self.assertEqual(result["sha256"], hashlib.sha256(b"").hexdigest())


if __name__ == "__main__":
    unittest.main()
