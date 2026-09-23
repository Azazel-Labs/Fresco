type BuiltinSignatureArg = {
  name?: unknown;
  ty?: unknown;
  required?: boolean;
  docs?: unknown;
  viz_role?: unknown;
};

type BuiltinSignature = {
  receiver?: unknown;
  args?: BuiltinSignatureArg[];
  returns?: unknown[];
};

type BuiltinDiscriminator = {
  name?: unknown;
  default?: unknown;
  variants?: unknown[];
};

type BuiltinReference = {
  name?: unknown;
  signatures?: BuiltinSignature[];
  summary?: unknown;
  discriminator?: BuiltinDiscriminator;
};

type BuiltinHelpState = {
  word: string;
  lineText: string;
  callExpr: string;
  builtinRef: BuiltinReference;
  position: unknown;
  start: number;
  end: number;
};

type SourceEditor = {
  onMouseMove(listener: (event: any) => void): { dispose(): void };
  onMouseLeave(listener: () => void): { dispose(): void };
  getModel(): any;
};

export function normalizeBuiltinReceiverKinds(builtinRef: BuiltinReference) {
  const receivers = Array.isArray(builtinRef?.signatures)
    ? builtinRef.signatures
      .map((sig) => (typeof sig?.receiver === "string" ? sig.receiver.trim().toLowerCase() : ""))
      .filter(Boolean)
    : [];
  return [...new Set(receivers)];
}

export function formatBuiltinSignature(name: unknown, sig: BuiltinSignature | null | undefined) {
  if (!sig || typeof sig !== "object") {
    return "";
  }

  const args = Array.isArray(sig.args)
    ? sig.args
      .map((arg) => {
        const argName = String(arg?.name || "").trim();
        const argTy = String(arg?.ty || "").trim();
        if (!argName) {
          return "";
        }
        const required = arg?.required !== false;
        return `${argName}${required ? "" : "?"}: ${argTy || "value"}`;
      })
      .filter(Boolean)
      .join(", ")
    : "";

  const returns = Array.isArray(sig.returns)
    ? sig.returns.map((ret) => String(ret || "").trim()).filter(Boolean).join(" | ")
    : "";

  const receiver = typeof sig.receiver === "string" ? sig.receiver.trim() : "";
  const head = receiver ? `${receiver} |> ${String(name || "").trim()}` : String(name || "").trim();
  return `${head}(${args})${returns ? ` -> ${returns}` : ""}`;
}

export function preferredBuiltinSignature(builtinRef: BuiltinReference) {
  const signatures = Array.isArray(builtinRef?.signatures) ? builtinRef.signatures : [];
  if (signatures.length === 0) {
    return "";
  }

  const receiverPreferred = signatures.find((sig) => {
    const receiver = typeof sig?.receiver === "string" ? sig.receiver.trim().toLowerCase() : "";
    return receiver === "shape" || receiver === "layer";
  });

  return formatBuiltinSignature(String(builtinRef?.name || "").trim(), receiverPreferred || signatures[0]);
}

function splitTopLevelArgs(text: unknown) {
  const source = String(text || "");
  const out: string[] = [];
  let current = "";
  let depthParen = 0;
  let depthBracket = 0;
  let depthBrace = 0;
  let quote = "";
  let prev = "";

  for (const ch of source) {
    if ((ch === '"' || ch === "'") && prev !== "\\") {
      if (!quote) {
        quote = ch;
      } else if (quote === ch) {
        quote = "";
      }
      current += ch;
      prev = ch;
      continue;
    }
    if (!quote) {
      if (ch === "(") depthParen += 1;
      else if (ch === ")") depthParen = Math.max(0, depthParen - 1);
      else if (ch === "[") depthBracket += 1;
      else if (ch === "]") depthBracket = Math.max(0, depthBracket - 1);
      else if (ch === "{") depthBrace += 1;
      else if (ch === "}") depthBrace = Math.max(0, depthBrace - 1);
      else if (ch === "," && depthParen === 0 && depthBracket === 0 && depthBrace === 0) {
        const token = current.trim();
        if (token) {
          out.push(token);
        }
        current = "";
        prev = ch;
        continue;
      }
    }
    current += ch;
    prev = ch;
  }

  const tail = current.trim();
  if (tail) {
    out.push(tail);
  }
  return out;
}

