> **Current interface:** WK1 is a compression/import codec. Current programs use FluxR 1 and the shared runtime; dictionary/version contracts below remain compatibility guarantees. See [Unified FluxR 1](unified-runtime-v1.md).

# WK1 common-word recipes

WK1 is an opt-in source profile that gives common short words precise, immutable
meanings made from existing checked FluxR operations. A model can write `step x
y z` for checked multiplication followed by checked addition, or `walk x n s`
for a loop that increments `x` until it reaches `n`. The VM receives the same
canonical FSM program as the explicit expansion. WK1 introduces no unchecked
execution path and changes no FA1, compact-source, FSM, FXH or FR1 reader.

The core vocabulary was authored for this increment. The dictionary mechanism
also admits bounded custom expression DAGs; it does not learn a dictionary,
train embeddings or establish that a model can use arbitrary words correctly.
Single-token behavior must be measured for each pinned tokenizer and surrounding
string. Spelling a keyword as a familiar word is a candidate-selection strategy,
not evidence that it is a single token in most models.

## Example and interfaces

```text
wk1 core v2
pub calc 3 x int y int z int int ret step x y z
```

The exact expanded expression is `add mul x y z`. The program uses normal
signed-i64 overflow checks and returns 94 for `(7, 13, 3)`. The header explicitly
selects WK1 and its frozen core dictionary, followed by the existing structured
module version. All remaining declarations and base expressions use FA1 grammar.
The host chooses the dictionary; source cannot download or replace one.
The core CLI exposes `encode-wordcode SOURCE OUTPUT` and `expand-wordcode SOURCE
OUTPUT`. Failed validation leaves an existing output untouched and creates no new
output. Custom dictionary installation uses the explicit library/worker API.

```rust
use fluxr::structured::wordcode::{self, Dictionary, ExpansionLimits};
let dictionary = Dictionary::core();
let compiled = wordcode::compile(text, &dictionary, ExpansionLimits::default())?;
let module = compiled.module;
let expanded_fa1 = compiled.expanded_agent;
let exact_subject = compiled.report;
```

`Module::compile_wordcode` is a convenience returning only the checked module.
`wordcode::compile` also provides a bounded FA1 expansion and a report binding the
input bytes, dictionary digest and canonical FSM digest. Its lexical word count
is explicitly distinct from model-token count. `admission_nodes` records the
conservative work charged during parsing, template substitution and type-obligation
copies; it is not the number of runtime instructions.

Dictionary installation and source compilation are separate phases. A host can
retain one immutable dictionary across programs. Core meanings belong to WK1;
future changes require a different profile or an explicit custom dictionary
digest. A content hash identifies bytes and semantics for comparison. It does not
authorize a candidate, verify execution or substitute for an independent oracle.

## Frozen core expression vocabulary

All arithmetic words except the two marked binary32 require signed-i64 operands.
The type check is exact: `square 1u` does not silently become an unsigned operation.
Raw binary32 bits retain the existing deterministic/explicit-profile boundaries.

| Word | Operands | Exact FA1 expression | Result |
| --- | --- | --- | --- |
| `up` | `x` | `add x 1` | int |
| `down` | `x` | `sub x 1` | int |
| `twice` | `x` | `mul x 2` | int |
| `square` | `x` | `mul x x` | int |
| `cube` | `x` | `mul mul x x x` | int |
| `step` | `x y z` | `add mul x y z` | int |
| `sum` | `x y z` | `add add x y z` | int |
| `ratio` | `x y z` | `div add x y z` | int |
| `zero` | `x` | `eq x 0` | bool |
| `positive` | `x` | `lt 0 x` | bool |
| `negative` | `x` | `lt x 0` | bool |
| `between` | `lo x hi` | `and le lo x lt x hi` | bool |
| `four` | `x` | `mul mul x 2 2` | int |
| `blend` | `x y z` | `add mul x y z` | float (binary32) |
| `root` | `x` | `sqrt abs x` | float (binary32) |

