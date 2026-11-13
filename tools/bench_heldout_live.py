"""Collect sealed FXM-04.A responses with complete, resumable attempt accounting.

Collection eligibility is not behavioral correctness and does not authorize
candidate compilation or execution. Generated-code replay remains disabled until
an independently validated OS isolation boundary is installed.
"""
import argparse
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path
import re
import tempfile
import tomllib

from bench_heldout_tokens import ACTIONS, CASES, LANGUAGES, PANEL, prompt, read, seal_inputs, write
from eval.runner import (DEFAULT_ERROR_BYTES, DEFAULT_EVENT_BYTES, MAX_PROMPT_BYTES,
                         SCHEMA, digest, file_digest, parse_events, run_bounded,
                         strict_json, write_json)


def _now():
    return datetime.now(timezone.utc).isoformat()


def _bounded_text(path):
    with path.open("rb") as stream:
        content = stream.read(MAX_PROMPT_BYTES + 1)
    if len(content) > MAX_PROMPT_BYTES:
        raise ValueError(f"input exceeds byte limit: {path.name}")
    return content.decode("utf-8")


def _validate_key(key):
    if not isinstance(key, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,159}", key):
        raise ValueError("invalid attempt key")


def collect_attempt(executable, evidence, settings, key, text):
    _validate_key(key)
    payload = text.encode("utf-8")
    if len(payload) > MAX_PROMPT_BYTES:
        raise ValueError("prompt exceeds byte limit")
    destination = evidence / key
    destination.mkdir()
    write(destination / "prompt.txt", text)
    intent = {"schema_version": SCHEMA, "key": key, "created_at": _now(),
              "prompt": file_digest(destination / "prompt.txt"),
              "settings_sha256": digest(settings), "provider": executable}
    write_json(destination / "intent.json", intent)
    with tempfile.TemporaryDirectory(prefix="fluxr-authoring-") as directory:
        command = [
            executable, "exec", "--ephemeral", "--ignore-user-config", "--skip-git-repo-check",
            "--sandbox", "read-only", "--json", "--color", "never",
            "--disable", "shell_tool", "--disable", "unified_exec", "--disable", "hooks",
            "-c", 'web_search="disabled"', "-c", f'model_reasoning_effort="{settings["effort"]}"',
            "--model", settings["model"], "--cd", directory, "-",
        ]
        result = run_bounded(
            command, payload, directory, destination / "events.jsonl", destination / "stderr.txt",
            timeout_seconds=settings["timeout_seconds"],
            max_stdout_bytes=settings.get("max_event_bytes", DEFAULT_EVENT_BYTES),
            max_stderr_bytes=settings.get("max_stderr_bytes", DEFAULT_ERROR_BYTES),
        )
    parsed = parse_events(destination / "events.jsonl",
                          max_bytes=settings.get("max_event_bytes", DEFAULT_EVENT_BYTES))
    answer = parsed.pop("answer")
    terminal = result.terminal
    if terminal == "exited":
        terminal = "nonzero_exit" if result.exit_code != 0 else parsed["event_terminal"]
    eligible = terminal == "eligible"
    status = {**intent, **parsed, "terminal": terminal, "finished_at": _now(),
              "exit_code": result.exit_code, "seconds": result.seconds,
              "provider_process_started": result.process_started, "process_detail": result.detail,
              "eligible_for_replay": eligible,
              "candidate_execution_authorized": False,
              "artifacts": {name: file_digest(destination / name)
                            for name in ("prompt.txt", "events.jsonl", "stderr.txt")}}
    if eligible:
        write(destination / "answer.txt", answer)
        status["artifacts"]["answer.txt"] = file_digest(destination / "answer.txt")
    write_json(destination / "status.json", status)
    print(f"{key}: {terminal}", flush=True)
    return status


def collect(executable, evidence, settings, key, text):
    """Compatibility entry point; return response eligibility, not exit success."""
    return collect_attempt(executable, evidence, settings, key, text)["eligible_for_replay"]


