import { createImplementationSelection } from "./app/implementation-selection";
import { editEngineProperty, renderEngineProperties } from "./app/engine-properties";
import "./style.scss";

import type { EditorFacade } from "./editor-monaco-adapter";
import {
  createVizRuntimeParamUniformWriter,
  normalizeTypeName,
} from "./preview/helpers";
import { createWasmCompletionProvider } from "./completion-provider";
import { VizZoneManager, configureVizZoneManagerDeps } from "./viz/viz-zone-manager";
import { isRetryableWorkerFailure, TRANSIENT_WORKER_RETRY_DELAY_MS } from "./worker-retry";
import { createWorkerRuntime } from "./worker-runtime";
import {
  createPreviewControls,
} from "./preview/controls";
import { renderShaderParamsUi } from "./shader-params-ui";
import {
  createBuiltinHelpController,
  normalizeBuiltinReceiverKinds,
  preferredBuiltinSignature
} from "./builtin-help";
import { textureOptionsForName as supportTextureOptionsForName } from "./preview/renderer-support";
import {
  mapDiagnosticsToMarkers as mapDiagnosticsToMarkersUi,
  normalizeSeverity as normalizeSeverityUi,
  showDiagnostics as showDiagnosticsUi,
  utf8LengthForCodePoint,
} from "./app/diagnostics-ui";
import {
  computeCompileDebounceMs as computeCompileDebounceMsValue,
  decodeSharedSource as decodeSharedSourceValue,
  encodeRawSource as encodeRawSourceValue,
  encodeSharedSource as encodeSharedSourceValue,
  isSourceBlank as isSourceBlankValue,
  resolveExampleFromUrlParam as resolveExampleFromUrlParamValue,
} from "./app/source-url-codec";
import {
  compilingPreviewStatus,
  formatQueuedCompileStatus,
  noValidShaderStatus,
  previewUnavailableStatus,
  waitingForShaderStatus,
} from "./app/compile-status";
import { createCompileQueueController } from "./app/compile-queue";
import { createVirtualFilesController } from "./app/virtual-files";
import { createSourceStateController } from "./app/source-state";
import { chooseManifestPreviewSurface } from "./preview/renderer-manifest";
import type {
  CompileQueuePayload,
  CompileQueueRequest,
  SourceModeOptions,
  VirtualFilesMap,
} from "./app/controller-types";
import { ExampleEngineRenderer } from "./preview/example-engine-renderer";
import { createVisualizerDevice } from "./preview/visualizer-device";
import type { SurfaceMeshKind } from "./preview/example-engine-renderer";
import type { ManifestRoot } from "./generated/wasm-contracts/ManifestRoot";
import {
  EXAMPLE_ASSET_OPTIONS,
  EXAMPLES_BY_ID,
  FILTERED_EXAMPLES,
  bundleForExampleId
} from "./example-catalog";
import {
  buildBuiltinVizSweepMeta,
  buildNumericEnvFromParams,
  type BuiltinVizSweepMeta,
  buildShaderExplainText,
  resolveAnnotationSweep as resolveAnnotationSweepValue,
  resolveAnnotationSweepMax as resolveAnnotationSweepMaxValue,
} from "./app/annotation-sweep";
import { createDocsOverlayController } from "./app/docs-overlay";
import { createExampleSelectionController } from "./app/example-selection";
import { createExampleBrowser } from "./app/example-browser";
import { createCompileOrchestrator } from "./app/compile-orchestrator";
import { createBootstrapCompileWatchdog } from "./app/bootstrap-watchdog";
import { createEditorHighlightingController } from "./app/editor-highlighting";
import { buildManifestInspectorState, renderManifestInspector } from "./app/manifest-inspector";
import {
  buildCompileFilesBundle as buildBaseCompileFilesBundle,
  type RendererMode,
  getRuntimeContentFilePathsForExample,
} from "./runtime-content-registry";
import {
  bindBootstrapEventHandlers,
  startPreviewFrameLoop,
} from "./app/bootstrap-lifecycle";

const rendererSelect = document.getElementById("preview-renderer") as HTMLSelectElement;
const rendererFromUrl = new URLSearchParams(location.search).get("renderer");
let selectedRenderer: RendererMode | undefined;
const rendererWarning = document.createElement("span");
rendererWarning.setAttribute("role", "status");
rendererSelect.after(rendererWarning);
const implementationControls = document.createElement("fieldset");
implementationControls.className = "static-options implementation-options";
implementationControls.hidden = true;
const staticOptionsEl = document.createElement("fieldset");
staticOptionsEl.className = "static-options";
staticOptionsEl.hidden = true;
let implementationDocumentId = 0;
const implementationSelection = createImplementationSelection(implementationControls, staticOptionsEl, () => {
  virtualFilesController.syncActiveEditorToMap();
  queueCompile({ delayMs: 0, cancelInFlight: true });
});
function buildCompileFilesBundle(files: Map<string, string>, exampleId: string) {
  return implementationSelection.bundle(buildBaseCompileFilesBundle(files, exampleId, selectedRenderer), String(implementationDocumentId));
}
function updateRendererCatalog(manifest: any) {
  const choices = Array.isArray(manifest?.renderers) ? manifest.renderers : [];
  rendererSelect.replaceChildren(...choices.map((choice: any) => new Option(choice.label, choice.id)));
  rendererSelect.disabled = choices.length === 0;
  if (!choices.length) return;
  const requested = selectedRenderer ?? rendererFromUrl;
  const choice = choices.find((c: any) => c.id === requested) ?? choices.find((c: any) => c.default);
  if (!choice) throw new Error("Engine renderer catalog has no default");
  if (requested && requested !== choice.id) {
    rendererWarning.textContent = `Renderer "${requested}" is unavailable; using ${choice.label}.`;
  }
  selectedRenderer = choice.id;
  rendererSelect.value = choice.id;
  if (!choice.selected) queueCompile({ delayMs: 0, cancelInFlight: true });
}
rendererSelect.addEventListener("change", () => {
  selectedRenderer = rendererSelect.value;
  rendererWarning.textContent = "";
  virtualFilesController.syncActiveEditorToMap();
  const url = new URL(location.href);
  url.searchParams.set("renderer", selectedRenderer);
  history.replaceState(null, "", url);
  queueCompile({ delayMs: 0, cancelInFlight: true });
});

const STARTER_SOURCE = "";
const CUSTOM_EXAMPLE_VALUE = "__custom__";
const URL_PARAM_EXAMPLE = "example";
const URL_PARAM_CODE = "code";
const URL_CODE_PREFIX_GZIP = "gz.";
const URL_CODE_PREFIX_RAW = "raw.";
const MAX_SHARE_PARAM_LENGTH = 12000;
const WARNING_SEVERITY_ALIASES = new Set(["warning", "warn"]);
const INFO_SEVERITY_ALIASES = new Set(["info", "information"]);
const VIRTUAL_DIAGNOSTIC_FILES = new Set(["", "playground.fr", "<source>", "source"]);

const writeVizRuntimeParamUniforms = createVizRuntimeParamUniformWriter(() => renderer);

type EditorAdapterModule = typeof import("./editor-monaco-adapter");

let editorAdapterModulePromise: Promise<EditorAdapterModule> | null = null;

function loadEditorAdapterModule(): Promise<EditorAdapterModule> {
  if (!editorAdapterModulePromise) {
    editorAdapterModulePromise = import("./editor-monaco-adapter");
  }
  return editorAdapterModulePromise;
}

