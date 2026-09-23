use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, unique_temp_path};

#[test]
fn svg_path_compiles_for_line_contour() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let icon = svg_path("M 0.10 0.10 L 0.90 0.10 L 0.90 0.90 L 0.10 0.90 Z")

  compose {
    icon |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase5_svg_path_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected svg_path line contour to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn svg_compiles_for_supported_subset() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let logo = svg("<svg viewBox='0 0 100 100'><path d='M10 10 L90 10 L90 90 L10 90 Z'/></svg>")

  compose {
    logo |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase5_svg_doc_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected supported svg subset to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn svg_compiles_for_triple_quoted_multiline_source() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let logo = svg("""
<svg viewBox='0 0 100 100'>
  <path d='M10 10 L90 10 L90 90 L10 90 Z'/>
</svg>
""")

  compose {
    logo |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase5_svg_doc_triple_quoted_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected triple-quoted multiline svg source to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn svg_path_rejects_unsupported_curve_command() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let bad = svg_path("M 0.10 0.10 C 0.20 0.80, 0.80 0.20, 0.90 0.90 Z")

  compose {
    bad |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase5_svg_path_curve_bad");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unsupported curve command to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unsupported SVG path command"),
        "expected unsupported path command diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn svg_rejects_unsupported_element() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let bad = svg("<svg viewBox='0 0 100 100'><rect x='10' y='10' width='80' height='80'/></svg>")

  compose {
    bad |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase5_svg_rect_bad");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unsupported svg element to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("does not support `<rect>` elements in v1"),
        "expected unsupported element diagnostic\nstderr:\n{stderr}"
    );
}
