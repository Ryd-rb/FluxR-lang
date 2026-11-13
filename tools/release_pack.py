#!/usr/bin/env python3
"""Create, validate and exactly extract bounded, deterministic FluxR source ZIPs.

Only the Python standard library is needed. This is a source packaging contract,
not authentication, a Rust build, or product qualification. See docs/release-pack.md.
"""
from __future__ import annotations

import argparse
import binascii
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import sys
import tempfile
import unicodedata
import zipfile
import zlib

# Import the verifier shipped beside this tool, never code from an inspected ZIP.
if str(Path(__file__).resolve().parent) not in sys.path:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_requirements

PACK_VERSION = "25"
SCHEMA = "fluxr-source-release-v1"
MANIFEST_NAME = "RELEASE-MANIFEST.json"
MAX_MEMBERS = 8192  # Includes the generated digest manifest.
MAX_ARCHIVE_BYTES = 64 * 1024 * 1024
MAX_UNCOMPRESSED_BYTES = 256 * 1024 * 1024
MAX_MANIFEST_BYTES = 2 * 1024 * 1024
FIXED_TIMESTAMP = (1980, 1, 1, 0, 0, 0)
FR1_ALPHABET_SHA256 = "ad5ec04db8fd979a5a8c14c8329284fe27d7253173c268c076253ea212418ecd"
REQUIRED_FILES = frozenset({
    "AGENTS.md", "Cargo.toml", "Cargo.lock", "LICENSE", "README.md",
    "rust-toolchain.toml", "requirements-dev.txt", "docs/requirements.json",
    "src/lib.rs", "src/main.rs", "crates/fluxr-entry/Cargo.toml",
    "crates/fluxr-entry/src/lib.rs",
})
ROOT_FILES = REQUIRED_FILES | frozenset({
    ".gitignore", ".gitattributes", "NOTICE", "COPYING",
})
SOURCE_ROOTS = frozenset({
    ".github", "src", "crates", "tools", "tests", "docs", "examples",
    "benchmarks", "third_party", "web", "syntaxes",
})
# Scope work-record exclusions to their actual locations. In particular, never
# blacklist the words evidence/history/checkpoint/dispatch within source names.
EXCLUDED_ROOTS = frozenset({
    ".git", ".hg", ".svn", ".idea", ".vscode", ".DS_Store", ".env", ".fluxr-agent", ".knowledge",
    "target", "build", "dist", "out", "output", "outputs", "generated",
    "artifacts", "release", "releases", "release-evidence", "evidence",
    "results", "logs", "recovery", "recovery-checkpoints", "checkpoint",
    "checkpoints", "history", "archive", "archives", "old", "backup",
    "backups", "dispatch", "dispatch-prompts", "dispatch_prompts", "prompts",
    "task-prompts", "session", "sessions", "notes", "tmp", "temp", "cache",
    "caches", "toolchain", "toolchains", "patches", "bundles",
})
CACHE_COMPONENTS = frozenset({
    ".git", ".hg", ".svn", "__pycache__", ".pytest_cache", ".mypy_cache",
    ".ruff_cache", ".tox", ".venv", "venv", "node_modules", "target",
})
EXCLUDED_PREFIXES = (
    "web/pkg/", "web/dist/", "web/build/", "web/coverage/",
    "tools/output/", "tools/outputs/", "tools/results/", "tools/logs/",
    "tests/output/", "tests/outputs/", "tests/results/", "tests/logs/",
    "docs/archive/", "docs/archives/", "docs/history/", "docs/dispatch-prompts/",
    "docs/dispatch_prompts/", "docs/task-prompts/", "tools/dispatch-prompts/",
)
DISPATCH_PROMPT = re.compile(r"(?:dispatch[-_]prompts?|agent[-_]dispatch[-_]prompts?|task[-_]prompts?)(?:[._-].*)?\Z", re.I)
PREVIOUS_PACK = re.compile(r"FluxR_Agent_Implementation_Pack_.*\.zip\Z", re.I)
HEX_DIGEST = re.compile(r"[0-9a-f]{64}\Z")
VERSION = re.compile(r"[0-9]+(?:\.[0-9]+)*\Z")
WINDOWS_RESERVED = {"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"} | {
    prefix + str(i) for prefix in ("COM", "LPT") for i in range(1, 10)
}


