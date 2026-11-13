"""Catalog integrity for measured source pairs, independent of token counts."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

TOOLS = Path(__file__).resolve().parents[1] / "tools"
sys.path.insert(0, str(TOOLS))
import bench_cookbook_tokens as counter


class CookbookTokenCatalogTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.catalog = self.root / "catalog.json"
        self.case = {"name": "example", "topics": ["i64"],
                     "rust": "example.rs", "fluxr": "example.fluxr", "compact": "example.compact"}
        for key in counter.ARMS:
            (self.root / self.case[key]).write_bytes((key + "\n").encode("utf-8"))
        self.save()

    def save(self, cases=None, format="fluxr-cookbook-v1"):
        self.catalog.write_text(json.dumps({"format": format, "cases": cases or [self.case]}), encoding="utf-8")

    def load(self):
        return counter.load_catalog(self.catalog, self.root)

    def test_exact_whole_sources_and_terminal_newlines_are_retained(self):
        _, rows = self.load()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["texts"], {arm: arm + "\n" for arm in counter.ARMS})

    def test_duplicate_json_keys_rejected_before_catalog_decoding(self):
        self.catalog.write_text('{"format":"fluxr-cookbook-v1","format":"fluxr-cookbook-v1","cases":[]}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate JSON"):
            self.load()

    def test_duplicate_names_and_source_paths_rejected(self):
        self.save([self.case, self.case])
        with self.assertRaisesRegex(ValueError, "duplicate case name"):
            self.load()
        self.save([self.case, {**self.case, "name": "second"}])
        with self.assertRaisesRegex(ValueError, "duplicate case source"):
            self.load()

    def test_traversal_and_absolute_paths_rejected(self):
        for path in ["../outside.rs", "/outside.rs", "sub/../example.rs", "sub\\example.rs"]:
            with self.subTest(path=path):
                self.save([{**self.case, "rust": path}])
                with self.assertRaises(ValueError):
                    self.load()

    def test_symlink_source_rejected(self):
        source = self.root / "example.rs"
        source.unlink()
        try:
            source.symlink_to(self.root / "example.fluxr")
        except OSError as error:
            if getattr(error, "winerror", None) != 1314:
                raise
            self.skipTest("Windows symlink privilege unavailable")
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.load()

    def test_unknown_format_empty_source_and_invalid_topics_rejected(self):
        self.save(format="future-format")
        with self.assertRaisesRegex(ValueError, "unsupported"):
            self.load()
        self.save([{**self.case, "topics": []}])
        with self.assertRaisesRegex(ValueError, "topics"):
            self.load()
        self.save()
        (self.root / "example.rs").write_text("\n ", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "empty case source"):
            self.load()

    def test_oversized_source_is_refused(self):
        with (self.root / "example.rs").open("wb") as stream:
            stream.truncate(counter.MAX_BYTES + 1)
        with self.assertRaisesRegex(ValueError, "byte bound"):
            self.load()

    def test_retained_catalog_covers_matching_complete_source_names(self):
        _, rows = counter.load_catalog()
        self.assertGreaterEqual(len(rows), 40)
        for row in rows:
            self.assertTrue(row["texts"]["fluxr"].startswith("fluxr 1\n"))
            self.assertTrue(row["texts"]["compact"].startswith("v2"))
            self.assertIn("pub fn run", row["texts"]["rust"])


if __name__ == "__main__":
    unittest.main()
