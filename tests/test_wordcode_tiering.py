"""Independent process-boundary probes for WK1 tier preparation.

Synthetic workers below are intentionally tiny frozen protocol fixtures. Their
claimed routes are mocks, never native execution evidence. Real-worker tests run
only when FLUXR_WORDCODE_WORKER identifies the separately built Rust executable.
All execution belongs to the external resource coordinator; this file neither
builds candidates nor alters the frozen generated-candidate replay allowlist.
"""
from dataclasses import replace
import errno
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import wordcode_tiering as tier

SOURCE = "wk1 core v2 pub calc 1 x int int ret up x"
DICTIONARY = "d" * 64
LIMITS = {"instructions": 1000, "cells": 1000, "frames": 8}

# No source supplied by the caller is evaluated by this fixture. MODE changes
# one prescribed protocol/resource fault after the initial immutable request.
FIXTURE = r'''
import errno, hashlib, json, os, struct, subprocess, sys, time
MODE = __MODE__
VALUE_OFFSET = __OFFSET__

def frame():
    h=sys.stdin.buffer.read(4)
    if not h:return None
    if len(h)!=4:raise SystemExit(3)
    n=struct.unpack(">I",h)[0]
    if not 0<n<=524288:raise SystemExit(4)
    b=sys.stdin.buffer.read(n)
    if len(b)!=n:raise SystemExit(5)
    return json.loads(b)

def send(value):
    data=json.dumps(value,separators=(",", ":")).encode()
    sys.stdout.buffer.write(struct.pack(">I",len(data))+data)
    sys.stdout.buffer.flush()

init=frame()
if init is None:raise SystemExit(6)
prepared=init["mode"]=="prepared"
if prepared and MODE=="compile_hang":time.sleep(20)
if prepared and MODE=="cpu_limit":
    while True:pass
if prepared and MODE=="memory_limit":hold=bytearray(2**30)
if prepared and MODE=="stdout_flood":
    while True:os.write(1,b"x"*8192)
if prepared and MODE=="stderr_flood":
    while True:os.write(2,b"x"*8192)
if prepared and MODE=="oversized_frame":
    os.write(1,struct.pack(">I",524289));time.sleep(20)
if prepared and MODE=="truncated_frame":
    os.write(1,struct.pack(">I",100)+b"{}");raise SystemExit(0)
if prepared and MODE=="duplicate_key":
    payload=b'{"protocol":1,"protocol":1,"kind":"ready"}'
    os.write(1,struct.pack(">I",len(payload))+payload);time.sleep(20)
if prepared and MODE=="deep_json":
    payload=b'{"protocol":1,"deep":'+b'['*1500+b'0'+b']'*1500+b'}'
    os.write(1,struct.pack(">I",len(payload))+payload);time.sleep(20)
artifact=hashlib.sha256(init["source"].encode()).hexdigest()
now=time.time_ns()
ready={"protocol":1,"kind":"ready","generation":init["generation"],
 "source_sha256":init["source_sha256"],"dictionary_sha256":init["dictionary_sha256"],
 "module_sha256":artifact,"expanded_sha256":artifact,"export":init["export"],
 "mode":init["mode"],"typed_jit_feature":True,"dictionary_seconds":0,"expansion_seconds":0,
 "preparation_seconds":0,"max_calls":65536,"max_frame_bytes":524288,
 "initialization_started_unix_ns":now,"preparation_started_unix_ns":now,"ready_unix_ns":now}
if prepared:
    if MODE=="wrong_artifact":ready["module_sha256"]="f"*64
    if MODE=="wrong_generation":ready["generation"]+=1
    if MODE=="bool_generation":ready["generation"]=True
    if MODE=="bool_protocol":ready["protocol"]=True
    if MODE=="wrong_limit":ready["max_calls"]=0
    if MODE=="wrong_feature_type":ready["typed_jit_feature"]="true"
    if MODE=="large_ready_duration":ready["dictionary_seconds"]=10**400
    if MODE=="bool_ready_timestamp":ready["ready_unix_ns"]=True
    if MODE=="reversed_ready_timestamp":ready["initialization_started_unix_ns"]=now+1
    if MODE=="extra_ready":ready["trusted"]=True
send(ready)
if prepared and MODE=="unsolicited":
    send({"protocol":1,"kind":"result","generation":init["generation"],"sequence":1,
          "module_sha256":artifact,"route":"TypedJit","value":{"i64":999},
          "error":None,"execution_seconds":0})
if prepared and MODE=="partial_unsolicited":
    # This is never a permitted worker frame: bytes appear before the request.
    # Keep the incomplete header idle so the host must reject it immediately.
    time.sleep(.05);os.write(1,b"\x00\x00");time.sleep(20)
while True:
    request=frame()
    if request is None or request.get("op")=="close":break
    if prepared and MODE=="call_hang":time.sleep(20)
    value=request["arguments"][0]["i64"]+VALUE_OFFSET
    if MODE=="process_probe":
        try:
            subprocess.run(["/bin/true"],check=True)
            value=0
        except OSError as error:value=error.errno
    result={"protocol":1,"kind":"result","generation":init["generation"],
            "sequence":request["sequence"],"module_sha256":artifact,
            "route":"TypedJit" if prepared else "Portable","value":{"i64":value},
            "error":None,"execution_seconds":0}
    if prepared:
        if MODE=="wrong_sequence":result["sequence"]+=1
        if MODE=="both_outcomes":result["error"]="forged"
        if MODE=="no_outcome":result["value"]=None
        if MODE=="missing_outcome":del result["value"]
        if MODE=="bad_value":result["value"]={"i64":True}
        if MODE=="bad_error":result["value"]=None;result["error"]={"forged":True}
        if MODE=="extra_result":result["authority"]="mine"
        if MODE=="bool_sequence":result["sequence"]=True
        if MODE=="large_result_duration":result["execution_seconds"]=10**400
    send(result)
'''


