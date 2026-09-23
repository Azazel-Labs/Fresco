import { afterEach, describe, expect, it, vi } from "vitest";
import { chooseEvaluationVariantEntry } from "../preview/evaluation-selection";
import { ExampleEngineRenderer } from "../preview/example-engine-renderer";
afterEach(() => vi.unstubAllGlobals());
function createAdapter() {
  vi.stubGlobal("location", { search: "" });
  const renderer = new ExampleEngineRenderer({ dataset: {}, style: {} }, {
    textureOptionsForName: () => ({ options: [], defaultUrl: "" }),
  });
  renderer.gpuReady = true;
  renderer.engine = { install_with_assets: vi.fn().mockResolvedValue(true),
    buffer_views: () => "[]", set_buffer_view: vi.fn(), supports_lighting_environment: () => true, set_lighting_environment: vi.fn(), cancel_pending: vi.fn(), parameter_values_json: () => "{}", render_async: vi.fn().mockResolvedValue(true) };
  return renderer;
}
async function expectSelectedLightingEntryFromManifest({ sourceName, expectedEntry, variants }) {
  expect(chooseEvaluationVariantEntry({ name: `${sourceName}_surface`, material_ty: "pbr",
    evaluation_shader_entry: `fresco_evaluation_shader_${sourceName}_surface`,
    evaluation_contract: { inputs: ["lighting_ctx"], runtime: ["camera"] },
    evaluation_variants: variants,
  })).toBe(expectedEntry);
}
function executableMeshFixture(manifest, wgsl) {
  const surfaces = manifest.surfaces.map(surface => {
    if (surface.mesh_passes?.length) return surface;
    const vertex = `test_${surface.name}_vertex`;
    const fragment = `test_${surface.name}_fragment`;
    wgsl += `\n@vertex fn ${vertex}() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }
@fragment fn ${fragment}() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }`;
    return { ...surface, mesh_passes: [{
      pass: "test_mesh", factory: "test_factory",
      entries: [
        { stage: "vertex", function: "vertex", entry: vertex, outputs: [] },
        { stage: "fragment", function: "fragment", entry: fragment, outputs: [{ location: 0, name: "color", ty: "vec4" }] },
      ],
      variants: [],
    }] };
  });
  return { wgsl, manifest: { ...manifest, surfaces, vertex_factories: [
    ...(manifest.vertex_factories || []),
    { name: "test_factory", array_stride: 52, attributes: [], bindings: [
      { name: "scene", group_index: 2, binding: 0, signature: "uniform<PreviewScene>" },
    ] },
  ] } };
}

describe("surface artifact forwarding and inspector metadata", () => {
  it("submits compiler stages unchanged without synthesizing a lighting wrapper", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [{
        name: "crate_side",
        material_ty: "pbr",
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
          ],
        },
        params: [],
        textures: [],
        sampler: null,
      }],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
}

fn fresco_crate_side(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(1.0), 0.25, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(shaderCode).toBe(executable.wgsl);
    expect(shaderCode).not.toContain("let base_color = mat.albedo;");
    expect(shaderCode).not.toContain("let N = normalize(in.world_normal);");
    expect(shaderCode).not.toContain("fn fresco_brdf(");
  });

  it("does not synthesize tangent-space normal-map logic", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [{
        name: "crate_side",
        material_ty: "pbr",
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
          ],
        },
        params: [],
        textures: [
          {
            name: "wood_normal",
            group: 1,
            binding: 1,
            metadata: { default_asset: "", texture_type: "NormalGL" },
          },
        ],
        sampler: { group: 1, binding: 0 },
      }],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
}

@group(1) @binding(0)
var fresco_sampler: sampler;
@group(1) @binding(1)
var wood_normal: texture_2d<f32>;

fn fresco_crate_side(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(1.0), 0.25, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(shaderCode).toBe(executable.wgsl);
    expect(shaderCode).not.toContain("struct VertOut");
    expect(shaderCode).not.toContain("textureSample(t_wood_normal, fresco_sampler, in.uv)");
    expect(shaderCode).not.toContain("let tangent = normalize(in.world_tangent - base_normal * dot(base_normal, in.world_tangent));");
    expect(shaderCode).not.toContain("let N = normalize(");
  });

  it("uses a manifest-selected Fresco lighting entry when present", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [{
        name: "crate_side",
        material_ty: "pbr",
        evaluation_shader_entry: "fresco_lighting_emissive_only",
        evaluation_variants: [{ entry: "fresco_lighting_emissive_only" }],
        evaluation_contract: {
          inputs: ["light_strength"],
          runtime: ["shadow_bias"],
        },
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
          ],
        },
        params: [],
        textures: [],
        sampler: null,
      }],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
}

