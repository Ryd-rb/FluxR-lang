"""Bounded immutable Git source inputs and fresh, verified Linux materialization.

This is source preparation, not execution isolation or a complete hermetic build.
Repositories, Git, libc and CAS/destination parent directories are trusted. No
candidate executable, filter, hook, submodule, or network fetch is invoked. The
whole committed tree remains an action input; no finer cache/selection grant is
issued. Source SHA-256 identity is independent of commit metadata and Git OIDs.
"""
from __future__ import annotations

from dataclasses import dataclass
import ctypes
import errno
import hashlib
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

from .identity import (MAX_JSON_BYTES, action_key, canonical_bytes, load_json_bytes,
                       relative_path, require_digest)
from .store import ContentStore, MAX_BLOB_BYTES

SCHEMA = "fluxr-source-snapshot-v1"
PROFILE = "git-regular-linux-v1"
OID = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")


@dataclass(frozen=True)
class Limits:
    entries: int = 10000
    file_bytes: int = MAX_BLOB_BYTES
    total_bytes: int = 128 * 1024 * 1024
    manifest_bytes: int = MAX_JSON_BYTES
    path_bytes: int = 4096
    path_depth: int = 128
    seconds: int = 120

    def __post_init__(self):
        for name, ceiling in (("entries", 100000), ("file_bytes", MAX_BLOB_BYTES),
                              ("total_bytes", 2**31), ("manifest_bytes", MAX_JSON_BYTES),
                              ("path_bytes", 4096), ("path_depth", 128), ("seconds", 3600)):
            value = getattr(self, name)
            if type(value) is not int or not 1 <= value <= ceiling:
                raise ValueError(f"invalid snapshot limit {name}")


class Cancelled(ValueError):
    """No complete snapshot/materialization result was accepted."""


class _Budget:
    def __init__(self, limits, cancelled):
        self.limits = limits
        self.cancelled = cancelled
        self.deadline = time.monotonic() + limits.seconds

    def check(self):
        if self.cancelled is not None and self.cancelled():
            raise Cancelled("source preparation cancelled")
        if time.monotonic() >= self.deadline:
            raise TimeoutError("source preparation deadline exceeded")


def _path(value, limits):
    relative_path(value)
    if (len(value.encode("utf-8")) > limits.path_bytes or
            len(value.split("/")) > limits.path_depth or
            any(part.casefold() == ".git" for part in value.split("/"))):
        raise ValueError("snapshot path is too large/deep or contains Git administration")
    return value


def _oid(value):
    if type(value) is not str or OID.fullmatch(value) is None:
        raise ValueError("an exact full lowercase Git object identity is required")
    return value


