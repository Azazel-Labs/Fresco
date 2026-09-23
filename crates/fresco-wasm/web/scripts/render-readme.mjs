import { readFile, writeFile, readdir, mkdir, mkdtemp, rename } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { chromium } from "@playwright/test";
import { createServer } from "vite";
import { buildWasm } from "../../../../integrations/example-engine-host/scripts/build-wasm.mjs";
import { digest, inputDigest, isCurrent, embedPreviews } from "./readme-media-lib.mjs";

const web = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const root = path.resolve(web, "../../..");
const output = path.join(root, "docs/readme/media");
const python = process.env.PYTHON || "python";
const force = process.argv.includes("--force");
if (process.argv.slice(2).some(arg => arg !== "--force")) throw new Error("Only --force is supported");

function run(command, args, cwd = root, quiet = false) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, stdio: quiet ? ["ignore", "pipe", "inherit"] : "inherit", windowsHide: true });
    let stdout = "";
    child.stdout?.on("data", data => { stdout += data; });
    child.on("error", reject);
    child.on("exit", code => code === 0 ? resolve(stdout.trim()) : reject(new Error(`${command} exited ${code}`)));
  });
}
async function walk(relative, accept = () => true) {
  const files = [];
  for (const item of await readdir(path.join(root, relative), { withFileTypes: true })) {
    const name = `${relative}/${item.name}`;
    if (item.isDirectory()) files.push(...await walk(name, accept));
    else if (accept(name)) files.push(name);
  }
  return files.sort();
}
async function optionalRead(file) {
  try { return await readFile(file); } catch (error) { if (error.code === "ENOENT") return null; throw error; }
}

await run("cargo", ["xtask", "readme-sync"]);
const config = JSON.parse(await readFile(path.join(root, "docs/readme/samples.json"), "utf8"));
const samples = config.samples;
const ids = new Set();
for (const sample of samples) {
  if (!/^[a-z][a-z0-9_]*$/.test(sample.id) || ids.has(sample.id)) {
    throw new Error(`Invalid/duplicate preview: ${sample.id}`);
  }
  for (const source of [sample.source, sample.entrypoint ?? sample.source]) {
    if (typeof source !== "string" || !(source.startsWith("examples/") || source.startsWith("integrations/example-engine/engine/")) || !source.endsWith(".fr")
        || source.includes("\\") || source.includes(":") || source.split("/").some(part => !part || part === "." || part === "..")) {
      throw new Error("Preview source must be a repository example");
    }
  }
  ids.add(sample.id);
  if (sample.preview === false) continue;
  if (!sample.alt || /[\[\]\r\n]/.test(sample.alt)) throw new Error("Preview needs plain-text alt text");
  const { width, height, fps } = { ...config, ...sample };
  if (![width, height, fps].every(value => Number.isInteger(value) && value > 0) || width > 1024 || height > 1024 || fps > 60) {
    throw new Error("Invalid capture dimensions/FPS");
  }
  if (sample.mesh != null && !["box", "sphere"].includes(sample.mesh)) throw new Error("Invalid preview mesh");
  if (sample.warmup != null && (!Number.isFinite(sample.warmup) || sample.warmup < 0 || sample.warmup > 10)) throw new Error("Invalid simulation warmup");
  if (sample.duration != null && (!(sample.duration > 0) || sample.duration * fps < 2 || sample.duration * fps > 1200)) {
    throw new Error("Animation must contain 2..1200 frames");
  }
}
const readmePath = path.join(root, "README.md");
const readme = (await readFile(readmePath, "utf8")).replaceAll("\r\n", "\n");
embedPreviews(readme, samples); // Validate mappings before the expensive capture.
const encoderVersion = await run(python, ["-c", "from PIL import Image, features; assert features.check('webp'); print(Image.__version__ + '/' + str(features.version('webp')))"], root, true);
const commonPaths = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml",
  "crates/fresco/Cargo.toml", "crates/fresco-macros/Cargo.toml", "crates/fresco-wasm/Cargo.toml",
  "crates/fresco-artifact/Cargo.toml", "integrations/example-engine/Cargo.toml",
  "integrations/example-engine/build.rs", "integrations/example-engine-host/Cargo.toml",
  "integrations/example-engine-host/scripts/build-wasm.mjs", "crates/fresco-wasm/web/scripts/build-wasm.mjs",
  ...await walk("crates/fresco-artifact/src"), ...await walk("integrations/example-engine/src"),
  ...await walk("integrations/example-engine-host/src"),
  "crates/fresco-wasm/web/package.json", "crates/fresco-wasm/web/package-lock.json",
  "crates/fresco-wasm/web/scripts/render-readme.mjs", "crates/fresco-wasm/web/scripts/readme-media-lib.mjs",
  "crates/fresco-wasm/web/scripts/encode-readme.py",
  ...await walk("crates/fresco/src"), ...await walk("crates/fresco-macros/src"),
  ...await walk("crates/fresco-wasm/src"), ...await walk("crates/fresco-wasm/web/src/preview"),
  ...await walk("crates/fresco-wasm/web/tests/readme"), ...await walk("integrations/example-engine/engine")];
