function normalizeTypeName(type: unknown): string {
  return String(type ?? "").replace(/\s+/g, "");
}

function isGeneratedHelperFunctionName(name: unknown): boolean {
  return /_scatter_l\d+$/.test(String(name || ""));
}

function isCanvasEntrySignature(types: string[]): boolean {
  const normalizedTypes = types.map((type) => normalizeTypeName(type));
  return (
    (normalizedTypes.length >= 3 && normalizedTypes[0] === "vec2<f32>" && normalizedTypes[1] === "f32" && normalizedTypes[2] === "vec2<f32>") ||
    (normalizedTypes.length >= 4 && normalizedTypes[0] === "vec2<f32>" && normalizedTypes[1] === "f32" && normalizedTypes[2] === "f32" && normalizedTypes[3] === "vec2<f32>")
  );
}

type EntryCandidate = {
  name: string;
  types: string[];
};

export function detectFrescoEntryFunction(wgsl: unknown): string | null {
  const fnRx = /fn\s+(fresco_[A-Za-z0-9_]+)\s*\(([^)]*)\)/g;
  const candidates: EntryCandidate[] = [];
  for (const match of String(wgsl || "").matchAll(fnRx)) {
    const name = match[1];
    const params = String(match[2] || "")
      .split(",")
      .map((param) => param.trim())
      .filter(Boolean);
    const types = params.map((param) => {
      const idx = param.indexOf(":");
      return idx >= 0 ? normalizeTypeName(param.slice(idx + 1)) : "";
    });
    candidates.push({ name, types });
  }

  const preferred = candidates.find((candidate) =>
    !isGeneratedHelperFunctionName(candidate.name) && isCanvasEntrySignature(candidate.types)
  );
  if (preferred) {
    return preferred.name;
  }

  const fallback = candidates.find((candidate) => !isGeneratedHelperFunctionName(candidate.name));
  if (fallback) {
    return fallback.name;
  }

  return candidates.length > 0 ? candidates[0].name : null;
}