class _Git:
    """Small bounded streaming reader; never subprocess.communicate a large blob."""
    def __init__(self, repository, arguments, budget, *, batch=False):
        budget.check()
        executable = shutil.which("git")
        if executable is None:
            raise ValueError("Git is unavailable")
        environment = {"PATH": str(Path(executable).parent), "LC_ALL": "C",
                       "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                       "GIT_TERMINAL_PROMPT": "0", "GIT_OPTIONAL_LOCKS": "0",
                       "GIT_NO_LAZY_FETCH": "1", "GIT_NO_REPLACE_OBJECTS": "1"}
        self.process = subprocess.Popen(
            [executable, "--no-replace-objects", "-c", "core.fsmonitor=false",
             "-c", "protocol.allow=never", "-C", str(repository), *arguments],
            stdin=subprocess.PIPE if batch else subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env=environment, start_new_session=True)
        self.budget, self.buffer = budget, bytearray()
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.selector.close()
        # Kill even on successful reading: a child/descendant must not outlive
        # the bounded operation. Git plumbing normally exits at stdin EOF.
        if self.process.stdin is not None:
            try:
                self.process.stdin.close()
            except BrokenPipeError:
                pass
        try:
            os.killpg(self.process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.process.wait(timeout=5)
        self.process.stdout.close()

    def _chunk(self, maximum):
        while True:
            self.budget.check()
            if self.selector.select(min(0.05, max(0, self.budget.deadline - time.monotonic()))):
                return os.read(self.process.stdout.fileno(), min(65536, maximum))

    def all(self, maximum):
        result = bytearray()
        while True:
            chunk = self._chunk(maximum + 1 - len(result))
            if not chunk:
                break
            result.extend(chunk)
            if len(result) > maximum:
                raise ValueError("Git metadata exceeds snapshot byte limit")
        if self.process.wait(timeout=5) != 0:
            raise ValueError("Git object command failed; unavailable/corrupt object")
        return bytes(result)

    def exact(self, size):
        result = bytearray()
        while len(result) < size:
            if self.buffer:
                take = min(len(self.buffer), size - len(result))
                result.extend(self.buffer[:take])
                del self.buffer[:take]
            else:
                chunk = self._chunk(size - len(result))
                if not chunk:
                    raise ValueError("truncated Git object stream")
                result.extend(chunk)
        return bytes(result)

    def header(self):
        while b"\n" not in self.buffer:
            if len(self.buffer) >= 256:
                raise ValueError("oversized Git object header")
            chunk = self._chunk(256 - len(self.buffer))
            if not chunk:
                raise ValueError("missing Git object header")
            self.buffer.extend(chunk)
        line, _, rest = self.buffer.partition(b"\n")
        self.buffer = bytearray(rest)
        return line

    def blob(self, oid, size):
        self.budget.check()
        self.process.stdin.write(oid.encode("ascii") + b"\n")
        self.process.stdin.flush()
        if self.header() != f"{oid} blob {size}".encode("ascii"):
            raise ValueError("Git blob identity/type/size changed")
        hasher = hashlib.sha1() if len(oid) == 40 else hashlib.sha256()
        hasher.update(f"blob {size}\0".encode("ascii"))
        remaining = size
        while remaining:
            self.budget.check()
            chunk = self.exact(min(65536, remaining))
            hasher.update(chunk)
            remaining -= len(chunk)
            yield chunk
        if self.exact(1) != b"\n" or hasher.hexdigest() != oid:
            raise ValueError("Git blob content differs from its object identity")


def _git(repository, arguments, budget, maximum=1024):
    with _Git(repository, arguments, budget) as reader:
        return reader.all(maximum)


def _raw_object(repository, kind, oid, budget, maximum):
    """Bind even a corrupted loose object's actual bytes to its advertised OID."""
    raw = _git(repository, ["cat-file", kind, oid], budget, maximum=maximum)
    hasher = hashlib.sha1() if len(oid) == 40 else hashlib.sha256()
    hasher.update(f"{kind} {len(raw)}\0".encode("ascii"))
    hasher.update(raw)
    if hasher.hexdigest() != oid:
        raise ValueError(f"Git {kind} content differs from its object identity")
    return raw


def _manifest(value, limits):
    raw = canonical_bytes(value)
    if len(raw) > limits.manifest_bytes:
        raise ValueError("snapshot manifest byte quota exceeded")
    value = load_json_bytes(raw)
    if (type(value) is not dict or set(value) != {"schema", "profile", "entries"}
            or value["schema"] != SCHEMA or value["profile"] != PROFILE):
        raise ValueError("unsupported source snapshot fields/profile")
    entries = value["entries"]
    if type(entries) is not list or len(entries) > limits.entries:
        raise ValueError("source snapshot entry quota exceeded")
    paths, total = {}, 0
    for entry in entries:
        if type(entry) is not dict or set(entry) != {"path", "mode", "size", "digest"}:
            raise ValueError("invalid source snapshot entry")
        path = _path(entry["path"], limits)
        if path in paths:
            raise ValueError("duplicate snapshot path")
        mode, size = entry["mode"], entry["size"]
        if type(size) is not int or not 0 <= size <= limits.file_bytes:
            raise ValueError("source file byte quota exceeded")
        if mode == "040000":
            if size != 0 or entry["digest"] is not None:
                raise ValueError("directories carry no payload")
        elif mode in ("100644", "100755"):
            require_digest(entry["digest"])
            total += size
        else:
            raise ValueError("symlinks/gitlinks/special entries are unsupported")
        if total > limits.total_bytes:
            raise ValueError("aggregate source byte quota exceeded")
        paths[path] = entry
    if list(paths) != sorted(paths):
        raise ValueError("snapshot paths must be uniquely sorted")
    for path in paths:
        parent = path.rpartition("/")[0]
        if parent and (parent not in paths or paths[parent]["mode"] != "040000"):
            raise ValueError("missing directory or file/directory path collision")
    return value, total


def capture_git(repository, commit, store: ContentStore, *, limits=Limits(), cancelled=None):
    """Read only a pinned commit's Git objects and publish its complete manifest last.

    Dirty/untracked worktree files are deliberately absent. A caller must commit
    a candidate first and pass its exact OID; refs, abbreviations and revision
    expressions refuse. Partial failures may leave unreferenced CAS data, never
    an accepted capture result. No source checkout is modified.
    """
    if not sys.platform.startswith("linux"):
        raise ValueError("source snapshot v1 is qualified only for Linux")
    budget = _Budget(limits, cancelled)
    commit = _oid(commit)
    actual = _git(repository, ["rev-parse", "--verify", commit + "^{commit}"], budget).decode().strip()
    if actual != commit:
        raise ValueError("requested identity is not the exact commit")
    raw_commit = _raw_object(repository, "commit", commit, budget, limits.manifest_bytes)
    header = raw_commit.split(b"\n", 1)[0]
    if not header.startswith(b"tree "):
        raise ValueError("Git commit lacks a canonical tree header")
    tree = _oid(header[5:].decode("ascii"))
    if len(tree) != len(commit):
        raise ValueError("Git commit/tree object formats differ")
    raw_tree = _raw_object(repository, "tree", tree, budget, limits.manifest_bytes)
    git_metadata_bytes = len(raw_commit) + len(raw_tree)
    verified_trees = {tree}
    listing = _git(repository, ["ls-tree", "--full-tree", "-r", "-t", "-l", "-z", tree], budget,
                   maximum=limits.manifest_bytes * 2)
    if listing and not listing.endswith(b"\0"):
        raise ValueError("truncated Git tree inventory")
    records, entries, total = [], [], 0
    # Charge metadata before any blob I/O. The manifest has fixed schema/profile
    # overhead, and each entry's exact escaped encoding consumes this budget.
    metadata = len(canonical_bytes({"schema": SCHEMA, "profile": PROFILE, "entries": []}))
    for record in listing.split(b"\0")[:-1]:
        budget.check()
        if len(records) >= limits.entries:
            raise ValueError("source snapshot entry quota exceeded")
        fields, name = record.split(b"\t", 1)
        mode, kind, oid, size = fields.decode("ascii").split()
        path = _path(name.decode("utf-8", errors="strict"), limits)
        _oid(oid)
        if len(oid) != len(commit):
            raise ValueError("Git inventory object formats differ")
        if mode == "040000" and kind == "tree" and size == "-":
            count = 0
        elif mode in ("100644", "100755") and kind == "blob":
            count = int(size)
            if not 0 <= count <= limits.file_bytes:
                raise ValueError("source file byte quota exceeded")
            total += count
        else:
            raise ValueError("symlinks/gitlinks/special entries are unsupported")
        if total > limits.total_bytes:
            raise ValueError("aggregate source byte quota exceeded")
        entry = {"path": path, "mode": mode, "size": count,
                 "digest": None if mode == "040000" else "0" * 64}
        metadata += len(canonical_bytes(entry)) + (1 if entries else 0)
        if metadata > limits.manifest_bytes:
            raise ValueError("snapshot manifest byte quota exceeded")
        records.append((oid, entry))
        entries.append(entry)
    # ls-tree is a useful mature inventory reader, but a path-shaped OID is
    # not proof that loose commit/tree objects still contain their named bytes.
    # Check every traversed raw tree before accepting its provenance.
    for oid, entry in records:
        budget.check()
        if entry["mode"] == "040000" and oid not in verified_trees:
            remaining = limits.manifest_bytes * 2 - git_metadata_bytes
            if remaining < 0:
                raise ValueError("Git metadata aggregate byte quota exceeded")
            raw = _raw_object(repository, "tree", oid, budget, remaining)
            git_metadata_bytes += len(raw)
            verified_trees.add(oid)
    if git_metadata_bytes > limits.manifest_bytes * 2:
        raise ValueError("Git metadata aggregate byte quota exceeded")
    entries.sort(key=lambda entry: entry["path"])
    _manifest({"schema": SCHEMA, "profile": PROFILE, "entries": entries}, limits)
    seen = {}
    with _Git(repository, ["cat-file", "--batch"], budget, batch=True) as reader:
        for oid, entry in records:
            budget.check()
            if entry["mode"] == "040000":
                continue
            if oid not in seen:
                seen[oid] = (entry["size"], store.put_stream(reader.blob(oid, entry["size"])))
            if seen[oid][0] != entry["size"]:
                raise ValueError("one Git object has inconsistent sizes")
            entry["digest"] = seen[oid][1]
    budget.check()
    value, total = _manifest({"schema": SCHEMA, "profile": PROFILE, "entries": entries}, limits)
    digest = store.put(canonical_bytes(value))
    budget.check()
    return {"schema": "fluxr-source-capture-v1", "snapshot": digest,
            "git_commit": commit, "git_tree": tree, "entries": len(entries),
            "files": sum(e["mode"] != "040000" for e in entries),
            "source_bytes": total, "manifest_bytes": len(canonical_bytes(value)),
            "verified_git_trees": len(verified_trees), "git_metadata_bytes": git_metadata_bytes,
            "scope": "Committed regular-file source inputs only; not a hermetic execution or authorization."}


def _load(store, digest, limits):
    require_digest(digest)
    raw = store.get(digest, maximum_bytes=limits.manifest_bytes)
    if len(raw) > limits.manifest_bytes:
        raise ValueError("snapshot manifest byte quota exceeded")
    value, total = _manifest(load_json_bytes(raw), limits)
    if raw != canonical_bytes(value):
        raise ValueError("snapshot manifest is not canonical")
    return value, total


def verify_snapshot(store, digest, *, limits=Limits(), cancelled=None):
    """Check the manifest and every referenced payload now, not just its hash label."""
    budget = _Budget(limits, cancelled)
    budget.check()
    value, total = _load(store, digest, limits)
    seen = {}
    for entry in value["entries"]:
        budget.check()
        if entry["digest"] is not None:
            if entry["digest"] not in seen:
                seen[entry["digest"]] = len(store.get(entry["digest"], maximum_bytes=entry["size"]))
            if seen[entry["digest"]] != entry["size"]:
                raise ValueError("source payload size differs from manifest")
    budget.check()
    return {"snapshot": digest, "entries": len(value["entries"]), "source_bytes": total}


def bind_action(store, digest, specification, *, limits=Limits(), cancelled=None):
    """Validate a legacy v1 action against actual full-source CAS inputs.

    A full source pin is mandatory even for an empty declared-input list. Only
    committed regular files can satisfy declared inputs; generated inputs must
    first become a separately captured committed snapshot. Toolchain, runner,
    environment and external inputs still need independently enforced closure.
    """
    specification = load_json_bytes(canonical_bytes(specification))
    key = action_key(specification)
    if specification["source_snapshot"] != digest:
        raise ValueError("stale or mismatched action source snapshot")
    verify_snapshot(store, digest, limits=limits, cancelled=cancelled)
    value, _ = _load(store, digest, limits)
    entries = {entry["path"]: entry for entry in value["entries"]}
    cwd = specification["working_directory"]
    if cwd != "." and (cwd not in entries or entries[cwd]["mode"] != "040000"):
        raise ValueError("action working directory is not a captured directory")
    for path, expected in specification["inputs"].items():
        if path not in entries or entries[path]["digest"] != expected:
            raise ValueError("declared action input differs from captured source")
    for output in specification["outputs"]:
        # Every ancestor directory is explicit in the validated inventory.
        if output in entries:
            raise ValueError("action output overlaps immutable source")
        ancestor = output.rpartition("/")[0]
        while ancestor:
            if ancestor in entries and entries[ancestor]["mode"] != "040000":
                raise ValueError("action output descends from a source file")
            ancestor = ancestor.rpartition("/")[0]
    return {"schema": "fluxr-source-bound-action-v1", "action_key": key,
            "source_snapshot": digest, "action": specification,
            "hermetic_execution": False, "cache_acceptance_authorized": False}


def _rename_no_replace(parent_fd, source, destination):
    library = ctypes.CDLL(None, use_errno=True)
    function = getattr(library, "renameat2", None)
    if function is None:
        raise ValueError("atomic no-replace directory rename is unavailable")
    function.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    function.restype = ctypes.c_int
    if function(parent_fd, os.fsencode(source), parent_fd, os.fsencode(destination), 1) != 0:
        code = ctypes.get_errno()
        raise OSError(code, os.strerror(code), destination)


def _cleanup(directory):
    if directory.is_symlink():
        directory.unlink()
        return
    if not directory.exists():
        return
    for root, dirs, _ in os.walk(directory, followlinks=False):
        os.chmod(root, 0o700, follow_symlinks=False)
        for name in dirs:
            path = Path(root) / name
            if not path.is_symlink():
                os.chmod(path, 0o700, follow_symlinks=False)
    shutil.rmtree(directory)


def materialize(store, digest, destination, *, limits=Limits(), cancelled=None):
    """Create a new read-only source tree, verified before atomic no-replace rename.

    No CAS hard links are used. Existing files/directories/symlinks refuse. A
    crash may leave a private staging directory, never a partially populated
    final path. Required parent fsync failure after rename is an unacknowledged
    complete copy; callers must verify it rather than assume success.
    """
    if not sys.platform.startswith("linux") or not hasattr(ctypes.CDLL(None), "renameat2"):
        raise ValueError("source snapshot materialization requires Linux renameat2")
    budget = _Budget(limits, cancelled)
    budget.check()
    value, _ = _load(store, digest, limits)
    destination = Path(destination).absolute()
    parent = destination.parent
    if destination.name in ("", ".", "..") or os.path.lexists(destination):
        raise FileExistsError(errno.EEXIST, "destination must be new", str(destination))
    # Parent directory is trusted, exists, and is not a symlink. No candidate
    # path may select another parent; every internal path has been validated.
    parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    staging = None
    try:
        staging = Path(tempfile.mkdtemp(prefix=".fluxr-source-", dir=parent))
        for entry in value["entries"]:
            budget.check()
            path = staging / entry["path"]
            if entry["mode"] == "040000":
                path.mkdir(mode=0o700)
                continue
            data = store.get(entry["digest"], maximum_bytes=entry["size"])
            if len(data) != entry["size"]:
                raise ValueError("source payload size differs from manifest")
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(data)
                os.fchmod(stream.fileno(), 0o555 if entry["mode"] == "100755" else 0o444)
                stream.flush()
                os.utime(stream.fileno(), (0, 0))
                os.fsync(stream.fileno())
        for entry in reversed(value["entries"]):
            if entry["mode"] == "040000":
                directory = staging / entry["path"]
                os.chmod(directory, 0o555)
                os.utime(directory, (0, 0))
                fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                try:
                    os.fsync(fd)
                finally:
                    os.close(fd)
        os.chmod(staging, 0o555)
        os.utime(staging, (0, 0))
        fd = os.open(staging, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
        verify_materialization(store, digest, staging, limits=limits, cancelled=cancelled, _budget=budget)
        budget.check()
        _rename_no_replace(parent_fd, staging.name, destination.name)
        staging = None
        os.fsync(parent_fd)
        return {"schema": "fluxr-source-materialization-v1", "snapshot": digest,
                "destination": str(destination), "verified": True,
                "source_read_only": True, "execution_isolated": False}
    finally:
        if staging is not None:
            _cleanup(staging)
        os.close(parent_fd)


def verify_materialization(store, digest, directory, *, limits=Limits(), cancelled=None, _budget=None):
    """Rehash a copied tree and reject every missing/extra/mode-changed entry.

    Directory-relative, no-follow opens prevent a replaced child symlink from
    redirecting reads outside the opened root. A trusted controller still owns
    tree lifetime; no snapshot lease or protection against administrator writes
    is created by this point-in-time check.
    """
    budget = _budget or _Budget(limits, cancelled)
    budget.check()
    value, total = _load(store, digest, limits)
    expected = {entry["path"]: entry for entry in value["entries"]}
    root_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    seen, pending = set(), [(root_fd, "")]
    try:
        if (stat.S_IMODE(os.fstat(root_fd).st_mode) != 0o555
                or os.fstat(root_fd).st_mtime_ns != 0):
            raise ValueError("source root permissions or normalized modification time changed")
        while pending:
            budget.check()
            directory_fd, prefix = pending.pop()
            try:
                with os.scandir(directory_fd) as listing:
                    for item in listing:
                        budget.check()
                        relative = prefix + item.name
                        if relative not in expected or relative in seen:
                            raise ValueError("extra or duplicate materialized source entry")
                        seen.add(relative)
                        entry = expected[relative]
                        metadata = os.stat(item.name, dir_fd=directory_fd, follow_symlinks=False)
                        expected_mode = 0o444 if entry["mode"] == "100644" else 0o555
                        if stat.S_IMODE(metadata.st_mode) != expected_mode or metadata.st_mtime_ns != 0:
                            raise ValueError("materialized source permissions or modification time changed")
                        if entry["mode"] == "040000":
                            if not stat.S_ISDIR(metadata.st_mode):
                                raise ValueError("materialized directory type changed")
                            child = os.open(item.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                            dir_fd=directory_fd)
                            if (os.fstat(child).st_ino, os.fstat(child).st_dev) != (metadata.st_ino, metadata.st_dev):
                                os.close(child)
                                raise ValueError("materialized directory changed during open")
                            pending.append((child, relative + "/"))
                            continue
                        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1
                                or metadata.st_size != entry["size"]):
                            raise ValueError("materialized file type/link/size changed")
                        fd = os.open(item.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                                     dir_fd=directory_fd)
                        hasher, count = hashlib.sha256(), 0
                        with os.fdopen(fd, "rb") as stream:
                            opened = os.fstat(stream.fileno())
                            if (opened.st_ino, opened.st_dev, opened.st_mode, opened.st_size) != (
                                    metadata.st_ino, metadata.st_dev, metadata.st_mode, metadata.st_size):
                                raise ValueError("materialized file changed during open")
                            while True:
                                budget.check()
                                chunk = stream.read(min(65536, entry["size"] + 1 - count))
                                if not chunk:
                                    break
                                count += len(chunk)
                                if count > entry["size"]:
                                    raise ValueError("materialized file grew during verification")
                                hasher.update(chunk)
                        if count != entry["size"] or hasher.hexdigest() != entry["digest"]:
                            raise ValueError("materialized source content changed")
            finally:
                os.close(directory_fd)
        if seen != set(expected):
            raise ValueError("missing materialized source entries")
        budget.check()
        return {"snapshot": digest, "entries": len(seen), "source_bytes": total, "verified": True}
    finally:
        for directory_fd, _ in pending:
            os.close(directory_fd)
