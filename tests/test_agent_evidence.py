"""Negative tests for exact-candidate evidence authorization boundaries."""
from copy import deepcopy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from agent_pipeline.evidence import assess_evidence, validate_requirements
from agent_pipeline.identity import canonical_bytes, content_digest


def h(value):
    return content_digest(str(value).encode())


def requirements():
    return {"schema": "fluxr-evidence-requirements-v1",
            "context": {**{key: h(key) for key in ("candidate_snapshot", "base_snapshot", "policy", "oracle",
                                                  "toolchain", "runner", "isolation")},
                        "repository": "github.com/repositories/1450130567",
                        "platform": "linux-x86_64", "target": "x86_64-unknown-linux-gnu", "publication_epoch": 2},
            "checks": {"runtime": {"action_key": h("action"), "cases": ["add", "trap"]}},
            "allowed_producers": ["trusted-builder"], "max_age_ms": 1000}


def receipt():
    r = requirements()
    return {"payload": {"schema": "fluxr-check-evidence-v1", "context": r["context"], "check_id": "runtime",
                        "action_key": h("action"), "producer": "trusted-builder", "issued_ms": 1000,
                        "expires_ms": 3000, "terminal": "passed", "exit_code": 0,
                        "cases": {"add": "passed", "trap": "passed"}, "artifacts": [h("log")]},
            "proof": {"test_fixture": "not a real signature"}}


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.req = requirements()
        self.env = receipt()
        self.trusted_bytes = canonical_bytes(self.env["payload"])

    def auth(self, payload, proof):
        # A test-only fixture: production MUST use a real trusted verifier.
        return "trusted-builder" if payload == self.trusted_bytes else None

    def verify(self, digest):
        return digest == h("log")

    def assess(self, env=None, **kwargs):
        return assess_evidence(self.req, [env or self.env], now_ms=1500,
                               authenticate=kwargs.get("authenticate", self.auth),
                               verify_artifact=kwargs.get("verify_artifact", self.verify))

    def test_exact_authenticated_complete_evidence_is_eligible(self):
        result = self.assess()
        self.assertTrue(result["eligible"])
        self.assertEqual([], result["missing"])

    def test_receipt_cannot_self_authenticate_and_hash_is_not_a_signature(self):
        for auth in (None, lambda payload, proof: None, lambda payload, proof: "other-builder"):
            with self.subTest(auth=auth):
                self.assertFalse(self.assess(authenticate=auth)["eligible"])
        changed = deepcopy(self.env)
        changed["payload"]["producer"] = "self-asserted-trusted-builder"
        self.assertFalse(self.assess(changed)["eligible"])

    def test_every_context_field_prevents_wrong_candidate_reuse(self):
        for key, value in self.req["context"].items():
            changed = deepcopy(self.env)
            changed["payload"]["context"][key] = value + 1 if type(value) is int else h("different")
            with self.subTest(key=key):
                result = self.assess(changed)
                self.assertFalse(result["eligible"])
                self.assertEqual("context_mismatch", result["rejected"][0]["reason"])

    def test_epoch_fences_aba_and_base_identity_is_separate(self):
        for key in ("publication_epoch", "base_snapshot"):
            old = deepcopy(self.req)
            old["context"][key] = 3 if key == "publication_epoch" else h("new base")
            self.assertFalse(assess_evidence(old, [self.env], now_ms=1500,
                                           authenticate=self.auth, verify_artifact=self.verify)["eligible"])

    def test_missing_failed_and_extra_cases_reject_even_authenticated(self):
        for cases in ({"add": "passed"}, {"add": "passed", "trap": "failed"},
                      {"add": "passed", "trap": "passed", "extra": "passed"}):
            changed = deepcopy(self.env)
            changed["payload"]["cases"] = cases
            self.assertFalse(self.assess(changed, authenticate=lambda p, s: "trusted-builder")["eligible"])

    def test_missing_checks_duplicate_results_and_empty_requirements_reject(self):
        result = assess_evidence(self.req, [], now_ms=1500, authenticate=self.auth, verify_artifact=self.verify)
        self.assertFalse(result["eligible"])
        self.assertEqual(["runtime"], result["missing"])
        result = assess_evidence(self.req, [self.env, self.env], now_ms=1500,
                                 authenticate=self.auth, verify_artifact=self.verify)
        self.assertFalse(result["eligible"])
        self.assertEqual("duplicate_check", result["rejected"][0]["reason"])
        self.req["checks"] = {}
        with self.assertRaises(ValueError):
            self.assess()

    def test_artifacts_must_be_verified_not_merely_named(self):
        for verifier in (None, lambda digest: False, lambda digest: 1):
            with self.subTest(verifier=verifier):
                self.assertFalse(self.assess(verify_artifact=verifier)["eligible"])

    def test_expiry_future_issue_and_age_limits_are_exact(self):
        for now in (999, 2001, 3000):
            with self.subTest(now=now):
                self.assertFalse(assess_evidence(self.req, [self.env], now_ms=now,
                                               authenticate=self.auth, verify_artifact=self.verify)["eligible"])
        for now in (1000, 2000):
            self.assertTrue(assess_evidence(self.req, [self.env], now_ms=now,
                                          authenticate=self.auth, verify_artifact=self.verify)["eligible"])

    def test_callback_errors_fail_closed_without_exception_details(self):
        def fail(*args):
            raise RuntimeError("sensitive backend detail")
        result = self.assess(authenticate=fail)
        self.assertFalse(result["eligible"])
        self.assertNotIn("sensitive", str(result))
        self.assertEqual("trusted_verifier_error", result["rejected"][0]["reason"])

    def test_callback_cannot_mutate_validated_receipt_or_policy(self):
        def mutate(payload, proof):
            self.req["checks"].clear()
            self.env["payload"]["terminal"] = "failed"
            return self.auth(payload, proof)
        self.assertTrue(self.assess(authenticate=mutate)["eligible"])

    def test_terminal_exit_and_unknown_fields_reject(self):
        for key, value in (("terminal", "cancelled"), ("terminal", "skipped"),
                           ("exit_code", True), ("exit_code", 1), ("action_key", h("other")),
                           ("issued_ms", True), ("extra", "field")):
            changed = deepcopy(self.env)
            changed["payload"][key] = value
            with self.subTest(key=key, value=value):
                self.assertFalse(self.assess(changed, authenticate=lambda p, s: "trusted-builder")["eligible"])

    def test_trusted_requirement_types_and_ledger_are_strict(self):
        for value in (True, 0, -1, 1.5):
            malformed = requirements()
            malformed["max_age_ms"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                validate_requirements(malformed)


if __name__ == "__main__":
    unittest.main()
