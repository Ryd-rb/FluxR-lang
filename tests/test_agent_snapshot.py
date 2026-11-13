"""Real Git/CAS preparation and adversarial complete-source identity contracts."""
from copy import deepcopy
from dataclasses import replace
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import ContentStore, action_key, canonical_bytes, content_digest
from agent_pipeline import snapshot as source
from agent_pipeline.identity import load_json_bytes


def git(root, *args, data=None):
    return subprocess.run(["git", "-C", str(root), *args], input=data, check=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout.decode().strip()


def action(digest):
    return {"schema": "fluxr-action-v1", "argv": ["cargo", "test", "--locked"],
            "working_directory": ".", "environment": {"LANG": "C"},
            "inputs": {}, "outputs": ["build/test.log"], "source_snapshot": digest,
            "toolchain": "a" * 64, "runner": "b" * 64, "policy": "c" * 64,
            "platform": "linux-x86_64", "target": "x86_64-unknown-linux-gnu", "profile": "debug"}


def cleanup(root):
    source._cleanup(root)


@unittest.skipUnless(sys.platform.startswith("linux"), "snapshot v1 requires Linux")
class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.name", "Snapshot test")
        git(self.repo, "config", "user.email", "snapshot-test@example.invalid")
        (self.repo / "src").mkdir()
        (self.repo / "src" / "model.flux").write_bytes(b"model\x00\xff\n")
        (self.repo / "run").write_bytes(b"#!/bin/sh\nexit 0\n")
        (self.repo / "run").chmod(0o755)
        (self.repo / "é whitespace").write_text("unicode data")
        self.commit = self.save()
        self.store = ContentStore(self.root / "objects")

    def tearDown(self):
        cleanup(self.root)
        self.temp.cleanup()

    def save(self):
        git(self.repo, "add", "--all")
        git(self.repo, "commit", "-q", "--allow-empty", "-m", "fixture")
        return git(self.repo, "rev-parse", "HEAD")

    def capture(self, **kwargs):
        return source.capture_git(self.repo, self.commit, self.store, **kwargs)

    def manifest(self, capture=None):
        capture = capture or self.capture()
        return load_json_bytes(self.store.get(capture["snapshot"]))

    def test_exact_bytes_modes_and_two_copies_with_no_shared_inodes(self):
        capture = self.capture()
        value = self.manifest(capture)
        self.assertEqual(capture["git_tree"], git(self.repo, "rev-parse", "HEAD^{tree}"))
        self.assertEqual(capture["files"], 3)
        self.assertEqual(capture["entries"], 4)
        one, two = self.root / "one", self.root / "two"
        for destination in (one, two):
            result = source.materialize(self.store, capture["snapshot"], destination)
            self.assertTrue(result["verified"])
            self.assertFalse(result["execution_isolated"])
            self.assertEqual(source.verify_materialization(self.store, capture["snapshot"], destination)["entries"], 4)
            self.assertEqual((destination / "src/model.flux").read_bytes(), b"model\x00\xff\n")
            self.assertEqual((destination / "run").stat().st_mode & 0o777, 0o555)
            self.assertEqual((destination / "src/model.flux").stat().st_mode & 0o777, 0o444)
            self.assertEqual((destination / "run").stat().st_mtime_ns, 0)
        for entry in value["entries"]:
            if entry["digest"]:
                paths = [one / entry["path"], two / entry["path"], self.store.root / entry["digest"]]
                self.assertEqual(len({path.stat().st_ino for path in paths}), 3)

    def test_commit_metadata_worktree_and_environment_do_not_rebind_source(self):
        original = self.capture()
        git(self.repo, "commit", "-q", "--allow-empty", "-m", "different metadata")
        second = git(self.repo, "rev-parse", "HEAD")
        (self.repo / "src/model.flux").write_text("dirty")
        (self.repo / "untracked").write_text("ignored")
        with patch.dict(os.environ, {"GIT_DIR": "/wrong", "GIT_WORK_TREE": "/wrong", "GIT_NAMESPACE": "wrong"}):
            again = source.capture_git(self.repo, second, self.store)
        self.assertNotEqual(original["git_commit"], again["git_commit"])
        self.assertEqual(original["snapshot"], again["snapshot"])
        self.assertEqual(original["git_tree"], again["git_tree"])

    def test_stale_refs_abbreviations_expressions_and_noncommit_objects_refuse(self):
        for value in ("HEAD", self.commit[:8], self.commit + "^", self.commit.upper(), "-h", "x" * 40,
                      git(self.repo, "rev-parse", "HEAD^{tree}")):
            with self.subTest(value=value), self.assertRaises(ValueError):
                source.capture_git(self.repo, value, self.store)

    def test_git_replace_refs_cannot_redirect_pinned_capture(self):
        original = self.capture()
        (self.repo / "src/model.flux").write_text("replaced")
        changed = self.save()
        git(self.repo, "replace", self.commit, changed)
        self.assertEqual(self.capture()["snapshot"], original["snapshot"])

    def test_symlink_and_submodule_refuse_without_following_them(self):
        outside = self.root / "outside"
        outside.write_text("must not import")
        (self.repo / "link").symlink_to(outside)
        self.commit = self.save()
        with self.assertRaisesRegex(ValueError, "symlinks/gitlinks"):
            self.capture()
        git(self.repo, "rm", "link")
        git(self.repo, "update-index", "--add", "--cacheinfo", f"160000,{self.commit},submodule")
        git(self.repo, "commit", "-q", "-m", "submodule")
        self.commit = git(self.repo, "rev-parse", "HEAD")
        with self.assertRaisesRegex(ValueError, "symlinks/gitlinks"):
            self.capture()
        self.assertEqual(outside.read_text(), "must not import")

    def test_capture_quotas_include_metadata_and_reject_before_payload_reads(self):
        capture = self.capture()
        for limits, message in ((replace(source.Limits(), entries=3), "entry quota"),
                (replace(source.Limits(), file_bytes=1), "file byte quota"),
                (replace(source.Limits(), total_bytes=1), "aggregate source"),
                (replace(source.Limits(), manifest_bytes=capture["manifest_bytes"] - 1), "manifest byte quota"),
                (replace(source.Limits(), path_bytes=5), "path is too"),
                (replace(source.Limits(), path_depth=1), "path is too")):
            with self.subTest(limits=limits), patch.object(self.store, "put_stream", side_effect=AssertionError("blob read")):
                with self.assertRaisesRegex(ValueError, message):
                    self.capture(limits=limits)

    def test_limits_reject_invalid_and_unbounded_values(self):
        for changes in ({"entries": 0}, {"file_bytes": 2**40}, {"seconds": True}, {"seconds": 3601},
                        {"total_bytes": -1}, {"manifest_bytes": 2**32}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                replace(source.Limits(), **changes)

    def test_cancelled_capture_does_not_return_or_change_source(self):
        count = 0
        def cancelled():
            nonlocal count
            count += 1
            return count > 5
        with self.assertRaises(source.Cancelled):
            self.capture(cancelled=cancelled)
        self.assertEqual(git(self.repo, "status", "--porcelain"), "")
        self.assertFalse(list(self.store.root.glob(".pending-*")))

    def test_absent_or_corrupt_git_objects_refuse(self):
        with self.assertRaises(ValueError):
            source.capture_git(self.repo, "0" * 40, self.store)
        oid = git(self.repo, "rev-parse", "HEAD:src/model.flux")
        object_path = self.repo / ".git/objects" / oid[:2] / oid[2:]
        object_path.chmod(0o600)
        object_path.write_bytes(b"corrupt")
        with self.assertRaises(ValueError):
            self.capture()

    def test_corrupt_commit_root_and_nested_tree_cannot_forge_provenance(self):
        import zlib
        for kind, revision in (("commit", "HEAD"), ("tree", "HEAD^{tree}"), ("tree", "HEAD:src")):
            oid = git(self.repo, "rev-parse", revision)
            path = self.repo / ".git/objects" / oid[:2] / oid[2:]
            compressed = path.read_bytes()
            raw = zlib.decompress(compressed)
            _, _, content = raw.partition(b"\0")
            # A commit message or a tree entry name can change while retaining
            # structurally readable Git bytes under the old loose-object path.
            changed = content + b"changed message\n" if kind == "commit" else content.replace(
                b"run\0" if revision == "HEAD^{tree}" else b"model.flux\0",
                b"ran\0" if revision == "HEAD^{tree}" else b"model.flox\0", 1)
            self.assertNotEqual(changed, content)
            path.chmod(0o600)
            path.write_bytes(zlib.compress(f"{kind} {len(changed)}\0".encode() + changed))
            try:
                with self.subTest(kind=kind, revision=revision), self.assertRaisesRegex(ValueError, "content differs|unavailable/corrupt object"):
                    self.capture()
            finally:
                path.write_bytes(compressed)

    def test_existing_or_concurrently_created_destination_never_overwritten(self):
        capture = self.capture()
        for name, kind in (("dir", "dir"), ("file", "file"), ("link", "link")):
            target = self.root / name
            if kind == "dir": target.mkdir()
            elif kind == "file": target.write_text("retained")
            else: target.symlink_to(self.root / "absent")
            with self.subTest(kind=kind), self.assertRaises(FileExistsError):
                source.materialize(self.store, capture["snapshot"], target)
        destination = self.root / "raced"
        real = source._rename_no_replace
        def collide(fd, staging, final):
            destination.mkdir()
            (destination / "retained").write_text("concurrent winner")
            return real(fd, staging, final)
        with patch.object(source, "_rename_no_replace", side_effect=collide), self.assertRaises(OSError):
            source.materialize(self.store, capture["snapshot"], destination)
        self.assertEqual((destination / "retained").read_text(), "concurrent winner")
        self.assertFalse(list(self.root.glob(".fluxr-source-*")))

    def test_cancellation_and_missing_corrupt_blobs_leave_no_final_tree(self):
        capture = self.capture()
        value = self.manifest(capture)
        calls = 0
        def cancelled():
            nonlocal calls
            calls += 1
            return calls > 4
        with self.assertRaises(source.Cancelled):
            source.materialize(self.store, capture["snapshot"], self.root / "cancelled", cancelled=cancelled)
        self.assertFalse((self.root / "cancelled").exists())
        self.assertFalse(list(self.root.glob(".fluxr-source-*")))
        entry = next(e for e in value["entries"] if e["digest"])
        payload = self.store.root / entry["digest"]
        original = payload.read_bytes()
        payload.unlink()
        with self.assertRaises(FileNotFoundError):
            source.materialize(self.store, capture["snapshot"], self.root / "missing")
        payload.write_bytes(b"wrong")
        with self.assertRaises(ValueError):
            source.materialize(self.store, capture["snapshot"], self.root / "corrupt")
        self.assertFalse((self.root / "missing").exists())
        self.assertFalse((self.root / "corrupt").exists())
        self.assertFalse(list(self.root.glob(".fluxr-source-*")))
        payload.write_bytes(original)
        self.assertTrue(source.verify_snapshot(self.store, capture["snapshot"]))

    def test_during_copy_corruption_is_rehashed_not_credited_by_manifest(self):
        capture = self.capture()
        actual = self.store.get
        blobs = {e["digest"] for e in self.manifest(capture)["entries"] if e["digest"]}
        def substituted(digest, **kwargs):
            data = actual(digest, **kwargs)
            if digest in blobs:
                return bytes(byte ^ 1 for byte in data)
            return data
        with patch.object(self.store, "get", side_effect=substituted), self.assertRaisesRegex(ValueError, "content changed"):
            source.materialize(self.store, capture["snapshot"], self.root / "substituted")
        self.assertFalse((self.root / "substituted").exists())

    def test_manifest_structural_faults_and_size_lies_refuse(self):
        value = self.manifest()
        faults = []
        x = deepcopy(value); x["profile"] = "future"; faults.append(x)
        x = deepcopy(value); x["entries"].reverse(); faults.append(x)
        x = deepcopy(value); x["entries"].append(x["entries"][-1]); faults.append(x)
        x = deepcopy(value); x["entries"] = [e for e in x["entries"] if e["path"] != "src"]; faults.append(x)
        x = deepcopy(value); x["entries"][0]["size"] += 1; faults.append(x)
        for path in ("../outside", "/absolute", "src/.git/config", "C:/host", "./dot", "a\\b"):
            x = deepcopy(value); x["entries"][0]["path"] = path; faults.append(x)
        for x in faults:
            digest = self.store.put(canonical_bytes(x))
            with self.subTest(value=x), self.assertRaises((ValueError, FileNotFoundError)):
                source.verify_snapshot(self.store, digest)

    def test_strict_manifest_and_payload_limits_refuse_before_read_allocation(self):
        import agent_pipeline.store as content
        original_fdopen = content.os.fdopen
        class NoRead:
            def __init__(self, descriptor, mode):
                self.file = original_fdopen(descriptor, mode)
            def __enter__(self): return self
            def __exit__(self, *args): self.file.close()
            def fileno(self): return self.file.fileno()
            def read(self, *args): raise AssertionError("oversized object was read before metadata refusal")
        large = self.store.put(b"x" * content.MAX_BLOB_BYTES)
        with patch.object(content.os, "fdopen", side_effect=NoRead):
            with self.assertRaisesRegex(ValueError, "invalid type or size"):
                self.store.get(large, maximum_bytes=1)
            with self.assertRaisesRegex(ValueError, "invalid type or size"):
                source.verify_snapshot(self.store, large, limits=replace(source.Limits(), manifest_bytes=1))
        value = self.manifest()
        entry = next(e for e in value["entries"] if e["digest"])
        entry["digest"], entry["size"] = large, 1
        manifest = self.store.put(canonical_bytes(value))
        actual = self.store.get
        def guarded(digest, **kwargs):
            if digest == large:
                with patch.object(content.os, "fdopen", side_effect=NoRead):
                    return actual(digest, **kwargs)
            return actual(digest, **kwargs)
        with patch.object(self.store, "get", side_effect=guarded):
            with self.assertRaisesRegex(ValueError, "invalid type or size"):
                source.verify_snapshot(self.store, manifest)
            with self.assertRaisesRegex(ValueError, "invalid type or size"):
                source.materialize(self.store, manifest, self.root / "oversized-payload")
        self.assertFalse((self.root / "oversized-payload").exists())
        self.assertEqual(self.store.get(self.store.put(b""), maximum_bytes=0), b"")
        for bound in (-1, True, content.MAX_BLOB_BYTES + 1):
            with self.assertRaises(ValueError): self.store.get(large, maximum_bytes=bound)

    def test_noncanonical_manifest_refuses(self):
        import json
        digest = self.store.put(json.dumps(self.manifest(), indent=2).encode())
        with self.assertRaisesRegex(ValueError, "not canonical"):
            source.verify_snapshot(self.store, digest)

    def test_materialization_verifier_rejects_missing_extra_content_modes_and_links(self):
        capture = self.capture()
        for fault in ("missing", "extra", "content", "mode", "symlink", "hardlink", "dirtype", "mtime"):
            destination = self.root / fault
            source.materialize(self.store, capture["snapshot"], destination)
            destination.chmod(0o755)
            file = destination / "run"
            if fault == "missing": file.unlink()
            elif fault == "extra": (destination / "extra").write_text("new")
            elif fault == "content": file.chmod(0o755); file.write_text("bad"); file.chmod(0o555)
            elif fault == "mode": file.chmod(0o444)
            elif fault == "symlink": file.unlink(); file.symlink_to(self.repo / "run")
            elif fault == "hardlink": file.unlink(); os.link(self.repo / "run", file); file.chmod(0o555)
            elif fault == "mtime": os.utime(file, (12, 12))
            else: cleanup(destination / "src"); (destination / "src").write_text("wrong")
            destination.chmod(0o555)
            with self.subTest(fault=fault), self.assertRaises(ValueError):
                source.verify_materialization(self.store, capture["snapshot"], destination)

    def test_cancel_at_last_digest_never_returns_verified_success(self):
        import hashlib
        from types import SimpleNamespace
        capture = self.capture()
        destination = self.root / "last-digest"
        source.materialize(self.store, capture["snapshot"], destination)
        calls, cancelled = 0, False
        original = hashlib.sha256
        class Hasher:
            def __init__(self):
                nonlocal calls
                calls += 1
                self.index, self.inner = calls, original()
            def update(self, data):
                self.inner.update(data)
            def hexdigest(self):
                nonlocal cancelled
                if self.index == capture["files"]:
                    cancelled = True
                return self.inner.hexdigest()
        with patch.object(source, "hashlib", SimpleNamespace(sha256=Hasher)):
            with self.assertRaises(source.Cancelled):
                source.verify_materialization(self.store, capture["snapshot"], destination,
                                               cancelled=lambda: cancelled)
        self.assertTrue(cancelled)

    def test_binding_validates_exact_full_snapshot_inputs_and_working_directory(self):
        capture = self.capture()
        value = self.manifest(capture)
        payload = next(e for e in value["entries"] if e["path"] == "src/model.flux")
        specification = action(capture["snapshot"])
        specification["inputs"] = {payload["path"]: payload["digest"]}
        bound = source.bind_action(self.store, capture["snapshot"], specification)
        self.assertEqual(bound["action_key"], action_key(specification))
        self.assertFalse(bound["hermetic_execution"])
        self.assertFalse(bound["cache_acceptance_authorized"])
        specification["argv"].clear()
        self.assertTrue(bound["action"]["argv"])
        for field, value in (("source_snapshot", "f" * 64), ("inputs", {"missing": "f" * 64}),
                             ("inputs", {"src/model.flux": "f" * 64}), ("inputs", {"src": "f" * 64}),
                             ("working_directory", "run"), ("working_directory", "missing"),
                             ("outputs", ["src"]), ("outputs", ["run/nested"])):
            changed = action(capture["snapshot"]); changed[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                source.bind_action(self.store, capture["snapshot"], changed)

    def test_all_committed_dependency_classes_conservatively_change_full_action_identity(self):
        baseline = self.capture()
        baseline_key = action_key(action(baseline["snapshot"]))
        for path in ("negative-created", "config/options", "generated/output", "schemas/model", "effects/policy", "tests/discovered"):
            file = self.repo / path
            file.parent.mkdir(exist_ok=True)
            file.write_text("changed")
            revision = self.save()
            captured = source.capture_git(self.repo, revision, self.store)
            self.assertNotEqual(captured["snapshot"], baseline["snapshot"])
            self.assertNotEqual(action_key(action(captured["snapshot"])), baseline_key)
        # An external resource is deliberately not made complete by tree capture.
        outside = self.root / "external-resource"
        outside.write_text("not covered")
        same = source.capture_git(self.repo, revision, self.store)
        self.assertEqual(same["snapshot"], captured["snapshot"])
        self.assertFalse(source.bind_action(self.store, same["snapshot"], action(same["snapshot"]))["hermetic_execution"])

    def test_mode_change_is_observable_even_when_file_bytes_are_identical(self):
        baseline = self.capture()
        (self.repo / "run").chmod(0o644)
        self.commit = self.save()
        changed = self.capture()
        self.assertNotEqual(baseline["snapshot"], changed["snapshot"])

    def test_empty_tree_round_trip(self):
        git(self.repo, "rm", "-r", ".")
        self.commit = self.save()
        capture = self.capture()
        self.assertEqual(capture["entries"], 0)
        result = source.materialize(self.store, capture["snapshot"], self.root / "empty")
        self.assertTrue(result["verified"])
        self.assertEqual(source.verify_snapshot(self.store, capture["snapshot"])["source_bytes"], 0)


if __name__ == "__main__":
    unittest.main()
