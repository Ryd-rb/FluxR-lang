# FluxR modernization

FluxR should accept compact, model-authored programs and edits, validate them against a versioned semantic contract, and execute them with the same optimized backend whether loaded at startup or injected into a running host. The targets are Rust-like safety, performance as close to equivalent optimized Rust as possible, and at most 10% of equivalent Rust source tokens on most tested tokenizers. Each target needs its own evidence.

## Design

- **Source.** Define a model-facing format over a small typed core, with positional references, inferred local types, and semantic forms for recurrent loops, scans, queries, state transitions, and timeline data. Make the format compositional and trainable. A deterministic encoder may select an approved spelling or alias table for a pinned tokenizer. All profiles must decode to the same program. Keep FR1's frozen alphabet and every released reader. Evaluate shared libraries, templates, and build interfaces against equally capable Rust libraries and macros. Count each prerequisite where a model actually receives or authors it.
- **Edits.** Use a typed operation stream with revision and interface identity, stable symbol IDs, and scoped edit operations. Provide grammar-constrained generation and checker diagnostics with source spans. Compare structured patches against Rust edits with identical task and repository context. Train or adapt models only after ordinary-model generation, repair rate, and task-token experiments show a worthwhile gap.
- **Semantics.** Specify fixed-width checked and wrapping integers, floating-point rounding and NaN policies, ownership and borrows, evaluation order, trap precedence, and logical resource charging. Version semantic changes separately from notation. Normalize legacy input to typed HIR, then lower to SSA with explicit trap, capability, allocation, and fuel effects. Preserve effects across transforms. Define resource-bounded execution as a host policy. The abstract core already has the control and storage needed to model a Turing machine using a loop and two growable tape halves. Demonstrate a universal machine and host-budget suspension before claiming the new core preserves that result.
- **Coverage.** Deliver modules, imports, generic specialization, closures, recursion, sum and product types, slices, strings, maps, arrays, iterators, and deterministic allocation. Provide safe host calls and an explicit capability model for I/O. Specify errors, cancellation, scheduling and concurrency separately, with a deterministic mode and replay contract. Publish a standard library, package and ABI identity, diagnostics, tooling, and portable conformance cases. Feature completeness means this published inventory and the representative host applications pass, with Rust features classified by semantics rather than keyword parity.
- **Runtime.** Keep the reference VM as a differential oracle and portable fallback. Compare direct Cranelift JIT against Wasm with Wasmtime/Winch for cold injection and against Cranelift/LLVM for warm execution and AOT. Add copy-and-patch only if measured compilation latency warrants another backend. Use one verified SSA, common target features, the same stable typed call ABI, and matched metering in JIT and AOT. Prefer monomorphized, unboxed hot paths, contiguous collections, inlining, and bounds-check elimination from verified range and ownership facts. Batch host calls, keep recursion bounded by host policy, and preserve diagnostic identity.
- **Live updates.** Check source and effects, hash the semantic body, prepare the chosen tier off the execution path, probe an isolated snapshot, and activate with a revision check. Cache by semantic and compiler version, target, features, metering profile, and interface. Reuse unaffected functions, bound and evict native code, and retain the current generation until its calls finish. Measure decode, validation, compile, lookup, activation, first execution, tier-up, and warm execution separately. Use the same optimized tier for injected and bundled code when making a parity claim.
- **Safety.** Specify invariants at the HIR, SSA, host ABI, and machine boundaries. Prove preservation of types, bounds, initialization, capabilities and exact traps where required. Keep injected code sandboxed until native isolation is justified. Validate optimizations with SMT or a small independent checker, differential-fuzz legal and malformed programs, and verify generated code's memory/control-flow restrictions. Separate the proof of a rule from sandbox and backend assurance. AI may propose rewrites or alternative encodings. Only deterministic verification and measured benefit admit them.

## Research decisions

