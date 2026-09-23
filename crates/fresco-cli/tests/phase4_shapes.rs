use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, run_fresco_with_args, unique_temp_path};

#[test]
fn trapezoid_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let mountain = trapezoid(at: (0.5, 0.6), top: 0.25, bottom: 0.55, height: 0.3)

  compose {
    mountain |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase4_trapezoid_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected trapezoid to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn polygon2_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let snow = polygon2(
    points: [
      (0.20, 0.20),
      (0.80, 0.20),
      (0.70, 0.70),
      (0.30, 0.80),
    ],
    fill_rule: even_odd,
  )

  compose {
    snow |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase4_polygon2_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected polygon2 to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn polygon3_compiles_with_vec3_points() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let panel = polygon3(
    points: [
      (0.15, 0.20, 0.0),
      (0.85, 0.20, 0.0),
      (0.75, 0.75, 0.2),
      (0.25, 0.82, 0.1),
    ],
    fill_rule: non_zero,
    plane_mode: auto,
  )

  compose {
    panel |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase4_polygon3_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected polygon3 to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn polygon2_rejects_too_few_points() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let bad = polygon2(points: [(0.2, 0.2), (0.8, 0.2)])

  compose {
    bad |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase4_polygon2_too_few");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected polygon2 with too few points to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("polygon expects at least 3 points"),
        "expected polygon point-count diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn explain_reports_polygon_lowering_notes() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let p2 = polygon2(points: [(0.2, 0.2), (0.8, 0.2), (0.7, 0.7), (0.3, 0.8)])
  let p3 = polygon3(points: [(0.2, 0.2, 0.0), (0.8, 0.2, 0.0), (0.7, 0.7, 0.3)])

  compose {
    p2 |> fill(#ffffff)
    p3 |> fill(#88ccff)
  }
}
"#;

    let path = unique_temp_path("phase4_polygon_explain_notes");
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
        explain.contains("polygon2: lowered"),
        "expected polygon2 explain note\nstderr:\n{explain}"
    );
    assert!(
        explain.contains("polygon3: lowered"),
        "expected polygon3 explain note\nstderr:\n{explain}"
    );
}

#[test]
fn shape_constructors_accept_rotate_argument() {
    let input = r#"canvas t(uv: coord) -> color {
  let verts = [
    (0.2, 0.2),
    (0.35, 0.2),
    (0.35, 0.35),
    (0.2, 0.35)
  ]

  compose {
    box(at: (0.2, 0.2), size: (0.08, 0.08), rotate: 0.1turn) |> fill(#66ccff)
    ellipse(at: (0.38, 0.2), radii: (0.07, 0.04), rotate: 0.2turn) |> fill(#88ddaa)
    polygon2(points: verts, rotate: 0.05turn) |> fill(#ffcc66)
    star(at: center, outer: 0.12, inner: 0.06, points: 5, rotate: 0.25turn) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase4_shape_constructor_rotate");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected rotated shape constructors to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
