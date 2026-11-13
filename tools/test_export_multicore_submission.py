"""Source-export and shell lifecycle tests; fake Cargo is not native evidence.

The fixture Cargo executable validates the build interface and creates a small
process probe. A separate guarded real build must qualify native compilation.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import export_multicore_submission as export


SOURCE = export.ROOT / "examples/multicore-server"


def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(path):
    return {p.relative_to(path).as_posix(): sha(p) for p in path.rglob("*") if p.is_file()}


class SubmissionExportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def export(self):
        destination = self.root / "submission with spaces"
        receipt = export.export_submission(SOURCE, destination)
        return destination, receipt

    def fake_cargo(self):
        cargo = self.root / "cargo-probe"
        probe = (
            f"#!{sys.executable}\n"
            "import json, os\n"
            "print(json.dumps({'pid':os.getpid(), 'config':{k:os.environ.get(k) for k in "
            "['DATABASE_URL','JWT_SECRET','HOST','PORT']}}))\n"
        )
        cargo.write_text(
            f"#!{sys.executable}\n"
            "import json, os, pathlib, sys\n"
            "args=sys.argv[1:]\n"
            "with open(os.environ['FLUXR_TEST_CARGO_ARGS'],'a') as log:\n"
            " log.write(json.dumps({'argv':args,'cwd':os.getcwd(),"
            "'toolchain':os.environ.get('RUSTUP_TOOLCHAIN')})+'\\n')\n"
            "if args[0]=='tree':sys.exit(0)\n"
            "if os.environ.get('FLUXR_TEST_BUILD_FAIL'):sys.exit(7)\n"
            "if os.environ.get('FLUXR_TEST_NO_BINARY'):sys.exit(0)\n"
            "target=pathlib.Path(args[args.index('--target-dir')+1])/'release'\n"
            "target.mkdir(parents=True,exist_ok=True)\n"
            f"(target/'conduit').write_text({probe!r})\n"
        )
        cargo.chmod(0o755)
        env = os.environ.copy()
        env.update(CARGO=str(cargo), FLUXR_TEST_CARGO_ARGS=str(self.root / "cargo-args.jsonl"))
        return env

    def run_build(self, destination, env):
        return subprocess.run(["bash", str(destination / "build.sh")], cwd=self.root,
                              env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)

    def test_export_contains_exact_compiler_verified_sources_and_no_build_output(self):
        destination, receipt = self.export()
        self.assertEqual(receipt["structured_functions"], 7)
        self.assertEqual(receipt["rust_toolchain"], "1.99.1")
        self.assertEqual(sha(destination / "compiler.py"), sha(Path(export.native.__file__)))
        for row in receipt["files"]:
            path = destination / row["path"]
            self.assertEqual(path.stat().st_size, row["bytes"])
            self.assertEqual(sha(path), row["sha256"])
        for path in (destination / "program").rglob("*"):
            if path.is_file():
                self.assertEqual(path.read_bytes(), (SOURCE / path.relative_to(destination / "program")).read_bytes())
        self.assertFalse((destination / "bin").exists())
        self.assertFalse((destination / "target").exists())
        self.assertTrue((destination / "runtime/src/runtime.rs").is_file())
        self.assertTrue((destination / "runtime/crates/fluxr-entry/src/lib.rs").is_file())
        self.assertFalse(list((destination / "program").rglob("*.rs")))
        self.assertTrue((destination / "program/Cargo.lock").is_file())
        self.assertIn("Front: direct", (destination / "README.md").read_text())
        self.assertIn("7 functions use counted native", (destination / "README.md").read_text())

    def test_no_overwrite_of_file_directory_or_symlink(self):
        for name, kind in (("file", "file"), ("directory", "directory"), ("link", "link")):
            target = self.root / name
            if kind == "file": target.write_text("keep")
            elif kind == "directory": target.mkdir(); (target / "keep").write_text("keep")
            else: target.symlink_to(self.root / "missing")
            with self.subTest(kind=kind), self.assertRaisesRegex(export.native.NativeError, "already exists"):
                export.export_submission(SOURCE, target)
        self.assertEqual((self.root / "file").read_text(), "keep")
        self.assertEqual((self.root / "directory/keep").read_text(), "keep")
        self.assertTrue((self.root / "link").is_symlink())
        self.assertFalse(list(self.root.glob(".fluxr-submission-*")))

    def test_refuses_export_inside_input_sources(self):
        source = self.root / "source"
        shutil.copytree(SOURCE, source)
        before = inventory(source)
        with self.assertRaisesRegex(export.native.NativeError, "outside source"):
            export.export_submission(source, source / "nested")
        self.assertEqual(inventory(source), before)

    def test_source_mutation_or_duplicate_provenance_refuses_before_output(self):
        source = self.root / "source"
        shutil.copytree(SOURCE, source)
        path = source / "src/main.fluxr"
        original = path.read_bytes()
        path.write_bytes(original + b"\n")
        with self.assertRaisesRegex(export.native.NativeError, "differs from import provenance"):
            export.export_submission(source, self.root / "out")
        self.assertFalse((self.root / "out").exists())
        path.write_bytes(original)
        provenance = source / "IMPORT_PROVENANCE.json"
        data = json.loads(provenance.read_text())
        data["files"][-1] = data["files"][0]
        provenance.write_text(json.dumps(data))
        with self.assertRaisesRegex(export.native.NativeError, "duplicate import provenance"):
            export.export_submission(source, self.root / "out")
        self.assertFalse((self.root / "out").exists())

    def test_changed_compiler_between_admission_and_copy_refuses(self):
        reader, reads = export.native.read_regular, 0
        compiler = Path(export.native.__file__).resolve()

        def changed(path, *args):
            nonlocal reads
            data = reader(path, *args)
            if path.resolve() == compiler:
                reads += 1
                if reads == 2: return data + b"\n# simulated concurrent compiler edit\n"
            return data

        with patch.object(export.native, "read_regular", side_effect=changed):
            with self.assertRaisesRegex(export.native.NativeError, "compiler changed"):
                export.export_submission(SOURCE, self.root / "out")
        self.assertEqual(reads, 2)
        self.assertFalse((self.root / "out").exists())

    def test_build_scripts_parse_and_installer_is_only_a_pinned_source_file(self):
        destination, _ = self.export()
        for script in ("build.sh", "start.sh", "install.sh"):
            result = subprocess.run(["bash", "-n", str(destination / script)], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(os.access(destination / script, os.X_OK))
        install = (destination / "install.sh").read_text()
        self.assertIn("--default-toolchain 1.99.1", install)
        self.assertIn("python3", install)
        self.assertEqual((destination / "rust-toolchain.toml").read_text(),
                         '[toolchain]\nchannel = "1.99.1"\nprofile = "minimal"\n')

    def test_build_and_failed_rebuild_preserve_sources_binary_logs_and_foreground_env(self):
        destination, _ = self.export()
        env = self.fake_cargo()
        source_before = inventory(destination / "program")
        initial = self.run_build(destination, env)
        self.assertEqual(initial.returncode, 0, initial.stderr.decode())
        binary = destination / "bin/conduit"
        before_binary = binary.read_bytes()
        self.assertTrue(os.access(binary, os.X_OK))
        arguments = next(json.loads(line) for line in (self.root / "cargo-args.jsonl").read_text().splitlines() if json.loads(line)["argv"][0] == "build")
        self.assertEqual(arguments["argv"][0], "build")
        for required in ("--release", "--locked"):
            self.assertIn(required, arguments["argv"])
        self.assertEqual(arguments["toolchain"], "1.99.1")
        self.assertTrue(Path(arguments["cwd"]).is_relative_to(destination))
        self.assertFalse(Path(arguments["cwd"]).is_relative_to(destination / "program"))
        env["FLUXR_TEST_BUILD_FAIL"] = "1"
        failed = self.run_build(destination, env)
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(binary.read_bytes(), before_binary)
        self.assertEqual(inventory(destination / "program"), source_before)
        self.assertFalse(list(destination.glob(".fluxr-build.*")))
        self.assertFalse(list((destination / "bin").glob(".conduit.*")))
        logs = list((destination / "build-logs").iterdir())
        self.assertEqual(len(logs), 2)
        self.assertEqual(sorted(json.loads((p / "build.json").read_text())["exit_code"] for p in logs), [0, 7])
        runtime = os.environ.copy()
        expected = {"DATABASE_URL": "postgres://user:password@127.0.0.1:5432/db",
                    "JWT_SECRET": "literal demo secret", "HOST": "127.0.0.1", "PORT": "31234"}
        runtime.update(expected)
        process = subprocess.Popen(["bash", str(destination / "start.sh")], cwd=self.root,
                                   env=runtime, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        output, error = process.communicate(timeout=10)
        self.assertEqual(process.returncode, 0, error)
        observed = json.loads(output)
        self.assertEqual(observed["config"], expected)
        self.assertEqual(observed["pid"], process.pid, "start.sh must exec the server in the foreground")

    def test_successful_cargo_without_binary_cannot_replace_previous_executable(self):
        destination, _ = self.export()
        env = self.fake_cargo()
        self.assertEqual(self.run_build(destination, env).returncode, 0)
        before = (destination / "bin/conduit").read_bytes()
        env["FLUXR_TEST_NO_BINARY"] = "1"
        result = self.run_build(destination, env)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((destination / "bin/conduit").read_bytes(), before)
        self.assertFalse(list(destination.glob(".fluxr-build.*")))
        self.assertFalse(list((destination / "bin").glob(".conduit.*")))

    def test_build_refuses_symlink_output_directory(self):
        destination, _ = self.export()
        foreign = self.root / "foreign"
        foreign.mkdir()
        (foreign / "keep").write_text("keep")
        (destination / "bin").symlink_to(foreign, target_is_directory=True)
        result = self.run_build(destination, self.fake_cargo())
        self.assertEqual(result.returncode, 2)
        self.assertEqual((foreign / "keep").read_text(), "keep")
        self.assertFalse(list(destination.glob(".fluxr-build.*")))


if __name__ == "__main__": unittest.main()
