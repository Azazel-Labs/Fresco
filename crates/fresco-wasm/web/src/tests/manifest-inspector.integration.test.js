import { describe, expect, it } from "vitest";
import { readdir } from "node:fs/promises";

import { buildManifestInspectorState } from "../app/manifest-inspector";
import { chooseManifestPipelineForMaterial, findManifestPipelinesByMaterial } from "../preview/renderer-manifest";
import { appendRuntimeContentToBundle, buildCompileFilesBundle } from "../runtime-content-registry";

describe("manifest inspector runtime registry integration", () => {
  it("reports declared raster stages independently of preferred lighting metadata", () => {
    const surface = {
      name: "probe", material_ty: "standard", evaluation_shader_entry: "lighting_helper",
      evaluation_variants: [{ entry: "lighting_helper" }],
      mesh_passes: [{ pass: "preview_mesh", entries: [{ stage: "vertex", function: "project", entry: "mesh_vertex" }, { stage: "fragment", function: "paint", entry: "mesh_fragment" }, { stage: "fragment", function: "pack", entry: "geometry_fragment" }] }],
    };
    const mesh = buildManifestInspectorState({ surfaces: [surface] }, "probe");
    expect(mesh.selectedEntry).toBe("lighting_helper");
    expect(mesh.declaredVertexEntry).toBe("mesh_vertex");
    expect(mesh.declaredFragmentEntry).toBe("mesh_fragment");
    surface.mesh_passes.unshift({ pass: "other_pass", entries: [{ stage: "fragment", function: "pack", entry: "other_fragment" }] });
    const deferred = buildManifestInspectorState({ surfaces: [surface], renderers: [{ selected: true, steps: [{ domain: "mesh", pass: "preview_mesh", vertex: "project", entry: "pack" }] }] }, "probe");
    expect(deferred.declaredFragmentEntry).toBe("geometry_fragment");
    const techniques = [{ surface: "probe", steps: [{ operation: { kind: "draw", vertex: "particle_vertex", fragment: "particle_fragment" } }] }];
    const particle = buildManifestInspectorState({ surfaces: [surface], techniques }, "probe");
    expect(particle.declaredVertexEntry).toBe("particle_vertex");
    expect(particle.declaredFragmentEntry).toBe("particle_fragment");
    const metadataOnly = buildManifestInspectorState({ surfaces: [{ name: "probe", evaluation_shader_entry: "helper" }] }, "probe");
    expect(metadataOnly.declaredVertexEntry).toBe("");
    expect(metadataOnly.declaredFragmentEntry).toBe("");
  });

  it("discovers the complete engine source catalog without a manual file list", async () => {
    const files = await readdir(new URL("../../../../../integrations/example-engine/engine/", import.meta.url), { recursive: true });
    const expected = files.filter(path => path.endsWith(".fr"))
      .map(path => `engine/${path.replaceAll("\\", "/")}`).sort();
    expect([...buildCompileFilesBundle(new Map(), "").keys()].sort()).toEqual(expected);
  });

  it("assembles a compile bundle with engine runtime files for example-backed sources", () => {
    const baseBundle = new Map([["main.fr", "surface demo(sp: surf) -> material { compose { base(albedo: #fff) } }"]]);
    const resolvedBundle = buildCompileFilesBundle(baseBundle, "test/runtime-registry-surface");

    expect(resolvedBundle.get("main.fr")).toContain("surface demo");
    expect(resolvedBundle.get("engine/engine.fr")).toContain("import \"core/01_core.fr\"");
    expect(resolvedBundle.get("engine/engine.fr")).toContain("import \"core/04_canvas_contract.fr\"");
    expect(resolvedBundle.get("engine/engine.fr")).toContain("import \"pipelines/10_forward.fr\"");
    expect(resolvedBundle.get("engine/core/01_core.fr")).toContain("material_properties standard");
    expect(resolvedBundle.get("engine/core/04_canvas_contract.fr")).toContain("interface Canvas");
    expect(resolvedBundle.get("engine/pipelines/10_forward.fr")).toContain("pipeline(lighting) forward for standard");
  });

  it("derives selectable lighting metadata from registry-backed surfaces", () => {
    const baseBundle = new Map([["main.fr", "surface demo(sp: surf) -> material { compose { base(albedo: #fff) } }"]]);
    const resolvedBundle = appendRuntimeContentToBundle(baseBundle, "test/runtime-registry-surface");

    const engineRuntimeFile = resolvedBundle.get("engine/engine.fr") || "";
    expect(engineRuntimeFile).toContain("import \"core/01_core.fr\"");
    expect(engineRuntimeFile).toContain("import \"core/04_canvas_contract.fr\"");
    expect(engineRuntimeFile).toContain("import \"pipelines/10_forward.fr\"");

    const manifest = {
      surfaces: [
        {
          name: "surface_a",
          evaluation_shader_entry: "fresco_evaluation_shader_surface_a",
          evaluation_contract: { inputs: ["lighting_ctx"], runtime: ["camera", "scene"] },
          evaluation_variants: [
            {
              entry: "forward_plus_16_32_8_",
              bindings: [
                { axis: "tile_size", value: "16" },
                { axis: "cluster_depth_slices", value: "32" },
                { axis: "light_count", value: "8" },
              ],
            },
          ],
        },
        {
          name: "surface_b",
          settings: { evaluation_axes: { tile_size: "8", cluster_depth_slices: "16", light_count: "8" } },
          evaluation_shader_entry: "fresco_evaluation_shader_surface_b",
          evaluation_contract: { inputs: ["lighting_ctx"], runtime: ["camera", "scene", "cluster_table"] },
          evaluation_variants: [
            {
              entry: "clustered_8_16_8_",
              bindings: [
                { axis: "tile_size", value: "8" },
                { axis: "cluster_depth_slices", value: "16" },
                { axis: "light_count", value: "8" },
              ],
            },
          ],
        },
      ],
    };

    const stateA = buildManifestInspectorState(
      manifest,
      "surface_a",
      ["engine/engine.fr"],
    );

    expect(stateA).toBeTruthy();
    expect(stateA.surfaceNames).toEqual(["surface_a", "surface_b"]);
    expect(stateA.selectedSurfaceName).toBe("surface_a");
    expect(stateA.runtimeFilePaths).toEqual(["engine/engine.fr"]);
    expect(stateA.selectedEntry).toBe("");
    expect(stateA.selectedEntry).toBe("");

    expect(stateA.selectedLightingPipeline).toBe("");

    const stateB = buildManifestInspectorState(
      manifest,
      "surface_b",
      ["engine/engine.fr"],
    );
    expect(stateB).toBeTruthy();
    expect(stateB.selectedSurfaceName).toBe("surface_b");
    expect(stateB.selectedEntry).toBe("clustered_8_16_8_");
    expect(stateB.selectedEntry).toBe("clustered_8_16_8_");
  });

  it("prefers lighting-typed pipelines for the active material profile", () => {
    const manifest = {
      surfaces: [
        {
          name: "crate_side",
          material_ty: "pbr",
          evaluation_shader_entry: "fresco_evaluation_shader_crate_side",
          evaluation_variants: [],
        },
      ],
      pipelines: [
        { name: "forward", material: "pbr", type: "lighting", passes: ["fwd_base", "fwd_add"] },
        { name: "preview", material: "pbr", type: "debug", passes: ["debug_pass"] },
        { name: "unrelated", material: "unlit", type: "lighting", passes: ["noop"] },
      ],
    };

    const state = buildManifestInspectorState(manifest, "crate_side", []);
    expect(state).toBeTruthy();
    expect(state.selectedSurfaceMaterial).toBe("pbr");
    expect(state.selectedLightingPipeline).toBe("forward");
    expect(state.pipelinesForSurface.map((entry) => entry.name)).toEqual(["forward", "preview"]);

    const lighting = chooseManifestPipelineForMaterial(manifest, "pbr", "lighting");
    expect(lighting?.name).toBe("forward");

    const all = findManifestPipelinesByMaterial(manifest, "pbr");
    expect(all.map((entry) => entry.name)).toEqual(["forward", "preview"]);
  });

  it("uses the explicitly selected renderer pipeline", () => {
    const manifest = {
      renderers: [{ id: "custom", selected: true, pipeline: "semantic_forward_plus" }],
      surfaces: [
        {
          name: "crate_side",
          material_ty: "pbr",
          evaluation_shader_entry: "fresco_evaluation_shader_crate_side",
          evaluation_variants: [],
        },
      ],
      pipelines: [
        {
          name: "legacy_lighting",
          material: "pbr",
          type: "lighting",
          passes: ["fwd_base", "fwd_add"],
          semantic_summary: { topology: "unknown", raster_passes: 2, compute_passes: 0, additive_passes: 1, resource_flow_edges: 0 },
        },
        {
          name: "semantic_forward_plus",
          material: "pbr",
          type: "lighting",
          passes: ["cluster_cull", "fp_shade"],
          semantic_summary: { topology: "forward_plus", raster_passes: 1, compute_passes: 1, additive_passes: 0, resource_flow_edges: 1 },
        },
      ],
    };

    const selected = chooseManifestPipelineForMaterial(manifest, "pbr", "lighting");
    expect(selected?.name).toBe("semantic_forward_plus");
  });
});
