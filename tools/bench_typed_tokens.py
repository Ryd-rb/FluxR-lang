"""Seal, replay and count the FXM-04.B token benchmark. Python 3.11+."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

from bench_heldout_tokens import CASES, LANGUAGES, PANEL, ROOT, codecs, read, summarize, write

REPLAY = ROOT / "benchmarks/typed-token-replay.rs"
TRAINING_HASH = "95e827180aaf6674ffe87ad40b588531144f308a8505cf85b18b36424ab9c091"
REFERENCE = {
    "fluxr": (
        "Start with v1 or v2, then (pub calc (arg:type ...) result-type = tail). "
        "A tail is an expression or (? bool then-tail else-tail); only the selected tail runs. "
        "Use prefix checked integer operators and the u suffix for u64 literals. "
        "Lists use [type], (at list index), (list type values...). "
        "Records use (rec Name field:type ...) and value.field. A method is "
        "(fn Name.method (self:Name ...) result-type = tail), called as (value.method args...). "
        "Enums use (enum Name Variant:type Empty), #Name, (is value Variant), (payload value Variant)."
    ),
    "rust": "Use Rust 2024 and the standard library. Use Option for operations that can trap.",
}


def digest(text):
    return sha256(text.encode()).hexdigest()


def source(case, language):
    return "".join(case[language])


def replace(original, pair):
    old, new = pair
    if original.count(old) != 1:
        raise ValueError(f"Expected a unique anchor: {old!r}")
    return original.replace(old, new)


def prompt(case, language, action):
    reference = REFERENCE[language]
    task = case["task"]
    if action == "source":
        return f"{reference}\n{task}\nReturn only the complete source.\n"
    anchor = case[f"{language}_span"][0]
    return (
        f"{reference}\n{task}\nOriginal source:\n{source(case, language)}\n"
        f"Change: {case['edit']}\nReplace the unique span {anchor!r}.\n"
        "Return only the replacement span.\n"
    )


def seal_inputs(cases, training):
    if sha256(training.read_bytes()).hexdigest() != TRAINING_HASH:
        raise ValueError("Unexpected training freeze")
    frozen = json.loads(read(training))
    for path, expected in frozen["files"].items():
        if digest(read(ROOT / path)) != expected:
            raise ValueError(f"Frozen form or panel changed: {path}")
    files = (CASES, PANEL, REPLAY, Path(__file__).resolve(),
             Path(__file__).with_name("bench_heldout_tokens.py").resolve())
    return {
        "training": TRAINING_HASH,
        "files": {p.relative_to(ROOT).as_posix(): digest(read(p)) for p in files},
        "prompts": {f"{key}.{language}.{action}": digest(prompt(case, language, action))
                    for key, case in cases.items() for language in LANGUAGES
                    for action in ("source", "edit")},
    }


def command(args, directory, *, ok=True, **env):
    result = subprocess.run(
        args, cwd=directory, env={**os.environ, **env}, capture_output=True,
        encoding="utf-8", timeout=90, errors="replace",
    )
    if (result.returncode == 0) != ok:
        raise RuntimeError(f"{args[0]} exited {result.returncode}\n{result.stdout}{result.stderr}")
    return result


def repair_cases(cases, directory, binary):
    results = {}
    for key, case in cases.items():
        if "fluxr_fault" not in case:
            continue
        for language in LANGUAGES:
            original = source(case, language)
            pair = case[f"{language}_fault"]
            failed = replace(original, pair)
            if language == "fluxr":
                path = "fault.fluxr"
                write(directory / path, failed)
                diagnostic = command([str(binary), path], directory).stdout.strip()
            else:
                path = "fault.rs"
                write(directory / path, failed)
                result = command(
                    ["rustc", "--edition=2024", "--crate-type=lib", "--emit=metadata",
                     "--error-format=short", "--color=never", path], directory, ok=False,
                )
                diagnostic = "\n".join(line for line in result.stderr.splitlines()
                                       if line.startswith(f"{path}:"))
            if not all(kind in diagnostic.casefold() for kind in ("bool", "u64")):
                raise ValueError(f"Expected condition type mismatch: {diagnostic}")
            anchor, fix = pair[::-1]
            assert replace(failed, (anchor, fix)) == original
            results[key, language] = {
                "failed_source": failed, "diagnostic": diagnostic, "anchor": anchor, "replacement": fix,
                "prompt": (f"{prompt(case, language, 'source')}\nRejected source:\n{failed}\n"
                           f"Compiler feedback:\n{diagnostic}\nReplace the unique span {anchor!r}. "
                           "Return only the replacement span.\n"),
            }
    return results


def rust_entry(name, module):
    calc = f"{module}::calc"
    if name == "tariff":
        return f"|per, base, n| {calc}({module}::Rate {{ per, base }}, n)"
    if name == "ack":
        return f"|n| {calc}(n.map_or({module}::Ack::Pending, {module}::Ack::Ok))"
    return calc


def replay(cases, target):
    with tempfile.TemporaryDirectory(prefix="fluxr-typed-") as temp:
        folder = Path(temp)
        modules, calls, keys = [], [], []
        for name, case in cases.items():
            for action in ("source", "edit"):
                key = f"{name}_{action}"
                keys.append(key)
                bodies = {lang: source(case, lang) for lang in LANGUAGES}
                if action == "edit":
                    bodies = {lang: replace(body, case[f"{lang}_span"]) for lang, body in bodies.items()}
                write(folder / f"{key}.fluxr", bodies["fluxr"])
                modules.append(f"mod {key} {{\n{bodies['rust']}}}\n")
                rust = rust_entry(name, key)
                calls.append(f'compare("{key}", |m| {name}(m, {rust}));')
        write(folder / "rust-modules.rs", "".join(modules) + "\nfn replay() {\n" + "\n".join(calls) + "\n}\n")
        build = target / "release"
        binary = folder / ("replay.exe" if os.name == "nt" else "replay")
        command(["rustc", "--edition=2024", "-C", "opt-level=3",
                 "--extern", f"fluxr={build / 'libfluxr.rlib'}", "-L", f"dependency={build / 'deps'}",
                 "-o", str(binary), str(REPLAY)], folder, FLUXR_TYPED_HOLDOUT=temp)
        log = command([str(binary)], folder, FLUXR_TELEMETRY="off").stdout
        print(log, end="")
        expanded = {key: read(folder / f"{key}.expanded") for key in keys}
        return log, expanded, repair_cases(cases, folder, binary)


def counts(panel, cases, cache, expanded, repairs):
    adapters, _ = read(REPLAY).split("// probes\n", 1)
    flux_host, rust_host = adapters.removeprefix("// adapter: fluxr\n").split("// adapter: rust\n")
    results = {}
    for name, family, count in codecs(panel, cache):
        rows = {}
        for key, case in cases.items():
            def pair(metric):
                return [metric(lang) for lang in LANGUAGES]

            rows[key] = {
                "source": pair(lambda lang: count(source(case, lang))),
                "declarations": pair(lambda lang: sum(map(count, case[lang][::2]))),
                "body": pair(lambda lang: sum(map(count, case[lang][1::2]))),
                "expanded_fluxr": count(expanded[f"{key}_source"]),
                "edit": pair(lambda lang: count(case[f"{lang}_span"][1])),
                "edit_anchor": pair(lambda lang: count(case[f"{lang}_span"][0])),
                "edit_context": pair(lambda lang: count(prompt(case, lang, "edit"))),
                "source_context": pair(lambda lang: count(prompt(case, lang, "source"))),
                "task": count(case["task"]),
                "rust_replay_entry": count(rust_entry(key, f"{key}_source")),
            }
            if (key, "fluxr") in repairs:
                rows[key]["scripted_repair"] = {
                    field: pair(lambda lang: count(repairs[key, lang][field]))
                    for field in ("failed_source", "diagnostic", "anchor", "replacement", "prompt")
                }
        results[name] = {
            "family": family, "cases": rows, "host_bindings": [count(flux_host.strip()), count(rust_host.strip())],
            "reference": [count(REFERENCE[lang]) for lang in LANGUAGES],
            "source": summarize([row["source"] for row in rows.values()], panel["limit"]),
            "edit": summarize([row["edit"] for row in rows.values()], panel["limit"]),
            "versus_expanded": summarize([(row["source"][0], row["expanded_fluxr"])
                                         for row in rows.values()], panel["limit"]),
        }
    return {
        "pair_order": LANGUAGES, "required_passes": panel["required_passes"], "encodings": results,
        "passes": {action: sum(result[action]["passes"] for result in results.values())
                   for action in ("source", "edit")},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("seal", "measure"))
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--training", required=True, type=Path)
    parser.add_argument("--cache", type=Path)
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    args = parser.parse_args()
    cases = tomllib.loads(read(CASES))["typed_holdout"]
    seal = seal_inputs(cases, args.training)
    path = args.evidence / "seal.json"
    if args.operation == "seal":
        args.evidence.mkdir(parents=True, exist_ok=True)
        with path.open("x", encoding="utf-8", newline="\n") as handle:
            handle.write(json.dumps(seal, indent=2) + "\n")
        return
    if seal != json.loads(read(path)):
        parser.error("Current inputs do not match the sealed corpus")
    log, expanded, repairs = replay(cases, args.target_dir.resolve())
    panel = tomllib.loads(read(PANEL))
    result = counts(panel, cases, args.cache or args.evidence, expanded, repairs)
    write(args.evidence / "replay.txt", log)
    write(args.evidence / "offline.json", json.dumps(result, indent=2) + "\n")
    write(args.evidence / "scripted-repairs.json",
          json.dumps({f"{key}.{lang}": row for (key, lang), row in repairs.items()}, indent=2) + "\n")


if __name__ == "__main__":
    main()
