# Deterministic clean source packs

Pack **25** supplies `tools/release_pack.py`, a Python standard-library builder,
validator and extractor for directly usable source ZIPs. It keeps the Cargo
project at the archive root. The implementation-pack version, package-manifest
format, FluxR language version, crate version and runtime artifact encodings
have separate versioning.

Packaging establishes source inventory, byte and permission consistency. It
does not execute the Rust implementation, authenticate the producer, complete a
release profile, synchronize repositories or publish anything. The runtime and
platform gates in the [integration guide](integration.md) remain independent.

## Create, validate and extract

Run these commands from the source root with Python 3.11 or newer. Keep the work
directory outside the source tree and use new archive and extraction paths:

```sh
FLUXR_PACK_WORK="$(mktemp -d "${TMPDIR:-/tmp}/fluxr-pack.XXXXXX")"
python3 -B tools/release_pack.py create . \
  "$FLUXR_PACK_WORK/FluxR_Agent_Implementation_Pack_v25.zip"
python3 -B tools/release_pack.py validate \
  "$FLUXR_PACK_WORK/FluxR_Agent_Implementation_Pack_v25.zip" \
  --source . --pack-version 25
python3 -B tools/release_pack.py extract \
  "$FLUXR_PACK_WORK/FluxR_Agent_Implementation_Pack_v25.zip" \
  "$FLUXR_PACK_WORK/extracted" --source . --pack-version 25
```

`create SOURCE ARCHIVE` defaults to `--pack-version 25`. An explicit version
changes only the package metadata. Creation independently reads the selected
source twice, requires identical bytes and modes, builds both archives, and
requires complete ZIP byte equality. It then validates the temporary ZIP against
the source again before publishing. No source file is rewritten. The archive's
parent must already exist outside the source tree, and the archive path must not
exist. Publication uses an exclusive hard link to a complete temporary file in
that same parent, so an existing or late-created file is never overwritten. The
output filesystem must support hard links; unsupported publication fails.

`validate ARCHIVE` checks the archive and its manifest. Adding `--source SOURCE`
also compares the exact selected source inventory, bytes, sizes and canonical
modes with that independently supplied directory. Adding `--pack-version VERSION`
requires that exact version. Supply the trusted source when validating against a
tested candidate: a self-consistent archive can contain an entirely different
source tree.

`extract ARCHIVE DESTINATION` validates before extraction. The destination must
not exist, must be outside a compared `--source` tree, and its path must have
safe, non-symlink ancestors. The source tree and destination parent must remain
quiescent throughout the operation. Extraction uses a
private staging directory, checks the actual extracted inventory and file bytes,
and verifies filesystem modes on POSIX before publishing the complete tree.
Other platforms report `filesystem_modes: "ZIP metadata only"`; they do not
claim Unix filesystem-mode verification. Failed validation or extraction
does not publish a partial destination. The same optional source and version
checks apply. The extracted source is directly buildable without replaying
patches, restoring Git metadata or retrieving an earlier pack.

## Clean payload policy

The admitted source roots are `.github/`, `src/`, `crates/`, `tools/`, `tests/`,
`docs/`, `examples/`, `benchmarks/`, `third_party/`, `web/` and `syntaxes/`.
Admitted root files are `AGENTS.md`, `Cargo.toml`, `Cargo.lock`, `LICENSE`,
`README.md`, `rust-toolchain.toml`, `requirements-dev.txt`, `.gitignore`,
`.gitattributes`, `NOTICE` and `COPYING`, plus `LICENSE`, `NOTICE` or `COPYING`
with a single dot suffix containing letters, digits, underscores or hyphens.
An unknown root file or directory fails admission unless it is an explicitly
excluded work location. New product roots require a reviewed policy update.

The tool applies the same cleanup policy to source selection and ZIP validation;
it does not use `.gitignore` as proof of cleanliness. Its named exclusion sets
in `tools/release_pack.py` define the exact current policy. Review the selected
inventory and source diff before freezing a release candidate.

