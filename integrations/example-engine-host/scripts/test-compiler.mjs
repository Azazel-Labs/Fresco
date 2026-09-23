// CPU-only regression: exercise the generated WASM, including linker inventory
// initialization. Native Rust tests cannot establish this boundary.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
const directory = process.argv[2] ? pathToFileURL(path.resolve(process.argv[2]) + path.sep) : new URL("../web/compiler/", import.meta.url);
const { initSync, compile_fresco_bundle, example_engine_sources_json, fresco_language_profile, fresco_wasm_build_mode } = await import(new URL("fresco_wasm.js", directory).href);

initSync({ module: readFileSync(new URL("fresco_wasm_bg.wasm", directory)) });
const buildMode = fresco_wasm_build_mode();
assert(["dev", "release"].includes(buildMode), `Unknown compiler build mode: ${buildMode}`);
if (process.argv[3]) assert.equal(buildMode, process.argv[3], "Compiler build mode must match the requested package profile");
const profile = fresco_language_profile();
for (const name of ["rgba", "normalize", "circle", "fill"]) {
  assert(profile.builtins.includes(name), `WASM registry is missing builtin ${name}`);
}
assert(profile.type_keywords.includes("color"), "WASM registry is missing color type");
const files = JSON.parse(example_engine_sources_json());
assert(Object.keys(files).length > 1, "profile must include imported modules");
const demo = readFileSync(new URL("../demo.fr", import.meta.url), "utf8");
const result = compile_fresco_bundle({ ...files, "main.fr": demo }, "main.fr", false);
assert(result.ok, JSON.stringify(result.diagnostics));
assert.equal(result.manifest.canvases[0].name, "demo");
assert(result.wgsl.includes("@vertex"));
assert(result.wgsl.includes("@fragment"));
const emitter = readFileSync(new URL("../../../examples/50) particles/drifting_sparks.fr", import.meta.url), "utf8");
const particles = compile_fresco_bundle({ ...files, "main.fr": emitter }, "main.fr", false);
assert(particles.ok, JSON.stringify(particles.diagnostics));
assert(particles.manifest.techniques.some(t => t.metadata.engine === "particle"), "emitter must produce an engine particle technique");
const technique = particles.manifest.techniques.find(t => t.metadata.engine === "particle");
assert.equal(technique.steps.find(s => s.name === "draw").operation.colors[0], "output");
assert.equal(technique.steps.find(s => s.name === "update").bindings.particles, "state");
assert.equal(particles.manifest.gpu_programs.find(p => p.metadata.capacity).metadata.capacity, "128");
assert(particles.wgsl.includes("@compute"));
// Browser responses retain collection fields even when the file format omits them.
for (const response of [result, particles]) {
  for (const key of ["canvases", "surfaces", "pipelines", "vertex_factories", "config_axes"]) {
    assert(Array.isArray(response.manifest[key]), `browser manifest requires array ${key}`);
  }
  for (const canvas of response.manifest.canvases) {
    for (const key of ["params", "global_uniforms", "storage_params", "textures", "path_buffers"]) {
      assert(Array.isArray(canvas[key]), `browser canvas requires array ${key}`);
    }
  }
  for (const surface of response.manifest.surfaces) {
    for (const key of ["params", "global_uniforms", "textures", "evaluation_variants", "custom_channels"]) {
      assert(Array.isArray(surface[key]), `browser surface requires array ${key}`);
    }
  }
}
const typed = compile_fresco_bundle({
  "engine/engine.fr": `
#pragma check.shape_aa_min_px = 1.5
#pragma check.shape_aa_max_px = 3.0
#pragma check.shape_aa_style = gradient
#pragma check.projective_footprint_max_px = 64.0
struct Domain { coordinate: vec2, count: u32, enabled: bool }
fn accumulate(previous: u32, next: u32, weight: f32) -> u32 {
  var value: u32 = previous
  value = value + next
  return select(previous, value, weight > 0.5)
}
@context(Domain, point) @composition(base, material, weight)
material_properties Data { channel power: f32 = 1.0
  channel @compose(accumulate) identity: u32 = 16777217
  channel @compose(combine_values) samples: array<vec2, 2> = [vec2(1.0), vec2(2.0)]
}
fn combine_values(previous: array<vec2, 2>, next: array<vec2, 2>, weight: f32) -> array<vec2, 2> {
  return [mix(previous[0], next[0], weight), previous[1]]
}
struct Packet { count: u32, enabled: bool, matrix: mat2, values: array<f32, 2> }
schema_program transfer for Data {
  fn apply(packet: Packet) -> Packet { return packet }
  output: apply
}`,
  "main.fr": "surface probe(point: Domain) -> material(Data) { compose { base()\nlayer material(samples: [vec2(3.0), vec2(4.0)], identity: point.count, weight: select(0.0, 1.0, point.enabled)) } }",
}, "main.fr", false);
assert(typed.ok, JSON.stringify(typed.diagnostics));
assert.equal(typed.manifest.surfaces[0].evaluation_variants[0].result_type, "Packet");
assert(typed.wgsl.includes("count: u32"));
assert(typed.wgsl.includes("enabled: bool"));
assert(typed.wgsl.includes("16777217u"), "native material default must retain all integer bits");
const samples = typed.manifest.surfaces[0].custom_channels.find(channel => channel.name === "samples");
assert.equal(samples.type.replace(/\s/g, ""), "array<vec2,2>");
assert.equal(samples.components, undefined);
console.log("Compiler WASM smoke passed: registry, embedded engine, canvas, particles, typed schema function ABI, and structured composition.");

