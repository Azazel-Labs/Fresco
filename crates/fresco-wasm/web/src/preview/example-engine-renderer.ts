import { stringifyManifest } from "../manifest-contract";
import { PreviewController, paramDefChanged } from "./preview-controller";
import { engineParameterToEditor, editorParameterToEngine } from "./engine-parameter-values";
import { wrapPreviewTime } from "./controls";
import { loadExampleEngine } from "./example-engine-module";
import { normalizeManifestParamDefault } from "./renderer-support";
import { chooseManifestPreviewSurface } from "./renderer-manifest";
import type { ManifestRoot } from "../generated/wasm-contracts/ManifestRoot";
import type { BrowserEngine, BrowserCamera } from "../../public/example-engine-renderer/fresco_example_engine_host.js";

export type SurfaceMeshKind = "sphere" | "plane" | "box";
export type LightingEnvironment = "preview" | "unlit" | "directional" | "three-lights";
export type BufferView = { id: string; label: string; group: string; description: string; tiles?: string[] };

// Share only editor parameter, staging, and playback state.
// GPU preparation and drawing are entirely owned by Rust.
export class ExampleEngineRenderer extends PreviewController {
  private engine: BrowserEngine | undefined;
  private camera: BrowserCamera | undefined;
  private revision = 0;
  private sourceRevision = 0;
  private pendingParameterCount = 0;
  private framePending = false;
  private parameterUpdates: Promise<void> = Promise.resolve();
  private engineChanges: Promise<unknown> = Promise.resolve();
  private assetRequest: AbortController | undefined;

  private enqueueEngineChange<T>(operation: () => Promise<T>): Promise<T> {
    const pending = this.engineChanges.then(operation);
    // A rejected candidate must not prevent later edits or replacements.
    this.engineChanges = pending.catch(() => {});
    return pending;
  }
  private redrawRequested = false;
  private styleSymbols = new Map<string, string>();
  private installed: { wgsl: string; manifest: ManifestRoot; entry: string; assets: Map<string, Uint8Array> } | undefined;
  private size = "";
  private resizeFrame: number | undefined;
  private pageSuspended = false;
  private previewBlank = false;
  private lightingEnvironment: LightingEnvironment = "preview";
  onLightingCapabilitiesChanged: (() => void) | undefined;
  onBufferViewsChanged: (() => void) | undefined;
  bufferView = "shaded";
  get bufferViews(): BufferView[] { return JSON.parse(this.engine?.buffer_views() ?? "[]"); }
  private restoreBufferView() {
    const views = this.bufferViews;
    if (!views.some(view => view.id === this.bufferView)) this.bufferView = "shaded";
    if (views.length) this.engine!.set_buffer_view(this.bufferView);
    this.canvas.dataset.bufferView = this.bufferView;
    this.onBufferViewsChanged?.();
  }
  async setBufferView(id: string) {
    await this.enqueueEngineChange(async () => {
      if (!this.engine || !this.installed) return;
      this.engine.set_buffer_view(id);
      this.bufferView = id;
      this.canvas.dataset.bufferView = id;
      this.onBufferViewsChanged?.();
      this.drawFrame({ advanceTime: false });
    });
  }
  get supportsLightingEnvironment() { return this.engine?.supports_lighting_environment() ?? false; }

  async setLightingEnvironment(environment: LightingEnvironment) {
    this.lightingEnvironment = environment;
    await this.enqueueEngineChange(async () => {
      if (this.engine && this.installed && this.supportsLightingEnvironment) {
        this.engine.set_lighting_environment(this.lightingEnvironment);
        this.drawFrame({ advanceTime: false });
      }
    });
  }
  private fpsLastRequestMs: number | null = null;
  private fpsGeneration = 0;

  private resetFps() {
    this.fpsGeneration++;
    this.fpsFrameCount = 0;
    this.fpsSampleStartMs = performance.now();
    this.fpsLastRequestMs = null;
    if (this.deps.previewFpsEl) {
      this.deps.previewFpsEl.textContent = this.playbackPaused ? "FPS Paused" : "FPS --";
    }
  }

