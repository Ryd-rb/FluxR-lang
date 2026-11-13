# Native interoperability in FluxR 1

Native declarations share the `fluxr 1` source document and
`fluxr-project-v1` builder with checked declarations. Every compiled application
includes the Rust runtime, checked loader and live injection API. See
[the runtime guide](unified-runtime-v1.md) for checked execution and services
called from injected programs.

Native forms emit ordinary Rust. Rust checks ownership, lifetimes, borrowing,
generics, trait bounds, `Send`/`Sync` and target APIs. Native function-to-function
calls use ordinary Rust values and calling conventions. A call from checked
FluxR into a registered service crosses the typed `Value` boundary described in
the runtime guide; records and enums require explicit service ABI support.

Native arithmetic, allocation, I/O, unsafe code, threading and panic behavior
follow the chosen Rust compiler and Cargo profile. Native code and Cargo build
scripts are trusted. VM fuel cannot interrupt a native callback, bound its I/O,
or roll back its external effects. Panic recovery requires `panic = "unwind"`;
`panic = "abort"` retains process termination behavior.

## Build and run

Use Python 3.11+, Rust/Cargo 1.99.1 and native build tools. From the repository
root, choose fresh output/log directories and keep Cargo targets outside source
and emitted projects:

```sh
python3 -B tools/fluxr.py build examples/multicore-server/fluxr.json \
  ../fluxr-server-emitted --target-dir ../fluxr-server-target \
  --logs ../fluxr-server-build
```

The builder defaults to a release, locked, offline build with one job and a
finite deadline. `--online` permits retrieval of pinned Cargo dependencies;
`--profile dev` selects the development profile. `--test` runs the emitted
project's tests. The executable runs without Python, Cargo or rustc.

```sh
python3 -B tools/fluxr.py emit source.fluxr ../source.rs
python3 -B tools/fluxr.py export project.json ../emitted-project
python3 -B tools/fluxr.py wrap source.rs ../source.fluxr
python3 -B tools/fluxr.py import-native legacy.fxn ../current.fluxr
```

Emission translates syntax and binds the runtime entry; rustc still must compile
and validate the result. `export` produces a structural source tree with checked
modules marked `pending-build`. Use `build` with the original project manifest
and a fresh output directory to complete the application: it projects the merged
lock onto the active Cargo dependency graph while preserving package versions,
sources and checksums, prepares checked bindings, then invokes locked Cargo.
`tools/fluxr_native.py` delegates to this same compiler.
FN1 input starts with `fluxr-native 1` and is accepted only by `import-native`:
the importer maps `use`, `fn`, `pubfn`, `asyncfn`, and `pubasyncfn` to their
`native-` declaration names and retains native item bytes. Legacy formats do not
select a separate execution path.

## Rust API access

The native interface has no table of permitted Rust method names. `call` and
`method` emit ordinary static Rust syntax. Method resolution, specialization,
inlining, monomorphization, and ABI selection belong to rustc.

| Required facility | Native access |
| --- | --- |
| OS threads, scoped threads, joining and panic propagation | `std::thread` calls and closures |
| Shared ownership, mutexes, read/write locks, barriers, condition variables | Ordinary `Arc`, `Mutex`, `RwLock`, `Barrier`, `Condvar` values and methods |
| Bounded channels and backpressure | `std::sync::mpsc::sync_channel`, `send`, `try_send`, `recv`, disconnect results |
| Atomics and fences | Native atomic types and the caller's exact `Ordering`; no substituted lock or weakened ordering |
| TCP/UDP/Unix sockets and partial I/O | `std::net`, `std::os::unix`, `Read`, `Write`, socket methods |
| Linux reactors and socket options | Explicit Rust `extern` declarations and direct unsafe calls to epoll, eventfd, sockets and vectored I/O |
| PostgreSQL and any other Rust crate API | Direct crate APIs through the project's Cargo manifest, or a native wire-protocol implementation |
| Async futures and executors | `async`, `async-move`, `await`, native futures and chosen Rust executor; FluxR adds no executor |
| Generics, traits, lifetimes, complex patterns, attributes | Native Rust items/types, with direct FluxR expressions where supported |
| SIMD, intrinsics and assembly | Explicit Rust target features, `std::arch`, inline/global assembly and build scripts |
| Allocation/layout/unsafe pointers | Rust's exact ownership/layout rules and explicit unsafe native expressions |

