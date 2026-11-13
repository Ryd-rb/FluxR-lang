"""Bounded provider collection and strict, loss-aware event accounting.

The provider executable is trusted infrastructure. This module is NOT a sandbox
for generated source. POSIX process groups bound ordinary descendants; a hostile
process can escape a process group and requires an OS sandbox instead.
"""
from __future__ import annotations

from dataclasses import dataclass
from hashlib import sha256
import json
import math
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time

SCHEMA = "fluxr-eval-attempt/1"
MAX_PROMPT_BYTES = 2 * 1024 * 1024
DEFAULT_EVENT_BYTES = 8 * 1024 * 1024
DEFAULT_ERROR_BYTES = 2 * 1024 * 1024
MAX_EVENT_LINE_BYTES = 1024 * 1024


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def digest(value):
    return sha256(canonical(value)).hexdigest()


def write_json(path, value):
    """Atomic replacement in the same directory; no partially written ledger."""
    path = Path(path)
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def file_digest(path):
    hasher = sha256()
    size = 0
    with Path(path).open("rb") as stream:
        while chunk := stream.read(65536):
            hasher.update(chunk)
            size += len(chunk)
    return {"sha256": hasher.hexdigest(), "bytes": size}


def strict_json(text):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def constant(_):
        raise ValueError("non-finite JSON number")

    def decimal(value):
        result = float(value)
        if not math.isfinite(result):
            raise ValueError("non-finite JSON number")
        return result

    return json.loads(text, object_pairs_hook=pairs, parse_constant=constant, parse_float=decimal)


@dataclass(frozen=True)
class ProcessResult:
    terminal: str
    exit_code: int | None
    seconds: float
    process_started: bool
    detail: str | None = None


def _positive_limit(value, name):
    if type(value) is not int or value <= 0:
        raise ValueError(f"{name} must be a positive integer")
    return value


def run_bounded(command, payload, cwd, stdout_path, stderr_path, *, timeout_seconds,
                max_stdout_bytes=DEFAULT_EVENT_BYTES, max_stderr_bytes=DEFAULT_ERROR_BYTES):
    """Capture bounded files without buffering an unbounded pipe in memory.

    No shell is invoked. Parent exit does not allow a child retaining an output
    pipe to keep this call alive forever. All timeout/cancellation/output-limit
    paths kill the process group, wait for the leader, then close local pipes.
    """
    _positive_limit(max_stdout_bytes, "max_stdout_bytes")
    _positive_limit(max_stderr_bytes, "max_stderr_bytes")
    if (isinstance(timeout_seconds, bool) or not isinstance(timeout_seconds, (int, float))
            or not math.isfinite(timeout_seconds) or timeout_seconds <= 0):
        raise ValueError("timeout_seconds must be finite and positive")
    if len(payload) > MAX_PROMPT_BYTES:
        raise ValueError("prompt exceeds byte limit")
    started = time.monotonic()
    process = None
    terminal, detail = "exited", None
    with open(stdout_path, "wb") as stdout, open(stderr_path, "wb") as stderr:
        if os.name != "posix":
            return ProcessResult("unsupported_platform", None, 0.0, False,
                                 "bounded provider collection currently requires POSIX")
        selector = selectors.DefaultSelector()
        try:
            try:
                process = subprocess.Popen(command, cwd=cwd, stdin=subprocess.PIPE,
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                           start_new_session=True)
            except OSError as error:
                return ProcessResult("launch_error", None, time.monotonic() - started,
                                     False, f"{type(error).__name__}: {error}")
            states = {}
            for pipe, output, limit in ((process.stdout, stdout, max_stdout_bytes),
                                        (process.stderr, stderr, max_stderr_bytes)):
                os.set_blocking(pipe.fileno(), False)
                states[pipe] = [output, limit, 0]
                selector.register(pipe, selectors.EVENT_READ)
            os.set_blocking(process.stdin.fileno(), False)
            if payload:
                selector.register(process.stdin, selectors.EVENT_WRITE)
            else:
                process.stdin.close()
            sent = 0
            while selector.get_map():
                remaining = timeout_seconds - (time.monotonic() - started)
                if remaining <= 0:
                    terminal = "timeout"
                    break
                for key, _ in selector.select(min(0.05, remaining)):
                    pipe = key.fileobj
                    if pipe is process.stdin:
                        try:
                            sent += os.write(pipe.fileno(), payload[sent:sent + 65536])
                        except BrokenPipeError:
                            sent = len(payload)
                        if sent == len(payload):
                            selector.unregister(pipe)
                            pipe.close()
                        continue
                    try:
                        chunk = os.read(pipe.fileno(), 65536)
                    except BlockingIOError:
                        continue
                    if not chunk:
                        selector.unregister(pipe)
                        pipe.close()
                        continue
                    output, limit, count = states[pipe]
                    output.write(chunk[:max(0, limit - count)])
                    states[pipe][2] += len(chunk)
                    if count + len(chunk) > limit:
                        terminal = "output_limit"
                        detail = "stdout" if pipe is process.stdout else "stderr"
                        break
                if terminal != "exited":
                    break
            if terminal == "exited":
                try:
                    process.wait(timeout=max(0.001, timeout_seconds - (time.monotonic() - started)))
                except subprocess.TimeoutExpired:
                    terminal = "timeout"
        except KeyboardInterrupt:
            terminal = "cancelled"
        except Exception as error:
            terminal, detail = "controller_error", f"{type(error).__name__}: {error}"
        finally:
            selector.close()
            if process is not None:
                # Also remove ordinary descendants after a successful parent exit.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                for pipe in (process.stdin, process.stdout, process.stderr):
                    pipe.close()
    return ProcessResult(terminal, process.returncode if process else None,
                         time.monotonic() - started, process is not None, detail)


