import type { CompileQueueRequest, VirtualFilesMap } from "./controller-types";

interface CreateVirtualFilesControllerOptions {
  fileTabsEl: HTMLElement | null;
  fileTabListEl: HTMLElement | null;
  fileTabPickerEl: HTMLSelectElement | null;
  getEditorValue: () => string;
  setEditorSource: (text: string) => void;
  queueCompile?: (request?: CompileQueueRequest) => void;
}

interface VirtualFilesController {
  renderFileTabs: () => void;
  switchToFile: (filename: string) => void;
  deleteLibraryFile: (filename: string) => void;
  resetVirtualFiles: (mainSource?: string) => void;
  loadVirtualBundle: (bundle: Iterable<[string, string]>) => void;
  syncActiveEditorToMap: () => void;
  setActiveFileContent: (text: string) => void;
  getActiveFile: () => string;
  getVirtualFiles: () => VirtualFilesMap;
  getMainSource: () => string;
  snapshotFilesOrNull: () => VirtualFilesMap | null;
}

export function createVirtualFilesController({
  fileTabsEl,
  fileTabListEl,
  fileTabPickerEl,
  getEditorValue,
  setEditorSource,
  queueCompile,
}: CreateVirtualFilesControllerOptions): VirtualFilesController {
  let virtualFiles = new Map([["main.fr", ""]]);
  let activeFile = "main.fr";
  let visibleFiles = new Set(["main.fr"]);

  function tabOrder(): string[] {
    const out: string[] = [];
    for (const [filename] of virtualFiles) {
      if (filename === "main.fr" || filename === activeFile || visibleFiles.has(filename)) {
        out.push(filename);
      }
    }
    return out;
  }

  function renderFileTabs() {
    if (!fileTabListEl || !fileTabsEl) return;
    fileTabListEl.textContent = "";
    const tabs = tabOrder();
    const isMultiFile = tabs.length > 1;
    fileTabsEl.classList.toggle("visible", isMultiFile);
    if (fileTabPickerEl) {
      fileTabPickerEl.textContent = "";
      fileTabPickerEl.hidden = true;
    }
    if (!isMultiFile) return;

    for (const filename of tabs) {
      const tab = document.createElement("div");
      tab.className = "file-tab" + (filename === activeFile ? " active" : "");
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-selected", String(filename === activeFile));
      tab.title = filename;

      const nameEl = document.createElement("span");
      nameEl.className = "file-tab-name";
      nameEl.textContent = filename;
      tab.appendChild(nameEl);

      if (filename !== "main.fr") {
        const closeBtn = document.createElement("button");
        closeBtn.className = "file-tab-close";
        closeBtn.type = "button";
        closeBtn.title = `Close ${filename}`;
        closeBtn.setAttribute("aria-label", `Close ${filename}`);
        closeBtn.textContent = "×";
        closeBtn.addEventListener("click", (e) => {
          e.stopPropagation();
          deleteLibraryFile(filename);
        });
        tab.appendChild(closeBtn);
      }

      tab.addEventListener("click", () => switchToFile(filename));
      fileTabListEl.appendChild(tab);
    }

    if (fileTabPickerEl) {
      for (const filename of tabs) {
        const option = document.createElement("option");
        option.value = filename;
        option.textContent = filename;
        fileTabPickerEl.appendChild(option);
      }
      fileTabPickerEl.value = activeFile;

      requestAnimationFrame(() => {
        const hasOverflow = fileTabListEl.scrollWidth > fileTabListEl.clientWidth + 2;
        fileTabPickerEl.hidden = !hasOverflow;
      });
    }
  }

  function syncActiveEditorToMap() {
    virtualFiles.set(activeFile, String(getEditorValue?.() ?? ""));
  }

  function switchToFile(filename: string) {
    if (!virtualFiles.has(filename) || filename === activeFile) return;
    syncActiveEditorToMap();
    visibleFiles.add(filename);
    activeFile = filename;
    setEditorSource(virtualFiles.get(filename) || "");
    renderFileTabs();
  }

  function deleteLibraryFile(filename: string) {
    if (filename === "main.fr" || !virtualFiles.has(filename)) return;
    virtualFiles.delete(filename);
    visibleFiles.delete(filename);
    if (activeFile === filename) {
      activeFile = "main.fr";
      setEditorSource(virtualFiles.get("main.fr") || "");
    }
    renderFileTabs();
    queueCompile?.({ delayMs: 0, cancelInFlight: true });
  }

  function resetVirtualFiles(mainSource = "") {
    virtualFiles = new Map([["main.fr", String(mainSource ?? "")]]);
    activeFile = "main.fr";
    visibleFiles = new Set(["main.fr"]);
    renderFileTabs();
  }

  function loadVirtualBundle(bundle: Iterable<[string, string]>) {
    virtualFiles = new Map(bundle);
    if (!virtualFiles.has("main.fr")) virtualFiles.set("main.fr", "");
    activeFile = "main.fr";
    visibleFiles = new Set(["main.fr"]);
    renderFileTabs();
  }

  function setActiveFileContent(text: string) {
    virtualFiles.set(activeFile, String(text ?? ""));
  }

  function getActiveFile() {
    return activeFile;
  }

  function getVirtualFiles() {
    return virtualFiles;
  }

  function getMainSource() {
    return virtualFiles.get("main.fr") || "";
  }

  function snapshotFilesOrNull() {
    return virtualFiles.size > 1 ? new Map(virtualFiles) : null;
  }

  fileTabPickerEl?.addEventListener("change", () => {
    const selected = fileTabPickerEl.value;
    if (selected) switchToFile(selected);
  });

  if (typeof window !== "undefined") {
    window.addEventListener("resize", () => {
      renderFileTabs();
    });
  }

  return {
    renderFileTabs,
    switchToFile,
    deleteLibraryFile,
    resetVirtualFiles,
    loadVirtualBundle,
    syncActiveEditorToMap,
    setActiveFileContent,
    getActiveFile,
    getVirtualFiles,
    getMainSource,
    snapshotFilesOrNull,
  };
}
