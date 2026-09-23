import type { ManifestRoot } from "../generated/wasm-contracts/ManifestRoot";
import { isColorLikeParamType } from "./helpers";
import { canonicalizeParamType, flattenParamComponents, normalizeScalarValueByType,
  parseArrayParamType, parseDynamicArrayParamType, scalarSlotWidthForType,
  totalSlotWidthForType } from "./param-codegen";
import { normalizePreviewLoopSeconds, wrapPreviewTime } from "./controls";

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function firstDefined(values: unknown[]): unknown {
  for (const value of values) {
    if (value !== undefined && value !== null) {
      return value;
    }
  }
  return undefined;
}

function normalizeManifestDefaultForType(type: unknown, fallback: unknown): unknown {
  if (fallback === undefined || fallback === null) {
    return undefined;
  }

  const arrayInfo = parseArrayParamType(type);
  if (arrayInfo) {
    if (Array.isArray(fallback)) {
      return fallback;
    }
    if (isPlainObject(fallback)) {
      const byCollection = firstDefined([
        fallback.values,
        fallback.items,
        fallback.elements,
        fallback.data,
      ]);
      if (Array.isArray(byCollection)) {
        return byCollection;
      }
      const bySingle = firstDefined([fallback.value, fallback.default]);
      if (bySingle !== undefined) {
        return bySingle;
      }
    }
    return fallback;
  }

  const t = canonicalizeParamType(type);
  if (Array.isArray(fallback)) {
    return fallback;
  }

  if (isPlainObject(fallback)) {
    const byCollection = firstDefined([
      fallback.values,
      fallback.items,
      fallback.elements,
      fallback.data,
      fallback.channels,
    ]);
    if (Array.isArray(byCollection)) {
      return byCollection;
    }

    const byScalar = firstDefined([
      fallback.value,
      fallback.default,
      fallback.scalar,
      fallback.f32,
      fallback.i32,
      fallback.u32,
      fallback.bool,
      fallback.number,
      fallback.n,
    ]);
    if (byScalar !== undefined && !t.startsWith("vec") && !t.startsWith("mat") && t !== "color") {
      return byScalar;
    }

    if (t === "color") {
      const rgba = [fallback.r, fallback.g, fallback.b, fallback.a].filter((v) => v !== undefined);
      if (rgba.length > 0) {
        return [fallback.r ?? 0, fallback.g ?? 0, fallback.b ?? 0, fallback.a ?? 1];
      }
    }

    if (t === "vec2<f32>") {
      const xy = [fallback.x, fallback.y].filter((v) => v !== undefined);
      if (xy.length > 0) {
        return [fallback.x ?? 0, fallback.y ?? 0];
      }
    }
    if (t === "vec3<f32>") {
      const xyz = [fallback.x, fallback.y, fallback.z].filter((v) => v !== undefined);
      if (xyz.length > 0) {
        return [fallback.x ?? 0, fallback.y ?? 0, fallback.z ?? 0];
      }
    }
    if (t === "vec4<f32>") {
      const xyzw = [fallback.x, fallback.y, fallback.z, fallback.w].filter((v) => v !== undefined);
      if (xyzw.length > 0) {
        return [fallback.x ?? 0, fallback.y ?? 0, fallback.z ?? 0, fallback.w ?? 0];
      }
    }
  }

  return fallback;
}

function stableJson(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return "";
  }
}

export function paramDefChanged(previousDef: any, nextDef: any): boolean {
  if (!previousDef) {
    return true;
  }
  return (
    String(previousDef.type || "") !== String(nextDef.type || "")
    || String(previousDef.callType || "") !== String(nextDef.callType || "")
    || Number(previousDef.min) !== Number(nextDef.min)
    || Number(previousDef.max) !== Number(nextDef.max)
    || stableJson(previousDef.default) !== stableJson(nextDef.default)
  );
}

interface PreviewTextureOption {
  url: string;
}

interface PreviewTextureOptionsResult {
  options: PreviewTextureOption[];
  defaultUrl: string;
}

export interface PreviewControllerDeps {
  previewResolutionEl?: HTMLElement | null;
  previewFpsEl?: HTMLElement | null;
  previewStatusEl?: HTMLElement | null;
  getSourceExampleId?: () => string;
  textureOptionsForName: (exampleId: string, textureName: string, defaultAsset: unknown) => PreviewTextureOptionsResult;
}

