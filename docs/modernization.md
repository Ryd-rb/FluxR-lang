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
| FXM-05 | Safety and language coverage. The bounded ledgers cover typed lists, variant payloads, shared host values, procedure calls, recursion, control flow and binary32 boundaries. FXM-05.B also demonstrates a universal tape simulator. | Extend the specification and proof checker beyond the bounded fragments. Complete the open feature inventory, backend safety checks and portable conformance matrix. |
| FXM-06 | Integrate accepted features on the combined baseline. Verify Windows x64, Linux x64, ARM64 and browser/portable paths, package compatibility, and isolated host launches. | FXM-01 through FXM-05. Integrate the volume branch with independent baseline evidence, then perform release-level replay and cleanup. |
| FXM-06.B | Reconcile local and remote branches. Land validated unique work on both defaults promptly and record already ported or superseded changes. | Accepted on both defaults. The consumer remains pinned. Reconcile again at each accepted checkpoint. |
| FXM-07 | Compare FluxR, optimized Rust and Rhai on matched performance, safety and token counts. Freeze common workloads, feature flags and safety settings. Separate warm invocation from compilation and reload, and count source, host bindings, edits and context separately. | Comparison accepted on both defaults. FXM-07.A screens typed-list traversal separately. |
| FXM-07.A | Screen compact typed-list traversal against equivalent expanded code on independent list workloads. Require identical artifacts, exact traps and fuel, and released reader replay. | Explicit-bound `scan` passed all six encoders and the compatibility checks. Length inference and runtime changes remain separate experiments. |

FXM-02.A established the first integrated runtime baseline. FXM-02.B delivered the Windows shared typed subset below. FXM-01.B freezes and measures the structured host corpus. FXM-05.A verifies typed list admission and the length-dependent scan effect. FXM-02.C can now test bulk admission and scan changes against this contract and corpus. The remaining program-wide gates stay open.

Evidence and semantic work can run in parallel once case IDs and the oracle interface are frozen. Source encoding work can use the same semantic version while the runtime explores backend choices. Native tier and ABI changes share a writer until they stabilize. Benchmarks use one local measurement lane.

FXM-02.D.4 retains a smaller packing path. FXM-04.C measures reusable
declarations and scoped edits against equal Rust controls. FXM-02.D.5 bounds
export resolution and admission costs and measures long-scan tail variation.
FXM-03.B.3 expands migration replay and cuts FXH guard preparation time.
FXM-05.C extends the independent effect specification to compound calls.
FXM-02.D.6 pairs long-scan wall time with thread-cycle counts.
FXM-02.D.7 reduces the cost of returning an unchanged host record.
FXM-05.E verifies bounded signed branches, backedges, and record ownership.
FXM-05.F verifies typed short-circuit effects and signed loop exits.
FXM-02.D.8 speeds up packing for a narrow signed-record argument shape.
FXM-05.G verifies nested loop exits and conditional helper calls.
FXM-03.B.4 adds opt-in checked execution for signed integer rule bitwise and.
FXM-03.B.5 extends that route to raw binary32 and mixed rules.
FXM-03.B.6 adds a second FR1 packet with ordered effects and list-boundary
replay against the released reader.
FXM-05.K checks admission of mixed host values with repeated references.
FXM-03.B.7 replays released FR1 semantic edits against checked ingress.
FXM-06.B reconciles three branches and adds the optional native host to both
defaults. FXM-02.D.9 rejects direct signed-pair packing after matched
measurements on both defaults. FXM-04.D accepts grouped bindings for
typed source after exact artifact and behavior checks.
Rust-host and injection parity, the whole-program token gate, wider safety,
native platform execution, and the isolated release launch remain open.

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

## FXM-04.B compact typed returns

Compact v1 and v2 now accept `= tail` instead of a local group and a
return statement when a function has no locals. A tail is an expression
or `(? test yes no)`, with two recursively defined tails. The same
conditional form works as a terminal statement in a compact edit. It
expands to ordinary typed branches and returns, so branch laziness,
instruction budgets, and portable bytes match the expanded source. The
existing printer emits the expanded form for migration and interchange.