function waitForNextPaintFrame(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

type FrescoLanguageConfig = {
  keywords: string[];
  typeKeywords: string[];
  units: string[];
  builtins: string[];
  builtinReference: Array<Record<string, unknown>>;
  spaceTransforms: string[];
  blendModes: string[];
  enumMembers: string[];
  stdlibExports: Array<Record<string, unknown>>;
};

function textureOptionsForName(exampleId: string, textureName: string, defaultAsset: unknown) {
  return supportTextureOptionsForName(exampleId, textureName, defaultAsset, EXAMPLE_ASSET_OPTIONS);
}

const statusEl = document.getElementById("compile-status") as HTMLElement;
const versionEl = document.getElementById("version") as HTMLElement;
const diagnosticsEl = document.getElementById("diagnostics") as HTMLElement;
const explainEl = document.getElementById("explain") as HTMLElement;
const manifestEl = document.getElementById("manifest") as HTMLElement;
const manifestInspectorEl = document.getElementById("manifest-inspector") as HTMLElement;
const workspaceEl = document.querySelector(".workspace");
const exampleSelectEl = document.getElementById("example-select") as HTMLSelectElement;
const exampleBrowser = createExampleBrowser(
  document.getElementById("example-browser") as HTMLElement,
  exampleSelectEl,
  document.getElementById("source-title") as HTMLElement,
);
const docsToggleEl = document.getElementById("docs-toggle");
const canvasEl = document.getElementById("preview-canvas") as HTMLCanvasElement;
const previewPanelEl = document.getElementById("preview-panel");
const paramsPanelEl = document.getElementById("params-panel");
const paramsPanelBodyEl = document.getElementById("params-panel-body");
const paramsTabEl = document.getElementById("params-tab");
const paramsTabButtonEl = document.querySelector('.tab[data-tab="params"]');
const diagnosticsTabButtonEl = document.querySelector('.outputs-panel .tab[data-tab="diagnostics"]');
const inspectorTabButtonEl = document.querySelector('.outputs-panel .tab[data-tab="inspector"]');
const outputsPanelTitleEl = document.querySelector(".outputs-panel .panel-head h2");
const shaderParamsEl = document.createElement("div");
shaderParamsEl.id = "shader-params";
shaderParamsEl.className = "shader-params";
const previewStatusEl = document.getElementById("preview-status") as HTMLElement;
const previewFpsEl = document.getElementById("preview-fps") as HTMLElement;
const previewResolutionEl = document.getElementById("preview-resolution");
const previewPlayButtonEl = document.getElementById("preview-play");
const previewSpinButtonEl = document.getElementById("preview-spin");
const previewFullscreenButtonEl = document.getElementById("preview-fullscreen");
const previewRateSelectEl = document.getElementById("preview-rate") as HTMLSelectElement | null;
const previewStepBackButtonEl = document.getElementById("preview-step-back") as HTMLButtonElement | null;
const previewStepForwardButtonEl = document.getElementById("preview-step-forward");
const previewTimeInputEl = document.getElementById("preview-time-input") as HTMLInputElement | null;
const fileTabsEl = document.getElementById("file-tabs");
const fileTabListEl = document.getElementById("file-tab-list");
const fileTabPickerEl = document.getElementById("file-tab-picker") as HTMLSelectElement | null;
// Old workbook hover/pin system removed - replaced by @viz inline annotations
const COMPILE_TIMEOUT_MS = 45000;
const WORKER_QUERY_TIMEOUT_MS = 15000;
const WORKER_BOOTSTRAP_QUERY_TIMEOUT_MS = 30000;
const BOOTSTRAP_COMPILE_WATCHDOG_MS = 30000;
const COMPILE_DEBOUNCE_MS = 200;
const WORKER_AUTO_RECOVERY_DELAY_MS = 250;
const WORKER_AUTO_RECOVERY_MAX_ATTEMPTS = 3;
const LARGE_SOURCE_DEBOUNCE_THRESHOLD_CHARS = 20000;
const LARGE_SOURCE_DEBOUNCE_MS = 450;
const HUGE_SOURCE_DEBOUNCE_THRESHOLD_CHARS = 50000;
const HUGE_SOURCE_DEBOUNCE_MS = 700;
const LIVE_LEX_HIGHLIGHT_MAX_CHARS = 20000;
const LIVE_LEX_LARGE_DEBOUNCE_MS = 500;
const LIVE_LEX_HUGE_DEBOUNCE_MS = 850;
const WORKER_REQUEST_MAX_ATTEMPTS = 2;
const APP_STARTUP_ORIGIN_MS = performance.now();
type EditorCompletionProvider = NonNullable<Parameters<EditorFacade["setCompletionProvider"]>[0]>;
type EditorSyntaxHighlighter = NonNullable<Parameters<EditorFacade["setSyntaxHighlighter"]>[0]>;
let compileInFlight = false;
let previewPaused = false;
let activeOutputTab = "wgsl";
let selectedInspectorSurfaceName = new URLSearchParams(location.search).get("surface") || "";
let paramsLayoutMode = "hidden";
let paramsLayoutRaf = 0;

const compileUiState = {
  compileToken: 0,
  workerAutoRecoveryAttempts: 0,
  previewBuildDeferred: false,
  previewBuildDeferredTimer: 0,
  firstCompileKickoffAtMs: null,
  startupFirstCompileTelemetryEmitted: false,
};

const DOCS_PAGE_URL = `${import.meta.env.BASE_URL}generated/language-reference.v1.html`;
const docsOverlayController = createDocsOverlayController(docsToggleEl, DOCS_PAGE_URL);

function showDocsOverlay() {
  docsOverlayController.show();
}

// Multi-file virtual filesystem: filename -> source.
// "main.fr" is always present and is the compiler entrypoint.
let virtualFiles: VirtualFilesMap = new Map([["main.fr", ""]]);
let activeFile: string = "main.fr";
const virtualFilesController = createVirtualFilesController({
  fileTabsEl,
  fileTabListEl,
  fileTabPickerEl,
  getEditorValue: () => sourceEditor?.getValue?.() || "",
  setEditorSource: (text) => setEditorSource(text),
  queueCompile: (options) => queueCompile(options)
});

function syncVirtualFileRefs(): void {
  virtualFiles = virtualFilesController.getVirtualFiles();
  activeFile = virtualFilesController.getActiveFile();
}

const DEFAULT_FRESCO_KEYWORDS = [
  "canvas",
  "let",
  "compose",
  "space",
  "param",
  "in",
  "blend",
  "return",
  "if",
  "else",
  "fn",
  "module",
  "use",
  "within",
  "seed",
  "lifetime",
  "respawn",
  "every"
];

const DEFAULT_FRESCO_TYPE_KEYWORDS = ["coord", "signal", "color", "f32", "i32", "u32", "bool", "vec2"];
const DEFAULT_FRESCO_UNITS = ["px", "uv", "deg", "s"];

let frescoLanguageConfig: FrescoLanguageConfig = {
  keywords: [...DEFAULT_FRESCO_KEYWORDS],
  typeKeywords: [...DEFAULT_FRESCO_TYPE_KEYWORDS],
  units: [...DEFAULT_FRESCO_UNITS],
  builtins: [],
  builtinReference: [],
  spaceTransforms: [],
  blendModes: [],
  enumMembers: [],
  stdlibExports: []
};

let builtinVizSweepMetaByName = new Map<string, BuiltinVizSweepMeta>();

let provideFrescoCompletions: EditorCompletionProvider = async () => [];
let provideFrescoSyntaxHighlights: EditorSyntaxHighlighter = () => [];
let sourceEditor: EditorFacade | null = null;
let wgslEditor: EditorFacade | null = null;
let sourceMode = "blank";
let sourceBaseline = STARTER_SOURCE;
let sourceExampleId = "";
let suppressSourceEvents = false;
let lexCacheSource = "";
let lexCacheTokens: unknown[] = [];
let lexUpdateTimer = 0;
let lexUpdateToken = 0;

const editorHighlightingController = createEditorHighlightingController({
  getLexTokens: () => lexCacheTokens,
  getLanguageConfig: () => frescoLanguageConfig,
});

const sourceStateController = createSourceStateController({
  exampleSelectEl,
  getSource: () => virtualFilesController.getMainSource(),
  setEditorSource: (text) => setEditorSource(text),
  isSourceBlank,
  encodeSharedSource,
  encodeRawSource,
  decodeSharedSource,
  resolveExampleFromUrlParam,
  examplesById: EXAMPLES_BY_ID,
  customExampleValue: CUSTOM_EXAMPLE_VALUE,
  maxShareParamLength: MAX_SHARE_PARAM_LENGTH,
  urlParamExample: URL_PARAM_EXAMPLE,
  urlParamCode: URL_PARAM_CODE,
  starterSource: STARTER_SOURCE,
});

function syncSourceStateRefs(): void {
  sourceMode = sourceStateController.getSourceMode();
  sourceBaseline = sourceStateController.getSourceBaseline();
  sourceExampleId = sourceStateController.getSourceExampleId();
  exampleBrowser.sync();
}

const compileQueueController = createCompileQueueController({
  captureInput: () => {
    virtualFilesController.syncActiveEditorToMap();
    syncVirtualFileRefs();
    return {
      source: virtualFilesController.getMainSource(),
      files: buildCompileFilesBundle(virtualFilesController.snapshotFilesOrNull()
        || new Map([["main.fr", virtualFilesController.getMainSource()]]), sourceExampleId || ""),
    };
  },
  computeCompileDebounceMs,
  getCompileInFlight: () => compileInFlight,
  setCompileInFlight: (inFlight) => {
    compileInFlight = inFlight;
  },
  onQueueRequested: ({ deferPreviewBuild }) => {
    // Invalidate results now, before debounce or an older compile finishes.
    compileUiState.compileToken++;
    renderer.cancelPendingShader();
    if (!deferPreviewBuild) {
      compileUiState.previewBuildDeferred = false;
      clearTimeout(compileUiState.previewBuildDeferredTimer);
      compileUiState.previewBuildDeferredTimer = 0;
    }
  },
  onDebounceScheduled: (delayMs, inFlight) => {
    setStatus("Debouncing", "compiling");
    previewStatusEl.textContent = formatQueuedCompileStatus(delayMs, inFlight);
  },
  runCompile: async (payload) => {
    await compileAndRender(payload);
  }
});

const builtinHelpController = createBuiltinHelpController({
  getSourceEditor: () => sourceEditor,
  showDocsOverlay
});

const workerRuntime = createWorkerRuntime({
  compileTimeoutMs: COMPILE_TIMEOUT_MS,
  workerQueryTimeoutMs: WORKER_QUERY_TIMEOUT_MS,
  workerRequestMaxAttempts: WORKER_REQUEST_MAX_ATTEMPTS,
  transientWorkerRetryDelayMs: TRANSIENT_WORKER_RETRY_DELAY_MS,
  workerAutoRecoveryDelayMs: WORKER_AUTO_RECOVERY_DELAY_MS,
  isRetryableWorkerFailure,
  onCompileRecoveryRequested: () => {
    queueCompile({ delayMs: 0, cancelInFlight: false });
  }
});

const requestWorkerQuery = (
  ...args: Parameters<typeof workerRuntime.requestWorkerQuery>
): ReturnType<typeof workerRuntime.requestWorkerQuery> => {
  const [kind, payload, ...rest] = args;
  if (["lex", "completion", "query_span", "compile_variant", "compile_visualizer"].includes(kind)) {
    virtualFilesController.syncActiveEditorToMap();
    const files = buildCompileFilesBundle(virtualFilesController.snapshotFilesOrNull()
      || new Map([["main.fr", virtualFilesController.getMainSource()]]), sourceExampleId || "");
    return workerRuntime.requestWorkerQuery(kind, { ...payload, files: Object.fromEntries(files) }, ...rest);
  }
  return workerRuntime.requestWorkerQuery(...args);
};
const compileInBackground = (
  ...args: Parameters<typeof workerRuntime.compileInBackground>
): ReturnType<typeof workerRuntime.compileInBackground> => workerRuntime.compileInBackground(...args);
const scheduleCompilerRecovery = (
  ...args: Parameters<typeof workerRuntime.scheduleCompilerRecovery>
): ReturnType<typeof workerRuntime.scheduleCompilerRecovery> => workerRuntime.scheduleCompilerRecovery(...args);

function scheduleLexUpdate(source: string, delayMs = 120): void {
  clearTimeout(lexUpdateTimer);
  const sourceLength = String(source || "").length;
  let effectiveDelayMs = delayMs;
  if (sourceLength >= HUGE_SOURCE_DEBOUNCE_THRESHOLD_CHARS) {
    effectiveDelayMs = Math.max(effectiveDelayMs, LIVE_LEX_HUGE_DEBOUNCE_MS);
  } else if (sourceLength >= LIVE_LEX_HIGHLIGHT_MAX_CHARS) {
    effectiveDelayMs = Math.max(effectiveDelayMs, LIVE_LEX_LARGE_DEBOUNCE_MS);
  }
  // Keep displaying the last known token set while the next lex request is pending.
  // This avoids temporary highlight dropouts during worker rebuild/startup windows.
  lexCacheSource = source;
  const token = ++lexUpdateToken;
  lexUpdateTimer = setTimeout(async () => {
    try {
      const tokens = await requestWorkerQuery("lex", { source, filename: activeFile });
      if (token !== lexUpdateToken) {
        return;
      }
      lexCacheSource = source;
      lexCacheTokens = Array.isArray(tokens) ? tokens : [];
      sourceEditor?.setSyntaxHighlighter(provideFrescoSyntaxHighlights);
    } catch {
      if (token !== lexUpdateToken) {
        return;
      }
      // Preserve the previous token set on transient failures so highlighting
      // remains stable until lexing recovers.
    }
  }, effectiveDelayMs);
}

function registerFrescoAutocomplete() {
  provideFrescoCompletions = createWasmCompletionProvider({
    completionItemsFn: (source, cursorUtf8) =>
      requestWorkerQuery("completion", { source, cursorUtf8, filename: activeFile }) as any,
    utf16OffsetToUtf8Byte,
    fallbackItemsFn: () => {
      const toItem = (label: string, kind: string, detail: string, boost = 0, extra: Record<string, unknown> = {}) => ({
        label,
        kind,
        detail,
        boost,
        allowed: true,
        ...extra
      });

      const builtinReferenceByName = new Map<string, Record<string, unknown>>(
        (frescoLanguageConfig.builtinReference || [])
          .map((builtin) => {
            const name = String(builtin?.name || "").trim();
            return [name, builtin] as [string, Record<string, unknown>];
          })
          .filter(([name]) => name.length > 0)
      );

      const items = [];
      for (const keyword of frescoLanguageConfig.keywords) {
        items.push(toItem(keyword, "keyword", "Fresco keyword", 50));
      }
      for (const typeKeyword of frescoLanguageConfig.typeKeywords) {
        items.push(toItem(typeKeyword, "type", "Fresco type", 45));
      }
      for (const builtin of frescoLanguageConfig.builtins) {
        const builtinRef = builtinReferenceByName.get(builtin);
        const signature = builtinRef ? preferredBuiltinSignature(builtinRef) : "";
        const receiverKinds = builtinRef ? normalizeBuiltinReceiverKinds(builtinRef) : [];
        items.push(
          toItem(builtin, "function", "Builtin", 60, {
            signature,
            receiverKinds
          })
        );
      }
      for (const transform of frescoLanguageConfig.spaceTransforms) {
        items.push(toItem(transform, "function", "Space transform", 55));
      }
      for (const blendMode of frescoLanguageConfig.blendModes) {
        items.push(toItem(blendMode, "enum", "Blend mode", 40));
      }
      for (const enumMember of frescoLanguageConfig.enumMembers) {
        items.push(toItem(enumMember, "enum", "Enum member", 35));
      }
      for (const unit of frescoLanguageConfig.units) {
        items.push(toItem(unit, "keyword", "Unit", 20));
      }

      return items;
    }
  });

  sourceEditor?.setCompletionProvider(provideFrescoCompletions);
}

function registerFrescoLanguage() {
  provideFrescoSyntaxHighlights = editorHighlightingController.syntaxHighlighter as EditorSyntaxHighlighter;

  sourceEditor?.setSyntaxHighlighter(provideFrescoSyntaxHighlights);
}

function registerFrescoSemanticTokens() {
  // Monaco semantic token plumbing removed with the CM6 migration.
}

function applyLanguageProfile(profile: unknown) {
  if (!profile || typeof profile !== "object") {
    return;
  }
  const profileObj = profile as Record<string, unknown>;

  const asList = (value: unknown, fallback: string[]): string[] => {
    if (!Array.isArray(value)) {
      return [...fallback];
    }
    const next = value
      .map((item) => (typeof item === "string" ? item.trim() : ""))
      .filter(Boolean);
    return next.length > 0 ? next : [...fallback];
  };

  frescoLanguageConfig = {
    keywords: asList(profileObj.keywords, DEFAULT_FRESCO_KEYWORDS),
    typeKeywords: asList(profileObj.type_keywords, DEFAULT_FRESCO_TYPE_KEYWORDS),
    units: asList(profileObj.units, DEFAULT_FRESCO_UNITS),
    builtins: asList(profileObj.builtins, []),
    builtinReference: Array.isArray(profileObj.builtin_reference)
      ? profileObj.builtin_reference.filter((entry): entry is Record<string, unknown> => Boolean(entry && typeof entry === "object"))
      : [],
    spaceTransforms: asList(profileObj.space_transforms, []),
    blendModes: asList(profileObj.blend_modes, []),
    enumMembers: asList(profileObj.enum_members, []),
    stdlibExports: Array.isArray(profileObj.stdlib_exports)
      ? profileObj.stdlib_exports.filter((entry): entry is Record<string, unknown> => Boolean(entry && typeof entry === "object"))
      : []
  };

  builtinHelpController.setBuiltinReference(frescoLanguageConfig.builtinReference);
  builtinVizSweepMetaByName = buildBuiltinVizSweepMeta(frescoLanguageConfig.builtinReference);

  registerFrescoLanguage();
  registerFrescoAutocomplete();
  registerFrescoSemanticTokens();
}

registerFrescoLanguage();
registerFrescoAutocomplete();
registerFrescoSemanticTokens();

// Let the shell paint before loading Monaco/editor code.
await waitForNextPaintFrame();
const { createCodeMirrorEditor } = await loadEditorAdapterModule();

sourceEditor = createCodeMirrorEditor(document.getElementById("editor") as HTMLElement, {
  value: STARTER_SOURCE,
  fontSize: 14,
  provideCompletions: (model, position) => provideFrescoCompletions(model, position)
});
editorHighlightingController.installColorLiteralSwatchPicker(sourceEditor);
sourceEditor.setCompletionProvider(provideFrescoCompletions);
sourceEditor.setSyntaxHighlighter(provideFrescoSyntaxHighlights);
builtinHelpController.setupBuiltinHoverHelp();

wgslEditor = createCodeMirrorEditor(document.getElementById("wgsl") as HTMLElement, {
  value: "",
  readOnly: true,
  fontSize: 12
});

const meshPickerEl = document.getElementById("mesh-picker");
let surfacePreviewActive = false;

function syncSurfaceSpinButtonVisibility() {
  if (!previewSpinButtonEl) return;
  const show = surfacePreviewActive && !_surfaceControls.getOrbitAuto();
  previewSpinButtonEl.hidden = !show;
}

const _rustRenderer = new ExampleEngineRenderer(canvasEl, {
  previewResolutionEl, previewFpsEl, previewStatusEl,
  getSourceExampleId: () => sourceExampleId, textureOptionsForName,
});
const _surfaceControls = _rustRenderer;
const bufferControl = document.getElementById("preview-buffer-control")!;
const bufferSelect = document.getElementById("preview-buffer") as HTMLSelectElement;
const bufferLabels = document.createElement("div");
bufferLabels.className = "preview-buffer-labels";
bufferLabels.hidden = true;
canvasEl.after(bufferLabels);
_rustRenderer.onBufferViewsChanged = () => {
  const views = _rustRenderer.bufferViews;
  bufferControl.hidden = views.length === 0;
  bufferSelect.disabled = views.length === 0;
  bufferSelect.replaceChildren();
  const groups = new Map<string, HTMLOptGroupElement>();
  for (const view of views) {
    let group = groups.get(view.group);
    if (!group) {
      group = document.createElement("optgroup");
      group.label = view.group;
      groups.set(view.group, group);
      bufferSelect.append(group);
    }
    const option = new Option(view.label, view.id);
    option.title = view.description;
    group.append(option);
  }
  bufferSelect.value = _rustRenderer.bufferView;
  bufferSelect.title = views.find(view => view.id === bufferSelect.value)?.description ?? "";
  const tiles = views.find(view => view.id === bufferSelect.value)?.tiles || [];
  bufferLabels.replaceChildren(...tiles.map(title => {
    const label = document.createElement("span");
    label.textContent = title;
    return label;
  }));
  bufferLabels.hidden = tiles.length === 0;
};
bufferSelect.addEventListener("change", () => {
  const id = bufferSelect.value;
  bufferSelect.title = _rustRenderer.bufferViews.find(view => view.id === id)?.description ?? "";
  void _rustRenderer.setBufferView(id).catch(error => {
    bufferSelect.value = _rustRenderer.bufferView;
    _rustRenderer.reportRuntimeIssue(error, "buffer visualization");
  });
});
const lightingMenu = document.getElementById("preview-lighting-menu") as HTMLDetailsElement;
_rustRenderer.onLightingCapabilitiesChanged = () => {
  const supported = _rustRenderer.supportsLightingEnvironment;
  lightingMenu.hidden = !supported;
  lightingMenu.querySelector("fieldset")!.disabled = !supported;
  if (!supported) lightingMenu.open = false;
};
lightingMenu.addEventListener("change", (event) => {
  const input = event.target as HTMLInputElement;
  if (input.name !== "preview-lighting") return;
  const value = input.value;
  if (value !== "preview" && value !== "unlit" && value !== "directional" && value !== "three-lights") return;
  lightingMenu.open = false;
  lightingMenu.querySelector("summary")?.focus();
  void _rustRenderer.setLightingEnvironment(value).catch(error => {
    _rustRenderer.reportRuntimeIssue(error, "lighting environment");
  });
});
document.addEventListener("pointerdown", event => {
  if (!lightingMenu.contains(event.target as Node)) lightingMenu.open = false;
});
lightingMenu.addEventListener("keydown", event => {
  if (event.key === "Escape") {
    lightingMenu.open = false;
    lightingMenu.querySelector("summary")?.focus();
  }
});

_surfaceControls.onOrbitAutoChanged = () => {
  syncSurfaceSpinButtonVisibility();
};

/**
 * Connects the shared engine adapter to the playground controls.
 */
const renderer = new (class PreviewControls {
  get gpuReady() { return _rustRenderer.gpuReady; }
  get errorMessage() { return _rustRenderer.errorMessage; }

  get paramDefs()   { return _rustRenderer.paramDefs; }
  get textureDefs() { return _rustRenderer.textureDefs; }
  get paramValues() { return _rustRenderer.paramValues; }
  get stagedShader() { return _rustRenderer.stagedShader; }
  get simTimeSeconds() { return _rustRenderer.simTimeSeconds; }
  get elapsed() { return _rustRenderer.elapsed; }
  get fpsFrameCount() { return _rustRenderer.fpsFrameCount; }
  set fpsFrameCount(v: number) { _rustRenderer.fpsFrameCount = v; }
  get fpsSampleStartMs() { return _rustRenderer.fpsSampleStartMs; }
  set fpsSampleStartMs(v: number) { _rustRenderer.fpsSampleStartMs = v; }

  set onParamDefsChanged(fn: any) {
    _rustRenderer.onParamDefsChanged = fn;
  }
  set onTextureDefsChanged(fn: any) {
    _rustRenderer.onTextureDefsChanged = fn;
  }
  set onParamValueChanged(fn: any) {
    _rustRenderer.onParamValueChanged = fn;
  }
  set onRuntimeDiagnostic(fn: any) {
    _rustRenderer.onRuntimeDiagnostic = fn;
  }

  async init() { await _rustRenderer.init(); }
  handleResize() { _rustRenderer.handleResize(); }

  reportRuntimeIssue(message: string, context: string) {
    _rustRenderer.reportRuntimeIssue(message, context);
  }

  clearPreview() {
    _rustRenderer.clearPreview();
    setPreviewFreshness("ready");
  }

  cancelPendingShader() { _rustRenderer.cancelPendingShader(); }
  get hasInstalledPreview() { return _rustRenderer.hasInstalledPreview; }


  stageShader(frescoWgsl: string, manifest: ManifestRoot | null) {
    surfacePreviewActive = Boolean(manifest?.surfaces?.some(s => s.name !== "fresco_scene_ground"));
    _rustRenderer.stageShader(frescoWgsl, manifest);
    if (meshPickerEl) meshPickerEl.hidden = !surfacePreviewActive || !!manifest?.techniques?.some(t => t.surface === manifest.surfaces?.[0]?.name && t.metadata.engine === "particle");
    syncSurfaceSpinButtonVisibility();
  }

  async maybeBuildStagedShader() {
    return _rustRenderer.maybeBuildStagedShader();
  }

  getAsyncShaderBuildSnapshot() {
    return _rustRenderer.getAsyncShaderBuildSnapshot();
  }

  drawFrame(options?: any): boolean {
    return _rustRenderer.drawFrame(options);
  }

  setPlaybackPaused(paused: boolean) { _rustRenderer.setPlaybackPaused(paused); }
  setPlaybackRate(rate: number) {
    _rustRenderer.setPlaybackRate(rate);
  }
  setPlaybackLoopDuration(seconds: number) { _rustRenderer.setPlaybackLoopDuration(seconds); }
  setPlaybackTime(seconds: number) { _rustRenderer.setPlaybackTime(seconds); }
  stepPlaybackFrames(frames: number, fps: number) { _rustRenderer.stepPlaybackFrames(frames, fps); }

  setParamValue(def: any, value: any) { _rustRenderer.setParamValue(def, value); }
  setTextureSelection(name: string, url: string) { return _rustRenderer.setTextureSelection(name, url); }

  defaultValueForType(type: string, fallback?: unknown) {
    return _rustRenderer.defaultValueForType(type, fallback);
  }
  normalizeParamValue(def: any, value: unknown) {
    return _rustRenderer.normalizeParamValue(def, value);
  }

  syncPreviewModeButtons() {
    for (const button of meshPickerEl?.querySelectorAll<HTMLButtonElement>(".mesh-pick-btn") || []) {
      const active = (button.dataset.mesh === "cube" ? "box" : button.dataset.mesh) === (_surfaceControls.meshKind === "cube" ? "box" : _surfaceControls.meshKind);
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    }
  }

  /** Switch the 3-D preview mesh kind. */
  setSurfaceMeshKind(kind: SurfaceMeshKind) {
    _surfaceControls.setMeshKind(kind);
    this.syncPreviewModeButtons();
  }
  setSurfaceOrbitAuto(enabled: boolean) {
    _surfaceControls.setOrbitAuto(enabled);
    syncSurfaceSpinButtonVisibility();
  }
  setActiveSurfaceName(name: string | null | undefined) {
    _surfaceControls.setActiveSurfaceName(name);
  }
})();

_rustRenderer.onMeshKindChanged = () => renderer.syncPreviewModeButtons();
_rustRenderer.onPlaybackCapabilitiesChanged = () => {
  const particles = _rustRenderer.renderMode === "particles";
  if (previewRateSelectEl) {
    for (const option of previewRateSelectEl.options) option.disabled = particles && Number(option.value) < 0;
    previewRateSelectEl.value = String(_rustRenderer.playbackRate);
  }
  if (previewStepBackButtonEl) {
    previewStepBackButtonEl.disabled = particles;
    previewStepBackButtonEl.title = particles
      ? "Particle simulation supports forward steps and reset to time 0"
      : "Step back one frame";
  }
};

function resolveAnnotationSweep(annotation: unknown) {
  const env = buildNumericEnvFromParams(renderer?.paramValues);
  return resolveAnnotationSweepValue(annotation, env, builtinVizSweepMetaByName);
}

function resolveAnnotationSweepMax(annotation: unknown) {
  const env = buildNumericEnvFromParams(renderer?.paramValues);
  return resolveAnnotationSweepMaxValue(annotation, env, builtinVizSweepMetaByName);
}

const previewControls = createPreviewControls({
  previewPanelEl,
  previewPlayButtonEl,
  previewFullscreenButtonEl,
  previewStepBackButtonEl,
  previewStepForwardButtonEl,
  previewTimeInputEl,
  previewFpsEl,
  renderer,
  getPreviewPaused: () => previewPaused,
  setPreviewPausedState: (nextPaused) => {
    previewPaused = Boolean(nextPaused);
  }
});

// ── Mesh picker ─────────────────────────────────────────────────────────────
if (meshPickerEl) {
  meshPickerEl.addEventListener("click", (event) => {
    const btn = (event.target as HTMLElement).closest<HTMLButtonElement>(".mesh-pick-btn");
    if (!btn) return;
    const kind = btn.dataset.mesh as SurfaceMeshKind | undefined;
    if (!kind) return;
    renderer.setSurfaceMeshKind(kind);
  });
}


function utf16OffsetToUtf8Byte(text: string, utf16Offset: number) {
  const raw = Number(utf16Offset);
  const finite = Number.isFinite(raw) ? Math.floor(raw) : 0;
  const clamped = Math.max(0, Math.min(finite, text.length));
  let u16 = 0;
  let u8 = 0;
  for (const ch of text) {
    if (u16 >= clamped) break;
    u8 += utf8LengthForCodePoint(ch.codePointAt(0) ?? 0);
    u16 += ch.length;
  }
  return u8;
}

// Global viz manager instance
let vizManager: VizZoneManager | null = null;

// Old workbook hover/pin system removed - replaced by @viz inline annotations


function normalizeSeverity(rawSeverity: unknown) {
  return normalizeSeverityUi(rawSeverity, WARNING_SEVERITY_ALIASES, INFO_SEVERITY_ALIASES);
}

function setPreviewFreshness(state: "busy" | "error" | "ready") {
  const indicator = document.getElementById("preview-freshness")!;
  indicator.hidden = state === "ready";
  indicator.dataset.state = state;
  indicator.textContent = state === "busy" ? "Updating preview"
    : state === "error" ? (renderer.hasInstalledPreview ? "Preview out of date" : "Preview unavailable") : "";
}

function setStatus(text: string, klass = "") {
  if (klass === "compiling") setPreviewFreshness("busy");
  else if (klass === "error") setPreviewFreshness("error");
  statusEl.textContent = text;
  statusEl.classList.remove("success", "warning", "error", "compiling");
  if (klass) {
    statusEl.classList.add(klass);
  }
}

function hashStringFNV1a(input: string) {
  let hash = 0x811c9dc5;
  for (let i = 0; i < input.length; i += 1) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

function isPreviewPanelActive() {
  if (!previewPanelEl || document.visibilityState === "hidden") {
    return false;
  }
  const rect = previewPanelEl.getBoundingClientRect();
  return (
    rect.width > 0
    && rect.height > 0
    && rect.bottom > 0
    && rect.top < window.innerHeight
    && rect.right > 0
    && rect.left < window.innerWidth
  );
}

function mountParamsContent(hostEl: HTMLElement | null) {
  if (!hostEl || shaderParamsEl.parentElement === hostEl) {
    return;
  }
  hostEl.appendChild(shaderParamsEl);
}

function hasActionableDiagnostics(diags: unknown[]): boolean {
  return (Array.isArray(diags) ? diags : []).some((diag: any) => {
    const severity = normalizeSeverity(diag?.severity);
    return severity === "warning" || severity === "error";
  });
}

function updateDiagnosticsTabVisibility(diags: unknown[]): void {
  const showDiagnosticsTab = hasActionableDiagnostics(diags);
  diagnosticsTabButtonEl?.toggleAttribute("hidden", !showDiagnosticsTab);
  if (!showDiagnosticsTab && activeOutputTab === "diagnostics") {
    setOutputTab("wgsl");
  }
}

function setOutputTab(tabName: string) {
  const tabs = Array.from(document.querySelectorAll<HTMLElement>(".outputs-panel .tab"));
  const visibleTabs = tabs.filter((tab) => !tab.hasAttribute("hidden"));
  const fallbackTab = visibleTabs[0]?.dataset.tab || "wgsl";
  const resolvedTab = visibleTabs.some((tab) => tab.dataset.tab === tabName)
    ? tabName
    : fallbackTab;
  activeOutputTab = resolvedTab;
  const contents = {
    diagnostics: diagnosticsEl,
    explain: explainEl,
    wgsl: document.getElementById("wgsl"),
    manifest: manifestEl,
    inspector: manifestInspectorEl,
    params: paramsTabEl
  };

  tabs.forEach((tab) => {
    tab.classList.toggle("active", tab.dataset.tab === resolvedTab);
  });

  if (outputsPanelTitleEl) {
    const activeTab = tabs.find((tab) => tab.dataset.tab === resolvedTab);
    const activeLabel = String(activeTab?.textContent || "").trim();
    if (activeLabel) {
      outputsPanelTitleEl.textContent = activeLabel;
    }
  }

  Object.entries(contents).forEach(([name, content]) => {
    if (!content) {
      return;
    }
    content.classList.toggle("active", name === resolvedTab);
    content.setAttribute("aria-hidden", String(name !== resolvedTab));
  });
}

function updateParamsLayoutMode(paramDefs: unknown[], textureDefs: unknown[]) {
  const rowCount = (Array.isArray(paramDefs) ? paramDefs.length : 0)
    + (Array.isArray(textureDefs) ? textureDefs.length : 0)
    + (staticOptionsEl.hidden ? 0 : 1)
    + (implementationControls.hidden ? 0 : 1);

  if (rowCount === 0) {
    paramsLayoutMode = "hidden";
    paramsPanelEl?.classList.remove("active");
    paramsPanelEl?.setAttribute("aria-hidden", "true");
    paramsTabButtonEl?.setAttribute("hidden", "");
    paramsTabEl?.classList.remove("active");
    paramsTabEl?.setAttribute("aria-hidden", "true");
    workspaceEl?.classList.remove("has-params", "params-tabbed");
    if (activeOutputTab === "params") {
      setOutputTab("wgsl");
    }
    return;
  }

  const nextMode = window.matchMedia("(max-width: 760px)").matches ? "tab" : "panel";
  const desiredHost = nextMode === "tab" ? paramsTabEl : paramsPanelBodyEl;
  if (paramsLayoutMode === nextMode && shaderParamsEl.parentElement === desiredHost) {
    return;
  }

  paramsLayoutMode = nextMode;
  paramsTabButtonEl?.toggleAttribute("hidden", nextMode !== "tab");

  if (nextMode === "tab") {
    paramsPanelEl?.classList.remove("active");
    paramsPanelEl?.setAttribute("aria-hidden", "true");
    paramsTabEl?.classList.add("active");
    paramsTabEl?.setAttribute("aria-hidden", "false");
    workspaceEl?.classList.remove("has-params");
    workspaceEl?.classList.add("params-tabbed");
    mountParamsContent(paramsTabEl);
    setOutputTab("params");
    return;
  }

  paramsTabEl?.classList.remove("active");
  paramsTabEl?.setAttribute("aria-hidden", "true");
  workspaceEl?.classList.add("has-params");
  workspaceEl?.classList.remove("params-tabbed");
  paramsPanelEl?.classList.add("active");
  paramsPanelEl?.setAttribute("aria-hidden", "false");
  mountParamsContent(paramsPanelBodyEl);
  if (activeOutputTab === "params") {
    setOutputTab("wgsl");
  }
}

function scheduleParamsLayoutRefresh() {
  if (paramsLayoutRaf) {
    cancelAnimationFrame(paramsLayoutRaf);
  }
  paramsLayoutRaf = requestAnimationFrame(() => {
    paramsLayoutRaf = 0;
    updateParamsLayoutMode(renderer.paramDefs, renderer.textureDefs);
  });
}

function renderShaderParams(paramDefs: unknown[], textureDefs: unknown[] = []) {
  renderShaderParamsUi({
    shaderParamsEl,
    renderer,
    paramDefs,
    textureDefs,
    updateParamsLayoutMode
  });
  shaderParamsEl.prepend(implementationControls, staticOptionsEl);
}

function parseManifestPanelJson() {
  try {
    const raw = String(manifestEl?.textContent || "").trim();
    if (!raw) return null;
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function refreshManifestInspector(manifest: ManifestRoot | null) {
  const propertySource = virtualFiles.get("main.fr") || "";
  const runtimeFilePaths = getRuntimeContentFilePathsForExample(sourceExampleId || "");
  const inspectorState = buildManifestInspectorState(
    manifest,
    selectedInspectorSurfaceName || null,
    runtimeFilePaths,
  );
  const hasInspectorOptions = Boolean(inspectorState);
  inspectorTabButtonEl?.toggleAttribute("hidden", !hasInspectorOptions);
  if (!hasInspectorOptions && activeOutputTab === "inspector") {
    setOutputTab("wgsl");
  }

  renderManifestInspector({
    hostEl: manifestInspectorEl,
    manifest,
    selectedSurfaceName: selectedInspectorSurfaceName || null,
    runtimeFilePaths,
    onSelectSurface: (surfaceName) => {
      selectedInspectorSurfaceName = String(surfaceName || "").trim();
      renderer.setActiveSurfaceName(selectedInspectorSurfaceName || null);
      const currentManifest = parseManifestPanelJson();
      const wgsl = String(wgslEditor?.getValue?.() || "").trim();
      if (!currentManifest || !wgsl) {
        return;
      }
      renderer.stageShader(wgsl, currentManifest);
      void renderer.maybeBuildStagedShader().catch((err) => {
        renderer.reportRuntimeIssue(err?.message || err, "surface selection rebuild");
      });
    },
    onOpenRuntimeFile: (filePath) => {
      virtualFilesController.syncActiveEditorToMap();
      const files = virtualFilesController.getVirtualFiles();
      if (!files.has(filePath)) {
        const source = buildCompileFilesBundle(files, sourceExampleId || "").get(filePath);
        if (source === undefined) return;
        files.set(filePath, source);
      }
      switchToFile(filePath);
    },
  });
  const properties = [...new Map([...(manifest?.surfaces?.flatMap(surface => surface.settings?.properties || []) || []), ...(manifest?.gpu_programs?.flatMap(program => program.properties) || [])]
    .map(property => [`${property.entry}:${property.name}`, property])).values()];
  renderEngineProperties(manifestInspectorEl, properties, (property, expression) => {
    if (virtualFiles.get("main.fr") !== propertySource) throw new Error("The source changed; wait for compilation before editing properties");
    const next = editEngineProperty(propertySource, property, expression);
    if (activeFile !== "main.fr") switchToFile("main.fr");
    setEditorSource(next);
    onSourceChanged();
  });
}

function showDiagnostics(diags: unknown[], model: unknown) {
  updateDiagnosticsTabVisibility(diags);
  showDiagnosticsUi({
    diags,
    model,
    diagnosticsEl,
    virtualFiles,
    sourceEditor,
    activeFile,
    switchToFile,
    virtualDiagnosticFiles: VIRTUAL_DIAGNOSTIC_FILES,
    normalizeSeverityFn: normalizeSeverity
  });
}

function mapDiagnosticsToMarkers(diags: unknown[], model: unknown) {
  return mapDiagnosticsToMarkersUi({
    diags,
    model,
    virtualFiles,
    activeFile,
    virtualDiagnosticFiles: VIRTUAL_DIAGNOSTIC_FILES,
    normalizeSeverityFn: normalizeSeverity
  });
}

function setupTabs() {
  const tabs = Array.from(document.querySelectorAll<HTMLElement>(".tab"));

  diagnosticsTabButtonEl?.setAttribute("hidden", "");
  inspectorTabButtonEl?.setAttribute("hidden", "");

  tabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      setOutputTab(tab.dataset.tab || "wgsl");
    });
  });

  setOutputTab(activeOutputTab);
}

