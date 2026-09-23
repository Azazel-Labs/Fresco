import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { validateManifestContract } from "../manifest-contract";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CONTRACT_PATH = path.resolve(HERE, "../../../../../docs/contracts/manifest-pass-plan.contract.v1.json");
const CONTRACT = JSON.parse(readFileSync(CONTRACT_PATH, "utf8"));

describe("manifest pass-plan contract", () => {
  it("accepts a valid manifest pass_plan shape", () => {
    const manifest = {
      canvases: [{
        name: "demo",
        pass_plan: {
          passes: [{
            id: 0,
            stage: 0,
            locality: "point",
            start_layer: 0,
            end_layer: 0,
            count: 1,
            kernel_strategy: "fused",
            kernel_radius_px: null,
            kernel_levels: null,
            entry_point: "fresco_demo",
            inputs: [],
            output_target: null
          }],
          edges: [],
          targets: []
        }
      }]
    };

    const result = validateManifestContract(manifest, CONTRACT);
    expect(result.ok).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it("rejects missing pass_plan contract fields", () => {
    const manifest = {
      canvases: [{
        name: "demo",
        pass_plan: {
          passes: [{ id: 0, stage: 0, locality: "point" }],
          edges: [],
          targets: []
        }
      }]
    };

    const result = validateManifestContract(manifest, CONTRACT);
    expect(result.ok).toBe(false);
    expect(result.errors.some((msg) => msg.includes("kernel_strategy"))).toBe(true);
    expect(result.errors.some((msg) => msg.includes("start_layer"))).toBe(true);
    expect(result.errors.some((msg) => msg.includes("end_layer"))).toBe(true);
  });

  it("accepts surface uv-channel requirement metadata", () => {
    const manifest = {
      canvases: [{
        name: "demo",
        pass_plan: {
          passes: [{
            id: 0,
            stage: 0,
            locality: "point",
            start_layer: 0,
            end_layer: 0,
            count: 1,
            kernel_strategy: "fused",
            kernel_radius_px: null,
            kernel_levels: null,
            entry_point: "fresco_demo",
            inputs: [],
            output_target: null
          }],
          edges: [],
          targets: []
        }
      }],
      surfaces: [{
        name: "mat",
        material_ty: "pbr",
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" },
            { selector: "uv3", stream_index: 2, semantic: "TEXCOORD2", components: 2, required: false, status: "reserved" },
            { selector: "uv4", stream_index: 3, semantic: "TEXCOORD3", components: 2, required: false, status: "reserved" }
          ]
        }
      }]
    };

    const result = validateManifestContract(manifest, CONTRACT);
    expect(result.ok).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it("accepts optional surface vertex-stage requirement metadata", () => {
    const manifest = {
      canvases: [{
        name: "demo",
        pass_plan: {
          passes: [{
            id: 0,
            stage: 0,
            locality: "point",
            start_layer: 0,
            end_layer: 0,
            count: 1,
            kernel_strategy: "fused",
            kernel_radius_px: null,
            kernel_levels: null,
            entry_point: "fresco_demo",
            inputs: [],
            output_target: null
          }],
          edges: [],
          targets: []
        }
      }],
      surfaces: [{
        name: "mat",
        material_ty: "pbr",
        surface_requirements: {
          uv_channels: [
            { selector: "uv", stream_index: 0, semantic: "TEXCOORD0", components: 2, required: true, status: "required" },
            { selector: "uv2", stream_index: 1, semantic: "TEXCOORD1", components: 2, required: true, status: "required" }
          ],
          vertex_stage: {
            entry_point: "fresco_surface_vertex_mat",
            fields: ["position"]
          }
        }
      }]
    };

    const result = validateManifestContract(manifest, CONTRACT);
    expect(result.ok).toBe(true);
    expect(result.errors).toEqual([]);
  });

  it("accepts optional config axis metadata and build profile", () => {
    const manifest = {
      build_profile: "runtime",
      canvases: [{
        name: "demo",
        pass_plan: {
          passes: [{
            id: 0,
            stage: 0,
            locality: "point",
            start_layer: 0,
            end_layer: 0,
            count: 1,
            kernel_strategy: "fused",
            kernel_radius_px: null,
            kernel_levels: null,
            entry_point: "fresco_demo",
            inputs: [],
            output_target: null
          }],
          edges: [],
          targets: []
        }
      }],
      config_axes: [{
        pipeline: "forward",
        pass: "preview_tone",
        axis: "zoom_mode",
        known_mode: "draw",
        axis_class: "editor",
        inclusion: "editor_only",
        included_in_build: false
      }]
    };

    const result = validateManifestContract(manifest, CONTRACT);
    expect(result.ok).toBe(true);
    expect(result.errors).toEqual([]);
  });
});
