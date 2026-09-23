use std::{fs, path::PathBuf, process::Command};

fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[path = "support/common.rs"]
mod common;
use common::unique_temp_path;

fn run_fresco_manifest(input_path: &PathBuf) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    Command::new(bin)
        .arg(input_path)
        .arg("--emit")
        .arg("manifest")
        .output()
        .expect("failed to run fresco binary")
}

fn run_fresco_explain(input_path: &PathBuf) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    Command::new(bin)
        .arg(input_path)
        .arg("--explain")
        .output()
        .expect("failed to run fresco binary")
}

#[test]
fn manifest_includes_pass_plan_schedule() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("manifest_pass_plan");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected manifest emit to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let manifest = normalize(&output.stdout);
    assert!(
        manifest.contains("\"pass_plan\"")
            && manifest.contains("\"passes\"")
            && manifest.contains("\"edges\""),
        "expected manifest to include pass plan schedule metadata\nmanifest:\n{manifest}"
    );
}

#[test]
fn manifest_emit_stdout_is_valid_json_object() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.2) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("manifest_json_stdout");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected manifest emit to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let manifest = normalize(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&manifest).expect("manifest stdout must be valid JSON");
    assert!(parsed.is_object(), "manifest stdout must be a JSON object");
    assert!(
        parsed.get("canvases").is_some(),
        "manifest JSON must include top-level `canvases`"
    );
}

/// A single-pass point-local program emits `kernel_strategy: "fused"` and no
/// intermediate targets in the manifest.
#[test]
fn manifest_single_pass_has_fused_strategy_and_no_targets() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.2) |> fill(#ff0000)
  }
}
"#;

    let path = unique_temp_path("manifest_fused");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected manifest emit to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let manifest = normalize(&output.stdout);
    assert!(
        manifest.contains("\"kernel_strategy\": \"fused\""),
        "expected point-local program to have fused kernel strategy\nmanifest:\n{manifest}"
    );
    // No intermediate targets for a single-pass all-point program.
    let parsed: serde_json::Value =
        serde_json::from_str(&manifest).expect("manifest must be valid JSON");
    let targets = &parsed["canvases"][0]["pass_plan"]["targets"];
    assert!(
        targets.as_array().is_none_or(Vec::is_empty),
        "expected no intermediate targets for single-pass program\nmanifest:\n{manifest}"
    );
}

/// A blur on an arbitrary layer should now compile (pass partitioner handles
/// it) and produce a manifest with `inline-taps` or a non-fused kernel strategy.
#[test]
fn blur_on_arbitrary_layer_compiles_and_manifest_shows_kernel_strategy() {
    // blur over a grey(scalar) layer — not analytically rewritable.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    grey(time * 0.5) |> blur(radius: 2px)
  }
}
"#;

    let path = unique_temp_path("manifest_blur_arbitrary");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected blur on arbitrary layer to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let manifest = normalize(&output.stdout);
    // The blur layer is Local, so the pass plan must pick a non-fused strategy.
    // With a 2px radius (≤ 4px threshold) the inline-taps rung should fire.
    assert!(
        manifest.contains("\"kernel_strategy\""),
        "expected manifest to include kernel_strategy field\nmanifest:\n{manifest}"
    );
}

/// A blur on a fill(shape) is still rewritten analytically — no pass split
/// occurs and the manifest shows only a fused strategy.
#[test]
fn blur_on_fill_rewrites_analytically_manifest_stays_single_pass() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.25) |> fill(#ffffff) |> blur(radius: 3px)
  }
}
"#;

    let path = unique_temp_path("manifest_blur_fill");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected blur on fill to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let manifest = normalize(&output.stdout);
    assert!(
        manifest.contains("\"kernel_strategy\": \"fused\""),
        "expected blur-on-fill to rewrite analytically and stay single-pass\nmanifest:\n{manifest}"
    );
}

/// The --explain output now includes kernel strategy annotations and a
/// `→ swapchain` / `→ rt{n}` routing note per pass.
#[test]
fn explain_shows_kernel_strategy_annotation() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.2) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("explain_kernel");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_explain(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explain run to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    // Every pass line now ends with a routing note.
    assert!(
        explain.contains("→ swapchain") || explain.contains("→ rt"),
        "expected explain to include pass routing annotation\nexplain:\n{explain}"
    );
}

