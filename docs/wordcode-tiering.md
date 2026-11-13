# WK1 bounded process tiering

The optional `tools/wordcode_tiering.py` host accepts an immutable WK1 program,
one host-selected dictionary, an export and explicit process limits. It starts
a portable worker from a sealed snapshot of `wordcode_worker`, then can prepare
the same program in a second worker. Calls continue on the portable worker while
the second process expands, checks and prepares its native tier. After the host
validates the completion, subsequent calls use the prepared worker. A call never
changes worker or revision in flight.

This is an executable process integration prototype. Its scope is reviewed
programs on Linux x86-64. It is not a real-time callback path, a hostile-candidate
grading sandbox, a distributed publication protocol, a model training result or
on-stack replacement. Its process IPC copies values; it does not transport Rust
pointers or claim cross-process sharing of the original `Arc` allocation.

## Reproduce

Build with the pinned stable Rust installation, external target and coordinator
described by the implementation pack. The following commands must run inside
that shared resource admission; they are not an instruction to start concurrent
heavy builds:

```sh
cargo build --locked --release --features typed-jit --example wordcode_worker --example wordcode_bench
python3 -B tools/qualify_wordcode.py --binary "$CARGO_TARGET_DIR/release/examples/wordcode_worker" --bench "$CARGO_TARGET_DIR/release/examples/wordcode_bench" --output /absolute/external/new-evidence
```

The same worker executable can inspect the fixed core dictionary and expand a
program. `expand` takes an optional custom dictionary document. The document is
bounded and versioned, and the source must carry its complete semantic digest.

```sh
"$CARGO_TARGET_DIR/release/examples/wordcode_worker" dictionary
"$CARGO_TARGET_DIR/release/examples/wordcode_worker" expand benchmarks/wordcode-v14/affine.wk1
"$CARGO_TARGET_DIR/release/examples/wordcode_worker" expand benchmarks/wordcode-v14/forest.wk1 benchmarks/wordcode-v14/forest.dictionary.json
```

The regular CLI adds explicit `encode-wordcode SOURCE OUTPUT` and
`expand-wordcode SOURCE OUTPUT` commands for the frozen core profile. They emit
the unchanged FSM artifact or ordinary FA1 respectively. Existing readers retain
their original meanings and do not auto-detect or reinterpret WK1.

## Host interface

```python
from wordcode_tiering import TieredSession

with TieredSession(worker_path, source_text, core_dictionary_sha256, "calc") as session:
    first = session.call([{"i64": 7}, {"i64": 13}, {"i64": 3}])
    ticket = session.begin_compile()
    # Continue portable calls; poll at a host-selected boundary.
    while not session.poll(ticket):
        pending = session.call([{"i64": 7}, {"i64": 13}, {"i64": 3}])
    prepared = session.call([{"i64": 7}, {"i64": 13}, {"i64": 3}])
receipt = session.receipt()
```

`begin_compile` permits one pending preparation. A second request refuses;
there is no unbounded queue. `cancel(ticket)` kills and reaps the pending worker,
invalidates its generation and retains the portable worker. Stale/absent tickets
refuse. A worker failure never silently retries a call. The caller receives a
failure, while a failed prepared worker is removed and future calls can use the
retained portable worker. VM traps are ordinary completed results with the exact
portable error; speculative native declines preserve the existing interpreter
replay behavior and report `Portable`.

The reference host serializes calls and publication with one host lock. It is
not a lock-free implementation, a fair multi-tenant compiler service or a claim
of scaling under multiple agent workloads. The inherited fair-dispatch and
publication contracts remain required independently.

## Immutable identity and publication

The host copies at most 64 MiB of the chosen executable into a Linux memfd and
applies `F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL`. The parent and
launcher verify those seals. Both workers execute that same descriptor; changing
the original path later cannot change a pending or future worker in the session.
The executable digest is an integrity binding to host-selected bytes. It does
not establish authorization or honest/independent semantics by itself.

An immutable ticket binds generation, full source digest, dictionary semantic
digest, exact executable digest and process configuration digest. Readiness must
have the exact protocol fields and types. The prepared worker must reproduce the
portable worker's canonical FSM and expanded-source identities before promotion.
These comparisons detect inconsistency; separately authored FA1 and checked
Rust references and adversarial tests supply behavioral evidence.