class ReleasePackError(ValueError):
    """An invalid source, archive, or extraction request."""


def require(condition, message):
    if not condition:
        raise ReleasePackError(message)


def canonical_json(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True,
                       separators=(",", ":"), allow_nan=False) + "\n").encode("utf-8")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def safe_path(name):
    """Require a portable, unambiguous relative file path; do not repair names."""
    require(isinstance(name, str) and bool(name), "empty or non-string path")
    try:
        encoded = name.encode("utf-8")
    except UnicodeError as error:
        raise ReleasePackError("invalid Unicode path") from error
    require(len(encoded) <= 1024 and unicodedata.normalize("NFC", name) == name,
            "path must be NFC and at most 1024 UTF-8 bytes: " + repr(name))
    parts = name.split("/")
    for part in parts:
        require(part not in {"", ".", ".."}, "unsafe path: " + repr(name))
        require(len(part.encode("utf-8")) <= 255, "path component too long")
        folded = unicodedata.normalize("NFKC", part)
        require(folded not in {"", ".", ".."}, "unsafe normalized path: " + repr(name))
        require(not part.endswith((".", " ")) and not folded.endswith((".", " ")),
                "trailing dot or space in path: " + repr(name))
        require(not any(c in '<>:"\\|?*' or unicodedata.category(c).startswith("C")
                        for c in part), "nonportable path: " + repr(name))
        require(not any(c in '/\\<>:"|?*' for c in folded),
                "nonportable normalized path: " + repr(name))
        require(folded.split(".", 1)[0].upper() not in WINDOWS_RESERVED,
                "reserved device path: " + repr(name))
    return name


def unique_paths(names):
    """Check complete paths and implicit parent directories for aliasing."""
    seen, aliases = set(), {}
    for name in names:
        safe_path(name)
        require(name not in seen, "duplicate path: " + name)
        seen.add(name)
        parts = name.split("/")
        for end in range(1, len(parts) + 1):
            prefix = "/".join(parts[:end])
            # Case folding itself may introduce decomposed sequences (e.g.
            # Greek iota with diaeresis/acute), so normalize again afterwards.
            key = unicodedata.normalize("NFKC", unicodedata.normalize("NFKC", prefix).casefold())
            require(key not in aliases or aliases[key] == prefix,
                    "case or Unicode path collision: " + name)
            aliases[key] = prefix
    for name in seen:
        require(not any("/".join(name.split("/")[:end]) in seen
                        for end in range(1, len(name.split("/")))),
                "file/directory path collision: " + name)


def exclusion_reason(name):
    """Explicit packaging policy, shared by source selection and ZIP validation."""
    parts = name.split("/")
    if name == MANIFEST_NAME:
        return "regenerated release manifest"
    if name == "docs/modernization.md":
        return "historical checkpoint evidence"
    if parts[0] in EXCLUDED_ROOTS or parts[0].startswith(".env."):
        return "generated or private top-level work area"
    if any(part == ".env" or part.startswith(".env.") for part in parts):
        return "private environment file"
    if any(part in CACHE_COMPONENTS for part in parts):
        return "build or repository cache"
    if name.startswith(EXCLUDED_PREFIXES):
        return "historical documentation or dispatch prompts"
    if parts[-1].endswith((".pyc", ".pyo")):
        return "Python bytecode"
    if DISPATCH_PROMPT.fullmatch(parts[-1]) or PREVIOUS_PACK.fullmatch(parts[-1]):
        return "dispatch prompt or previous pack"
    if parts[-1] in {"build.log", "test.log", "cargo-build.log", "cargo-test.log"} and not (
            "fixtures" in parts or name.startswith("third_party/")):
        return "named build or test log"
    return None


