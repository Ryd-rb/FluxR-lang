"""Seal, replay and count the FXM-04.C declaration and edit-context experiment."""
import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import tempfile
import tomllib

from bench_heldout_tokens import CASES, LANGUAGES, PANEL, ROOT, codecs, read, summarize, write
from bench_typed_tokens import command, digest, replace

REPLAY = ROOT / "benchmarks/context-token-replay.rs"
TRAINING_HASH = "c2b01c0e9a74fa9b4e51640ba14416b13ebce67b5c5e28ae2efd246ee9f1dd3d"


def render(template, body):
    prefix, suffix = template
    return prefix + body + suffix


def initial(template):
    return f"Use this fixed program template. Only BODY changes.\n{render(template, '<BODY>')}"


def request_source(case):
    return f"{case['task']}\nReturn only BODY.\n"


def request_edit(case, language, context):
    anchor = case[f"{language}_span"][0]
    return f"{case['edit']}\nDraft:\n{context}\nReplace {anchor!r}. Return only its replacement.\n"


def seal_inputs(training, templates, cases):
    if sha256(training.read_bytes()).hexdigest() != TRAINING_HASH:
        raise ValueError("Unexpected training freeze")
    for path, expected in json.loads(read(training))["files"].items():
        if digest(read(ROOT / path)) != expected:
            raise ValueError(f"Frozen training input changed: {path}")
    tools = Path(__file__).resolve().parent
    files = (CASES, PANEL, REPLAY, Path(__file__).resolve(),
             tools / "bench_heldout_tokens.py", tools / "bench_typed_tokens.py")
    prompts = {}
    for language in LANGUAGES:
        template = templates[language]
        setup = initial(template)
        prompts[f"{language}.initial"] = digest(setup)
        for key, case in cases.items():
            body = case[language]
            base = f"{language}.{key}"
            prompts[f"{base}.source_cold"] = digest(setup + request_source(case))
            prompts[f"{base}.source_scoped"] = digest(request_source(case))
            prompts[f"{base}.edit_cold"] = digest(request_edit(case, language, render(template, body)))
            prompts[f"{base}.edit_scoped"] = digest(request_edit(case, language, body))
    return {"training": TRAINING_HASH,
            "files": {p.relative_to(ROOT).as_posix(): digest(read(p)) for p in files},
            "prompts": prompts}


def apply_edit(template, body, revision, span):
    if digest(render(template, body)) != revision:
        raise ValueError("stale draft")
    return replace(body, span)


def rejects_edit(template, body, revision, span, message):
    try:
        apply_edit(template, body, revision, span)
    except ValueError as error:
        if message in str(error):
            return
        raise
    raise AssertionError(f"Edit should fail: {message}")


def rust_entry(key):
    return f"|lo, hi, d| {key}::calc({key}::Reading {{ lo, hi }}, d)"


def repair(case, templates, folder, binary):
    results = {}
    for language in LANGUAGES:
        template = templates[language]
        fault = case[f"{language}_fault"]
        body = case[language]
        failed_body = replace(body, fault)
        failed_source = render(template, failed_body)
        revision = digest(failed_source)
        anchor, fix = fault[::-1]
        # The bad expression is a variable also used elsewhere in the body.
        # Anchor the repair to the complete condition site in each language.
        condition = ("(? ", " (") if language == "fluxr" else ("if ", " {")
        anchor, fix = (condition[0] + value + condition[1] for value in (anchor, fix))
        if apply_edit(template, failed_body, revision, (anchor, fix)) != body:
            raise ValueError("Repair did not restore the sealed body")
        if language == "fluxr":
            path = "fault.fluxr"
            write(folder / path, failed_source)
            diagnostic = command([str(binary), path], folder).stdout.strip()
        else:
            path = "fault.rs"
            write(folder / path, failed_source)
            error = command(["rustc", "--edition=2024", "--crate-type=lib", "--emit=metadata",
                             "--error-format=short", "--color=never", path], folder, ok=False)
            diagnostic = "\n".join(line for line in error.stderr.splitlines() if line.startswith(f"{path}:"))
        if not all(kind in diagnostic.casefold() for kind in ("bool", "i64")):
            raise ValueError(f"Expected a condition type mismatch: {diagnostic}")
        feedback = (f"Compiler feedback:\n{diagnostic}\n"
                    f"Replace {anchor!r}. Return only its replacement.\n")
        results[language] = {
            "failed_body": failed_body, "diagnostic": diagnostic, "anchor": anchor, "replacement": fix,
            "cold_context": initial(template) + request_source(case) + failed_source + "\n" + feedback,
            "scoped_context": failed_body + "\n" + feedback,
        }
    return results


