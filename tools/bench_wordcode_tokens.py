#!/usr/bin/env python3
"""Measure authored WK1 sources and explicit setup/script scenarios, offline.

This never compiles or executes candidates, runs models, verifies behavior, or
admits frozen replay. A completed measurement can legitimately fail the retained
<=10% Rust source-token target. All output and cache paths must be external.
"""
from __future__ import annotations

import argparse
import ast
from fractions import Fraction
import gc
import json
from pathlib import Path
import platform
import re
import sys
import time
import tomllib

if sys.platform != "win32":
    import resource

sys.dont_write_bytecode = True
import wordcode_token_audit as audit

ROOT = Path(__file__).resolve().parents[1]
ARMS = ("wordcode", "expanded_agent", "rust_full", "rust_shared_call")
PRACTICAL = ("WK14-affine", "WK14-polynomial", "WK14-repeat", "WK14-walk")
SYNTHETIC = ("WK14-forest",)
CASE_IDS = PRACTICAL + SYNTHETIC
DICT_PREFIX = b"FluxR WK1 dictionary v1\0"
TARGET = Fraction(1, 10)
DESCRIPTIONS = {
    "WK14-affine": "In calc(x,y,z), return checked signed-i64 x*y, then checked addition of z.",
    "WK14-polynomial": "In calc(x,y,z), evaluate checked signed-i64 x*x, y*2, their sum, then add z, in that order.",
    "WK14-repeat": "In calc(n,x), increment signed-i64 x by one n times with checked addition; n<=0 performs no iterations.",
    "WK14-walk": "In calc(x,n,s), while signed-i64 x<n, add s to x with checked arithmetic; return x.",
    "WK14-forest": "In calc(x), return signed-i64 x to the power 256 with arithmetic overflow reported.",
}
RUST_PROTOCOL = (
    "Use stable Rust 2024 and the standard library. Export calc with signed-i64 "
    "parameters in the task's stated order and return Option<i64>. Some is the "
    "scalar result and None is arithmetic overflow. For the shared-library arm, "
    "the retrieved words module is installed and its public functions are available. "
    "Return the complete calc declaration.\n"
)


def require_keys(value, keys, label):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError("malformed or unknown fields: " + label)


def require_int(value, label, low=0, high=2**64 - 1):
    if type(value) is not int or not low <= value <= high:
        raise ValueError("bounded integer required: " + label)
    return value


def exact_ratio(numerator, denominator):
    require_int(numerator, "token numerator")
    require_int(denominator, "token denominator", 1)
    return Fraction(numerator, denominator)


def ratio_record(value):
    return {"numerator": value.numerator, "denominator": value.denominator,
            "decimal_display": float(value), "passes_1_over_10": value <= TARGET}


