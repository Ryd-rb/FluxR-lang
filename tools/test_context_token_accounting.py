"""Independent admission/accounting checks for static AC1 evidence measurement.

Synthetic receipt flags here exercise validator behavior only. They are never
evidence of FluxR compilation, execution, isolation or model task success.
"""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import tempfile
import tomllib
import unittest

import bench_context_compact as counter


def receipt_fixture():
    catalog = tomllib.loads(counter.read_bytes(counter.CATALOG).decode())
    result = {"schema": catalog["measurement"]["observation_schema"],
              "profile": catalog["measurement"]["profile"], "source_token_target": 0.1,
              "protocol_setup": counter.source(catalog["measurement"]["protocol"]),
              "cases": [], "locality_cases": []}
    for name, spec in catalog["case"].items():
        binding = {"fixture_only": name}
        row = {"id": name, "source": counter.source(spec["base"]),
               "completed_source": counter.source(spec["fluxr"]),
               "fill": counter.source(spec["fill"]), "rust_source": counter.source(spec["rust"]),
               "exact_fsm_equal": True, "binding": binding,
               "failed_fill": {"text": "fa1 body 1 ret true", "kind": "type",
                               "detail": "synthetic type mismatch", "display": "AH1 Type: synthetic type mismatch",
                               "unchanged": True}, "recorded_host_calls": {}}
        for arm in counter.ARMS:
            row[arm + "_packet"] = arm + " synthetic context\n"
            row["recorded_host_calls"][arm] = json.dumps({"binding": binding, "selection": arm})
        result["cases"].append(row)
    for helpers in (0, 31, 127):
        original = counter.source(catalog["case"]["scalar"]["base"]) + "".join(
            f"fn unrelated{n} 1 value int int ret value\n" for n in range(helpers))
        row = {"id": f"locality_{helpers}", "helpers": helpers, "source": original,
               "completed_source": original.replace("ret add x y", "ret rem add mul x y 17 997", 1),
               "fill": counter.source(catalog["case"]["scalar"]["fill"]), "exact_fsm_equal": True}
        row.update({arm + "_packet": arm + " synthetic locality\n" for arm in counter.ARMS})
        result["locality_cases"].append(row)
    return result, catalog


