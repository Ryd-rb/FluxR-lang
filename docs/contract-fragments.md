# Parallel design with immutable contract fragments

`tools/agent_pipeline/contracts.py` is an opt-in local primitive for C08. Each
agent can own a separate fragment file; the complete registry is derived. There
is no shared counter or hand-maintained registry file to edit on every change.
The registry is a hash of exact fragment revisions and export ownership, so
shuffling input files cannot change it. Descriptors use the restricted canonical
JSON contract already used by the local pipeline. This is machine transport;
it is not proposed as the token-efficient surface shown to a model.

A fragment has exactly `schema`, `key`, `requires`, `exports`, and `invariants`.
`schema` is `fluxr-contract-fragment-v1`. `key` is a bounded namespace name.
`requires` maps contract keys to their complete fragment digests. `exports`
maps globally unique namespace names to nonempty descriptor objects.
`invariants` maps stable requirement names to descriptions. Descriptions and
descriptor fields are preserved and hashed, not interpreted as executable code
or proofs. Define shared mutually recursive schemas in one fragment.

```python
from agent_pipeline.contracts import Catalog, fragment_digest

leaf = {
    "schema": "fluxr-contract-fragment-v1", "key": "billing/policy",
    "requires": {},
    "exports": {"billing/Decision": {"kind": "enum", "cases": ["allow", "deny"]}},
    "invariants": {"default-deny": "Missing policy inputs must deny the action."},
}
catalog = Catalog([leaf])
revision = fragment_digest(leaf)
```

To check files and print their derived registry without writing anything:

```sh
PYTHONPATH=tools python3 -m agent_pipeline.contracts contracts/*.json
```

The API `catalog.compose(changes)` accepts guarded add, replace, and delete
operations, each with `key`, `expected`, and `replacement`. `expected` is null
for insertion; `replacement` is null for deletion. Guards compare with the
original catalog, then the complete batch is validated at once. Duplicate edit
targets, duplicate contract keys, export collisions, missing imports and stale
pins fail the whole preparation. A provider change requires consumers to update
their declared pins explicitly. Independent leaf edits can be prepared and
composed in either order. Caller mutations cannot alter retained fragments or
identities; returned manifests and descriptors are detached copies.

The limits are 4,096 fragments, 256 KiB per canonical fragment, 16 MiB aggregate
fragment bytes, and 32,768 total declared dependency edges or exports. The shared
canonical JSON encoder also enforces its 4 MiB and depth-32 protocol bounds.
Construction and composition are bounded scans of the supplied catalog; this
first slice does not claim a persistent incremental graph or remote store.

## Admission boundary

The assembler checks structural agreement on exact declared contents. It does
not establish semantic compatibility, completeness of dependency declarations,
type/effect safety, ownership authorization, oracle quality, or correctness of
an invariant. Pin changes are a review event, not permission to weaken a
requirement. Wire compiler-produced interface manifests and a separately trusted
contract-review policy into C08 before production use. Publication still needs
an authoritative compare-and-swap against the current registry and policy epoch;
an offline candidate does not publish itself. No Git branch or queue authority
is modified by this module.

Regression coverage exercises disjoint edit commutation, stale concurrent edits,
provider/consumer atomic changes, dangling deletion, namespace collision,
permutation-invariant identities, mutation detachment, and input budgets.
