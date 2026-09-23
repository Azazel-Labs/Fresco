use std::{fs, path::PathBuf, process::Command};

fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[path = "support/common.rs"]
mod common;
use common::unique_temp_path;

fn run_fresco(input_path: &PathBuf, extra: &[&str]) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    Command::new(bin)
        .arg(input_path)
        .args(extra)
        .output()
        .expect("failed to run fresco binary")
}

#[test]
fn anisotropic_glow_with_gradient_and_gaussian_falloff_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let haze = capsule(from: (0.12, 0.5), to: (0.88, 0.5), radius: 0.02)
    |> glow(
      reach: (0.24, 0.08),
      strength: 0.7,
      color: gradient(
        along: y,
        stops: [
          stop(at: 0.0, color: #ff2ac6),
          stop(at: 1.0, color: #66b8ff),
        ]
      ),
      falloff: gaussian,
      color_space: glow
    )

  compose {
    fill(#0b1020)
    haze |> blend(add)
  }
}
"#;

    let path = unique_temp_path("glow_progression");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path, &["--emit", "wgsl"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected anisotropic glow program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn layer_glow_with_gradient_and_linear_falloff_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let core = circle(at: center, radius: 0.09) |> fill(#ffffff)
  let haze = core |> glow(
    reach: (0.18, 0.08),
    strength: 0.65,
    color: gradient(
      along: y,
      stops: [
        stop(at: 0.0, color: #ff8bd6),
        stop(at: 1.0, color: #7dd3fc),
      ]
    ),
    falloff: linear
  )

  compose {
    fill(#070b15)
    haze |> blend(add)
  }
}
"#;

    let path = unique_temp_path("layer_glow_progression");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path, &["--emit", "wgsl"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected layer glow program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