Visibility, feature gates, installed crate versions, target architecture, OS
permissions, Rust stability rules, and unsafe-code obligations still apply.
"Access" does not promise that every possible crate or unsafe program has been
tested. Full syntax breadth uses explicit Rust interop segments; the prefix
forms below are a composable subset. An interop-heavy program must be described
as such, with its source composition visible.

## Source format and direct expressions

Each UTF8 source starts with the exact line `fluxr 1` followed by LF.
Whitespace separates tokens. Double-quoted tokens use JSON string escaping;
the frontend re-encodes string literals using Rust escaping. Counts make each
expression or block independently parseable. Identifiers and ordinary paths
are ASCII Rust names, with `r#` identifiers supported. Complex qualified generic
paths can be written in an explicit native expression.

For example, this function accepts a real borrowed atomic value and returns
the previous value of an actual `fetch_add`:

```text
fluxr 1
native-pubfn add 2 counter "&std::sync::atomic::AtomicUsize" amount usize usize 1
  return method path counter fetch_add 2
    path amount path std::sync::atomic::Ordering::AcqRel
```

Its generated function is equivalent to:

```rust
pub fn add(counter: &std::sync::atomic::AtomicUsize, amount: usize) -> usize {
    return counter.fetch_add(amount, std::sync::atomic::Ordering::AcqRel);
}
```

No marshaling of the atomic or borrowed pointer occurs. A Rust host calling
this function uses ordinary Rust values. Calling a nonexistent method, sending
an `Rc` between threads, escaping a scoped borrow, or violating exclusive
borrowing fails in rustc. The frontend's `emit` command alone is a syntax
translation, **not** proof that these Rust checks have passed.

### Top-level forms

| Form | Meaning |
| --- | --- |
| `native-use PATH` | Rust import |
| `native-fn NAME N PARAM TYPE... RESULT M STMT...` | Function with N parameters and M statements |
| `native-pubfn`, `native-asyncfn`, `native-pubasyncfn` | Public and/or async function variants |
| `rust N` followed immediately by newline and N UTF8 bytes | Exact native Rust item payload |

Types use one plain or quoted token. `unit` means `()`. Examples include `u64`,
`&AtomicUsize`, `"&mut [u8]"`, `"Vec<JoinHandle<()>>"`, and `"Result<(), Error>"`.
The frontend accepts bounded type fragments; rustc is authoritative for their
meaning. Arbitrary declarations, advanced types and patterns remain available
inside explicit `rust` payloads. Attributes can precede a generated function
as a native item segment.

### Expressions

| Form | Rust meaning |
| --- | --- |
| `path PATH`, `int NUMBER`, `str STRING`, `bool true`, `unit` | Name or literal |
| `call PATH N ARG...` | Function or associated-function call |
| `method RECEIVER NAME N ARG...` | Method call on the real receiver value |
| `macro PATH N ARG...` | Expression-list macro invocation |
| `ref E`, `mutref E`, `deref E`, `not E`, `neg E` | Borrow, dereference or unary operation |
| `try E`, `await E` | `?` and `.await` |
| `cast E TYPE`, `field E NAME`, `index E INDEX` | Cast, field access, indexing |
| `array N E...`, `tuple N E...` | Fixed array or tuple |
| `bin OP LEFT RIGHT` | Arithmetic/comparison/Boolean/bitwise/range operator |
| `if CONDITION YES NO` | Conditional expression |
| `block M STMT... TAIL` | Block with tail value |
| `closure N NAME... E`, `move N NAME... E` | Borrowing or move closure |
| `unsafe E`, `async E`, `async-move E` | Explicit native block |
| `rust N` newline N bytes | Parenthesized native Rust expression |

Binary names: `add sub mul div rem eq ne lt le gt ge and or bitand bitor xor shl
shr range range-inclusive`. Operands are grouped so closures and casts preserve
their intended parse. Rust defines left-to-right operand evaluation and
short-circuit behavior; The frontend does not duplicate operands or reorder calls.
Native assignment evaluates its right-hand side before its destination, as
specified by Rust; an independent side-effect test covers that distinction.
Integer tokens are decimal with optional Rust integer suffix; other literal
forms use native expressions.

