"""Independent format, preservation, and fault tests for clean source releases.

These tests exercise the actual ZIP bytes and filesystem results. Temporary
sources and archives live outside the checkout; no build products or receipts
are part of the release payload.
"""
from __future__ import annotations

from copy import copy, deepcopy
import hashlib
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import warnings
import zipfile

sys.dont_write_bytecode = True
BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE / "tools"))
import release_pack as release

SCRIPT = BASE / "tools/release_pack.py"
MANIFEST = "RELEASE-MANIFEST.json"


def canonical(value):
    """The documented encoding, intentionally independent of the packer."""
    return (json.dumps(value, ensure_ascii=False, sort_keys=True,
                       separators=(",", ":")) + "\n").encode("utf-8")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def tiny_requirements():
    """A valid small graph keeps malformed-archive cases fast and focused."""
    return {
        "format": "fluxr-implementation-requirements-v1",
        "tasks": [{"id": "I22", "depends_on": [], "acceptance_checks": [
            {"id": "I22-AC08", "criterion": "Validate release bytes", "evidence": "E01"}
        ]}],
        "release_profiles": [{"id": "source", "required_tasks": ["I22"],
                              "required_closure": ["I22"], "scope_dependencies": {}}],
        "evidence_profiles": {"E01": ["A separately retained validation result"]},
        "reference_sources": {}, "shared_contract_owners": {}, "decision_gates": [],
        "counts": {"tasks": 1, "acceptance_criteria": 1, "release_profiles": 1},
        "source_language": {"header": "fluxr 1", "qualification_scopes_are_languages": False},
    }


