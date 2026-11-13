#!/usr/bin/env python3
"""Count exact retained AC1/AH1 strings without admitting generated candidate code.

Run the reviewed agent_context_demo example and retain its complete JSON first.
This is development-fixture text accounting, not provider billing, a live-model
trial, a held-out benchmark or an independent assertion of truthful execution.
"""
from __future__ import annotations

import argparse
import hashlib
from importlib.metadata import version
import json
from pathlib import Path
import sys
import time
import tomllib

if sys.platform != "win32":
    import resource

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "benchmarks/context-compact-v1.toml"
MAX_INPUT_BYTES = 2 * 1024 * 1024
ARMS = ("legacy", "compact", "focused")
CASE_IDS = ("scalar", "trap_order", "helper", "cow")
TOKENIZERS = ("o200k_base", "cl100k_base")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key: " + key)
        result[key] = value
    return result


def reject_constant(text):
    raise ValueError("nonfinite JSON constant: " + text)


def read_bytes(path):
    path = Path(path)
    if not path.is_file() or path.stat().st_size > MAX_INPUT_BYTES:
        raise ValueError("regular bounded input required: " + str(path))
    with path.open("rb") as stream:
        raw = stream.read(MAX_INPUT_BYTES + 1)
    if len(raw) > MAX_INPUT_BYTES:
        raise ValueError("input grew beyond bound")
    return raw


def source(path):
    actual = (ROOT / path).resolve(strict=True)
    if not actual.is_relative_to(ROOT):
        raise ValueError("source path escapes checkout")
    return read_bytes(actual).decode("utf-8")


def identity(text):
    raw = text.encode("utf-8")
    return {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}


def require_text(value, label):
    if not isinstance(value, str) or not value or len(value.encode("utf-8")) > MAX_INPUT_BYTES:
        raise ValueError("nonempty bounded UTF-8 string required: " + label)
    return value


def require_object(value, label):
    if not isinstance(value, dict):
        raise ValueError("JSON object required: " + label)
    return value


def validate_catalog(catalog):
    measurement = require_object(catalog.get("measurement"), "catalog measurement")
    if (measurement.get("schema") != "fluxr-ac1-token-measurement-v1"
            or measurement.get("profile") != "fluxr-ac1-v1"
            or measurement.get("observation_schema") != "fluxr-ac1-context-observations-v1"
            or measurement.get("tiktoken_version") != "0.14.0"
            or measurement.get("tokenizers") != list(TOKENIZERS)
            or measurement.get("equivalent_rust_max_ratio") != 0.1):
        raise ValueError("pinned catalog identity, tokenizers and source target required")
    cases = require_object(catalog.get("case"), "catalog cases")
    if tuple(cases) != CASE_IDS:
        raise ValueError("exact four catalog cases required in declared order")


