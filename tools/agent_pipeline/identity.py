"""Versioned, deliberately restricted JSON identities for build/test actions.

This is not RFC 8785: floats are forbidden, strings retain exact Unicode, and
all clients must implement the specified Python-compatible JSON encoding.
Identity does not establish that the input closure or producer is trustworthy.
"""
from __future__ import annotations

import hashlib
import json
import re

MAX_JSON_BYTES = 4 * 1024 * 1024
HEX = re.compile(r"[0-9a-f]{64}\Z")
_LONG_ESCAPES = re.compile("[\x00-\x07\x0b\x0e-\x1f]")


def require_digest(value: str) -> str:
    if not isinstance(value, str) or HEX.fullmatch(value) is None:
        raise ValueError("expected lowercase SHA-256")
    return value


class _JsonBudget:
    """Count exact canonical UTF-8 bytes before allocating serialized output."""

    def __init__(self):
        self.remaining = MAX_JSON_BYTES

    def charge(self, size):
        self.remaining -= size
        if self.remaining < 0:
            raise ValueError("canonical JSON exceeds byte limit")

    def string(self, value):
        self.charge(2)  # quotes
        # UTF-8 and JSON escaping cannot shorten a string. Reject huge scalar
        # values before encoding/copying them; encode admitted chunks only.
        if len(value) > self.remaining:
            raise ValueError("canonical JSON exceeds byte limit")
        for start in range(0, len(value), 8192):
            chunk = value[start:start + 8192]
            size = len(chunk.encode("utf-8", errors="strict"))
            size += sum(chunk.count(c) for c in ('"', "\\", "\b", "\f", "\n", "\r", "\t"))
            # The remaining C0 controls use six-byte \\u00XX escapes. Ordinary
            # Unicode retains its original UTF-8 encoding (ensure_ascii=False).
            size += 5 * len(_LONG_ESCAPES.findall(chunk))
            self.charge(size)


def _validate(value, depth=0, budget=None, *, own=False):
    if budget is None:
        budget = _JsonBudget()
    if depth > 32:
        raise ValueError("JSON nesting exceeds 32")
    if value is None:
        budget.charge(4)
        return
    if type(value) is bool:
        budget.charge(4 if value else 5)
        return value
    if type(value) is int:
        if not -(2**63) <= value < 2**63:
            raise ValueError("JSON integer is outside signed 64-bit range")
        budget.charge(len(str(value)))
        return value
    if type(value) is str:
        budget.string(value)
        return value
    if type(value) is list:
        budget.charge(2)
        if len(value) > budget.remaining:
            raise ValueError("canonical JSON exceeds byte limit")
        result = [] if own else None
        for index, item in enumerate(value):
            budget.charge(1 if index else 0)
            validated = _validate(item, depth + 1, budget, own=own)
            if own:
                result.append(validated)
        return result if own else value
    if type(value) is dict:
        budget.charge(2)
        if len(value) > budget.remaining:
            raise ValueError("canonical JSON exceeds byte limit")
        result = {} if own else None
        for index, (key, item) in enumerate(value.items()):
            budget.charge(2 if index else 1)  # colon and optional comma
            if type(key) is not str:
                raise ValueError("JSON object keys must be strings")
            _validate(key, depth + 1, budget)
            validated = _validate(item, depth + 1, budget, own=own)
            if own:
                result[key] = validated
        return result if own else value
    raise ValueError("only null, booleans, int64, strings, lists and string-key objects are allowed")


def canonical_bytes(value) -> bytes:
    # Validation shares an exact cumulative budget across containers. Neither
    # a giant scalar nor many individually small values can cause an unbounded
    # json.dumps/UTF-8 allocation before the 4 MiB protocol limit is enforced.
    try:
        owned = _validate(value, own=True)
    except RuntimeError as error:
        raise ValueError("JSON input changed while being validated") from error
    result = json.dumps(owned, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    if len(result) > MAX_JSON_BYTES:
        raise ValueError("canonical JSON exceeds byte limit")
    return result


def load_json_bytes(data: bytes):
    if len(data) > MAX_JSON_BYTES:
        raise ValueError("JSON input exceeds byte limit")

    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"duplicate JSON key {key!r}")
            result[key] = value
        return result

    def reject(value):
        raise ValueError(f"non-finite JSON constant {value}")

    try:
        result = json.loads(data.decode("utf-8"), object_pairs_hook=pairs, parse_constant=reject)
        _validate(result)
        return result
    except RecursionError as error:
        raise ValueError("JSON nesting exceeds parser limit") from error


def content_digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def relative_path(value: str) -> str:
    if not isinstance(value, str) or not value or "\\" in value:
        raise ValueError("expected a nonempty relative POSIX path")
    if any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise ValueError("control characters in path")
    if re.match(r"[A-Za-z]:", value):
        raise ValueError("Windows drive-qualified paths are forbidden")
    if any(part in ("", ".", "..") for part in value.split("/")):
        raise ValueError("absolute, empty and dot path components are forbidden")
    return value


def action_key(spec: dict) -> str:
    """Hash every declared semantic input; never execute the command.

    source_snapshot is the digest of the complete immutable source input tree
    for this conservative v1 prototype. Fine-grained reuse needs a trusted
    compiler/runner-generated dependency manifest and a new contract version.
    """
    # Validate the same detached bytes that are ultimately hashed. Otherwise a
    # caller mutating an already checked field while later fields are examined
    # can obtain an identity for a specification that never passed validation.
    payload = canonical_bytes(spec)
    spec = load_json_bytes(payload)
    fields = {"schema", "argv", "working_directory", "environment", "inputs", "outputs", "source_snapshot",
              "toolchain", "runner", "platform", "target", "profile", "policy"}
    if not isinstance(spec, dict) or set(spec) != fields or spec["schema"] != "fluxr-action-v1":
        raise ValueError("invalid action-v1 fields or schema")
    if not isinstance(spec["argv"], list) or not spec["argv"] or not all(isinstance(x, str) and "\0" not in x for x in spec["argv"]):
        raise ValueError("argv must be a nonempty list of NUL-free strings")
    if not spec["argv"][0]:
        raise ValueError("empty executable")
    if spec["working_directory"] != ".":
        relative_path(spec["working_directory"])
    env = spec["environment"]
    if not isinstance(env, dict) or not all(isinstance(k, str) and k and "=" not in k and "\0" not in k and isinstance(v, str) and "\0" not in v for k, v in env.items()):
        raise ValueError("invalid explicit environment")
    inputs = spec["inputs"]
    if not isinstance(inputs, dict):
        raise ValueError("inputs must map paths to SHA-256 values")
    for path, digest in inputs.items():
        relative_path(path)
        require_digest(digest)
    outputs = spec["outputs"]
    if not isinstance(outputs, list):
        raise ValueError("outputs must be a list")
    for path in outputs:
        relative_path(path)
    if len(set(outputs)) != len(outputs) or outputs != sorted(outputs):
        raise ValueError("outputs must be unique and sorted")
    for field in ("source_snapshot", "toolchain", "runner", "policy"):
        require_digest(spec[field])
    for field in ("platform", "target", "profile"):
        if not isinstance(spec[field], str) or not spec[field].strip():
            raise ValueError(f"{field} must be nonempty")
    return content_digest(b"fluxr/action/v1\0" + payload)
