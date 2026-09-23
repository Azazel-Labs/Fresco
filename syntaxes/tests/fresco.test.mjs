import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";
import textmate from "vscode-textmate";
import oniguruma from "vscode-oniguruma";

const grammarUrl = new URL("../fresco.tmLanguage.json", import.meta.url);
const wasm = await readFile(new URL("../node_modules/vscode-oniguruma/release/onig.wasm", import.meta.url));
await oniguruma.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
const registry = new textmate.Registry({
  onigLib: Promise.resolve({
    createOnigScanner: patterns => new oniguruma.OnigScanner(patterns),
    createOnigString: value => new oniguruma.OnigString(value),
  }),
  loadGrammar: async scope => scope === "source.fresco"
    ? textmate.parseRawGrammar(await readFile(grammarUrl, "utf8"), grammarUrl.pathname)
    : null,
});
const grammar = await registry.loadGrammar("source.fresco");

function tokenize(source) {
  let state = textmate.INITIAL;
  return source.split(/\r?\n/).map(line => {
    const result = grammar.tokenizeLine(line, state);
    assert.equal(result.stoppedEarly, false);
    state = result.ruleStack;
    return result.tokens.map(token => ({
      text: line.slice(token.startIndex, token.endIndex),
      scopes: token.scopes,
    }));
  });
}

function hasScope(tokens, value, scope) {
  assert.ok(tokens.some(token => token.text === value && token.scopes.includes(scope)),
    `${JSON.stringify(value)} missing ${scope}: ${JSON.stringify(tokens)}`);
}

test("covers compiler keywords and every numeric suffix", async () => {
  const lexer = await readFile(new URL("../../crates/fresco/src/lexer.rs", import.meta.url), "utf8");
  for (const [, word] of lexer.matchAll(/#\[token\("([a-z_]+)"\)\]/g)) {
    hasScope(tokenize(word)[0], word, "keyword.control.fresco");
  }
  const suffixes = lexer.match(/pub const UNIT_SUFFIXES[^=]*= &\[([\s\S]*?)\];/)[1];
  for (const [, suffix] of [...suffixes.matchAll(/"([a-z]+)"/g), [null, "u"], [null, "i"]]) {
    const tokens = tokenize(`1.25e-2${suffix}`)[0];
    hasScope(tokens, "1.25e-2", "constant.numeric.fresco");
    hasScope(tokens, suffix, "storage.type.numeric-suffix.fresco");
  }
});

test("separates ranges, pipes, declarations, types, colors, and host names", () => {
  const lines = tokenize(`canvas badge(uv: coord, time: signal) -> color {
    let steps = [for i in 0..7 => i]
    shape |> fill(#abc) |> fill(#abcd) |> fill(#aabbcc) |> fill(#aabbccdd)
    @builtin #pragma $resolution
}`);
  hasScope(lines[0], "badge", "entity.name.function.fresco");
  hasScope(lines[0], "coord", "support.type.fresco");
  hasScope(lines[1], "..", "keyword.operator.range.fresco");
  hasScope(lines[1], "0", "constant.numeric.fresco");
  hasScope(lines[1], "7", "constant.numeric.fresco");
  hasScope(lines[2], "|>", "keyword.operator.pipe.fresco");
  hasScope(lines[2], "fill", "support.function.fresco");
  for (const color of ["#abc", "#abcd", "#aabbcc", "#aabbccdd"]) {
    hasScope(lines[2], color, "constant.other.color.fresco");
  }
  hasScope(lines[3], "@builtin", "entity.other.attribute-name.fresco");
  hasScope(lines[3], "#pragma", "keyword.control.directive.fresco");
  hasScope(lines[3], "$resolution", "variable.other.host.fresco");
  for (const literal of ["#ab", "#abcde", "#abcdefg", "#abcdefghi"]) {
    assert.ok(tokenize(literal)[0].every(token => !token.scopes.includes("constant.other.color.fresco")));
  }
});

test("comments and multiline strings shield their contents and restore code scopes", () => {
  const lines = tokenize('/* canvas #fff\nfn ignored() */\nlet text = """\n// raw \\q #fff\n"""\nlet value = "ok\\n bad\\q" // comment\nreturn #fff');
  assert.ok(lines[0].every(token => token.scopes.includes("comment.block.fresco")));
  assert.ok(lines[3].every(token => token.scopes.includes("string.quoted.triple.fresco")));
  hasScope(lines[5], "\\n", "constant.character.escape.fresco");
  hasScope(lines[5], "\\q", "invalid.illegal.escape.fresco");
  hasScope(lines[6], "return", "keyword.control.fresco");
  hasScope(lines[6], "#fff", "constant.other.color.fresco");
});

test("loads and tokenizes every repository example with the TextMate engine", async () => {
  let count = 0;
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const url = new URL(encodeURIComponent(entry.name) + (entry.isDirectory() ? "/" : ""), directory);
      if (entry.isDirectory()) await visit(url);
      else if (entry.name.endsWith(".fr")) {
        tokenize(await readFile(url, "utf8"));
        count++;
      }
    }
  }
  await visit(new URL("../../examples/", import.meta.url));
  assert.ok(count > 0);
});
