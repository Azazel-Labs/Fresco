import { describe, expect, it } from "vitest";

import { detectFrescoEntryFunction } from "../wgsl-entry";

describe("WGSL entry detection", () => {
  it("prefers the canvas entry over generated scatter helpers", () => {
    const wgsl = `
fn fresco_night_sky_scatter_l3(
  p: vec2<f32>,
  px: f32,
  aa: f32,
  time: f32,
  delta: f32,
  res: vec2<f32>,
  inst_pos_x: f32
) -> vec4<f32> {
  return vec4<f32>(0.0);
}

fn fresco_night_sky(
  uv: vec2<f32>,
  time: f32,
  res: vec2<f32>
) -> vec4<f32> {
  return vec4<f32>(0.0);
}
`;

    expect(detectFrescoEntryFunction(wgsl)).toBe("fresco_night_sky");
  });
});
