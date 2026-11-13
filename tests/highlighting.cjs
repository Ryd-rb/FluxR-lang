const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { createRequire } = require("node:module");

const root = path.resolve(__dirname, "..");
const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
const grammarPath = "syntaxes/fluxr.tmLanguage.json";
const load = createRequire(path.resolve(process.argv[2] || "target/linguist-qa", "package.json"));
const textmate = load("vscode-textmate");
const oniguruma = load("vscode-oniguruma");

async function main() {
  await oniguruma.loadWASM(fs.readFileSync(load.resolve("vscode-oniguruma/release/onig.wasm")));
  const raw = textmate.parseRawGrammar(read(grammarPath), grammarPath);
  assert.deepEqual(raw.fileTypes, ["fluxr"]);
  assert.equal(raw.scopeName, "source.fluxr");
  const registry = new textmate.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (sources) => new oniguruma.OnigScanner(sources),
      createOnigString: (value) => new oniguruma.OnigString(value),
    }),
    loadGrammar: async (scope) => (scope === raw.scopeName ? raw : null),
  });
  const grammar = await registry.loadGrammar(raw.scopeName);

  function tokens(source) {
    let stack = textmate.INITIAL;
    // Preserve CR bytes: the current reader requires its exact LF-only header,
    // while the released grammars intentionally accept their existing spacing.
    return source.split("\n").flatMap((line, lineNumber) => {
      const result = grammar.tokenizeLine(line, stack);
      stack = result.ruleStack;
      return result.tokens.map(({ startIndex, endIndex, scopes }) => ({
        text: line.slice(startIndex, endIndex),
        lineNumber,
        startIndex,
        endIndex,
        scopes,
      }));
    });
  }

  function scoped(result, token, scope) {
    assert.ok(
      result.some(({ text, scopes }) => text === token && scopes.includes(scope)),
      JSON.stringify({ token, scope }),
    );
  }

  function sample(file, context, expected) {
    const result = tokens(read(file));
    for (const [token, scope] of expected) {
      scoped(result, token, context);
      scoped(result, token, scope);
    }
  }

  function without(result, ...prefixes) {
    const unexpected = result.filter(({ scopes }) =>
      scopes.some((scope) => prefixes.some((prefix) => scope.startsWith(prefix))),
    );
    assert.deepEqual(unexpected, [], JSON.stringify({ forbiddenScopes: prefixes }));
  }

  // Check every character in a span. An exact-token search alone can miss a
  // wrongly scoped substring or pass by finding a different occurrence.
  function spanScoped(result, lineNumber, start, end, scope) {
    for (let position = start; position < end; position += 1) {
      assert.ok(
        result.some((token) => token.lineNumber === lineNumber
          && token.startIndex <= position && position < token.endIndex
          && token.scopes.includes(scope)),
        JSON.stringify({ lineNumber, position, scope }),
      );
    }
  }

  function spanWithout(result, lineNumber, start, end, ...prefixes) {
    without(result.filter((token) => token.lineNumber === lineNumber
      && token.startIndex < end && token.endIndex > start), ...prefixes);
  }

  const currentScope = "meta.module.current.fluxr";
  const current = (source) => tokens(`fluxr 1\n${source}`);
  const body = (result) => result.filter(({ lineNumber }) => lineNumber > 0);

  sample("web/well.fluxr", "meta.timeline.fluxr", [
    ["v1", "keyword.other.version.fluxr"],
    ["6600", "constant.numeric.integer.fluxr"],
    ["2", "constant.numeric.curve.fluxr"],
    [":", "punctuation.separator.key-value.fluxr"],
    ["-1120", "constant.numeric.integer.fluxr"],
  ]);
  sample("examples/fan-speed.fluxr", "meta.rule.fluxr", [
    ["r4", "keyword.other.version.fluxr"],
    ["# Enabled flag, then temperature as binary32 bits.", "comment.line.number-sign.fluxr"],
    ["i1", "variable.parameter.fluxr"],
    ["guard", "keyword.control.fluxr"],
    ["f32:75.0", "constant.numeric.float.fluxr"],
    ["f<", "keyword.operator.fluxr"],
    ["f*", "keyword.operator.fluxr"],
    ["?", "keyword.control.fluxr"],
  ]);
  sample("examples/structured-counter.fluxr", "meta.module.compact.fluxr", [
    ["v1", "keyword.other.version.fluxr"],
    ["fn", "keyword.declaration.fluxr"],
    ["advance", "entity.name.fluxr"],
    ["i64", "storage.type.fluxr"],
    ["$bonus", "variable.other.input.fluxr"],
    ["while", "keyword.control.fluxr"],
    ["==", "keyword.operator.fluxr"],
    [".", "punctuation.accessor.fluxr"],
  ]);
  sample("examples/route-gate.fluxr", "meta.gate.fluxr", [
    ["&", "keyword.operator.fluxr"],
    ["!", "keyword.operator.fluxr"],
    ["e", "variable.parameter.fluxr"],
  ]);

  const compact = tokens("\r\n v2(inputs levels:[[f32]] result:#Choice)\r\n(ret (list f32 0x80000000f -1.5e-2f 2f))\r\n(ret 3u)");
  scoped(compact, "#Choice", "entity.name.type.fluxr");
  scoped(compact, "f32", "storage.type.fluxr");
  for (const literal of ["0x80000000f", "-1.5e-2f", "2f"]) {
    scoped(compact, literal, "constant.numeric.float.fluxr");
  }
  scoped(compact, "3u", "constant.numeric.integer.fluxr");
  assert.ok(compact.every(({ scopes }) => !scopes.includes("comment.line.number-sign.fluxr")));
  const tail = tokens("v2 (pub pick (b:bool) i64 = (? b 1 0))");
  scoped(tail, "=", "keyword.operator.fluxr");
  scoped(tail, "?", "keyword.control.fluxr");
  const traversal = tokens("v2 (pub first (xs:[i64] n:u64) u64 (i:u64) (scan i n ((when (< (at xs i) 0) (break)))) (ret i))");
  scoped(traversal, "scan", "keyword.control.fluxr");
  scoped(traversal, "at", "support.function.fluxr");
  const projection = tokens("v2 (pub project (x:i64) i64 = (ftoi64 (sqrt (i64tof x))))");
  for (const op of ["i64tof", "ftoi64", "sqrt"]) {
    scoped(projection, op, "support.function.fluxr");
  }
  const earlier = tokens("\nr3 2\ni0 f32:+.5 f<= # no comment in v3");
  scoped(earlier, "f32:+.5", "constant.numeric.float.fluxr");
  scoped(earlier, "f<=", "keyword.operator.fluxr");
  assert.ok(earlier.every(({ scopes }) => !scopes.includes("comment.line.number-sign.fluxr")));
  scoped(tokens("r4 1 i0#comment"), "#comment", "comment.line.number-sign.fluxr");
  scoped(tokens("|abc"), "abc", "variable.parameter.fluxr");
  scoped(tokens("a"), "a", "variable.parameter.fluxr");

  // Keep the current language separate from every released reader. In
  // particular, legacy compact decimals have a deliberately different grammar.
  for (const literal of [".5f", "1.f", "-inff", "-NaNf"]) {
    scoped(tokens(`v2 (ret ${literal})`), literal, "constant.numeric.float.fluxr");
  }
  for (const legacy of ["r4 1\ni0 1 +", "r3 1\ni0 1 +", "v1 1\n0 0 0 0:0", "v2 (ret 1)", "&ab", "(and a b)", "a"]) {
    without(tokens(legacy), currentScope);
  }
  for (const legacy of ["r4 1", "v1 1", "v2 (ret 1)", "(and a b)"]) {
    without(tokens(`${legacy}\nfluxr 1\npub f 0 float ret 0.5f`), currentScope);
  }

  sample("examples/unified/hello.fluxr", currentScope, [
    ["fluxr 1", "keyword.other.version.fluxr"],
    ["pub", "keyword.declaration.fluxr"],
    ["int", "storage.type.fluxr"],
    ["ret", "keyword.control.fluxr"],
    ["add", "keyword.operator.fluxr"],
    ["40", "constant.numeric.integer.fluxr"],
  ]);
  for (const source of [
    "fluxr 2\npub run 0 float ret 0.5f",
    "fluxr 01\npub run 0 float ret 0.5f",
    "fluxr +1\npub run 0 float ret 0.5f",
    "fluxr 1x\npub run 0 float ret 0.5f",
    "xfluxr 1\npub run 0 float ret 0.5f",
    "FluxR 1\npub run 0 float ret 0.5f",
    " fluxr 1\npub run 0 float ret 0.5f",
    "\tfluxr 1\npub run 0 float ret 0.5f",
    "fluxr 1 \npub run 0 float ret 0.5f",
    "fluxr 1\t\npub run 0 float ret 0.5f",
    "fluxr\t1\npub run 0 float ret 0.5f",
    "fluxr  1\npub run 0 float ret 0.5f",
    "fluxr 1 # comment\npub run 0 float ret 0.5f",
    "fluxr 1\r\npub run 0 float ret 0.5f",
    "\nfluxr 1\npub run 0 float ret 0.5f",
    "unknown\nfluxr 1\npub run 0 float ret 0.5f",
    "\uFEFFfluxr 1\npub run 0 float ret 0.5f",
    "fluxr\u00a01\npub run 0 float ret 0.5f",
  ]) {
    without(tokens(source), currentScope, "constant.numeric.float.fluxr", "string.quoted.double.fluxr");
  }

  const declarations = current([
    "inputs 1 enabled bool",
    "rec Pair 2 left int right uint",
    "enum Choice 2 Empty none Value int",
    "fn internal 0 int ret 0",
    "pub run 1 x float float do 0 1 ret x",
    "method Pair sum 0 int ret 1",
    "native-use std::thread",
    "native-fn helper 0 unit 0",
    "native-pubfn helper2 0 unit 0",
    "native-asyncfn helper3 0 unit 0",
    "native-pubasyncfn helper4 0 unit 0",
    "rust 0",
    "",
  ].join("\n"));
  for (const word of ["inputs", "rec", "enum", "fn", "pub", "method", "native-use", "native-fn", "native-pubfn", "native-asyncfn", "native-pubasyncfn", "rust"]) {
    scoped(declarations, word, "keyword.declaration.fluxr");
  }
  for (const word of ["bool", "int", "uint", "float"]) {
    scoped(declarations, word, "storage.type.fluxr");
  }
  for (const word of ["record", "list"]) {
    scoped(current(word), word, "storage.type.fluxr");
  }
  for (const word of ["do", "ret", "if", "while", "break", "continue", "for", "return", "tail", "if-stmt"]) {
    scoped(current(word), word, "keyword.control.fluxr");
  }
  for (const word of ["true", "false", "unit", "none"]) {
    scoped(current(word), word, "constant.language.fluxr");
  }
  for (const word of ["set", "put", "push", "pop", "local", "input", "new", "call", "field", "at", "is", "payload", "tag", "wrap", "m", "host", "abs", "tof", "i64tof", "ftoi64", "sqrt", "empty", "last", "minidx"]) {
    scoped(current(word), word, "support.function.fluxr");
  }

  const updates = current("pub update 1 x int int do 0 8 bump x 2 scale x 3 sub x 4 div x 2 rem x 3 next x prev x ret x");
  for (const word of ["bump", "scale", "sub", "div", "rem", "next", "prev"]) {
    scoped(updates, word, "keyword.operator.fluxr");
  }
  for (const word of ["add", "sub", "mul", "div", "rem", "eq", "lt", "le", "and", "or", "not"]) {
    scoped(current(`ret ${word} x 2`), word, "keyword.operator.fluxr");
  }
  // Keyword-shaped local names remain a compiler concern; only whole tokens
  // receive keyword scopes. Punctuation and Unicode spaces are not delimiters.
  for (const word of ["bumped", "scalex", "submarine", "divide", "remainder", "next_value", "preview", "xnext", "pubfn", "integer", "native-fn-extra", "fn-name", "(next)", "next,", "next#tail", "#next", "next\u00a0x", "x\u00a0next"]) {
    without(body(current(word)), "keyword.", "support.function.", "storage.type.", "constant.numeric.");
  }
  for (const separator of [" ", "\t", "\r", "\v", "\f", "\n"]) {
    const separated = current(`next${separator}x${separator}prev${separator}x`);
    scoped(separated, "next", "keyword.operator.fluxr");
    scoped(separated, "prev", "keyword.operator.fluxr");
  }
  without(current("# not a current-source comment\nret 0.5f"), "comment.");
  without(current("v2 (ret .5f)\nr4 1 # legacy-looking text\n&ab"), "meta.module.compact.fluxr", "meta.rule.fluxr", "meta.gate.fluxr", "comment.");

  for (const literal of [
    "0f", "1f", "-0f", "-1f", "0.5f", "-0.000f", "0001.0E+00f",
    "1e-8f", "1E+8f", "-1.5e-2f", "1e-100f", "3.4028235e38f",
    "1.000000059604644775390626f", "1.17549435e-38f",
    "0x0f", "0xff", "0x3f800000f", "0x80000000f", "0x7f800000f",
    "0x7fc00001f", "0xFFFFFFFFf", "0x0000000000000000f",
  ]) {
    scoped(current(`ret ${literal}`), literal, "constant.numeric.float.fluxr");
  }
  for (const literal of ["0", "-0", "42", "-42", "0001", "0u", "42u", "18446744073709551615u"]) {
    scoped(current(`ret ${literal}`), literal, "constant.numeric.integer.fluxr");
  }
  for (const literal of [
    ".5f", "-.5f", "1.f", "+1f", "1_0f", "1ef", "1e+f", "1e-f",
    "1.2.3f", "1e1.5f", "--1f", "-inff", "-NaNf", "1ff", "1F",
    "nanf", "inff", "NaNf", "Infinityf", "f", "0xf", "0xzzf", "0X1f",
    "-0x1f", "0x1F", "0x1_fg", "x1f", "1fx", "1f.foo", "1f#tail",
    "#1f", "(1f)", "1f,", "1f\u00a0x", "x\u00a01f", "1\u00a0f",
    "+1", "42ux", "x42u", "42u#tail", "(42)", "1.2", "0x1",
  ]) {
    without(body(current(literal)), "constant.numeric.");
  }
  for (const separator of [" ", "\t", "\r", "\v", "\f", "\n"]) {
    const separated = current(`0.5f${separator}0x80000000f${separator}3u`);
    scoped(separated, "0.5f", "constant.numeric.float.fluxr");
    scoped(separated, "0x80000000f", "constant.numeric.float.fluxr");
    scoped(separated, "3u", "constant.numeric.integer.fluxr");
  }
  // The runtime's historical raw-bit decoder permits an optional plus after
  // 0x; the project frontend does not. Highlight their documented shared HEX
  // spelling without changing either reader's existing admission behavior.
  without(body(current("0x+1f")), "constant.numeric.");
  // TextMate is lexical, not a binary32 evaluator or a source-budget checker.
  // These have valid decimal spelling but the checked compiler rejects them.
  for (const literal of ["1e9999f", `0.${"0".repeat(254)}f`]) {
    scoped(current(literal), literal, "constant.numeric.float.fluxr");
  }

  const quotedLine = String.raw`native-use "pub next ret 0.5f 0x80000000f # text \" \\ \/ \b \f \n \r \t \u0041"`;
  const quoted = current(quotedLine);
  const quoteStart = quotedLine.indexOf('"');
  spanScoped(quoted, 1, quoteStart, quotedLine.length, "string.quoted.double.fluxr");
  spanWithout(quoted, 1, quoteStart, quotedLine.length, "keyword.", "constant.numeric.", "comment.", "storage.type.", "support.function.");
  scoped(quoted, '"', "punctuation.definition.string.begin.fluxr");
  scoped(quoted, '"', "punctuation.definition.string.end.fluxr");
  for (const escape of [String.raw`\"`, String.raw`\\`, String.raw`\/`, String.raw`\b`, String.raw`\f`, String.raw`\n`, String.raw`\r`, String.raw`\t`, String.raw`\u0041`]) {
    scoped(quoted, escape, "constant.character.escape.fluxr");
  }
  without(quoted, "invalid.");
  for (const quotedToken of ['""', '"std::thread"', '"café 🦀"', String.raw`"\uD83E\uDD80"`]) {
    const line = `native-use ${quotedToken}`;
    const result = current(line);
    spanScoped(result, 1, line.indexOf('"'), line.length, "string.quoted.double.fluxr");
    without(result, "invalid.");
  }
  for (const escape of [String.raw`\u0000`, String.raw`\uD7FF`, String.raw`\uE000`, String.raw`\uD800\uDC00`, String.raw`\uDBFF\uDFFF`, String.raw`\ud83e\udd80`]) {
    const result = current(`native-use "${escape}"`);
    scoped(result, escape, "constant.character.escape.fluxr");
    without(result, "invalid.");
  }
  for (const quotedToken of [String.raw`"escaped \" next"`, String.raw`"slash \\"`]) {
    const line = `native-use ${quotedToken} next x`;
    const result = current(line);
    const start = line.indexOf('"');
    spanScoped(result, 1, start, start + quotedToken.length, "string.quoted.double.fluxr");
    spanWithout(result, 1, start, start + quotedToken.length, "keyword.", "constant.numeric.");
    spanScoped(result, 1, line.lastIndexOf("next"), line.lastIndexOf("next") + 4, "keyword.operator.fluxr");
    without(result, "invalid.");
  }
  for (const bad of [String.raw`"bad \q next"`, String.raw`"bad \x41 next"`, String.raw`"bad \u12 next"`, String.raw`"bad \uZZZZ next"`, '"bad\t next"', '"bad\r next"', '"bad\v next"', '"bad\u0001 next"']) {
    const line = `native-use ${bad}`;
    const result = current(line);
    const start = line.indexOf('"');
    spanScoped(result, 1, start, line.length, "string.quoted.double.fluxr");
    spanWithout(result, 1, start, line.length, "keyword.", "constant.numeric.", "comment.");
    assert.ok(result.some(({ scopes }) => scopes.some((scope) => scope.startsWith("invalid.illegal."))), bad);
  }
  for (const [escaped, invalid] of [
    [String.raw`\uD800`, String.raw`\uD800`],
    [String.raw`\uDFFF`, String.raw`\uDFFF`],
    [String.raw`\uDBFF\uE000`, String.raw`\uDBFF`],
    [String.raw`\uD800\\uDC00`, String.raw`\uD800`],
  ]) {
    const result = current(`native-use "${escaped}"`);
    scoped(result, invalid, "invalid.illegal.escape.fluxr");
    spanWithout(result, 1, 11, 13 + escaped.length, "keyword.", "constant.numeric.");
  }
  const unterminated = current('native-use "next 0.5f\nret 0.25f');
  spanScoped(unterminated, 1, 11, 21, "string.quoted.double.fluxr");
  spanScoped(unterminated, 1, 11, 21, "invalid.illegal.unterminated-string.fluxr");
  spanWithout(unterminated, 1, 11, 21, "keyword.", "constant.numeric.");
  scoped(unterminated, "ret", "keyword.control.fluxr");
  scoped(unterminated, "0.25f", "constant.numeric.float.fluxr");
  without(unterminated.filter(({ lineNumber }) => lineNumber === 2), "string.quoted.", "invalid.");
  const trailingSlash = current('native-use "bad\\\nret 0.25f');
  spanScoped(trailingSlash, 1, 11, 16, "invalid.illegal.unterminated-string.fluxr");
  scoped(trailingSlash, "ret", "keyword.control.fluxr");
  scoped(trailingSlash, "0.25f", "constant.numeric.float.fluxr");
  without(trailingSlash.filter(({ lineNumber }) => lineNumber === 2), "string.quoted.", "invalid.");
  without(body(current('name"next"')), "string.quoted.", "keyword.", "constant.numeric.");
  for (const suffix of ["tail", '"prev"', "#tail"]) {
    const attached = `"next"${suffix}`;
    const result = current(attached);
    without(body(result), "keyword.", "constant.numeric.", "constant.character.escape.");
    scoped(result, suffix, "invalid.illegal.quoted-token.fluxr");
  }

  // Byte-counted native bodies can contain arbitrary UTF-8/Rust and end midline.
  // A line grammar cannot delimit them. Do not introduce a false Rust embedding
  // or an unbounded raw-body state that swallows the following checked source.
  const nativeBody = 'fn helper() { let _ = "🦀"; }';
  const mixed = current(`rust ${Buffer.byteLength(nativeBody, "utf8")}\n${nativeBody} pub after 0 float ret 0.25f`);
  scoped(mixed, "rust", "keyword.declaration.fluxr");
  scoped(mixed, "pub", "keyword.declaration.fluxr");
  scoped(mixed, "0.25f", "constant.numeric.float.fluxr");
  without(mixed, "source.rust", "meta.embedded.");
  console.log("FluxR TextMate scope checks passed");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