def validate_observations(observations, catalog):
    validate_catalog(catalog)
    require_object(observations, "observations")
    if observations.get("schema") != catalog["measurement"]["observation_schema"]:
        raise ValueError("unknown observation schema")
    if observations.get("protocol_setup") != source(catalog["measurement"]["protocol"]):
        raise ValueError("observed tutorial bytes differ from measured setup")
    if observations.get("profile") != catalog["measurement"]["profile"]:
        raise ValueError("context profile differs from catalog")
    if observations.get("source_token_target") != 0.1:
        raise ValueError("source-token target changed")
    cases = observations.get("cases")
    expected = catalog["case"]
    if not isinstance(cases, list) or len(cases) != len(CASE_IDS):
        raise ValueError("all four source cases required exactly once")
    if tuple(require_object(row, "case").get("id") for row in cases) != CASE_IDS:
        raise ValueError("missing, unknown, duplicate or reordered source case")
    for row in cases:
        spec = expected[row["id"]]
        for field, key in (("source", "base"), ("completed_source", "fluxr"),
                           ("fill", "fill"), ("rust_source", "rust")):
            if row.get(field) != source(spec[key]):
                raise ValueError("observed fixture bytes differ: " + row["id"] + "/" + field)
        if row.get("exact_fsm_equal") is not True:
            raise ValueError("complete candidate equality observation missing")
        binding = require_object(row.get("binding"), "host binding")
        if not binding:
            raise ValueError("complete host binding observation required")
        host_calls = require_object(row.get("recorded_host_calls"), "host call records")
        if set(host_calls) != set(ARMS):
            raise ValueError("all three complete host call records required")
        for arm in ARMS:
            require_text(row.get(arm + "_packet"), arm + " packet")
            record = require_text(host_calls.get(arm), arm + " host call")
            parsed = require_object(json.loads(record, object_pairs_hook=unique_object,
                                                parse_constant=reject_constant), "host call")
            if parsed.get("binding") != binding:
                raise ValueError("retained host call does not bind the observed draft")
        failed = require_object(row.get("failed_fill"), "failed fill")
        if failed.get("unchanged") is not True:
            raise ValueError("failed fill and unchanged draft observation required")
        for field in ("text", "kind", "detail", "display"):
            require_text(failed.get(field), "failed_fill." + field)
    locality = observations.get("locality_cases")
    if not isinstance(locality, list) or len(locality) != 3:
        raise ValueError("all three fixed locality cases required")
    for row, helpers in zip(locality, (0, 31, 127), strict=True):
        require_object(row, "locality row")
        if type(row.get("helpers")) is not int or row["helpers"] != helpers:
            raise ValueError("fixed locality counts required in declared order")
        expected_source = source(catalog["case"]["scalar"]["base"]) + "".join(
            f"fn unrelated{index} 1 value int int ret value\n" for index in range(helpers))
        if (row.get("id") != "locality_" + str(helpers) or row.get("source") != expected_source
                or row.get("fill") != source(catalog["case"]["scalar"]["fill"])
                or row.get("completed_source") != expected_source.replace(
                    "ret add x y", "ret rem add mul x y 17 997", 1)):
            raise ValueError("locality fixture identity differs")
        for arm in ARMS:
            require_text(row.get(arm + "_packet"), "locality " + arm)
        if row.get("exact_fsm_equal") is not True:
            raise ValueError("locality candidate equality observation missing")
    return cases, locality


def count(enc, text):
    """Encode the complete UTF-8 string once; never add per-lexeme estimates."""
    require_text(text, "measurement string")
    quoted = json.dumps(text, ensure_ascii=False)
    return {**identity(text), "tokens": len(enc.encode_ordinary(text)),
            "json_quoted_wire": {**identity(quoted), "tokens": len(enc.encode_ordinary(quoted))}}


def encoding_identity(enc):
    ranks = hashlib.sha256()
    for token, rank in sorted(enc._mergeable_ranks.items(), key=lambda pair: pair[1]):
        ranks.update(rank.to_bytes(4, "little"))
        ranks.update(len(token).to_bytes(4, "little"))
        ranks.update(token)
    return {"name": enc.name, "rank_bytes_sha256": ranks.hexdigest(),
            "regex_sha256": hashlib.sha256(enc._pat_str.encode()).hexdigest(),
            "special_token_policy": "encode_ordinary; special-token-looking source stays ordinary text",
            "custom_vocabulary": False, "trained_model_or_embeddings": None}


