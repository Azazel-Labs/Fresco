export function normalizeManifestAssetPath(path) {
  return String(path || "")
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\.\//, "")
    .replace(/^examples\//, "");
}

export function canvasNameFromEntryFunction(fnName) {
  return String(fnName || "")
    .replace(/^fresco_/, "")
    .replace(/_pass\d+$/, "");
}

function snapManifestFloat(value) {
  if (!Number.isFinite(value)) {
    return value;
  }
  const snapped = Number(value.toPrecision(7));
  return Number.isFinite(snapped) ? snapped : value;
}

function normalizeManifestNumericShape(value, type = "") {
  if (typeof value === "number") {
    return /\b(?:u32|i32|int|uvec[234]|ivec[234])\b/.test(type) ? value : snapManifestFloat(value);
  }
  if (Array.isArray(value)) {
    return value.map((entry) => normalizeManifestNumericShape(entry, type));
  }
  if (value && typeof value === "object") {
    const out = {};
    for (const [key, entry] of Object.entries(value)) {
      out[key] = normalizeManifestNumericShape(entry, type);
    }
    return out;
  }
  return value;
}

export function normalizeManifestParamDefault(rawDefault, type = "") {
  if (typeof rawDefault === "number") {
    return normalizeManifestNumericShape(rawDefault, type);
  }

  if (!rawDefault || typeof rawDefault !== "object") {
    return rawDefault;
  }

  if (rawDefault instanceof Map) {
    if (Array.isArray(rawDefault.get("values"))) {
      return normalizeManifestNumericShape(rawDefault.get("values"), type);
    }
    if (Array.isArray(rawDefault.get("items"))) {
      return normalizeManifestNumericShape(rawDefault.get("items"), type);
    }
    if (Array.isArray(rawDefault.get("elements"))) {
      return normalizeManifestNumericShape(rawDefault.get("elements"), type);
    }
    if (Array.isArray(rawDefault.get("data"))) {
      return normalizeManifestNumericShape(rawDefault.get("data"), type);
    }
    return normalizeManifestParamDefault(Object.fromEntries(rawDefault.entries()), type);
  }

  if (typeof rawDefault.toJSON === "function") {
    try {
      const asJson = rawDefault.toJSON();
      if (asJson !== rawDefault) {
        return normalizeManifestParamDefault(asJson, type);
      }
    } catch {
      // Ignore toJSON errors and continue with best-effort normalization.
    }
  }

  if (typeof rawDefault.entries === "function" && Object.keys(rawDefault).length === 0) {
    try {
      const fromEntries = Object.fromEntries(rawDefault.entries());
      if (fromEntries && Object.keys(fromEntries).length > 0) {
        return normalizeManifestParamDefault(fromEntries, type);
      }
    } catch {
      // Ignore non-standard iterable failures.
    }
  }

  if (Array.isArray(rawDefault)) {
    return normalizeManifestNumericShape(rawDefault, type);
  }
  if (Array.isArray(rawDefault.values)) {
    return normalizeManifestNumericShape(rawDefault.values, type);
  }
  return normalizeManifestNumericShape(rawDefault, type);
}

export function textureOptionsForName(exampleId, textureName, defaultAsset, availableOptions) {
  const normalizedDefault = normalizeManifestAssetPath(defaultAsset);
  const examplePrefix = String(exampleId || "")
    .trim()
    .replace(/\\/g, "/")
    .replace(/\/[^/]*$/, "/");
  const defaultBase = normalizedDefault.split("/").pop() || "";

  const scored = availableOptions.map((entry) => {
    let score = 0;
    if (normalizedDefault && entry.relPath === normalizedDefault) {
      score += 100;
    }
    if (defaultBase && entry.relPath.endsWith(`/${defaultBase}`)) {
      score += 20;
    }
    if (examplePrefix && entry.relPath.startsWith(examplePrefix)) {
      score += 10;
    }
    if (textureName && entry.relPath.toLowerCase().includes(String(textureName).toLowerCase())) {
      score += 2;
    }
    return { ...entry, score };
  })
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score || a.relPath.localeCompare(b.relPath, undefined, { numeric: true, sensitivity: "base" }));

  const options = [];
  const seen = new Set();
  for (const entry of scored) {
    if (seen.has(entry.url)) {
      continue;
    }
    seen.add(entry.url);
    options.push({ relPath: entry.relPath, url: entry.url });
  }

  const defaultUrl = options.find((entry) => entry.relPath === normalizedDefault)?.url
    || options[0]?.url
    || null;

  return {
    options,
    defaultUrl
  };
}
