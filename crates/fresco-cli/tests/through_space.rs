use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, unique_temp_path};

// ── basic compilation ───────────────────────────────────────────────────────

#[test]
fn through_space_translate_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let scene = circle(at: center, radius: 0.2) |> fill(#4ade80)
    scene through space translate(by: (0.05, 0))
}
"#;
    let path = unique_temp_path("through_space_translate_ok");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space translate` should compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn through_space_warp_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let bg = fill(#0a1220)
    let fg = circle(at: center, radius: 0.3) |> fill(#38bdf8)
    let offset = (sin(coord.x * 8.0 + time) * 0.02, cos(coord.y * 8.0 + time) * 0.02)
    fg through space warp(by: offset)
}
"#;
    let path = unique_temp_path("through_space_warp_ok");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space warp` should compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn through_space_rotate_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let scene = box(at: center, size: (0.6, 0.4)) |> fill(#7c3aed)
    scene through space rotate(angle: time * 0.5)
}
"#;
    let path = unique_temp_path("through_space_rotate_ok");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space rotate` should compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

// ── chained space ─────────────────────────────────────────────────────────────

#[test]
fn through_space_chained_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let scene = circle(at: center, radius: 0.25) |> fill(#f59e0b)
    scene through space translate(by: (0.05, 0)) . rotate(angle: 0.3)
}
"#;
    let path = unique_temp_path("through_space_chained_ok");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space S1 . S2` should compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

// ── named space reference ─────────────────────────────────────────────────────

#[test]
fn through_named_space_ref_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    space lens = scale(factor: 2.0, around: center)
    let scene = circle(at: center, radius: 0.15) |> fill(#22d3ee)
    scene through space lens
}
"#;
    let path = unique_temp_path("through_named_space_ok");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space <name>` should compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

// ── piped result ─────────────────────────────────────────────────────────────

#[test]
fn through_space_followed_by_pipe_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let scene = circle(at: center, radius: 0.2) |> fill(#c084fc)
    scene through space translate(by: (0.1, 0)) |> opacity(0.6)
}
"#;
    let path = unique_temp_path("through_space_then_pipe_ok");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space ... |> pipe` should compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

// ── compose stack usage ───────────────────────────────────────────────────────

#[test]
fn through_space_in_compose_stack_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let scene = circle(at: center, radius: 0.25) |> fill(#38bdf8)
    let offset = (sin(coord.x * 6.0) * 0.025, cos(coord.y * 6.0) * 0.025)
    compose {
        fill(#0a1220)
        scene
        scene through space warp(by: offset) |> opacity(0.5) |> blend(add)
    }
}
"#;
    let path = unique_temp_path("through_space_in_compose_ok");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space` inside a compose stack should compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

// ── WGSL validity ─────────────────────────────────────────────────────────────

#[test]
fn through_space_emits_valid_wgsl() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let scene = circle(at: center, radius: 0.2) |> fill(#4ade80)
    let offset = (sin(coord.x * 8.0 + time) * 0.02, cos(coord.y * 8.0 + time) * 0.02)
    scene through space warp(by: offset)
}
"#;
    let path = unique_temp_path("through_space_wgsl_valid");
    fs::write(&path, input).expect("failed to write test source");
    // Default emit is `wgsl`, so a successful exit with --emit wgsl is enough.
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        output.status.success(),
        "`through space` should emit valid WGSL\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
    // Also ensure the warp displacement variable name appears, confirming
    // the InSpace warp lowering path was exercised.
    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("space_warp_by"),
        "emitted WGSL should contain warp-displacement variable\nwgsl:\n{wgsl}"
    );
}

// ── error: non-layer operand ──────────────────────────────────────────────────

#[test]
fn through_space_non_layer_operand_is_an_error() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let sample_value = 0.5
    sample_value through space translate(by: (0.1, 0))
}
"#;
    let path = unique_temp_path("through_space_bad_operand");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        !output.status.success(),
        "`through space` on a scalar should fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("through space"),
        "error should mention `through space`\nstderr:\n{stderr}"
    );
}

// ── error: unknown space constructor ─────────────────────────────────────────

#[test]
fn through_unknown_space_is_an_error() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let scene = circle(at: center, radius: 0.2) |> fill(#4ade80)
    scene through space no_such_space_fn(amount: 0.3)
}
"#;
    let path = unique_temp_path("through_space_unknown_space");
    fs::write(&path, input).expect("failed to write test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);
    assert!(
        !output.status.success(),
        "unknown space constructor should fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
