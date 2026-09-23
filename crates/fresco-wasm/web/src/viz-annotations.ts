type DirectiveOptions = {
  kind: string;
  domain: string;
  title: string;
  controls: string[];
  sweepWindow: string | null;
};

type DirectiveMatch = DirectiveOptions & {
  lineNumber?: number;
};

type BindingSpan = {
  name: string;
  spanStart: number;
  spanEnd: number;
};

export type VizAnnotation = {
  name: string;
  title: string;
  kind: string;
  controls: string[];
  sweepMax: number;
  sweepHints: {
    periodExpr: string;
    frequencyExpr: string;
    windowExpr: string;
    rangeStartExpr: string;
    rangeEndExpr: string;
  } | null;
  spanStart: number;
  spanEnd: number;
  domain: string;
  lineNumber: number;
  lineText: string;
  directiveLineNumber: number;
  directivePlacement: "leading" | "trailing" | "below";
  stableKey?: string;
};

type LineModel = {
  getValue(): string;
};

type PendingBinding = {
  binding: BindingSpan;
  exprText: string;
  lineNumber: number;
  lineText: string;
};

function utf8LengthForCodePoint(codePoint: number) {
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

function utf8ByteLength(text: string) {
  let bytes = 0;
  for (const ch of text) {
    bytes += utf8LengthForCodePoint(ch.codePointAt(0) || 0);
  }
  return bytes;
}

function stripQuotes(value: unknown) {
  const text = String(value ?? "").trim();
  if (text.length >= 2) {
    const first = text[0];
    const last = text[text.length - 1];
    if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
      return text.slice(1, -1);
    }
  }
  return text;
}

function splitDirectiveTokens(text: string) {
  const tokens: string[] = [];
  let current = "";
  let quote = "";

  for (let index = 0; index < text.length; index += 1) {
    const ch = text[index];
    const prev = index > 0 ? text[index - 1] : "";

    if ((ch === '"' || ch === "'") && prev !== "\\") {
      if (!quote) {
        quote = ch;
      } else if (quote === ch) {
        quote = "";
      }
      current += ch;
      continue;
    }

    if (ch === "," && !quote) {
      const token = current.trim();
      if (token) {
        tokens.push(token);
      }
      current = "";
      continue;
    }

    current += ch;
  }

  const tail = current.trim();
  if (tail) {
    tokens.push(tail);
  }

  return tokens;
}

function parseControls(value: unknown) {
  const raw = stripQuotes(value);
  if (!raw) {
    return [];
  }
  return raw
    .split(/[|+/\s]+/)
    .map((item) => item.trim().toLowerCase())
    .filter(Boolean);
}

function parseDirectiveOptions(optionText: string): DirectiveOptions {
  const result: DirectiveOptions = {
    kind: "",
    domain: "auto",
    title: "",
    controls: [],
    sweepWindow: null
  };

  const tokens = splitDirectiveTokens(String(optionText || "").trim());
  for (const token of tokens) {
    const eqIdx = token.indexOf("=");
    if (eqIdx === -1) {
      const normalized = token.trim().toLowerCase();
      if (normalized === "time" || normalized === "x" || normalized === "thumb") {
        result.domain = normalized;
      } else if (normalized === "timeseries" || normalized === "chart") {
        result.kind = "timeseries";
      } else if (normalized === "thumbnail" || normalized === "preview" || normalized === "thumb") {
        result.kind = "thumbnail";
      } else if (!result.title) {
        result.title = stripQuotes(token);
      }
      continue;
    }

    const key = token.slice(0, eqIdx).trim().toLowerCase();
    const value = stripQuotes(token.slice(eqIdx + 1));
    if (key === "kind" || key === "mode") {
      const normalized = value.toLowerCase();
      if (normalized === "timeseries" || normalized === "chart" || normalized === "sparkline") {
        result.kind = "timeseries";
      } else if (normalized === "thumbnail" || normalized === "preview" || normalized === "thumb") {
        result.kind = "thumbnail";
      } else if (normalized === "color" || normalized === "swatch") {
        result.kind = "swatch";
      } else {
        result.kind = normalized;
      }
      continue;
    }

    if (key === "domain") {
      const normalized = value.toLowerCase();
      if (normalized === "time" || normalized === "x" || normalized === "thumb") {
        result.domain = normalized;
      }
      continue;
    }

    if (key === "title") {
      result.title = value;
      continue;
    }

    if (key === "controls") {
      result.controls = parseControls(value);
      continue;
    }

    if (key === "window" || key === "span") {
      result.sweepWindow = value;
    }
  }

  return result;
}

