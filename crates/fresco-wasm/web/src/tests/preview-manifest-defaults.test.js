import { describe, expect, it } from "vitest";

import { PreviewController } from "../preview/preview-controller";
import { parseManifestParams } from "../preview/renderer-manifest";
import { storageBufferByteSize } from "../preview/param-codegen";

function createRenderer() {
  const canvas = { clientWidth: 800, clientHeight: 600, width: 800, height: 600 };
  return new PreviewController(canvas, {
    textureOptionsForName() {
      return { options: [], defaultUrl: null };
    }
  });
}

describe("preview metadata manifest defaults", () => {
  it("seeds manifest defaults for fixed arrays", () => {
    const renderer = createRenderer();

    const wgsl = `
fn fresco_bar_chart(
  sampleUv: vec2<f32>,
  _fu_time: f32,
  _fu_res: vec2<f32>,
  values: array<f32, 12u>,
  accent: vec4<f32>
) -> vec4<f32> {
  return vec4<f32>(sampleUv, _fu_time, 1.0);
}
`;

    const manifest = {
      canvases: [
        {
          name: "bar_chart",
          params: [
            {
              name: "values",
              type: "array<f32, 12>",
              default: {
                type: "f32",
                values: [0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95],
              },
              min: 0,
              max: 1,
            },
            {
              name: "accent",
              type: "color",
              default: [0.36, 0.91, 0.83, 1],
              min: null,
              max: null,
            },
          ],
          storage_params: [],
          textures: [],
          sampler: null,
          path_buffers: [],
          pass_plan: {
            passes: [],
            edges: [],
            targets: [],
          },
        },
      ],
    };

    const defs = parseManifestParams(manifest, "fresco_bar_chart");
    renderer.syncParamDefs(defs);

    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);
  });

  it("normalizes map-like manifest defaults for array params", () => {
    const renderer = createRenderer();

    const wgsl = `
fn fresco_bar_chart(
  sampleUv: vec2<f32>,
  _fu_time: f32,
  _fu_res: vec2<f32>,
  values: array<f32, 12u>
) -> vec4<f32> {
  return vec4<f32>(sampleUv, _fu_time, 1.0);
}
`;

    const manifest = {
      canvases: [
        {
          name: "bar_chart",
          params: [
            {
              name: "values",
              type: "array<f32, 12>",
              default: new Map([
                ["type", "f32"],
                ["values", [0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]],
              ]),
              min: 0,
              max: 1,
            },
          ],
          storage_params: [],
          textures: [],
          sampler: null,
          path_buffers: [],
          pass_plan: {
            passes: [],
            edges: [],
            targets: [],
          },
        },
      ],
    };

    const defs = parseManifestParams(manifest, "fresco_bar_chart");
    renderer.syncParamDefs(defs);

    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.5, 0.42, 0.61, 0.55, 0.7, 0.66, 0.8, 0.74, 0.9, 0.85, 0.95]);
  });

  it("snaps manifest float artifacts to conceptual decimal defaults", () => {
    const renderer = createRenderer();

    const wgsl = `
fn fresco_bar_chart(
  sampleUv: vec2<f32>,
  _fu_time: f32,
  _fu_res: vec2<f32>,
  values: array<f32, 3u>
) -> vec4<f32> {
  return vec4<f32>(sampleUv, _fu_time, 1.0);
}
`;

    const manifest = {
      canvases: [
        {
          name: "bar_chart",
          params: [
            {
              name: "values",
              type: "array<f32, 3>",
              default: {
                type: "f32",
                values: [0.30000001192092896, 0.41999998688697815, 0.949999988079071],
              },
              min: 0,
              max: 1,
            },
          ],
          storage_params: [],
          textures: [],
          sampler: null,
          path_buffers: [],
          pass_plan: {
            passes: [],
            edges: [],
            targets: [],
          },
        },
      ],
    };

    const defs = parseManifestParams(manifest, "fresco_bar_chart");
    renderer.syncParamDefs(defs);

    expect(renderer.paramValues.get("values")).toEqual([0.3, 0.42, 0.95]);
  });
});

describe("preview metadata dynamic array storage", () => {
  it("retains dynamic array parameter types", () => {
    const renderer = createRenderer();

    const wgsl = `
fn fresco_TestDynamicArray(uv: vec2<f32>, time: f32, res: vec2<f32>) -> vec4<f32> {
  return vec4<f32>(uv, time, 1.0);
}
`;

    const manifest = {
      canvases: [
        {
          name: "TestDynamicArray",
          params: [
            {
              name: "weights",
              type: "array<f32>",
              default: { type: "f32", values: [] },
              min: null,
              max: null,
            },
          ],
          storage_params: [
            { name: "weights", type: "array<f32>", group: 0, binding: 0 },
          ],
          textures: [],
          sampler: null,
          path_buffers: [],
          pass_plan: { passes: [], edges: [], targets: [] },
        },
      ],
    };

    const defs = parseManifestParams(manifest, "fresco_TestDynamicArray");
    expect(defs).toHaveLength(1);
    expect(defs[0].type).toBe("array<f32>");
  });

  it("handles mixed uniform and dynamic array params", () => {
    const renderer = createRenderer();

    const wgsl = `
fn fresco_Mixed(
  uv: vec2<f32>,
  time: f32,
  res: vec2<f32>,
  intensity: f32
) -> vec4<f32> {
  return vec4<f32>(uv, time, intensity);
}
`;

    const manifest = {
      canvases: [
        {
          name: "Mixed",
          params: [
            {
              name: "weights",
              type: "array<f32>",
              default: { type: "f32", values: [] },
              min: null,
              max: null,
            },
            {
              name: "intensity",
              type: "f32",
              default: 0.5,
              min: null,
              max: null,
            },
          ],
          storage_params: [
            { name: "weights", type: "array<f32>", group: 0, binding: 0 },
          ],
          textures: [],
          sampler: null,
          path_buffers: [],
          pass_plan: { passes: [], edges: [], targets: [] },
        },
      ],
    };

    const defs = parseManifestParams(manifest, "fresco_Mixed");
    renderer.syncParamDefs(defs);

    const weightsDef = defs.find((d) => d.name === "weights");
    const intensityDef = defs.find((d) => d.name === "intensity");

    expect(weightsDef.type).toBe("array<f32>");
    expect(intensityDef.type).toBe("f32");

    expect(renderer.paramValues.get("weights")).toEqual([]);
    expect(renderer.paramValues.get("intensity")).toBeCloseTo(0.5);
  });

  it("storageBufferByteSize always allocates at least one element stride for empty arrays", () => {
    // f32: stride = 4 bytes → arrayLength = floor(4/4) = 1
    expect(storageBufferByteSize("f32", [])).toBe(4);

    // vec2<f32>: stride = 8 bytes → arrayLength = floor(8/8) = 1
    expect(storageBufferByteSize("vec2<f32>", [])).toBe(8);

    // vec4<f32>: stride = 16 bytes → arrayLength = floor(16/16) = 1
    expect(storageBufferByteSize("vec4<f32>", [])).toBe(16);

    // color (treated as vec4): stride = 16 bytes
    expect(storageBufferByteSize("color", [])).toBe(16);

    // mat4x4<f32>: 16 components × 4 bytes = 64-byte stride
    expect(storageBufferByteSize("mat4x4<f32>", [])).toBe(64);

    // Non-empty: size reflects actual element count, not the minimum.
    // 2 × vec2 elements = 4 scalars = 4 × 4 = 16 bytes
    expect(storageBufferByteSize("vec2<f32>", [1, 2, 3, 4])).toBe(16);
  });

});
