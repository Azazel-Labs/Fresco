import type { ExampleEntry, SourceModeOptions } from "./controller-types";

interface CreateSourceStateControllerOptions {
  exampleSelectEl: HTMLSelectElement;
  getSource: () => string;
  setEditorSource: (text: string) => void;
  isSourceBlank: (source: string) => boolean;
  encodeSharedSource: (source: string) => Promise<string>;
  encodeRawSource: (source: string) => string;
  decodeSharedSource: (payload: string) => Promise<string | null>;
  resolveExampleFromUrlParam: (param: string | null) => ExampleEntry | null;
  examplesById: Map<string, ExampleEntry>;
  customExampleValue: string;
  maxShareParamLength: number;
  urlParamExample: string;
  urlParamCode: string;
  starterSource: string;
}

interface SourceStateController {
  setSourceMode: (mode: string, options?: SourceModeOptions) => void;
  applySourceFromUrl: () => Promise<void>;
  scheduleUrlSync: () => void;
  onSourceChanged: (source: string) => void;
  getSourceMode: () => string;
  getSourceBaseline: () => string;
  getSourceExampleId: () => string;
}

export function createSourceStateController({
  exampleSelectEl,
  getSource,
  setEditorSource,
  isSourceBlank,
  encodeSharedSource,
  encodeRawSource,
  decodeSharedSource,
  resolveExampleFromUrlParam,
  examplesById,
  customExampleValue,
  maxShareParamLength,
  urlParamExample,
  urlParamCode,
  starterSource,
}: CreateSourceStateControllerOptions): SourceStateController {
  let sourceMode = "blank";
  let sourceBaseline = String(starterSource ?? "");
  let sourceExampleId = "";
  let urlSyncToken = 0;
  let urlSyncHandle = 0;

  function setSourceMode(mode: string, options: SourceModeOptions = {}) {
    sourceMode = mode;
    sourceBaseline = options.baseline || "";
    sourceExampleId = options.exampleId || "";

    if (mode === "example" && sourceExampleId) {
      exampleSelectEl.value = sourceExampleId;
    } else if (mode === "custom") {
      exampleSelectEl.value = customExampleValue;
    } else {
      exampleSelectEl.value = "";
    }
  }

  async function applySourceFromUrl() {
    const params = new URLSearchParams(window.location.search);
    const payload = params.get(urlParamCode);
    if (payload) {
      const decoded = await decodeSharedSource(payload);
      if (typeof decoded === "string") {
        setSourceMode("custom");
        setEditorSource(decoded);
        return;
      }
    }

    const example = resolveExampleFromUrlParam(params.get(urlParamExample));
    if (example) {
      setSourceMode("example", {
        exampleId: example.id,
        baseline: example.source
      });
      setEditorSource(example.source);
      return;
    }

    setSourceMode("blank");
    setEditorSource(starterSource);
  }

  async function syncUrlStateNow() {
    const token = ++urlSyncToken;
    const params = new URLSearchParams(window.location.search);
    params.delete(urlParamExample);
    params.delete(urlParamCode);
    // Strip the retired completion preference from previously shared URLs.
    params.delete("completion");

    const source = getSource();
    const selectedExampleId = exampleSelectEl.value;
    const selectedExample = examplesById.get(selectedExampleId);
    if (
      selectedExampleId &&
      selectedExampleId !== customExampleValue &&
      selectedExample &&
      source === selectedExample.source
    ) {
      params.set(urlParamExample, selectedExampleId);
    } else if (!isSourceBlank(source)) {
      let payload = "";
      try {
        payload = await encodeSharedSource(source);
      } catch {
        payload = encodeRawSource(source);
      }
      if (token !== urlSyncToken) {
        return;
      }
      if (!payload) {
        payload = encodeRawSource(source);
      }
      if (payload.length > maxShareParamLength) {
        payload = encodeRawSource(source);
      }
      if (payload.length <= maxShareParamLength) {
        params.set(urlParamCode, payload);
      }
    }

    // Encoding may await compression. Merge only source-owned parameters into
    // the current URL so a concurrent renderer/surface choice is preserved.
    const current = new URLSearchParams(window.location.search);
    for (const key of [urlParamExample, urlParamCode, "completion"]) current.delete(key);
    for (const key of [urlParamExample, urlParamCode]) {
      const value = params.get(key);
      if (value !== null) current.set(key, value);
    }
    const query = current.toString();
    const nextUrl = `${window.location.pathname}${query ? `?${query}` : ""}${window.location.hash}`;
    try {
      window.history.replaceState(null, "", nextUrl);
    } catch {
      // Ignore URL update failures for extremely long custom payloads.
    }
  }

  function scheduleUrlSync() {
    clearTimeout(urlSyncHandle);
    urlSyncHandle = setTimeout(() => {
      void syncUrlStateNow();
    }, 700);
  }

  function onSourceChanged(source: string) {
    if (sourceMode === "example" && source !== sourceBaseline) {
      setSourceMode("custom");
    } else if (sourceMode === "blank" && !isSourceBlank(source)) {
      setSourceMode("custom");
    } else if (sourceMode === "custom" && isSourceBlank(source)) {
      setSourceMode("blank");
    }

    scheduleUrlSync();
  }

  return {
    setSourceMode,
    applySourceFromUrl,
    scheduleUrlSync,
    onSourceChanged,
    getSourceMode: () => sourceMode,
    getSourceBaseline: () => sourceBaseline,
    getSourceExampleId: () => sourceExampleId,
  };
}
