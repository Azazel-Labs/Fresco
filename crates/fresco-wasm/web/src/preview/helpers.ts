import {
  MAX_RUNTIME_PARAMS,
  PREVIEW_UNIFORM_PARAM_BASE
} from "./uniforms";

type RuntimeParamDef = {
  name: string;
  type?: unknown;
  slotOffset?: unknown;
};

type RuntimeRenderer = {
  paramDefs?: RuntimeParamDef[];
  paramValues?: Map<string, unknown>;
  normalizeParamValue?: (def: RuntimeParamDef, raw: unknown) => unknown;
};

type PreviewUniformTarget = ArrayLike<number> & { [index: number]: number };

const COLOR_LIKE_PARAM_TYPES = new Set(["color", "vec4<f32>"]);
const INTEGER_PARAM_TYPES = new Set(["i32", "u32"]);

export function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

export function normalizeTypeName(type: unknown): string {
  return String(type ?? "").replace(/\s+/g, "");
}

export function isColorLikeParamType(type: unknown): boolean {
  return COLOR_LIKE_PARAM_TYPES.has(normalizeTypeName(type));
}

export function isIntegerParamType(type: unknown): boolean {
  return INTEGER_PARAM_TYPES.has(normalizeTypeName(type));
}

export function entryPrefixInfoFromTypes(types: unknown[]): { hasDelta: boolean; fixedCount: number } {
  const normalizedTypes = types.map((type) => normalizeTypeName(type));
  const hasBasePrefix =
    normalizedTypes.length >= 3 &&
    normalizedTypes[0] === "vec2<f32>" &&
    normalizedTypes[1] === "f32";
  if (!hasBasePrefix) {
    return { hasDelta: false, fixedCount: Math.min(3, normalizedTypes.length) };
  }
  if (normalizedTypes[2] === "vec2<f32>") {
    return { hasDelta: false, fixedCount: 3 };
  }
  if (
    normalizedTypes.length >= 4 &&
    normalizedTypes[2] === "f32" &&
    normalizedTypes[3] === "vec2<f32>"
  ) {
    return { hasDelta: true, fixedCount: 4 };
  }
  return { hasDelta: false, fixedCount: 3 };
}

export function rgbaToHex(rgba: ArrayLike<number> | null | undefined): string {
  const [r, g, b] = Array.isArray(rgba) ? rgba : [1, 1, 1];
  const toHex = (value: unknown) => Math.round(clamp01(Number(value) || 0) * 255).toString(16).padStart(2, "0");
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

export function hexToRgba(hex: unknown, alpha = 1): [number, number, number, number] {
  const normalized = String(hex || "").trim().replace(/^#/, "");
  if (!/^[0-9a-fA-F]{6}$/.test(normalized)) {
    return [1, 1, 1, clamp01(alpha)];
  }

  return [
    parseInt(normalized.slice(0, 2), 16) / 255,
    parseInt(normalized.slice(2, 4), 16) / 255,
    parseInt(normalized.slice(4, 6), 16) / 255,
    clamp01(alpha)
  ];
}

export function createVizRuntimeParamUniformWriter(getRenderer: () => RuntimeRenderer | null | undefined) {
  return function writeVizRuntimeParamUniforms(targetUniform: PreviewUniformTarget | null | undefined) {
    if (!targetUniform) {
      return;
    }

    for (let i = 0; i < MAX_RUNTIME_PARAMS; i += 1) {
      targetUniform[PREVIEW_UNIFORM_PARAM_BASE + i] = 0;
    }

    const renderer = typeof getRenderer === "function" ? getRenderer() : null;
    if (!renderer || !Array.isArray(renderer.paramDefs)) {
      return;
    }

    for (const def of renderer.paramDefs) {
      const offset = Number(def?.slotOffset);
      if (!Number.isFinite(offset) || offset < 0 || offset >= MAX_RUNTIME_PARAMS) {
        continue;
      }

      const raw = renderer.paramValues?.get(def.name);
      const normalized = typeof renderer.normalizeParamValue === "function"
        ? renderer.normalizeParamValue(def, raw)
        : raw;
      const type = normalizeTypeName(def?.type);

      if (isColorLikeParamType(type)) {
        for (let i = 0; i < 4 && (offset + i) < MAX_RUNTIME_PARAMS; i += 1) {
          targetUniform[PREVIEW_UNIFORM_PARAM_BASE + offset + i] = Number((normalized as ArrayLike<number> | null | undefined)?.[i]) || 0;
        }
        continue;
      }

      if (type === "bool") {
        targetUniform[PREVIEW_UNIFORM_PARAM_BASE + offset] = normalized ? 1.0 : 0.0;
        continue;
      }

      targetUniform[PREVIEW_UNIFORM_PARAM_BASE + offset] = Number(normalized) || 0;
    }
  };
}
