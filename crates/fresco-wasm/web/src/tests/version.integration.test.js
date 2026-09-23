import { readFile } from "node:fs/promises";
import { expect, it } from "vitest";
import initWasm, { fresco_wasm_version } from "../../pkg/fresco_wasm.js";

it("reports the release version embedded in the actual WASM compiler", async () => {
  const bytes = await readFile(new URL("../../pkg/fresco_wasm_bg.wasm", import.meta.url));
  await initWasm({ module_or_path: bytes });
  const pkg = JSON.parse(await readFile(new URL("../../package.json", import.meta.url), "utf8"));
  expect(fresco_wasm_version()).toBe(pkg.version);
});
