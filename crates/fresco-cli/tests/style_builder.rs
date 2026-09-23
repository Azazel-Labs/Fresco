use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, run_fresco_with_args, unique_temp_path};

#[test]
fn style_builder_chain_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  style neon_outline(color: color = #fdb1ff, width: f32 = 0.0035) = stroke(width: width) |> fill(color)
  let mountain = box(at: center, size: (0.35, 0.2))

  compose {
    mountain |> neon_outline
  }
}
"#;

    let path = unique_temp_path("style_builder_chain_compiles");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected style builder chain to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn style_builder_args_override_defaults() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  style neon(color: color = #ff93f5, width: f32 = 0.01) = stroke(width: width) |> fill(color)
  let mountain = box(at: center, size: (0.35, 0.2))

  compose {
    mountain |> neon(color: #00ff66)
  }
}
"#;

    let path = unique_temp_path("style_builder_args_override_defaults");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected style call-site arguments to override defaults\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn style_builder_unknown_arg_is_rejected() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  style neon(color: color = #ff93f5) = fill(color)
  let mountain = box(at: center, size: (0.35, 0.2))

  compose {
    mountain |> neon(width: 0.01)
  }
}
"#;

    let path = unique_temp_path("style_builder_unknown_arg_rejected");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unknown style argument to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unknown argument `width`"),
        "expected style argument diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn style_builder_cycle_is_rejected() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  style a = b
  style b = a

  compose {
    circle(at: center, radius: 0.1) |> a
  }
}
"#;

    let path = unique_temp_path("style_builder_cycle_rejected");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected recursive style expansion to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("recursive style expansion"),
        "expected recursive style diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn style_builder_explain_reports_stage_chain() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    style neon_outline(color: color = #fdb1ff, width: f32 = 0.0035) = stroke(width: width) |> fill(color)

    compose {
    box(at: center, size: (0.35, 0.2)) |> neon_outline
    }
  }
  "#;

    let path = unique_temp_path("style_builder_explain_chain");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explain run to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("style: expanded `neon_outline` as stroke(1 arg(s)) |> fill(1 arg(s))"),
        "expected style expansion explain note\nstderr:\n{explain}"
    );
}
