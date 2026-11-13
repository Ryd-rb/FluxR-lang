import copy
from hashlib import sha256
import json
from pathlib import Path
import sys
import tomllib
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import bench_heldout_tokens as heldout
from eval.grader import Case, PROTOCOL, grade_transcript, valid_observation
from eval import grader
from eval.runner import canonical
from eval.isolation import UntrustedExecutionDisabled, admit_frozen_fixtures, require_untrusted_sandbox

NONCE = "ab" * 32


def observation(value=7):
    return {"result": {"value": {"type": "i64", "value": value}},
            "fuel": 10, "live_bytes": 8, "trace": []}


class GraderTests(unittest.TestCase):
    def setUp(self):
        self.cases = [Case("first", observation(7)), Case("second", observation(9))]
        self.responses = [{"protocol": PROTOCOL, "run_nonce": NONCE,
                           "case_id": c.case_id, "observation": c.observation} for c in self.cases]

    def grade(self, rows=None, **kwargs):
        if rows is None:
            rows = self.responses
        transcript = b"".join(json.dumps(row).encode() + b"\n" for row in rows)
        return grade_transcript(self.cases, transcript, run_nonce=NONCE, exit_code=0, **kwargs)

    def test_complete_typed_observations_pass(self):
        result = self.grade()
        self.assertTrue(result["accepted"])
        self.assertEqual(result["checked"], 2)

    def test_early_exit_zero_and_forged_pass_are_failures(self):
        for output in (b"", b"PASS\n", b"first passed\nsecond passed\n"):
            with self.subTest(output=output):
                result = grade_transcript(self.cases, output, run_nonce=NONCE, exit_code=0)
                self.assertFalse(result["accepted"])

    def test_missing_duplicate_out_of_order_extra_and_wrong_nonce_fail(self):
        wrong = copy.deepcopy(self.responses)
        wrong[1]["run_nonce"] = "cd" * 32
        for rows in (self.responses[:1], self.responses * 2, self.responses[::-1],
                     [self.responses[0], self.responses[0]], wrong):
            with self.subTest(rows=rows):
                self.assertFalse(self.grade(rows)["accepted"])

    def test_wrong_value_meter_trace_and_trap_reject(self):
        changes = [("fuel", 9), ("live_bytes", 0), ("trace", [{"type": "bool", "value": True}]),
                   ("result", {"value": {"type": "i64", "value": 8}}),
                   ("result", {"trap": "overflow"})]
        for key, value in changes:
            rows = copy.deepcopy(self.responses)
            rows[0]["observation"][key] = value
            with self.subTest(key=key, value=value):
                self.assertEqual(self.grade(rows)["terminal"], "observation_mismatch")

    def test_type_confusion_and_extra_fields_fail(self):
        for value in (True, 7.0, "7", 1 << 64):
            rows = copy.deepcopy(self.responses)
            rows[0]["observation"]["result"]["value"]["value"] = value
            self.assertEqual(self.grade(rows)["terminal"], "protocol_violation")
        rows = copy.deepcopy(self.responses)
        rows[0]["accepted"] = True
        self.assertFalse(self.grade(rows)["accepted"])

    def test_f32_bit_identity_preserves_nan_payload_and_signed_zero(self):
        for bits in ("7fc00011", "80000000"):
            expected = observation()
            expected["result"] = {"value": {"type": "f32_bits", "value": bits}}
            cases = [Case("bits", expected)]
            row = {"protocol": PROTOCOL, "run_nonce": NONCE, "case_id": "bits", "observation": expected}
            transcript = json.dumps(row).encode() + b"\n"
            self.assertTrue(grade_transcript(cases, transcript, run_nonce=NONCE, exit_code=0)["accepted"])
            actual = copy.deepcopy(row)
            actual["observation"]["result"]["value"]["value"] = "00000000"
            self.assertFalse(grade_transcript(cases, json.dumps(actual).encode() + b"\n",
                                              run_nonce=NONCE, exit_code=0)["accepted"])

    def test_malformed_duplicate_key_truncated_and_nonzero_exit_fail(self):
        rows = b"".join(json.dumps(r).encode() + b"\n" for r in self.responses)
        for output in (rows[:-1], b"{\n{}\n", rows.replace(b'"protocol":', b'"x": 1, "x": 2, "protocol":', 1)):
            self.assertFalse(grade_transcript(self.cases, output, run_nonce=NONCE, exit_code=0)["accepted"])
        self.assertFalse(grade_transcript(self.cases, rows, run_nonce=NONCE, exit_code=1)["accepted"])
        self.assertFalse(self.grade(process_terminal="timeout")["accepted"])

    def test_invalid_or_empty_trusted_ledger_is_not_a_vacuous_success(self):
        for cases in ([], [self.cases[0], self.cases[0]], [Case("first", observation(True))]):
            with self.assertRaises(ValueError):
                grade_transcript(cases, b"", run_nonce=NONCE, exit_code=0)

    def test_generator_mutation_cannot_rewrite_a_previously_yielded_oracle(self):
        original = copy.deepcopy(self.cases)
        transcript = b"".join(json.dumps(row).encode() + b"\n" for row in self.responses)
        def cases():
            yield self.cases[0]
            self.cases[0].observation["fuel"] = 999
            yield self.cases[1]
        result = grade_transcript(cases(), transcript, run_nonce=NONCE, exit_code=0)
        expected = sha256(canonical([{"id": c.case_id, "observation": c.observation} for c in original])).hexdigest()
        self.assertTrue(result["accepted"])
        self.assertEqual(result["ledger_sha256"], expected)

    def test_unbounded_case_iterable_is_rejected_without_materializing_it(self):
        consumed = []
        def cases():
            for index in range(100):
                consumed.append(index)
                if len(consumed) > 4:
                    raise AssertionError("unbounded ledger consumption")
                yield Case(f"case-{index}", observation())
        with mock.patch.object(grader, "MAX_CASES", 3), self.assertRaisesRegex(ValueError, "bounded"):
            grade_transcript(cases(), b"", run_nonce=NONCE, exit_code=0)
        self.assertEqual(consumed, [0, 1, 2, 3])

    def test_aggregate_ledger_budget_and_exact_case_limit(self):
        with mock.patch.object(grader, "MAX_CASES", 2):
            self.assertTrue(self.grade()["accepted"])
        with mock.patch.object(grader, "MAX_LEDGER_BYTES", 32), self.assertRaisesRegex(ValueError, "byte limit"):
            self.grade()
        with self.assertRaisesRegex(ValueError, "invalid trusted"):
            grade_transcript([object()], b"", run_nonce=NONCE, exit_code=0)


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.cases = tomllib.loads(heldout.CASES.read_text())["heldout"]

    def test_current_frozen_corpus_is_admitted_with_exact_hashes(self):
        admission = admit_frozen_fixtures(heldout.ROOT, self.cases)
        self.assertEqual(admission["scope"], "trusted_repository_fixtures_only")
        self.assertFalse(admission["sandboxed"])
        self.assertIsNot(admission["cases"], self.cases)
        self.cases["checked_quotient"]["rust"] = "changed after admission"
        self.assertNotEqual(admission["cases"]["checked_quotient"]["rust"], "changed after admission")
        self.assertEqual(admission["replay_bytes"], heldout.REPLAY.read_bytes())

    def test_experimental_catalog_cannot_extend_frozen_execution_admission(self):
        # New authoring/token catalogs may evolve independently, but they are
        # not executable fixture authority. A deliberately injected companion
        # entry must neither alter the frozen copy nor become an allowed case.
        with tempfile.TemporaryDirectory(prefix="fluxr-catalog-isolation-") as temporary:
            root = Path(temporary)
            (root / "benchmarks").mkdir()
            for filename in ("cases.toml", "token-replay.rs"):
                (root / "benchmarks" / filename).write_bytes((heldout.ROOT / "benchmarks" / filename).read_bytes())
            companion = root / "benchmarks/agent-syntax.toml"
            companion.write_text('[heldout.injected]\nrust = "unreviewed executable source"\n')
            admitted = admit_frozen_fixtures(root, self.cases)
            self.assertEqual(admitted["cases"], self.cases)
            injected = {**self.cases, "injected": {"rust": "unreviewed executable source"}}
            with self.assertRaisesRegex(UntrustedExecutionDisabled, "caller-supplied"):
                admit_frozen_fixtures(root, injected)
            companion.write_text("not even valid TOML")
            self.assertEqual(admit_frozen_fixtures(root, self.cases)["cases"], admitted["cases"])

    def test_no_claim_or_flag_enables_generated_replay(self):
        with self.assertRaises(UntrustedExecutionDisabled):
            require_untrusted_sandbox()
        with mock.patch.object(heldout.subprocess, "run", side_effect=AssertionError("candidate launched")):
            with self.assertRaises(UntrustedExecutionDisabled):
                heldout.replay(self.cases, Path("target"),
                               ("checked_quotient", "rust", "source", "std::process::exit(0)"))

    def test_mutating_cases_without_override_is_not_a_bypass(self):
        cases = copy.deepcopy(self.cases)
        cases["checked_quotient"]["rust"] = "pub fn calc() { std::process::exit(0); }"
        with self.assertRaises(UntrustedExecutionDisabled):
            admit_frozen_fixtures(heldout.ROOT, cases)


if __name__ == "__main__":
    unittest.main()