def _resume_status(folder, arm, settings):
    intent = strict_json(_bounded_text(folder / "intent.json"))
    if (intent.get("key") != arm["key"] or intent.get("prompt") != arm["prompt"]
            or intent.get("settings_sha256") != digest(settings)):
        raise ValueError(f"attempt identity mismatch: {arm['key']}")
    if file_digest(folder / "prompt.txt") != arm["prompt"]:
        raise ValueError(f"prompt changed: {arm['key']}")
    path = folder / "status.json"
    if not path.exists():
        # An interruption may already have incurred cost. Never silently repeat it.
        status = {**intent, "terminal": "interrupted", "finished_at": _now(),
                  "eligible_for_replay": False, "candidate_execution_authorized": False,
                  "usage": None, "usage_accounting": "unknown_after_interruption",
                  "artifacts": {p.name: file_digest(p) for p in folder.iterdir()
                                if p.name in ("prompt.txt", "events.jsonl", "stderr.txt")}}
        if (folder / "events.jsonl").exists():
            parsed = parse_events(folder / "events.jsonl",
                                  max_bytes=settings.get("max_event_bytes", DEFAULT_EVENT_BYTES))
            parsed.pop("answer")
            status.update(parsed)
            status["observed_usage_accounting"] = parsed["usage_accounting"]
            status["usage_accounting"] = "unknown_after_interruption"
        write_json(path, status)
    status = strict_json(_bounded_text(path))
    if (status.get("schema_version") != SCHEMA or status.get("key") != arm["key"]
            or status.get("settings_sha256") != digest(settings)
            or status.get("prompt") != arm["prompt"]):
        raise ValueError(f"terminal identity mismatch: {arm['key']}")
    for name, expected in status.get("artifacts", {}).items():
        if name not in ("prompt.txt", "events.jsonl", "stderr.txt", "answer.txt"):
            raise ValueError("unknown evidence artifact")
        if file_digest(folder / name) != expected:
            raise ValueError(f"attempt artifact changed: {arm['key']}/{name}")
    return status


def _cancel_before_launch(executable, folder, settings, key, text):
    """Persist a scheduled arm that cancellation prevented from launching."""
    folder.mkdir()
    write(folder / "prompt.txt", text)
    write(folder / "events.jsonl", "")
    write(folder / "stderr.txt", "")
    intent = {"schema_version": SCHEMA, "key": key, "created_at": _now(),
              "prompt": file_digest(folder / "prompt.txt"),
              "settings_sha256": digest(settings), "provider": executable}
    write_json(folder / "intent.json", intent)
    status = {**intent, "terminal": "cancelled_before_launch", "finished_at": _now(),
              "exit_code": None, "seconds": 0.0, "provider_process_started": False,
              "usage": {"input_tokens": 0, "cached_input_tokens": 0, "output_tokens": 0},
              "usage_accounting": "not_invoked", "usage_observations": [],
              "eligible_for_replay": False, "candidate_execution_authorized": False,
              "artifacts": {name: file_digest(folder / name)
                            for name in ("prompt.txt", "events.jsonl", "stderr.txt")}}
    write_json(folder / "status.json", status)
    return status


