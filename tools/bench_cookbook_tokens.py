#!/usr/bin/env python3
"""Measure complete Rust/current-FluxR/public-compatible cookbook sources.

The independent executable cookbook tests establish equivalence. This counter
only measures exact UTF-8 source and keeps shared validation/context costs
separate; it does not certify behavior or live-agent efficiency.
"""
from __future__ import annotations

import argparse
from importlib.metadata import version
import json
from pathlib import Path
import re

from bench_update_tokens import count, identity, tokenizer_identity

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CATALOG = ROOT / "examples/cookbook/catalog.json"
MAX_BYTES = 2 * 1024 * 1024
ARMS = ("rust", "fluxr", "compact")
TOKENIZERS = ("o200k_base", "cl100k_base")
SHARED = (
    "examples/cookbook/support.rs", "tests/cookbook.rs",
    "examples/bench_cookbook.rs", "examples/cookbook/tests.json",
    "src/testing.rs", "docs/testing.md", "docs/cookbook.md",
)


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field: " + key)
        result[key] = value
    return result


def read_text(path: Path, root: Path) -> str:
    root = root.resolve(strict=True)
    path = path.absolute()
    if not path.is_relative_to(root):
        raise ValueError("source must be inside the source tree")
    for component in (path, *path.parents):
        if component == root:
            break
        if component.is_symlink():
            raise ValueError("source symlinks are not admitted")
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to(root) or not resolved.is_file():
        raise ValueError("source must be a regular file inside the source tree")
    with resolved.open("rb") as stream:
        raw = stream.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES:
        raise ValueError("source exceeds measurement byte bound")
    return raw.decode("utf-8")


def load_catalog(path: Path = DEFAULT_CATALOG, root: Path = ROOT):
    root = root.resolve(strict=True)
    path = path.absolute()
    raw = read_text(path, root)
    data = json.loads(raw, object_pairs_hook=unique)
    if not isinstance(data, dict) or data.get("format") != "fluxr-cookbook-v1":
        raise ValueError("unsupported cookbook catalog")
    cases = data.get("cases")
    if not isinstance(cases, list) or not 1 <= len(cases) <= 256:
        raise ValueError("catalog must contain 1..256 cases")
    names, paths, rows = set(), set(), []
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("case must be an object")
        name, topics = case.get("name"), case.get("topics")
        if not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,79}", name) or name in names:
            raise ValueError("invalid or duplicate case name")
        if not isinstance(topics, list) or not topics or not all(isinstance(t, str) and t for t in topics):
            raise ValueError("case must declare topics")
        names.add(name)
        texts, declared_paths = {}, {}
        for arm in ARMS:
            value = case.get(arm)
            if not isinstance(value, str) or not value or "\\" in value:
                raise ValueError("case source path must be a relative POSIX path")
            relative = Path(value)
            if relative.is_absolute() or any(part in (".", "..") for part in value.split("/")):
                raise ValueError("unsafe case source path")
            source = path.parent / relative
            key = source.absolute()
            if key in paths:
                raise ValueError("duplicate case source path")
            paths.add(key)
            texts[arm] = read_text(source, root)
            if not texts[arm].strip():
                raise ValueError("empty case source")
            declared_paths[arm] = source.relative_to(root).as_posix()
        rows.append({"name": name, "topics": topics, "paths": declared_paths, "texts": texts})
    return raw, rows


def measure(catalog: Path = DEFAULT_CATALOG) -> dict:
    import tiktoken

    if version("tiktoken") != "0.14.0":
        raise ValueError("install tiktoken==0.14.0 from requirements-dev.txt")
    raw, cases = load_catalog(catalog)
    common = {path: read_text(ROOT / path, ROOT) for path in SHARED}
    profiles = []
    for name in TOKENIZERS:
        encoding = tiktoken.get_encoding(name)
        rows = []
        for case in cases:
            arms = {arm: {"path": case["paths"][arm], **count(encoding, case["texts"][arm])}
                    for arm in ARMS}
            rows.append({"name": case["name"], "topics": case["topics"], **arms,
                         "fluxr_over_rust": arms["fluxr"]["tokens"] / arms["rust"]["tokens"],
                         "fluxr_over_public_compact": arms["fluxr"]["tokens"] / arms["compact"]["tokens"]})
        totals = {arm: sum(row[arm]["tokens"] for row in rows) for arm in ARMS}
        profiles.append({"tokenizer": tokenizer_identity(encoding), "cases": rows,
                         "source_totals": totals,
                         "fluxr_over_rust": totals["fluxr"] / totals["rust"],
                         "fluxr_over_public_compact": totals["fluxr"] / totals["compact"],
                         "separate_context_and_validation": {
                             path: count(encoding, value) for path, value in common.items()}})
    return {"schema": "fluxr-cookbook-tokens-v1", "catalog": identity(raw),
            "fixture_count": len(cases), "tiktoken_version": version("tiktoken"),
            "normalization": "none; entire UTF-8 files including final newlines; encode_ordinary",
            "baseline_representation": "retained compact source executable on the pinned public baseline",
            "incoming_comparison": "this new corpus was absent from the incoming pack; current-source syntax is unchanged",
            "semantic_validation_command": "cargo test --locked --test cookbook",
            "semantic_validation_executed_by_counter": False,
            "native_rust_boundary": "kernel source only; adapters, resource checks and test context are separate",
            "live_model_trials": "not performed", "profiles": profiles}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=DEFAULT_CATALOG)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.is_relative_to(ROOT) or output.exists():
        parser.error("output must be a new path outside the source tree")
    report = measure(args.catalog)
    payload = json.dumps(report, indent=2, sort_keys=True, allow_nan=False) + "\n"
    if len(payload.encode()) > MAX_BYTES * 8:
        parser.error("token report exceeds output bound")
    with output.open("x", encoding="utf-8") as stream:
        stream.write(payload)
    print(json.dumps({"output": str(output), "fixture_count": report["fixture_count"],
                      "profiles": [{"tokenizer": p["tokenizer"]["name"],
                                    "source_totals": p["source_totals"],
                                    "fluxr_over_rust": p["fluxr_over_rust"],
                                    "fluxr_over_public_compact": p["fluxr_over_public_compact"]}
                                   for p in report["profiles"]]}, indent=2))


if __name__ == "__main__":
    main()
