# Syntax highlighting

`syntaxes/fluxr.tmLanguage.json` supplies the `source.fluxr` TextMate grammar for
`.fluxr` files. The current `fluxr 1` context supplements the released rule,
timeline, compact-module and gate contexts. This is lexical editor support;
the compiler remains authoritative for accepted source and meaning. The grammar
does not change the language, artifact format, compiler/runtime APIs or the
checked and native testing interfaces.

## Current source

The initial `fluxr 1` header selects `meta.module.current.fluxr`. A version-like
identifier, another version number, an indented header or a header appearing
later in the document does not select that context. The compiler requires the
exact initial bytes `fluxr 1\n`; editor coloring is not evidence that the whole
document satisfies that admission rule. The TextMate line API supplies a line
ending during tokenization, so coloring cannot prove that a header at end of
file was actually followed by LF. The fixtures preserve carriage returns and
reject a CRLF header, matching the current parser's header contract.

Current checked syntax uses whitespace-delimited prefix forms. Token separators
are ASCII space, horizontal tab (`\t`), LF (`\n`), CR (`\r`), vertical tab
(`\x0b`) and form feed (`\x0c`). Unicode spaces and attached punctuation do not
split tokens. For example:

```text
fluxr 1
pub adjust 1 value float float do 0 3
  bump value 0.5f
  scale value 2f
  ret value
```

The following scope families preserve the existing FluxR naming conventions.
An editor theme chooses their visual appearance.

| Source category | Scope |
| --- | --- |
| Current document | `meta.module.current.fluxr` |
| `fluxr 1` header | `keyword.other.version.fluxr` |
| Declaration keywords | `keyword.declaration.fluxr` |
| Built-in types | `storage.type.fluxr` |
| Control forms | `keyword.control.fluxr` |
| Operations and arithmetic updates | `keyword.operator.fluxr` |
| Built-in functions | `support.function.fluxr` |
| Boolean/unit constants | `constant.language.fluxr` |
| Integer and unsigned literals | `constant.numeric.integer.fluxr` |
| Decimal and raw-bit floats | `constant.numeric.float.fluxr` |
| Native JSON-quoted tokens | `string.quoted.double.fluxr` |

Scopes approximate lexical roles without parsing counted-prefix positions:
`unit` receives a constant scope, `enum` a declaration scope and `list` a type
scope even where they have another grammatical role. Keyword spellings can also
be legal names where the parser expects a name; the grammar still applies the
corresponding lexical scope. It does not resolve declarations or infer types.

Current source has no `#` line-comment syntax. Legacy `r4` comments retain
`comment.line.number-sign.fluxr`, but that rule is not imported into the current
context. Likewise, current decimal and update patterns do not broaden the
released legacy contexts.

### Native quoted tokens

Native declarations can contain JSON-quoted tokens, including Rust type
fragments containing spaces. For example,
`native-use "std::sync::atomic::AtomicUsize"` contains a quoted import path. This does not
introduce checked string values or general quoted checked identifiers: native
token decoding and checked-source token parsing have different contracts.

Quoted tokens protect their contents from keyword and numeric scopes. The
grammar distinguishes quote delimiters and JSON escapes. Valid surrogate-pair
escapes receive `constant.character.escape.fluxr`; lone or mismatched surrogate
escapes receive `invalid.illegal.escape.fluxr`. An unfinished quoted token also
receives `invalid.illegal.unterminated-string.fluxr` through the end of its line.
Quotes are bounded by the editor line so an unfinished token does not keep the
rest of the document in a string scope. Literal newlines are not valid inside a
JSON string; `\n` is an escape within a single source line.
The native parser performs JSON decoding, enforces token boundaries and rejects
invalid strings, including invalid Unicode surrogate combinations. Coloring
does not establish successful decoding or a valid Rust type, path or value.

### Floats and arithmetic updates

