import { fileURLToPath } from "node:url";
import { buildWasm } from "../../../../integrations/example-engine-host/scripts/build-wasm.mjs";

buildWasm({
  compilerOutput: fileURLToPath(new URL("../pkg", import.meta.url)),
  rendererOutput: fileURLToPath(new URL("../public/example-engine-renderer", import.meta.url)),
  release: !process.argv.includes("--dev"),
});
