// CPU-only package check. GPU execution remains a separate local test.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
const directory = process.argv[2] ? pathToFileURL(path.resolve(process.argv[2]) + path.sep) : new URL("../web/renderer/", import.meta.url);
const { initSync, BrowserCamera, BrowserEngine } = await import(new URL("fresco_example_engine_host.js", directory).href);
initSync({ module: readFileSync(new URL("fresco_example_engine_host_bg.wasm", directory)) });
for (const method of ["set_point_lights", "install_with_options", "install_with_assets", "render_async", "update_parameters", "resize", "reset_playback", "cancel_pending", "particle_state_bytes", "particle_slots_json"]) {
  assert.equal(typeof BrowserEngine.prototype[method], "function", `missing renderer method ${method}`);
}
const camera = new BrowserCamera();
try {
  camera.drag(10, -5);
  camera.zoom(20);
  assert.throws(() => camera.drag(Number.NaN, 0));
  camera.reset();
} finally { camera.free(); }
console.log("Renderer WASM package passed: browser API and CPU camera lifecycle.");