// ── Multi-file / virtual file system UI ────────────────────────────────────

function renderFileTabs() {
  virtualFilesController.renderFileTabs();
  syncVirtualFileRefs();
}

function switchToFile(filename: string) {
  virtualFilesController.switchToFile(filename);
  syncVirtualFileRefs();
}

function deleteLibraryFile(filename: string) {
  virtualFilesController.deleteLibraryFile(filename);
  syncVirtualFileRefs();
}

function resetVirtualFiles(mainSource = "") {
  virtualFilesController.resetVirtualFiles(mainSource);
  syncVirtualFileRefs();
}

function loadVirtualBundle(bundle: Map<string, string>) {
  virtualFilesController.loadVirtualBundle(bundle);
  syncVirtualFileRefs();
}

function isSourceBlank(source: string) {
  return isSourceBlankValue(source);
}

function computeCompileDebounceMs(source: string) {
  return computeCompileDebounceMsValue(source, {
    hugeThreshold: HUGE_SOURCE_DEBOUNCE_THRESHOLD_CHARS,
    hugeDelay: HUGE_SOURCE_DEBOUNCE_MS,
    largeThreshold: LARGE_SOURCE_DEBOUNCE_THRESHOLD_CHARS,
    largeDelay: LARGE_SOURCE_DEBOUNCE_MS,
    defaultDelay: COMPILE_DEBOUNCE_MS,
  });
}