def allowed_path(name):
    if "/" in name:
        return name.split("/", 1)[0] in SOURCE_ROOTS
    return name in ROOT_FILES or bool(re.fullmatch(r"(?:LICENSE|NOTICE|COPYING)(?:\.[A-Za-z0-9_-]+)?", name))


def checked_local_path(path):
    """Reject symlinks in every existing ancestor, without resolving through them."""
    path = Path(os.path.abspath(os.fspath(path)))
    for parent in reversed((path, *path.parents)):
        try:
            mode = parent.lstat().st_mode
        except FileNotFoundError:
            continue
        require(not stat.S_ISLNK(mode), "symlink in local path: " + str(parent))
        if parent != path:
            require(stat.S_ISDIR(mode), "non-directory ancestor: " + str(parent))
    return path


def _identity(st):
    return st.st_dev, st.st_ino, st.st_mode, st.st_size, st.st_mtime_ns


def _read_regular(path, maximum):
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode), "expected regular file: " + str(path))
    require(before.st_size <= maximum, "file exceeds size bound: " + str(path))
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_BINARY", 0)
    with os.fdopen(os.open(path, flags), "rb") as stream:
        opened = os.fstat(stream.fileno())
        # Windows can report different ctime values through the path and handle.
        require(_identity(opened) == _identity(before)
                and (os.name == "nt" or opened.st_ctime_ns == before.st_ctime_ns),
                "file changed before read: " + str(path))
        data = stream.read(maximum + 1)
        after = os.fstat(stream.fileno())
    current = path.lstat()
    require(len(data) == before.st_size
            and _identity(after) == _identity(opened) and after.st_ctime_ns == opened.st_ctime_ns
            and _identity(current) == _identity(before) and current.st_ctime_ns == before.st_ctime_ns,
            "file changed during read: " + str(path))
    return data, before.st_mode


def _json(data, label):
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=check_requirements.unique_object,
                          parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
    except (ValueError, UnicodeError, RecursionError) as error:
        raise ReleasePackError("invalid " + label + ": " + str(error)) from error


def _validate_product(payload):
    missing = REQUIRED_FILES - payload.keys()
    require(not missing, "missing required product files: " + ", ".join(sorted(missing)))
    try:
        check_requirements.validate(_json(payload["docs/requirements.json"][0], "requirements"))
    except (ValueError, KeyError, TypeError, AttributeError, RecursionError) as error:
        raise ReleasePackError("requirements invalid: " + str(error)) from error
    if any(name.startswith("examples/multicore-server/") for name in payload):
        require("examples/multicore-server/LICENSE.upstream" in payload,
                "missing attributed multicore example license")
    pin_name, prefix = "benchmarks/multicore-reference.json", "third_party/multicore-reference/"
    inventory = {name[len(prefix):]: item for name, item in payload.items() if name.startswith(prefix)}
    if pin_name in payload or inventory:
        require(pin_name in payload and bool(inventory), "missing pinned third-party inventory")
        pin = _json(payload[pin_name][0], "third-party pin")
        require(isinstance(pin, dict) and pin.get("format") == "fluxr-pinned-multicore-reference-v1",
                "invalid third-party pin format")
        require(isinstance(pin.get("revision"), str) and re.fullmatch(r"[0-9a-f]{40}", pin["revision"]),
                "invalid third-party revision")
        files = pin.get("files")
        require(isinstance(files, dict) and set(files) == set(inventory),
                "pinned third-party inventory differs")
        unique_paths(files)
        require("LICENSE" in files, "missing third-party license")
        for name, digest in files.items():
            require(isinstance(digest, str) and HEX_DIGEST.fullmatch(digest)
                    and sha256(inventory[name][0]) == digest, "third-party pin mismatch: " + name)
    alphabet, notice = "tools/fr1/o200k-fr1.txt.gz", "tools/fr1/NOTICE"
    if alphabet in payload or notice in payload:
        require(alphabet in payload and notice in payload, "missing FR1 vocabulary or attribution")
        # Limit the nested pinned gzip too; do not allow its compressed length to
        # hide an unrelated expansion bomb. The released vocabulary is < 2 MiB.
        try:
            decoder = zlib.decompressobj(16 + zlib.MAX_WBITS)
            decoded = decoder.decompress(payload[alphabet][0], MAX_MANIFEST_BYTES + 1)
            require(len(decoded) <= MAX_MANIFEST_BYTES and decoder.eof
                    and not decoder.unconsumed_tail and not decoder.unused_data,
                    "invalid or oversized FR1 vocabulary")
        except zlib.error as error:
            raise ReleasePackError("invalid FR1 vocabulary gzip") from error
        require(sha256(decoded) == FR1_ALPHABET_SHA256, "FR1 vocabulary pin mismatch")