def episode(enc, row, arm, tutorial):
    """An explicit synthetic sequence, not an observed LLM conversation."""
    failure = row["failed_fill"]
    feedback = "candidate admitted after full compilation; fixture equality checked\n"
    messages = [
        ("setup", "input", tutorial),
        ("base_source", "input", row["source"]),
        ("initial_context", "input", row[arm + "_packet"]),
        ("failed_body", "output", failure["text"]),
        ("failure_diagnostic", "input", failure["display"]),
        ("repeated_context", "input", row[arm + "_packet"]),
        ("repaired_body", "output", row["fill"]),
        ("admission_feedback", "input", feedback),
    ]
    measured = [{"event": event, "direction": direction, **count(enc, text)}
                for event, direction, text in messages]
    transcript = "".join(event + " " + direction + "\n" + text + "\n"
                         for event, direction, text in messages)
    host_record = row["recorded_host_calls"][arm]
    with_host_records = messages[:2] + [("host_query_record", "input", host_record)] + messages[2:5] + [
        ("host_requery_record", "input", host_record)] + messages[5:]
    full_record_transcript = "".join(event + " " + direction + "\n" + text + "\n"
                                     for event, direction, text in with_host_records)
    return {"classification": "scripted_development_fixture_text_accounting",
            "attempts": [{"order": 1, "status": "rejected_type_fill", "charged": True},
                         {"order": 2, "status": "fixture_accepted_after_full_check", "charged": True}],
            "messages": measured,
            "message_body_token_sum": sum(message["tokens"] for message in measured),
            "input_body_tokens": sum(message["tokens"] for message in measured if message["direction"] == "input"),
            "output_body_tokens": sum(message["tokens"] for message in measured if message["direction"] == "output"),
            "joined_transcript": count(enc, transcript),
            "if_full_host_query_records_are_model_visible": {
                "message_body_token_sum": sum(message["tokens"] for message in measured) + 2 * count(enc, host_record)["tokens"],
                "joined_transcript": count(enc, full_record_transcript),
                "scope": "Explicit alternative charges each complete retained query-call record twice, without also charging its redundant standalone binding. This remains a synthetic accounting scenario, not an observed model exchange."},
            "setup_amortization": "none: full setup is charged independently for every case/arm",
            "feedback_provenance": "failure diagnostic is actual retained example output; admission feedback is declared fixed measurement text",
            "provider_framing_or_billing": "not inferred from static body sums or the joined synthetic transcript"}


