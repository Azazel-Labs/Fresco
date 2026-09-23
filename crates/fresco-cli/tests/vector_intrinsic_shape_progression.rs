use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

fn compile_to_wgsl(suffix: &str, input: &str) -> String {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected non-helper intrinsic compile to succeed\nstdout:\n{}\nstderr:\n{}",
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

#[test]
fn direct_lowering_preserves_vec2_max_scalar_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let q = abs(vec2(uv.x - 0.5, uv.y - 0.5)) - vec2(0.2, 0.2);
  let d = length(max(q, 0.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * d) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec2_max_scalar_shape", input);
    assert!(
        wgsl.contains("max(vec2<f32>(") && wgsl.contains("vec2<f32>(0f"),
        "expected direct lowering to keep vec2 max(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "max(vec2<f32>(", 2, "vec2 max");
}

#[test]
fn direct_lowering_preserves_vec3_max_scalar_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let q = abs(vec3(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2)) - vec3(0.2, 0.2, 0.2);
  let d = length(max(q, 0.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * d) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec3_max_scalar_shape", input);
    assert!(
        wgsl.contains("max(vec3<f32>(") && wgsl.contains("vec3<f32>(0f"),
        "expected direct lowering to keep vec3 max(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "max(vec3<f32>(", 3, "vec3 max");
}

#[test]
fn direct_lowering_preserves_vec4_max_scalar_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let q = abs(vec4(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2, 1.0)) - vec4(0.2, 0.2, 0.2, 0.2);
  let d = length(max(q, 0.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * d) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec4_max_scalar_shape", input);
    assert!(
        wgsl.contains("max(vec4<f32>(") && wgsl.contains("vec4<f32>(0f"),
        "expected direct lowering to keep vec4 max(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "max(vec4<f32>(", 4, "vec4 max");
}

#[test]
fn direct_lowering_preserves_vec2_min_scalar_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let q = abs(vec2(uv.x - 0.5, uv.y - 0.5)) - vec2(0.2, 0.2);
  let d = length(min(q, 0.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * abs(d)) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec2_min_scalar_shape", input);
    assert!(
        wgsl.contains("min(vec2<f32>(") && wgsl.contains("vec2<f32>(0f"),
        "expected direct lowering to keep vec2 min(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "min(vec2<f32>(", 2, "vec2 min");
}

#[test]
fn direct_lowering_preserves_vec3_min_scalar_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let q = abs(vec3(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2)) - vec3(0.2, 0.2, 0.2);
  let d = length(min(q, 0.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * abs(d)) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec3_min_scalar_shape", input);
    assert!(
        wgsl.contains("min(vec3<f32>(") && wgsl.contains("vec3<f32>(0f"),
        "expected direct lowering to keep vec3 min(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "min(vec3<f32>(", 3, "vec3 min");
}

#[test]
fn direct_lowering_preserves_vec4_min_scalar_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let q = abs(vec4(uv.x - 0.5, uv.y - 0.5, sin(time) * 0.2, 1.0)) - vec4(0.2, 0.2, 0.2, 0.2);
  let d = length(min(q, 0.0));
  compose {
    circle(at: center, radius: 0.03 + 0.02 * abs(d)) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec4_min_scalar_shape", input);
    assert!(
        wgsl.contains("min(vec4<f32>(") && wgsl.contains("vec4<f32>(0f"),
        "expected direct lowering to keep vec4 min(q, 0.0) shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "min(vec4<f32>(", 4, "vec4 min");
}

#[test]
fn direct_lowering_preserves_vec2_clamp_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let rg = clamp(vec2(uv.x, uv.y), vec2(0.0), vec2(1.0));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * rg.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec2_clamp_shape", input);
    assert!(
        wgsl.contains("clamp(vec2<f32>(") && wgsl.contains("vec2<f32>(1f"),
        "expected direct lowering to keep vec2 clamp shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "clamp(vec2<f32>(", 2, "vec2 clamp");
}

#[test]
fn direct_lowering_preserves_vec3_clamp_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let rgb = clamp(vec3(uv.x, uv.y, sin(time) * 0.5 + 0.5), vec3(0.0), vec3(1.0));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * rgb.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec3_clamp_shape", input);
    assert!(
        wgsl.contains("clamp(vec3<f32>(") && wgsl.contains("vec3<f32>(1f"),
        "expected direct lowering to keep vec3 clamp shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "clamp(vec3<f32>(", 3, "vec3 clamp");
}

#[test]
fn direct_lowering_preserves_vec4_clamp_shape() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let rgba = clamp(vec4(uv.x, uv.y, sin(time) * 0.5 + 0.5, 1.0), vec4(0.0), vec4(1.0));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * rgba.x) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("direct_vec4_clamp_shape", input);
    assert!(
        wgsl.contains("clamp(vec4<f32>(") && wgsl.contains("vec4<f32>(1f"),
        "expected direct lowering to keep vec4 clamp shape\nwgsl:\n{wgsl}"
    );
    assert_intrinsic_call_count_at_most(&wgsl, "clamp(vec4<f32>(", 4, "vec4 clamp");
}

#[test]
fn direct_lowering_dot_vec2_on_vec3_swizzle_keeps_vec2_arity() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let p = vec3(uv.x - 0.5, uv.y - 0.5, sin(time));
  let d = dot(vec2(p.x, p.y), vec2(0.5, 0.25));
  compose {
    circle(at: center, radius: 0.02 + 0.01 * abs(d)) |> fill(#ffffff)
  }
}
"#;

    let wgsl = compile_to_wgsl("dot_vec2_on_vec3_swizzle", input);
    assert!(
        wgsl.contains("dot(vec2<f32>(") || wgsl.contains("dot(vec2("),
        "expected vec2 dot call shape for vec3 swizzle input\nwgsl:\n{wgsl}"
    );
}
