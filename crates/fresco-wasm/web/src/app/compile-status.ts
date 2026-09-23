export function formatQueuedCompileStatus(delayMs: number, inFlight: boolean): string {
  return inFlight
    ? `Compile queued • debounce ${delayMs}ms (worker busy)`
    : `Compile queued • debounce ${delayMs}ms`;
}

export function waitingForShaderStatus(): string {
  return "Waiting for shader";
}

export function compilingPreviewStatus(): string {
  return "Compiling preview...";
}

export function compileWorkerRecoveryStatus(): string {
  return "Compile worker crashed; attempting recovery with last source...";
}

export function formatPreviewReadyStatus(
  totalMs: number,
  formatMs: (ms: number) => string
): string {
  return Number.isFinite(totalMs)
    ? `Preview ready • compile ${formatMs(totalMs)}`
    : "Preview ready";
}

export function formatPreviewDeferredInactiveStatus(
  totalMs: number,
  formatMs: (ms: number) => string
): string {
  return Number.isFinite(totalMs)
    ? `Preview deferred (panel inactive) • compile ${formatMs(totalMs)}`
    : "Preview deferred (panel inactive)";
}

export function formatPreviewDeferredExampleSwitchStatus(
  totalMs: number,
  formatMs: (ms: number) => string
): string {
  return Number.isFinite(totalMs)
    ? `Preview deferred (example switch) • compile ${formatMs(totalMs)}`
    : "Preview deferred (example switch)";
}

export function formatPreviewBuildingAsyncStatus(
  totalMs: number,
  formatMs: (ms: number) => string
): string {
  return Number.isFinite(totalMs)
    ? `Preview building (async) • compile ${formatMs(totalMs)}`
    : "Preview building (async)";
}

export function noValidShaderStatus(): string {
  return "No valid shader to render";
}

export function compileFailedPreviewFallbackStatus(hasInstalledPreview = false): string {
  return hasInstalledPreview ? "Compile failed; showing previous preview" : "Compile failed; no preview";
}

export function previewUnavailableStatus(): string {
  return "Preview unavailable";
}
