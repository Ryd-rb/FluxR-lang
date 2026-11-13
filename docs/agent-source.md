# FA1 agent source projection

FA1 is an **opt-in, versioned, delimiter-free authoring projection** for the existing
structured FluxR language. It supports every current `Expr`, `Stmt`, type and
valid declaration form. It compiles into the same canonical FSM bytes and uses
the same type checker, execution engine, traps, effects, budgets and collaboration
identities. It is not a new runtime or released-artifact format. Compact source,
FSM and FR1 remain available. Select a source representation by measured token
cost and successful editing performance for the chosen model.

```text
fa1 v2
pub calc 2 x int y int int ret rem add mul x y 17 997
```

This has two parameters, a signed integer result, no locals, and one return.
The equivalent compact source is:

```text
v2
(pub calc (x:i64 y:i64) i64 () (ret (% (+ (* x y) 17) 997)))
```

## Interfaces

```rust
let source = fluxr::structured::Source::parse_agent(text)?;
let module = fluxr::structured::Module::compile_agent(text)?;
let text = module.agent_text()?;
let body = fluxr::structured::Stmt::parse_agent_body("fa1 body 1 ret add x 1")?;
```

`parse_agent` constructs a bounded AST; it does not assert semantic validity.
`compile_agent` runs the normal checker. `agent_text` only renders a checked
module, bounds output before append, reparses it, and compares the AST and exact
canonical bytes. It can refuse a projection that exceeds FA1's text budget.
`parse_agent_body` supplies a `Vec<Stmt>` for the existing `Edit::ReplaceBody` API;
expected revisions, read dependencies, target authority and whole-change
validation are still required. FA1 introduces no alternate publication route.

```sh
cargo run --locked --bin fluxr-cli -- project-agent input.flux output.fa1
cargo run --locked --bin fluxr-cli -- encode-agent output.fa1 output.fsm
```

`project-agent` accepts checked compact source, a released FSM artifact or legacy
structured JSON. `encode-agent` explicitly selects FA1. Readers do not guess a
source dialect from arbitrary prefixes. CLI input is bounded before full
allocation; failed validation precedes output creation.

## Grammar

All tokens are separated by ASCII whitespace. Newlines have no syntactic meaning.
There are no parentheses, brackets, braces, colons, commas, periods, semicolons,
indentation rules or automatic terminators. Identifier underscores, negative
literal signs and the numeric `0x`/`u`/`f` spelling are lexical content. Identifiers
follow `[A-Za-z_][A-Za-z0-9_]{0,63}`.

In this documentation, `N item...` means exactly N consecutive items. The count
is an ordinary canonical decimal integer, not model-token length or byte length.
The parser consumes the count and checks its permitted bound before reading the
items. It does not reserve a vector based solely on an untrusted count.

| Form | Grammar |
| --- | --- |
| Module | `fa1 v1` or `fa1 v2`, followed by declarations to end of input |
| Inputs | `inputs N binding...` |
| Record | `rec name N binding...` |
| Enum | `enum name N variant-name payload-type-or-none...` |
| Private function | `fn name N binding... result function-body` |
| Export | `pub name N binding... result function-body` |
| Method | `method receiver-type name N binding... result function-body` |
| Binding | `name type` |
| Single-return function body | `ret expression` |
| General function body | `do N local-binding... N statement...` |
| Independent edit body | `fa1 body N statement...` |

Types are `unit`, `bool`, `int` (i64), `uint` (u64), `float` (binary32),
`record name`, `enum name`, or `list type`. The `none` word only represents an
absent enum payload; a payload of type `unit` is distinct. Methods include their
receiver parameter in the parameter count, as in the existing AST. Exported
methods are already disallowed by the existing checker.

| Statement | Grammar |
| --- | --- |
| Assignment | `set local expression` |
| Field assignment | `put local field expression` |
| List push/pop | `push local expression`, `pop local` |
| Conditional | `if condition N then-statement... N else-statement...` |
| Loop | `while condition N statement...` |
| Loop exit | `break`, `continue` |
| Return | `ret expression` |

