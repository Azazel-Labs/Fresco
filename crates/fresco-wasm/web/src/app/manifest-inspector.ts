import { chooseEvaluationVariantEntry, type EvaluationManifestSurface } from "../preview/evaluation-selection";
import { chooseManifestPipelineForMaterial, findManifestPipelinesByMaterial } from "../preview/renderer-manifest";

type ManifestSurfaceLike = EvaluationManifestSurface & {
  name?: unknown;
  mesh_passes?: Array<{ pass: string; entries?: Array<{ stage: string; function: string; entry: string }> }>;
  settings?: { recipe_conditions?: Record<string, boolean> };
};

type ManifestInspectorDeps = {
  hostEl: HTMLElement | null;
  manifest: any;
  selectedSurfaceName: string | null;
  runtimeFilePaths?: string[];
  onSelectSurface?: (surfaceName: string) => void;
  onOpenRuntimeFile?: (filePath: string) => void;
};

export type ManifestInspectorState = {
  surfaceNames: string[];
  selectedSurfaceName: string;
  runtimeFilePaths: string[];
  selectedEntry: string;
  declaredVertexEntry: string;
  declaredFragmentEntry: string;
  selectedSurfaceMaterial: string;
  selectedLightingPipeline: string;
  pipelinesForSurface: Array<{ name: string; type: string; passCount: number }>;
  variants: Array<{
    entry: string;
    isSelected: boolean;
    bindings: Array<{ axis: string; value: string }>;
  }>;
};

function text(value: unknown): string {
  return String(value ?? "").trim();
}

function createRow(label: string, value: string): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "manifest-inspector-row";

  const key = document.createElement("span");
  key.className = "manifest-inspector-key";
  key.textContent = label;

  const val = document.createElement("span");
  val.className = "manifest-inspector-value";
  val.textContent = value;

  row.appendChild(key);
  row.appendChild(val);
  return row;
}

export function buildManifestInspectorState(
  manifest: any,
  selectedSurfaceName: string | null,
  runtimeFilePaths: string[] = [],
): ManifestInspectorState | null {
  const surfaces: ManifestSurfaceLike[] = Array.isArray(manifest?.surfaces) ? (manifest.surfaces as ManifestSurfaceLike[]) : [];
  if (surfaces.length === 0) {
    return null;
  }

  const surfaceNames = surfaces.map((surface) => text(surface?.name) || "(unnamed)");
  const resolvedSelectedName = text(selectedSurfaceName) || surfaceNames[0];
  const selectedSurface = surfaces.find((surface) => text(surface?.name) === resolvedSelectedName) || surfaces[0];
  const selectedSurfaceMaterial = text((selectedSurface as { material_ty?: unknown } | null)?.material_ty);
  const selectedEntry = text(chooseEvaluationVariantEntry(selectedSurface));
  const techniqueDraw = manifest?.techniques?.filter((t: any) => t.surface === selectedSurface.name)
    .flatMap((t: any) => t.steps).find((s: any) => s.operation.kind === "draw")?.operation;
  const recipe = manifest?.renderers?.find((renderer: any) => renderer.selected);
  const meshStep = recipe?.steps?.find((step: any) => step.domain === "mesh"
    && (!step.condition || selectedSurface.settings?.recipe_conditions?.[step.condition]));
  const meshEntries = (meshStep ? selectedSurface.mesh_passes?.find(pass => pass.pass === meshStep.pass) : selectedSurface.mesh_passes?.[0])?.entries || [];
  const meshEntry = (stage: string, fn: string | undefined) =>
    meshEntries.find(entry => entry.stage === stage && (!fn || entry.function === fn))?.entry;
  const selectedLightingPipeline = text(
    chooseManifestPipelineForMaterial(manifest, selectedSurfaceMaterial, "lighting")?.name,
  );
  const pipelinesForSurface = findManifestPipelinesByMaterial(manifest, selectedSurfaceMaterial)
    .map((pipeline) => ({
      name: pipeline.name,
      type: pipeline.type || "(unknown)",
      passCount: pipeline.passes.length,
    }));

  const variants = (Array.isArray(selectedSurface?.evaluation_variants) ? selectedSurface.evaluation_variants : [])
    .map((variant: any) => {
      const entry = text(variant?.entry);
      const bindings = (Array.isArray(variant?.bindings) ? variant.bindings : [])
        .map((binding: any) => ({
          axis: text(binding?.axis) || "axis",
          value: text(binding?.value) || "(none)",
        }));
      return {
        entry,
        isSelected: entry.length > 0 && entry === selectedEntry,
        bindings,
      };
    });

  return {
    surfaceNames,
    selectedSurfaceName: text(selectedSurface?.name) || surfaceNames[0],
    runtimeFilePaths: Array.from(new Set((runtimeFilePaths || []).map((path) => text(path)).filter((path) => path.length > 0))),
    selectedEntry,
    declaredVertexEntry: text(techniqueDraw?.vertex ?? meshEntry("vertex", meshStep?.vertex)),
    declaredFragmentEntry: text(techniqueDraw?.fragment ?? meshEntry("fragment", meshStep?.entry)),
    selectedSurfaceMaterial,
    selectedLightingPipeline,
    pipelinesForSurface,
    variants,
  };
}

