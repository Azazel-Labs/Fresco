export const MAX_RUNTIME_PARAMS = 64;
export const PREVIEW_UNIFORM_TIME_INDEX = 0;
export const PREVIEW_UNIFORM_DT_INDEX = 1;
export const PREVIEW_UNIFORM_RES_INDEX = 8;
export const PREVIEW_UNIFORM_PARAM_BASE = 12;
export const PREVIEW_UNIFORM_PARAM_VECS = Math.ceil(MAX_RUNTIME_PARAMS / 4);
export const PREVIEW_UNIFORM_FLOATS = PREVIEW_UNIFORM_PARAM_BASE + (PREVIEW_UNIFORM_PARAM_VECS * 4);

export function createPreviewUniformData(): Float32Array {
  return new Float32Array(PREVIEW_UNIFORM_FLOATS);
}

export function uniformParamExpression(index: number): string {
  const vecIndex = Math.floor(index / 4);
  const componentIndex = index % 4;
  return `u.params[${vecIndex}][${componentIndex}]`;
}

export function uniformVec4Expression(index: number): string {
  return `vec4<f32>(${uniformParamExpression(index)}, ${uniformParamExpression(index + 1)}, ${uniformParamExpression(index + 2)}, ${uniformParamExpression(index + 3)})`;
}

export function previewUniformParamsFieldWgsl(): string {
  return `array<vec4<f32>, ${PREVIEW_UNIFORM_PARAM_VECS}>`;
}
