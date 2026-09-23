use std::fs;
use std::process::Command;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

#[test]
fn verbose_mode_emits_pipeline_progress_logs() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.2) |> fill(#ffffff)
  }
}

"#;

    let path = unique_temp_path("logging_progression_verbose");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--verbose", "--trace-level", "info"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected verbose compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("phase start: lex"),
        "expected verbose logs to include lex phase start\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("phase done: lower_and_validate"),
        "expected verbose logs to include lower/validate phase completion\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("checker phase start: register_functions"),
        "expected verbose logs to include checker function registration phase\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("checker stmt start"),
        "expected verbose logs to include checker statement-level breadcrumbs\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("checker compose entry start"),
        "expected verbose logs to include compose entry breadcrumbs\nstderr:\n{stderr}"
    );
}

#[test]
fn verbose_mode_emits_expression_watchdog_heartbeat() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let p = (uv.x, uv.y);
  let q = (p.x + 0.1, p.y + 0.2);
  compose {
    circle(at: q, radius: 0.2 + sin(time) * 0.01) |> fill(#ffffff)
  }
}

"#;

    let path = unique_temp_path("logging_progression_watchdog");
    fs::write(&path, input).expect("failed to write temporary test source");

    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");
    let output = Command::new(bin)
        .arg(&path)
        .arg("--emit")
        .arg("wgsl")
        .arg("--verbose")
        .arg("--trace-level")
        .arg("info")
        .env("RUST_LOG", "off")
        .env("FRESCO_CHECK_EXPR_WATCHDOG_EVERY", "1")
        .env("FRESCO_CHECK_EXPR_HOTSPOT_TOP", "3")
        .env("FRESCO_CHECK_EXPR_HOTSPOT_MIN_MS", "0.0")
        .output()
        .expect("failed to run fresco binary");

    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected verbose watchdog compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("checker expr watchdog heartbeat"),
        "expected verbose logs to include expression watchdog heartbeat\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("checker expr hotspot summary"),
        "expected verbose logs to include hotspot summary\nstderr:\n{stderr}"
    );
}

#[test]
fn expr_timeout_emits_timeout_checkpoint() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let p = (uv.x, uv.y);
  compose {
    circle(at: p, radius: 0.2 + sin(time) * 0.01) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("logging_progression_timeout");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(
        &path,
        &[
            "--verbose",
            "--trace-level",
            "info",
            "--expr-timeout-ms",
            "0",
        ],
    );
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected timeout compile to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("checker expr timeout checkpoint"),
        "expected timeout checkpoint in stderr\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("checker expression evaluation timed out after 0ms"),
        "expected timeout diagnostic in stderr\nstderr:\n{stderr}"
    );
}
