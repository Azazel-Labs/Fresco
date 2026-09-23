use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

#[test]
fn motion_blur_rejects_previous_frame_history_leaf() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let base = image(previous_frame)
  compose {
    base |> motion_blur(shutter: 8ms)
  }
}
"#;

    let path = unique_temp_path("motion_blur_reject_previous_frame");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected motion_blur with previous_frame to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("motion_blur") && stderr.contains("history-bound leaf `previous_frame`"),
        "expected temporal-purity motion_blur diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn motion_blur_allows_signal_driven_scene() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let base = circle(at: (0.5 + sin(time) * 0.1, 0.5), radius: 0.1) |> fill(#ffffff)
  compose {
    base |> motion_blur(shutter: 8ms)
  }
}
"#;

    let path = unique_temp_path("motion_blur_signal_driven_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected signal-driven motion_blur scene to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn motion_blur_rejects_previous_frame_prefixed_feedback_texture() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let base = image(previous_frame_feedback)
    compose {
    base |> motion_blur(shutter: 8ms)
    }
  }
  "#;

    let path = unique_temp_path("motion_blur_reject_previous_frame_prefixed");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected motion_blur with previous_frame-prefixed feedback texture to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("history-bound leaf `previous_frame_feedback`"),
        "expected temporal-purity feedback-texture diagnostic\nstderr:\n{stderr}"
    );
}
