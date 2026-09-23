use std::{fs, path::PathBuf, process::Command};

fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[path = "support/common.rs"]
mod common;
use common::unique_temp_path;

fn run_fresco(input_path: &PathBuf) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    Command::new(bin)
        .arg(input_path)
        .arg("--emit")
        .arg("wgsl")
        .output()
        .expect("failed to run fresco binary")
}

#[test]
fn top_level_fn_decl_parses_when_unused() {
    let input = r#"fn sparkle(amount: f32) -> f32 {
  amount
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_decl_unused");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected unused fn declaration to parse and compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_block_allows_newline_before_open_brace() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose
  {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("compose_newline_before_brace");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose block with newline before `{{` to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_call_compiles_for_scalar_helpers() {
    let input = r#"fn sparkle(amount: f32) -> f32 {
  amount
}

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_call_stub");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scalar fn call to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_call_compiles_for_vec2_helpers() {
    let input = r#"fn wobble(center: vec2, offset: vec2) -> vec2 {
  center + offset
}

canvas t(uv: coord, time: signal) -> color {
  let p = wobble(center, (0.02, 0.0))
  compose {
    circle(at: p, radius: 18px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_call_vec2");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vec2 fn call to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_call_accepts_keyword_only_parameter_by_name() {
    let input = r#"fn grade(amount: f32, *, boost: f32) -> f32 {
  amount + boost
}

canvas t(uv: coord, time: signal) -> color {
  let k = grade(0.25, boost: 0.5)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_keyword_only_named");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected keyword-only parameter call by name to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_call_rejects_positional_argument_for_keyword_only_parameter() {
    let input = r#"fn grade(amount: f32, *, boost: f32) -> f32 {
  amount + boost
}

canvas t(uv: coord, time: signal) -> color {
  let k = grade(0.25, 0.5)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_keyword_only_positional_reject");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected positional argument for keyword-only parameter to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("missing argument `boost`"),
        "expected missing keyword-only argument diagnostic\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("keyword-only"),
        "expected keyword-only help diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_decl_rejects_trailing_keyword_only_separator() {
    let input = r#"fn grade(amount: f32, *) -> f32 {
  amount
}

canvas t(uv: coord, time: signal) -> color {
  let k = grade(0.25)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_keyword_only_trailing_star");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected trailing keyword-only separator to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("must be followed by at least one keyword-only parameter"),
        "expected trailing `*` parser diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_decl_rejects_duplicate_keyword_only_separator() {
    let input = r#"fn grade(amount: f32, *, *, boost: f32) -> f32 {
  amount + boost
}

canvas t(uv: coord, time: signal) -> color {
  let k = grade(0.25, boost: 0.5)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_keyword_only_duplicate_star");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected duplicate keyword-only separator to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("duplicate `*` keyword-only separator in parameter list"),
        "expected duplicate `*` parser diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn local_fn_decl_rejects_trailing_keyword_only_separator() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  fn grade(amount: f32, *) -> f32 {
    amount
  }

  let k = grade(uv.x)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("local_fn_keyword_only_trailing_star");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected local fn trailing keyword-only separator to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("must be followed by at least one keyword-only parameter"),
        "expected local fn trailing `*` parser diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn local_fn_decl_rejects_duplicate_keyword_only_separator() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  fn grade(amount: f32, *, *, boost: f32) -> f32 {
    amount + boost
  }

  let k = grade(uv.x, boost: 0.1)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("local_fn_keyword_only_duplicate_star");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected local fn duplicate keyword-only separator to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("duplicate `*` keyword-only separator in parameter list"),
        "expected local fn duplicate `*` parser diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_call_reports_return_type_mismatch() {
    let input = r#"fn bad(amount: f32) -> vec2 {
  amount
}

canvas t(uv: coord, time: signal) -> color {
  let _p = bad(0.2)
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_return_mismatch");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected return type mismatch to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("declared return type is vec2"),
        "expected explicit return type mismatch diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_body_reports_compose_statement_as_unsupported() {
    let input = r#"fn build() -> f32 {
  compose {
    circle(at: center, radius: 10px) |> fill(#ffffff)
  }
  1.0
}

canvas t(uv: coord, time: signal) -> color {
  let _k = build()
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_body_compose_unsupported");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unsupported compose statement in fn body to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("statement `compose` is not supported yet"),
        "expected precise unsupported statement diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_call_compiles_with_explicit_return_statement() {
    let input = r#"fn sparkle(amount: f32) -> f32 {
  return amount
}

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_return_stmt_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explicit return statement to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_reports_unreachable_statement_after_return() {
    let input = r#"fn sparkle(amount: f32) -> f32 {
  return amount
  let extra = 1.0
  extra
}

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_unreachable_after_return");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unreachable statement after return to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unreachable `let` statement after `return`"),
        "expected unreachable-after-return diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_reports_missing_return_on_fallthrough_path() {
    let input = r#"fn sparkle(amount: f32) -> f32 {
  let v = amount
}

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_missing_return");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected missing return path to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("not all control-flow paths in function `sparkle` return a value"),
        "expected all-path return diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_if_else_returns_scalar_compiles() {
    let input = r#"fn sparkle(amount: f32, time: signal) -> f32 {
  if sin(time) {
    return amount * 2.0
  } else {
    return amount * 0.5
  }
}

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0, time)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_if_else_returns_scalar");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected if/else with return statements in function body to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_if_else_reports_branch_value_type_mismatch() {
    let input = r#"fn bad(amount: f32, time: signal) -> f32 {
  if sin(time) {
    return amount
  } else {
    return center
  }
}

