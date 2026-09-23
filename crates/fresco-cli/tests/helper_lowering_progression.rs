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
        "expected helper-lowering compile to succeed\nstdout:\n{}\nstderr:\n{}",
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
        "expected helper-lowering compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("expected utf-8 WGSL output")
}

fn assert_intrinsic_call_count_at_most(wgsl: &str, needle: &str, max_count: usize, label: &str) {
    let count = wgsl.matches(needle).count();
    assert!(
        count <= max_count,
        "expected at most {max_count} {label} intrinsic call(s) ({needle}), found {count}\nwgsl:\n{wgsl}"
    );
}

fn assert_compile_fails_with_code(
    suffix: &str,
    input: &str,
    expected_code: &str,
    expected_in_stderr: &str,
) -> String {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected helper-lowering compile to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(expected_code),
        "expected stderr to contain code `{expected_code}`\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains(expected_in_stderr),
        "expected stderr to contain `{expected_in_stderr}`\nstderr:\n{stderr}"
    );
    stderr
}

fn assert_called_helper(wgsl: &str, prefix: &str, matrix_first: bool) {
    let module = naga::front::wgsl::parse_str(wgsl).expect("helper WGSL must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("helper WGSL must validate");
    let (handle, helper) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref().is_some_and(|n| n.starts_with(prefix)))
        .expect("expected typed helper definition");
    assert_eq!(helper.arguments.len(), 2);
    for (index, arg) in helper.arguments.iter().enumerate() {
        let ty = &module.types[arg.ty].inner;
        if index == 0 && matrix_first {
            assert!(matches!(
                ty,
                naga::TypeInner::Matrix {
                    columns: naga::VectorSize::Tri,
                    rows: naga::VectorSize::Tri,
                    scalar: naga::Scalar::F32
                }
            ));
        } else {
            assert!(matches!(
                ty,
                naga::TypeInner::Vector {
                    size: naga::VectorSize::Tri,
                    scalar: naga::Scalar::F32
                }
            ));
        }
    }
    assert!(matches!(
        module.types[helper.result.as_ref().expect("scalar result").ty].inner,
        naga::TypeInner::Scalar(naga::Scalar::F32)
    ));
    fn calls(block: &naga::Block, target: naga::Handle<naga::Function>) -> bool {
        block.iter().any(|stmt| match stmt {
            naga::Statement::Call { function, .. } => *function == target,
            naga::Statement::Block(b) => calls(b, target),
            naga::Statement::If { accept, reject, .. } => {
                calls(accept, target) || calls(reject, target)
            }
            naga::Statement::Switch { cases, .. } => cases.iter().any(|c| calls(&c.body, target)),
            naga::Statement::Loop {
                body, continuing, ..
            } => calls(body, target) || calls(continuing, target),
            _ => false,
        })
    }
    assert!(
        module
            .entry_points
            .iter()
            .any(|e| calls(&e.function.body, handle))
            || module
                .functions
                .iter()
                .any(|(h, f)| h != handle && calls(&f.body, handle)),
        "helper must be called, not merely declared"
    );
}

#[test]
fn helper_lowering_emits_vector_signature_helper_calls() {
    let input = r#"fn sd_mock(p: vec3, b: vec3) -> f32 {
  return p.x + b.y;
}