Decimal float tokens have an optional minus, decimal digits, an optional decimal
point followed by digits, an optional `e`/`E` exponent with an optional sign and
digits, and a final `f`. Examples are `1f`, `0.5f`, `-0f` and `1.5e-2f`.
Forms such as `+1f`, `.5f`, `1.f` and `1e+f` do not receive a float scope.
`nanf` and `inff` remain identifier spellings. Raw-bit tokens use `0xHEXf`,
including `0x80000000f` for negative zero and encodings for infinities and NaN
payloads. Malformed or attached numeric fragments must not obtain a scope by
matching a valid substring.

The highlighted raw spelling is the documented `0xHEXf` form shared by both
frontends. The existing [runtime reader](../src/structured/agent.rs) also admits
an optional `+` after `0x`, while the [project frontend](../tools/fluxr.py) rejects
it. That runtime-reader-only spelling is outside the highlighted form; no
parser behavior changes or exact acceptance-parity claim are implied.

The grammar recognizes token shapes. It does not round decimal values to
binary32, check finite decimal overflow, enforce the decimal byte ceiling, or
establish raw-bit range validity. Those checks belong to the unchanged parser
and compiler described in [the language guide](unified-runtime-v1.md).

`bump`, `scale`, `sub`, `div`, `rem`, `next` and `prev` receive the arithmetic
operator scope, consistent with their expanded checked operations. Matching is
by complete token; names containing these words are not operators. Coloring
cannot determine whether an update target exists, has the right type, or is in
a valid statement position. In particular, `sub`, `div` and `rem` also occur in
ordinary expressions, while `next`/`prev` retain their signed unit-step
semantics.

## Native byte-span limitation

Native `rust N` forms are followed immediately by LF or CRLF and then exactly
`N` UTF-8 **bytes** of raw Rust. The payload may contain newlines, strings,
braces, FluxR-looking tokens and multibyte characters. Its end may be any valid
UTF-8 character boundary, including the middle of an editor line. The form can
occur as a top-level item or inside native expressions and statements.

TextMate applies regular expressions one editor line at a time. A captured
decimal byte count cannot drive an exact dynamic byte-length match across
subsequent lines. Brace matching, indentation, a following keyword or an
end-of-file region would not reproduce the native parser's boundary contract.
The grammar therefore does not claim an embedded Rust region or an exact native
payload span. It continues best-effort lexical coloring in the current context.
Rust payload text can consequently receive FluxR scopes, and a quote inside a
payload can affect apparent tokenization through the remainder of its line,
including checked text after the actual payload end.

Do not use highlighting as a native-span validator, a source extraction tool,
or a trust boundary. Use the compiler/frontend's byte-span handling for exact
payloads; see [native interoperability](native-rust-v1.md). The highlighting
fixtures retain this limitation explicitly rather than asserting false Rust
embedding or semantic rejection.

## Execute the scope fixtures

Use Node.js and install the exact test dependencies into a separate work
directory. From the source root:

```sh
FLUXR_HIGHLIGHT_WORK="$(mktemp -d "${TMPDIR:-/tmp}/fluxr-highlighting.XXXXXX")"
npm install --prefix "$FLUXR_HIGHLIGHT_WORK" --no-audit --no-fund --ignore-scripts \
  --save-exact vscode-textmate@9.2.0 vscode-oniguruma@2.0.1
node tests/highlighting.cjs "$FLUXR_HIGHLIGHT_WORK"
```

The harness loads the actual JSON grammar through `vscode-textmate`, executes
`vscode-oniguruma`'s WebAssembly engine and carries the TextMate state between
lines. Positive fixtures check the emitted scopes; negative fixtures check that
malformed token shapes and misleading substrings do not acquire those scopes.
Released example fixtures retain their original scope expectations. These
checks do not substitute JavaScript regular expressions for Oniguruma or replace
the Rust/Python behavior and compatibility gates in the
[integration guide](integration.md).

This editor-support slice does not complete the planned language-server,
authoring-protocol or platform qualification scopes in
[requirements.json](requirements.json).