function parseNumberWithUnit(value: unknown) {
  const text = String(value || "").trim().toLowerCase();
  const match = text.match(/^([+-]?(?:\d+\.?\d*|\d*\.\d+))(ms|s|hz)?$/);
  if (!match) {
    return null;
  }
  const numeric = Number(match[1]);
  if (!Number.isFinite(numeric)) {
    return null;
  }
  const unit = match[2] || "";
  return {
    value: unit === "ms" ? numeric / 1000 : numeric,
    unit
  };
}

function extractNamedArgExpr(exprText: unknown, names: string[]) {
  const expr = String(exprText || "");
  for (const name of names) {
    const escaped = String(name).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const rx = new RegExp(`\\b${escaped}\\s*:\\s*([^,\\)]+)`, "i");
    const m = expr.match(rx);
    if (m && m[1]) {
      return m[1].trim();
    }
  }
  return "";
}

function extractSweepHints(exprText: unknown) {
  const expr = String(exprText || "");
  const periodExpr = extractNamedArgExpr(expr, ["period", "cycle"]);
  const frequencyExpr = extractNamedArgExpr(expr, ["frequency", "freq", "hz", "rate"]);
  const windowExpr = extractNamedArgExpr(expr, ["window", "span", "duration", "horizon"]);
  const rangeMatch = expr.match(/\brange\s*:\s*([^\.]+?)\.\.([^,\)]+)/i);
  const rangeStartExpr = rangeMatch?.[1]?.trim() || "";
  const rangeEndExpr = rangeMatch?.[2]?.trim() || "";

  if (!periodExpr && !frequencyExpr && !windowExpr && !rangeStartExpr && !rangeEndExpr) {
    return null;
  }

  return {
    periodExpr,
    frequencyExpr,
    windowExpr,
    rangeStartExpr,
    rangeEndExpr
  };
}

function findNamedArgValues(exprText: unknown, names: string[]) {
  const expr = String(exprText || "");
  const escaped = names
    .map((name) => String(name).replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
    .join("|");
  if (!escaped) {
    return [];
  }

  const pattern = new RegExp(`\\b(?:${escaped})\\s*:\\s*([+-]?(?:\\d+\\.?\\d*|\\d*\\.\\d+)\\s*(?:ms|s|hz)?)`, "gi");
  const out: Array<{ value: number; unit: string }> = [];
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(expr)) !== null) {
    const parsed = parseNumberWithUnit(match[1]);
    if (parsed && Number.isFinite(parsed.value)) {
      out.push(parsed);
    }
  }
  return out;
}

function inferFromRangeLiteral(domain: string, exprText: unknown) {
  const expr = String(exprText || "");
  const match = expr.match(/\brange\s*:\s*([+-]?(?:\d+\.?\d*|\d*\.\d+)\s*(?:ms|s)?)\s*\.\.\s*([+-]?(?:\d+\.?\d*|\d*\.\d+)\s*(?:ms|s)?)/i);
  if (!match) {
    return null;
  }
  const start = parseNumberWithUnit(match[1]);
  const end = parseNumberWithUnit(match[2]);
  if (!start || !end || !Number.isFinite(start.value) || !Number.isFinite(end.value)) {
    return null;
  }

  const span = Math.abs(end.value - start.value);
  if (!(span > 0)) {
    return null;
  }
  if (domain === "time") {
    return Math.max(0.1, Math.min(span, 32.0));
  }
  return Math.max(0.1, Math.min(span, 64.0));
}

const SWEEP_INFERENCE_RULES: Array<{
  id: string;
  infer: (domain: string, exprText: unknown) => number | null;
}> = [
  {
    id: "window-arg",
    infer(domain: string, exprText: unknown) {
      const vals = findNamedArgValues(exprText, ["window", "span", "duration", "horizon"]);
      const first = vals.find((v) => v.value > 0 && v.unit !== "hz");
      return first ? first.value : null;
    }
  },
  {
    id: "period-arg",
    infer(domain: string, exprText: unknown) {
      const vals = findNamedArgValues(exprText, ["period", "cycle"]);
      const first = vals.find((v) => v.value > 0 && v.unit !== "hz");
      if (!first) {
        return null;
      }
      const inferred = first.value;
      return Math.max(0.25, Math.min(inferred, 16.0));
    }
  },
  {
    id: "frequency-arg",
    infer(domain: string, exprText: unknown) {
      const vals = findNamedArgValues(exprText, ["frequency", "freq", "hz", "rate"]);
      const hz = vals.find((v) => v.value > 0 && (v.unit === "hz" || v.unit === ""));
      if (!hz) {
        return null;
      }
      const period = 1 / hz.value;
      const inferred = period;
      return Math.max(0.25, Math.min(inferred, 16.0));
    }
  },
  {
    id: "range-literal",
    infer(domain: string, exprText: unknown) {
      return inferFromRangeLiteral(domain, exprText);
    }
  }
];

