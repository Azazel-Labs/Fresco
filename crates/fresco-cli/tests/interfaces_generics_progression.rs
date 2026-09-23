use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

fn assert_compiles(suffix: &str, input: &str) {
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

#[test]
fn generic_id_function_compiles() {
    let input = r#"fn id<T>(x: T) -> T {
  return x;
}

canvas t(uv: coord) -> color {
  let r = id(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("generic_id", input);
}

#[test]
fn generic_same_type_multiple_params_compile() {
    let input = r#"fn choose_second<T>(a: T, b: T) -> T {
  return b;
}

canvas t(uv: coord) -> color {
  let r = choose_second(0.1, uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("generic_same_type", input);
}

#[test]
fn const_template_u32_drives_unroll_bound() {
    let input = r#"fn tap_sum<const TAPS: u32>(x: f32) -> f32 {
  var acc = 0.0;
  unroll for i in 0 .. TAPS {
    acc += i + x * 0.0;
  }
  return acc;
}

canvas t(uv: coord) -> color {
  let r = 0.03 + 0.001 * tap_sum<4>(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("const_template_unroll_u32", input);
}

#[test]
fn const_template_missing_specialization_is_rejected() {
    let input = r#"fn tap_sum<const TAPS: u32>(x: f32) -> f32 {
  var acc = 0.0;
  unroll for i in 0 .. TAPS {
    acc += i + x * 0.0;
  }
  return acc;
}

canvas t(uv: coord) -> color {
  let r = 0.03 + 0.001 * tap_sum(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "const_template_missing_specialization",
        input,
        "has no overload accepting 1 argument(s)",
    );
}

#[test]
fn const_template_rejects_non_integer_literal() {
    let input = r#"fn tap_sum<const TAPS: u32>(x: f32) -> f32 {
  var acc = 0.0;
  unroll for i in 0 .. TAPS {
    acc += i + x * 0.0;
  }
  return acc;
}

canvas t(uv: coord) -> color {
  let r = 0.03 + 0.001 * tap_sum<0.5>(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "const_template_non_integer",
        input,
        "must be a u32 integer literal",
    );
}

#[test]
fn const_template_i32_positive_and_negative_literal_compile() {
    let input = r#"fn offset<const DELTA: i32>(x: f32) -> f32 {
  return x + f32(DELTA);
}

canvas t(uv: coord) -> color {
  let r0 = offset<-2>(uv.x);
  let r1 = offset<3>(uv.x);
  let r = 0.03 + 0.001 * (r0 + r1);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("const_template_i32_signed_literals", input);
}

#[test]
fn const_template_i32_rejects_out_of_range_literal() {
    let input = r#"fn offset<const DELTA: i32>(x: f32) -> f32 {
  return x + f32(DELTA);
}

canvas t(uv: coord) -> color {
  let r = 0.03 + 0.001 * offset<2147483648>(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "const_template_i32_out_of_range",
        input,
        "must be a i32 integer literal",
    );
}

#[test]
fn const_template_missing_specialization_reports_template_signature() {
    let input = r#"fn offset<const DELTA: i32>(x: f32) -> f32 {
  return x + f32(DELTA);
}

canvas t(uv: coord) -> color {
  let r = 0.03 + 0.001 * offset(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "const_template_i32_missing_specialization_signature",
        input,
        "expected const template(s) for this arity: <DELTA: i32>",
    );
}

#[test]
fn const_template_specializations_compile_for_helper_keys() {
    let input = r#"fn bias<const B: i32>(x: f32) -> f32 {
  return x + f32(B) * 0.001;
}

canvas t(uv: coord) -> color {
  let a = bias<-1>(uv.x);
  let b = bias<2>(uv.x);
  let r = 0.03 + 0.001 * (a + b);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("const_template_i32_multi_specialization", input);
}

#[test]
fn const_template_identifier_argument_forwards_compile_time_value() {
    let input = r#"fn inner<const N: u32>(x: f32) -> f32 {
  return x + f32(N) * 0.001;
}

fn outer<const N: u32>(x: f32) -> f32 {
  return inner<N>(x);
}

canvas t(uv: coord) -> color {
  let r = 0.03 + 0.001 * outer<4>(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("const_template_identifier_forwarding", input);
}

#[test]
fn interface_and_conformance_enable_bound_generic() {
    let input = r#"interface Colorable {
  fn apply(x: color) -> color
}

interface MyBound {
  fn tag(x: f32) -> f32
}

conform f32 : MyBound {
  fn tag(x: f32) -> f32 {
    return x;
  }
}

fn keep<T: MyBound>(x: T) -> T {
  return x;
}

canvas t(uv: coord) -> color {
  let r = keep(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("interface_conformance", input);
}

#[test]
fn interface_conformance_allows_matching_keyword_only_method_signature() {
    let input = r#"interface Spreadable {
  fn spread(x: f32, *, gain: f32) -> f32
}

conform f32 : Spreadable {
  fn spread(x: f32, *, gain: f32) -> f32 {
    return x + gain;
  }
}

fn keep<T: Spreadable>(x: T) -> T {
  return x;
}

canvas t(uv: coord) -> color {
  let r = keep(uv.x);
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("interface_keyword_only_match", input);
}

#[test]
fn interface_conformance_rejects_keyword_only_boundary_mismatch() {
    let input = r#"interface Spreadable {
  fn spread(x: f32, *, gain: f32) -> f32
}

conform f32 : Spreadable {
  fn spread(x: f32, gain: f32) -> f32 {
    return x + gain;
  }
}

canvas t(uv: coord) -> color {
  compose {
    circle(at: center, radius: uv.x) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "interface_keyword_only_mismatch",
        input,
        "keyword-only boundary mismatch",
    );
}

#[test]
fn bound_check_rejects_non_conforming_type() {
    let input = r#"interface Sdf {
  fn sample(x: f32) -> f32
}

fn keep<T: Sdf>(x: T) -> T {
  return x;
}

canvas t(uv: coord) -> color {
  let bad = keep(#ffffff);
  compose {
    circle(at: center, radius: 0.2) |> fill(bad)
  }
}
"#;

    assert_compile_fails_with(
        "generic_bound_fail",
        input,
        "does not conform to interface `Sdf`",
    );
}

#[test]
fn interface_types_are_rejected_as_value_types() {
    let input = r#"interface Sdf {
  fn sample(x: f32) -> f32
}

fn foo(x: Sdf) -> Sdf {
  return x;
}

canvas t(uv: coord) -> color {
  compose {
    circle(at: center, radius: 0.2) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "interface_dyn_reject",
        input,
        "interface `Sdf` cannot be used as a value type in v1",
    );
}

#[test]
fn typevar_typed_declaration_compiles() {
    // A generic function should be able to use its type parameter as a declared
    // variable type anywhere a variable type may appear (not just fn param positions).
    let input = r#"fn pass_through<T>(x: T) -> T {
  T y = x
  return y
}

canvas t(uv: coord) -> color {
  let r = pass_through(uv.x)
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("typevar_typed_decl", input);
}

#[test]
fn enum_typed_declaration_compiles() {
    // A user-defined enum type should be usable as a declared variable type
    // everywhere a variable type may appear, not only in fn parameter positions.
    let input = r#"enum BlendKind { Normal, Multiply, Screen }

fn identity_blend(w: BlendKind) -> BlendKind {
  BlendKind result = w
  return result
}

canvas t(uv: coord) -> color {
  let r = uv.x
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("enum_typed_decl", input);
}

#[test]
fn qualified_enum_variant_is_usable_in_normal_expressions() {
    let input = r#"enum BlendKind { Normal, Multiply, Screen }

canvas t(uv: coord) -> color {
  BlendKind mode = BlendKind.Screen
  let r = 0.08 * (mode + 1.0)
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("qualified_enum_variant_expr", input);
}

#[test]
fn ambiguous_bare_enum_variant_is_rejected_in_expressions() {
    let input = r#"enum AxisA { down }
enum AxisB { down }

canvas t(uv: coord) -> color {
  let v = down
  compose {
    circle(at: center, radius: v + 0.05) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "ambiguous_bare_enum_variant",
        input,
        "ambiguous enum variant `down`",
    );
}

#[test]
fn match_arms_accept_mixed_shorthand_and_qualified_enum_variants() {
    let input = r#"enum LightKind { directional, point, spot }

fn weight(kind: LightKind) -> f32 {
  match kind {
    directional: { return 1.0 }
    LightKind.point: { return 2.0 }
    spot: { return 3.0 }
  }
}

canvas t(uv: coord) -> color {
  let r = 0.05 + 0.01 * weight(LightKind.directional)
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("enum_match_mixed_shorthand_and_qualified", input);
}

#[test]
fn match_without_default_rejects_non_exhaustive_enum_arms() {
    let input = r#"enum LightKind { directional, point, spot }

fn weight(kind: LightKind) -> f32 {
  match kind {
    directional: { return 1.0 }
    LightKind.point: { return 2.0 }
  }
  return 0.0
}

canvas t(uv: coord) -> color {
  let r = 0.05 + 0.01 * weight(LightKind.directional)
  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "enum_match_non_exhaustive",
        input,
        "non-exhaustive enum match on `LightKind`; missing variant(s): spot",
    );
}

#[test]
fn axis_group_declaration_is_rejected() {
    let input = r#"axis_group direct_lit {
  ambient: flat|sh2|lightmap|lightmap_dir
}

canvas t(uv: coord) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with("axis_group_rejected", input, "axis_group");
}

#[test]
fn pass_use_block_is_rejected() {
    let input = r#"pass fwd_base {
  use direct_lit { fog }
}

canvas t(uv: coord) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with("pass_use_rejected", input, "use");
}

#[test]
fn pipeline_use_block_is_rejected() {
    let input = r#"pipeline forward {
  use direct_lit { fog }
}

canvas t(uv: coord) -> color {
  compose {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with("pipeline_use_rejected", input, "use");
}

#[test]
fn enum_function_call_rejects_wrong_enum_type() {
    let input = r#"enum BlendKind { Normal, Multiply, Screen }
enum ToneKind { Warm, Cool }

fn choose(kind: BlendKind) -> BlendKind {
  return kind
}

canvas t(uv: coord) -> color {
  let selected = choose(ToneKind.Warm)
  compose {
    circle(at: center, radius: 0.08 * (selected + 1.0)) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "enum_fn_call_wrong_enum",
        input,
        "argument `kind` to `choose` expected enum `BlendKind`, found variant `ToneKind.Warm`",
    );
}

#[test]
fn enum_function_call_rejects_non_variant_scalar() {
    let input = r#"enum BlendKind { Normal, Multiply, Screen }

fn choose(kind: BlendKind) -> BlendKind {
  return kind
}

canvas t(uv: coord) -> color {
  let selected = choose(1.5)
  compose {
    circle(at: center, radius: 0.08 * (selected + 1.0)) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "enum_fn_call_bad_scalar",
        input,
        "argument `kind` to `choose` is not a valid `BlendKind` variant value",
    );
}

#[test]
fn enum_function_return_rejects_invalid_variant_value() {
    let input = r#"enum BlendKind { Normal, Multiply, Screen }

fn bad(kind: BlendKind) -> BlendKind {
  return 7.0
}

canvas t(uv: coord) -> color {
  let selected = bad(BlendKind.Normal)
  compose {
    circle(at: center, radius: 0.08 * (selected + 1.0)) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "enum_fn_return_bad_variant",
        input,
        "function `bad` return value is not a valid `BlendKind` variant value",
    );
}

#[test]
fn generic_function_over_field_type_compiles() {
    let input = r#"fn field_identity<T>(x: field<T>) -> field<T> {
  field<T> y = x
  return y
}

canvas t(uv: coord) -> color {
  let sample_value = field_identity(field fbm(uv))
  let t = field_identity(field (uv.x * 0.5))
  compose {
    circle(at: center, radius: (sample_value + t) * 0.08 + 0.18) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("generic_field_type", input);
}

#[test]
fn field_type_requires_single_type_argument() {
    let input = r#"fn bad(x: field<f32, color>) -> f32 {
  return 0.0
}

canvas t(uv: coord) -> color {
  compose {
    circle(at: center, radius: 0.2) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "generic_field_type_arity",
        input,
        "`field<...>` expects exactly one type argument",
    );
}