async function encodeSharedSource(source: string) {
  return encodeSharedSourceValue({
    source,
    requestWorkerQuery,
    workerQueryTimeoutMs: WORKER_QUERY_TIMEOUT_MS,
    urlCodePrefixRaw: URL_CODE_PREFIX_RAW,
  });
}

function encodeRawSource(source: string) {
  return encodeRawSourceValue(source, URL_CODE_PREFIX_RAW);
}

async function decodeSharedSource(payload: string) {
  return decodeSharedSourceValue({
    payload,
    urlCodePrefixGzip: URL_CODE_PREFIX_GZIP,
    urlCodePrefixRaw: URL_CODE_PREFIX_RAW,
    timeoutMs: 150,
  });
}

function setSourceMode(mode: string, options: SourceModeOptions = {}): void {
  sourceStateController.setSourceMode(mode, options);
  syncSourceStateRefs();
}

function setEditorSource(text: string): void {
  suppressSourceEvents = true;
  const next = String(text ?? "");
  sourceEditor!.setValue(next);
  virtualFilesController.setActiveFileContent(next);
  syncVirtualFileRefs();
  suppressSourceEvents = false;

  // Programmatic source swaps (example dropdown, URL load) should repaint
  // highlights immediately using fallback classification instead of waiting
  // for async worker lex responses.
  lexCacheSource = next;
  lexCacheTokens = [];
  sourceEditor!.setSyntaxHighlighter(provideFrescoSyntaxHighlights);

  scheduleLexUpdate(next, 0);
}

