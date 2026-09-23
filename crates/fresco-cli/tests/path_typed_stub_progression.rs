use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

fn assert_compile_fails_with(suffix: &str, input: &str, expected_in_stderr: &str) {
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
        stderr.contains(expected_in_stderr),
        "expected stderr to contain `{expected_in_stderr}`\nstderr:\n{stderr}"
    );
}

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

#[test]
fn path_length_property_is_typed_as_scalar() {
    let input = r#"canvas t(uv: coord) -> color {
  let p = path {
    move (0.1, 0.2)
    arc center: (0.4, 0.5) radius: 0.1 sweep: 180deg
  }
  let d = p.length
  compose {
    circle(at: center, radius: 0.02 + d * 0.01) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("path_length_property_typed", input);
}

#[test]
fn path_channel_member_is_typed_for_placeholder_lowering() {
    let input = r#"canvas t(uv: coord) -> color {
  let p = path {
    move (0.1, 0.2)
    arc center: (0.4, 0.5) radius: 0.1 sweep: 180deg
  }
  let x = p.along
  compose {
    circle(at: center, radius: 0.02 + x * 0.01) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("path_channel_member_placeholder", input);
}

#[test]
fn path_point_at_is_typed_for_placeholder_lowering() {
    let input = r#"canvas t(uv: coord) -> color {
  let p = path {
    move (0.1, 0.2)
    arc center: (0.4, 0.5) radius: 0.1 sweep: 180deg
  }
  let q = point_at(path: p, s: 0.5)
  compose {
    circle(at: q, radius: 0.05) |> fill(#ffffff)
  }
}
"#;

    assert_compiles("path_point_at_placeholder", input);
}

#[test]
fn path_point_at_rejects_non_path_argument() {
    let input = r#"canvas t(uv: coord) -> color {
  let q = point_at(path: uv, s: 0.5)
  compose {
    circle(at: q, radius: 0.05) |> fill(#ffffff)
  }
}
"#;

    assert_compile_fails_with(
        "path_point_at_non_path",
        input,
        "argument `path` to `point_at` expected path, found vec2",
    );
}
