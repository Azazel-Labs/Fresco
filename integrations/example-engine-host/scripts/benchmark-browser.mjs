// Local-only measurements, not a GPU test or performance gate in CI.
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(new URL("../../../crates/fresco-wasm/web/package.json", import.meta.url));
const { chromium } = require("@playwright/test");
const base = new URL(process.env.FRESCO_ENGINE_URL ?? "http://127.0.0.1:5182/");
const scenes = [
  { name: "canvas", entry: "demo", source: readFileSync(new URL("../demo.fr", import.meta.url), "utf8") },
  { name: "mesh", entry: "adjustable", source: readFileSync(new URL("../../example-engine/examples/material_parameters.fr", import.meta.url), "utf8") },
  { name: "particles", entry: "drifting_sparks", source: readFileSync(new URL("../../../examples/50) particles/drifting_sparks.fr", import.meta.url), "utf8") },
];
const browser = await chromium.launch({ channel: process.env.FRESCO_GPU_BROWSER ?? "chrome", headless: true });
const deadline = setTimeout(() => browser.close().catch(console.error), 180_000);
try {
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", error => errors.push(String(error)));
  const url = new URL("benchmark.html", base).href;
  // Use an otherwise empty same-origin host: no playground or demo animation.
  await page.route(url, route => route.fulfill({ contentType: "text/html", body: '<canvas width="512" height="512"></canvas>' }));
  await page.goto(url);
  const result = await page.evaluate(async scenes => {
    const now = () => performance.now();
    const summarize = values => {
      const sorted = [...values].sort((a, b) => a - b);
      return { samples: values.length, min_ms: sorted[0], median_ms: (sorted[Math.floor((sorted.length - 1) / 2)] + sorted[Math.floor(sorted.length / 2)]) / 2,
        p95_ms: sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * .95) - 1)], max_ms: sorted.at(-1) };
    };
    const started = now();
    const worker = new Worker("./compiler-worker.js", { type: "module" });
    await new Promise((resolve, reject) => {
      worker.onerror = reject;
      worker.onmessage = ({ data }) => data.ready ? resolve() : reject(Error(data.error));
    });
    const compilerStartup = now() - started;
    const rendererStart = now();
    const module = await import("./renderer/fresco_example_engine_host.js");
    await module.default();
    const rendererStartup = now() - rendererStart;
    const deviceStart = now();
    const engine = await module.BrowserEngine.create(document.querySelector("canvas"));
    const camera = new module.BrowserCamera(); engine.set_camera(camera);
    await engine.resize(512, 512);
    const deviceStartup = now() - deviceStart;
    const outputs = [];
    let request = 0;
    try {
      for (const scene of scenes) {
        const compileStart = now();
        const artifact = await new Promise((resolve, reject) => {
          worker.onmessage = ({ data }) => data.error ? reject(Error(data.error)) : resolve(data.result);
          worker.postMessage({ id: ++request, source: scene.source });
        });
        const compileMs = now() - compileStart;
        if (!artifact.ok) throw Error(JSON.stringify(artifact.diagnostics));
        const manifest = JSON.stringify(artifact.manifest, (_, value) => value instanceof Map ? Object.fromEntries(value) : value);
        const prepare = [];
        for (let i = 0; i < 5; i++) {
          const start = now();
          if (!await engine.install_with_assets(artifact.wgsl, manifest, scene.entry, new Map(), "{}", "sphere")) throw Error("installation canceled");
          prepare.push(now() - start);
        }
        const firstStart = now();
        if (!await engine.render_async(0, 0)) throw Error("first frame not submitted");
        const firstFrameMs = now() - firstStart;
        for (let i = 0; i < 20; i++) await engine.render_async(i / 60, 1 / 60);
        const frames = [];
        for (let i = 0; i < 120; i++) {
          const start = now();
          if (!await engine.render_async((i + 20) / 60, 1 / 60)) throw Error("frame not submitted");
          frames.push(now() - start);
        }
        outputs.push({ scene: scene.name, compile_worker_roundtrip_ms: compileMs,
          first_prepare_ms: prepare[0], repeated_prepare: summarize(prepare.slice(1)),
          first_frame_submission_ms: firstFrameMs, frame_submission: summarize(frames),
          wgsl_bytes: new TextEncoder().encode(artifact.wgsl).length, manifest_bytes: new TextEncoder().encode(manifest).length });
      }
      const payloads = {};
      for (const [name, path] of [["compiler", "compiler/fresco_wasm_bg.wasm"], ["renderer", "renderer/fresco_example_engine_host_bg.wasm"]]) {
        const response = await fetch(path);
        if (!response.ok) throw Error(`${path}: HTTP ${response.status}`);
        payloads[name + "_wasm_bytes"] = (await response.arrayBuffer()).byteLength;
      }
      const adapter = await navigator.gpu.requestAdapter();
      return { resolution: [512, 512], adapter: adapter ? { vendor: adapter.info.vendor, architecture: adapter.info.architecture, device: adapter.info.device, description: adapter.info.description } : null,
        compiler_worker_startup_ms: compilerStartup, renderer_module_startup_ms: rendererStartup,
        renderer_device_startup_ms: deviceStartup, payloads, scenes: outputs };
    } finally { worker.terminate(); engine.free(); camera.free(); }
  }, scenes);
  assert.deepEqual(errors, []);
  const report = { measured_at: new Date().toISOString(), browser: browser.version(), platform: process.platform,
    architecture: process.arch, host: base.href,
    caveats: ["Local headless run; browser and driver shader caches may be warm.",
      "Frame timings measure host preparation/submission, not GPU completion or vsync throughput.",
      "Browser timer quantization can report zero for short submissions; zero is not zero execution cost.",
      "WASM byte counts are uncompressed responses. Build with --release for comparable shipping sizes."], ...result };
  const json = JSON.stringify(report, null, 2) + "\n";
  const outputIndex = process.argv.indexOf("--output");
  if (outputIndex >= 0) {
    const output = process.argv[outputIndex + 1];
    if (!output) throw Error("--output requires a path");
    writeFileSync(output, json);
  }
  console.log(json);
} finally { clearTimeout(deadline); await browser.close(); }