function extractCallExpressionFromLine(lineText: unknown, builtinName: unknown, column: unknown) {
  const line = String(lineText || "");
  const needle = String(builtinName || "").trim();
  if (!line || !needle) {
    return "";
  }

  const matches: number[] = [];
  const rx = new RegExp(`\\b${needle.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}\\s*\\(`, "g");
  let match: RegExpExecArray | null;
  while ((match = rx.exec(line)) !== null) {
    matches.push(match.index);
  }
  if (matches.length === 0) {
    return "";
  }

  const col = Number(column) || 0;
  const preferred = matches.find((idx) => col >= idx && col <= idx + needle.length + 1) ?? matches[0];
  const openIdx = line.indexOf("(", preferred + needle.length);
  if (openIdx < 0) {
    return "";
  }

  let depth = 0;
  let quote = "";
  let prev = "";
  for (let i = openIdx; i < line.length; i += 1) {
    const ch = line[i];
    if ((ch === '"' || ch === "'") && prev !== "\\") {
      if (!quote) quote = ch;
      else if (quote === ch) quote = "";
    } else if (!quote) {
      if (ch === "(") depth += 1;
      else if (ch === ")") {
        depth -= 1;
        if (depth === 0) {
          return line.slice(preferred, i + 1);
        }
      }
    }
    prev = ch;
  }
  return line.slice(preferred).trim();
}

function getModelLineText(model: any, lineNumber: number) {
  const maxColumn = model.getLineMaxColumn(lineNumber);
  return model.getValueInRange({
    startLineNumber: lineNumber,
    startColumn: 1,
    endLineNumber: lineNumber,
    endColumn: maxColumn
  });
}

function getIdentifierAtPosition(model: any, position: any) {
  if (!model || !position) {
    return null;
  }
  const lineText = getModelLineText(model, position.lineNumber);
  const columnIndex = Math.max(0, Number(position.column || 1) - 1);
  const rx = /[A-Za-z_][A-Za-z0-9_]*/g;
  let match: RegExpExecArray | null;
  while ((match = rx.exec(lineText)) !== null) {
    const start = match.index;
    const end = start + match[0].length;
    if (columnIndex >= start && columnIndex <= end) {
      return {
        word: match[0],
        lineText,
        start,
        end
      };
    }
  }
  return null;
}

function chooseBuiltinSignatureForCall(builtinRef: BuiltinReference, callExpr: unknown) {
  const signatures = Array.isArray(builtinRef?.signatures) ? builtinRef.signatures : [];
  if (signatures.length === 0) {
    return null;
  }
  const hasPipe = String(callExpr || "").includes("|>");
  if (hasPipe) {
    const receiverSig = signatures.find((sig) => typeof sig?.receiver === "string" && sig.receiver.trim());
    if (receiverSig) {
      return receiverSig;
    }
  }
  return signatures[0];
}

function extractNamedArgExprFromExpression(exprText: unknown, names: unknown[] = []) {
  const expr = String(exprText || "").trim();
  if (!expr || !Array.isArray(names) || names.length === 0) {
    return "";
  }

  for (const rawName of names) {
    const name = String(rawName || "").trim();
    if (!name) {
      continue;
    }
    const pattern = new RegExp(`\\b${name.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}\\s*:\\s*([^,\\)]+)`);
    const match = pattern.exec(expr);
    if (match?.[1]) {
      return match[1].trim();
    }
  }
  return "";
}

function extractRangeExprFromExpression(exprText: unknown, names: unknown[] = []) {
  const expr = String(exprText || "").trim();
  if (!expr || !Array.isArray(names) || names.length === 0) {
    return null;
  }

  for (const rawName of names) {
    const name = String(rawName || "").trim();
    if (!name) {
      continue;
    }
    const pattern = new RegExp(`\\b${name.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}\\s*:\\s*([^,\\)]*)\\.\\.\\s*([^,\\)]+)`);
    const match = pattern.exec(expr);
    if (match?.[1] && match?.[2]) {
      return {
        startExpr: match[1].trim(),
        endExpr: match[2].trim()
      };
    }
  }
  return null;
}

