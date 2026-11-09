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
    return source.split(/\r?\n/).flatMap((line) => {
      const result = grammar.tokenizeLine(line, stack);
      stack = result.ruleStack;
      return result.tokens.map(({ startIndex, endIndex, scopes }) => ({
        text: line.slice(startIndex, endIndex),
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
  const earlier = tokens("\nr3 2\ni0 f32:+.5 f<= # no comment in v3");
  scoped(earlier, "f32:+.5", "constant.numeric.float.fluxr");
  scoped(earlier, "f<=", "keyword.operator.fluxr");
  assert.ok(earlier.every(({ scopes }) => !scopes.includes("comment.line.number-sign.fluxr")));
  scoped(tokens("r4 1 i0#comment"), "#comment", "comment.line.number-sign.fluxr");
  scoped(tokens("|abc"), "abc", "variable.parameter.fluxr");
  scoped(tokens("a"), "a", "variable.parameter.fluxr");
  console.log("FluxR TextMate scope checks passed");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
