export function normalizeSeverity(
  rawSeverity: unknown,
  warningAliases: Set<string>,
  infoAliases: Set<string>
): "warning" | "info" | "error" {
  const s = typeof rawSeverity === "string" ? rawSeverity.toLowerCase() : "error";
  if (warningAliases.has(s)) {
    return "warning";
  }
  if (infoAliases.has(s)) {
    return "info";
  }
  return "error";
}

export function utf8LengthForCodePoint(codePoint: number): number {
  if (codePoint <= 0x7f) {
    return 1;
  }
  if (codePoint <= 0x7ff) {
    return 2;
  }
  if (codePoint <= 0xffff) {
    return 3;
  }
  return 4;
}

export function utf8ByteLength(text: string): number {
  let bytes = 0;
  for (const ch of text) {
    bytes += utf8LengthForCodePoint(ch.codePointAt(0) || 0);
  }
  return bytes;
}

export function utf8ByteOffsetToUtf16Offset(text: string, targetByteOffset: number): number {
  const maxBytes = utf8ByteLength(text);
  const raw = Number(targetByteOffset);
  const finite = Number.isFinite(raw) ? Math.floor(raw) : 0;
  const clampedTarget = Math.max(0, Math.min(finite, maxBytes));
  let consumedBytes = 0;
  let consumedUtf16 = 0;

  for (const ch of text) {
    if (consumedBytes >= clampedTarget) {
      break;
    }

    const codePoint = ch.codePointAt(0) || 0;
    const nextBytes = utf8LengthForCodePoint(codePoint);
    if (consumedBytes + nextBytes > clampedTarget) {
      break;
    }

    consumedBytes += nextBytes;
    consumedUtf16 += ch.length;
  }

  return consumedUtf16;
}

export function getDiagnosticRange(diag: any, model: any) {
  const srcText = model.getValue();
  const srcByteLen = utf8ByteLength(srcText);
  const startByteOffset = Math.max(0, Math.min(Number(diag.span_start) || 0, srcByteLen));
  let endByteOffset = Math.max(startByteOffset, Math.min(Number(diag.span_end) || startByteOffset, srcByteLen));

  const startOffset = utf8ByteOffsetToUtf16Offset(srcText, startByteOffset);
  let endOffset = utf8ByteOffsetToUtf16Offset(srcText, endByteOffset);
  if (endOffset === startOffset) {
    endOffset = Math.min(model.getValueLength(), startOffset + 1);
  }
  const startPos = model.getPositionAt(startOffset);
  const endPos = model.getPositionAt(endOffset);

  return {
    startOffset,
    endOffset,
    startPos,
    endPos
  };
}

export function focusDiagnostic(diag: any, model: any, sourceEditor: any): void {
  const range = getDiagnosticRange(diag, model);
  sourceEditor.setPosition(range.startPos);
  sourceEditor.revealRangeInCenter({
    startLineNumber: range.startPos.lineNumber,
    startColumn: range.startPos.column,
    endLineNumber: range.endPos.lineNumber,
    endColumn: range.endPos.column
  });
  sourceEditor.focus();
}

export function isVirtualDiagnosticFile(file: unknown, virtualDiagnosticFiles: Set<string>): boolean {
  const f = typeof file === "string" ? file.trim().toLowerCase() : "";
  return virtualDiagnosticFiles.has(f);
}

export function resolveDiagnosticFileForDisplay(
  file: unknown,
  activeFile: string,
  virtualDiagnosticFiles: Set<string>
): string {
  const raw = typeof file === "string" ? file.trim() : "";
  if (!raw || isVirtualDiagnosticFile(raw, virtualDiagnosticFiles)) {
    return activeFile || "main.fr";
  }
  return raw;
}

export function formatDiagnosticLocation(
  diag: any,
  activeFile: string,
  virtualDiagnosticFiles: Set<string>
): string {
  const span = `${diag.span_start}-${diag.span_end}`;
  const file = resolveDiagnosticFileForDisplay(diag.file, activeFile, virtualDiagnosticFiles);
  return `${file}:${span}`;
}

