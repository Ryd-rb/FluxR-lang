# FluxR development guidance

- Preserve checked semantics, released source readers, canonical artifact
  encodings, compatibility dictionary pins and observable runtime behavior.
  Keep notation versions separate from semantic and artifact versions. Current
  application source is FluxR 1 and includes the mandatory runtime.
- Aim for performance close to equivalent optimized Rust. Compare identical
  behavior on the same target, profile, instrumentation and resource settings.
  Measure host entry points and kernels separately, and separate execution from
  compilation, preparation, startup and replacement. Scope claims to measured
  workloads and platforms; retain a reproducible baseline where no Rust control
  exists.
- Target source tokens at most 10% of equivalent Rust. Name the tokenizer and
  report authored source, Rust bindings, shared libraries, task context, edits,
  retries and diagnostics separately. Prefer the smaller measured representation
  for agent-facing source and edit bodies. Preserve every released reader.
- Add a behavior check for each new language case and use the independent
  semantic fixtures for changes to evaluation, traps, ownership or metering.
  Add matched source/edit pairs where token cost matters and a matched release
  benchmark for a new execution path or material performance risk. Reuse tests
  and benchmarks that already cover the change.
- Keep the README and current interface documentation accurate. Update
  `docs/requirements.json` and `docs/current-status.md` when implementation scope
  or acceptance changes. Product goals and proposed features must remain
  distinguishable from implemented behavior and completed validation.
- Keep generated projects, Cargo targets, compiler caches, temporary files and
  qualification outputs outside verified source trees. Use the resource
  coordinator for sustained concurrent qualification; see
  `docs/resource-safety-v1.md`. Never treat trusted native code or the local
  verification driver as a hostile-code sandbox.
- When integrating with development and standalone release repositories, carry
  accepted language, runtime, tooling and example changes to both and run
  equivalent behavior and compatibility checks. Preserve each repository's Git
  history and report any unresolved parity work. A source snapshot alone does
  not demonstrate synchronization or authorize remote publication.
