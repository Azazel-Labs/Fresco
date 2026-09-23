import {
  WORKER_REQUEST,
  isWorkerCancelledMessage,
  isWorkerResponseMessage,
  isWorkerResultMessage,
  makeWorkerRequest
} from "./worker-protocol";

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (reason?: unknown) => void;
  timeoutHandle: ReturnType<typeof setTimeout>;
};

type CreateWorkerRuntimeOptions = {
  compileTimeoutMs: number;
  workerQueryTimeoutMs: number;
  workerRequestMaxAttempts: number;
  transientWorkerRetryDelayMs: number;
  workerAutoRecoveryDelayMs: number;
  isRetryableWorkerFailure: (err: unknown) => boolean;
  onCompileRecoveryRequested?: () => void;
};

function formatWorkerCrashReason(kind: string, event: ErrorEvent): string {
  const label = String(kind || "worker").trim() || "worker";
  const parts = [];
  const eventMessage = String(event?.message || "").trim();
  const error = event?.error;
  const errorName = String(error?.name || "").trim();
  const errorMessage = String(error?.message || "").trim();
  const filename = String(event?.filename || "").trim();
  const lineNumber = Number(event?.lineno);
  const columnNumber = Number(event?.colno);
  const stack = String(error?.stack || "").trim();

  if (errorName || errorMessage) {
    parts.push(`${errorName || "Error"}${errorMessage ? `: ${errorMessage}` : ""}`.trim());
  } else if (eventMessage) {
    parts.push(eventMessage);
  }

  if (filename) {
    const location = `${filename}${Number.isFinite(lineNumber) ? `:${lineNumber}` : ""}${Number.isFinite(columnNumber) ? `:${columnNumber}` : ""}`;
    parts.push(`at ${location}`);
  }

  if (stack) {
    const stackLines = stack.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
    if (stackLines.length > 0) {
      parts.push(stackLines[0]);
    }
  }

  const reason = parts.join("\n").trim();
  return reason ? `${label} crashed:\n${reason}` : `${label} crashed`;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

export function createWorkerRuntime({
  compileTimeoutMs,
  workerQueryTimeoutMs,
  workerRequestMaxAttempts,
  transientWorkerRetryDelayMs,
  workerAutoRecoveryDelayMs,
  isRetryableWorkerFailure,
  onCompileRecoveryRequested
}: CreateWorkerRuntimeOptions) {
  let compileWorker: Worker | null = null;
  let queryWorker: Worker | null = null;
  let compileRequestId = 0;
  const compilePending = new Map<number, PendingRequest>();
  const workerQueryPending = new Map<number, PendingRequest>();
  let workerAutoRecoveryTimer = 0;

  function ensureCompileWorker() {
    if (compileWorker) {
      return compileWorker;
    }

    compileWorker = new Worker(new URL("./compile-worker.ts", import.meta.url), {
      type: "module"
    });

    compileWorker.addEventListener("message", (event: MessageEvent) => {
      const payload = (event?.data || {}) as Record<string, unknown>;
      const id = Number(payload.id);
      if (!Number.isFinite(id)) {
        return;
      }

      const pending = compilePending.get(id);
      if (!pending) {
        return;
      }

      clearTimeout(pending.timeoutHandle);

      if (isWorkerCancelledMessage(payload)) {
        compilePending.delete(id);
        pending.reject(new Error("compile cancelled"));
        return;
      }

      if (!isWorkerResultMessage(payload)) {
        return;
      }

      compilePending.delete(id);

      if (Boolean(payload.ok)) {
        pending.resolve(payload.result);
      } else {
        pending.reject(new Error(String(payload.error || "compile worker failed")));
      }
    });

    compileWorker.addEventListener("error", (event: ErrorEvent) => {
      const message = formatWorkerCrashReason("compile worker", event);
      for (const pending of compilePending.values()) {
        clearTimeout(pending.timeoutHandle);
        pending.reject(new Error(message));
      }
      compilePending.clear();

      try {
        compileWorker?.terminate();
      } catch {
        // Ignore terminate failures and recreate on next request.
      }
      compileWorker = null;
    });

    compileWorker.addEventListener("messageerror", () => {
      for (const pending of compilePending.values()) {
        clearTimeout(pending.timeoutHandle);
        pending.reject(new Error("compile worker message decode failed"));
      }
      compilePending.clear();
      try {
        compileWorker?.terminate();
      } catch {
        // Ignore terminate failures and recreate on next request.
      }
      compileWorker = null;
    });

    return compileWorker;
  }

  function ensureQueryWorker() {
    if (queryWorker) {
      return queryWorker;
    }

    queryWorker = new Worker(new URL("./compile-worker.ts", import.meta.url), {
      type: "module"
    });

    queryWorker.addEventListener("message", (event: MessageEvent) => {
      const payload = (event?.data || {}) as Record<string, unknown>;
      const id = Number(payload.id);
      if (!Number.isFinite(id) || !isWorkerResponseMessage(payload)) {
        return;
      }

      const pending = workerQueryPending.get(id);
      if (!pending) {
        return;
      }
      clearTimeout(pending.timeoutHandle);
      workerQueryPending.delete(id);
      if (Boolean(payload.ok)) {
        pending.resolve(payload.result);
      } else {
        pending.reject(new Error(String(payload.error || "worker query failed")));
      }
    });

    queryWorker.addEventListener("error", (event: ErrorEvent) => {
      const message = formatWorkerCrashReason("query worker", event);
      for (const pending of workerQueryPending.values()) {
        clearTimeout(pending.timeoutHandle);
        pending.reject(new Error(message));
      }
      workerQueryPending.clear();
      try {
        queryWorker?.terminate();
      } catch {
        // Ignore terminate failures and recreate on next request.
      }
      queryWorker = null;
    });

    queryWorker.addEventListener("messageerror", () => {
      for (const pending of workerQueryPending.values()) {
        clearTimeout(pending.timeoutHandle);
        pending.reject(new Error("query worker message decode failed"));
      }
      workerQueryPending.clear();
      try {
        queryWorker?.terminate();
      } catch {
        // Ignore terminate failures and recreate on next request.
      }
      queryWorker = null;
    });

    return queryWorker;
  }

  function terminateCompileWorker() {
    try {
      compileWorker?.terminate();
    } catch {
      // Ignore terminate failures and recreate on next request.
    }
    compileWorker = null;
  }

  function terminateQueryWorker() {
    try {
      queryWorker?.terminate();
    } catch {
      // Ignore terminate failures and recreate on next request.
    }
    queryWorker = null;
  }

  function resetBackgroundWorkers() {
    clearTimeout(workerAutoRecoveryTimer);
    workerAutoRecoveryTimer = 0;
    terminateCompileWorker();
    for (const pending of workerQueryPending.values()) {
      clearTimeout(pending.timeoutHandle);
      pending.reject(new Error("query worker reset"));
    }
    workerQueryPending.clear();
    terminateQueryWorker();
  }

  function scheduleCompilerRecovery({ delayMs = workerAutoRecoveryDelayMs }: { delayMs?: number } = {}) {
    clearTimeout(workerAutoRecoveryTimer);
    resetBackgroundWorkers();
    workerAutoRecoveryTimer = window.setTimeout(() => {
      workerAutoRecoveryTimer = 0;
      if (typeof onCompileRecoveryRequested === "function") {
        onCompileRecoveryRequested();
      }
    }, Math.max(0, Number(delayMs) || 0));
  }

  function postWorkerQuery(kind: string, payload: Record<string, unknown> = {}, timeoutMs = workerQueryTimeoutMs): Promise<unknown> {
    const worker = ensureQueryWorker();
    const id = ++compileRequestId;

    return new Promise((resolve, reject) => {
      const timeoutHandle = setTimeout(() => {
        const pending = workerQueryPending.get(id);
        if (!pending) {
          return;
        }
        workerQueryPending.delete(id);
        pending.reject(new Error(`worker query timed out: ${kind}`));
      }, timeoutMs);

      workerQueryPending.set(id, { resolve, reject, timeoutHandle });
      worker.postMessage(makeWorkerRequest(kind, id, payload));
    });
  }

  async function requestWorkerQuery(kind: string, payload: Record<string, unknown> = {}, timeoutMs = workerQueryTimeoutMs): Promise<unknown> {
    let lastError: unknown = null;
    for (let attempt = 1; attempt <= workerRequestMaxAttempts; attempt += 1) {
      try {
        return await postWorkerQuery(kind, payload, timeoutMs);
      } catch (err) {
        lastError = err;
        if (attempt >= workerRequestMaxAttempts || !isRetryableWorkerFailure(err)) {
          throw err;
        }
        terminateQueryWorker();
        await delay(transientWorkerRetryDelayMs);
      }
    }
    throw lastError || new Error(`worker query failed: ${kind}`);
  }

  function cancelPendingCompileRequests(reason = "compile superseded", { forceTerminateWorker = false }: { forceTerminateWorker?: boolean } = {}) {
    for (const pending of compilePending.values()) {
      clearTimeout(pending.timeoutHandle);
      pending.reject(new Error(reason));
    }
    compilePending.clear();

    if (forceTerminateWorker || compileWorker) {
      terminateCompileWorker();
    }
  }

  function postCompileInBackground(source: string, { includeExplain = false, files = null }: { includeExplain?: boolean; files?: Map<string, string> | null } = {}): Promise<unknown> {
    const worker = ensureCompileWorker();
    const id = ++compileRequestId;
    return new Promise((resolve, reject) => {
      const timeoutHandle = setTimeout(() => {
        const pending = compilePending.get(id);
        if (!pending) {
          return;
        }
        compilePending.delete(id);
        pending.reject(new Error("compile worker timed out"));
        terminateCompileWorker();
      }, compileTimeoutMs);

      compilePending.set(id, { resolve, reject, timeoutHandle });
      if (files instanceof Map && files.size > 1) {
        const filesObj = Object.fromEntries(files);
        worker.postMessage(
          makeWorkerRequest(WORKER_REQUEST.COMPILE_BUNDLE, id, {
            files: filesObj,
            entrypoint: "main.fr",
            includeExplain,
            sentAtMs: performance.now()
          })
        );
      } else {
        worker.postMessage(
          makeWorkerRequest(WORKER_REQUEST.COMPILE, id, {
            source,
            includeExplain,
            sentAtMs: performance.now()
          })
        );
      }
    });
  }

  async function compileInBackground(
    source: string,
    { cancelInFlight = false, includeExplain = false, files = null }: { cancelInFlight?: boolean; includeExplain?: boolean; files?: Map<string, string> | null } = {}
  ): Promise<unknown> {
    if (cancelInFlight && compilePending.size > 0) {
      cancelPendingCompileRequests("compile cancelled due to selection change", {
        forceTerminateWorker: true
      });
    }

    let lastError: unknown = null;
    for (let attempt = 1; attempt <= workerRequestMaxAttempts; attempt += 1) {
      try {
        return await postCompileInBackground(source, { includeExplain, files });
      } catch (err) {
        lastError = err;
        if (attempt >= workerRequestMaxAttempts || !isRetryableWorkerFailure(err)) {
          throw err;
        }
        terminateCompileWorker();
        await delay(transientWorkerRetryDelayMs);
      }
    }
    throw lastError || new Error("compile worker failed");
  }

  return {
    compileInBackground,
    requestWorkerQuery,
    scheduleCompilerRecovery,
    resetBackgroundWorkers
  };
}