function resolveExampleFromUrlParam(paramValue: string | null) {
  return resolveExampleFromUrlParamValue(paramValue, EXAMPLES_BY_ID);
}

async function applySourceFromUrl() {
  await sourceStateController.applySourceFromUrl();
  syncSourceStateRefs();
}

function scheduleUrlSync(): void {
  sourceStateController.scheduleUrlSync();
}

const exampleSelectionController = createExampleSelectionController({
  exampleSelectEl: exampleSelectEl as HTMLSelectElement,
  filteredExamples: FILTERED_EXAMPLES,
  examplesById: EXAMPLES_BY_ID,
  customExampleValue: CUSTOM_EXAMPLE_VALUE,
  getSourceMode: () => sourceMode,
  setSourceMode,
  resetVirtualFiles,
  loadVirtualBundle,
  buildRuntimeBundleForExample: (exampleId, baseBundle) => buildCompileFilesBundle(baseBundle, exampleId),
  bundleForExampleId,
  setEditorSource,
  getMainSource: () => virtualFilesController.getMainSource(),
  scheduleUrlSync,
  queueCompile,
  setStatus,
  compilingPreviewStatus,
  waitingForShaderStatus,
  rendererClearPreview: () => {
    if (exampleSelectEl.value !== CUSTOM_EXAMPLE_VALUE) implementationDocumentId++;
    renderer.clearPreview();
  },
  resetPreviewTime: () => renderer.setPlaybackTime(0),
  sourceEditor,
  wgslEditor,
  showDiagnostics,
  renderShaderParams,
  explainEl,
  manifestEl,
  diagnosticsEl,
  previewStatusEl,
});

