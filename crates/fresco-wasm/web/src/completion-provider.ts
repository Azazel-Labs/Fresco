import type { CompletionItem } from "./generated/wasm-contracts/CompletionItem";

type LocalFallbackCompletionItem = Partial<CompletionItem> & {
  type?: string;
  kind?: string;
  receiverKinds?: string[];
  receiver_kinds?: string[];
};

type CompletionSourceItem = CompletionItem | LocalFallbackCompletionItem | null | undefined;

type MonacoCompletionItem = {
  label: string;
  type: string;
  insertText: string;
  detail: string;
  boost: number;
  allowed: boolean;
  info?: string;
  signature?: string;
  documentation?: string;
  snippet?: string;
  receiverKinds: string[];
};

function buildCompletionInfo(item: CompletionSourceItem): string | undefined {
  const explicitInfo = typeof item?.info === "string" ? item.info.trim() : "";
  if (explicitInfo) {
    return explicitInfo;
  }

  const signature = typeof item?.signature === "string" ? item.signature.trim() : "";
  const documentation = typeof item?.documentation === "string" ? item.documentation.trim() : "";

  if (signature && documentation) {
    return `${signature}\n\n${documentation}`;
  }
  if (signature) {
    return signature;
  }
  if (documentation) {
    return documentation;
  }
  return undefined;
}

function buildFallbackCompletions(
  model: { getValue(): string; getOffsetAt(position: unknown): number },
  position: unknown,
  fallbackItemsFn: ((ctx: { model: unknown; position: unknown; source: string; prefix: string }) => CompletionSourceItem[] | null | undefined) | null
): MonacoCompletionItem[] {
  if (typeof fallbackItemsFn !== "function") {
    return [];
  }

  const source = model.getValue();
  const offset = model.getOffsetAt(position);
  const head = source.slice(0, Math.max(0, offset));
  const match = /[A-Za-z_][A-Za-z0-9_]*$/.exec(head);
  const prefix = match ? match[0] : "";
  const loweredPrefix = prefix.toLowerCase();

  const rawItems = fallbackItemsFn({ model, position, source, prefix }) || [];
  const uniq = new Set<string>();
  const results: MonacoCompletionItem[] = [];

  for (const rawItem of rawItems) {
    const mapped = mapWasmCompletionItem(rawItem);
    if (!mapped || !mapped.allowed) {
      continue;
    }
    const key = mapped.label.toLowerCase();
    if (uniq.has(key)) {
      continue;
    }
    if (loweredPrefix && !mapped.label.toLowerCase().startsWith(loweredPrefix)) {
      continue;
    }
    uniq.add(key);
    results.push(mapped);
  }

  return results;
}

function linePrefixAtOffset(source: string, offset: number): string {
  const cursor = Math.max(0, Math.min(Number(offset) || 0, source.length));
  const head = source.slice(0, cursor);
  const lineStart = head.lastIndexOf("\n") + 1;
  return head.slice(lineStart);
}

function isPipeContext(source: string, offset: number): boolean {
  return /\|>\s*[A-Za-z_0-9]*$/.test(linePrefixAtOffset(source, offset));
}

function isFunctionCompletion(item: CompletionSourceItem): boolean {
  const itemObj = item && typeof item === "object" ? (item as Record<string, unknown>) : null;
  const typeValue = itemObj && "type" in itemObj ? itemObj.type : "";
  const kindValue = itemObj && "kind" in itemObj ? itemObj.kind : "";
  const kind = String(typeValue || kindValue || "").toLowerCase();
  return kind.includes("function") || kind.includes("method");
}

function normalizeReceiverKinds(item: CompletionSourceItem): string[] {
  const itemObj = item && typeof item === "object" ? (item as Record<string, unknown>) : null;
  const rawCamel = itemObj && "receiverKinds" in itemObj ? itemObj.receiverKinds : undefined;
  const rawSnake = itemObj && "receiver_kinds" in itemObj ? itemObj.receiver_kinds : undefined;
  const raw = rawCamel ?? rawSnake;
  if (!Array.isArray(raw)) {
    return [];
  }

  return raw
    .map((receiver) => String(receiver || "").trim().toLowerCase())
    .filter(Boolean);
}

function hasShapePipelineReceiverSignature(item: CompletionSourceItem): boolean {
  const signature = String(item?.signature || "").trim().toLowerCase();
  return signature.startsWith("shape |>") || signature.startsWith("layer |>");
}

function isShapePipelineCompletion(item: CompletionSourceItem): boolean {
  if (!isFunctionCompletion(item)) {
    return false;
  }

  const receiverKinds = normalizeReceiverKinds(item);
  if (receiverKinds.includes("shape") || receiverKinds.includes("layer")) {
    return true;
  }

  if (hasShapePipelineReceiverSignature(item)) {
    return true;
  }

  return false;
}