### Statements

| Form | Meaning |
| --- | --- |
| `let NAME E` | Inferred immutable local |
| `let-type NAME TYPE E` | Explicit immutable local |
| `var NAME TYPE E` | Mutable local |
| `set LEFT RIGHT` | Assignment |
| `do E`, `return E`, `tail E` | Expression statement, return, or tail value |
| `for NAME ITERATOR M STMT...` | Native iterator loop |
| `while CONDITION M STMT...` | Native while loop |
| `if-stmt CONDITION M STMT... N STMT...` | Conditional statement branches |
| `break`, `continue` | Native loop control |
| `rust N` newline N bytes | Native Rust statements |

`tail` is meaningful as the final statement of a value-returning block; rustc
rejects invalid placement or result types. Native loops are not metered.
The raw payload count is **UTF8 bytes**, not characters. Its exact bytes are
preserved; a wrong count, truncated payload or split UTF8 code point refuses.
Use the `wrap` command to calculate counts rather than hand-counting large
interop segments.

## Project manifest and bounds

```json
{
  "format": "fluxr-project-v1",
  "rustflags": ["-C", "target-cpu=native"],
  "files": [
    {"source":"main.fluxr","output":"src/main.rs","mode":"source","executable":false},
    {"source":"Cargo.toml","output":"Cargo.toml","mode":"copy","executable":false},
    {"source":"Cargo.lock","output":"Cargo.lock","mode":"copy","executable":false}
  ]
}
```

Every input is explicit. Duplicate JSON keys and paths, file/directory collisions,
traversal, symlinks, invalid modes and copied application `.rs` inputs are rejected.
Declare Rust build scripts as canonical FluxR source entries. The builder
supplies the runtime dependency, source catalog and entry macro and merges the
locked dependencies. Native item digests bind runtime admission to the compiled
native bytes; changing those bytes requires a rebuild.

The frontend limits source to 8 MiB, emitted source to 16 MiB, combined project
inputs/outputs to 64 MiB, project files to 1,024, syntax nodes to 100,000, nesting
to 128 levels and each counted list to 10,000 entries. Runtime-bundled outputs
also remain within project limits. These are build admission bounds. The runtime
has separate module, source, service, frame and execution budgets.

The manifest's `rustflags` array is passed as `CARGO_ENCODED_RUSTFLAGS`, without
shell expansion. Builds still inherit the selected host toolchain and environment.
Snapshot validation and fresh output directories do not provide an arbitrary-code
sandbox or a hermetic build service.

## Multicore reference and qualification

The attributed [multicore application](../examples/multicore-server/README.md)
contains seven native prefix functions and explicit upstream Rust payloads.
Its original and current hashes and source composition remain in
`IMPORT_PROVENANCE.json`; its upstream MIT notice is in `LICENSE.upstream`.
The exact pinned reference, official harness and rules are retained under
`third_party/multicore-reference`. Keep that directory byte-exact: the qualifier
checks its full 45-file inventory against `benchmarks/multicore-reference.json`.

Use the [native qualification protocol](../tools/native_rust_tests/PROTOCOL.md)
for paired native controls, compiler rejection, PostgreSQL, compiled/live runtime
integration and performance commands. These are independent gates. The current
status is [current-status.md](current-status.md); a command being supplied does
not mean it has passed on this package or on the deployment target.

The performance comparison must include the runtime-bearing application and
measure kernel execution separately from dispatch, startup, compilation and
replacement. Count native payload, Cargo, setup and retrieved context when
evaluating the <=10% equivalent-Rust token target. Official conformance and a
compliant matched-hardware load result are required separately for a challenge
score. Local compilation, import fidelity or a microbenchmark cannot establish
that score.

## References

- [Rust expressions](https://doc.rust-lang.org/reference/expressions.html)
- [Rust operators](https://doc.rust-lang.org/reference/expressions/operator-expr.html)
- [Rust atomics](https://doc.rust-lang.org/std/sync/atomic/)
- [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)
- [Pinned challenge source](https://github.com/arjaythedev/multicore-server-challenge/tree/6145e7291a3625c184a58b63b86ed9d1f7304700)