export function renderManifestInspector({
  hostEl,
  manifest,
  selectedSurfaceName,
  runtimeFilePaths,
  onSelectSurface,
  onOpenRuntimeFile,
}: ManifestInspectorDeps): void {
  if (!hostEl) return;
  hostEl.innerHTML = "";

  const state = buildManifestInspectorState(manifest, selectedSurfaceName, runtimeFilePaths || []);
  if (!state) {
    hostEl.textContent = "No surface metadata in manifest.";
    return;
  }

  const layout = document.createElement("div");
  layout.className = "manifest-inspector-layout";

  const detailsCol = document.createElement("div");
  detailsCol.className = "manifest-inspector-col manifest-inspector-details";

  const contentCol = document.createElement("div");
  contentCol.className = "manifest-inspector-col manifest-inspector-content";

  const pickerWrap = document.createElement("div");
  pickerWrap.className = "manifest-inspector-picker";

  const pickerLabel = document.createElement("label");
  pickerLabel.textContent = "Surface";
  pickerLabel.className = "manifest-inspector-picker-label";

  const picker = document.createElement("select");
  picker.className = "manifest-inspector-select";
  for (const surfaceName of state.surfaceNames) {
    const option = document.createElement("option");
    option.value = surfaceName;
    option.textContent = surfaceName;
    option.selected = option.value === state.selectedSurfaceName;
    picker.appendChild(option);
  }

  picker.addEventListener("change", () => {
    const name = text(picker.value);
    if (name && onSelectSurface) {
      onSelectSurface(name);
    }
  });

  pickerWrap.appendChild(pickerLabel);
  pickerWrap.appendChild(picker);
  detailsCol.appendChild(pickerWrap);

  layout.appendChild(detailsCol);

  const runtimeTitle = document.createElement("h3");
  runtimeTitle.className = "manifest-inspector-subhead";
  runtimeTitle.textContent = "Runtime Content";
  contentCol.appendChild(runtimeTitle);
  contentCol.appendChild(createRow("Declared vertex stage", state.declaredVertexEntry || "(none)"));
  contentCol.appendChild(createRow("Declared fragment stage", state.declaredFragmentEntry || "(none)"));

  const pipelineTitle = document.createElement("h3");
  pipelineTitle.className = "manifest-inspector-subhead";
  pipelineTitle.textContent = "Pipelines";
  contentCol.appendChild(pipelineTitle);

  if (state.pipelinesForSurface.length === 0) {
    contentCol.appendChild(createRow("Matched", "none"));
  } else {
    contentCol.appendChild(createRow("Matched", String(state.pipelinesForSurface.length)));
    contentCol.appendChild(createRow("Preferred lighting", state.selectedLightingPipeline || "(none)"));
    for (const pipeline of state.pipelinesForSurface) {
      contentCol.appendChild(
        createRow(
          pipeline.name,
          `${pipeline.type} (${pipeline.passCount} pass${pipeline.passCount === 1 ? "" : "es"})`,
        ),
      );
    }
  }

  if (state.runtimeFilePaths.length === 0) {
    const emptyRuntime = document.createElement("p");
    emptyRuntime.className = "manifest-inspector-empty";
    emptyRuntime.textContent = "No injected runtime files for this source.";
    contentCol.appendChild(emptyRuntime);
  } else {
    const runtimeList = document.createElement("div");
    runtimeList.className = "manifest-runtime-files";
    for (const filePath of state.runtimeFilePaths) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "manifest-runtime-file";
      button.textContent = filePath;
      button.title = `Open ${filePath}`;
      button.addEventListener("click", () => {
        if (onOpenRuntimeFile) {
          onOpenRuntimeFile(filePath);
        }
      });
      runtimeList.appendChild(button);
    }
    contentCol.appendChild(runtimeList);
  }

  const variantsTitle = document.createElement("h3");
  variantsTitle.className = "manifest-inspector-subhead";
  variantsTitle.textContent = "Lighting metadata variants";
  contentCol.appendChild(variantsTitle);

  if (state.variants.length === 0) {
    const empty = document.createElement("p");
    empty.className = "manifest-inspector-empty";
    empty.textContent = "No evaluation_variants for this surface.";
    contentCol.appendChild(empty);
    layout.appendChild(contentCol);
    hostEl.appendChild(layout);
    return;
  }

  const list = document.createElement("div");
  list.className = "manifest-inspector-variants";

  for (const variant of state.variants) {
    const card = document.createElement("div");
    const entry = text(variant.entry);
    card.className = `manifest-inspector-variant${variant.isSelected ? " is-selected" : ""}`;

    const head = document.createElement("div");
    head.className = "manifest-inspector-variant-head";

    const entryEl = document.createElement("code");
    entryEl.textContent = entry || "(missing entry)";
    head.appendChild(entryEl);

    if (variant.isSelected) {
      const badge = document.createElement("span");
      badge.className = "manifest-inspector-selected-badge";
      badge.textContent = "preferred metadata";
      head.appendChild(badge);
    }

    card.appendChild(head);

    if (variant.bindings.length > 0) {
      const bindingList = document.createElement("ul");
      bindingList.className = "manifest-inspector-bindings";
      for (const binding of variant.bindings) {
        const item = document.createElement("li");
        item.textContent = `${binding.axis}: ${binding.value}`;
        bindingList.appendChild(item);
      }
      card.appendChild(bindingList);
    }

    list.appendChild(card);
  }

  contentCol.appendChild(list);
  layout.appendChild(contentCol);
  hostEl.appendChild(layout);
}
