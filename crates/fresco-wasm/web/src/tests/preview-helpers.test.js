import { describe, expect, it } from "vitest";

import {
  clamp01,
  createVizRuntimeParamUniformWriter,
  entryPrefixInfoFromTypes,
  hexToRgba,
  normalizeTypeName,
  rgbaToHex
} from "../preview/helpers";
import {
  createPreviewUniformData,
  PREVIEW_UNIFORM_PARAM_BASE
} from "../preview/uniforms";
import { buildReadableRuntimeParamAliases, sanitizeWgslIdentifier } from "../runtime-param-aliases";

describe("preview helper smoke coverage", () => {
  it("detects canvas entry prefixes for preview argument wiring", () => {
    expect(entryPrefixInfoFromTypes(["vec2<f32>", "f32", "vec2<f32>"])).toEqual({
      hasDelta: false,
      fixedCount: 3
    });
    expect(entryPrefixInfoFromTypes(["vec2<f32>", "f32", "f32", "vec2<f32>"])).toEqual({
      hasDelta: true,
      fixedCount: 4
    });
  });

  it("packs renderer params into preview uniform storage", () => {
    const uniform = createPreviewUniformData();
    const fakeRenderer = {
      paramDefs: [
        { name: "gain", type: "f32", slotOffset: 0 },
        { name: "enabled", type: "bool", slotOffset: 1 },
        { name: "tint", type: "vec4<f32>", slotOffset: 4 }
      ],
      paramValues: new Map([
        ["gain", 0.75],
        ["enabled", true],
        ["tint", [0.2, 0.4, 0.6, 0.8]]
      ]),
      normalizeParamValue(def, raw) {
        return raw ?? def.default ?? null;
      }
    };

    const writeUniforms = createVizRuntimeParamUniformWriter(() => fakeRenderer);
    writeUniforms(uniform);

    expect(uniform[PREVIEW_UNIFORM_PARAM_BASE]).toBeCloseTo(0.75);
    expect(uniform[PREVIEW_UNIFORM_PARAM_BASE + 1]).toBe(1);
    expect(uniform[PREVIEW_UNIFORM_PARAM_BASE + 4]).toBeCloseTo(0.2);
    expect(uniform[PREVIEW_UNIFORM_PARAM_BASE + 7]).toBeCloseTo(0.8);
  });

  it("normalizes shared color and type helpers", () => {
    expect(normalizeTypeName(" vec4<f32> ")).toBe("vec4<f32>");
    expect(clamp01(2)).toBe(1);
    expect(rgbaToHex([1, 0.5, 0])).toBe("#ff8000");
    expect(hexToRgba("#336699", 0.25)).toEqual([0.2, 0.4, 0.6, 0.25]);
  });

  it("builds readable runtime param aliases for preview wrapper WGSL", () => {
    const defs = [
      {
        name: "accent",
        reflectedGroup: [{ name: "accent_1", type: "vec4<f32>" }]
      },
      {
        name: "stroke_w",
        reflectedGroup: [{ name: "stroke_w_3", type: "f32" }]
      }
    ];

    expect(buildReadableRuntimeParamAliases(defs, ["sampleUv", "u"])).toEqual([
      ["accent"],
      ["stroke_w"]
    ]);
  });

  it("sanitizes and de-conflicts WGSL runtime param aliases", () => {
    expect(sanitizeWgslIdentifier("3 bad-name", "param1")).toBe("_3_bad_name");
    expect(
      buildReadableRuntimeParamAliases(
        [
          { name: "sampleUv", reflectedGroup: [{ name: "sampleUv", type: "f32" }] },
          { name: "fn", reflectedGroup: [{ name: "fn", type: "f32" }] },
          {
            name: "curve",
            reflectedGroup: [
              { name: "curve_x", type: "f32" },
              { name: "curve_y", type: "f32" }
            ]
          }
        ],
        ["sampleUv"]
      )
    ).toEqual([
      ["sampleUv_2"],
      ["fn_param"],
      ["curve_x", "curve_y"]
    ]);
  });
});
