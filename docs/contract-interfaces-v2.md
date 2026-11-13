# Contract interfaces and documentation, explicit v2 profile

`tools/agent_pipeline/contracts_v2.py` extends the existing local contract
assembler with three immutable identities. `contracts.py` and its v1 domains,
readers, APIs and identities are unchanged. V2 is an opt-in protocol, not an
interpretation change to released v1 artifacts.

## Why a second profile

V1 pins the entire contract fragment. That is conservative and remains correct,
but changing designated explanatory text also forces every dependent fragment to
change its exact pin, recursively. V2 puts explicitly non-normative documentation
in a separate field, allowing agents to update it without rewriting downstream
interfaces. It still guards an edit with the full old fragment identity, so two
agents cannot overwrite each other's documentation with the same semantic pin.

This is a structural contract primitive. It does not prove the completeness of a
consumer's declared dependencies, interpret arbitrary descriptors, validate the
implementation against an interface, authenticate an author or publish a result.
A compiler, cache or review system may use an interface ID only for an action
whose contract excludes documentation. Documentation generators and model
retrieval actions still need the exact document/artifact identity. A prose
clarification that changes a requirement belongs in `invariants` or `exports`;
it must not be disguised as a documentation-only change.

## Schema and identity contracts

A v2 fragment has exactly these fields:

```json
{
  "schema": "fluxr-contract-fragment-v2",
  "key": "tensor/layout",
  "requires": {},
  "exports": {
    "tensor/layout/Shape": {"kind": "record", "dimensions": "u64[]", "layout": "dense-row-major"}
  },
  "invariants": {"checked-size": "Reject multiplication overflow before allocation."},
  "documentation": {"summary": "Shapes carry checked dimensions."}
}
```

`requires` maps provider keys to their **interface** SHA-256 identities. Every
field in `exports` and `invariants` is normative for this structural profile,
including nested fields named `description`, `default`, `examples` or `doc`.
The assembler never guesses which of those fields are unimportant. Only the
separate `documentation` map is non-normative; its keys use the same bounded
namespace grammar as contract keys and its values must be strings.

Each digest hashes the UTF-8 domain followed by one zero byte and the existing
restricted canonical-JSON payload. Canonical JSON retains exact Unicode and
int64 values, rejects floats/duplicate keys, sorts map keys and has the unchanged
4 MiB/depth-32 bounds. Domains and payloads are:

| Identity | Domain | Payload |
| --- | --- | --- |
| Full fragment | `fluxr/contract-fragment/v2` | Every fragment field |
| Interface | `fluxr/contract-interface/v2` | Every fragment field except `documentation` |
| Documentation | `fluxr/contract-documentation/v2` | `key` and `documentation` |
| Full catalog | `fluxr/contract-catalog/v2` | Complete v2 manifest, excluding its derived `revision` field |
| Interface catalog | `fluxr/contract-interface-catalog/v2` | `schema: fluxr-contract-interface-catalog-v2`, sorted `interfaces` and export ownership |

A full manifest includes all three IDs per fragment, export ownership, and the
interface-catalog revision. Two documentation revisions can share an interface
ID; neither is an alias for the other's full artifact. Dependency pins include
other interface pins, so changing a normative provider requires explicit
transitive consumer updates in the composed candidate. All original budget
limits remain: 4,096 fragments, 256 KiB each, 16 MiB aggregate fragment bytes,
32,768 dependency edges and 32,768 exports. Manifests also obey the canonical
4 MiB bound. The catalog is a bounded full scan; no persistent incremental
compiler or distributed service is claimed.

## APIs and stale edit handling

```python
from agent_pipeline.contracts_v2 import Catalog, fragment_digest, interface_digest

provider = {
    "schema": "fluxr-contract-fragment-v2", "key": "provider", "requires": {},
    "exports": {"provider/Value": {"kind": "i32"}},
    "invariants": {"overflow": "Signed overflow traps."},
    "documentation": {"summary": "Initial explanation."},
}
consumer = {
    "schema": "fluxr-contract-fragment-v2", "key": "consumer",
    "requires": {"provider": interface_digest(provider)},
    "exports": {"consumer/Run": {"kind": "function", "result": "i32"}},
    "invariants": {}, "documentation": {},
}
base = Catalog([provider, consumer])
updated = base.fragment("provider")
updated["documentation"]["summary"] = "Clarified explanation."
candidate = base.compose([{
    "key": "provider", "expected": fragment_digest(provider), "replacement": updated,
}])
assert candidate.interface_revision == base.interface_revision
assert candidate.revision != base.revision
assert candidate.fragment("consumer") == consumer
```