// Shared editor state only: no device, shader generation, or GPU resources.
export abstract class PreviewController {
  deps: PreviewControllerDeps;
  canvas;
  gpuReady = false;
  onTextureDefsChanged = null;
  textureDefs = [];
  textureSelections = new Map();
  stagedShader = null;
  stagedBuildPromise = null;
  asyncShaderBuildPending = false;
  asyncShaderBuildStartedAtMs = 0;
  lastAsyncShaderCompletedAtMs = 0;
  lastAsyncShaderTimings = null;
  lastAsyncShaderError = "";
  simTimeSeconds = 0;
  deltaTimeSeconds = 1 / 60;
  lastTickMs = null;
  playbackPaused = false;
  playbackRate = 1.0;
  playbackLoopSeconds = 0;
  elapsed = 0;
  errorMessage = "";
  onRuntimeDiagnostic = null;
  lastRuntimeIssue = "";
  onParamDefsChanged = null;
  onParamValueChanged = null;
  paramDefs = [];
  paramValues = new Map();
  touchedParamNames = new Set();
  fpsFrameCount = 0;
  fpsSampleStartMs = performance.now();
  resizeObserver = null;

  abstract setShader(wgsl: string, manifest: ManifestRoot | null): Promise<any>;

  constructor(canvasEl: HTMLCanvasElement, deps: PreviewControllerDeps) {
    this.canvas = canvasEl;
    this.deps = deps;
  }

  setPlaybackPaused(paused) {
    this.playbackPaused = Boolean(paused);
    if (this.playbackPaused) {
      this.lastTickMs = null;
      return;
    }
    this.lastTickMs = performance.now();
  }

  setPlaybackRate(rate) {
    const nextRate = Number(rate);
    if (!Number.isFinite(nextRate)) {
      return;
    }
    this.playbackRate = Math.max(-16, Math.min(nextRate, 16));
  }

  setPlaybackLoopDuration(seconds) {
    this.playbackLoopSeconds = normalizePreviewLoopSeconds(seconds);
    this.simTimeSeconds = wrapPreviewTime(this.simTimeSeconds, this.playbackLoopSeconds);
    this.elapsed = this.simTimeSeconds;
    this.lastTickMs = performance.now();
  }

  setPlaybackTime(seconds) {
    const next = wrapPreviewTime(seconds, this.playbackLoopSeconds);
    if (!Number.isFinite(next)) {
      return;
    }
    this.simTimeSeconds = next;
    this.elapsed = next;
    this.deltaTimeSeconds = 0;
    this.lastTickMs = performance.now();
  }

  stepPlaybackFrames(frameCount = 1, baseFps = 60) {
    const frames = Number(frameCount);
    const fps = Number(baseFps);
    if (!Number.isFinite(frames) || !Number.isFinite(fps) || fps <= 0) {
      return;
    }
    const dt = frames / fps;
    this.simTimeSeconds = wrapPreviewTime(this.simTimeSeconds + dt, this.playbackLoopSeconds);
    this.elapsed = this.simTimeSeconds;
    this.deltaTimeSeconds = dt;
    this.lastTickMs = performance.now();
  }

  tickTime(nowMs = performance.now()) {
    if (this.playbackPaused) {
      return;
    }
    if (!Number.isFinite(this.lastTickMs)) {
      this.lastTickMs = nowMs;
      return;
    }
    const dt = Math.max(0, Math.min((nowMs - this.lastTickMs) / 1000, 0.25));
    const dtScaled = dt * this.playbackRate;
    this.simTimeSeconds = wrapPreviewTime(this.simTimeSeconds + dtScaled, this.playbackLoopSeconds);
    this.deltaTimeSeconds = dtScaled;
    this.elapsed = this.simTimeSeconds;
    this.lastTickMs = nowMs;
  }

  slotWidthForType(type) {
    return totalSlotWidthForType(type);
  }

  cloneParamValue(value) {
    if (Array.isArray(value)) {
      return value.map((item) => this.cloneParamValue(item));
    }
    if (value && typeof value === "object") {
      return { ...value };
    }
    return value;
  }