def snapshot_source(source):
    """Return selected bytes and canonical modes; excluded trees are never followed."""
    source = checked_local_path(source)
    require(source.is_dir(), "source must be a directory")
    payload, total = {}, 0

    def visit(directory, prefix=""):
        nonlocal total
        with os.scandir(directory) as iterator:
            entries = sorted(iterator, key=lambda item: item.name)
        for entry in entries:
            name = prefix + entry.name
            if exclusion_reason(name):
                continue
            safe_path(name)
            st = entry.stat(follow_symlinks=False)
            if stat.S_ISDIR(st.st_mode):
                require(name.split("/", 1)[0] in SOURCE_ROOTS, "unreviewed source root: " + name)
                visit(Path(entry.path), name + "/")
            else:
                require(stat.S_ISREG(st.st_mode), "non-regular source entry: " + name)
                require(allowed_path(name), "unreviewed source path: " + name)
                require(not st.st_mode & (stat.S_ISUID | stat.S_ISGID | stat.S_ISVTX),
                        "special permission bits in source: " + name)
                require(len(payload) + 2 <= MAX_MEMBERS, "source member count exceeds bound")
                data, mode = _read_regular(Path(entry.path), MAX_UNCOMPRESSED_BYTES - total)
                total += len(data)
                payload[name] = (data, "0755" if mode & 0o111 else "0644")
    visit(source)
    unique_paths(payload)
    _validate_product(payload)
    return payload


def _manifest(payload, pack_version):
    require(isinstance(pack_version, str) and VERSION.fullmatch(pack_version)
            and len(pack_version) <= 64, "invalid pack version")
    rows = [{"path": name, "mode": mode, "size": len(data), "sha256": sha256(data)}
            for name, (data, mode) in sorted(payload.items())]
    identity = {"pack_version": pack_version, "files": rows}
    manifest = {"schema": SCHEMA, **identity, "source_digest": sha256(canonical_json(identity))}
    encoded = canonical_json(manifest)
    require(len(encoded) <= MAX_MANIFEST_BYTES, "digest manifest exceeds size bound")
    require(sum(len(data) for data, _ in payload.values()) + len(encoded) <= MAX_UNCOMPRESSED_BYTES,
            "uncompressed archive exceeds size bound")
    return manifest, encoded


def _encode_archive(payload, manifest_bytes):
    members = {**payload, MANIFEST_NAME: (manifest_bytes, "0644")}
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", compression=zipfile.ZIP_DEFLATED,
                         compresslevel=9, allowZip64=False) as archive:
        for name, (data, mode) in sorted(members.items()):
            info = zipfile.ZipInfo(name, FIXED_TIMESTAMP)
            info.create_system, info.create_version, info.extract_version = 3, 20, 20
            info.external_attr = (stat.S_IFREG | int(mode, 8)) << 16
            info.internal_attr = 0
            archive.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
            require(stream.tell() <= MAX_ARCHIVE_BYTES, "compressed archive exceeds size bound")
    result = stream.getvalue()
    require(len(result) <= MAX_ARCHIVE_BYTES, "archive exceeds size bound")
    return result