| Primary source | FluxR experiment |
| --- | --- |
| [Token Sugar](https://arxiv.org/abs/2512.08266), [information-theoretic study of tokenizers](https://arxiv.org/abs/2601.09039), [CodeGolf Bench](https://arxiv.org/abs/2605.30394) | Mine reusable typed forms on training data. Test held-out tasks and multiple tokenizers. Measure success and repair cost as well as density. |
| [Flexible grammar-constrained decoding](https://arxiv.org/abs/2502.05111), [XGrammar](https://arxiv.org/abs/2411.15100) | Compile token masks for the authoring grammar and compare generation throughput. The type and capability checker remains the semantic authority. |
| [Copy-and-patch](https://compilers.stanford.edu/publications/copy-and-patch/), [Deegen](https://arxiv.org/abs/2411.11469), [NEAR's 2026 Wasmtime evaluation](https://blog.nearone.org/research/2026/07/01/replacing-nears-contract-vm-with-wasmtime.html), [Wasmtime platform support](https://docs.wasmtime.dev/stability-platform-support.html) | Compare cold and hot tiers under identical bounds. Wasmtime offers Winch and Cranelift but documents no automatic switch between them. Prototype FluxR's activation and tier policy separately. |
| [Cranelift instruction selection verification](https://cfallin.org/pubs/oopsla2025_arrival.pdf), [Alive2](https://github.com/AliveToolkit/alive2), [2026 LLM peephole generalization](https://arxiv.org/abs/2603.18477) | Use proof obligations for checked arithmetic, memory, and effect order. Require a counterexample or proof result and a measured win before accepting proposed rewrites. |
| [WebAssembly 3.0](https://webassembly.github.io/spec/core/), [VeriWasm](https://www.ndss-symposium.org/wp-content/uploads/ndss2021_5B-3_24078_paper.pdf), [Wasmtime's April 2026 security review](https://bytecodealliance.org/articles/wasmtime-security-advisories) | Compare a portable validation baseline and machine-level isolation. Track the trusted code base, target-specific validation and remaining backend risk. |

## Ordered delivery and acceptance

| ID | Scope and acceptance | Dependency and next action |
| --- | --- | --- |
| FXM-01 | Repeatable baseline with exact input manifest, native and portable settings, behavior oracle, and per-case negative results. FXM-01.B freezes the structured host corpus with pinned Windows x64 ABBA and Node Wasm replay. | Collect hardware counters with an authorized collector and run native Linux x64 and ARM64 on suitable runners. |
| FXM-02 | Shared optimized runtime. Match injected and bundled p50 within 5% and p95 within 10% in each supported hot workload. Target FluxR/Rust geometric mean ≤1.10 and p95 ≤1.25 over the frozen corpus. Keep individual failures visible. The optional signed-leaf JIT and three runtime optimizations are the first implementation. | FXM-01 and FXM-05 semantic contract. Profile stateful loops, validate typed SSA and test register-based portable, Cranelift, and Wasm tiers before choosing production defaults. |
| FXM-03 | Preserve released inputs and artifact bytes. Migrate automatically with a deterministic proof receipt, old/new differential replay, and an explicit refusal if any semantic feature cannot be translated. The first pass has timeline v2, structured compact, and canonical rule migration. | FXM-05 for new core semantics. Add rule and FR1 ingress to shared HIR, historical fixture checks, migration reports, and versioned IR round trips. |
| FXM-04 | Model-facing source and edit formats. Pre-register a six-tokenizer panel spanning at least four independent model families. At least four tokenizers must meet both median and aggregate FluxR/Rust ≤0.10 on held-out, behavior-matched tasks. Report every family and p95. Include compiler feedback, retries, host bindings, and initial context in separate metrics. | FXM-01 for the locked corpus and FXM-05 for typed semantics. Prototype typed shape inference and reusable semantic forms. Run generation and edit success comparisons at matched model settings. |
| FXM-05 | Safety and language coverage. FXM-05.B checks typed list effects and the structured host boundary, and demonstrates a universal tape simulator. | Extend the specification and proof checker beyond the bounded fragment. Complete the open feature inventory, backend safety checks and portable conformance matrix. |
| FXM-06 | Integrate accepted features on the combined baseline. Verify Windows x64, Linux x64, ARM64 and browser/portable paths, package compatibility, and isolated host launches. | FXM-01 through FXM-05. Integrate the volume branch with independent baseline evidence, then perform release-level replay and cleanup. |

FXM-02.A established the first integrated runtime baseline. FXM-02.B delivered the Windows shared typed subset below. FXM-01.B freezes and measures the structured host corpus. FXM-05.A verifies typed list admission and the length-dependent scan effect. FXM-02.C can now test bulk admission and scan changes against this contract and corpus. The remaining program-wide gates stay open.

Evidence and semantic work can run in parallel once case IDs and the oracle interface are frozen. Source encoding work can use the same semantic version while the runtime explores backend choices. Native tier and ABI changes share a writer until they stabilize. Benchmarks use one local measurement lane.

### Corpus and measurement contract

Freeze case IDs, sources, input seeds and distributions, edge cases, checked semantics, outputs, toolchain, optimization profile, CPU features, telemetry and resource policy before tuning. Use more than one scale and branch distribution. Measure dynamic inputs with whole outputs or hashed output streams, and run full equality checks outside the timing loop. Compare a fast typed Rust kernel and a separate Rust host adapter that performs equivalent input validation, metering and effects. The first pass uses the kernel comparison.

| Group | Representative cases |
| --- | --- |
| Rules and control | Checked scalar arithmetic and errors, f32 NaN/ties and conversion, short-circuit fact order, recursion and budget boundaries, branch-heavy decision graphs |
| State and collections | Counter with helper calls and record updates, list scans and arg-min with NaN/ties, sparse/dense filters, nested enum/list transformations, sort/search, parser, hash map, deterministic graph traversal |
| Numeric and layout | Fixed-point volume interpolation, integer reduction, array and matrix loops, vectorizable f32 reduction with an explicit order, image or signal transform with varied size and alignment |
| Timeline and adapters | Single track and batch, translated and heterogeneous data, long key tables, all curves, browser and native host calls, scenario replay and instrumentation on/off |
| Load and update | Source and portable artifact decode, cold preparation, cache hit and miss, single and large edits, interface mismatch, stale revision, failed probe rollback, concurrent old/new invocations, real host loop |

Report median, spread, p95, units, hardware, settings, allocation and throughput where available. For token counting, report authored source, generated code, hand-written host bindings, shared grammar/API context, initial prompt and edit traffic separately. Use the same libraries, behaviors and interfaces on both sides. Track pass rates per model on independent held-out programs. Any safety proof must name the property, assumptions, target, checker and limitations.

## First integrated evidence

Windows x64, Ryzen 9 7945HX, Rust 1.99.1, Cargo release, `native-jit` enabled, telemetry off. Values are medians of two process medians in baseline/candidate/candidate/baseline order. The modern harness uses nine passes per process and interleaves Rust and FluxR within each pass. It checks individual results for up to 4,096 inputs and a full checksum per timed pass. The inherited volume harness uses five passes and checks all 20,000 inputs and two overflow cases. The modern baseline is commit `2b1fc1e` (`c8352b4` plus the identical harness). Volume baseline is `bd4dbca`.

| Execution workload | FluxR before, ns | FluxR after, ns | Matched Rust, ns | Change |
| --- | ---: | ---: | ---: | ---: |
| Structured scalar, direct | 100.48 | 21.40 | 0.80 | 4.70× faster |
| Structured scalar, injected | 100.27 | 21.37 | 0.79 | 4.69× faster |
| Structured record loop | 1,953.64 | 1,685.29 | 3.64 | 1.16× faster |
| Rule VM | 12.00 | 11.76 | 0.81 | About even |
| Prepared rule JIT | 3.19 | 3.16 | 0.81 | About even |
| Lazy flat gate | 9.90 | 1.34 | 0.44 | 7.41× faster |
| Timeline, 128 translated tracks per frame | 481.70 | 66.92 | 60.58 | 7.20× faster |
| Bound timeline track | 4.27 | 2.87 | 0.93 | 1.49× faster |
| Volume, structured VM | 902.36 | 866.21 | 6.29 | 1.04× faster |
| Volume, typed AOT kernel | 6.12 | 6.21 | 6.29 | About even |

All values are per call except volume, which is per sample. The Rust column is the equivalent checked kernel for the measured inputs. It excludes FluxR host representation, validation and resource dispatch. Volume AOT excludes the one-time admission work. Injected volume still takes the VM path and is about 140× slower than its AOT kernel. The record loop remains about 460× slower than the matched Rust kernel.

Scalar source compilation was about 2.4 µs in both builds. Activation of an already prepared clone took about 0.44 µs. Source compilation, activation and first successful call together rose from 2.64 to 7.37 µs with native preparation. Warm savings on this case recover that additional cost after roughly 60 calls. Legacy timeline text compilation was about 17.7 µs in both builds. These are separate lifecycle measurements.

Run `cargo run --locked --release --features native-jit --example bench_modern` on both modern refs, and `cargo run --locked --release --features native-jit --example bench_volume` on both volume refs. The original measurement retained per-process outputs, exact executable hashes and symbols in the local owner-request evidence store.

### Source counts

Counts use UTF-8 with LF and exclude special tokens. The panel uses tiktoken 0.14.0 and Hugging Face tokenizers 0.23.3. Qwen2.5-Coder-7B-Instruct is pinned to `c03e6d358207e414f1eca0bb1891e29f1db0e242`. StarCoder2-3B is pinned to `733247c55e3f73af49ce8e9c7949bf14af205928`. Source pairs describe the measured workloads. Hosts and prompt context are separate.

| Authored body | o200k | cl100k | Qwen Coder | StarCoder2 |
| --- | ---: | ---: | ---: | ---: |
| Structured scalar, FluxR / Rust | 30 / 35 | 30 / 35 | 36 / 41 | 40 / 50 |
| Counter, legacy FluxR / compact FluxR / Rust | 606 / 123 / 67 | 590 / 123 / 67 | 603 / 131 / 71 | 592 / 152 / 81 |
| Rule, FluxR / Rust | 15 / 35 | 15 / 35 | 18 / 41 | 19 / 50 |
| Gate, FluxR / Rust | 3 / 39 | 3 / 39 | 3 / 39 | 3 / 42 |
| Timeline, v1 FluxR / v2 FluxR / Rust | 2437 / 31 / 65 | 2437 / 31 / 65 | 3645 / 41 / 74 | 3645 / 41 / 81 |

The translated timeline retained 1.12–1.27% of legacy source tokens and used 47.7–55.4% of Rust tokens. The translated counter retained 20.3–25.7% of legacy tokens and used 184–188% of Rust tokens. The gate met 10% on all four encodings. Volume measured 147 authored FluxR tokens versus 160 Rust tokens on o200k, with 95 separate host binding tokens and 436 generated Rust tokens. The scalar invocation helper alone took 41–45 tokens. No model prompt or repair workload was executed.

The frozen pairs are listed in [benchmarks/cases.toml](../benchmarks/cases.toml). Run `python tools/bench_modern_tokens.py` with the pinned tokenizer JSON files passed via repeatable `--tokenizer` options. To verify the frozen counter source, build `fluxr-cli`, run `migrate-structured tests/fixtures/structured_counter.fluxm counter-compact.flux`, and pass `--counter-compact counter-compact.flux`. Use `python tools/bench_tokens.py volume` for the inherited volume count.

### Safety properties exercised

- **Timeline translation.** All tracks in the fast batch must have the same curve, key times and endpoint differences. Translating both endpoints by a constant leaves the phase, curve factor and signed adjustment unchanged. Adding that adjustment to each original first value therefore yields the same sample as independent interpolation. Both endpoints are i32 and interpolation stays between them. Differences and intermediate products fit i64 under the validated 600,000 ms duration and Q16 curve bound. The v2 decoder checks object IDs, total expanded tracks and keys, multiplication, signed addition, and final i32 conversion before normal compilation. Tests exercise signed extremes, malformed runs, sorted output, complete periods, and identical LSP bytes.
- **Structured native admission.** Only validated signed arithmetic, known host inputs/arguments, and a final return can reach this tier. Accepted programs have a static instruction sum, one frame, and a fixed cell count covering host values, slots, frame and result. The native emitter uses checked arithmetic and fixed scratch offsets. Invalid arguments and smaller limits go through the VM to preserve trap order. A successful first call may populate the immutable RX cache. Failed probes preserve the previous revision, and cached code belongs to the compiled body. Tests cover zero division, the signed-minimum remainder edge, overflow, budget and host-type boundaries, and replacement after a rejected probe. Machine-code safety still depends on the existing Windows emitter and its OS boundary.
- **VM dispatch.** Arithmetic and the next store or branch can share a dispatch. The second instruction still charges fuel after arithmetic succeeds, preserving which error wins. Call and return save and restore the caller PC only at frame boundaries. The suite exercises recursion, calls, budgets, failed injection, copy-on-write values and collection behavior.
- **Migration.** Structured source is re-parsed and compared to the original canonical bytes before emission. The migration smoke compares LSP, FSM and FXD artifacts before and after migration. The isolated live process verifies exact state across successful compact injection and failed-edit rollback. The WASM binding smoke replays the shipped timeline, verifies byte identity and checks v2 samples and rejection under Node.

## FXM-02.B shared typed tier

The checked VM instructions lower to typed blocks with explicit local reads and writes, calls, branches, and resource charges. The optional `typed-jit` feature adds Cranelift on Windows x64. The existing scalar leaf emitter takes priority for its supported subset. `emit-native-structured <source> <export> <output>` emits Rust from the same blocks. Its preparation binds the generated kernel to the canonical module bytes, export and ABI version. The runtime rebuilds native caches on body edits, including when the checked caller body is reused. [Cranelift code generation API](https://docs.rs/cranelift-codegen/0.136.3/cranelift_codegen/struct.Context.html)

The native tier admits checked signed arithmetic, scalar booleans, flat signed records, branches, loops and bounded acyclic calls. It preflights boundary shape and frame depth. Straight-line code uses a static resource bound. Loops debit fuel by block, debit cells for each frame, and reserve the full flat-record copy on each update. Unchanged records retain their original host identity. Native execution reads an isolated snapshot and publishes a result only on success. Any decline replays in the VM, which applies the original charges and reports the original first error. A decline can execute part of the program twice. Unsupported types, recursive calls and operand stacks crossing block boundaries remain on the VM.

The native call uses fixed lane offsets into checked Rust buffers, owned RX memory, guarded signed division and registered Win64 unwind information. Code with external relocations or unexpected trap metadata is rejected. This is a bounded pilot that trusts the checker, Cranelift and the platform. [Win64 function tables](https://learn.microsoft.com/en-us/windows/win32/api/winnt/nf-winnt-rtladdfunctiontable)

Measurements at `57702c5` on Ryzen 9 7945HX, Windows x64, Rust 1.99.1, Cargo release and telemetry off. A uses `native-jit`. B uses `typed-jit`. The process order was A/B/B/A at default affinity. Each process checked all 20,000 samples per workload and every timed checksum. The p50 and p95 are medians of the two process statistics over 21 timed pass means.

| Workload and route | p50 (ns/call) | p95 (ns/call) |
| --- | ---: | ---: |
| Volume VM, A direct | 838.89 | 844.75 |
| Volume JIT, B direct | 36.97 | 37.69 |
| Volume JIT, B injected | 36.85 | 37.41 |
| Volume shared AOT host, B | 20.59 | 20.86 |
| Volume prior AOT kernel, B | 7.02 | 7.20 |
| Volume checked Rust kernel / admitted host, B | 5.27 / 8.16 | 5.37 / 8.50 |
| Counter VM, A direct | 1696.05 | 1778.96 |
| Counter JIT, B direct | 90.10 | 91.16 |
| Counter JIT, B injected | 90.31 | 92.50 |
| Counter shared AOT host, B | 69.78 | 70.50 |
| Counter checked Rust kernel / admitted host, B | 6.88 / 32.44 | 7.10 / 33.06 |

Volume and counter improved 22.7× and 18.8× over A direct at p50. Injected and direct differed by less than 0.4% at p50 and 1.5% at p95 on both cases. JIT p50 was 4.53× and 2.78× the corresponding admitted Rust hosts. The Rust host measurements include value validation, budget admission and result materialization on successful samples. The program-wide Rust parity target remains open.

For volume and counter respectively, B median source compilation was 18.05 and 32.05 µs, JIT preparation was 508.35 and 446.20 µs, AOT preparation was 4.25 and 5.50 µs, prepared activation was 0.80 and 1.10 µs, and the first call after activation was 0.20 and 0.35 µs. In the pilot, Cranelift took 128.4 µs to prepare the small scalar leaf and ran it at 32.63 ns p50. The existing leaf emitter took 3.4 µs and ran it at 24.30 ns p50, so it retains priority.

Run `cargo run --locked --release --features native-jit --example bench_typed` and the same command with `typed-jit` in A/B/B/A order. Tests cover signed minimum division and remainder, malformed values, uninitialized locals, resource and arithmetic precedence, COW and snapshot identity, rejected probes, and edited callee invalidation. The isolated Windows host replay matched historical FSM bytes, regenerated both AOT fixtures and retained state after a rejected edit. Linux x64 and wasm32 passed compilation checks. The local evidence manifest binds the source, inputs, executable hashes and process logs.

## FXM-01.B frozen structured host corpus

The Windows corpus at `0dc2b7b` retains volume interpolation and the record counter. It adds a first-tie f32 arg-min at lengths 16 and 256, and a decision tree with eight leaves under uniform and skewed input. Each list population includes finite ties, first-element quiet NaNs and later signaling NaNs. The decision seed is `0xca3a5eedb52808ae`. The busiest uniform and skewed leaves account for 12.9% and 66.8% of their populations. [The benchmark](../examples/bench_typed.rs) fixes the input streams and [the runner](../tools/bench_host_corpus.py) checks their ordered output hashes.

Windows 11 x64, Ryzen 9 7945HX, logical CPU 12, Rust 1.99.1, Cargo release and telemetry off. A uses `native-jit` and B uses `typed-jit`, in A1/B1/B2/A2 process order. Each route checks every result before timing, then checks the ordered output hash after each of 21 timed passes. Each pass uses 20,000 calls for volume, counter and each decision distribution, and 2,048 calls per list length. The table reports ns per call as p50 / p95. Each figure is the median of two process statistics over pass means.

| Workload | A direct | B direct | B injected | B Rust kernel | B Rust host |
| --- | ---: | ---: | ---: | ---: | ---: |
| Volume | 856.32 / 860.96 | 38.04 / 40.52 | 38.11 / 39.90 | 7.34 / 7.62 | 8.59 / 9.03 |
| Counter | 1729.46 / 1792.65 | 93.82 / 99.26 | 93.73 / 95.88 | 7.05 / 8.18 | 33.65 / 35.34 |
| Scan, 16 values | 109.00 / 144.71 | 112.89 / 125.95 | 112.60 / 129.59 | 7.50 / 10.32 | 14.84 / 18.04 |
| Scan, 256 values | 883.72 / 1084.79 | 947.90 / 1038.77 | 954.50 / 1073.97 | 177.54 / 209.00 | 258.79 / 396.81 |
| Decision, uniform | 114.54 / 127.36 | 32.62 / 35.08 | 32.47 / 33.42 | 9.34 / 10.59 | 9.88 / 11.41 |
| Decision, skewed | 102.95 / 107.25 | 28.56 / 30.65 | 28.30 / 29.57 | 3.49 / 4.38 | 3.83 / 4.83 |

The Rust host column includes input shape validation, budget admission and result materialization for successful calls. Scalar and record adapters can use a conservative bound. Exact diagnostics and trap precedence are checked across the FluxR routes. B shared AOT host p50 / p95 was 21.81 / 23.80 ns for volume and 72.93 / 75.02 ns for counter. The order-sensitive fold and CPU affinity differ from FXM-02.B. Compare variants within each run.

The scan declines native preparation. At length 256, its VM p50 was 948.83 ns and the admission control was 793.95 ns. At length 16 the corresponding values were 109.13 and 88.94 ns. The control retains the same list boundary validation and argument setup and returns a constant. It accounts for 82–84% of the measured VM time. The scan at length 256 has the largest absolute gap to its Rust host at about 689 ns per call. FXM-05.A should cover cell charging before type errors, first-element and later NaN behavior, first ties, scan fuel before evaluation, and the result cell charge. Validate this effect order before specializing admission.

| Lifecycle p50 (µs) | Source compile | Native prepare | Activate after preparation attempt | First call, reused workspace |
| --- | ---: | ---: | ---: | ---: |
| Volume | 17.65 | 526.60 | 0.90 | 0.15 |
| Counter | 35.10 | 477.70 | 1.15 | 0.40 |
| Scan | 2.05 | 0.10 (declined) | 0.15 | 0.10 |
| Decision | 16.25 | 435.40 | 0.80 | 0.20 |

The replay checks 35 exact cases each on direct, portable, decoded and injected routes. The same assertions pass in the `wasm32-unknown-unknown` release build under Node 22.14.0 with wasm-bindgen 0.2.130. Linux x64 passes a compilation check. Native Linux execution needs a runner because WSL is absent here. ARM64 needs an installed target and a runner. WPR listed hardware counter sources, and the session lacked elevation to collect them. The admission control does not give instruction, cycle, or cache attribution.

On tiktoken 0.14.0, the authored scan is 21 FluxR tokens versus 96 Rust kernel tokens on both o200k and cl100k. The decision is 136 versus 161 on both. The Rust host adapter and its local helpers add 223 tokens for the scan and 154 / 153 for the decision. The counts use UTF-8 with LF and exclude task context and generated code. These corpus cases are outside the held-out evaluation.

Reproduce with `python tools/bench_host_corpus.py <new-evidence-directory> --cpu 12 --wasm --check-target x86_64-unknown-linux-gnu` from a clean tracked checkout. The manifest records the source commit, build commands, affinity, target checks, expected hashes, and SHA-256 of the binaries, symbols and logs.

## FXM-05.A list admission and scan effects

After checking counts, the boundary visits inputs in declaration order and then arguments in parameter order. For each value, it checks depth, debits one cell, and checks the tag. For a list, it then visits the elements in index order. The flat `[f32]` visitor enters after the outer list check. After any valid prefix of `k` elements, both the recursive visitor and the typed visitor have debited the same `k` cells. On the next element, they check depth before the debit and debit before checking the tag. A malformed element outside the affordable prefix therefore produces a cell error. A malformed element inside it produces a type error. An empty list visits no children. Nested lists still reach the flat visitor at their actual depth.

After boundary admission, the VM checks frame capacity and charges frame and slot cells. The checker accepts `minidx` only on `[f32]` in v2 and assigns its result `u64`. Execution charges operand evaluation and the scan's base instruction, then debits the entire list length before comparing elements. After each nonempty prefix, the retained pair is the first minimum among ordered values, except that a NaN in position zero remains selected. Strict less-than preserves ties, including either ordering of signed zeros. Later NaNs cannot replace the selected element. An empty scan yields zero. Execution then continues with the following instruction's fuel and traps. The result cell is charged only after a successful return.

The portable test uses a bit-order oracle and covers every sequence of up to three values from nine representative IEEE 754 encodings. It checks exact and short budgets, malformed tags at each position, competing boundary errors and downstream traps. The 4,141 cases pass on direct, portable, decoded and injected routes, including under Node on Wasm. The frozen 35 corpus checks still pass. Release builds with and without `typed-jit` pass the Windows suite, and Linux x64 passes a release compilation check. Reproduce the differential check with `cargo test --locked --release --features typed-jit --example bench_typed list_effects_match_oracle`. The Node module produced from the Wasm example exports `list_effect_check`. The proof covers the flat boundary visitor and checked VM scan. Native preparation still declines this export.

The local evidence at `list-effects/manifest.json` compares source `4e126cd` to the frozen `0dc2b7b` typed executable in A/B/B/A order. Both use the same CPU 12 affinity, toolchain, release profile, telemetry setting and corpus hashes as FXM-01.B. Values are ns per call, p50 / p95, with the median of two process statistics per variant.

| Route | Before | After |
| --- | ---: | ---: |
| 16 values, VM scan | 111.13 / 114.43 | 76.88 / 78.63 |
| 16 values, boundary control | 91.31 / 100.27 | 56.08 / 72.58 |
| 16 values, matched Rust host | 15.34 / 15.79 | 14.91 / 16.29 |
| 256 values, VM scan | 931.25 / 1063.43 | 345.98 / 490.94 |
| 256 values, boundary control | 739.97 / 834.62 | 153.95 / 170.82 |
| 256 values, matched Rust host | 370.61 / 424.32 | 255.20 / 416.16 |

The typed visitor improved scan VM p50 by 1.45× and 2.69×. The 256-value Rust host p50 varied from 223 to 415 ns across processes. The p50 gap to Rust in B was 5.16× at length 16 and 1.36× at 256. Median scan source compilation was 2.10 / 2.15 µs before and after. Preparation declined in about 0.10 µs, activation after the attempt took 0.20 µs, and the first call on a reused workspace took 0.10 µs in both variants.

Keep the timing misses in FXM-02.C. The injected 16-value p95 was 14.1% above direct, and counter injected p50 was 5.5% above direct. Counter and volume AOT host p50s were higher by 12% and 18%, respectively. The unrelated routes and Rust controls varied across the processes. The run establishes the list improvement under this configuration and leaves the causes of those differences open.

## FXM-02.C.1 portable list runtime

The flat visitor checks the common child depth once for nonempty lists. It inspects only the affordable prefix, so a bad tag wins exactly when its element can be charged. An all-valid prefix yields the original cell error if any elements remain. Successful admission debits the same total as the recursive visitor. Failure discards the invocation budget.

A checked body consisting of a host input or parameter load, `minidx`, and return can borrow the admitted list. It charges frame cells, the load and scan dispatch, full scan fuel, return fuel, and result cells in that order. The admitted load cannot trap between the first two fuel charges. The general interpreter handles computed lists and uninitialized locals. Both paths use the same comparison loop. The portable format still encodes the canonical source.

The retained Windows x64 run compares the FXM-05.A binary from `4e126cd` with `a06e588` in A/B/B/A order. Both use `typed-jit`, logical CPU 12, Rust 1.99.1, Cargo release, and telemetry off. Figures are ns per call, p50 / p95, using the median of two process statistics over 21 pass means. The B/A/A/B recheck uses the same binaries.

| Route | Before | After |
| --- | ---: | ---: |
| 16 values, portable scan | 77.03 / 79.12 | 39.06 / 41.45 |
| 16 values, boundary control | 56.64 / 59.52 | 54.03 / 56.27 |
| 16 values, matched Rust host | 14.77 / 16.60 | 14.72 / 16.36 |
| 256 values, portable scan | 314.14 / 368.26 | 262.94 / 292.48 |
| 256 values, boundary control | 149.54 / 174.15 | 131.91 / 139.99 |
| 256 values, matched Rust host | 223.44 / 275.12 | 220.63 / 283.03 |

The recheck reproduced the portable p50 gains at 76.15 to 38.91 ns and 308.78 to 259.27 ns. Two earlier comparison-loop candidates reduced long-scan p50 but regressed short-scan p50. The accepted change restores the original comparison loop. Windows release suites with and without `typed-jit` passed, including the focused uninitialized-list test. Windows and Node Wasm passed 35 corpus and 4,141 list effect checks on each route. Linux x64 was compile-checked.

The candidate met the injection thresholds on the two-process statistics for each workload in both trials. One recheck process missed p95 parity for volume (+16.72%) and the 256-value scan (+12.22%). The original short-scan and counter misses did not recur in the candidate. In the recheck, volume and counter AOT p50 were 25.48 and 76.81 ns, within 1.4% of their matched controls. Direct typed JIT remained 11.93 ns above AOT for volume and 15.01 ns for counter. This is a host-route difference, with kernel and dispatch attribution still open. Across the six workloads with direct and injected routes, the candidate's geometric mean of direct/Rust host ratios was 3.18× at p50 and 2.91× at p95 in the primary run. The scan still declines native preparation.

Candidate scan source compilation took 2.25 µs at p50, the native preparation decline took 0.10 µs, activation took 0.15 µs, and the first call took 0.05 µs on a reused workspace. The last figure is near the timer resolution. Local evidence is retained in `list-runtime-leaf/manifest.json` and `list-runtime-leaf-recheck/manifest.json`, with scripts, binary hashes, symbols, logs, and the two comparison-loop trials.

## FXM-02.C.2 native host dispatch

A local diagnostic overlay used the frozen inputs to compare the full public route, the same typed boundary with the export resolved ahead of time, and prepacked calls to the same JIT and AOT kernels. Each control checked all 20,000 outputs against the same oracle and checksum. Figures are p50 ns per call, taking the median of two process statistics over 21 pass means. The prepacked routes include result extraction and harness costs. Their differences from host routes are comparative measurements, not cycle attribution.

| Workload | Full JIT | Resolved boundary | Prepacked JIT | Prepacked AOT |
| --- | ---: | ---: | ---: | ---: |
| Volume | 36.38 | 21.63 | 9.82 | 10.22 |
| Counter | 91.55 | 78.52 | 29.34 | 21.43 |
| Uniform decision | 32.57 | 19.63 | 11.73 | — |
| Skewed decision | 28.51 | 15.77 | 6.51 | — |

Pre-resolving the native boundary reduced p50 by 12.7–14.8 ns across the controls. The counter retained a 7.9 ns gap between the two prepacked routes. Volume's prepacked timings were close. WPR hardware counters require elevation on this host. These are source-bound controls with telemetry off and no profiler attached.

The accepted change returns successful native calls before portable scratch cleanup. Portable calls and native declines use the same interpreter and release all scratch values on success or error. Export lookup, admission, scalar and typed tier ordering, the signed and record ABI, and artifact bytes are preserved. The counter's shared-workspace test covers alternating admitted and declined budgets and arithmetic errors. A forced out-of-line fallback slowed short scans, contiguous export lookup had no repeatable native win and slowed short scans, and single-field record construction had no repeatable counter win.

The Windows x64 comparison uses logical CPU 12, Rust 1.99.1, Cargo release with `typed-jit`, and the frozen binaries from `a06e588` and `652d21d`. Figures are ns per call, p50 / p95, using the median of two process statistics over 21 pass means.

| Route | Before, ABBA | After, ABBA | Before, BAAB | After, BAAB |
| --- | ---: | ---: | ---: | ---: |
| Volume, direct | 37.25 / 37.54 | 30.58 / 31.05 | 38.30 / 39.33 | 30.84 / 31.36 |
| Counter, direct | 91.81 / 92.25 | 85.81 / 86.64 | 91.65 / 92.10 | 88.28 / 89.00 |
| Uniform decision, direct | 32.34 / 32.60 | 26.09 / 26.46 | 32.62 / 32.92 | 26.19 / 26.42 |
| Skewed decision, direct | 28.70 / 28.99 | 21.63 / 22.02 | 28.77 / 29.12 | 21.70 / 21.95 |
| Scalar leaf, direct | 22.31 / 22.81 | 19.27 / 19.55 | 22.55 / 23.25 | 19.34 / 19.66 |
| 16 values, portable scan | 39.21 / 41.19 | 38.99 / 40.89 | 38.81 / 40.67 | 39.45 / 41.97 |
| 256 values, portable scan | 259.79 / 293.97 | 259.55 / 282.56 | 259.45 / 279.78 | 268.01 / 306.06 |

All six dual-route two-process statistics met the injection thresholds in both trials. The BAAB candidate B2 long scan exceeded the two-sided p95 threshold by measuring 12.70% faster injected than direct. An earlier run with the same candidate binary had a two-process long-scan injection p95 11.57% slower than direct, so tail parity remains open. Portable decision p50 was 3.5–5.8% higher in the committed-source trials, with mixed results across earlier runs of the identical candidate binary. The scan still declines native preparation. Direct/Rust host geometric means over the six dual routes improved from 3.18× / 2.94× to 2.79× / 2.62× in ABBA and from 3.20× / 3.01× to 2.83× / 2.66× in BAAB. These exceed the program targets.

In ABBA the candidate volume, counter and decision source compilation p50s were 17.6, 34.5 and 15.6 µs. Native preparation p50s were 498, 453 and 412 µs, activation p50s 0.8, 1.1 and 0.8 µs, and first calls 0.2, 0.4 and 0.2 µs on reused workspaces. First-call times are near timer resolution.

Windows release suites passed with `typed-jit`, `native-jit` alone, and the portable feature configuration. Windows and Node Wasm passed 35 corpus and 4,141 exact list-effect cases per route. Linux x64 was compile-checked. The local `native-boundary/manifest.json` retains exact binaries, symbols, the control patch, matched trials, rejected candidates and validation logs. Its SHA-256 is `22A338DEAC70AA3E16997461F4BA728A4684927FEB8B7B1D6887BCC758819B7D`.

FXM-02.C.3 carries forward native scan feasibility, broader typed effects and injection tail parity. A native scan candidate must keep exact portable fallback, the full fuel-before-scan rule, and float-bit replay at both lengths. FXM-05, FXM-03.B, FXM-04.A and FXM-06.A retain their wider gates.

## FXM-02.C.3 prepared float scan

Win64 `typed-jit` now prepares a precompiled Rust kernel for a checked body that directly scans one `[f32]` argument or one host input and returns its `u64` index. The kernel borrows the host list and uses the interpreter's strict comparison loop. The admission guard reserves the wrapper, frame and result cells, one cell and one fuel unit per element, and all static fuel. It confirms every element tag before the loop. A decline replays the portable evaluator, which keeps admission and trap precedence. The signed and record JIT ABI and portable artifact bytes are unchanged.

The frozen Windows x64 release comparison uses `652d21d` and `21bb131`, logical CPU 12, Rust 1.99.1, `typed-jit`, and telemetry off. Each cell gives p50 / p95 ns per call, using the median of two process statistics over 21 pass means.

| Route | Before, ABBA | After, ABBA | Before, BAAB | After, BAAB |
| --- | ---: | ---: | ---: | ---: |
| 16 values, direct scan | 41.77 / 43.61 | 25.51 / 26.49 | 40.94 / 41.66 | 25.83 / 29.32 |
| 256 values, direct scan | 257.13 / 267.36 | 235.94 / 252.84 | 255.86 / 268.00 | 237.30 / 256.03 |
| Volume, direct | 30.31 / 30.82 | 30.29 / 30.78 | 30.59 / 31.09 | 30.09 / 30.47 |
| Uniform decision, portable | 118.15 / 119.17 | 115.27 / 116.68 | 119.18 / 120.07 | 115.86 / 116.61 |
| Skewed decision, portable | 106.58 / 108.39 | 103.78 / 105.10 | 106.36 / 107.06 | 104.24 / 105.18 |

The direct scan gained 37–39% at 16 values and 7–8% at 256 values at p50. The 256-value portable control rose 0.6–2.3% at p50 and 3.5–4.4% at p95. The scan routine is out of line after the first layout showed mixed volume and decision regressions. In the final trials the candidate's direct/Rust host geometric means were 2.55× / 2.42× in ABBA and 2.50× / 2.39× in BAAB. Both exceed the program targets.

The six two-process injection comparisons passed the 5% p50 and 10% p95 thresholds in both orders. Individual BAAB short-scan processes measured −10.82% and +15.19% injected/direct p95, and one ABBA volume process measured −6.41% injected/direct p50. The parity gate remains open. Candidate scan compilation was 2.15–2.25 µs p50, preparation 0.05–0.10 µs, and activation 0.15–0.20 µs. First calls were at timer resolution.

Windows release suites passed in all three feature modes. Windows and Node Wasm replay passed 35 corpus and 4,164 list cases per route, including signed zeros, first and later NaNs, and exact metering at both scan lengths. Linux x64 was compile-checked. Local `typed-effects/manifest.json` retains the exact binaries, settings, oracle logs, and earlier layout trials. Its SHA-256 is `43FD6F95C45050FD00C66C06FAFAF44609D211AC2D96D9402EB68BA33E58EB1F`.

FXM-05.B next extends the executable semantic specification and independent checks for typed effects. FXM-02.D retains the native/Rust target, injection parity and portable timing work. FXM-03.B, FXM-04.A and FXM-06.A retain their original gates.

## FXM-05.B typed effects and portable boundary

The executable specification models the checked v2 float-list fragment with host values, source expressions, and cumulative budget use derived from the source semantics. The checker supplies the same cases to direct execution, the portable evaluator, the decoded portable artifact, and an injected module. It prepares eligible exports and reuses one workspace across successes and failures. [Executable specification](../examples/fixtures/typed_effect_spec.rs)

### Scan obligations

For a single admitted list of length `n`, let `s` be the number of slots in the export. Both borrowed shapes require one frame, `n + s + 3` cells and `n + 3` fuel. Admission charges the wrapper and each element in order. After an affordable prefix of valid elements, the next element fails on cells when the prefix has exhausted the budget. If its cell is affordable, its tag determines whether admission continues. This gives the flat visitor and the recursive specification the same first failure. Nested list fixtures exercise the child visitor before execution.

The frame and slots are charged after admission. Operand evaluation and scan dispatch cost one fuel each, then the scan requires all `n` units before comparing any values. Return costs one fuel, and the result costs one cell after return. A first NaN selects index zero while still requiring the full debit. Otherwise, the least integer-ordered pair of float key and index agrees with strict floating-point comparison, including coalesced signed zeros, skipped later NaNs and first ties. The empty result is zero.

For the two prepared shapes, exact arity and tags plus the total budget inequalities imply every debit succeeds. On a decline the portable evaluator decides which failure wins. The separate portable expressions check literal allocation, a nested index, an uninitialized load, and an out-of-bounds index after an empty scan. The checker covers lists up to length three exhaustively over nine bit patterns, budget edges at lengths 16 and 256, malformed tags and counts, and 256 deterministic randomized cases with seed `0x505b7a5d`. It also checks rejection of v1 scans and invalid operand or result types.

### Host boundary and inventory

- **Snapshot capability.** A slot grants a declared input by name and exact type. Calls supply a read-only positional snapshot in declaration order. An update must retain ordered input bindings, exports and signatures, and declared record and enum layouts. Revision and interface checks precede the optional probe. A failed probe leaves the active revision and program available. The checker tests grant and positional mismatches, stale and incompatible updates, rollback, and successful replacement with the Win64 signed-record tier prepared where available.
- **ABI and artifact.** Structured artifacts use the `FSM\0` prefix and little-endian source version followed by canonical source. Decode rechecks the source and its version. The checker rejects all proper prefixes of one artifact, one mutation at each envelope byte, invalid UTF-8, and three ill-typed or malformed payloads. The signed-record JIT still uses the Win64 `extern "system"` lane ABI, with bounded input and result arrays, fuel and cell arguments, and a status return. Its record origin lane identifies an existing host value or a newly written record. The borrowed scan is a Rust leaf over tagged values. Existing signed arithmetic, record update and fallback cases run in the release matrix.
- **Coverage.** Structured v2 supports fixed-width checked integers, f32, booleans, records, variants, lists, procedures and methods, bounded recursion, control flow, and typed snapshot inputs. Native preparation remains shape-specific and the portable evaluator handles the wider subset. The delivery inventory still has open gates for modules and imports, generics, closures, slices, strings, maps, arrays, iterators, host I/O capabilities, package and standard library identity, cancellation and concurrency. The boundary evidence is for this subset. Whole-backend memory and control-flow verification remains open.

### Universality witness

The tape simulator takes the transition table of a deterministic single-tape machine with a finite alphabet, numbered nonhalting states and zero as blank and halt. A state and the scanned symbol select a row. The row supplies the write, the new state and the motion. For right motion, the written cell becomes the nearest left cell and the nearest right cell becomes the head, or blank if that half is empty. Left motion is symmetric. By induction on steps, the two lists and head represent exactly the simulated tape. With abstract growable lists and repeated invocations, the program is a universal simulator. Concrete executions remain bounded by the host's fuel, cells and frame limits. [FluxR tape simulator](../examples/fixtures/universal_tape.flux)

The independent Rust checker uses absolute positions and sparse memory. It compares a halting two-state machine, a nonhalting blank-tape walk over a bounded prefix, and a three-symbol rewrite. A caller can request a finite number of transitions, retain the returned tape and state, and resume in a fresh call. On fuel exhaustion the caller can retry from the previous checkpoint. A budget error returns no intermediate state.

### Reproduction

Run `cargo test --locked --release --features typed-jit --example bench_typed` on Windows x64. Run the same suite with `native-jit` and with no features. For Node, build the release example for `wasm32-unknown-unknown` and use `wasm-bindgen --target nodejs`. Call `host_corpus_check`, `list_effect_check`, `typed_effect_check`, `universal_tape_check` and `structured_boundary_check` on the generated module.

At `ec8083d`, Windows x64 passed 49 release tests with `typed-jit` and 47 each with `native-jit` and with no features. The five example tests passed in all three configurations. Those tests and Node Wasm exercised 35 frozen corpus cases, 4,164 earlier list cases, 19,770 specification cases and 75 tape cases per route, plus 761 malformed artifacts and the depth and capability assertions. The frozen FXM-02.C.3 Node binary repeated the 35 and 4,164 counts. Linux x64 was compile-checked. ARM64 execution remains open. Local `typed-effect-spec/manifest.json` retains the commands, hashes and results. Its SHA-256 is `FF162F2EB9F45838DA26C37D3719F1A64A73FA6B009313F415F4EAB72DA07762`.

FXM-02.D next works on matched Rust-host parity, injection parity and the long portable scan tails. The broader FXM-05 safety and coverage gates remain in scope alongside FXM-03.B, FXM-04.A and FXM-06.A.

## FXM-02.D.1 dispatch controls and scan trials

The resolved-export and prepacked controls were rebuilt on the frozen runtime. At p50, resolving the export saved 6.1–8.5 ns across volume, counter and both decision distributions. The resolved counter boundary took 80.04 ns, compared with 29.80 ns for the prepacked JIT and 21.84 ns for the prepacked AOT control. These figures include harness and result handling. Windows hardware counters require elevation, so the controls do not attribute cycles to individual instructions.

An outlined candidate shared a four-value tag check between portable admission and the Win64 prepared scan. It preserved the affordable prefix and the native decline path. Preliminary trials showed 4.7–8.6% lower long direct p50 and 3.2–8.4% lower long portable p50. All four preliminary candidate processes met injection thresholds.

The confirmation of the committed candidate used Windows x64, logical CPU 12, Rust 1.99.1, `typed-jit` and telemetry off. Each cell shows p50 / p95 ns per call, using the median of two process statistics over 21 pass means.

| Route | Before, ABBA | After, ABBA | Before, BAAB | After, BAAB |
| --- | ---: | ---: | ---: | ---: |
| 16 values, direct | 26.12 / 27.03 | 26.69 / 27.63 | 26.39 / 28.88 | 25.59 / 27.62 |
| 16 values, portable | 39.70 / 40.75 | 41.29 / 42.85 | 39.58 / 41.36 | 41.75 / 44.54 |
| 256 values, direct | 240.48 / 295.90 | 224.66 / 248.90 | 241.31 / 256.30 | 245.36 / 272.15 |
| 256 values, portable | 260.03 / 304.86 | 241.56 / 289.41 | 262.06 / 300.15 | 260.94 / 298.22 |

Short portable p50 rose 4.0–5.5%. Long direct results were mixed, and one BAAB candidate process missed long-scan injection p95 at +17.75%. Both two-process confirmation statistics met the injection thresholds. A compiler was observed during an earlier committed-binary recheck that had aggregate misses. Its effect on those results is unknown. The Win64-only variant reduced long direct p50 in both orders and missed aggregate injection thresholds in both orders. The fused loop, first-element initialization, unsigned-borrow JIT meter, inlined groups and length-gated groups also failed to give repeatable gains across the relevant routes.

The outlined candidate passed the Windows release suites in all three feature modes. Node Wasm replay passed the same 35 corpus, 4,164 list, 19,770 specification, 75 tape and 761 malformed artifact cases as FXM-05.B. Linux x64 was compile-checked. The candidate's Rust-host confirmation geometric means were 2.51× / 2.37× and 2.46× / 2.25×. Scan compilation p50 was near 2 µs and preparation and activation were near 0.1 and 0.2 µs. First calls were at timer resolution.

The experiments were reverted and the frozen runtime remains the integration baseline. Local `runtime-parity/manifest.json` SHA-256 `D378F3D8246DF455D6627308C036501430CEA1F35B0F66CFC7FBD5E4D13B8698` binds the controls, exact binaries, source patches, replays and per-process results. The next runtime pass should isolate signed/record boundary work and use another tail collection method before accepting a scan change. FXM-03.B can advance independently using the frozen runtime and released-format readers.

## FXM-03.B.1 migration receipts and historical replay

Each migration recompiles its generated source and compares canonical artifacts before exclusive output creation. The optional receipt binds exact input, output, and canonical artifact hashes. An optional bounded replay compares timeline samples, rule results and traps, or structured portable invocations with exact value and error equality. FR1 uses the released packet reader and nominal binding. Its receipt also binds the interface and frozen alphabet.

| Reader | Verified source and encoding |
| --- | --- |
| Timeline | v1 and v2 text, v1 JSON, and LSP v1. The existing grouped-track test covers v2 grouping, signed shifts and wraparound. |
| Rule | v1–v4 postfix text, `.rule.json`, and FXD. The replay includes checked division, bitwise and, guard short-circuit, canonical NaN and signed zero. |
| Structured | v1 JSON, compact source and FSM, and v2 compact source and FSM. Noncanonical FSM whitespace has its own input hash and the same canonical artifact hash. |
| FR1 | A frozen packet exercises both ordered effects and a collection export with the historical nominal interface and alphabet. The released compiler and migration produce the same FSM v2 bytes. |

The archived `bd4dbca` and current `2d094ae` Windows x64 release binaries matched across 23 migration inputs and 126 probes. Seven malformed or over-budget inputs were refused without output. All-target release suites passed 53 tests each in portable and `native-jit` modes and 55 in `typed-jit` mode. Node Wasm re-encoded the migrated timeline and rule to identical bytes and reran the frozen structured checks. Linux x64 passed a compile check. Native Linux and ARM64 execution remain open.

Local `legacy-migration/manifest.json` SHA-256 `FC0E21340B3D5FD73BD990E04A9747C407CB2E3E627BF77BDF29766D915D3F76` binds the archived sources, binaries, exact inputs, outputs, and logs. FXM-03.B.2 carries shared rule and FR1 HIR ingress and versioned IR round trips. Preserve eager rule select, guard exits, raw binary32 transport, exact traps and the released artifact readers. FXM-02.D, FXM-04.A, wider FXM-05, and FXM-06.A remain open.

## FXM-03.B.2 checked HIR and FXH v1

FR1 binds its released packet alphabet and nominal interface, lowers into
structured `Source`, and passes the existing module checker. Rules project
into the same `Source` where the structured type system can express their
computation. The projection stores each operand in postfix order before
selecting, makes a failed guard return zero at its original position, and
checks signed arithmetic. Its original FXD retains exact diagnostics and
the legacy host contract. Raw binary32 transport and bitwise and refuse
structured projection. The checked rule dialect carries those operations,
including signed zero and canonical NaN, through exact FXD restoration.

FXH v1 uses the four bytes `FXH\0`, a version byte, a kind byte, and a
little-endian u32 payload length. Kind 0 carries canonical compact structured
source. Kind 1 carries a little-endian u16 FXD length, the checked FXD, and
the canonical compact projection if available. Decoding checks lengths,
rechecks the origin and regenerates the canonical payload before accepting.
Migration can bind exact input, FXH, artifact and bounded replay hashes.
FR1 also binds the nominal interface and alphabet. Restore emits canonical
FXD or FSM for the existing execution paths and metering.

Windows release testing passed 56/56/58 tests in portable, native-jit and
typed-jit modes. FXH matched the archived FXD/FSM bytes on 19 historical
inputs and 82 replay probes. Ten malformed or over-budget cases were refused
without output. The shared rule projection separately matched results or
checked arithmetic traps on 637 probes. Node Wasm restored three FXH
fixtures and passed the frozen structured suite. Linux x64 compiled.

| Decode and validation | Legacy p50 / p95 (ns) | FXH p50 / p95 (ns) |
| --- | ---: | ---: |
| Guard rule | 64.60 / 72.00 | 12,960.40 / 13,052.64 |
| Binary32 rule | 51.18 / 55.46 | 530.31 / 535.83 |
| FR1 module | 10,668.89 / 12,590.63 | 17,788.66 / 19,938.11 |

The measurements use Windows x64 portable release with telemetry off.
Each value averages the corresponding percentile from two processes, each
with 11 interleaved rounds of 10,000 decodes per route. The percentiles
describe round averages. FXH adds preparation work, including rebuilding
and checking the structured projection where available.

Local `shared-hir/manifest.json` SHA-256
`921B178C4D7BD39AC08BAC5A6251D2A63A1CB84296B0F20433125C61D41D55DD`
binds source `939a68c`, artifacts, scripts, replays and measurements.
FXM-02.D next resumes Rust-host and injection parity and portable scan
tails. FXM-04.A, wider FXM-05 and FXM-06.A remain open.

## FXM-02.D.2 matched source and artifact injection

The benchmark now compares the bundled module with independently injected
source and decoded artifact modules at the same prepared tier. It checks
every result and an order-sensitive checksum. On Windows x64, logical CPU 12,
Rust 1.99.1 release with `typed-jit` and telemetry off, each full-corpus cell
is the median of two process statistics over 21 pass means. Scan blocks
rotate route positions and report 672 means of 64 calls per process.

| Workload, ABBA | Rust host p50 / p95 (ns) | Bundled | Source injection | Artifact injection |
| --- | ---: | ---: | ---: | ---: |
| Volume | 14.32 / 16.14 | 31.07 / 34.17 | 30.84 / 34.69 | 31.73 / 35.19 |
| Counter | 41.94 / 42.42 | 87.30 / 88.21 | 87.44 / 88.31 | 87.42 / 88.56 |
| Scan, 16 values | 14.57 / 18.66 | 27.41 / 34.81 | 27.44 / 30.66 | 28.10 / 32.47 |
| Scan, 256 values | 230.88 / 254.49 | 253.64 / 299.88 | 253.74 / 295.05 | 252.78 / 280.91 |
| Uniform decision | 9.64 / 10.89 | 26.84 / 28.05 | 26.86 / 29.06 | 26.92 / 28.92 |
| Skewed decision | 3.33 / 5.11 | 22.39 / 22.70 | 22.49 / 22.88 | 22.48 / 23.21 |

Bundled/Rust-host geometric means were 2.36× / 2.19× in ABBA and
2.37× / 2.17× in BAAB. All six aggregate p50 injection comparisons met the
5% threshold for both modes and orders. ABBA source-injected short-scan
p95 missed the two-sided 10% threshold at −11.92%. BAAB aggregates passed.
Individual full-pass misses were ABBA B2 source short-scan p95 −20.53%,
artifact volume p50 +5.69% and artifact long-scan p95 −11.97%. BAAB B1
artifact short-scan p95 was −10.32%. BAAB B2 source and artifact long-scan
p95 were −10.48% and −11.31%. The older named routes also missed long-scan
p95 in ABBA B2 at −17.25%. The 64-call block artifact short scan missed
p50 in ABBA B2 at +5.87%. The portable 256-value block p50 / p95 was
242.19 / 357.81 ns in ABBA and 241.41 / 332.81 ns in BAAB.

Preparation is separate from execution. Across the two orders, p50 for the
scan was 2.10–2.15 µs to compile source and 2.75–2.90 µs to decode the
artifact. Native preparation was 0.05 µs, activation 0.20 µs, and first calls
and cache hits were at timer resolution. Source and artifact native
preparation took 411.6–522.5 µs across volume, counter and decision.
The benchmark reports load, native and AOT preparation, cache-hit,
activation and first-call distributions separately from steady-state execution.

The resolved handle saved 3.76–15.02 ns p50 against the same binary's named
routes. Its combined candidate increased the established named short-scan
p50 by about 25% and had additional injection misses. The scan-only trial
preserved the named dispatch API. Its long portable p50 changed from
275.49 to 335.89 ns in ABBA and from 263.24 to 259.38 ns in BAAB. The Rust
host control also slowed from 238.28 to 297.98 ns in ABBA. These trials
do not establish a repeatable portable gain. Kernel hardware counters
require elevation. The Windows release suites passed 56/56/58 tests, the
five Node Wasm suites passed, and Linux x64 compiled. The branch retains
the benchmark and restores the frozen runtime after both experiments.

Local `runtime-parity-resume/manifest.json` SHA-256 `89F1A340B2890CBF9AD197751FC8E134FBAAA232672E42834E8B431B7A364F1F` binds the
exact binaries, patches, logs, reproduction script, and per-process misses.
FXM-02.D remains open. Its next pass should isolate signed and record
boundary costs with an identical named harness and retain both source and
artifact injection controls. FXM-04.A can advance against the frozen
semantics while runtime parity, wider FXM-05, and FXM-06.A remain open.

## FXM-04.A held-out source and edit baseline

The six-tokenizer panel spans five model families. Tokenizer revisions,
content hashes, library versions, and the live pilot settings are pinned in
[token-panel.toml](../benchmarks/token-panel.toml). The held-out cases and
their prompts were sealed before counting. The release replay passed all
seven original and seven edited pairs, covering checked arithmetic, eager
rule selection, ordered short-circuiting, timeline translation, raw binary32
results, collection bounds, and record updates. The replay compares values
and application traps for the sampled inputs with the VM's default budget.
Execution speed and Rust host metering use the separate runtime corpus.

Each cell reports median / aggregate / nearest-rank p95 of FluxR divided by
Rust. Aggregate divides total counts. With seven cases, p95 is the largest
per-case ratio. Source counts include declarations. Edit counts include the
replacement span, with the original source and exact anchor counted as input
context. Both arms use the same edit protocol. Counts exclude special tokens
and provider framing.

| Tokenizer | Family | Source | Edit |
| --- | --- | ---: | ---: |
| o200k_base | OpenAI | 38.7% / 48.2% / 84.6% | 50.0% / 66.7% / 100.0% |
| cl100k_base | OpenAI | 38.7% / 48.8% / 84.8% | 50.0% / 66.7% / 100.0% |
| Qwen2.5-Coder | Qwen | 40.6% / 50.6% / 85.5% | 50.0% / 66.7% / 100.0% |
| StarCoder2 | StarCoder | 38.9% / 49.6% / 87.8% | 50.0% / 55.6% / 100.0% |
| Mistral-7B | Mistral | 35.8% / 48.6% / 90.1% | 50.0% / 61.1% / 150.0% |
| OLMo 2 | OLMo | 38.7% / 48.8% / 84.8% | 50.0% / 66.7% / 100.0% |

Zero of six tokenizers met the joint median and aggregate threshold for
source or edits. The gate workload was 3 versus 32 o200k tokens. The record update
was 55 versus 65. The shared replay binding excerpts were 190 versus 19
o200k tokens, separate from the authored programs and prompts.

The matched live pilot used the existing Codex sign-in with GPT-6.1 Sol at
low effort. Each arm had a fresh session, one attempt, and at most one
compiler-feedback repair. Rust passed 4/4 initially. FluxR passed 3/4
initially and 4/4 after the compiler identified the unreachable return
following `find`. The traces recorded zero tool calls. The pilot covers one
model and two workloads, eager rules and checked collection search, each with
generation and an anchored edit.

On o200k, initial prompt totals were 831 FluxR versus 638 Rust tokens. The
FluxR retry used a 285-token prompt, including the 44-token failed response
and 7 tokens of feedback. It returned 39 tokens. The four passing bodies
totaled 59 versus 146 tokens. Counting the failed response raises the
FluxR total to 103. The provider separately reported 64,681 versus 52,058
input tokens and 206 versus 162 output tokens across all attempts, with
46,080 versus 27,648 cached input tokens and 77 versus 0 reasoning output
tokens. These provider totals include the Codex harness and differ from
the measured prompt and body counts.

Reproduce the offline baseline with Python 3.11+ and the pinned tokenizer
libraries. The evidence directory should be outside the checkout. Reuse
the original seal for an evidence-bound rerun.

```sh
cargo build --locked --release --lib
python -B tools/bench_heldout_tokens.py seal --evidence /path/to/evidence
python -B tools/bench_heldout_tokens.py measure --evidence /path/to/evidence
```

Local `token-heldout/seal.json` SHA-256
`EDBF26608C1E543FC08E136B4B466710FEFA8E490576786430DA76E58E21769B`
binds the inputs and prompts. The offline and live score digests are
`ADCE442A4A4F4F19E5351391DA390E970C1F8C27F55B76BF637D8C77421E29D7` and
`C21ADF532BC1FDF7B3724EF17A66F81977F7360811C0A5E0AC34768BBCF20680`.
Keep this corpus as a regression baseline. The next token pass should
select reusable typed forms on the training workloads and validate them
on a new sealed set with equally capable Rust interfaces and separate
context costs. The broader FXM-04 gate remains open.

## Standalone repository checkpoint

The default branch at 706baa3 declares the MIT license. Its browser scene lab
and Rust examples run from a FluxR checkout, and telemetry snapshots stay local.
All maintained branches passed the Windows x64 Cargo tests with all features
and targets. The default branch browser harness verified native artifact
identity, nine replay assertions, live edits from both representations, and
rollback after a rejected edit.

## FXM-02.D.3 signed and record call costs

The Windows x64 typed JIT uses two four-lane scratch buffers when the prepared
input and result shapes fit. The emitter derives all input loads and result
stores from those same shapes. Wider JIT calls and the AOT ABI use 32-lane
buffers. Budget failures and typed JIT declines replay the portable evaluator
to preserve trap order.

The named harness adds a signed-leaf control, typed signed addition, the
same addition over a record field, record reuse, and record update. Each
checks 20,000 inputs against the portable evaluator and a Rust host adapter.
The trap matrix checks malformed tags and arithmetic overflow at and below
the instruction, cell, and frame frontiers. A release test exercises both
sides of the four-lane cutoff with three- and four-field records.

Rust 1.99.1 release with `typed-jit` and telemetry off ran on logical CPU 12.
Each binary ran eight times over two ABBA and two BAAB orders. The table reports
the medians of process p50 and p95 from 21 pass means, in ns per call.

| Workload | Rust host | Baseline bundled | Bundled | Source injection | Artifact injection |
| --- | ---: | ---: | ---: | ---: | ---: |
| Signed leaf | 5.56 / 5.66 | 18.63 / 18.87 | 18.75 / 19.04 | 18.85 / 19.37 | 18.88 / 19.38 |
| Typed signed | 5.75 / 5.96 | 19.41 / 19.91 | 17.63 / 17.91 | 17.37 / 17.75 | 17.33 / 17.64 |
| Record read | 5.79 / 6.24 | 28.39 / 29.09 | 25.45 / 26.18 | 25.54 / 26.38 | 25.43 / 26.14 |
| Record reuse | 7.16 / 7.50 | 43.16 / 44.03 | 38.16 / 38.91 | 36.80 / 37.67 | 38.11 / 38.76 |
| Record update | 37.78 / 39.05 | 60.56 / 61.67 | 58.09 / 59.42 | 58.06 / 60.48 | 57.96 / 59.13 |

Across the four ordered comparisons, bundled p50 fell 7.0–9.8% for typed
signed calls, 9.1–17.8% for record reads, 10.2–24.5% for reuse, and
2.5–10.2% for writes. The signed-leaf control moved by −0.1% to +2.2%.
A separate allocator probe counted zero allocations per warm read or reuse
and one per write in both FluxR and the Rust record control. Record packing,
native entry and result handling still share the timed interval. Windows
kernel counters require elevation, so the experiment measures the effect of
the buffer change on the full call and leaves cycle attribution open.

Each lifecycle phase used 31 samples per process. Candidate p50 for source
compilation was 2.70–7.15 µs and artifact decoding 3.80–10.60 µs. Native
preparation was 3.30 µs for the signed leaf and 55.70–78.55 µs for the typed
calls. Prepared activation was 0.20–0.80 µs and first calls 0.10–0.30 µs,
including timer quantization.

Candidate process results had 13 injection threshold misses across six
processes, compared with six misses across four baseline processes. The
new workloads missed at −6.35% source p50 for record reuse and −21.76%
artifact p95 for record reads, both in the first BAAB B2. The latter also
missed in the BAAB candidate aggregate at −12.24%. The baseline signed-leaf
source p50 and p95 spiked +69.52% and +72.17% in the first ABBA A2. The
256-value bundled scan p50 rose 9.4–9.7% in ABBA alongside 8.4–9.5% for
the Rust control. Its 64-call block p50 moved by +0.4% to +3.1% across
all orders. Counter bundled p95 rose 32.2% in one ABBA order alongside
28.3% for the Rust control and fell in the other three orders.

The candidate is retained for the repeatable small-call gains. The
six-workload bundled/Rust-host geometric mean remains 2.26–2.27× p50 and
2.03–2.22× p95. Rust-host and injection parity remain open. The Windows
release suites passed 57/57/59 tests, five Node Wasm suites passed, and
Linux x64 compiled. ARM64 still needs a target and runner.

Local `signed-record-boundary/manifest.json` SHA-256
`6A89BB094DB46BC0F91C5EC3DB7A9F562B93FE8DB983B772FCA887EA73D58BA5`
binds the exact binaries, PDBs, source patches, run scripts, logs, allocator
probe and each missed comparison. FXM-04.B can advance on the frozen
semantics while FXM-02.D, wider FXM-05 and FXM-06.A retain their open gates.

The development and standalone default branches integrate the typed runtime
and its prerequisites in separate commits. Ordinary structured compilation
uses compact source. The migration commands accept legacy JSON and FSM and
verify canonical bytes. The CLI accepts both `.fluxr` and `.flux` for timeline
and rule text.

Both integrated trees passed 57 portable, 57 native-JIT, and 59 typed-JIT
Windows release tests. Both passed the CPU 12 named benchmark checksums,
14 sealed source and edit pairs, five Node Wasm suites, highlighting,
and nine browser replay assertions with live edit and rollback. The timeline
and counter artifacts matched byte for byte. Linux x64 compiled. The
integration runs check behavior. The four ordered comparisons above supply
the performance evidence.

Local `default-sync/manifest.json` SHA-256
`1DEF27C82C6D27429337762BCCAF1952EFB4140AFFCE491AE82E224E28059044`
binds both tested trees, scripts, artifacts, and validation logs.
