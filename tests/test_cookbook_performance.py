"""Synthetic report/collection regressions; never performance measurements."""
from argparse import Namespace
from copy import deepcopy
from pathlib import Path
import json
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import cookbook_performance as cp
import fragment_performance as fp
from eval.runner import ProcessResult


def stats(count, unit, value=100):
    return fp.statistics([value] * count, unit)


def report(public=False, common=False):
    common = common or public
    manifest = cp.source_manifest(ROOT)
    lanes = cp.BASE_LANES + ([] if common else ["prepared"])
    cases = []
    for source in manifest["cases"]:
        contract = {"name": source["name"], "limits": dict(cp.LIMITS), "rows": [
            {"row": {"inputs": [], "arguments": [{"f32": 0x80000000}]}, "expected": {"Ok": {"f32": 0x80000000}}},
            {"row": {"inputs": [], "arguments": [{"i64": 0}]}, "expected": {"Err": "synthetic error"}},
        ]}
        times = {}
        for lane in lanes:
            witness = {"rust_kernel": "RustKernel", "rust_value_adapter": "RustValueAdapter",
                       "portable": "Portable", "named": "Unattributed", "prepared": "Portable"}[lane]
            times[lane] = {"individual_call": stats(64, cp.INDIVIDUAL), "batch_mean": stats(3, cp.BATCH),
                           "timed_individual_route_counts": {witness: 64}}
        cases.append({**source, "status": "passed", "artifact_sha256": "a"*64, "artifact_bytes": 100,
                      "row_contract": contract, "rows_sha256": fp.sha(cp.canonical(contract)),
                      "validation_rows": 2, "expected_successes": 1, "expected_traps": 1,
                      "validation_route_counts": {"Unattributed" if public else "Portable": 2},
                      "timings": times, "lifecycle": {phase: stats(7, unit) for phase, unit in cp.PHASES.items()}})
    result = {"schema": "fluxr-cookbook-performance-v1", "catalog": "fluxr-cookbook-v1",
              **{key: manifest[key] for key in ("probe_sha256", "support_sha256", "catalog_sha256")},
              "os": "linux", "arch": "x86_64", "target_env": "gnu", "typed_jit_feature": True,
              "debug_assertions": False, "public_baseline_adapter": public, "telemetry": "off",
              "compiled_source_format": "compact-v2" if public else "fluxr-1", "limits": dict(cp.LIMITS),
              "rust_kernel_scope": cp.KERNEL_SCOPE, "rust_value_adapter_scope": cp.ADAPTER_SCOPE,
              "settings": {"individual_samples": 64, "batch_rounds": 3, "calls_per_batch": 8,
                           "warmup_calls_per_lane": 128, "lifecycle_samples": 7,
                           "common_routes": common, "timed_lanes": lanes},
              "route_order": "rotate first lane each individual sample and batch round",
              "row_order": cp.ROW_ORDER,
              "elapsed_ns": 100000, "failed_cases": 0, "cases": cases,
              "calibration": {phase: stats(3 if phase == "batch_loop" else 64, unit)
                              for phase, unit in cp.CALIBRATION.items()}}
    return result, manifest


def execution(size=100):
    return {"terminal": "exited", "exit_code": 0, "seconds": 0.1, "process_started": True,
            "detail": None, "stdout": {"sha256": "a"*64, "bytes": size},
            "stderr": {"sha256": "b"*64, "bytes": 0}}


def capture(public=False, common=False):
    data, manifest = report(public, common)
    flags = dict.fromkeys(cp.FLAGS)
    if public:
        flags["RUSTFLAGS"] = "--cfg fluxr_public_baseline"
    return {"schema": "fluxr-cookbook-capture-v1", "source_and_binary_unchanged": True,
            "bounded_collection": "posix-process-groups-v1", "source_identity": "synthetic-only", "build_id": "synthetic",
            "inputs": manifest, "source": {"files": 20, "bytes": 1000, "sha256": "c"*64, "cargo_lock_sha256": "d"*64},
            "binary": {"sha256": "e"*64, "bytes": 100},
            "compiler": {"rustc": "rustc synthetic", "cargo": "cargo synthetic"},
            "hardware": {"system": "Linux", "machine": "x86_64", "release": "synthetic",
                         "cpu_models": ["synthetic"], "cpu_affinity": [0], "host_fingerprint": "f"*64},
            "configuration": {"profile": "release", "features": ["typed-jit"], "environment_flags": flags},
            "execution": execution(), "startup_execution": [execution(20) for _ in range(cp.STARTUPS)],
            "process_startup": stats(cp.STARTUPS, cp.STARTUP_UNIT), "report": data}


