use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

fn assert_compiles(suffix: &str, input: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

fn assert_explain_contains(suffix: &str, input: &str, needle: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(needle),
        "expected explain receipt to contain `{needle}`\nstderr:\n{stderr}"
    );
}

fn assert_compile_fails_contains(suffix: &str, input: &str, needle: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected compile to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(needle),
        "expected compile failure to contain `{needle}`\nstderr:\n{stderr}"
    );
}

#[test]
fn path_future_method_and_channel_placeholders_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
    arc center: (0.72, 0.55) radius: 0.10 sweep: 200deg
  }

  let head_s = (0.2 + time * 0.1) * curve.length
  let pen = curve.point_at(head_s)
  let tangent = curve.tangent_at(head_s)
  let m = smoothstep(lo: head_s - 0.1, hi: head_s, x: curve.along)
  let ink = curve |> stroke(width: 0.03)

  compose {
    ink |> opacity(m)
    circle(at: pen + tangent * 0.01, radius: 0.02 + curve.dist * 0.1)
      |> fill(#ffffff)
      |> opacity(m)
  }
}
"#;

    assert_compiles("path_future_method_channel_placeholders", input);
}

#[test]
fn fill_path_width_reports_migration_error() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    line (0.90, 0.30)
  }

  compose {
    fill(curve, width: 0.02)
  }
}
"#;

    assert_compile_fails_contains(
        "fill_path_width_removed",
        input,
        "`fill(path, ...)` is no longer supported",
    );
}

#[test]
fn path_receiver_fill_closed_path_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg("M 0.20 0.20 L 0.80 0.20 L 0.80 0.80 L 0.20 0.80 Z")

  compose {
    curve |> fill(#ffffff)
  }
}
"#;

    assert_compiles("path_receiver_fill_closed_path", input);
}

#[test]
fn path_receiver_fill_supports_shape_anchor_gradient() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg("M 0.12 0.62 C 0.30 0.18, 0.56 0.20, 0.74 0.52 C 0.82 0.66, 0.90 0.60, 0.92 0.48 Z")

  compose {
    curve |> fill(gradient(
      along: y,
      anchor: shape,
      stops: [
        stop(at: 0.0, color: #0b1f2e),
        stop(at: 1.0, color: #7de0ff),
      ]
    ))
  }
}
"#;

    assert_compiles("path_receiver_fill_shape_anchor_gradient", input);
}

#[test]
fn path_receiver_fill_open_path_reports_closed_contour_error() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    line (0.90, 0.30)
    line (0.90, 0.70)
  }

  compose {
    curve |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_contains(
        "path_receiver_fill_open_path",
        input,
        "`fill` on a path receiver requires a closed contour",
    );
}

#[test]
fn path_explain_reports_const_module_embedded_storage_for_small_paths() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path {
    move (0.10, 0.30)
    line (0.90, 0.30)
    line (0.90, 0.70)
  }

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_explain_contains(
        "path_explain_const_storage",
        input,
        "path: 2 segs (const, module-embedded)",
    );
}

#[test]
fn path_explain_reports_threshold_buffer_reason_for_large_paths() {
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

    assert_explain_contains(
        "path_explain_threshold_storage",
        input,
        "path: 79 segs (buffer: exceeds const threshold 64)",
    );
}

#[test]
fn path_svg_cubic_outline_compiles_as_real_path_value() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg("M 0.12 0.62 C 0.30 0.18, 0.56 0.20, 0.74 0.52 C 0.82 0.66, 0.90 0.60, 0.92 0.48 Z")

  let head_s = fract(time * 0.2) * curve.length
  let pen = curve.point_at(head_s)
  let m = smoothstep(lo: head_s - 0.08, hi: head_s, x: curve.along)

  compose {
    fill(#08111d)
    curve |> stroke(width: 0.02) |> opacity(m)
    circle(at: pen, radius: 0.01) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("path_svg_cubic_outline", input);
}

#[test]
fn path_svg_explain_reports_svg_path_lowering_note() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg("M 0.12 0.62 C 0.30 0.18, 0.56 0.20, 0.74 0.52 C 0.82 0.66, 0.90 0.60, 0.92 0.48 Z")

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_explain_contains(
        "path_svg_explain_note",
        input,
        "path_svg: preserved SVG path commands as native path primitives",
    );
}

