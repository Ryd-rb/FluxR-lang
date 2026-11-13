"""Seal, replay and count the FXM-04.A token benchmark. Python 3.11+."""
import argparse
from hashlib import sha1, sha256
from importlib.metadata import version
import json
from math import ceil
import os
from pathlib import Path
from statistics import median
import subprocess
import tempfile
import tomllib
from urllib.request import urlopen

from eval.isolation import admit_frozen_fixtures

ROOT = Path(__file__).resolve().parents[1]
PANEL = ROOT / "benchmarks/token-panel.toml"
CASES = ROOT / "benchmarks/cases.toml"
REPLAY = ROOT / "benchmarks/token-replay.rs"
LANGUAGES = ("fluxr", "rust")
ACTIONS = ("source", "edit")
FLUXR_HELP = {
    "structured": (
        "Compact modules start with v2. An export is (pub name (arg:type ...) type "
        "(local:type ...) statement ...). Return with (ret expr). Operators are prefix "
        "in parentheses. Compare with < or <=. Integer arithmetic is checked. Types include i64, u64, f32, "
        "bool and [type]. Unsigned literals end in u. (at list index) checks bounds. "
        "(minidx list) returns the first strictly least f32 index, or zero for empty "
        "input. (rec Name field:type ...) defines a record. Read a field as arg.field "
        "and assign with (= arg.field expr). (when condition statement) guards a "
        "statement. (find index end row list predicate) scans from zero up to end "
        "using the declared index and row locals."
    ),
    "rule": (
        "Start with r1 then the input count. After that, use space-separated postfix "
        "tokens. iN reads input N starting at zero. Signed integer literals and "
        "checked + - * / % are available. ? consumes condition, true value, false "
        "value in that order after all three have been evaluated."
    ),
    "gate": (
        "A flat gate starts with & for all or | for any, followed immediately by "
        "the ordered fact names, consecutive from a. Gates short-circuit."
    ),
    "timeline": (
        "Start with v2 and the period in ms. Ordinary rows use object field curve "
        "then time:value keys. Run rows start with + and use first-object count "
        "field curve signed-step then time:value keys. Curve 0 holds and 1 is linear."
    ),
}
RUST_HELP = {
    "checked_quotient": "pub fn calc(x: i64, y: i64, d: i64) -> Option<i64>",
    "ordered_rule": "pub fn calc(x: i64, y: i64, flag: i64) -> Option<i64>",
    "route_gate": "pub fn calc(fact: impl FnMut(usize) -> bool) -> bool",
    "translated_pulse": "pub fn calc(object: u16, field: u16, time: u32) -> Option<i32>",
    "minimum_bits": "pub fn calc(x: &[f32]) -> Option<f32>",
    "first_alert": "pub fn calc(x: &[i64], n: u64, t: i64) -> Option<u64>",
    "ledger": "pub struct Ledger { pub balance: i64, pub watermark: i64 } "
              "and pub fn calc(r: Ledger, d: i64) -> Option<Ledger>",
}


def read(path):
    return path.read_text(encoding="utf-8")


def edit(case, language, replacement):
    before = case[language]
    anchor = case[f"{language}_span"][0]
    if before.count(anchor) != 1:
        raise ValueError(f"Expected exactly one edit anchor {anchor!r}")
    return before.replace(anchor, replacement)


def prompt(name, case, language, action):
    if language == "fluxr":
        reference = FLUXR_HELP[case["kind"]]
        language_name = "compact FluxR"
    else:
        reference = (f"Use Rust 2024 and the standard library. Provide {RUST_HELP[name]}. "
                     "None represents a trap or missing sample.")
        language_name = "Rust"
    text = f"Produce {language_name} for the task below. Do not use tools.\n{reference}\n\n{case['task']}\n"
    if action == "source":
        return text + "Return only the complete source, without Markdown.\n"
    anchor = case[f"{language}_span"][0]
    return (
        f"{text}\nOriginal source:\n{case[language]}\nChange: {case['edit']}\n"
        f"Replace the unique span {anchor!r}. Return only the exact replacement text, without Markdown.\n"
    )


def write(path, text):
    path.write_text(text, encoding="utf-8", newline="\n")


def seal_inputs(cases):
    evaluator = Path(__file__).resolve()
    sources = (PANEL, CASES, REPLAY, evaluator, evaluator.with_name("bench_heldout_live.py"),
               *sorted((evaluator.parent / "eval").glob("*.py")))
    return {
        "files": {path.relative_to(ROOT).as_posix(): sha256(read(path).encode()).hexdigest() for path in sources},
        "prompts": {
            f"{name}.{language}.{action}": sha256(prompt(name, case, language, action).encode()).hexdigest()
            for name, case in cases.items() for language in LANGUAGES for action in ACTIONS
        },
    }


