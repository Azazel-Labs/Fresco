// Local path diagnostic; --verify enforces exact matched-data transport parity.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { writeFileSync } from "node:fs";
const require = createRequire(new URL("../../../crates/fresco-wasm/web/package.json", import.meta.url));
const { chromium } = require("@playwright/test");
const base = new URL(process.env.FRESCO_ENGINE_URL ?? "http://127.0.0.1:5182/");
const browser = await chromium.launch({ channel: process.env.FRESCO_GPU_BROWSER ?? "chrome", headless: true });
const deadline = setTimeout(() => browser.close().catch(console.error), 120_000);
try {
  const page = await browser.newPage({ viewport: { width: 600, height: 600 }, deviceScaleFactor: 1 });
  const url = new URL("path-diagnostic.html", base).href;
  await page.route(url, route => route.fulfill({ contentType: "text/html", body: '<canvas width="514" height="514"></canvas>' }));
  await page.goto(url);
  const variants = await page.evaluate(async () => {
    const worker = new Worker("./compiler-worker.js", { type: "module" });
    await new Promise((resolve, reject) => {
      worker.onerror = reject;
      worker.onmessage = ({ data }) => data.ready ? resolve() : reject(Error(data.error));
    });
    const compile = copies => new Promise((resolve, reject) => {
      worker.onmessage = ({ data }) => data.error || !data.result?.ok
        ? reject(Error(data.error || JSON.stringify(data.result?.diagnostics)))
        : resolve(JSON.parse(JSON.stringify(data.result, (_, value) => value instanceof Map ? Object.fromEntries(value) : value)));
      worker.postMessage({ id: copies, source: `canvas path_probe(ctx: CanvasContext) -> color {
        param widths: array<f32> = [0.03]
        let curve = path_svg("M 0.1 0.5 ${"C 0.2 0.2 0.8 0.8 0.9 0.5 C 0.8 0.8 0.2 0.2 0.1 0.5 ".repeat(copies)}", preserve_cubics: 1)
        compose { curve |> stroke(width: widths[0]) }
      }` });
    });
    let short, buffered;
    try { short = await compile(1); buffered = await compile(33); }
    finally { worker.terminate(); }
    const path = buffered.manifest.canvases[0].path_buffers[0];
    if (!path || path.data.rows.length !== 66) throw Error("Expected the reflected 66-segment path");
    const declaration = new RegExp(`@group\\(${path.group}\\) @binding\\(${path.binding}\\)\\s*var<storage(?:, read)?> (\\w+): array<FrescoPathSeg, 66>;`);
    const match = buffered.wgsl.match(declaration);
    if (!match) throw Error("Path declaration changed; update this explicit diagnostic transformation");
    const scalar = value => `${Number(value).toExponential(16)}f`;
    const vector = value => `vec2<f32>(${value.map(scalar).join(",")})`;
    const rows = path.data.rows.map(row => `FrescoPathSeg(${[row.p0,row.p1,row.p2,row.p3].map(vector).join(",")},${scalar(row.s0)},${scalar(row.len)},${row.kind}u,${scalar(row.mid_u)},0f)`);
    const constant = buffered.wgsl.replace(declaration, `const ${match[1]}: array<FrescoPathSeg, 66> = array<FrescoPathSeg, 66>(${rows.join(",")});`);
    const inlineManifest = structuredClone(buffered.manifest);
    inlineManifest.canvases[0].path_buffers = [];
    const loop2 = code => {
      if (!code.includes(">= 66u")) throw Error("Path loop bound changed");
      return code.replace(">= 66u", ">= 2u");
    };
    const privateShort = short.wgsl.replace(/const (fresco_path_\w+:)/, "var<private> $1");
    if (privateShort === short.wgsl) throw Error("Short-path declaration changed");
    const module = await import("./renderer/fresco_example_engine_host.js");
    await module.default();
    window.pathEngine = await module.BrowserEngine.create(document.querySelector("canvas"));
    return [
      { name: "buffer66", wgsl: buffered.wgsl, manifest: buffered.manifest },
      { name: "constant66", wgsl: constant, manifest: inlineManifest },
      { name: "constant2", wgsl: short.wgsl, manifest: short.manifest },
      { name: "private2", wgsl: privateShort, manifest: short.manifest },
      { name: "buffer66_loop2", wgsl: loop2(buffered.wgsl), manifest: buffered.manifest },
      { name: "constant66_loop2", wgsl: loop2(constant), manifest: inlineManifest },
    ];
  });
  const images = [];
  for (const variant of variants) {
    await page.evaluate(async ({ wgsl, manifest }) => {
      if (!await window.pathEngine.install(wgsl, JSON.stringify(manifest), "path_probe")) throw Error("Installation canceled");
      if (!await window.pathEngine.render_async(0, 0)) throw Error("Frame not submitted");
    }, variant);
    images.push({ name: variant.name, png: (await page.locator("canvas").screenshot()).toString("base64") });
  }
  const differences = await page.evaluate(async images => {
    const pixels = await Promise.all(images.map(async ({ name, png }) => {
      const bitmap = await createImageBitmap(await (await fetch("data:image/png;base64," + png)).blob());
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext("2d"); context.drawImage(bitmap, 0, 0); bitmap.close();
      return { name, width: canvas.width, height: canvas.height, data: context.getImageData(0, 0, canvas.width, canvas.height).data };
    }));
    return pixels.map(image => {
      if (image.width !== pixels[0].width || image.height !== pixels[0].height) throw Error("Mismatched image sizes");
      let changedPixels = 0, maximumChannelDifference = 0;
      for (let i = 0; i < image.data.length; i += 4) {
        let changed = false;
        for (let channel = 0; channel < 4; channel++) {
          const difference = Math.abs(image.data[i + channel] - pixels[0].data[i + channel]);
          changed ||= difference !== 0; maximumChannelDifference = Math.max(maximumChannelDifference, difference);
        }
        changedPixels += changed;
      }
      return { name: image.name, changedPixels, maximumChannelDifference };
    });
  }, images);
  await page.evaluate(() => window.pathEngine.free());
  if (process.argv.includes("--verify")) {
    assert.deepEqual(differences.find(item => item.name === "constant66"),
      { name: "constant66", changedPixels: 0, maximumChannelDifference: 0 },
      "matched 66-row constant/storage paths must have identical decoded RGBA pixels");
  }
  const report = { measured_at: new Date().toISOString(), browser: browser.version(), resolution: [514, 514],
    reference: "buffer66", differences,
    note: "Matched 66-row constant/storage RGBA equality is enforced with --verify; other matrix rows are diagnostic." };
  const json = JSON.stringify(report, null, 2) + "\n";
  const index = process.argv.indexOf("--output");
  if (index >= 0) { if (!process.argv[index + 1]) throw Error("--output requires a path"); writeFileSync(process.argv[index + 1], json); }
  console.log(json);
} finally { clearTimeout(deadline); await browser.close(); }
