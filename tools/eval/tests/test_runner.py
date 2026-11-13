import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
import bench_heldout_live as live
from eval.runner import (MAX_EVENT_LINE_BYTES, ProcessResult, file_digest,
                         parse_events, run_bounded, strict_json)

FAKE = r'''
import json, os, subprocess, sys, time
mode = sys.stdin.read()
answer = {"type":"item.completed", "item":{"id":"a", "type":"agent_message", "text":"v2 (pub calc () i64 () (ret 1))"}}
complete = {"type":"turn.completed", "usage":{"input_tokens":7,"cached_input_tokens":2,"output_tokens":3}}
def emit(value): print(json.dumps(value), flush=True)
if mode == "timeout": time.sleep(10)
if mode == "flood":
    while True: os.write(1, b"x" * 8192)
if mode == "descendant":
    subprocess.Popen([sys.executable, "-c", "import time; time.sleep(10)"])
    sys.exit(0)
if mode == "empty": answer["item"]["text"] = " \n\t"
if mode != "missing_answer": emit(answer)
if mode == "multiple_answers":
    answer["item"]["id"] = "b"
    emit(answer)
if mode == "tool": emit({"type":"item.completed","item":{"id":"tool","type":"command_execution"}})
if mode == "usage_bool": complete["usage"]["input_tokens"] = True
if mode not in ("missing_completion", "error"): emit(complete)
if mode == "duplicate": emit(complete)
if mode == "malformed": print('{"type":')
if mode == "invalid_utf8": os.write(1, b"\xff\n")
if mode == "error": emit({"type":"turn.failed","error":{"message":"provider failed"},"usage":complete["usage"]})
if mode == "exit": sys.exit(17)
'''


