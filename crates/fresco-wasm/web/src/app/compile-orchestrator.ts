import { stringifyManifest } from "../manifest-contract";
import {
  compileFailedPreviewFallbackStatus,
  compileWorkerRecoveryStatus,
  compilingPreviewStatus,
  formatPreviewBuildingAsyncStatus,
  formatPreviewDeferredExampleSwitchStatus,
  formatPreviewDeferredInactiveStatus,
  formatPreviewReadyStatus,
  noValidShaderStatus,
  waitingForShaderStatus,
} from "./compile-status";
import { formatHostTimingBreakdown, formatMs, formatTimingBreakdown } from "./timing-format";
import type { CompileQueuePayload } from "./controller-types";

type CompileState = {
  compileToken: number;
  workerAutoRecoveryAttempts: number;
  previewBuildDeferred: boolean;
  previewBuildDeferredTimer: number;
  firstCompileKickoffAtMs: number | null;
  startupFirstCompileTelemetryEmitted: boolean;
};

type VizManagerLike = {
  scheduleRefresh: () => void;
};

type CompileOrchestratorDeps = {
  state: CompileState;
  sourceEditor: any;
  wgslEditor: any;
  explainEl: HTMLElement;
  manifestEl: HTMLElement;
  previewStatusEl: HTMLElement;
  renderer: any;
  isSourceBlank: (source: string) => boolean;
  showDiagnostics: (diags: any[], model: any) => void;
  renderShaderParams: (paramDefs: any[], textureDefs?: any[]) => void;
  mapDiagnosticsToMarkers: (diags: any[], model: any) => any[];
  normalizeSeverity: (rawSeverity: unknown) => string;
  setStatus: (text: string, klass?: string) => void;
  compileInBackground: (source: string, options: { cancelInFlight?: boolean; includeExplain?: boolean; files?: any }) => Promise<any>;
  isRetryableWorkerFailure: (err: unknown) => boolean;
  scheduleCompilerRecovery: (options: { delayMs: number }) => void;
  workerAutoRecoveryDelayMs: number;
  workerAutoRecoveryMaxAttempts: number;
  appStartupOriginMs: number;
  getVizManager: () => VizManagerLike | null;
  getPreviewPaused: () => boolean;
  isPreviewPanelActive: () => boolean;
  onManifestUpdated?: (manifest: any | null) => void;
};

