import { copyFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { buildWasm } from "./build-wasm.mjs";

buildWasm({
  compilerOutput: fileURLToPath(new URL("../web/compiler", import.meta.url)),
  rendererOutput: fileURLToPath(new URL("../web/renderer", import.meta.url)),
  release: process.argv.includes("--release"),
});
copyFileSync(new URL("../demo.fr", import.meta.url), new URL("../web/demo.fr", import.meta.url));
copyFileSync(new URL("../../example-engine/examples/material_parameters.fr", import.meta.url), new URL("../web/mesh.fr", import.meta.url));
copyFileSync(new URL("../../../examples/50) particles/drifting_sparks.fr", import.meta.url), new URL("../web/particles.fr", import.meta.url));
copyFileSync(new URL("../../../examples/40) surface shaders/forward_plus.fr", import.meta.url), new URL("../web/forward_plus.fr", import.meta.url));
copyFileSync(new URL("../../../examples/40) surface shaders/deferred.fr", import.meta.url), new URL("../web/deferred.fr", import.meta.url));
console.log("Built compiler worker and shared Rust renderer. Run node integrations/example-engine-host/scripts/serve-web.mjs");
