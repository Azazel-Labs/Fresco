import { detectFrescoEntryFunction } from "../wgsl-entry";

export type SweepArgRole = "window" | "period" | "frequency" | "range";

export type BuiltinVizSweepMeta = {
  windowArgs: string[];
  periodArgs: string[];
  frequencyArgs: string[];
  rangeArgs: string[];
};

export type SweepInfo = {
  value: number;
  source: string;
  detail: string;
  domain: string;
  exprText: string;
  calledBuiltin: string;
  builtinMeta: BuiltinVizSweepMeta | null;
};

export function buildBuiltinVizSweepMeta(builtinReference: unknown): Map<string, BuiltinVizSweepMeta> {
  const map = new Map<string, BuiltinVizSweepMeta>();
  const list = Array.isArray(builtinReference) ? builtinReference : [];
  for (const builtin of list) {
    const name = String((builtin as any)?.name || "").trim();
    if (!name) {
      continue;
    }
    const meta = {
      windowArgs: new Set<string>(),
      periodArgs: new Set<string>(),
      frequencyArgs: new Set<string>(),
      rangeArgs: new Set<string>()
    };

    const signatures = Array.isArray((builtin as any)?.signatures) ? (builtin as any).signatures : [];
    for (const sig of signatures) {
      const args = Array.isArray((sig as any)?.args) ? (sig as any).args : [];
      for (const arg of args) {
        const argName = String((arg as any)?.name || "").trim();
        const kind = String((arg as any)?.viz_role || "").trim().toLowerCase() as SweepArgRole | "";
        if (!argName || !kind) {
          continue;
        }
        if (kind === "window") {
          meta.windowArgs.add(argName);
        } else if (kind === "period") {
          meta.periodArgs.add(argName);
        } else if (kind === "frequency") {
          meta.frequencyArgs.add(argName);
        } else if (kind === "range") {
          meta.rangeArgs.add(argName);
        }
      }
    }

    map.set(name, {
      windowArgs: [...meta.windowArgs],
      periodArgs: [...meta.periodArgs],
      frequencyArgs: [...meta.frequencyArgs],
      rangeArgs: [...meta.rangeArgs]
    });
  }
  return map;
}

export function buildNumericEnvFromParams(paramValues: Map<unknown, unknown> | null | undefined): Record<string, number> {
  const env: Record<string, number> = Object.create(null);
  if (!paramValues) {
    return env;
  }

  for (const [name, value] of paramValues.entries()) {
    const key = String(name || "").trim();
    if (!key) {
      continue;
    }
    if (Array.isArray(value)) {
      const first = Number(value[0]);
      if (Number.isFinite(first)) {
        env[key] = first;
      }
      continue;
    }
    if (typeof value === "boolean") {
      env[key] = value ? 1 : 0;
      continue;
    }
    const num = Number(value);
    if (Number.isFinite(num)) {
      env[key] = num;
    }
  }

  return env;
}

export function evaluateNumericExpression(
  expr: string | number | null | undefined,
  env: Record<string, number> = {},
  kind: "scalar" | "time" = "scalar"
): number {
  const raw = String(expr || "").trim();
  if (!raw) {
    return Number.NaN;
  }

  const normalized = raw
    .replace(/([0-9]+(?:\.[0-9]+)?)\s*ms\b/gi, (_, n) => `(${Number(n) / 1000})`)
    .replace(/([0-9]+(?:\.[0-9]+)?)\s*s\b/gi, (_, n) => `(${n})`)
    .replace(/([0-9]+(?:\.[0-9]+)?)\s*hz\b/gi, (_, n) => `(${n})`);

  try {
    const argNames = Object.keys(env);
    const argValues = argNames.map((name) => env[name]);
    const value = Function(...argNames, `"use strict"; return (${normalized});`)(...argValues);
    const num = Number(value);
    if (!Number.isFinite(num)) {
      return Number.NaN;
    }
    if (kind === "time") {
      return Math.max(0.0001, num);
    }
    return num;
  } catch {
    return Number.NaN;
  }
}

