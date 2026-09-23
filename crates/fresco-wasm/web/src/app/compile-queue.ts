import type {
  CompileQueueFiles,
  CompileQueuePayload,
  CompileQueueRequest,
} from "./controller-types";

export type { CompileQueueFiles, CompileQueuePayload, CompileQueueRequest };

interface CreateCompileQueueControllerOptions {
  captureInput: () => { source: string; files: CompileQueueFiles };
  computeCompileDebounceMs: (source: string) => number;
  getCompileInFlight: () => boolean;
  setCompileInFlight: (inFlight: boolean) => void;
  onQueueRequested?: (request: Required<CompileQueueRequest>) => void;
  onDebounceScheduled?: (delayMs: number, compileInFlight: boolean) => void;
  runCompile: (payload: CompileQueuePayload) => Promise<void>;
}

export function createCompileQueueController(options: CreateCompileQueueControllerOptions) {
  let compileDebounceHandle = 0;
  let compileQueued = false;
  let compileQueuedSource = "";
  let compileQueuedFiles: CompileQueueFiles = null;
  let compileQueuedCancelInFlight = false;
  let compileQueuedDeferPreviewBuild = false;
  let compileQueuedAtMs = 0;
  let compileDebounceMs = 0;
  let compileDebounceScheduledForMs = 0;
  let compileDebounceTimerLagMs = 0;

  function queueCompile(request: CompileQueueRequest = {}) {
    const normalized = {
      delayMs: Number.isFinite(request.delayMs) ? Number(request.delayMs) : Number.NaN,
      cancelInFlight: Boolean(request.cancelInFlight),
      deferPreviewBuild: Boolean(request.deferPreviewBuild),
    };

    options.onQueueRequested?.(normalized);

    compileQueued = true;
    const captured = options.captureInput();
    compileQueuedSource = captured.source;
    compileQueuedFiles = captured.files;
    compileQueuedCancelInFlight = compileQueuedCancelInFlight || normalized.cancelInFlight;
    compileQueuedDeferPreviewBuild = normalized.deferPreviewBuild;
    compileQueuedAtMs = performance.now();

    const nextDelayMs = Number.isFinite(normalized.delayMs)
      ? Math.max(0, Math.floor(normalized.delayMs))
      : options.computeCompileDebounceMs(compileQueuedSource);
    compileDebounceMs = nextDelayMs;
    compileDebounceScheduledForMs = performance.now() + nextDelayMs;
    compileDebounceTimerLagMs = 0;

    if (nextDelayMs > 0) {
      options.onDebounceScheduled?.(nextDelayMs, options.getCompileInFlight());
    }

    clearTimeout(compileDebounceHandle);
    compileDebounceHandle = window.setTimeout(() => {
      if (compileDebounceScheduledForMs > 0) {
        compileDebounceTimerLagMs = Math.max(0, performance.now() - compileDebounceScheduledForMs);
      }
      void flushCompileQueue();
    }, nextDelayMs);
  }

  async function flushCompileQueue() {
    clearTimeout(compileDebounceHandle);
    if (options.getCompileInFlight() || !compileQueued) {
      return;
    }

    while (compileQueued) {
      const payload: CompileQueuePayload = {
        source: compileQueuedSource,
        files: compileQueuedFiles,
        cancelInFlight: compileQueuedCancelInFlight,
        deferPreviewBuild: compileQueuedDeferPreviewBuild,
        queuedAtMs: compileQueuedAtMs,
        debounceDelayMs: compileDebounceMs,
        debounceTimerLagMs: compileDebounceTimerLagMs,
      };

      compileQueued = false;
      compileQueuedFiles = null;
      compileQueuedCancelInFlight = false;
      compileQueuedDeferPreviewBuild = false;
      compileQueuedAtMs = 0;
      compileDebounceMs = 0;
      compileDebounceScheduledForMs = 0;
      compileDebounceTimerLagMs = 0;

      options.setCompileInFlight(true);
      try {
        await options.runCompile(payload);
      } finally {
        options.setCompileInFlight(false);
      }
    }
  }

  return {
    queueCompile,
    flushCompileQueue,
  };
}
