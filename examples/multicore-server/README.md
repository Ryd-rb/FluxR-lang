# FluxR 1 multicore reference application

Front: direct

This is an attributed port of `rust-epoll-theCatWisel` at upstream revision
`6145e7291a3625c184a58b63b86ed9d1f7304700`. Seven functions use native FluxR
prefix forms; the remaining Rust payload is explicit native interoperability.
The upstream implementation and optimization design belong to their authors.
`LICENSE.upstream` retains their MIT notice and `IMPORT_PROVENANCE.json` binds
each current source file to the original source and records its composition.

The compiled application contains the Rust FluxR runtime, checked loader, VM
and live injection API. Native item digests bind checked source admission to
the actual compiled native code. Updating native Rust requires rebuilding;
compatible checked declarations can be installed into the running process.

## Build and run

From the repository root, with Python 3.11+, Rust/Cargo 1.99.1 and native build
tools installed:

```sh
python3 -B tools/fluxr.py build examples/multicore-server/fluxr.json \
  ../fluxr-server-emitted --target-dir ../fluxr-server-target \
  --logs ../fluxr-server-build
DATABASE_URL=postgres://conduit:conduit@127.0.0.1:5432/conduit \
JWT_SECRET=multicore-server-challenge HOST=127.0.0.1 PORT=3000 \
  ../fluxr-server-target/release/conduit
```

The database URL and secret above are challenge fixture settings. Prepare a
separate PostgreSQL 18 database with the pinned challenge schema before running
the server. The builder is locked and offline by default; add `--online` for an
initial download of pinned runtime dependencies. Output and log directories must
be new; Cargo targets remain outside source and emitted trees.

## Standalone source export

```sh
python3 -B tools/export_multicore_submission.py ../fluxr-multicore-export
bash ../fluxr-multicore-export/build.sh
```

The export includes canonical `.fluxr` sources, the exact compiler and runtime,
pinned Cargo inputs, licenses, build/start/install scripts and usage instructions.
The exporter does not run the installer. The build script permits the initial
download of locked dependencies, retains its build logs and atomically publishes
the executable. Each new build binds the current source hashes. Keep attribution
and update the source composition description when editing the exported program.

## Qualification

The exact official source/harness is `third_party/multicore-reference` at the
repository root. `tools/qualify_multicore.py` verifies its hashes, creates a
private PostgreSQL cluster, and runs the official 197-step suite. Supply a
previously built server and fresh work/output paths:

```sh
PG_BIN=/usr/lib/postgresql/18/bin PG_SHARE=/usr/share/postgresql/18 \
python3 -B tools/qualify_multicore.py \
  --reference third_party/multicore-reference \
  --server fluxr=../fluxr-server-target/release/conduit --runtime-server fluxr \
  --work ../fluxr-conformance-work --output ../fluxr-conformance-result
```

Go, PostgreSQL 18 and its command-line tools must be installed. Run as a user
permitted to create and stop a private cluster; PostgreSQL does not run as root.
The command uses the full seed. Repeat with `--dev` and different work/output
paths for the separate development seed. Add a distinct `--server LABEL=PATH`
for a matched direct Rust control; mark each runtime-bearing binary with its
own `--runtime-server LABEL`. The runtime marker requires live injection while
the server handles real HTTP traffic.

The application keeps the reference release optimization, LTO, codegen-unit,
panic and CPU settings, while its required runtime adds dependencies and startup
work. Native callback panic recovery therefore depends on the selected panic
profile; the challenge application's abort profile terminates on panic.

Challenge rules require the fixed schema, at most 95 database connections,
committed writes before replies, per-request database reads, bcrypt cost 10 on
login and HS256/expiry validation. Pools and prepared queries are allowed;
cross-request data, password or JWT-result caches are prohibited by the pinned
rules. Conformance does not establish a load score. The full workload and
matching hardware, warm-up/probes and measured hold remain separate gates.

See [native interoperability](../../docs/native-rust-v1.md),
[qualification protocol](../../tools/native_rust_tests/PROTOCOL.md),
[pinned rules](../../third_party/multicore-reference/SPEC.md) and
[current status](../../docs/current-status.md).