| Expression | Grammar |
| --- | --- |
| Literals | `unit`, `true`, `false`, signed decimal i64, decimal u64 followed by `u`, exact binary32 bits `0xHEXf` |
| Local | `name`, or `local name` when the name is an expression keyword |
| Input | `input name` |
| Binary | `operator left right`: `add sub mul div rem eq lt le and or` |
| Unary | `operator operand`: `not abs tof i64tof ftoi64 sqrt` |
| Record construction | `new record-name N expression...` |
| Field read | `field receiver field-name` |
| Enum construction | `tag enum-name variant-name`, `wrap enum-name variant-name payload` |
| Enum inspection | `is value variant-name`, `payload value variant-name` |
| List construction | `list element-type N expression...` |
| List operations | `empty list`, `last list`, `at list index`, `minidx list` |
| Function call | `call name N argument...` |
| Method call | `m receiver name N argument...` |

A prefix expression always determines how many children follow. Record/call/list
arity is explicit, so parsing does not depend on a concurrently edited function
signature. Counts for blocks and variadic forms are bounded parser attributes;
FA1 does not ship a pure CFG decoder or claim that a generic CFG tool enforces
these count constraints. A future decoder must track count obligations and the
typed environment in addition to lexical grammar state.

## Bounds and semantic preservation

The source/output bound is 256 KiB. Parser limits are 65,536 cumulative nodes,
96 parser nesting levels, 32 type levels, 256 bindings/variants per declaration,
512 functions, 512 total records/enums, and 32,768 items per block or variadic
expression. The existing structured checker can impose tighter semantic limits.
These are implementation resource guards, not a relaxation of existing budgets.

There is no arithmetic reassociation, implicit coercion, speculative evaluation
or macro optimization. Prefix order preserves operand order, short-circuit
conditions, receiver-before-argument evaluation, trap order, instruction fuel,
copy-on-write identity, and exact float literal bits. Counts and declaration order
survive the projection. Binary32 values print as bits so NaN payloads and signed
zero do not depend on decimal formatting.

## Measurement and admission

```sh
cargo test --locked --test agent_source
cargo test --locked --lib structured::agent
cargo build --locked --release --bin fluxr-cli
python3 tools/bench_agent_syntax.py --output ../fa1-token-report.json
python3 tools/audit_agent_vocabulary.py --output ../fa1-vocabulary-report.json
python3 tools/bench_tokens.py --manifest benchmarks/agent-syntax.toml
cargo run --locked --release --example bench_agent_source
```

The token harness verifies byte-identical FSM artifacts for all 14 corpus cases
before counting authored compact source, canonical compact source and FA1 on the
six pinned tokenizers in `benchmarks/token-panel.toml`. It reports exact bytes,
source/artifact hashes, per-case wins and losses, and illustrative syntax-card
amortization. It does not silently replace authored compact programs with a larger
canonical rendering to inflate a saving. It leaves the source corpus unchanged.
The vocabulary audit measures the existing FR1 65,536-codeword alphabet across
the same panel. A single token carrying a bit chunk is a transport property;
semantic understanding by a pretrained model is a different, unmeasured property.

A punctuation character is not invariably one model token: adjacent punctuation,
whitespace and keywords can merge. A long keyword is not invariably one token
either. This implementation deliberately leaves default authoring policy open:
benchmark complete task context, generated edits, failed attempts and repairs,
semantic success, reviewer effort, and accepted work per elapsed second before
promoting a source format. A valid parse is only the first admission gate.

The FA1 fixture and source-pair catalog is `benchmarks/agent-syntax.toml`. The
legacy in-process replay pins the entire `benchmarks/cases.toml` file as reviewed
executable evidence, so that file remains byte-for-byte unchanged. The separate
FA1 catalog is usable for source counting and artifact equality checks; it does
not add any case to the executable replay allowlist or authorize generated Rust.
