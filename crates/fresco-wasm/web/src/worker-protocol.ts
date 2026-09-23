export const WORKER_REQUEST = {
  CANCEL: "cancel",
  COMPILE: "compile",
  COMPILE_BUNDLE: "compile_bundle",
  LEX: "lex",
  COMPLETION: "completion",
  ENCODE_SHARED_SOURCE: "encode_shared_source",
  QUERY_SPAN: "query_span",
  COMPILE_VARIANT: "compile_variant",
  COMPILE_VISUALIZER: "compile_visualizer",
  LANGUAGE_PROFILE: "language_profile",
  WASM_VERSION: "wasm_version"
};

export const WORKER_MESSAGE = {
  CANCELLED: "cancelled",
  RESULT: "result",
  RESPONSE: "response"
};

/**
 * @typedef {import("./generated/wasm-contracts/CompileSuccess").CompileSuccess} CompileSuccess
 * @typedef {import("./generated/wasm-contracts/CompileFailure").CompileFailure} CompileFailure
 * @typedef {import("./generated/wasm-contracts/LexToken").LexToken} LexToken
 * @typedef {import("./generated/wasm-contracts/CompletionItem").CompletionItem} CompletionItem
 * @typedef {import("./generated/wasm-contracts/SpanQueryResponse").SpanQueryResponse} SpanQueryResponse
 * @typedef {import("./generated/wasm-contracts/SpanQueryFailure").SpanQueryFailure} SpanQueryFailure
 * @typedef {import("./generated/wasm-contracts/VariantCompileSuccess").VariantCompileSuccess} VariantCompileSuccess
 * @typedef {import("./generated/wasm-contracts/VisualizerCompileSuccess").VisualizerCompileSuccess} VisualizerCompileSuccess
 */

/**
 * @typedef {(
 *   | { type: "compile", id: number, source?: string, includeExplain?: boolean }
 *   | { type: "compile_bundle", id: number, files?: Record<string, string>, entrypoint?: string, includeExplain?: boolean }
 *   | { type: "cancel", id: number }
 *   | { type: "lex", id: number, source?: string }
 *   | { type: "completion", id: number, source?: string, cursorUtf8?: number }
 *   | { type: "encode_shared_source", id: number, source?: string }
 *   | { type: "query_span", id: number, source?: string, files?: Record<string, string>, spanStart?: number, spanEnd?: number }
 *   | { type: "compile_variant", id: number, source?: string, files?: Record<string, string>, spanStart?: number, spanEnd?: number }
 *   | { type: "compile_visualizer", id: number, source?: string, files?: Record<string, string>, spanStart?: number, spanEnd?: number, kind?: string, domain?: string, sweepMax?: number }
 *   | { type: "language_profile", id: number }
 *   | { type: "wasm_version", id: number }
 * )} WorkerRequest
 */

/**
 * @typedef {{ type: "cancelled", id: number }} WorkerCancelledMessage
 * @typedef {{ type: "result", id: number, ok: true, result: CompileSuccess | CompileFailure }} WorkerCompileSuccessMessage
 * @typedef {{ type: "result", id: number, ok: false, error: string }} WorkerCompileErrorMessage
 * @typedef {{ type: "response", id: number, ok: true, result: unknown }} WorkerQuerySuccessMessage
 * @typedef {{ type: "response", id: number, ok: false, error: string }} WorkerQueryErrorMessage
 * @typedef {WorkerCancelledMessage | WorkerCompileSuccessMessage | WorkerCompileErrorMessage | WorkerQuerySuccessMessage | WorkerQueryErrorMessage} WorkerOutboundMessage
 */

function toFiniteId(value: unknown): number | null {
  const id = Number(value);
  return Number.isFinite(id) ? id : null;
}

function isKnownRequestType(value: unknown): boolean {
  return Object.values(WORKER_REQUEST).includes(String(value || ""));
}

export function normalizeIncomingWorkerRequest(raw: unknown) {
  const payload = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : null;
  if (!payload) {
    return null;
  }

  const id = toFiniteId(payload.id);
  if (id === null) {
    return null;
  }

  const type = String(payload.type || "");
  if (!isKnownRequestType(type)) {
    return {
      id,
      type,
      payload,
      knownType: false
    };
  }

  return {
    id,
    type,
    payload,
    knownType: true
  };
}

export function makeWorkerCancelled(id: number) {
  return { type: WORKER_MESSAGE.CANCELLED, id };
}

export function makeWorkerResult(id: number, ok: boolean, result: unknown, error = "") {
  if (ok) {
    return { type: WORKER_MESSAGE.RESULT, id, ok: true, result };
  }
  return { type: WORKER_MESSAGE.RESULT, id, ok: false, error: String(error || "compile worker failed") };
}

export function makeWorkerResponse(id: number, ok: boolean, result: unknown, error = "") {
  if (ok) {
    return { type: WORKER_MESSAGE.RESPONSE, id, ok: true, result };
  }
  return { type: WORKER_MESSAGE.RESPONSE, id, ok: false, error: String(error || "worker request failed") };
}

export function isWorkerCancelledMessage(payload: unknown): boolean {
  const data = payload as { type?: unknown; id?: unknown } | null;
  return data?.type === WORKER_MESSAGE.CANCELLED && Number.isFinite(Number(data?.id));
}

export function isWorkerResultMessage(payload: unknown): boolean {
  const data = payload as { type?: unknown; id?: unknown } | null;
  return data?.type === WORKER_MESSAGE.RESULT && Number.isFinite(Number(data?.id));
}

export function isWorkerResponseMessage(payload: unknown): boolean {
  const data = payload as { type?: unknown; id?: unknown } | null;
  return data?.type === WORKER_MESSAGE.RESPONSE && Number.isFinite(Number(data?.id));
}

export function makeWorkerRequest(type: string, id: number, payload: Record<string, unknown> = {}) {
  return { type, id, ...payload };
}