def run_scheduled(executable, evidence, settings, scheduled, *, seal, resume=False):
    """Run independent arms despite individual failure, preserving the denominator."""
    scheduled = list(scheduled)
    for key, text in scheduled:
        _validate_key(key)
        if not isinstance(text, str) or len(text.encode()) > MAX_PROMPT_BYTES:
            raise ValueError("prompt must be a bounded UTF-8 string")
    evidence.mkdir(parents=True, exist_ok=True)
    arms = [{"key": key, "prompt": {"sha256": sha256(text.encode()).hexdigest(),
                                     "bytes": len(text.encode())}}
            for key, text in scheduled]
    if len({arm["key"] for arm in arms}) != len(arms):
        raise ValueError("duplicate scheduled attempt")
    manifest = {"schema_version": "fluxr-eval-run/1", "settings": settings,
                "seal_sha256": digest(seal), "provider": executable, "arms": arms}
    manifest_path = evidence / "run.json"
    if manifest_path.exists():
        if not resume:
            raise ValueError("run already exists; use --resume to reuse recorded attempts")
        if strict_json(_bounded_text(manifest_path)) != manifest:
            raise ValueError("resumed run does not match sealed manifest")
    else:
        # Publish the complete schedule before starting any provider process.
        write_json(manifest_path, manifest)
    statuses = {}

    def summary():
        completed = sum(s.get("eligible_for_replay", False) for s in statuses.values())
        result = {"schema_version": "fluxr-eval-summary/1", "run_sha256": digest(manifest),
                  "scheduled": len(arms), "complete": completed,
                  "explicit_failure": len(statuses) - completed, "pending": len(arms) - len(statuses),
                  "attempts": {key: status["terminal"] for key, status in statuses.items()},
                  "usage_by_attempt": {key: {"usage": status.get("usage"),
                                             "accounting": status.get("usage_accounting"),
                                             "observations": status.get("usage_observations", [])}
                                       for key, status in statuses.items()}}
        write_json(evidence / "summary.json", result)
        return result

    summary()
    for index, (arm, (_, text)) in enumerate(zip(arms, scheduled, strict=True)):
        key = arm["key"]
        folder = evidence / key
        if folder.exists():
            if not resume:
                raise ValueError(f"attempt already exists: {key}")
            status = _resume_status(folder, arm, settings)
        else:
            try:
                status = collect_attempt(executable, evidence, settings, key, text)
            except Exception as error:
                # Controller failures have a denominator too. Do not retry them.
                folder.mkdir(exist_ok=True)
                if not (folder / "prompt.txt").exists():
                    write(folder / "prompt.txt", text)
                intent = {"schema_version": SCHEMA, "key": key, "prompt": arm["prompt"],
                          "settings_sha256": digest(settings), "provider": executable,
                          "created_at": _now()}
                if not (folder / "intent.json").exists():
                    write_json(folder / "intent.json", intent)
                status = {**intent, "terminal": "controller_error", "detail": str(error),
                          "finished_at": _now(), "eligible_for_replay": False,
                          "candidate_execution_authorized": False, "usage": None,
                          "usage_accounting": "unknown_after_controller_error",
                          "artifacts": {p.name: file_digest(p) for p in folder.iterdir()
                                        if p.name in ("prompt.txt", "events.jsonl", "stderr.txt")}}
                write_json(folder / "status.json", status)
        statuses[key] = status
        summary()
        if status["terminal"] == "cancelled":
            for later, (_, later_text) in zip(arms[index + 1:], scheduled[index + 1:], strict=True):
                later_key = later["key"]
                later_folder = evidence / later_key
                if later_folder.exists():
                    # Preserve earlier terminal evidence on a resumed run.
                    later_status = _resume_status(later_folder, later, settings)
                else:
                    later_status = _cancel_before_launch(executable, later_folder, settings,
                                                         later_key, later_text)
                statuses[later_key] = later_status
                summary()
            break
    return summary()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--codex", required=True)
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--repair", nargs=3, metavar=("CASE", "LANGUAGE", "ACTION"))
    parser.add_argument("--feedback", type=Path)
    args = parser.parse_args()
    cases = tomllib.loads(read(CASES))["heldout"]
    settings = tomllib.loads(read(PANEL))["live"]
    seal = seal_inputs(cases)
    if seal != strict_json(_bounded_text(args.evidence / "seal.json")):
        parser.error("Current inputs do not match the sealed corpus")
    if settings["initial_attempts_per_arm"] != 1 or settings["max_repair_attempts_per_arm"] != 1:
        parser.error("Expected one initial attempt and at most one repair per arm")
    if len(settings["cases"]) != len(set(settings["cases"])) or any(name not in cases for name in settings["cases"]):
        parser.error("pilot case list must be unique and refer to the sealed corpus")
    results = args.evidence / "live"
    if args.repair:
        name, language, action = args.repair
        if name not in settings["cases"] or language not in LANGUAGES or action not in ACTIONS or not args.feedback:
            parser.error("Repair requires a pilot arm and recorded feedback")
        key = f"{name}.{language}.{action}"
        answer = _bounded_text(results / key / "answer.txt")
        feedback = _bounded_text(args.feedback)
        text = prompt(name, cases[name], language, action)
        text += f"\nPrevious response:\n{answer}\n\nReplay feedback:\n{feedback}\n"
        scheduled = [(key + ".repair", text)]
        results = args.evidence / "repairs" / key
    else:
        if args.feedback:
            parser.error("--feedback requires --repair")
        scheduled = []
        for i, name in enumerate(settings["cases"]):
            for action in ACTIONS:
                order = LANGUAGES if (i + ACTIONS.index(action)) % 2 == 0 else LANGUAGES[::-1]
                scheduled.extend((f"{name}.{language}.{action}", prompt(name, cases[name], language, action))
                                 for language in order)
    try:
        summary = run_scheduled(args.codex, results, settings, scheduled, seal=seal, resume=args.resume)
    except (ValueError, OSError) as error:
        parser.error(str(error))
    if summary["explicit_failure"] or summary["pending"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
