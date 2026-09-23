type LanguageConfig = {
  keywords: string[];
  typeKeywords: string[];
  builtins: string[];
  spaceTransforms: string[];
  enumMembers: string[];
};

type EditorHighlightingDeps = {
  getLexTokens: () => any[];
  getLanguageConfig: () => LanguageConfig;
};

const STRUCTURAL_KEYWORDS = new Set([
  "fn",
  "let",
  "param",
  "module",
  "space",
  "if",
  "else",
  "for",
  "while",
  "return",
  "break",
  "continue",
  "in",
  "use",
]);

function utf8LengthForCodePoint(cp: number): number {
  if (!Number.isFinite(cp)) return 1;
  if (cp <= 0x7f) return 1;
  if (cp <= 0x7ff) return 2;
  if (cp <= 0xffff) return 3;
  return 4;
}

function utf8ByteOffsetToUtf16Offset(text: string, utf8Offset: number): number {
  const input = String(text || "");
  const target = Math.max(0, Number(utf8Offset) || 0);
  let u16 = 0;
  let u8 = 0;
  for (const ch of input) {
    if (u8 >= target) {
      break;
    }
    u8 += utf8LengthForCodePoint(ch.codePointAt(0) || 0);
    u16 += ch.length;
  }
  return Math.min(u16, input.length);
}

