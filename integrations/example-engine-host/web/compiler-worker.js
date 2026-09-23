import init, { compile_fresco_bundle, example_engine_sources_json } from "./compiler/fresco_wasm.js";

// Only this worker owns the compiler WASM instance. Rendering never waits on its
// synchronous compiler call, and no playground UI module participates.
const ready = init().then(() => JSON.parse(example_engine_sources_json()));
self.onmessage = async ({ data: { id, source, renderer } }) => {
  try {
    const profile = await ready;
    if (renderer !== "forward" && renderer !== "forward-plus" && renderer !== "deferred") throw new Error("Unknown renderer");
    const configuration = `const preview_forward_plus: bool = ${renderer !== "forward"}\nconst preview_deferred: bool = ${renderer === "deferred"}\n`;
    const result = compile_fresco_bundle({ ...profile, "engine/config/renderer.fr": configuration, "main.fr": source }, "main.fr", false);
    self.postMessage({ id, result });
  } catch (error) {
    self.postMessage({ id, error: String(error) });
  }
};
ready.then(() => self.postMessage({ ready: true }), error => self.postMessage({ error: String(error) }));
