import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const webDir = path.resolve(__dirname, "..");
const workspaceRoot = path.resolve(webDir, "../../..");

const DEFAULT_EXAMPLE = path.join(
  workspaceRoot,
  "examples",
  "90) gallery",
  "iq_distance_gallery_original_direct.fr"
);
const DEFAULT_OUT = path.join(
  workspaceRoot,
  "target",
  "tmp",
  "wasm-iq-distance-perf.json"
);
const NODE_PKG_DIR = path.join(webDir, "perf", "pkg-node");

const args = process.argv.slice(2);

function parseArg(name, fallback) {
  const idx = args.indexOf(name);
  if (idx === -1) {
    return fallback;
  }
  return args[idx + 1] ?? fallback;
}

function parseIntArg(name, fallback) {
  const raw = parseArg(name, String(fallback));
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback;
}

const examplePath = path.resolve(parseArg("--example", DEFAULT_EXAMPLE));
const outPath = path.resolve(parseArg("--out", DEFAULT_OUT));
const runs = parseIntArg("--runs", 12);
const warmupRuns = parseIntArg("--warmup", 2);
const skipBuild = args.includes("--skip-build");

function runCommand(command, commandArgs, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, commandArgs, {
      cwd,
      stdio: "inherit",
      shell: process.platform === "win32",
    });

    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`${command} exited with code ${code ?? 1}`));
      }
    });
  });
}

async function buildNodeWasmPackage() {
  await runCommand(
    "wasm-pack",
    ["build", "..", "--target", "nodejs", "--out-dir", "web/perf/pkg-node"],
    webDir
  );
}

async function ensureNodeWasmPackage() {
  const pkgEntrypoint = path.join(NODE_PKG_DIR, "fresco_wasm.js");
  if (!skipBuild) {
    console.log(`[perf] Building fresco-wasm Node package in ${NODE_PKG_DIR}...`);
    await buildNodeWasmPackage();
    return;
  }

  try {
    await access(pkgEntrypoint);
    console.log(`[perf] Using existing fresco-wasm Node package at ${NODE_PKG_DIR} (--skip-build)`);
  } catch {
    throw new Error(
      `--skip-build was requested but ${pkgEntrypoint} is missing. Run once without --skip-build.`
    );
  }
}

function summarize(numbers) {
  const sorted = [...numbers].sort((a, b) => a - b);
  const total = numbers.reduce((acc, n) => acc + n, 0);
  const avg = numbers.length ? total / numbers.length : 0;
  const median =
    sorted.length % 2 === 0
      ? (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2
      : sorted[Math.floor(sorted.length / 2)] ?? 0;
  return {
    min_ms: sorted[0] ?? 0,
    max_ms: sorted[sorted.length - 1] ?? 0,
    avg_ms: avg,
    median_ms: median,
  };
}

async function main() {
  await mkdir(path.dirname(outPath), { recursive: true });
  await ensureNodeWasmPackage();

  const require = createRequire(import.meta.url);
  const wasm = require(path.join(NODE_PKG_DIR, "fresco_wasm.js"));
  const compileFresco = wasm.compile_fresco;

  if (typeof compileFresco !== "function") {
    throw new Error("fresco_wasm.js does not export compile_fresco");
  }

  const source = await readFile(examplePath, "utf8");

  console.log(`[perf] Warmup runs: ${warmupRuns}`);
  for (let i = 0; i < warmupRuns; i += 1) {
    compileFresco(source);
  }

  console.log(`[perf] Measured runs: ${runs}`);
  const samples = [];

  for (let i = 0; i < runs; i += 1) {
    const start = performance.now();
    const result = compileFresco(source);
    const end = performance.now();

    if (!result || result.ok !== true) {
      throw new Error(`compile_fresco failed on run ${i + 1}`);
    }

    samples.push({
      run: i + 1,
      host_elapsed_ms: end - start,
      wasm_total_ms: result.timings.total_ms,
      pipeline: result.timings.pipeline,
      emit_wgsl_ms: result.timings.emit_wgsl_ms,
      emit_manifest_ms: result.timings.emit_manifest_ms,
      explain_ms: result.timings.explain_ms,
    });
  }

  const hostSummary = summarize(samples.map((s) => s.host_elapsed_ms));
  const wasmSummary = summarize(samples.map((s) => s.wasm_total_ms));

  const payload = {
    generated_at: new Date().toISOString(),
    example: path.relative(workspaceRoot, examplePath).replaceAll("\\", "/"),
    runs,
    warmup_runs: warmupRuns,
    host_summary: hostSummary,
    wasm_summary: wasmSummary,
    samples,
  };

  await writeFile(outPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");

  console.log("[perf] Done.");
  console.log(`[perf] host avg=${hostSummary.avg_ms.toFixed(3)} ms median=${hostSummary.median_ms.toFixed(3)} ms`);
  console.log(`[perf] wasm avg=${wasmSummary.avg_ms.toFixed(3)} ms median=${wasmSummary.median_ms.toFixed(3)} ms`);
  console.log(`[perf] report -> ${outPath}`);
}

main().catch((error) => {
  console.error(`[perf] Failed: ${error?.message ?? error}`);
  process.exit(1);
});