  defaultValueForType(type, fallback = undefined) {
    const normalizedFallback = normalizeManifestDefaultForType(type, fallback);

    const dynamicArrayInfo = parseDynamicArrayParamType(type);
    if (dynamicArrayInfo) {
      // Dynamic arrays are backed by storage buffers. Default to the fallback
      // values if provided as an array, otherwise start empty.
      if (Array.isArray(normalizedFallback)) {
        return normalizedFallback;
      }
      return [];
    }

    const arrayInfo = parseArrayParamType(type);
    if (arrayInfo) {
      const elementWidth = scalarSlotWidthForType(arrayInfo.elementType);
      if (elementWidth <= 0 || arrayInfo.length <= 0) {
        return null;
      }
      if (normalizedFallback !== undefined) {
        const source = Array.isArray(normalizedFallback)
          ? normalizedFallback
          : [normalizedFallback];
        const out = [];
        const sourceIsPerElement = source.length === arrayInfo.length;
        for (let itemIndex = 0; itemIndex < arrayInfo.length; itemIndex += 1) {
          const sourceValue = elementWidth === 1
            ? source[itemIndex]
            : (sourceIsPerElement
              ? source[itemIndex]
              : source.slice(itemIndex * elementWidth, (itemIndex + 1) * elementWidth));
          const normalized = flattenParamComponents(arrayInfo.elementType, sourceValue, 0);
          if (Array.isArray(normalized) && normalized.length === elementWidth) {
            out.push(...normalized);
          } else {
            for (let i = 0; i < elementWidth; i += 1) {
              out.push(0);
            }
          }
        }
        return out;
      }
      return Array.from({ length: arrayInfo.length * elementWidth }, () => 0);
    }

    const t = canonicalizeParamType(type);
    if (normalizedFallback !== undefined && (t === "f32" || t === "i32" || t === "u32" || t === "bool")) {
      return normalizeScalarValueByType(t, normalizedFallback, t === "f32" ? 0.72 : 0);
    }
    if (t === "f32") {
      return 0.72;
    }
    if (t === "i32") {
      return 0;
    }
    if (t === "u32") {
      return 0;
    }
    if (t === "bool") {
      return false;
    }
    const componentWidth = scalarSlotWidthForType(t);
    if (componentWidth > 1) {
      if (normalizedFallback !== undefined) {
        const normalized = flattenParamComponents(t, normalizedFallback, t === "color" ? [1, 1, 1, 1] : 0);
        if (Array.isArray(normalized) && normalized.length === componentWidth) {
          return normalized;
        }
      }
      if (t === "color") {
        return [1, 1, 1, 1];
      }
      return Array.from({ length: componentWidth }, () => 0);
    }
    if (isColorLikeParamType(t)) {
      if (normalizedFallback !== undefined) {
        const normalized = flattenParamComponents("color", normalizedFallback, [1, 1, 1, 1]);
        if (Array.isArray(normalized) && normalized.length === 4) {
          return normalized;
        }
      }
      return [1, 1, 1, 1];
    }

    if (normalizedFallback !== undefined) {
      return normalizeScalarValueByType(t, normalizedFallback, t === "f32" ? 0.72 : 0);
    }

    return null;
  }

  normalizeParamValue(def, value) {
    const type = typeof def === "string" ? def : def.type;
    const fallback = typeof def === "string" ? undefined : def.default;
    const paramType = typeof def === "string" ? null : (def.paramType ?? null);

    const isDynamicArray = paramType
      ? paramType.name === "array" && paramType.size == null
      : parseDynamicArrayParamType(type) !== null;
    const dynamicElementType = isDynamicArray
      ? (paramType?.params?.[0] ?? parseDynamicArrayParamType(type)?.elementType ?? "f32")
      : null;

    if (isDynamicArray) {
      // Dynamic arrays hold a flat list of component values (variable length).
      if (!Array.isArray(value)) {
        const defaultVal = this.defaultValueForType(type, fallback);
        return Array.isArray(defaultVal) ? defaultVal.slice() : [];
      }
      // Normalize each element component to a number.
      const elementType = dynamicElementType!;
      const componentWidth = scalarSlotWidthForType(elementType);
      if (componentWidth <= 0) {
        return value.map((v) => Number(v) || 0);
      }
      const elementCount = Math.floor(value.length / componentWidth);
      const out: number[] = [];
      for (let i = 0; i < elementCount; i += 1) {
        const slice = value.slice(i * componentWidth, (i + 1) * componentWidth);
        const normalized = flattenParamComponents(elementType, slice, 0);
        if (normalized) {
          out.push(...normalized);
        } else {
          for (let j = 0; j < componentWidth; j += 1) {
            out.push(0);
          }
        }
      }
      return out;
    }

    const arrayInfo = parseArrayParamType(type);
    if (arrayInfo) {
      const base = this.defaultValueForType(type, fallback);
      if (!Array.isArray(base)) {
        return null;
      }
      const componentWidth = scalarSlotWidthForType(arrayInfo.elementType);
      if (componentWidth <= 0) {
        return null;
      }
      const out = [];
      const source = Array.isArray(value) ? value : base;
      const sourceIsPerElement = Array.isArray(source) && source.length === arrayInfo.length;
      const baseIsPerElement = Array.isArray(base) && base.length === arrayInfo.length;
      for (let itemIndex = 0; itemIndex < arrayInfo.length; itemIndex += 1) {
        const sourceValue = componentWidth === 1
          ? source[itemIndex]
          : (sourceIsPerElement
            ? source[itemIndex]
            : source.slice(itemIndex * componentWidth, (itemIndex + 1) * componentWidth));
        const fallbackValue = componentWidth === 1
          ? base[itemIndex]
          : (baseIsPerElement
            ? base[itemIndex]
            : base.slice(itemIndex * componentWidth, (itemIndex + 1) * componentWidth));
        const normalized = flattenParamComponents(arrayInfo.elementType, sourceValue, fallbackValue);
        if (!Array.isArray(normalized) || normalized.length !== componentWidth) {
          for (let k = 0; k < componentWidth; k += 1) {
            out.push(Number(fallbackValue?.[k]) || 0);
          }
          continue;
        }
        for (const slotValue of normalized) {
          out.push(slotValue);
        }
      }
      return out;
    }

    const t = canonicalizeParamType(type);
    if (t === "bool") {
      return value === true || value === "true";
    }

    const width = scalarSlotWidthForType(t);
    if (width > 1) {
      const base = this.defaultValueForType(type, fallback);
      const normalized = flattenParamComponents(t, value, base);
      return Array.isArray(normalized) ? normalized : base;
    }

    const num = Number(value);
    if (!Number.isFinite(num)) {
      return this.defaultValueForType(type, fallback);
    }

    if (t === "u32") {
      return Math.max(0, Math.round(num));
    }
    if (t === "i32") {
      return Math.round(num);
    }
    return num;
  }