const common = new Map(await Promise.all(commonPaths.map(async name => [name, await readFile(path.join(root, name), "utf8")])));
for (const name of await walk("integrations/example-engine/assets")) {
  common.set(name, await readFile(path.join(root, name)));
}
// Include shared renderer utilities/types outside preview/ without generated files.
for (const name of await walk("crates/fresco-wasm/web/src", name => /\.(ts|js)$/.test(name) && !/\/(tests|examples|generated)\//.test(name))) {
  common.set(name, await readFile(path.join(root, name), "utf8"));
}
const files = Object.fromEntries(await Promise.all([
  ...await walk("examples", name => name.endsWith(".fr")),
  ...await walk("integrations/example-engine/engine", name => name.endsWith(".fr")),
].map(async name => [name, await readFile(path.join(root, name), "utf8")])));
function addImports(name, dependencies) {
  if (dependencies.has(name)) return;
  const source = files[name];
  if (source == null) throw new Error(`Missing bundled source: ${name}`);
  dependencies.set(name, source);
  for (const match of source.matchAll(/^\s*import\s+"([^"]+)"/gm)) {
    addImports(path.posix.normalize(path.posix.join(path.posix.dirname(name), match[1])), dependencies);
  }
}
await mkdir(output, { recursive: true });
const manifestPath = path.join(output, "manifest.json");
const previousBytes = await optionalRead(manifestPath);
const previous = previousBytes ? JSON.parse(previousBytes.toString()) : { samples: {} };
const next = { version: 1, format: "webp", samples: {} };
const pending = [];
const previews = samples.filter(sample => sample.preview !== false);
for (const sample of previews) {
  const settings = { width: config.width, height: config.height, fps: config.fps, ...sample, encoderVersion };
  const dependencies = new Map(common);
  addImports(sample.entrypoint ?? sample.source, dependencies);
  addImports(sample.source, dependencies);
  const textures = {};
  for (const [name, asset] of Object.entries(sample.textures || {})) {
    if (!asset.startsWith("examples/assets/") || asset.includes("..")) throw new Error("Textures must be repository example assets");
    const bytes = await readFile(path.join(root, asset));
    dependencies.set(asset, bytes);
    textures[name] = `data:image/png;base64,${bytes.toString("base64")}`;
  }
  const inputHash = inputDigest(dependencies, settings);
  const bytes = await optionalRead(path.join(output, `${sample.id}.webp`));
  if (!force && isCurrent(previous.samples[sample.id], inputHash, bytes)) {
    next.samples[sample.id] = previous.samples[sample.id];
    console.log(`Cached ${sample.id}`);
  } else pending.push({ sample, settings, inputHash, textures });
}
if (pending.length) {
  // Never use a stale WASM binary when source dependencies have changed.
  // Separate packages from the live playground's watcher-owned output directories.
  const packages = path.join(root, "target/readme-media/packages");
  const compilerOutput = path.join(packages, "compiler");
  const rendererOutput = path.join(packages, "renderer");
  buildWasm({ compilerOutput, rendererOutput, release: true });
  const fsUrl = file => `/@fs/${file.replaceAll("\\", "/")}`;
  const server = await createServer({ root: web, base: "/",
    define: {
      __README_COMPILER_URL__: JSON.stringify(fsUrl(path.join(compilerOutput, "fresco_wasm.js"))),
      __README_RENDERER_URL__: JSON.stringify(fsUrl(path.join(rendererOutput, "fresco_example_engine_host.js"))),
    },
    server: { host: "127.0.0.1", port: 5193, strictPort: true, fs: { allow: [root] } },
  });
  let browser;
  try {
    await server.listen();
    const software = process.env.FRESCO_README_SOFTWARE_GPU === "1";
    const channel = process.env.FRESCO_GPU_BROWSER || "chrome";
    browser = await chromium.launch({ channel: channel === "chromium" ? undefined : channel,
      headless: true, args: software ? ["--enable-unsafe-webgpu", "--use-webgpu-adapter=swiftshader"] : [] });
    const temporary = path.join(root, "target/readme-media");
    await mkdir(temporary, { recursive: true });
    const staging = await mkdtemp(path.join(temporary, "capture-"));
    for (const { sample, settings, inputHash, textures } of pending) {
      const frameDir = path.join(staging, sample.id);
      await mkdir(frameDir);
      const page = await browser.newPage({ viewport: { width: settings.width, height: settings.height }, deviceScaleFactor: 1 });
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      try {
        await page.goto("http://127.0.0.1:5193/tests/readme/");
        await page.waitForFunction(() => window.readmeCapture);
        const adapter = await page.evaluate(args => window.readmeCapture.init(args), {
          files, entrypoint: sample.entrypoint ?? sample.source, width: settings.width, height: settings.height, textures, mesh: sample.mesh ?? "box",
        });
        if (software && !adapter.fallback) throw new Error("Software capture requested but browser selected a hardware adapter");
        const warmupFrames = Math.round((sample.warmup ?? 0) * settings.fps);
        for (let i = 0; i < warmupFrames; i++) {
          await page.evaluate(({ time, dt }) => window.readmeCapture.frame(time, dt), { time: i / settings.fps, dt: 1 / settings.fps });
        }
        const count = sample.duration ? Math.round(sample.duration * settings.fps) : 1;
        console.log(`Rendering ${sample.id}: ${count} frames`);
        for (let i = 0; i < count; i++) {
          const time = warmupFrames / settings.fps + (sample.duration ? i * sample.duration / count : 0);
          const dt = sample.duration || warmupFrames ? 1 / settings.fps : 0;
          const png = await page.evaluate(({ time, dt }) => window.readmeCapture.frame(time, dt), { time, dt });
          await writeFile(path.join(frameDir, `frame-${String(i).padStart(4, "0")}.png`), Buffer.from(png.split(",")[1], "base64"));
        }
        if (errors.length) throw new Error(errors.join("\n"));
        const stagedOutput = path.join(staging, `${sample.id}.webp`);
        const encoded = JSON.parse(await run(python, [path.join(web, "scripts/encode-readme.py"), frameDir,
          stagedOutput, String(sample.duration || 1)], root, true));
        const bytes = await readFile(stagedOutput);
        next.samples[sample.id] = { source: sample.source, inputSha256: inputHash, outputSha256: digest(bytes),
          bytes: bytes.length, ...encoded, duration: sample.duration || 0, capturedFrames: count,
          encoderVersion, browser: browser.version(), adapter };
      } finally { await page.close(); }
    }
    // Publish only after every changed preview rendered and encoded successfully.
    for (const { sample } of pending) await rename(path.join(staging, `${sample.id}.webp`), path.join(output, `${sample.id}.webp`));
  } finally {
    await browser?.close();
    await server.close();
  }
}
await writeFile(manifestPath, JSON.stringify(next, null, 2) + "\n");
const latestReadme = (await readFile(readmePath, "utf8")).replaceAll("\r\n", "\n");
const updatedReadme = embedPreviews(latestReadme, samples);
if (latestReadme !== updatedReadme) await writeFile(readmePath, updatedReadme);
console.log(`README media ready: ${pending.length} rendered, ${previews.length - pending.length} cached.`);
