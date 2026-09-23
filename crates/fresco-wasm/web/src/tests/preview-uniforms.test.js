import { describe, expect, it } from "vitest";

import {
  createPreviewUniformData,
  MAX_RUNTIME_PARAMS,
  PREVIEW_UNIFORM_DT_INDEX,
  PREVIEW_UNIFORM_FLOATS,
  PREVIEW_UNIFORM_PARAM_BASE,
  PREVIEW_UNIFORM_PARAM_VECS,
  PREVIEW_UNIFORM_RES_INDEX,
  PREVIEW_UNIFORM_TIME_INDEX,
  previewUniformParamsFieldWgsl,
  uniformParamExpression,
  uniformVec4Expression
} from "../preview/uniforms";

describe("preview uniform helpers", () => {
  it("packs runtime params into vec4-backed uniform storage", () => {
    expect(previewUniformParamsFieldWgsl()).toBe(`array<vec4<f32>, ${PREVIEW_UNIFORM_PARAM_VECS}>`);
    expect(uniformParamExpression(0)).toBe("u.params[0][0]");
    expect(uniformParamExpression(3)).toBe("u.params[0][3]");
    expect(uniformParamExpression(4)).toBe("u.params[1][0]");
    expect(uniformParamExpression(MAX_RUNTIME_PARAMS - 1)).toBe("u.params[15][3]");
    expect(uniformVec4Expression(0)).toBe("vec4<f32>(u.params[0][0], u.params[0][1], u.params[0][2], u.params[0][3])");
  });

  it("allocates uniform storage that matches the shader layout", () => {
    const uniform = createPreviewUniformData();

    expect(uniform).toBeInstanceOf(Float32Array);
    expect(uniform.length).toBe(PREVIEW_UNIFORM_FLOATS);
    expect(PREVIEW_UNIFORM_TIME_INDEX).toBe(0);
    expect(PREVIEW_UNIFORM_DT_INDEX).toBe(1);
    expect(PREVIEW_UNIFORM_RES_INDEX).toBe(8);
    expect(PREVIEW_UNIFORM_PARAM_BASE).toBe(12);
    expect(PREVIEW_UNIFORM_FLOATS).toBe(PREVIEW_UNIFORM_PARAM_BASE + MAX_RUNTIME_PARAMS);
    expect(PREVIEW_UNIFORM_FLOATS * Float32Array.BYTES_PER_ELEMENT).toBe(304);
  });
});