class ReleasePackTests(unittest.TestCase):
    def setUp(self):
        temporary_root = Path(tempfile.gettempdir()).resolve(strict=True)
        if temporary_root.is_relative_to(BASE):
            raise RuntimeError("release pack tests require TMPDIR outside the checkout")
        self.temporary = tempfile.TemporaryDirectory(prefix="fluxr-release-test-", dir=temporary_root)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        # Real identity files avoid inventing a different Cargo/toolchain profile.
        for name in ("AGENTS.md", "Cargo.toml", "Cargo.lock", "LICENSE", "README.md",
                     "rust-toolchain.toml", "requirements-dev.txt", ".gitignore",
                     "tools/check_requirements.py", "tools/release_pack.py",
                     "crates/fluxr-entry/Cargo.toml", "crates/fluxr-entry/src/lib.rs"):
            self.put(name, (BASE / name).read_bytes())
        self.put("docs/requirements.json", canonical(tiny_requirements()))
        self.put("src/lib.rs", b"pub fn answer() -> u32 { 42 }\n")
        self.put("src/main.rs", b"fn main() {}\n")
        self.put("tests/fixtures/history.json", b'{"fixture":"history"}\n')
        self.put("tests/fixtures/evidence.json", b'{"fixture":"evidence"}\n')
        self.put("tests/fixtures/checkpoint.json", b'{"fixture":"checkpoint"}\n')
        self.put("tools/agent_pipeline/evidence.py", b"def evidence(): return 42\n")
        self.put("src/checkpoint.rs", b"pub fn checkpoint() {}\n")
        self.put("docs/agent-fair-dispatch.md", b"Functional dispatch API documentation.\n")
        self.counter = 0

    def put(self, name, data, mode=0o644, source=None):
        path = (source or self.source) / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(mode)
        return path

    def next_path(self, suffix=".zip"):
        self.counter += 1
        return self.root / (f"archive-{self.counter}" + suffix)

    def pack(self, source=None, version="25"):
        path = self.next_path()
        release.create_archive(source or self.source, path, pack_version=version)
        return path

    def payload(self, archive):
        with zipfile.ZipFile(archive) as stream:
            return [(copy(info), stream.read(info)) for info in stream.infolist()]

    def manifest(self, archive):
        with zipfile.ZipFile(archive) as stream:
            return json.loads(stream.read(MANIFEST))

    def write_members(self, members, *, comment=b""):
        archive = self.next_path()
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED,
                                 compresslevel=9) as stream:
                for info, data in members:
                    stream.writestr(info, data, compress_type=info.compress_type, compresslevel=9)
                stream.comment = comment
        return archive

    @staticmethod
    def member(name, data=b"x\n", mode=stat.S_IFREG | 0o644):
        info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
        # Keep malformed separators in the ZIP bytes on Windows as well.
        info.filename = name
        info.create_system = 3
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = mode << 16
        return info, data

    def replace_manifest(self, archive, update, *, encode=canonical):
        members = self.payload(archive)
        for index, (info, data) in enumerate(members):
            if info.filename == MANIFEST:
                value = json.loads(data)
                update(value)
                members[index] = info, encode(value)
                break
        return self.write_members(members)

    def rebind(self, archive, replacements):
        """Forge a self-consistent manifest to test checks beyond ZIP checksums."""
        members = [(info, replacements.get(info.filename, data))
                   for info, data in self.payload(archive)]
        return self.bind_members(members)

    def bind_members(self, members):
        """Bind even adversarial entries, so inventory mismatch cannot mask bugs."""
        manifest = json.loads(next(data for info, data in members if info.filename == MANIFEST))
        rows = []
        for index, (info, data) in enumerate(members):
            if info.filename == MANIFEST:
                continue
            rows.append({"path": info.filename, "mode": f"{(info.external_attr >> 16) & 0o777:04o}",
                         "size": len(data), "sha256": sha(data)})
        rows.sort(key=lambda row: row["path"])
        manifest["files"] = rows
        manifest["source_digest"] = sha(canonical({"pack_version": manifest["pack_version"], "files": rows}))
        for index, (info, _) in enumerate(members):
            if info.filename == MANIFEST:
                members[index] = info, canonical(manifest)
                break
        return self.write_members(sorted(members, key=lambda member: member[0].filename))

    def assert_refused(self, archive, **kwargs):
        with self.assertRaises(release.ReleasePackError):
            release.validate_archive(archive, **kwargs)

    def test_manifest_binds_paths_modes_sizes_bytes_and_pack_version(self):
        self.put("tools/executable.sh", b"#!/bin/sh\nexit 0\n", 0o755)
        archive = self.pack()
        value = self.manifest(archive)
        expected = []
        for path in sorted(self.source.rglob("*")):
            if path.is_file():
                data = path.read_bytes()
                expected.append({"path": path.relative_to(self.source).as_posix(),
                                 "mode": "0755" if path.stat().st_mode & 0o111 else "0644",
                                 "size": len(data), "sha256": sha(data)})
        expected.sort(key=lambda row: row["path"])
        self.assertEqual(value["schema"], "fluxr-source-release-v1")
        self.assertEqual(value["pack_version"], "25")
        self.assertEqual(value["files"], expected)
        self.assertEqual(value["source_digest"], sha(canonical({"pack_version": "25", "files": expected})))
        with zipfile.ZipFile(archive) as stream:
            self.assertEqual(stream.read(MANIFEST), canonical(value))
        release.validate_archive(archive, source=self.source, pack_version="25")

    def test_members_have_canonical_safe_metadata(self):
        self.put("tools/run.sh", b"#!/bin/sh\n", 0o755)
        with zipfile.ZipFile(self.pack()) as stream:
            names = stream.namelist()
            self.assertEqual(names, sorted(names))
            self.assertEqual(len(names), len(set(names)))
            self.assertEqual(stream.comment, b"")
            self.assertIsNone(stream.testzip())
            for info in stream.infolist():
                with self.subTest(path=info.filename):
                    self.assertFalse(info.is_dir())
                    self.assertEqual(info.create_system, 3)
                    self.assertEqual(info.date_time, (1980, 1, 1, 0, 0, 0))
                    self.assertEqual(info.compress_type, zipfile.ZIP_DEFLATED)
                    self.assertEqual(info.extra, b"")
                    self.assertEqual(info.comment, b"")
                    self.assertFalse(info.flag_bits & (1 | 8))
                    self.assertIn(info.external_attr >> 16, (stat.S_IFREG | 0o644, stat.S_IFREG | 0o755))

    def test_repeated_builds_ignore_location_creation_order_timestamps_and_read_permissions(self):
        first = self.pack()
        other = self.root / "different checkout name"
        other.mkdir()
        for path in reversed(sorted(p for p in self.source.rglob("*") if p.is_file())):
            copied = self.put(path.relative_to(self.source).as_posix(), path.read_bytes(), 0o600, source=other)
            os.utime(copied, (172800, 172800))
        second = self.pack(source=other)
        self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_source_digest_changes_with_content_path_mode_or_version(self):
        original = self.manifest(self.pack())["source_digest"]
        path = self.source / "src/checkpoint.rs"
        old = path.read_bytes()
        path.write_bytes(old + b"// changed\n")
        self.assertNotEqual(original, self.manifest(self.pack())["source_digest"])
        path.write_bytes(old)
        path.rename(path.with_name("renamed.rs"))
        self.assertNotEqual(original, self.manifest(self.pack())["source_digest"])
        path.with_name("renamed.rs").rename(path)
        if os.name == "posix":
            path.chmod(0o755)
            self.assertNotEqual(original, self.manifest(self.pack())["source_digest"])
            path.chmod(0o644)
        self.assertNotEqual(original, self.manifest(self.pack(version="24.5"))["source_digest"])

    def test_normalized_unicode_names_are_supported_and_repeat_exactly(self):
        name = "src/caf\u00e9.rs"
        data = b"pub fn cafe() {}\n"
        self.put(name, data)
        archive = self.pack()
        self.assertEqual(archive.read_bytes(), self.pack().read_bytes())
        with zipfile.ZipFile(archive) as stream:
            self.assertEqual(stream.read(name), data)
            self.assertTrue(stream.getinfo(name).flag_bits & 0x800)
            self.assertIn(name.encode("utf-8"), stream.read(MANIFEST))
        destination = self.root / "unicode-extraction"
        release.extract_archive(archive, destination, source=self.source)
        self.assertEqual((destination / name).read_bytes(), data)

    def test_clean_policy_preserves_functional_words_and_excludes_known_output_locations(self):
        excluded = [".git/config", "target/release/output", "web/pkg/generated.js",
                    "tools/__pycache__/stale.pyc", ".pytest_cache/result", ".venv/bin/python",
                    ".fluxr-agent/evidence.json", ".knowledge/notes.json", "docs/modernization.md",
                    "recovery/checkpoint.zip",
                    "dispatch-prompts/task.md", "src/.env", "tests/.env.local",
                    "web/build/bundle.js", "web/dist/site.js", "web/coverage/index.html",
                    "tools/output/generated.zip", "tests/results/test.json",
                    "tools/logs/build.log", "docs/build.log"]
        for name in excluded:
            self.put(name, b"Generated work material: exclude.\n")
        self.put("tests/fixtures/build.log", b"A functional parser fixture, not this build's log.\n")
        archive = self.pack()
        with zipfile.ZipFile(archive) as stream:
            names = set(stream.namelist())
            for name in excluded:
                self.assertNotIn(name, names)
            for name in ("LICENSE", "Cargo.lock", "rust-toolchain.toml", "requirements-dev.txt",
                         "tests/fixtures/history.json", "tests/fixtures/evidence.json",
                         "tests/fixtures/checkpoint.json", "src/checkpoint.rs",
                         "tools/agent_pipeline/evidence.py", "docs/agent-fair-dispatch.md",
                         "tests/fixtures/build.log"):
                self.assertEqual(stream.read(name), (self.source / name).read_bytes())
        release.validate_archive(archive, source=self.source)

    def test_actual_pinned_third_party_and_tokenizer_inputs_survive_exactly(self):
        pins = json.loads((BASE / "benchmarks/multicore-reference.json").read_text())
        self.put("benchmarks/multicore-reference.json", (BASE / "benchmarks/multicore-reference.json").read_bytes())
        for name in pins["files"]:
            full = "third_party/multicore-reference/" + name
            self.put(full, (BASE / full).read_bytes())
        for name in ("tools/fr1/NOTICE", "tools/fr1/o200k-fr1.txt.gz",
                     "examples/multicore-server/LICENSE.upstream", "examples/multicore-server/Cargo.lock"):
            self.put(name, (BASE / name).read_bytes())
        archive = self.pack()
        destination = self.root / "extracted-pins"
        release.extract_archive(archive, destination, source=self.source)
        for name, digest in pins["files"].items():
            with self.subTest(path=name):
                self.assertEqual(sha((destination / "third_party/multicore-reference" / name).read_bytes()), digest)
        for name in ("tools/fr1/NOTICE", "tools/fr1/o200k-fr1.txt.gz",
                     "examples/multicore-server/LICENSE.upstream", "examples/multicore-server/Cargo.lock"):
            self.assertEqual((destination / name).read_bytes(), (BASE / name).read_bytes())

    def test_archive_inventory_and_file_modes_equal_the_extracted_tree(self):
        self.put("tools/run.sh", b"#!/bin/sh\nexit 0\n", 0o755)
        archive = self.pack()
        destination = self.root / "extract"
        release.extract_archive(archive, destination, source=self.source)
        with zipfile.ZipFile(archive) as stream:
            expected = set(stream.namelist())
            actual = {path.relative_to(destination).as_posix() for path in destination.rglob("*") if path.is_file()}
            self.assertEqual(actual, expected)
            expected_dirs = {parent.as_posix() for name in expected for parent in Path(name).parents if parent != Path(".")}
            actual_dirs = {path.relative_to(destination).as_posix() for path in destination.rglob("*") if path.is_dir()}
            self.assertEqual(actual_dirs, expected_dirs)
            for info in stream.infolist():
                path = destination / info.filename
                self.assertFalse(path.is_symlink())
                self.assertEqual(path.read_bytes(), stream.read(info))
                if os.name == "posix":
                    self.assertEqual(stat.S_IMODE(path.stat().st_mode), (info.external_attr >> 16) & 0o777)
        # A validated extracted source is itself reproducible input; the old
        # manifest must not recursively become a source payload member.
        self.assertEqual(self.pack(source=destination).read_bytes(), archive.read_bytes())

    def test_validate_with_source_detects_content_mode_missing_and_added_files(self):
        archive = self.pack()
        path = self.source / "src/lib.rs"
        old = path.read_bytes()
        path.write_bytes(old + b"// altered\n")
        self.assert_refused(archive, source=self.source)
        path.write_bytes(old)
        if os.name == "posix":
            path.chmod(0o755)
            self.assert_refused(archive, source=self.source)
            path.chmod(0o644)
        path.unlink()
        self.assert_refused(archive, source=self.source)
        path.write_bytes(old)
        self.put("src/added.rs", b"pub fn added() {}\n")
        self.assert_refused(archive, source=self.source)

    def test_forged_consistent_manifest_does_not_prove_equality_to_trusted_source(self):
        forged = self.rebind(self.pack(), {"src/lib.rs": b"pub fn answer() -> u32 { 43 }\n"})
        release.validate_archive(forged)
        self.assert_refused(forged, source=self.source)

    def test_version_expectation_is_checked(self):
        archive = self.pack()
        self.assert_refused(archive, pack_version="24.5")
        destination = self.root / "wrong-version"
        with self.assertRaises(release.ReleasePackError):
            release.extract_archive(archive, destination, pack_version="24.5")
        self.assertFalse(destination.exists())

    def test_output_inside_source_is_refused_before_any_product_change(self):
        path = self.source / "archive.zip"
        with self.assertRaises(release.ReleasePackError):
            release.create_archive(self.source, path)
        self.assertFalse(path.exists())

    def test_extraction_inside_compared_source_is_refused_without_source_changes(self):
        archive = self.pack()
        destination = self.source / "extracted"
        before = {path.relative_to(self.source).as_posix(): path.read_bytes()
                  for path in self.source.rglob("*") if path.is_file()}
        with self.assertRaisesRegex(release.ReleasePackError, "outside"):
            release.extract_archive(archive, destination, source=self.source)
        self.assertFalse(destination.exists())
        after = {path.relative_to(self.source).as_posix(): path.read_bytes()
                 for path in self.source.rglob("*") if path.is_file()}
        self.assertEqual(after, before)

    def test_existing_archive_is_never_overwritten(self):
        archive = self.root / "existing.zip"
        archive.write_bytes(b"keep the existing file")
        with self.assertRaises((release.ReleasePackError, FileExistsError)):
            release.create_archive(self.source, archive)
        self.assertEqual(archive.read_bytes(), b"keep the existing file")

    @unittest.skipUnless(os.name == "posix", "POSIX symlink and file mode semantics")
    def test_source_symlinks_and_special_files_are_refused(self):
        for kind in ("file-link", "directory-link", "fifo"):
            path = self.source / "src/unsafe"
            if kind == "file-link":
                path.symlink_to(self.source / "src/lib.rs")
            elif kind == "directory-link":
                path.symlink_to(self.source / "tests", target_is_directory=True)
            else:
                os.mkfifo(path)
            try:
                with self.subTest(kind=kind), self.assertRaises(release.ReleasePackError):
                    self.pack()
            finally:
                path.unlink()

    @unittest.skipUnless(os.name == "posix", "POSIX special permission bits")
    def test_source_setid_or_sticky_permissions_are_refused(self):
        path = self.put("tools/special.py", b"pass\n")
        for mode in (0o4644, 0o2644, 0o1644):
            path.chmod(mode)
            with self.subTest(mode=oct(mode)), self.assertRaises(release.ReleasePackError):
                self.pack()
        path.chmod(0o644)

    @unittest.skipUnless(os.name == "posix", "POSIX symlink semantics")
    def test_source_root_or_ancestor_symlink_is_refused(self):
        link = self.root / "linked-source"
        link.symlink_to(self.source, target_is_directory=True)
        with self.assertRaises(release.ReleasePackError):
            self.pack(source=link)
        parent = self.root / "linked-parent"
        parent.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(release.ReleasePackError):
            self.pack(source=parent / "source")

    @unittest.skipUnless(os.name == "posix", "POSIX permits names rejected for portable ZIPs")
    def test_unsafe_source_names_fail_instead_of_being_normalized(self):
        for name in ("CON.txt", "aux", "trailing.", "trailing ", "back\\slash.rs", "colon:name", "tab\tname"):
            path = self.put("src/" + name, b"unsafe source name\n")
            try:
                with self.subTest(name=name), self.assertRaises(release.ReleasePackError):
                    self.pack()
            finally:
                path.unlink()

    def test_source_case_collisions_include_parent_directories(self):
        for first, second in (("src/Name.rs", "src/name.rs"),
                              ("src/Mixed/a.rs", "src/mixed/b.rs")):
            one, two = self.put(first, b"one"), self.put(second, b"two")
            merged_parent = (str(one.parent) != str(two.parent)
                             and os.path.samefile(one.parent, two.parent))
            if os.path.samefile(one, two) or merged_parent:
                self.skipTest("filesystem merges case aliases")
            try:
                with self.subTest(paths=(first, second)), self.assertRaises(release.ReleasePackError):
                    self.pack()
            finally:
                one.unlink()
                two.unlink()
                if first.count("/") > 1:
                    one.parent.rmdir()
                    two.parent.rmdir()

    def test_unsafe_archive_names_are_rejected(self):
        members = self.payload(self.pack())
        for name in ("../escape", "/absolute", "src/../../escape", "C:/drive", "C:drive",
                     "\\\\server\\share", "src\\escape", "src//double", "src/./dot",
                     "src/trailing.", "src/trailing ", "src/CON.txt", "src/NUL", "src/control\x01"):
            bad = self.bind_members(members + [self.member(name)])
            with self.subTest(name=name):
                self.assert_refused(bad)

    def test_duplicate_casefold_unicode_and_file_prefix_aliases_are_rejected(self):
        members = self.payload(self.pack())
        duplicates = [
            [self.member("src/lib.rs", (self.source / "src/lib.rs").read_bytes())],
            [self.member("src/LIB.rs")],
            [self.member("SRC/other.rs")],
            [self.member("src/caf\u00e9.rs"), self.member("src/cafe\u0301.rs")],
            # Both names are NFC; casefold itself introduces normalization
            # differences, so collision keys need normalization after folding.
            [self.member("src/\u0390.rs"), self.member("src/\u03aa\u0301.rs")],
            [self.member("src")],
        ]
        for additions in duplicates:
            with self.subTest(paths=[info.filename for info, _ in additions]):
                self.assert_refused(self.bind_members(members + additions))

    def test_directory_symlink_special_and_unsafe_permission_entries_are_rejected(self):
        members = self.payload(self.pack())
        for mode in (stat.S_IFDIR | 0o755, stat.S_IFLNK | 0o644, stat.S_IFIFO | 0o644,
                     stat.S_IFREG | 0o777, stat.S_IFREG | 0o4755, stat.S_IFREG | 0o600):
            with self.subTest(mode=oct(mode)):
                self.assert_refused(self.bind_members(members + [self.member("src/unsafe", b"lib.rs", mode)]))
        self.assert_refused(self.bind_members(members + [self.member("src/empty/", b"", stat.S_IFDIR | 0o755)]))

    def test_noncanonical_member_metadata_or_order_is_rejected(self):
        original = self.payload(self.pack())
        modifications = {
            "timestamp": lambda info: setattr(info, "date_time", (2001, 2, 3, 4, 5, 6)),
            "creator": lambda info: setattr(info, "create_system", 0),
            "compression": lambda info: setattr(info, "compress_type", zipfile.ZIP_STORED),
            "comment": lambda info: setattr(info, "comment", b"unexpected metadata"),
            "extra": lambda info: setattr(info, "extra", b"\xfe\xca\x00\x00"),
        }
        for name, modify in modifications.items():
            members = [(copy(info), data) for info, data in original]
            modify(members[0][0])
            with self.subTest(metadata=name):
                self.assert_refused(self.write_members(members))
        self.assert_refused(self.write_members(list(reversed(original))))
        self.assert_refused(self.write_members(original, comment=b"archive comment"))

    def test_prefix_suffix_and_null_name_bytes_are_rejected(self):
        data = self.pack().read_bytes()
        for changed in (b"arbitrary executable prefix" + data, data + b"hidden trailing bytes",
                        data.replace(b"README.md", b"READ\x00E.md")):
            path = self.next_path()
            path.write_bytes(changed)
            self.assert_refused(path)

    def test_local_central_name_flags_and_method_disagreements_are_rejected(self):
        data = self.pack().read_bytes()
        self.assertEqual(data[:4], b"PK\x03\x04")
        changed_name = bytearray(data)
        changed_name[30] ^= 1
        changed_flag = bytearray(data)
        struct.pack_into("<H", changed_flag, 6, struct.unpack_from("<H", data, 6)[0] | 8)
        changed_method = bytearray(data)
        struct.pack_into("<H", changed_method, 8, zipfile.ZIP_STORED)
        for kind, changed in (("name", changed_name), ("flags", changed_flag), ("method", changed_method)):
            path = self.next_path()
            path.write_bytes(changed)
            with self.subTest(kind=kind):
                self.assert_refused(path)

    def test_corrupt_compressed_bytes_and_truncated_zip_are_rejected(self):
        data = bytearray(self.pack().read_bytes())
        name_size, extra_size = struct.unpack_from("<HH", data, 26)
        data[30 + name_size + extra_size] ^= 0xFF
        corrupt = self.next_path()
        corrupt.write_bytes(data)
        self.assert_refused(corrupt)
        truncated = self.next_path()
        truncated.write_bytes(data[:-22])
        self.assert_refused(truncated)

    def test_decompression_rejects_a_stream_larger_than_its_declared_size(self):
        data = bytearray(self.pack().read_bytes())
        cursor = struct.unpack_from("<I", data, len(data) - 22 + 16)[0]
        while True:
            name_size, extra_size, comment_size = struct.unpack_from("<HHH", data, cursor + 28)
            name = bytes(data[cursor + 46:cursor + 46 + name_size]).decode("utf-8")
            if name == "src/checkpoint.rs":
                size = struct.unpack_from("<I", data, cursor + 24)[0]
                offset = struct.unpack_from("<I", data, cursor + 42)[0]
                struct.pack_into("<I", data, cursor + 24, size - 1)
                struct.pack_into("<I", data, offset + 22, size - 1)
                break
            cursor += 46 + name_size + extra_size + comment_size
        archive = self.next_path()
        archive.write_bytes(data)
        self.assert_refused(archive)

    def test_unused_bytes_inside_a_compressed_member_are_rejected(self):
        original = self.pack()
        with zipfile.ZipFile(original) as stream:
            last = stream.infolist()[-1]
        data = bytearray(original.read_bytes())
        old_directory = struct.unpack_from("<I", data, len(data) - 22 + 16)[0]
        tail = b"unclaimed bytes inside the final member"
        data[old_directory:old_directory] = tail
        new_directory = old_directory + len(tail)
        struct.pack_into("<I", data, len(data) - 22 + 16, new_directory)
        struct.pack_into("<I", data, last.header_offset + 18, last.compress_size + len(tail))
        cursor = new_directory
        while True:
            name_size, extra_size, comment_size = struct.unpack_from("<HHH", data, cursor + 28)
            name = bytes(data[cursor + 46:cursor + 46 + name_size]).decode("utf-8")
            if name == last.filename:
                struct.pack_into("<I", data, cursor + 20, last.compress_size + len(tail))
                break
            cursor += 46 + name_size + extra_size + comment_size
        archive = self.next_path()
        archive.write_bytes(data)
        self.assert_refused(archive)

    def test_manifest_field_faults_are_rejected(self):
        archive = self.pack()
        faults = [
            lambda value: value.update(schema="unknown"),
            lambda value: value.update(source_digest="0" * 64),
            lambda value: value.update(extra="not part of the schema"),
            lambda value: value["files"][0].update(size=True),
            lambda value: value["files"][0].update(size=-1),
            lambda value: value["files"][0].update(size="12"),
            lambda value: value["files"][0].update(mode="0777"),
            lambda value: value["files"][0].update(sha256="not a digest"),
            lambda value: value["files"][0].update(path="../unsafe"),
            lambda value: value["files"].append(deepcopy(value["files"][0])),
            lambda value: value["files"].pop(),
            lambda value: value["files"].reverse(),
        ]
        for index, fault in enumerate(faults):
            with self.subTest(fault=index):
                self.assert_refused(self.replace_manifest(archive, fault))

    def test_manifest_must_be_present_unique_canonical_json_without_duplicate_keys(self):
        archive = self.pack()
        members = self.payload(archive)
        entry = next(member for member in members if member[0].filename == MANIFEST)
        self.assert_refused(self.write_members([member for member in members if member[0].filename != MANIFEST]))
        self.assert_refused(self.write_members(members + [entry]))
        self.assert_refused(self.replace_manifest(archive, lambda value: None,
                           encode=lambda value: json.dumps(value, indent=2).encode()))
        self.assert_refused(self.replace_manifest(archive, lambda value: None,
                           encode=lambda value: b'{"schema":"fluxr-source-release-v1",' + canonical(value)[1:]))

    def test_unlisted_missing_and_modified_payloads_are_rejected(self):
        members = self.payload(self.pack())
        self.assert_refused(self.write_members(members + [self.member("src/unlisted.rs")]))
        self.assert_refused(self.write_members([member for member in members if member[0].filename != "src/lib.rs"]))
        changed = [(info, b"changed contents" if info.filename == "src/lib.rs" else data) for info, data in members]
        self.assert_refused(self.write_members(changed))

    def test_regenerated_hashes_cannot_hide_inconsistent_requirements(self):
        archive = self.pack()
        faults = []
        value = tiny_requirements(); value["counts"]["tasks"] = 2; faults.append(value)
        value = tiny_requirements(); value["tasks"].append(deepcopy(value["tasks"][0])); faults.append(value)
        value = tiny_requirements(); value["tasks"][0]["depends_on"] = ["I99"]; faults.append(value)
        value = tiny_requirements(); value["release_profiles"][0]["required_closure"] = []; faults.append(value)
        for index, value in enumerate(faults):
            with self.subTest(fault=index):
                self.assert_refused(self.rebind(archive, {"docs/requirements.json": canonical(value)}))

    def test_corrupt_pinned_input_is_refused_even_when_archive_digests_are_regenerated(self):
        original = b"Pinned third-party license fixture\n"
        self.put("third_party/multicore-reference/LICENSE", original)
        self.put("benchmarks/multicore-reference.json", canonical({
            "format": "fluxr-pinned-multicore-reference-v1", "revision": "1" * 40,
            "files": {"LICENSE": sha(original)},
        }))
        archive = self.pack()
        forged = self.rebind(archive, {"third_party/multicore-reference/LICENSE": b"tampered license\n"})
        self.assert_refused(forged)

    def test_retained_multicore_example_requires_upstream_license(self):
        self.put("examples/multicore-server/src/main.fluxr", b"fluxr 1\n")
        with self.assertRaises(release.ReleasePackError):
            self.pack()
        self.put("examples/multicore-server/LICENSE.upstream",
                 (BASE / "examples/multicore-server/LICENSE.upstream").read_bytes())
        archive = self.pack()
        without_license = [(info, data) for info, data in self.payload(archive)
                           if info.filename != "examples/multicore-server/LICENSE.upstream"]
        self.assert_refused(self.bind_members(without_license))

    def test_archive_byte_limit_is_checked_before_zip_parsing_or_decompression(self):
        archive = self.next_path()
        with archive.open("wb") as stream:
            stream.truncate(64 * 1024 * 1024 + 1)
        with mock.patch.object(release.zlib, "decompressobj", side_effect=AssertionError("must not decompress")), \
                self.assertRaisesRegex(release.ReleasePackError, "size.*bound"):
            release.validate_archive(archive)

    def test_member_count_limit_is_checked_before_decompression(self):
        archive = self.write_members([self.member(MANIFEST, b"")] + [
            self.member(f"src/file-{index:05d}.rs", b"") for index in range(8192)])
        with mock.patch.object(release.zlib, "decompressobj", side_effect=AssertionError("must not decompress")):
            self.assert_refused(archive)

    def test_declared_raw_and_manifest_size_limits_are_checked_before_decompression(self):
        original = self.pack().read_bytes()
        for member, size in (("src/lib.rs", 256 * 1024 * 1024 + 1),
                             (MANIFEST, 2 * 1024 * 1024 + 1)):
            data = bytearray(original)
            cursor = 0
            while True:
                cursor = data.find(b"PK\x01\x02", cursor)
                self.assertNotEqual(cursor, -1)
                name_size, extra_size, comment_size = struct.unpack_from("<HHH", data, cursor + 28)
                name = bytes(data[cursor + 46:cursor + 46 + name_size]).decode("utf-8")
                if name == member:
                    struct.pack_into("<I", data, cursor + 24, size)
                    offset = struct.unpack_from("<I", data, cursor + 42)[0]
                    struct.pack_into("<I", data, offset + 22, size)
                    break
                cursor += 46 + name_size + extra_size + comment_size
            archive = self.next_path()
            archive.write_bytes(data)
            with self.subTest(member=member), mock.patch.object(release.zlib, "decompressobj", side_effect=AssertionError("must not decompress")):
                self.assert_refused(archive)

    def test_existing_extraction_destinations_are_never_modified(self):
        archive = self.pack()
        kinds = ("file", "directory", "symlink") if os.name == "posix" else ("file", "directory")
        for kind in kinds:
            destination = self.root / kind
            if kind == "file":
                destination.write_bytes(b"retain")
            elif kind == "directory":
                destination.mkdir()
                (destination / "retain").write_bytes(b"retain")
            else:
                destination.symlink_to(self.source, target_is_directory=True)
            with self.subTest(kind=kind), self.assertRaises((release.ReleasePackError, FileExistsError)):
                release.extract_archive(archive, destination)
            if kind == "file":
                self.assertEqual(destination.read_bytes(), b"retain")
            elif kind == "directory":
                self.assertEqual((destination / "retain").read_bytes(), b"retain")
            else:
                self.assertTrue(destination.is_symlink())

    @unittest.skipUnless(os.name == "posix", "POSIX symlink semantics")
    def test_symlink_destination_parent_is_refused_without_outside_writes(self):
        archive = self.pack()
        outside = self.root / "outside"
        outside.mkdir()
        link = self.root / "parent-link"
        link.symlink_to(outside, target_is_directory=True)
        with self.assertRaises(release.ReleasePackError):
            release.extract_archive(archive, link / "extracted")
        self.assertEqual(list(outside.iterdir()), [])

    def test_invalid_archive_is_refused_before_final_extraction_tree_exists(self):
        archive = self.replace_manifest(self.pack(), lambda value: value.update(source_digest="0" * 64))
        destination = self.root / "must-not-exist"
        before = {path.name for path in self.root.iterdir()}
        with self.assertRaises(release.ReleasePackError):
            release.extract_archive(archive, destination)
        self.assertFalse(destination.exists())
        self.assertEqual({path.name for path in self.root.iterdir()}, before)

    def test_extraction_verifies_exact_written_inventory_bytes_and_modes_before_publication(self):
        archive = self.pack()
        original_verify = release._verify_extracted

        def missing(staging):
            (staging / "src/lib.rs").unlink()

        def changed(staging):
            (staging / "src/lib.rs").write_bytes(b"incorrectly written payload")

        def extra(staging):
            (staging / "unexpected.txt").write_bytes(b"unexpected")

        def empty_directory(staging):
            (staging / "unexpected-directory").mkdir()

        corruptions = [missing, changed, extra, empty_directory]
        if os.name == "posix":
            corruptions.extend([
                lambda staging: (staging / "src/lib.rs").chmod(0o755),
                lambda staging: (staging / "src").chmod(0o700),
            ])
        for index, corrupt in enumerate(corruptions):
            destination = self.root / f"tampered-extraction-{index}"
            before = {path.name for path in self.root.iterdir()}

            def corrupt_then_verify(staging, expected):
                corrupt(staging)
                return original_verify(staging, expected)

            with self.subTest(corruption=index), \
                    mock.patch.object(release, "_verify_extracted", side_effect=corrupt_then_verify), \
                    self.assertRaises(release.ReleasePackError):
                release.extract_archive(archive, destination)
            self.assertFalse(destination.exists())
            self.assertEqual({path.name for path in self.root.iterdir()}, before)

    def test_cli_create_validate_extract_and_errors(self):
        archive = self.next_path()
        destination = self.root / "cli-extract"
        environment = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1", "PYTHONHASHSEED": "171", "TZ": "Pacific/Honolulu"}

        def invoke(*args):
            return subprocess.run([sys.executable, "-B", str(SCRIPT), *map(str, args)],
                                  env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  timeout=30, check=False)

        for arguments in (("create", self.source, archive),
                          ("validate", archive, "--source", self.source, "--pack-version", "25"),
                          ("extract", archive, destination, "--source", self.source)):
            result = invoke(*arguments)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            self.assertIsInstance(json.loads(result.stdout), dict)
        self.assertEqual(archive.read_bytes(), self.pack().read_bytes())
        result = invoke("validate", archive, "--pack-version", "24.5")
        self.assertEqual(result.returncode, 1)
        self.assertNotIn(b"Traceback", result.stderr)
        self.assertEqual((destination / "src/lib.rs").read_bytes(), (self.source / "src/lib.rs").read_bytes())


if __name__ == "__main__":
    unittest.main()
