import { describe, expect, it } from "vitest";

import {
  formatDiagnosticLocation,
  resolveDiagnosticFileForDisplay,
} from "../app/diagnostics-ui";

describe("diagnostics UI file attribution", () => {
  const virtualDiagnosticFiles = new Set(["", "playground.fr", "<source>", "source"]);

  it("maps virtual diagnostic files to the active file for display", () => {
    const activeFile = "main.fr";
    const diag = { file: "playground.fr", span_start: 10, span_end: 24 };

    expect(resolveDiagnosticFileForDisplay(diag.file, activeFile, virtualDiagnosticFiles)).toBe("main.fr");
    expect(formatDiagnosticLocation(diag, activeFile, virtualDiagnosticFiles)).toBe("main.fr:10-24");
  });

  it("preserves imported file paths for multi-file diagnostics", () => {
    const activeFile = "main.fr";
    const diag = { file: "engine/pipelines/10_forward.fr", span_start: 124, span_end: 139 };

    expect(resolveDiagnosticFileForDisplay(diag.file, activeFile, virtualDiagnosticFiles)).toBe(
      "engine/pipelines/10_forward.fr"
    );
    expect(formatDiagnosticLocation(diag, activeFile, virtualDiagnosticFiles)).toBe(
      "engine/pipelines/10_forward.fr:124-139"
    );
  });
});
