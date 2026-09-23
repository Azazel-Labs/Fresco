import { describe, expect, it } from "vitest";
import { chooseEvaluationVariantEntry } from "../preview/evaluation-selection";

describe("explicit evaluator selection", () => {
  const evaluation_variants = [
    { entry: "small", bindings: [{ axis: "batch", value: "2" }] },
    { entry: "large", bindings: [{ axis: "batch", value: "128" }] },
  ];
  it("preserves the declared base and does not rank numeric axis values", () => {
    expect(chooseEvaluationVariantEntry({ evaluation_shader_entry: "base", evaluation_variants: [...evaluation_variants, { entry: "base" }] })).toBe("base");
    expect(chooseEvaluationVariantEntry({ evaluation_shader_entry: "base", evaluation_variants })).toBeNull();
  });
  it("uses arbitrary explicitly selected axes", () => {
    expect(chooseEvaluationVariantEntry({ evaluation_variants, settings: { evaluation_axes: { batch: "2" } } })).toBe("small");
  });
  it("does not invent a selection for missing or ambiguous matches", () => {
    expect(chooseEvaluationVariantEntry({ evaluation_variants, settings: { evaluation_axes: { batch: "3" } } })).toBeNull();
    expect(chooseEvaluationVariantEntry({ evaluation_variants: [...evaluation_variants, evaluation_variants[0]], settings: { evaluation_axes: { batch: "2" } } })).toBeNull();
  });
});
