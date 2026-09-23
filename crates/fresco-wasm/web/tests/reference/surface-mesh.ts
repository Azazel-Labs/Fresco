// Independent CPU test reference; excluded from production.
/** Vertex layout: position (vec3), normal (vec3), tangent (vec3), uv (vec2), uv2 (vec2) — 13 floats per vertex. */
export const SURFACE_VERTEX_FLOATS = 13;
export const SURFACE_VERTEX_STRIDE = SURFACE_VERTEX_FLOATS * 4;

export const DEFAULT_SURFACE_SPHERE_RINGS = 64;
export const DEFAULT_SURFACE_SPHERE_SEGMENTS = 64;
export const DEFAULT_SURFACE_PLANE_SUBDIVISIONS = 16;

export type SurfaceMeshData = {
  vertices: Float32Array;
  indices: Uint32Array;
};

/** UV sphere with `rings` latitude bands and `segments` longitude columns. */
export function generateSphere(
  rings = DEFAULT_SURFACE_SPHERE_RINGS,
  segments = DEFAULT_SURFACE_SPHERE_SEGMENTS,
): SurfaceMeshData {
  const vertices: number[] = [];
  const indices: number[] = [];

  const normalize3 = (x: number, y: number, z: number): [number, number, number] => {
    const len = Math.hypot(x, y, z);
    return len > 0 ? [x / len, y / len, z / len] : [0, 0, 0];
  };

  for (let r = 0; r <= rings; r++) {
    const phi = (Math.PI * r) / rings;
    const sinPhi = Math.sin(phi);
    const cosPhi = Math.cos(phi);
    for (let s = 0; s <= segments; s++) {
      const theta = (2 * Math.PI * s) / segments;
      const sinTheta = Math.sin(theta);
      const cosTheta = Math.cos(theta);
      const x = cosTheta * sinPhi;
      const y = cosPhi;
      const z = sinTheta * sinPhi;
      const tangent = sinPhi < 1e-6 ? [1, 0, 0] : normalize3(-sinTheta, 0, cosTheta);
      const u = s / segments;
      const v = r / rings;
      // uv2 defaults to a second projection stream so `sp.uv` and `sp.uv2` can differ in preview.
      const u2 = x * 0.5 + 0.5;
      const v2 = z * 0.5 + 0.5;
      vertices.push(x, y, z, x, y, z, tangent[0], tangent[1], tangent[2], u, v, u2, v2);
    }
  }

  for (let r = 0; r < rings; r++) {
    for (let s = 0; s < segments; s++) {
      const a = r * (segments + 1) + s;
      const b = a + (segments + 1);
      // Use CCW winding for outward-facing triangles.
      indices.push(a, a + 1, b);
      indices.push(a + 1, b + 1, b);
    }
  }

  return {
    vertices: new Float32Array(vertices),
    indices: new Uint32Array(indices),
  };
}

/** Subdivided plane in XZ, facing +Y, UV from (0,0) to (1,1). */
export function generatePlane(subdivisions = DEFAULT_SURFACE_PLANE_SUBDIVISIONS): SurfaceMeshData {
  const vertices: number[] = [];
  const indices: number[] = [];
  const n = Math.max(1, subdivisions);

  for (let row = 0; row <= n; row++) {
    for (let col = 0; col <= n; col++) {
      const u = col / n;
      const v = row / n;
      const x = u * 2 - 1;
      const z = v * 2 - 1;
      const u2 = u * 2;
      const v2 = v * 2;
      vertices.push(x, 0, z, 0, 1, 0, 1, 0, 0, u, v, u2, v2);
    }
  }

  for (let row = 0; row < n; row++) {
    for (let col = 0; col < n; col++) {
      const a = row * (n + 1) + col;
      const b = a + (n + 1);
      // Use CCW winding for +Y-facing plane triangles.
      indices.push(a, b, a + 1);
      indices.push(a + 1, b, b + 1);
    }
  }

  return {
    vertices: new Float32Array(vertices),
    indices: new Uint32Array(indices),
  };
}

/** Axis-aligned box, 6 separate faces for clean normals/UVs. */
export function generateBox(): SurfaceMeshData {
  // Each face: 4 verts (quad), 2 triangles
  // pos(3), normal(3), tangent(3), uv(2), uv2(2)
  const faces: Array<{
    normal: [number, number, number];
    right: [number, number, number];
    up: [number, number, number];
  }> = [
    { normal: [0, 0, 1], right: [1, 0, 0], up: [0, 1, 0] },   // +Z front
    { normal: [0, 0, -1], right: [-1, 0, 0], up: [0, 1, 0] },  // -Z back
    { normal: [1, 0, 0], right: [0, 0, -1], up: [0, 1, 0] },   // +X right
    { normal: [-1, 0, 0], right: [0, 0, 1], up: [0, 1, 0] },   // -X left
    { normal: [0, 1, 0], right: [1, 0, 0], up: [0, 0, -1] },   // +Y top
    { normal: [0, -1, 0], right: [1, 0, 0], up: [0, 0, 1] },   // -Y bottom
  ];

  const vertices: number[] = [];
  const indices: number[] = [];

  for (const face of faces) {
    const base = vertices.length / SURFACE_VERTEX_FLOATS;
    const [nx, ny, nz] = face.normal;
    const [rx, ry, rz] = face.right;
    const [ux, uy, uz] = face.up;
    // 4 corners: center + ±right ±up
    for (const [su, sv] of [[-1, -1], [1, -1], [1, 1], [-1, 1]] as [number, number][]) {
      const px = nx + rx * su + ux * sv;
      const py = ny + ry * su + uy * sv;
      const pz = nz + rz * su + uz * sv;
      const u = (su + 1) / 2;
      const v = (sv + 1) / 2;
      const u2 = u * 2;
      const v2 = v * 2;
      vertices.push(px, py, pz, nx, ny, nz, rx, ry, rz, u, v, u2, v2);
    }
    indices.push(base, base + 1, base + 2, base, base + 2, base + 3);
  }

  return {
    vertices: new Float32Array(vertices),
    indices: new Uint32Array(indices),
  };
}
