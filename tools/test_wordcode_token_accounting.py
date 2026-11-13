"""Accounting admission and independently hand-computable text boundary checks.

No real tokenizer, worker, compiler or model is loaded. Synthetic dictionary and
codec fixtures are validator inputs, never execution/competence evidence.
"""
from copy import deepcopy
from decimal import Decimal
import unittest
import tomllib

import bench_wordcode_tokens as counter
import wordcode_token_audit as audit


def panel_fixture():
    return tomllib.loads((counter.ROOT / "benchmarks/token-panel.toml").read_bytes().decode(), parse_float=Decimal)


def dictionary_fixture():
    text = '{"version":1,"definitions":[{"word":"echo","params":["int"],"result":"int","body":{"op":"param","index":0}}]}'
    return {"profile": "wk1", "binding": "core", "dictionary_json": text,
            "dictionary_sha256": audit.digest(counter.DICT_PREFIX + text.encode())}


class PanelAndCatalogAdmission(unittest.TestCase):
    def test_complete_pinned_panel_admitted(self):
        audit.validate_panel(panel_fixture())

    def test_missing_duplicate_reordered_and_mutated_profiles_refuse(self):
        for mutation in (
            lambda p: p["panel"].pop(),
            lambda p: p["panel"].__setitem__(5, deepcopy(p["panel"][0])),
            lambda p: p["panel"].reverse(),
            lambda p: p["panel"][0].__setitem__("sha256", "0" * 64),
            lambda p: p["panel"][0].__setitem__("family", "Other"),
            lambda p: p.__setitem__("required_passes", 3),
            lambda p: p.__setitem__("limit", Decimal("0.11")),
            lambda p: p.__setitem__("limit", "0.10"),
            lambda p: p.__setitem__("limit", Decimal("NaN")),
            lambda p: p.__setitem__("schema", True),
            lambda p: p.__setitem__("tokenizers", "newer"),
        ):
            panel = panel_fixture()
            mutation(panel)
            with self.assertRaises(ValueError):
                audit.validate_panel(panel)

    def test_synthetic_cannot_replace_practical_or_missing_cases(self):
        original = tomllib.loads((counter.ROOT / "benchmarks/wordcode-v14.toml").read_bytes().decode())
        counter.validate_catalog(original)
        for mutation in (
            lambda c: c["cases"].pop(),
            lambda c: c["cases"].__setitem__(0, deepcopy(c["cases"][-1])),
            lambda c: c["cases"][0].__setitem__("class", "synthetic_density_stress"),
            lambda c: c["cases"][-1].__setitem__("class", "authored_practical_microcase"),
            lambda c: c["cases"][0].__setitem__("expected", True),
            lambda c: c["cases"][0].__setitem__("rust_full", c["cases"][0]["wordcode"]),
        ):
            changed = deepcopy(original)
            mutation(changed)
            with self.assertRaises(ValueError):
                counter.validate_catalog(changed)

    def test_missing_dictionary_bytes_bad_identity_and_unknown_observation_fields_refuse(self):
        counter.validate_dictionary_observation(dictionary_fixture())
        for mutation in (
            lambda d: d.pop("dictionary_json"),
            lambda d: d.__setitem__("dictionary_sha256", "0" * 64),
            lambda d: d.__setitem__("dictionary_json", d["dictionary_json"] + "\n"),
            lambda d: d.__setitem__("binding", "mutable-latest"),
            lambda d: d.__setitem__("execution_verified", True),
        ):
            changed = dictionary_fixture()
            mutation(changed)
            with self.assertRaises(ValueError):
                counter.validate_dictionary_observation(changed)

    def test_duplicate_keys_nonfinite_and_dictionary_shape_refuse(self):
        for raw in ('{"count":1,"count":2}', '{"count":NaN}', '{"count":Infinity}'):
            with self.assertRaises(ValueError):
                audit.parse_json(raw)
        for bad in ('{"version":true,"definitions":[]}',
                    '{"version":1,"definitions":[{}]}'):
            with self.assertRaises(ValueError):
                counter.canonical_dictionary(bad)

    def test_evidence_cannot_be_written_into_checkout(self):
        with self.assertRaises(ValueError):
            audit.output_path(counter.ROOT / "would-be-evidence.json")


class ExactCountArithmetic(unittest.TestCase):
    def test_hand_calculated_threshold_and_even_median(self):
        self.assertTrue(counter.ratio_record(counter.exact_ratio(3, 30))["passes_1_over_10"])
        self.assertFalse(counter.ratio_record(counter.exact_ratio(3, 29))["passes_1_over_10"])
        observed = counter.summarize([(1, 20), (3, 20)])
        # Ratios 1/20 and 3/20: median=1/10, aggregate=4/40,
        # nearest-rank p95=3/20. Inherited median/aggregate gate excludes p95.
        self.assertEqual(observed["median"]["numerator"], 1)
        self.assertEqual(observed["median"]["denominator"], 10)
        self.assertEqual(observed["aggregate"]["denominator"], 10)
        self.assertEqual(observed["p95_nearest_rank"]["numerator"], 3)
        self.assertEqual(observed["p95_nearest_rank"]["denominator"], 20)
        self.assertTrue(observed["passes_median_and_aggregate"])
        self.assertFalse(observed["p95_nearest_rank"]["passes_1_over_10"])

    def test_malformed_count_populations_never_pass(self):
        for pairs in ([], [(True, 10)], [(1, False)], [(1, 0)], [(-1, 10)],
                      [(1.0, 10)], [(1, 10.0)], [(1,)], [(2**64, 10)]):
            with self.assertRaises(ValueError):
                counter.summarize(pairs)
        with self.assertRaises(ValueError):
            audit.count_text(lambda _: [True], "source")
        with self.assertRaises(ValueError):
            audit.count_text(lambda _: [-1], "source")

    def test_joined_string_is_not_replaced_by_message_sum(self):
        # Tiny independent tokenizer oracle: 'ab' is one token, every other
        # character is one token. Therefore 'a' + 'b' counts 1, separately 2.
        def encode(text):
            return [0] * len(text.replace("ab", "X"))

        payloads = {"left": {"text": "a"}, "right": {"text": "b"}}
        measured = {key: audit.count_with_json(encode, value["text"]) for key, value in payloads.items()}
        messages = [{"event": "task", "direction": "input", "payload": "left"},
                    {"event": "authored_source", "direction": "output", "payload": "right"}]
        result = counter.count_transcript(encode, messages, payloads, measured)
        self.assertEqual(result["message_body_token_sum"], 2)
        self.assertEqual(result["joined_body_string"]["tokens"], 1)
        self.assertEqual(result["joined_body_string"]["bytes"], 2)
        # JSON literal "ab" has two quotes plus the single 'ab' token.
        self.assertEqual(result["joined_body_string"]["json_quoted_string"]["tokens"], 3)
        self.assertIsNone(result["accepted_task_success"])
        self.assertFalse(result["provider_billing_or_chat_framing"])

    def test_lexical_audit_preserves_numeric_argument_and_adjacency_contexts(self):
        contexts = audit.contexts("walk")
        self.assertEqual(len(contexts), 11)
        self.assertEqual(contexts["numeric_arguments"], "walk 1 -2 3u")
        self.assertEqual(contexts["numeric_adjacency"], "walk1")
        self.assertEqual(contexts["leading_space"], " walk")
        self.assertEqual(contexts["newline"], "\nwalk")


if __name__ == "__main__":
    unittest.main()