`compose` retains v1's guarded add/replace/delete shape. Every guard is a **full
fragment** ID. An interface ID cannot authorize an edit. Duplicate targets,
stale guards, unknown fields, ambiguous export ownership and missing/stale pins
reject the whole candidate. All inputs are detached before validation/hash;
returned manifests and fragments are copies. Input order does not change IDs.

`base.delta(candidate)` records both exact and interface catalog identities and
separates added, removed, interface-changed, and documentation-only keys. It is a
review input, not a semantic compatibility proof or an inherited approval.

To validate bounded regular JSON files without mutating them:

```sh
PYTHONPATH=tools python3 -m agent_pipeline.contracts_v2 contracts/*.json
```

The reader rejects symlinks, nonregular inputs, duplicate JSON keys and excessive
bytes. As in v1, filesystem endpoint checks do not promise hostile-path isolation.

## Conservative migration, retention, and rollback

`migrate_v1(validated_v1_catalog)` is explicit. It keeps every existing export,
invariant and key; rewrites declared provider pins in dependency order; creates
empty documentation maps; and changes the schema/domain. It does not move any
legacy text out of interface identity. Mutually recursive fragments need an
explicit combined-fragment design; an unorderable migration refuses.

The returned `Migration` retains all original canonical fragments, provides a
v2 `catalog`, and exposes a detached receipt mapping every old full ID to the new
full/interface IDs with source/result catalog revisions. `original_fragment(key)`
returns retained original content. `restore()` reconstructs the v1 catalog and
checks its exact original revision. Input-file whitespace was never stored by
v1 Catalog, so the migration's lossless guarantee covers canonical artifact bytes
and identities. Preserve original raw files independently when their formatting
is part of a separate source identity.

Before a service switches profiles, keep v1 artifacts and the migration receipt,
review every identity change, and stage the complete v2 consumer/provider set.
Never update only a version string. Roll back by selecting the retained v1
catalog and v1 reader; new v2-only edits must remain separate history. The v2
module does not modify queues, publication authority, legacy files or evidence.

## Reproducible verification and performance scope

```sh
python3 -m unittest discover -s tests -p 'test_agent_contracts*.py' -v
python3 tools/bench_contract_interfaces.py --counts 32 256 1024 --repeats 5 --output contract-benchmark.json
```

The benchmark prepares complete validated candidates for a docs-only update in
fanout and chain dependency shapes. Its v1 fixture designates a descriptor field
as documentation; the independently authored v2 fixture places it in the explicit
map. It does not misuse automatic migration to erase normative v1 fields. All
runs check a workload-specific independent projection, exact required keys,
intended text changes and v2 interface stability. The timer includes edit
construction, hashing and full composition. It excludes initial catalog creation
and behavior checks. Python allocation peaks are measured in separate runs;
they are not resident-memory measurements. Canonical input/transport/manifest
bytes, all timing samples, code hashes and source identities are recorded.

Tests retain v1 hardcoded identities and cover unknown fields, normative nested
metadata, transitive pins, stale docs races, mutation detachment, bounded
iterators, all edit orders, collision/deletion rejection, reversible migration,
long dependency chains and the actual CLI. Complete repository suites still run
on the final combined candidate. These finite tests do not complete C08's trusted
compiler, invariant-governance, authorization, distributed recovery or production
publication acceptance gates.

## Research and inference

The official [Unison explanation](https://www.unison-lang.org/docs/the-big-idea/)
separates content identities from metadata names. It motivates distinct identity
roles; this v2 format uses FluxR's existing SHA-256 canonical contract, not
Unison's hash scheme or semantics. The [Rust compiler query guide](https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html)
shows why unchanged query results can avoid downstream invalidation and why
stable IDs matter. The inference here is narrower: explicit unchanged interface
content can avoid rewriting consumers' pins.

[JSON Schema's annotation reference](https://json-schema.org/understanding-json-schema/reference/annotations)
distinguishes validation annotations while describing other tooling uses. This
informs the conservative choice not to strip arbitrary nested descriptor fields:
a field unused by one validator may matter to a generator or agent. [Bazel's
hermeticity documentation](https://bazel.build/basics/hermeticity) reinforces the
remaining boundary: interface hashes alone do not describe every build input.
These live official pages were read on 11 November 2026; publication dates were
not supplied by the retrieved pages. No third-party implementation was vendored,
and no external performance result is presented as a FluxR measurement.
