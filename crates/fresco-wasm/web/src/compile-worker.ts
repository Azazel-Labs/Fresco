import initWasm, {
  compile_fresco,
  compile_fresco_bundle,
  compile_fresco_with_options,
  fresco_compile_visualizer,
  fresco_compile_variant,
  fresco_completion_items_with_files,
  fresco_language_profile,
  fresco_lex_tokens_with_files,
  fresco_query_span,
  fresco_wasm_version,
  fresco_wasm_build_mode
} from "../pkg/fresco_wasm.js";
import {
  WORKER_REQUEST,
  makeWorkerCancelled,
  makeWorkerResponse,
  makeWorkerResult,
  normalizeIncomingWorkerRequest
} from "./worker-protocol";

let initPromise: Promise<unknown> | null = null;
const cancelledIds = new Set<number>();

function serializeWorkerError(err: unknown): string {
  const error = (err && typeof err === "object") ? (err as Record<string, unknown>) : {};
  const name = String(error.name || "Error").trim();
  const message = String(error.message || err || "").trim();
  const stack = String(error.stack || "").trim();
  const lines: string[] = [];

  if (name || message) {
    lines.push(`${name}${message ? `: ${message}` : ""}`.trim());
  }
  if (stack) {
    const stackLines = stack.split(/\r?\n/).filter(Boolean);
    for (const line of stackLines) {
      if (!lines.includes(line)) {
        lines.push(line);
      }
    }
  }

  return lines.join("\n").trim();
}

function toBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

async function gzipSourceToUrlPayload(source: unknown): Promise<string> {
  const rawBytes = new TextEncoder().encode(String(source || ""));
  if (!("CompressionStream" in globalThis) || rawBytes.length <= 2048) {
    return `raw.${toBase64Url(rawBytes)}`;
  }

  try {
    const stream = new CompressionStream("gzip");
    const writer = stream.writable.getWriter();
    await writer.write(rawBytes);
    await writer.close();
    const buffer = await new Response(stream.readable).arrayBuffer();
    return `gz.${toBase64Url(new Uint8Array(buffer))}`;
  } catch {
    return `raw.${toBase64Url(rawBytes)}`;
  }
}

async function ensureWasm(): Promise<void> {
  if (!initPromise) {
    initPromise = initWasm();
  }
  await initPromise;
}

self.addEventListener("message", async (event: MessageEvent) => {
  const normalized = normalizeIncomingWorkerRequest(event?.data);
  if (!normalized) {
    return;
  }

  const { id, type: kind, payload } = normalized;
  const payloadRecord = payload as Record<string, unknown>;

  if (kind === WORKER_REQUEST.CANCEL) {
    cancelledIds.add(id);
    self.postMessage(makeWorkerCancelled(id));
    return;
  }

  try {
    await ensureWasm();
    let result: unknown;

    if (kind === WORKER_REQUEST.COMPILE) {
      if (cancelledIds.delete(id)) {
        self.postMessage(makeWorkerCancelled(id));
        return;
      }

      const source = String(payloadRecord.source || "");
      const includeExplain = Boolean(payloadRecord.includeExplain);
      const workerCompileStartMs = performance.now();
      result = typeof compile_fresco_with_options === "function"
        ? compile_fresco_with_options(source, includeExplain)
        : compile_fresco(source);
      const workerCompileEndMs = performance.now();
      if (result && typeof result === "object") {
        (result as Record<string, unknown>).worker_timings = {
          worker_total_ms: workerCompileEndMs - workerCompileStartMs
        };
      }
      if (cancelledIds.delete(id)) {
        self.postMessage(makeWorkerCancelled(id));
        return;
      }
      self.postMessage(makeWorkerResult(id, true, result));
      return;
    }

    if (kind === WORKER_REQUEST.COMPILE_BUNDLE) {
      if (cancelledIds.delete(id)) {
        self.postMessage(makeWorkerCancelled(id));
        return;
      }

      const filesRaw = payloadRecord.files && typeof payloadRecord.files === "object"
        ? (payloadRecord.files as Record<string, unknown>)
        : {};
      const files: Record<string, string> = Object.fromEntries(
        Object.entries(filesRaw).map(([name, value]) => [name, String(value ?? "")])
      );
      const entrypoint = String(payloadRecord.entrypoint || "main.fr");
      const includeExplain = Boolean(payloadRecord.includeExplain);
      const workerCompileStartMs = performance.now();
      result = compile_fresco_bundle(files, entrypoint, includeExplain);
      const workerCompileEndMs = performance.now();
      if (result && typeof result === "object") {
        (result as Record<string, unknown>).worker_timings = {
          worker_total_ms: workerCompileEndMs - workerCompileStartMs
        };
      }
      if (cancelledIds.delete(id)) {
        self.postMessage(makeWorkerCancelled(id));
        return;
      }
      self.postMessage(makeWorkerResult(id, true, result));
      return;
    }

    switch (kind) {
      case WORKER_REQUEST.LEX:
        result = fresco_lex_tokens_with_files(String(payloadRecord.source || ""), String(payloadRecord.filename || "main.fr"), payloadRecord.files || null);
        break;
      case WORKER_REQUEST.COMPLETION:
        result = fresco_completion_items_with_files(
          String(payloadRecord.source || ""),
          Number(payloadRecord.cursorUtf8) || 0,
          String(payloadRecord.filename || "main.fr"),
          payloadRecord.files || null
        );
        break;
      case WORKER_REQUEST.ENCODE_SHARED_SOURCE:
        result = await gzipSourceToUrlPayload(payloadRecord.source || "");
        break;
      case WORKER_REQUEST.QUERY_SPAN:
        result = fresco_query_span(
          String(payloadRecord.source || ""),
          Number(payloadRecord.spanStart) || 0,
          Number(payloadRecord.spanEnd) || 0,
          payloadRecord.files
        );
        break;
      case WORKER_REQUEST.COMPILE_VARIANT:
        result = fresco_compile_variant(
          String(payloadRecord.source || ""),
          Number(payloadRecord.spanStart) || 0,
          Number(payloadRecord.spanEnd) || 0,
          payloadRecord.files
        );
        break;
      case WORKER_REQUEST.COMPILE_VISUALIZER:
        result = fresco_compile_visualizer(
          String(payloadRecord.source || ""),
          Number(payloadRecord.spanStart) || 0,
          Number(payloadRecord.spanEnd) || 0,
          String(payloadRecord.kind || "sparkline"),
          String(payloadRecord.domain || "time"),
          Number(payloadRecord.sweepMax) || 0,
          payloadRecord.files
        );
        break;
      case WORKER_REQUEST.LANGUAGE_PROFILE:
        result = fresco_language_profile();
        break;
      case WORKER_REQUEST.WASM_VERSION:
        result = { version: fresco_wasm_version(), buildMode: fresco_wasm_build_mode() };
        break;
      default:
        self.postMessage(makeWorkerResponse(id, false, null, `unknown worker request type: ${kind}`));
        return;
    }

    self.postMessage(makeWorkerResponse(id, true, result));
  } catch (err) {
    const message = serializeWorkerError(err);
    if (kind === WORKER_REQUEST.COMPILE || kind === WORKER_REQUEST.COMPILE_BUNDLE) {
      self.postMessage(makeWorkerResult(id, false, null, message || "compile worker failed"));
    } else {
      self.postMessage(makeWorkerResponse(id, false, null, message || "worker request failed"));
    }
  }
});