  setPlaybackPaused(paused: boolean) {
    super.setPlaybackPaused(paused);
    this.resetFps();
  }

  private explicitDelta = 0;
  private selectedSurface: string | null = new URLSearchParams(location.search).get("surface");
  meshKind = "sphere";
  renderMode = "mesh";
  private orbitAuto = false;
  onOrbitAutoChanged: (() => void) | undefined;
  onRuntimeRecovered: (() => void) | undefined;
  onPreviewInstalled: (() => void) | undefined;
  onMeshKindChanged: (() => void) | undefined;
  onPlaybackCapabilitiesChanged: (() => void) | undefined;

  get hasInstalledPreview() { return !this.previewBlank && Boolean(this.installed); }

  async init() {
    const module = await loadExampleEngine();
    this.camera = new module.BrowserCamera();
    this.engine = await module.BrowserEngine.create(this.canvas);
    this.engine.set_camera(this.camera);
    this.gpuReady = true;
    this.handleResize();
    this.resizeObserver = new ResizeObserver(() => {
      // Canvas dimensions affect layout; apply them after observer delivery ends.
      if (this.resizeFrame !== undefined) return;
      this.resizeFrame = requestAnimationFrame(() => {
        this.resizeFrame = undefined;
        this.handleResize();
      });
    });
    this.resizeObserver.observe(this.canvas);
    this.canvas.style.touchAction = "none";
    let pointer: { id: number; x: number; y: number } | undefined;
    this.canvas.addEventListener("pointerdown", event => {
      if (event.button !== 0 || pointer) return;
      pointer = { id: event.pointerId, x: event.clientX, y: event.clientY };
      this.canvas.setPointerCapture(event.pointerId);
    });
    this.canvas.addEventListener("pointermove", event => {
      if (!pointer || pointer.id !== event.pointerId || !this.camera || !this.engine) return;
      const dx = event.clientX - pointer.x, dy = event.clientY - pointer.y;
      if (dx === 0 && dy === 0) return;
      this.setOrbitAuto(false);
      this.camera.drag(dx, dy);
      pointer.x = event.clientX; pointer.y = event.clientY;
      this.engine.set_camera(this.camera); this.drawFrame({ advanceTime: false });
    });
    const releasePointer = (event: PointerEvent) => {
      if (pointer?.id !== event.pointerId) return;
      pointer = undefined;
      if (this.canvas.hasPointerCapture(event.pointerId)) this.canvas.releasePointerCapture(event.pointerId);
    };
    this.canvas.addEventListener("pointerup", releasePointer);
    this.canvas.addEventListener("pointercancel", releasePointer);
    this.canvas.addEventListener("lostpointercapture", event => {
      if (pointer?.id === event.pointerId) pointer = undefined;
    });
    this.canvas.addEventListener("wheel", event => {
      if (!this.camera || !this.engine) return;
      event.preventDefault();
      this.camera.zoom(event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? this.canvas.clientHeight : 1));
      this.engine.set_camera(this.camera); this.drawFrame({ advanceTime: false });
    }, { passive: false });
    addEventListener("pagehide", event => this.handlePageHide(event.persisted));
    addEventListener("pageshow", event => {
      if (event.persisted) this.handlePageRestore();
    });
  }

  handlePageHide(persisted: boolean) {
    this.pageSuspended = true;
    this.resetFps();
    this.lastTickMs = null;
    if (this.resizeFrame !== undefined) cancelAnimationFrame(this.resizeFrame);
    this.resizeFrame = undefined;
    this.resizeObserver?.disconnect();
    // A cached document resumes with its installed resources and editor state.
    if (persisted) return;
    this.clearPreview(); this.engine?.free(); this.engine = undefined;
    this.camera?.free(); this.camera = undefined;
    this.gpuReady = false;
  }

  handlePageRestore() {
    if (!this.pageSuspended || !this.engine) return;
    this.pageSuspended = false;
    this.lastTickMs = null;
    this.resizeObserver?.observe(this.canvas);
    this.handleResize();
    this.drawFrame({ advanceTime: false });
  }

  handleResize() {
    if (this.pageSuspended || !this.engine) return;
    const rect = this.canvas.getBoundingClientRect();
    const width = Math.max(0, Math.round(rect.width * devicePixelRatio));
    const height = Math.max(0, Math.round(rect.height * devicePixelRatio));
    const size = `${width}x${height}`;
    if (size === this.size) return;
    this.size = size;
    void this.engine.resize(width, height).then(() => {
      if (this.deps.previewResolutionEl) this.deps.previewResolutionEl.textContent = `${width} x ${height}`;
      this.drawFrame({ advanceTime: false });
    }).catch(error => { this.size = ""; this.reportRuntimeIssue(error); });
  }

  reportRuntimeIssue(message, label = "Rust example engine") {
    this.errorMessage = String(message?.message || message);
    this.lastRuntimeIssue = this.errorMessage;
    if (this.deps.previewStatusEl) this.deps.previewStatusEl.textContent = this.errorMessage;
    this.onRuntimeDiagnostic?.({ file: "preview", severity: "error", message: this.errorMessage, label, span_start: 0, span_end: 0 });
  }

  cancelPendingShader() {
    this.sourceRevision++;
    this.revision++; this.assetRequest?.abort(); this.engine?.cancel_pending(); this.stagedShader = null;
  }

  clearPreview() {
    this.cancelPendingShader();
    // Hide synchronously, including any old frame already in flight on the GPU.
    this.previewBlank = true;
    this.resetFps();
    this.canvas.style.visibility = "hidden";
    this.lastTickMs = null;
  }

  stageShader(wgsl: string, manifest: ManifestRoot | null = null) {
    this.sourceRevision++;
    this.revision++; this.assetRequest?.abort(); this.engine?.cancel_pending();
    this.stagedShader = { wgsl, manifest };
  }

  async setShader(wgsl: string, manifest: ManifestRoot | null = null): Promise<any> {
    if (!this.engine || !manifest) throw new Error("Rust preview requires a compiled manifest and initialized GPU.");
    const revision = this.revision;
    this.assetRequest?.abort();
    const request = new AbortController(); this.assetRequest = request;
    return this.enqueueEngineChange(async () => {
      if (revision !== this.revision) return;
      try {
        const entries = [...manifest.canvases, ...(manifest.surfaces ?? []).filter(s => s.name !== "fresco_scene_ground")];
        // Preview selection is host policy. Match the existing playground's
        // surface preference; Rust still receives an explicit entry to execute.
        const preferredSurface = chooseManifestPreviewSurface(manifest);
        const entry = this.selectedSurface ? entries.find(item => item.name === this.selectedSurface)
          : preferredSurface ?? (entries.length === 1 ? entries[0] : undefined);
        if (!entry) throw new Error("Select one entry using ?surface=name when the artifact has multiple entries.");
        const textures = (entry.textures ?? []).map(texture => {
          const defaultAsset = texture.metadata?.default_asset ?? "";
          const options = this.deps.textureOptionsForName(this.deps.getSourceExampleId?.() || "", texture.name, defaultAsset);
          const selected = this.textureSelections.get(texture.name);
          return { name: texture.name, defaultAsset, ...options, selectedUrl: options.options.some(item => item.url === selected) ? selected : options.defaultUrl };
        });
        const assets = new Map<string, Uint8Array>();
        for (const texture of textures) {
          if (!texture.selectedUrl || texture.selectedUrl.startsWith("example://")) continue;
          const response = await fetch(texture.selectedUrl, { signal: request.signal });
          if (!response.ok) throw new Error(`Texture ${texture.name}: HTTP ${response.status}`);
          assets.set(texture.name, new Uint8Array(await response.arrayBuffer()));
        }
        if (revision !== this.revision) return;
        const nextStyleSymbols = new Map("settings" in entry ? (entry.settings?.implementations ?? []).map(selection => [selection.name, selection.symbol]) : []);
        const styleParameters = "settings" in entry ? (entry.settings?.implementations ?? []).flatMap(selection =>
          (selection.parameters ?? []).map(parameter => ({ ...parameter, name: `${selection.name}.${parameter.name}` }))) : [];
        const definitions = [...(entry.params ?? []), ...styleParameters].map(param => ({ name: param.name, type: param.type, callType: param.type, paramType: "param_type" in param ? param.param_type : null, default: engineParameterToEditor(param.type, normalizeManifestParamDefault(param.default, param.type)), min: typeof param.min === "number" ? param.min : null, max: typeof param.max === "number" ? param.max : null }));
        const previous = new Map(this.paramDefs.map(def => [def.name, def]));
        const overrides = Object.fromEntries(definitions
          .filter(def => this.touchedParamNames.has(def.name) && !paramDefChanged(previous.get(def.name), def)
            && (!def.name.includes(".") || (this.installed?.entry === entry.name && this.styleSymbols.get(def.name.split(".")[0]) === nextStyleSymbols.get(def.name.split(".")[0]))))
          .map(def => [def.name, this.paramValues.get(def.name)]));
        const started = performance.now();
        const installed = await this.engine.install_with_assets(wgsl, stringifyManifest(manifest), entry.name, assets, JSON.stringify(Object.fromEntries(definitions.filter(def => def.name in overrides).map(def => [def.name, editorParameterToEngine(def.type, overrides[def.name])]))), this.meshKind);
        if (!installed || revision !== this.revision) return;
        this.installed = { wgsl, manifest, entry: entry.name, assets };
        this.resetFps();
        this.styleSymbols = nextStyleSymbols;
        this.canvas.dataset.engineEntry = entry.name;
        this.canvas.dataset.engineMesh = this.meshKind;
        this.renderMode = manifest.techniques?.some(t => t.surface === entry.name && t.metadata.engine === "particle") ? "particles" : "mesh";
        if (this.supportsLightingEnvironment) {
          this.engine.set_lighting_environment(this.lightingEnvironment);
        }
        this.onLightingCapabilitiesChanged?.();
        this.restoreBufferView();
        if (this.renderMode === "particles") {
          if (this.playbackRate < 0) this.playbackRate = 1;
          this.playbackLoopSeconds = 0;
        }
        this.onPlaybackCapabilitiesChanged?.();
        this.onOrbitAutoChanged?.();
        this.errorMessage = ""; this.lastRuntimeIssue = "";
        this.simTimeSeconds = 0; this.elapsed = 0; this.lastTickMs = null; this.deltaTimeSeconds = 0; this.explicitDelta = 0;
        this.readParameterValues(definitions);
        this.touchedParamNames = new Set(Object.keys(overrides));
        this.syncParamDefs(definitions);
        this.textureDefs = textures;
        this.textureSelections = new Map(textures.map(texture => [texture.name, texture.selectedUrl]));
        this.onTextureDefsChanged?.(textures);
        if (this.deps.previewStatusEl) this.deps.previewStatusEl.textContent = entry.name;
        const first = await this.engine.render_async(0, 0);
        if (revision !== this.revision) return;
        if (first) {
          this.previewBlank = false;
          this.canvas.style.visibility = "";
          this.onPreviewInstalled?.();
        }
        return { first_draw_ok: first, total_ms: performance.now() - started };
      } catch (error) {
        if (revision !== this.revision) return;
        throw error;
      }
    });
  }

  drawFrame(options: { advanceTime?: boolean; nowMs?: number } = {}) {
    if (this.pageSuspended || this.previewBlank || !this.engine || !this.installed) return false;
    if (this.framePending) {
      if (options.advanceTime === false) this.redrawRequested = true;
      return false;
    }
    const sampleNow = performance.now();
    if (this.fpsLastRequestMs === null || sampleNow - this.fpsLastRequestMs > 1000) this.resetFps();
    this.fpsLastRequestMs = sampleNow;
    const fpsGeneration = this.fpsGeneration;
    const now = options.nowMs ?? sampleNow;
    let delta = this.explicitDelta; this.explicitDelta = 0;
    if (options.advanceTime !== false && !this.playbackPaused) {
      if (this.lastTickMs !== null) delta += Math.max(0, Math.min((now - this.lastTickMs) / 1000, 0.25)) * this.playbackRate;
      this.simTimeSeconds = wrapPreviewTime(this.simTimeSeconds + delta, this.playbackLoopSeconds);
    }
    this.elapsed = this.simTimeSeconds;
    this.lastTickMs = now; this.deltaTimeSeconds = delta;
    if (this.orbitAuto && this.camera && delta > 0) { this.camera.drag(delta * 25, 0); this.engine.set_camera(this.camera); }
    this.framePending = true;
    const revision = this.revision;
    void this.engine.render_async(this.simTimeSeconds, delta).then(drawn => {
      if (drawn) {
        if (revision === this.revision && fpsGeneration === this.fpsGeneration && !this.playbackPaused) {
          this.fpsFrameCount++;
          const completedAt = performance.now();
          const elapsedMs = completedAt - this.fpsSampleStartMs;
          if (elapsedMs >= 500) {
            if (this.deps.previewFpsEl) {
              this.deps.previewFpsEl.textContent = `FPS ${Math.round(this.fpsFrameCount * 1000 / elapsedMs)}`;
            }
            this.fpsFrameCount = 0;
            this.fpsSampleStartMs = completedAt;
          }
        }
        this.canvas.dataset.engineFrames = String(Number(this.canvas.dataset.engineFrames || 0) + 1);
        this.canvas.dataset.engineTime = String(this.simTimeSeconds);
      }
    }).catch(error => { if (revision === this.revision) this.reportRuntimeIssue(error); })
      .finally(() => {
        this.framePending = false;
        if (this.redrawRequested) { this.redrawRequested = false; this.drawFrame({ advanceTime: false }); }
      });
    return true;
  }

  setPlaybackRate(rate) {
    if (this.renderMode === "particles" && Number(rate) < 0) { this.reportRuntimeIssue("Reverse particle playback requires reset/replay and is not supported yet."); return; }
    super.setPlaybackRate(rate);
  }
  setPlaybackTime(seconds) {
    if (this.renderMode === "particles" && Number(seconds) !== 0 && Number(seconds) !== this.simTimeSeconds) {
      this.reportRuntimeIssue("Particle seeking requires reset/replay; use time 0 to reset."); return;
    }
    if (Number(seconds) === 0) { this.engine?.reset_playback(); this.explicitDelta = 0; }
    super.setPlaybackTime(seconds);
  }
  stepPlaybackFrames(frames = 1, fps = 60) {
    if (this.renderMode === "particles" && frames < 0) { this.reportRuntimeIssue("Reverse particle stepping requires reset/replay."); return; }
    super.stepPlaybackFrames(frames, fps); this.explicitDelta += this.deltaTimeSeconds;
  }
  setPlaybackLoopDuration(seconds) {
    if (this.renderMode === "particles" && Number(seconds) > 0) { this.reportRuntimeIssue("Particle looping requires reset/replay and is not supported yet."); return; }
    super.setPlaybackLoopDuration(seconds);
  }

  private readParameterValues(definitions) {
    const values = JSON.parse(this.engine!.parameter_values_json());
    this.paramValues = new Map(definitions.map(def => [def.name, engineParameterToEditor(def.type, values[def.name])]));
  }

  setParamValue(def, value) {
    if (!this.engine) return;
    const engine = this.engine;
    const revision = this.sourceRevision;
    const normalized = this.normalizeParamValue(def, value);
    const displayedDefinitions = this.paramDefs;
    // The engine applies atomic candidates, so overlapping partial updates would
    // cancel one another. Preserve event order across different controls.
    this.pendingParameterCount++;
    this.parameterUpdates = this.enqueueEngineChange(async () => {
      if (revision !== this.sourceRevision || engine !== this.engine) return;
      const applied = await engine.update_parameters(JSON.stringify({ [def.name]: editorParameterToEngine(def.type, normalized) }));
      if (!applied || revision !== this.sourceRevision || engine !== this.engine) return;
      const previous = this.paramValues.get(def.name);
      this.readParameterValues(this.paramDefs);
      this.touchedParamNames.add(def.name);
      if (displayedDefinitions !== this.paramDefs) this.onParamDefsChanged?.(this.paramDefs);
      this.onParamValueChanged?.({ name: def.name, value: this.paramValues.get(def.name), previous });
      this.drawFrame({ advanceTime: false });
    }).catch(error => { if (revision === this.sourceRevision) this.reportRuntimeIssue(error); })
      .finally(() => { this.pendingParameterCount--; });
  }
  async setTextureSelection(name, url) {
    if (!this.installed) return;
    const previous = this.textureSelections.get(name);
    this.textureSelections.set(name, url);
    const replacement = this.reinstall();
    const revision = this.revision;
    try {
      if (await replacement && revision === this.revision) {
        await this.parameterUpdates;
        if (revision !== this.revision) return;
        this.textureDefs = this.textureDefs.map(def => ({ ...def, selectedUrl: this.textureSelections.get(def.name) }));
        this.onTextureDefsChanged?.(this.textureDefs);
      }
    } catch (error) {
      if (revision !== this.revision) return;
      if (previous === undefined) this.textureSelections.delete(name); else this.textureSelections.set(name, previous);
      this.onTextureDefsChanged?.(this.textureDefs);
      this.reportRuntimeIssue(error);
    }
  }
  private async reinstall(mesh = this.meshKind): Promise<boolean> {
    if (!this.installed || !this.engine) return false;
    const { wgsl, manifest, entry, assets } = this.installed;
    const revision = ++this.revision;
    // Source changes invalidate edits; resource changes retain their queue order.
    if (this.pendingParameterCount === 0) this.engine.cancel_pending();
    this.assetRequest?.abort();
    const request = new AbortController(); this.assetRequest = request;
    const selections = new Map(this.textureSelections);
    return this.enqueueEngineChange(async () => {
      if (revision !== this.revision) return false;
      try {
        const replacements = new Map(assets);
        const textureNames = new Set(this.textureDefs.map(texture => texture.name));
        for (const [name, url] of selections) {
          if (!textureNames.has(name)) continue;
          if (!url || url.startsWith("example://")) { replacements.delete(name); continue; }
          const response = await fetch(url, { signal: request.signal }); if (!response.ok) throw new Error(`Texture ${name}: HTTP ${response.status}`);
          replacements.set(name, new Uint8Array(await response.arrayBuffer()));
        }
        if (revision !== this.revision) return false;
        const values = this.engine.parameter_values_json();
        if (await this.engine.install_with_assets(wgsl, stringifyManifest(manifest), entry, replacements, values, mesh) && revision === this.revision) {
          if (this.supportsLightingEnvironment) this.engine.set_lighting_environment(this.lightingEnvironment);
          this.restoreBufferView();
          this.installed = { wgsl, manifest, entry, assets: replacements };
          this.meshKind = mesh;
          this.canvas.dataset.engineMesh = mesh;
          this.onMeshKindChanged?.();
          const recovered = Boolean(this.errorMessage);
          this.errorMessage = ""; this.lastRuntimeIssue = "";
          this.canvas.dataset.engineInputRevision = String(revision);
          if (this.deps.previewStatusEl) this.deps.previewStatusEl.textContent = entry;
          if (recovered) this.onRuntimeRecovered?.();
          this.drawFrame({ advanceTime: false });
          return true;
        }
        return false;
      } catch (error) {
        if (revision !== this.revision) return false;
        throw error;
      }
    });
  }

  async setMeshKind(kind: string) {
    const candidate = kind === "cube" ? "box" : kind;
    if (!this.installed) { this.meshKind = candidate; this.onMeshKindChanged?.(); return; }
    const replacement = this.reinstall(candidate);
    const revision = this.revision;
    try { await replacement; }
    catch (error) { if (revision === this.revision) this.reportRuntimeIssue(error); }
  }
  setOrbitAuto(value: boolean) { this.orbitAuto = value; this.onOrbitAutoChanged?.(); }
  getOrbitAuto() { return this.orbitAuto; }
  setActiveSurfaceName(name: string | null | undefined) { this.selectedSurface = name ?? null; if (this.installed) this.stageShader(this.installed.wgsl, this.installed.manifest); }
}
