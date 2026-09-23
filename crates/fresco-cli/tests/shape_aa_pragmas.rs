use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

fn run_source(suffix: &str, input: &str) -> std::process::Output {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);
    output
}

#[test]
fn shape_aa_pragmas_tune_emitted_band_limits() {
    let input = r#"#pragma check.shape_aa_min_px = 0.75
#pragma check.shape_aa_max_px = 5.5

canvas t(uv: coord, time: signal) -> color {
  space warped = rotate(14deg) . scale(1.0 + uv.y * 2.0)
  let ring = circle(at: (0.5, 0.5), radius: 0.18) |> stroke(2px)

  compose {
    fill(#0f172a)
    in space warped {
      ring |> fill(#ffffff)
    }
  }
}
"#;

    let output = run_source("shape_aa_pragmas_tune", input);
    assert!(
        output.status.success(),
        "expected compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("0.75f") && wgsl.contains("dpdx(p_") && wgsl.contains("dpdy(p_"),
        "expected pragma-tuned AA bounds in WGSL\nwgsl:\n{wgsl}"
    );
    assert!(wgsl.contains("5.5f") && wgsl.contains("aa_directional_px"));
}

#[test]
fn shape_aa_style_and_level_pragmas_apply() {
    let input = r#"#pragma check.shape_aa_style = conservative
#pragma check.shape_aa_level = max

canvas t(uv: coord, time: signal) -> color {
  space warped = rotate(14deg) . scale(1.0 + uv.y * 2.0)
  let ring = star(at: (0.5, 0.5), outer: 0.18, inner: 0.09, points: 5) |> stroke(2px)

  compose {
    fill(#0f172a)
    in space warped {
      ring |> fill(#ffffff)
    }
  }
}
"#;

    let output = run_source("shape_aa_style_level", input);
    assert!(
        output.status.success(),
        "expected compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("2.5f")
            && wgsl.contains("5.5f")
            && wgsl.contains("dpdx(p_")
            && wgsl.contains("dpdy(p_")
            && wgsl.contains("sqrt(")
            && wgsl.contains("abs("),
        "expected style + level pragmas to affect AA estimator and clamp bounds\nwgsl:\n{wgsl}"
    );
}

#[test]
fn shape_aa_style_pragma_invalid_value_reports_diagnostic() {
    let input = r#"#pragma check.shape_aa_style = potato

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.2) |> stroke(2px) |> fill(#ffffff)
  }
}
"#;

    let output = run_source("shape_aa_style_invalid", input);
    assert!(
        !output.status.success(),
        "expected compile failure\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("check.shape_aa_style") && stderr.contains("gradient"),
        "expected invalid shape AA style pragma diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn shape_aa_pragmas_invalid_range_reports_diagnostic() {
    let input = r#"#pragma check.shape_aa_min_px = 6.0
#pragma check.shape_aa_max_px = 2.0

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 0.2) |> stroke(2px) |> fill(#ffffff)
  }
}
"#;

    let output = run_source("shape_aa_pragmas_invalid", input);
    assert!(
        !output.status.success(),
        "expected compile failure\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("invalid pragma configuration")
            && stderr.contains("check.shape_aa_min_px")
            && stderr.contains("check.shape_aa_max_px"),
        "expected invalid shape AA pragma diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn shape_aa_polar_space_avoids_derivative_span_regression() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  space dial = polar(center: (0.5, 0.5), from: -90deg, direction: clockwise)
  let track = box(at: (0.5, 0.32), size: (1.0, 0.045))

  compose {
    fill(#0f172a)
    in space dial {
      track |> fill(#ffffff)
    }
  }
}
"#;

    let output = run_source("shape_aa_polar_span_regression", input);
    assert!(
        output.status.success(),
        "expected compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("space_polar_x"),
        "expected polar-space lowering in WGSL\nwgsl:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("dpdx(p_space") && !wgsl.contains("dpdy(p_space"),
        "AA must not differentiate wrapped polar coordinates across the seam\nwgsl:\n{wgsl}"
    );
    assert!(
        wgsl.contains("dpdx(p)")
            && wgsl.contains("dpdy(p)")
            && wgsl.contains("polar_filter_dx")
            && wgsl.contains("polar_filter_dy")
            && wgsl.contains("box_periodic_coverage"),
        "expected the analytic polar Jacobian applied to the Cartesian pixel footprint, with periodic interval coverage\nwgsl:\n{wgsl}"
    );
    assert_eq!(wgsl.matches("dpdx(").count(), 1, "{wgsl}");
    assert_eq!(wgsl.matches("dpdy(").count(), 1, "{wgsl}");
}