def _preflight(data):
    """Check all lengths, names and ZIP structures before any decompression.

    The bounded writer uses classic ZIP only. Rejecting comments, extra fields,
    ZIP64, descriptors, padding and split archives prevents hidden second views.
    """
    require(22 <= len(data) <= MAX_ARCHIVE_BYTES, "archive size outside bounds")
    end = struct.unpack_from("<4s4H2LH", data, len(data) - 22)
    sig, disk, start_disk, disk_count, count, cd_size, cd_start, comment = end
    require(sig == b"PK\x05\x06" and not disk and not start_disk and not comment,
            "invalid end record, split archive, comment or trailing bytes")
    require(0 < count <= MAX_MEMBERS and count == disk_count, "invalid archive member count")
    require(cd_start + cd_size == len(data) - 22, "noncontiguous central directory")
    cursor, local_cursor, total, members = cd_start, 0, 0, []
    for _ in range(count):
        require(cursor + 46 <= len(data) - 22, "truncated central directory")
        fields = struct.unpack_from("<4s6H3L5H2L", data, cursor)
        (sig, made, needed, flags, method, time, date, crc, compressed, size,
         name_len, extra_len, comment_len, member_disk, internal, external, offset) = fields
        require(sig == b"PK\x01\x02" and made == (3 << 8) | 20 and needed == 20,
                "unsupported ZIP version or creator")
        require(flags in (0, 0x800) and method == 8 and time == 0 and date == 33,
                "noncanonical flags, compression or timestamp")
        require(not extra_len and not comment_len and not member_disk and not internal,
                "extra ZIP metadata or split member")
        require(external in ((stat.S_IFREG | 0o644) << 16, (stat.S_IFREG | 0o755) << 16),
                "unsupported file type or mode")
        require(0 < name_len <= 1024 and cursor + 46 + name_len <= len(data) - 22,
                "invalid member name length")
        raw_name = data[cursor + 46:cursor + 46 + name_len]
        try:
            name = raw_name.decode("utf-8" if flags & 0x800 else "ascii")
        except UnicodeError as error:
            raise ReleasePackError("noncanonical filename encoding") from error
        require(bool(flags & 0x800) == (not name.isascii()), "noncanonical UTF-8 flag")
        safe_path(name)
        if name != MANIFEST_NAME:
            require(not exclusion_reason(name) and allowed_path(name), "excluded or unreviewed archive path: " + name)
        total += size
        require(total <= MAX_UNCOMPRESSED_BYTES and compressed <= MAX_ARCHIVE_BYTES,
                "archive expansion exceeds bounds")
        if name == MANIFEST_NAME:
            require(size <= MAX_MANIFEST_BYTES and external == (stat.S_IFREG | 0o644) << 16,
                    "invalid manifest size or mode")
        require(offset == local_cursor and offset + 30 <= cd_start,
                "overlapping, reordered or hidden local records")
        local = struct.unpack_from("<4s5H3L2H", data, offset)
        require(local == (b"PK\x03\x04", needed, flags, method, time, date, crc,
                          compressed, size, name_len, 0), "local/central ZIP header mismatch")
        require(data[offset + 30:offset + 30 + name_len] == raw_name,
                "local/central filename mismatch")
        body = offset + 30 + name_len
        local_cursor = body + compressed
        require(local_cursor <= cd_start, "member overlaps central directory")
        members.append({"path": name, "mode": format(external >> 16 & 0o777, "04o"),
                        "size": size, "compressed": compressed, "crc": crc, "body": body})
        cursor += 46 + name_len
    require(cursor == cd_start + cd_size and local_cursor == cd_start,
            "hidden bytes or inconsistent central directory")
    names = [member["path"] for member in members]
    unique_paths(names)
    require(names == sorted(names), "archive members are not sorted")
    require(MANIFEST_NAME in names, "missing release manifest")
    return members


