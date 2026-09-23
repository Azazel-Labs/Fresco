type ExampleEntry = {
  id: string;
  source: string;
};

type ExampleSelectionControllerDeps = {
  exampleSelectEl: HTMLSelectElement;
  filteredExamples: ExampleEntry[];
  examplesById: Map<string, ExampleEntry>;
  customExampleValue: string;
  getSourceMode: () => string;
  setSourceMode: (mode: string, options?: Record<string, unknown>) => void;
  resetVirtualFiles: (mainSource?: string) => void;
  loadVirtualBundle: (bundle: Map<string, string>) => void;
  buildRuntimeBundleForExample?: (exampleId: string, baseBundle: Map<string, string>) => Map<string, string>;
  bundleForExampleId: (exampleId: string) => Map<string, string> | null;
  setEditorSource: (text: string) => void;
  getMainSource: () => string;
  scheduleUrlSync: () => void;
  queueCompile: (options: { delayMs?: number; cancelInFlight?: boolean; deferPreviewBuild?: boolean }) => void;
  setStatus: (text: string, klass?: string) => void;
  compilingPreviewStatus: () => string;
  waitingForShaderStatus: () => string;
  rendererClearPreview: () => void;
  sourceEditor: {
    getModel: () => unknown;
    setDiagnostics: (markers: unknown[]) => void;
    setScrollPosition: (position: { scrollTop: number; scrollLeft: number }) => void;
  };
  wgslEditor: {
    setValue: (text: string) => void;
    setScrollPosition: (position: { scrollTop: number; scrollLeft: number }) => void;
  };
  showDiagnostics: (diags: unknown[], model: unknown) => void;
  renderShaderParams: (paramDefs: unknown[], textureDefs?: unknown[]) => void;
  resetPreviewTime: () => void;
  explainEl: HTMLElement;
  manifestEl: HTMLElement;
  diagnosticsEl: HTMLElement;
  previewStatusEl: HTMLElement;
};

export type ExampleSelectionController = {
  populateExamples: () => void;
  onExampleSelected: () => void;
};

export function createExampleSelectionController(deps: ExampleSelectionControllerDeps): ExampleSelectionController {
  const {
    exampleSelectEl,
    filteredExamples,
    examplesById,
    customExampleValue,
    getSourceMode,
    setSourceMode,
    resetVirtualFiles,
    loadVirtualBundle,
    buildRuntimeBundleForExample,
    bundleForExampleId,
    setEditorSource,
    getMainSource,
    scheduleUrlSync,
    queueCompile,
    setStatus,
    compilingPreviewStatus,
    waitingForShaderStatus,
    rendererClearPreview,
    sourceEditor,
    wgslEditor,
    showDiagnostics,
    renderShaderParams,
    resetPreviewTime,
    explainEl,
    manifestEl,
    diagnosticsEl,
    previewStatusEl,
  } = deps;

  function populateExamples(): void {
    const groups = new Map<string, HTMLOptGroupElement>();

    for (const example of filteredExamples) {
      const [rawGroup, ...rest] = example.id.split("/");
      const groupLabel = rawGroup
        ? rawGroup
            .replace(/^\d+\)\s*/, "")
            .replace(/\b\w/g, (ch) => ch.toUpperCase())
        : "Other";
      let itemLabel = rest.length > 0 ? rest.join("/") : example.id;

      if (itemLabel.endsWith("/main")) {
        itemLabel = itemLabel.slice(0, -"/main".length);
      }
      if (!itemLabel) {
        itemLabel = "main";
      }

      let group = groups.get(rawGroup);
      if (!group) {
        group = document.createElement("optgroup");
        group.label = groupLabel;
        groups.set(rawGroup, group);
        exampleSelectEl.appendChild(group);
      }

      const option = document.createElement("option");
      option.value = example.id;
      option.textContent = `  ${itemLabel}`;
      group.appendChild(option);
    }
  }

  function onExampleSelected(): void {
    const selected = exampleSelectEl.value;

    const sourceModel = sourceEditor.getModel();
    rendererClearPreview();
    resetPreviewTime();
    sourceEditor.setDiagnostics([]);
    showDiagnostics([], sourceModel);
    renderShaderParams([], []);
    wgslEditor.setValue("");
    explainEl.textContent = "";
    manifestEl.textContent = "";
    previewStatusEl.textContent = waitingForShaderStatus();

    sourceEditor.setScrollPosition({ scrollTop: 0, scrollLeft: 0 });
    wgslEditor.setScrollPosition({ scrollTop: 0, scrollLeft: 0 });
    diagnosticsEl.scrollTop = 0;
    explainEl.scrollTop = 0;
    manifestEl.scrollTop = 0;

    if (selected === customExampleValue) {
      if (getSourceMode() !== "custom") {
        setSourceMode("custom");
        scheduleUrlSync();
      }
      return;
    }

    if (selected === "") {
      resetVirtualFiles("");
      setSourceMode("blank");
      setEditorSource("");
      scheduleUrlSync();
      queueCompile({ delayMs: 0, cancelInFlight: true, deferPreviewBuild: true });
      return;
    }

    const example = examplesById.get(selected);
    if (!example) {
      return;
    }

    const baseBundle = bundleForExampleId(example.id)
      ? new Map(bundleForExampleId(example.id)!)
      : new Map([["main.fr", example.source]]);
    const resolvedBundle = buildRuntimeBundleForExample
      ? buildRuntimeBundleForExample(example.id, baseBundle)
      : baseBundle;
    loadVirtualBundle(resolvedBundle);

    setSourceMode("example", {
      exampleId: example.id,
      baseline: example.source
    });
    setStatus("Compiling", "compiling");
    previewStatusEl.textContent = compilingPreviewStatus();
    requestAnimationFrame(() => {
      setEditorSource(getMainSource() || example.source);
      scheduleUrlSync();
      queueCompile({ delayMs: 0, cancelInFlight: true, deferPreviewBuild: true });
    });
  }

  return {
    populateExamples,
    onExampleSelected,
  };
}
