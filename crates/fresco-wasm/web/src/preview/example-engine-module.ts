/** Load the independently built Rust renderer from the deployed site's base URL. */
type ExampleEngineModule = typeof import("../../public/example-engine-renderer/fresco_example_engine_host.js");
let pending: Promise<ExampleEngineModule> | undefined;

export function loadExampleEngine(): Promise<ExampleEngineModule> {
  if (!pending) {
    const url = new URL(
      `${import.meta.env.BASE_URL}example-engine-renderer/fresco_example_engine_host.js`,
      window.location.href,
    );
    pending = import(/* @vite-ignore */ url.href)
      .then(async (module: ExampleEngineModule) => {
        await module.default();
        return module;
      })
      .catch(error => {
        pending = undefined;
        throw new Error(`Unable to load the example engine. Run npm run wasm:build. ${String(error)}`);
      });
  }
  return pending;
}
