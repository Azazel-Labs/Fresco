import { describe, expect, it } from "vitest";

import { detectFrescoEntryFunction } from "../wgsl-entry";

describe("WGSL entry detection", () => {
  it("prefers the registered canvas entry over generated scatter helpers", () => {
    const wgsl = `
struct CanvasContext {
  uv: vec2<f32>,
  time: f32,
}

fn fresco_night_sky_scatter_l3(
  p: vec2<f32>,
  px: f32,
  aa: f32,
  inst_pos_x: f32
) -> vec4<f32> {
  return vec4<f32>(0.0);
}

fn fresco_night_sky(ctx_1: CanvasContext) -> vec4<f32> {
  return vec4<f32>(0.0);
}
`;

    expect(detectFrescoEntryFunction(wgsl)).toBe("fresco_night_sky");
  });

  it("returns null when no Fresco function is emitted", () => {
    expect(detectFrescoEntryFunction("fn main() {}")).toBeNull();
  });
});