const styleSettings = readFileSync(new URL("../../example-engine/tests/fixtures/style-settings.fr", import.meta.url), "utf8");
for (const renderer of ["forward", "forward-plus", "deferred"]) {
  const styled = compile_fresco_bundle({ ...files, "main.fr": styleSettings, "fresco.config.json": JSON.stringify({ renderer }) }, "main.fr", false);
  assert(styled.ok, JSON.stringify(styled.diagnostics));
  assert.equal(styled.manifest.schema_version, 12);
  const [first, second] = styled.manifest.surfaces.map(surface => surface.settings.implementations[0]);
  assert.equal(first.id, second.id, "one style must share executable dispatch");
  assert.notEqual(first.settings_offset, second.settings_offset, "materials need independent records");
  assert.equal(first.parameters[0].default, 0.25);
  assert.equal(second.parameters[0].default, 0.75);
}
console.log("Compiler WASM runtime style settings passed in every renderer.");

const exactSettings = styleSettings
  .replace("param gain: f32", "static param bands: u32 in [1, 32] = u32(4)\nparam seed: u32 = u32(4294967295)\nparam enabled: bool = true\nparam gain: f32")
  .replace("return tint.rgb * gain", "if enabled == false { return vec3(0.0) }\nreturn tint.rgb * gain * f32(bands + (seed & u32(1)))");
for (const renderer of ["forward", "forward-plus", "deferred"]) {
  const configuration = {renderer, property_overrides: {first: {style: {symbol: "Adjustable", settings: {bands: 8, seed: 16777217, enabled: false}}}}};
  const result = compile_fresco_bundle({...files, "main.fr": exactSettings, "fresco.config.json": JSON.stringify(configuration)}, "main.fr", false);
  assert(result.ok, JSON.stringify(result.diagnostics));
  const [first, second] = result.manifest.surfaces.map(surface => surface.settings.implementations[0]);
  assert.notEqual(first.id, second.id, "different static choices require specialized dispatch");
  assert.equal(first.static_parameters[0].default, 8);
  assert.equal(first.parameters[0].default, 16777217);
  assert.equal(second.parameters[0].default, 4294967295);
  assert.equal(first.parameters[1].default, false);
}
console.log("Compiler WASM exact scalar and static style settings passed in every renderer.");

const toon = readFileSync(new URL("../../../examples/40) surface shaders/style_sample.fr", import.meta.url), "utf8");
for (const renderer of ["forward", "forward-plus", "deferred"]) {
  const result = compile_fresco_bundle({...files, "main.fr": toon, "fresco.config.json": JSON.stringify({renderer})}, "main.fr", false);
  assert(result.ok, JSON.stringify(result.diagnostics));
  const recipe = result.manifest.renderers.find(recipe => recipe.selected);
  const outline = recipe.steps.find(step => step.invocation?.operation === "InvertedHull");
  assert(outline.after.includes(renderer === "deferred" ? "lighting_resolve" : "preview_mesh"));
  const material = result.manifest.surfaces.find(surface => surface.name === "style_sample");
  const producer = material.mesh_passes.find(pass => pass.preparation);
  assert.equal(producer.preparation.node, renderer === "deferred" ? "opaque" : "preview_mesh");
  assert(result.wgsl.includes(producer.preparation.entry));
  assert.equal(material.mesh_passes.find(pass => pass.pass === outline.pass).prepared_source, producer.pass);
  const disabled = compile_fresco_bundle({...files, "main.fr": toon.replace("outline_enabled: bool = true", "outline_enabled: bool = false"), "fresco.config.json": JSON.stringify({renderer})}, "main.fr", false);
  assert(disabled.ok, JSON.stringify(disabled.diagnostics));
  assert(disabled.manifest.surfaces.every(surface => surface.mesh_passes.every(pass => !pass.preparation && !pass.prepared_source)));
  assert(disabled.manifest.renderers.find(recipe => recipe.selected).steps.every(step => !step.invocation));
  for (const name of ["transparent", "scene_background"]) {
    assert(recipe.steps.find(step => step.name === name).after.includes(outline.name));
  }
}
console.log("Compiler WASM prepared geometry, inactive outlines, and typed style boundaries passed in every renderer.");

