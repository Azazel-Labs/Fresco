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
fn rejects_legacy_postfix_round_syntax() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let b = box(at: center, size: (0.3, 0.2))
  let c = b round 0.04
  compose {
    c |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("legacy_round");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected legacy postfix `round` syntax to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unexpected `round`"),
        "expected diagnostic to mention unexpected `round`\nstderr:\n{stderr}"
    );
}

#[test]
fn rejects_legacy_postfix_smooth_syntax() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let a = circle(at: center, radius: 0.2)
  let b = box(at: center, size: (0.3, 0.2))
  let d = a | b smooth 0.05
  compose {
    d |> fill(#ff00ff)
  }
}
"#;

    let path = unique_temp_path("legacy_smooth");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected legacy postfix `smooth` syntax to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unexpected `smooth`"),
        "expected diagnostic to mention unexpected `smooth`\nstderr:\n{stderr}"
    );
}