export function extractAnnotationExprText(annotation: any): string {
  const line = String(annotation?.lineText || "");
  if (!line) {
    return "";
  }

  const eqIdx = line.indexOf("=");
  if (eqIdx >= 0) {
    return line.slice(eqIdx + 1).trim();
  }

  const paramMatch = line.match(/^\s*param\s+[A-Za-z_][A-Za-z0-9_]*\s*:\s*[^=]+?=\s*(.+)$/);
  if (paramMatch?.[1]) {
    return String(paramMatch[1]).trim();
  }

  return line.trim();
}

function detectCalledBuiltinName(exprText: string): string {
  const expr = String(exprText || "").trim();
  if (!expr) {
    return "";
  }

  const directCall = expr.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*\(/);
  if (directCall?.[1]) {
    return directCall[1];
  }

  const pipeMatches = [...expr.matchAll(/\|>\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(/g)];
  if (pipeMatches.length > 0) {
    return pipeMatches[pipeMatches.length - 1][1];
  }

  return "";
}

function extractNamedArgExprFromExpression(exprText: string, names: string[] = []): string {
  const expr = String(exprText || "");
  for (const rawName of names) {
    const name = String(rawName || "").trim();
    if (!name) {
      continue;
    }
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const rx = new RegExp(`\\b${escaped}\\s*:\\s*([^,\\)]+)`, "i");
    const m = expr.match(rx);
    if (m && m[1]) {
      return m[1].trim();
    }
  }
  return "";
}

function extractRangeExprFromExpression(exprText: string, names: string[] = []): { startExpr: string; endExpr: string } | null {
  const expr = String(exprText || "");
  for (const rawName of names) {
    const name = String(rawName || "").trim();
    if (!name) {
      continue;
    }
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const rx = new RegExp(`\\b${escaped}\\s*:\\s*([^\.]+?)\\.\\.([^,\\)]+)`, "i");
    const m = expr.match(rx);
    if (m && m[1] && m[2]) {
      return {
        startExpr: m[1].trim(),
        endExpr: m[2].trim()
      };
    }
  }
  return null;
}

export function resolveAnnotationSweep(
  annotation: any,
  env: Record<string, number>,
  builtinVizSweepMetaByName: Map<string, BuiltinVizSweepMeta>
): SweepInfo {
  const domain = String(annotation?.domain || "");
  const fallback = domain === "x" ? 1.0 : 4.0;
  const base = Number(annotation?.sweepMax);
  const hints = annotation?.sweepHints || {};
  const exprText = extractAnnotationExprText(annotation);
  const calledBuiltin = detectCalledBuiltinName(exprText);
  const builtinMeta = builtinVizSweepMetaByName.get(calledBuiltin) || null;

  const result = (value: number, source: string, detail: string): SweepInfo => ({
    value,
    source,
    detail,
    domain,
    exprText,
    calledBuiltin,
    builtinMeta
  });

  const builtinWindowExpr = extractNamedArgExprFromExpression(exprText, builtinMeta?.windowArgs || []);
  const windowExpr = (hints as any).windowExpr || builtinWindowExpr;
  const windowValue = evaluateNumericExpression(windowExpr, env, "time");
  if (Number.isFinite(windowValue) && windowValue > 0) {
    return result(windowValue, "window", `window=${windowExpr}`);
  }

  if (domain === "time") {
    const builtinPeriodExpr = extractNamedArgExprFromExpression(exprText, builtinMeta?.periodArgs || []);
    const periodExpr = (hints as any).periodExpr || builtinPeriodExpr;
    const period = evaluateNumericExpression(periodExpr, env, "time");
    if (Number.isFinite(period) && period > 0) {
      const clamped = Math.max(0.1, Math.min(period, 16.0));
      return result(clamped, "period", `period=${periodExpr}`);
    }

    const builtinFreqExpr = extractNamedArgExprFromExpression(exprText, builtinMeta?.frequencyArgs || []);
    const freqExpr = (hints as any).frequencyExpr || builtinFreqExpr;
    const freq = evaluateNumericExpression(freqExpr, env, "scalar");
    if (Number.isFinite(freq) && freq > 0) {
      const inferred = 1 / freq;
      const clamped = Math.max(0.1, Math.min(inferred, 16.0));
      return result(clamped, "frequency", `frequency=${freqExpr}`);
    }
  }

  if (domain === "x") {
    const builtinRange = extractRangeExprFromExpression(exprText, builtinMeta?.rangeArgs || []);
    const rangeStart = evaluateNumericExpression((hints as any).rangeStartExpr || builtinRange?.startExpr, env, "scalar");
    const rangeEnd = evaluateNumericExpression((hints as any).rangeEndExpr || builtinRange?.endExpr, env, "scalar");
    if (Number.isFinite(rangeStart) && Number.isFinite(rangeEnd)) {
      const span = Math.abs(rangeEnd - rangeStart);
      if (span > 0) {
        const clamped = Math.max(0.1, Math.min(span, 64.0));
        const startExpr = (hints as any).rangeStartExpr || builtinRange?.startExpr || "?";
        const endExpr = (hints as any).rangeEndExpr || builtinRange?.endExpr || "?";
        return result(clamped, "range", `range=${startExpr}..${endExpr}`);
      }
    }
  }

  if (Number.isFinite(base) && base > 0) {
    return result(base, "annotation", `annotation sweep=${base}`);
  }
  return result(fallback, "fallback", `fallback sweep=${fallback}`);
}

export function resolveAnnotationSweepMax(
  annotation: any,
  env: Record<string, number>,
  builtinVizSweepMetaByName: Map<string, BuiltinVizSweepMeta>
): number {
  return resolveAnnotationSweep(annotation, env, builtinVizSweepMetaByName).value;
}

export function buildShaderExplainText(
  annotation: any,
  semanticType: string,
  renderPath: string,
  sweepInfo: SweepInfo,
  wgsl: string,
  visualizerMeta: any = null
): string {
  const meta = sweepInfo?.builtinMeta || null;
  const roles: string[] = [];
  if (meta?.windowArgs?.length) {
    roles.push(`window args: ${meta.windowArgs.join(", ")}`);
  }
  if (meta?.periodArgs?.length) {
    roles.push(`period args: ${meta.periodArgs.join(", ")}`);
  }
  if (meta?.frequencyArgs?.length) {
    roles.push(`frequency args: ${meta.frequencyArgs.join(", ")}`);
  }
  if (meta?.rangeArgs?.length) {
    roles.push(`range args: ${meta.rangeArgs.join(", ")}`);
  }

  const entry = detectFrescoEntryFunction(String(wgsl || "")) || "unknown";
  const lines = [
    `annotation: ${annotation?.title || annotation?.name || "viz"}`,
    `line: ${annotation?.lineNumber || "?"} (${annotation?.directivePlacement || "trailing"} directive)`,
    `span: ${annotation?.spanStart ?? "?"}..${annotation?.spanEnd ?? "?"}`,
    `domain: ${visualizerMeta?.domain || annotation?.domain || "auto"}`,
    `semantic type: ${semanticType || "unknown"}`,
    `renderer path: ${visualizerMeta?.previewLabel || visualizerMeta?.kind || renderPath || "unknown"}`,
    `entry function: ${entry}`,
    `source expression: ${sweepInfo?.exprText || extractAnnotationExprText(annotation) || ""}`,
    `called builtin: ${sweepInfo?.calledBuiltin || "(none)"}`,
    `sweep: ${Number((visualizerMeta?.sweepMax ?? sweepInfo?.value) || 0).toFixed(3)} (${sweepInfo?.source || "fallback"}; ${sweepInfo?.detail || ""})`
  ];

  if (visualizerMeta?.xAxisLabel || visualizerMeta?.yAxisLabel || visualizerMeta?.fitMode) {
    lines.push(
      `visualizer metadata: x=${visualizerMeta?.xAxisLabel || "-"}, y=${visualizerMeta?.yAxisLabel || "-"}, fit=${visualizerMeta?.fitMode || "none"}`
    );
  }
  if (visualizerMeta?.previewDetail) {
    lines.push(`visualizer preview: ${visualizerMeta.previewDetail}`);
  }

  if (roles.length > 0) {
    lines.push("builtin metadata:");
    for (const roleLine of roles) {
      lines.push(`  - ${roleLine}`);
    }
  }

  return lines.join("\n");
}