def replay(templates, cases, target):
    with tempfile.TemporaryDirectory(prefix="fluxr-context-") as temp:
        folder = Path(temp)
        modules, calls = [], []
        for key, case in cases.items():
            for action in ("source", "edit"):
                name = f"{key}_{action}"
                for language, template in templates.items():
                    body = case[language]
                    original = render(template, body)
                    if action == "edit":
                        span = case[f"{language}_span"]
                        revision = digest(original)
                        rejects_edit(template, body + " ", revision, span, "stale draft")
                        rejects_edit([template[0] + " ", template[1]], body, revision, span, "stale draft")
                        rejects_edit(template, body, revision, ("<missing>", ""), "unique anchor")
                        rejects_edit(template, body + body, digest(render(template, body + body)), span, "unique anchor")
                        body = apply_edit(template, body, revision, span)
                        assert render(template, body) == replace(original, span)
                    source = render(template, body)
                    if language == "fluxr":
                        write(folder / f"{name}.fluxr", source)
                    else:
                        modules.append(f"mod {name} {{\n{source}}}\n")
                calls.append(f'let {name} = compare("{name}", {rust_entry(name)});')
            calls.append(f"assert_ne!({key}_source, {key}_edit);")
        write(folder / "rust-modules.rs", "".join(modules) + "fn replay() {\n" + "\n".join(calls) + "\n}\n")
        build = target / "release"
        binary = folder / ("replay.exe" if os.name == "nt" else "replay")
        command(["rustc", "--edition=2024", "-C", "opt-level=3",
                 "--extern", f"fluxr={build / 'libfluxr.rlib'}", "-L", f"dependency={build / 'deps'}",
                 "-o", str(binary), str(REPLAY)], folder, FLUXR_CONTEXT_REPLAY=temp)
        log = command([str(binary)], folder, FLUXR_TELEMETRY="off").stdout
        repairs = {key: repair(case, templates, folder, binary)
                   for key, case in cases.items() if "fluxr_fault" in case}
        return log, repairs


def counts(panel, templates, cases, cache, repairs):
    adapters, _ = read(REPLAY).split("// probes\n", 1)
    flux_host, rust_host = adapters.removeprefix("// adapter: fluxr\n").split("// adapter: rust\n")
    results = {}
    for name, family, count in codecs(panel, cache):
        def pair(metric):
            return [metric(lang) for lang in LANGUAGES]

        rows = {}
        for key, case in cases.items():
            rows[key] = {
                "source": pair(lambda lang: count(render(templates[lang], case[lang]))),
                "body": pair(lambda lang: count(case[lang])),
                "edit": pair(lambda lang: count(case[f"{lang}_span"][1])),
                "anchor": pair(lambda lang: count(case[f"{lang}_span"][0])),
                "source_cold": pair(lambda lang: count(initial(templates[lang]) + request_source(case))),
                "source_scoped": pair(lambda lang: count(request_source(case))),
                "edit_cold": pair(lambda lang: count(request_edit(case, lang, render(templates[lang], case[lang])))),
                "edit_scoped": pair(lambda lang: count(request_edit(case, lang, case[lang]))),
                "rust_entries": sum(count(rust_entry(f"{key}_{action}")) for action in ("source", "edit")),
            }
        setup = pair(lambda lang: count(initial(templates[lang])))
        phases = ("source", "edit")
        results[name] = {
            "family": family, "cases": rows,
            "host_bindings": [count(flux_host.strip()), count(rust_host.strip())],
            "shared_definition": pair(lambda lang: sum(map(count, templates[lang]))),
            "initial_context": setup,
            "amortized_context_per_task": [
                (base + sum(row["source_scoped"][i] + row["edit_scoped"][i] for row in rows.values()))
                / len(rows) for i, base in enumerate(setup)
            ],
            "cold_context_per_task": [
                sum(row["source_cold"][i] + row["edit_cold"][i] for row in rows.values()) / len(rows)
                for i in range(len(LANGUAGES))
            ],
            "scripted_repairs": {
                key: {field: pair(lambda lang: count(row[lang][field])) for field in row["fluxr"]}
                for key, row in repairs.items()
            },
            "ratios": {field: summarize([row[field] for row in rows.values()], panel["limit"])
                       for field in (*phases, "body")},
        }
    return {"pair_order": LANGUAGES, "required_passes": panel["required_passes"], "encodings": results,
            "passes": {field: sum(row["ratios"][field]["passes"] for row in results.values())
                       for field in ("source", "body", "edit")}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("seal", "measure"))
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--training", required=True, type=Path)
    parser.add_argument("--cache", required=True, type=Path)
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    args = parser.parse_args()
    data = tomllib.loads(read(CASES))["reuse"]
    templates, cases = data["template"], data["cases"]
    seal = seal_inputs(args.training, templates, cases)
    path = args.evidence / "seal.json"
    if args.operation == "seal":
        args.evidence.mkdir(parents=True, exist_ok=True)
        with path.open("x", encoding="utf-8", newline="\n") as handle:
            handle.write(json.dumps(seal, indent=2) + "\n")
        return
    if seal != json.loads(read(path)):
        parser.error("Current inputs do not match the sealed corpus")
    log, repairs = replay(templates, cases, args.target_dir.resolve())
    panel = tomllib.loads(read(PANEL))
    result = counts(panel, templates, cases, args.cache, repairs)
    write(args.evidence / "replay.txt", log)
    write(args.evidence / "offline.json", json.dumps(result, indent=2) + "\n")
    write(args.evidence / "scripted-repairs.json", json.dumps(repairs, indent=2) + "\n")
    print(log, end="")


if __name__ == "__main__":
    main()
