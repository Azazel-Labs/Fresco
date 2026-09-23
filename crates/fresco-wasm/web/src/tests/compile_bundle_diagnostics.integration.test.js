import { beforeAll, describe, expect, it } from "vitest";
import { readFile } from "node:fs/promises";

import initWasm, {
  compile_fresco_bundle,
  compile_fresco_with_options,
  compile_fresco_bundle_with_profile,
} from "../../pkg/fresco_wasm.js";
import enginePolicy from "../../../../../tests/render-policy/engine/engine.fr?raw";
import { buildCompileFilesBundle } from "../runtime-content-registry";

const root = new URL("../../../../../", import.meta.url);
const readmeManifest = JSON.parse(await readFile(new URL("docs/readme/samples.json", root), "utf8"));
const runnableReadmeSamples = readmeManifest.samples.filter(sample => sample.preview !== false);

beforeAll(async () => {
  const wasmPath = new URL("../../pkg/fresco_wasm_bg.wasm", import.meta.url);
  const wasmBytes = await readFile(wasmPath);
  await initWasm({ module_or_path: wasmBytes });
});

describe("bundle compile diagnostics keep source file provenance", () => {
  it("requires engine declarations for standalone rendering", () => {
    const result = compile_fresco_with_options("canvas t(uv: coord) -> color { compose { fill(#fff) } }", false);
    expect(result.ok).toBe(false);
    expect(result.diagnostics.filter(diag => diag.message.includes("missing required engine setting"))).toHaveLength(4);
  });
  // Give each compilation its own timeout as the README sample collection grows.
  it.each(runnableReadmeSamples)("loads README sample $id through the Lab engine bundle", async (sample) => {
    const source = await readFile(new URL(sample.source, root), "utf8");
    const exampleId = sample.source.replace(/^examples\//, "").replace(/\.fr$/, "");
    const files = buildCompileFilesBundle(new Map([["main.fr", source]]), exampleId);
    const result = compile_fresco_bundle(Object.fromEntries(files), "main.fr", false);
    expect(result.ok, `${sample.id}: ${JSON.stringify(result.diagnostics)}`).toBe(true);
  });

  it("attributes pass validation diagnostics to imported files", () => {
    const files = {
      "engine/render_policy.fr": enginePolicy,
      "main.fr": `import "lib.fr"\ncanvas demo(uv: coord) -> color {\n  compose {\n    fill(#ffffff)\n  }\n}\n`,
      "lib.fr": `pass bad_stage {\n  stage: not_a_stage\n}\n`,
    };

    const result = compile_fresco_bundle(files, "main.fr", false);

    expect(result.ok).toBe(false);
    expect(Array.isArray(result.diagnostics)).toBe(true);

    const stageDiag = result.diagnostics.find((diag) =>
      String(diag?.message || "").includes("unknown pass stage")
    );
    expect(stageDiag).toBeTruthy();
    expect(stageDiag.file).toBe("lib.fr");
  });

  it("accepts quoted pass permutation literals without standalone axis declarations", () => {
    const files = {
      "engine/render_policy.fr": enginePolicy,
      "main.fr": `canvas demo(uv: coord) -> color {\n  compose {\n    fill(#ffffff)\n  }\n}\n`,
      "engine/pipelines/10_forward.fr": `pass fwd_base {\n  stage: raster\n  permutations {\n    @known(compile) ambient: \"flat\"|\"sh2\"\n  }\n}\n`,
    };

    const result = compile_fresco_bundle(files, "main.fr", false);

    expect(result.ok).toBe(true);
    expect(Array.isArray(result.diagnostics)).toBe(true);
    expect(result.diagnostics.length).toBe(0);
  });

  it("threads runtime build profile through bundle compile", () => {
    const files = {
      "engine/render_policy.fr": enginePolicy,
      "main.fr": `canvas demo(uv: coord) -> color {\n  compose {\n    fill(#ffffff)\n  }\n}\n`,
    };

    const result = compile_fresco_bundle_with_profile(files, "main.fr", false, "runtime");

    expect(result.ok).toBe(true);
    expect(result.manifest.build_profile).toBe("runtime");
    expect(Array.isArray(result.diagnostics)).toBe(true);
    expect(result.diagnostics.length).toBe(0);
  });

  it("compiles bundled engine runtime files including the canvas contract", () => {
    const baseBundle = new Map([
      [
        "main.fr",
        "surface demo(sp: surf) -> material {\n  compose {\n    base(albedo: #fff)\n  }\n}\n",
      ],
    ]);
    const resolvedBundle = buildCompileFilesBundle(baseBundle, "test/runtime-registry-surface");

    expect(resolvedBundle.get("engine/core/04_canvas_contract.fr")).toContain("interface Canvas");

    const files = Object.fromEntries(resolvedBundle.entries());
    const result = compile_fresco_bundle(files, "main.fr", false);

    expect(result.ok, JSON.stringify(result.diagnostics)).toBe(true);
    expect(result.diagnostics.filter(diag => diag.severity !== "info")).toEqual([]);
    expect(result.manifest.surfaces[0].surface_shader_entry).toBe("fresco_demo");
    expect(result.manifest.surfaces[0].material_properties).toBe("standard");
    expect(result.manifest.surfaces[0].contract_requirements).toEqual({ required_textures: [] });
    expect(result.manifest.surfaces[0].evaluation_shader_entry).toBe("fresco_evaluation_shader_demo");
    expect(result.manifest.surfaces[0].evaluation_shader_source).toBe("schema_program:preview_lit");
    expect(result.wgsl).toContain("_pbr_direct_response(");
    expect(result.wgsl).toContain("_environment.settings.x");
    expect(result.manifest.surfaces[0].global_uniforms).toEqual([
      expect.objectContaining({ name: "frame", group: 3, binding: 0, byte_size: 16 }),
    ]);
    // The sample engine now shades standard materials using its lighting
    // environment. Authored hooks must still be able to call schema evaluators.
    const finish = "return rgba(style_finish(style, draw_record.style_settings_offset, surface, context, lighting), m.albedo.a * m.opacity)";
    expect(files["engine/core/05_mesh_contract.fr"]).toContain(finish);
    files["engine/core/05_mesh_contract.fr"] = files["engine/core/05_mesh_contract.fr"].replace(
      finish,
      "return evaluate_schema(surf(sp.uv, sp.uv2, scene.time, scene.res, sp.world_pos, sp.world_normal, 0.0, 1.0, 0.0, vec3(0.0)), m)"
    );
    const custom = compile_fresco_bundle(files, "main.fr", false);
    expect(custom.ok, JSON.stringify(custom.diagnostics)).toBe(true);
    expect(custom.wgsl).toContain("return fresco_evaluation_shader_demo(surf(sp.uv");
  });
  it("keeps badge animation connected to the engine buffer through WASM", async () => {
    const source = await readFile(new URL("../../../../../examples/10) fundamentals/badge.fr", import.meta.url), "utf8");
    const files = buildCompileFilesBundle(new Map([["main.fr", source]]), "");
    const result = compile_fresco_bundle(Object.fromEntries(files), "main.fr", false);
    expect(result.ok, JSON.stringify(result.diagnostics)).toBe(true);
    expect(result.wgsl).toContain("frame.time");
    expect(result.manifest.canvases[0].global_uniforms).toEqual([{
      name: "frame", type: "FrameGlobals", group: 3, binding: 0, byte_size: 16,
      fields: [
        { name: "time", type: "f32", offset: 0, components: 1, scalar_type: "f32" },
        { name: "delta_time", type: "f32", offset: 4, components: 1, scalar_type: "f32" },
        { name: "resolution", type: "vec2", offset: 8, components: 2, scalar_type: "f32" },
      ],
    }]);
  });

  it("preserves the authored surface response and its entrypoint through WASM", async () => {
    const source = await readFile(new URL("../../../../../examples/40) surface shaders/style_sample.fr", import.meta.url), "utf8");
    const files = buildCompileFilesBundle(new Map([["main.fr", source]]), "");
    const result = compile_fresco_bundle(Object.fromEntries(files), "main.fr", false);
    expect(result.ok, JSON.stringify(result.diagnostics)).toBe(true);
    const surface = result.manifest.surfaces[0];
    expect(result.manifest.techniques ?? []).toEqual([]);
    expect(surface.surface_shader).toBe("style_sample");
    expect(surface.material_properties).toBe("standard");
    expect(surface.evaluation_shader_entry).toBe("fresco_evaluation_shader_style_sample");
    expect(result.wgsl).toContain(`fn ${surface.evaluation_shader_entry}(`);
  });

  it("preserves authored emitter state through WASM", async () => {
    const source = await readFile(new URL("../../../../../examples/50) particles/drifting_sparks.fr", import.meta.url), "utf8");
    const files = buildCompileFilesBundle(new Map([["main.fr", source]]), "");
    const result = compile_fresco_bundle(Object.fromEntries(files), "main.fr", false);
    expect(result.ok, JSON.stringify(result.diagnostics)).toBe(true);
    const simulation = result.manifest.gpu_programs.find(p => p.metadata.capacity);
    expect(simulation.metadata.capacity).toBe("128");
    expect(simulation.bindings.find(b => b.name === "particles")).toMatchObject({
      element_stride: 48,
      fields: [
        { name: "position", offset: 0, ty: "vec4" },
        { name: "velocity", offset: 16, ty: "vec4" },
        { name: "age", offset: 32, ty: "f32" },
        { name: "lifespan", offset: 36, ty: "f32" },
        { name: "id", offset: 40, ty: "f32" },
      ],
    });
  });

  it("binds engine-chosen declaration and block names through WASM", () => {
    const source = "picture sample { paint { return rgba(ctx.uv.x, ctx.uv.y, 0.0, 1.0) } }";
    const files = buildCompileFilesBundle(new Map([["main.fr", source]]), "");
    const contract = files.get("engine/core/04_canvas_contract.fr");
    files.set("engine/core/04_canvas_contract.fr", contract
      .replace("@entry(canvas, draw)", "@entry(picture, paint)")
      .replace("fn draw(", "fn paint(")
      .replace("t.draw(", "t.paint("));
    const result = compile_fresco_bundle(Object.fromEntries(files), "main.fr", false);
    expect(result.ok, JSON.stringify(result.diagnostics)).toBe(true);
    expect(result.manifest.canvases[0].name).toBe("sample");
    expect(result.wgsl).toContain("fresco_sample");
  });

  it("checks engine settings and composes ordered modules through WASM", () => {
    const source = `picture sample {
      gain: 0.25
      adjust { brighten(gain) }
      paint { return rgba(adjust(ctx.uv.x), ctx.uv.y, 0.0, 1.0) }
    }`;
    const files = buildCompileFilesBundle(new Map([["main.fr", source]]), "");
    const contract = files.get("engine/core/04_canvas_contract.fr");
    files.set("engine/core/04_canvas_contract.fr", contract
      .replace("@entry(canvas, draw)", "@entry(picture, paint)")
      .replace("interface Canvas {", `interface Canvas {
        param gain: f32
        @compose(value) fn adjust(value: f32) -> f32`)
      .replace("fn draw(", "fn paint(")
      .replace("t.draw(", "t.paint(")
      + "\nfn brighten(value: f32, gain: f32) -> f32 { return value + gain }\n");
    const result = compile_fresco_bundle(Object.fromEntries(files), "main.fr", false);
    expect(result.ok, JSON.stringify(result.diagnostics)).toBe(true);
    expect(result.manifest.canvases[0].name).toBe("sample");
    files.set("main.fr", source.replace("gain: 0.25", "gain: vec2(0.25)"));
    const invalid = compile_fresco_bundle(Object.fromEntries(files), "main.fr", false);
    expect(invalid.ok).toBe(false);
    expect(invalid.diagnostics.some(diag => diag.message.includes("const `gain` expected"))).toBe(true);
  });

  it("keeps standalone time dynamic with a policy-only engine", () => {
    const source = "fn animated(x: f32) -> f32 { return x * sin(time) }\ncanvas t(uv: coord, time: signal) -> color { compose { circle(at: center, radius: sin(time) + animated(uv.x)) |> fill(#fff) } }";
    const result = compile_fresco_bundle({ "main.fr": source, "engine/engine.fr": enginePolicy }, "main.fr", false);
    expect(result.ok, JSON.stringify(result.diagnostics)).toBe(true);
    expect(result.wgsl).toContain("sin(t)");
    expect(result.wgsl).toContain("sin(entry_time)");
    expect(result.manifest.canvases[0].global_uniforms).toEqual([]);
  });

});