def _valid_usage(value):
    if not isinstance(value, dict):
        return False
    for key in ("input_tokens", "output_tokens"):
        if type(value.get(key)) is not int or value[key] < 0:
            return False
    cached = value.get("cached_input_tokens", 0)
    return type(cached) is int and 0 <= cached <= value["input_tokens"]


def parse_events(path, *, max_bytes=DEFAULT_EVENT_BYTES):
    """Retain valid usage observations even when the attempt is ineligible.

    Duplicate completion usage is ambiguous: observations are preserved but are
    never silently deduplicated or represented as a reliable billed total.
    """
    _positive_limit(max_bytes, "max_bytes")
    errors, answers, usages, tool_items, failed = [], [], [], [], []
    completions = 0
    starts = 0
    threads = 0
    work_seen = False
    terminal_seen = False
    event_count = 0
    completed_ids = set()
    open_items = {}
    total_bytes = 0

    def error(message):
        if len(errors) < 32:
            errors.append(message)

    known = {"thread.started", "turn.started", "item.started", "item.updated",
             "item.completed", "turn.completed", "turn.failed", "error"}
    with Path(path).open("rb") as stream:
        while True:
            line = stream.readline(min(MAX_EVENT_LINE_BYTES, max_bytes) + 1)
            if not line:
                break
            total_bytes += len(line)
            if total_bytes > max_bytes or len(line) > MAX_EVENT_LINE_BYTES:
                error("event input exceeds byte or line limit")
                break
            try:
                event = strict_json(line.decode("utf-8"))
                if (not isinstance(event, dict) or type(event.get("type")) is not str
                        or event["type"] not in known):
                    raise ValueError("unknown or invalid event type")
            except (ValueError, UnicodeError, RecursionError) as exc:
                error(f"line {event_count + 1}: {exc}")
                event_count += 1
                continue
            event_count += 1
            kind = event["type"]
            # A completed/failed turn seals this attempt. No semantic tail is
            # allowed, including an answer emitted after a completion marker.
            # Duplicate completion remains its own explicit terminal category.
            if terminal_seen and kind != "turn.completed":
                error("event after terminal turn event")
            if kind == "thread.started":
                threads += 1
                if threads > 1 or starts or work_seen:
                    error("thread start after attempt activity")
            if kind == "turn.started":
                starts += 1
                if starts > 1 or work_seen:
                    error("turn start after attempt activity")
            if kind.startswith("item."):
                work_seen = True
            if "usage" in event:
                usages.append({"event": event_count, "type": kind, "usage": event["usage"],
                               "valid": _valid_usage(event["usage"])})
            if kind == "turn.completed":
                terminal_seen = True
                completions += 1
                if "usage" not in event:
                    error("completion lacks usage")
            elif kind in ("turn.failed", "error"):
                terminal_seen = True
                failed.append(kind)
            elif kind.startswith("item."):
                item = event.get("item")
                if (not isinstance(item, dict) or type(item.get("type")) is not str
                        or not item["type"]):
                    error("invalid item event")
                    continue
                item_type = item["type"]
                # A tool attempt violates this collection experiment even if
                # its completion event is lost. Retain every observed tool
                # phase; these are observations, not completed-call totals.
                if item_type not in ("agent_message", "reasoning"):
                    tool_items.append(item_type)
                item_id = item.get("id")
                if item_id is None and kind != "item.completed":
                    error("started or updated item requires an id")
                    continue
                if item_id is not None:
                    if type(item_id) is not str or not item_id:
                        error("invalid item id")
                        continue
                    if item_id in completed_ids:
                        error("item activity after completion or duplicate completed item id")
                    if kind == "item.started":
                        if item_id in open_items:
                            error("duplicate started item id")
                        else:
                            open_items[item_id] = item_type
                    elif kind == "item.updated":
                        if item_id not in open_items:
                            error("updated item has no active start")
                        elif open_items[item_id] != item_type:
                            error("item type changed during lifecycle")
                    else:
                        if item_id in open_items and open_items[item_id] != item_type:
                            error("item type changed during lifecycle")
                        open_items.pop(item_id, None)
                        completed_ids.add(item_id)
                # Complete-only adapters remain supported. Once a start is
                # observed, that item's identity/type and completion must agree.
                if kind != "item.completed":
                    continue
                if item_type == "agent_message":
                    if not isinstance(item.get("text"), str):
                        error("agent_message lacks text")
                    else:
                        try:
                            item["text"].encode("utf-8")
                            answers.append(item["text"])
                        except UnicodeError:
                            error("agent_message contains invalid Unicode")
    if completions and open_items:
        error("completed turn contains unfinished started items")
    usage = usages[0]["usage"] if len(usages) == 1 and usages[0]["valid"] else None
    if errors:
        terminal = "malformed_events"
    elif failed:
        terminal = "provider_error"
    elif completions == 0:
        terminal = "missing_completion"
    elif completions != 1:
        terminal = "multiple_completions"
    elif len(usages) != 1 or usage is None or usages[0]["type"] != "turn.completed":
        terminal = "invalid_usage"
    elif len(answers) == 0:
        terminal = "missing_answer"
    elif len(answers) != 1:
        terminal = "multiple_answers"
    elif not answers[0].strip():
        terminal = "empty_answer"
    elif tool_items:
        terminal = "tool_violation"
    else:
        terminal = "eligible"
    return {
        "event_terminal": terminal, "usage": usage, "usage_observations": usages,
        "usage_accounting": "reported_once" if usage is not None else ("ambiguous" if usages else "missing"),
        "event_count": event_count, "completion_count": completions,
        "answer_count": len(answers), "tool_items": tool_items, "event_errors": errors,
        "unfinished_item_ids": sorted(open_items),
        "provider_errors": failed, "answer": answers[0] if terminal == "eligible" else None,
    }
