# WK1 source and setup token accounting

`tools/bench_wordcode_tokens.py` measures the authored catalog in
`benchmarks/wordcode-v14.toml`. It reads four practical microcases and the separate
synthetic forest case. It never compiles or runs source, calls a model, admits a
candidate to frozen replay, or verifies behavioral equivalence. Compilation,
execution, exact FA1 comparison, independent Rust results and resource behavior
need their own receipts. Completing this measurement does not complete an inherited
task or profile.

## Reproduction

Admit the measurement through the shared resource coordinator. Use the existing
hash-verified tokenizer assets and exactly `tiktoken==0.14.0` and
`tokenizers==0.23.3`; this tool cannot acquire missing dependencies or assets.
Keep the cache, worker binary, observation and output outside the checkout. The
output path is required, and an existing receipt is never overwritten.

First retain the exact standard output of the same candidate worker's
`dictionary` command. That command emits `profile`, `binding`,
`dictionary_sha256` and the actual canonical `dictionary_json`. Then run:

```sh
python3 -B tools/bench_wordcode_tokens.py \
  --cache "$FLUXR_TOKENIZER_CACHE" \
  --dictionary-observation "$FLUXR_WORD_EVIDENCE/core-dictionary.json" \
  --worker-binary "$FLUXR_WORD_WORKER" \
  --output "$FLUXR_WORD_EVIDENCE/source-tokens.json"
```

The worker is read and hashed, never executed by the counter. Its bytes and the
dictionary observation are bound into the report. The caller must retain the
actual command/exit receipt and candidate tree separately: matching hashes do
not prove that the command ran, authorize source, or establish an independent
oracle. Dictionary JSON field order and the WK1 domain-separated digest are
checked for consistent accounting; this Python check does not replace the
compiler's dictionary/type/effect/resource validation.

Run the dependency-free admission/accounting tests separately through the same
coordinator:

```sh
python3 -B tools/test_wordcode_token_accounting.py
```

These tests use malformed panel/catalog/dictionary inputs, hand-computable exact
ratios and a tiny synthetic tokenizer whose `ab` pair is one token. They do not
stand in for measurements by the six actual tokenizers or for FluxR execution.

## Compared strings and gates

Every tokenizer receives each complete, unmodified UTF-8 source string:
authored WK1, expanded FA1, minimal standalone checked Rust, and a Rust call to
the reusable vocabulary. Whitespace, punctuation, terminal newlines and custom
dictionary pins are retained. Each JSON-quoted version is encoded independently.
The forest Rust baseline uses `checked_pow(256)`, so a large explicit Rust
multiplication tree cannot inflate its denominator.

The retained source target is exactly `1/10`. Integer fractions determine each
ratio, median, ratio of sums and nearest-rank p95. Floating-point values are
display aids only. The static practical slice passes only when at least four of
the pinned six encodings meet both median and aggregate targets against both
standalone Rust and the shared-helper source arm. The synthetic slice is reported
separately. Combined results are descriptive and cannot make a failing practical
population pass. This authored slice does not satisfy the inherited held-out or
live-model gates even if its numeric target passes.

Source counts exclude shared setup symmetrically. Separate costs retain the
complete inherited FA1 card, WK1 protocol, actual core dictionary JSON and
identity, custom dictionary file and identity, whole Rust helper library, and
Rust scenario instructions. Cold scenarios charge applicable setup for each case.
Reuse scenarios charge setup once, including each distinct dictionary when first
retrieved. The synthetic dictionary's on-disk bytes and its canonical identity
remain distinguishable.

For every scenario, the report provides three independently encoded complete
strings: concatenated message bodies, a deterministic labeled transcript, and a
complete compact JSON message array. It also gives the sum of separately encoded
message bodies. These quantities are intentionally distinct because BPE counts
need not be additive across boundaries. Original texts appear once in the payload
table; message records and the declared rendering policy reconstruct the exact
transcript bytes and hashes.

These constructed messages are static accounting scenarios. There are no
observed model attempts, repairs, accepted tasks, cached/uncached provider usage,
tool calls, reviews, handoffs or model latency. Missing fields remain `null` and
the report makes no hosted-billing or trained-model competence claim.

## Offline spelling audit

`tools/wordcode_token_audit.py` shares the same offline loader and can audit a
bounded candidate-word JSON document. Its eleven contexts include a bare word,
leading spaces, newline, repeated words, punctuation, identifier and numeric
adjacency, numeric arguments, and JSON escaping. The original nine-context
research helper remains retained with its exact origin hash; this is an additive
version of that helper. Even a one-token result in every measured context would
describe only this named finite panel, not most deployed models or a model's
ability to use the word correctly.