`poll` and `cancel` require the exact ticket object returned by that live session.
Its fields remain useful diagnostic identities, but equality of those fields
does not admit a copied, reconstructed or other-session handle. This also binds
the handle to the session's selected export. Receipt dictionaries are observations,
not transferable completion or cancellation capabilities. Absent, stale and
foreign handles refuse before any lifecycle mutation. This is an in-process
host boundary, not a claim of distributed authorization against hostile host code.

The native page stays inside its preparing process. Cranelift preparation and
its destructors therefore remain outside the parent invocation path. The parent
does not load untrusted executable bytes or trust a child-supplied function
pointer. It can kill/reap preparation at the process boundary. All code and
workspace owners outlive their own calls; closing the session reaps every owned
worker and closes the executable descriptor.

## Resource admission and cancellation

Defaults are one portable worker plus at most one preparing/prepared worker,
1 GiB virtual address space and 20 CPU seconds per worker, 10 seconds for
initialization, 2 seconds per call, and 120 seconds per worker session. These
are configurable execution limits within the supported maximums, not staffing
or calendar caps. CPU time is cumulative process CPU time; it is distinct from
the active wall-clock watchdog. The outer pack resource coordinator additionally
admits aggregate memory/disk/process use and preserves finalization reserves.

Before the Rust executable starts, the launcher installs Linux `no_new_privs`
and a seccomp filter that checks `AUDIT_ARCH_X86_64`, rejects x32 and denies
`clone`, `clone3`, `fork` and `vfork`. Extra compiler threads/processes are thus
unsupported and fail closed. No filesystem or network isolation is claimed by
this filter. The frozen arbitrary-candidate compiler/executor admission remains
unchanged and blocked where its stronger requirements are unavailable.

The launcher sets `RLIMIT_AS`, `RLIMIT_CPU`, zero core/file output limits and a
32-descriptor ceiling. The per-worker watchdog is live before it receives source.
It remains active even if the caller stops polling. Timeout/cancellation signals
the owned process group and waits for the leader; process creation is denied, so
the worker cannot leave child writers behind. A failed resource-filter install
prevents execution instead of silently reducing isolation.

WK1 source is bounded at 256 KiB, custom dictionaries at 64 KiB, frames at
512 KiB, diagnostics at 64 KiB, and calls at 65,536 per worker. A bounded graph
walk rejects cyclic/deep/oversized or unsupported host values before JSON
encoding or sequence mutation. Protocol replies reject unknown/duplicate fields,
nonfinite or invalid durations, boolean integer impostors, malformed values,
unsolicited output, stale sequence numbers and mismatched artifact identity.
Event admission reserves pending-completion and final-close capacity; receipt
copies cannot mutate live bounds or tickets.

## Measurements and qualification limits

`wordcode_bench` independently compares canonical artifacts and arithmetic values
for five authored cases, four practical microcases and one explicitly synthetic
density stress. It separately measures dictionary admission, expansion/checking,
native preparation, portable invocation, prepared invocation and checked Rust.
Its warm-loop statistics are batch averages; they are not individual-call tail
latency. Rust has the same arithmetic on the fixed terminating success domains,
but does not include the VM's logical fuel/cell/frame machinery. A fair Rust
`checked_pow` comparator is retained for the synthetic forest.

`qualify_wordcode.py` measures the process path, actual routes, complete call
latency samples and p95/p99, child resident/high-water memory and exact budget
errors. It records same-host phase timestamps so an observed overlap count can
distinguish portable calls during preparation from a merely unread completion.
Clock observations are local measurements, not a cross-host clock guarantee.
The unchanged executable handles all independently supplied WK1 programs and
the custom forest dictionary.

Token evaluation counts complete strings under six pinned tokenizer profiles,
including punctuation, spacing, JSON escaping, dictionary/protocol retrieval,
symmetrically reusable Rust helpers and complete scripted transcript variants.
No single-token spelling, synthetic density ratio, static transcript or
successful worker call proves model competence or the inherited ≤10% Rust source
target. Live episodes, holdout transfer, mining, training and model negotiation
retain their independent gates.

## Rollback and migration

Close/reap the session, retain its receipt and return to the ordinary portable
module API. Existing FSM and FA1 consumers need no change. Convert accepted WK1
to explicit FA1/FSM before removing the optional source profile. Preserve custom
dictionary bytes and digest with the source. Common-word collisions when
migrating old FA1 need explicit `local NAME` escapes; blindly replacing the
header is not a lossless migration. The ordinary FA1 reader remains unchanged.