Words with a repeated hole accept only a scalar atom in that hole: a literal,
local or immutable host input. Thus `square x` is admitted but `square add x 1`
refuses. Bind the expression to a local once before using the word. This prevents
an apparently single argument from duplicating a call, allocation, bounds check,
arithmetic trap or effect. Linear holes accept ordinary expressions and preserve
their left-to-right first evaluation order. Every argument is checked against the
recipe's declared scalar type by the same compiler environment that checks normal
source. Dictionary templates cannot introduce calls, free names or host effects.

The short-circuit behavior of boolean operators, including a skipped later
operand, is exactly the existing FA1 expansion. These words are source recipes,
not eager Rust-style function calls. No algebraic reassociation, fused floating
operation or approximate numerical rewrite occurs.

A local whose name collides with a vocabulary word uses the existing explicit
`local WORD` form. FA1 retains all of its released meanings; only WK1 consults the
selected dictionary. The `wk_private_` identifier prefix is reserved in WK1 for
hygienic lowering and is rejected in authored names and expressions.

## Statement recipes and bounded execution

| Statement | Exact behavior |
| --- | --- |
| `next x` | `set x add x 1` |
| `bump x amount` | `set x add x amount` |
| `scale x factor` | `set x mul x factor` |
| `walk x bound stride` | `while lt x bound 1 set x add x stride` |
| `repeat count N statement...` | Evaluate count once into a private signed counter; while positive, decrement it and execute the N authored statements. |

`walk` requires a scalar atom for its bound and stride. Each is read at the same
point as in the expanded loop. A zero/negative stride can make a loop fail to
progress; ordinary runtime instruction limits still bound it, and arithmetic
overflow remains a trap. WK1 does not infer termination or silently skip the loop.

`repeat` adds a fresh signed-i64 local to the enclosing function. Its expansion
is an initializer followed by a normal `while`. The private counter decrements
before each body, so `continue` advances to the next iteration. `break` exits the
nearest loop, and `return` keeps its ordinary behavior. Nonpositive counts perform
zero iterations. Nested repeat statements receive distinct deterministic names;
no authored variable can access or capture them. The function's combined ordinary
and generated bindings must still fit the existing 256-binding limit.

For example:

```text
wk1 core v2
pub calc 2 n int x int int do 0 2 repeat n 1 next x ret x
```

expands to:

```text
fa1 v2
pub calc 2 n int x int int do 1 wk_private_0 int 3
set wk_private_0 n
while lt 0 wk_private_0 2
set wk_private_0 sub wk_private_0 1
set x add x 1
ret x
```

This expansion has the same instructions, cell charges, frames, traps, ordering
and ownership behavior as writing that FA1 source directly. Compilation does not
replace the checked sequence with a closed-form addition. Runtime/compiled-tier
optimizations still have to satisfy the inherited semantic and resource contracts.

## Custom typed dictionary DAGs

`Dictionary::new(&definitions)` borrows host-owned definitions. A definition has
a word, one to eight scalar parameters, a scalar result and a template. Templates
contain typed parameter indices, scalar literals, existing binary/unary operations
or `Use` references to earlier definitions. Forward references, recursion, duplicate
words, unknown fields, free identifiers and invalid types refuse. Every parameter
must appear, and first appearances follow the declared parameter order. Repeated
parameters remain subject to the atom rule at every call site.

`Dictionary::from_json` admits the corresponding bounded version-1 document;
`to_json` emits its canonical representation. JSON is an out-of-band installation
and measurement format, not a required per-program model message. Source using a
custom dictionary starts `wk1 SHA256 v2`, where SHA256 is the complete identity
returned by the host-selected dictionary. No prefix, abbreviated digest or mutable
alias is accepted. The exact setup, dictionary and pin cost must be charged when
the model receives it. Reuse can amortize setup only in an explicitly identified
scenario where the same installed dictionary persists.

Installation bounds are 64 definitions, 16 lowercase-ASCII letters per word,
eight parameters, 64 KiB canonical dictionary bytes, 8,192 raw/expanded recipe
work nodes, and depth below 32. All externally sized fields are checked before
cloning or serialization. DAG expansion charges before every node is constructed,
so small recursive-looking composition cannot become an unbounded decompression
bomb. A rejected borrowed host tree remains the caller's property; callers remain
responsible for how they construct and eventually destroy their own objects.