def replay(cases, target, override=None):
    """Replay only byte-pinned, reviewed repository fixtures.

    Generated overrides are deliberately denied. The legacy comparison binary
    embeds Rust fixture code in its oracle process and cannot safely grade an
    untrusted candidate. See tools/eval/README.md for the replacement boundary.
    """
    admitted = admit_frozen_fixtures(ROOT, cases, override)
    cases = admitted["cases"]
    with tempfile.TemporaryDirectory(prefix="fluxr-heldout-") as directory:
        folder = Path(directory)
        replay_source = folder / "trusted-replay.rs"
        replay_source.write_bytes(admitted["replay_bytes"])
        modules = []
        calls = []
        for name, case in cases.items():
            for action in ACTIONS:
                bodies = {language: case[language] for language in LANGUAGES}
                if action == "edit":
                    bodies = {language: edit(case, language, case[f"{language}_span"][1]) for language in LANGUAGES}
                key = f"{name}_{action}"
                write(folder / f"{key}.flux", bodies["fluxr"])
                modules.append(f"mod {key} {{\n{bodies['rust']}}}\n")
                rust = f"{key}::calc"
                if name == "ledger":
                    rust = (f"|balance, watermark, d| {rust}({key}::Ledger {{ balance, watermark }}, d)"
                            ".map(|v| (v.balance, v.watermark))")
                elif name == "route_gate":
                    rust = f"|fact| {rust}(fact)"
                calls.append(f'    {name}(include_str!("{key}.flux"), {rust});\n    println!("{key} passed");')
        write(folder / "rust-modules.rs", "".join(modules) + "\nfn replay() {\n" + "\n".join(calls) + "\n}\n")
        build = target / "release"
        binary = folder / ("replay.exe" if os.name == "nt" else "replay")
        subprocess.run(
            ["rustc", "--edition=2024", "-C", "opt-level=3", "--extern", f"fluxr={build / 'libfluxr.rlib'}",
             "-L", f"dependency={build / 'deps'}", "-o", str(binary), str(replay_source)],
            check=True, timeout=90, env={**os.environ, "FLUXR_HELDOUT": directory},
        )
        subprocess.run([str(binary)], check=True, timeout=10, env={**os.environ, "FLUXR_TELEMETRY": "off"})


def codecs(panel, cache):
    import tiktoken
    from tokenizers import Tokenizer

    for dependency in ("tiktoken", "tokenizers"):
        if version(dependency) != panel[dependency]:
            raise ValueError(f"Expected {dependency} {panel[dependency]}")
    cache.mkdir(parents=True, exist_ok=True)
    os.environ["TIKTOKEN_CACHE_DIR"] = str(cache)
    for entry in panel["panel"]:
        name, url, expected = (entry[key] for key in ("name", "url", "sha256"))
        openai = entry["family"] == "OpenAI"
        path = cache / (sha1(url.encode()).hexdigest() if openai else f"{name}.json")
        if not path.exists():
            with urlopen(url, timeout=30) as response:
                data = response.read()
            if sha256(data).hexdigest() != expected:
                raise ValueError(f"Downloaded tokenizer hash mismatch: {name}")
            path.write_bytes(data)
        if sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"Cached tokenizer hash mismatch: {name}")
        if openai:
            codec = tiktoken.get_encoding(name)
            count = lambda text, codec=codec: len(codec.encode_ordinary(text))
        else:
            codec = Tokenizer.from_file(str(path))
            count = lambda text, codec=codec: len(codec.encode(text, add_special_tokens=False).ids)
        yield name, entry["family"], count


def summarize(pairs, limit):
    ratios = sorted(flux / rust for flux, rust in pairs)
    aggregate = sum(flux for flux, _ in pairs) / sum(rust for _, rust in pairs)
    middle = median(ratios)
    return {"median": middle, "aggregate": aggregate, "p95": ratios[ceil(0.95 * len(ratios)) - 1],
            "passes": middle <= limit and aggregate <= limit}


def counts(panel, cases, cache):
    bindings, _ = read(REPLAY).split("// probes\n", 1)
    flux_host, rust_host = bindings.split("// host: rust\n", 1)
    flux_host = flux_host.removeprefix("// host: fluxr\n").strip()
    rust_host = rust_host.strip()
    results = {}
    for name, family, count in codecs(panel, cache):
        rows = {}
        for case_id, case in cases.items():
            rows[case_id] = {
                "source": [count(case[language]) for language in LANGUAGES],
                "edit": [count(case[f"{language}_span"][1]) for language in LANGUAGES],
                "source_context": [count(prompt(case_id, case, language, "source")) for language in LANGUAGES],
                "edit_context": [count(prompt(case_id, case, language, "edit")) for language in LANGUAGES],
            }
        results[name] = {
            "family": family, "cases": rows,
            "host_bindings": [count(flux_host), count(rust_host)],
            "source": summarize([row["source"] for row in rows.values()], panel["limit"]),
            "edit": summarize([row["edit"] for row in rows.values()], panel["limit"]),
            "feedback": None, "retries": None,
        }
    return {
        "pair_order": LANGUAGES, "required_passes": panel["required_passes"],
        "encodings": results, "passes": {
            action: sum(result[action]["passes"] for result in results.values()) for action in ACTIONS
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("seal", "replay", "measure"))
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    args = parser.parse_args()
    cases = tomllib.loads(read(CASES))["heldout"]
    panel = tomllib.loads(read(PANEL))
    if len(panel["panel"]) != 6 or len({item["family"] for item in panel["panel"]}) < 4:
        parser.error("Expected six encodings from at least four model families")
    seal_path = args.evidence / "seal.json"
    seal = seal_inputs(cases)
    if args.operation == "seal":
        args.evidence.mkdir(parents=True, exist_ok=True)
        with seal_path.open("x", encoding="utf-8", newline="\n") as handle:
            json.dump(seal, handle, indent=2)
            handle.write("\n")
        return
    if seal != json.loads(read(seal_path)):
        parser.error("Current inputs do not match the sealed corpus")
    replay(cases, args.target_dir.resolve())
    if args.operation == "measure":
        result = counts(panel, cases, args.evidence)
        write(args.evidence / "offline.json", json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
