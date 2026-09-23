import { afterEach, describe, expect, it, vi } from "vitest";
import { ExampleEngineRenderer } from "../preview/example-engine-renderer";

afterEach(() => vi.unstubAllGlobals());

function createRenderer() {
  vi.stubGlobal("location", { search: "" });
  const renderer = new ExampleEngineRenderer({ dataset: {} }, {
    textureOptionsForName() { return { options: [], defaultUrl: null }; },
  });
  const values = {};
  renderer.engine = {
    async update_parameters(json) { Object.assign(values, JSON.parse(json)); return true; },
    parameter_values_json() { return JSON.stringify(values); },
  };
  return renderer;
}

describe("Rust adapter default edits", () => {
  it("resets param value to default when param definition changes", async () => {
    const renderer = createRenderer();

    const originalDef = {
      name: "weights",
      type: "array<f32, 2>",
      callType: "array<f32, 2>",
      default: { type: "f32", values: [0.2, 0.8] },
      min: null,
      max: null,
    };

    renderer.syncParamDefs([originalDef]);
    expect(renderer.paramValues.get("weights")).toEqual([0.2, 0.8]);

    renderer.setParamValue(originalDef, [0.4, 0.6]);
    await renderer.parameterUpdates;
    expect(renderer.paramValues.get("weights")).toEqual([0.4, 0.6]);

    renderer.syncParamDefs([{ ...originalDef }]);
    expect(renderer.paramValues.get("weights")).toEqual([0.4, 0.6]);

    renderer.syncParamDefs([
      {
        ...originalDef,
        default: { type: "f32", values: [0.9, 0.1] },
      }
    ]);
    expect(renderer.paramValues.get("weights")).toEqual([0.9, 0.1]);
  });

  it("reseeds untouched params from defaults but preserves touched values", async () => {
    const renderer = createRenderer();

    const def = {
      name: "values",
      type: "array<f32, 12>",
      callType: "array<f32, 12>",
      default: {
        type: "f32",
        values: [0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95],
      },
      min: null,
      max: null,
    };

    renderer.syncParamDefs([def]);
    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);

    // Simulate stale runtime state before user interaction: sync should reseed.
    renderer.paramValues.set("values", Array.from({ length: 12 }, () => 0));
    renderer.syncParamDefs([{ ...def }]);
    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);

    // After user edit, sync should preserve unless definition changes.
    renderer.setParamValue(def, [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
    await renderer.parameterUpdates;
    renderer.syncParamDefs([{ ...def }]);
    expect(renderer.paramValues.get("values")).toEqual([1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
  });
});
