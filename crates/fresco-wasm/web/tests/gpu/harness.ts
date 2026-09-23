import initWasm, { compile_fresco_bundle } from "../../pkg/fresco_wasm.js";
import { RustEngineRenderer } from "./rust-engine-renderer";
import { buildCompileFilesBundle } from "../../src/runtime-content-registry";
import badge from "../../src/examples/10) fundamentals/badge.fr?raw";
import flipCard from "../../src/examples/90) gallery/perspective_flip_card.fr?raw";
import particleFixture from "../../../../../tests/fixtures/particle_contract.fr?raw";
import canaryEngine from "../../../../../tests/canaries/engine.fr?raw";
import canaryFrame from "../../../../../tests/canaries/frame.fr?raw";
import canaryPalette from "../../../../../tests/canaries/palette.fr?raw";
import canarySource from "../../../../../tests/canaries/main.fr?raw";

// This harness uses the production compiler, engine bundle, renderers and GPU.
// Only time and frame scheduling are controlled by the test.
const canvas = document.querySelector<HTMLCanvasElement>("#preview")!;
let renderer: RustEngineRenderer;
const failures: string[] = [];
let installedArtifact: { wgsl: string; manifest: Parameters<RustEngineRenderer["setShader"]>[1] } | undefined;
const frames = new Map<string, { pixels: Uint8ClampedArray; png: string; width: number; height: number }>();
await initWasm();

function requireHealthy() {
  if (failures.length) throw new Error(failures.join("\n"));
  if (renderer.errorMessage) throw new Error(renderer.errorMessage);
}

