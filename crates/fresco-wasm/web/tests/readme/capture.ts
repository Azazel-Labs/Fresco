declare const __README_COMPILER_URL__: string;
declare const __README_RENDERER_URL__: string;
const { default: initWasm, compile_fresco_bundle, example_engine_sources_json }:
  typeof import("../../pkg/fresco_wasm.js") = await import(/* @vite-ignore */ __README_COMPILER_URL__);
import { stringifyManifest } from "../../src/manifest-contract";
import type { BrowserEngine } from "../../public/example-engine-renderer/fresco_example_engine_host.js";

await initWasm();
const canvas = document.querySelector<HTMLCanvasElement>("#preview")!;
let renderer: BrowserEngine;
let device: GPUDevice;
const failures: string[] = [];
function healthy() {
  if (failures.length) throw new Error(failures.join("\n"));
}

const api = {
  async init({ files, entrypoint, width, height, textures, mesh }: {
    files: Record<string, string>; entrypoint: string; width: number; height: number;
    textures: Record<string, string>; mesh: string;
  }) {
    const compiled = compile_fresco_bundle({ ...JSON.parse(example_engine_sources_json()), ...files }, entrypoint, false);
    if (!compiled.ok) throw new Error(JSON.stringify(compiled.diagnostics));
    const entries = [...compiled.manifest.canvases, ...compiled.manifest.surfaces];
    if (entries.length !== 1) throw new Error("README capture requires exactly one entry");
    const entry = entries[0];
    const assets = new Map<string, Uint8Array>();
    for (const texture of entry.textures ?? []) {
      const url = textures[texture.name];
      if (!url) throw new Error(`Missing README texture fixture: ${texture.name}`);
      const response = await fetch(url);
      if (!response.ok) throw new Error(`README texture failed: ${texture.name} (${response.status})`);
      assets.set(texture.name, new Uint8Array(await response.arrayBuffer()));
    }
    canvas.width = width;
    canvas.height = height;
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    const module: typeof import("../../public/example-engine-renderer/fresco_example_engine_host.js") =
      await import(/* @vite-ignore */ __README_RENDERER_URL__);
    await module.default();
    const gpu = navigator.gpu;
    if (!gpu) throw new Error("README capture requires WebGPU; no placeholder images are generated.");
    // Observe the engine's actual device solely for capture validation and metadata.
    // No renderer resources or shaders are created by this harness.
    const requestAdapter = gpu.requestAdapter;
    let info: GPUAdapterInfo | undefined;
    gpu.requestAdapter = async options => {
      const adapter = await requestAdapter.call(gpu, options);
      if (!adapter) return null;
      info = adapter.info;
      const requestDevice = adapter.requestDevice;
      adapter.requestDevice = async descriptor => {
        device = await requestDevice.call(adapter, descriptor);
        device.addEventListener("uncapturederror", event => failures.push(event.error.message));
        return device;
      };
      return adapter;
    };
    try {
      renderer = await module.BrowserEngine.create(canvas);
    } finally {
      gpu.requestAdapter = requestAdapter;
    }
    if (!info || !device) throw new Error("Could not observe the README engine device");
    const camera = new module.BrowserCamera();
    camera.drag(-25, 20);
    camera.zoom(Math.log(4.5 / 3) / 0.0015);
    renderer.set_camera(camera);
    camera.free();
    if (!await renderer.install_with_assets(compiled.wgsl, stringifyManifest(compiled.manifest), entry.name, assets, "{}", mesh)) {
      throw new Error("README engine installation was cancelled");
    }
    await device.queue.onSubmittedWorkDone();
    healthy();
    return { vendor: info.vendor, architecture: info.architecture,
      device: info.device, description: info.description,
      fallback: info.isFallbackAdapter, diagnostics: compiled.diagnostics };
  },
  async frame(time: number, deltaTime = 0) {
    device.pushErrorScope("validation");
    let png: string;
    try {
      if (!await renderer.render_async(time, deltaTime)) throw new Error("README GPU draw failed");
      // Copy immediately, before presentation clears the canvas backing buffer.
      const copy = document.createElement("canvas");
      copy.width = canvas.width;
      copy.height = canvas.height;
      copy.getContext("2d")!.drawImage(canvas, 0, 0);
      png = copy.toDataURL("image/png");
    } finally {
      const error = await device.popErrorScope();
      if (error) failures.push(error.message);
    }
    await device.queue.onSubmittedWorkDone();
    healthy();
    return png;
  },
};
Object.assign(window, { readmeCapture: api });
