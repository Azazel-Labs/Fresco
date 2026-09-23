import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ExampleEngineRenderer } from "../preview/example-engine-renderer";

function deferred() { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; }
function renderer() {
  const result = new ExampleEngineRenderer({ dataset: {}, style: {} }, { textureOptionsForName: () => ({ options: [], defaultUrl: "" }) });
  result.gpuReady = true;
  result.installed = { wgsl: "shader", manifest: { canvases: [] }, entry: "probe", assets: new Map() };
  result.engine = { render_async: vi.fn().mockResolvedValue(true), buffer_views: () => "[]", set_buffer_view: vi.fn(), supports_lighting_environment: () => true, set_lighting_environment: vi.fn(), cancel_pending: vi.fn(), reset_playback: vi.fn(), install_with_assets: vi.fn().mockResolvedValue(true), parameter_values_json: () => "{}" };
  return result;
}
beforeEach(() => vi.stubGlobal("location", { search: "" }));
afterEach(() => vi.unstubAllGlobals());

describe("Rust playground adapter", () => {
  it("reports completed frames per wall-clock second, excluding skipped and failed draws", async () => {
    let now = 0;
    vi.stubGlobal("performance", { now: () => now });
    const host = renderer();
    host.deps.previewFpsEl = { textContent: "FPS --" };
    host.setPlaybackRate(4);
    const frame = deferred();
    host.engine.render_async.mockReturnValueOnce(frame.promise);
    host.drawFrame();
    now = 100;
    expect(host.drawFrame()).toBe(false);
    frame.resolve(true);
    await host.engine.render_async.mock.results[0].value;
    await Promise.resolve(); await Promise.resolve();
    now = 200;
    host.engine.render_async.mockResolvedValueOnce(false);
    host.drawFrame();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    now = 300;
    host.engine.render_async.mockRejectedValueOnce(new Error("draw failed"));
    host.drawFrame();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    now = 500;
    host.drawFrame();
    await Promise.resolve();
    expect(host.deps.previewFpsEl.textContent).toBe("FPS 4");
  });

  it("resets FPS across pause, resume, idle gaps, and clearing the preview", async () => {
    let now = 0;
    vi.stubGlobal("performance", { now: () => now });
    const host = renderer();
    host.deps.previewFpsEl = { textContent: "FPS 60" };
    const frame = deferred();
    host.engine.render_async.mockReturnValueOnce(frame.promise);
    host.drawFrame();
    host.setPlaybackPaused(true);
    now = 5000;
    frame.resolve(true);
    await frame.promise;
    await Promise.resolve(); await Promise.resolve();
    expect(host.deps.previewFpsEl.textContent).toBe("FPS Paused");
    host.drawFrame({ advanceTime: false });
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(host.deps.previewFpsEl.textContent).toBe("FPS Paused");
    host.setPlaybackPaused(false);
    expect(host.deps.previewFpsEl.textContent).toBe("FPS --");
    host.drawFrame();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    now = 5500;
    host.drawFrame();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(host.deps.previewFpsEl.textContent).toBe("FPS 4");
    now = 10000;
    host.drawFrame();
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    expect(host.deps.previewFpsEl.textContent).toBe("FPS --");
    host.clearPreview();
    expect(host.deps.previewFpsEl.textContent).toBe("FPS --");
    expect(host.fpsFrameCount).toBe(0);
  });

  it("switches engine buffer views live and restores them after replacement", async () => {
    const host = renderer();
    host.engine.buffer_views = () => JSON.stringify([{ id: "shaded" }, { id: "normals" }]);
    host.onBufferViewsChanged = vi.fn();
    await host.setBufferView("normals");
    expect(host.onBufferViewsChanged).toHaveBeenCalledOnce();
    host.onBufferViewsChanged.mockClear();
    expect(host.engine.set_buffer_view).toHaveBeenLastCalledWith("normals");
    expect(host.engine.install_with_assets).not.toHaveBeenCalled();
    await host.setShader("replacement", multiSurface());
    expect(host.engine.set_buffer_view).toHaveBeenLastCalledWith("normals");
    expect(host.onBufferViewsChanged).toHaveBeenCalledOnce();
    host.engine.buffer_views = () => JSON.stringify([{ id: "shaded" }, { id: "tiles" }]);
    await host.setShader("another renderer", multiSurface());
    expect(host.bufferView).toBe("shaded");
    expect(host.engine.set_buffer_view).toHaveBeenLastCalledWith("shaded");
    host.engine.buffer_views = () => "[]";
    await host.setShader("canvas", { canvases: [{ name: "canvas" }] });
    expect(host.bufferViews).toEqual([]);
  });
  it("keeps the current buffer view when the engine rejects a selection", async () => {
    const host = renderer();
    host.engine.set_buffer_view.mockImplementation(() => { throw Error("unavailable"); });
    await expect(host.setBufferView("missing")).rejects.toThrow("unavailable");
    expect(host.bufferView).toBe("shaded");
  });
  it("installs Preview lighting by default", async () => {
    const host = renderer();
    await host.setShader("replacement", multiSurface());
    expect(host.engine.set_lighting_environment).toHaveBeenCalledWith("preview");
  });
  it("changes lighting without reinstalling and reapplies it after shader replacement", async () => {
    const host = renderer();
    await host.setLightingEnvironment("directional");
    expect(host.engine.set_lighting_environment).toHaveBeenCalledWith("directional");
    expect(host.engine.install_with_assets).not.toHaveBeenCalled();
    await host.setShader("replacement", multiSurface());
    expect(host.engine.set_lighting_environment).toHaveBeenLastCalledWith("directional");
  });
  it("cancels pending candidates without hiding or replacing the installed preview", async () => {
    const host = renderer();
    await host.setShader("working", multiSurface());
    const installed = host.installed;
    host.stageShader("pending", multiSurface());
    host.cancelPendingShader();
    expect(host.stagedShader).toBeNull();
    expect(host.engine.cancel_pending).toHaveBeenCalled();
    expect(host.hasInstalledPreview).toBe(true);
    expect(host.canvas.style.visibility).toBe("");
    host.engine.install_with_assets.mockRejectedValueOnce(Error("candidate preparation failed"));
    await expect(host.setShader("bad", multiSurface())).rejects.toThrow("candidate preparation failed");
    expect(host.installed).toBe(installed);
    expect(host.hasInstalledPreview).toBe(true);
    expect(host.drawFrame()).toBe(true);
  });
  it("blanks immediately and only reveals a replacement after its first frame", async () => {
    const host = renderer();
    host.onPreviewInstalled = vi.fn();
    host.clearPreview();
    expect(host.canvas.style.visibility).toBe("hidden");
    expect(host.hasInstalledPreview).toBe(false);
    expect(host.drawFrame()).toBe(false);
    const frame = deferred();
    host.engine.render_async.mockReturnValue(frame.promise);
    const install = host.setShader("replacement", multiSurface());
    await vi.waitFor(() => expect(host.engine.render_async).toHaveBeenCalled());
    expect(host.canvas.style.visibility).toBe("hidden");
    expect(host.onPreviewInstalled).not.toHaveBeenCalled();
    frame.resolve(true);
    await install;
    expect(host.onPreviewInstalled).toHaveBeenCalledTimes(1);
    expect(host.canvas.style.visibility).toBe("");
    expect(host.hasInstalledPreview).toBe(true);
  });

  it("does not reveal an obsolete frame after another edit", async () => {
    const host = renderer(), frame = deferred();
    host.engine.render_async.mockReturnValue(frame.promise);
    host.onPreviewInstalled = vi.fn();
    const install = host.setShader("obsolete", multiSurface());
    await vi.waitFor(() => expect(host.engine.render_async).toHaveBeenCalled());
    host.clearPreview();
    frame.resolve(true);
    await install;
    expect(host.onPreviewInstalled).not.toHaveBeenCalled();
    expect(host.canvas.style.visibility).toBe("hidden");
    expect(host.drawFrame()).toBe(false);
  });
  const multiSurface = () => ({
    canvases: [],
    surfaces: [
      { name: "plain", material_ty: "unlit", params: [{ name: "gain", type: "f32", default: 0.25 }] },
      { name: "lit", material_ty: "standard", params: [{ name: "gain", type: "f32", default: 0.75 }] },
    ],
    pipelines: [{ name: "forward", material: "standard", type: "lighting", passes: ["base"] }],
  });
  it("automatically previews the surface with a matching lighting pipeline", async () => {
    const host = renderer(), manifest = multiSurface();
    await host.setShader("authored stages", manifest);
    expect(host.engine.install_with_assets.mock.calls[0].slice(0, 3)).toEqual([
      "authored stages", JSON.stringify(manifest), "lit",
    ]);
    expect(host.canvas.dataset.engineEntry).toBe("lit");
    expect(host.paramDefs[0].default).toBe(0.75);
  });
  it("uses the first surface when no material has a pipeline", async () => {
    const host = renderer(), manifest = multiSurface();
    manifest.pipelines = [];
    await host.setShader("authored stages", manifest);
    expect(host.engine.install_with_assets.mock.calls[0][2]).toBe("plain");
    expect(host.paramDefs[0].default).toBe(0.25);
  });
  it("keeps manifest order when several surfaces have matching pipelines", async () => {
    const host = renderer(), manifest = multiSurface();
    manifest.surfaces.push({ name: "also_lit", material_ty: "standard" });
    await host.setShader("authored stages", manifest);
    expect(host.engine.install_with_assets.mock.calls[0][2]).toBe("lit");
  });
  it("lets explicit selection override the automatic preference and clearing restores it", async () => {
    const host = renderer(), manifest = multiSurface();
    host.setActiveSurfaceName("plain");
    await host.setShader("authored stages", manifest);
    expect(host.engine.install_with_assets.mock.lastCall[2]).toBe("plain");
    host.setActiveSurfaceName(null);
    await host.setShader("authored stages", manifest);
    expect(host.engine.install_with_assets.mock.lastCall[2]).toBe("lit");
  });
  it("honors a surface selected through the URL", async () => {
    vi.stubGlobal("location", { search: "?surface=plain" });
    const host = renderer();
    await host.setShader("authored stages", multiSurface());
    expect(host.engine.install_with_assets.mock.lastCall[2]).toBe("plain");
  });
  it("rejects an unknown explicit entry without replacing the installed program", async () => {
    const host = renderer();
    host.setActiveSurfaceName("missing");
    await expect(host.setShader("authored stages", multiSurface())).rejects.toThrow("Select one entry");
    expect(host.engine.install_with_assets).not.toHaveBeenCalled();
    expect(host.installed.entry).toBe("probe");
  });
  it("retains cached-page resources and resumes without advancing simulation time", async () => {
    const host = renderer();
    const engine = host.engine;
    engine.free = vi.fn();
    host.camera = { free: vi.fn() };
    host.resizeObserver = { disconnect: vi.fn(), observe: vi.fn() };
    host.handleResize = vi.fn();
    host.simTimeSeconds = 3;
    host.lastTickMs = 100;
    host.handlePageHide(true);
    expect(host.drawFrame({ nowMs: 10000 })).toBe(false);
    expect(engine.free).not.toHaveBeenCalled();
    expect(engine.cancel_pending).not.toHaveBeenCalled();
    host.handlePageRestore();
    expect(host.resizeObserver.observe).toHaveBeenCalledWith(host.canvas);
    expect(engine.render_async).toHaveBeenCalledWith(3, 0);
    expect(host.simTimeSeconds).toBe(3);
    await vi.waitFor(() => expect(host.framePending).toBe(false));
    host.handlePageHide(true);
    host.handlePageRestore();
    expect(engine.render_async).toHaveBeenCalledTimes(2);
    host.handlePageHide(false);
    expect(engine.free).toHaveBeenCalledOnce();
    expect(host.camera).toBeUndefined();
    expect(host.gpuReady).toBe(false);
    host.handlePageRestore();
    expect(host.drawFrame()).toBe(false);
  });
  it("serializes asynchronous frames and sends zero delta for paused redraws", async () => {
    const host = renderer(), frame = deferred();
    host.engine.render_async.mockReturnValue(frame.promise);
    host.lastTickMs = 100;
    expect(host.drawFrame({ nowMs: 200 })).toBe(true);
    expect(host.drawFrame({ nowMs: 300 })).toBe(false);
    expect(host.engine.render_async).toHaveBeenCalledTimes(1);
    expect(host.engine.render_async).toHaveBeenCalledWith(0.1, 0.1);
    expect(host.elapsed).toBe(0.1);
    frame.resolve(true); await frame.promise; await Promise.resolve(); await Promise.resolve();
    host.setPlaybackPaused(true);
    host.drawFrame({ advanceTime: false });
    expect(host.engine.render_async).toHaveBeenLastCalledWith(0.1, 0);
  });
  it("queues a paused step while an earlier redraw is still pending", async () => {
    const host = renderer(), first = deferred();
    host.setPlaybackPaused(true); host.engine.render_async.mockReturnValueOnce(first.promise);
    host.drawFrame({ advanceTime: false });
    host.stepPlaybackFrames(1, 60); host.drawFrame({ advanceTime: false });
    expect(host.engine.render_async).toHaveBeenCalledTimes(1);
    first.resolve(true);
    await vi.waitFor(() => expect(host.engine.render_async).toHaveBeenCalledTimes(2));
    expect(host.engine.render_async).toHaveBeenLastCalledWith(1 / 60, 1 / 60);
  });
  it("applies a paused forward step once and resets particle state explicitly", async () => {
    const host = renderer(); host.renderMode = "particles"; host.setPlaybackPaused(true);
    host.stepPlaybackFrames(1, 60); host.drawFrame({ advanceTime: false });
    expect(host.engine.render_async).toHaveBeenLastCalledWith(1 / 60, 1 / 60);
    await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
    host.drawFrame({ advanceTime: false });
    expect(host.engine.render_async).toHaveBeenLastCalledWith(1 / 60, 0);
    host.setPlaybackTime(0); expect(host.engine.reset_playback).toHaveBeenCalledOnce();
    expect(host.simTimeSeconds).toBe(0);
  });
  it("does not install a texture replacement after newer source has been staged", async () => {
    const host = renderer(), bytes = deferred();
    host.textureDefs = [{ name: "paint" }];
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, arrayBuffer: () => bytes.promise }));
    const replacing = host.setTextureSelection("paint", "/paint.png");
    await Promise.resolve();
    host.stageShader("new shader", { canvases: [] });
    bytes.resolve(new ArrayBuffer(4)); await replacing;
    expect(host.engine.install_with_assets).not.toHaveBeenCalled();
    expect(host.stagedShader.wgsl).toBe("new shader");
    expect(host.installed.wgsl).toBe("shader");
  });
  it("preserves compatible edited parameters when preparing a new shader", async () => {
    const host = renderer();
    host.paramDefs = [{ name: "gain", type: "f32", callType: "f32", default: 1, min: null, max: null }];
    host.paramValues.set("gain", 0.25); host.touchedParamNames.add("gain");
    host.engine.parameter_values_json = () => '{"gain":0.25}';
    await host.setShader("updated shader", { canvases: [{ name: "probe", params: [{ name: "gain", type: "f32", default: 1 }] }] });
    expect(host.engine.install_with_assets.mock.calls[0][4]).toBe('{"gain":0.25}');
    expect(host.paramValues.get("gain")).toBe(0.25);
  });
  it("reflects style controls, updates them live, and preserves edits only for the same style", async () => {
    const host = renderer();
    const manifest = symbol => ({ canvases: [], surfaces: [{ name: "probe", material_ty: "standard", settings: { implementations: [{ name: "style", symbol, parameters: [{ name: "gain", type: "f32", default: 0.5, min: 0, max: 1 }] }] } }] });
    const values = { "style.gain": 0.5 };
    host.engine.parameter_values_json = () => JSON.stringify(values);
    host.engine.update_parameters = vi.fn(async json => { Object.assign(values, JSON.parse(json)); return true; });
    await host.setShader("initial", manifest("Toon"));
    expect(host.paramDefs).toEqual([expect.objectContaining({ name: "style.gain", default: 0.5, min: 0, max: 1 })]);
    host.engine.install_with_assets.mockClear();
    host.setParamValue(host.paramDefs[0], 0.25);
    await host.parameterUpdates;
    expect(host.engine.update_parameters).toHaveBeenCalledWith('{"style.gain":0.25}');
    expect(host.engine.install_with_assets).not.toHaveBeenCalled();
    await host.setShader("another renderer", manifest("Toon"));
    expect(host.engine.install_with_assets.mock.lastCall[4]).toBe('{"style.gain":0.25}');
    await host.setShader("another style", manifest("Different"));
    expect(host.engine.install_with_assets.mock.lastCall[4]).toBe('{}');
    host.touchedParamNames.add("style.gain");
    host.paramValues.set("style.gain", 0.25);
    const otherMaterial = manifest("Different");
    otherMaterial.surfaces[0].name = "other";
    await host.setShader("another material", otherMaterial);
    expect(host.engine.install_with_assets.mock.lastCall[4]).toBe('{}');
  });
  it("preserves every integer bit in reflected style defaults and edits", async () => {
    const host = renderer();
    const values = {"style.seed": 4294967295, "style.offset": -2147483648, "style.enabled": true};
    host.engine.parameter_values_json = () => JSON.stringify(values);
    host.engine.update_parameters = vi.fn(async json => { Object.assign(values, JSON.parse(json)); return true; });
    await host.setShader("exact settings", {canvases: [], surfaces: [{name: "probe", material_ty: "standard", settings: {implementations: [{name: "style", symbol: "Exact", parameters: [
      {name: "seed", type: "u32", default: 4294967295}, {name: "offset", type: "i32", default: -2147483648}, {name: "enabled", type: "bool", default: true}
    ]}]}}]});
    expect(host.paramDefs.map(def => def.default)).toEqual([4294967295, -2147483648, true]);
    host.setParamValue(host.paramDefs[0], 16777217);
    await host.parameterUpdates;
    expect(host.engine.update_parameters).toHaveBeenCalledWith('{"style.seed":16777217}');
  });
  it("preserves rapid edits to separate controls in submission order", async () => {
    const host = renderer(), first = deferred();
    const values = { red: 0, green: 0 };
    host.paramDefs = [{ name: "red", type: "f32" }, { name: "green", type: "f32" }];
    host.engine.update_parameters = vi.fn(async json => {
      if (JSON.parse(json).red !== undefined) await first.promise;
      Object.assign(values, JSON.parse(json)); return true;
    });
    host.engine.parameter_values_json = () => JSON.stringify(values);
    host.onParamValueChanged = vi.fn();
    host.setParamValue({ name: "red", type: "f32" }, 0.25);
    host.setParamValue({ name: "green", type: "f32" }, 0.75);
    await Promise.resolve();
    expect(host.engine.update_parameters).toHaveBeenCalledTimes(1);
    first.resolve(); await host.parameterUpdates;
    expect(values).toEqual({ red: 0.25, green: 0.75 });
    expect(Object.fromEntries(host.paramValues)).toEqual(values);
    expect(host.engine.update_parameters.mock.calls.map(([json]) => JSON.parse(json)))
      .toEqual([{ red: 0.25 }, { green: 0.75 }]);
    expect(host.onParamValueChanged).toHaveBeenLastCalledWith({ name: "green", value: 0.75, previous: 0 });
  });
  it.each([[false, "mesh"], [true, "mesh"], [false, "texture"], [true, "texture"]])("preserves parameter edits before replacement (in flight: %s, resource: %s)", async (inFlight, resource) => {
    const host = renderer(), update = deferred();
    const def = { name: "gain", type: "f32" };
    host.paramDefs = [def];
    const values = { gain: 1 };
    host.engine.parameter_values_json = () => JSON.stringify(values);
    host.engine.update_parameters = vi.fn(async json => {
      await update.promise;
      Object.assign(values, JSON.parse(json));
      return true;
    });
    host.setParamValue(def, 0.25);
    if (inFlight) await Promise.resolve();
    host.textureDefs = [{ name: "paint" }];
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }));
    const replacement = resource === "mesh" ? host.setMeshKind("plane") : host.setTextureSelection("paint", "/paint.png");
    expect(host.engine.cancel_pending).not.toHaveBeenCalled();
    update.resolve();
    await host.parameterUpdates;
    await replacement;
    expect(host.engine.update_parameters).toHaveBeenCalledOnce();
    expect(host.paramValues.get("gain")).toBe(0.25);
    expect(JSON.parse(host.engine.install_with_assets.mock.calls[0][4])).toEqual({ gain: 0.25 });
    if (resource === "mesh") expect(host.meshKind).toBe("plane");
    else expect(host.textureDefs[0].selectedUrl).toBe("/paint.png");
  });
  it("does not submit queued edits or publish old results after a shader change", async () => {
    const host = renderer(), first = deferred();
    host.engine.update_parameters = vi.fn().mockReturnValue(first.promise);
    host.onParamValueChanged = vi.fn();
    host.setParamValue({ name: "red", type: "f32" }, 0.25);
    host.setParamValue({ name: "green", type: "f32" }, 0.75);
    await Promise.resolve();
    host.stageShader("new shader", { canvases: [] });
    first.resolve(true); await host.parameterUpdates;
    expect(host.engine.update_parameters).toHaveBeenCalledTimes(1);
    expect(host.onParamValueChanged).not.toHaveBeenCalled();
    expect(host.paramValues.size).toBe(0);
  });
  it("preserves WASM map defaults and translates vector storage edits", async () => {
    const host = renderer();
    let values = { points: [[0, 1, 0]] };
    host.engine.parameter_values_json = () => JSON.stringify(values);
    host.engine.update_parameters = vi.fn(async json => { values = JSON.parse(json); return true; });
    const manifest = { canvases: [{ name: "probe", params: [{ name: "points", type: "array<vec3>",
      default: new Map([["type", "vec3"], ["values", [[0, 1, 0]]]]) }] }] };
    await host.setShader("shader", manifest);
    const installed = JSON.parse(host.engine.install_with_assets.mock.calls[0][1]);
    expect(installed.canvases[0].params[0].default).toEqual({ type: "vec3", values: [[0, 1, 0]] });
    expect(host.paramValues.get("points")).toEqual([0, 1, 0]);
    host.setParamValue(host.paramDefs[0], [1, 0, 0, 0, 0, 1]);
    await host.parameterUpdates;
    expect(values).toEqual({ points: [[1, 0, 0], [0, 0, 1]] });
    expect(host.paramValues.get("points")).toEqual([1, 0, 0, 0, 0, 1]);
    await host.setShader("replacement", manifest);
    expect(JSON.parse(host.engine.install_with_assets.mock.calls[1][4]))
      .toEqual({ points: [[1, 0, 0], [0, 0, 1]] });
  });
  it("retains the installed texture and controls when a replacement fails", async () => {
    const host = renderer();
    host.textureDefs = [{ name: "paint", selectedUrl: "/old.png" }];
    host.textureSelections.set("paint", "/old.png");
    host.onTextureDefsChanged = vi.fn(); host.onRuntimeDiagnostic = vi.fn();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 404 }));
    await host.setTextureSelection("paint", "/missing.png");
    expect(host.textureSelections.get("paint")).toBe("/old.png");
    expect(host.textureDefs[0].selectedUrl).toBe("/old.png");
    expect(host.hasInstalledPreview).toBe(true);
    expect(host.engine.install_with_assets).not.toHaveBeenCalled();
    expect(host.onTextureDefsChanged).toHaveBeenCalled();
    expect(host.onRuntimeDiagnostic).toHaveBeenCalledOnce();
    host.onRuntimeRecovered = vi.fn();
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }));
    await host.setTextureSelection("paint", "/good.png");
    expect(host.textureDefs[0].selectedUrl).toBe("/good.png");
    expect(host.onRuntimeRecovered).toHaveBeenCalledOnce();
    expect(host.errorMessage).toBe("");
  });
  it("ignores a texture fetch failure from a superseded shader", async () => {
    const host = renderer(), response = deferred();
    host.textureDefs = [{ name: "paint", selectedUrl: "/old.png" }];
    host.onRuntimeDiagnostic = vi.fn();
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(response.promise));
    const replacement = host.setTextureSelection("paint", "/slow.png");
    host.stageShader("new shader", { canvases: [] });
    response.resolve({ ok: false, status: 404 }); await replacement;
    expect(host.onRuntimeDiagnostic).not.toHaveBeenCalled();
    expect(host.stagedShader.wgsl).toBe("new shader");
  });
  it("applies parameter edits after an in-flight shader installation", async () => {
    const host = renderer(), installation = deferred();
    const def = { name: "gain", type: "f32", callType: "f32", default: 1, min: null, max: null };
    host.paramDefs = [def];
    let values = { gain: 1 };
    host.engine.parameter_values_json = () => JSON.stringify(values);
    host.engine.install_with_assets.mockReturnValueOnce(installation.promise);
    host.engine.update_parameters = vi.fn(async json => { Object.assign(values, JSON.parse(json)); return true; });
    host.onParamDefsChanged = vi.fn();
    const preparing = host.setShader("replacement", { canvases: [{ name: "probe", params: [def] }] });
    await Promise.resolve();
    host.setParamValue(def, 0.25);
    await Promise.resolve();
    expect(host.engine.update_parameters).not.toHaveBeenCalled();
    installation.resolve(true); await preparing; await host.parameterUpdates;
    expect(host.installed.wgsl).toBe("replacement");
    expect(host.paramValues.get("gain")).toBe(0.25);
    expect(values.gain).toBe(0.25);
    expect(host.onParamDefsChanged).toHaveBeenCalledTimes(2);
  });
  it("applies edits after texture installation instead of canceling it", async () => {
    const host = renderer(), installation = deferred();
    const def = { name: "gain", type: "f32" };
    host.paramDefs = [def]; host.textureDefs = [{ name: "paint", selectedUrl: "/old.png" }];
    let values = { gain: 1 };
    host.engine.parameter_values_json = () => JSON.stringify(values);
    host.engine.install_with_assets.mockReturnValueOnce(installation.promise);
    host.engine.update_parameters = vi.fn(async json => { Object.assign(values, JSON.parse(json)); return true; });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, arrayBuffer: async () => new ArrayBuffer(4) }));
    const replacing = host.setTextureSelection("paint", "/new.png");
    await vi.waitFor(() => expect(host.engine.install_with_assets).toHaveBeenCalledOnce());
    host.setParamValue(def, 0.5); await Promise.resolve();
    expect(host.engine.update_parameters).not.toHaveBeenCalled();
    installation.resolve(true); await replacing; await host.parameterUpdates;
    expect(host.textureDefs[0].selectedUrl).toBe("/new.png");
    expect(host.paramValues.get("gain")).toBe(0.5);
  });
  it("aborts obsolete asset loading and allows the newer shader to install", async () => {
    const host = renderer();
    host.deps.textureOptionsForName = () => ({ options: [{ url: "/slow.png" }], defaultUrl: "/slow.png" });
    vi.stubGlobal("fetch", vi.fn((url, { signal }) => new Promise((resolve, reject) => {
      signal.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), { once: true });
    })));
    const obsolete = host.setShader("obsolete", { canvases: [{ name: "probe", textures: [{ name: "paint" }] }] });
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledOnce());
    host.stageShader("new", { canvases: [{ name: "probe" }] });
    const newest = host.setShader("new", { canvases: [{ name: "probe" }] });
    await Promise.all([obsolete, newest]);
    expect(host.engine.install_with_assets).toHaveBeenCalledOnce();
    expect(host.engine.install_with_assets.mock.calls[0][0]).toBe("new");
    expect(host.installed.wgsl).toBe("new");
  });
  it("commits mesh selection only after successful preparation", async () => {
    const host = renderer(), installation = deferred();
    host.onMeshKindChanged = vi.fn(); host.onRuntimeDiagnostic = vi.fn();
    host.engine.install_with_assets.mockReturnValueOnce(installation.promise);
    const replacing = host.setMeshKind("plane");
    await Promise.resolve();
    expect(host.meshKind).toBe("sphere");
    expect(host.onMeshKindChanged).not.toHaveBeenCalled();
    installation.resolve(true); await replacing;
    expect(host.meshKind).toBe("plane");
    expect(host.canvas.dataset.engineMesh).toBe("plane");
    expect(host.onMeshKindChanged).toHaveBeenCalledOnce();
    host.engine.install_with_assets.mockRejectedValueOnce(Error("geometry allocation failed"));
    await host.setMeshKind("cube");
    expect(host.meshKind).toBe("plane");
    expect(host.canvas.dataset.engineMesh).toBe("plane");
    expect(host.onMeshKindChanged).toHaveBeenCalledOnce();
    expect(host.onRuntimeDiagnostic).toHaveBeenCalledOnce();
  });
  it("does not publish an obsolete mesh when a newer selection arrives", async () => {
    const host = renderer(), installation = deferred();
    host.onMeshKindChanged = vi.fn();
    host.engine.install_with_assets.mockReturnValueOnce(installation.promise);
    const first = host.setMeshKind("plane");
    await Promise.resolve();
    const second = host.setMeshKind("cube");
    installation.resolve(false); await Promise.all([first, second]);
    expect(host.meshKind).toBe("box");
    expect(host.canvas.dataset.engineMesh).toBe("box");
    expect(host.onMeshKindChanged).toHaveBeenCalledOnce();
  });
  it("clears unsupported playback settings when a particle entry installs", async () => {
    const host = renderer();
    host.setPlaybackRate(-1); host.setPlaybackLoopDuration(2);
    host.onPlaybackCapabilitiesChanged = vi.fn();
    await host.setShader("particles", { canvases: [], surfaces: [{ name: "probe" }], techniques: [{ surface: "probe", metadata: { engine: "particle" } }] });
    expect(host.renderMode).toBe("particles");
    expect(host.playbackRate).toBe(1);
    expect(host.playbackLoopSeconds).toBe(0);
    expect(host.elapsed).toBe(0);
    expect(host.onPlaybackCapabilitiesChanged).toHaveBeenCalledOnce();
    host.lastTickMs = 100; host.drawFrame({ nowMs: 200 });
    expect(host.engine.render_async).toHaveBeenLastCalledWith(0.1, 0.1);
    await host.setShader("canvas", { canvases: [{ name: "probe" }] });
    expect(host.onPlaybackCapabilitiesChanged).toHaveBeenCalledTimes(2);
    host.setPlaybackRate(-1);
    expect(host.playbackRate).toBe(-1);
  });
  it("reports unsupported particle seeks without advancing or discarding the installed artifact", () => {
    const host = renderer(); host.renderMode = "particles"; host.onRuntimeDiagnostic = vi.fn();
    host.setPlaybackTime(5);
    expect(host.simTimeSeconds).toBe(0);
    expect(host.engine.reset_playback).not.toHaveBeenCalled();
    expect(host.installed.wgsl).toBe("shader");
    expect(host.onRuntimeDiagnostic).toHaveBeenCalledOnce();
  });
});
