import {
  canvasNameFromEntryFunction,
  normalizeManifestAssetPath,
  normalizeManifestParamDefault,
} from "./renderer-support";
import type { ManifestRoot } from "../generated/wasm-contracts/ManifestRoot";

export type ManifestPipelineDescriptor = {
  name: string;
  material: string | null;
  type: string;
  passes: string[];
  semanticSummary: {
    rasterPasses: number;
    computePasses: number;
    additivePasses: number;
    resourceFlowEdges: number;
  } | null;
};

function normalizePipeline(entry: any): ManifestPipelineDescriptor | null {
  const name = String(entry?.name || "").trim();
  if (name.length === 0) {
    return null;
  }

  const material = String(entry?.material || "").trim() || null;
  const type = String(entry?.type || "").trim();
  const passes = Array.isArray(entry?.passes)
    ? entry.passes
        .map((pass: unknown) => String(pass || "").trim())
        .filter((pass: string) => pass.length > 0)
    : [];
  const semanticSummary = entry?.semantic_summary && typeof entry.semantic_summary === "object"
    ? {
        rasterPasses: Number(entry.semantic_summary.raster_passes ?? 0),
        computePasses: Number(entry.semantic_summary.compute_passes ?? 0),
        additivePasses: Number(entry.semantic_summary.additive_passes ?? 0),
        resourceFlowEdges: Number(entry.semantic_summary.resource_flow_edges ?? 0),
      }
    : null;
  return { name, material, type, passes, semanticSummary };
}

function chooseBestPipeline(candidates: ManifestPipelineDescriptor[], selected?: string): ManifestPipelineDescriptor | null {
  return candidates.find(entry => entry.name === selected)
    ?? [...candidates].sort((a, b) => a.name.localeCompare(b.name))[0] ?? null;
}

export function findManifestPipelinesByMaterial(
  manifest: ManifestRoot | null | undefined,
  materialName: string,
  pipelineType?: string,
): ManifestPipelineDescriptor[] {
  const root = manifest as unknown as { pipelines?: unknown } | null;
  const pipelines = Array.isArray(root?.pipelines) ? (root?.pipelines as any[]) : [];
  const wantedMaterial = String(materialName || "").trim();
  const wantedType = String(pipelineType || "").trim();

  return pipelines
    .map(normalizePipeline)
    .filter((entry): entry is ManifestPipelineDescriptor => Boolean(entry))
    .filter((entry) => (wantedMaterial.length > 0 ? entry.material === wantedMaterial : true))
    .filter((entry) => (wantedType.length > 0 ? entry.type === wantedType : true));
}

export function chooseManifestPipelineForMaterial(
  manifest: ManifestRoot | null | undefined,
  materialName: string,
  pipelineType = "lighting",
): ManifestPipelineDescriptor | null {
  const selected = manifest?.renderers?.find(renderer => renderer.selected)?.pipeline;
  const candidates = findManifestPipelinesByMaterial(manifest, materialName, pipelineType);
  const bestTyped = chooseBestPipeline(candidates, selected);
  if (bestTyped) {
    return bestTyped;
  }

  const fallback = findManifestPipelinesByMaterial(manifest, materialName);
  return chooseBestPipeline(fallback, selected);
}

/** Playground preview policy; execution still receives one explicit surface. */
export function chooseManifestPreviewSurface(manifest: ManifestRoot | null | undefined, name?: string | null) {
  const surfaces = (manifest?.surfaces ?? []).filter(surface => surface.name !== "fresco_scene_ground");
  if (name) return surfaces.find(surface => surface.name === name);
  return surfaces.find(surface => {
    const material = surface.material_ty?.trim();
    return material && chooseManifestPipelineForMaterial(manifest, material, "lighting");
  }) ?? surfaces[0];
}

export function parseManifestStorageParams(manifest, fnName) {
  if (!manifest || typeof manifest !== "object") {
    return [];
  }

  const canvasName = canvasNameFromEntryFunction(fnName);
  const canvases = Array.isArray(manifest.canvases) ? manifest.canvases : [];
  const canvas = canvases.find((entry) => entry.name === canvasName) || canvases[0];
  const storageParams = Array.isArray(canvas?.storage_params) ? canvas.storage_params : [];
  return storageParams.map((param) => ({
    name: String(param.name || ""),
    type: String(param.type || ""),
    paramType: param.param_type ?? null,
    group: Number(param.group ?? 0),
    binding: Number(param.binding ?? 0),
    default: normalizeManifestParamDefault(param.default, param.type)
  }));
}