function inferSweepMax(domain: string, exprText: unknown, directiveWindow: unknown): number {
  const parsedWindow = parseNumberWithUnit(directiveWindow);
  if (parsedWindow && parsedWindow.value > 0) {
    return parsedWindow.value;
  }

  for (const rule of SWEEP_INFERENCE_RULES) {
    const value = rule.infer(domain, exprText);
    if (typeof value === "number" && Number.isFinite(value) && value > 0) {
      return value;
    }
  }

  return domain === "x" ? 1.0 : 4.0;
}

function detectDomainFromExpression(exprText: unknown) {
  const expr = String(exprText || "");
  if (/\btime\b/.test(expr)) {
    return "time";
  }

  if (/\b(period|cycle|frequency|freq|hz|rate|every|over|window|duration|span|horizon)\s*:/i.test(expr)) {
    return "time";
  }

  return "x";
}

function extractBindingSpan(line: string, lineStartByte: number, bindingKind: "let" | "space" | "param") {
  const commentIndex = String(line || "").indexOf("//");
  const lineEndIndex = commentIndex === -1 ? String(line || "").length : commentIndex;

  if (bindingKind === "let" || bindingKind === "space") {
    const keyword = bindingKind === "space" ? "space" : "let";
    const match = line.match(new RegExp(`^\\s*${keyword}\\s+([A-Za-z_][A-Za-z0-9_]*)\\s*=`));
    if (!match) {
      return null;
    }
    const eqIdx = line.indexOf("=", match.index);
    if (eqIdx === -1) {
      return null;
    }
    const afterEq = line.slice(eqIdx + 1);
    const leadingWhitespace = afterEq.match(/^\s*/)?.[0].length || 0;
    const startOffset = eqIdx + 1 + leadingWhitespace;
    const start = lineStartByte + utf8ByteLength(line.slice(0, startOffset));
    const end = lineStartByte + utf8ByteLength(line.slice(0, lineEndIndex).trimEnd());
    return {
      name: match[1],
      spanStart: start,
      spanEnd: end
    };
  }

  const match = line.match(/^\s*param\s+([A-Za-z_][A-Za-z0-9_]*)\s*:/);
  if (!match) {
    return null;
  }

  const name = match[1];
  const nameStartOffset = line.indexOf(name, match.index);
  if (nameStartOffset === -1) {
    return null;
  }

  const nameEndOffset = nameStartOffset + name.length;
  const start = lineStartByte + utf8ByteLength(line.slice(0, nameStartOffset));
  const end = lineStartByte + utf8ByteLength(line.slice(0, nameEndOffset));
  return {
    name,
    spanStart: start,
    spanEnd: end
  };
}

function extractDirectiveMatch(line: string) {
  const text = String(line || "");
  const commentIndex = text.indexOf("//");
  if (commentIndex === -1) {
    return null;
  }

  const commentText = text.slice(commentIndex).trim();
  const match = commentText.match(/^\/\/\s*@(?:viz|visualizer)(?:\(([^)]*)\))?(?:\s+(.*))?$/i);
  if (!match) {
    return null;
  }

  const optionText = [match[1], match[2]].filter(Boolean).join(" ").trim();
  return parseDirectiveOptions(optionText);
}

function isDirectiveOnlyLine(line: string) {
  return /^\s*\/\/\s*@(?:viz|visualizer)/i.test(String(line || "")) && !/\b(let|param)\b/.test(line);
}

function normalizeStablePart(value: unknown) {
  return String(value || "").trim().replace(/\s+/g, " ").toLowerCase();
}

function stableAnnotationSignature(annotation: VizAnnotation) {
  const controls = Array.isArray(annotation?.controls) ? annotation.controls.join("|") : "";
  return [
    normalizeStablePart(annotation?.directivePlacement),
    normalizeStablePart(annotation?.name),
    normalizeStablePart(annotation?.title),
    normalizeStablePart(annotation?.kind),
    normalizeStablePart(annotation?.domain),
    normalizeStablePart(controls),
    normalizeStablePart(annotation?.lineText)
  ].join("::");
}

function assignStableKeys(annotations: VizAnnotation[]) {
  const seen = new Map<string, number>();
  for (const annotation of annotations) {
    const signature = stableAnnotationSignature(annotation);
    const nextIndex = (seen.get(signature) || 0) + 1;
    seen.set(signature, nextIndex);
    annotation.stableKey = `${signature}::${nextIndex}`;
  }
}

