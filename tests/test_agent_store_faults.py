"""Fault tests for atomic content-store writes and durable acknowledgements."""
import os
from pathlib import Path
import sys
import tempfile
import tracemalloc
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline import ContentStore
from agent_pipeline.identity import content_digest
from agent_pipeline.store import MAX_BLOB_BYTES, STREAM_CHUNK_BYTES


class StoreFaultTests(unittest.TestCase):
    def test_streamed_bytes_have_same_identity_and_preserve_existing_content(self):
        with tempfile.TemporaryDirectory() as directory:
            store = ContentStore(directory)
            expected = b"prefix\0" + bytes(range(256)) * 1024 + b"suffix"
            chunks = (expected[index:index + 17] for index in range(0, len(expected), 17))
            digest = store.put_stream(chunks)
            self.assertEqual(content_digest(expected), digest)
            self.assertEqual(expected, store.get(digest))
            self.assertTrue(store.verify(digest))
            self.assertEqual(digest, store.put_stream([expected]))
            self.assertEqual([digest], [path.name for path in Path(directory).iterdir()])
            (Path(directory) / digest).write_bytes(b"corrupt")
            with self.assertRaises(ValueError):
                store.verify(digest)
            with self.assertRaises(ValueError):
                store.put_stream([expected])
            self.assertEqual(b"corrupt", (Path(directory) / digest).read_bytes())

    def test_stream_failure_size_type_and_empty_chunk_budgets_clean_pending(self):
        def interrupted():
            yield b"partial"
            raise OSError("producer lost")
        with tempfile.TemporaryDirectory() as directory:
            store = ContentStore(directory)
            with self.assertRaises(OSError):
                store.put_stream(interrupted())
            for chunks in ([b"x", "not bytes"], [b"x" * MAX_BLOB_BYTES, b"y"]):
                with self.assertRaises(ValueError):
                    store.put_stream(chunks)
            with patch("agent_pipeline.store.MAX_STREAM_CHUNKS", 4):
                with self.assertRaisesRegex(ValueError, "chunks"):
                    store.put_stream([b""] * 5)
            self.assertEqual([], list(Path(directory).iterdir()))
            self.assertEqual(b"", store.get(store.put_stream([])))

    def test_verify_large_blob_uses_bounded_working_memory(self):
        with tempfile.TemporaryDirectory() as directory:
            store = ContentStore(directory)
            chunk = b"x" * STREAM_CHUNK_BYTES
            digest = store.put_stream(chunk for _ in range(MAX_BLOB_BYTES // len(chunk)))
            tracemalloc.start()
            try:
                self.assertTrue(store.verify(digest))
                _, peak = tracemalloc.get_traced_memory()
            finally:
                tracemalloc.stop()
            # Enough slack for allocator/platform details, far less than the
            # 16 MiB object. This detects accidental get()/join materialization.
            self.assertLess(peak, 1024 * 1024)

    @unittest.skipUnless(sys.platform.startswith("linux"), "duplicate read-fsync fast path is Linux-specific")
    def test_duplicate_put_verifies_without_staging_a_second_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            store = ContentStore(directory)
            digest = store.put(b"same durable bytes")
            with patch("agent_pipeline.store.tempfile.mkstemp", side_effect=AssertionError("duplicate staged")):
                self.assertEqual(digest, store.put(b"same durable bytes"))
            (Path(directory) / digest).write_bytes(b"corrupt retained bytes")
            with self.assertRaises(ValueError):
                store.put(b"same durable bytes")

    def test_parallel_streaming_writers_publish_one_verified_object(self):
        from concurrent.futures import ThreadPoolExecutor
        with tempfile.TemporaryDirectory() as directory:
            store = ContentStore(directory)
            def publish(_):
                return store.put_stream([b"first", b"second"])
            with ThreadPoolExecutor(max_workers=8) as pool:
                digests = list(pool.map(publish, range(32)))
            self.assertEqual(1, len(set(digests)))
            self.assertEqual(b"firstsecond", store.get(digests[0]))
            self.assertEqual([digests[0]], [path.name for path in Path(directory).iterdir()])

    def test_failed_fsync_cleans_pending_file(self):
        with tempfile.TemporaryDirectory() as directory:
            store = ContentStore(directory)
            with patch("agent_pipeline.store.os.fsync", side_effect=OSError("injected disk error")):
                with self.assertRaises(OSError):
                    store.put(b"data")
            self.assertEqual([], list(Path(directory).iterdir()))
            self.assertEqual(b"data", store.get(store.put(b"data")))

    @unittest.skipUnless(os.name == "posix", "directory fsync is POSIX-specific")
    def test_existing_object_put_also_fsyncs_directory(self):
        import stat
        with tempfile.TemporaryDirectory() as directory:
            store = ContentStore(directory)
            store.put(b"data")
            synced = []
            real_fsync = os.fsync
            def record(descriptor):
                synced.append(stat.S_ISDIR(os.fstat(descriptor).st_mode))
                real_fsync(descriptor)
            with patch("agent_pipeline.store.os.fsync", side_effect=record):
                store.put(b"data")
            self.assertIn(True, synced)
            self.assertIn(False, synced)


if __name__ == "__main__":
    unittest.main()