def summarize(pairs):
    """Integer/rational decisions; floats are display values only."""
    if not isinstance(pairs, list) or not pairs:
        raise ValueError("nonempty token-count population required")
    if any(not isinstance(pair, (tuple, list)) or len(pair) != 2 for pair in pairs):
        raise ValueError("numerator/denominator pairs required")
    ratios = sorted(exact_ratio(a, b) for a, b in pairs)
    n = len(ratios)
    middle = ratios[n // 2] if n % 2 else (ratios[n // 2 - 1] + ratios[n // 2]) / 2
    aggregate = exact_ratio(sum(a for a, _ in pairs), sum(b for _, b in pairs))
    p95 = ratios[(95 * n + 99) // 100 - 1]
    return {"cases": n, "median": ratio_record(middle), "aggregate": ratio_record(aggregate),
            "p95_nearest_rank": ratio_record(p95), "per_case_passes": sum(v <= TARGET for v in ratios),
            "passes_median_and_aggregate": middle <= TARGET and aggregate <= TARGET}


def canonical_dictionary(text):
    """Reproduce declared serde field order for byte/hash accounting only.

    This does not type-check, resolve definitions, prove effects or authorize the
    dictionary. Those remain the real compiler's separate acceptance gates.
    """
    if not isinstance(text, str) or not 1 <= len(text.encode()) <= 65_536:
        raise ValueError("bounded nonempty dictionary document required")
    document = audit.parse_json(text)
    require_keys(document, ("version", "definitions"), "dictionary")
    if type(document["version"]) is not int or document["version"] != 1:
        raise ValueError("dictionary version must be 1")
    definitions = document["definitions"]
    if not isinstance(definitions, list) or not 1 <= len(definitions) <= 64:
        raise ValueError("1..64 dictionary definitions required")
    nodes = 0

    def template(body, depth):
        nonlocal nodes
        nodes += 1
        if nodes > 8192 or depth >= 32 or not isinstance(body, dict):
            raise ValueError("dictionary accounting tree exceeds shape bounds")
        fields = {"param": ("index",), "int": ("value",), "uint": ("value",),
                  "float": ("bits",), "bool": ("value",),
                  "binary": ("kind", "left", "right"), "unary": ("kind", "value"),
                  "use": ("word", "args")}
        op = body.get("op")
        if not isinstance(op, str) or op not in fields:
            raise ValueError("unknown dictionary accounting node")
        require_keys(body, ("op", *fields[op]), "dictionary " + op)
        result = {"op": op}
        if op == "binary":
            if body["kind"] not in ("add", "sub", "mul", "div", "rem", "eq", "lt", "le", "and", "or"):
                raise ValueError("unknown binary operation")
            result.update(kind=body["kind"], left=template(body["left"], depth + 1),
                          right=template(body["right"], depth + 1))
        elif op == "unary":
            if body["kind"] not in ("not", "abs_f32", "u64_to_f32", "i64_to_f32", "f32_to_i64", "sqrt_f32"):
                raise ValueError("unknown unary operation")
            result.update(kind=body["kind"], value=template(body["value"], depth + 1))
        elif op == "use":
            if (not isinstance(body["word"], str) or not re.fullmatch(r"[a-z]{1,16}", body["word"])
                    or not isinstance(body["args"], list) or not 1 <= len(body["args"]) <= 8):
                raise ValueError("malformed dictionary use")
            result.update(word=body["word"], args=[template(arg, depth + 1) for arg in body["args"]])
        elif op == "bool":
            if type(body["value"]) is not bool:
                raise ValueError("dictionary bool must be boolean")
            result["value"] = body["value"]
        else:
            key = fields[op][0]
            low, high = {"param": (0, 7), "int": (-2**63, 2**63 - 1),
                         "uint": (0, 2**64 - 1), "float": (0, 2**32 - 1)}[op]
            result[key] = require_int(body[key], "dictionary " + op, low, high)
        return result

    ordered, words = [], set()
    for definition in definitions:
        require_keys(definition, ("word", "params", "result", "body"), "definition")
        word = definition["word"]
        types = ("int", "uint", "float", "bool")
        params = definition["params"]
        if (not isinstance(word, str) or not re.fullmatch(r"[a-z]{1,16}", word) or word in words
                or not isinstance(params, list) or not 1 <= len(params) <= 8
                or any(item not in types for item in params) or definition["result"] not in types):
            raise ValueError("malformed or duplicate dictionary declaration")
        words.add(word)
        ordered.append({"word": word, "params": params, "result": definition["result"],
                        "body": template(definition["body"], 0)})
    canonical = json.dumps({"version": 1, "definitions": ordered}, ensure_ascii=False,
                           separators=(",", ":"), allow_nan=False)
    if len(canonical.encode()) > 65_536:
        raise ValueError("canonical dictionary exceeds byte bound")
    return canonical, audit.digest(DICT_PREFIX + canonical.encode())


def validate_dictionary_observation(observation):
    require_keys(observation, ("profile", "binding", "dictionary_sha256", "dictionary_json"),
                 "worker dictionary observation")
    if observation["profile"] != "wk1" or observation["binding"] != "core":
        raise ValueError("worker WK1 core dictionary observation required")
    canonical, sha = canonical_dictionary(observation["dictionary_json"])
    if observation["dictionary_json"] != canonical or observation["dictionary_sha256"] != sha:
        raise ValueError("observed dictionary bytes, canonical order or immutable identity differ")
    return canonical, sha


def validate_catalog(catalog):
    require_keys(catalog, ("schema", "scope", "rust_shared_library", "protocol", "cases"), "catalog")
    if catalog["schema"] != "fluxr-wordcode-v14-catalog-v1" or not isinstance(catalog["scope"], str):
        raise ValueError("authored v14 catalog schema/scope required")
    cases = catalog["cases"]
    if (not isinstance(cases, list) or len(cases) != 5
            or any(not isinstance(row, dict) for row in cases)
            or tuple(row.get("id") for row in cases) != CASE_IDS):
        raise ValueError("all four practical cases and synthetic forest required, in catalog order")
    for case in cases:
        fields = ("id", "class", *ARMS, "arguments", "expected")
        if case["id"] in SYNTHETIC:
            fields += ("dictionary", "dictionary_sha256")
        require_keys(case, fields, "case " + case["id"])
        expected_class = "authored_practical_microcase" if case["id"] in PRACTICAL else "synthetic_density_stress"
        if case["class"] != expected_class:
            raise ValueError("practical/synthetic classification cannot change")
        if not isinstance(case["arguments"], list) or not 1 <= len(case["arguments"]) <= 8:
            raise ValueError("bounded declared example arguments required")
        for argument in (*case["arguments"], case["expected"]):
            require_int(argument, "declared example value", -2**63, 2**63 - 1)
        paths = [case[arm] for arm in ARMS]
        if any(not isinstance(p, str) or not p for p in paths) or len(set(paths)) != len(paths):
            raise ValueError("four distinct authored source paths required")


class Inputs:
    def __init__(self):
        self.files = {}
        self.payloads = {}
        self.total = 0

    def file(self, label, path, *, source=False, limit=2 * 1024 * 1024):
        path = Path(path)
        if source:
            if path.is_absolute() or ".." in path.parts:
                raise ValueError("relative source path required")
            path = ROOT / path
        path = path.resolve(strict=True)
        if source and not path.is_relative_to(ROOT):
            raise ValueError("source path escapes checkout")
        raw = audit.read_bounded(path, limit)
        self.total += len(raw)
        if self.total > 8 * 1024 * 1024:
            raise ValueError("measurement inputs exceed aggregate 8 MiB")
        self.files[label] = {"path": str(path), "bytes": len(raw), "sha256": audit.digest(raw)}
        return raw

    def text(self, key, text, kind, origin=None):
        if key in self.payloads or not isinstance(text, str) or not text:
            raise ValueError("unique nonempty text identity required")
        self.payloads[key] = {"text": text, **audit.identity(text), "kind": kind, "origin": origin}
        return key

    def source_text(self, key, relative, kind):
        raw = self.file(key, relative, source=True, limit=256 * 1024)
        self.text(key, raw.decode("utf-8"), kind, relative)
        return raw.decode("utf-8")

    def verify_unchanged(self):
        for label, observed in self.files.items():
            if audit.hash_file(Path(observed["path"]), 2 * 1024 * 1024) != {
                    key: observed[key] for key in ("bytes", "sha256")}:
                raise ValueError("measurement subject changed during encoding: " + label)


def load_inputs(args):
    inputs = Inputs()
    catalog_raw = inputs.file("catalog", args.catalog)
    catalog = tomllib.loads(catalog_raw.decode("utf-8"))
    validate_catalog(catalog)
    panel, panel_raw = audit.read_panel(args.panel)
    inputs.file("panel", args.panel)
    if inputs.files["panel"]["sha256"] != audit.digest(panel_raw):
        raise ValueError("tokenizer panel changed during input snapshot")
    observation_raw = inputs.file("dictionary_observation", args.dictionary_observation)
    observation = audit.parse_json(observation_raw)
    core, core_sha = validate_dictionary_observation(observation)
    inputs.text("core_dictionary", core, "actual_worker_dictionary_json", "dictionary_observation.dictionary_json")
    inputs.source_text("wk1_protocol", catalog["protocol"], "authored_WK1_protocol_retrieval")
    inputs.source_text("rust_library", catalog["rust_shared_library"], "whole_shared_Rust_library_retrieval")
    card_source = inputs.file("inherited_fa1_card_source", "tools/bench_agent_syntax.py", source=True)
    cards = [ast.literal_eval(node.value) for node in ast.parse(card_source).body
             if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "AGENT_CARD"
                                                    for t in node.targets)]
    if len(cards) != 1 or not isinstance(cards[0], str):
        raise ValueError("exact inherited FA1 card not found")
    inputs.text("fa1_protocol", cards[0], "inherited_FA1_reference_card", "tools/bench_agent_syntax.py:AGENT_CARD")
    inputs.text("rust_protocol", RUST_PROTOCOL, "authored_Rust_scenario_protocol", "this_tool:RUST_PROTOCOL")
    core_binding = {"profile": "wk1", "binding": "core", "dictionary_sha256": core_sha}
    inputs.text("core_binding", json.dumps(core_binding, separators=(",", ":")), "dictionary_identity_setup")
    rows = []
    for case in catalog["cases"]:
        row = {**case, "payloads": {}, "dictionary_payload": "core_dictionary", "binding_payload": "core_binding"}
        if "dictionary" in case:
            raw = inputs.source_text(case["id"] + "/dictionary", case["dictionary"], "custom_dictionary_exact_retrieval")
            canonical, sha = canonical_dictionary(raw)
            if case["dictionary_sha256"] != sha:
                raise ValueError("custom dictionary catalog identity mismatch")
            row["dictionary_payload"] = case["id"] + "/dictionary"
            row["binding_payload"] = case["id"] + "/binding"
            row["dictionary_canonical_sha256"] = audit.digest(canonical.encode())
            row["dictionary_sha256"] = sha
            inputs.text(row["binding_payload"], json.dumps({"profile": "wk1", "binding": sha,
                        "dictionary_sha256": sha}, separators=(",", ":")), "custom_dictionary_identity_setup")
        else:
            row["dictionary_sha256"] = core_sha
        for arm in ARMS:
            key = case["id"] + "/" + arm
            text = inputs.source_text(key, case[arm], arm)
            if arm == "wordcode":
                binding = case.get("dictionary_sha256", "core")
                if text.split()[:3] != ["wk1", binding, "v2"]:
                    raise ValueError("WK1 source profile/dictionary binding mismatch")
            if arm == "expanded_agent" and text.split()[:2] != ["fa1", "v2"]:
                raise ValueError("expanded source must retain its FA1 v2 profile")
            row["payloads"][arm] = key
        task = (DESCRIPTIONS[case["id"]] + "\nDeclared example: arguments="
                + json.dumps(case["arguments"], separators=(",", ":")) + ", result=" + str(case["expected"])
                + ".\nThe arithmetic result/error is the Rust comparison boundary; VM fuel, cells, frames "
                "and exact first-trap behavior require the separate FA1 differential. Loop execution "
                "is admitted only on bounded terminating fixture inputs. Return only the complete source.\n")
        row["task_payload"] = inputs.text(case["id"] + "/task", task, "scripted_task_not_live_prompt")
        rows.append(row)
    for label, path in (("evaluator", "tools/bench_wordcode_tokens.py"),
                        ("offline_loader", "tools/wordcode_token_audit.py"),
                        ("wordcode_implementation", "src/structured/wordcode.rs"),
                        ("worker_source", "examples/wordcode_worker.rs"), ("cargo_lock", "Cargo.lock")):
        inputs.file(label, path, source=True)
    return inputs, catalog, panel, audit.digest(panel_raw), rows


def scripted_messages(cases, arm):
    """An explicit scenario, not a retained agent conversation or accepted task."""
    if arm not in ARMS or not cases:
        raise ValueError("known arm and nonempty scenario required")
    messages = []

    def add(event, direction, payload):
        messages.append({"event": event, "direction": direction, "payload": payload})

    if arm in ("wordcode", "expanded_agent"):
        add("language_setup", "input", "fa1_protocol")
        if arm == "wordcode":
            add("wordcode_setup", "input", "wk1_protocol")
    else:
        add("language_setup", "input", "rust_protocol")
        if arm == "rust_shared_call":
            add("library_retrieval", "input", "rust_library")
    retrieved = set()
    for case in cases:
        if arm == "wordcode" and case["dictionary_payload"] not in retrieved:
            add("dictionary_binding", "input", case["binding_payload"])
            add("dictionary_retrieval", "input", case["dictionary_payload"])
            retrieved.add(case["dictionary_payload"])
        add("task", "input", case["task_payload"])
        add("authored_source", "output", case["payloads"][arm])
    return messages


def count_transcript(encode, messages, payloads, measured):
    texts = [payloads[m["payload"]]["text"] for m in messages]
    # These are complete strings, never a sum of fragment estimates.
    body = "".join(texts)
    transcript = "".join(f"[{m['event']}:{m['direction']}]\n{text}\n"
                         for m, text in zip(messages, texts, strict=True))
    packet = json.dumps([{**m, "text": text} for m, text in zip(messages, texts, strict=True)],
                        ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    per_message = [measured[m["payload"]]["tokens"] for m in messages]
    return {"messages": messages, "message_body_token_sum": sum(per_message),
            "input_body_token_sum": sum(n for m, n in zip(messages, per_message, strict=True) if m["direction"] == "input"),
            "output_body_token_sum": sum(n for m, n in zip(messages, per_message, strict=True) if m["direction"] == "output"),
            "joined_body_string": audit.count_with_json(encode, body),
            "joined_labeled_transcript": audit.count_with_json(encode, transcript),
            "complete_json_message_array": audit.count_text(encode, packet),
            "classification": "scripted_static_scenario_not_observed_model_episode",
            "model_attempts": None, "accepted_task_success": None, "cached_input_tokens": None,
            "uncached_input_tokens": None, "repairs_reviews_tools_handoffs": None,
            "provider_billing_or_chat_framing": False}


def population_summary(rows, ids, measured):
    selected = [row for row in rows if row["id"] in ids]
    if tuple(row["id"] for row in selected) != ids:
        raise ValueError("population missing or reordered")
    result = {}
    for comparator in ("rust_full", "rust_shared_call", "expanded_agent"):
        pairs = [(measured[row["payloads"]["wordcode"]]["tokens"],
                  measured[row["payloads"][comparator]]["tokens"]) for row in selected]
        result["wordcode_vs_" + comparator] = summarize(pairs)
    result["both_fair_Rust_baselines_pass"] = all(result["wordcode_vs_" + arm]["passes_median_and_aggregate"]
                                                   for arm in ("rust_full", "rust_shared_call"))
    return result


def measure_profile(codec, inputs, rows):
    started = time.perf_counter_ns()
    measured = {key: audit.count_with_json(codec.encode, row["text"]) for key, row in inputs.payloads.items()}
    populations = {"practical": PRACTICAL, "synthetic": SYNTHETIC, "combined_descriptive_only": CASE_IDS}
    cases = []
    for row in rows:
        counts = {arm: measured[key]["tokens"] for arm, key in row["payloads"].items()}
        cases.append({"id": row["id"], "class": row["class"], "payloads": row["payloads"],
                      "tokens": counts, "ratios": {"wordcode_vs_" + arm: ratio_record(exact_ratio(counts["wordcode"], counts[arm]))
                                                    for arm in ARMS[1:]},
                      "json_quoted_ratios": {"wordcode_vs_" + arm: ratio_record(exact_ratio(
                          measured[row["payloads"]["wordcode"]]["json_quoted_string"]["tokens"],
                          measured[row["payloads"][arm]]["json_quoted_string"]["tokens"])) for arm in ARMS[1:]}})
    cold = {row["id"]: {arm: count_transcript(codec.encode, scripted_messages([row], arm), inputs.payloads, measured)
                        for arm in ARMS} for row in rows}
    reused = {name: {arm: count_transcript(codec.encode, scripted_messages(
        [row for row in rows if row["id"] in ids], arm), inputs.payloads, measured) for arm in ARMS}
              for name, ids in populations.items()}
    scenario_ratios = {name: {"wordcode_vs_" + arm: ratio_record(exact_ratio(
        session["wordcode"]["joined_labeled_transcript"]["tokens"],
        session[arm]["joined_labeled_transcript"]["tokens"])) for arm in ("rust_full", "rust_shared_call")}
                       for name, session in reused.items()}
    return {"tokenizer": codec.metadata, "strings": measured, "cases": cases,
            "populations": {name: population_summary(rows, ids, measured) for name, ids in populations.items()},
            "scripted_cold_cases": cold, "scripted_reuse_sessions": reused,
            "scripted_reuse_whole_transcript_ratios": scenario_ratios,
            "measurement_ns": time.perf_counter_ns() - started}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=ROOT / "benchmarks/wordcode-v14.toml")
    parser.add_argument("--panel", type=Path, default=ROOT / "benchmarks/token-panel.toml")
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--dictionary-observation", type=Path, required=True,
                        help="exact JSON stdout of this candidate's wordcode_worker dictionary")
    parser.add_argument("--worker-binary", type=Path, required=True,
                        help="same external worker binary; read/hash only, never executed here")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    output = audit.output_path(args.output)
    started = time.perf_counter_ns()
    inputs, catalog, panel, panel_sha, rows = load_inputs(args)
    worker = args.worker_binary.resolve(strict=True)
    if worker.is_relative_to(ROOT):
        raise ValueError("worker binary must remain outside verified checkout")
    worker_identity = audit.hash_file(worker, 512 * 1024 * 1024)
    assets = audit.preflight_assets(panel, args.cache)
    report = {
        "schema": "fluxr-wordcode-token-measurement-v14-v1", "classification": catalog["scope"],
        "source_root": str(ROOT), "source_files": inputs.files, "payloads": inputs.payloads,
        "panel_sha256": panel_sha, "dependencies": audit.DEPENDENCIES,
        "python": {"version": platform.python_version(), "implementation": platform.python_implementation()},
        "dictionary_observation": {"file": inputs.files["dictionary_observation"],
            "worker_binary": {"path": str(worker), **worker_identity},
            "expected_command": [str(worker), "dictionary"],
            "provenance": "Caller-supplied observation bound to bytes; this counter does not prove command execution or authorization."},
        "encoding_policy": "Each complete exact UTF-8 string encoded once with ordinary encoding and no added special tokens/chat template.",
        "json_policy": "json.dumps(text, ensure_ascii=False, allow_nan=False); complete arrays use separators=(',',':').",
        "transcript_policy": {"joined_body_string": "concatenate message bodies without added separators",
            "joined_labeled_transcript": "concatenate '[' + event + ':' + direction + ']\\n' + body + '\\n' for every message in order",
            "message_body_token_sum": "sum independent full-message encodings; explicitly distinct from either joined-string count",
            "reuse": "charge language setup and full shared library once; charge each distinct dictionary once per session",
            "cold": "charge all applicable language/dictionary/library setup independently for each case"},
        "source_target": {"maximum_exact_ratio": "1/10", "required_tokenizers": 4, "panel_size": 6,
            "per_encoding_gate": "median and aggregate <=1/10 for both standalone Rust and equally available shared-helper source arms",
            "population_gate": "practical and synthetic reported separately; combined descriptive population never closes either",
            "setup_cost_boundary": "source gate excludes shared setup on both sides; complete cold/reuse scenarios retain all specified setup separately"},
        "measurement_status": "complete", "profiles": [], "task_completion": False, "profile_completion": False,
        "limitations": [
            "Five authored fixtures are neither held-out tasks nor representative model coding episodes.",
            "This counter does not validate source equality, traps, execution, compilation, native routes or grading isolation.",
            "Rust arithmetic/result equivalence excludes VM resource accounting; loops need separately bounded terminating inputs.",
            "Synthetic forest uses compact checked_pow and a reusable Rust helper; expanded FA1 density is a different metric.",
            "No custom tokenizer, embedding compatibility, model competence or most-deployed-models claim.",
            "Scripted messages are constructed here; absent live attempts, cache usage, repairs, reviews, tools, handoffs and latency remain unknown.",
            "Binary/content hashes bind comparison subjects; they establish neither truthful execution nor authorization.",
            "Tokenizer normalization belongs to each pinned asset; measurement host never normalizes source whitespace/newlines.",
        ],
    }
    for asset in assets:
        codec = audit.load_codec(asset)
        report["profiles"].append(measure_profile(codec, inputs, rows))
        del codec
        gc.collect()
    report["population_gates"] = {}
    for population in ("practical", "synthetic"):
        passes = sum(profile["populations"][population]["both_fair_Rust_baselines_pass"] for profile in report["profiles"])
        report["population_gates"][population] = {
            "passing_tokenizers": passes, "required_tokenizers": 4, "status": "met_in_authored_static_slice" if passes >= 4 else "not_met",
            "per_baseline_passing_tokenizers": {arm: sum(profile["populations"][population]["wordcode_vs_" + arm]["passes_median_and_aggregate"]
                                                       for profile in report["profiles"]) for arm in ("rust_full", "rust_shared_call")},
            "complete_task_or_profile_pass": False,
        }
    inputs.verify_unchanged()
    if audit.hash_file(worker, 512 * 1024 * 1024) != worker_identity:
        raise ValueError("worker binary changed during measurement")
    report["component_overhead"] = {"elapsed_ns": time.perf_counter_ns() - started,
        "process_peak_rss": None if sys.platform == "win32" else resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
        "process_peak_rss_unit": None if sys.platform == "win32" else (
            "KiB" if sys.platform.startswith("linux") else "platform_defined"
        ),
        "scope": "counter process high-water RSS and elapsed time; not isolated model inference, compiler or kernel performance"}
    audit.write_report(output, report)
    print(json.dumps({"output": str(output), "measurement_status": "complete",
                      "population_gates": report["population_gates"], "task_completion": False, "profile_completion": False}, sort_keys=True))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, TypeError, UnicodeError, RecursionError) as error:
        print(json.dumps({"status": "refused", "error": str(error)}), file=sys.stderr)
        raise SystemExit(1)