#[test]
fn path_svg_samples_knob_can_keep_curve_const_embedded() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 55 142 C 145 34, 275 50, 358 112 C 425 161, 512 152, 562 110 C 588 88, 616 94, 618 114 C 620 132, 600 138, 592 126 C 588 119, 594 111, 601 114 C 596 113, 593 118, 596 123 C 601 131, 615 127, 613 115 C 611 99, 588 95, 568 116 C 520 162, 424 174, 350 122 C 278 70, 152 48, 55 158 Z",
    samples: 6,
  )

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_explain_contains(
        "path_svg_samples_const_threshold",
        input,
        "path: 61 segs (const, module-embedded)",
    );
}

#[test]
fn path_svg_tolerance_mode_reports_adaptive_sampling_note() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 55 142 C 145 34, 275 50, 358 112 C 425 161, 512 152, 562 110 C 588 88, 616 94, 618 114 C 620 132, 600 138, 592 126 C 588 119, 594 111, 601 114 C 596 113, 593 118, 596 123 C 601 131, 615 127, 613 115 C 611 99, 588 95, 568 116 C 520 162, 424 174, 350 122 C 278 70, 152 48, 55 158 Z",
    tolerance: 6.0,
  )

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_explain_contains(
        "path_svg_tolerance_note",
        input,
        "adaptive tolerance: 6.0000",
    );
}

#[test]
fn path_svg_preserve_cubics_mode_reports_retention_note() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 0.12 0.62 C 0.30 0.18, 0.56 0.20, 0.74 0.52 C 0.82 0.66, 0.90 0.60, 0.92 0.48 Z",
    preserve_cubics: 1,
  )

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_explain_contains("path_svg_preserve_cubics_note", input, "preserve_cubics=on");
    assert_explain_contains(
        "path_svg_preserve_cubics_retained",
        input,
        "path preprocess: cubics retained=",
    );
}

#[test]
fn path_svg_preserve_cubics_reports_deterministic_single_segment_count() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 0.08 0.52 C 0.30 0.10, 0.70 0.90, 0.92 0.48",
    preserve_cubics: 1,
  )

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_explain_contains(
        "path_svg_preserve_cubics_one_seg_count",
        input,
        "path: 1 segs (const, module-embedded)",
    );
}

#[test]
fn path_svg_preserve_cubics_reports_deterministic_two_segment_count() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 0.12 0.52 C 0.26 0.15, 0.44 0.82, 0.58 0.46 C 0.70 0.18, 0.82 0.78, 0.92 0.48",
    preserve_cubics: 1,
  )

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_explain_contains(
        "path_svg_preserve_cubics_two_seg_count",
        input,
        "path: 2 segs (const, module-embedded)",
    );
}

#[test]
fn path_svg_preserve_cubics_inflection_curve_compiles_with_nearest_channels() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 0.08 0.52 C 0.30 0.10, 0.70 0.90, 0.92 0.48",
    preserve_cubics: 1,
  )

  let sample_value = fract(time * 0.17) * curve.length
  let p = curve.point_at(sample_value)
  let n = curve.tangent_at(sample_value)
  let head = smoothstep(lo: sample_value - 0.10, hi: sample_value, x: curve.along)

  compose {
    curve |> stroke(width: 0.02) |> opacity(head)
    circle(at: p + n * 0.008, radius: 0.008 + curve.dist * 0.04)
      |> fill(#ffffff)
      |> opacity(head)
  }
}
"#;

    assert_compiles("path_svg_preserve_cubics_inflection_compile", input);
    assert_explain_contains(
        "path_svg_preserve_cubics_inflection_channels",
        input,
        "path channels: {along, dist}",
    );
}

#[test]
fn path_svg_preserve_cubics_near_degenerate_handles_compile_and_report_retention() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 0.12 0.50 C 0.1205 0.5003, 0.8795 0.4997, 0.88 0.50 C 0.8803 0.5002, 0.9197 0.5398, 0.92 0.54",
    preserve_cubics: 1,
  )

  let sample_value = (0.15 + 0.65 * fract(time * 0.11)) * curve.length
  let p = curve.point_at(sample_value)
  let n = curve.tangent_at(sample_value)

  compose {
    curve |> stroke(width: 0.018)
    circle(at: p + n * 0.006, radius: 0.006 + curve.dist * 0.03)
      |> fill(#ffffff)
  }
}
"#;

    assert_compiles("path_svg_preserve_cubics_near_degenerate_compile", input);
    assert_explain_contains(
        "path_svg_preserve_cubics_near_degenerate_retained",
        input,
        "path preprocess: cubics retained=",
    );
}

