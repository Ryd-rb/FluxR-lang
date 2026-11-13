"""Atomic, verified local content store for data and evidence, not executables.

The directory is trusted and must not be shared with untrusted users. A digest
proves byte identity only. This is not a network cache or a signature system.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import stat
import sys
import tempfile

from .identity import content_digest, require_digest

MAX_BLOB_BYTES = 16 * 1024 * 1024
STREAM_CHUNK_BYTES = 64 * 1024
MAX_STREAM_CHUNKS = 65536


class ContentStore:
    def __init__(self, root):
        self.root = Path(root)
        if self.root.is_symlink():
            raise ValueError("store root must not be a symlink")
        self.root.mkdir(parents=True, exist_ok=True)
        self.root = self.root.resolve(strict=True)

    def _sync_directory(self):
        if os.name == "posix":
            directory = os.open(self.root, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(directory)
            finally:
                os.close(directory)

    def _publish(self, temporary, digest):
        target = self.root / digest
        try:
            os.link(temporary, target, follow_symlinks=False)
        except FileExistsError:
            pass
        # Verify new and pre-existing objects without materializing a second
        # blob. The staged bytes supply an exact comparator as well as a hash.
        with temporary.open("rb") as expected:
            self._read_verified(digest, expected=expected)
        # The losing writer must synchronize the winning directory entry too:
        # the winner may not yet have acknowledged its new link durably.
        self._sync_directory()
        return digest

    def put(self, data: bytes) -> str:
        if not isinstance(data, bytes) or len(data) > MAX_BLOB_BYTES:
            raise ValueError("blob must be bytes within the 16 MiB limit")
        digest = content_digest(data)
        # Duplicate evidence is common across agents. Reverify the bytes and
        # synchronize the existing object instead of writing another full copy.
        if sys.platform.startswith("linux"):
            try:
                self._read_verified(digest, expected=data, synchronize=True)
            except FileNotFoundError:
                pass
            else:
                self._sync_directory()
                return digest
        return self.put_stream((data,))

    def put_stream(self, chunks) -> str:
        """Stage bounded byte chunks, then atomically publish verified content.

        Total content is limited to 16 MiB and chunk count to 65,536 (including
        empty chunks), so a broken producer cannot loop indefinitely on empties.
        Producer failures leave no published object and clean the temporary.
        The caller owns any allocations made before a chunk reaches this method.
        """
        descriptor, name = tempfile.mkstemp(prefix=".pending-", dir=self.root)
        temporary = Path(name)
        digest, total = hashlib.sha256(), 0
        try:
            with os.fdopen(descriptor, "wb") as stream:
                for count, chunk in enumerate(chunks, 1):
                    if count > MAX_STREAM_CHUNKS:
                        raise ValueError("too many content stream chunks")
                    if not isinstance(chunk, bytes) or len(chunk) > MAX_BLOB_BYTES - total:
                        raise ValueError("blob chunks must be bytes within the 16 MiB total limit")
                    stream.write(chunk)
                    digest.update(chunk)
                    total += len(chunk)
                stream.flush()
                os.fsync(stream.fileno())
            return self._publish(temporary, digest.hexdigest())
        finally:
            temporary.unlink(missing_ok=True)

    def _read_verified(self, digest, *, retain=False, expected=None, synchronize=False,
                       maximum_bytes=MAX_BLOB_BYTES):
        if type(maximum_bytes) is not int or not 0 <= maximum_bytes <= MAX_BLOB_BYTES:
            raise ValueError("invalid content read byte limit")
        require_digest(digest)
        path = self.root / digest
        if path.is_symlink():
            raise ValueError("content object must not be a symlink")
        descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0))
        hasher, size, pieces = hashlib.sha256(), 0, []
        with os.fdopen(descriptor, "rb") as stream:
            metadata = os.fstat(stream.fileno())
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > maximum_bytes:
                raise ValueError("content object has invalid type or size")
            while True:
                chunk = stream.read(min(STREAM_CHUNK_BYTES, maximum_bytes + 1 - size))
                if not chunk:
                    break
                if expected is not None:
                    reference = (expected[size:size + len(chunk)] if isinstance(expected, bytes)
                                 else expected.read(len(chunk)))
                    if reference != chunk:
                        raise ValueError("content collision")
                hasher.update(chunk)
                size += len(chunk)
                if size > maximum_bytes:
                    raise ValueError("content object failed digest or size validation")
                if retain:
                    pieces.append(chunk)
            if (size != metadata.st_size or hasher.hexdigest() != digest
                    or (isinstance(expected, bytes) and size != len(expected))
                    or (expected is not None and not isinstance(expected, bytes) and expected.read(1))):
                raise ValueError("content object failed digest or size validation")
            if synchronize:
                os.fsync(stream.fileno())
        return b"".join(pieces) if retain else True

    def get(self, digest: str, *, maximum_bytes=MAX_BLOB_BYTES) -> bytes:
        """Read verified bytes under an optional stricter pre-read allocation cap.

        The default preserves the released 16 MiB ceiling. The stored file's
        advertised size is checked before reading, and the bounded read loop
        rejects a concurrently grown object without retaining excess chunks.
        """
        return self._read_verified(digest, retain=True, maximum_bytes=maximum_bytes)

    def verify(self, digest: str) -> bool:
        """Rehash retained bytes using bounded working memory; errors fail closed.

        This is directly usable as assess_evidence's verify_artifact adapter. It
        establishes byte availability/integrity only; producer authentication,
        retention leases and publication authority remain separate obligations.
        """
        return self._read_verified(digest)
