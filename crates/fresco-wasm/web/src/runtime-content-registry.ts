// Bundle the canonical integration directly. Catalog synchronization replaces
// src/examples and must never make the compiler's engine bundle incomplete.
const enginePrefix = "../../../../integrations/example-engine/engine/";
const engineModules = import.meta.glob("../../../../integrations/example-engine/engine/**/*.fr", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

type RuntimeContentFile = {
  path: string;
  source: string;
};

export type RuntimeContentProfile = {
  id: string;
  label: string;
  files: RuntimeContentFile[];
};

const SURFACE_LIGHTING_PROFILE: RuntimeContentProfile = {
  id: "surface-lighting-lab",
  label: "Surface Lighting Lab",
  files: Object.entries(engineModules)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([path, source]) => ({ path: `engine/${path.slice(enginePrefix.length)}`, source })),
};

const DEFAULT_PROFILES: RuntimeContentProfile[] = [SURFACE_LIGHTING_PROFILE];

export function getRuntimeContentProfilesForExample(_exampleId: string): RuntimeContentProfile[] {
  return DEFAULT_PROFILES;
}

export function getRuntimeContentFilePathsForExample(exampleId: string): string[] {
  const profiles = getRuntimeContentProfilesForExample(exampleId);
  const files: string[] = [];
  for (const profile of profiles) {
    for (const file of profile.files) {
      files.push(file.path);
    }
  }
  return Array.from(new Set(files));
}

export type RendererMode = string;

export function buildCompileFilesBundle(
  baseBundle: Map<string, string>,
  exampleId: string,
  renderer?: RendererMode,
): Map<string, string> {
  const out = new Map(baseBundle);
  const profiles = getRuntimeContentProfilesForExample(exampleId);
  for (const profile of profiles) {
    for (const file of profile.files) {
      if (!out.has(file.path)) {
        out.set(file.path, file.source);
      }
    }
  }
  const root = out.get("engine/engine.fr");
  if (root && !baseBundle.has("engine/engine.fr")) out.set("engine/engine.fr", `${root}\nimport "scenes/preview.fr"\n`);
  if (renderer) out.set("fresco.config.json", JSON.stringify({ ...JSON.parse(out.get("fresco.config.json") || "{}"), renderer }));
  return out;
}

export function appendRuntimeContentToBundle(
  baseBundle: Map<string, string>,
  exampleId: string,
): Map<string, string> {
  return buildCompileFilesBundle(baseBundle, exampleId);
}