export function parseManifestCanvasName(manifest) {
  if (!manifest || typeof manifest !== "object") {
    return "";
  }
  const canvases = Array.isArray(manifest.canvases) ? manifest.canvases : [];
  const canvasName = String(canvases[0]?.name || "").trim();
  return canvasName;
}

export function resolveEntryFunctionName({
  wgsl,
  manifest,
  hasFunctionSignature,
  detectEntryFunction,
}: {
  wgsl: string;
  manifest: ManifestRoot | null | undefined;
  hasFunctionSignature: (fnName: string) => boolean;
  detectEntryFunction: (wgslText: string) => string;
}) {
  const manifestCanvasName = parseManifestCanvasName(manifest);
  const manifestFnName = manifestCanvasName ? `fresco_${manifestCanvasName}` : "";
  if (manifestFnName && hasFunctionSignature(manifestFnName)) {
    return manifestFnName;
  }

  const reflectedFnName = detectEntryFunction(wgsl);
  if (manifestFnName && reflectedFnName && manifestFnName !== reflectedFnName) {
    console.warn(
      `Preview entry mismatch: manifest requested ${manifestFnName}, WGSL reflection chose ${reflectedFnName}`
    );
  }
  return reflectedFnName || manifestFnName || "";
}

export function parseManifestParams(manifest, fnName) {
  if (!manifest || typeof manifest !== "object") {
    return [];
  }

  const canvasName = canvasNameFromEntryFunction(fnName);
  const canvases = Array.isArray(manifest.canvases) ? manifest.canvases : [];
  const canvas = canvases.find((entry) => entry.name === canvasName) || canvases[0];
  const params = Array.isArray(canvas?.params) ? canvas.params : [];
  return params.map((param) => ({
    name: param.name,
    type: param.type,
    callType: param.type,
    paramType: param.param_type ?? null,
    default: normalizeManifestParamDefault(param.default, param.type),
    min: typeof param.min === "number" ? param.min : null,
    max: typeof param.max === "number" ? param.max : null
  }));
}

export function parseManifestTextures(manifest, fnName) {
  if (!manifest || typeof manifest !== "object") {
    return [];
  }

  const entryName = canvasNameFromEntryFunction(fnName);
  const surfaces = Array.isArray(manifest.surfaces) ? manifest.surfaces : [];
  const surface = surfaces.find((entry) => entry.name === entryName) || surfaces[0];
  const surfaceTextures = Array.isArray(surface?.textures) ? surface.textures : [];

  if (surfaceTextures.length > 0) {
    return surfaceTextures
      .map((entry) => ({
        name: String(entry?.name || "").trim(),
        group: Number(entry?.group),
        binding: Number(entry?.binding),
        defaultAsset: normalizeManifestAssetPath(entry?.metadata?.default_asset || ""),
        textureType: String(entry?.metadata?.texture_type || "").trim() || null,
      }))
      .filter((entry) => entry.name && entry.group === 1 && Number.isInteger(entry.binding))
      .sort((a, b) => a.binding - b.binding);
  }

  const canvases = Array.isArray(manifest.canvases) ? manifest.canvases : [];
  const canvas = canvases.find((entry) => entry.name === entryName) || canvases[0];
  const textures = Array.isArray(canvas?.textures) ? canvas.textures : [];
  return textures
    .map((entry) => ({
      name: String(entry?.name || "").trim(),
      group: Number(entry?.group),
      binding: Number(entry?.binding),
      defaultAsset: normalizeManifestAssetPath(entry?.metadata?.default_asset || ""),
      textureType: String(entry?.metadata?.texture_type || "").trim() || null,
    }))
    .filter((entry) => entry.name && entry.group === 1 && Number.isInteger(entry.binding))
    .sort((a, b) => a.binding - b.binding);
}
