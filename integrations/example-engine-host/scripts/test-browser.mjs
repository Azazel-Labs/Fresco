// Local GPU check; deliberately excluded from workspace/CI test commands.
// Uses the existing playground's Playwright installation only as a test driver.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { assertPathPixels, decodePixels } from "./path-pixels.mjs";
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(new URL("../../../crates/fresco-wasm/web/package.json", import.meta.url));
const { chromium } = require("@playwright/test");
const browser = await chromium.launch({ channel: process.env.FRESCO_GPU_BROWSER ?? "chrome", headless: true });
const deadline = setTimeout(() => {
  console.error("Browser GPU smoke exceeded its three-minute deadline.");
  browser.close().catch(error => console.error(error));
}, 180_000);
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 800 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", error => { errors.push(String(error)); console.error("Browser page error:", String(error)); });
  await page.goto(process.env.FRESCO_ENGINE_URL ?? "http://127.0.0.1:5182");
  const status = page.locator("#status");
  await page.waitForFunction(() => document.querySelector("#status").textContent.startsWith("Rendering") || document.querySelector("#diagnostics").textContent.length > 0, undefined, { timeout: 120_000 });
  assert.equal(await page.locator("#diagnostics").innerText(), "", "startup diagnostics");
  assert.equal(await status.innerText(), "Rendering demo");
  await page.waitForFunction(() => Number(document.querySelector("canvas").dataset.frames) > 3);
  await page.locator("#pause").click();
  const canvas = page.locator("canvas");
  const compile = async text => {
    await page.locator("#source").fill(text);
    await page.locator("#compile").click();
  };
  await compile("canvas red(ctx: CanvasContext) -> color { rgba(1.0, 0.0, 0.0, 1.0) }");
  await status.filter({ hasText: "Rendering red" }).waitFor();
  const red = await canvas.screenshot();
  const before = Number(await canvas.getAttribute("data-frames"));
  await compile("canvas invalid { this is invalid }");
  await status.filter({ hasText: "Failed; keeping" }).waitFor();
  await page.waitForFunction(n => Number(document.querySelector("canvas").dataset.frames) > n + 3, before);
  assert.deepEqual(await canvas.screenshot(), red, "compiler failure must preserve visible pixels");
  await compile("canvas green(ctx: CanvasContext) -> color { rgba(0.0, 1.0, 0.0, 1.0) }");
  await status.filter({ hasText: "Rendering green" }).waitFor();
  const green = await canvas.screenshot();
  assert.notDeepEqual(green, red, "a valid replacement must change visible pixels");
  const pathSource = copies => `canvas path_probe(ctx: CanvasContext) -> color {
    param widths: array<f32> = [0.03]
    let curve = path_svg("M 0.1 0.5 ${"C 0.2 0.2 0.8 0.8 0.9 0.5 C 0.8 0.8 0.2 0.2 0.1 0.5 ".repeat(copies)}", preserve_cubics: 1)
    compose { curve |> stroke(width: widths[0]) }
  }`;
  await compile(pathSource(1));
  await status.filter({ hasText: "Rendering path_probe" }).waitFor();
  const constantPath = await canvas.screenshot();
  await compile(pathSource(33));
  await status.filter({ hasText: "Rendering path_probe" }).waitFor();
  const bufferedPath = await canvas.screenshot();
  const [constantPixels, bufferedPixels] = await decodePixels(page, [constantPath, bufferedPath]);
  // Keep diagnostic images, including when a comparison exceeds the bounds.
  writeFileSync(new URL("../../../target/path-constant.png", import.meta.url), constantPath);
  writeFileSync(new URL("../../../target/path-buffered.png", import.meta.url), bufferedPath);
  await page.locator("#parameters").fill('{"widths":[0.015,0.03]}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Rendering path_probe" }).waitFor();
  const narrowPath = await canvas.screenshot();
  assert.notDeepEqual(narrowPath, constantPath, "array resizing preserves path-buffer bindings while updating stroke width");
  await page.locator("#rebuild").click();
  await status.filter({ hasText: "Rendering path_probe" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), narrowPath, "GPU recreation restores path data and edited arrays");
  await compile("canvas storage_defaults(ctx: CanvasContext) -> color { param gain: f32 = 1; param index: f32 = 1; param points: array<vec3> = [vec3(0,0,0), vec3(0,1,0)]; rgba(points[index].x * gain, points[index].y * gain, points[index].z, 1.0) }");
  await status.filter({ hasText: "Rendering storage_defaults" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), green, "storage vec3 defaults use padded strides alongside instance uniforms");
  await page.locator("#parameters").fill('{"gain":1,"index":2,"points":[[0,0,0],[0,0,0],[1,0,0]]}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Rendering storage_defaults" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), red, "storage arrays grow without recompiling the shader");
  await page.locator("#parameters").fill('{"index":0,"points":[[0,1,0]]}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Rendering storage_defaults" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), green, "storage arrays shrink while retaining unspecified inputs");
  await page.locator("#parameters").fill('{"gain":0,"points":[[0,1]]}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Failed; keeping" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), green, "invalid mixed batches preserve scalar and storage values together");
  await page.locator("#rebuild").click();
  await status.filter({ hasText: "Rendering storage_defaults" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), green, "GPU recreation restores resized storage values");
  assert.deepEqual(JSON.parse(await page.locator("#parameters").inputValue()).points, [[0,1,0]]);
  await compile("canvas adjustable(ctx: CanvasContext) -> color { param red: f32 = 1 in 0 .. 1; param green: f32 = 0 in 0 .. 1; rgba(red, green, 0.0, 1.0) }");
  await status.filter({ hasText: "Rendering adjustable" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), red, "authored defaults reach the GPU");
  await page.locator("#parameters").fill('{"red":0,"green":1}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Rendering adjustable" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), green, "parameter edits reach the GPU without recompilation");
  await page.locator("#parameters").fill('{"red":0.5,"green":2}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Failed; keeping" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), green, "invalid parameter batches preserve every value");
  await page.locator("#reset").click();
  await page.waitForFunction(() => Number(document.querySelector("canvas").dataset.time) === 0);
  await page.locator("#rebuild").click();
  await status.filter({ hasText: "Rendering adjustable" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), green, "GPU rebuild must restore the current artifact");
  const width = await canvas.evaluate(c => c.width);
  await page.setViewportSize({ width: 900, height: 700 });
  await page.waitForFunction(w => document.querySelector("canvas").width !== w, width);
  const beforeTexture = await canvas.screenshot();
  await compile("canvas textured(ctx: CanvasContext) -> color { param strength: f32 = 1 in 0 .. 1; param weights: array<f32> = [1.0]; uniform paint: texture; paint.at(ctx.uv) |> opacity(strength * weights[0]) }");
  await status.filter({ hasText: "Failed; keeping" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), beforeTexture, "missing assets preserve the previous renderer");
  await page.locator("#texture-file").setInputFiles({
    name: "paint.png", mimeType: "image/png",
    buffer: readFileSync(new URL("../../example-engine/assets/checker.png", import.meta.url)),
  });
  await status.filter({ hasText: "Rendering textured" }).waitFor();
  const originalTexture = await canvas.screenshot();
  assert.notDeepEqual(originalTexture, beforeTexture, "uploaded bytes must affect rendered pixels");
  await page.locator("#parameters").fill('{"strength":0.5}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Rendering textured" }).waitFor();
  const textured = await canvas.screenshot();
  assert.notDeepEqual(textured, originalTexture, "textured shader must consume parameter edits");
  await page.locator("#parameters").fill('{"strength":1,"weights":[0.5,0.25]}');
  await page.locator("#apply-parameters").click();
  await status.filter({ hasText: "Rendering textured" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), textured, "storage replacement retains texture bindings");
  await page.locator("#texture-file").setInputFiles({
    name: "replacement.png", mimeType: "image/png",
    buffer: readFileSync(new URL("../../example-engine/assets/checker.png", import.meta.url)),
  });
  await status.filter({ hasText: "Rendering textured" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), textured, "texture replacement preserves edited parameters");
  await page.locator("#texture-file").setInputFiles({ name: "broken.png", mimeType: "image/png", buffer: Buffer.from("broken image") });
  await status.filter({ hasText: "Failed; keeping" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), textured, "invalid image bytes preserve the current texture");
  await page.locator("#rebuild").click();
  await status.filter({ hasText: "Rendering textured" }).waitFor();
  assert.deepEqual(await canvas.screenshot(), textured, "GPU recreation restores uploaded texture bytes");
  const lifecycle = await page.evaluate(async () => {
    const { BrowserEngine } = await import("./renderer/fresco_example_engine_host.js");
    const worker = new Worker("./compiler-worker.js", { type: "module" });
    const result = await new Promise((resolve, reject) => {
      worker.onerror = event => reject(new Error(event.message));
      worker.onmessage = ({ data }) => {
        if (data.ready) worker.postMessage({ id: 1, source: "canvas probe(ctx: CanvasContext) -> color { param gain: f32 = 1; param colors: array<vec3> = [vec3(0,0,1)]; rgba(colors[0].x, colors[0].y, colors[0].z * gain, 1.0) }" });
        else if (data.error) reject(new Error(data.error));
        else resolve(data.result);
      };
    });
    worker.terminate();
    if (!result.ok) throw new Error(JSON.stringify(result.diagnostics));
    const target = document.createElement("canvas");
    target.width = target.height = 32;
    const renderer = await BrowserEngine.create(target);
    const manifest = JSON.stringify(result.manifest, (_, v) => v instanceof Map ? Object.fromEntries(v) : v);
    try {
      const first = await renderer.install(result.wgsl, manifest, "probe");
      const pending = renderer.install(result.wgsl, manifest, "probe");
      const duringPrepare = renderer.render(0, 0);
      renderer.cancel_pending();
      const obsolete = await pending;
      let invalidRejected = false;
      try { await renderer.install("invalid shader", manifest, "probe"); }
      catch { invalidRejected = true; }
      const afterFailure = renderer.render(0, 0);
      renderer.resize(0, 0);
      const zero = renderer.render(0, 0);
      renderer.resize(32, 32);
      const restored = renderer.render(0, 0);
      const pendingParameters = renderer.update_parameters('{"gain":0.5,"colors":[[1,0,0],[0,1,0]]}');
      await Promise.resolve();
      const duringParameters = renderer.render(0, 0);
      renderer.cancel_pending();
      const canceledParameters = await pendingParameters;
      const unchanged = JSON.parse(renderer.parameter_values_json());
      const earlier = renderer.update_parameters('{"gain":0.25,"colors":[[1,0,0]]}');
      await Promise.resolve();
      const later = renderer.update_parameters('{"gain":0.75,"colors":[[0,1,0],[0,0,1]]}');
      const parameterOrder = await Promise.all([earlier, later]);
      const latest = JSON.parse(renderer.parameter_values_json());
      const replaced = renderer.update_parameters('{"gain":0,"colors":[]}');
      await Promise.resolve();
      await renderer.install(result.wgsl, manifest, "probe");
      const replacedParameters = await replaced;
      let installParametersRejected = false;
      try { await renderer.install_with_assets(result.wgsl, manifest, "probe", new Map(), '{"gain":0,"colors":[[1,2]]}'); }
      catch { installParametersRejected = true; }
      const afterInvalidInstall = JSON.parse(renderer.parameter_values_json());
      return { first, duringPrepare, obsolete, invalidRejected, afterFailure, zero, restored,
        duringParameters, canceledParameters, unchanged, parameterOrder, latest,
        replacedParameters, installParametersRejected, afterInvalidInstall };
    } finally { renderer.cancel_pending(); renderer.free(); }
  });
  assert.deepEqual(lifecycle, {
    first: true, duringPrepare: true, obsolete: false, invalidRejected: true,
    afterFailure: true, zero: false, restored: true,
    duringParameters: true, canceledParameters: false, unchanged: {gain:1, colors:[[0,0,1]]},
    parameterOrder: [false,true], latest: {gain:0.75, colors:[[0,1,0],[0,0,1]]},
    replacedParameters: false, installParametersRejected: true, afterInvalidInstall: {gain:1, colors:[[0,0,1]]},
  }, "WASM installation must be atomic, cancellable, and safe during rendering");
  assert.deepEqual(errors, [], "no uncaught browser errors");
  console.log("Equivalent path comparison:", assertPathPixels(constantPixels, bufferedPixels));
  // The matched 66-row shader forms must still produce exactly equal RGBA pixels.
  execFileSync(process.execPath, [fileURLToPath(new URL("./diagnose-path-parity.mjs", import.meta.url)), "--verify"],
    { stdio: "inherit", timeout: 120_000 });
  console.log("Browser GPU smoke passed: render, atomic storage resizing, concurrent/canceled edits, texture upload/recovery, failed/valid reload, frame continuity, reset, rebuild, resize, cancellation, invalid shader, zero-size suspension.");
} finally { clearTimeout(deadline); await browser.close(); }
