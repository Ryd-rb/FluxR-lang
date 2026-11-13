"""Fail-closed eligibility of exact-candidate evidence from trusted verifiers.

This module is a policy predicate, not a signing service, sandbox or publisher.
The caller supplies an OUT-OF-BAND authenticator and artifact verifier. A receipt
cannot grant itself trust by naming a producer. Production adapters should verify
an established attestation envelope, identity policy and revocation state before
returning a principal. A hash alone never authenticates an execution.
"""
from __future__ import annotations

from .identity import canonical_bytes, content_digest, load_json_bytes, require_digest

CONTEXT = frozenset({"repository", "candidate_snapshot", "base_snapshot", "policy", "oracle", "toolchain", "runner",
                     "isolation", "platform", "target", "publication_epoch"})
DIGEST_CONTEXT = CONTEXT - {"repository", "platform", "target", "publication_epoch"}
TERMINALS = frozenset({"passed", "failed", "timeout", "cancelled", "infrastructure_error"})
MAX_CHECKS = 10_000
MAX_CASES = 100_000


def _fields(value, fields):
    if type(value) is not dict or set(value) != set(fields):
        raise ValueError("unexpected or missing evidence fields")


def _name(value):
    if (type(value) is not str or not value or len(value.encode("utf-8")) > 1024
            or any(ord(c) < 32 or ord(c) == 127 for c in value)):
        raise ValueError("invalid check/case/producer identifier")


def _time(value):
    if type(value) is not int or not 0 <= value < 2**63:
        raise ValueError("time and epoch values must be nonnegative int64")


def _context(value):
    _fields(value, CONTEXT)
    for key in DIGEST_CONTEXT:
        require_digest(value[key])
    for key in ("repository", "platform", "target"):
        _name(value[key])
    _time(value["publication_epoch"])


def validate_requirements(requirements):
    """Validate and detach trusted policy before invoking external callbacks."""
    requirements = load_json_bytes(canonical_bytes(requirements))
    _fields(requirements, {"schema", "context", "checks", "allowed_producers", "max_age_ms"})
    if requirements["schema"] != "fluxr-evidence-requirements-v1":
        raise ValueError("unsupported evidence requirements")
    _context(requirements["context"])
    _time(requirements["max_age_ms"])
    if not requirements["max_age_ms"]:
        raise ValueError("evidence lifetime must be positive")
    allowed = requirements["allowed_producers"]
    if type(allowed) is not list or not allowed:
        raise ValueError("at least one trusted producer is required")
    for producer in allowed:
        _name(producer)
    if allowed != sorted(set(allowed)):
        raise ValueError("allowed producers must be unique and sorted")
    checks = requirements["checks"]
    if type(checks) is not dict or not 1 <= len(checks) <= MAX_CHECKS:
        raise ValueError("expected a nonempty bounded check ledger")
    case_count = 0
    for check_id, check in checks.items():
        _name(check_id)
        _fields(check, {"action_key", "cases"})
        require_digest(check["action_key"])
        cases = check["cases"]
        if type(cases) is not list or not cases:
            raise ValueError("each check must declare its expected cases")
        for case in cases:
            _name(case)
        if cases != sorted(set(cases)):
            raise ValueError("case ledger must be unique and sorted")
        case_count += len(cases)
    if case_count > MAX_CASES:
        raise ValueError("too many expected cases")
    return requirements


def _receipt(value):
    _fields(value, {"schema", "context", "check_id", "action_key", "producer", "issued_ms", "expires_ms",
                    "terminal", "exit_code", "cases", "artifacts"})
    if value["schema"] != "fluxr-check-evidence-v1":
        raise ValueError("unsupported receipt schema")
    _context(value["context"])
    _name(value["check_id"])
    _name(value["producer"])
    require_digest(value["action_key"])
    for key in ("issued_ms", "expires_ms"):
        _time(value[key])
    if value["expires_ms"] <= value["issued_ms"]:
        raise ValueError("receipt expiry must follow issuance")
    if type(value["terminal"]) is not str or value["terminal"] not in TERMINALS:
        raise ValueError("unknown receipt terminal")
    code = value["exit_code"]
    if code is not None and (type(code) is not int or not -(2**31) <= code < 2**31):
        raise ValueError("invalid exit code")
    cases = value["cases"]
    if type(cases) is not dict or len(cases) > MAX_CASES:
        raise ValueError("invalid observed case ledger")
    for name, terminal in cases.items():
        _name(name)
        if type(terminal) is not str or terminal not in TERMINALS:
            raise ValueError("unknown case terminal")
    artifacts = value["artifacts"]
    if type(artifacts) is not list or not artifacts:
        raise ValueError("evidence artifacts are required")
    for digest in artifacts:
        require_digest(digest)
    if artifacts != sorted(set(artifacts)):
        raise ValueError("artifacts must be unique and sorted")


