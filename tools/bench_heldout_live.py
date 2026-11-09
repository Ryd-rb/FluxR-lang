"""Collect isolated Codex responses for the sealed FXM-04.A pilot."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib

from bench_heldout_tokens import ACTIONS, CASES, LANGUAGES, PANEL, prompt, read, seal_inputs, write


def collect(executable, evidence, settings, key, text):
    destination = evidence / key
    destination.mkdir()
    write(destination / "prompt.txt", text)
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="fluxr-authoring-") as directory:
        command = [
            executable, "exec", "--ephemeral", "--ignore-user-config", "--skip-git-repo-check",
            "--sandbox", "read-only", "--json", "--color", "never",
            "--disable", "shell_tool", "--disable", "unified_exec", "--disable", "hooks",
            "-c", 'web_search="disabled"', "-c", f'model_reasoning_effort="{settings["effort"]}"',
            "--model", settings["model"], "--cd", directory, "-",
        ]
        with (destination / "events.jsonl").open("wb") as events, (destination / "stderr.txt").open("wb") as errors:
            try:
                process = subprocess.run(command, input=text.encode(), stdout=events, stderr=errors,
                                         cwd=directory, timeout=settings["timeout_seconds"])
                exit_code = process.returncode
            except subprocess.TimeoutExpired:
                exit_code = None
    events = [json.loads(line) for line in read(destination / "events.jsonl").splitlines()]
    items = [event["item"] for event in events if event["type"] == "item.completed"]
    answers = [item["text"] for item in items if item["type"] == "agent_message"]
    usage = next((event["usage"] for event in events if event["type"] == "turn.completed"), None)
    tool_items = [item["type"] for item in items if item["type"] not in ("agent_message", "reasoning")]
    accepted = exit_code == 0 and usage is not None and len(answers) == 1 and not tool_items
    status = {"exit_code": exit_code, "seconds": time.monotonic() - started,
              "usage": usage, "tool_items": tool_items, "eligible_for_replay": accepted}
    write(destination / "status.json", json.dumps(status, indent=2) + "\n")
    if accepted:
        write(destination / "answer.txt", answers[0])
    print(f"{key}: {status}", flush=True)
    return exit_code == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--codex", required=True)
    parser.add_argument("--repair", nargs=3, metavar=("CASE", "LANGUAGE", "ACTION"))
    parser.add_argument("--feedback", type=Path)
    args = parser.parse_args()
    cases = tomllib.loads(read(CASES))["heldout"]
    settings = tomllib.loads(read(PANEL))["live"]
    if seal_inputs(cases) != json.loads(read(args.evidence / "seal.json")):
        parser.error("Current inputs do not match the sealed corpus")
    if settings["initial_attempts_per_arm"] != 1 or settings["max_repair_attempts_per_arm"] != 1:
        parser.error("Expected one initial attempt and at most one repair per arm")
    results = args.evidence / "live"
    results.mkdir(exist_ok=True)
    if args.repair:
        name, language, action = args.repair
        if name not in settings["cases"] or language not in LANGUAGES or action not in ACTIONS or not args.feedback:
            parser.error("Repair requires a pilot arm and recorded feedback")
        key = f"{name}.{language}.{action}"
        answer = read(results / key / "answer.txt")
        feedback = read(args.feedback)
        text = prompt(name, cases[name], language, action)
        text += f"\nPrevious response:\n{answer}\n\nReplay feedback:\n{feedback}\n"
        if not collect(args.codex, results, settings, key + ".repair", text):
            raise SystemExit(1)
        return
    if args.feedback:
        parser.error("--feedback requires --repair")
    for i, name in enumerate(settings["cases"]):
        for action in ACTIONS:
            order = LANGUAGES if (i + ACTIONS.index(action)) % 2 == 0 else LANGUAGES[::-1]
            for language in order:
                key = f"{name}.{language}.{action}"
                if (results / key).exists():
                    parser.error(f"Attempt already exists: {key}")
                if not collect(args.codex, results, settings, key, prompt(name, cases[name], language, action)):
                    raise SystemExit(1)


if __name__ == "__main__":
    main()