const compileOrchestrator = createCompileOrchestrator({
  state: compileUiState,
  sourceEditor,
  wgslEditor,
  explainEl,
  manifestEl,
  previewStatusEl,
  renderer,
  isSourceBlank,
  showDiagnostics,
  renderShaderParams,
  mapDiagnosticsToMarkers,
  normalizeSeverity,
  setStatus,
  compileInBackground,
  isRetryableWorkerFailure,
  scheduleCompilerRecovery,
  workerAutoRecoveryDelayMs: WORKER_AUTO_RECOVERY_DELAY_MS,
  workerAutoRecoveryMaxAttempts: WORKER_AUTO_RECOVERY_MAX_ATTEMPTS,
  appStartupOriginMs: APP_STARTUP_ORIGIN_MS,
  getVizManager: () => vizManager,
  getPreviewPaused: () => previewPaused,
  isPreviewPanelActive,
  onManifestUpdated: (manifest) => {
    if (manifest) {
      updateRendererCatalog(manifest);
      implementationSelection.render((manifest.surfaces || []).filter(s => s.name !== "fresco_scene_ground"));
      shaderParamsEl.prepend(implementationControls, staticOptionsEl);
      scheduleParamsLayoutRefresh();
    }
    if (!manifest || !Array.isArray(manifest?.surfaces) || manifest.surfaces.length === 0) {
      selectedInspectorSurfaceName = "";
      refreshManifestInspector(null);
      return;
    }

    const available = manifest.surfaces.filter(s => s.name !== "fresco_scene_ground").map((surface: any) => String(surface?.name || "").trim());
    if (!available.includes(selectedInspectorSurfaceName)) {
      selectedInspectorSurfaceName = chooseManifestPreviewSurface(manifest)?.name || "";
    }
    renderer.setActiveSurfaceName(selectedInspectorSurfaceName || null);
    refreshManifestInspector(manifest);
  },
});

