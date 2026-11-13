# Immutable source inputs v1

`agent_pipeline.snapshot` now prepares actual committed inputs for the existing
`fluxr-action-v1` contract. It captures an exact full Git commit into the inherited
local content store and creates a new, verified source directory for each worker.
The contract is additive: `fluxr-source-snapshot-v1` uses the explicitly named
`git-regular-linux-v1` profile. Existing arbitrary action dictionaries and action
key meanings are unchanged; the new `bind_action` API adds a checked source-binding
route without granting execution, cache acceptance or test omission authority.

## Capture and identity

Pass an exact, full lowercase commit OID. Branch names, abbreviated IDs, tags,
revision expressions and tree objects are refused. Git plumbing reads objects
from that commit without using checkout files, filters, hooks, replacement refs,
Git environment overrides or lazy network fetches. Dirty and untracked files are
intentionally absent: commit a candidate locally before capture. The trusted Git
executable and repository object database remain part of this preparation route.

The implementation verifies raw commit and every traversed raw tree object against
its Git OID, including the object type/byte-length prefix. It separately checks
every blob OID while streaming the bytes into CAS. This is necessary because
reading an object stored under a hash-shaped filename does not establish that its
contents still match that identity. SHA-1 and SHA-256 Git object formats are
supported; source snapshot identity itself is SHA-256 over a canonical manifest.
The manifest contains the versioned profile and complete sorted path, Git mode,
size and payload-digest inventory, including explicit directory entries. Commit
metadata and Git's hash algorithm are provenance, not source identity: identical
inventories produce the same snapshot even under different commit metadata.

Regular files and executable files preserve their distinct Git modes. Directories
are explicit. Symlinks, gitlinks/submodules, special files, non-UTF-8 paths,
absolute/dot/escaping paths and Git-administration components refuse before source
materialization. This profile does not quietly flatten links or fetch submodules.
Git LFS pointer blobs are ordinary bytes; no LFS download or claim of hydrated
content is made. Compatible link support requires a separately named profile.

Default execution limits are 10,000 entries, 16 MiB per payload, 128 MiB aggregate
file bytes, a 4 MiB canonical manifest, 4,096 UTF-8 path bytes, 128 path components
and a 120-second cooperative deadline. Limits validate before use; metadata bytes
are charged separately before payload reads. Git inventory output is capped at
twice the manifest budget; aggregate raw commit/tree metadata is separately capped
at twice that budget. Partial capture can leave verified unreferenced CAS objects,
but only a complete manifest receives a returned capture receipt. There is no
cache success record to publish on interruption.

The inherited content store has a 16 MiB object ceiling. Its additive
`get(maximum_bytes=...)` option preserves that default and lets snapshot readers
apply their smaller manifest/declared-file cap before reading or retaining any
payload. The file metadata is checked first; streaming then catches concurrent
growth with at most one extra detection byte, never an oversized retained chunk.
Deadlines and cancellation are cooperative: the
reader polls while waiting for Git and checks between bounded byte/entry steps.
Kernel file I/O, fsync and rename, and a single bounded store read, cannot be
preempted by this API. This is not aggregate process-resource isolation.

## Materialization and verification

A destination must be new. The API writes an exclusive private staging directory
beside it, verifies every output byte and mode, then uses Linux
`renameat2(RENAME_NOREPLACE)` for atomic no-overwrite publication of source data.
Unsupported hosts refuse this profile. A concurrent destination winner is
preserved. Missing/corrupt data, quota failures and cancellation remove the staged
copy and return no successful final path. A process crash can leave an unreferenced
private staging directory. After a rename succeeds but parent fsync fails, the
complete copy may exist without a success acknowledgement; verify it explicitly
before reuse. Neither a copy nor its receipt is a build/test acceptance record.

No CAS hard links are exposed to workers. Regular source files become mode 0444,
executables 0555 and directories 0555, with modification times normalized to the
Unix epoch. The original executable bit remains in the manifest. Read-only modes
are convenience protection under the trusted-local controller, not protection
against an owner who changes permissions or an administrator. Other filesystem
metadata such as creation/change/access times and inode numbers is not a declared
hermetic observation. Filesystem-semantic isolation remains a runner obligation.

`verify_materialization` rejects extra, missing, aliased, symlinked, writable,
mode/mtime-changed or content-changed entries. Directory-relative no-follow opens
prevent a child symlink replacement from redirecting reads outside the opened
root. Verification is a point-in-time result; it does not establish a retention
lease against trusted-controller modification, GC or filesystem failure. The
parent directories and local store must be inaccessible to untrusted actors.

