"""Measure FA1 against exact authored compact source on a pinned tokenizer panel.

Compile equality is checked first. Report code and explicit syntax-context costs
separately; no live model success or overall coding-throughput claim is made.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
import json
from pathlib import Path
from statistics import median
import subprocess
import tempfile
import tomllib

from bench_heldout_tokens import codecs

ROOT = Path(__file__).resolve().parents[1]
PANEL = ROOT / "benchmarks/token-panel.toml"
CORPUS = ROOT / "benchmarks/agent-syntax.toml"
# These are fixed comparison aids, not complete prompts used for live trials.
COMPACT_CARD = """Compact FluxR: v2. Function: (pub name (name:type ...) result (local:type ...) statements...). A single return with no locals can be (pub name (name:type ...) result = expression). Types: unit bool i64 u64 f32, record name or @name, #enum, [type]. Group adjacent binding names before :type. Statements: (= local expr), (= local.field expr), (push local expr), (pop local), (if test (statements...) (statements...)), (while test (statements...)), (break), (continue), (ret expr). Expressions: unit true false signed-integer unsigned-integer-u float-bits-0xHEXf, local, (local reserved-name), $input, (operator operands...), (new record expr...), (field expr field), (tag enum.variant optional-payload), (is expr variant), (payload expr variant), (list type expr...), (empty expr), (last expr), (at list index), (minidx expr), (call name expr...), (m receiver name expr...). Binary operators: + - * / % == < <= && ||. Unary: ! abs tof i64tof ftoi64 sqrt. Top declarations: (inputs bindings...), (rec name bindings...), (enum name variant variant:type...), (fn name ...) or (fn receiver.method ...). Existing typed checking, limits, order, checked arithmetic and canonical FSM apply."""
AGENT_CARD = """FA1 FluxR: fa1 v2. Every token is whitespace-separated. Function: pub name COUNT binding... result ret expression, or pub name COUNT binding... result do COUNT local... COUNT statement.... Binding: name type. Types: unit bool int uint float, record name, enum name, list type. Statements: set local expr; put local field expr; push local expr; pop local; if test COUNT then-statement... COUNT else-statement...; while test COUNT statement...; break; continue; ret expr. Semicolons here separate grammar alternatives; never output them. Expressions: unit true false signed-integer unsigned-integer-u exact-float-bits-0xHEXf, local-name, local reserved-name, input name, prefix-operator operands, new record COUNT expr..., field expr field, tag enum variant, wrap enum variant payload, is expr variant, payload expr variant, list type COUNT expr..., empty expr, last expr, at list index, minidx expr, call name COUNT expr..., m receiver name COUNT expr.... Binary: add sub mul div rem eq lt le and or. Unary: not abs tof i64tof ftoi64 sqrt. Top declarations: inputs COUNT binding..., rec name COUNT binding..., enum name COUNT variant payload-type-or-none..., fn name ..., method receiver name.... Counts are bounded canonical decimal integers, independent of declaration lookup. Existing typed checking, limits, order, checked arithmetic and canonical FSM apply."""


def text(path: Path) -> str:
    # Equal terminal newline policy, retain all other authored whitespace.
    return path.read_text(encoding="utf-8").replace("\r\n", "\n").rstrip("\n") + "\n"


