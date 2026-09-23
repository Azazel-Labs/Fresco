import { describe, expect, it } from "vitest";

import { createShaderParamViewModel } from "../app/shader-param-view-model";

function createFakeRenderer() {
  const paramValues = new Map();
  return {
    paramValues,
    normalizeParamValue(def, value) {
      if (Array.isArray(value)) {
        return value.map((entry) => Number(entry));
      }
      const fallback = def?.default;
      if (Array.isArray(fallback)) {
        return fallback.map((entry) => Number(entry));
      }
      return value;
    },
    setParamValue(def, value) {
      const normalized = this.normalizeParamValue(def, value);
      this.paramValues.set(def.name, normalized);
    },
  };
}

describe("shader param view model", () => {
  it("returns default array values for UI reads when runtime value is missing", () => {
    const renderer = createFakeRenderer();
    const viewModel = createShaderParamViewModel(renderer);
    const def = {
      name: "values",
      type: "array<f32, 4>",
      default: [0.3, 0.5, 0.42, 0.61],
    };

    const values = viewModel.getArrayValue(def, 4);

    expect(values).toEqual([0.3, 0.5, 0.42, 0.61]);
    expect(renderer.paramValues.get("values")).toBeUndefined();
  });

  it("can explicitly seed defaults into renderer state", () => {
    const renderer = createFakeRenderer();
    const viewModel = createShaderParamViewModel(renderer);
    const def = {
      name: "values",
      type: "array<f32, 4>",
      default: [0.3, 0.5, 0.42, 0.61],
    };

    const seeded = viewModel.seedFromDefault(def);

    expect(seeded).toEqual([0.3, 0.5, 0.42, 0.61]);
    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.5, 0.42, 0.61]);
  });

  it("reseeds from defaults when an existing array is too short", () => {
    const renderer = createFakeRenderer();
    renderer.paramValues.set("values", [0, 0]);
    const viewModel = createShaderParamViewModel(renderer);
    const def = {
      name: "values",
      type: "array<f32, 4>",
      default: [0.3, 0.5, 0.42, 0.61],
    };

    const values = viewModel.getArrayValue(def, 4);

    expect(values).toEqual([0.3, 0.5, 0.42, 0.61]);
    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.5, 0.42, 0.61]);
  });

  it("preserves a complete existing array value", () => {
    const renderer = createFakeRenderer();
    renderer.paramValues.set("values", [0.11, 0.22, 0.33, 0.44]);
    const viewModel = createShaderParamViewModel(renderer);
    const def = {
      name: "values",
      type: "array<f32, 4>",
      default: [0.3, 0.5, 0.42, 0.61],
    };

    const values = viewModel.getArrayValue(def, 4);

    expect(values).toEqual([0.11, 0.22, 0.33, 0.44]);
    expect(renderer.paramValues.get("values")).toEqual([0.11, 0.22, 0.33, 0.44]);
  });
});
