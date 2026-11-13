#!/usr/bin/env python3
"""Count complete current-source update pairs on pinned existing tokenizers.

This static counter does not run code or establish semantic acceptance. Execute
the separately declared Rust regression test against the same source candidate.
"""
from __future__ import annotations

import argparse
import hashlib
from importlib.metadata import version
import json
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "benchmarks/unified-updates-v1.toml"
MAX_INPUT = 1024 * 1024


def text(path: str) -> str:
    target = (ROOT / path).resolve(strict=True)
    if not target.is_relative_to(ROOT) or not target.is_file():
        raise ValueError("fixture must be a regular file inside the source tree")
    with target.open("rb") as stream:
        raw = stream.read(MAX_INPUT + 1)
    if len(raw) > MAX_INPUT:
        raise ValueError("fixture exceeds measurement input bound")
    return raw.decode("utf-8")


def identity(value: str) -> dict:
    raw = value.encode("utf-8")
    return {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}


def count(encoding, value: str) -> dict:
    wire = json.dumps(value, ensure_ascii=False)
    return {**identity(value), "tokens": len(encoding.encode_ordinary(value)),
            "json_wire": {**identity(wire), "tokens": len(encoding.encode_ordinary(wire))}}


def tokenizer_identity(encoding) -> dict:
    ranks = hashlib.sha256()
    for token, rank in sorted(encoding._mergeable_ranks.items(), key=lambda item: item[1]):
        ranks.update(rank.to_bytes(4, "little"))
        ranks.update(len(token).to_bytes(4, "little"))
        ranks.update(token)
    return {"name": encoding.name, "vocabulary_rank_bytes_sha256": ranks.hexdigest(),
            "regex_sha256": hashlib.sha256(encoding._pat_str.encode()).hexdigest(),
            "method": "encode_ordinary once per complete UTF-8 string; no per-word sums"}


def measure(catalog_path: Path = CATALOG) -> dict:
    import tiktoken

    catalog_text = text(catalog_path.resolve(strict=True).relative_to(ROOT).as_posix())
    catalog = tomllib.loads(catalog_text)
    config = catalog["measurement"]
    if version("tiktoken") != config["tiktoken_version"]:
        raise ValueError("install the exact tiktoken version in requirements-dev.txt")
    cases = [{"name": name, "paths": spec,
              "texts": {arm: text(spec[arm]) for arm in ("before", "after", "rust")}}
             for name, spec in catalog["case"].items()]
    common = {"syntax_protocol": text(config["protocol"]),
              "shared_validation_host": text(config.get("shared_validation_host", "tests/unified_updates.rs"))}
    profiles = []
    for name in config["tokenizers"]:
        encoding = tiktoken.get_encoding(name)
        rows = []
        for case in cases:
            arms = {arm: {"path": case["paths"][arm], **count(encoding, value)}
                    for arm, value in case["texts"].items()}
            before, after = arms["before"]["tokens"], arms["after"]["tokens"]
            rows.append({"case": case["name"], **arms,
                         "source_tokens_saved": before - after,
                         "source_saving_percent": 100.0 * (before - after) / before,
                         "after_over_before": after / before,
                         "after_over_rust_kernel": after / arms["rust"]["tokens"]})
        totals = {arm: sum(row[arm]["tokens"] for row in rows) for arm in ("before", "after", "rust")}
        profiles.append({"tokenizer": tokenizer_identity(encoding), "cases": rows,
                         "source_totals": totals,
                         "aggregate_source_saving_percent": 100.0 * (totals["before"] - totals["after"]) / totals["before"],
                         "wins": sum(row["source_tokens_saved"] > 0 for row in rows),
                         "ties": sum(row["source_tokens_saved"] == 0 for row in rows),
                         "regressions": sum(row["source_tokens_saved"] < 0 for row in rows),
                         "separate_context_and_host": {key: count(encoding, value) for key, value in common.items()}})
    return {"schema": config["schema"], "scope": config["scope"],
            "catalog": identity(catalog_text), "tiktoken_version": version("tiktoken"),
            "fixture_count": len(cases), "profiles": profiles,
            "normalization": "none; exact retained UTF-8 including terminal newlines",
            "semantic_validation": {"command": config["validation"], "executed_by_counter": False},
            "live_model_trials": "not performed",
            "full_rust_token_target": "not established by arithmetic-kernel source ratios"}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=CATALOG,
                        help="retained fixture catalog inside the source tree")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    if output.is_relative_to(ROOT):
        parser.error("measurement output must be outside the source tree")
    if output.exists():
        parser.error("measurement output must be a new path")
    report = measure(args.catalog)
    with output.open("x", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")
    print(json.dumps({"output": str(output), "fixture_count": report["fixture_count"],
                      "profiles": [{"tokenizer": profile["tokenizer"]["name"],
                                    "totals": profile["source_totals"],
                                    "saving_percent": profile["aggregate_source_saving_percent"],
                                    "regressions": profile["regressions"]}
                                   for profile in report["profiles"]]}, indent=2))


if __name__ == "__main__":
    main()