export function createCompileOrchestrator(deps: CompileOrchestratorDeps) {
  const {
    state,
    sourceEditor,
    wgslEditor,
    explainEl,
    manifestEl,
    previewStatusEl,
    renderer,
    isSourceBlank,
    showDiagnostics,
    renderShaderParams,
    mapDiagnosticsToMarkers,
    normalizeSeverity,
    setStatus,
    compileInBackground,
    isRetryableWorkerFailure,
    scheduleCompilerRecovery,
    workerAutoRecoveryDelayMs,
    workerAutoRecoveryMaxAttempts,
    appStartupOriginMs,
    getVizManager,
    getPreviewPaused,
    isPreviewPanelActive,
    onManifestUpdated,
  } = deps;

  async function compileAndRender(options: Partial<CompileQueuePayload> & { waitForPreviewBuild?: boolean } = {}) {
    const {
      source = sourceEditor.getValue(),
      files = null,
      cancelInFlight = false,
      deferPreviewBuild = false,
      queuedAtMs = 0,
      debounceDelayMs = 0,
      debounceTimerLagMs = 0
    } = options;
    const includeExplain = true;
    const wallStartMs = performance.now();
    if (state.firstCompileKickoffAtMs === null) {
      state.firstCompileKickoffAtMs = wallStartMs;
    }
    const startupTimeToFirstCompileMs = Math.max(0, state.firstCompileKickoffAtMs - appStartupOriginMs);
    const token = ++state.compileToken;
    const sourceModel = sourceEditor.getModel();
    if (isSourceBlank(source)) {
      renderer.clearPreview();
      sourceEditor.setDiagnostics([]);
      wgslEditor.setValue("");
      explainEl.textContent = "";
      manifestEl.textContent = "";
      onManifestUpdated?.(null);
      showDiagnostics([], sourceModel);
      renderShaderParams([], []);
      previewStatusEl.textContent = waitingForShaderStatus();
      setStatus("Idle");
      return;
    }

    setStatus("Compiling", "compiling");
    previewStatusEl.textContent = compilingPreviewStatus();

    let result;
    const workerStartMs = performance.now();
    const queueWaitMs = Number.isFinite(queuedAtMs) && queuedAtMs > 0
      ? Math.max(0, workerStartMs - queuedAtMs)
      : 0;
    try {
      result = await compileInBackground(source, { cancelInFlight, includeExplain, files });
    } catch (err: any) {
      if (token !== state.compileToken) {
        return;
      }
      const message = err?.message || String(err);
      const retryable = isRetryableWorkerFailure(err);
      const canAutoRecover = retryable && state.workerAutoRecoveryAttempts < workerAutoRecoveryMaxAttempts;
      const details = message.includes("\n") ? message : "";
      if (canAutoRecover) {
        state.workerAutoRecoveryAttempts += 1;
        scheduleCompilerRecovery({ delayMs: workerAutoRecoveryDelayMs });
      }
      sourceEditor.setDiagnostics([]);
      showDiagnostics([
        {
          file: "worker",
          span_start: 0,
          span_end: 0,
          message,
          severity: "error",
          label: "background compiler",
          help: canAutoRecover
            ? `attempting automatic recovery (${state.workerAutoRecoveryAttempts}/${workerAutoRecoveryMaxAttempts})`
            : retryable
              ? "automatic recovery exhausted; try recover compiler"
              : details || "reload the page if this persists",
          actionLabel: retryable ? "recover compiler" : "retry compile",
          action: () => {
            state.workerAutoRecoveryAttempts = 0;
            scheduleCompilerRecovery({ delayMs: 0 });
          }
        }
      ], sourceModel);
      setStatus(canAutoRecover ? "Recovering Compiler" : "Compile Failed", canAutoRecover ? "compiling" : "error");
      previewStatusEl.textContent = canAutoRecover
        ? compileWorkerRecoveryStatus()
        : message;
      return;
    }
    state.workerAutoRecoveryAttempts = 0;
    const workerEndMs = performance.now();
    if (token !== state.compileToken) {
      return;
    }

    if (result.ok) {
      const successDiagnostics = Array.isArray(result.diagnostics) ? [...result.diagnostics] : [];
      if (!state.startupFirstCompileTelemetryEmitted) {
        state.startupFirstCompileTelemetryEmitted = true;
      }
      const setDiagnosticsStartMs = performance.now();
      sourceEditor.setDiagnostics(mapDiagnosticsToMarkers(successDiagnostics, sourceModel));
      const setDiagnosticsEndMs = performance.now();

      const setWgslStartMs = performance.now();
      wgslEditor.setValue(result.wgsl);
      const setWgslEndMs = performance.now();

      manifestEl.textContent = stringifyManifest(result.manifest || {}, 2);
      onManifestUpdated?.(result.manifest || null);
      // A renderer catalog update can queue a corrected configuration.
      if (token !== state.compileToken) return;

      const renderExplainStartMs = performance.now();
      const timingBreakdown = formatTimingBreakdown(result.timings);
      const workerTimings = result.worker_timings || {};
      const hostTimings: any = {
        total_wall_ms: 0,
        active_wall_ms: 0,
        worker_round_trip_ms: workerEndMs - workerStartMs,
        worker_compile_ms: Number(workerTimings.worker_total_ms) || 0,
        worker_overhead_ms: 0,
        queue_wait_ms: queueWaitMs,
        debounce_delay_ms: Number.isFinite(debounceDelayMs) ? Math.max(0, debounceDelayMs) : 0,
        debounce_timer_lag_ms: Number.isFinite(debounceTimerLagMs) ? Math.max(0, debounceTimerLagMs) : 0,
        startup_time_to_first_compile_ms: startupTimeToFirstCompileMs,
        browser_consume_wall_ms: 0,
        main_apply_ms: 0,
        set_source_diagnostics_ms: setDiagnosticsEndMs - setDiagnosticsStartMs,
        set_wgsl_editor_ms: setWgslEndMs - setWgslStartMs,
        render_explain_panel_ms: 0,
        render_diagnostics_panel_ms: 0,
        preview_shader_build_ms: 0,
        preview_shader_build_captured: false,
        preview_apply_mode: "staged",
        preview_async_build_pending: false,
        preview_async_build_running_ms: 0,
        preview_last_async_build_total_ms: 0,
        preview_last_async_build_age_ms: 0,
        preview_last_async_build_error: "",
        preview_pipeline_cache_hit: false,
        preview_setup_texture_bindings_ms: 0,
        preview_build_shader_code_ms: 0,
        preview_create_shader_module_ms: 0,
        preview_create_pipeline_layout_ms: 0,
        preview_create_pipeline_async_ms: 0,
        preview_first_draw_warmup_ms: 0,
        preview_first_draw_ok: false,
        render_params_panel_ms: 0,
        payload_wgsl_chars: String(result.wgsl || "").length,
        payload_manifest_chars: stringifyManifest(result.manifest || {}).length,
        payload_explain_chars: String(result.explain || "").length
      };
      const compileTotalMs = Number(result?.timings?.total_ms);
      if (Number.isFinite(compileTotalMs)) {
        hostTimings.worker_overhead_ms = Math.max(0, hostTimings.worker_round_trip_ms - compileTotalMs);
      }
      const explainText = result.explain || "No explain output.";
      const updateExplainReport = () => {
        const breakdown = formatHostTimingBreakdown(hostTimings);
        explainEl.textContent = timingBreakdown
          ? `${timingBreakdown}\n\n${breakdown}\n\n${explainText}`
          : `${breakdown}\n\n${explainText}`;
      };
      updateExplainReport();
      const renderExplainEndMs = performance.now();
      hostTimings.render_explain_panel_ms = renderExplainEndMs - renderExplainStartMs;

      const renderDiagnosticsStartMs = performance.now();
      showDiagnostics(successDiagnostics, sourceModel);
      const renderDiagnosticsEndMs = performance.now();
      hostTimings.render_diagnostics_panel_ms = renderDiagnosticsEndMs - renderDiagnosticsStartMs;

      const hasWarnings = successDiagnostics.some(
        (diag: any) => normalizeSeverity(diag?.severity) === "warning"
      );
      try {
        const previewBuildActive = isPreviewPanelActive();
        let previewBuild: Promise<unknown> | undefined;
        clearTimeout(state.previewBuildDeferredTimer);
        state.previewBuildDeferredTimer = 0;
        renderer.stageShader(result.wgsl, result.manifest || null);
        state.previewBuildDeferred = Boolean(previewBuildActive && deferPreviewBuild);
        hostTimings.preview_apply_mode = state.previewBuildDeferred
          ? "staged-deferred"
          : previewBuildActive ? "staged-active" : "staged-inactive";
        if (previewBuildActive && !state.previewBuildDeferred) {
          const stagedBuildPromise = renderer.maybeBuildStagedShader();
          previewBuild = stagedBuildPromise?.then((shaderTimings: any) => {
            if (token !== state.compileToken) {
              return;
            }
            if (shaderTimings && typeof shaderTimings === "object") {
              hostTimings.preview_shader_build_captured = true;
              hostTimings.preview_shader_build_ms = Number(shaderTimings.total_ms) || 0;
              hostTimings.preview_pipeline_cache_hit = Boolean(shaderTimings.cache_hit);
              hostTimings.preview_setup_texture_bindings_ms = Number(shaderTimings.setup_texture_bindings_ms) || 0;
              hostTimings.preview_build_shader_code_ms = Number(shaderTimings.build_shader_code_ms) || 0;
              hostTimings.preview_create_shader_module_ms = Number(shaderTimings.create_shader_module_ms) || 0;
              hostTimings.preview_create_pipeline_layout_ms = Number(shaderTimings.create_pipeline_layout_ms) || 0;
              hostTimings.preview_create_pipeline_async_ms = Number(shaderTimings.create_pipeline_async_ms) || 0;
              hostTimings.preview_first_draw_warmup_ms = Number(shaderTimings.first_draw_warmup_ms) || 0;
              hostTimings.preview_first_draw_ok = Boolean(shaderTimings.first_draw_ok);
              hostTimings.preview_last_async_build_total_ms = Number(shaderTimings.total_ms) || 0;
              hostTimings.preview_last_async_build_age_ms = 0;
              hostTimings.preview_last_async_build_error = "";
            }

            const asyncSnapshot = renderer.getAsyncShaderBuildSnapshot();
            hostTimings.preview_async_build_pending = Boolean(asyncSnapshot?.pending);
            hostTimings.preview_async_build_running_ms = Number(asyncSnapshot?.running_ms) || 0;
            hostTimings.preview_last_async_build_age_ms = Number(asyncSnapshot?.last_completed_age_ms) || 0;

            hostTimings.active_wall_ms = Math.max(hostTimings.active_wall_ms, performance.now() - wallStartMs);
            hostTimings.total_wall_ms = hostTimings.queue_wait_ms + hostTimings.active_wall_ms;
            hostTimings.browser_consume_wall_ms = Math.max(0, performance.now() - workerEndMs);
            hostTimings.main_apply_ms = hostTimings.active_wall_ms - hostTimings.worker_round_trip_ms;

            const totalMs = Number(result?.timings?.total_ms);
            previewStatusEl.textContent = formatPreviewReadyStatus(totalMs, formatMs);

            updateExplainReport();
          }).catch((err: any) => {
            renderer.reportRuntimeIssue(err?.message || err, "async preview shader build");
          });
        }
        const asyncSnapshot = renderer.getAsyncShaderBuildSnapshot();
        hostTimings.preview_async_build_pending = Boolean(asyncSnapshot?.pending);
        hostTimings.preview_async_build_running_ms = Number(asyncSnapshot?.running_ms) || 0;
        hostTimings.preview_last_async_build_age_ms = Number(asyncSnapshot?.last_completed_age_ms) || 0;
        hostTimings.preview_last_async_build_error = String(asyncSnapshot?.last_error || "");
        if (asyncSnapshot?.last_timings && typeof asyncSnapshot.last_timings === "object") {
          const timings = asyncSnapshot.last_timings;
          hostTimings.preview_last_async_build_total_ms = Number(timings.total_ms) || 0;
          hostTimings.preview_pipeline_cache_hit = Boolean(timings.cache_hit);
          hostTimings.preview_setup_texture_bindings_ms = Number(timings.setup_texture_bindings_ms) || 0;
          hostTimings.preview_build_shader_code_ms = Number(timings.build_shader_code_ms) || 0;
          hostTimings.preview_create_shader_module_ms = Number(timings.create_shader_module_ms) || 0;
          hostTimings.preview_create_pipeline_layout_ms = Number(timings.create_pipeline_layout_ms) || 0;
          hostTimings.preview_create_pipeline_async_ms = Number(timings.create_pipeline_async_ms) || 0;
          hostTimings.preview_first_draw_warmup_ms = Number(timings.first_draw_warmup_ms) || 0;
          hostTimings.preview_first_draw_ok = Boolean(timings.first_draw_ok);
        }
        if (token !== state.compileToken) {
          return;
        }
        const vizManager = getVizManager();
        if (vizManager) {
          vizManager.scheduleRefresh();
        }
        if (getPreviewPaused()) {
          renderer.drawFrame({ advanceTime: false });
        }
        const renderParamsStartMs = performance.now();
        renderShaderParams(renderer.paramDefs, renderer.textureDefs);
        const renderParamsEndMs = performance.now();
        hostTimings.render_params_panel_ms = renderParamsEndMs - renderParamsStartMs;

        hostTimings.active_wall_ms = performance.now() - wallStartMs;
        hostTimings.total_wall_ms = hostTimings.queue_wait_ms + hostTimings.active_wall_ms;
        hostTimings.browser_consume_wall_ms = Math.max(0, performance.now() - workerEndMs);
        hostTimings.main_apply_ms = hostTimings.active_wall_ms - hostTimings.worker_round_trip_ms;
        updateExplainReport();

        const totalMs = Number(result?.timings?.total_ms);
        if (!previewBuildActive) {
          previewStatusEl.textContent = formatPreviewDeferredInactiveStatus(totalMs, formatMs);
        } else if (state.previewBuildDeferred) {
          previewStatusEl.textContent = formatPreviewDeferredExampleSwitchStatus(totalMs, formatMs);
          state.previewBuildDeferredTimer = window.setTimeout(() => {
            if (token !== state.compileToken || !state.previewBuildDeferred) {
              return;
            }
            state.previewBuildDeferred = false;
            void renderer.maybeBuildStagedShader().catch((err: any) => {
              renderer.reportRuntimeIssue(err?.message || err, "deferred preview shader build");
            });
          }, 180);
        } else {
          previewStatusEl.textContent = formatPreviewBuildingAsyncStatus(totalMs, formatMs);
        }
        setStatus(hasWarnings ? "Compiled with Warnings" : "Compiled", hasWarnings ? "warning" : "success");
        // Startup keeps the workspace covered until the first preview is ready.
        // Subsequent edits retain asynchronous preview replacement.
        if (options.waitForPreviewBuild) await previewBuild;
      } catch (err: any) {
        // Failed candidates leave the installed preview intact; expose preparation errors.
        showDiagnostics([
          {
            file: "preview",
            span_start: 0,
            span_end: 0,
            message: err.message,
            severity: "error",
            label: "runtime shader build",
            help: "Check generated WGSL output"
          }
        ], sourceModel);
        previewStatusEl.textContent = noValidShaderStatus();
        setStatus("Preview Error", "error");
      }
    } else {
      manifestEl.textContent = "";
      onManifestUpdated?.(null);
      sourceEditor.setDiagnostics(mapDiagnosticsToMarkers(result.diagnostics, sourceModel));
      showDiagnostics(result.diagnostics, sourceModel);
      const vizManager = getVizManager();
      if (vizManager) {
        vizManager.scheduleRefresh();
      }
      setStatus("Compile Failed", "error");
      previewStatusEl.textContent = compileFailedPreviewFallbackStatus(renderer.hasInstalledPreview);
    }
  }

  return {
    compileAndRender,
  };
}