function onSourceChanged() {
  if (suppressSourceEvents) {
    return;
  }

  const source = sourceEditor!.getValue();
  virtualFilesController.setActiveFileContent(source);
  syncVirtualFileRefs();
  scheduleLexUpdate(source);
  sourceStateController.onSourceChanged(virtualFilesController.getMainSource());
  syncSourceStateRefs();
}

async function compileAndRender(options: Partial<CompileQueuePayload> = {}) {
  try {
    await compileOrchestrator.compileAndRender({
      ...options,
      waitForPreviewBuild: document.body.classList.contains("app-loading"),
    });
  } finally {
    revealPlayground();
  }
}

function revealPlayground() {
  document.body.classList.remove("app-loading");
  document.body.removeAttribute("aria-busy");
  document.getElementById("app-startup")!.hidden = true;
}

function queueCompile({ delayMs = Number.NaN, cancelInFlight = false, deferPreviewBuild = false }: CompileQueueRequest = {}): void {
  compileQueueController.queueCompile({
    delayMs,
    cancelInFlight,
    deferPreviewBuild,
  });
}

const bootstrapWatchdog = createBootstrapCompileWatchdog({
  timeoutMs: BOOTSTRAP_COMPILE_WATCHDOG_MS,
  onTimeout: (elapsedMs) => {
    revealPlayground();
    const message = `startup exceeded ${BOOTSTRAP_COMPILE_WATCHDOG_MS}ms before first compile kickoff (${elapsedMs}ms elapsed)`;
    console.warn(`[fresco] ${message}`);
    setStatus("Startup Delayed", "warning");
    previewStatusEl.textContent = "Compile startup delayed";
    showDiagnostics([
      {
        file: "bootstrap",
        span_start: 0,
        span_end: 0,
        message,
        severity: "warning",
        label: "startup watchdog",
        help: "The editor is loaded, but compile wiring did not become ready in time. Check worker logs and recover compiler."
      }
    ], sourceEditor.getModel());
  }
});