def fixture(path, mode="normal", offset=1):
    source = "#!" + sys.executable + "\n" + FIXTURE.replace("__MODE__", repr(mode)).replace("__OFFSET__", str(offset))
    path.write_text(source)
    path.chmod(0o700)
    return path


def await_promotion(session, ticket, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if session.poll(ticket):
            return
        time.sleep(.005)
    raise AssertionError("preparation did not reach a terminal state within the test bound")


@unittest.skipUnless(sys.platform == "linux" and platform.machine() == "x86_64", "qualified worker supervisor is Linux x86_64 only")
class ProcessBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="wk1-independent-")
        self.root = Path(self.directory.name)
        self.sessions = []
        self.bounds = tier.Bounds(compile_seconds=2, call_seconds=.5, session_seconds=20,
                                  address_bytes=256 * 1024**2, cpu_seconds=3, max_calls=32)

    def tearDown(self):
        failures = []
        for session in self.sessions:
            try:
                session.close()
            except Exception as error:
                failures.append(repr(error))
        self.directory.cleanup()
        self.assertEqual(failures, [], "all fixture workers must be reaped")

    def session(self, mode="normal", bounds=None):
        binary = fixture(self.root / ("fixture-" + str(len(self.sessions)) + ".py"), mode)
        session = tier.TieredSession(binary, SOURCE, DICTIONARY, "calc", bounds or self.bounds)
        self.sessions.append(session)
        return session

    def result(self, session, expected=4):
        result = session.call([{"i64": 3}], limits=LIMITS)
        self.assertIsNone(result["error"])
        self.assertEqual(result["value"], {"i64": expected})
        return result

    def test_portable_remains_callable_until_exact_future_call_promotion(self):
        session = self.session()
        self.assertEqual(self.result(session)["tier"], "portable_process")
        ticket = session.begin_compile()
        with self.assertRaises(tier.TierError):
            session.begin_compile()
        self.assertEqual(self.result(session)["tier"], "portable_process")
        await_promotion(session, ticket)
        self.assertEqual(self.result(session)["tier"], "prepared_process")
        with self.assertRaises(tier.TierError):
            session.poll(ticket)

    def test_cancel_reaps_worker_and_stale_ticket_cannot_publish(self):
        session = self.session("compile_hang")
        ticket = session.begin_compile()
        worker = session.pending
        session.cancel(ticket)
        self.assertIsNotNone(worker.process.returncode)
        self.assertTrue(worker.closed)
        self.assertIsNone(session.pending)
        self.assertIsNone(session.native)
        with self.assertRaises(tier.TierError):
            session.poll(ticket)
        self.assertEqual(self.result(session)["tier"], "portable_process")
        replacement = session.begin_compile()
        with self.assertRaises(tier.TierError):
            session.cancel(ticket)
        session.cancel(replacement)

    def test_mutating_returned_receipt_cannot_change_live_policy_or_ticket(self):
        session = self.session("compile_hang")
        ticket = session.begin_compile()
        receipt = session.receipt()
        receipt["bounds"]["address_bytes"] = 1
        for event in receipt["events"]:
            if "ticket" in event:
                event["ticket"]["generation"] = -1
        receipt["events"].clear()
        self.assertEqual(session.bounds.address_bytes, self.bounds.address_bytes)
        self.assertEqual(ticket.generation, 1)
        self.assertGreater(len(session.receipt()["events"]), 0)
        session.cancel(ticket)

    def test_executable_path_replacement_cannot_change_sealed_worker_bytes(self):
        session = self.session()
        frozen = session.binary_sha256
        fixture(session.binary, "normal", offset=999)
        self.assertNotEqual(hashlib.sha256(session.binary.read_bytes()).hexdigest(), frozen)
        ticket = session.begin_compile()
        await_promotion(session, ticket)
        self.assertEqual(self.result(session)["value"], {"i64": 4})
        self.assertEqual(session.receipt()["binary_sha256"], frozen)

    def test_hard_wall_timeout_is_failure_and_reaps_before_fallback(self):
        session = self.session("compile_hang", replace(self.bounds, compile_seconds=.75))
        start = time.monotonic(); ticket = session.begin_compile(); worker = session.pending
        with self.assertRaises(tier.TierError):
            await_promotion(session, ticket)
        self.assertLess(time.monotonic() - start, 3)
        self.assertIsNotNone(worker.process.returncode)
        self.assertIsNone(session.native)
        self.assertEqual(self.result(session)["tier"], "portable_process")

    def test_cpu_and_address_space_failures_keep_portable_revision(self):
        for mode in ("cpu_limit", "memory_limit"):
            with self.subTest(mode=mode):
                session = self.session(mode, replace(self.bounds, compile_seconds=4, cpu_seconds=1))
                ticket = session.begin_compile(); worker = session.pending
                with self.assertRaises(tier.TierError):
                    await_promotion(session, ticket)
                self.assertIsNotNone(worker.process.returncode)
                self.assertIsNone(session.native)
                self.assertEqual(self.result(session)["tier"], "portable_process")
                session.close()

    def test_seccomp_rejects_child_creation_with_eperm(self):
        session = self.session("process_probe")
        result = session.call([{"i64": 3}], limits=LIMITS)
        self.assertEqual(result["value"], {"i64": errno.EPERM})

    def test_malformed_binding_and_frame_cases_never_promote(self):
        for mode in ("wrong_artifact", "wrong_generation", "bool_generation", "bool_protocol",
                     "wrong_limit", "wrong_feature_type", "large_ready_duration", "bool_ready_timestamp",
                     "reversed_ready_timestamp", "extra_ready", "oversized_frame",
                     "truncated_frame", "duplicate_key", "deep_json", "stdout_flood", "stderr_flood"):
            with self.subTest(mode=mode):
                session = self.session(mode)
                ticket = session.begin_compile(); worker = session.pending
                with self.assertRaises(tier.TierError):
                    await_promotion(session, ticket)
                self.assertIsNone(session.native)
                self.assertIsNotNone(worker.process.returncode)
                self.assertEqual(self.result(session)["tier"], "portable_process")
                session.close()

    def test_forged_result_outcomes_fail_and_are_not_retried(self):
        for mode in ("wrong_sequence", "both_outcomes", "no_outcome", "missing_outcome",
                     "bad_value", "bad_error", "extra_result", "bool_sequence", "large_result_duration", "call_hang"):
            with self.subTest(mode=mode):
                session = self.session(mode)
                ticket = session.begin_compile(); await_promotion(session, ticket)
                native = session.native
                with self.assertRaises(tier.TierError):
                    session.call([{"i64": 3}], limits=LIMITS)
                self.assertIsNone(session.native)
                self.assertIsNotNone(native.process.returncode)
                # Failed prepared call was not automatically retried on portable.
                self.assertEqual(session.portable.sequence, 0)
                self.assertEqual(self.result(session)["tier"], "portable_process")
                session.close()

    def test_unsolicited_result_is_rejected_before_success(self):
        session = self.session("unsolicited")
        ticket = session.begin_compile()
        try:
            await_promotion(session, ticket)
        except tier.TierError:
            self.assertIsNone(session.native)
        else:
            # The fixture may arrive between the bounded readiness check and the
            # next host call. A short bounded delay puts the unsolicited frame
            # in the idle channel before the legitimate request.
            time.sleep(.05)
            with self.assertRaises(tier.TierError):
                session.call([{"i64": 3}], limits=LIMITS)
            self.assertIsNone(session.native)

    def test_incomplete_idle_bytes_refuse_before_a_call_is_sent(self):
        session = self.session("partial_unsolicited")
        ticket = session.begin_compile(); worker = session.pending
        try:
            await_promotion(session, ticket)
        except tier.TierError:
            # A delayed host may already observe the extra bytes with ready.
            self.assertEqual(worker.sequence, 0)
            self.assertIsNotNone(worker.process.returncode)
            self.assertIsNone(session.native)
            return
        time.sleep(.1)
        with self.assertRaises(tier.TierError):
            session.call([{"i64": 3}], limits=LIMITS)
        self.assertEqual(worker.sequence, 0, "unsolicited bytes must reject before request admission")
        self.assertIsNotNone(worker.process.returncode)
        self.assertIsNone(session.native)

    def test_oversized_cyclic_and_invalid_calls_refuse_before_sequence_or_deadline(self):
        session = self.session()
        cycle = []; cycle.append(cycle)
        bad = [cycle, ["x" * (tier.MAX_FRAME + 1)], [{"i64": 2**200}], [float("nan")], [object()]]
        for arguments in bad:
            with self.subTest(kind=type(arguments[0]).__name__):
                before = session.portable.sequence
                with self.assertRaises(tier.TierError):
                    session.call(arguments, limits=LIMITS)
                self.assertEqual(session.portable.sequence, before)
                self.assertIsNone(session.portable.deadline)
                self.assertIsNone(session.portable.reason)
        self.result(session)

    def test_event_exhaustion_refuses_admission_and_preserves_close_slot(self):
        session = self.session()
        session.events = [{"event": "retained_test_event"} for _ in range(4095)]
        with patch.object(session, "_spawn") as spawn:
            with self.assertRaises(tier.TierError):
                session.begin_compile()
            spawn.assert_not_called()
        self.assertIsNone(session.pending)
        self.assertIsNone(session.ticket)
        session.close()
        self.assertEqual(len(session.receipt()["events"]), 4096)
        self.assertTrue(session.closed)

    def test_launch_failure_has_terminal_event_and_keeps_portable_worker(self):
        session = self.session()
        with patch.object(session, "_spawn", side_effect=OSError("injected spawn refusal")):
            with self.assertRaises((OSError, tier.TierError)):
                session.begin_compile()
        self.assertIsNone(session.pending)
        self.assertIsNone(session.ticket)
        self.assertTrue(any(event["event"] == "compile_spawn_failed" for event in session.receipt()["events"]))
        self.result(session)

    def test_postspawn_watchdog_start_failure_reaps_child_and_clears_ticket(self):
        session = self.session()
        spawned = []
        original = tier.subprocess.Popen
        def record_spawn(*args, **kwargs):
            process = original(*args, **kwargs)
            spawned.append(process)
            return process
        with patch.object(tier.subprocess, "Popen", side_effect=record_spawn):
            with patch.object(tier.threading.Thread, "start", side_effect=RuntimeError("injected thread admission failure")):
                with self.assertRaises((RuntimeError, tier.TierError)):
                    session.begin_compile()
        self.assertEqual(len(spawned), 1)
        self.assertIsNotNone(spawned[0].returncode)
        self.assertIsNone(session.pending)
        self.assertIsNone(session.ticket)
        self.assertTrue(any(event["event"] == "compile_spawn_failed" for event in session.receipt()["events"]))
        self.result(session)

    def test_call_capacity_is_finite_and_close_reaps_both_workers(self):
        session = self.session(bounds=replace(self.bounds, max_calls=2))
        self.result(session); self.result(session)
        with self.assertRaises(tier.TierError):
            self.result(session)
        ticket = session.begin_compile(); await_promotion(session, ticket)
        workers = (session.portable, session.native)
        session.close()
        self.assertTrue(all(worker.closed and worker.process.returncode is not None for worker in workers))

    def test_compile_tickets_are_session_owned_opaque_handles(self):
        # Frozen Python workers test host lifecycle admission only. Their echoed
        # exports/routes are mocks, not FluxR or native execution evidence.
        policy = replace(self.bounds, compile_seconds=5)

        def snapshot(session):
            return (session.pending, session.ticket, session.generation,
                    session.native, session.closed, session.receipt()["events"])

        def unchanged(session, before):
            self.assertIs(session.pending, before[0])
            self.assertIs(session.ticket, before[1])
            self.assertEqual(session.generation, before[2])
            self.assertIs(session.native, before[3])
            self.assertEqual(session.closed, before[4])
            self.assertEqual(session.receipt()["events"], before[5])

        for other_export in ("calc", "another_export"):
            with self.subTest(other_export=other_export):
                left = self.session("compile_hang", bounds=policy)
                right = tier.TieredSession(left.binary, SOURCE, DICTIONARY,
                                          other_export, policy)
                self.sessions.append(right)
                left_ticket = left.begin_compile()
                right_ticket = right.begin_compile()
                self.assertIsNot(left_ticket, right_ticket)
                self.assertEqual(left_ticket, right_ticket,
                                 "equal diagnostic fields must not grant another session's handle")

                for target, original, foreign in ((left, left_ticket, right_ticket),
                                                   (right, right_ticket, left_ticket)):
                    copied = replace(original)
                    self.assertIsNot(copied, original)
                    self.assertEqual(copied, original)
                    for handle_name, handle in (("copied", copied), ("foreign_session", foreign),
                                                ("absent", None), ("foreign_object", object())):
                        for operation in ("poll", "cancel"):
                            with self.subTest(target_export=target.export,
                                              handle=handle_name, operation=operation):
                                before_left, before_right = snapshot(left), snapshot(right)
                                with self.assertRaises(tier.TierError):
                                    getattr(target, operation)(handle)
                                unchanged(left, before_left)
                                unchanged(right, before_right)

                # The real handles remain live after every rejected alias/copy.
                # Prepared fixtures deliberately hang, so legitimate poll is false.
                self.assertFalse(left.poll(left_ticket))
                self.assertFalse(right.poll(right_ticket))
                workers = (left.pending, right.pending)
                left.cancel(left_ticket)
                right.cancel(right_ticket)
                for session, worker in zip((left, right), workers):
                    self.assertIsNone(session.pending)
                    self.assertIsNone(session.ticket)
                    self.assertIsNone(session.native)
                    self.assertEqual(session.generation, 2)
                    self.assertTrue(worker.closed)
                    self.assertIsNotNone(worker.process.returncode)
                    self.assertEqual(sum(event["event"] == "compile_cancelled_and_reaped"
                                         for event in session.receipt()["events"]), 1)
                    # No pending handle exists now; None must still refuse with
                    # TierError before attempting to read any ticket attributes.
                    for absent in (None, object()):
                        for operation in ("poll", "cancel"):
                            before = snapshot(session)
                            with self.assertRaises(tier.TierError):
                                getattr(session, operation)(absent)
                            unchanged(session, before)
                    self.assertEqual(self.result(session)["tier"], "portable_process")
                    # Keep at most two sessions live for the next export case.
                    # tearDown also owns these registered sessions on any failure.
                    session.close()


