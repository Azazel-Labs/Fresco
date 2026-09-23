export function isSourceBlank(source: string): boolean {
  return String(source || "").trim().length === 0;
}

export function computeCompileDebounceMs(
  source: string,
  thresholds: {
    hugeThreshold: number;
    hugeDelay: number;
    largeThreshold: number;
    largeDelay: number;
    defaultDelay: number;
  }
): number {
  const len = String(source || "").length;
  if (len >= thresholds.hugeThreshold) {
    return thresholds.hugeDelay;
  }
  if (len >= thresholds.largeThreshold) {
    return thresholds.largeDelay;
  }
  return thresholds.defaultDelay;
}

export function toBase64Url(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i += 1) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

export function fromBase64Url(input: string): Uint8Array {
  const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
  const pad = normalized.length % 4;
  const padded = pad === 0 ? normalized : normalized + "=".repeat(4 - pad);
  const binary = atob(padded);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    out[i] = binary.charCodeAt(i);
  }
  return out;
}

async function gzipBytes(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream("gzip");
  const writer = stream.writable.getWriter();
  await writer.write(new Uint8Array(bytes));
  await writer.close();
  const buffer = await new Response(stream.readable).arrayBuffer();
  return new Uint8Array(buffer);
}

async function gunzipBytes(bytes: Uint8Array): Promise<Uint8Array> {
  const stream = new DecompressionStream("gzip");
  const writer = stream.writable.getWriter();
  await writer.write(new Uint8Array(bytes));
  await writer.close();
  const buffer = await new Response(stream.readable).arrayBuffer();
  return new Uint8Array(buffer);
}

export function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => {
      setTimeout(() => reject(new Error("timeout")), ms);
    })
  ]);
}

export async function encodeSharedSource(params: {
  source: string;
  requestWorkerQuery: (kind: string, payload: Record<string, unknown>, timeoutMs?: number) => Promise<unknown>;
  workerQueryTimeoutMs: number;
  urlCodePrefixRaw: string;
}): Promise<string> {
  const { source, requestWorkerQuery, workerQueryTimeoutMs, urlCodePrefixRaw } = params;
  try {
    const encoded = await requestWorkerQuery("encode_shared_source", { source }, workerQueryTimeoutMs);
    if (typeof encoded === "string" && encoded) {
      return encoded;
    }
  } catch {
    // Fall through to local raw encoding below.
  }

  const rawBytes = new TextEncoder().encode(source);
  return `${urlCodePrefixRaw}${toBase64Url(rawBytes)}`;
}

export function encodeRawSource(source: string, urlCodePrefixRaw: string): string {
  const rawBytes = new TextEncoder().encode(source);
  return `${urlCodePrefixRaw}${toBase64Url(rawBytes)}`;
}

export async function decodeSharedSource(params: {
  payload: string | null;
  urlCodePrefixGzip: string;
  urlCodePrefixRaw: string;
  timeoutMs?: number;
}): Promise<string | null> {
  const {
    payload,
    urlCodePrefixGzip,
    urlCodePrefixRaw,
    timeoutMs = 150
  } = params;

  if (!payload) {
    return null;
  }

  try {
    if (payload.startsWith(urlCodePrefixGzip)) {
      const encoded = payload.slice(urlCodePrefixGzip.length);
      const zipped = fromBase64Url(encoded);
      if (!("DecompressionStream" in globalThis)) {
        return null;
      }
      const rawBytes = await withTimeout(gunzipBytes(zipped), timeoutMs);
      return new TextDecoder().decode(rawBytes);
    }
    if (payload.startsWith(urlCodePrefixRaw)) {
      const encoded = payload.slice(urlCodePrefixRaw.length);
      const rawBytes = fromBase64Url(encoded);
      return new TextDecoder().decode(rawBytes);
    }
  } catch {
    return null;
  }

  return null;
}

export function resolveExampleFromUrlParam<T>(paramValue: string | null, examplesById: Map<string, T>): T | null {
  if (!paramValue) {
    return null;
  }

  const normalized = paramValue.replace(/\\/g, "/").replace(/^\.\//, "");
  return examplesById.get(normalized) || null;
}
