# FluxR

- Aim for performance as close to equivalent optimized Rust as possible on every FluxR execution path. Compare identical semantics and representative workloads on the same target, optimization profile and instrumentation setting. Measure host entry points as well as isolated kernels. Report execution separately from compilation, preparation and reload.
- Where equivalent Rust does not exist, record a reproducible baseline for the current workload. Improve clear bottlenecks and rerun the same benchmark with behavior checks. Apply this to bundled code, injected code, timelines, rules, runtime internals and host adapters.
- Preserve released source, compiled artifacts and observable behavior. Keep optimization claims scoped to the measured workload and platforms.

- Source token efficiency is a primary FluxR goal. Target at most 10% of the source tokens of equivalent Rust. Compare the same behavior with a named tokenizer and report authored FluxR, Rust host bindings, and task context separately. For AI-facing source and edit bodies, use the smaller measured representation. Avoid JSON wherever it costs more tokens. Preserve released readers and encodings.
- Add a behavior check for each new language case. Add representative source or edit pairs to `benchmarks/cases.toml` when token cost is relevant, and a matched release benchmark when the case exercises a new execution path or performance risk. Reuse existing benchmarks when they already cover the case.

- Keep the development and standalone release default branches at the same accepted feature level. Port each accepted language, runtime, tooling, and example change to both in the same checkpoint. Run equivalent behavior and compatibility checks in both repositories and preserve their independent Git histories.
