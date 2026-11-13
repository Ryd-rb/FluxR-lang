#!/usr/bin/env python3
"""Export a new, self-contained, source-only FluxR 1 multicore challenge folder.

This packages the attributed reference port; it does not submit it or establish
challenge eligibility, conformance or a benchmark score. Source inputs must be
idle while exported. Native Rust and Cargo build scripts remain trusted code.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile

import fluxr as native

ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN = "1.99.1"

BUILD = r'''#!/usr/bin/env bash
set -euo pipefail
FLUXR_SUBMISSION_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
FLUXR_CARGO_BIN="${CARGO:-cargo}"
if [[ -z "${CARGO:-}" && -x /usr/local/cargo/bin/cargo ]]; then
  export RUSTUP_HOME=/usr/local/rustup
  export CARGO_HOME="${HOME}/.cargo"
  FLUXR_CARGO_BIN=/usr/local/cargo/bin/cargo
fi
export RUSTUP_TOOLCHAIN=1.99.1
for FLUXR_OUTPUT_DIR in bin build-logs; do
  if [[ -L "$FLUXR_SUBMISSION_DIR/$FLUXR_OUTPUT_DIR" ]]; then
    printf '%s\n' "refusing symlink output directory: $FLUXR_OUTPUT_DIR" >&2
    exit 2
  fi
done
mkdir -p -- "$FLUXR_SUBMISSION_DIR/bin" "$FLUXR_SUBMISSION_DIR/build-logs"
FLUXR_BUILD_STAGE="$(mktemp -d "$FLUXR_SUBMISSION_DIR/.fluxr-build.XXXXXXXX")"
FLUXR_BINARY_TMP=""
fluxr_cleanup() {
  if [[ -n "$FLUXR_BINARY_TMP" ]]; then rm -f -- "$FLUXR_BINARY_TMP"; fi
  rm -rf -- "$FLUXR_BUILD_STAGE"
}
trap fluxr_cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
FLUXR_LOG_DIR="$FLUXR_SUBMISSION_DIR/build-logs/$(basename -- "$FLUXR_BUILD_STAGE")"
python3 "$FLUXR_SUBMISSION_DIR/compiler.py" build \
  "$FLUXR_SUBMISSION_DIR/program/fluxr.json" "$FLUXR_BUILD_STAGE/emitted" \
  --target-dir "$FLUXR_BUILD_STAGE/target" --logs "$FLUXR_LOG_DIR" \
  --cargo "$FLUXR_CARGO_BIN" --jobs "${FLUXR_BUILD_JOBS:-2}" \
  --timeout "${FLUXR_BUILD_TIMEOUT:-600}" --online
FLUXR_BINARY_TMP="$(mktemp "$FLUXR_SUBMISSION_DIR/bin/.conduit.XXXXXXXX")"
install -m 755 -- "$FLUXR_BUILD_STAGE/target/release/conduit" "$FLUXR_BINARY_TMP"
mv -fT -- "$FLUXR_BINARY_TMP" "$FLUXR_SUBMISSION_DIR/bin/conduit"
FLUXR_BINARY_TMP=""
printf '%s\n' "built bin/conduit; retained logs: $FLUXR_LOG_DIR"
'''

START = r'''#!/usr/bin/env bash
set -euo pipefail
FLUXR_SUBMISSION_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec "$FLUXR_SUBMISSION_DIR/bin/conduit"
'''

INSTALL = r'''#!/usr/bin/env bash
set -euo pipefail
# Challenge convention: install.sh is run once as root on Ubuntu24.04.
# This installer is not executed by the exporter.
apt-get update -q
apt-get install -y -q build-essential curl ca-certificates python3
export RUSTUP_HOME=/usr/local/rustup
export CARGO_HOME=/usr/local/cargo
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal \
  --default-toolchain 1.99.1 --no-modify-path
chmod -R a+rX /usr/local/rustup /usr/local/cargo
/usr/local/cargo/bin/rustc --version
python3 --version
'''

IGNORE = "/bin/\n/build-logs/\n/.fluxr-build.*/\n__pycache__/\n*.pyc\n"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def json_bytes(value) -> bytes:
    return (json.dumps(value, indent=2, ensure_ascii=False) + "\n").encode()


def readme(provenance, structured_count, rust_bytes):
    return f'''# FluxR 1 multicore reference application

Front: direct

Language: FluxR 1. Every compiled application retains the Rust-implemented
Runtime, checked source loader, VM and live injection API. Native Rust extensions
compile directly with Rust {TOOLCHAIN}; they share the same Runtime owner.
Python 3.11+ drives the one compiler and project builder. The exported `runtime/`
contains the exact Runtime and entry macro sources, plus its pinned Cargo lock.

## Attribution and source composition

This is an attributed port of `{provenance['submission']}` from
{provenance['upstream']}, commit `{provenance['revision']}`.
The upstream MIT license is retained in `program/LICENSE.upstream`.

**{structured_count} functions use counted native extension forms.** The remainder
uses explicit attributed foreign Rust items ({rust_bytes} payload bytes).
`program/IMPORT_PROVENANCE.json` records the composition and original file hashes.
The imported epoll reactor, worker queues, prepared/pipelined PostgreSQL driver,
SIMD codecs and bcrypt optimizations belong to the upstream authors. No new
leaderboard result or independent all-FluxR server authorship is claimed.

The server keeps its original optimized release profile and hot native functions.
The Rust Runtime is initialized once before the application main, and remains
available throughout execution. Checked declarations are installed through the
same checked loader used by live injection. Compiled native extensions are bound
to exact source digests; live replacement cannot introduce uncompiled Rust.

## Build and run

On the challenge host, `install.sh` installs Rust {TOOLCHAIN}, Python and native
build tools. Review it before running as root; the exporter never executes it.
Then, as a normal user:

```bash
bash build.sh
DATABASE_URL=postgres://conduit:conduit@127.0.0.1:5432/conduit \\
JWT_SECRET=multicore-server-challenge HOST=127.0.0.1 PORT=3000 bash start.sh
```

`build.sh` validates `program/fluxr.json`, bundles the Runtime, and performs locked
release compilation. It allows the initial download of pinned Cargo dependencies;
subsequent builds reuse the user's Cargo cache. The default CLI builder also
supports offline builds once the dependencies are present. Complete compiler
logs, source hashes and build receipts remain in `build-logs/`. A fresh temporary
stage and atomic `bin/conduit` publication preserve the last successful executable
when a rebuild fails.

`start.sh` replaces its shell with the executable in the foreground. Runtime
execution requires no Python, rustc, Cargo or FluxR compiler process. Normal server
environment variables pass through unchanged. For live control, additionally set
`FLUXR_CONTROL_ADDR=127.0.0.1:0`, `FLUXR_CONTROL_TOKEN` and `FLUXR_CONTROL_FILE`;
the Runtime publishes its selected loopback address to that file. With no endpoint
configured, the same embedded Runtime and Rust host API remain available in process.

Build overrides are `CARGO`, `FLUXR_BUILD_JOBS` (default 2), and
`FLUXR_BUILD_TIMEOUT` (default 600 seconds). The Rustup toolchain remains pinned to
{TOOLCHAIN}. No prebuilt application, compiler cache or database is included.

## Reproduction and limits

`EXPORT_RECEIPT.json` records exact compiler, source, Runtime and emission hashes.
These bind bytes; real execution, injection, official conformance and matched
performance tests remain separate evidence. Foreign Rust, unsafe code, macros and
Cargo build scripts are trusted native code. Cargo build scripts are host build
tools, not FluxR application entry points; the builder derives this distinction
from Cargo's target declarations and refuses ambiguous use of a source file.

Run the official suite against fresh development/full seeds before performance
measurement. The application must use the challenge database and fixed schema,
its connection limit, real password/JWT verification, current request reads and
durable commit-before-response semantics. This exported source alone does not
establish challenge eligibility or a result exceeding the leading Rust submission.
'''


def export_submission(source: Path, destination: Path):
    source = source.resolve()
    destination = destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise native.NativeError("submission destination already exists")
    if destination.resolve().is_relative_to(source):
        raise native.NativeError("submission output must be outside source")
    manifest_path = source / "fluxr.json"
    project = native.load_project(manifest_path)
    manifest = native.read_regular(manifest_path, 1024 * 1024)
    if sha(manifest) != project.receipt["manifest_sha256"]:
        raise native.NativeError("manifest changed during export")
    provenance_raw = native.read_regular(source / "IMPORT_PROVENANCE.json")
    provenance = json.loads(provenance_raw, object_pairs_hook=native.no_duplicates)
    if not isinstance(provenance, dict) or provenance.get("mode") != "structured":
        raise native.NativeError("structured challenge import provenance required")
    for key in ("upstream", "revision", "submission"):
        if not isinstance(provenance.get(key), str) or not provenance[key]:
            raise native.NativeError("missing source attribution: " + key)
    rows = provenance.get("files")
    if not isinstance(rows, list) or len(rows) != len(project.receipt["files"]):
        raise native.NativeError("source composition inventory disagrees")
    imported = {}
    structured_count = rust_bytes = 0
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get("source_path"), str):
            raise native.NativeError("invalid import provenance entry")
        path = row["source_path"]
        if path in imported:
            raise native.NativeError("duplicate import provenance source")
        imported[path] = row
        for key in ("structured_functions", "native_rust_payload_bytes"):
            if type(row.get(key, 0)) is not int or row.get(key, 0) < 0:
                raise native.NativeError("invalid source composition count")
        structured_count += row.get("structured_functions", 0)
        rust_bytes += row.get("native_rust_payload_bytes", 0)
    payloads = {"program/fluxr.json": manifest,
                "program/IMPORT_PROVENANCE.json": provenance_raw,
                "program/LICENSE.upstream": native.read_regular(source / "LICENSE.upstream")}
    for row in project.receipt["files"]:
        relative = native.relpath(row["source"])
        if row["mode"] == "source":
            if relative.suffix != ".fluxr" or Path(row["output"]).suffix != ".rs":
                raise native.NativeError("challenge native files must be canonical .fluxr sources")
        elif row["source"] not in {"Cargo.toml", "Cargo.lock"} or row["output"] != row["source"]:
            raise native.NativeError("challenge copy inputs must be source manifests, not binary payloads")
        original = native.read_regular(native.beneath(source, relative))
        if sha(original) != row["source_sha256"]:
            raise native.NativeError("source changed during export: " + row["source"])
        if imported.get(row["source"], {}).get("source_sha256") != row["source_sha256"]:
            raise native.NativeError("source differs from import provenance: " + row["source"])
        payloads["program/" + relative.as_posix()] = original
    if set(imported) != {row["source"] for row in project.receipt["files"]}:
        raise native.NativeError("source composition paths disagree")
    if "program/Cargo.lock" not in payloads:
        raise native.NativeError("challenge export requires Cargo.lock")
    compiler = native.read_regular(Path(native.__file__))
    if sha(compiler) != project.receipt["compiler_sha256"]:
        raise native.NativeError("compiler changed during export")
    for relative, data in project.files.items():
        if relative.startswith(".fluxr-runtime/"):
            payloads["runtime/" + relative.removeprefix(".fluxr-runtime/")] = data
    payloads.update({"compiler.py": compiler, "build.sh": BUILD.encode(),
                     "start.sh": START.encode(), "install.sh": INSTALL.encode(),
                     "README.md": readme(provenance, structured_count, rust_bytes).encode(),
                     ".gitignore": IGNORE.encode(),
                     "rust-toolchain.toml": f'[toolchain]\nchannel = "{TOOLCHAIN}"\nprofile = "minimal"\n'.encode()})
    receipt = {"format": "fluxr-multicore-source-export-v2", "language_version": 1, "runtime": "unified", "rust_toolchain": TOOLCHAIN,
               "upstream": provenance["upstream"], "upstream_revision": provenance["revision"],
               "source_manifest_sha256": sha(manifest), "compiler_sha256": sha(compiler),
               "structured_functions": structured_count, "native_rust_payload_bytes": rust_bytes,
               "native_emission": project.receipt,
               "files": [{"path": path, "bytes": len(data), "sha256": sha(data)}
                         for path, data in sorted(payloads.items())],
               "scope": "Source export and byte identity only; no execution, conformance, eligibility or benchmark result."}
    payloads["EXPORT_RECEIPT.json"] = json_bytes(receipt)
    destination.parent.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix=".fluxr-submission-", dir=destination.parent))
    try:
        for relative, data in payloads.items():
            path = stage / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            path.chmod(0o755 if relative in {"build.sh", "start.sh", "install.sh"} else 0o644)
        # Like native export, this uses a trusted exclusive output directory;
        # do not intentionally overwrite a pre-existing destination.
        if destination.exists() or destination.is_symlink():
            raise native.NativeError("submission destination appeared during export")
        stage.rename(destination)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path, help="new source-only challenge folder")
    parser.add_argument("--source", type=Path, default=ROOT / "examples/multicore-server")
    args = parser.parse_args(argv)
    try:
        receipt = export_submission(args.source, args.destination)
    except (native.NativeError, OSError, ValueError) as error:
        print(f"multicore-export: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"destination": str(args.destination.absolute()),
                      "files": len(receipt["files"]) + 1,
                      "compiler_sha256": receipt["compiler_sha256"],
                      "structured_functions": receipt["structured_functions"],
                      "claim": receipt["scope"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