export function createEditorHighlightingController(deps: EditorHighlightingDeps) {
  const { getLexTokens, getLanguageConfig } = deps;

  const colorLiteralSwatchClassByHex = new Map<string, string>();
  let colorLiteralSwatchStyleEl: HTMLStyleElement | null = null;

  function isLabelLikeToken(sourceText: string, tokenEndOffset: number): boolean {
    const text = String(sourceText || "");
    let i = Math.max(0, Number(tokenEndOffset) || 0);
    while (i < text.length && /\s/.test(text[i])) {
      i += 1;
    }
    return text[i] === ":";
  }

  function isCallLikeToken(sourceText: string, tokenEndOffset: number): boolean {
    const text = String(sourceText || "");
    let i = Math.max(0, Number(tokenEndOffset) || 0);
    while (i < text.length && /\s/.test(text[i])) {
      i += 1;
    }
    return text[i] === "(";
  }

  function isPropertyAccessToken(sourceText: string, tokenStartOffset: number): boolean {
    const text = String(sourceText || "");
    let i = Math.max(0, Number(tokenStartOffset) || 0) - 1;
    while (i >= 0 && /\s/.test(text[i])) {
      i -= 1;
    }
    return i >= 0 && text[i] === ".";
  }

  function isLikelyEnumValueContext(sourceText: string, tokenStartOffset: number): boolean {
    if (isPropertyAccessToken(sourceText, tokenStartOffset)) {
      return false;
    }
    const text = String(sourceText || "");
    let i = Math.max(0, Number(tokenStartOffset) || 0) - 1;
    while (i >= 0 && /\s/.test(text[i])) {
      i -= 1;
    }
    if (i < 0) {
      return false;
    }
    const prev = text[i];
    if (prev === "(" || prev === "," || prev === "=" || prev === "[" || prev === ":" || prev === "{") {
      return true;
    }

    // Support enum-like values that appear after statement keywords, e.g. `return shape`.
    if (/[A-Za-z0-9_]/.test(prev)) {
      let end = i + 1;
      let start = i;
      while (start >= 0 && /[A-Za-z0-9_]/.test(text[start])) {
        start -= 1;
      }
      const prevWord = text.slice(start + 1, end).toLowerCase();
      if (prevWord === "return") {
        return true;
      }
    }

    return false;
  }

  function shouldTreatAsEnumMemberToken(
    token: string,
    sourceText: string,
    tokenStartOffset: number,
    enumLike: Set<string>,
  ): boolean {
    if (!isLikelyEnumValueContext(sourceText, tokenStartOffset)) {
      return false;
    }
    if (enumLike.has(token)) {
      return true;
    }
    return !STRUCTURAL_KEYWORDS.has(String(token || "").toLowerCase());
  }

  function isDeclarationTypeContext(sourceText: string, tokenStartOffset: number): boolean {
    const text = String(sourceText || "");
    let i = Math.max(0, Number(tokenStartOffset) || 0) - 1;
    while (i >= 0 && /\s/.test(text[i])) {
      i -= 1;
    }
    if (i < 0 || text[i] !== ":") {
      return false;
    }

    const lineStart = text.lastIndexOf("\n", i) + 1;
    const prefix = text.slice(lineStart, i + 1);

    if (/^\s*(?:param|let)\s+[A-Za-z_][A-Za-z0-9_]*\s*:\s*$/.test(prefix)) {
      return true;
    }

    // Treat function/canvas signatures as declaration type contexts.
    if (/\b(?:canvas|fn)\b[^\n{}]*\([^\n)]*:\s*$/.test(prefix)) {
      return true;
    }

    return false;
  }

  function collectGenericTypeIdentifierRanges(sourceText: string): Array<{ from: number; to: number }> {
    const text = String(sourceText || "");
    const ranges: Array<{ from: number; to: number }> = [];
    const seen = new Set<string>();
    const idRx = /\b[A-Za-z_][A-Za-z0-9_]*\b/g;
    let match: RegExpExecArray | null = null;

    while ((match = idRx.exec(text)) !== null) {
      const idFrom = match.index;
      const idText = String(match[0] || "");
      const idTo = idFrom + idText.length;
      let i = idTo;
      while (i < text.length && /\s/.test(text[i])) {
        i += 1;
      }
      if (text[i] !== "<") {
        continue;
      }

      // Treat generics as type annotations only in clear type positions.
      let j = idFrom - 1;
      while (j >= 0 && /\s/.test(text[j])) {
        j -= 1;
      }
      const before = j >= 0 ? text[j] : "";
      if (!(before === ":" || before === "," || before === "(" || before === ")" || (before === ">" && text[j - 1] === "-"))) {
        continue;
      }

      let depth = 0;
      let end = i;
      while (end < text.length) {
        const ch = text[end];
        if (ch === "<") {
          depth += 1;
        } else if (ch === ">") {
          depth -= 1;
          if (depth === 0) {
            end += 1;
            break;
          }
        }
        end += 1;
      }
      if (depth !== 0 || end <= i) {
        continue;
      }

      const genericChunk = text.slice(idFrom, end);
      const localIdRx = /\b[A-Za-z_][A-Za-z0-9_]*\b/g;
      let local: RegExpExecArray | null = null;
      while ((local = localIdRx.exec(genericChunk)) !== null) {
        const from = idFrom + local.index;
        const to = from + String(local[0] || "").length;
        const key = `${from}:${to}`;
        if (!seen.has(key)) {
          seen.add(key);
          ranges.push({ from, to });
        }
      }
    }

    return ranges;
  }

  function inRanges(from: number, to: number, ranges: Array<{ from: number; to: number }>): boolean {
    if (!Array.isArray(ranges) || ranges.length === 0) {
      return false;
    }
    return ranges.some((range) => from < range.to && to > range.from);
  }

  function applyCommentSpanPrecedence(spans: any[]): any[] {
    const list = Array.isArray(spans) ? spans : [];
    const commentRanges = list
      .filter((span) => span?.className === "cm-fresco-comment")
      .map((span) => ({
        from: Math.max(0, Number(span?.from) || 0),
        to: Math.max(0, Number(span?.to) || 0)
      }))
      .filter((range) => range.to > range.from);

    if (commentRanges.length === 0) {
      return list;
    }

    const overlapsComment = (from: number, to: number) =>
      commentRanges.some((range) => from < range.to && to > range.from);

    return list.filter((span) => {
      const className = String(span?.className || "");
      if (className === "cm-fresco-comment") {
        return true;
      }
      const from = Math.max(0, Number(span?.from) || 0);
      const to = Math.max(0, Number(span?.to) || 0);
      if (to <= from) {
        return false;
      }
      return !overlapsComment(from, to);
    });
  }

  function applyColorLiteralSpanPrecedence(spans: any[]): any[] {
    const list = Array.isArray(spans) ? spans : [];
    const colorRanges = list
      .filter((span) => String(span?.className || "").includes("cm-fresco-color-literal"))
      .map((span) => ({
        from: Math.max(0, Number(span?.from) || 0),
        to: Math.max(0, Number(span?.to) || 0)
      }))
      .filter((range) => range.to > range.from);

    if (colorRanges.length === 0) {
      return list;
    }

    const overlapsColor = (from: number, to: number) =>
      colorRanges.some((range) => from < range.to && to > range.from);

    return list.filter((span) => {
      const className = String(span?.className || "");
      if (className.includes("cm-fresco-color-literal") || className === "cm-fresco-comment") {
        return true;
      }
      const from = Math.max(0, Number(span?.from) || 0);
      const to = Math.max(0, Number(span?.to) || 0);
      if (to <= from) {
        return false;
      }
      return !overlapsColor(from, to);
    });
  }

  function collectColorLiteralRanges(sourceText: string) {
    const text = String(sourceText || "");
    const ranges: Array<{ from: number; to: number; hex: string }> = [];
    const seen = new Set<string>();
    const rx = /(["'])?((?<![0-9a-fA-F])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{4}|[0-9a-fA-F]{3})(?![0-9a-fA-F]))(?:\1)?/g;
    let match: RegExpExecArray | null = null;
    while ((match = rx.exec(text)) !== null) {
      const full = String(match[0] || "");
      const hex = String(match[2] || "");
      const fullFrom = match.index;
      const hexOffsetInMatch = full.indexOf(hex);
      const from = fullFrom + Math.max(0, hexOffsetInMatch);
      const to = from + hex.length;
      if (to <= from) {
        continue;
      }
      const key = `${from}:${to}`;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      ranges.push({ from, to, hex });
    }
    return ranges;
  }

  function normalizeHexForColorInput(rawHex: string): string {
    const hex = String(rawHex || "").toLowerCase();
    if (/^#[0-9a-f]{3}$/.test(hex)) {
      return `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}`;
    }
    if (/^#[0-9a-f]{4}$/.test(hex)) {
      return `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}`;
    }
    if (/^#[0-9a-f]{6}$/.test(hex)) {
      return hex;
    }
    if (/^#[0-9a-f]{8}$/.test(hex)) {
      return hex.slice(0, 7);
    }
    return "#ffffff";
  }

  function ensureColorLiteralSwatchClass(rawHex: string): string {
    const hex = String(rawHex || "").toLowerCase();
    if (!/^#[0-9a-f]{3,8}$/.test(hex)) {
      return "";
    }

    const existing = colorLiteralSwatchClassByHex.get(hex);
    if (existing) {
      return existing;
    }

    const slug = hex.slice(1).replace(/[^0-9a-f]/g, "");
    if (!slug) {
      return "";
    }

    const className = `cm-fresco-color-swatch-${slug}`;
    if (!colorLiteralSwatchStyleEl) {
      colorLiteralSwatchStyleEl = document.createElement("style");
      colorLiteralSwatchStyleEl.setAttribute("data-fresco-color-swatch-styles", "true");
      document.head.appendChild(colorLiteralSwatchStyleEl);
    }
    colorLiteralSwatchStyleEl.appendChild(
      document.createTextNode(`\n#editor .${className} { color: ${hex}; }\n`)
    );
    colorLiteralSwatchClassByHex.set(hex, className);
    return className;
  }

  function collectDeclaredIdentifiers(sourceText: string): Set<string> {
    const text = String(sourceText || "");
    const names = new Set<string>();
    const addIfValid = (name: unknown) => {
      const trimmed = String(name || "").trim();
      if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(trimmed)) {
        names.add(trimmed);
      }
    };

    const topLevelDeclRx = /\b(?:let|param|fn|module|space)\s+([A-Za-z_][A-Za-z0-9_]*)/g;
    let match: RegExpExecArray | null = null;
    while ((match = topLevelDeclRx.exec(text)) !== null) {
      addIfValid(match[1]);
    }

    const fnSigRx = /\bfn\s+[A-Za-z_][A-Za-z0-9_]*\s*\(([^)]*)\)/g;
    while ((match = fnSigRx.exec(text)) !== null) {
      const rawArgs = String(match[1] || "");
      for (const part of rawArgs.split(",")) {
        const lhs = String(part || "").split(":", 1)[0].trim();
        addIfValid(lhs);
      }
    }

    return names;
  }

  function buildFallbackSyntaxHighlights(sourceText: string) {
    const text = String(sourceText || "");
    if (!text) {
      return [];
    }
    const colorLiteralRanges = collectColorLiteralRanges(text);
    const genericTypeRanges = collectGenericTypeIdentifierRanges(text);

    const spans: any[] = [];
    const pushCommentSpans = () => {
      let i = 0;
      let quote = "";
      while (i < text.length) {
        const ch = text[i];
        if (quote) {
          if (ch === "\\") {
            i += 2;
            continue;
          }
          if (ch === quote) {
            quote = "";
          }
          i += 1;
          continue;
        }

        if (ch === '"' || ch === "'") {
          quote = ch;
          i += 1;
          continue;
        }

        if (ch === "/" && text[i + 1] === "/") {
          const from = i;
          i += 2;
          while (i < text.length && text[i] !== "\n" && text[i] !== "\r") {
            i += 1;
          }
          if (i > from) {
            spans.push({ from, to: i, className: "cm-fresco-comment" });
          }
          continue;
        }

        if (ch === "/" && text[i + 1] === "*") {
          const from = i;
          i += 2;
          while (i + 1 < text.length && !(text[i] === "*" && text[i + 1] === "/")) {
            i += 1;
          }
          i = Math.min(i + 2, text.length);
          if (i > from) {
            spans.push({ from, to: i, className: "cm-fresco-comment" });
          }
          continue;
        }

        i += 1;
      }
    };

    const pushMatches = (rx: RegExp, className: string) => {
      rx.lastIndex = 0;
      let match: RegExpExecArray | null = null;
      while ((match = rx.exec(text)) !== null) {
        const from = match.index;
        const to = from + String(match[0] || "").length;
        if (to > from) {
          spans.push({ from, to, className });
        }
      }
    };

    pushCommentSpans();
    pushMatches(/"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'/g, "cm-fresco-string");
    pushMatches(/\b(?:\d+\.?\d*|\d*\.\d+)(?:[eE][+-]?\d+)?\b/g, "cm-fresco-number");

    const config = getLanguageConfig();
    const keywordLike = new Set(Array.isArray(config.keywords) ? config.keywords : []);
    const builtinLike = new Set(Array.isArray(config.builtins) ? config.builtins : []);
    const transformLike = new Set(Array.isArray(config.spaceTransforms) ? config.spaceTransforms : []);
    const enumLike = new Set(Array.isArray(config.enumMembers) ? config.enumMembers : []);
    const typeLike = new Set(Array.isArray(config.typeKeywords) ? config.typeKeywords : []);
    const shadowedIdentifiers = collectDeclaredIdentifiers(text);

    pushMatches(/\b[A-Za-z_][A-Za-z0-9_]*\b/g, "cm-fresco-variable");
    for (let i = 0; i < spans.length; i += 1) {
      const span = spans[i];
      if (span.className !== "cm-fresco-variable") {
        continue;
      }
      const token = text.slice(span.from, span.to);
      if (shadowedIdentifiers.has(token)) {
        span.className = "cm-fresco-variable";
      } else if (isLabelLikeToken(text, span.to)) {
        span.className = "cm-fresco-parameter";
      } else if (inRanges(span.from, span.to, genericTypeRanges)) {
        span.className = "cm-fresco-type";
      } else if (shouldTreatAsEnumMemberToken(token, text, span.from, enumLike)
        && (!typeLike.has(token) || !isDeclarationTypeContext(text, span.from))) {
        span.className = "cm-fresco-enum-member";
      } else if (typeLike.has(token)) {
        span.className = "cm-fresco-type";
      } else if ((builtinLike.has(token) || transformLike.has(token)) && isCallLikeToken(text, span.to)) {
        span.className = "cm-fresco-function";
      } else if (keywordLike.has(token)) {
        span.className = "cm-fresco-keyword";
      }
    }

    for (const range of colorLiteralRanges) {
      const swatchClass = ensureColorLiteralSwatchClass(range.hex);
      spans.push({
        from: range.from,
        to: range.to,
        className: "cm-fresco-color-literal",
        beforeContent: "■",
        beforeClassName: swatchClass
          ? `cm-fresco-color-swatch-glyph ${swatchClass}`
          : "cm-fresco-color-swatch-glyph"
      });
    }

    return applyColorLiteralSpanPrecedence(applyCommentSpanPrecedence(spans));
  }

  const syntaxHighlighter = (_model: unknown, sourceText: string) => {
    const lexed = Array.isArray(getLexTokens()) ? getLexTokens() : [];
    const colorLiteralRanges = collectColorLiteralRanges(sourceText);
    const genericTypeRanges = collectGenericTypeIdentifierRanges(sourceText);
    const shadowedIdentifiers = collectDeclaredIdentifiers(sourceText);
    const config = getLanguageConfig();
    const keywordLike = new Set(Array.isArray(config.keywords) ? config.keywords : []);
    const builtinLike = new Set(Array.isArray(config.builtins) ? config.builtins : []);
    const transformLike = new Set(Array.isArray(config.spaceTransforms) ? config.spaceTransforms : []);
    const enumLike = new Set(Array.isArray(config.enumMembers) ? config.enumMembers : []);
    const typeLike = new Set(Array.isArray(config.typeKeywords) ? config.typeKeywords : []);

    if (lexed.length === 0) {
      return buildFallbackSyntaxHighlights(sourceText);
    }

    const spans: any[] = lexed
      .map((token: any) => {
        const startByte = Number(token?.start) || 0;
        const endByte = Number(token?.end) || startByte;
        const from = utf8ByteOffsetToUtf16Offset(sourceText, startByte);
        const to = utf8ByteOffsetToUtf16Offset(sourceText, endByte);
        if (to <= from) {
          return null;
        }

        const text = sourceText.slice(from, to);
        let className = "cm-fresco-variable";
        switch (token?.kind) {
          case "engine_keyword":
            className = "cm-fresco-keyword";
            break;
          case "keyword":
            if (shadowedIdentifiers.has(text)) {
              className = "cm-fresco-variable";
            } else if (isLabelLikeToken(sourceText, to)) {
              className = "cm-fresco-parameter";
            } else if (inRanges(from, to, genericTypeRanges)) {
              className = "cm-fresco-type";
            } else if (shouldTreatAsEnumMemberToken(text, sourceText, from, enumLike)
              && (!typeLike.has(text) || !isDeclarationTypeContext(sourceText, from))) {
              className = "cm-fresco-enum-member";
            } else if (typeLike.has(text)) {
              className = "cm-fresco-type";
            } else if ((builtinLike.has(text) || transformLike.has(text)) && isCallLikeToken(sourceText, to)) {
              className = "cm-fresco-function";
            } else {
              className = "cm-fresco-keyword";
            }
            break;
          case "operator":
            className = "cm-fresco-operator";
            break;
          case "number":
            className = "cm-fresco-number";
            break;
          case "string":
            className = "cm-fresco-string";
            break;
          case "comment":
            className = "cm-fresco-comment";
            break;
          case "identifier": {
            if (isLabelLikeToken(sourceText, to)) {
              className = "cm-fresco-parameter";
            } else if (shadowedIdentifiers.has(text)) {
              className = "cm-fresco-variable";
            } else if (inRanges(from, to, genericTypeRanges)) {
              className = "cm-fresco-type";
            } else if (shouldTreatAsEnumMemberToken(text, sourceText, from, enumLike)
              && (!typeLike.has(text) || !isDeclarationTypeContext(sourceText, from))) {
              className = "cm-fresco-enum-member";
            } else if (typeLike.has(text)) {
              className = "cm-fresco-type";
            } else if ((builtinLike.has(text) || transformLike.has(text)) && isCallLikeToken(sourceText, to)) {
              className = "cm-fresco-function";
            } else if (keywordLike.has(text)) {
              className = "cm-fresco-keyword";
            } else {
              className = "cm-fresco-variable";
            }
            break;
          }
          default:
            className = "cm-fresco-variable";
            break;
        }

        return { from, to, className };
      })
      .filter(Boolean);

    for (const range of colorLiteralRanges) {
      const swatchClass = ensureColorLiteralSwatchClass(range.hex);
      spans.push({
        from: range.from,
        to: range.to,
        className: "cm-fresco-color-literal",
        beforeContent: "■",
        beforeClassName: swatchClass
          ? `cm-fresco-color-swatch-glyph ${swatchClass}`
          : "cm-fresco-color-swatch-glyph"
      });
    }

    return applyColorLiteralSpanPrecedence(applyCommentSpanPrecedence(spans));
  };

  function installColorLiteralSwatchPicker(sourceEditor: any): void {
    const input = document.createElement("input");
    input.type = "color";
    input.style.position = "fixed";
    input.style.left = "-9999px";
    input.style.top = "-9999px";
    input.style.width = "0";
    input.style.height = "0";
    input.style.opacity = "0";
    input.setAttribute("aria-hidden", "true");
    document.body.appendChild(input);

    let pending: { from: number; to: number; hex: string } | null = null;

    const isIntentionalSwatchPick = (event: any) => {
      const browserEvent = event?.event;
      if (browserEvent?.ctrlKey || browserEvent?.metaKey) {
        return true;
      }
      const targetElement = event?.target?.element;
      if (!targetElement || typeof targetElement.closest !== "function") {
        return false;
      }
      return Boolean(targetElement.closest(".cm-fresco-color-swatch-glyph"));
    };

    const applyColor = () => {
      if (!pending) {
        return;
      }
      const next = String(input.value || "").toLowerCase();
      if (!/^#[0-9a-f]{6}$/.test(next)) {
        pending = null;
        return;
      }

      const originalHex = String(pending.hex || "").toLowerCase();
      let replacement = next;
      if (/^#[0-9a-f]{4}$/.test(originalHex)) {
        replacement = `${next}${originalHex[4]}`;
      } else if (/^#[0-9a-f]{8}$/.test(originalHex)) {
        replacement = `${next}${originalHex.slice(7, 9)}`;
      }

      const model = sourceEditor?.getModel();
      if (!model) {
        pending = null;
        return;
      }
      const fromPos = model.getPositionAt(pending.from);
      const toPos = model.getPositionAt(pending.to);
      model.pushEditOperations([], [{
        range: {
          startLineNumber: fromPos.lineNumber,
          startColumn: fromPos.column,
          endLineNumber: toPos.lineNumber,
          endColumn: toPos.column
        },
        text: replacement
      }], null);
      pending = null;
    };

    input.addEventListener("input", applyColor);
    input.addEventListener("change", applyColor);

    sourceEditor.onMouseDown((event: any) => {
      if (!isIntentionalSwatchPick(event)) {
        return;
      }
      const position = event?.target?.position;
      if (!position) {
        return;
      }
      const model = sourceEditor?.getModel();
      if (!model) {
        return;
      }

      const offset = model.getOffsetAt(position);
      const sourceText = model.getValue();
      const ranges = collectColorLiteralRanges(sourceText);
      const hit = ranges.find((range) => offset >= range.from && offset <= range.to);
      if (!hit) {
        return;
      }

      pending = hit;
      input.value = normalizeHexForColorInput(hit.hex);
      input.click();
    });
  }

  return {
    syntaxHighlighter,
    installColorLiteralSwatchPicker,
  };
}