def _decode_archive(data, pack_version=None):
    members = _preflight(data)
    payload = {}
    for member in members:
        try:
            decoder = zlib.decompressobj(-zlib.MAX_WBITS)
            body = member["body"]
            content = decoder.decompress(data[body:body + member["compressed"]], member["size"] + 1)
            require(len(content) == member["size"] and decoder.eof
                    and not decoder.unconsumed_tail and not decoder.unused_data,
                    "invalid compressed stream or length: " + member["path"])
        except zlib.error as error:
            raise ReleasePackError("corrupt compressed member: " + member["path"]) from error
        require(binascii.crc32(content) & 0xffffffff == member["crc"],
                "CRC mismatch: " + member["path"])
        payload[member["path"]] = content, member["mode"]
    manifest_bytes, _ = payload.pop(MANIFEST_NAME)
    manifest = _json(manifest_bytes, "release manifest")
    require(isinstance(manifest, dict) and set(manifest) == {"schema", "pack_version", "source_digest", "files"},
            "invalid release manifest fields")
    require(manifest["schema"] == SCHEMA, "unsupported manifest schema")
    require(isinstance(manifest["pack_version"], str) and VERSION.fullmatch(manifest["pack_version"])
            and len(manifest["pack_version"]) <= 64, "invalid manifest pack version")
    if pack_version is not None:
        require(manifest["pack_version"] == pack_version, "pack version mismatch")
    rows = manifest["files"]
    require(isinstance(rows, list) and len(rows) == len(payload), "manifest inventory count differs")
    for row in rows:
        require(isinstance(row, dict) and set(row) == {"path", "mode", "size", "sha256"},
                "invalid manifest file record")
        safe_path(row["path"])
        require(type(row["size"]) is int and 0 <= row["size"] <= MAX_UNCOMPRESSED_BYTES,
                "invalid manifest file size")
        require(row["mode"] in ("0644", "0755") and isinstance(row["sha256"], str)
                and HEX_DIGEST.fullmatch(row["sha256"]), "invalid manifest mode or digest")
    expected, encoded = _manifest(payload, manifest["pack_version"])
    require(manifest == expected and manifest_bytes == encoded,
            "source digest, inventory, file digest or canonical manifest differs")
    _validate_product(payload)
    return manifest, manifest_bytes, payload


def _load_archive(archive, source=None, pack_version=None):
    archive = checked_local_path(archive)
    data, _ = _read_regular(archive, MAX_ARCHIVE_BYTES)
    manifest, manifest_bytes, payload = _decode_archive(data, pack_version)
    if source is not None:
        source_payload = snapshot_source(source)
        require(source_payload == payload, "archive differs from independently hashed source bytes or modes")
    return data, manifest, manifest_bytes, payload


def _summary(data, manifest, source):
    return {"status": "passed", "pack_version": manifest["pack_version"],
            "source_digest": manifest["source_digest"], "payload_files": len(manifest["files"]),
            "archive_bytes": len(data), "archive_sha256": sha256(data),
            "source_compared": source is not None}


def validate_archive(archive, source=None, pack_version=None):
    data, manifest, _, _ = _load_archive(archive, source, pack_version)
    return _summary(data, manifest, source)