canvas t(uv: coord, time: signal) -> color {
  let _k = bad(1.0, time)
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_if_else_value_mismatch");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected function if-branch value kind mismatch to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("incompatible `if` branch value kinds"),
        "expected branch value mismatch diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_for_loop_body_without_return_compiles() {
    let input = r#"fn sparkle(amount: f32) -> f32 {
  for i in 0 .. 4 {
    let probe = amount + i
    probe
  }
  amount
}

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_for_loop_body_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected function for-loop body without returns to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_for_loop_reports_return_in_body_as_unsupported() {
    let input = r#"fn sparkle(amount: f32) -> f32 {
  for i in 0 .. 4 {
    return amount
  }
  amount
}

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("fn_for_loop_return_unsupported");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected return in function for-loop body to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("does not support `return` inside `for` bodies yet"),
        "expected return-in-loop diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn canvas_resolution_param_compiles_and_is_callable_in_functions() {
    let input = r#"fn aspect(res: resolution) -> f32 {
  res.x / max(res.y, 1.0)
}

canvas t(uv: coord, time: signal, res: resolution) -> color {
  let a = aspect(res)
  let p = ((uv.x - 0.5) * a, uv.y - 0.5)
  compose {
    circle(at: p, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("canvas_resolution_param_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected canvas `resolution` param support to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn duplicate_canvas_resolution_param_reports_error() {
    let input = r#"canvas t(uv: coord, r0: resolution, r1: resolution) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("canvas_resolution_param_duplicate");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected duplicate resolution params to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("duplicate `resolution` canvas parameter"),
        "expected duplicate resolution diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn fn_vec3_ops_and_swizzle_compiles() {
    let input = r#"fn tint(base: vec3, gain: f32) -> vec3 {
  base * gain + (0.05, 0.0, 0.08)
}

canvas t(uv: coord, time: signal) -> color {
  let v = tint((0.2, 0.4, 0.7), 1.1)
  let r = v.r
  let g = v.g
  let b = v.z
  compose {
    circle(at: center, radius: 20px) |> fill(rgb(r, g, b))
  }
}
"#;

    let path = unique_temp_path("fn_vec3_ops_swizzle_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vec3 arithmetic and swizzles to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn canvas_delta_param_compiles() {
    let input = r#"canvas t(uv: coord, time: signal, dt: delta) -> color {
  let pulse = 0.5 + 0.5 * sin(time + dt * 60.0)
  compose {
    circle(at: center, radius: 20px * (0.7 + 0.4 * pulse)) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("canvas_delta_param_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected canvas `delta` param support to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_vec34_geometric_builtins_compile() {
    let input = r#"fn warp3(i: vec3, n: vec3, eta: f32) -> vec3 {
  let ni = normalize(i)
  let nn = normalize(n)
  let d = dot(ni, nn)
  let r = reflect(ni, nn)
  let t = refract(ni, nn, eta)
  let w = distance(i, n)
  r * (0.5 + 0.5 * d) + t * (0.2 + 0.1 * w)
}

fn warp4(i: vec4, n: vec4, eta: f32) -> vec4 {
  let ni = normalize(i)
  let nn = normalize(n)
  let d = dot(ni, nn)
  let r = reflect(ni, nn)
  let t = refract(ni, nn, eta)
  let w = distance(i, n)
  r * (0.5 + 0.5 * d) + t * (0.2 + 0.1 * w)
}

canvas t(uv: coord, time: signal) -> color {
  let a = warp3((uv.x, uv.y, sin(time)), (0.0, 1.0, 0.5), 0.8)
  let b = warp4((a.x, a.y, a.z, 1.0), (0.0, 1.0, 0.0, 0.5), 0.9)
  compose {
    circle(at: center, radius: 20px) |> fill(rgb(b.x, b.y, b.z))
  }
}
"#;

    let path = unique_temp_path("fn_vec34_geometric_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vec3/vec4 geometric builtins to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_matrix_constructors_and_mul_compile() {
    let input = r#"fn warp(m: mat3, v: vec3) -> vec3 {
  let a = m * v
  let b = v * m
  (a + b) * 0.5
}

canvas t(uv: coord, time: signal) -> color {
  let c0 = (1.0, 0.0, 0.0)
  let c1 = (0.0, 1.0, 0.0)
  let c2 = (0.0, 0.0, 1.0)
  let m = mat3(c0, c1, c2)
  let v = (uv.x, uv.y, 0.5 + 0.5 * sin(time))
  let out = warp(m, v)
  compose {
    circle(at: center, radius: 20px) |> fill(rgb(out.x, out.y, out.z))
  }
}
"#;

    let path = unique_temp_path("fn_matrix_ctor_mul_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected matrix constructors and mul forms to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_vector_step_smoothstep_and_swizzles_compile() {
    let input = r#"fn shade(v3: vec3, v4: vec4) -> vec3 {
  let s3 = step((0.2, 0.3, 0.4), v3)
  let k3 = smoothstep((0.0, 0.0, 0.0), (1.0, 1.0, 1.0), v3)
  let s4 = step(0.5, v4)
  let k4 = smoothstep((0.0, 0.0, 0.0, 0.0), (1.0, 1.0, 1.0, 1.0), v4)

  let a = v3.yzx
  let b = v3.bgr
  let c = v4.xyz
  let d = v4.rgba
  let e = d.bgr

  let base = a * 0.15 + b * 0.15 + c * 0.20 + e * 0.20
  let shape = s3 * 0.15 + k3 * 0.15 + s4.xyz * 0.1 + k4.xyz * 0.1
  base + shape
}

canvas t(uv: coord, time: signal) -> color {
  let v3 = (uv.x, uv.y, 0.5 + 0.5 * sin(time))
  let v4 = (v3.x, v3.y, v3.z, 1.0)
  let out = shade(v3, v4)
  compose {
    circle(at: center, radius: 20px) |> fill(rgb(out.r, out.g, out.b))
  }
}
"#;

    let path = unique_temp_path("fn_vector_step_smoothstep_swizzle_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vector step/smoothstep and swizzles to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_component_wise_vec_mul_div_compile() {
    let input = r#"fn shade(a: vec3, b: vec3) -> vec3 {
  let m = a * b
  let d = m / (0.5, 0.5, 0.5)
  d
}

canvas t(uv: coord, time: signal) -> color {
  let a = (uv.x + 0.1, uv.y + 0.2, 0.5 + 0.5 * sin(time))
  let b = (0.9, 0.8, 0.7)
  let out = shade(a, b)
  compose {
    circle(at: center, radius: 20px) |> fill(rgb(out.x, out.y, out.z))
  }
}
"#;

    let path = unique_temp_path("fn_component_wise_vec_mul_div_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected component-wise vec mul/div to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn fn_vec_scalar_add_sub_compile() {
    let input = r#"fn shade(a: vec3) -> vec3 {
  let b = a - 0.25
  let c = 0.5 + b
  c
}

canvas t(uv: coord, time: signal) -> color {
  let a = (uv.x + 0.1, uv.y + 0.2, 0.5 + 0.5 * sin(time))
  let out = shade(a)
  compose {
    circle(at: center, radius: 20px) |> fill(rgb(out.x, out.y, out.z))
  }
}
"#;

    let path = unique_temp_path("fn_vec_scalar_add_sub_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vec-scalar add/sub to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_postprocess_function_compiles() {
    let input = r#"fn finish(src: color, uv: coord_like, time: signal, res: resolution) -> color {
  let pulse = 0.5 + 0.5 * sin(time + uv.x * 6.0)
  return rgba(
    r: src.r * pulse,
    g: src.g,
    b: src.b,
    a: 1.0,
  )
}

canvas t(uv: coord, time: signal, res: resolution) -> color {
  compose {
    fill(#203040)
    circle(at: center, radius: 0.18) |> fill(#ff66aa) |> blend(add)
  } |> postprocess(finish)
}
"#;

    let path = unique_temp_path("compose_postprocess_compile");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose postprocess function to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_postprocess_rejects_incompatible_function_signature() {
    let input = r#"fn finish(src: color, amount: f32) -> color {
  return src
}

canvas t(uv: coord, time: signal, res: resolution) -> color {
  compose {
    fill(#203040)
    circle(at: center, radius: 0.18) |> fill(#ff66aa) |> blend(add)
  } |> postprocess(finish)
}
"#;

    let path = unique_temp_path("compose_postprocess_bad_sig");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected incompatible compose postprocess signature to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("requires a function with a supported signature"),
        "expected postprocess signature diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn compose_promotes_color_returning_function_to_layer() {
    let input = r#"fn sky(uv: coord_like) -> color {
  return rgba(
    r: uv.x,
    g: uv.y,
    b: 0.25,
    a: 1.0,
  )
}

canvas t(uv: coord, time: signal) -> color {
  compose {
    sky(uv)
  }
}
"#;

    let path = unique_temp_path("compose_color_promotes_to_layer");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected color-returning function to compose directly\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
