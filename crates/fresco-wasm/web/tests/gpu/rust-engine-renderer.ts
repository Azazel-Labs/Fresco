import { PreviewController } from "../../src/preview/preview-controller";
import { loadExampleEngine } from "../../src/preview/example-engine-module";
import { stringifyManifest } from "../../src/manifest-contract";
import { editorParameterToEngine } from "../../src/preview/engine-parameter-values";
import type { ManifestRoot } from "../../src/generated/wasm-contracts/ManifestRoot";
import type { BrowserEngine, BrowserCamera } from "../../public/example-engine-renderer/fresco_example_engine_host.js";

/** Test scheduling and device observation around the production Rust renderer. */
export class RustEngineRenderer extends PreviewController {
  constructor(canvas: HTMLCanvasElement) {
    super(canvas, { textureOptionsForName: () => { throw new Error("Rust engine test requires explicit texture fixtures"); } });
  }

  device!: GPUDevice;
  private engine!: BrowserEngine;
  private camera!: BrowserCamera;
  private installed: { wgsl: string; manifest: ManifestRoot; entry: string } | undefined;
  private readonly events = new AbortController();
  private meshKind = "box";
  private lightingEnvironment = "preview";
  renderMode = "mesh";
  orbitYaw = 0.25;
  orbitPitch = 0.2;
  orbitDistance = 2.5;
  orbitAuto = false;
  get computePipeline() { return this.renderMode === "particles"; }
  private variant: Record<string, string> | null = null;
  private pending: Promise<unknown> = Promise.resolve();

  async init() {
    const module = await loadExampleEngine();
    const gpu = navigator.gpu;
    if (!gpu) throw new Error("Rust GPU tests require WebGPU");
    const requestAdapter = gpu.requestAdapter;
    gpu.requestAdapter = async options => {
      const adapter = await requestAdapter.call(gpu, options);
      if (!adapter) return null;
      const requestDevice = adapter.requestDevice;
      adapter.requestDevice = async descriptor => {
        this.device = await requestDevice.call(adapter, descriptor);
        this.device.addEventListener("uncapturederror", event => {
          this.errorMessage = event.error.message;
        });
        return this.device;
      };
      return adapter;
    };
    try { this.engine = await module.BrowserEngine.create(this.canvas); }
    finally { gpu.requestAdapter = requestAdapter; }
    if (!this.device) throw new Error("Could not observe the Rust engine device");
    this.camera = new module.BrowserCamera();
    this.canvas.addEventListener("wheel", event => {
      event.preventDefault();
      this.orbitDistance = Math.max(1.1, Math.min(8, this.orbitDistance * Math.exp(event.deltaY * 0.0015)));
    }, { passive: false, signal: this.events.signal });
    this.gpuReady = true;
    await this.handleResize();
  }

  async handleResize() {
    const rect = this.canvas.getBoundingClientRect();
    await this.engine.resize(Math.round(rect.width * devicePixelRatio), Math.round(rect.height * devicePixelRatio));
  }

  async setShader(wgsl: string, manifest: ManifestRoot | null = null, parameters?: Record<string, unknown>, entryName?: string) {
    const entries = manifest ? [...manifest.canvases, ...manifest.surfaces] : [];
    const candidates = entryName ? entries.filter(entry => entry.name === entryName)
      : entries.filter(entry => entry.name !== "fresco_scene_ground");
    if (!manifest || candidates.length !== 1) throw new Error("Rust test adapter requires an unambiguous entry");
    const entry = candidates[0];
    if (!await this.engine.install_with_options(wgsl, stringifyManifest(manifest), entry.name, new Map(),
      JSON.stringify({ variant: this.variant, mesh: this.meshKind, parameters }))) {
      throw new Error("Rust test shader installation was cancelled");
    }
    this.installed = { wgsl, manifest, entry: entry.name };
    if (this.engine.supports_lighting_environment()) this.engine.set_lighting_environment(this.lightingEnvironment);
    this.deltaTimeSeconds = 0;
    this.renderMode = manifest.techniques?.some(t => t.surface === entry.name && t.metadata.engine === "particle") ? "particles" : "mesh";
    this.paramDefs = (entry.params ?? []).map(param => ({ ...param, callType: param.type }));
  }

  setLightingEnvironment(value: string) { this.lightingEnvironment = value; this.engine.set_lighting_environment(value); }

  setPointLights(lights: unknown[]) { this.engine.set_point_lights(JSON.stringify(lights)); }

  setMeshKind(kind: string) {
    this.meshKind = kind;
    if (this.installed) {
      this.pending = this.pending.then(() => {
        const { wgsl, manifest, entry } = this.installed!;
        return this.setShader(wgsl, manifest, JSON.parse(this.engine.parameter_values_json()), entry);
      });
    }
  }

  setRenderMode(mode: string) { this.renderMode = mode; }
  setOrbitAuto(enabled: boolean) { this.orbitAuto = enabled; }

  setPlaybackTime(seconds) {
    super.setPlaybackTime(seconds);
    if (this.computePipeline && Number(seconds) === 0) this.engine.reset_playback();
  }

  resetParticles() { this.engine.reset_playback(); }
  particleSlots() { return JSON.parse(this.engine.particle_slots_json()); }
  async particleBytes() {
    await this.pending;
    return Array.from(await this.engine.particle_state_bytes() as Uint8Array);
  }

  setParamValue(def, value) {
    this.pending = this.pending.then(() => this.engine.update_parameters(JSON.stringify({
      [def.name]: editorParameterToEngine(def.type, value),
    })));
  }

  async drawFrame(_options?: unknown) {
    await this.pending;
    // Each requested draw is one deterministic orbit tick, independent of wall time.
    if (this.orbitAuto && !this.playbackPaused) this.orbitYaw += 0.003 * this.playbackRate;
    this.camera.reset();
    this.camera.drag(-this.orbitYaw / 0.01, this.orbitPitch / 0.01);
    this.camera.zoom(Math.log(this.orbitDistance / 3) / 0.0015);
    this.engine.set_camera(this.camera);
    const drawn = await this.engine.render_async(this.simTimeSeconds, this.deltaTimeSeconds);
    if (this.computePipeline) this.deltaTimeSeconds = 0;
    return drawn;
  }

  setEnginePassVariant(selection: Record<string, string> | null) {
    this.variant = selection ? { ...selection } : null;
  }

  dispose() { this.events.abort(); this.camera.free(); this.engine.free(); this.device.destroy(); }
}