function showPreviewUnavailable(reason: string) {
  const notice = document.getElementById("preview-unavailable")!;
  const title = document.getElementById("preview-unavailable-title")!;
  const help = document.getElementById("preview-unavailable-help")!;
  if (!window.isSecureContext) {
    title.textContent = "The preview needs a secure connection";
    help.textContent = "Open this page using HTTPS to enable WebGPU, which powers the live preview.";
  } else if (!navigator.gpu) {
    title.textContent = "This browser can't show the preview";
    help.textContent = "The live preview needs WebGPU. Try opening this page in an up-to-date Chrome or Edge browser on a supported device.";
  } else {
    title.textContent = "Couldn't start the graphics preview";
    help.textContent = "WebGPU couldn't access your graphics device. Try updating your browser and device software, then reload this page. If it still fails, try another browser or device.";
  }
  document.getElementById("preview-unavailable-detail")!.textContent = reason;
  notice.hidden = false;
  document.getElementById("preview-panel")!.classList.add("is-unavailable");
  previewStatusEl.textContent = previewUnavailableStatus();
}

async function bootstrap() {
  bootstrapWatchdog.arm();
  exampleSelectionController.populateExamples();
  exampleBrowser.populate();
  await applySourceFromUrl();
  // Do not block startup on metadata queries: compile wiring should be ready
  // even when profile/version requests are slow or transiently unavailable.
  void requestWorkerQuery(
    "language_profile",
    {},
    WORKER_BOOTSTRAP_QUERY_TIMEOUT_MS
  )
    .then((profile) => {
      applyLanguageProfile(profile);
    })
    .catch((err) => {
      const message = String(err?.message || err || "");
      if (!message.includes("worker query timed out: language_profile")) {
        console.error("[fresco] Failed to apply runtime language profile", err);
      }
    });
  let previewInitError = "";
  try {
    await renderer.init();
  } catch (err) {
    previewInitError = err instanceof Error ? err.message : String(err);
    console.warn("[fresco] Preview initialization failed", err);
  }
  rendererSelect.disabled = rendererSelect.options.length === 0;
  if (!renderer.gpuReady) {
    showPreviewUnavailable(previewInitError || renderer.errorMessage || "WebGPU unavailable");
  }
  configureVizZoneManagerDeps({
    requestWorkerQuery,
    writeVizRuntimeParamUniforms,
    resolveAnnotationSweep,
    resolveAnnotationSweepMax,
    buildShaderExplainText,
    renderer,
    sourceEditor,
    switchToFile,
  });
  let visualizerDevice: GPUDevice | undefined;
  let visualizerFormat: GPUTextureFormat | undefined;
  try {
    const resources = await createVisualizerDevice(navigator.gpu, message => {
      console.warn("[fresco] Editor visualizer:", message);
    });
    visualizerDevice = resources.device;
    visualizerFormat = resources.format;
    window.addEventListener("pagehide", event => {
      if (!event.persisted) resources.dispose();
    });
  } catch (error) {
    console.warn("[fresco] Editor visualizers unavailable", error);
  }

  vizManager = new VizZoneManager(sourceEditor, visualizerDevice, visualizerFormat);
  void vizManager.initialize().catch((err) => {
    console.warn("[fresco] Viz manager initialization failed", err);
  });
  renderer.onParamDefsChanged = () => {
    renderShaderParams(renderer.paramDefs, renderer.textureDefs);
  };
  renderer.onTextureDefsChanged = () => {
    renderShaderParams(renderer.paramDefs, renderer.textureDefs);
  };
  renderer.onParamValueChanged = () => {
    if (vizManager) {
      vizManager.scheduleRefresh();
    }
  };
  _rustRenderer.onPreviewInstalled = () => setPreviewFreshness("ready");
  _rustRenderer.onRuntimeRecovered = () => {
    showDiagnostics([], sourceEditor!.getModel());
    setStatus("Preview Ready", "success");
  };
  renderer.onRuntimeDiagnostic = (diag: unknown) => {
    const retained = renderer.hasInstalledPreview;
    if (!retained) {
      renderer.clearPreview();
      renderShaderParams([], []);
    }
    showDiagnostics([diag], sourceEditor!.getModel());
    setStatus("Preview Error", "error");
    previewStatusEl.textContent = retained ? "Preview error; showing previous preview" : noValidShaderStatus();
  };
  void requestWorkerQuery(
    "wasm_version",
    {},
    WORKER_BOOTSTRAP_QUERY_TIMEOUT_MS
  )
    .then((info: any) => {
      versionEl.textContent = `Fresco v${info.version} (${info.buildMode} build)`;
    })
    .catch((err) => {
      const message = String(err?.message || err || "");
      if (message && !message.includes("worker query timed out: wasm_version")) {
        console.warn("[fresco] Failed to fetch wasm version", err);
      }
      versionEl.textContent = "Fresco (version unavailable)";
    });

  rendererSelect.disabled = rendererSelect.options.length === 0;
  if (!renderer.gpuReady) {
    showDiagnostics([
      {
        file: "preview",
        span_start: 0,
        span_end: 0,
        message: previewInitError || renderer.errorMessage || "WebGPU unavailable",
        severity: "warning",
        label: "preview renderer",
        help: "Use a browser with WebGPU support (Chrome/Edge stable)"
      }
    ], sourceEditor!.getModel());
    previewStatusEl.textContent = previewUnavailableStatus();
  }

  setupTabs();
  renderFileTabs();
  scheduleParamsLayoutRefresh();
  previewControls.syncPreviewPlayButtonState();
  previewControls.syncPreviewTimeReadout();
  previewControls.syncPreviewFullscreenButtonState();
  bindBootstrapEventHandlers({
    scheduleParamsLayoutRefresh,
    exampleSelectEl,
    onExampleSelected: exampleSelectionController.onExampleSelected,
    docsToggleEl,
    toggleDocsOverlay: () => docsOverlayController.toggle(),
    previewControls,
    previewPlayButtonEl,
    previewSpinButtonEl,
    previewRateSelectEl,
    previewStepBackButtonEl,
    previewStepForwardButtonEl,
    previewTimeInputEl,
    previewFullscreenButtonEl,
    renderer,
    getPreviewPaused: () => previewPaused,
    sourceEditor: sourceEditor!,
    onSourceChanged,
    queueCompile,
  });

  bootstrapWatchdog.clear();
  queueCompile({ delayMs: 0 });
  startPreviewFrameLoop({
    renderer,
    previewControls,
    getPreviewPaused: () => previewPaused,
    isPreviewPanelActive,
    getPreviewBuildDeferred: () => compileUiState.previewBuildDeferred,
    previewFpsEl,
  });
}

bootstrap().catch((err) => {
  revealPlayground();
  bootstrapWatchdog.clear();
  setStatus("Init Error", "error");
  showDiagnostics([
    {
      file: "bootstrap",
      span_start: 0,
      span_end: 0,
      message: String(err),
      severity: "error",
      label: "startup",
      help: "Check browser console for details"
    }
  ], sourceEditor.getModel());
});
