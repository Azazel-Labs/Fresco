/** Reopen contextual suggestions at empty slots, excluding comments and strings. */
export function shouldAutoSuggest(prefix: string, deleting = false): boolean {
  let code = "", state = "code", depth = 0;
  for (let i = 0; i < prefix.length; i++) {
    const c = prefix[i], next = prefix[i + 1];
    if (state === "line") { if (c === "\n") { state = "code"; code += c; } continue; }
    if (state === "block") { if (c === "*" && next === "/") { state = "code"; i++; } continue; }
    if (state === "string") { if (c === "\\") i++; else if (c === '"') state = "code"; continue; }
    if (c === "/" && next === "/") { state = "line"; i++; continue; }
    if (c === "/" && next === "*") { state = "block"; i++; continue; }
    if (c === '"') { state = "string"; continue; }
    code += c;
    if (c === "(") depth++; else if (c === ")") depth = Math.max(0, depth - 1);
  }
  if (state !== "code") return false;
  return /[.]\s*$/.test(code) || (depth > 0 && /[(:,]\s*$/.test(code))
    || (deleting && /(?:^|[^=!<>])=\s*$/.test(code));
}
