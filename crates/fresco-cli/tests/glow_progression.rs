use std::fs;

#[path = "support/common.rs"]
mod common;
use common::{normalize, run_fresco_with_args, unique_temp_path};

#[test]
fn anisotropic_glow_with_gradient_and_gaussian_falloff_compiles() {
    let input = r#"canvas t(ctx: CanvasContext) -> color { let uv = context(coord); let time = context(time);
  let haze = capsule(from: (0.12, 0.5), to: (0.88, 0.5), radius: 0.02)
    |> glow(
      reach: (0.24, 0.08),
      strength: 0.7,
      color: gradient(
        along: y,
        stops: [
          stop(at: 0.0, color: #ff2ac6),
          stop(at: 1.0, color: #66b8ff),
        ]
      ),
      falloff: gaussian,
      color_space: glow
    )

  compose {
    fill(#0b1020)
    haze |> blend(add)
  }
}
"#;

    let path = unique_temp_path("glow_progression");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--emit", "wgsl"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected anisotropic glow program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn layer_glow_with_gradient_and_linear_falloff_compiles() {
    let input = r#"canvas t(ctx: CanvasContext) -> color { let uv = context(coord); let time = context(time);
  let core = circle(at: center, radius: 0.09) |> fill(#ffffff)
  let haze = core |> glow(
    reach: (0.18, 0.08),
    strength: 0.65,
    color: gradient(
      along: y,
      stops: [
        stop(at: 0.0, color: #ff8bd6),
        stop(at: 1.0, color: #7dd3fc),
      ]
    ),
    falloff: linear
  )

  compose {
    fill(#070b15)
    haze |> blend(add)
  }
}
"#;

    let path = unique_temp_path("layer_glow_progression");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--emit", "wgsl"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected layer glow program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
