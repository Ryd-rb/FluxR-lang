"""Admission boundary for the legacy in-process Rust replay.

No generated source is executable through this API. Hash checks constrain the
remaining legacy route to reviewed repository fixtures; they are not a sandbox
and do not authenticate a hostile checkout or hostile compiler/toolchain.
"""
from hashlib import sha256
from pathlib import Path
import tomllib

FIXTURE_BASELINE = "72e27cacd1df7286b8a6490c4d31f943c660d833"
FIXTURE_SHA256 = {
    "benchmarks/cases.toml": "640a9ca7edb9b8b40fcca33d2a87f1faef79abf970f82f842f3fd1ef09ce1d8e",
    "benchmarks/token-replay.rs": "a044b0daa211c3ab1a2687e434b6ab82d3aec6cdf531ffd0f55eafedc7b96b91",
}
MAX_FIXTURE_BYTES = 2 * 1024 * 1024


class UntrustedExecutionDisabled(RuntimeError):
    pass


def require_untrusted_sandbox():
    """Fail closed until a separately reviewed, enforced adapter is implemented."""
    raise UntrustedExecutionDisabled(
        "generated-code replay is disabled: no validated compile-and-run OS sandbox is installed; "
        "provider sandbox flags, process timeouts and typed transcripts do not provide this boundary"
    )


def admit_frozen_fixtures(root, cases, override=None):
    if override is not None:
        require_untrusted_sandbox()
    observed = {}
    for name, expected in FIXTURE_SHA256.items():
        with (Path(root) / name).open("rb") as stream:
            body = stream.read(MAX_FIXTURE_BYTES + 1)
        if len(body) > MAX_FIXTURE_BYTES or sha256(body).hexdigest() != expected:
            raise UntrustedExecutionDisabled(f"trusted fixture bytes changed: {name}")
        observed[name] = body
    frozen = tomllib.loads(observed["benchmarks/cases.toml"].decode("utf-8"))["heldout"]
    if cases != frozen:
        raise UntrustedExecutionDisabled("caller-supplied cases differ from the reviewed fixture corpus")
    return {"scope": "trusted_repository_fixtures_only", "baseline": FIXTURE_BASELINE,
            "files": dict(FIXTURE_SHA256), "sandboxed": False,
            # Replay uses these verified copies, not a second read of mutable
            # checkout files or the caller's mutable dictionary.
            "cases": frozen, "replay_bytes": observed["benchmarks/token-replay.rs"]}
