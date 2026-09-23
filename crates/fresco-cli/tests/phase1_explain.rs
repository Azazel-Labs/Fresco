use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

#[test]
fn explain_reports_phase1_loop_and_reduce_notes() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let ys = [0.62, 0.58, 0.54]
  let cuts = [for y in ys => box(at: (0.5, y), size: (0.3, 0.02))]
  let base = circle(at: center, radius: 0.25)
  let carved = base - cuts

  let xs = [0.03, 0.02, 0.01]
  let r = reduce(xs, 0.08, op: |acc, item| acc + item)

  compose {
    for y in ys {
      box(at: (0.5, y), size: (0.8, 0.003)) |> fill(#88aaff)
    } |> blend(add)

    carved |> fill(#ffffff)
    circle(at: (0.2, 0.2), radius: r) |> fill(#ff77aa)
  }
}
"#;

    let path = unique_temp_path("phase1_explain_notes");
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
        explain.contains("loop: unrolled"),
        "expected loop explain note\nstderr:\n{explain}"
    );
    assert!(
        explain.contains("collection-subtract: lowered to ordered subtract chain"),
        "expected collection subtraction explain note\nstderr:\n{explain}"
    );
    assert!(
        explain.contains("reduce: op=lambda"),
        "expected reduce lambda explain note\nstderr:\n{explain}"
    );
}
