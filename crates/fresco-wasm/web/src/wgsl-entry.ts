function isGeneratedHelperFunctionName(name: unknown): boolean {
  return /_scatter_l\d+$/.test(String(name || ""));
}

/**
 * Find the emitted Fresco entry function when no manifest names it.
 * Registered entries take their engine context record as the first
 * parameter, so detection only needs to skip generated scatter helpers.
 */
export function detectFrescoEntryFunction(wgsl: unknown): string | null {
  const fnRx = /fn\s+(fresco_[A-Za-z0-9_]+)\s*\(/g;
  const names = Array.from(String(wgsl || "").matchAll(fnRx), (match) => match[1]);
  return names.find((name) => !isGeneratedHelperFunctionName(name)) ?? names[0] ?? null;
}
