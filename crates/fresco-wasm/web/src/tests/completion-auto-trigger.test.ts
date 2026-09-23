import { describe, it, expect } from "vitest";
import { shouldAutoSuggest } from "../completion-auto-trigger";
describe("contextual suggestion triggers", () => {
  it("reopens on argument whitespace, newlines, and member access", () => {
    for (const source of ["chase(", "chase( ", "chase(\n  ", "chase(along: ", "chase(along: outer,\n ", "outer."])
      expect(shouldAutoSuggest(source), source).toBe(true);
  });
  it("stays closed in comments, strings, and ordinary whitespace", () => {
    for (const source of ['chase("along: ', "// chase( ", "/* chase( ", "let outer = 1 ", "chase(along: outer) "])
      expect(shouldAutoSuggest(source), source).toBe(false);
  });
  it("reopens after deletion exposes an empty expression slot", () => {
    for (const source of ["tile.", "chase(along: ", "chase(", "let value = "])
      expect(shouldAutoSuggest(source, true), source).toBe(true);
  });
  it("does not reopen for deletion in ordinary text, comments, or strings", () => {
    for (const source of ["let val", "tile.dis", "\n", "value == ", "value != ", "// tile.", "/* let x = ", 'chase("tile.'])
      expect(shouldAutoSuggest(source, true), source).toBe(false);
    expect(shouldAutoSuggest("let value = ")).toBe(false);
  });
});
