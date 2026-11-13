# Native interoperability qualification

Run these commands from the repository root with Python 3.11+, Rust/Cargo 1.99.1
and the locked dependencies available. Create fresh work/result directories
outside the source tree and use a separate external Cargo target directory.
Use the resource controls described in [integration](../../docs/integration.md)
for expensive runs. Each command's receipt identifies its exact subject; missing
tools, unsupported targets and failed cases do not become passes.

## Paired fixtures

`capabilities.rs` provides 30 independently asserted native Rust cases covering
ownership, threads, synchronization, atomics, networking and sockets.
`capabilities.fxn` contains current `fluxr 1` source, including three native
prefix implementations: atomic increment, owned thread closure/join and TCP
`write_all`. The `.fxn` suffix is a fixture filename; its header selects the
current language. Rust scaffolding is explicit, length-delimited native payload.

`structured_semantics.fxn` contains 20 independently specified function bodies
and five grouped tests of native forms, evaluation order, closure casts,
short-circuiting, references, assignment, unsafe pointers, results, async values,
UTF-8, loops and aggregates. Expected results are authored independently in
`structured_semantics.rs`.

Nine compile-fail pairs cover invalid borrowing/ownership, thread safety, atomic
orderings and main signatures. A negative case passes only with a nonzero compiler
exit and its expected structured diagnostic code. Missing compilers, malformed
fixtures and unrelated errors cannot satisfy that gate.

```sh
python3 -B tools/native_rust_tests/run_native_tests.py \
  --output ../fluxr-native-rust-results
python3 -B tools/native_rust_tests/run_native_tests.py \
  --frontend tools/fluxr.py --target-dir ../fluxr-native-target \
  --output ../fluxr-native-runtime-results
```

The first command compiles the direct Rust controls. The second uses the shared
project builder and links the mandatory runtime. Each positive emitted harness
must also pass an additional runtime admission test. Both run capability and
semantic cases in debug and release, and the nine rejection cases. The runner
retains stdout/stderr, compiler/source identities, exits and test counts. It
does not silently skip the filesystem Unix socket case when permissions deny it.

## Real PostgreSQL controls

The wire fixture covers null/empty/UTF-8 values, parameter binding, commit and
reconnect visibility, rollback, key violations, failed-transaction recovery,
type errors, concurrent incrementers, MVCC visibility and lock-timeout recovery.
It is a bounded test client with local-trust authentication; it supplies no
production SSL/SCRAM or cancellation guarantee. The challenge application and
its official qualifier exercise their separate production protocol behavior.

```sh
PG_BIN=/usr/lib/postgresql/18/bin PG_SHARE=/usr/share/postgresql/18 \
python3 -B tools/native_rust_tests/with_postgres.py \
  --output ../fluxr-postgres-rust-results
PG_BIN=/usr/lib/postgresql/18/bin PG_SHARE=/usr/share/postgresql/18 \
python3 -B tools/native_rust_tests/with_postgres.py \
  --frontend tools/fluxr.py --target-dir ../fluxr-postgres-target \
  --output ../fluxr-postgres-runtime-results
```

Run as a user permitted to start a private PostgreSQL 18 cluster. The wrapper
creates its own loopback cluster, enables `fsync` and `synchronous_commit`, runs
the capability/semantic/rejection tests plus five database tests, and stops/reaps
the server. It disables PostgreSQL filesystem socket creation so TCP database
qualification is independent of the separately required socket capability.

## Compiled runtime and CLI

```sh
cargo build --release --locked --target-dir ../fluxr-cli-target --bin fluxr-cli
python3 -B tools/qualify_unified_cli.py \
  --binary ../fluxr-cli-target/release/fluxr-cli --output ../fluxr-cli-results
PG_BIN=/usr/lib/postgresql/18/bin PG_SHARE=/usr/share/postgresql/18 \
python3 -B tools/unified_tests/run_integration.py \
  --cli ../fluxr-cli-target/release/fluxr-cli \
  --work ../fluxr-integration-work --output ../fluxr-integration-results \
  --target-dir ../fluxr-integration-target
```

These gates exercise compiled startup, authenticated control, installed code,
native bindings, revisions, in-flight snapshots, typed callbacks, real threads,
sockets and PostgreSQL. Read the result files for the exact executed populations
and outcomes. A source or compiler change requires re-running affected gates on
the changed subject.

## Performance

```sh
python3 -B tools/unified_tests/run_performance.py \
  --cli ../fluxr-cli-target/release/fluxr-cli \
  --work ../fluxr-performance-work --output ../fluxr-performance-results \
  --target-dir ../fluxr-performance-target --samples 15
```

Compare identical semantics, data, output checksums, optimization/overflow/panic
settings, target CPU, features and observable work. Record the exact compiler,
source and Cargo lock. Alternate paired order, repeat samples and disclose every
sample; distinguish batch averages from individual tail latency. Keep native
kernel cost, runtime dispatch, checked execution, preparation, compilation,
startup and publication separate. The current finite fixture cannot establish
universal Rust-speed parity or a challenge result.

## Acceptance boundaries

Native calls retain Rust ownership, unsafe-code and panic semantics. Runtime
panic catching requires unwinding; abort profiles terminate. VM fuel does not
preempt native work or undo its effects. A native callback must implement its
own cancellation, I/O deadline and resource policy.

Filesystem `UnixListener::bind` must pass on the intended platform. Permission
denial keeps the capability gate failed even when TCP, UDP and anonymous socket
pairs succeed. Full compiled/runtime/export qualification, the <=10% source-token
target, target-specific domains, and repository synchronization remain distinct
requirements; consult [current status](../../docs/current-status.md).

Use the [multicore guide](../../examples/multicore-server/README.md) for exact
official conformance commands on both full and development seeds. A challenge
load result additionally requires equivalent 8-vCPU/16-GB hardware, PostgreSQL 18,
the prescribed full dataset, three-minute warm-up, probes and five-minute hold,
p95 below 500 ms, p99 below 1 s, errors below 1%, and load-generator CPU at most
75%. Conformance, compilation and local kernel timing cannot establish the score.

## References

- [Rust threads](https://doc.rust-lang.org/std/thread/)
- [Rust atomics](https://doc.rust-lang.org/std/sync/atomic/)
- [Rust networking](https://doc.rust-lang.org/std/net/)
- [Rust Unix sockets](https://doc.rust-lang.org/std/os/unix/net/)
- [PostgreSQL 18 protocol](https://www.postgresql.org/docs/18/protocol.html)
- [Pinned challenge workload](../../third_party/multicore-reference/WORKLOAD.md)