def create_archive(source, archive, pack_version=PACK_VERSION):
    source, archive = checked_local_path(source), checked_local_path(archive)
    require(not archive.is_relative_to(source), "archive output must be outside source")
    require(archive.parent.is_dir(), "archive parent must already exist")
    require(not archive.exists(), "archive output must not exist")
    payload = snapshot_source(source)
    manifest, manifest_bytes = _manifest(payload, pack_version)
    first = _encode_archive(payload, manifest_bytes)
    # Take a new source snapshot and build into a new ZIP writer. File mtimes,
    # traversal order and output filenames do not contribute to archive identity.
    again = snapshot_source(source)
    require(again == payload, "source changed between archive builds")
    require(_encode_archive(again, _manifest(again, pack_version)[1]) == first,
            "repeated archive bytes differ")
    descriptor, temp_name = tempfile.mkstemp(prefix=".fluxr-release-", suffix=".zip", dir=archive.parent)
    temporary = Path(temp_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(first)
            stream.flush()
            os.fsync(stream.fileno())
        summary = validate_archive(temporary, source=source, pack_version=pack_version)
        checked_local_path(archive)
        os.chmod(temporary, 0o644)
        # The temporary file is on the same filesystem. An exclusive hard link
        # publishes complete bytes atomically and cannot overwrite a late file.
        os.link(temporary, archive)
    finally:
        temporary.unlink(missing_ok=True)
    return {**summary, "repeat_bytes": "passed"}


def _verify_extracted(directory, expected):
    actual, directories = {}, set()
    for root, dirs, files in os.walk(directory, followlinks=False):
        for child in dirs:
            path = Path(root) / child
            require(stat.S_ISDIR(path.lstat().st_mode), "unexpected extracted directory type")
            if os.name == "posix":
                require(stat.S_IMODE(path.lstat().st_mode) == 0o755, "extracted directory mode differs")
            directories.add(path.relative_to(directory).as_posix())
        for child in files:
            path = Path(root) / child
            name = path.relative_to(directory).as_posix()
            require(name in expected, "unexpected extracted file: " + name)
            content, mode = _read_regular(path, MAX_UNCOMPRESSED_BYTES)
            actual[name] = content
            require(content == expected[name][0], "extracted file bytes differ: " + name)
            if os.name == "posix":
                require(stat.S_IMODE(mode) == int(expected[name][1], 8), "extracted file mode differs: " + name)
    expected_dirs = {"/".join(name.split("/")[:end]) for name in expected
                     for end in range(1, len(name.split("/")))}
    require(set(actual) == set(expected) and directories == expected_dirs, "extracted inventory differs")


def extract_archive(archive, destination, source=None, pack_version=None):
    destination = checked_local_path(destination)
    if source is not None:
        require(not destination.is_relative_to(checked_local_path(source)),
                "extraction destination must be outside compared source")
    require(not destination.exists(), "extraction destination must not exist")
    require(destination.parent.is_dir(), "extraction parent must already exist")
    data, manifest, manifest_bytes, payload = _load_archive(archive, source, pack_version)
    expected = {**payload, MANIFEST_NAME: (manifest_bytes, "0644")}
    staging = Path(tempfile.mkdtemp(prefix=".fluxr-extract-", dir=destination.parent))
    try:
        for name, (content, mode) in sorted(expected.items()):
            path = staging / name
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as stream:
                stream.write(content)
            os.chmod(path, int(mode, 8))
        for root, dirs, _ in os.walk(staging):
            for child in dirs:
                os.chmod(Path(root) / child, 0o755)
        _verify_extracted(staging, expected)
        checked_local_path(destination)
        require(not destination.exists(), "extraction destination appeared during validation")
        os.chmod(staging, 0o755)
        os.rename(staging, destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)
    return {**_summary(data, manifest, source), "exact_extraction": "passed",
            "filesystem_modes": "verified" if os.name == "posix" else "ZIP metadata only"}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("create", help="build twice, compare bytes, validate and atomically save")
    create.add_argument("source", type=Path)
    create.add_argument("archive", type=Path)
    create.add_argument("--pack-version", default=PACK_VERSION)
    for command in ("validate", "extract"):
        operation = commands.add_parser(command)
        operation.add_argument("archive", type=Path)
        if command == "extract":
            operation.add_argument("destination", type=Path)
        operation.add_argument("--source", type=Path, help="independently compare selected source bytes and modes")
        operation.add_argument("--pack-version", help="require this exact implementation-pack version")
    args = parser.parse_args(argv)
    try:
        if args.command == "create":
            result = create_archive(args.source, args.archive, args.pack_version)
        elif args.command == "validate":
            result = validate_archive(args.archive, args.source, args.pack_version)
        else:
            result = extract_archive(args.archive, args.destination, args.source, args.pack_version)
    except (OSError, ValueError, KeyError, TypeError, struct.error, zipfile.BadZipFile,
            zipfile.LargeZipFile, RecursionError) as error:
        parser.exit(1, f"release pack invalid: {error}\n")
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
