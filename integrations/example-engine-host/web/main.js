import init, { BrowserEngine, BrowserCamera } from "./renderer/fresco_example_engine_host.js";

const source = document.querySelector("#source");
const rendererChoice = document.querySelector("#renderer");
rendererChoice.onchange = () => compile();
const sample = document.querySelector("#sample");
const sampleSources = new Map();
const canvas = document.querySelector("#canvas");
const status = document.querySelector("#status");
const diagnostics = document.querySelector("#diagnostics");
const compileButton = document.querySelector("#compile");
const pauseButton = document.querySelector("#pause");
const resetButton = document.querySelector("#reset");
const rebuildButton = document.querySelector("#rebuild");
const parameters = document.querySelector("#parameters");
const applyParametersButton = document.querySelector("#apply-parameters");
const textureName = document.querySelector("#texture-name");
const textureFile = document.querySelector("#texture-file");
const mesh = document.querySelector("#mesh");
let engine;
let camera;
let pointer;
const resetCamera = document.querySelector('#reset-camera');

canvas.onpointerdown = event => {
  if (!active?.isSurface || rebuilding || event.button !== 0 || pointer) return;
  canvas.setPointerCapture(event.pointerId);
  pointer = { id: event.pointerId, x: event.clientX, y: event.clientY };
};
canvas.onpointermove = event => {
  if (!pointer || pointer.id !== event.pointerId) return;
  if (!engine || rebuilding) { pointer = undefined; return; }
  camera.drag(event.clientX - pointer.x, event.clientY - pointer.y);
  pointer.x = event.clientX; pointer.y = event.clientY;
  engine.set_camera(camera);
};
const releasePointer = event => {
  if (pointer?.id !== event.pointerId) return;
  pointer = undefined;
  if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
};
canvas.onpointerup = canvas.onpointercancel = canvas.onlostpointercapture = releasePointer;
canvas.addEventListener('wheel', event => {
  if (!active?.isSurface || !engine || rebuilding) return;
  event.preventDefault();
  const pixels = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? canvas.clientHeight : 1);
  camera.zoom(pixels);
  engine.set_camera(camera);
}, { passive: false });
resetCamera.onclick = () => {
  camera.reset();
  engine?.set_camera(camera);
};
let worker;
let request = 0;
let paused = false;
let time = 0;
let previous;
let frames = 0;
let active;
let candidate;
let rebuilding = false;
let renderFailed = false;
let resizeEngine;
let resizeWidth;
let resizeHeight;

function report(error) {
  diagnostics.textContent = String(error);
  status.textContent = active ? "Failed; keeping the last valid program." : "Unable to render.";
  textureName.disabled = textureFile.disabled = !engine || rebuilding || !candidate?.textures.length;
}

function resize() {
  if (!engine) return;
  const rect = canvas.getBoundingClientRect();
  const width = Math.round(rect.width * devicePixelRatio);
  const height = Math.round(rect.height * devicePixelRatio);
  if (engine === resizeEngine && width === resizeWidth && height === resizeHeight) return;
  resizeEngine = engine;
  resizeWidth = width;
  resizeHeight = height;
  const current = engine;
  current.resize(width, height).catch(error => {
    if (engine === current) report(error);
  });
}

function compile() {
  request += 1;
  engine?.cancel_pending();
  diagnostics.textContent = "";
  status.textContent = "Compiling…";
  textureName.disabled = textureFile.disabled = true;
  worker.postMessage({ id: request, source: source.value, renderer: rendererChoice.value });
}

async function frame(now) {
  const delta = previous === undefined || paused ? 0 : Math.min((now - previous) / 1000, 0.1);
  previous = now;
  time += delta;
  if (engine && !renderFailed) {
    const current = engine;
    try {
      resize();
      if (await current.render_async(time, delta)) frames += 1;
    } catch (error) {
      if (engine === current) { renderFailed = true; report(error); }
    }
  }
  // Observable host state is useful for inspecting the sample and local smoke tests.
  canvas.dataset.frames = String(frames);
  canvas.dataset.time = String(time);
  requestAnimationFrame(frame);
}

async function install(result, id) {
  const entries = [...(result.manifest?.canvases ?? []), ...(result.manifest?.surfaces ?? [])];
  if (entries.length !== 1) throw new Error("This host requires exactly one canvas or surface entry.");
  const artifact = {
    wgsl: result.wgsl,
    manifest: JSON.stringify(result.manifest, (_, value) => value instanceof Map ? Object.fromEntries(value) : value),
    entry: entries[0].name,
    assets: new Map(),
    textures: entries[0].textures ?? [],
    mesh: mesh.value,
    isSurface: (result.manifest?.surfaces ?? []).includes(entries[0]),
    isParticles: !!result.manifest.techniques?.some(t => t.surface === entries[0].name && t.metadata.engine === "particle"),
  };
  candidate = artifact;
  textureName.replaceChildren(...artifact.textures.map(def => {
    const option = document.createElement("option");
    option.value = def.name;
    option.textContent = `${def.name}${def.metadata?.default_asset ? ` (${def.metadata.default_asset})` : ""}`;
    return option;
  }));
  textureFile.value = "";
  textureName.disabled = textureFile.disabled = !artifact.textures.length;
  await installArtifact(artifact, id);
}