class ReceiptAdmission(unittest.TestCase):
    def setUp(self):
        self.observed, self.catalog = receipt_fixture()

    def reject(self, mutate):
        changed = deepcopy(self.observed)
        mutate(changed)
        with self.assertRaises(ValueError):
            counter.validate_observations(changed, self.catalog)

    def test_complete_synthetic_population_accepted_only_as_accounting_input(self):
        cases, locality = counter.validate_observations(self.observed, self.catalog)
        self.assertEqual(len(cases), 4)
        self.assertEqual([row["helpers"] for row in locality], [0, 31, 127])

    def test_missing_and_duplicate_cases_refuse(self):
        self.reject(lambda x: x["cases"].pop())
        self.reject(lambda x: x["cases"].__setitem__(1, deepcopy(x["cases"][0])))
        self.reject(lambda x: x["cases"].reverse())

    def test_source_newline_and_rust_baseline_drift_refuse(self):
        self.reject(lambda x: x["cases"][0].__setitem__("source", x["cases"][0]["source"].replace("\n", "\r\n")))
        self.reject(lambda x: x["cases"][0].__setitem__("rust_source", x["cases"][0]["rust_source"] + "// inflated baseline\n"))

    def test_tutorial_and_profile_drift_refuse(self):
        self.reject(lambda x: x.__setitem__("protocol_setup", "shorter new tutorial"))
        self.reject(lambda x: x.__setitem__("profile", "unknown"))

    def test_target_and_equality_claim_cannot_be_silently_dropped(self):
        self.reject(lambda x: x.__setitem__("source_token_target", 0.2))
        self.reject(lambda x: x["cases"][0].pop("exact_fsm_equal"))
        for key, value in (("equivalent_rust_max_ratio", 0.2), ("tokenizers", ["o200k_base"]),
                           ("tiktoken_version", "unversioned")):
            catalog = deepcopy(self.catalog)
            catalog["measurement"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                counter.validate_observations(self.observed, catalog)

    def test_failed_attempt_cannot_be_omitted(self):
        self.reject(lambda x: x["cases"][0].pop("failed_fill"))
        self.reject(lambda x: x["cases"][0]["failed_fill"].__setitem__("display", ""))
        self.reject(lambda x: x["cases"][0]["failed_fill"].__setitem__("unchanged", False))

    def test_host_binding_drift_refuses(self):
        self.reject(lambda x: x["cases"][0]["recorded_host_calls"].__setitem__(
            "compact", json.dumps({"binding": {"different": True}})))
        self.reject(lambda x: x["cases"][0]["recorded_host_calls"].pop("focused"))

    def test_locality_missing_row_and_changed_population_refuse(self):
        self.reject(lambda x: x["locality_cases"].pop())
        self.reject(lambda x: x["locality_cases"][1].__setitem__("helpers", 32))
        self.reject(lambda x: x["locality_cases"][0].__setitem__("helpers", False))
        self.reject(lambda x: x["locality_cases"][1].__setitem__("source", "fa1 v2"))

    def test_duplicate_json_object_keys_refuse(self):
        with self.assertRaises(ValueError):
            json.loads('{"cases": [], "cases": []}', object_pairs_hook=counter.unique_object)
        with self.assertRaises(ValueError):
            json.loads('{"field":NaN}', parse_constant=counter.reject_constant)

    def test_file_read_preserves_crlf_and_terminal_newline(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "exact.txt"
            path.write_bytes(b"first\r\nsecond\n")
            self.assertEqual(counter.read_bytes(path), b"first\r\nsecond\n")


class WholeStringAccounting(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import tiktoken
        cls.encoding = tiktoken.get_encoding("o200k_base")

    def test_adjacency_is_encoded_once(self):
        enc = self.encoding
        whole = "hello world"
        self.assertEqual(counter.count(enc, whole)["tokens"], len(enc.encode_ordinary(whole)))
        self.assertLess(len(enc.encode_ordinary(whole)),
                        sum(len(enc.encode_ordinary(part)) for part in ("hel", "lo", " world")))

    def test_quote_and_crlf_costs_are_distinct_exact_strings(self):
        text = 'name "quoted"\r\nnext\\line\n'
        measured = counter.count(self.encoding, text)
        self.assertEqual(measured["sha256"], hashlib.sha256(text.encode()).hexdigest())
        quoted = json.dumps(text, ensure_ascii=False)
        self.assertEqual(measured["json_quoted_wire"]["tokens"], len(self.encoding.encode_ordinary(quoted)))
        self.assertNotEqual(measured["sha256"], counter.identity(text.replace("\r\n", "\n"))["sha256"])

    def test_both_attempts_setup_and_retrieval_are_charged(self):
        observed, _ = receipt_fixture()
        row = observed["cases"][0]
        report = counter.episode(self.encoding, row, "compact", observed["protocol_setup"])
        messages = report["messages"]
        self.assertEqual([attempt["order"] for attempt in report["attempts"]], [1, 2])
        self.assertEqual([message["event"] for message in messages], ["setup", "base_source", "initial_context", "failed_body", "failure_diagnostic", "repeated_context", "repaired_body", "admission_feedback"])
        self.assertEqual(report["message_body_token_sum"], sum(message["tokens"] for message in messages))
        self.assertEqual(report["output_body_tokens"], len(self.encoding.encode_ordinary(row["failed_fill"]["text"])) + len(self.encoding.encode_ordinary(row["fill"])))
        extra = report["if_full_host_query_records_are_model_visible"]["message_body_token_sum"]
        self.assertEqual(extra - report["message_body_token_sum"], 2 * len(self.encoding.encode_ordinary(row["recorded_host_calls"]["compact"])))


if __name__ == "__main__":
    unittest.main()