Retained product material includes:

- Rust implementation, Cargo configuration and every required lockfile;
  Python tools, declared dependency pins and reusable validation suites.
- Current documentation, requirements, CI and syntax/browser support.
- Functional examples, semantic oracles, compatibility fixtures and benchmark
  inputs, including runtime checkpoint and evidence-processing behavior.
- The root `LICENSE`, `examples/multicore-server/LICENSE.upstream`,
  `third_party/multicore-reference/LICENSE` and `tools/fr1/NOTICE`.
- The frozen `tools/fr1/o200k-fr1.txt.gz` vocabulary and the exact third-party
  inventory pinned by `benchmarks/multicore-reference.json`.

The exact cleanup locations and filename patterns are:

| Location or pattern | Policy |
| --- | --- |
| Named root work areas | Omit build/output, qualification/evidence, logs, recovery/checkpoint, archived/history, session/note, prompt/dispatch, cache/toolchain, patch/bundle and previous-release areas named in `EXCLUDED_ROOTS`; also omit root `.fluxr-agent` and editor metadata. |
| Environment files at any depth | Omit any `.env` or `.env.*` path component. |
| Cache path components at any depth | Omit `.git`, `.hg`, `.svn`, `__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`, `.tox`, `.venv`, `venv`, `node_modules` and `target`. |
| Specific subtrees | Omit `web/pkg/`, `web/dist/`, `web/build/`, `web/coverage/`; `output/`, `outputs/`, `results/` and `logs/` directly under `tools/` or `tests/`; `docs/archive/`, `docs/archives/`, `docs/history/`, `docs/dispatch-prompts/`, `docs/dispatch_prompts/`, `docs/task-prompts/` and `tools/dispatch-prompts/`. |
| Generated filenames | Omit `.pyc`/`.pyo` files, `FluxR_Agent_Implementation_Pack_*.zip`, and names beginning with `dispatch-prompt`, `dispatch-prompts`, `agent-dispatch-prompt`, `agent-dispatch-prompts`, `task-prompt` or `task-prompts`, allowing underscore separators and an optional dot/underscore/hyphen suffix. Prompt and previous-pack matching is case-insensitive. |
| Named command logs | Omit `build.log`, `test.log`, `cargo-build.log` and `cargo-test.log`, except beneath a `fixtures` path component or `third_party/`. |
| Development history | Omit the private checkout's `.knowledge/` workspace and historical `docs/modernization.md` checkpoint notes. |

These are output locations and artifact patterns, not broad substring
categories. Unknown nested content is not automatically an execution record;
keep generated work in the excluded locations or outside the source. Binary and
log suffixes are not blanket exclusions, because required fixtures may use
them. Inventory review remains necessary. For example,
`src/cpu_compute/stream/stateful/checkpoint.rs`, `tests/cpu_checkpoint.rs`,
`tools/agent_pipeline/evidence.py`, `tests/test_agent_evidence.py`,
`tools/bench_agent_dispatch.py` and `docs/agent-fair-dispatch.md` are functional
product files and remain eligible. The pinned third-party inventory may contain
upstream historical-looking filenames; its exact retained bytes remain binding.
If any multicore example files are retained, their `LICENSE.upstream` is required.

The source manifest is required package metadata, not an execution receipt.
`RELEASE-MANIFEST.json` is generated for the archive and is excluded from its own
payload inventory. An extracted pack can therefore be repackaged without
recursively incorporating its old manifest.

## Paths, file types and modes

Archive entries use unique safe relative paths with `/` separators. Validation
rejects traversal and absolute paths, ambiguous or colliding names, unsupported
file types, symlinks, duplicate members and file/directory conflicts. The archive
contains regular-file entries only; parent directories are implicit.

