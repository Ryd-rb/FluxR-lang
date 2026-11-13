"""Frontend routing tests; real native behavior uses run_native_tests.py."""
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import fluxr


class NativeTestOptions(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fluxr-test-options-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.arguments = ["build", str(self.root / "source/project.json"),
                          str(self.root / "emitted"), "--target-dir", str(self.root / "target"),
                          "--logs", str(self.root / "logs")]

    def build_command(self, options):
        project = SimpleNamespace(rustflags=[], receipt={})
        output, errors = io.StringIO(), io.StringIO()
        def run(command, cwd, logs, timeout, env):
            logs.mkdir(parents=True)
            return {"argv": command, "exit_code": 0, "timed_out": False}
        with mock.patch.object(fluxr, "load_project", return_value=project) as load, \
             mock.patch.object(fluxr, "export_project", return_value=project) as export, \
             mock.patch.object(fluxr, "resolve_pinned_lock") as lock, \
             mock.patch.object(fluxr, "compile_checked_bindings") as compile_checked, \
             mock.patch.object(fluxr, "run_logged", side_effect=run) as runner, \
             redirect_stdout(output), redirect_stderr(errors):
            code = fluxr.main(self.arguments + options)
        self.assertEqual(code, 0, errors.getvalue())
        load.assert_called_once()
        export.assert_called_once()
        lock.assert_called_once()
        compile_checked.assert_called_once()
        runner.assert_called_once()
        receipt = json.loads((self.root / "logs/build.json").read_text())
        self.assertEqual(receipt["argv"], runner.call_args.args[0])
        self.assertEqual(json.loads(output.getvalue())["argv"], receipt["argv"])
        return receipt["argv"]

    def test_test_suffix_is_literal_and_after_all_cargo_options(self):
        suffix = ["module::test name", "--exact", "--ignored", "--test-threads", "1",
                  "--nocapture", "--skip", "$(touch impossible);echo no"]
        command = self.build_command(["--test", "--message-format", "json", "--", *suffix])
        self.assertEqual(command[:2], ["cargo", "test"])
        self.assertEqual(command[command.index("--") + 1:], suffix)
        self.assertEqual(command[command.index("--") - 1], "--offline")
        self.assertIn("--message-format=json", command[:command.index("--")])
        self.assertFalse((self.root / "impossible").exists())

    def test_ordinary_build_has_no_harness_separator(self):
        command = self.build_command([])
        self.assertEqual(command[:2], ["cargo", "build"])
        self.assertNotIn("--", command)

    def test_compile_only_test_does_not_gain_harness_options(self):
        command = self.build_command(["--test", "--no-run", "--profile", "dev"])
        self.assertEqual(command[:2], ["cargo", "test"])
        self.assertIn("--no-run", command)
        self.assertNotIn("--release", command)
        self.assertNotIn("--", command)

    def test_empty_suffix_is_valid_for_compile_only_test(self):
        command = self.build_command(["--test", "--no-run", "--"])
        self.assertIn("--no-run", command)
        self.assertNotIn("--", command)

    def test_invalid_suffix_modes_fail_before_project_emission(self):
        for options, message in [
            (["--", "--list"], "require --test"),
            (["--"], "require --test"),
            (["--test", "--no-run", "--", "--list"], "cannot be used with --no-run"),
            (["--no-run"], "--no-run requires --test"),
        ]:
            with self.subTest(options=options), mock.patch.object(fluxr, "load_project") as load, \
                 redirect_stderr(io.StringIO()) as errors:
                self.assertEqual(fluxr.main(self.arguments + options), 2)
                self.assertIn(message, errors.getvalue())
                load.assert_not_called()

    def test_unknown_frontend_options_do_not_silently_become_test_options(self):
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            fluxr.main(self.arguments + ["--test", "--include-ignored"])
        self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