#[test]
fn path_svg_preserve_cubics_endpoint_lock_and_overlapping_handles_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "M 0.14 0.54 C 0.14 0.54, 0.92 0.46, 0.92 0.46 C 0.92 0.46, 0.60 0.80, 0.30 0.22",
    preserve_cubics: 1,
  )

  let p0 = curve.point_at(0.0)
  let p1 = curve.point_at(curve.length)
  let n0 = curve.tangent_at(0.0)
  let n1 = curve.tangent_at(curve.length)
  let p = mix(p0, p1, fract(time * 0.25))
  let n = mix(n0, n1, fract(time * 0.25))
  let gate = smoothstep(lo: 0.0, hi: 1.0, x: curve.dist * 30.0)

  compose {
    curve |> stroke(width: 0.017)
    circle(at: p + n * 0.004, radius: 0.006)
      |> fill(#ffffff)
      |> opacity(1.0 - gate)
  }
}
"#;

    assert_compiles(
        "path_svg_preserve_cubics_endpoint_lock_overlap_compile",
        input,
    );
    assert_explain_contains(
        "path_svg_preserve_cubics_endpoint_lock_overlap_channels",
        input,
        "path channels: {dist}",
    );
}

#[test]
fn path_svg_accepts_raw_path_tag_and_auto_normalizes() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "<path fill=\"var(--p)\" stroke=\"none\" d=\"M 55 142&#10;C 145 34, 275 50, 358 112&#10;C 425 161, 512 152, 562 110&#10;C 588 88, 616 94, 618 114&#10;C 620 132, 600 138, 592 126&#10;C 588 119, 594 111, 601 114&#10;C 596 113, 593 118, 596 123&#10;C 601 131, 615 127, 613 115&#10;C 611 99, 588 95, 568 116&#10;C 520 162, 424 174, 350 122&#10;C 278 70, 152 48, 55 158&#10;Z\" style=\"fill:rgb(11,11,11);stroke:none\"/>",
    preserve_cubics: 1,
  )

  let head = fract(time * 0.2) * curve.length
  let pen = curve.point_at(head)

  compose {
    curve |> stroke(width: 0.02)
    circle(at: pen, radius: 0.01) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("path_svg_raw_path_tag_auto_normalize", input);
    assert_explain_contains(
        "path_svg_raw_path_tag_auto_normalize_note",
        input,
        "path_svg: normalized to uv using primitive AABB",
    );
}

#[test]
fn path_svg_accepts_full_svg_with_viewbox_and_normalizes_from_viewbox() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let curve = path_svg(
    "<svg width=\"100%\" viewBox=\"0 0 680 210\" xmlns=\"http://www.w3.org/2000/svg\"><path d=\"M 55 142 C 145 34, 275 50, 358 112 C 425 161, 512 152, 562 110 C 588 88, 616 94, 618 114 C 620 132, 600 138, 592 126 C 588 119, 594 111, 601 114 C 596 113, 593 118, 596 123 C 601 131, 615 127, 613 115 C 611 99, 588 95, 568 116 C 520 162, 424 174, 350 122 C 278 70, 152 48, 55 158 Z\"/></svg>",
    preserve_cubics: 1,
  )

  compose {
    curve |> stroke(width: 0.02)
  }
}
"#;

    assert_compiles("path_svg_full_svg_viewbox_normalize", input);
    assert_explain_contains(
        "path_svg_full_svg_viewbox_normalize_note",
        input,
        "path_svg: normalized to uv using inferred SVG viewBox",
    );
}

#[test]
fn path_explain_reports_channel_demand_and_shared_search_note() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let curve = path {
    move (0.10, 0.30)
    cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
    arc center: (0.72, 0.55) radius: 0.10 sweep: 200deg
    }

    let head_s = (0.2 + time * 0.1) * curve.length
    let pen = curve.point_at(head_s)
    let tangent = curve.tangent_at(head_s)
    let m = smoothstep(lo: head_s - 0.1, hi: head_s, x: curve.along)
    let ink = curve |> stroke(width: 0.03)

    compose {
    ink |> opacity(m)
    circle(at: pen + tangent * 0.01, radius: 0.02 + curve.dist * 0.1)
      |> fill(#ffffff)
      |> opacity(m)
    }
  }
  "#;

    let path = unique_temp_path("path_explain_channel_demand");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("path channels: {along, dist}"),
        "expected explain receipt to include path channel demand summary\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("path nearest-search shared=true (demanded channels > 1)"),
        "expected explain receipt to include shared nearest-search note\nstderr:\n{stderr}"
    );
}