  syncParamDefs(nextDefs) {
    const previousDefsByName = new Map(this.paramDefs.map((def) => [def.name, def]));
    this.paramDefs = nextDefs.map((def) => ({
      ...def,
      slotWidth: Number.isFinite(def.slotWidth) ? def.slotWidth : this.slotWidthForType(def.type)
    }));
    const allowed = new Set(nextDefs.map((d) => d.name));
    for (const key of this.paramValues.keys()) {
      if (!allowed.has(key)) {
        this.paramValues.delete(key);
      }
    }
    for (const key of this.touchedParamNames) {
      if (!allowed.has(key)) {
        this.touchedParamNames.delete(key);
      }
    }
    for (const def of this.paramDefs) {
      const previousDef = previousDefsByName.get(def.name);
      const hasCurrentValue = this.paramValues.has(def.name);
      const wasTouched = this.touchedParamNames.has(def.name);
      const shouldResetToDefault = !hasCurrentValue || paramDefChanged(previousDef, def) || !wasTouched;
      if (shouldResetToDefault) {
        const seeded = this.normalizeParamValue(def, undefined);
        this.paramValues.set(def.name, seeded);
        this.touchedParamNames.delete(def.name);
      } else {
        const normalized = this.normalizeParamValue(def, this.paramValues.get(def.name));
        this.paramValues.set(def.name, normalized);
      }
    }
    if (typeof this.onParamDefsChanged === "function") {
      this.onParamDefsChanged(this.paramDefs);
    }
  }

  async maybeBuildStagedShader() {
    if (!this.stagedShader) {
      return;
    }
    if (this.stagedBuildPromise) {
      return this.stagedBuildPromise;
    }

    const next = this.stagedShader;
    this.stagedShader = null;
    this.asyncShaderBuildPending = true;
    this.asyncShaderBuildStartedAtMs = performance.now();
    this.lastAsyncShaderError = "";
    this.stagedBuildPromise = this.setShader(next.wgsl, next.manifest)
      .then((shaderTimings) => {
        this.lastAsyncShaderTimings = shaderTimings && typeof shaderTimings === "object"
          ? { ...shaderTimings }
          : null;
        this.lastAsyncShaderCompletedAtMs = performance.now();
        return shaderTimings;
      })
      .catch((err) => {
        this.lastAsyncShaderError = String(err?.message || err || "preview shader build failed");
        this.lastAsyncShaderCompletedAtMs = performance.now();
        throw err;
      })
      .finally(() => {
        this.asyncShaderBuildPending = false;
        this.stagedBuildPromise = null;
      });
    return this.stagedBuildPromise;
  }

  getAsyncShaderBuildSnapshot() {
    const now = performance.now();
    return {
      pending: Boolean(this.asyncShaderBuildPending),
      running_ms: this.asyncShaderBuildPending && this.asyncShaderBuildStartedAtMs > 0
        ? Math.max(0, now - this.asyncShaderBuildStartedAtMs)
        : 0,
      last_completed_age_ms: this.lastAsyncShaderCompletedAtMs > 0
        ? Math.max(0, now - this.lastAsyncShaderCompletedAtMs)
        : 0,
      last_error: this.lastAsyncShaderError || "",
      last_timings: this.lastAsyncShaderTimings && typeof this.lastAsyncShaderTimings === "object"
        ? { ...this.lastAsyncShaderTimings }
        : null
    };
  }

}