@unittest.skipUnless(os.name == "posix", "bounded provider runner supports POSIX")
class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.provider = self.root / "provider"
        self.provider.write_text(f"#!{sys.executable}\n" + FAKE)
        self.provider.chmod(0o700)
        self.settings = {"model": "offline-fixture", "effort": "low", "timeout_seconds": 0.5,
                         "max_event_bytes": 4096, "max_stderr_bytes": 1024}

    def attempt(self, mode, **settings):
        return live.collect_attempt(str(self.provider), self.root,
                                    {**self.settings, **settings}, mode, mode)

    def test_terminal_statuses_and_usage_survive_failure(self):
        expected = {"ok": "eligible", "empty": "empty_answer", "missing_answer": "missing_answer",
                    "multiple_answers": "multiple_answers", "missing_completion": "missing_completion",
                    "duplicate": "multiple_completions", "malformed": "malformed_events",
                    "invalid_utf8": "malformed_events", "tool": "tool_violation",
                    "usage_bool": "invalid_usage", "error": "provider_error", "exit": "nonzero_exit"}
        for mode, terminal in expected.items():
            with self.subTest(mode=mode):
                status = self.attempt(mode)
                self.assertEqual(status["terminal"], terminal)
                self.assertEqual(status["eligible_for_replay"], terminal == "eligible")
                self.assertFalse(status["candidate_execution_authorized"])
                self.assertTrue((self.root / mode / "status.json").exists())
                if mode not in ("missing_completion", "usage_bool", "duplicate"):
                    self.assertEqual(status["usage"]["input_tokens"], 7)
        self.assertEqual(len(json.loads((self.root / "duplicate/status.json").read_text())["usage_observations"]), 2)

    def test_launch_error_is_terminal(self):
        status = live.collect_attempt(str(self.root / "missing"), self.root, self.settings, "launch", "ok")
        self.assertEqual(status["terminal"], "launch_error")
        self.assertFalse(status["provider_process_started"])

    def test_timeout_and_descendant_pipe_are_bounded(self):
        for mode in ("timeout", "descendant"):
            with self.subTest(mode=mode):
                status = self.attempt(mode, timeout_seconds=0.15)
                self.assertEqual(status["terminal"], "timeout")
                self.assertLess(status["seconds"], 3)

    def test_output_flood_keeps_bounded_evidence(self):
        status = self.attempt("flood")
        self.assertEqual(status["terminal"], "output_limit")
        self.assertEqual((self.root / "flood/events.jsonl").stat().st_size, 4096)

    def test_failure_does_not_drop_remaining_arms_and_resume_invokes_nobody(self):
        schedule = [("first", "exit"), ("second", "ok"), ("third", "malformed")]
        directory = self.root / "run"
        result = live.run_scheduled(str(self.provider), directory, self.settings, schedule, seal={"v": 1})
        self.assertEqual((result["scheduled"], result["complete"], result["explicit_failure"], result["pending"]),
                         (3, 1, 2, 0))
        before = {path: file_digest(path) for path in directory.glob("*/events.jsonl")}
        with mock.patch.object(live, "collect_attempt", side_effect=AssertionError("provider repeated")):
            resumed = live.run_scheduled(str(self.provider), directory, self.settings, schedule,
                                         seal={"v": 1}, resume=True)
        self.assertEqual(result, resumed)
        self.assertEqual(before, {path: file_digest(path) for path in before})

    def test_resume_rejects_changed_evidence_or_manifest(self):
        directory = self.root / "run"
        schedule = [("one", "ok")]
        live.run_scheduled(str(self.provider), directory, self.settings, schedule, seal={})
        with self.assertRaisesRegex(ValueError, "manifest"):
            live.run_scheduled(str(self.provider), directory, self.settings, [("one", "empty")],
                               seal={}, resume=True)
        (directory / "one/events.jsonl").write_text("changed\n")
        with self.assertRaisesRegex(ValueError, "artifact changed"):
            live.run_scheduled(str(self.provider), directory, self.settings, schedule, seal={}, resume=True)

    def test_interrupted_attempt_is_not_automatically_repeated(self):
        directory = self.root / "run"
        schedule = [("one", "ok")]
        live.run_scheduled(str(self.provider), directory, self.settings, schedule, seal={})
        (directory / "one/status.json").unlink()
        with mock.patch.object(live, "collect_attempt", side_effect=AssertionError("provider repeated")):
            resumed = live.run_scheduled(str(self.provider), directory, self.settings, schedule,
                                         seal={}, resume=True)
        self.assertEqual(resumed["attempts"], {"one": "interrupted"})
        self.assertEqual(resumed["complete"], 0)
        self.assertEqual(resumed["usage_by_attempt"]["one"]["accounting"], "unknown_after_interruption")
        self.assertEqual(resumed["usage_by_attempt"]["one"]["usage"]["input_tokens"], 7)

    def test_cancellation_finalizes_unlaunched_arms_without_retry(self):
        directory = self.root / "run"
        schedule = [("first", "ok"), ("second", "ok"), ("third", "ok")]
        def cancelled(command, payload, cwd, stdout, stderr, **kwargs):
            stdout.write_bytes(b"")
            stderr.write_bytes(b"")
            return ProcessResult("cancelled", -9, 0.01, True)
        with mock.patch.object(live, "run_bounded", side_effect=cancelled) as provider:
            result = live.run_scheduled(str(self.provider), directory, self.settings, schedule, seal={})
        self.assertEqual(provider.call_count, 1)
        self.assertEqual(result["attempts"], {"first": "cancelled", "second": "cancelled_before_launch",
                                             "third": "cancelled_before_launch"})
        self.assertEqual((result["scheduled"], result["explicit_failure"], result["pending"]), (3, 3, 0))
        for key in ("second", "third"):
            status = json.loads((directory / key / "status.json").read_text())
            self.assertFalse(status["provider_process_started"])
            self.assertEqual(status["usage_accounting"], "not_invoked")
            self.assertEqual(status["usage"]["input_tokens"], 0)
        with mock.patch.object(live, "collect_attempt", side_effect=AssertionError("provider repeated")):
            resumed = live.run_scheduled(str(self.provider), directory, self.settings, schedule,
                                         seal={}, resume=True)
        self.assertEqual(result, resumed)

    def test_controller_error_does_not_omit_following_attempt(self):
        real = live.collect_attempt
        def collect(*args):
            if args[3] == "first":
                raise RuntimeError("offline controller fault")
            return real(*args)
        with mock.patch.object(live, "collect_attempt", side_effect=collect):
            result = live.run_scheduled(str(self.provider), self.root / "run", self.settings,
                                        [("first", "ok"), ("second", "ok")], seal={})
        self.assertEqual(result["attempts"], {"first": "controller_error", "second": "eligible"})

    def test_invalid_attempt_key_cannot_escape_evidence_root(self):
        with self.assertRaises(ValueError):
            live.run_scheduled(str(self.provider), self.root / "run", self.settings,
                               [("../escape", "ok")], seal={})
        self.assertFalse((self.root / "escape").exists())