export function showDiagnostics(params: {
  diags: any[];
  model: any;
  diagnosticsEl: HTMLElement;
  virtualFiles: Map<string, string>;
  sourceEditor: any;
  activeFile: string;
  switchToFile: (filename: string) => void;
  virtualDiagnosticFiles: Set<string>;
  normalizeSeverityFn: (value: unknown) => "warning" | "info" | "error";
}) {
  const {
    diags,
    model,
    diagnosticsEl,
    virtualFiles,
    sourceEditor,
    activeFile,
    switchToFile,
    virtualDiagnosticFiles,
    normalizeSeverityFn
  } = params;

  if (!diags || diags.length === 0) {
    diagnosticsEl.textContent = "No diagnostics.";
    return;
  }

  diagnosticsEl.innerHTML = "";
  let previousGroupFile = "";
  for (const d of diags) {
    const severity = normalizeSeverityFn(d.severity);
    const resolvedFile = resolveDiagnosticFileForDisplay(d.file, activeFile, virtualDiagnosticFiles);
    if (virtualFiles.size > 1 && resolvedFile !== previousGroupFile) {
      previousGroupFile = resolvedFile;
      const header = document.createElement("div");
      header.className = "diag-group-header";
      header.textContent = `File: ${resolvedFile}`;
      diagnosticsEl.appendChild(header);
    }

    const item = document.createElement("div");
    item.className = `diag-item ${severity}`;
    const title = document.createElement("strong");
    title.textContent = `${severity.toUpperCase()}  ${formatDiagnosticLocation(d, activeFile, virtualDiagnosticFiles)}`;
    const message = document.createElement("span");
    message.className = "diag-message";
    message.textContent = d.message;
    item.appendChild(title);
    item.appendChild(message);

    if (d.label) {
      const label = document.createElement("span");
      label.className = "diag-detail";
      label.textContent = `label: ${d.label}`;
      item.appendChild(label);
    }

    if (d.help) {
      const help = document.createElement("span");
      help.className = "diag-detail";
      help.textContent = `help: ${d.help}`;
      item.appendChild(help);
    }

    if (typeof d.action === "function" && d.actionLabel) {
      const actions = document.createElement("div");
      actions.className = "diag-actions";
      const actionBtn = document.createElement("button");
      actionBtn.type = "button";
      actionBtn.className = "diag-action-btn";
      actionBtn.textContent = String(d.actionLabel);
      actionBtn.addEventListener("click", (evt) => {
        evt.preventDefault();
        evt.stopPropagation();
        d.action();
      });
      actions.appendChild(actionBtn);
      item.appendChild(actions);
    }

    if (model) {
      item.tabIndex = 0;
      item.setAttribute("role", "button");
      item.addEventListener("click", () => {
        if (
          virtualFiles.size > 1
          && virtualFiles.has(resolvedFile)
        ) {
          switchToFile(resolvedFile);
        }
        focusDiagnostic(d, sourceEditor.getModel(), sourceEditor);
      });
      item.addEventListener("keydown", (evt) => {
        if (evt.key === "Enter" || evt.key === " ") {
          evt.preventDefault();
          if (
            virtualFiles.size > 1
            && virtualFiles.has(resolvedFile)
          ) {
            switchToFile(resolvedFile);
          }
          focusDiagnostic(d, sourceEditor.getModel(), sourceEditor);
        }
      });
    }

    diagnosticsEl.appendChild(item);
  }
}

export function mapDiagnosticsToMarkers(params: {
  diags: any[];
  model: any;
  virtualFiles: Map<string, string>;
  activeFile: string;
  virtualDiagnosticFiles: Set<string>;
  normalizeSeverityFn: (value: unknown) => "warning" | "info" | "error";
}) {
  const {
    diags,
    model,
    virtualFiles,
    activeFile,
    virtualDiagnosticFiles,
    normalizeSeverityFn
  } = params;

  return (diags || [])
    .filter((d) => {
      if (virtualFiles.size > 1) {
        if (activeFile === "main.fr") {
          return isVirtualDiagnosticFile(d.file, virtualDiagnosticFiles) || d.file === "main.fr";
        }
        return d.file === activeFile;
      }
      return true;
    })
    .map((d) => {
      const severity = normalizeSeverityFn(d.severity);
      const range = getDiagnosticRange(d, model);
      const extra = [
        `severity: ${severity.toUpperCase()}`,
        d.label ? `label: ${d.label}` : "",
        d.help ? `help: ${d.help}` : ""
      ]
        .filter(Boolean)
        .join("\n");

      return {
        from: range.startOffset,
        to: range.endOffset,
        severity,
        message: extra ? `${d.message}\n${extra}` : d.message
      };
    });
}
