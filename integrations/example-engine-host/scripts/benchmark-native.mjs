// Local-only process timings. Run after cargo build --release.
import assert from "node:assert/strict";
import { statSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
const option = name => {
  const index = process.argv.indexOf(name);
  if (index < 0) return undefined;
  if (!process.argv[index + 1] || process.argv[index + 1].startsWith("--")) throw Error(`${name} requires a value`);
  return process.argv[index + 1];
};
const executable = resolve(option("--exe") ?? fileURLToPath(new URL(`../../../target/release/fresco-example-engine${process.platform === "win32" ? ".exe" : ""}`, import.meta.url)));
const bytes = statSync(executable).size;
const scenes = [
  { scene: "embedded_canvas", args: [] },
  { scene: "mesh", args: ["--source", fileURLToPath(new URL("../../example-engine/examples/material_parameters.fr", import.meta.url))] },
  { scene: "particles", args: ["--source", fileURLToPath(new URL("../../../examples/50) particles/drifting_sparks.fr", import.meta.url))] },
];
function measure(args, presented) {
  const start = performance.now();
  const result = spawnSync(executable, args, { cwd: tmpdir(), encoding: "utf8", windowsHide: true, timeout: 45_000 });
  const elapsed = performance.now() - start;
  if (result.error) throw result.error;
  assert.equal(result.status, 0, result.stderr || result.stdout);
  if (presented) assert.match(result.stdout, new RegExp(`Presented ${presented} frame`));
  return elapsed;
}
const results = scenes.map(({ scene, args }) => ({ scene,
  check_process_ms: measure([...args, "--check"]),
  one_frame_process_ms: Array.from({ length: 3 }, () => measure([...args, "--hidden", "--frames", "1"], 1)),
  one_hundred_twenty_frame_process_ms: measure([...args, "--hidden", "--frames", "120"], 120),
}));
const report = { measured_at: new Date().toISOString(), platform: process.platform, architecture: process.arch,
  executable, executable_bytes: bytes, working_directory: tmpdir(),
  caveats: ["Whole-process timings include compilation, device/window setup, rendering, and shutdown.",
    "These are not isolated GPU frame timings; presentation policy and warm driver caches affect results.",
    "The default canvas runs outside the checkout using only its embedded sources and assets."], scenes: results };
const json = JSON.stringify(report, null, 2) + "\n";
const output = option("--output"); if (output) writeFileSync(output, json);
console.log(json);