fn fresco_crate_side(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv, uv2.x, 1.0), 0.5, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(chooseEvaluationVariantEntry(manifest.surfaces.find(surface => surface.name === renderer.installed.entry))).toBe("fresco_lighting_emissive_only");
    expect(shaderCode).toBe(executable.wgsl);
    expect(shaderCode).not.toContain("let light_strength = 1.0;");
    expect(shaderCode).not.toContain("let shadow_bias = 1.0;");
    expect(shaderCode).not.toContain("fn fresco_brdf(");
  });

  it("does not invent values for manifest lighting runtime bindings", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [{
        name: "crate_side",
        material_ty: "pbr",
        evaluation_shader_entry: "fresco_lighting_simple_forward_directional_2",
        evaluation_contract: {
          inputs: ["lighting_ctx", "shading_ctx", "light_sample"],
          runtime: ["light_positions", "light_colors", "light_ranges", "light_types"],
        },
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
          ],
        },
        params: [],
        textures: [],
        sampler: null,
      }],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
}

fn fresco_crate_side(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(1.0), 0.25, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(shaderCode).toBe(executable.wgsl);
    expect(shaderCode).not.toContain("let lighting_ctx = vec3<f32>(0.25, 0.5, 0.75);");
    expect(shaderCode).not.toContain("let light_positions = vec3<f32>(0.7, 1.0, 0.6);");
    expect(shaderCode).not.toContain("let light_colors = vec3<f32>(0.8, 0.9, 1.0);");
    expect(shaderCode).not.toContain("let light_sample = 0.35;");
    expect(shaderCode).not.toContain("let light_positions = 1.0;");
  });

  it("uses manifest render_policy over material_ty for runtime surface selection", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [{
        name: "crate_side",
        material_ty: "pbr",
        render_policy: "unlit",
        schema_evaluator: "emissive_only",
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
          ],
        },
        params: [],
        textures: [],
        sampler: null,
      }],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
}