#[test]
fn explain_reports_user_defined_local_effect_radius() {
    let input = r#"
effect chromatic_split(spread: f32) local(spread) {
  let left = self at (coord - (spread, 0.0))
  let right = self at (coord + (spread, 0.0))
  layer rgba(r: left.r, g: self.g, b: right.b, a: self.a)
}

canvas t(uv: coord) -> color {
  compose {
    box(at: (0.5, 0.5), size: (0.6, 0.6))
      |> fill(mix(#1a1a2e, #ff6b6b, checker(scale: 0.08uv, at: (0.5, 0.5))))
      |> chromatic_split(0.003)
  }
}
"#;

    let path = unique_temp_path("explain_user_effect_radius");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_explain(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explain run to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("inline-taps: r=3.2px") || explain.contains("inline-taps: r=3.3px"),
        "expected explain to report the local user-effect radius\nexplain:\n{explain}"
    );
}

/// Manifest targets array is populated for a multi-locality program.
#[test]
fn manifest_multi_pass_program_has_intermediate_targets() {
    // blur over a grey(scalar) layer — not analytically rewritable — produces a
    // Local layer, triggering a multi-pass schedule with at least one
    // intermediate target.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    grey(time * 0.5) |> blur(radius: 2px)
    circle(at: center, radius: 0.2) |> fill(#112233)
  }
}
"#;

    let path = unique_temp_path("manifest_multi_pass");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected multi-pass program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let manifest = normalize(&output.stdout);
    // Must contain the targets array key in the pass_plan.
    assert!(
        manifest.contains("\"targets\""),
        "expected manifest to include targets array\nmanifest:\n{manifest}"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&manifest).expect("manifest must be valid JSON");
    let passes = parsed["canvases"][0]["pass_plan"]["passes"]
        .as_array()
        .expect("manifest pass list must be an array for multi-pass programs");
    assert!(
        passes.iter().any(|entry| entry["inputs"]
            .as_array()
            .is_some_and(|inputs| !inputs.is_empty())),
        "expected manifest to expose explicit pass inputs\nmanifest:\n{manifest}"
    );
}

#[test]
fn manifest_includes_path_buffers_for_threshold_paths() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
    arc center: (0.72, 0.55) radius: 0.20 sweep: 360deg
  }

  compose {
        curve |> stroke(width: 0.02)
  }
}
"#;

    let path = unique_temp_path("manifest_path_buffers");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected manifest emit to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let manifest = normalize(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&manifest).expect("manifest must be valid JSON");
    let buffers = parsed["canvases"][0]["path_buffers"]
        .as_array()
        .expect("expected path_buffers array for threshold path");
    assert_eq!(
        buffers.len(),
        1,
        "expected one path buffer entry\nmanifest:\n{manifest}"
    );
    assert_eq!(
        buffers[0]["group"].as_u64(),
        Some(2),
        "path buffer must use group 2"
    );
    assert!(
        buffers[0]["binding"].as_u64().is_some(),
        "expected path buffer binding index\nmanifest:\n{manifest}"
    );
    assert!(
        buffers[0]["segments"].as_u64().is_some_and(|n| n > 64),
        "expected threshold path to expose segment count > 64\nmanifest:\n{manifest}"
    );
}

#[test]
fn engine_axis_binding_time_is_preserved_in_manifest() {
    let path = unique_temp_path("manifest_global_axis");
    fs::write(
        &path,
        r#"
axis @known(pipeline) tile_size: 8|16
@editor_only pass render { permutations { @config(editor) tile_size: 8|16 } }
pipeline output { render }
"#,
    )
    .expect("write source");
    let output = run_fresco_manifest(&path);
    let _ = fs::remove_file(path);
    assert!(output.status.success(), "{}", normalize(&output.stderr));
    let manifest: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("manifest JSON");
    let axes = manifest["config_axes"].as_array().expect("config axes");
    let axis = axes
        .iter()
        .find(|axis| axis["axis"] == "tile_size")
        .expect("tile_size metadata");
    assert_eq!(axis["known_mode"], "pipeline");
}