class BoundValidationTests(unittest.TestCase):
    def test_nonfinite_boolean_zero_and_overlarge_policy_values_refuse(self):
        for policy in (tier.Bounds(compile_seconds=float("nan")), tier.Bounds(call_seconds=True),
                       tier.Bounds(session_seconds=0), tier.Bounds(cpu_seconds=True),
                       tier.Bounds(address_bytes=1), tier.Bounds(max_calls=65537),
                       tier.Bounds(compile_seconds=10**400)):
            with self.assertRaises(tier.TierError):
                policy.validate()



REAL_WORKER = os.environ.get("FLUXR_WORDCODE_WORKER")


@unittest.skipUnless(REAL_WORKER, "actual Rust worker execution requires FLUXR_WORDCODE_WORKER")
class ActualWorkerTests(unittest.TestCase):
    def test_actual_fluxr_portable_to_prepared_route_and_exact_budget_observations(self):
        binary = Path(REAL_WORKER).resolve(strict=True)
        info = json.loads(subprocess.check_output([str(binary), "dictionary"], timeout=10))
        with tier.TieredSession(binary, SOURCE, info["dictionary_sha256"], "calc") as session:
            frozen = []
            for fuel in range(0, 15):
                limits = {"instructions": fuel, "cells": 10, "frames": 2}
                row = session.call([{"i64": 3}], limits=limits)
                self.assertEqual(row["route"], "Portable")
                frozen.append((limits, row["value"], row["error"]))
            ticket = session.begin_compile(); await_promotion(session, ticket)
            native_success = 0
            for limits, expected_value, expected_error in frozen:
                row = session.call([{"i64": 3}], limits=limits)
                self.assertEqual((row["value"], row["error"]), (expected_value, expected_error))
                if row["error"] is None:
                    self.assertEqual(row["value"], {"i64": 4})
                    if session.native.ready["typed_jit_feature"]:
                        self.assertEqual(row["route"], "TypedJit")
                        native_success += 1
            if session.native.ready["typed_jit_feature"]:
                self.assertGreater(native_success, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
