use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

fn assert_compiles_and_validates_wgsl(suffix: &str, input: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = String::from_utf8(output.stdout).expect("expected utf-8 WGSL output");
    let module = naga::front::wgsl::parse_str(&wgsl)
        .unwrap_or_else(|e| panic!("expected WGSL parse to succeed\nerror:\n{e}\nwgsl:\n{wgsl}"));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    validator.validate(&module).unwrap_or_else(|e| {
        panic!("expected WGSL validation to succeed\nerror:\n{e:#?}\nwgsl:\n{wgsl}")
    });
}

fn compile_to_wgsl(suffix: &str, input: &str) -> String {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("expected utf-8 WGSL output")
}

fn assert_compile_fails_with(suffix: &str, input: &str, expected_in_stderr: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected compile to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(expected_in_stderr),
        "expected stderr to contain `{expected_in_stderr}`\nstderr:\n{stderr}"
    );
}

// ─── Local fn declarations inside canvas bodies ───────────────────────────────

#[test]
fn local_fn_decl_scalar_return_inside_canvas() {
    // A local fn that returns a scalar can be used as a helper called inside the canvas.
    let input = r#"canvas t(uv: coord) -> color {
  fn double(x: f32) -> f32 {
    return x * 2.0;
  }
  let r = double(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("local_fn_scalar", input);
}

#[test]
fn local_fn_decl_vec2_return_inside_canvas() {
    let input = r#"canvas t(uv: coord) -> color {
  fn flip(p: vec2) -> vec2 {
    return vec2(p.y, p.x);
  }
  let q = flip(uv);
  compose {
    circle(at: q, radius: 0.15) |> fill(#ff0000)
  }
}
"#;
    assert_compiles_and_validates_wgsl("local_fn_vec2", input);
}

#[test]
fn local_fn_decl_shape_return_inside_canvas() {
    // A local fn that returns a shape is evaluated inline at each call site.
    let input = r#"canvas t(uv: coord) -> color {
  fn my_circle(r: f32) -> shape {
    circle(at: center, radius: r)
  }
  compose {
    my_circle(0.2) |> fill(#00ff00)
  }
}
"#;
    assert_compiles_and_validates_wgsl("local_fn_shape", input);
}

#[test]
fn local_fn_decl_layer_return_inside_canvas() {
    let input = r#"canvas t(uv: coord) -> color {
  fn red_dot(r: f32) -> layer {
    circle(at: center, radius: r) |> fill(#ff0000)
  }
  compose {
    red_dot(0.25)
  }
}
"#;
    assert_compiles_and_validates_wgsl("local_fn_layer", input);
}

#[test]
fn local_fn_decl_multiple_inside_canvas() {
    let input = r#"canvas t(uv: coord) -> color {
  fn bias(x: f32) -> f32 {
    return x + 0.5;
  }
  fn scale(x: f32) -> f32 {
    return x * 0.5;
  }
  let r = bias(scale(uv.x));
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("local_fn_multiple", input);
}

#[test]
fn local_fn_decl_inside_fn_body() {
    // Local fn declarations inside a top-level fn body also work.
    let input = r#"fn outer(x: f32) -> f32 {
  fn inner(y: f32) -> f32 {
    return y * 2.0;
  }
  return inner(x) + 1.0;
}

canvas t(uv: coord) -> color {
  let v = outer(uv.x);
  compose {
    circle(at: center, radius: v * 0.1) |> fill(#ffffff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("local_fn_in_fn", input);
}

#[test]
fn local_fn_decl_wgsl_emits_helper_function() {
    // A local scalar fn should be lowered to a real WGSL helper.
    let input = r#"canvas t(uv: coord) -> color {
  fn remap(x: f32) -> f32 {
    return x * 0.5 + 0.5;
  }
  let r = remap(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;
    let wgsl = compile_to_wgsl("local_fn_wgsl_helper", input);
    assert!(
        wgsl.contains("_remap_"),
        "expected emitted WGSL helper for local fn `remap`\nwgsl:\n{wgsl}"
    );
}

#[test]
fn enum_typed_helper_signature_lowers_without_inline_fallback() {
    // Enum-typed fn params/returns should flow through the same helper lowering
    // path as scalar/vector helpers, and qualified enum variants should resolve
    // in regular expressions.
    let input = r#"enum BlendKind { Normal, Multiply, Screen }

fn choose(kind: BlendKind) -> BlendKind {
  return kind;
}

fn radius(kind: BlendKind) -> f32 {
  return 0.05 * (kind + 1.0);
}

canvas t(uv: coord) -> color {
  let selected = choose(BlendKind.Screen);
  let normal_radius = radius(BlendKind.Normal);
  let selected_radius = radius(selected);
  compose {
    fill(#101018)
    circle(at: (0.35, 0.5), radius: normal_radius) |> fill(#7fd1ff)
    circle(at: (0.65, 0.5), radius: selected_radius) |> fill(#ff9bc7)
  }
}
"#;

    let wgsl = compile_to_wgsl("enum_helper_signature", input);
    assert!(
        wgsl.contains("_choose_") || wgsl.contains("__choose_"),
        "expected enum-typed helper `choose` to lower via helper path\nwgsl:\n{wgsl}"
    );
    assert_compiles_and_validates_wgsl("enum_helper_signature_validate", input);
}

// ─── Callable fn parameters (§16.5) ──────────────────────────────────────────

#[test]
fn callable_param_fn_reference_scalar() {
    // Pass a top-level fn as a callable parameter; `apply(f, x)` dispatches through the fn ref.
    let input = r#"fn double(x: f32) -> f32 {
  return x * 2.0;
}

fn apply(f: fn(f32)->f32, x: f32) -> f32 {
  return f(x);
}

canvas t(uv: coord) -> color {
  let r = apply(double, uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("callable_param_scalar", input);
}

#[test]
fn callable_param_fn_reference_shape() {
    // Pass a shape-returning fn as a callable parameter.
    let input = r#"fn my_dot(r: f32) -> shape {
  circle(at: center, radius: r)
}

fn make_shape(f: fn(f32)->shape, r: f32) -> shape {
  f(r)
}

canvas t(uv: coord) -> color {
  compose {
    make_shape(my_dot, 0.2) |> fill(#0000ff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("callable_param_shape", input);
}

#[test]
fn callable_param_local_fn_reference() {
    // Pass a locally declared fn as a callable.
    let input = r#"fn apply(f: fn(f32)->f32, x: f32) -> f32 {
  return f(x);
}

canvas t(uv: coord) -> color {
  fn triple(x: f32) -> f32 {
    return x * 3.0;
  }
  let r = apply(triple, uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("callable_param_local", input);
}

#[test]
fn callable_param_no_args() {
    // fn() -> f32: a callable with no parameters.
    let input = r#"fn const_val() -> f32 {
  return 0.25;
}

fn invoke(f: fn()->f32) -> f32 {
  return f();
}

canvas t(uv: coord) -> color {
  let r = invoke(const_val);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("callable_param_no_args", input);
}

#[test]
fn callable_param_top_level_fn_type_annotation() {
    // Top-level fn with a callable parameter type is parsed without error.
    let input = r#"fn twice(f: fn(f32)->f32, x: f32) -> f32 {
  return f(f(x));
}

fn add1(x: f32) -> f32 {
  return x + 1.0;
}

canvas t(uv: coord) -> color {
  let r = twice(add1, uv.x);
  compose {
    circle(at: center, radius: r * 0.1) |> fill(#ffffff)
  }
}
"#;
    assert_compiles_and_validates_wgsl("callable_param_toplevel_type", input);
}

// ─── Negative tests ──────────────────────────────────────────────────────────

#[test]
fn callable_param_wrong_arity_rejected() {
    // Passing a fn with wrong arity should produce an error.
    let input = r#"fn double(x: f32, y: f32) -> f32 {
  return x + y;
}

fn apply(f: fn(f32)->f32, x: f32) -> f32 {
  return f(x);
}

canvas t(uv: coord) -> color {
  let r = apply(double, uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;
    assert_compile_fails_with("callable_param_wrong_arity", input, "arity");
}

// ─── field…at operator ───────────────────────────────────────────────────────

/// `field X at Y` must produce the same value kind as `field X` so that the
/// two results can be used in arithmetic expressions together.
/// Regression for the type normalization bug where `field fbm(p)` returned
/// `Value::Distance` but `field fbm(p) at new_coord` returned `Value::Mask`.
#[test]
fn field_at_type_matches_field() {
    let input = r#"canvas t(uv: coord) -> color {
  let e = 0.008
  let base = field fbm(coord)
  let bump = field fbm(coord) at (coord + (e, 0.0))
  // Arithmetic between `field X` (Distance) and `field X at Y` (must also
  // be Distance after normalization) must not produce a type-mismatch error.
  let slope = bump - base
  compose {
    grey(abs(slope) * 10.0)
  }
}
"#;
    assert_compiles_and_validates_wgsl("field_at_type_matches_field", input);
}

/// `field X at Y` produces correct WGSL: the offset coordinate appears in the
/// fbm call arguments rather than the ambient `p` coordinates.
#[test]
fn field_at_emits_offset_in_wgsl() {
    let input = r#"canvas t(uv: coord) -> color {
  let e = 0.004
  let bump = field fbm(coord) at (coord + (e, 0.0))
  compose {
    grey(bump)
  }
}
"#;
    let wgsl = compile_to_wgsl("field_at_offset_wgsl", input);
    // The neighbour offset must appear somewhere in the fbm call site.
    assert!(
        wgsl.contains("0.004") || wgsl.contains("4e-3"),
        "expected the `at` offset value to appear in the emitted WGSL\nwgsl:\n{wgsl}"
    );
}

// ─── `expr at coord` binary operator (variable-reference form) ───────────────

/// `let f = field …; f at coord` must compile and produce the same result as
/// the inline `field … at coord` form.  The left-hand side of `at` is a bound
/// variable rather than an inline `field` expression.
#[test]
fn field_at_variable_reference_form() {
    let input = r#"canvas t(uv: coord) -> color {
  let e = 0.008
  let base = field fbm(coord)
  // Binary `at` operator: `base` is a bound field variable, not an inline
  // `field …` expression.  Result must be the same type as `base`.
  let bump = base at (coord + (e, 0.0))
  let slope = bump - base
  compose {
    grey(abs(slope) * 10.0)
  }
}
"#;
    assert_compiles_and_validates_wgsl("field_at_var_ref", input);
}

/// The binary `at` operator on a bound variable must emit correct WGSL —
/// the offset coordinate must appear in the generated noise call, not the
/// ambient `p` coordinates.
#[test]
fn field_at_variable_reference_emits_offset_in_wgsl() {
    let input = r#"canvas t(uv: coord) -> color {
  let e = 0.004
  let base = field fbm(coord)
  let bump = base at (coord + (e, 0.0))
  compose {
    grey(bump)
  }
}
"#;
    let wgsl = compile_to_wgsl("field_at_var_ref_wgsl", input);
    assert!(
        wgsl.contains("0.004") || wgsl.contains("4e-3"),
        "expected the `at` offset value to appear in the emitted WGSL\nwgsl:\n{wgsl}"
    );
}

/// `field … at coord` (inline form) and `(let f = field …; f at coord)`
/// (variable-reference form) must produce identical WGSL output.
#[test]
fn field_at_inline_and_var_ref_produce_same_wgsl() {
    let inline = r#"canvas t(uv: coord) -> color {
  let e = 0.005
  let bump = field fbm(coord) at (coord + (e, 0.0))
  compose { grey(bump) }
}
"#;
    let var_ref = r#"canvas t(uv: coord) -> color {
  let e = 0.005
  let base = field fbm(coord)
  let bump = base at (coord + (e, 0.0))
  compose { grey(bump) }
}
"#;
    let wgsl_inline = compile_to_wgsl("field_at_inline_equiv", inline);
    let wgsl_var = compile_to_wgsl("field_at_var_equiv", var_ref);
    assert_eq!(
        wgsl_inline, wgsl_var,
        "`field … at coord` and `let f = field …; f at coord` must produce identical WGSL"
    );
}

/// The binary `at` operator must work on arithmetic combinations of field
/// values, not only on single bound variables.
#[test]
fn field_at_on_arithmetic_expression() {
    let input = r#"canvas t(uv: coord) -> color {
  let e = 0.006
  let a = field fbm(coord)
  let b = field fbm(coord * 2.0)
  // `at` applied to an arithmetic expression involving field values.
  let combined = (a + b) at (coord + (e, 0.0))
  compose {
    grey(combined)
  }
}
"#;
    assert_compiles_and_validates_wgsl("field_at_arith_expr", input);
}

/// fn returning layer + field…at: the canonical rung-1 cloudscape idiom.
/// A helper function can use `field X at Y` for slope estimation and still
/// return a fully-formed layer consumed by `compose`.
#[test]
fn fn_layer_return_with_field_at() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  fn cloud(scale: f32, tint: color) -> layer {
    let base  = field fbm(coord * scale)
    let e     = 0.006
    let bump  = field fbm(coord * scale) at (coord + (e, 0.0))
    let slope = (bump - base) / e
    let lit   = smoothstep(-1.0, 1.0, 0.5 - slope)
    layer mix(tint * 0.1, #ffffff, lit * 0.6 + 0.4) |> opacity(field smoothstep(0.0, 0.6, base - 0.3))
  }
  compose {
    fill(#113366)
    cloud(scale: 2.5, tint: #8899bb)
    cloud(scale: 4.0, tint: #99aabb)
  }
}
"#;
    assert_compiles_and_validates_wgsl("fn_layer_field_at", input);
}