const operations = readFileSync(new URL("../../example-engine/tests/fixtures/style-operations.fr", import.meta.url), "utf8");
for (const renderer of ["forward", "forward-plus", "deferred"]) {
  const result = compile_fresco_bundle({...files, "main.fr": operations, "fresco.config.json": JSON.stringify({renderer})}, "main.fr", false);
  assert(result.ok, JSON.stringify(result.diagnostics));
  const recipe = result.manifest.renderers.find(recipe => recipe.selected);
  const draws = recipe.steps.filter(step => step.invocation);
  assert.equal(draws.length, 2);
  assert.deepEqual(draws.map(step => step.invocation.ordinal), [0, 1]);
  assert(draws.every(step => step.invocation.material === "item" && step.invocation.operation === "Paint"));
  assert(draws[1].after.includes(draws[0].name));
  assert.equal(recipe.resources.filter(resource => resource.source === "style_parameters").length, 1);
  assert(result.manifest.surfaces[0].mesh_passes.some(pass => pass.bindings?.some(binding => binding.name === "__operation_settings")));
}
console.log("Compiler WASM reusable style draws, explicit captures, and invocation ordering passed in every renderer.");

const computeDefinitions = readFileSync(new URL("../../example-engine/tests/fixtures/owned-compute.fr", import.meta.url), "utf8");
const computeDisplay = readFileSync(new URL("../../example-engine/tests/fixtures/owned-compute-display.fr", import.meta.url), "utf8");
const computeSource = computeDefinitions.replace("let dimensions = FieldDimensions(source: field)",
  "let dimensions = FieldDimensions(source: field)\nShow(geometry: self, scene: frame, values: copied, field: field, target: target.color)") + computeDisplay;
for (const renderer of ["forward", "forward-plus", "deferred"]) {
  const result = compile_fresco_bundle({...files, "main.fr": computeSource, "fresco.config.json": JSON.stringify({renderer})}, "main.fr", false);
  assert(result.ok, JSON.stringify(result.diagnostics));
  const kernels = result.manifest.gpu_programs.filter(program => program.compute_invocation);
  assert.equal(kernels.length, 4);
  const project = kernels.find(program => program.compute_invocation.operation === "ProjectPoints");
  const copy = kernels.find(program => program.compute_invocation.operation === "CopyPoints");
  assert.deepEqual(project.compute_invocation.engine_dependencies, [], "prepared compute must stay independent of opaque completion");
  assert.deepEqual(copy.compute_invocation.dependencies, [project.pass]);
  const draw = result.manifest.renderers.find(recipe => recipe.selected).steps.find(step => step.invocation?.operation === "Show");
  assert.equal(draw.invocation.compute_inputs.values.producer, copy.pass);
  assert.equal(draw.invocation.compute_inputs.__fresco_values_count.dimension, "count");
}
console.log("Compiler WASM owned compute outputs, dependency provenance, and draw consumption passed in every renderer.");

const fur = readFileSync(new URL("../../../examples/40) surface shaders/style_sample_fur.fr", import.meta.url), "utf8");
for (const renderer of ["forward", "forward-plus", "deferred"]) {
  const result = compile_fresco_bundle({...files, "main.fr": fur, "fresco.config.json": JSON.stringify({renderer})}, "main.fr", false);
  assert(result.ok, JSON.stringify(result.diagnostics));
  const kernels = result.manifest.gpu_programs.filter(program => program.compute_invocation);
  assert.equal(kernels.length, 4);
  for (const kernel of kernels) {
    assert.deepEqual(kernel.compute_invocation.dependencies, []);
    assert.deepEqual(kernel.compute_invocation.engine_dependencies, []);
  }
  const shells = result.manifest.renderers.find(recipe => recipe.selected).steps
    .filter(step => step.invocation?.operation === "FurShell");
  assert.equal(shells.length, 32);
  assert.equal(shells.filter(step => step.invocation.material === "chestnut_fur").length, 12);
  assert.equal(shells.filter(step => step.invocation.material === "pale_fur").length, 20);
  for (const {invocation} of shells) {
    assert.equal(invocation.point, "StandardStyle::transparent");
    assert(invocation.generated_vertices && invocation.bounds);
    assert(invocation.compute_inputs.vertices && invocation.compute_inputs.density);
  }
  assert(result.manifest.surfaces.every(surface => surface.mesh_passes.some(pass => Object.keys(pass.shading_inputs ?? {}).length > 0)));
}
console.log("Compiler WASM full MeadowFur, lighting iterators, projected captures, and independent compute passed in every renderer.");