fn fresco_crate_side(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv, uv2.x, 1.0), 0.5, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(shaderCode).not.toContain("let base_color = mat.albedo;");
    expect(shaderCode).not.toContain("fn fresco_brdf(");
  });

  it("selects a specific surface by host resolver when manifest has multiple surfaces", async () => {
    const renderer = createAdapter();
    renderer.setActiveSurfaceName("mat_b");
    const manifest = {
      canvases: [],
      surfaces: [
        {
          name: "mat_a",
          material_ty: "pbr",
          surface_requirements: {
            uv_channels: [
              { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
              { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
            ],
          },
          params: [],
          textures: [],
          sampler: null,
        },
        {
          name: "mat_b",
          material_ty: "unlit",
          surface_requirements: {
            uv_channels: [
              { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
              { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
            ],
          },
          params: [],
          textures: [],
          sampler: null,
        },
      ],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
  custom0: vec4<f32>,
  custom1: vec4<f32>,
  custom2: vec4<f32>,
  custom3: vec4<f32>,
  custom4: vec4<f32>,
  custom5: vec4<f32>,
  custom6: vec4<f32>,
  custom7: vec4<f32>,
}

fn fresco_mat_a(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv, uv2.x, 1.0), 0.5, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0,
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0),
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

fn fresco_mat_b(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv2, uv.x, 1.0), 0.25, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0,
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0),
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(renderer.engine.install_with_assets.mock.calls[0][2]).toBe("mat_b");
    expect(manifest.surfaces.find(surface => surface.name === renderer.installed.entry)?.name).toBe("mat_b");
  });

  it("prefers the surface whose material has a lighting-tagged pipeline when no surface is selected", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [
        {
          name: "plain_surface",
          material_ty: "unlit",
          surface_requirements: {
            uv_channels: [
              { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
              { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
            ],
          },
          params: [],
          textures: [],
          sampler: null,
        },
        {
          name: "lit_surface",
          material_ty: "pbr",
          surface_requirements: {
            uv_channels: [
              { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
              { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
            ],
          },
          params: [],
          textures: [],
          sampler: null,
        },
      ],
      pipelines: [
        { name: "forward", material: "pbr", type: "lighting", passes: ["fwd_base", "fwd_add"] },
      ],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
}

fn fresco_plain_surface(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv, uv2.x, 1.0), 0.5, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0);
}

fn fresco_lit_surface(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv2, uv.x, 1.0), 0.25, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    expect(manifest.surfaces.find(surface => surface.name === renderer.installed.entry)?.name).toBe("lit_surface");
  });

  it("selects forward-plus or clustered authored lighting variant from manifest metadata", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [{
        name: "clustered_surface",
        material_ty: "pbr",
        render_policy: "default",
        evaluation_shader_entry: "fresco_evaluation_shader_clustered_surface",
        evaluation_contract: {
          inputs: ["lighting_ctx"],
          runtime: ["camera", "scene", "cluster_table"],
        },
        evaluation_variants: [
          {
            entry: "clustered_8_16_8_",
            bindings: [
              { axis: "tile_size", value: "8" },
              { axis: "cluster_depth_slices", value: "16" },
              { axis: "light_count", value: "8" },
            ],
          },
          {
            entry: "clustered_16_32_16_",
            bindings: [
              { axis: "tile_size", value: "16" },
              { axis: "cluster_depth_slices", value: "32" },
              { axis: "light_count", value: "16" },
            ],
          },
        ],
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
          ],
        },
        params: [],
        textures: [],
        sampler: null,
      }],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
  custom0: vec4<f32>,
  custom1: vec4<f32>,
  custom2: vec4<f32>,
  custom3: vec4<f32>,
  custom4: vec4<f32>,
  custom5: vec4<f32>,
  custom6: vec4<f32>,
  custom7: vec4<f32>,
}

fn fresco_clustered_surface(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv, uv2.x, 1.0), 0.5, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0,
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0),
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

fn fresco_evaluation_shader_clustered_surface(
  _uv: vec2<f32>,
  _uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>,
  mat: FrescoMaterial
) -> vec4<f32> {
  return vec4<f32>(mat.albedo.rgb, mat.albedo.a);
}

fn clustered_8_16_8_(
  _uv: vec2<f32>,
  _uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>,
  mat: FrescoMaterial
) -> vec4<f32> {
  return vec4<f32>(mat.albedo.rgb * 0.5, mat.albedo.a);
}