class ParserTests(unittest.TestCase):
    def parse(self, events):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events"
            path.write_text("".join(json.dumps(event) + "\n" for event in events))
            return parse_events(path)

    @staticmethod
    def finish_events():
        return [{"type": "item.completed", "item": {"id": "answer", "type": "agent_message", "text": "source"}},
                {"type": "turn.completed", "usage": {"input_tokens": 7, "output_tokens": 3}}]

    def test_unfinished_tool_attempt_is_retained_and_cannot_be_eligible(self):
        parsed = self.parse([{"type": "item.started", "item": {"id": "tool", "type": "command_execution"}}]
                            + self.finish_events())
        self.assertEqual("malformed_events", parsed["event_terminal"])
        self.assertEqual(["command_execution"], parsed["tool_items"])
        self.assertEqual(["tool"], parsed["unfinished_item_ids"])
        self.assertEqual(7, parsed["usage"]["input_tokens"])
        self.assertIsNone(parsed["answer"])

    def test_complete_tool_lifecycle_is_still_a_tool_violation(self):
        item = {"id": "tool", "type": "command_execution"}
        parsed = self.parse([{"type": phase, "item": item} for phase in
                             ("item.started", "item.updated", "item.completed")] + self.finish_events())
        self.assertEqual("tool_violation", parsed["event_terminal"])
        self.assertEqual(["command_execution"] * 3, parsed["tool_items"])
        self.assertEqual([], parsed["unfinished_item_ids"])

    def test_non_tool_stream_and_complete_only_adapter_both_remain_eligible(self):
        thought = {"id": "thought", "type": "reasoning"}
        for prefix in ([], [{"type": phase, "item": thought} for phase in
                            ("item.started", "item.updated", "item.completed")]):
            with self.subTest(prefix=prefix):
                parsed = self.parse(prefix + self.finish_events())
                self.assertEqual("eligible", parsed["event_terminal"])
                self.assertEqual([], parsed["unfinished_item_ids"])
                self.assertEqual("source", parsed["answer"])
        # Existing adapters may omit identifiers on complete-only messages.
        events = self.finish_events()
        del events[0]["item"]["id"]
        self.assertEqual("eligible", self.parse(events)["event_terminal"])

    def test_incomplete_rebound_duplicate_or_late_item_lifecycle_refuses(self):
        item = {"id": "thought", "type": "reasoning"}
        start = {"type": "item.started", "item": item}
        update = {"type": "item.updated", "item": item}
        completed = {"type": "item.completed", "item": item}
        changed = {"type": "item.completed", "item": {"id": "thought", "type": "agent_message", "text": "changed"}}
        for prefix in ([start], [start, start, completed], [start, changed], [update],
                       [completed, update], [completed, start], [completed, completed]):
            with self.subTest(prefix=prefix):
                parsed = self.parse(prefix + self.finish_events())
                self.assertEqual("malformed_events", parsed["event_terminal"])
                self.assertEqual(7, parsed["usage"]["input_tokens"])

    def test_malformed_item_and_unhashable_event_types_fail_closed(self):
        for event in ({"type": "item.started"}, {"type": "item.updated", "item": []},
                      {"type": "item.started", "item": {"type": "reasoning"}},
                      {"type": "item.started", "item": {"id": [], "type": "reasoning"}},
                      {"type": "item.completed", "item": {"id": "", "type": "reasoning"}},
                      {"type": []}, {"type": {}}):
            with self.subTest(event=event):
                self.assertEqual("malformed_events", self.parse([event] + self.finish_events())["event_terminal"])

    def test_completion_seals_turn_and_forbids_late_answers_or_restarts(self):
        answer = {"type": "item.completed", "item": {"type": "agent_message", "text": "answer"}}
        complete = {"type": "turn.completed", "usage": {"input_tokens": 1, "output_tokens": 1}}
        for events in ([complete, answer], [answer, complete, {"type": "turn.started"}],
                       [answer, {"type": "turn.started"}, complete],
                       [answer, complete, {"type": "thread.started", "thread_id": "late"}],
                       [{"type": "turn.failed"}, answer, complete]):
            with self.subTest(events=events), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "events"
                path.write_text("".join(json.dumps(event) + "\n" for event in events))
                result = parse_events(path)
                self.assertEqual(result["event_terminal"], "malformed_events")
                self.assertEqual(len(result["usage_observations"]), 1)

    def test_duplicate_keys_and_nonfinite_numbers_reject(self):
        for value in ('{"x":1,"x":2}', '{"x":NaN}', '{"x":Infinity}', '{"x":1e999}'):
            with self.assertRaises(ValueError):
                strict_json(value)

    def test_long_event_line_stops_before_parsing(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events"
            path.write_bytes(b"x" * (MAX_EVENT_LINE_BYTES + 2))
            parsed = parse_events(path)
            self.assertEqual(parsed["event_terminal"], "malformed_events")


if __name__ == "__main__":
    unittest.main()