async function installArtifact(artifact, id, preserveParameters = false) {
  const target = engine;
  const overrides = preserveParameters && active?.manifest === artifact.manifest && active?.wgsl === artifact.wgsl
    ? active.parameters : undefined;
  const installed = await target.install_with_assets(artifact.wgsl, artifact.manifest, artifact.entry, artifact.assets, overrides, artifact.mesh);
  if (id !== request || target !== engine || !installed) return;
  active = artifact;
  mesh.disabled = !active.isSurface || active.isParticles;
  resetCamera.disabled = !active.isSurface;
  mesh.value = active.mesh;
  active.parameters = engine.parameter_values_json();
  parameters.value = JSON.stringify(JSON.parse(active.parameters), null, 2);
  applyParametersButton.disabled = false;
  renderFailed = false;
  diagnostics.textContent = "";
  status.textContent = `Rendering ${artifact.entry}`;
}

mesh.onchange = async () => {
  if (!active?.isSurface || rebuilding) return;
  const id = ++request;
  engine.cancel_pending();
  const artifact = { ...active, mesh: mesh.value };
  status.textContent = "Preparing mesh…";
  try {
    await installArtifact(artifact, id, true);
    if (id === request && active === artifact) candidate = artifact;
  } catch (error) {
    if (id === request) { mesh.value = active.mesh; report(error); }
  }
};

textureFile.onchange = async () => {
  const file = textureFile.files[0];
  if (!file || !candidate || rebuilding) return;
  const id = ++request;
  engine.cancel_pending();
  const artifact = { ...candidate, assets: new Map(candidate.assets) };
  const name = textureName.value;
  status.textContent = `Loading texture ${name}…`;
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    if (id !== request) return;
    artifact.assets.set(name, bytes);
    candidate = artifact;
    await installArtifact(artifact, id, true);
  } catch (error) { if (id === request) report(error); }
};

compileButton.onclick = compile;
sample.onchange = () => {
  source.value = sampleSources.get(sample.value);
  compile();
};
applyParametersButton.onclick = async () => {
  const id = ++request;
  const target = engine;
  status.textContent = "Applying parameters…";
  try {
    const applied = await target.update_parameters(parameters.value);
    if (id !== request || target !== engine || !applied) return;
    active.parameters = engine.parameter_values_json();
    diagnostics.textContent = "";
    status.textContent = `Rendering ${active.entry}`;
  } catch (error) { if (id === request && target === engine) report(error); }
};
pauseButton.onclick = () => {
  paused = !paused;
  pauseButton.textContent = paused ? "Resume" : "Pause";
};
resetButton.onclick = () => { engine?.reset_playback(); time = 0; previous = undefined; };
rebuildButton.onclick = async () => {
  if (rebuilding) return;
  rebuilding = true;
  pointer = undefined;
  resetCamera.disabled = true;
  mesh.disabled = true;
  rebuildButton.disabled = compileButton.disabled = applyParametersButton.disabled = true;
  sample.disabled = true;
  textureName.disabled = textureFile.disabled = true;
  request += 1;
  status.textContent = "Rebuilding GPU…";
  try {
    engine?.cancel_pending();
    engine?.free();
    engine = undefined;
    engine = await BrowserEngine.create(canvas);
    engine.set_camera(camera);
    resize();
    if (active) {
      await engine.install_with_assets(active.wgsl, active.manifest, active.entry, active.assets, active.parameters, active.mesh);
      parameters.value = JSON.stringify(JSON.parse(active.parameters), null, 2);
    }
    renderFailed = false;
    diagnostics.textContent = "";
    status.textContent = active ? `Rendering ${active.entry}` : "Ready to compile.";
  } catch (error) { report(error); }
  finally {
    rebuilding = false;
    mesh.disabled = !engine || !active?.isSurface || active.isParticles;
    resetCamera.disabled = !engine || !active?.isSurface;
    rebuildButton.disabled = false;
    compileButton.disabled = !engine;
    sample.disabled = !engine;
    applyParametersButton.disabled = !engine || !active;
    textureName.disabled = textureFile.disabled = !engine || !candidate?.textures.length;
  }
};

try {
  await Promise.all(Array.from(sample.options, async ({ value }) => {
    const response = await fetch(value);
    if (!response.ok) throw new Error(`Unable to load ${value} (${response.status})`);
    sampleSources.set(value, await response.text());
  }));
  source.value = sampleSources.get(sample.value);
  await init();
  camera = new BrowserCamera();
  engine = await BrowserEngine.create(canvas);
  resize();
  worker = new Worker(new URL("compiler-worker.js", import.meta.url), { type: "module" });
  worker.onerror = event => report(event.message);
  worker.onmessage = async ({ data }) => {
    if (data.ready) {
      compileButton.disabled = pauseButton.disabled = resetButton.disabled = rebuildButton.disabled = false;
      sample.disabled = false;
      rendererChoice.disabled = false;
      compile();
      return;
    }
    if (data.id !== undefined && data.id !== request) return;
    try {
      if (data.error) throw new Error(data.error);
      if (!data.result.ok) {
        throw new Error((data.result.diagnostics ?? []).map(d => d.message ?? String(d)).join("\n") || "Compilation failed.");
      }
      await install(data.result, data.id);
    } catch (error) { if (data.id === undefined || data.id === request) report(error); }
  };
  requestAnimationFrame(frame);
} catch (error) { report(error); }

addEventListener("pagehide", () => {
  worker?.terminate();
  engine?.cancel_pending();
  engine?.free();
  engine = undefined;
  camera?.free();
});