def assess_evidence(requirements, envelopes, *, now_ms, authenticate=None, verify_artifact=None):
    """Require every check, subject, case, artifact and authenticated principal.

    authenticate(canonical_payload_bytes, proof) returns a verified principal or
    None; verify_artifact(sha256) returns True only after verifying retained bytes.
    Both functions and requirements belong to trusted orchestration, not candidate
    code. Omitting either verifier rejects. Callback failures reject. Duplication,
    stale epochs, expired evidence, missing cases and skipped outcomes never pass.

    A successful predicate is not an atomic publication: recheck current epoch,
    policy/revocation and expiry at the actual publication linearization point.
    This v1 predicate only accepts exact-candidate evidence, not semantic reuse.
    """
    policy = validate_requirements(requirements)
    _time(now_ms)
    if type(envelopes) is not list or len(envelopes) > MAX_CHECKS:
        raise ValueError("invalid bounded evidence envelope list")
    # Detachment prevents callbacks mutating objects after validation (TOCTOU).
    envelopes = load_json_bytes(canonical_bytes(envelopes))
    seen, accepted, rejected = set(), {}, []
    verified_artifacts = {}
    for index, envelope in enumerate(envelopes):
        check_id, identity = None, None
        reason = None
        try:
            _fields(envelope, {"payload", "proof"})
            payload = envelope["payload"]
            _receipt(payload)
            check_id = payload["check_id"]
            encoded = canonical_bytes(payload)
            identity = content_digest(encoded)
            if check_id in seen:
                reason = "duplicate_check"
            seen.add(check_id)
            expected = policy["checks"].get(check_id)
            if reason is None and expected is None:
                reason = "unexpected_check"
            if reason is None and payload["context"] != policy["context"]:
                reason = "context_mismatch"
            if reason is None and payload["action_key"] != expected["action_key"]:
                reason = "action_mismatch"
            if reason is None and (payload["issued_ms"] > now_ms or now_ms >= payload["expires_ms"]
                                   or now_ms - payload["issued_ms"] > policy["max_age_ms"]):
                reason = "outside_validity_window"
            if reason is None:
                if not callable(authenticate):
                    reason = "authentication_unavailable"
                else:
                    # The proof is detached from the receipt and carries no
                    # built-in authority; the trusted adapter must verify it.
                    principal = authenticate(encoded, envelope["proof"])
                    if (type(principal) is not str or principal != payload["producer"]
                            or principal not in policy["allowed_producers"]):
                        reason = "untrusted_producer"
            if reason is None and set(payload["cases"]) != set(expected["cases"]):
                reason = "incomplete_or_extra_cases"
            if reason is None and (payload["terminal"] != "passed" or payload["exit_code"] != 0
                                   or any(value != "passed" for value in payload["cases"].values())):
                reason = "check_not_passed"
            if reason is None:
                if not callable(verify_artifact):
                    reason = "artifact_verifier_unavailable"
                else:
                    for artifact in payload["artifacts"]:
                        if artifact not in verified_artifacts:
                            verified_artifacts[artifact] = verify_artifact(artifact) is True
                        if not verified_artifacts[artifact]:
                            reason = "artifact_unavailable_or_corrupt"
                            break
            if reason is None:
                accepted[check_id] = identity
        except (ValueError, TypeError, UnicodeError, KeyError, RecursionError):
            reason = "malformed_evidence"
        except Exception:
            # Do not leak authenticator exceptions/credentials into reports.
            reason = "trusted_verifier_error"
        if reason is not None:
            rejected.append({"index": index, "check_id": check_id, "receipt_digest": identity, "reason": reason})
    missing = sorted(set(policy["checks"]) - set(accepted))
    return {"schema": "fluxr-evidence-decision-v1", "eligible": not rejected and not missing,
            "requirements_digest": content_digest(canonical_bytes(policy)),
            "evaluated_at_ms": now_ms, "accepted": dict(sorted(accepted.items())),
            "missing": missing, "rejected": rejected,
            "scope": "Exact-candidate eligibility under supplied trusted verifiers; no publication performed."}