The installer checks inferred types and then independently submits each resolved
recipe to the released structured compiler. Source expansion additionally checks
the concrete arguments with that compiler, and the complete program is compiled
normally. This is independent type-checking machinery within one implementation,
not a proof of semantic equivalence or independent model competence.

## Limits and asynchronous tiering boundary

`ExpansionLimits` may tighten, but never raise, source and expanded-source byte
limits of 256 KiB, conservative admission work of 65,536 nodes and maximum nesting
depth 32. Zero and oversized policies refuse. Parsing, copies, generated statements
and dictionary substitutions charge before growth; the FA1 printer checks its
requested byte ceiling before each append. Full canonical FSM limits and compiler
instruction limits remain independent gates. Output is not published after a
failed pin, parse, type, budget or projection check.

WK1 itself performs no JIT compilation, starts no worker thread and replaces no
active module. Its admitted immutable FSM payload can use existing portable and
prepared/JIT paths. Asynchronous tier preparation must remain a separate host
operation with bounded admission, cancellation, isolated/killable third-party
compilation and stale-result rejection. Merely abandoning a synchronous compiler
thread would not satisfy that requirement. In-flight calls retain their selected
revision/tier; publication requires the host's exact source and configuration pin.

## Evaluation and actual qualification scope

`benchmarks/wordcode-v14.toml` is a static measurement catalog. Frozen replay admission in
`benchmarks/cases.toml` is unchanged. Four authored practical microcases have WK1,
expanded FA1, minimal checked Rust and reusable Rust-helper variants: affine
evaluation, a polynomial, repeated increment and a stride loop. Every fixture is
an executable program. Exact logical-budget and first-trap comparisons use the
canonical FA1 counterpart. Rust comparisons use matched scalar results and
arithmetic-overflow behavior on declared success/error domains; shared VM admission
machinery is a separate runtime cost. Rust loop execution is restricted to bounded
terminating fixture inputs by the measurement host.

The custom `forest` dictionary is explicitly synthetic. Four nested DAG recipes
compress 255 checked multiplications to one vocabulary word, while preserving the
expanded expression and resource cost. Its fair Rust baseline uses `checked_pow`
and its reusable-helper baseline is also retained. Comparing only with an expanded
Rust expression would overstate the gain. This stress case demonstrates bounded
semantic abstraction, not general programming ability, representative throughput,
a majority-tokenizer claim or attainment of the ≤10% equivalent-Rust target.

The tokenizer harness must encode complete source strings, JSON-escaped scenarios,
leading spaces, newlines, punctuation and adjacency; summing isolated keyword
counts is insufficient. Dictionary/protocol cost, Rust helper setup and actual
retrieval remain separately visible. No custom tokenizer is installed and no
trained-model/embedding compatibility or live-agent held-out episode is claimed.

Regression tests compare canonical FSM bytes and exact outcomes across fuel,
cells and frames; preserve arithmetic overflow/first-trap order and binary32 bits;
exercise nested loops, break/continue and private-name rejection; validate dictionary
pins, type constraints and bounded expansion; and probe malformed text. Separate
independent oracles and qualification of the final combined source remain
required. Passing these slices does not close a complete task or profile.

## Migration and rollback

Select WK1 explicitly and retain the dictionary identity in host session/source
provenance. Migrating an existing FA1 program also requires escaping locals that
collide with dictionary words using `local WORD`; blindly replacing only the
header is not a generally lossless migration. The emitted FA1 or FSM is a lossless executable escape route. A source
consumer that has not negotiated WK1 should refuse the header; a runtime that
understands the emitted FSM needs no vocabulary or tokenizer changes.

Rollback disables WK1 entry points and uses the compatible expanded
FA1/FSM. Released FA1/compact/FSM/FXH/FR1 encodings retain their meanings.
Never change the meaning of `wk1 core` in place, reinterpret old sources
with a newer dictionary, or treat a matching digest as a grant of authority.