function buildBuiltinLineExplanation(help: BuiltinHelpState) {
  const builtinRef = help?.builtinRef;
  const callExpr = String(help?.callExpr || "").trim();
  const signature = chooseBuiltinSignatureForCall(builtinRef, callExpr);
  const lines: string[] = [];
  const titleSig = formatBuiltinSignature(help?.word || builtinRef?.name || "builtin", signature);
  if (titleSig) {
    lines.push(`signature: ${titleSig}`);
    lines.push("");
  }

  const returns = Array.isArray(signature?.returns) ? signature.returns.join(" | ") : "value";
  lines.push(`This line calls ${help.word} and produces ${returns}.`);

  const summary = String(builtinRef?.summary || "").trim();
  if (summary && !/^Builtin:/i.test(summary)) {
    lines.push(summary);
  }

  const discriminator = builtinRef?.discriminator;
  if (discriminator?.name) {
    const explicit = extractNamedArgExprFromExpression(callExpr, [discriminator.name]);
    const active = explicit || discriminator.default || "";
    if (active) {
      const variants = Array.isArray(discriminator.variants) ? discriminator.variants.join(", ") : "";
      lines.push(`shape variant: ${active}${variants ? ` (${variants})` : ""}`);
    }
  }

  const args = Array.isArray(signature?.args) ? signature.args : [];
  if (args.length > 0) {
    lines.push("");
    lines.push("arguments on this line:");
    for (const arg of args) {
      const argName = String(arg?.name || "").trim();
      if (!argName) {
        continue;
      }
      let expr = extractNamedArgExprFromExpression(callExpr, [argName]);
      if (!expr && String(arg?.viz_role || "").toLowerCase() === "range") {
        const rangeExpr = extractRangeExprFromExpression(callExpr, [argName]);
        expr = rangeExpr ? `${rangeExpr.startExpr} .. ${rangeExpr.endExpr}` : "";
      }
      if (!expr && discriminator?.name === argName && discriminator.default) {
        expr = `${discriminator.default} (default)`;
      }
      if (!expr) {
        continue;
      }
      const docs = String(arg?.docs || "").trim();
      lines.push(`- ${argName} = ${expr}${docs ? ` — ${docs}` : ""}`);
    }
  }

  const paramsText = callExpr.includes("(") ? callExpr.slice(callExpr.indexOf("(") + 1, callExpr.lastIndexOf(")")) : "";
  const positional = splitTopLevelArgs(paramsText).filter((part) => !/:/.test(part));
  if (positional.length > 0) {
    lines.push("");
    lines.push(`positional inputs: ${positional.join(", ")}`);
  }

  return lines.join("\n");
}