We selected the spellings on the existing scalar and host decision training
examples before creating the validation cases. Their short spellings saved
two tokens for the return and four or five for the branch across the pinned
panel. Both training programs produced identical before and after artifacts.
A synthetic arithmetic chain also saved tokens, but we found no reuse in the
inspected training modules and left it out. This small manual selection follows
the principle of evaluating corpus-derived abstractions separately from
training, as in [Top-Down Synthesis for Library Learning](https://arxiv.org/abs/2211.16605).

The new seal covers six original and six edited programs against optimized
Rust 2024 using the standard library. All pairs passed on unsigned overflow,
lazy checked division, raw binary32 indexing, a record method, list results
and enum payloads, including trap cases. Each FluxR program also produced
identical portable bytes after expansion and recompilation. This checkpoint
adds parser forms with no new execution path or timing claim.

| Tokenizer | Full source median / aggregate / p95 | Edit median / aggregate / p95 | Aggregate vs expanded FluxR |
| --- | ---: | ---: | ---: |
| o200k_base | 80.5% / 78.2% / 89.2% | 100.0% / 105.6% / 200.0% | 85.1% |
| cl100k_base | 80.5% / 79.2% / 91.4% | 100.0% / 105.6% / 200.0% | 85.2% |
| Qwen2.5-Coder | 81.6% / 81.0% / 97.3% | 108.3% / 111.1% / 200.0% | 86.2% |
| StarCoder2 | 81.1% / 78.7% / 92.7% | 100.0% / 105.0% / 200.0% | 89.5% |
| Mistral-7B | 82.8% / 78.9% / 92.9% | 100.0% / 104.8% / 150.0% | 89.4% |
| OLMo 2 | 80.5% / 79.2% / 91.4% | 100.0% / 105.6% / 200.0% | 85.2% |

No tokenizer met the joint 10% gate. On o200k_base, full source was
222 FluxR versus 284 Rust tokens. Separately encoded declarations were
148/185 and bodies 87/106. These counts do not add to full source counts
because token boundaries change. Edit bodies were 19/18, anchors 17/16,
source prompts 1,021/361, and edit prompts 1,371/771. The shared replay
binding excerpts were 101/20; per-case Rust replay entries are counted
separately from the excerpts and the authored program. These prompt and
binding costs show why shorter local syntax alone cannot meet the gate.

A scripted repair introduced the same non-boolean condition in each
language, verified both compiler diagnostics, and restored the sealed
source. On o200k_base, failed bodies were 39/55, diagnostics 6/25,
replacement spans 2/2, and repair prompts 238/162. These are offline
costs for one known repair, with no live model attempts or pass-rate claim.

Run the new evaluator with the pinned tokenizer dependencies:

```sh
cargo build --locked --release --lib
python -B tools/bench_typed_tokens.py measure --evidence /path/to/typed-forms --training /path/to/typed-forms/training-freeze.json --cache /path/to/token-heldout
```

Use the same command with `seal` to make a new seal only for a new evidence
directory. The original FXM-04.A seal stays unchanged. To replay it against
a newer build, run the old harness and case file from `fe3fd2f` in development
or `88a8fbd` in standalone, and pass the newer build's directory via
`--target-dir`. The old seal includes the whole case file. For the FXM-04.B
seal, archive the harness, its inputs and the files listed in its training
freeze from `1d7216d` in development or `8e8403e` in standalone. Pass the
newer build's directory to that snapshot's evaluator.

The FXM-04.B training freeze SHA-256 is
`95E827180AAF6674FFE87AD40B588531144F308A8505CF85B18B36424AB9C091`.
The final validation seal is
`464DE04AFCDA82E93763CAA9DEA6BEE4D6B605352A9B273C6B3A2401C1AFD69E`.
The preflight seal was retained; only the diagnostic check's case handling
changed before counting. The offline score digest is
`C52B08E36EE18DF6A3E2F699EB4519745E14AFC66539C00D7A1F887FF8120681`.

Both repositories passed 58 portable, 58 native-JIT and 60 typed-JIT Windows
release tests, both sealed corpora, the pinned runtime behavior checks,
five Node Wasm suites, browser replay, editing and rollback, and TextMate
scope checks. Timeline and counter artifacts matched across repositories.
Linux x64 compiled; native Linux and ARM64 remain open. The local
`typed-forms/manifest.json` SHA-256
`489A878A00D2277C19F3ED4D1FC878CD0AB1911ACEDCAED2C84B5C02986DFE5B`
binds the tested inputs and evidence.

Retain the forms as a modest reduction with byte equivalence. The broader
token gate still needs a shared way to remove repeated declarations and
reduce grammar and anchor context. Future comparisons should give Rust the
same reuse opportunity and keep the two sealed corpora as regressions.

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

## FXM-02.D.4 host admission and native entry

A bounded diagnostic used the frozen Windows x64 corpus, logical CPU 12,
Rust 1.99.1 release with `typed-jit`, and telemetry off. Two diagnostic
processes rotated 256-call blocks for typed calls and 64-call blocks for
scans. All routes checked every result and an order-sensitive checksum.
The prepacked signed and record-read JIT entries both took 3.52 ns p50.
The packing and admission controls, which use a checked Rust addition in
place of the JIT entry, took 11.33 and 16.80 ns. Full named calls took
16.80–17.19 and 24.22–24.61 ns. An independent named lookup took
6.64–7.03 ns against a 1.56 ns known-index control. These timings overlap
in setup and result handling; they do not add up to the named call.

For 16 and 256 values, the portable p50 was 39.06 and 242.19 ns. The
prevalidated scan kernel took 12.50 and 151.56–153.12 ns. The native scan
with admission took 25.00 and 218.75–220.31 ns. The portable constant-return
control uses the same list type and includes the generic frame and result
work. These controls bound admission cost without attributing cycles to
individual instructions. WPR kernel counters still require elevation.

Direct lookup for single-export modules was reverted. Record-read p50
fell 6.6–6.8%, while record reuse slowed 5.5–14.7% and one order regressed
the skewed decision by 24.3%. The retained change packs host inputs and
arguments in separate slices. It keeps their contiguous lane order and
one-based origin index, and uses that index to reuse the original record.
It preserves the ABI and uses the same portable fallback on decline.

Each binary ran eight times across two ABBA and two BAAB orders. The table
shows the median of eight process statistics over 21 pass means, in ns
per call. All three entry routes use the same prepared tier.

| Workload | Route | Baseline p50 / p95 | Split packing p50 / p95 |
| --- | --- | ---: | ---: |
| Record read | Bundled | 25.41 / 26.17 | 24.06 / 24.96 |
| Record read | Source injection | 25.41 / 26.07 | 23.97 / 25.09 |
| Record read | Artifact injection | 25.37 / 26.09 | 24.08 / 24.92 |
| Record reuse | Bundled | 36.31 / 37.00 | 35.33 / 37.57 |
| Record write | Bundled | 58.04 / 59.30 | 57.35 / 58.20 |
| Counter | AOT host | 76.61 / 78.38 | 74.07 / 74.94 |

Record-read bundled p50 fell 4.4–5.1% in three orders and 17.1% in the
fourth. The latter order also moved the long-scan Rust-host block p95 by
−35.0%, alongside −38.6% for the bundled path and −34.1% for portable.
Record reuse was mixed. Counter AOT host p50 fell 2.4–8.1% in all four
orders. Lifecycle timings are separate: record-read native preparation from source
median p50 was 78.25 µs versus 79.55 µs before, with mixed per-order
changes. This pass makes no lifecycle speed claim.

The split-packing runs had 20 per-process injection threshold misses
versus 11 at baseline across the named and block routes. One aggregate
missed both signed-typed p50 injection comparisons by about −5.7%.
The other aggregate misses were scans, with up to +15.6% p95 on the
short artifact-injected scan. The six-workload bundled/Rust-host
geometric mean stayed at 2.28–2.33× p50 and 2.09–2.14× p95. Keep the
measured record-read and AOT host gains. Injection and Rust parity
remain open. Any future timing pass should retain the rotating block
and Rust-host controls and examine tail drift before changing the scan.

Both repositories passed 59 portable, 59 native-JIT and 61 typed-JIT
Windows release tests, the frozen host and budget checks, both sealed
token corpora, five Node Wasm suites, browser edit and rollback, and
TextMate scope checks. The timeline and counter artifacts match each
other and the previous checkpoint byte for byte. Linux x64 compiled;
native Linux and ARM64 execution remain open.

Local `runtime-parity-next/manifest.json` SHA-256
`3464AFAD2D10D875FB0E3D6C113220ECA79B916A9608C273070E46483B153F39`
binds the exact binaries, symbols, patches, run orders, checks, and all
threshold misses.

## FXM-04.C reusable declarations and edit context

The training examples were the structured host record read and write, each
with a checked subtraction edit. On both examples and all six tokenizers,
supplying the function body and exact edit span cost fewer prompt tokens
than resending the whole program. The selected protocol pins a record and
export signature. Generation supplies a body, and an edit supplies a
replacement for a unique span within that body. Both languages use the
same protocol. The replay harness binds the full draft and template by
digest and rejects a stale draft, a missing span, or a repeated span.

The fresh seal covers six original and six edited record programs, each
tested on 343 input triples against optimized Rust 2024. The probes check
values, arithmetic traps, and that each edit changes behavior on at least
one input. The compact printer and released decoder replay the same
portable bytes. The experiment uses released source forms and runs
composition and edits in the benchmark harness.

Each cell reports median / aggregate / nearest-rank p95 of FluxR divided
by Rust. Full source includes the template in each program. The body column
counts only newly authored bodies, with the shared definition charged
separately. The edit column counts replacement spans.

| Tokenizer | Full source | Body with reuse | Edit |
| --- | ---: | ---: | ---: |
| o200k_base | 72.9% / 73.0% / 77.6% | 83.3% / 81.2% / 95.2% | 112.5% / 166.7% / 800.0% |
| cl100k_base | 72.9% / 73.0% / 77.6% | 83.3% / 81.2% / 95.2% | 112.5% / 166.7% / 800.0% |
| Qwen2.5-Coder | 75.0% / 74.9% / 79.0% | 83.3% / 81.2% / 95.2% | 112.5% / 166.7% / 800.0% |
| StarCoder2 | 74.6% / 75.6% / 81.7% | 61.3% / 66.7% / 84.6% | 76.2% / 128.6% / 900.0% |
| Mistral-7B | 74.9% / 76.6% / 83.3% | 63.4% / 68.8% / 88.5% | 92.9% / 133.3% / 900.0% |
| OLMo 2 | 72.9% / 73.0% / 77.6% | 83.3% / 81.2% / 95.2% | 112.5% / 166.7% / 800.0% |

Zero tokenizers met the joint 10% gate. On o200k_base, shared definitions
cost 27 FluxR and 37 Rust tokens, and six authored bodies cost 82 and 101.
The shared replay binding excerpts cost 166 and 19, plus 274 Rust tokens
for the per-case entries. Edit anchors cost 25 and 16, and edit bodies cost
25 and 15. The comparison reversal accounts for the largest edit ratio:
the FluxR span swaps operands and the Rust span changes a single operator.

On o200k_base, the first template prompt costs 38 and 49 tokens. Across six
generation-and-edit tasks, counting it once brings mean context from
124.8 to 68.2 for FluxR and from 149.2 to 72.2 for Rust. This is logical
context under reuse, without provider framing, cache or conversation costs.
One scripted repair per arm restored the sealed conditional after the
compiler rejected an integer condition. The failed bodies cost 11 and 18
tokens, diagnostics 6 and 25, repair anchors 3 and 3, and replacements 7 and
6. The scoped repair context cost 32 and 57 versus 125 and 172 cold. The
experiment makes no live generation or runtime performance claim.

Reproduce with the pinned tokenizer libraries and cache:

```sh
cargo build --locked --release --lib
python -B tools/bench_context_tokens.py measure --evidence /path/to/context-reuse --training /path/to/context-reuse/training-freeze.json --cache /path/to/token-heldout
```

The training freeze SHA-256 is
`C2B01C0E9A74FA9B4E51640BA14416B13EBCE67B5C5E28AE2EFD246EE9F1DD3D`.
The validation seal is
`CFCFA9B0F24ADBA43DCB513C13571E93F0216BDCAC114358C2CFA67B2A6EF51D`.
The offline score digest is
`522DB0712972976B88F4391EA8C19B1BCD6EC6CA8CA68AB21ECD0C4CCB3D6DA0`.

Both repositories passed 59 portable, 59 native-JIT and 61 typed-JIT
Windows release tests, all three sealed corpora, pinned host and budget
checks, five Node Wasm suites, browser replay and rollback, and
highlighting. Timeline and counter bytes match the preceding checkpoint.
Linux x64 compiled. Local `context-reuse/manifest.json` SHA-256
`17B865C2FA8EB15931678F20D02AD56E15304BC6D6591C493735A37B30D126DC`
binds the training freeze, seal, scores, repair traces and compatibility
evidence.

## FXM-02.D.5 paired admission and scan diagnostics

The Windows x64 release benchmark reused the frozen D.4 cases and checksums
on logical CPU 12 with Rust 1.99.1, typed JIT and telemetry off. Each
comparison ran in ABBA and BAAB order. The diagnostic rotates all eight
routes over the same input blocks and retains each sample. The original
five-route scan comparison still runs with 64-call blocks.

Binding the export before the timed diagnostic call reduced p50 by
7.8–8.0 ns for signed typed calls, 5.7–6.3 ns for record reads, and
2.5–3.5 ns for record reuse across bundled, source and artifact routes.
The isolated lookup cost 4.3–6.3 ns above a known-index control. The
prepacked signed and record-read JIT entries took 3.3–3.5 ns at p50,
against 11.1–11.5 ns and 15.2–15.4 ns for their packing controls.
The controls overlap in result handling. The diagnostic itself added
roughly 6 ns to named signed calls compared with the production binary,
so the paired differences do not establish production gains.

For 256-element scans, the median source-minus-bundled and
artifact-minus-bundled block differences were zero in all four
diagnostic processes. Their paired p05 ranged from −57.8 to −39.1 ns
and p95 from +42.2 to +59.4 ns. Removing each input block's median gave
Pearson correlations of −0.09 to +0.20 between the bundled residuals
and the Rust-host, portable and injection residuals. The 16-element
differences had p05 and p95 magnitudes of at most 4.7 ns. This confirms
substantial long-scan variation and gives no consistent source or artifact
penalty. It does not attribute individual outliers to scheduling or the
scan implementation. Kernel counters require elevation.

Moving arity checks into the selected tier passed the targeted tests.
It improved bundled signed typed p50 by 1.6–2.5% but slowed bundled
record reuse by 3.4–4.2% in both orders. A one-field packing shortcut
improved bundled record-read p50 by 1.2–2.0%, with mixed results in
the source route. Bundled signed typed p50 regressed by 6.5% in one
order and bundled record write slowed by 1.1–3.6%. Both changes were
reverted. The unmodified baseline's six-workload bundled/Rust-host
geometric mean was 2.23–2.31× at p50 and 2.09–2.11× at p95 in these
trials. Rust-host and injection parity remain open.

Both independent repositories passed 59 portable, 59 native-JIT and
61 typed-JIT Windows release tests, all three token seals, pinned host
and budget checks, five Node Wasm suites, browser edit and rollback,
and highlighting. Timeline and counter artifacts match prior bytes.
Linux x64 compiled. Native Linux and ARM64 execution remain open.

Local `runtime-admission/manifest.json` SHA-256
`98C35A93BEC37FC7391B90764DED2CE2CB2F8F851065484EBF60CD4361C06B69` binds the diagnostic, reverted patches, binaries,
symbols, per-order misses, raw scan blocks, and compatibility evidence.
FXM-03.B.3 uses the restored runtime to expand historical replay and
measure FXH preparation separately from execution.

## FXM-03.B.3 historical round trips and FXH preparation

Rule FXH v1 still checks FXD, rebuilds and compiles the shared projection,
and compares the complete canonical envelope. It now prints the projection
from the checked compiler input and compares the reparsed compact AST
directly. The checked HIR retains that compact text for encoding. This
removes a JSON decode and reserialization from projection encoding without
changing the rule evaluator, the general structured migration path, or
any released wire format.

The frozen expansion compares 18 inputs and 843 probes with the archived
`bd4dbca` engine. It covers eager select, both sides of a guard exit, signed
and stack limits, raw binary32, structured budget and type errors, and
FR1's nominal interface. New writes reproduce the old FXH v1 bytes and
receipts. Fifteen malformed envelopes, origins and projections reproduce
the old refusal diagnostics and leave no output. The previous FXH replay
also passes its 19 inputs, 82 probes and ten refusals on both repositories.

On Windows x64 logical CPU 12, Rust 1.99.1 portable release with telemetry
off, `examples/bench_hir.rs` compares the same sealed files in ABBA and
BAAB order. Each process measures 11 rounds, 10,000 operations per round
(200 for the 255-operation guard). Each p50/p95 pair is the median of
the two process percentiles for that binary and order. Times are µs per
decode and restore; execution was measured separately.

| Input | ABBA old → new p50 / p95 | BAAB old → new p50 / p95 |
| --- | ---: | ---: |
| Guard | 13.04 / 13.48 → 6.60 / 7.50 | 13.62 / 16.32 → 6.32 / 6.38 |
| 255-operation guard | 556.50 / 604.45 → 169.60 / 251.47 | 540.73 / 692.11 → 190.16 / 274.80 |
| Raw binary32 | 0.56 / 0.70 → 0.73 / 0.76 | 0.57 / 0.70 → 0.51 / 0.51 |
| FR1 | 18.10 / 19.31 → 17.93 / 20.31 | 18.64 / 21.25 → 17.52 / 20.73 |

Encoding an already checked guard took 0.32–0.40 µs at p50, down from
9.24–9.69 µs. The short guard's restored execution was about 10 ns on both
binaries. On the long guard, legacy and restored execution tracked each
other within each binary. Raw binary32 and FR1 timings varied across orders.
The measurements establish a guard preparation gain on this host.

Both independent repositories passed 60 portable, 60 native-JIT and
62 typed-JIT Windows release tests, the three token seals, pinned host and
budget checks, the five Node Wasm suites and three released FXH Wasm
fixtures, browser replay and highlighting. The timeline and counter match
prior bytes. Linux x64 was compile-checked. Local
`migration-roundtrip/manifest.json` SHA-256 `AD27EA89C9B28E589945319B998EA2A6E05788D2666E41BC5B3677307A7AF70C`
binds the frozen inputs, source patch, binaries, timings and verification.

## FXM-05.C compound values and procedure effects

The independent checker passes a record containing a variant and another
record through a three-frame call chain. The inner procedure updates two
fields and the result wraps the updated record with a host-supplied variant.
The ledger follows source evaluation order. It charges each visited boundary
cell before checking its tag and each call's dispatch before the callee can
enter. It checks frame depth before charging for the frame and its slots.
The first field write charges two cells to copy the shared record. The second
writes the private copy. With populated variants in the state and snapshot,
a successful call needs 24 instruction units, 36 cells and three frames.
An empty snapshot variant saves six cells across input and result visits.

The checker compares 24,253 invocations per route across direct, forced
portable, decoded artifact and injected execution. Its grids cover malformed
counts, tags, field shapes and payloads, both directions of signed addition
overflow, zero division and signed division overflow. A reused workspace
also runs successfully after failures. Four failed replacement probes
preserve the revision, encoded active module and host-owned values. A
successful probe then publishes the edited procedure. Both Windows native
tiers decline this shape and use portable execution. The checkpoint adds
conformance evidence and leaves runtime and artifact encoding code unchanged.

In both repositories, Windows x64 release passed 61 portable, 61 native-JIT
and 63 typed-JIT tests. Node Wasm passed all six suites and three released
FXH fixtures. The pinned host checks, browser replay and rollback,
highlighting, all three token seals, and historical FXH replays also passed.
The context-reuse seal used the archived training files and the current
runtime. The prior timeline and counter artifacts match byte for byte.
Linux x64 was compile-checked.

Run `cargo test --locked --release --all-features --example bench_typed
compound_effects_match_specification`, or call `compound_effect_check()` in
the Node Wasm build of that example. Local `compound-effects/manifest.json`
SHA-256 `11415DA9BEFE89131B9EC43FF82C1F98B53FBCEF75CA286B58C28A8A0F0E22CB`
binds the frozen checker, exact routes, replays, builds and seal recovery.

## FXM-02.D.6 thread cycles in paired scan tails

The D.5 diagnostic ran on the frozen Windows x64 corpus with Rust 1.99.1
release, typed JIT, telemetry off, and one timing lane pinned to logical CPU
12. Both repositories ran ABBA and BAAB with the same diagnostic executable
per repository and the thread-cycle probe off or on. Each scan length used
672 timed blocks of 64 calls per route after an untimed pass that checked
every input. The eight routes kept the same input blocks, rotation and
checksums. The probe samples the current thread's cycle count around the
wall-timed loop. Its sleep control records far fewer cycles than its busy
control. Counts include user and kernel execution on the measured thread.
[Windows thread cycle counter](https://learn.microsoft.com/en-us/windows/win32/api/realtimeapiset/nf-realtimeapiset-querythreadcycletime)

After subtracting each route's median for each input block, 32–34 of the 34
largest 256-element wall times were also among the 34 highest cycle counts
for each route in every probed process. The Rust host, forced portable,
bundled, source and artifact routes all show this overlap. The probed
long-scan bundled p50 differed from the unprobed diagnostic by −0.7% to
+2.4% across the four matched order summaries. Its p95 differed by −7.3%
to +8.8%. The overlap shows that most large wall deviations also carry
higher accounted cycle counts. Kernel scheduling and instruction-level
attribution still require a trace.

The clean release binaries ran separately in both orders. Across the four
two-process summaries, bundled execution over the six matched workloads
took 2.24–2.35× Rust host time at p50 and 2.03–2.21× at p95 by geometric mean.
Long-scan 64-call source and artifact p95 deltas ranged from −1.4% to +2.1%
against bundled execution. The full-pass short-scan source p95 missed at
−11.1% in one summary, and the long-scan artifact p50 missed at +19.1% in
another. Across the named, matched and block comparisons, seven summarized
comparisons and 24 individual process comparisons exceeded the injection
thresholds. Both numeric parity gates remain open. This measurement
checkpoint retains the two D.5 reverts.

Both independent repositories passed 61 portable, 61 native-JIT and 63
typed-JIT Windows release tests, including exact scan budgets at both
lengths and the compound effect oracle. Pinned host checks, six Node Wasm
suites, three released FXH fixtures, browser edit and rollback, highlighting,
all three token seals, and historical replays at 18/843/15 and 19/82/10
passed. Timeline and counter artifacts match prior bytes. Linux x64 compiled.
Local `runtime-parity-d6/manifest.json` SHA-256
`543BD0B3C947CEA8AEF7F295D545AEC75EFB5F04FF7816BF6401A1D235E8962F`
binds raw blocks, binaries and symbols, diagnostic patches, clean release
timings and compatibility logs. FXM-05.D can advance recursive effect coverage
on this baseline.

## FXM-05.D bounded recursive effects

The independent source ledger follows a signed fold that returns its seed for
nonpositive counts. Each positive frame retains its count across a recursive
call and combines it with the returned value using checked addition. For a
nonnegative count `n`, success requires `12n + 6` instruction units,
`3n + 6` cells, and `n + 1` frames when arithmetic succeeds. Each argument
consumes a boundary cell before its tag check. Dispatch consumes fuel
before frame admission, which checks depth before charging three cells.
On unwind, arithmetic can trap before the frame returns. The result cell
is charged after the outer return.

The grid compares 27,014 cases per route across direct, forced portable,
decoded artifact, and injected calls. It includes negative and zero counts,
overflows at two unwind depths, malformed arguments, intersecting budget
limits, and a 1,025-frame call with exact and one-short limits. A failed
probe on each of the three budget dimensions and on checked subtraction
preserves the revision, active bytes, and prior result. An exact probe then
publishes the subtracting fold. Both Win64 native configurations decline
the recursive export and execute through the portable fallback.

Both repositories passed Windows x64 release with 62 portable, 62 native-JIT,
and 64 typed-JIT tests. Seven Node Wasm suites, three released FXH fixtures,
pinned host checks and exact budgets, browser edit and rollback, highlighting,
all three token seals, and historical replays at 18/843/15 and 19/82/10 passed.
Timeline and counter artifacts match prior bytes. Linux x64 compiled.
Runtime and wire code were unchanged in this conformance slice.

Run `cargo test --locked --release --all-features --example bench_typed
recursive_effects_match_specification` or call `recursive_effect_check()`
in the Node Wasm build of that example. Local `recursive-effects/manifest.json`
SHA-256 `9892A6AE81461E517387E1FE877FCD6CC15BC60682ABEB417B28714C6ADCD56A` binds source, checks and compatibility logs.
FXM-02.D.7 next isolates result handling in the matched signed-record host
routes. Both numeric parity gates remain open.

## FXM-02.D.7 signed record result ownership

The Windows x64 typed-JIT diagnostic used Rust 1.99.1 release, telemetry
off, logical CPU 12, and the frozen 20,000-input record-write and
record-reuse cases. Each process checked complete values and allocation
identity against the Rust host and prepared JIT. It then rotated routes
over 21 passes of matched 256-call blocks, including the final 32-call
block in each pass. Both repositories ran in ABBA and BAAB order.

| Isolated paired p50 difference | Record reuse | Record write |
| --- | ---: | ---: |
| Limit and count validation above known budgets | 1.56 ns | 1.56 ns |
| Tag checks and packing above prepacked input | 12.89–14.84 ns | 12.89–14.45 ns |
| Native entry above reading the frozen output | 7.03–7.42 ns | 7.03 ns |
| Result above the prepacked native entry | 16.02–19.14 ns | 30.86–32.42 ns |
| Rust result above reading the frozen output | 3.12 ns | 25.78–31.25 ns |

Allocating and releasing one uninitialized record slot cost 23.44–25.39 ns
over the raw output control. The Rust write materializer includes initializing
the field, consuming the result and dropping it. Reuse adds and releases an
owner on the existing allocation. The control differences include each
route's observation and dispatch costs. They overlap and cannot be added to
predict a complete call.

The shared JIT and AOT boundary now clones the validated record's Arc
directly when the native result carries its original input or argument index.
A write still constructs a new owned record. A combined candidate also
specialized one-field writes. It improved bundled record-write p50 by
1.2–7.9% but increased the bundled signed-scalar p50 by 7.5–8.3% in three
of four order summaries. The one-field shortcut was reverted. With only the
reuse change, bundled signed-scalar p50 changed by −1.4% to +1.7%.

In uninstrumented release binaries, bundled reuse fell from 35.39–36.91 ns
to 30.55–31.55 ns at p50 and improved 8.0–15.0% at p95. Source and artifact
reuse p50 improved 15.4–17.1% and 13.4–16.0%, respectively. Their p95 was
6.2–7.5% slower than baseline in the development ABBA order and improved in
the other three summaries. The matched Rust reuse p50 was 7.25–7.57 ns.
Across the six frozen hot workloads, bundled/Rust-host geometric means
were 2.21–2.27× at p50 and 2.05–2.12× at p95. Eight summarized injection
comparisons and 23 individual comparisons missed the injection thresholds.
Both parity gates remain open. Long-scan variation was visible on both
FluxR and the Rust host. The D.5 reverts and D.6 attribution limits stand.

Both repositories passed Windows x64 release with 62 portable, 62
native-JIT and 64 typed-JIT tests, including exact budgets and the
record-ownership checks. Seven Node Wasm suites, three released FXH fixtures,
the pinned host corpus, browser edit and rollback, highlighting, all three
token seals, and historical replays at 18/843/15 and 19/82/10 passed. Prior
timeline and counter artifacts match byte for byte. Linux x64 compiled.

Local `record-return/manifest.json` SHA-256 `C1447C892FC815CD7CFBEF985863E9757BACC3BF0AD750BE8AD1963F2B29E72B` binds the diagnostics,
both production variants, their binaries and symbols, and the validation logs.
FXM-05.E next checks exact effects for bounded native branches and backedges
against an independent source ledger and the portable routes.

## FXM-05.E bounded signed control flow

The independent source ledger checks a signed countdown with a record return.
A nonpositive count reuses the input allocation with 6 instruction units,
13 cells, and one frame. For a positive count `n`, a successful call needs
`21 + 13(n - 1)` instruction units, 18 cells, and two frames. Each backedge
charges the loop test, checked addition, countdown update, and jump. The final
division runs in a helper frame before the record detaches. The marker field
and the host record retain their original values.

The frozen grid checks 41,110 invocations per route through prepared Win64
typed JIT, forced portable, decoded artifact, injected module, and generated
AOT. Node Wasm runs the same grid with portable and generated AOT execution.
Cases include intersecting budget limits, malformed host values, an untaken
division by zero, signed division and addition overflow, and 1,024 backedges.
Direct kernel checks confirm that JIT and AOT complete both return paths at
exact instruction and cell budgets with two frames. Native admission reserves
the export's maximum frame depth. The one-frame early return succeeds through
the portable path. Five rejected replacement probes preserve the prior
revision, artifact bytes, record ownership, and result. An exact probe
publishes the subtracting replacement.

Both repositories passed Windows x64 release with 64 portable, 64 native-JIT,
and 66 typed-JIT tests. The pinned host corpus, eight Node Wasm suites, three
released FXH fixtures, browser rollback, highlighting, all three token seals,
historical replays at 18/843/15 and 19/82/10, and prior timeline and counter
artifact bytes passed. Both CLIs reproduced the generated AOT fixture.
Linux x64 compiled. The existing counter and volume workloads serve as
differential controls for this conformance slice.

Run `cargo test --locked --release --all-features control_flow` or call
`control_flow_check()` from the Node Wasm build of `bench_typed`.
Local `control-flow/manifest.json` SHA-256 `FCD22B2D2A94F69D611CF05E7A6C57E831B2EBE7BF3F12ADA30ACA7BBBB7255B` binds the frozen
ledger, generated fixture, and compatibility logs. FXM-05.F next checks
short-circuit boolean effects and explicit loop exits. Both runtime parity
gates remain open.

## FXM-05.F short-circuit effects and signed loop exits

Two source ledgers check the first observable result or trap. The boolean
fixture evaluates the right side of AND only when enabled and the right
side of OR only when the left side is false. The grid distinguishes a
skipped zero divisor, a taken zero divisor, signed division overflow,
and overflow in the subsequent record write. When the left helper returns
true, the record write completes in 24 instruction units, 19 cells, and
two frames. The native planner refuses this export because the boolean
result crosses a block edge. Forced portable, decoded, injected, and
direct fallback calls check 24,492 cases per route.

The loop fixture charges a forward jump for a taken break and skips the
next guard. A taken continue charges a backedge and skips the body
arithmetic. The signed countdown can also complete through its ordinary
backedge. Prepared Win64 JIT and generated AOT both finish the no-entry,
immediate break, single continue, mixed, and 1,024-continue cases without
portable fallback at their exact limits. The mixed path takes 88
instruction units and 19 cells. The 1,024-continue path takes 17,422
instruction units and 17 cells. Both need one frame. The loop grid checks
34,596 cases per route across direct, forced portable, decoded, injected,
and generated AOT calls. Node Wasm replays both grids through its
available routes.

Returning an unchanged record preserves its allocation. A changed result
detaches and keeps the marker field. Eight failed replacement probes
preserve the active revision, bytes, result, and host values. An exact
probe publishes the replacement for each export. The initial loop trap
probe exhausted its fuel before the division. The amended freeze reaches
the divide-by-zero at 92 instruction units and retains the original.

Both repositories passed Windows x64 release with 66 portable, 66
native-JIT, and 68 typed-JIT tests. The pinned host corpus, nine Node Wasm
suites, three released FXH fixtures, browser rollback, highlighting, all
three token seals, historical replays at 18/843/15 and 19/82/10, and prior
timeline and counter artifact bytes passed. Linux x64 compiled. The
existing signed control-flow, counter, and volume checks serve as
differential controls.

Run `cargo test --locked --release --all-features loop_exits` or call
`loop_exits_check()` from the Node Wasm build of `bench_typed`. Local
`loop-exits/manifest.json` SHA-256 `7C50DA4A5ADBFFDD2D2904A07CAC1E9E1EA9D12B8561A2117A18D29971D0DF33` binds both ledgers, the
generated fixture, and the compatibility logs. FXM-02.D.8 next isolates
signed-record validation and packing against the matched production
host controls. Both runtime parity gates remain open.

## FXM-02.D.8 signed record packing

The shared JIT and AOT boundary preselects direct packing for an export
with no host inputs and two arguments, a one-field signed record and a
signed integer. The fast path checks the record and scalar tags and the
exact field count, then writes the origin, field and scalar into three
lanes. Other shapes use the general packer. The focused admission test
checks lane values, unused lanes, malformed arity and values, one-unit
budget shortfalls and the identity of a returned host allocation.

Both independent defaults ran the frozen 20,000-input cases with Rust
1.99.1 release, typed JIT, telemetry off and logical CPU 12. Fresh D.7
and candidate binaries ran in ABBA and BAAB process orders. Each process
checked complete values and exact budget behavior against the portable
and Rust hosts. The table shows the range of bundled per-call medians
across the four repository/order groups.

| Workload | D.7 bundled p50 | D.8 bundled p50 | Matched D.8 Rust host p50 |
| --- | ---: | ---: | ---: |
| Record read | 23.80–24.61 ns | 19.72–21.04 ns | 5.67–5.81 ns |
| Record reuse | 30.52–31.87 ns | 25.70–26.67 ns | 7.29–7.42 ns |
| Record write | 58.25–60.70 ns | 53.64–54.94 ns | 38.65–39.31 ns |

Across record reads, reuse and writes on the bundled, source-injected
and artifact-injected routes, the geometric mean of paired p50 ratios
improved 13.0–16.7%. Each grouped record p95 improved 3.1–31.5%.
The signed-scalar controls changed −2.3% to +3.6% at p50 and −4.9% to
+3.1% at p95, within the frozen guards. These timings cover warm calls.
The logs report compilation, preparation and replacement separately.

Across the six frozen hot workloads, bundled calls took 2.22–2.29×
as long as the matched Rust host at p50 and 2.01–2.15× at p95 by
geometric mean. Eight grouped and 23 individual injection comparisons
missed the thresholds. Both runtime parity gates remain open.

Both defaults passed Windows release with 67 portable, 67 native-JIT
and 69 typed-JIT tests, the pinned host corpus, nine Node Wasm suites,
three released FXH fixtures, browser rollback, highlighting, all three
token seals, and historical replays at 18/843/15 and 19/82/10. Timeline,
counter and both generated AOT fixtures matched byte for byte. Linux
x64 compiled. Native Linux and ARM64 replay remain open.

Local `runtime-parity-d8/manifest.json` SHA-256 `2F6823F3A86BA17DBE8ED2B7A9A46A95E91EC9CB27B0B911E4DBBDB3E50CA6B0` binds the
freeze, candidate, binaries, individual and grouped timing data, and
compatibility logs. FXM-05.G next checks nested loop exits and conditional
helper calls with an independent source ledger.

## FXM-05.G nested signed loop exits and conditional calls

The source ledger distinguishes both loop exits at each nesting level.
Inner break still runs the outer update. Inner continue skips a matching
call trigger and reaches the next inner guard. Outer continue skips both
the inner loop and the outer update. Outer break skips the next guard.
The ledger also distinguishes an untaken zero divisor, a reached zero
divisor, signed division overflow, and overflow in the helper's caller
and the outer update. Budget sweeps check which trap occurs first.

Across direct, forced portable, decoded, artifact-injected, and generated
AOT invocation, each available route replayed 52,511 cases with exact
instruction, cell and frame limits. The mixed path with a reached helper
needs 110 instructions, 25 cells and two frames. The cancellation path
needs 108 instructions and 23 cells and returns the original allocation.
A changed result detaches the record and retains its marker. Four
rejected probes preserve the active revision, artifact bytes and host
values. A successful exact probe activates the changed result.

Win64 typed JIT and generated AOT complete six paths directly with two
frames, including a long path with 1,022 helper calls. The planner accepts
the export. Native admission declines a one-frame call because it reserves
the call graph's maximum depth. Portable execution can complete with one
frame on a path that skips the helper. Node Wasm exercises the portable
routes and compiled Rust AOT fixture.

Both defaults passed Windows release 69/69/71, the pinned host corpus,
ten Node Wasm suites, three released FXH fixtures, browser rollback,
highlighting, all three token seals, and historical replays at 18/843/15
and 19/82/10. Timeline and counter bytes and three generated AOT fixtures
matched. Linux x64 compiled.

Run `cargo test --locked --release --all-features nested_loop` to replay
the focused tests. Local `nested-loop-calls/manifest.json` SHA-256
`87C98E8512921D7EB5D6F4863672AFD8257BF5EED109644D9240DF855343FAA9` binds the freeze, fixtures and compatibility logs. FXM-06.B
reconciles outstanding branches before the next runtime pass. Both
runtime parity gates remain open.

## FXM-06.B synchronized branch integration

`codex/fluxr-modernization` at `6dbbbe4` reached the typed JIT, migration,
benchmark, and token work already ported through `fe3fd2f` and `88a8fbd`.
The signed arithmetic, division, and release cleanup from
`codex/fr1-arithmetic` at `2ab4463` were already present. The defaults carry subsequent compiler, documentation,
and telemetry work.

The snapshot of `codex/native-window-audio` at `1550756` adds an optional
native application host, authenticated TCP, and the structured CLI. Both
defaults gained equivalent code with their own histories. The compact
checker now accepts 512 functions and 512 record and enum declarations
combined. The FSM reader accepts canonical payloads up to 1 MiB. The checked VM, compact printer, and
highlighting support signed binary32 conversions and square root. Released
timeline, rule, FSM, and FXH readers remain available. The Blueward
consumer retains its clean checkout at the original native branch tip.

Both checkouts passed 72 portable, 72 native-JIT, 74 typed-JIT, and 79
all-feature Windows release tests, the pinned host corpus, 11 Node Wasm
suites, three released FXH fixtures, browser rollback, highlighting, and
all three token seals. Historical replay matched 18/843/15 and 19/82/10.
Timeline, counter, and three generated AOT outputs matched the released
fixtures. Each isolated Win64 app exercised frame input, audio host
initialization, PNG capture, timing, and graceful close. Each service
exchange passed against an independent authenticated loopback peer. Linux
x64 passed all-feature cross-compilation. Native Linux and ARM64 execution
remain open.

The signed projection fixture used 29 authored FluxR tokens and 33 Rust
tokens with `o200k_base`. Its Rust host setup and invocation used 103
tokens, measured separately. Win64 release calls through a reused portable
workspace took 99.33 and 103.01 ns/call at the median across nine batches
on the two checkouts. The equivalent Rust control took 1.27 and 1.30
ns/call. These timings exclude compilation and preparation.

Local `branch-integration/manifest.json` SHA-256
`474631507479E7BEA503EFAD30624862F39AF4F998F59BCBBCD1699152D40ED0` binds the paired source, branch audit, binaries, and compatibility
logs. FXM-02.D.9 measures the next independent residual. Both warm runtime
parity gates remain open.

## FXM-02.D.9 signed-pair packing experiment

The frozen Win64 corpus measured a residual in typed signed addition.
The bundled and injected routes took about 17 ns per call at p50, and
the matched optimized Rust host took 5.57–5.93 ns. The candidate
preselected direct lane packing for two signed arguments and no host
inputs. It used the same JIT and AOT kernel ABI and kept the existing
budget and tag checks.

Both independent defaults ran baseline and candidate builds in ABBA and
BAAB process orders on logical CPU 12, with Rust 1.99.1 release and
telemetry off. Each process verified the frozen checksums, exact budget
frontiers and portable trap order on bundled, source-injected and
artifact-injected calls. Focused checks passed for lane contents and
malformed values, and confirmed that unchanged records retain the host
allocation.

| Typed signed route | Baseline p50 | Candidate p50 |
| --- | ---: | ---: |
| Bundled | 17.04–17.15 ns | 16.11–17.08 ns |
| Source-injected | 16.88–17.04 ns | 16.94–17.31 ns |
| Artifact-injected | 16.94–17.16 ns | 16.79–16.98 ns |

Across the three routes, grouped p50 improved 0.3–2.0%, below the frozen
5% minimum in all four groups. Record controls exceeded the frozen
regression guards, with the largest observed p50 and p95 increases at
12.0% and 62.3% on different routes. The candidate was rejected and the
original source bytes were restored in both checkouts.

The retained baseline's source compilation p50 was 5.95–6.30 µs and
artifact decode was 9.05–9.35 µs. Native preparation was 74.70–79.20 µs
across source and artifact. Activation was 0.60–0.70 µs and first
execution was 0.10 µs. The seal retains both builds' p50 and p95 results
for each phase separately from warm calls.

The retained runtime measured 2.21–2.25× the matched Rust host at p50
and 2.01–2.09× at p95 by geometric mean across the six frozen workloads.
Five grouped and 18 individual injection comparisons missed the warm
parity thresholds. Both warm parity gates remain open.

All three token seals replayed against both restored runtimes using the
archived inputs and cached tokenizers. The FXM-06.B compatibility seal
applies to the identical runtime trees. Local
`runtime-parity-d9/manifest.json` SHA-256
`57757C2859DF8A913E2151596AA4B4E6539A456E27DB2AA0A6DFDE48982B96CA` binds the freeze, binaries, behavior checks, timing data and
token replay.

FXM-04.D next tests one source-density change using archived training inputs
and a frozen behavior-matched holdout. Live generation and edit evidence
remain gated by model budget.

## FXM-04.D grouped typed bindings

The archived scalar and decision examples repeat a type across adjacent
names. Both compact source versions now accept `x y:i64` in inputs,
record fields, parameters and locals. The parser expands the names in
order and charges the full expanded spelling to the existing byte budget
before copying types. Canonical printing retains explicit annotations.

The freeze fixes six typed-return and six record-context tasks, their
edits and optimized Rust controls, and the six cached tokenizers. Six
tasks share the record declaration. Both independent defaults compiled
each original and edited program in both spellings to identical bytes
and replayed them against the Rust controls. A focused test checks all
four binding positions in v1 and v2, missing types, duplicate names,
and expanded source limits.

All six tokenizers met the frozen local guard of 3% reduction in median
and aggregate authored tokens on originals and edits, with zero individual
regressions. Original source fell 4.9–7.1% at median and 4.4–6.6% in
aggregate. The p95 ratio to explicit annotations was 100% on every
tokenizer. The edit replacements were identical. None of the six met
the project-wide 10%-of-Rust gate.

Each cell gives median / aggregate / nearest-rank p95 as a percentage of
the matched Rust count. Authored source includes the whole declaration and
body on each task. Bindings count the logical replay adapter and the Rust
entry expression per task. Cold context includes the language reference,
task instructions and, for edits, the current source and unique anchor.

| Tokenizer | Explicit source | Grouped source | Grouped edited source | Edit body |
| --- | ---: | ---: | ---: | ---: |
| o200k_base | 72.9 / 75.4 / 89.2 | 68.8 / 72.1 / 89.2 | 68.8 / 72.3 / 89.2 | 100.0 / 133.3 / 800.0 |
| cl100k_base | 72.9 / 75.9 / 91.4 | 69.0 / 72.5 / 91.4 | 69.0 / 72.8 / 89.2 | 100.0 / 133.3 / 800.0 |
| Qwen2.5-Coder | 75.0 / 77.7 / 97.3 | 69.6 / 73.2 / 97.3 | 69.9 / 73.4 / 94.9 | 108.3 / 136.4 / 800.0 |
| StarCoder2 | 74.6 / 77.0 / 92.7 | 68.6 / 71.9 / 92.7 | 69.0 / 72.1 / 90.7 | 100.0 / 117.1 / 900.0 |
| Mistral-7B | 76.9 / 77.7 / 92.9 | 69.2 / 72.8 / 92.9 | 69.9 / 72.9 / 90.9 | 100.0 / 119.0 / 900.0 |
| OLMo 2 | 72.9 / 75.9 / 91.4 | 69.0 / 72.5 / 91.4 | 69.0 / 72.8 / 89.2 | 100.0 / 133.3 / 800.0 |

| Tokenizer | Host bindings | Cold source context | Cold edit context |
| --- | ---: | ---: | ---: |
| o200k_base | 395.2 / 364.9 / 420.8 | 345.1 / 337.0 / 404.9 | 194.6 / 189.5 / 213.0 |
| cl100k_base | 404.9 / 380.5 / 459.1 | 351.0 / 342.7 / 412.2 | 197.9 / 192.3 / 216.7 |
| Qwen2.5-Coder | 426.8 / 393.3 / 459.1 | 339.3 / 331.3 / 397.7 | 192.4 / 187.7 / 214.2 |
| StarCoder2 | 390.0 / 355.1 / 400.0 | 334.2 / 328.3 / 391.5 | 182.2 / 179.9 / 208.8 |
| Mistral-7B | 430.7 / 391.4 / 451.0 | 348.4 / 337.8 / 392.2 | 187.2 / 185.2 / 217.3 |
| OLMo 2 | 404.9 / 380.5 / 459.1 | 351.0 / 342.7 / 412.2 | 197.9 / 192.3 / 216.7 |

Relative to explicit FluxR, the added syntax instruction increased
aggregate cold source context 9.0–9.9% and edit context 5.4–6.0%.
These are offline logical prompt counts. Live model success, retries
and provider framing still require a matched generation and edit run.

Both defaults passed 73 / 73 / 75 / 80 Win64 release tests across portable,
native JIT, typed JIT and all-feature profiles. Historical artifacts, the
host corpus, Node Wasm, browser rollback, highlighting, and the three
archived token seals replayed. Linux x64 was compile-checked.

Local `token-density-d/manifest.json` SHA-256
`E4C33196E281A4A5E5947BF5A0234167CC29B762E2F315A1E74A6B5F1609A57B` binds the freeze, counts, artifacts and compatibility results.
Run `token-density-d/replay-frozen.py` with the archived Python
dependencies on `PYTHONPATH` to recompute the counts.

## FXM-05.H binary32 conversion and square-root boundary

The fixture exercises the released `i64tof`, `ftoi64`, and `sqrt` operators
individually and in a composed projection. Sixty fixed numerical cases
specify result bits or exact signed integers and use optimized Rust controls
with dynamic inputs. The inputs include integer ties, a value just above a
large midpoint that exposes double rounding, and both signed saturation
edges. Square root cases cover signed zero, subnormals, infinities, and
quiet and signaling NaNs. A negative nonzero input or any NaN yields
`0x7fc00000` for square root. Conversion to `i64` truncates toward zero,
saturates at the signed bounds, and yields zero for NaN.

Each unary export needs three instructions. The composed export needs five.
All four exports need four cells and one frame. The checker cuts each limit
by one, tests competing instruction, cell and admission errors, and checks a
successful call in the same workspace after the traps. It runs 372 cases per
route through prepared direct fallback, forced portable execution, decoded
artifacts, and a decoded candidate injected through `ModuleSlot`. The Windows
JITs and generated Rust AOT decline all four exports. The Node Wasm route
exercises the portable engine.

Both defaults passed 74 portable, 74 native-JIT, 76 typed-JIT, and 81
all-feature Windows release tests. The host corpus, 12 Node Wasm suites,
three released FXH fixtures, historical replay, browser rollback,
highlighting, and three token seals passed. Linux x64 was compile-checked.
Both defaults produced identical binary32 artifact bytes.

Local `binary32-safety/manifest.json` SHA-256
`C94C83531FC8C89E973BC01CB87EF8EAF34BBAA69F30561B03B74910C89F5F8C` binds the
frozen inputs, bit expectations, compiled artifact, and compatibility logs.
Reproduce the focused check with
`cargo test --locked --release --features typed-jit --example bench_typed binary32_boundary_matches_frozen_bits_and_limits`.

The fixture covers this numerical and resource boundary. Other float
arithmetic, NaN payload and comparison behavior, broader malformed input
replay, machine-level backend isolation, native Linux and ARM64 execution,
and the broader language and capability inventory remain open.

## FXM-07 comparison with optimized Rust and Rhai

The comparison freezes four programs across seven workload sizes on Windows
x64. It uses Rust 1.99.1 release builds with overflow checks, FluxR typed JIT
with telemetry off, and checked Rhai 1.27.0 with Simple optimization and
fast operators. The optional Grain build has zero residuals on these cases.
The development pin is `51ef8e01e763447a577ead8758eec67fefcb9803` and the
standalone pin is `cad8c444ae8839f76a6772ef378b4e2eb28c6fbe`. Their structured
runtime paths match the current defaults. The compact parser has since gained
grouped bindings. Token counts use the frozen source spellings.

Both builds ran twice per repository on RYDLEGION CPU 12 in development,
standalone, standalone, development order. Each warm route recorded 25 batches
of 512 calls with matching checksums. The table gives median process p50 and p95
of batch means in ns. An asterisk marks a repeat spread above 5% at p50 or 10%
at p95 within the same checkout and build.

| Warm host group | Rust | FluxR | Default Rhai AST | Checked Grain |
| --- | ---: | ---: | ---: | ---: |
| Signed leaf | 8.30 / 8.50 | 33.30 / 34.77 | 439.36 / 456.05 | 310.94 / 321.58 |
| Eight-way decision | 8.20 / 8.40 | 30.76 / 31.25 * | 751.86 / 760.45 | 467.97 / 481.93 |
| Fold 16 | 9.38 / 9.38 | 1320.41 / 1336.33 | 2172.07 / 2194.73 | 1639.36 / 1663.67 |
| Fold 256 | 67.48 / 88.57 * | 18848.34 / 18962.11 | 24609.96 / 24916.21 | 17241.70 / 17666.99 |
| Record 0 updates | 8.59 / 8.79 | 75.10 / 77.64 * | 776.56 / 787.31 | 619.24 / 633.79 |
| Record 8 updates | 9.96 / 10.06 | 134.77 / 136.72 | 4215.14 / 4233.98 | 2485.45 / 2542.19 * |
| Record 64 updates | 37.70 / 37.99 | 218.85 / 224.02 | 27494.14 / 27721.09 | 15115.43 / 15505.86 |

By p50 geometric mean across these groups, default Rhai AST took 11.59×
the FluxR host time and FluxR took 16.98× the Rust host time. FluxR prepared
native exports for the leaf, decision and record cases. Both folds used the
portable route. Grain led FluxR on the 256-element host fold. The packed
public calls were nearly tied there at p50, with FluxR at 18.76 µs and Grain
at 18.73 µs. The checked Grain VM retained its operation count across calls.
The initial reuse attempt failed in the 16-element fold. A separate probe at
a 64-operation limit rejected reused call 65, and 128 fresh VMs passed.
Corrected warm Grain timings include VM construction for every call.

The sealed report separates startup, compilation, preparation, first call,
artifact decode and activation. For the decision case, median process p50
was 17.95 µs for FluxR source compilation, 434.15 µs for native preparation,
and 459.85 µs for source activation with a probe and lazy preparation. Rhai
source compilation took 27.55 µs and Grain lowering took 14.15 µs. The
Rhai AST replacement adapter took 4.50 µs for activation. Grain artifact
decode and execution were checked separately from transactional replacement.

The cases exercise ordered checked arithmetic, argument shape, configured
resource limits, host grants or registration, snapshot ownership and
replacement rollback. FluxR meters instructions, logical cells and frames.
Rhai's configured sandbox meters operations, call depth and individual
collection sizes, disables `eval` and uses a dummy module resolver. FluxR
checks the full typed replacement interface. The Rhai adapter checks
revision and arity and probes the result type. The Rust control uses trusted
typed code and host input limits. Machine isolation, hostile Grain bytecode,
callbacks with external state, and broader cross-platform behavior remain open.

Across six tokenizers, FluxR authored source totaled 83.7–88.3% of Rust and
134.5–154.5% of Rhai. The decision case used fewer FluxR tokens than Rhai
on every tokenizer. FluxR edit bodies totaled 75.5–85.9% of Rust and
102.5–110.4% of Rhai. FluxR initial and edit contexts totaled 151.4–161.9%
and 119.8–126.2% of Rust, respectively. The raw counts track handwritten
host bindings separately. No tokenizer met the 10% Rust source threshold.

The local `fxm-07/manifest.json` SHA-256
`0603D47126E4D16790A68C8369BF745B2ACCEC02847725A8D10830F1FC1CDFE0`
binds the source and toolchain pins, raw runs, failed Grain reuse, token counts
and reproduction commands. Review recomputed the paired summaries and recounted
all six encoders. The frozen AST and checked Grain harnesses passed behavior
and lifecycle replay against both current defaults.

Typed-list traversal is a separate experiment. A compact form must preserve
released signatures and artifact bytes, replay exact traps and fuel, and
reduce authored and edit tokens on independent list workloads. Measure
grammar and host binding costs separately. A runtime change needs at least
5% lower target p50, p95 within 10%, and unrelated warm p50 within 5%.
Both warm parity gates, the whole-program token target, and live generation
and editing remain open.

## FXM-07.A explicit-bound scan

`scan` expands through the same checked indexed loop as the existing compact
collection forms. Its body uses ordinary list access and control flow. The
bound is tested on each iteration, and continue skips the trailing increment.
Public interfaces, canonical printing and the artifact format are preserved.

Four workloads cover checked squares, filtered differences, sentinel
termination, and ragged nested traversal. Across six encoders, the scan
spellings saved 21.4–24.1% of original source tokens, 24.1–26.2% of edited
source tokens and 33.3–36.2% of replacement payload tokens. Both localized
edits tied. The two edits adding a traversal supplied the payload savings.
Shared host bindings were identical. Teaching the form added 33–39 tokens.
With the instruction shared, the four source and edit pairs saved 89–119
tokens after paying for it. Standalone cold prompts grew when each request
carried its own instruction.

All eight original and edited artifacts match the preceding defaults byte
for byte. Both released CLIs read, print and recompile the candidate artifacts
to those same bytes. The frozen workloads match across 31,200 fuel and cell
limit comparisons on each checkout. The focused test also exercises a
changing bound and continue. Both checkouts pass 82 release tests, TextMate
scope checks and 12 Node Wasm suites. The local
`typed-list-traversal/manifest.json` SHA-256
`BBA9C2FFD5A07532A9CD898EAEFA37D63C147745E40E6B1D0D51429734E5DBF2`
binds the freeze, token counts, artifact hashes and validation logs.

## FXM-02.D.10 borrowed indexed reads

Both independent Win64 defaults reproduced the six D.9 host workloads
with their frozen checksums and exact behavior checks. The next residual
was the portable FXM-07 fold. The Rust control checks ordered integer
addition, bounds, and the matching logical instruction, cell, and frame
limits. Both hosts use the same prepacked inputs. Rust's signed element
shape is enforced by its type system.

The candidate marked direct list and index operands for borrowing in the
portable evaluator. It kept the original bytecode positions and separate
fuel debits. General indexed expressions continued to use the stack.
Rust 1.99.1 release builds ran with telemetry off on logical CPU 12, in
ABBA and BAAB order on each checkout. Each warm percentile describes 21
batch means of 512 fold calls. The table gives the range of the grouped
p50 across both defaults, process orders, and the bundled, source-injected
and artifact-injected routes.

| Workload | Metered Rust | FluxR baseline | FluxR candidate |
| --- | ---: | ---: | ---: |
| Fold 16 | 0.0109–0.0113 µs | 1.232–1.251 µs | 0.999–1.018 µs |
| Fold 256 | 0.138–0.156 µs | 18.29–18.45 µs | 14.62–14.94 µs |

The three FluxR routes improved by 17.6–20.6% in grouped p50 geometric
mean across the two fold sizes. Three of four groups met every frozen
guard. In standalone ABBA, source-injected record reuse rose from
26.01 to 35.40 ns at p50 and from 28.19 to 36.73 ns at p95. One candidate
process supplied the outlying timings. The group exceeded the 5% p50 and
10% p95 control limits. The candidate was rejected and the original
runtime was restored in both checkouts.

For the retained 256-element fold, source compile p50 was 5.00–5.35 µs
and artifact decode was 9.70–9.80 µs. The native preparation attempt
declined in 0.10 µs. Activation without a probe took 0.15–0.20 µs, and
the first call on a reused workspace took 18.20–20.95 µs. The seal
retains both builds' full lifecycle measurements.

Each of the 16 measured processes passed 39,864 exact comparisons across
limits, traps, and admission, with list ownership checked after each
validly typed call. The fold and eight scan artifacts retained identical
bytes across both builds and both released readers. The restored defaults
each passed 82 all-target all-feature release tests and 12 Node Wasm suites.

Across the six D.9 workloads, the retained bundled runtime took
2.25–2.30× the Rust host at p50 and 2.07–2.11× at p95 by geometric mean.
Four grouped injection comparisons missed the warm parity thresholds.
Both warm gates remain open. Local `runtime-parity-d10/manifest.json`
SHA-256 `6F8065C2506EC872C560A4C8D3900F3946E9A69539D6C06136E545C88B272244`
binds the freeze, patch, binaries, compatibility logs and timing data.

## FXM-05.I nested list admission, access, and mutation

A two-frame procedure selects a row from a nested list, pushes and pops
on its local copy, then appends a value read through two indices from the
original host snapshot. The source ledger charges the snapshot before
the arguments, even when they share the same list, and visits each row
and element before entering either frame. The first push charges the
row length plus one. The pop uses the private row, and the final push
charges one cell. A successful call with two aliases of a two-element
row in both roots needs 18 instructions, 32 cells, and two frames.

The 33,601 cases per route cross every nearby fuel, cell, and frame
threshold with malformed roots, rows, elements, indices, and arity,
as well as empty rows and row and column bounds. Indices with bit 32
set check against truncation on Wasm. Bundled, forced portable, decoded
portable, source-injected, and artifact-injected calls match the ledger.
Both Windows native tiers decline the export. The separate structured
boundary suite covers the maximum admitted type depth.

The successful result owns a detached row. Failed replacement probes
cover frame entry, fuel after the first push, bounds after the push and
pop, and cell exhaustion during result admission. Each preserves the
active bytes, revision, and host values. A successful probe publishes
the edited row calculation.

Each independent default passed 83 all-target, all-feature Windows
release tests and 13 Node Wasm suites, plus three released FXH fixtures
under Node. Linux x64 was compile-checked. The new fixture, binary32
fixture, fold, and eight scan artifacts matched byte for byte across
both builds and both retained readers. Local
`list-boundary-safety/manifest.json` SHA-256
`A54C606B5246D7C16FB11B9B02BB98494980652C6A0DCF32518C8EAAFA921828`
binds the freeze, results, and compatibility artifacts.

Reproduce the focused check with `cargo test --locked --release --features
typed-jit --example bench_typed list_boundary_matches_effect_ledger`.
The Node Wasm example exports `list_boundary_check()`. Wider safety
coverage stays open. FXM-02.D.11 resumes warm measurements with a
separately frozen residual and retains the D.9 and D.10 rejections.

## FXM-02.D.11 bulk signed-list admission

The pilot replayed the six retained host workloads on both defaults. To
isolate admission, it varied a signed list from 16 to 256 elements while
running zero fold iterations. Bundled p50 rose from 123–129 ns to
723–729 ns. Reading one element took 193–196 ns and 789–803 ns,
respectively. This experiment targets admission before frame entry.
The indexed-read and signed-record paths are unchanged.

The portable evaluator now checks the affordable prefix of a flat signed
list before debiting its cells in bulk. A bad tag within that prefix
traps before cell exhaustion. If a cell cannot be charged, exhaustion
wins over any bad tag at or beyond that element. An empty list does not
visit an element depth. Each nonempty flat list checks depth before
inspecting the prefix. Nested admission retains its visit order, and
floating-point lists use the same existing prefix rule.

Win64 Rust 1.99.1 release builds ran with typed JIT enabled, telemetry off,
and affinity set to logical CPU 12. Each default ran ABBA and BAAB twice.
The table gives p50 ranges across defaults in ns per call. Each p50 is
the median of eight process p50s, each from 21 means of 512 calls. All
routes checked every timed batch checksum. The equivalent Rust host
uses a prepacked typed slice and the same logical limit debits. The
slice type supplies the signed element shape.

| One read | Route | Before | After |
| --- | --- | ---: | ---: |
| 16 values | Rust host | 3.91 | 3.91 |
| 16 values | Bundled | 193.85–195.12 | 159.47–160.84 |
| 16 values | Source-injected | 194.04–195.61 | 159.76–160.06 |
| 16 values | Artifact-injected | 193.45–193.94 | 159.38–159.96 |
| 256 values | Rust host | 4.10 | 3.91 |
| 256 values | Bundled | 786.91–802.73 | 225.39–226.56 |
| 256 values | Source-injected | 787.60–804.98 | 224.81–226.75 |
| 256 values | Artifact-injected | 786.62–802.63 | 224.71–226.85 |

Route geometric means improved 17.3–18.1% and 71.2–72.0%, respectively.
The full 256-value fold moved from 18.29–18.40 µs to 17.70–17.90 µs at
p50 across all three FluxR routes. The unchanged signed-record reuse
control increased by at most 0.94% at p50 and 3.11% at p95. All frozen
regression guards passed on both defaults.

| 256-value list, one read | Before p50, µs | After p50, µs |
| --- | ---: | ---: |
| Compile source | 5.05 | 5.00 |
| Decode artifact | 9.95 | 9.75–9.80 |
| Native preparation attempt, both routes | 0.10 | 0.10 |
| Source activation | 0.20 | 0.20 |
| Artifact activation | 0.20 | 0.15–0.20 |
| First call, both routes | 0.80 | 0.30 |

The fold runs in the portable evaluator. The six original workloads took
2.29–2.31× matched Rust at p50 and 2.08–2.11× at p95 by geometric mean.
The grouped source and artifact comparisons met both injection bounds
in this run. Forty candidate process comparisons exceeded a bound, and
both program-wide warm parity gates remain open.

Each timing process passed 39,864 exact fold comparisons across
fuel, cells, frames, malformed inputs, bounds, overflow and host-list
ownership. Both defaults passed 83 all-target, all-feature Windows
release tests, 13 Node Wasm suites, three released FXH fixtures and a
Linux x64 compile check. The FXM-05.I ledger matched 33,601 cases per route
including replacement rollback. Eleven artifacts matched byte for byte
across both builds and both retained readers.

Local `runtime-parity-d11/manifest.json` SHA-256
`E69FC1A98F767E22DBB2E5BDD769C5F6F0F14C802E707D037F55F32B37956FC0` binds the sealed candidate, profiles, timing
guards, raw results and compatibility replay. FXM-05.J next examines
variant payload extraction under competing traps and replacement.

## FXM-05.J guarded variant payload extraction

The source ledger follows two variants that carry the same signed-list type
and a third variant with no payload. A tag test bypasses extraction for
either alternate variant. Forcing extraction traps for both an absent
payload and a populated payload with the wrong tag. The helper copies the
selected list on its first write and reads the original snapshot through
a separate payload extraction. With two elements shared by the input and
argument, success needs 18 instructions, 24 cells, and two frames.

The ledger matches 30,619 calls per route across bundled, forced portable,
decoded portable, source-injected, and artifact-injected execution. The grid
crosses nearby fuel, cell, and frame limits with malformed root types, tags,
payloads, and arguments. Both Windows native tiers use the portable path
for this export. The result owns a detached list and the host retains both
original variants.

The replacement switches the snapshot extraction to the other populated
variant. Probe failures cover frame entry, fuel before extraction, cell
exhaustion at the first write, and a wrong tag after that write. Each leaves
the active bytes, revision, and host values intact. Probing with the expected
snapshot tag then activates the replacement.

Both repositories passed 84 Windows release tests, 14 Node Wasm suites,
three historical FXH fixtures under Node, and a Linux x64 compile check.
The new fixture and the eleven retained FSM artifacts matched both current
builds and both retained readers. The D.11 runtime is byte-identical.
Local `variant-payload-safety/manifest.json` SHA-256
`4EFD1831754204F5C976A2B998BCD03F3A9708F3DFE9BEA76969B0DACD0ACD68`
binds the frozen ledger and compatibility results.

Run `cargo test --locked --release --all-features --example bench_typed
variant_payload_matches_effect_ledger` or call `variant_payload_check()`
in the Node Wasm example. Wider safety coverage and both warm parity gates
remain open. FXM-03.B.4 next examines the remaining rule projection gap
for integer bitwise and while retaining canonical FXH v1.

## FXM-03.B.4 opt-in signed rule projection

`Hir::prepare_rule_projection()` prepares integer rules, including signed
bitwise and, for the portable structured evaluator. Its opaque result runs
with explicit instruction, cell, and frame limits. The rule-only compiler
mode checks both operands as signed integers. Ordinary structured boolean
and retains its short-circuit semantics. FXH v1 still omits the compact
projection for these rules, and the existing source inspection, migration,
restoration, and rule telemetry keep their released behavior.

Each platform run matched 16,195 cases against the rule evaluator and
independent Rust oracles, including eager select in both directions, guard
ordering, signed extrema, checked overflow and remainder, and stack and
instruction limits. The two-input AND requires 10 instructions, eight
cells, and one frame. Workspace reuse succeeds after budget traps. Rules
with raw binary32 operations retain their projection refusal.

Both defaults passed 85 Windows release tests, 15 Node Wasm suites, three
historical FXH fixtures under Node, and a Linux x64 compile check. Six CLI
builds, including four retained binaries, agreed on 15 rule and limit cases
and a malformed FXH envelope. The replay compares canonical FXD and FXH,
restored bytes, receipts, and exact refusals. Twelve sealed FSM artifacts
also match. The D.9 and D.10 negative results and D.11 admission evidence
remain in the retained corpus.

Windows x64 portable release on CPU 12 with telemetry off measured two
sign distributions in four processes per default. Each reported percentile
is the median of four process percentiles over 21 round averages. Projected
AND ran at 81.18–81.40 ns p50 and 81.91–82.50 ns p95. It was within
0.3% p50 and 1.4% p95 of a checked structured add with the same stack shape
and host work. The legacy rule evaluator took 9.95–15.85 ns p50, and the
optimized Rust kernel took 1.83 ns. Projection preparation took 2.01–2.02 µs
p50 from an already checked HIR. The new route adds an explicit checked
execution option with its own metering.

Run `cargo test --locked --release --all-features --example
bench_rule_projection` for the behavior ledger. The same example provides
the Node Wasm check and portable host benchmark. Local
`rule-and-projection/manifest.json` SHA-256
`5B0BE0C2600424FC5E5A2FFA08ACC787C0C0CD87AA71FE78067D10D8721CCDE7`
binds the freeze, source, controls, and replay. FXM-03.B.5 next scopes raw
binary32 projection and its low-word transport contract.

## FXM-03.B.5 opt-in raw binary32 rule projection

`Hir::prepare_rule_projection()` now handles all six released raw binary32
operators. Rule slots remain signed integers. Each float operation reads the
low 32 bits of both operands. Arithmetic returns zero-extended result bits,
including signed zero, infinities and canonical NaN `0x7fc00000`. Ordered
comparisons return signed zero or one. The portable checker preserves eager
select, guard order and checked signed traps in mixed rules. A two-input raw
operator takes ten VM instructions, eight cells and one frame.

The 68,712 probes per platform compared exact words and traps with the legacy
evaluator, the restored prepared rule and independent Rust. The signed AND
ledger retained its 16,195 checks. Both independent defaults passed 86 Windows
release tests, 16 Node Wasm suites, three historical FXH fixtures under Node,
and Linux x64 compilation. Six CLI builds agreed across 32 rule and limit
cases and a malformed envelope. Canonical FXD, FXH v1, receipts, refusals and
twelve retained FSM artifacts matched. The opt-in VM leaves legacy rule
telemetry unchanged. D.9 through D.11 evidence remains retained.

Windows x64 portable release used CPU 12 with telemetry off. Each percentile
is the median of four process percentiles from 21 round averages. Projected
add, divide and ordered comparison took 80.63–86.08 ns p50 and
81.39–86.57 ns p95 across the two distributions and defaults. They were
within 1.7% p50 and 1.5% p95 of typed controls with the same VM limits and
workspace reuse. The legacy rule took 10.21–12.77 ns p50, and the Rust kernel
took 3.02–4.52 ns. The mixed float and AND chain took 126.41–130.08 ns p50
versus 14.74–17.86 ns legacy and 3.64–4.56 ns in Rust. Preparation from
checked HIR took 1.78–1.80 µs for scalar rules and 2.75–2.77 µs for the
mixed rule. Signed AND passed its matched typed and prior-build guards.

Run `cargo test --locked --release --all-features --example
bench_raw_rule_projection` for the differential ledger. The same example
provides the Node Wasm check and portable benchmark. Local
`raw-rule-projection/manifest.json` SHA-256
`842A0B17F59A35E673AE3CAB34B80AB6B2AF2AC4E39134BDA6E14516E2E22C01`
binds source, controls and replay. FXM-02.D.12 next isolates the remaining
per-element cost in the portable signed-list fold after D.11.

## FXM-02.D.12 portable signed-list fold

On both Win64 defaults, the 256-element signed fold took 43,689â€“43,826
thread cycles with a reused portable workspace. The same admission with
zero iterations took 347â€“350 cycles. A count-only loop on the same
admitted list took 21,068â€“21,183 cycles at 256 iterations. The controls
separate the loop from admission and show substantial checked loop work
even without an indexed read. Cycle counts cover the whole call.

The candidate recognized the exact checked fold instructions and borrowed
the admitted list. It preserved fuel charges before bounds and overflow
checks, and the frame and cell limits. Rust used a prepacked signed slice
with the same logical debits. The table shows warm p50 across both
defaults and all three FluxR routes on Win64 CPU 12, Rust 1.99.1 release,
with telemetry off. Each cell spans medians of eight process p50s per
build and default. Each process measured 21 means of 512 checked calls.

| Elements read | Metered Rust | Retained FluxR | Candidate FluxR |
| --- | ---: | ---: | ---: |
| 16 of 16 | 13.48 ns | 1.184â€“1.190 Âµs | 42.58â€“42.87 ns |
| 64 of 256 | 43.55â€“47.27 ns | 4.492â€“4.523 Âµs | 147.07â€“149.32 ns |
| 256 of 256 | 164.06â€“191.02 ns | 17.45â€“17.56 Âµs | 288.87â€“291.12 ns |

The candidate improved full-fold p50 about 60Ã—, but failed five
development source-injected guards. Record reuse rose from 26.20 to
36.24 ns at p50 and from 27.41 to 37.32 ns at p95. Signed typed calls
had the largest p50 increase, 55.4%. The standalone build passed the
relative guards, though its baseline already showed slow source-injected
record reuse at 36.84 ns p50. Both defaults retain the prior runtime.
D.9 and D.10 remain rejected. The retained one-read p50 of 220.70â€“223.73
ns on a 256-element list reproduces the D.11 admission gain.

In the retained sparse-read lifecycle, source compilation took
5.05â€“5.10 Âµs p50 and artifact decode took 9.70â€“9.85 Âµs. The native
preparation attempt declined in 0.10 Âµs. Activation took 0.20 Âµs and
the first call on a reused workspace took 0.30 Âµs.

Each timed process passed 41,040 exact fold comparisons. Both retained
defaults passed 86 Windows release tests, the 16 prior Node Wasm suites
plus the fold ledger, three historical FXH fixtures, and Linux x64
compilation. Six CLI builds agreed on 32 rule and limit cases and a
malformed envelope. All twelve retained FSM artifacts matched.
The raw binary32 and signed AND controls met their matched timing guards.
Local `runtime-parity-d12/manifest.json` SHA-256
`4AD6C02F449B0D2F23F3AC18B5A957AB3BCBFCDB6D4D2099F0082ABBA5C517F3` binds the candidate, controls, logs, and replays.
Both program-wide warm parity gates remain open. FXM-02.D.13 next
isolates the source-injected native route variance on independent builds
before reconsidering a portable fold change.

## FXM-02.D.13 native source-injection replay

A sealed Win64 replay used the exact D.12 A and B executables, Rust
1.99.1 release, typed JIT, CPU 12, telemetry off, and ABBA and BAAB
process orders on both defaults. All 16 processes passed 41,040 exact
fold comparisons and the retained host checks. Development reproduced
all five source-injected regressions. The standalone replay crossed
two record-reuse guards on bundled and artifact calls.

| Warm source route | Development A p50 / p95 | Development B p50 / p95 | Standalone A p50 / p95 | Standalone B p50 / p95 |
| --- | ---: | ---: | ---: | ---: |
| Record reuse | 27.37 / 28.60 ns | 36.55 / 37.21 ns | 37.68 / 38.85 ns | 36.81 / 37.87 ns |
| Signed typed | 17.05 / 17.40 ns | 26.68 / 27.62 ns | 27.13 / 27.83 ns | 27.25 / 27.95 ns |

Each value is the median of four process percentiles. The retained
256-element sparse read took 225.10–228.71 ns p50. The full signed fold
took 17.81–18.07 µs on A and 296.10–299.12 ns on B across all three
routes. The retained runtime still includes the D.11 sparse-admission gain.

For the signed and record controls, source compilation took 6.00–6.60 µs
p50 and artifact decode took 7.70–9.60 µs. Native preparation took
55.05–78.75 µs, depending on shape and route. Prepared record reuse
activation took 0.70–0.80 µs and its first call took 0.10 µs at p50.
The warm timings exclude these phases.

The diagnostic crossed source and artifact preparation order and swapped
the two prepared slots at fixed closure call sites. It rotated timed calls
and measured wall time and thread cycles separately. Generated JIT bytes
matched across all origins, builds, and defaults for each case. Signed
named calls took 23.05–24.23 ns p50 across source and artifact. Resolved
calls took 14.88–15.38 ns, while prebound native calls took 11.07–11.49 ns.
The pack-only control stopped before kernel execution and output handling.
The control overlaps other routes and cannot provide an exclusive admission
cost. The record and signed source gaps from the sealed executables did
not reproduce in the crossed calls.

With the diagnostic branches disabled, the rebuilt host replay still
showed a smaller development B record-reuse gap, 32.39 ns for source versus
30.05 ns for artifact at p50. Signed route medians differed by at most
0.44 ns within any rebuilt binary. CDB with matching PDBs showed changes
in host instruction selection and placement between A and B. The results
establish sensitivity to the build and harness. They leave layout alone
and the cause of the residual unresolved.

Both defaults retain the runtime. D.9, D.10 and D.12 remain rejected.
Both raw and signed projection timing guards passed. Each retained default
passed 86 Windows release tests, 17 sealed Node Wasm suites, three historical
FXH fixtures and Linux x64 compilation. Six CLI builds agreed on 32 rule
and limit cases and a malformed envelope. All twelve retained FSM artifacts
matched. Local `runtime-parity-d13/manifest.json` SHA-256
`5E0D059DC1E6AA6B274451B90809A27299AB355D533C993AF105781EBF233CE5` binds the executables, diagnostic, replay, and validation.
FXM-02.D.14 tests layout independently before another runtime candidate.
Both program-wide warm parity gates remain open.

## FXM-02.D.14 same-object Win64 link layout

MSVC LINK 14.44 relinked one development build from identical COFF objects,
libraries and flags, with three hot symbols placed first by `/ORDER` in B.
The `Module::run_in`, `ModuleSlot::run_with` and typed `Native::run` RVAs
moved from `0x5cb20`, `0x74c70` and `0x99d80` to `0x1000`, `0x1b40` and
`0x1b60`. CDB loaded the matching PDBs. The three entries and two benchmark
call sites matched byte for byte after masking 84 COFF relocation fields.
Resolved destinations for named function targets matched the maps. A separate
preflight matched JIT bytes and encoded modules across all three origins for
record reuse and signed typed calls. The JIT bytes also matched D.13.

Win64 CPU 12, Rust 1.99.1 release, typed JIT, telemetry off. Values are medians
of four process percentiles per layout in ABBA and BAAB order.

| Warm route | A p50 / p95 (ns) | B p50 / p95 (ns) |
| --- | ---: | ---: |
| Record reuse, bundled | 27.385 / 28.310 | 25.225 / 26.410 |
| Record reuse, source | 26.755 / 27.400 | 25.190 / 26.755 |
| Record reuse, artifact | 26.655 / 27.950 | 25.375 / 26.850 |
| Signed typed, bundled | 17.370 / 17.790 | 16.990 / 17.405 |
| Signed typed, source | 17.070 / 17.675 | 17.025 / 17.285 |
| Signed typed, artifact | 17.020 / 17.470 | 16.975 / 17.390 |

The source-minus-artifact p50 contrast moved by 0.285 ns for record reuse
and was flat to the reported precision for signed typed. This placement
contrast did not reproduce the sealed D.13 gap. Bundled record reuse changed
about 7.9% p50, outside the 5% control band. All 57 injection comparisons
passed the one-sided D.12 non-regression limits. The instrumented build
and single layout contrast leave the broader cause unresolved.

Each of eight processes passed 41,040 fold comparisons and the exact host
ledger. Separate sealed projection binaries replayed both defaults. Ten
standalone raw projection comparisons missed the 10% p50 limit, peaking at
14.91% over the control. Development raw and both signed guards passed.
Both defaults passed 86 Win64 release tests, 17 sealed Node suites, three
historical FXH checks and Linux x64 compilation. Six CLI readers agreed on
32 rule and limit cases and a malformed envelope. All twelve FSM artifacts
matched. Both defaults retain the runtime, D.11 sparse-admission gain,
and D.9, D.10 and D.12 rejections. Both warm parity gates remain open.

Local `runtime-parity-d14/manifest.json` SHA-256 `98782DBB7BD6D7C7DA5FFF60F8B120D37FC671566AAACDC3F22CF645617F818D` binds the link
inputs, normalized code, replays and guard results. FXM-03.B.6 can advance
independently by extending the checked FR1 ingress and historical replay
corpus.

## FXM-03.B.6 checked FR1 replay

The new `route` and `pick` packet binds a nominal record, a two-tag effect
list, and an indexed collection export through the released FR1 alphabet.
The planner emits a payload, then guards the rest of the sequence. A false
guard returns the first effect without reading the list. A true guard emits
the tag without a payload and then reads index 1 for the final payload.
`pick` covers both ends of the list and indices that exceed its length.

The archived `bd4dbca` reader and both defaults emitted identical compact
source and FSM v2 bytes. The FXM-03.B.2 CLI and both defaults emitted identical
FXH v1 bytes and receipts. Each default matched all 256 archived results
through direct compile, compact compile, compact migration, current FXH
restoration, and released FXH restoration. Values and first traps matched
across the instruction, cell, and frame sweeps. The successful full planner
path needs 28 instructions, 21 cells, and one frame. With the list too short,
the bounds trap becomes visible at 23 instructions and 12 cells.

Six malformed packets or interfaces exited with exact diagnostics and produced
no output. The original reader reports a missing nominal lookup for an
`i64` collection argument. Both defaults report a signature mismatch because
they recognize the scalar shape. The freeze records that diagnostic split
and the extension of the full planner cell sweep from 14 to 26.

Each default passed 87 Win64 release tests, four Node Wasm FXH checks, and
Linux x64 compilation. The B.3 replay also passed 18 inputs, 843 probes, and
15 refusals per default. Run `cargo test --locked --release --test hir
fr1_route_and_pick_preserve_released_results_and_metering` for the retained
fixture. Readers and runtime code retain their released behavior.

Local `fr1-hir-replay/manifest.json` SHA-256
`B9FA5882122B73F25E6F6D50A919258373C0CFBF1299C3C330B4662C01662F51`
binds the frozen packet and interface, amended probes, outputs, and
validation. Wider checked ingress and historical replay remain open.

## FXM-05.K shared host-value admission

The fixture nests records, lists and variant payloads down to depth 31,
counting the root as depth zero. A deeper declared type is rejected. The
same leaf allocation occurs at depth 27 under the tree root and at depth zero
as a separate input or argument. The shared values use 31 allocations across
four roots. The equal expanded values use 8,192.

The independent ledger counts seven visits for the base record and uses
`T(n + 1) = 4 + width × (1 + T(n))` for each layer. The four roots need
13,314 admission cells. Success needs two instructions, 13,318 cells and one
frame, including the frame, two argument slots and scalar result. Ranked
errors verify declaration order for inputs, parameter order for arguments,
and admission before frame and fuel checks. The first malformed scalar
fails on cells at 41 and reports its type error at 42. Its unexpected child
would sit at depth 32. The checker stops at the scalar's type error.

Each recursive visit checks depth and debits a cell before its tag can affect
the result. Record, variant and list children are reached through admitted
parents. The bulk scalar-list path bounds tag reads to the affordable prefix.
For a fixed interface and a cell cap of `C`, admission inspects at most
`C` value tags and can attempt one more depth or cell check. The bound
follows from the evaluator's control flow. The capped stress probes check
observable errors on a width-eight root with 31 allocations and
1,169,611,628 logical visits. They spend at most 4,096 cells and include
both reachable and unaffordable malformed values. This bound covers
admission of already-constructed host values.

The ledger matches 433 cases per route, including 412 shared and expanded
cases and 21 stress cases. It covers bundled, forced portable, decoded
portable, source-injected and artifact-injected calls. Win64 native
preparation declines this shape. A rejected replacement probe preserves the
revision and bytes at both sides of the malformed leaf's cell threshold. A
valid probe then activates the replacement.

Each default passed 88 Win64 release tests, one focused portable-only test,
15 Node Wasm suites, four FXH cases under Node, and a Linux x64 compile check.
The new FSM fixture and 12 earlier fixtures matched both current builds and
both retained readers byte for byte. Runtime and reader sources are unchanged.
Run `cargo test --locked --release --all-features --example bench_typed
shared_host_values_match_admission_ledger` or call
`shared_host_admission_check()` in the Node Wasm example.

Local `shared-host-admission/manifest.json` SHA-256
`166FC99D75D21CF9741666CC079CDA549F46F76EF374EA1E729E492541F33D41`
binds the freeze, validation and artifacts. Wider safety remains open.

## FXM-03.B.7 released FR1 semantic edits

The archived B.6 packet supplies the guard and indexed collection shape. Its
archived writer seals a related packet with raw binary32 effect payloads.
The effect edit targets one tag and matches negative zero by bits. The
combined batch replaces it with the smallest positive subnormal, shifts
the collection index from `index` to `index + 1`, then replaces the new
literal with the smallest negative subnormal. Another tag's negative zero,
the target tag's positive zero, and its NaN payload remain exact.

The original, effect-only, collection-only, and combined packets match
the archived reader byte for byte after editing, lowering to compact source,
and compiling to FSM v2. The B.2 migration tools and both defaults agree on
canonical compact source, FXH v1, and both receipt types. The receipts
cover 123 probes per packet around observed transitions. The full replay
uses 664 probes per packet through direct compile, compact compile, compact
migration, current FXH restoration, and released FXH restoration.

The guard exits after the first effect at 14 instructions and 11 cells.
The complete effect list needs 32 instructions and 23 cells. The shifted
index returns the second element at six instructions and eight cells.
An index of one on that two-element list traps at five instructions and
seven cells. An index of `u64::MAX` traps on overflow at four instructions
and seven cells. All these paths require one frame. The unedited collection
reports a bounds trap at `u64::MAX`.

Three batches fail on their last edit. The reader refuses a missing effect
literal, an unknown collection export, and an incompatible collection
result. Every refusal leaves fresh output paths absent and preserves
preexisting packet, source, FSM, and generated Rust files. The archived
reader reports the incompatible result from the compiler. B.2 and both
defaults report it from the FR1 binder. Migrate the emitted edited packet
in a separate invocation, as the migration flags reject `--edits`.

Each default passed 89 Win64 release tests, one focused portable check,
15 Node Wasm suites, five FXH cases in Node, and Linux x64 compilation.
B.3 repeats 18 inputs, 843 probes, and 15 refusals per default. B.6 repeats
256 probes across five routes. Run `cargo test --locked --release --test
hir fr1_semantic_edits_preserve_binary32_bits_and_trap_order` to check
the retained combined fixture. This checkpoint preserves the alphabet,
runtime, and readers.

Local `fr1-semantic-edit-replay/manifest.json` SHA-256
`8E89B512D15DB0E35A12FE90CA9F38E680DF19C26ECBFF59A04BBD4F4411A7E3`
binds the frozen inputs, archived and current results, and validation.
Wider HIR and safety coverage remain open. FXM-05.L next checks ordered
method calls with a record receiver and trapping arguments.
