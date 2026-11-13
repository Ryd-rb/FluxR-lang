"""External-file and process-boundary checks for the semantic campaign.

The deliberately incorrect synthetic driver tests comparison/replay/reduction
plumbing only. It never evaluates supplied source and is not native semantic
evidence. Real public-API execution is exercised separately when
FLUXR_SEMANTIC_DRIVER names the built semantic_campaign_driver executable.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import uuid


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "semantic_campaign.py"
sys.path.insert(0, str(ROOT / "tools"))
import semantic_campaign as campaign


def reduction_case():
    """A fixed independent witness, not generated expectations from the tool."""
    program = {
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
    }
    return {
        "id": "cli-reduction",
        "program": program,
        "calls": [{
            "inputs": [], "arguments": [{"i64": value}],
            "limits": {"instructions": 1000, "cells": 1000, "frames": 8},
        } for value in (4, 9223372036854775807, -3)],
        "expected": [
            {"value": {"i64": 7}},
            {"trap": "invalid or overflowing integer arithmetic"},
            {"value": {"i64": 0}},
        ],
    }

# This fixture intentionally knows no FluxR semantics. A fixed incorrect
# value exposes comparator failures without borrowing the campaign oracle.
FAKE_DRIVER = r'''
import json, sys
MODE = __MODE__
request = json.load(sys.stdin)
if MODE == "exit":
    print("synthetic driver failure", file=sys.stderr)
    raise SystemExit(7)
if MODE == "malformed":
    print("this is not JSON")
    raise SystemExit(0)
response = {
    "schema": "fluxr-semantic-response-v1",
    "driver": {"typed_jit": False, "os": "synthetic", "arch": "synthetic"},
    "cases": []
}
for case in request["cases"]:
    calls = []
    for call in case["calls"]:
        observed = {"value": {"i64": 9223372036854775807}}
        calls.append({"portable": observed, "prepared": observed, "route": "Portable"})
    response["cases"].append({"id": case["id"], "calls": calls})
if MODE == "missing_call":
    response["cases"][0]["calls"].pop()
if MODE == "wrong_id":
    response["cases"][0]["id"] = "unsolicited-id"
if MODE == "bad_scalar":
    response["cases"][0]["calls"][0]["prepared"] = {"value": {"i64": True}}
if MODE == "both_outcomes":
    response["cases"][0]["calls"][0]["prepared"] = {
        "value": {"i64": 1}, "trap": "module instruction budget exhausted"}
if MODE == "unknown_route":
    response["cases"][0]["calls"][0]["route"] = "PretendNative"
if MODE == "inconsistent_native_feature":
    response["cases"][0]["calls"][0]["route"] = "TypedJit"
print(json.dumps(response, separators=(",", ":")))
'''


class CampaignCliTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="fluxr-semantic-cli-")
        self.root = Path(self.directory.name)

    def tearDown(self):
        self.directory.cleanup()

    def cli(self, *arguments, timeout=30):
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), *map(str, arguments)],
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )

    def assert_exit(self, result, expected):
        self.assertEqual(
            result.returncode,
            expected,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )

    def fake(self, mode="mismatch"):
        path = self.root / f"driver-{mode}.py"
        path.write_text(
            "#!" + sys.executable + "\n" + FAKE_DRIVER.replace("__MODE__", repr(mode)),
            encoding="utf-8",
        )
        path.chmod(0o700)
        return path

    def generate(self, name="corpus.json", seed=243):
        output = self.root / name
        result = self.cli(
            "generate", "--seed", seed, "--programs", 2,
            "--calls-per-program", 3, "--max-depth", 2, "--output", output,
        )
        self.assert_exit(result, 0)
        value = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(value["schema"], "fluxr-semantic-corpus-v1")
        return output, value

    def write_json(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        return path

    def fixed_corpus(self, name="fixed.json"):
        return self.write_json(name, campaign.make_corpus([reduction_case()]))

    def mismatch_report(self, name="mismatch.json"):
        corpus = self.fixed_corpus()
        output = self.root / name
        driver = self.fake()
        result = self.cli("replay", "--input", corpus, "--driver", driver, "--output", output)
        self.assert_exit(result, 1)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["schema"], "fluxr-semantic-campaign-v1")
        self.assertEqual(report["summary"]["status"], "failed")
        self.assertEqual(report["summary"]["mismatches"], len(report["mismatches"]))
        self.assertGreater(len(report["mismatches"]), 0)
        return output, report, driver

    def test_generation_is_byte_reproducible_and_seed_sensitive(self):
        first, _ = self.generate("first.json")
        second, _ = self.generate("second.json")
        other, _ = self.generate("other.json", seed=244)
        self.assertEqual(first.read_bytes(), second.read_bytes())
        self.assertNotEqual(first.read_bytes(), other.read_bytes())

    def test_output_never_overwrites_an_existing_file(self):
        target = self.root / "existing.json"
        original = b"existing caller-owned bytes\n"
        target.write_bytes(original)
        result = self.cli(
            "generate", "--seed", 243, "--programs", 1,
            "--calls-per-program", 2, "--max-depth", 2, "--output", target,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(target.read_bytes(), original)

    def test_output_must_resolve_outside_the_source_tree(self):
        name = ".semantic-cli-forbidden-" + uuid.uuid4().hex + ".json"
        target = ROOT / name
        self.assertFalse(target.exists())
        link = self.root / "source-alias"
        link.symlink_to(ROOT, target_is_directory=True)
        try:
            for path in (target, link / name):
                with self.subTest(path=path):
                    result = self.cli(
                        "generate", "--seed", 243, "--programs", 1,
                        "--calls-per-program", 2, "--max-depth", 2,
                        "--output", path,
                    )
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(target.exists())
        finally:
            # Only the uniquely named path this test attempted is eligible.
            target.unlink(missing_ok=True)

    def test_invalid_generator_bounds_are_refused_before_output(self):
        for option, value in (
            ("--programs", 0), ("--programs", 1000000),
            ("--calls-per-program", 0), ("--max-depth", 1000000),
            ("--seed", "not-an-integer"),
        ):
            with self.subTest(option=option, value=value):
                output = self.root / f"bad-{option[2:]}-{value}.json"
                result = self.cli("generate", option, value, "--output", output)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(output.exists())

    def test_semantic_mismatch_replay_is_not_an_infrastructure_success(self):
        path, first, driver = self.mismatch_report()
        output = self.root / "replayed.json"
        result = self.cli("replay", "--input", path, "--driver", driver, "--output", output)
        self.assert_exit(result, 1)
        second = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(second["mismatches"], first["mismatches"])
        self.assertEqual(second["summary"]["comparisons"], 6)
        self.assertEqual(second["summary"]["native_successes"], 0)

    def test_driver_protocol_and_process_failures_produce_no_semantic_report(self):
        corpus = self.fixed_corpus()
        for mode in (
            "exit", "malformed", "missing_call", "wrong_id",
            "bad_scalar", "both_outcomes", "unknown_route", "inconsistent_native_feature",
        ):
            with self.subTest(mode=mode):
                output = self.root / f"incomplete-{mode}.json"
                result = self.cli(
                    "replay", "--input", corpus, "--driver", self.fake(mode), "--output", output,
                )
                self.assert_exit(result, 2)
                self.assertFalse(output.exists(), "incomplete infrastructure cannot publish a test result")

    def test_replay_refuses_corrupted_integrity_before_launching_driver(self):
        corpus = campaign.make_corpus([reduction_case()])
        corpus["cases"][0]["calls"][0]["arguments"][0]["i64"] += 1
        path = self.write_json("corrupted.json", corpus)
        output = self.root / "should-not-exist.json"
        driver = self.root / "must-not-run.py"
        marker = self.root / "launched"
        driver.write_text(
            "#!" + sys.executable + "\nfrom pathlib import Path\n"
            + "Path(" + repr(str(marker)) + ").write_text('launched')\n",
            encoding="utf-8",
        )
        driver.chmod(0o700)
        result = self.cli("replay", "--input", path, "--driver", driver, "--output", output)
        self.assert_exit(result, 2)
        self.assertFalse(marker.exists())
        self.assertFalse(output.exists())

    def test_replay_recomputes_source_binding_after_a_forged_reseal(self):
        corpus = campaign.make_corpus([reduction_case()])
        forged = copy.deepcopy(corpus)
        forged["sources"][0]["source"] = "v2 (pub calc (x:i64) i64 = 99)\n"
        forged["sources"][0]["sha256"] = hashlib.sha256(
            forged["sources"][0]["source"].encode("utf-8")
        ).hexdigest()
        forged.pop("integrity_sha256")
        path = self.write_json("forged-source.json", campaign.seal(forged))
        output = self.root / "forged-source-result.json"
        result = self.cli("replay", "--input", path, "--driver", self.fake(), "--output", output)
        self.assert_exit(result, 2)
        self.assertFalse(output.exists())

    def test_replay_recomputes_expected_values_after_a_forged_reseal(self):
        forged = campaign.make_corpus([reduction_case()])
        forged["cases"][0]["expected"][0] = {"value": {"i64": 8}}
        forged.pop("integrity_sha256")
        path = self.write_json("forged-expectation.json", campaign.seal(forged))
        output = self.root / "forged-expectation-result.json"
        result = self.cli("replay", "--input", path, "--driver", self.fake(), "--output", output)
        self.assert_exit(result, 2)
        self.assertFalse(output.exists())

    def test_replay_never_replaces_its_input(self):
        path = self.fixed_corpus()
        original = path.read_bytes()
        result = self.cli("replay", "--input", path, "--driver", self.fake(), "--output", path)
        self.assert_exit(result, 2)
        self.assertEqual(path.read_bytes(), original)

    def test_bounded_reduction_preserves_a_replayed_mismatch_and_call_history(self):
        path, original, driver = self.mismatch_report()
        output = self.root / "reduced.json"
        result = self.cli(
            "reduce", "--input", path, "--driver", driver,
            "--case", "cli-reduction", "--call", 2,
            "--max-attempts", 4, "--timeout", 10, "--output", output,
        )
        self.assert_exit(result, 0)
        reduced = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(reduced["kind"], "reduction")
        stats = reduced["reduction"]
        self.assertEqual(stats["max_attempts"], 4)
        self.assertLessEqual(stats["attempts"], 4)
        self.assertLessEqual(stats["reduced_cost"], stats["original_cost"])
        self.assertIs(type(stats["complete"]), bool)
        self.assertIs(type(stats["budget_exhausted"]), bool)
        cases = reduced["corpus"]["cases"]
        self.assertEqual(len(cases), 1)
        self.assertEqual(len(cases[0]["calls"]), 3, "preceding invocations preserve workspace history")
        target = [m for m in reduced["mismatches"] if m["case"] == "cli-reduction" and m["call"] == 2]
        self.assertTrue(target)
        before = {(m["backend"], m["kind"]) for m in original["mismatches"] if m["call"] == 2}
        self.assertTrue(any((m["backend"], m["kind"]) in before for m in target))
        replayed_path = self.root / "reduced-replayed.json"
        replayed = self.cli(
            "replay", "--input", output, "--driver", driver, "--output", replayed_path,
        )
        self.assert_exit(replayed, 1)
        actual = json.loads(replayed_path.read_text(encoding="utf-8"))
        self.assertEqual(actual["mismatches"], reduced["mismatches"])

    def test_reduction_rejects_a_nonexistent_case_and_unbounded_attempt_count(self):
        path, _, driver = self.mismatch_report()
        for name, attempts in (("missing", 4), ("cli-reduction", 1000000)):
            with self.subTest(name=name, attempts=attempts):
                output = self.root / f"bad-reduction-{name}-{attempts}.json"
                result = self.cli(
                    "reduce", "--input", path, "--driver", driver, "--case", name,
                    "--call", 0, "--max-attempts", attempts, "--timeout", 10, "--output", output,
                )
                self.assert_exit(result, 2)
                self.assertFalse(output.exists())

    @unittest.skipUnless(os.environ.get("FLUXR_SEMANTIC_DRIVER"), "built public-API driver not configured")
    def test_real_driver_run_and_replay(self):
        driver = Path(os.environ["FLUXR_SEMANTIC_DRIVER"]).resolve()
        self.assertTrue(driver.is_file(), "configured driver must exist; absence is not a skip")
        output = self.root / "real-run.json"
        require_native = ["--require-native"] if os.environ.get("FLUXR_SEMANTIC_REQUIRE_NATIVE") == "1" else []
        result = self.cli(
            "run", "--driver", driver, "--seed", 243, "--programs", 8,
            "--calls-per-program", 4, "--max-depth", 3, "--output", output,
            *require_native,
        )
        self.assert_exit(result, 0)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["summary"]["status"], "passed")
        self.assertEqual(report["summary"]["mismatches"], 0)
        self.assertGreater(report["summary"]["calls"], 32, "fixed witnesses run alongside generated cases")
        if require_native:
            self.assertGreater(report["summary"]["native_successes"], 0)
        replay_path = self.root / "real-replay.json"
        replayed = self.cli("replay", "--input", output, "--driver", driver, "--output", replay_path)
        self.assert_exit(replayed, 0)
        replay = json.loads(replay_path.read_text(encoding="utf-8"))
        self.assertEqual(replay["summary"]["mismatches"], 0)
        self.assertEqual(replay["summary"]["comparisons"], report["summary"]["comparisons"])

    @unittest.skipUnless(os.environ.get("FLUXR_SEMANTIC_DRIVER"), "built public-API driver not configured")
    def test_real_driver_mutation_checks_detect_every_sentinel(self):
        driver = Path(os.environ["FLUXR_SEMANTIC_DRIVER"]).resolve()
        self.assertTrue(driver.is_file(), "configured driver must exist; absence is not a skip")
        output = self.root / "real-mutations.json"
        result = self.cli("mutations", "--driver", driver, "--output", output)
        self.assert_exit(result, 0)
        report = json.loads(output.read_text(encoding="utf-8"))
        self.assertEqual(report["schema"], "fluxr-semantic-mutations-v1")
        self.assertEqual(report["summary"]["status"], "passed")
        self.assertGreater(report["summary"]["mutants"], 0)
        self.assertEqual(report["summary"]["detected"], report["summary"]["mutants"])


if __name__ == "__main__":
    unittest.main()