export function createBuiltinHelpController({ getSourceEditor, showDocsOverlay }: { getSourceEditor: () => SourceEditor | null; showDocsOverlay?: () => void }) {
  let builtinExplainDialog: any = null;
  let builtinHelpButton: HTMLButtonElement | null = null;
  let builtinHelpHideTimer = 0;
  let builtinHelpHoverState: BuiltinHelpState | null = null;
  let builtinHelpHoverKey = "";
  let builtinReferenceByName = new Map<string, BuiltinReference>();
  let hoverHelpRegistered = false;

  function ensureBuiltinExplainDialog() {
    if (builtinExplainDialog) {
      return builtinExplainDialog;
    }

    const backdrop = document.createElement("div");
    backdrop.className = "builtin-help-backdrop";
    backdrop.setAttribute("role", "dialog");
    backdrop.setAttribute("aria-modal", "true");

    const panel = document.createElement("div");
    panel.className = "builtin-help-panel";

    const header = document.createElement("div");
    header.className = "docs-header";

    const title = document.createElement("div");
    title.className = "docs-title";
    title.textContent = "Builtin Help";

    const controls = document.createElement("div");
    controls.className = "docs-controls";

    const docsBtn = document.createElement("button");
    docsBtn.className = "viz-action-btn";
    docsBtn.type = "button";
    docsBtn.textContent = "docs";

    const closeBtn = document.createElement("button");
    closeBtn.className = "viz-action-btn";
    closeBtn.type = "button";
    closeBtn.textContent = "close";

    const subtitle = document.createElement("div");
    subtitle.className = "builtin-help-subtitle";

    const body = document.createElement("pre");
    body.className = "builtin-help-body";

    const line = document.createElement("pre");
    line.className = "builtin-help-line";

    const close = () => {
      backdrop.style.display = "none";
    };

    docsBtn.addEventListener("click", () => {
      if (typeof showDocsOverlay === "function") {
        showDocsOverlay();
      }
    });
    closeBtn.addEventListener("click", close);
    backdrop.addEventListener("click", (event) => {
      if (event.target === backdrop) {
        close();
      }
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && backdrop.style.display !== "none") {
        close();
      }
    });

    controls.appendChild(docsBtn);
    controls.appendChild(closeBtn);
    header.appendChild(title);
    header.appendChild(controls);
    panel.appendChild(header);
    panel.appendChild(subtitle);
    panel.appendChild(line);
    panel.appendChild(body);
    backdrop.appendChild(panel);
    document.body.appendChild(backdrop);

    builtinExplainDialog = { backdrop, title, subtitle, line, body };
    return builtinExplainDialog;
  }

  function showBuiltinExplainDialog(help: BuiltinHelpState) {
    if (!help) {
      return;
    }
    const dialog = ensureBuiltinExplainDialog();
    dialog.title.textContent = `Builtin Help: ${help.word}`;
    dialog.subtitle.textContent = preferredBuiltinSignature(help.builtinRef) || help.word;
    dialog.line.textContent = String(help.lineText || "").trim();
    dialog.body.textContent = buildBuiltinLineExplanation(help);
    dialog.backdrop.style.display = "flex";
  }

  function ensureBuiltinHelpButton() {
    if (builtinHelpButton) {
      return builtinHelpButton;
    }
    const button = document.createElement("button");
    button.className = "builtin-help-fab";
    button.type = "button";
    button.textContent = "?";
    button.title = "Explain builtin on this line";
    button.style.display = "none";
    button.addEventListener("mouseenter", () => {
      clearTimeout(builtinHelpHideTimer);
    });
    button.addEventListener("mouseleave", () => {
      builtinHelpHideTimer = setTimeout(() => {
        button.style.display = "none";
      }, 120);
    });
    button.addEventListener("click", () => {
      if (builtinHelpHoverState) {
        showBuiltinExplainDialog(builtinHelpHoverState);
      }
    });
    document.body.appendChild(button);
    builtinHelpButton = button;
    return button;
  }

  function hideBuiltinHelpButton() {
    if (builtinHelpButton) {
      builtinHelpButton.style.display = "none";
    }
    builtinHelpHoverState = null;
    builtinHelpHoverKey = "";
  }

  function showBuiltinHelpButton(help: BuiltinHelpState, clientX: number, clientY: number) {
    const button = ensureBuiltinHelpButton();
    const nextKey = `${help.word}:${help.lineText || ""}:${help.start || 0}:${help.end || 0}`;
    const shouldReposition = button.style.display === "none" || nextKey !== builtinHelpHoverKey;
    builtinHelpHoverState = help;
    builtinHelpHoverKey = nextKey;
    if (shouldReposition) {
      button.style.left = `${Math.min(window.innerWidth - 36, Math.max(8, clientX + 10))}px`;
      button.style.top = `${Math.min(window.innerHeight - 36, Math.max(8, clientY - 12))}px`;
    }
    button.style.display = "flex";
  }

  function setupBuiltinHoverHelp() {
    if (hoverHelpRegistered) {
      return;
    }

    const sourceEditor = typeof getSourceEditor === "function" ? getSourceEditor() : null;
    if (!sourceEditor || typeof sourceEditor.onMouseMove !== "function") {
      return;
    }

    const model = sourceEditor.getModel();
    sourceEditor.onMouseMove((event) => {
      const position = event?.target?.position || null;
      const browserEvent = event?.event?.browserEvent || event?.browserEvent || null;
      if (!position || !browserEvent) {
        hideBuiltinHelpButton();
        return;
      }
      const ident = getIdentifierAtPosition(model, position);
      if (!ident) {
        hideBuiltinHelpButton();
        return;
      }
      const builtinRef = builtinReferenceByName.get(ident.word);
      if (!builtinRef) {
        hideBuiltinHelpButton();
        return;
      }
      const help: BuiltinHelpState = {
        word: ident.word,
        lineText: ident.lineText,
        callExpr: extractCallExpressionFromLine(ident.lineText, ident.word, ident.start),
        builtinRef,
        position,
        start: ident.start,
        end: ident.end
      };
      clearTimeout(builtinHelpHideTimer);
      showBuiltinHelpButton(help, browserEvent.clientX, browserEvent.clientY);
    });
    sourceEditor.onMouseLeave(() => {
      clearTimeout(builtinHelpHideTimer);
      builtinHelpHideTimer = setTimeout(() => {
        hideBuiltinHelpButton();
      }, 900);
    });
    hoverHelpRegistered = true;
  }

  function setBuiltinReference(builtinReference: BuiltinReference[]) {
    builtinReferenceByName = new Map(
      (Array.isArray(builtinReference) ? builtinReference : []).map((builtin) => [String(builtin.name || ""), builtin])
    );
  }

  return {
    setupBuiltinHoverHelp,
    setBuiltinReference
  };
}
