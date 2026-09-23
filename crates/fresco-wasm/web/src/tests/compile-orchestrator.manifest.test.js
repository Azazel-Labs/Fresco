import { describe, expect, it, vi } from "vitest";

import { createCompileOrchestrator } from "../app/compile-orchestrator";

function makeState() {
  return {
    compileToken: 0,
    workerAutoRecoveryAttempts: 0,
    previewBuildDeferred: false,
    previewBuildDeferredTimer: 0,
    firstCompileKickoffAtMs: null,
    startupFirstCompileTelemetryEmitted: false,
  };
}

function makeDeps(overrides = {}) {
  const sourceEditor = {
    getValue: () => "canvas t(uv: coord, time: signal) -> color { compose { circle(at: center, radius: 0.2) |> fill(#fff) } }",
    getModel: () => ({ id: "model" }),
    setDiagnostics: vi.fn(),
  };
  const wgslEditor = {
    setValue: vi.fn(),
  };
  const explainEl = { textContent: "" };
  const manifestEl = { textContent: "stale" };
  const previewStatusEl = { textContent: "" };
  const renderer = {
    clearPreview: vi.fn(),
    stageShader: vi.fn(),
    maybeBuildStagedShader: vi.fn(() => null),
    getAsyncShaderBuildSnapshot: vi.fn(() => ({
      pending: false,
      running_ms: 0,
      last_completed_age_ms: 0,
      last_error: "",
      last_timings: null,
    })),
    drawFrame: vi.fn(),
    reportRuntimeIssue: vi.fn(),
    paramDefs: [],
    textureDefs: [],
  };

  return {
    state: makeState(),
    sourceEditor,
    wgslEditor,
    explainEl,
    manifestEl,
    previewStatusEl,
    renderer,
    isSourceBlank: (source) => !String(source || "").trim(),
    showDiagnostics: vi.fn(),
    renderShaderParams: vi.fn(),
    mapDiagnosticsToMarkers: vi.fn(() => []),
    normalizeSeverity: vi.fn(() => "info"),
    setStatus: vi.fn(),
    compileInBackground: vi.fn(async () => ({
      ok: true,
      wgsl: "@fragment fn fs() -> @location(0) vec4f { return vec4f(1.0); }",
      explain: "explain",
      manifest: { passes: [{ id: "canvas" }] },
      diagnostics: [],
      timings: { total_ms: 1 },
      worker_timings: { worker_total_ms: 1 },
    })),
    isRetryableWorkerFailure: vi.fn(() => false),
    scheduleCompilerRecovery: vi.fn(),
    workerAutoRecoveryDelayMs: 10,
    workerAutoRecoveryMaxAttempts: 1,
    appStartupOriginMs: 0,
    getVizManager: () => null,
    getPreviewPaused: () => false,
    isPreviewPanelActive: () => false,
    ...overrides,
  };
}

describe("compile orchestrator manifest output", () => {
  it.each([false, true])("waits for the startup preview, including failure=%s", async (fail) => {
    const deps = makeDeps({ isPreviewPanelActive: () => true });
    let resolveBuild;
    let rejectBuild;
    deps.renderer.maybeBuildStagedShader.mockImplementation(() => new Promise((resolve, reject) => {
      resolveBuild = resolve;
      rejectBuild = reject;
    }));
    let completed = false;
    const compile = createCompileOrchestrator(deps).compileAndRender({ waitForPreviewBuild: true })
      .then(() => { completed = true; });
    await vi.waitFor(() => expect(deps.renderer.maybeBuildStagedShader).toHaveBeenCalled());
    expect(completed).toBe(false);
    if (fail) rejectBuild(new Error("Preview build failed"));
    else resolveBuild({ total_ms: 1 });
    await compile;
    expect(completed).toBe(true);
    if (fail) expect(deps.renderer.reportRuntimeIssue).toHaveBeenCalledWith("Preview build failed", "async preview shader build");
  });

  it("writes formatted manifest JSON on successful compile", async () => {
    const deps = makeDeps();
    const orchestrator = createCompileOrchestrator(deps);

    await orchestrator.compileAndRender({ source: "canvas t(uv: coord, time: signal) -> color { compose { circle(at: center, radius: 0.2) |> fill(#fff) } }" });

    expect(deps.manifestEl.textContent).toBe(JSON.stringify({ passes: [{ id: "canvas" }] }, null, 2));
  });

  it("clears manifest output when source is blank", async () => {
    const deps = makeDeps();
    const orchestrator = createCompileOrchestrator(deps);

    await orchestrator.compileAndRender({ source: "   " });

    expect(deps.manifestEl.textContent).toBe("");
    expect(deps.compileInBackground).not.toHaveBeenCalled();
  });
});


it("reports a failed candidate while leaving the installed preview intact", async () => {
  const deps = makeDeps();
  deps.renderer.hasInstalledPreview = true;
  deps.compileInBackground.mockResolvedValue({ ok: false, diagnostics: [{message: "unsupported style"}] });
  await createCompileOrchestrator(deps).compileAndRender();
  expect(deps.renderer.clearPreview).not.toHaveBeenCalled();
  expect(deps.renderer.stageShader).not.toHaveBeenCalled();
  expect(deps.previewStatusEl.textContent).toBe("Compile failed; showing previous preview");
  expect(deps.setStatus).toHaveBeenCalledWith("Compile Failed", "error");
});