const api = {
  setLightingEnvironment(value: string) { renderer.setLightingEnvironment(value); },
  setPointLights(lights: unknown[]) { renderer.setPointLights(lights); },

  async initCanary() {
    const compiled = compile_fresco_bundle({
      "engine/engine.fr": canaryEngine,
      "engine/frame.fr": canaryFrame,
      "palette.fr": canaryPalette,
      "main.fr": canarySource,
      "engine/unreachable.fr": "deliberately invalid unused source",
    }, "main.fr", false);
    if (!compiled.ok) throw new Error(JSON.stringify(compiled.diagnostics));
    renderer = new RustEngineRenderer(canvas);
    renderer.onRuntimeDiagnostic = diag => failures.push(diag.message);
    await renderer.init();
    requireHealthy();
    if (!renderer.device) throw new Error("WebGPU is required; canaries never silently skip.");
    renderer.setPlaybackPaused(true);
    await renderer.setShader(compiled.wgsl, compiled.manifest);
    installedArtifact = compiled;
    await renderer.device.queue.onSubmittedWorkDone();
    requireHealthy();
    return compiled.manifest;
  },

  setCanaryParam(name: string, value: number | number[]) {
    const def = renderer.paramDefs.find(def => def.name === name);
    if (!def) throw new Error(`Missing canary parameter: ${name}`);
    renderer.setParamValue(def, value);
  },

  pixel(name: string, x: number, y: number, width: number) {
    const frame = frames.get(name);
    if (!frame) throw new Error(`Missing frame: ${name}`);
    const offset = (y * width + x) * 4;
    if (offset < 0 || offset + 4 > frame.pixels.length) throw new Error("Pixel outside frame");
    return Array.from(frame.pixels.slice(offset, offset + 4));
  },

  async init(kind: "badge" | "flip-card" | "canvas" | "surface" | "orbit" | "mesh-default" | "mesh-fragment-edited" | "mesh-vertex-edited" | "particles" | "particle-sim-edited" | "particle-shade-edited" | "pass-default" | "pass-edited" | "pass-vertex-edited" | "pass-variant-low" | "pass-variant-high" | "pass-variant-single", sourceOverride?: string, particleContractOverride?: string, engineRenames?: Record<string, string>, rendererMode?: "forward" | "forward-plus" | "deferred") {
    const adapter = await navigator.gpu?.requestAdapter();
    if (!adapter) throw new Error("A WebGPU adapter is required; this test must not silently skip.");
    const info = adapter.info;
    const adapterDetails = {
      vendor: info.vendor, architecture: info.architecture,
      device: info.device, description: info.description,
      fallback: info.isFallbackAdapter,
    };
    if (adapterDetails.fallback || /swiftshader|llvmpipe|software|warp/i.test(JSON.stringify(adapterDetails))) {
      throw new Error(`Hardware GPU required: ${JSON.stringify(adapterDetails)}`);
    }
    const probe = "rgba(time() / 4.0, delta_time(), resolution().x / 1024.0, 1.0)";
    const builtinSource = kind === "badge" ? badge
      : kind === "flip-card" ? flipCard
      : kind === "pass-default" || kind === "pass-edited" || kind === "pass-vertex-edited" || kind === "pass-variant-low" || kind === "pass-variant-high" || kind === "pass-variant-single"
        ? "canvas pass_probe(ctx: CanvasContext) -> color { let uv = context(coord); compose { fill(rgba(uv.x, uv.y, 0.25, 1.0)) } }"
      : kind === "canvas" ? `canvas probe(ctx: CanvasContext) -> color { compose { fill(${probe}) } }`
      : kind === "surface" ? `surface probe(sp: surf) -> material(unlit) { compose { base(albedo: ${probe}) } }`
      : kind === "mesh-default" || kind === "mesh-fragment-edited" || kind === "mesh-vertex-edited"
        ? `surface mesh_probe(sp: surf) -> material(unlit) { compose { base(albedo: rgba(0.8, 0.2, 0.1, 1.0)) } }`
      : kind === "particles" || kind === "particle-sim-edited" || kind === "particle-shade-edited"
        ? `surface particle_probe(sp: surf) -> material(unlit) { compose { base(albedo: rgba(0.9, 0.35, 0.1, 1.0)) } }`
      : `surface orbit_probe(sp: surf) -> material(unlit) { compose { base(albedo: rgba(0.2 + 0.6 * sp.uv.x, 0.2 + 0.6 * sp.uv.y, 0.4, 1.0)) } }`;
    const source = sourceOverride ?? builtinSource;
    const bundle = buildCompileFilesBundle(new Map([["main.fr", source]]), "gpu-regression", rendererMode);
    if (!sourceOverride && ["particles", "particle-sim-edited", "particle-shade-edited"].includes(kind)) bundle.set("engine/core/06_particle_contract.fr", particleFixture);
    if (particleContractOverride !== undefined) bundle.set("engine/core/06_particle_contract.fr", particleContractOverride);
    if (kind === "pass-edited") {
      const contractPath = "engine/core/04_canvas_contract.fr";
      const contract = bundle.get(contractPath)!;
      const edited = contract.replace(
        "return t.draw(ctx)",
        "return t.draw(CanvasContext(uv: vec2(0.25, 0.75), frame: frame))",
      );
      if (edited === contract) throw new Error("Authored shade-hook edit did not match engine source.");
      bundle.set(contractPath, edited);
    }
    if (kind === "pass-vertex-edited") {
      const contractPath = "engine/core/04_canvas_contract.fr";
      const contract = bundle.get(contractPath)!;
      const edited = contract.replace(
        "uv: p * 0.5 + vec2(0.5, 0.5)",
        "uv: vec2(0.125, 0.875)",
      );
      if (edited === contract) throw new Error("Authored vertex-hook edit did not match engine source.");
      bundle.set(contractPath, edited);
    }
    if (kind === "pass-variant-low" || kind === "pass-variant-high" || kind === "pass-variant-single") {
      const contractPath = "engine/core/04_canvas_contract.fr";
      const contract = bundle.get(contractPath)!;
      const withAxis = contract.replace(
        "    binding {",
        "    permutations {\n        @known(compile) quality: \"low\" | \"high\"\n    }\n\n    binding {",
      );
      const edited = withAxis.replace(
        "        return t.draw(ctx)",
        "        if quality == \"low\" {\n            return t.draw(CanvasContext(uv: vec2(0.25, 0.75), frame: frame))\n        } else {\n            return t.draw(ctx)\n        }",
      );
      if (edited === contract || edited === withAxis) {
        throw new Error("Compile-known authored-pass edit did not match engine source.");
      }
      bundle.set(contractPath, kind === "pass-variant-single"
        ? edited.replace('quality: "low" | "high"', 'quality: "low"') : edited);
    }
    if (kind === "mesh-fragment-edited") {
      const contractPath = "engine/core/05_mesh_contract.fr";
      const contract = bundle.get(contractPath)!;
      const edited = contract.replace(
        "return evaluate_schema(surf(sp.uv, sp.uv2, scene.time, scene.res, sp.world_pos, sp.world_normal), m)",
        "return rgba(0.1, 0.7, 0.9, 1.0)",
      );
      if (edited === contract) throw new Error("Authored mesh shade-hook edit did not match engine source.");
      bundle.set(contractPath, edited);
    }
    if (kind === "mesh-vertex-edited") {
      const contractPath = "engine/core/05_mesh_contract.fr";
      const contract = bundle.get(contractPath)!;
      const edited = contract.replace(
        "let local_position = v.position + normalize(v.normal) * displacement_offset",
        "let local_position = v.position * 0.5 + normalize(v.normal) * displacement_offset",
      );
      if (edited === contract) throw new Error("Authored mesh vertex-hook edit did not match engine source.");
      bundle.set(contractPath, edited);
    }
    if (kind === "particle-sim-edited") {
      const contractPath = "engine/core/06_particle_contract.fr";
      const contract = bundle.get(contractPath)!;
      const edited = contract.replace(
        "return particle_integrate(p, dt)",
        "return particle_integrate(p, dt * 4.0)",
      );
      if (edited === contract) throw new Error("Authored particle simulation edit did not match engine source.");
      bundle.set(contractPath, edited);
    }
    if (kind === "particle-shade-edited") {
      const contractPath = "engine/core/06_particle_contract.fr";
      const contract = bundle.get(contractPath)!;
      const particlePassStart = contract.indexOf("@shader pass preview_particle_draw");
      const before = contract.slice(0, particlePassStart);
      const particlePass = contract.slice(particlePassStart);
      const editedPass = particlePass.replace(
        "return m.albedo",
        "return rgba(0.1, 0.75, 0.95, 1.0)",
      );
      if (editedPass === particlePass) throw new Error("Authored particle shade edit did not match engine source.");
      bundle.set(contractPath, before + editedPass);
    }
    if (engineRenames) {
      for (const [path, text] of bundle) {
        bundle.set(path, text.replace(/\b[A-Za-z_]\w*\b/g, word => engineRenames[word] ?? word));
      }
    }
    const compiled = compile_fresco_bundle(Object.fromEntries(bundle), "main.fr", false);
    if (!compiled.ok) throw new Error(JSON.stringify(compiled.diagnostics));
    renderer?.dispose();
    renderer = new RustEngineRenderer(canvas);
    renderer.onRuntimeDiagnostic = diag => failures.push(diag.message);
    await renderer.init();
    requireHealthy();
    if (renderer instanceof RustEngineRenderer) {
      if (kind === "particles" || kind === "particle-sim-edited" || kind === "particle-shade-edited") {
        renderer.setRenderMode("particles");
      }
      renderer.setMeshKind("box");
      renderer.setOrbitAuto(kind === "orbit");
      // Keep every box edge in view through both zoom directions.
      if (kind === "orbit") renderer.orbitDistance = 4.5;
    }
    renderer.setPlaybackPaused(kind !== "orbit");
    if (kind === "pass-variant-low" || kind === "pass-variant-high") {
      renderer.setEnginePassVariant({ quality: kind === "pass-variant-low" ? "low" : "high" });
    }
    await renderer.setShader(compiled.wgsl, compiled.manifest);
    installedArtifact = compiled;
    await renderer.device.queue.onSubmittedWorkDone();
    requireHealthy();
    return { adapter: adapterDetails, diagnostics: compiled.diagnostics,
      enginePass: compiled.manifest.canvases[0]?.engine_pass || null,
      meshPass: compiled.manifest.surfaces[0]?.mesh_passes?.[0] || null,
      technique: compiled.manifest.techniques.find(t => t.metadata.engine === "particle") || null,
      simulation: compiled.manifest.gpu_programs.find(p => p.metadata.capacity) || null,
      uniforms: (compiled.manifest.canvases[0] || compiled.manifest.surfaces[0]).global_uniforms };
  },

  // Generic authored-module replacement; failed compilation never reaches installation.
  async installBundle(files: Record<string, string>, rendererMode: string, entry: string) {
    const bundle = buildCompileFilesBundle(new Map(Object.entries(files)), "gpu-regression", rendererMode);
    const compiled = compile_fresco_bundle(Object.fromEntries(bundle), "main.fr", false);
    if (!compiled.ok) return { ok: false, diagnostics: compiled.diagnostics };
    await renderer.setShader(compiled.wgsl, compiled.manifest, undefined, entry);
    installedArtifact = compiled;
    await renderer.device.queue.onSubmittedWorkDone();
    requireHealthy();
    return { ok: true, manifest: compiled.manifest };
  },

  async selectVariant(selection: Record<string, string> | null) {
    if (!installedArtifact || !installedArtifact.manifest?.canvases?.length) throw new Error("Canvas artifact required");
    renderer.setEnginePassVariant(selection);
    await renderer.setShader(installedArtifact.wgsl, installedArtifact.manifest);
    requireHealthy();
  },

  async frame(name: string, time: number, delta = 0, width = 256, height = 256, draws = 1) {
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    await renderer.handleResize();
    // Seek resets dt; stepping back up to the requested time supplies exact dt.
    renderer.setPlaybackTime(time - delta);
    if (delta) renderer.stepPlaybackFrames(1, 1 / delta);
    renderer.device.pushErrorScope("validation");
    for (let i = 0; i < draws; i++) {
      if (i > 0 && delta && (renderer instanceof RustEngineRenderer) && renderer.computePipeline) renderer.stepPlaybackFrames(1, 1 / delta);
      if (!await renderer.drawFrame({ advanceTime: false })) {
        requireHealthy();
        throw new Error("GPU draw failed without a renderer diagnostic");
      }
    }
    // Read the rendered canvas in the same task, before the next presentation.
    const copy = document.createElement("canvas");
    copy.width = canvas.width;
    copy.height = canvas.height;
    const ctx = copy.getContext("2d")!;
    ctx.drawImage(canvas, 0, 0);
    const pixels = ctx.getImageData(0, 0, copy.width, copy.height).data;
    const png = copy.toDataURL("image/png");
    const error = await renderer.device.popErrorScope();
    if (error) throw new Error(error.message);
    await renderer.device.queue.onSubmittedWorkDone();
    requireHealthy();
    frames.set(name, { pixels, png, width: copy.width, height: copy.height });
    const center = (Math.floor(copy.height / 2) * copy.width + Math.floor(copy.width / 2)) * 4;
    return { center: Array.from(pixels.slice(center, center + 4)), width: copy.width, height: copy.height,
      orbitAuto: (renderer instanceof RustEngineRenderer) ? renderer.orbitAuto : null,
      distance: (renderer instanceof RustEngineRenderer) ? renderer.orbitDistance : null };
  },

  mesh(kind: "plane" | "box" | "sphere") {
    if (!(renderer instanceof RustEngineRenderer)) throw new Error("Surface renderer required");
    renderer.setMeshKind(kind);
  },

  camera(yaw: number, pitch: number) {
    if (!(renderer instanceof RustEngineRenderer)) throw new Error("Surface renderer required");
    renderer.orbitYaw = yaw;
    renderer.orbitPitch = pitch;
  },

  particleSlots() { return renderer.particleSlots(); },
  async particleBytes() { return renderer.particleBytes(); },

  async particleStep(dt: number, draws = 1, reset = false) {
    if (!(renderer instanceof RustEngineRenderer)) throw new Error("Particle renderer required");
    if (reset) renderer.resetParticles();
    if (dt > 0) renderer.stepPlaybackFrames(1, 1 / dt);
    for (let i = 0; i < draws; i++) {
      if (!await renderer.drawFrame({ advanceTime: false })) throw new Error("Particle draw failed");
    }
    await renderer.device.queue.onSubmittedWorkDone();
    requireHealthy();
  },

  // Diagnostic shader replacement permits before/after captures without saved
  // image baselines. Normal regression tests compile from current sources.
  async replaceShader(wgsl: string, manifest: Parameters<RustEngineRenderer["setShader"]>[1]) {
    await renderer.setShader(wgsl, manifest);
    await renderer.device.queue.onSubmittedWorkDone();
    requireHealthy();
  },

  downsample(name: string, source: string, scale: number) {
    const frame = frames.get(source)!;
    const width = frame.width / scale;
    const height = frame.height / scale;
    if (!Number.isInteger(width) || !Number.isInteger(height) || scale < 1) {
      throw new Error("Reference dimensions must be a whole multiple of the output");
    }
    const pixels = new Uint8ClampedArray(width * height * 4);
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        for (let c = 0; c < 4; c++) {
          let sum = 0;
          for (let dy = 0; dy < scale; dy++) {
            for (let dx = 0; dx < scale; dx++) {
              sum += frame.pixels[((y * scale + dy) * frame.width + x * scale + dx) * 4 + c];
            }
          }
          pixels[(y * width + x) * 4 + c] = sum / (scale * scale);
        }
      }
    }
    const copy = document.createElement("canvas");
    copy.width = width;
    copy.height = height;
    copy.getContext("2d")!.putImageData(new ImageData(pixels, width, height), 0, 0);
    frames.set(name, { pixels, width, height, png: copy.toDataURL("image/png") });
  },

  stripeError(actual: string, reference: string) {
    const a = frames.get(actual)!;
    const b = frames.get(reference)!;
    if (a.width !== b.width || a.height !== b.height) throw new Error("Compare equal-sized frames");
    let error = 0;
    let count = 0;
    // Include the stripes and their surrounding card, excluding background.
    for (let i = 0; i < b.pixels.length; i += 4) {
      if (b.pixels[i + 1] > 85 && b.pixels[i + 2] > 75) {
        error += Math.abs(a.pixels[i] - b.pixels[i]);
        count++;
      }
    }
    if (!count) throw new Error("Reference contains no visible card");
    return { meanRedError: error / count, cardPixels: count };
  },

  compare(a: string, b: string) {
    const first = frames.get(a)!.pixels;
    const second = frames.get(b)!.pixels;
    if (first.length !== second.length) throw new Error("Compare equal-sized frames");
    let changed = 0;
    let maxDifference = 0;
    for (let i = 0; i < first.length; i += 4) {
      const d = Math.max(...[0, 1, 2].map(c => Math.abs(first[i + c] - second[i + c])));
      maxDifference = Math.max(maxDifference, d);
      if (d > 6) changed++;
    }
    return { changedFraction: changed / (first.length / 4), maxDifference };
  },
  pixels(name: string) { return Array.from(frames.get(name)!.pixels); },
  png(name: string) { return frames.get(name)!.png; },
  distance() { return (renderer instanceof RustEngineRenderer) ? renderer.orbitDistance : null; },
  pause() { renderer.setPlaybackPaused(true); },
};

Object.assign(window, { gpuTest: api });