def measure(observations_path):
    started = time.perf_counter_ns()
    import tiktoken
    if version("tiktoken") != "0.14.0":
        raise ValueError("pinned tiktoken==0.14.0 required")
    raw = read_bytes(observations_path)
    observations = json.loads(raw, object_pairs_hook=unique_object, parse_constant=reject_constant)
    catalog_raw = read_bytes(CATALOG)
    catalog = tomllib.loads(catalog_raw.decode("utf-8"))
    cases, locality = validate_observations(observations, catalog)
    tutorial = source(catalog["measurement"]["protocol"])
    report = {"schema": "fluxr-ac1-token-panel-v1", "tiktoken_version": version("tiktoken"),
              "catalog": {"path": str(CATALOG.relative_to(ROOT)), **identity(catalog_raw.decode("utf-8"))},
              "observations": {"path": str(Path(observations_path).resolve()), "bytes": len(raw),
                               "sha256": hashlib.sha256(raw).hexdigest()},
              "counter": {"path": "tools/bench_context_compact.py", **identity(read_bytes(__file__).decode("utf-8"))},
              "equivalent_rust_max_ratio": catalog["measurement"]["equivalent_rust_max_ratio"],
              "population": catalog["measurement"]["population"], "profiles": [],
              "live_model_episode": {"status": "not_executed", "model": None, "tokenizer_model_pair": None,
                                     "cached_input_tokens": None, "uncached_input_tokens": None,
                                     "billed_output_tokens": None, "reasoning_tokens": None,
                                     "tools": None, "repairs": None, "review_tokens": None,
                                     "handoffs": None, "latency_seconds": None, "accepted_task_success": None,
                                     "blocker": "No qualified arbitrary-candidate compilation/execution adapter and pinned live-provider telemetry; development agents and scripted fills are not trials."},
              "claims": {"source_target_changed": False, "released_packet_changed": False,
                         "custom_tokenizer_competence": False, "static_effect_resource_proof": False,
                         "source_tokens_equal_episode_cost": False, "compressed_bytes_equal_tokens": False,
                         "source_equality_verified_by_counter": False,
                         "equality_scope": "Counter checks retained fixture bytes and required receipt fields; independent behavioral validation is separate."}}
    for name in catalog["measurement"]["tokenizers"]:
        init_start = time.perf_counter_ns()
        enc = tiktoken.get_encoding(name)
        init_ns = time.perf_counter_ns() - init_start
        count_start = time.perf_counter_ns()
        rows = []
        for row in cases:
            texts = {key: count(enc, row[key]) for key in ("source", "completed_source", "fill", "rust_source")}
            packets = {arm: count(enc, row[arm + "_packet"]) for arm in ARMS}
            episodes = {arm: episode(enc, row, arm, tutorial) for arm in ARMS}
            rust_tokens = texts["rust_source"]["tokens"]
            flux_tokens = texts["completed_source"]["tokens"]
            binding = json.dumps(row["binding"], sort_keys=True, separators=(",", ":"))
            rows.append({"case": row["id"], "strings": texts, "context_packets": packets,
                         "source_ratio": flux_tokens / rust_tokens,
                         "passes_0_1_target": flux_tokens <= 0.1 * rust_tokens,
                         "packet_reduction": {arm: 1 - packets[arm]["tokens"] / packets["legacy"]["tokens"]
                                              for arm in ("compact", "focused")},
                         "scripted_episodes": episodes,
                         "scripted_body_token_reduction": {
                             arm: 1 - episodes[arm]["message_body_token_sum"] / episodes["legacy"]["message_body_token_sum"]
                             for arm in ("compact", "focused")},
                         "retained_host_binding_json": count(enc, binding),
                         "recorded_host_calls": {arm: count(enc, row["recorded_host_calls"][arm]) for arm in ARMS},
                         "host_binding_allocation": "Measured separately; if the actual host sends it to a model, charge the complete string to that episode too. No host bytes are silently called free model context.",
                         "rust_baseline_scope": catalog["case"][row["id"]]["functionality"],
                         "matched_rust_live_episode": None})
        locality_rows = []
        for row in locality:
            packets = {arm: count(enc, row[arm + "_packet"]) for arm in ARMS}
            locality_rows.append({"id": row["id"], "helpers": row["helpers"],
                                  "source": count(enc, row["source"]),
                                  "completed_source": count(enc, row["completed_source"]),
                                  "fill": count(enc, row["fill"]),
                                  "counters": row.get("counters"),
                                  "context_packets": packets,
                                  "packet_reduction": {arm: 1 - packets[arm]["tokens"] / packets["legacy"]["tokens"]
                                                       for arm in ("compact", "focused")},
                                  "rust_source_ratio": None,
                                  "scope": "Synthetic signature-catalog locality stress, excluded from the four source-target cases and held-out/live claims."})
        shared = []
        for path, role in (("examples/agent_context_demo.rs", "shared measurement host and independent checked-Rust oracles"),
                           ("docs/agent-authoring-v1.md", "complete inherited AH1 retrieval document"),
                           ("docs/agent-context-v1.md", "complete AC1 retrieval document")):
            shared.append({"path": path, "role": role, **count(enc, source(path)),
                           "allocation": "Separate shared implementation/retrieval source, not added only to the Rust algorithm baseline. Charge whenever actually retrieved in a live episode."})
        report["profiles"].append({"tokenizer": encoding_identity(enc), "rows": rows,
                                   "locality_rows": locality_rows, "protocol_setup": count(enc, tutorial),
                                   "shared_host_and_retrieval": shared,
                                   "source_target_passes": sum(row["passes_0_1_target"] for row in rows),
                                   "source_target_cases": len(rows),
                                   "tokenizer_initialization_ns": init_ns,
                                   "measurement_elapsed_ns": time.perf_counter_ns() - count_start})
    report["component_overhead"] = {"counter_elapsed_ns": time.perf_counter_ns() - started,
                                    "process_peak_rss_kib": (
                                        None if sys.platform == "win32"
                                        else resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
                                        // (1024 if sys.platform == "darwin" else 1)
                                    ),
                                    "scope": "Linux process high-water RSS and single static-counter elapsed time; not isolated tokenizer allocation, model inference latency or kernel runtime."}
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--observations", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        print(json.dumps(measure(args.observations), indent=2, allow_nan=False))
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(1, "context measurement refused: " + str(error) + "\n")


if __name__ == "__main__":
    main()
