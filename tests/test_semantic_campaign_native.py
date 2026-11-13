"""Real compiled execution through a controlled source-fault boundary.

The wrapper below changes addition to subtraction before the existing Rust
bridge compiles each requested source. It forwards actual bridge observations;
it does not import an oracle or calculate expected values. This simulates a
lowering fault at a test boundary while leaving runtime/compiler sources and
the compiled bridge unchanged. The test qualifies detection, reduction and
replay of this bounded fault witness, not arbitrary compiler mutations.
"""
import os
from pathlib import Path
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import semantic_campaign as campaign


MAX_I64 = (1 << 63) - 1
ARITHMETIC = "invalid or overflowing integer arithmetic"


def source_fault_case():
    return {
        "id": "native-add-lowering-fault",
        "program": {
            "params": [{"name": "x", "type": "int"}],
            "locals": [{"name": "work", "type": "int"}],
            "result": "int",
            "body": [
                {"op": "set", "name": "work", "value": {
                    "op": "add", "left": {"op": "var", "name": "x"},
                    "right": {"op": "int", "value": 1},
                }},
                {"op": "return", "value": {
                    "op": "add", "left": {"op": "var", "name": "work"},
                    "right": {"op": "int", "value": 2},
                }},
            ],
        },
        "calls": [{
            "inputs": [], "arguments": [{"i64": value}],
            "limits": {"instructions": 1000, "cells": 1000, "frames": 8},
        } for value in (4, MAX_I64, -3)],
        "expected": [
            {"value": {"i64": 7}},
            {"trap": ARITHMETIC},
            {"value": {"i64": 0}},
        ],
    }


# No semantic implementation or expected-value computation belongs here.
# The child stays in its parent's process group so the bounded runner owns
# both processes if a deadline or output cap is reached.
SOURCE_FAULT_WRAPPER = r'''
import json
import subprocess
import sys

DRIVER = __DRIVER__
request = json.load(sys.stdin)
for case in request["cases"]:
    case["source"] = case["source"].replace("(+ ", "(- ")
result = subprocess.run(
    [DRIVER], input=json.dumps(request, separators=(",", ":")).encode("utf-8"),
    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10, check=False,
)
sys.stdout.buffer.write(result.stdout)
sys.stderr.buffer.write(result.stderr)
raise SystemExit(result.returncode)
'''


@unittest.skipUnless(os.name == "posix", "bounded semantic runner requires POSIX; Windows is not qualified")
@unittest.skipUnless(os.environ.get("FLUXR_SEMANTIC_DRIVER"), "built public-API driver not configured")
class NativeSourceFaultReduction(unittest.TestCase):
    def test_real_source_fault_is_detected_reduced_and_replayed(self):
        driver = Path(os.environ["FLUXR_SEMANTIC_DRIVER"]).expanduser().resolve()
        self.assertTrue(driver.is_file(), "configured compiled driver must exist; absence is not a skip")
        self.assertTrue(os.access(driver, os.X_OK), "configured compiled driver must be executable")
        case = source_fault_case()
        require_native = os.environ.get("FLUXR_SEMANTIC_REQUIRE_NATIVE") == "1"

        # Establish that the unchanged compiler/runtime produces all three
        # independent control outcomes, including a trap followed by success.
        control = campaign.run_cases([case], driver, timeout=15, require_native=require_native)
        self.assertEqual(control["summary"]["status"], "passed")
        self.assertEqual(control["summary"]["comparisons"], 6)
        self.assertEqual(control["mismatches"], [])
        for observation, expected in zip(control["observations"][0]["calls"], case["expected"]):
            for backend in ("portable", "prepared"):
                self.assertTrue(campaign.compare_outcome(expected, observation[backend]))

        temp_root = Path(tempfile.gettempdir()).resolve()
        self.assertNotEqual(temp_root, ROOT, "temporary outputs must stay outside the source tree")
        self.assertNotIn(ROOT, temp_root.parents, "temporary outputs must stay outside the source tree")
        with tempfile.TemporaryDirectory(prefix="fluxr-semantic-native-", dir=temp_root) as directory:
            wrapper = Path(directory) / "source_fault_driver.py"
            wrapper.write_text(
                "#!" + sys.executable + "\n"
                + SOURCE_FAULT_WRAPPER.replace("__DRIVER__", repr(str(driver))),
                encoding="utf-8",
            )
            wrapper.chmod(0o700)

            observed = campaign.run_cases([case], wrapper, timeout=15)
            self.assertEqual(observed["summary"]["status"], "failed")
            self.assertEqual(observed["summary"]["comparisons"], 6)
            self.assertEqual(observed["summary"]["mismatches"], 6)
            # Both paths genuinely ran the mutated source through the compiled
            # bridge. These three faulty results are frozen hand arithmetic.
            wrong_values = [1, MAX_I64 - 3, -6]
            for observation, value in zip(observed["observations"][0]["calls"], wrong_values):
                for backend in ("portable", "prepared"):
                    self.assertEqual(observation[backend], {"value": {"i64": value}})

            signature = campaign.failure_signature(observed, case["id"], 2)
            self.assertEqual({item[0] for item in signature}, {"portable", "prepared"})
            reduced = campaign.reduce_failure(
                observed, wrapper, case["id"], 2, max_attempts=8, timeout=15,
            )
            self.assertEqual(reduced["kind"], "reduction")
            self.assertEqual(reduced["summary"]["status"], "failed")
            self.assertEqual(campaign.failure_signature(reduced, case["id"], 2), signature)
            stats = reduced["reduction"]
            self.assertLessEqual(stats["attempts"], 8)
            self.assertGreater(stats["accepted"], 0, "the concrete fault witness admits a smaller reproducer")
            self.assertLess(stats["reduced_cost"], stats["original_cost"])
            self.assertEqual(stats["prefix_calls_retained"], 3)
            self.assertFalse(stats["globally_minimal"])
            self.assertEqual(len(reduced["corpus"]["cases"]), 1)
            reduced_case = reduced["corpus"]["cases"][0]
            self.assertEqual(reduced_case["id"], case["id"])
            # Arguments may shrink, but all three invocation positions remain
            # in the prefix and are executed again before accepting a change.
            self.assertEqual(len(reduced_case["calls"]), 3)
            self.assertEqual(stats["call"], 2)

            replayed = campaign.replay(reduced, wrapper, timeout=15)
            self.assertEqual(replayed["summary"]["status"], "failed")
            self.assertEqual(replayed["summary"]["comparisons"], 6)
            self.assertEqual(campaign.failure_signature(replayed, case["id"], 2), signature)
            self.assertEqual(replayed["mismatches"], reduced["mismatches"])


if __name__ == "__main__":
    unittest.main()