def run(binary: Path, *args: str) -> None:
    result = subprocess.run([str(binary), *args], capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise ValueError(f"compiler refused {args[0]}: {result.stderr[-4000:]}")


def measure(binary: Path, cache: Path) -> dict:
    corpus = tomllib.loads(CORPUS.read_text())["agent_syntax"]["fixtures"]
    rows = []
    with tempfile.TemporaryDirectory(prefix="fluxr-agent-syntax-") as temporary:
        folder = Path(temporary)
        for index, item in enumerate(corpus):
            path = ROOT / item
            original = text(path)
            if not original.startswith(("v1", "v2")):
                raise ValueError(f"Fixture is not a whole structured module: {item}")
            source = folder / f"source{index}.flux"
            source.write_text(original)
            fa = folder / f"source{index}.fa1"
            before = folder / f"before{index}.fsm"
            after = folder / f"after{index}.fsm"
            normalized = folder / f"normalized{index}.flux"
            run(binary, "project-agent", str(source), str(fa))
            run(binary, "encode-structured", str(source), str(before))
            run(binary, "encode-agent", str(fa), str(after))
            run(binary, "migrate-structured", str(before), str(normalized))
            if before.read_bytes() != after.read_bytes():
                raise ValueError(f"Semantic artifact changed for {item}")
            rows.append({"path": item, "authored_compact": original, "canonical_compact": text(normalized),
                         "agent": text(fa), "canonical_sha256": sha256(before.read_bytes()).hexdigest()})
    panel = tomllib.loads(PANEL.read_text())
    results = []
    for name, family, count in codecs(panel, cache):
        cases = []
        for row in rows:
            sizes = {key: count(row[key]) for key in ("authored_compact", "canonical_compact", "agent")}
            cases.append({"path": row["path"], "tokens": sizes,
                          "bytes": {key: len(row[key].encode()) for key in sizes},
                          "source_sha256": {key: sha256(row[key].encode()).hexdigest() for key in sizes},
                          "canonical_sha256": row["canonical_sha256"],
                          "agent_over_authored_compact": sizes["agent"] / sizes["authored_compact"]})
        ratios = [case["agent_over_authored_compact"] for case in cases]
        old = sum(case["tokens"]["authored_compact"] for case in cases)
        new = sum(case["tokens"]["agent"] for case in cases)
        contexts = {"compact": count(COMPACT_CARD), "agent": count(AGENT_CARD)}
        examples = ["(", ")", "))),", ")\n", " ()", "(ret (+ x y))", "ret add x y",
                    "foobarbaz", " increment_then_return_checked_i64"]
        results.append({"name": name, "family": family, "cases": cases,
                        "code_totals": {"authored_compact": old, "agent": new},
                        "agent_over_compact": {"median": median(ratios), "aggregate": new / old},
                        "wins": sum(r < 1 for r in ratios), "ties": sum(r == 1 for r in ratios),
                        "losses": sum(r > 1 for r in ratios),
                        "syntax_card_tokens": contexts,
                        "one_card_once_plus_corpus": {"compact": old + contexts["compact"], "agent": new + contexts["agent"]},
                        "card_repeated_per_case_plus_corpus": {"compact": old + len(cases) * contexts["compact"], "agent": new + len(cases) * contexts["agent"]},
                        "punctuation_and_keyword_probes": [{"text": s, "tokens": count(s), "bytes": len(s.encode())} for s in examples]})
    return {"schema": "fluxr-agent-syntax-benchmark-v1", "panel_sha256": sha256(PANEL.read_bytes()).hexdigest(),
            "corpus_sha256": sha256(CORPUS.read_bytes()).hexdigest(), "tool_sha256": sha256(Path(__file__).read_bytes()).hexdigest(),
            "binary_sha256": sha256(binary.read_bytes()).hexdigest(),
            "syntax_cards": {"compact": COMPACT_CARD, "agent": AGENT_CARD},
            "panel": panel, "fixture_count": len(rows), "results": results,
            "canonical_equality_checked": True, "normalization": "UTF-8, LF, one terminal LF; other authored whitespace retained",
            "live_model_trials": "not run", "scope": "Pinned tokenizer code costs and illustrative syntax-card amortization. Syntax cards are not matched live prompts; no 10-percent Rust target or agent productivity claim."}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/fluxr-cli")
    parser.add_argument("--cache", type=Path, default=ROOT / "target/token-cache")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = measure(args.binary.resolve(strict=True), args.cache.resolve())
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"output": str(args.output), "fixtures": report["fixture_count"],
                      "results": [{"name": r["name"], "ratios": r["agent_over_compact"], "wins": r["wins"], "losses": r["losses"]} for r in report["results"]]}, indent=2))


if __name__ == "__main__":
    main()