class CookbookReportTests(unittest.TestCase):
    def setUp(self):
        self.report, self.manifest = report()

    def test_complete_catalog_and_boundaries_are_consistent(self):
        self.assertIs(cp.validate_report(self.report, self.manifest), self.report)
        public, manifest = report(public=True)
        self.assertIs(cp.validate_report(public, manifest), public)
        self.assertNotIn("prepared", public["settings"]["timed_lanes"])

    def test_old_batch_reset_schedule_and_ambiguous_adapter_unit_are_rejected(self):
        bad = deepcopy(self.report)
        bad["row_order"] = "repeat deterministic per-case edge/seeded rows identically for every lane"
        with self.assertRaisesRegex(ValueError, "scheduling"):
            cp.validate_report(bad, self.manifest)
        bad = deepcopy(self.report)
        bad["cases"][0]["timings"]["rust_value_adapter"]["individual_call"]["observation_unit"] = (
            "ns per invocation; timer and harness dispatch included; checking/conversion/destruction excluded")
        with self.assertRaisesRegex(ValueError, "timing unit"):
            cp.validate_report(bad, self.manifest)

    def test_missing_duplicate_reordered_and_unknown_cases_are_rejected(self):
        for mutation in (lambda rows: rows.pop(), lambda rows: rows.append(deepcopy(rows[0])),
                         lambda rows: rows.reverse(), lambda rows: rows[0].update(name="unknown")):
            bad = deepcopy(self.report)
            mutation(bad["cases"])
            with self.assertRaises(ValueError):
                cp.validate_report(bad, self.manifest)

    def test_actual_source_reference_and_probe_identity_are_required(self):
        for key in ("probe_sha256", "support_sha256", "catalog_sha256"):
            bad = deepcopy(self.report)
            bad[key] = "0"*64
            with self.subTest(key=key), self.assertRaises(ValueError):
                cp.validate_report(bad, self.manifest)
        for key in ("source_sha256", "rust_sha256", "compact_sha256", "topics"):
            bad = deepcopy(self.report)
            bad["cases"][0][key] = ["wrong"] if key == "topics" else "0"*64
            with self.subTest(key=key), self.assertRaises(ValueError):
                cp.validate_report(bad, self.manifest)

    def test_row_hash_limits_and_outcome_counts_are_recomputed(self):
        mutations = [
            lambda row: row.update(rows_sha256="0"*64),
            lambda row: row["row_contract"]["rows"][0]["row"].update(arguments=[{"i64": 7}]),
            lambda row: row["row_contract"]["limits"].update(instructions=100),
            lambda row: row.update(validation_rows=3),
            lambda row: row.update(expected_successes=2),
            lambda row: row.update(expected_traps=0),
        ]
        for mutation in mutations:
            bad = deepcopy(self.report)
            mutation(bad["cases"][0])
            with self.assertRaises(ValueError):
                cp.validate_report(bad, self.manifest)

    def test_value_encodings_preserve_raw_float_bits_and_reject_wrong_types(self):
        for value in ["unit", {"f32": 0x7fa00001}, {"variant": [1, {"record": [{"list": [{"u64": 2**64-1}]}]}]}]:
            cp.validate_value(value)
        for value in [None, True, 1, {"i64": True}, {"u64": -1}, {"f32": 2**32},
                      {"f32": 1.0}, {"record": "bad"}, {"i64": 1, "u64": 1}, {"variant": [True, None]}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                cp.validate_value(value)

    def test_resource_numbers_and_jit_attribution_are_not_coerced(self):
        bad = deepcopy(self.report)
        bad["limits"]["instructions"] = float(cp.LIMITS["instructions"])
        with self.assertRaises(ValueError):
            cp.validate_report(bad, self.manifest)
        bad = deepcopy(self.report)
        bad["typed_jit_feature"] = False
        bad["cases"][0]["validation_route_counts"] = {"TypedJit": 2}
        with self.assertRaises(ValueError):
            cp.validate_report(bad, self.manifest)

    def test_forged_percentiles_nonfinite_and_bool_samples_are_rejected(self):
        for field, invalid in [("p99_9_ns", 99), ("sample_count", 64.0),
                               ("percentiles_p0_through_p100_ns", [99]*101),
                               ("observation_unit", "ns per different boundary"),
                               ("samples_ns", [float("inf")]*64), ("samples_ns", [True]*64)]:
            bad = deepcopy(self.report)
            bad["cases"][0]["timings"]["portable"]["individual_call"][field] = invalid
            with self.subTest(field=field), self.assertRaises(ValueError):
                cp.validate_report(bad, self.manifest)

    def test_nearest_rank_endpoints_and_all_percentiles_are_kept(self):
        summary = fp.statistics(list(range(1, 1001)), cp.INDIVIDUAL)
        self.assertEqual(summary["percentiles_p0_through_p100_ns"], [1] + list(range(10, 1001, 10)))
        self.assertEqual(summary["p99_9_ns"], 999)

    def test_missing_lanes_phases_and_invalid_witnesses_fail(self):
        for mutation in [lambda row: row["timings"].pop("prepared"),
                         lambda row: row["lifecycle"].pop("cold_prepare"),
                         lambda row: row["timings"]["named"].update(timed_individual_route_counts={"TypedJit": 64}),
                         lambda row: row.update(validation_route_counts={"Portable": 1}),
                         lambda row: row.update(status="failed")]:
            bad = deepcopy(self.report)
            mutation(bad["cases"][0])
            with self.assertRaises(ValueError):
                cp.validate_report(bad, self.manifest)

    def test_duplicate_json_keys_and_nonfinite_json_are_rejected(self):
        for raw in [b'{"cases":[],"cases":[]}', b'{"time":NaN}', b'{"time":Infinity}']:
            with self.assertRaises(ValueError):
                fp.decode(raw)


class CookbookComparisonTests(unittest.TestCase):
    def test_same_capture_compares_all_lanes_and_every_percentile(self):
        value = capture()
        result = cp.compare(value, deepcopy(value))
        self.assertEqual(len(result["cases"]), len(value["report"]["cases"]))
        self.assertEqual(len(result["cases"][0]["warm"]), 10)
        for measurement in result["cases"][0]["warm"]:
            self.assertEqual(len(measurement["percentiles"]), 102)
            self.assertEqual(measurement["regressed_percentiles"], [])
        self.assertIn("no universal", result["qualification"])

    def test_lower_median_cannot_hide_tail_regression(self):
        before, after = capture(), capture()
        value = after["report"]["cases"][0]["timings"]["portable"]
        value["individual_call"] = fp.statistics([50]*63+[10000], cp.INDIVIDUAL)
        result = cp.compare(before, after)
        row = next(row for row in result["cases"][0]["warm"]
                   if row["candidate_lane"] == "portable" and row["measurement"] == "individual_call")
        self.assertEqual(row["percentiles"][50]["observation"], "improved")
        self.assertIn("p100", row["regressed_percentiles"])
        self.assertIn("p99.9", row["regressed_percentiles"])
        self.assertFalse(row["all_observed_percentiles_nonregressing"])

    def test_public_cfg_is_only_allowed_build_flag_difference(self):
        before, after = capture(public=True), capture(common=True)
        result = cp.compare(before, after)
        self.assertTrue(result["public_api_adapter_changed"])
        self.assertEqual(result["prepared_lane_status"], {"baseline": "unavailable_public_api", "candidate": "not_timed_common_schedule"})
        self.assertEqual([phase["phase"] for phase in result["cases"][0]["lifecycle"]], ["artifact_decode"])
        self.assertEqual(len(result["cases"][0]["lifecycle_not_compared"]), 3)
        self.assertEqual(result["cases"][0]["baseline_validation_routes"], {"Unattributed": 2})
        bad = deepcopy(before)
        bad["configuration"]["environment_flags"]["RUSTFLAGS"] += " -C opt-level=0"
        with self.assertRaises(ValueError):
            cp.compare(bad, after)

    def test_public_and_candidate_must_use_same_lane_schedule(self):
        with self.assertRaisesRegex(ValueError, "settings"):
            cp.compare(capture(public=True), capture())

    def test_compiler_hardware_features_and_input_changes_refuse(self):
        before = capture()
        for mutation in [lambda v: v["compiler"].update(rustc="rustc different"),
                         lambda v: v["hardware"].update(cpu_affinity=[1]),
                         lambda v: v["configuration"]["features"].append("vulkan"),
                         lambda v: v["report"]["cases"][0].update(artifact_sha256="0"*64)]:
            after = deepcopy(before)
            mutation(after)
            with self.assertRaises(ValueError):
                cp.compare(before, after)

    def test_artifact_and_row_semantics_cannot_change_between_captures(self):
        before, after = capture(), capture()
        row = after["report"]["cases"][0]
        row["row_contract"]["rows"][0]["row"]["arguments"] = [{"i64": 10}]
        row["rows_sha256"] = fp.sha(cp.canonical(row["row_contract"]))
        with self.assertRaisesRegex(ValueError, "semantic/input/resource"):
            cp.compare(before, after)

    def test_explicit_lane_mapping_reports_changed_boundary_and_missing_lane(self):
        value = capture()
        result = cp.compare(value, deepcopy(value), ("portable", "prepared"))
        self.assertTrue(result["api_boundary_changed"])
        self.assertEqual(len(result["cases"][0]["warm"]), 2)
        with self.assertRaisesRegex(ValueError, "unavailable"):
            cp.compare(value, deepcopy(value), ("portable", "absent"))

    def test_incomplete_or_failed_collection_cannot_be_compared(self):
        for mutation in [lambda value: value.update(source_and_binary_unchanged=False),
                         lambda value: value["execution"].update(terminal="output_limit", exit_code=-9),
                         lambda value: value["startup_execution"].pop(),
                         lambda value: value["configuration"]["environment_flags"].update(RUSTFLAGS="--cfg fluxr_public_baseline")]:
            value = capture()
            mutation(value)
            with self.assertRaises(ValueError):
                cp.validate_capture(value)


class CookbookCaptureFailureTests(unittest.TestCase):
    def test_failed_bounded_child_preserves_attempt_and_does_not_publish_capture(self):
        # The collector itself has real process tests elsewhere; this checks the
        # cookbook controller's failure publication and precise limit request.
        with tempfile.TemporaryDirectory(prefix="cookbook-controller-") as temporary:
            output = Path(temporary) / "failed.json"
            args = Namespace(source=ROOT, binary=Path(sys.executable), output=output,
                             identity="synthetic-failure", build_id="synthetic-build", features="typed-jit",
                             profile="release", samples=64, rounds=3, batch_calls=8, timeout=1, common_routes=False)
            observed = []
            def failed(command, payload, cwd, stdout, stderr, **limits):
                observed.append(limits)
                stdout.write_bytes(b'{"partial":')
                stderr.write_bytes(b"intentional collector failure")
                return ProcessResult("output_limit", -9, 0.01, True, "stdout")
            with patch.object(cp, "tool_version", side_effect=["rustc synthetic", "cargo synthetic"]), \
                 patch.object(cp, "run_bounded", side_effect=failed):
                with self.assertRaisesRegex(ValueError, "benchmark failed"):
                    cp.capture(args)
            logs = output.with_name(output.name + ".logs")
            self.assertFalse(output.exists())
            self.assertEqual((logs / "stdout").read_bytes(), b'{"partial":')
            self.assertEqual(json.loads((logs / "execution.json").read_text())["terminal"], "output_limit")
            self.assertEqual(json.loads((logs / "failure.json").read_text())["phase"], "benchmark")
            self.assertEqual(observed, [{"timeout_seconds": 1, "max_stdout_bytes": 64*1024*1024, "max_stderr_bytes": 64*1024}])
            with self.assertRaises(FileExistsError):
                cp.capture(args)


if __name__ == "__main__":
    unittest.main()