export class VizAnnotationScanner {
  scan(model: LineModel): VizAnnotation[] {
    const source = model.getValue();
    const lines = source.split("\n");
    const annotations: VizAnnotation[] = [];
    let pendingDirective: DirectiveMatch | null = null;
    let pendingBinding: PendingBinding | null = null;
    let byteOffset = 0;

    for (let index = 0; index < lines.length; index += 1) {
      const line = lines[index];
      const lineNumber = index + 1;
      const lineStartByte = byteOffset;
      const directive = extractDirectiveMatch(line);
      const letSpan = extractBindingSpan(line, lineStartByte, "let");
      const spaceSpan = letSpan ? null : extractBindingSpan(line, lineStartByte, "space");
      const paramSpan = letSpan || spaceSpan ? null : extractBindingSpan(line, lineStartByte, "param");
      const binding = letSpan || spaceSpan || paramSpan;

      if (directive && binding) {
        const exprText = line.slice(line.indexOf("=") + 1);
        const domain = directive.domain === "auto"
          ? detectDomainFromExpression(exprText)
          : directive.domain;
        const sweepMax = inferSweepMax(domain, exprText, directive.sweepWindow);
        const sweepHints = extractSweepHints(exprText);
        annotations.push({
          name: directive.title || binding.name,
          title: directive.title || binding.name,
          kind: directive.kind,
          controls: directive.controls,
          sweepMax,
          sweepHints,
          spanStart: binding.spanStart,
          spanEnd: binding.spanEnd,
          domain,
          lineNumber,
          lineText: line.trim(),
          directiveLineNumber: lineNumber,
          directivePlacement: "trailing"
        });
        pendingDirective = null;
        pendingBinding = null;
      } else if (pendingDirective && binding) {
        const exprText = line.slice(line.indexOf("=") + 1);
        const domain = pendingDirective.domain === "auto"
          ? detectDomainFromExpression(exprText)
          : pendingDirective.domain;
        const sweepMax = inferSweepMax(domain, exprText, pendingDirective.sweepWindow);
        const sweepHints = extractSweepHints(exprText);
        annotations.push({
          name: pendingDirective.title || binding.name,
          title: pendingDirective.title || binding.name,
          kind: pendingDirective.kind,
          controls: pendingDirective.controls,
          sweepMax,
          sweepHints,
          spanStart: binding.spanStart,
          spanEnd: binding.spanEnd,
          domain,
          lineNumber,
          lineText: line.trim(),
          directiveLineNumber: pendingDirective.lineNumber || lineNumber,
          directivePlacement: "leading"
        });
        pendingDirective = null;
        pendingBinding = null;
      } else if (directive && !binding && isDirectiveOnlyLine(line)) {
        if (pendingBinding) {
          const exprText = pendingBinding.exprText;
          const domain = directive.domain === "auto"
            ? detectDomainFromExpression(exprText)
            : directive.domain;
          const sweepMax = inferSweepMax(domain, exprText, directive.sweepWindow);
          const sweepHints = extractSweepHints(exprText);
          annotations.push({
            name: directive.title || pendingBinding.binding.name,
            title: directive.title || pendingBinding.binding.name,
            kind: directive.kind,
            controls: directive.controls,
            sweepMax,
            sweepHints,
            spanStart: pendingBinding.binding.spanStart,
            spanEnd: pendingBinding.binding.spanEnd,
            domain,
            lineNumber: pendingBinding.lineNumber,
            lineText: pendingBinding.lineText,
            directiveLineNumber: lineNumber,
            directivePlacement: "below"
          });
          pendingBinding = null;
          pendingDirective = null;
        } else {
          pendingDirective = {
            ...directive,
            lineNumber
          };
        }
      } else if (binding) {
        const eqIdx = line.indexOf("=");
        const exprText = eqIdx >= 0 ? line.slice(eqIdx + 1) : "";
        pendingBinding = {
          binding,
          exprText,
          lineNumber,
          lineText: line.trim()
        };
      } else if (pendingDirective && line.trim() && !line.trim().startsWith("//")) {
        pendingDirective = null;
        pendingBinding = null;
      } else if (!directive && line.trim() && !line.trim().startsWith("//")) {
        pendingBinding = null;
      }

      byteOffset += utf8ByteLength(line) + 1;
    }

    assignStableKeys(annotations);
    return annotations;
  }
}

export function removeVizDirectiveFromLine(line: unknown) {
  const text = String(line || "");
  const match = text.match(/^(.*?)(\s*\/\/\s*@(?:viz|visualizer)(?:\([^)]*\))?(?:\s+.*)?)$/i);
  if (!match) {
    return null;
  }

  const next = match[1].trimEnd();
  return next;
}
