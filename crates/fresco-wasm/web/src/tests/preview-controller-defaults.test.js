import { describe, expect, it } from "vitest";
import { PreviewController } from "../preview/preview-controller";

function createRenderer() {
  const canvas = { clientWidth: 800, clientHeight: 600, width: 800, height: 600 };
  return new PreviewController(canvas, {
    textureOptionsForName() { return { options: [], defaultUrl: null }; },
  });
}

describe("shared preview controller parameter normalization", () => {
  it("normalizes structured array defaults", () => {
    const renderer = createRenderer();

    expect(
      renderer.defaultValueForType("array<f32, 3>", { type: "f32", values: [0.1, 0.2, 0.3] })
    ).toEqual([0.1, 0.2, 0.3]);

    expect(
      renderer.defaultValueForType("array<vec2, 2>", {
        type: "vec2",
        values: [
          [1, 2],
          [3, 4],
        ]
      })
    ).toEqual([1, 2, 3, 4]);

    expect(
      renderer.defaultValueForType("array<f32, 12>", {
        type: "f32",
        values: [0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95],
      })
    ).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);

    expect(
      renderer.defaultValueForType("array<f32, 12>", [0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95])
    ).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);

    expect(
      renderer.defaultValueForType("array<f32, 12>", {
        type: "f32",
        values: [0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95],
      })
    ).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);

    const spacedTypeDef = {
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
    renderer.syncParamDefs([spacedTypeDef]);
    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);

    expect(
      renderer.defaultValueForType("array<f32, 12u>", {
        type: "f32",
        values: [0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95],
      })
    ).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);
  });

  it("normalizes structured scalar and color defaults", () => {
    const renderer = createRenderer();

    expect(renderer.defaultValueForType("i32", { value: 7.9 })).toBe(8);
    expect(renderer.defaultValueForType("u32", { scalar: -2 })).toBe(0);
    expect(renderer.defaultValueForType("bool", { bool: true })).toBe(true);

    expect(
      renderer.defaultValueForType("color", { r: 0.25, g: 0.5, b: 0.75, a: 1.0 })
    ).toEqual([0.25, 0.5, 0.75, 1]);
  });

  it("defaultValueForType returns empty array for unsized array types", () => {
    const renderer = createRenderer();
    expect(renderer.defaultValueForType("array<f32>")).toEqual([]);
    expect(renderer.defaultValueForType("array<f32>", [])).toEqual([]);
    expect(renderer.defaultValueForType("array<vec2<f32>>")).toEqual([]);
  });

  it("defaultValueForType returns provided fallback values for unsized arrays", () => {
    const renderer = createRenderer();
    expect(renderer.defaultValueForType("array<f32>", [0.1, 0.2, 0.3])).toEqual([0.1, 0.2, 0.3]);
  });

  it("normalizeParamValue handles dynamic array values", () => {
    const renderer = createRenderer();
    const def = { name: "weights", type: "array<f32>", default: [] };

    expect(renderer.normalizeParamValue(def, undefined)).toEqual([]);
    expect(renderer.normalizeParamValue(def, [])).toEqual([]);
    expect(renderer.normalizeParamValue(def, [0.1, 0.5, 0.9])).toEqual([0.1, 0.5, 0.9]);
  });

  it("normalizeParamValue seeds from default when value is missing", () => {
    const renderer = createRenderer();
    const def = { name: "weights", type: "array<f32>", default: [0.5, 0.7] };
    expect(renderer.normalizeParamValue(def, undefined)).toEqual([0.5, 0.7]);
  });
});
