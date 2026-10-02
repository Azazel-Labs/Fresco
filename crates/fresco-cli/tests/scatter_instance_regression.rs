#[path = "support/common.rs"]
mod common;

use common::{
    assert_compiles_valid_wgsl, compile_to_wgsl, normalize, parse_scatter_lowering_value,
    repo_root, run_example_with_args as run_fresco_with_args,
};

#[test]
fn scatter_index_phase_keeps_all_instances_in_wgsl() {
    let root = repo_root();
    let file = root
        .join("examples")
        .join("10) fundamentals")
        .join("scatter_index_phase.fr");

    let output = run_fresco_with_args(&file, &["--explain"]);
    assert!(
        output.status.success(),
        "expected scatter_index_phase to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    let emitted_instances = parse_scatter_lowering_value(&explain, "emitted_instances");
    assert_eq!(
        emitted_instances, 12,
        "expected 12 emitted scatter instances in explain output\nexplain:\n{explain}"
    );
}

#[test]
fn scatter_body_reads_fixed_color_and_dynamic_array_params() {
    let input = r#"
canvas t(ctx: CanvasContext) -> color { let uv = context(coord);
  param radii: array<f32, 2> = [0.01, 0.02]
  param palette: array<color, 2> = [#ff0000, #00ff00]
  param bands: array<f32> = [0.3, 0.6, 0.45]
  let dots = scatter 6 within region((0.1, 0.1) .. (0.9, 0.9)) seed 3 strategy compact
  lifetime dot: 2s respawn every rand(1s .. 2s) {
    circle(at: dot.pos, radius: radii[1] + bands[0] * 0.01) |> fill(palette[1])
  }
  compose { dots |> blend(add) }
}
"#;

    assert_compiles_valid_wgsl("scatter_body_array_params", input);
    let wgsl = compile_to_wgsl("scatter_body_array_params_wgsl", input);
    let start = wgsl
        .find("fn fresco_t_scatter_l")
        .unwrap_or_else(|| panic!("expected a scatter body helper\nwgsl:\n{wgsl}"));
    let body = &wgsl[start..];
    let body = &body[..body.find("\n}\n").map_or(body.len(), |end| end + 2)];
    let signature = &body[..body.find('{').expect("scatter helper signature")];

    // Fixed-size arrays arrive as one scalar arg per slot, and the body
    // reads the indexed slot rather than a stale or default value.
    for arg in [
        "radii_0_: f32",
        "radii_1_: f32",
        "palette_1_r: f32",
        "palette_1_a: f32",
    ] {
        assert!(
            signature.contains(arg),
            "expected scatter helper arg `{arg}`\nsignature:\n{signature}"
        );
    }
    assert!(
        body.contains("radii_1_") && body.contains("palette_1_r"),
        "expected scatter body to read the indexed array slots\nbody:\n{body}"
    );
    // Dynamic arrays stay in their storage buffer instead of being passed.
    assert!(
        !signature.contains("bands"),
        "dynamic array must not be passed as a scatter arg\nsignature:\n{signature}"
    );
    assert!(
        body.contains("fresco_param_t_bands["),
        "expected scatter body to index the dynamic array storage buffer\nbody:\n{body}"
    );
}
