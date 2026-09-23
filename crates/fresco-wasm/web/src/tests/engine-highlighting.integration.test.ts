import { beforeAll, expect, it } from "vitest";
import { readFile } from "node:fs/promises";
import initWasm, { fresco_lex_tokens_with_files } from "../../pkg/fresco_wasm.js";
import { createEditorHighlightingController } from "../app/editor-highlighting";
beforeAll(async () => { await initWasm({module_or_path: await readFile(new URL("../../pkg/fresco_wasm_bg.wasm", import.meta.url))}); });
it("derives keyword spans from the current engine and retains identifier uses", () => {
  const source = "// \u{1f31f}\nswarm sparks { birth { let swarm = 1; } tick { birth(); } }";
  const files = {"engine/engine.fr": "@entry(swarm, tick) interface System { fn birth(p: f32) -> f32\n fn tick(p: f32) -> f32 }"};
  const tokens = fresco_lex_tokens_with_files(source, "main.fr", files);
  const controller = createEditorHighlightingController({getLexTokens: () => tokens,
    getLanguageConfig: () => ({keywords: ["let"], typeKeywords: [], builtins: [], spaceTransforms: [], enumMembers: []})});
  const spans = controller.syntaxHighlighter(null, source);
  const words = spans.filter(s => s.className === "cm-fresco-keyword").map(s => source.slice(s.from,s.to));
  expect(words).toEqual(["swarm", "birth", "let", "tick"]);
  expect(fresco_lex_tokens_with_files(source,"main.fr", {}).filter(t => t.kind === "engine_keyword")).toHaveLength(0);
  files["engine/engine.fr"] = files["engine/engine.fr"].replace("swarm", "flock");
  expect(fresco_lex_tokens_with_files(source,"main.fr", files).filter(t => t.kind === "engine_keyword")).toHaveLength(0);
});