canvas t(uv: coord, time: signal) -> color {
  let pos = vec3(uv.x, uv.y, sin(time));
  compose {
    circle(at: center, radius: 0.1 + 0.01 * sd_mock(pos, vec3(1.0, 2.0, 3.0))) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("helper_lowering_vecsig");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected helper-lowering compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert_called_helper(&wgsl, "_sd_mock_", false);
}

#[test]
fn helper_lowering_emits_matrix_signature_helper_calls() {
    let input = r#"fn apply_m3(m: mat3, v: vec3) -> f32 {
  let r = m * v;
  return r.x + r.y + r.z;
}

canvas t(uv: coord, time: signal) -> color {
  let m = mat3(
    vec3(1.0, 0.0, 0.0),
    vec3(0.0, 1.0, 0.0),
    vec3(0.0, 0.0, 1.0)
  );
  let p = vec3(uv.x, uv.y, sin(time));
  let d = apply_m3(m, p);
  compose {
    circle(at: center, radius: 0.1 + 0.01 * d) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("helper_lowering_matsig");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected helper-lowering compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert_called_helper(&wgsl, "_apply_m3_", true);
}

#[test]
fn helper_lowering_preserves_vec3_dot_normalize_pattern() {
    let input = r#"fn lambert(n: vec3, l: vec3) -> f32 {
  return dot(normalize(n), normalize(l));
}
canvas t(uv: coord, time: signal) -> color {
  let n = vec3(uv.x - 0.5, uv.y - 0.5, 1.0);
  let l = vec3(0.5, 0.25, 1.0);
  let d = lambert(n, l);
  compose {
    circle(at: center, radius: 0.08 + 0.02 * d) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("helper_lowering_vec3_dot_normalize");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected helper-lowering compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert_called_helper(&wgsl, "_lambert_", false);
    assert!(
        (wgsl.contains("dot(vec3<f32>(") && wgsl.contains("normalize(vec3<f32>("))
            || wgsl.contains("dot(normalize(arg_n), normalize(arg_l))")
            || wgsl.contains("dot(normalize(loc_n), normalize(loc_l))")
            || (wgsl.contains("dot(vec3<f32>(") && wgsl.contains("length(vec3<f32>("))
            || ((wgsl.contains("arg_n_x /") || wgsl.contains("arg_n.x /"))
                && (wgsl.contains("arg_l_x /") || wgsl.contains("arg_l.x /"))
                && (wgsl.contains("sqrt((((arg_n_x * arg_n_x)")
                    || wgsl.contains("sqrt((((arg_n.x * arg_n.x)"))
                && (wgsl.contains("sqrt((((arg_l_x * arg_l_x)")
                    || wgsl.contains("sqrt((((arg_l.x * arg_l.x)"))),
        "expected vec3 normalize-dot math shape to survive helper lowering\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_preserves_vec2_dot_self_shape() {
    let input = r#"fn len2_sq(v: vec2) -> f32 {
  return dot(v, v);
}

canvas t(uv: coord, time: signal) -> color {
  let d = len2_sq(vec2(uv.x - 0.5, uv.y - 0.5));
  compose {
    circle(at: center, radius: 0.03 + 0.01 * d) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec2_dot_self_shape", input);
    assert!(
        wgsl.contains("dot(arg_v, arg_v)") || wgsl.contains("dot(loc_v, loc_v)"),
        "expected helper lowering to keep dot(v, v) without rebuilding vec2 from swizzles\nwgsl:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("dot(vec2<f32>(arg_v.x, arg_v.y), vec2<f32>(arg_v.x, arg_v.y))"),
        "unexpected redundant vec2 swizzle-compose pattern for dot(v, v)\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_emits_nested_call_usage_helpers() {
    let input = r#"fn inner(x: f32) -> f32 {
  return x * x;
}

fn outer(x: f32) -> f32 {
  return x + 1.0;
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.1 + 0.02 * outer(inner(sin(time)))) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("helper_lowering_nested");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected nested-helper compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("fn _inner_1_"),
        "expected emitted WGSL helper function for inner\nwgsl:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fn _outer_1_"),
        "expected emitted WGSL helper function for outer\nwgsl:\n{wgsl}"
    );
    assert!(
        wgsl.contains("_inner_1_(") && wgsl.contains("_outer_1_("),
        "expected emitted WGSL call sites for nested helper usage\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_preserves_vec3_max_scalar_shape() {
    let input = r#"fn box_fold(p: vec3, b: vec3) -> f32 {
  let q = abs(p) - b;
  return length(max(q, 0.0));
}

canvas t(uv: coord, time: signal) -> color {
  let p = vec3(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2);
  let d = box_fold(p, vec3(0.2, 0.2, 0.2));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * d) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec3_max_scalar_shape", input);
    assert!(
        wgsl.contains("max(") && wgsl.contains("vec3<f32>(0f"),
        "expected helper lowering to keep vec3 max(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "length(max(", 3, "vec3 max");
}

#[test]
fn helper_lowering_preserves_vec2_max_scalar_shape() {
    let input = r#"fn fold2(p: vec2) -> f32 {
  let q = abs(p) - vec2(0.2, 0.2);
  return length(max(q, 0.0));
}

canvas t(uv: coord, time: signal) -> color {
  let d = fold2(vec2(uv.x - 0.5, uv.y - 0.5));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * d) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec2_max_scalar_shape", input);
    assert!(
        wgsl.contains("max(") && wgsl.contains("vec2<f32>(0f"),
        "expected helper lowering to keep vec2 max(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "length(max(", 2, "vec2 max");
}

#[test]
fn helper_lowering_preserves_vec4_max_scalar_shape() {
    let input = r#"fn fold4(p: vec4) -> f32 {
  let q = abs(p) - vec4(0.2, 0.2, 0.2, 0.2);
  return length(max(q, 0.0));
}

canvas t(uv: coord, time: signal) -> color {
  let d = fold4(vec4(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2, 1.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * d) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec4_max_scalar_shape", input);
    assert!(
        wgsl.contains("max(") && wgsl.contains("vec4<f32>(0f"),
        "expected helper lowering to keep vec4 max(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "length(max(", 4, "vec4 max");
}

#[test]
fn helper_lowering_preserves_vec3_min_scalar_shape() {
    let input = r#"fn box_fold_min(p: vec3, b: vec3) -> f32 {
  let q = abs(p) - b;
  return length(min(q, 0.0));
}

canvas t(uv: coord, time: signal) -> color {
  let p = vec3(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2);
  let d = box_fold_min(p, vec3(0.2, 0.2, 0.2));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * abs(d)) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec3_min_scalar_shape", input);
    assert!(
        wgsl.contains("min(") && wgsl.contains("vec3<f32>(0f"),
        "expected helper lowering to keep vec3 min(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "length(min(", 3, "vec3 min");
}

#[test]
fn helper_lowering_preserves_vec2_min_scalar_shape() {
    let input = r#"fn fold2_min(p: vec2) -> f32 {
  let q = abs(p) - vec2(0.2, 0.2);
  return length(min(q, 0.0));
}

canvas t(uv: coord, time: signal) -> color {
  let d = fold2_min(vec2(uv.x - 0.5, uv.y - 0.5));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * abs(d)) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec2_min_scalar_shape", input);
    assert!(
        wgsl.contains("min(") && wgsl.contains("vec2<f32>(0f"),
        "expected helper lowering to keep vec2 min(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "length(min(", 2, "vec2 min");
}

#[test]
fn helper_lowering_preserves_vec4_min_scalar_shape() {
    let input = r#"fn fold4_min(p: vec4) -> f32 {
  let q = abs(p) - vec4(0.2, 0.2, 0.2, 0.2);
  return length(min(q, 0.0));
}

canvas t(uv: coord, time: signal) -> color {
  let d = fold4_min(vec4(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2, 1.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * abs(d)) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec4_min_scalar_shape", input);
    assert!(
        wgsl.contains("min(") && wgsl.contains("vec4<f32>(0f"),
        "expected helper lowering to keep vec4 min(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "length(min(", 4, "vec4 min");
}

#[test]
fn helper_lowering_preserves_vec3_clamp_shape() {
    let input = r#"fn clamp_rgb(v: vec3) -> vec3 {
  return clamp(v, vec3(0.0), vec3(1.0));
}

canvas t(uv: coord, time: signal) -> color {
  let rgb = clamp_rgb(vec3(uv.x, uv.y, sin(time) * 0.5 + 0.5));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * rgb.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec3_clamp_shape", input);
    assert!(
        wgsl.contains("clamp(vec3<f32>(") && wgsl.contains("vec3<f32>(1f"),
        "expected helper lowering to keep vec3 clamp shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "clamp(vec3<f32>(", 3, "vec3 clamp");
}

#[test]
fn helper_lowering_preserves_vec2_clamp_shape() {
    let input = r#"fn clamp_rg(v: vec2) -> vec2 {
  return clamp(v, vec2(0.0), vec2(1.0));
}

canvas t(uv: coord, time: signal) -> color {
  let rg = clamp_rg(vec2(uv.x, uv.y));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * rg.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec2_clamp_shape", input);
    assert!(
        wgsl.contains("clamp(vec2<f32>(") && wgsl.contains("vec2<f32>(1f"),
        "expected helper lowering to keep vec2 clamp shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "clamp(vec2<f32>(", 2, "vec2 clamp");
}

#[test]
fn helper_lowering_preserves_vec4_clamp_shape() {
    let input = r#"fn clamp_rgba(v: vec4) -> vec4 {
  return clamp(v, vec4(0.0), vec4(1.0));
}

canvas t(uv: coord, time: signal) -> color {
  let rgba = clamp_rgba(vec4(uv.x, uv.y, sin(time) * 0.5 + 0.5, 1.0));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * rgba.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec4_clamp_shape", input);
    assert!(
        wgsl.contains("clamp(vec4<f32>(") && wgsl.contains("vec4<f32>(1f"),
        "expected helper lowering to keep vec4 clamp shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "clamp(vec4<f32>(", 4, "vec4 clamp");
}

#[test]
fn helper_lowering_preserves_normalize_chain_shape() {
    let input = r#"fn normalize_twice(v: vec3) -> vec3 {
  return normalize(normalize(v));
}

canvas t(uv: coord, time: signal) -> color {
  let n = normalize_twice(vec3(uv.x - 0.5, uv.y - 0.5, 1.0));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * n.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_normalize_chain_shape", input);
    // Both normalizes must survive (inner may use param shortcut e.g. normalize(arg_v)).
    let normalize_count = wgsl.matches("normalize(").count();
    assert!(
        normalize_count >= 2,
        "expected helper lowering to keep chained normalize(vec3) calls\nwgsl:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("return vec3<f32>(_e") && !wgsl.contains("return vec3<f32>(loc_"),
        "expected helper vec3 return to avoid decompose-recompose pattern\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_preserves_vec2_normalize_chain_shape() {
    let input = r#"fn normalize_twice2(v: vec2) -> vec2 {
  return normalize(normalize(v));
}

canvas t(uv: coord, time: signal) -> color {
  let n = normalize_twice2(vec2(uv.x - 0.5, uv.y - 0.5));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * n.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec2_normalize_chain_shape", input);
    // Both normalizes must survive (inner may use param shortcut e.g. normalize(arg_v)).
    let normalize_count = wgsl.matches("normalize(").count();
    assert!(
        normalize_count >= 2,
        "expected helper lowering to keep chained normalize(vec2) calls\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_preserves_vec4_normalize_chain_shape() {
    let input = r#"fn normalize_twice4(v: vec4) -> vec4 {
  return normalize(normalize(v));
}

canvas t(uv: coord, time: signal) -> color {
  let n = normalize_twice4(vec4(uv.x - 0.5, uv.y - 0.5, 1.0, 1.0));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * n.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_vec4_normalize_chain_shape", input);
    // Both normalizes must survive (inner may use param shortcut e.g. normalize(arg_v)).
    let normalize_count = wgsl.matches("normalize(").count();
    assert!(
        normalize_count >= 2,
        "expected helper lowering to keep chained normalize(vec4) calls\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_statement_ir_lowers_if_for_break_and_assignments() {
    let input = r#"fn march(ro: vec3, rd: vec3) -> vec2 {
  vec2 res = vec2(-1.0, -1.0);
  float tmin = 0.1;
  float tmax = 5.0;

  float tp = (0.8 - ro.y) / rd.y;
  if (tp > 0.0) {
    tmax = min(tmax, tp);
  }

  float t = tmin;
  for i in 0 .. 24 {
    float h = abs(t - 1.2);
    if (h < 0.001) {
      res = vec2(t, 2.0);
      break;
    }
    t += clamp(h, 0.01, 0.2);
    if (t > tmax) break;
  }

  return res;
}

canvas t(uv: coord, time: signal) -> color {
  let ro = vec3(0.0, 0.3, -2.0);
  let rd = normalize(vec3(uv.x - 0.5, uv.y - 0.5, 1.0));
  let hit = march(ro, rd);
  compose {
    circle(at: center, radius: 0.02 + 0.03 * clamp(hit.x, 0.0, 1.0)) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_and_validates_wgsl("helper_stmt_ir_if_for_break", input);
}

#[test]
fn helper_lowering_nested_calls_in_branches_are_scope_safe() {
    let input = r#"fn sd_box(p: vec3, b: vec3) -> f32 {
  let q = abs(p) - b;
  return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn map(pos: vec3) -> vec2 {
  vec2 res = vec2(pos.y, 0.0);
  if (sd_box(pos - vec3(0.0, 0.2, 0.0), vec3(0.2, 0.2, 0.2)) < res.x) {
    res = vec2(sd_box(pos - vec3(0.0, 0.2, 0.0), vec3(0.2, 0.2, 0.2)), 1.0);
  } else {
    res = vec2(sd_box(pos - vec3(0.3, 0.2, 0.0), vec3(0.15, 0.15, 0.15)), 2.0);
  }
  return res;
}

canvas t(uv: coord, time: signal) -> color {
  let p = vec3(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2);
  let h = map(p);
  compose {
    circle(at: center, radius: 0.03 + 0.02 * clamp(h.x, 0.0, 1.0)) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_and_validates_wgsl("helper_scope_cache_branch_calls", input);
}

#[test]
fn helper_raycast_map_ibox_chain_stress_regression_compiles() {
    let input = r#"fn iBox(ro: vec3, rd: vec3, rad: vec3) -> vec2 {
  let m = 1.0 / rd;
  let n = m * ro;
  let k = abs(m) * rad;
  let t1 = -n - k;
  let t2 = -n + k;
  let tN = max(max(t1.x, t1.y), t1.z);
  let tF = min(min(t2.x, t2.y), t2.z);
  if (tN > tF || tF < 0.0) return vec2(-1.0, -1.0);
  return vec2(tN, tF);
}

fn map(pos: vec3) -> vec2 {
  vec2 res = vec2(pos.y, 0.0);
  vec2 ib = iBox(pos - vec3(0.0, 0.35, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.35));
  if (ib.x > 0.0 && ib.x < res.x) {
    res = vec2(ib.x, 1.0);
  }
  return res;
}

fn raycast(ro: vec3, rd: vec3) -> vec2 {
  vec2 hit = vec2(-1.0, -1.0);
  float t = 0.2;
  for (int i = 0; i < 56; i++) {
    vec2 h = map(ro + rd * t);
    if (abs(h.x) < 0.0005 * t) {
      hit = vec2(t, h.y);
      break;
    }
    t += h.x;
    if (t > 15.0) break;
  }
  return hit;
}

canvas t(uv: coord, time: signal) -> color {
  let ro = vec3(0.0, 0.6, -2.0);
  let rd = normalize(vec3(uv.x - 0.5, uv.y - 0.5, 1.0));
  let h = raycast(ro, rd);
  compose {
    circle(at: center, radius: 0.02 + 0.02 * clamp(h.x, 0.0, 1.0)) |> fill(#ffffff)
  }
}
"#;

    assert_compiles_and_validates_wgsl("helper_stress_raycast_map_ibox", input);
}

#[test]
fn helper_stmt_ir_reports_unsupported_statement_with_helper_context() {
    let input = r#"fn bad_helper(x: f32) -> f32 {
  compose {
    circle(at: center, radius: 0.1 + x) |> fill(#ffffff)
  }
  return x;
}

canvas t(uv: coord, time: signal) -> color {
  let v = bad_helper(uv.x);
  compose {
    circle(at: center, radius: 0.03 + 0.01 * v) |> fill(#ffffff)
  }
}
"#;

    let diagnostic = assert_compile_fails_with_code(
        "helper_neg_unsupported_stmt",
        input,
        "E_HELPER_UNSUPPORTED_STMT",
        "statement `compose` is not supported yet",
    );
    assert!(diagnostic.contains("helper `bad_helper`"), "{diagnostic}");
}

#[test]
fn helper_stmt_ir_reports_assignment_arity_mismatch_with_helper_context() {
    let input = r#"fn bad_assign(v: vec2) -> f32 {
  vec2 q = v;
  q = 1.0;
  return q.x;
}

canvas t(uv: coord, time: signal) -> color {
  let v = bad_assign((uv.x, uv.y));
  compose {
    circle(at: center, radius: 0.03 + 0.01 * v) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with_code(
        "helper_neg_assign_arity",
        input,
        "E_HELPER_ASSIGN_ARITY",
        "expects 2 component(s), but got 1",
    );
}

#[test]
fn helper_stmt_ir_reports_break_outside_loop_with_code() {
    let input = r#"fn bad_break(x: f32) -> f32 {
  break;
  return x;
}

canvas t(uv: coord, time: signal) -> color {
  let v = bad_break(uv.x);
  compose {
    circle(at: center, radius: 0.03 + 0.01 * v) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with_code(
        "helper_neg_break_outside_loop",
        input,
        "E_HELPER_BREAK_OUTSIDE_LOOP",
        "is only valid inside loop bodies",
    );
}

#[test]
fn helper_for_loop_break_cap_uses_live_continuing_index() {
    let input = r#"fn sum_steps() -> f32 {
    float acc = 0.0;
    for i in 0 .. 4 {
    acc += i;
    }
    return acc;
  }

  canvas t(uv: coord, time: signal) -> color {
    let v = sum_steps();
    compose {
    circle(at: center, radius: 0.02 + 0.005 * v) |> fill(#ffffff)
    }
  }
  "#;

    let wgsl = compile_to_wgsl("helper_loop_cap_shape", input);

    let loop_pos = wgsl
        .find("loop {")
        .expect("expected helper loop in emitted WGSL");
    let pre_start = loop_pos.saturating_sub(200);
    let pre_loop = &wgsl[pre_start..loop_pos];
    let stale_capture = pre_loop.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("let ") && trimmed.contains("_hl_loop_idx")
    });
    assert!(
        !stale_capture,
        "unexpected pre-loop let-capture of loop index; expected live continuing break cap\npre-loop snippet:\n{pre_loop}\nwgsl:\n{wgsl}"
    );

    let continuing_rel = wgsl[loop_pos..]
        .find("continuing {")
        .expect("expected continuing block in emitted helper loop");
    let continuing_pos = loop_pos + continuing_rel;
    let continuing_end = (continuing_pos + 260).min(wgsl.len());
    let continuing_block = &wgsl[continuing_pos..continuing_end];

    assert!(
        continuing_block.contains("loop_idx ="),
        "expected continuing block to increment helper loop index\ncontinuing:\n{continuing_block}\nwgsl:\n{wgsl}"
    );
    assert!(
        continuing_block.contains("break if ("),
        "expected continuing block to contain loop break cap\ncontinuing:\n{continuing_block}\nwgsl:\n{wgsl}"
    );
    assert!(
        continuing_block.contains("_hl_loop_idx") || continuing_block.contains("loop_idx"),
        "expected break cap path to reference live loop index state\ncontinuing:\n{continuing_block}\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_omits_along_accumulator_for_dist_only_paths() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
    arc center: (0.72, 0.55) radius: 0.10 sweep: 200deg
  }

  let d = curve.dist

  compose {
    grey(smoothstep(lo: 0.03, hi: 0.0, x: d))
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_path_dist_only", input);
    assert!(
        wgsl.contains("fn fresco_path_sample_"),
        "expected dist-only path lowering to emit a shared path sample helper\nwgsl:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("path_best_along"),
        "did not expect an along accumulator when only dist is demanded\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_skips_path_tables_when_only_length_is_used() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
    arc center: (0.72, 0.55) radius: 0.10 sweep: 200deg
  }

  let l = curve.length
  compose {
    circle(at: center, radius: 0.02 + 0.00001 * l) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_path_length_only", input);
    assert!(
        !wgsl.contains("FrescoPathSeg"),
        "did not expect path segment struct when no path geometry sampling is demanded\nwgsl:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("const fresco_path_") && !wgsl.contains("fresco_path_buf_"),
        "did not expect emitted path tables when only path.length is used\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_uses_shared_arc_sample_helper_for_point_and_tangent_at() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
    arc center: (0.72, 0.55) radius: 0.10 sweep: 200deg
  }

  let sample_value = fract(time * 0.2) * curve.length
  let p = point_at(path: curve, s: sample_value)
  let tng = tangent_at(path: curve, s: sample_value)
  let wobble = 0.01 * (tng.x + tng.y)

  compose {
    circle(at: p, radius: 0.02 + wobble) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("helper_path_arc_sample", input);
    assert_eq!(
        wgsl.matches("fn fresco_path_arc_sample_").count(),
        1,
        "expected exactly one emitted arc sample helper function\nwgsl:\n{wgsl}"
    );
    assert!(
        wgsl.matches("fresco_path_arc_sample_").count() >= 2,
        "expected emitted WGSL to include both arc helper definition and at least one call site\nwgsl:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("path_best_dist"),
        "did not expect nearest-sample helper locals for point_at/tangent_at-only demand\nwgsl:\n{wgsl}"
    );
}

#[test]
fn helper_lowering_explain_has_no_placeholder_path_eval_note() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
    arc center: (0.72, 0.55) radius: 0.10 sweep: 200deg
  }

  let sample_value = fract(time * 0.2) * curve.length
  let p = point_at(path: curve, s: sample_value)
  let tng = tangent_at(path: curve, s: sample_value)

  compose {
    circle(at: p, radius: 0.02 + 0.01 * (tng.x + tng.y)) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("helper_path_no_placeholder_note");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected helper-lowering compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        !explain.contains("temporary placeholder evaluator"),
        "did not expect legacy placeholder-evaluator explain note\nexplain:\n{explain}"
    );
}