## Binding actions and dependency completeness

`bind_action(store, snapshot, specification)` detaches and validates the original
v1 action, requires its exact source pin, verifies all CAS bytes, and checks each
declared input path/digest and working directory against the captured inventory.
Outputs cannot overwrite captured files/directories or descend from a source
file. Source copies are read-only, so builds use independently owned output and
temporary directories, for example an external `CARGO_TARGET_DIR` and `TMPDIR`.
The returned key retains the original v1 encoding/domain; changing any existing
v1 semantic input changes the key exactly as before.

This route retains the *whole* committed source snapshot as an input. Creating a
previously absent path, changing directory membership, a committed configuration,
generator, schema, effect declaration or discovered test changes that pin and
conservatively invalidates all bound actions. It does not establish a finer
compiler dependency closure. Generated data must be explicitly committed into a
new captured candidate or handled by a future qualified input-root composer.
Untracked generated files are never silently imported.

Toolchain/native library bytes, build scripts' external observations, environment,
external resources, network and test discovery outside the tree still need a
qualified runner and trusted dependency producer. Declaring their digests does
not prove closure or authorization. The API therefore always returns
`hermetic_execution: false` and `cache_acceptance_authorized: false`. It cannot
enable action-cache hits or selective-test omissions. Keep full validation and
arbitrary-candidate compile/run sandbox gates unchanged.

## Reproduction

Use an explicit workspace outside the source checkout. Capture a reviewed commit
from the integration repository; this API requires actual Git objects:

```sh
python3 -B tools/source_snapshot.py --store "$WORK/source-objects" \
  capture . "$EXACT_COMMIT"
python3 -B tools/source_snapshot.py --store "$WORK/source-objects" \
  materialize "$SNAPSHOT_SHA256" "$WORK/worker-source"
python3 -B tools/source_snapshot.py --store "$WORK/source-objects" \
  verify-tree "$SNAPSHOT_SHA256" "$WORK/worker-source"
```

The `capture` result reports the exact commit/tree, snapshot, counts and bytes.
Use that returned snapshot; never substitute an example digest. To execute the
already reviewed FluxR greeting example, build from the copied source with a
unique target directory tied to that canonical path, then run `hello_agent`.
The module never runs this automatically. Rehash the source copy after the build.
A second copy uses its own target directory. Compare actual command outcomes and
output bytes, and retain any native-binary mismatch instead of claiming bitwise
reproducibility from identical source alone.

`tools/bench_source_snapshot.py` records cold/warm source preparation and distinct
verified worker copies at 1, 2 and 4 processes under enforced fixed-total and
expanding CPU affinity. It retains every task and reports waiting, service time,
empirical quantiles, RSS and complete copy receipts. CAS storage, memory and I/O
remain shared; concurrent host activity is not calibrated. These are source-copy
acceptance units, not accepted coding requirements, compilation speedups or fleet
scaling proof. Defaults bound one reproducible experiment, not development staffing.

The `agent-snapshot` suite is mandatory in Linux debug/python/full verifier
profiles alongside the other required suites. The regressions use real Git commits,
corrupt object/CAS fixtures, path/link faults, cancellation, metadata/mode changes,
no-overwrite races, raw-tree provenance, final cancellation boundaries and full-source
action binding. Full-task C03/C05 acceptance remains open.

## Design references and rollback

The source-input root design follows the separation between source identity and
execution isolation in [Bazel's hermeticity documentation](https://bazel.build/basics/hermeticity),
and the explicit CAS input-root requirement in the
[Remote Execution v2 Action message](https://github.com/bazelbuild/remote-apis/blob/main/build/bazel/remote/execution/v2/remote_execution.proto).
The implementation reuses [Git ls-tree](https://git-scm.com/docs/git-ls-tree)
and [Git cat-file](https://git-scm.com/docs/git-cat-file). Atomic no-overwrite
publication reuses the documented [Linux renameat2 contract](https://man7.org/linux/man-pages/man2/rename.2.html).
These tools do not supply an execution sandbox or authenticate the producer.

Rollback removes callers of source capture/materialization while keeping the v1 action
route/full-suite validation. No queue/database or released source encoding changes.
Preserve manifests, original source objects and failed receipts needed for audit;
private staging cleanup and CAS garbage collection require trusted lifecycle policy.