fn clustered_16_32_16_(
  _uv: vec2<f32>,
  _uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>,
  mat: FrescoMaterial
) -> vec4<f32> {
  return vec4<f32>(mat.albedo.rgb, mat.albedo.a);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(chooseEvaluationVariantEntry(manifest.surfaces.find(surface => surface.name === renderer.installed.entry))).toBeNull();
    expect(shaderCode).toBe(executable.wgsl);
  });

  it("applies forward_plus tie-break ordering using slices, tile size, then light count", async () => {
    const renderer = createAdapter();
    const manifest = {
      canvases: [],
      surfaces: [{
        name: "fp_surface",
        material_ty: "pbr",
        render_policy: "default",
        evaluation_shader_entry: "fresco_evaluation_shader_fp_surface",
        evaluation_contract: {
          inputs: ["lighting_ctx"],
          runtime: ["camera", "scene"],
        },
        evaluation_variants: [
          {
            entry: "forward_plus_8_32_16_",
            bindings: [
              { axis: "tile_size", value: "8" },
              { axis: "cluster_depth_slices", value: "32" },
              { axis: "light_count", value: "16" },
            ],
          },
          {
            entry: "forward_plus_16_16_16_",
            bindings: [
              { axis: "tile_size", value: "16" },
              { axis: "cluster_depth_slices", value: "16" },
              { axis: "light_count", value: "16" },
            ],
          },
          {
            entry: "forward_plus_16_32_8_",
            bindings: [
              { axis: "tile_size", value: "16" },
              { axis: "cluster_depth_slices", value: "32" },
              { axis: "light_count", value: "8" },
            ],
          },
        ],
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
          ],
        },
        params: [],
        textures: [],
        sampler: null,
      }],
    };

    const wgsl = `
struct FrescoMaterial {
  albedo: vec4<f32>,
  roughness: f32,
  metallic: f32,
  normal: vec3<f32>,
  emissive: vec3<f32>,
  opacity: f32,
  custom0: vec4<f32>,
  custom1: vec4<f32>,
  custom2: vec4<f32>,
  custom3: vec4<f32>,
  custom4: vec4<f32>,
  custom5: vec4<f32>,
  custom6: vec4<f32>,
  custom7: vec4<f32>,
}

fn fresco_fp_surface(
  uv: vec2<f32>,
  uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>
) -> FrescoMaterial {
  return FrescoMaterial(vec4<f32>(uv, uv2.x, 1.0), 0.5, 0.0, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0), 1.0,
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0),
    vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

fn fresco_evaluation_shader_fp_surface(
  _uv: vec2<f32>,
  _uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>,
  mat: FrescoMaterial
) -> vec4<f32> {
  return vec4<f32>(mat.albedo.rgb, mat.albedo.a);
}

fn forward_plus_8_32_16_(
  _uv: vec2<f32>,
  _uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>,
  mat: FrescoMaterial
) -> vec4<f32> {
  return vec4<f32>(mat.albedo.rgb * 0.5, mat.albedo.a);
}

fn forward_plus_16_16_16_(
  _uv: vec2<f32>,
  _uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>,
  mat: FrescoMaterial
) -> vec4<f32> {
  return vec4<f32>(mat.albedo.rgb * 0.75, mat.albedo.a);
}

fn forward_plus_16_32_8_(
  _uv: vec2<f32>,
  _uv2: vec2<f32>,
  _time: f32,
  _res: vec2<f32>,
  _world_pos: vec3<f32>,
  _world_normal: vec3<f32>,
  mat: FrescoMaterial
) -> vec4<f32> {
  return vec4<f32>(mat.albedo.rgb, mat.albedo.a);
}
`;

    const executable = executableMeshFixture(manifest, wgsl);
    await renderer.setShader(executable.wgsl, executable.manifest);

    const shaderCode = renderer.engine.install_with_assets.mock.calls[0][0];
    expect(chooseEvaluationVariantEntry(manifest.surfaces.find(surface => surface.name === renderer.installed.entry))).toBeNull();
    expect(shaderCode).toBe(executable.wgsl);
  });

  it("forward_plus does not infer selection from missing or malformed axes", async () => {
    await expectSelectedLightingEntryFromManifest({
      sourceExampleId: "test/surface-forward-plus-fallback-missing-malformed",
      sourceName: "forward_plus",
      expectedEntry: null,
      variants: [
        {
          entry: "forward_plus_missing_slices",
          bindings: [
            { axis: "tile_size", value: "64" },
            { axis: "light_count", value: "64" },
          ],
        },
        {
          entry: "forward_plus_malformed_slices",
          bindings: [
            { axis: "tile_size", value: "64" },
            { axis: "cluster_depth_slices", value: "lots" },
            { axis: "light_count", value: "64" },
          ],
        },
        {
          entry: "forward_plus_valid_16_8_4",
          bindings: [
            { axis: "tile_size", value: "8" },
            { axis: "cluster_depth_slices", value: "16" },
            { axis: "light_count", value: "4" },
          ],
        },
      ],
    });
  });

  it("clustered does not infer selection from missing or malformed axes", async () => {
    await expectSelectedLightingEntryFromManifest({
      sourceExampleId: "test/surface-clustered-fallback-missing-malformed",
      sourceName: "clustered",
      expectedEntry: null,
      variants: [
        {
          entry: "clustered_missing_slices",
          bindings: [
            { axis: "tile_size", value: "32" },
            { axis: "light_count", value: "32" },
          ],
        },
        {
          entry: "clustered_malformed_slices",
          bindings: [
            { axis: "tile_size", value: "32" },
            { axis: "cluster_depth_slices", value: "invalid" },
            { axis: "light_count", value: "32" },
          ],
        },
        {
          entry: "clustered_valid_24_8_2",
          bindings: [
            { axis: "tile_size", value: "8" },
            { axis: "cluster_depth_slices", value: "24" },
            { axis: "light_count", value: "2" },
          ],
        },
      ],
    });
  });
});
