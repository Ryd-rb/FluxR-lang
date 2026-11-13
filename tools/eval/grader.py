"""Trusted complete-transcript grading, independent of the candidate process.

This primitive validates an externally supplied typed transcript. It launches no
candidate and grants no sandbox admission. Expected observations must be computed
and stored outside the candidate boundary. A nonce binds a transcript to a run;
it is not a secret and does not authenticate truthful execution.
"""
from dataclasses import dataclass
from hashlib import sha256
import re

from .runner import canonical, strict_json

PROTOCOL = "fluxr-grade/1"
MAX_CASES = 10000
MAX_TRANSCRIPT_BYTES = 8 * 1024 * 1024
MAX_LEDGER_BYTES = 8 * 1024 * 1024
MAX_LINE_BYTES = 1024 * 1024
MAX_NODES = 65536
MAX_DEPTH = 32


@dataclass(frozen=True)
class Case:
    case_id: str
    observation: dict


def _uint(value, bits=64):
    return type(value) is int and 0 <= value < 1 << bits


def _typed(value, depth=0, remaining=None):
    if remaining is None:
        remaining = [MAX_NODES]
    remaining[0] -= 1
    if depth > MAX_DEPTH or remaining[0] < 0 or not isinstance(value, dict):
        return False
    kind = value.get("type")
    if kind == "unit":
        return set(value) == {"type"}
    if set(value) != {"type", "value"}:
        return False
    item = value["value"]
    if kind == "i64":
        return type(item) is int and -(1 << 63) <= item < (1 << 63)
    if kind == "u64":
        return _uint(item)
    if kind == "bool":
        return type(item) is bool
    if kind == "f32_bits":
        return isinstance(item, str) and re.fullmatch("[0-9a-f]{8}", item) is not None
    if kind in ("list", "tuple"):
        return isinstance(item, list) and len(item) <= MAX_NODES and all(
            _typed(child, depth + 1, remaining) for child in item)
    return False


def valid_observation(value):
    if not isinstance(value, dict) or set(value) != {"result", "fuel", "live_bytes", "trace"}:
        return False
    if not _uint(value["fuel"]) or not _uint(value["live_bytes"]):
        return False
    result = value["result"]
    if not isinstance(result, dict):
        return False
    if set(result) == {"value"}:
        if not _typed(result["value"]):
            return False
    elif set(result) == {"trap"}:
        if not isinstance(result["trap"], str) or not re.fullmatch(r"[a-z][a-z0-9_.-]{0,127}", result["trap"]):
            return False
    else:
        return False
    trace = value["trace"]
    remaining = [MAX_NODES]
    return isinstance(trace, list) and len(trace) <= MAX_NODES and all(
        _typed(item, remaining=remaining) for item in trace)


def snapshot_cases(cases):
    """Consume at most MAX_CASES + 1 entries and freeze each before advancing.

    A frozen dataclass does not freeze its observation dictionary. Keeping only
    canonical bytes prevents later generator steps or caller mutations from
    changing the oracle after its identity is recorded. The aggregate ledger
    bound is independent of the per-observation depth/node limits.
    """
    snapshot, seen = [], set()
    hasher, total = sha256(b"["), 2
    for index, case in enumerate(cases):
        if index >= MAX_CASES:
            raise ValueError("expected a nonempty bounded case ledger")
        if (not isinstance(case, Case) or not isinstance(case.case_id, str)
                or not re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", case.case_id)
                or case.case_id in seen or not valid_observation(case.observation)):
            raise ValueError("invalid trusted case ledger")
        observation = canonical(case.observation)
        # Validate the owned representation too; caller-owned containers are
        # never consulted again after this point.
        if not valid_observation(strict_json(observation)):
            raise ValueError("invalid trusted case ledger snapshot")
        entry = b'{"id":' + canonical(case.case_id) + b',"observation":' + observation + b'}'
        total += len(entry) + bool(index)
        if total > MAX_LEDGER_BYTES:
            raise ValueError("trusted case ledger exceeds byte limit")
        if index:
            hasher.update(b",")
        hasher.update(entry)
        seen.add(case.case_id)
        snapshot.append((case.case_id, observation))
    if not snapshot:
        raise ValueError("expected a nonempty bounded case ledger")
    hasher.update(b"]")
    return tuple(snapshot), hasher.hexdigest()


def grade_transcript(cases, transcript, *, run_nonce, exit_code, process_terminal="exited"):
    """Every expected case must occur exactly once, in supervisor-chosen order.

    Unexpected keys, IDs, ordering, malformed/truncated JSON, extra output,
    mismatched semantic observations, and operational failure all reject. Passing
    this finite suite is test evidence, not proof of semantic equivalence.
    """
    if not isinstance(run_nonce, str) or not re.fullmatch("[0-9a-f]{64}", run_nonce):
        raise ValueError("expected a 256-bit lowercase hexadecimal run nonce")
    cases, ledger_sha256 = snapshot_cases(cases)
    if not isinstance(transcript, bytes):
        raise TypeError("transcript must be bounded bytes")
    result = {"schema_version": "fluxr-grade-result/1", "protocol": PROTOCOL,
              "ledger_sha256": ledger_sha256,
              "transcript_sha256": sha256(transcript).hexdigest(), "run_nonce": run_nonce,
              "expected": len(cases), "checked": 0, "accepted": False,
              "exit_code": exit_code, "process_terminal": process_terminal}

    def reject(reason):
        return {**result, "terminal": reason}

    if len(transcript) > MAX_TRANSCRIPT_BYTES:
        return reject("output_limit")
    if process_terminal != "exited" or type(exit_code) is not int or exit_code != 0:
        return reject("process_failure")
    if transcript and not transcript.endswith(b"\n"):
        return reject("truncated_transcript")
    rows = transcript.splitlines()
    if len(rows) != len(cases):
        return reject("incomplete_or_extra_responses")
    for (case_id, expected_observation), row in zip(cases, rows, strict=True):
        if len(row) > MAX_LINE_BYTES:
            return reject("output_limit")
        try:
            response = strict_json(row.decode("utf-8"))
        except (ValueError, UnicodeError, RecursionError):
            return reject("malformed_response")
        if (not isinstance(response, dict)
                or set(response) != {"protocol", "run_nonce", "case_id", "observation"}
                or response["protocol"] != PROTOCOL or response["run_nonce"] != run_nonce
                or response["case_id"] != case_id or not valid_observation(response["observation"])):
            return reject("protocol_violation")
        # Canonical bytes distinguish Python bool/int equality and preserve f32 bits.
        if canonical(response["observation"]) != expected_observation:
            return reject("observation_mismatch")
        result["checked"] += 1
    return {**result, "accepted": True, "terminal": "passed"}
