use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, unique_temp_path};

#[test]
fn shape_minus_array_comprehension_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stripes_y = [0.772, 0.745, 0.718]
  let cuts = [for y in stripes_y => box(at: (0.29, y), size: (0.25, 0.006))]
  let sun_disc = circle(at: (0.29, 0.70), radius: 0.105)
  let sun = sun_disc - cuts

  compose {
    sun |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase1_shape_minus_array_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected shape - [shape] subtraction to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn reduce_builtin_with_shape_array_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let cuts = [
    box(at: (0.5, 0.55), size: (0.30, 0.02)),
    box(at: (0.5, 0.50), size: (0.30, 0.02)),
  ]
  let base = circle(at: center, radius: 0.25)
  let shape = reduce(cuts, base, op: subtract)

  compose {
    shape |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase1_reduce_shape_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected reduce(shape-array) to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn reduce_builtin_with_scalar_array_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let xs = [0.10, 0.12, 0.14]
  let r = reduce(xs, 0.0, op: add)

  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase1_reduce_scalar_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected reduce(scalar-array) to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn reduce_builtin_with_lambda_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let xs = [0.10, 0.12, 0.14]
  let r = reduce(xs, 0.0, op: |acc, item| acc + item)

  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase1_reduce_lambda_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected reduce(lambda) to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn reduce_builtin_with_block_lambda_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let xs = [0.10, 0.12, 0.14]
  let r = reduce(xs, 0.0, op: |acc, item| { return acc + item })

  compose {
    circle(at: center, radius: r) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("phase1_reduce_block_lambda_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected reduce(block-lambda) to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_for_loop_over_array_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let ray = capsule(from: (0.5, 0.5), to: (0.5, 0.15), radius: 0.003)
  let angles = [-30deg, 0deg, 30deg]

  compose {
    for angle in angles {
      in space rotate(angle: angle, around: center) {
        ray |> fill(#ffffff)
      }
    } |> blend(add)
  }
}
"#;

    let path = unique_temp_path("phase1_compose_for_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose for-loop to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn for_loop_rejects_non_iterable_value() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  for x in center {
    compose {
      circle(at: center, radius: 0.1) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("phase1_for_bad_iterable");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected non-iterable for-loop source to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("for-loop expects an array or range iterable"),
        "expected iterable diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn shape_subtraction_across_newlines_compiles() {
    let input = r##"canvas t(uv: coord, time: signal) -> color {
  let base = circle(at: center, radius: 0.25)
  let cutout = (
    base -
    circle(at: (0.45, 0.50), radius: 0.06) -
    circle(at: (0.55, 0.50), radius: 0.06)
  )

  compose {
    cutout |> fill(#ffffff)
  }
}
"##;

    let path = unique_temp_path("phase1_shape_sub_newline_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected newline-wrapped shape subtraction to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn each_loop_with_index_in_compose_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  param values: array<f32, 3> = [0.3, 0.6, 0.9] in 0 .. 1

  compose {
    fill(#0e1420)
    each (v, i) in values {
      let x = 0.25 + i * 0.25
      let h = v * 0.6
      box(at: (x, 0.12 + h / 2), size: (0.08, h))
        |> fill(#5eead4)
    }
  }
}
"#;

    let path = unique_temp_path("each_loop_index_compose_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected each (v, i) in array to compile in compose\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stdout = normalize(&output.stdout);
    assert!(
        stdout.contains("0.25f") && stdout.contains("0.5f") && stdout.contains("0.75f"),
        "expected 3 bars at x=0.25, 0.5, 0.75\nstdout:\n{stdout}"
    );
}

#[test]
fn each_loop_single_binding_in_compose_compiles() {
    let input = r#"canvas t(uv: coord) -> color {
  let angles = [0.0, 0.5, 1.0]

  compose {
    fill(#000000)
    each (v) in angles {
      circle(at: (v, 0.5), radius: 0.05) |> fill(#ffffff)
    }
  }
}
"#;

    let path = unique_temp_path("each_loop_single_compose_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected each (v) in array to compile in compose\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn each_loop_index_used_for_position_offset() {
    // Regression: verify that the index variable `i` is correctly bound as 0, 1, 2, ...
    // and can be used in arithmetic expressions inside the loop body.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  param vals: array<f32, 4> = [0.2, 0.4, 0.6, 0.8]

  compose {
    fill(#000000)
    each (v, i) in vals {
      let x = i * 0.2 + 0.1
      box(at: (x, 0.5), size: (0.05, v)) |> fill(#aabbcc)
    }
  }
}
"#;

    let path = unique_temp_path("each_loop_index_position_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected each loop with index arithmetic to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn array_literal_indexing_compiles() {
    // Regression: `[a, b, c][i]` should parse and evaluate to the i-th element.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stops = [0.2, 0.5, 0.8]
  let first = stops[0]
  let last = stops[2]

  compose {
    circle(at: center, radius: first) |> fill(#ff0000)
    circle(at: (0.7, 0.5), radius: last) |> fill(#0000ff)
  }
}
"#;

    let path = unique_temp_path("array_literal_index_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected array literal indexing to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn array_param_indexing_compiles() {
    // Regression: indexing a fixed-size array param `stops[1]` should work.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  param stops: array<f32, 3> = [0.2, 0.5, 0.8]
  let mid = stops[1]

  compose {
    circle(at: center, radius: mid) |> fill(#00ff00)
  }
}
"#;

    let path = unique_temp_path("array_param_index_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected fixed-size array param indexing to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn array_out_of_bounds_index_is_rejected() {
    // Array index out of bounds should produce a compile error, not silently succeed.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stops = [0.2, 0.5, 0.8]
  let bad = stops[5]

  compose {
    circle(at: center, radius: bad) |> fill(#ff0000)
  }
}
"#;

    let path = unique_temp_path("array_oob_index_err");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected out-of-bounds array index to be rejected\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn array_vec2_element_indexing_compiles() {
    // Indexing an array of vec2 values should return a vec2.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let points = [(0.2, 0.3), (0.5, 0.5), (0.8, 0.7)]
  let p = points[1]

  compose {
    circle(at: p, radius: 0.05) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("array_vec2_index_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vec2 array element indexing to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
