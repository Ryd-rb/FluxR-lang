#!/usr/bin/env python3
"""Independent negative acceptance gates for the official conformance qualifier.

These tests exercise evidence validation directly. They do not substitute for
an actual server, database, official conformance run or full-seed benchmark.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import qualify_multicore as qualifier


SUMMARY = "\n197 steps, 197 passed, 0 failed (12.3s)\nRESULT: PASS\n"
SUCCESS = {"exit_code": 0, "timed_out": False}
REVISION = "6145e7291a3625c184a58b63b86ed9d1f7304700"


def rows(count=197):
    return [
        {"step": f"independent-step-{index}", "method": "GET", "path": f"/api/articles?offset={index}",
         "status": 200, "body": {"articles": [], "articlesCount": 0}}
        for index in range(count)
    ]


def encode(records):
    return b"".join((json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
                    for record in records)


class ConformanceEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="fluxr-conformance-gate-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.transcript = self.root / "transcript.jsonl"
        self.transcript.write_bytes(encode(rows()))

    def refused(self, stdout=SUMMARY, command=None, transcript=None):
        with self.assertRaises((ValueError, OSError)):
            qualifier.validate_conformance_result(
                stdout, SUCCESS if command is None else command,
                self.transcript if transcript is None else transcript)

    def test_accepts_complete_evidence_and_hashes_exact_transcript(self):
        expected_hash = hashlib.sha256(self.transcript.read_bytes()).hexdigest()
        result = qualifier.validate_conformance_result(SUMMARY, SUCCESS, self.transcript)
        self.assertEqual(result, {"steps": [197, 197, 0], "transcript_records": 197,
                                  "transcript_sha256": expected_hash})

    def test_timeout_even_with_zero_exit_and_passing_summary_is_rejected(self):
        self.refused(command={"exit_code": 0, "timed_out": True})

    def test_nonzero_exit_is_rejected_even_with_complete_transcript(self):
        for code in [1, 2, -9, 101]:
            with self.subTest(exit_code=code):
                self.refused(command={"exit_code": code, "timed_out": False})

    def test_missing_or_wrongly_typed_exit_and_timeout_fields_are_rejected(self):
        for command in [{}, {"exit_code": 0}, {"timed_out": False},
                        {"exit_code": False, "timed_out": False},
                        {"exit_code": "0", "timed_out": False},
                        {"exit_code": 0.0, "timed_out": False},
                        {"exit_code": 0, "timed_out": 0},
                        {"exit_code": 0, "timed_out": None}]:
            with self.subTest(command=command):
                self.refused(command=command)

    def test_partial_failed_or_excess_counts_are_rejected(self):
        for summary in ["196 steps, 196 passed, 0 failed", "197 steps, 196 passed, 1 failed",
                        "198 steps, 198 passed, 0 failed", "197 steps, 197 passed, 1 failed"]:
            with self.subTest(summary=summary):
                self.refused(summary + " (1.0s)\nRESULT: PASS\n")

    def test_duplicate_summary_is_rejected(self):
        self.refused(SUMMARY + SUMMARY)

    def test_summary_must_be_an_exact_line(self):
        self.refused("PREFIX 197 steps, 197 passed, 0 failed (1.0s) SUFFIX\nRESULT: PASS\n")

    def test_missing_duplicate_or_contradictory_pass_marker_is_rejected(self):
        for stdout in [SUMMARY.replace("RESULT: PASS\n", ""),
                       SUMMARY + "RESULT: PASS\n", SUMMARY + "RESULT: FAIL\n"]:
            with self.subTest(stdout=stdout):
                self.refused(stdout)

    def test_missing_transcript_is_rejected(self):
        self.transcript.unlink()
        self.refused()

    def test_empty_transcript_is_rejected(self):
        self.transcript.write_bytes(b"")
        self.refused()

    def test_truncated_record_population_is_rejected(self):
        self.transcript.write_bytes(encode(rows(196)))
        self.refused()

    def test_excess_record_population_is_rejected(self):
        self.transcript.write_bytes(encode(rows(198)))
        self.refused()

    def test_duplicate_step_is_rejected_without_reducing_row_count(self):
        records = rows()
        records[-1]["step"] = records[0]["step"]
        self.transcript.write_bytes(encode(records))
        self.refused()

    def test_aborted_step_is_rejected(self):
        records = rows()
        records[-1]["step"] = "ABORTED"
        self.transcript.write_bytes(encode(records))
        self.refused()

    def test_malformed_json_is_rejected(self):
        self.transcript.write_bytes(b'{"broken":\n' + encode(rows(196)))
        self.refused()

    def test_truncated_last_json_record_is_rejected(self):
        data = encode(rows())
        self.transcript.write_bytes(data[:-5])
        self.refused()

    def test_blank_record_is_rejected(self):
        self.transcript.write_bytes(b"\n" + encode(rows(196)))
        self.refused()

    def test_invalid_utf8_is_rejected(self):
        self.transcript.write_bytes(encode(rows()).replace(b"independent-step-0", b"invalid-\xff", 1))
        self.refused()

    def test_duplicate_json_fields_are_rejected(self):
        data = encode(rows()).replace(b'"status":200', b'"status":200,"status":201', 1)
        self.transcript.write_bytes(data)
        self.refused()

    def test_nested_duplicate_json_fields_are_rejected(self):
        data = encode(rows()).replace(b'"articlesCount":0', b'"articlesCount":0,"articlesCount":1', 1)
        self.transcript.write_bytes(data)
        self.refused()

    def test_non_json_numeric_constants_are_rejected(self):
        for token in [b"NaN", b"Infinity", b"-Infinity"]:
            with self.subTest(token=token):
                data = encode(rows()).replace(b'"articlesCount":0', b'"articlesCount":' + token, 1)
                self.transcript.write_bytes(data)
                self.refused()

    def test_missing_extra_and_nonobject_records_are_rejected(self):
        for bad in [{"step": "missing-fields"}, rows(1)[0] | {"extra": True}, [], 42, None]:
            with self.subTest(record=bad):
                records = rows()
                records[0] = bad
                self.transcript.write_bytes(encode(records))
                self.refused()

    def test_invalid_record_fields_are_rejected(self):
        for key, value in [("step", ""), ("step", None), ("method", 7), ("path", None),
                           ("status", False), ("status", "200"), ("status", 200.0),
                           ("status", 99), ("status", 600)]:
            with self.subTest(key=key, value=value):
                records = rows()
                records[0][key] = value
                self.transcript.write_bytes(encode(records))
                self.refused()

    def test_transcript_symlink_is_rejected(self):
        target = self.root / "actual.jsonl"
        self.transcript.rename(target)
        self.transcript.symlink_to(target)
        self.refused()

    def test_transcript_byte_limit_is_enforced_before_json_parsing(self):
        with self.transcript.open("wb") as stream:
            stream.truncate(16 * 1024 * 1024 + 1)
        self.refused()


class PinnedReferenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="fluxr-reference-gate-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.reference = self.root / "reference"
        self.reference.mkdir()
        fixtures = {"schema.sql": b"CREATE TABLE test(id integer);\n",
                    "bench/cmd/conformance/main.go": b"package main\nfunc main() {}\n",
                    "seed/make-seed.sh": b"#!/bin/sh\nexit 0\n"}
        for relative, data in fixtures.items():
            path = self.reference / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        self.expected = {name: hashlib.sha256(data).hexdigest() for name, data in fixtures.items()}
        self.manifest = self.root / "manifest.json"
        self.manifest.write_text(json.dumps({"format": "fluxr-pinned-multicore-reference-v1",
                                             "revision": REVISION, "files": self.expected}))
        patcher = patch.object(qualifier, "REFERENCE_MANIFEST", self.manifest)
        patcher.start()
        self.addCleanup(patcher.stop)

    def refused(self):
        with self.assertRaises((ValueError, OSError)):
            qualifier.check_reference(self.reference)

    def test_accepts_exact_inventory_and_hashes(self):
        self.assertEqual(qualifier.check_reference(self.reference), self.expected)

    def test_source_mutation_is_rejected(self):
        with (self.reference / "bench/cmd/conformance/main.go").open("ab") as stream:
            stream.write(b"// modified after source retrieval\n")
        self.refused()

    def test_missing_file_is_rejected(self):
        (self.reference / "schema.sql").unlink()
        self.refused()

    def test_unexpected_file_is_rejected(self):
        (self.reference / "extra.go").write_text("package main\n")
        self.refused()

    def test_same_bytes_under_wrong_path_are_rejected(self):
        (self.reference / "schema.sql").rename(self.reference / "schema-renamed.sql")
        self.refused()

    def test_source_file_symlink_is_rejected_even_when_bytes_match(self):
        source = self.reference / "schema.sql"
        target = self.root / "outside.sql"
        source.rename(target)
        source.symlink_to(target)
        self.refused()

    def test_source_directory_symlink_is_rejected(self):
        source = self.reference / "bench"
        target = self.root / "outside-bench"
        source.rename(target)
        source.symlink_to(target, target_is_directory=True)
        self.refused()

    def test_wrong_pinned_revision_is_rejected(self):
        manifest = json.loads(self.manifest.read_text())
        manifest["revision"] = "0" * 40
        self.manifest.write_text(json.dumps(manifest))
        self.refused()

    def test_wrong_manifest_format_is_rejected(self):
        manifest = json.loads(self.manifest.read_text())
        manifest["format"] = "unrecognized"
        self.manifest.write_text(json.dumps(manifest))
        self.refused()


if __name__ == "__main__":
    unittest.main()
