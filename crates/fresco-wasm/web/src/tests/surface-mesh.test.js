import { describe, expect, it } from "vitest";
import {
  DEFAULT_SURFACE_PLANE_SUBDIVISIONS,
  DEFAULT_SURFACE_SPHERE_RINGS,
  DEFAULT_SURFACE_SPHERE_SEGMENTS,
  generatePlane,
  generateSphere,
  SURFACE_VERTEX_FLOATS,
} from "../../tests/reference/surface-mesh";

describe("surface mesh defaults", () => {
  it("uses a denser default sphere mesh", () => {
    const mesh = generateSphere();
    expect(DEFAULT_SURFACE_SPHERE_RINGS).toBe(64);
    expect(DEFAULT_SURFACE_SPHERE_SEGMENTS).toBe(64);
    expect(mesh.vertices.length).toBe((64 + 1) * (64 + 1) * 13);
    expect(mesh.indices.length).toBe(64 * 64 * 6);
  });

  it("uses a denser default plane mesh", () => {
    const mesh = generatePlane();
    expect(DEFAULT_SURFACE_PLANE_SUBDIVISIONS).toBe(16);
    expect(mesh.vertices.length).toBe((16 + 1) * (16 + 1) * 13);
    expect(mesh.indices.length).toBe(16 * 16 * 6);
  });
});

it.each([[4, 8], [64, 64]])("sphere winding faces outward (%i rings, %i segments)", (rings, segments) => {
  const { vertices, indices } = generateSphere(rings, segments);
  let nondegenerateTriangles = 0;
  for (let i = 0; i < indices.length; i += 3) {
    const [a, b, c] = Array.from(indices.slice(i, i + 3), index => {
      const offset = index * SURFACE_VERTEX_FLOATS;
      return Array.from(vertices.slice(offset, offset + 3));
    });
    const ab = b.map((value, axis) => value - a[axis]);
    const ac = c.map((value, axis) => value - a[axis]);
    const normal = [
      ab[1] * ac[2] - ab[2] * ac[1],
      ab[2] * ac[0] - ab[0] * ac[2],
      ab[0] * ac[1] - ab[1] * ac[0],
    ];
    // The latitude grid duplicates pole vertices, producing zero-area triangles.
    if (Math.hypot(...normal) < 1e-10) continue;
    nondegenerateTriangles++;
    const center = a.map((value, axis) => (value + b[axis] + c[axis]) / 3);
    expect(normal.reduce((dot, value, axis) => dot + value * center[axis], 0)).toBeGreaterThan(0);
  }
  expect(nondegenerateTriangles).toBe(2 * segments * (rings - 1));
});

it("plane winding agrees with its authored positive-Y normal", () => {
  const {vertices, indices} = generatePlane(2);
  for (let i = 0; i < indices.length; i += 3) {
    const [a,b,c] = Array.from(indices.slice(i,i+3)).map(index => Array.from(vertices.slice(index*13,index*13+3)));
    const ab = b.map((v,i) => v-a[i]); const ac = c.map((v,i) => v-a[i]);
    expect(ab[2]*ac[0] - ab[0]*ac[2]).toBeGreaterThan(0);
  }
});
