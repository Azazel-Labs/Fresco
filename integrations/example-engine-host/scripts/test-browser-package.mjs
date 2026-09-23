// Local Chrome/WebGPU check of the distributable static directory, outside CI.
import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";

const require = createRequire(new URL("../../../crates/fresco-wasm/web/package.json", import.meta.url));
const { chromium } = require("@playwright/test");
const directory = await mkdtemp(path.join(tmpdir(), "fresco-web-package-"));
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".fr": "text/plain" };
let browser;
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, "http://localhost");
    const file = path.resolve(directory, `.${decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname)}`);
    const relative = path.relative(directory, file);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      response.writeHead(403).end();
      return;
    }
    const body = await readFile(file);
    response.writeHead(200, { "Content-Type": types[path.extname(file)] ?? "application/octet-stream" }).end(body);
  } catch { response.writeHead(404).end(); }
});
try {
  await cp(new URL("../web/", import.meta.url), directory, { recursive: true });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ channel: process.env.FRESCO_GPU_BROWSER ?? "chrome", headless: true });
  const page = await browser.newPage();
  page.setDefaultTimeout(120_000);
  const errors = [];
  const requests = [];
  page.on("pageerror", error => errors.push(String(error)));
  page.on("request", request => requests.push(request.url()));
  await page.goto(origin);
  const status = page.locator("#status");
  await status.filter({ hasText: "Rendering demo" }).waitFor();
  await page.locator("#renderer").selectOption("deferred");
  const samples = [["deferred.fr", "deferred_material"], ["forward_plus.fr", "tiled_lights"], ["mesh.fr", "adjustable"], ["particles.fr", "drifting_sparks"], ["demo.fr", "demo"]];
  for (const [file, entry] of samples) {
    await page.locator("#sample").selectOption(file);
    await status.filter({ hasText: `Rendering ${entry}` }).waitFor();
    assert.equal(await page.locator("#source").inputValue(), await readFile(path.join(directory, file), "utf8"));
    const frames = Number(await page.locator("canvas").getAttribute("data-frames"));
    await page.waitForFunction(before => Number(document.querySelector("canvas").dataset.frames) > before + 3, frames);
    assert.equal(await page.locator("#diagnostics").textContent(), "");
  }
  await page.locator("#sample").selectOption("mesh.fr");
  await status.filter({ hasText: "Rendering adjustable" }).waitFor();
  const material = await page.locator("#source").inputValue();
  await page.locator("#renderer").selectOption("forward");
  await status.filter({ hasText: "Rendering adjustable" }).waitFor();
  assert.equal(await page.locator("#source").inputValue(), material);
  assert.equal(await page.locator("#diagnostics").textContent(), "");
  assert.deepEqual(errors, []);
  assert.ok(requests.length > 0);
  assert.ok(requests.every(url => url.startsWith(`${origin}/`)), "all browser resources come from the copied package");
  console.log("Copied browser package passed: embedded engine, compiler worker, renderer, and canvas/mesh/particle sample selection.");
} finally {
  await browser?.close();
  await new Promise(resolve => server.close(resolve));
  // Only remove the exact directory returned by mkdtemp above.
  await rm(directory, { recursive: true, force: true });
}