Payload files use canonical Unix modes `0644` or `0755`; source execute bits
select the executable mode. The manifest and ZIP file attributes must agree.
Ownership, local timestamps, writable-bit variations and other host-specific
metadata do not become release metadata. Extraction restores and verifies the
declared file modes on POSIX, including `0755` implicit directories. Other
platforms validate the ZIP's Unix metadata and report that narrower result.
POSIX mode fidelity requires a filesystem that supports the declared
permissions; a mismatch fails extraction. Source or archive file entries cannot
be symlinks or special files, and set-ID/sticky source permission bits are
rejected.

## Manifest and source identity

The archive includes one canonical UTF-8 `RELEASE-MANIFEST.json` with format
`fluxr-source-release-v1` in its `schema` field. Its fields are:

| Field | Meaning |
| --- | --- |
| `schema` | Exact manifest format identifier. |
| `pack_version` | Implementation-pack version supplied to the builder. |
| `files` | Sorted payload rows containing `path`, `mode`, `size` and `sha256`. |
| `source_digest` | SHA-256 of the canonical version-and-inventory object below. |

Every `mode` is the string `0644` or `0755`; every `size` is the exact number of
file bytes. Each SHA-256 binds those bytes. The source digest binds filenames,
modes, sizes, content digests and the pack version:

```python
identity = {"pack_version": pack_version, "files": sorted_file_rows}
canonical = json.dumps(
    identity, ensure_ascii=False, sort_keys=True, separators=(",", ":")
).encode("utf-8") + b"\n"
source_digest = hashlib.sha256(canonical).hexdigest()
```

The manifest uses the same canonical JSON serialization. There is no recursive
manifest self-hash. An independently retained SHA-256 of the complete ZIP binds
the manifest and all archive bytes. Hashes provide consistency, not producer
authentication or permission to publish.

The requirements graph is also checked for its declared task/criterion counts,
unique identifiers, dependencies and qualification-scope references. Updating
payload hashes cannot make an inconsistent requirements document valid. These
checks preserve the current contract; they do not mark its acceptance criteria
complete.

## Deterministic ZIP encoding and bounded validation

Members are sorted by path and use the fixed DOS timestamp
`1980-01-01 00:00:00`, Unix regular-file attributes and standard DEFLATE level 9.
There are no directory entries, extra fields or comments. Host filesystem order,
mtimes, absolute source location and user identity do not affect archive bytes.
Complete repeated-build byte equality is required within the same Python/zlib
runtime. Different compression-library versions are not promised to produce
identical compressed bytes, and validation does not require recompressing with
the validator's local zlib version.

Validation checks the raw ZIP structure and local/central header agreement,
rejects hidden or trailing archive data, then checks payload CRCs, SHA-256 values,
sizes and exact manifest membership. Fixed metadata and manifest serialization
are validated as well as file content. Before decompressing payloads, admission
enforces these existing I22 limits:

| Bound | Maximum |
| --- | ---: |
| File members, including the manifest | 8,192 |
| Archive bytes | 64 MiB |
| Total uncompressed bytes | 256 MiB |
| Uncompressed digest manifest | 2 MiB |

These limits bound the accepted package; temporary storage and process memory
still require their own resource budget. Keep source and the extraction parent
stable while preparing and validating them. The utility is a release tool for
trusted local source preparation, not a hostile-code execution sandbox or a
transaction against concurrent adversarial filesystem writers.
It does not execute code from the archive during validation or extraction.

## Regression and integration checks

Run the dedicated suite and requirements check:

```sh
python3 -B tools/run_python_suite.py tests test_release_pack.py
python3 -B tools/check_requirements.py
```

`tools/verify_checkout.py` includes the named `release-pack` suite in its
`python`, `debug` and `full` profiles. Its checkout-cleanliness and external-output
requirements still apply. The packaging tests exercise deterministic repeated
creation, safe path/type/mode admission, tampering and digest mismatches, exact
extraction, clean payload selection, pin/license preservation and CLI behavior.
Run the normal Rust, native-helper and application tests separately against the
candidate when qualifying those capabilities. A packaging pass does not replace
any of those gates.
