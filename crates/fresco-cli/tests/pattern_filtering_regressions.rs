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
fn filtered_patterns_in_transformed_space_do_not_use_local_px_derivatives() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  space warped = rotate(45deg) . scale(0.5)

  compose {
    in space warped {
      filtering(on)
      fill(mix(#000000, #ffffff, checker(scale: 0.1uv, at: center)))
    }
  }
}
"#;

    let output = run_source("pattern_filter_width_regression", input);
    assert!(
        output.status.success(),
        "expected compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);

    // Width conversion should use screen-space pixel size (ctx.res.y), not local px
    // from derivative-based local span, which over-blurs transformed patterns.
    assert!(
        wgsl.contains("max(ctx.res.y, 0.000001f)"),
        "expected pattern filter width to use screen-space pixel size\nwgsl:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("dpdx(") && !wgsl.contains("dpdy("),
        "unexpected derivative ops in pattern-only filtered shader\nwgsl:\n{wgsl}"
    );
}
