use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, unique_temp_path};

fn assert_compiles_with_suffix(suffix: &str, input: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected parser-compat sample to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

fn assert_fails_with_suffix(suffix: &str, input: &str, expected: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected parser-compat sample to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(expected),
        "expected stderr to contain `{expected}`\nstderr:\n{stderr}"
    );
}

#[test]
fn semicolon_terminated_statements_compile() {
    let input = r#"fn radius_from(x: f32) -> f32 {
  let base = x * 0.5;
  return 0.08 + 0.04 * base;
}

canvas t(uv: coord, time: signal) -> color {
  let r = radius_from(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_semicolons", input);
}

#[test]
fn c_style_for_increment_and_decrement_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    for (int i = 0; i < 4; i++) {
      circle(at: (0.2 + i * 0.15, 0.35), radius: 0.035) |> fill(#7dd3fc)
    }
    for (int j = 4; j >= 1; j--) {
      circle(at: (0.2 + j * 0.15, 0.65), radius: 0.03) |> fill(#f472b6)
    }
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_c_for", input);
}

#[test]
fn c_style_for_with_and_condition_compiles() {
    let input = r#"fn march(limit: f32) -> f32 {
  var t = 0.0;
  for (int i = 0; i < 8 && t < limit; i++) {
    t += 0.2;
  }
  return t;
}

canvas t(uv: coord, time: signal) -> color {
  let r = 0.02 + 0.02 * march(1.0 + uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_c_for_and", input);
}

#[test]
fn for_range_single_statement_body_compiles() {
    let input = r#"fn sum_steps() -> f32 {
  var t = 0.0;
  for i in 0 .. 4 t += 0.25;
  return t;
}

canvas t(uv: coord, time: signal) -> color {
  let r = 0.03 + 0.02 * sum_steps();
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_for_range_single_stmt", input);
}

#[test]
fn c_style_for_single_statement_body_compiles() {
    let input = r#"fn sum_steps() -> f32 {
  var t = 0.0;
  for (int i = 0; i < 4; i++) t += 0.25;
  return t;
}

canvas t(uv: coord, time: signal) -> color {
  let r = 0.03 + 0.02 * sum_steps();
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_c_for_single_stmt", input);
}

#[test]
fn nested_c_style_for_single_statement_bodies_compile() {
    let input = r#"#define ZERO 0
#define AA 2

fn sample_count() -> f32 {
  var total = 0.0;
  for (int m = ZERO; m < AA; m++)
    for (int n = ZERO; n < AA; n++)
      total += 1.0;
  return total;
}

canvas t(uv: coord, time: signal) -> color {
  let r = 0.03 + 0.01 * sample_count();
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_c_for_nested_single_stmt", input);
}

#[test]
fn break_in_function_for_loop_compiles() {
    let input = r#"fn march_limit(limit: f32) -> f32 {
  var t = 0.0;
  for (int i = 0; i < 8; i++) {
    t += 0.2;
    if (t > limit) break;
  }
  return t;
}

canvas t(uv: coord, time: signal) -> color {
  let r = 0.02 + 0.02 * march_limit(0.6 + uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_break_for", input);
}

#[test]
fn bit_helper_builtins_compile() {
    let input = r#"fn mask_bits(i: f32) -> vec3 {
  return (
    bit_and(bit_shr(i + 3.0, 1.0), 1.0),
    bit_and(bit_shr(i, 1.0), 1.0),
    bit_and(i, 1.0)
  );
}

canvas t(uv: coord, time: signal) -> color {
  let b = mask_bits(3.0);
  compose {
    circle(at: center, radius: 0.03 + 0.01 * b.x) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_bit_helpers", input);
}

#[test]
fn pipeline_kind_header_compiles_in_staged_pipeline_dsl() {
    let input = r#"
  pass fwd_base for standard {
    stage: raster
  }

  pipeline(lighting) forward for standard {
    fwd_base
  }
  "#;

    assert_compiles_with_suffix("parser_compat_pipeline_kind_header", input);
}

#[test]
fn pipeline_without_kind_clause_defaults_to_postprocess_in_staged_pipeline_dsl() {
    let input = r#"
  pass fwd_base for standard {
    stage: raster
  }

  pipeline forward for standard {
    fwd_base
  }
  "#;

    assert_compiles_with_suffix("parser_compat_pipeline_default_kind", input);
}

#[test]
fn pass_semantic_headers_compile_in_staged_pipeline_dsl() {
    let input = r#"
  pass gbuffer for standard {
    stage: raster
    draw: geometry
    blend: over
    reads: depth_prepass, normal_buffer
    writes: gbuffer_albedo, gbuffer_normal
  }

  pipeline(lighting) deferred for standard {
    gbuffer
  }
  "#;

    assert_compiles_with_suffix("parser_compat_pass_semantic_headers", input);
}

#[test]
fn vertex_factory_declarations_compile() {
    let input = r#"@group(2) group draw

vertex_interface Shaded {
    position: vec3 in object
    normal: vec3 in object
}

vertex_format StaticMesh {
    required {
        position: vec3 in object
    }
    optional {
        normal: vec3 in object = vec3(0, 0, 1)
    }
}

vertex_factory static for StaticMesh {
    binding {
        @group(draw) model: uniform<mat4 from object to world>
    }
}
"#;

    assert_compiles_with_suffix("parser_compat_vertex_declarations", input);
}

#[test]
fn vertex_factory_target_format_must_exist() {
    let input = r#"@group(2) group draw

vertex_interface Shaded {
    position: vec3 in object
}

vertex_factory static for MissingMesh {
    binding {
        @group(draw) model: uniform<mat4 from object to world>
    }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_vertex_factory_unknown_format",
        input,
        "vertex factory `static` targets unknown vertex_format `MissingMesh`",
    );
}

#[test]
fn pass_semantic_duplicate_stage_is_rejected() {
    let input = r#"
  pass gbuffer for standard {
    stage: raster
    stage: compute
  }

  pipeline(lighting) deferred for standard {
    gbuffer
  }
  "#;

    assert_fails_with_suffix(
        "parser_compat_pass_semantic_duplicate_stage",
        input,
        "conflicting pass semantic `stage` values `raster` and `compute`",
    );
}

#[test]
fn pass_semantic_duplicate_reads_is_rejected() {
    let input = r#"
  pass gbuffer for standard {
    stage: raster
    reads: depth_prepass
    reads: normal_prepass
  }

  pipeline(lighting) deferred for standard {
    gbuffer
  }
  "#;

    assert_fails_with_suffix(
        "parser_compat_pass_semantic_duplicate_reads",
        input,
        "duplicate pass semantic `reads` in pass `gbuffer`",
    );
}

#[test]
fn pass_semantic_compute_blend_conflict_is_rejected() {
    let input = r#"
  pass cluster_cull for standard {
    stage: compute
    blend: additive
  }

  pipeline(compute) clustered for standard {
    cluster_cull
  }
  "#;

    assert_fails_with_suffix(
        "parser_compat_pass_semantic_compute_blend_conflict",
        input,
        "compute passes do not support blending",
    );
}

#[test]
fn pass_semantic_unknown_raster_draw_value_is_rejected() {
    let input = r#"
  pass fwd_base for standard {
    stage: raster
    draw: per_material
  }

  pipeline(lighting) forward for standard {
    fwd_base
  }
  "#;

    assert_fails_with_suffix(
        "parser_compat_pass_semantic_unknown_draw",
        input,
        "unknown draw semantic `per_material`",
    );
}

#[test]
fn pass_semantic_unknown_raster_blend_value_is_rejected() {
    let input = r#"
  pass fwd_base for standard {
    stage: raster
    blend: multiply
  }

  pipeline(lighting) forward for standard {
    fwd_base
  }
  "#;

    assert_fails_with_suffix(
        "parser_compat_pass_semantic_unknown_blend",
        input,
        "unknown blend semantic `multiply`",
    );
}

#[test]
fn pass_semantic_fn_shade_hook_compiles() {
    let input = r#"
  pass fwd_base for standard {
    stage: raster
    draw: per_object
    blend: opaque

    fn shade(sp: Surf, m: standard) -> color {
      return rgba(1.0, 1.0, 1.0, 1.0)
    }
  }

  pipeline(lighting) forward for standard {
    fwd_base
  }
  "#;

    assert_compiles_with_suffix("parser_compat_pass_fn_shade_hook", input);
}

#[test]
fn duplicate_pass_hooks_are_rejected_even_without_vertex_parameters() {
    for hook in ["vertex", "shade", "main", "helper"] {
        let input = format!("pass p {{\nfn {hook}() {{}}\nfn {hook}() {{}}\n}}");
        assert_fails_with_suffix(
            &format!("duplicate_pass_hook_{hook}"),
            &input,
            &format!("duplicate pass hook `{hook}` in pass `p`"),
        );
    }
}

#[test]
fn pass_hooks_require_matching_stages_even_without_parameters() {
    for (hook, stage, required) in [
        ("vertex", "compute", "raster"),
        ("shade", "compute", "raster"),
        ("main", "raster", "compute"),
    ] {
        let input = format!("pass p {{\nstage: {stage}\nfn {hook}() {{}}\n}}");
        assert_fails_with_suffix(
            &format!("pass_hook_stage_{hook}"),
            &input,
            &format!("hook `{hook}` requires stage `{required}`"),
        );
    }
}

#[test]
fn pass_hook_duplicate_parameters_are_rejected() {
    assert_fails_with_suffix(
        "pass_hook_duplicate_parameters",
        "pass p {\nfn shade(v: vec2, v: vec2) -> color {}\n}",
        "duplicate parameter `v` in pass `p` hook `shade`",
    );
}

#[test]
fn interface_fullscreen_pass_rejects_malformed_body() {
    assert_fails_with_suffix(
        "fullscreen_contract_malformed_body",
        "interface ScreenEffect { fn draw(uv: vec2) -> color }\n\
         pass present for ScreenEffect {\ndraw: fullscreen\n\
         fn shade(uv: vec2, instance: ScreenEffect) -> color { let broken = }\n}",
        "invalid body of pass `present` hook `shade`",
    );
}

#[test]
fn interface_fullscreen_pass_retains_nested_control_flow() {
    assert_compiles_with_suffix(
        "fullscreen_contract_nested_body",
        "struct ScreenVarying { clip_pos: vec4, uv: vec2 }\n\
         interface ScreenEffect { fn draw(uv: vec2) -> color }\n\
         pass present for ScreenEffect {\nstage: raster\ndraw: fullscreen\n\
         fn vertex(vertex_id: u32) -> ScreenVarying {\n\
         return ScreenVarying(clip_pos: vec4(0.0, 0.0, 0.0, 1.0), uv: vec2(0.0, 0.0))\n}\n\
         fn shade(v: ScreenVarying, instance: ScreenEffect) -> color {\n\
         if v.uv.x > 0.5 { return instance.draw(v.uv) }\n\
         return rgba(0.0, 0.0, 0.0, 1.0)\n}\n}",
    );
}

#[test]
fn pass_semantic_fn_vertex_hook_contract_compiles() {
    let input = r#"@group(2) group draw

vertex_interface Shaded {
  position: vec3 in object
}

vertex_format StaticMesh {
  required {
    position: vec3 in object
  }
}

vertex_factory static for StaticMesh {
  binding {
    @group(draw) model: uniform<mat4 from object to world>
  }
}

pass fwd_base for standard {
  stage: raster
  draw: per_object
  blend: opaque

  fn vertex(v: Shaded) -> Surf {
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_pass_fn_vertex_hook_contract", input);
}

#[test]
fn pass_semantic_fn_vertex_hook_requires_declared_interface() {
    let input = r#"
pass fwd_base for standard {
  stage: raster
  draw: per_object
  blend: opaque

  fn vertex(v: MissingInterface) -> Surf {
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_pass_fn_vertex_hook_unknown_interface",
        input,
        "references unknown vertex_interface `MissingInterface`",
    );
}

#[test]
fn pass_vertex_contract_with_satisfying_factory_compiles() {
    let input = r#"@group(2) group draw

vertex_interface Shaded {
  position: vec3 in object
  normal: vec3 in object
}

vertex_format StaticMesh {
  required {
    position: vec3 in object
  }
  optional {
    normal: vec3 in object = vec3(0, 0, 1)
  }
}

vertex_factory static for StaticMesh {
  binding {
    @group(draw) model: uniform<mat4 from object to world>
  }
}

pass fwd_base for standard {
  stage: raster
  draw: per_object

  fn vertex(v: Shaded) -> Surf {
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix(
        "parser_compat_pass_vertex_factory_satisfies_contract",
        input,
    );
}

#[test]
fn pass_vertex_contract_without_satisfying_factory_is_rejected() {
    let input = r#"@group(2) group draw

vertex_interface Shaded {
  position: vec3 in object
  tangent: vec4
}

vertex_format StaticMesh {
  required {
    position: vec3 in object
  }
}

vertex_factory static for StaticMesh {
  binding {
    @group(draw) model: uniform<mat4 from object to world>
  }
}

pass fwd_base for standard {
  stage: raster
  draw: per_object

  fn vertex(v: Shaded) -> Surf {
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_pass_vertex_factory_missing_contract",
        input,
        "no vertex_factory target format satisfies that contract",
    );
}

#[test]
fn pass_semantic_fn_main_hook_compiles() {
    let input = r#"
  pass cluster_cull {
    stage: compute

    fn main(gid: uvec3, lid: uvec3) {
      let x = gid.x + lid.x
      let y = gid.y + lid.y
      let z = gid.z + lid.z
      let _sum = x + y + z
    }
  }

  pipeline(compute) clustered {
    cluster_cull
  }
  "#;

    assert_compiles_with_suffix("parser_compat_pass_fn_main_hook", input);
}

#[test]
fn pass_semantic_bare_shade_hook_is_rejected() {
    let input = r#"
  pass fwd_base for standard {
    stage: raster
    draw: per_object
    blend: opaque

    shade(sp: Surf, m: standard) -> color {
      return rgba(1.0, 1.0, 1.0, 1.0)
    }
  }

  pipeline(lighting) forward for standard {
    fwd_base
  }
  "#;

    assert_fails_with_suffix(
        "parser_compat_pass_bare_shade_hook_rejected",
        input,
        "expected",
    );
}

#[test]
fn pass_semantic_bare_main_hook_is_rejected() {
    let input = r#"
  pass cluster_cull {
    stage: compute

    main(gid: uvec3, lid: uvec3) {
      let x = gid.x + lid.x
      let y = gid.y + lid.y
      let z = gid.z + lid.z
      let _sum = x + y + z
    }
  }

  pipeline(compute) clustered {
    cluster_cull
  }
  "#;

    assert_fails_with_suffix(
        "parser_compat_pass_bare_main_hook_rejected",
        input,
        "expected",
    );
}

#[test]
fn scalar_specialized_overloads_compile() {
    let input = r#"fn pick_radius(x: f32) -> f32 {
  return 0.07 + x * 0.0;
}

fn pick_radius(x: i32) -> f32 {
  return 0.12 + f32(x) * 0.0;
}

canvas t(uv: coord, time: signal) -> color {
  let a = pick_radius(0.5);
  let b = pick_radius(3);
  compose {
    circle(at: center, radius: (a + b) * 0.5) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_scalar_specialized_overloads", input);
}

#[test]
fn nested_axis_value_sets_compile() {
    let input = r#"
axis @known(compile) ambient: flat|sh2|lightmap|lightmap_dir
axis @known(compile) main: none
                         | lit
                         | shadowed { cascades: 1|2|4, cascade_taps: 1|3|5 }

pass fwd_base for standard {
  stage: raster

  permutations {
    ambient: flat|sh2|lightmap|lightmap_dir
    main: none|lit|shadowed when ambient != lightmap | lightmap_dir else none
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_nested_axis_values", input);
}

#[test]
fn child_axis_is_rejected_outside_parent_scope() {
    let input = r#"
axis @known(compile) main: none|lit|shadowed { cascades: 1|2|4 }

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit|shadowed
  }

  require {
    cascades == 1
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_child_axis_scope",
        input,
        "sub-axis `cascades` is only valid under parent axis `main`",
    );
}

#[test]
fn doc_axis_and_pass_require_forms_compile() {
    let input = r#"
enum FogMode {
  off
  linear
}

axis @known(compile) ambient: flat|sh2|lightmap|lightmap_dir
axis @known(compile) main: none|lit|shadowed
axis @known(draw) fog: FogMode

pass fwd_base for standard {
  stage: raster

  permutations {
    ambient: flat|sh2|lightmap|lightmap_dir
    main: none|lit|shadowed when ambient != lightmap | lightmap_dir else none
    @known(draw) fog: off|linear
  }

  require {
    main == shadowed => main != none
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_doc_axis_require", input);
}

#[test]
fn doc_vertex_declarations_parse() {
    let input = r#"@group(2) group draw

vertex_interface Shaded {
  position: vec3 in object
  normal: vec3 in object
  tangent: vec4
  uv: vec2
}

vertex_format StaticMesh {
  required {
    position: vec3 in object
  }
  optional {
    normal: vec3 in object = vec3(0, 0, 1)
    tangent: vec4 = vec4(1, 0, 0, 1)
    uv: vec2 = vec2(0)
  }
}

vertex_factory static for StaticMesh {
  binding { @group(draw) model: uniform<mat4 from object to world> }

  fn transform(v: StaticMesh) -> mat4 from object to world {
    return model
  }
}

pass fwd_base for standard {
  stage: raster
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_doc_vertex_decls", input);
}

#[test]
fn doc_pass_render_state_blocks_parse() {
    let input = r#"
pass deferred_light for standard {
  stage: raster
  draw: per_binding(light)
  cull: front
  depth { test: greater_equal, write: off }
  stencil {
    test: equal, ref: 1, mask: 0xff
    on_pass: keep, on_fail: keep, on_depth_fail: keep
  }
  blend { all: additive }
  sort: back_to_front
}

pipeline(lighting) deferred for standard {
  deferred_light
}
"#;

    assert_compiles_with_suffix("parser_compat_doc_pass_state", input);
}

#[test]
fn texture_type_fn_at_signature_compiles() {
    let input = r#"
texture_type ShadowDepthArray -> f32 {
  sample: comparison
  layered
  r: depth
  fn at(uv: vec2, layer: u32, reference: f32) -> f32
}

canvas t(uv: coord) -> color {
  compose {
    circle(at: center, radius: 0.2) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_texture_type_fn_at", input);
}

#[test]
fn pass_permutation_requires_declared_axis() {
    let input = r#"
axis @known(compile) ambient: flat|sh2

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_axis_required_for_permutation",
        input,
        "references undeclared axis `main`",
    );
}

#[test]
fn pass_permutation_value_must_exist_in_axis_domain() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|shadowed
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_axis_value_domain",
        input,
        "but that value is not declared for the axis",
    );
}

#[test]
fn require_rejects_draw_known_axis_reference() {
    let input = r#"
axis @known(compile) main: none|lit
axis @known(draw) fog: off|linear

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit
    fog: off|linear
  }

  require {
    fog == off => main == none
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_require_draw_known_axis",
        input,
        "require guard references draw-known axis `fog`",
    );

    assert_fails_with_suffix(
        "parser_compat_require_draw_known_axis_decl_context",
        input,
        "declared here as draw-known",
    );
}

#[test]
fn draw_known_axis_requires_symbolic_enum_domain() {
    let input = r#"
axis @known(draw) fog: off|linear

pass fwd_base for standard {
  stage: raster
  permutations {
    fog: off|linear
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_draw_axis_symbolic_domain",
        input,
        "must declare a symbolic enum domain",
    );
}

#[test]
fn permutation_guard_rejects_draw_known_axis_reference() {
    let input = r#"
axis @known(compile) main: none|lit
axis @known(draw) fog: FogMode

enum FogMode {
  off
  linear
}

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit when fog == off else none
    fog: off|linear
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_permutation_guard_draw_axis",
        input,
        "permutation guard references draw-known axis `fog`",
    );
}

#[test]
fn binding_array_length_requires_compile_known_axis() {
    let input = r#"
axis @known(pipeline) cascades: 1|2|4

pass fwd_base for standard {
  stage: raster
  permutations {
    cascades: 1|2|4
  }
  binding {
    main_shadow: uniform<ShadowData>[cascades]
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_binding_array_compile_known",
        input,
        "uses axis `cascades` as an array length, but that axis is @known(pipeline)",
    );
}

#[test]
fn require_constraint_rejects_unknown_axis_identifier() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit
  }

  require {
    ghost == none => main == lit
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_require_unknown_axis",
        input,
        "require guard references unknown axis `ghost`",
    );
}

#[test]
fn permutation_guard_rejects_unknown_axis_identifier() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit when ghost == none else none
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_permutation_unknown_axis",
        input,
        "permutation guard references unknown axis `ghost`",
    );
}

#[test]
fn permutation_guard_unknown_axis_suggests_canonical_match() {
    let input = r#"
axis @known(compile) main_light: on|off

pass fwd_base for standard {
  stage: raster
  permutations {
    main_light: on|off when Main_Light == on else off
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_permutation_unknown_axis_suggestion",
        input,
        "did you mean axis `main_light`",
    );
}

#[test]
fn permutation_guard_allows_rhs_value_set_identifiers() {
    let input = r#"
axis @known(compile) ambient: flat|sh2|lightmap|lightmap_dir
axis @known(compile) main: none|lit|shadowed

pass fwd_base for standard {
  stage: raster
  permutations {
    ambient: flat|sh2|lightmap|lightmap_dir
    main: none|lit|shadowed when ambient != lightmap | lightmap_dir else none
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_permutation_rhs_values", input);
}

#[test]
fn permutation_guard_ordered_comparison_rejects_unknown_rhs_axis() {
    let input = r#"
axis @known(compile) max_per_cluster: 0|2|4|8
axis @known(compile) extra_lights: 0|2|4

pass fwd_base for standard {
  stage: raster
  permutations {
    extra_lights: 0|2|4 when max_per_cluster >= missing_axis else 0
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_permutation_ordered_unknown_rhs",
        input,
        "permutation guard references unknown axis `missing_axis`",
    );
}

#[test]
fn known_mode_matrix_axes_compile_with_legal_usage_sites() {
    let input = r#"
enum FogMode {
  off
  linear
}

axis @known(compile) ambient: flat|sh2
axis @known(pipeline) tile_size: 8|16|32
axis @known(draw) fog: FogMode

pass fwd_base for standard {
  stage: raster
  permutations {
    ambient: flat|sh2 when tile_size >= 8 else flat
    tile_size: 8|16|32
    fog: off|linear
  }

  require {
    ambient == sh2 => tile_size >= 8
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_known_mode_matrix", input);
}

#[test]
fn shared_axis_mismatch_within_one_pipeline_is_rejected() {
    let input = r#"
enum FogMode {
  off
  linear
}

enum FogModeAlt {
  off
  linear
  haze
}

pass deferred_ambient for standard {
  stage: raster
  permutations {
    @known(draw) fog: FogMode
  }
}

pass deferred_light for standard {
  stage: raster
  permutations {
    @known(draw) fog: FogModeAlt
  }
}

pipeline(lighting) forward for standard {
  deferred_ambient
  deferred_light
}
"#;

    assert_fails_with_suffix(
        "parser_compat_shared_axis_mismatch",
        input,
        "pipeline `forward` axis `fog` must keep the same permutation domain across every pass that defines it",
    );
}

#[test]
fn require_block_accepts_plain_constraints_and_logical_forms() {
    let input = r#"
axis @known(compile) ambient: flat|sh2
axis @known(compile) main: none|lit
axis @known(pipeline) tile_size: 8|16

pass fwd_base for standard {
  stage: raster
  permutations {
    ambient: flat|sh2
    main: none|lit
    tile_size: 8|16
  }

  require {
    tile_size >= 8
    ambient == flat || ambient == sh2
    main == none || main == lit
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_require_plain_constraints", input);
}

#[test]
fn require_block_accepts_mixed_implication_plain_and_parenthesized_groups() {
    let input = r#"
axis @known(compile) ambient: flat|sh2
axis @known(compile) main: none|lit
axis @known(pipeline) tile_size: 8|16|32

pass fwd_base for standard {
  stage: raster
  permutations {
    ambient: flat|sh2
    main: none|lit
    tile_size: 8|16|32
  }

  require {
    (ambient == flat || ambient == sh2) && (main == none || main == lit)
    main == lit => tile_size >= 8
    tile_size >= 8
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_compiles_with_suffix("parser_compat_require_mixed_parenthesized", input);
}

#[test]
fn require_constraint_rejects_malformed_set_expression_shape() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit
  }

  require {
    main == is_multiple_of(2, 1)
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_require_malformed_set_expr",
        input,
        "require constraint must be axis-only",
    );
}

#[test]
fn require_rejects_invalid_set_branch_syntax() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit
  }

  require {
    main == none |
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_require_invalid_set_branch",
        input,
        "unexpected",
    );
}

#[test]
fn require_rejects_invalid_parenthesization() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit
  }

  require {
    (main == lit
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_require_invalid_parenthesization",
        input,
        "unexpected",
    );
}

#[test]
fn require_rejects_invalid_comparison_operator_sequence() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit
  }

  require {
    main === lit
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_require_invalid_operator_sequence",
        input,
        "unexpected",
    );
}

#[test]
fn ordered_comparison_rejects_non_ordered_lhs_axis() {
    let input = r#"
axis @known(compile) ambient: flat|sh2

pass fwd_base for standard {
  stage: raster
  permutations {
    ambient: flat|sh2
  }

  require {
    ambient >= 1
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_ordered_axis_lhs",
        input,
        "ordered comparison on non-ordered axis `ambient`",
    );

    assert_fails_with_suffix(
        "parser_compat_ordered_axis_lhs_decl_context",
        input,
        "declared here as symbolic value-set domain",
    );
}

#[test]
fn ordered_comparison_rejects_non_ordered_rhs_axis() {
    let input = r#"
axis @known(compile) tile_size: 8|16|32
axis @known(compile) ambient: flat|sh2

pass fwd_base for standard {
  stage: raster
  permutations {
    tile_size: 8|16|32
    ambient: flat|sh2
  }

  require {
    tile_size >= ambient
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_ordered_axis_rhs",
        input,
        "ordered comparison on non-ordered axis `ambient`",
    );

    assert_fails_with_suffix(
        "parser_compat_ordered_axis_rhs_decl_context",
        input,
        "declared here as symbolic value-set domain",
    );
}

#[test]
fn permutation_guard_rejects_non_axis_expression_shape() {
    let input = r#"
axis @known(compile) main: none|lit

pass fwd_base for standard {
  stage: raster
  permutations {
    main: none|lit when is_multiple_of(2, 1) == true else none
  }
}

pipeline(lighting) forward for standard {
  fwd_base
}
"#;

    assert_fails_with_suffix(
        "parser_compat_permutation_guard_axis_only",
        input,
        "permutation guard must be axis-only",
    );
}

#[test]
fn scalar_specialized_overloads_can_be_ambiguous() {
    let input = r#"fn pick_bits(x: i32) -> f32 {
  return f32(x) * 0.0 + 0.2;
}

fn pick_bits(x: u32) -> f32 {
  return x * 0.0 + 0.4;
}

canvas t(uv: coord, time: signal) -> color {
  let b = pick_bits(3);
  compose {
    circle(at: center, radius: b) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_scalar_specialized_ambiguous",
        input,
        "ambiguous across specialized overloads",
    );
}

#[test]
fn declaration_union_scalar_overload_compiles() {
    let input = r#"fn pick_bits(x: i32|u32) -> f32 {
  return f32(x) * 0.0 + 0.25;
}

canvas t(uv: coord, time: signal) -> color {
  let a = pick_bits(-1);
  let b = pick_bits(4289379276);
  compose {
    circle(at: center, radius: (a + b) * 0.5) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_union_scalar_overload", input);
}

#[test]
fn declaration_union_cross_family_overload_compiles() {
    let input = r#"fn lift(x: i32|vec2) -> f32 {
  return 0.21;
}

canvas t(uv: coord, time: signal) -> color {
  let a = lift(-1);
  let b = lift((0.1, 0.2));
  compose {
    circle(at: center, radius: (a + b) * 0.5) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_union_cross_family_overload", input);
}

#[test]
fn declaration_union_with_additional_specialization_can_be_ambiguous() {
    let input = r#"fn pick_bits(x: i32|u32) -> f32 {
  return f32(x) * 0.0 + 0.2;
}

fn pick_bits(x: f32) -> f32 {
  return x * 0.0 + 0.4;
}

canvas t(uv: coord, time: signal) -> color {
  let b = pick_bits(3);
  compose {
    circle(at: center, radius: b) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_union_with_extra_specialization_ambiguous",
        input,
        "ambiguous across specialized overloads",
    );
}

#[test]
fn declaration_union_all_alternatives_validate_without_calls() {
    let input = r#"fn must_work(x: f32|vec2) -> f32 {
  return x + 1.0;
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_union_eager_alternative_validation",
        input,
        "declared return type is scalar",
    );
}

#[test]
fn declaration_union_multi_param_cartesian_expansion_compiles() {
    let input = r#"fn cart(a: i32|u32, b: f32|vec2, c: vec3|f32) -> f32 {
  return 0.31;
}

canvas t(uv: coord, time: signal) -> color {
  let r1 = cart(-1, 0.5, (0.1, 0.2, 0.3));
  let r2 = cart(4289379276, (0.1, 0.2), 0.75);
  compose {
    circle(at: center, radius: (r1 + r2) * 0.5) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_union_cartesian_expansion", input);
}

#[test]
fn declaration_union_duplicate_signature_reports_diagnostic() {
    let input = r#"fn duplicate_union(x: i32|i32) -> f32 {
  return 0.1;
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_union_duplicate_signature",
        input,
        "matching overload signature",
    );
}

#[test]
fn scalar_amp_operator_suggests_bit_and_builtin() {
    let input = r#"fn mask_bit(i: f32) -> f32 {
  return i & 1.0;
}

canvas t(uv: coord, time: signal) -> color {
  let b = mask_bit(3.0);
  compose {
    circle(at: center, radius: 0.03 + 0.01 * b) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_scalar_amp_suggest_builtin",
        input,
        "use `bit_and(a, b)`",
    );
}

#[test]
fn scalar_bar_operator_suggests_bit_or_builtin() {
    let input = r#"fn mask_bit(i: f32) -> f32 {
  return i | 1.0;
}

canvas t(uv: coord, time: signal) -> color {
  let b = mask_bit(3.0);
  compose {
    circle(at: center, radius: 0.03 + 0.01 * b) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_scalar_bar_suggest_builtin",
        input,
        "use `bit_or(a, b)`",
    );
}

#[test]
fn shift_operator_suggests_bit_shl_builtin() {
    let input = r#"fn shift_left(i: f32) -> f32 {
  return i << 2.0;
}
canvas t(uv: coord, time: signal) -> color {
  let b = shift_left(3.0);
  compose {
    circle(at: center, radius: 0.03 + 0.001 * b) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_shl_suggest_builtin",
        input,
        "use `bit_shl(x, shift)`",
    );
}

#[test]
fn modulo_operator_suggests_is_multiple_of_builtin() {
    let input = r#"fn has_even_stride(i: f32) -> f32 {
  return i % 2.0;
}

canvas t(uv: coord, time: signal) -> color {
  let b = has_even_stride(4.0);
  compose {
    circle(at: center, radius: 0.03 + 0.001 * b) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_modulo_operator", input);
}

#[test]
fn xor_operator_suggests_bit_xor_builtin() {
    let input = r#"fn xor_bits(i: f32) -> f32 {
  return i ^ 3.0;
}

canvas t(uv: coord, time: signal) -> color {
  let b = xor_bits(3.0);
  compose {
    circle(at: center, radius: 0.03 + 0.001 * b) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_xor_suggest_builtin",
        input,
        "use `bit_xor(a, b)`",
    );
}

#[test]
fn ternary_operator_compiles() {
    let input = r#"fn ternary_radius(x: f32) -> f32 {
  return x > 0.5 ? 0.18 : 0.08;
}

canvas t(uv: coord, time: signal) -> color {
  let r = ternary_radius(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_ternary", input);
}

#[test]
fn typed_local_declaration_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  vec3 q = (uv.x - 0.5, uv.y - 0.5, sin(time));
  let r = 0.05 + 0.05 * q.x;
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_typed_decl_ok", input);
}

#[test]
fn glsl_style_function_accepts_tagged_vector_params() {
    let input = r#"vec3 add(vec3 in world a, vec3 in world b) {
  return a + b;
}
"#;

    assert_compiles_with_suffix("parser_compat_glsl_tagged_vec_params", input);
}

#[test]
fn typed_local_declaration_mismatch_errors() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  vec3 q = uv;
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_typed_decl_mismatch",
        input,
        "typed local `q` expected vec3, but got vec2",
    );
}

#[test]
fn mutable_assignment_compiles() {
    let input = r#"fn grow_radius(x: f32) -> f32 {
  var r = x;
  r = r + 0.1;
  return r;
}

canvas t(uv: coord, time: signal) -> color {
  let r = grow_radius(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_assign_ok", input);
}

#[test]
fn mutable_assignment_to_undeclared_local_errors() {
    let input = r#"fn broken(x: f32) -> f32 {
  y = x + 1.0;
  return y;
}

canvas t(uv: coord, time: signal) -> color {
  let r = broken(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_assign_undeclared",
        input,
        "cannot assign to undeclared local `y`",
    );
}

#[test]
fn function_brace_on_next_line_compiles() {
    let input = r#"fn radius(x: f32) -> f32
{
  return x + 0.1;
}

canvas t(uv: coord, time: signal) -> color {
  let r = radius(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_fn_brace_newline", input);
}

#[test]
fn simple_preprocessor_if_else_compiles() {
    let input = r#"#define HW_PERFORMANCE 0
#if HW_PERFORMANCE==0
#define AA 1
#else
#define AA 2
#endif

fn samples() -> f32 {
  return AA;
}

canvas t(uv: coord, time: signal) -> color {
  let r = 0.03 * samples();
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_preprocessor_if_else", input);
}

#[test]
fn typed_local_float_alias_compiles() {
    let input = r#"fn radius(x: f32) -> f32 {
  float r = x + 0.2;
  return r;
}

canvas t(uv: coord, time: signal) -> color {
  let r = radius(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_float_alias", input);
}

#[test]
fn if_single_statement_body_compiles() {
    let input = r#"fn clamp_up(ro: vec2, rd: vec2, tmax0: f32) -> f32 {
  var tmax = tmax0;
  float tp = (0.8 - ro.y) / rd.y;
  if(tp > 0.0) tmax = min(tmax, tp);
  return tmax;
}

canvas t(uv: coord, time: signal) -> color {
  let r = clamp_up(uv, (1.0, 1.0), 1.0);
  compose {
    circle(at: center, radius: 0.05 + 0.02 * r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_if_single_stmt", input);
}

#[test]
fn swizzle_field_assignment_compiles() {
    let input = r#"fn nudge(input: vec3) -> vec3 {
  var mutv = input;
  mutv.x = abs(mutv.x);
  mutv.xy = (mutv.y, mutv.x);
  return mutv;
}

canvas t(uv: coord, time: signal) -> color {
  let v = nudge((uv.x - 0.5, uv.y - 0.5, 0.0));
  let r = 0.05 + 0.02 * (v.x + v.y);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_swizzle_assign", input);
}

#[test]
fn const_typed_local_declaration_compiles() {
    let input = r#"fn pick_k() -> f32 {
  const vec3 k = (-0.8660254, 0.5, 0.57735);
  return k.y;
}

canvas t(uv: coord, time: signal) -> color {
  let r = 0.05 + 0.03 * pick_k();
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_const_typed_decl", input);
}

#[test]
fn compound_assignment_compiles() {
    let input = r#"fn adjust(p0: vec2) -> vec2 {
  var p = p0;
  p += (0.1, -0.2);
  p *= (2.0, 2.0);
  return p;
}

canvas t(uv: coord, time: signal) -> color {
  let p = adjust(uv);
  let r = 0.03 + 0.04 * p.x;
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_compound_assign", input);
}

#[test]
fn comma_separated_typed_declarations_compile() {
    let input = r#"fn capsule_terms(p: vec3, a: vec3, b: vec3) -> f32 {
  vec3 pa = p - a, ba = b - a;
  float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
  return length(pa - ba * h);
}

canvas t(uv: coord, time: signal) -> color {
  let v = capsule_terms((uv.x, uv.y, 0.0), (0.0, 0.0, 0.0), (1.0, 1.0, 0.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * v) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_typed_decl_list", input);
}

#[test]
fn uninitialized_typed_declaration_compiles() {
    let input = r#"fn build() -> vec3 {
  vec3 q;
  q.x = 0.25;
  q.y = 0.5;
  q.z = 0.75;
  return q;
}

canvas t(uv: coord, time: signal) -> color {
  let v = build();
  compose {
    circle(at: center, radius: 0.03 + 0.02 * v.x) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_uninit_typed_decl", input);
}

#[test]
fn if_else_chain_single_statement_compiles() {
    let input = r#"fn pick(p: vec3, m: f32) -> vec3 {
  vec3 q;
  if(3.0 * p.x < m) q = p.xyz;
  else if(3.0 * p.y < m) q = p.yzx;
  else q = p.zxy;
  return q;
}

canvas t(uv: coord, time: signal) -> color {
  let v = pick((uv.x, uv.y, 0.2), 1.0);
  compose {
    circle(at: center, radius: 0.03 + 0.02 * v.x) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_if_else_single_stmt", input);
}

#[test]
fn glsl_vec_constructor_calls_compile() {
    let input = r#"fn mk() -> vec3 {
  vec3 a = vec3(0.1, 0.2, 0.3);
  vec2 b = vec2(0.4, 0.5);
  return vec3(a.x + b.x, a.y + b.y, a.z);
}

canvas t(uv: coord, time: signal) -> color {
  let v = mk();
  compose {
    circle(at: center, radius: 0.03 + 0.02 * v.x) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_glsl_vec_ctor", input);
}

#[test]
fn glsl_float_cast_and_vec_constructor_variants_compile() {
    let input = r#"fn mk2(i: f32) -> vec3 {
  vec2 p = vec2(i);
  vec3 a = vec3(p, float(0.25));
  vec3 b = vec3(0.0);
  return a + b;
}

canvas t(uv: coord, time: signal) -> color {
  let v = mk2(uv.x);
  compose {
    circle(at: center, radius: 0.03 + 0.02 * v.x) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_glsl_vec_ctor_variants", input);
}

#[test]
fn glsl_vec_ctor_wraps_clamp_vec3_result_compile() {
    let input = r#"fn saturate(col: vec3) -> vec3 {
  return vec3(clamp(col, 0.0, 1.0));
}

canvas t(uv: coord, time: signal) -> color {
  let col = (uv.x * 2.0 - 0.5, uv.y * 2.0 - 0.5, sin(time));
  let v = saturate(col);
  compose {
    circle(at: center, radius: 0.03 + 0.02 * v.x) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_vec3_ctor_clamp_vec3", input);
}

#[test]
fn struct_declaration_constructor_and_field_read_compile() {
    let input = r#"struct OrbitalState {
  center: vec2,
  radius: f32,
}

fn make_state(uv: vec2) -> OrbitalState {
  return OrbitalState(center: uv, radius: 0.12)
}

canvas t(uv: coord, time: signal) -> color {
  let state = make_state(center)
  compose {
    circle(at: state.center, radius: state.radius) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_struct_decl_ctor_read", input);
}

#[test]
fn record_keyword_declaration_is_rejected() {
    let input = r#"record LegacyState {
  center: vec2,
  radius: f32,
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    // Identifier-led headers and name: value settings are engine-entry syntax.
    // Legacy comma-separated record fields still fail at the first comma.
    assert_fails_with_suffix(
        "parser_compat_record_decl_rejected",
        input,
        "unexpected `,`",
    );
}

#[test]
fn struct_layout_clause_is_rejected() {
    let input = r#"struct Packed layout(std140) {
  p: vec2,
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_struct_layout_rejected",
        input,
        "unexpected `layout`",
    );
}

#[test]
fn struct_constructor_requires_named_arguments() {
    let input = r#"struct Probe {
  p: vec2,
  gain: f32,
}

canvas t(uv: coord, time: signal) -> color {
  let bad = Probe((0.25, 0.5), 0.8)
  compose {
    circle(at: bad.p, radius: bad.gain * 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_struct_ctor_named_required",
        input,
        "struct constructor `Probe` requires named arguments",
    );
}

#[test]
fn struct_constructor_rejects_unknown_field() {
    let input = r#"struct Probe {
  p: vec2,
  gain: f32,
}

canvas t(uv: coord, time: signal) -> color {
  let bad = Probe(pos: center, gain: 0.8)
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_struct_ctor_unknown_field",
        input,
        "unknown field `pos` for struct `Probe`",
    );
}

#[test]
fn struct_constructor_reports_missing_fields() {
    let input = r#"struct Probe {
  p: vec2,
  gain: f32,
}

canvas t(uv: coord, time: signal) -> color {
  let bad = Probe(p: center)
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_fails_with_suffix(
        "parser_compat_struct_ctor_missing_field",
        input,
        "struct constructor `Probe` is missing field arguments: gain",
    );
}

#[test]
fn struct_field_assignment_compiles() {
    let input = r#"struct Probe {
  p: vec2,
  gain: f32,
}

fn nudge(input: Probe) -> Probe {
  var pr = input
  pr.p = pr.p + (0.1, 0.0)
  pr.gain = pr.gain + 0.05
  return pr
}

canvas t(uv: coord, time: signal) -> color {
  let probe = Probe(p: center, gain: 0.2)
  compose {
    circle(at: probe.p, radius: probe.gain) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_struct_field_assign_ok", input);
}

#[test]
fn struct_field_assignment_type_mismatch_currently_compiles() {
    let input = r#"struct Probe {
  p: vec2,
  gain: f32,
}

fn broken(input: Probe) -> Probe {
  var pr = input
  pr.gain = center
  return pr
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_with_suffix("parser_compat_struct_field_assign_mismatch", input);
}

#[test]
fn explicit_axis_binding_time_cannot_override_engine_declaration() {
    assert_fails_with_suffix(
        "axis_mode_conflict",
        r#"
axis @known(compile) quality: low|high
pass render {
    permutations { @known(draw) quality: low|high }
}
"#,
        "binding time conflicts with its declaration",
    );
}

#[test]
fn draw_axis_enum_names_are_not_case_constrained() {
    assert_compiles_with_suffix(
        "lowercase_axis_enum",
        r#"
enum fog_mode { off, linear }
axis @known(draw) fog: fog_mode
pass render {
    permutations { fog: off|linear }
}
"#,
    );
}

#[test]
fn local_permutation_guard_rejects_unknown_axis_without_global_declarations() {
    assert_fails_with_suffix(
        "local_guard_typo",
        r#"
pass render {
    permutations { @known(compile) quality: low|high when typo == high else low }
}
"#,
        "permutation guard references unknown axis `typo`",
    );
}

#[test]
fn plain_statements_require_a_separator() {
    assert_fails_with_suffix(
        "missing_statement_separator",
        r#"
fn helper(x: f32) -> f32 { let y = x return y }
canvas t(uv: coord) -> color { compose { circle(radius: helper(0.2)) |> fill(#ffffff) } }
"#,
        "unexpected `return`",
    );
}

#[test]
fn closed_control_block_can_precede_same_line_statement() {
    assert_compiles_with_suffix(
        "block_statement_boundary",
        r#"
fn helper(x: f32) -> f32 { if x > 0.0 { return x } return 0.1 }
canvas t(uv: coord) -> color { compose { circle(at: center, radius: helper(uv.x)) |> fill(#ffffff) } }
"#,
    );
}

#[test]
fn invalid_constant_clamp_reports_a_diagnostic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  const f32 radius = clamp(0.5, 2.0, 1.0);
  compose {
    circle(at: center, radius: radius) |> fill(#ffffff)
  }
}
"#;
    assert_fails_with_suffix(
        "signal_const_clamp",
        input,
        "clamp lower bound must not exceed upper bound",
    );
}

#[test]
fn constant_domain_errors_survive_shared_evaluation() {
    for (expression, message) in [
        ("1.0 / 0.0", "cannot divide by zero"),
        ("sqrt(-1.0)", "cannot take sqrt of a negative value"),
    ] {
        let input = format!(
            "canvas t(uv: coord, time: signal) -> color {{\n  const f32 radius = {expression};\n  compose {{\n    circle(at: center, radius: radius) |> fill(#ffffff)\n  }}\n}}\n"
        );
        assert_fails_with_suffix("signal_const_domain", &input, message);
    }
}

#[test]
fn typed_runtime_scalar_helpers_keep_native_operations() {
    assert_compiles_with_suffix(
        "typed_runtime_helpers",
        r#"
fn exact(count: u32, enabled: bool) -> u32 {
    var value: u32 = 16777217 + 2
    value = 16777219
    value = min(value + count + (16777217 + 2) - 16777219, 4294967295)
    if enabled { return value }
    return count
}
fn signed(count: i32) -> i32 { return count >> 1 }
canvas t(uv: coord) -> color {
    let amount = exact(u32(uv.x * 10.0), uv.y > 0.5)
    let radius = f32(amount - 16777217) * 0.01 + f32(signed(i32(-16777217))) * 0.000000001
    compose { circle(at: center, radius: radius) |> fill(#ffffff) }
}
"#,
    );
}

#[test]
fn native_scalar_assignment_rejects_implicit_float_conversion() {
    assert_fails_with_suffix(
        "typed_assignment_mismatch",
        r#"
fn wrong(count: u32, value: f32) -> u32 {
    var result: u32 = count
    result = value
    return result
}
canvas t(uv: coord) -> color { compose { fill(#ffffff) } }
"#,
        "expected u32, found f32",
    );
}

#[test]
fn native_vectors_preserve_element_types_through_helpers() {
    assert_compiles_with_suffix(
        "typed_vector_helpers",
        r#"
fn ids(value: uvec2) -> uvec2 {
    var result: uvec2 = value + uvec2(16777217, 3)
    result = clamp(result + uvec2(1, 2), uvec2(0), uvec2(4294967295))
    return result
}
canvas t(uv: coord) -> color {
    let result = ids(uvec2(u32(uv.x), 0))
    compose { circle(at: center, radius: f32(result.x - 16777217) * 0.01) |> fill(#ffffff) }
}
"#,
    );
}

#[test]
fn native_runtime_overload_resolution_uses_bound_types() {
    assert_compiles_with_suffix(
        "typed_runtime_overloads",
        r#"
fn same(x: f32) -> f32 { return x }
fn same(x: u32) -> u32 { return x }
fn same(x: bool) -> bool { return x }
fn exact(x: u32) -> u32 {
    let enabled = true
    if same(enabled) { return same(x) }
    return 0
}
canvas t(uv: coord) -> color {
    compose { circle(at: center, radius: f32(exact(u32(uv.x))) * 0.01) |> fill(#ffffff) }
}
"#,
    );
}

#[test]
fn native_integer_context_does_not_truncate_fractional_literals() {
    assert_fails_with_suffix(
        "typed_fractional_literal",
        r#"
fn wrong(count: u32) -> u32 { return count + 0.25 }
canvas t(uv: coord) -> color { compose { fill(#ffffff) } }
"#,
        "literal cannot be implicitly converted to u32",
    );
}