function mergeUniqueByLabel(primary: MonacoCompletionItem[], additional: MonacoCompletionItem[]): MonacoCompletionItem[] {
  const merged: MonacoCompletionItem[] = [];
  const seen = new Set<string>();

  for (const item of [...(primary || []), ...(additional || [])]) {
    const label = String(item?.label || "").trim();
    if (!label) {
      continue;
    }
    const key = label.toLowerCase();
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    merged.push(item);
  }

  return merged;
}

function mapWasmCompletionItem(item: CompletionSourceItem): MonacoCompletionItem | null {
  const safeItem = item || {};
  const label = String(item?.label || "").trim();
  if (!label) {
    return null;
  }

  const insertTextRaw = item?.insert_text ?? item?.label ?? "";
  const insertText = String(insertTextRaw);
  const detailRaw = typeof item?.detail === "string" ? item.detail.trim() : "";
  const signatureRaw = typeof item?.signature === "string" ? item.signature.trim() : "";

  return {
    label,
    type: String(item?.kind || "variable"),
    insertText,
    detail: detailRaw || signatureRaw,
    boost: Number.isFinite((safeItem as { boost?: unknown }).boost)
      ? Number((safeItem as { boost?: unknown }).boost)
      : 0,
    allowed: item?.allowed !== false,
    info: buildCompletionInfo(item),
    signature: signatureRaw || undefined,
    documentation:
      typeof item?.documentation === "string" && item.documentation.trim()
        ? item.documentation.trim()
        : undefined,
    snippet:
      typeof item?.snippet === "string" && item.snippet.trim()
        ? item.snippet.trim()
        : undefined,
    receiverKinds: normalizeReceiverKinds(item)
  };
}

export function createWasmCompletionProvider({
  completionItemsFn,
  utf16OffsetToUtf8Byte,
  fallbackItemsFn = null
}: {
  completionItemsFn: (source: string, cursorUtf8: number) => Promise<CompletionSourceItem[] | null | undefined> | CompletionSourceItem[] | null | undefined;
  utf16OffsetToUtf8Byte: (source: string, utf16Offset: number) => number;
  fallbackItemsFn?: ((ctx: { model: unknown; position: unknown; source: string; prefix: string }) => CompletionSourceItem[] | null | undefined) | null;
}) {
  return async (model: { getValue(): string; getOffsetAt(position: unknown): number }, position: unknown) => {
    try {
      const source = model.getValue();
      const utf16Offset = model.getOffsetAt(position);
      const cursorUtf8 = utf16OffsetToUtf8Byte(source, utf16Offset);
      const raw = await completionItemsFn(source, cursorUtf8);
      if (!Array.isArray(raw)) {
        return buildFallbackCompletions(model, position, fallbackItemsFn);
      }
      const mapped = raw.map((item) => mapWasmCompletionItem(item)).filter((item): item is MonacoCompletionItem => Boolean(item));
      const allowedItems = mapped.filter((item) => item.allowed);
      // An enum-only compiler response is a constrained value slot. Adding the
      // general catalog here would reintroduce unrelated functions and keywords.
        if (allowedItems.length > 0 && allowedItems.every((item) => item.type === "enum" || item.detail.startsWith("cell member (") || item.detail === "Local (contour)")) {
        return allowedItems;
      }
      if (allowedItems.some((item) => item.type === "property" && item.label.endsWith(":"))) {
        return allowedItems;
      }
      // Local catalog entries must not override a contextual compiler rejection.
      const rejectedLabels = new Set(mapped.filter((item) => !item.allowed).map((item) => item.label.toLowerCase()));
      const fallback = buildFallbackCompletions(model, position, fallbackItemsFn)
        .filter((item) => !rejectedLabels.has(item.label.toLowerCase()));

      if (isPipeContext(source, utf16Offset)) {
        const shapeScoped = allowedItems.filter((item) => isShapePipelineCompletion(item));
        if (shapeScoped.length > 0) {
          return mergeUniqueByLabel(shapeScoped, fallback.filter((item) => isShapePipelineCompletion(item)));
        }

        const fallbackFunctions = fallback.filter(
          (item) => isShapePipelineCompletion(item)
        );
        if (fallbackFunctions.length > 0) {
          return mergeUniqueByLabel([], fallbackFunctions);
        }
      }

      if (allowedItems.length > 0) {
        return mergeUniqueByLabel(allowedItems, fallback);
      }
      return fallback;
    } catch {
      return buildFallbackCompletions(model, position, fallbackItemsFn);
    }
  };
}

export { mapWasmCompletionItem };
