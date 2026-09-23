use std::fs;
use std::path::PathBuf;

use fresco::driver;
use serde_json::Value;

#[path = "support/common.rs"]
mod common;

use common::{normalize, repo_root, run_fresco, run_fresco_with_args, unique_temp_path};

#[test]
fn showcase_neon_sign_example_compiles() {
    let input = repo_root()
        .join("examples")
        .join("90) gallery")
        .join("neon_sign.fr");
    let source = fs::read_to_string(&input).expect("failed to read showcase neon_sign source");

    assert!(
        source.contains("shape: square"),
        "expected neon_sign showcase to exercise square wave support"
    );
    assert!(
        source.contains("shape: saw"),
        "expected neon_sign showcase to exercise saw wave support"
    );
    let output = common::run_example(&input);

    assert!(
        output.status.success(),
        "expected showcase neon_sign example to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn showcase_floating_boxes_example_compiles() {
    let input = repo_root()
        .join("examples")
        .join("90) gallery")
        .join("floating_boxes.fr");
    let source = fs::read_to_string(&input).expect("failed to read showcase floating_boxes source");

    assert!(
        source.contains("scatter total within screen"),
        "expected floating_boxes showcase to exercise scatter layering"
    );
    assert!(
        source.contains("wrap(x:"),
        "expected floating_boxes showcase to exercise wrap signal lowering"
    );
    assert!(
        source.contains("rotate(angle:"),
        "expected floating_boxes showcase to exercise rotate space transforms"
    );

    let output = common::run_example(&input);

    assert!(
        output.status.success(),
        "expected showcase floating_boxes example to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn showcase_perspective_flip_card_preserves_background_through_compose_alpha() {
    let input = repo_root()
        .join("examples")
        .join("90) gallery")
        .join("perspective_flip_card.fr");

    let output = common::run_example_with_args(&input, &["--explain"]);

    assert!(
        output.status.success(),
        "expected perspective_flip_card showcase to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stdout = normalize(&output.stdout);
    let stderr = normalize(&output.stderr);
    let explain = format!("{stdout}\n{stderr}");
    let final_mix = explain
        .lines()
        .find(|line| line.contains("let col = mix("))
        .expect("expected explain output to include final `let col = mix(...)` line");

    assert!(
        !final_mix.contains("vec3(1f)"),
        "expected final compose mix to use computed alpha, not forced opaque alpha\nline:\n{final_mix}\n"
    );
}

#[test]
fn compose_if_keeps_derivative_work_uniform_and_selects_one_face() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let front = circle(at: center, radius: 0.25) |> fill(#f5f5f5ff)
    let back = circle(at: center, radius: 0.25) |> fill(#1f2937ff)
    let spin = cos(time)

    compose {
        if spin {
            front
        } else {
            back
        }
    }
}
"#;

    let path = unique_temp_path("compose_if_dynamic_branch");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose-if sample to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    let module = naga::front::wgsl::parse_str(&wgsl).expect("valid WGSL");
    let scene = module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some("fresco_scene_t"))
        .expect("scene function")
        .1;
    let selections = scene
        .expressions
        .iter()
        .filter_map(|(_, expression)| {
            if let naga::Expression::Select { condition, .. } = expression {
                Some(*condition)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(selections.len(), 2, "RGB and alpha must both select a face");
    assert_eq!(
        selections[0], selections[1],
        "RGB and alpha must use the same dynamic condition"
    );
    assert!(
        scene
            .expressions
            .iter()
            .any(|(_, expression)| matches!(expression, naga::Expression::Derivative { .. })),
        "AA derivatives must remain"
    );
    assert!(
        !scene
            .body
            .iter()
            .any(|statement| matches!(statement, naga::Statement::If { .. })),
        "derivative arms must not run in divergent control flow"
    );
}

#[test]
fn compose_else_if_chain_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let a = circle(at: center, radius: 0.2) |> fill(#ef4444ff)
    let b = circle(at: center, radius: 0.2) |> fill(#22c55eff)
    let c = circle(at: center, radius: 0.2) |> fill(#3b82f6ff)
    let x = uv.x

    compose {
        if x < 0.33 {
            a
        } else if x < 0.66 {
            b
        } else {
            c
        }
    }
}
"#;

    let path = unique_temp_path("compose_else_if_chain");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose else-if chain to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_for_body_supports_if_else_statements() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let treble = 0.72

    compose {
        for i in 0 .. 120 {
            let fi = i / 120.0
            let sx = noise2((fi * 73.1, fi * 19.7))
            let sy = noise2((fi * 41.3, fi * 91.9))
            let tw = wave(period: 2.8s, shape: sine, phase: fi * 6.283, range: 0.35 .. 1.0)
            let r = 0.0008 + 0.0015 * noise2((fi * 7.0, fi * 33.0))

            if sy > 0.45 {
                circle(at: (sx, 0.48 + sy * 0.48), radius: r)
                    |> fill(#b9d8ff * tw * (0.35 + 1.15 * treble))
            }
        }
    }
}
"#;

    let path = unique_temp_path("compose_for_if_stmt");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose-for body with if-statement to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_for_if_branches_accept_blend_pipe_syntax() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        for i in 0 .. 8 {
            let fi = i / 8.0
            let star = circle(at: (0.1 + fi * 0.8, 0.7), radius: 0.01)
            if fi > 0.5 {
                star |> fill(#9ad7ff) |> blend(add)
            } else {
                star |> fill(#264653)
            }
        }
    }
}
"#;

    let path = unique_temp_path("compose_for_if_blend_pipe");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose-for if branches with blend pipe syntax to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_accepts_dynamic_color_returned_from_function_call() {
    let input = r#"fn tone(uv: coord_like, time: signal) -> color {
    let wave = 0.5 + 0.5 * sin((uv.x + uv.y) * 8.0 + time)
    return rgba(
        r: 0.15 + wave * 0.75,
        g: 0.08 + wave * 0.40,
        b: 0.30 + (1.0 - wave) * 0.55,
        a: 1.0
    )
}

canvas t(uv: coord, time: signal) -> color {
    compose {
        tone(uv, time)
    }
}
"#;

    let path = unique_temp_path("compose_fn_dynamic_color");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose entry to accept function-returned dynamic color\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn warp_and_closed_form_signals_compile_with_coord_ambient() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    space shimmer = warp(by: (
        (fbm(uv * 6.0 + (time * 0.1, 0.0), octaves: 3) - 0.5) * 0.03,
        (fbm(uv * 6.0 + (7.0, time * 0.08), octaves: 3) - 0.5) * 0.02
    ))

    let flash = pulse(every: 2s, width: 140ms, ease: out_quad)
    let arrive = ramp(from: 0s, over: 1.2s, ease: out_back)

    in space shimmer {
        compose {
            circle(at: center, radius: 0.14 + arrive * 0.03)
            |> fill(rgb(0.2, 0.5, 0.9) * (0.6 + flash * 0.4))
        }
    }
}
"#;

    let path = unique_temp_path("warp_pulse_ramp_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected warp/pulse/ramp sample to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn millisecond_units_compile_for_signal_windows() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let flash = pulse(every: 2s, width: 100ms, ease: out_quad)
    let arrive = ramp(from: 0ms, over: 1200ms, ease: out_back)

    compose {
        circle(at: center, radius: 0.12 + arrive * 0.02)
        |> fill(rgb(0.15, 0.5, 0.9) * (0.5 + flash * 0.5))
    }
}
"#;

    let path = unique_temp_path("ms_units_signals_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected millisecond signal literals to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn turn_units_compile_for_angle_math() {
    let input = r#"canvas t(uv: coord) -> color {
    let spin = 0.25turn

    compose {
        in space rotate(angle: spin) {
            box(at: center, size: (0.35, 0.16)) |> fill(#ffffff)
        }
    }
}
"#;

    let path = unique_temp_path("turn_units_angles_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected turn angle literals to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn pulse_and_ramp_support_transition_and_mode_args() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let flash = pulse(every: 2s, width: 120ms, transition: quad, mode: ease_in_out)
    let arrive = ramp(from: 0ms, over: 1200ms, transition: cubic, mode: out_in)

    compose {
        circle(at: center, radius: 0.11 + arrive * 0.03)
        |> fill(rgb(0.2, 0.55, 0.95) * (0.45 + flash * 0.55))
    }
}
"#;

    let path = unique_temp_path("signal_transition_mode_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected pulse/ramp transition+mode sample to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn pulse_rejects_mixing_ease_with_transition_mode() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let flash = pulse(every: 2s, width: 120ms, ease: out_quad, transition: quad, mode: out)
    compose {
        circle(at: center, radius: 0.1) |> fill(rgb(0.2, 0.4, 0.8) * flash)
    }
}
"#;

    let path = unique_temp_path("signal_mixed_ease_mode_error");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected pulse shorthand+structured ease mix to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("either `ease: ...` or `transition: ...`/`mode: ...`, not both"),
        "expected explicit mixed-easing diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn nested_warp_chains_with_coord_driven_shape_params_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    space coarse = warp(by: (
        (noise2(uv * 4.0 + (time * 0.1, 0.0)) - 0.5) * 0.04,
        (noise2(uv * 4.0 + (11.0, time * 0.1)) - 0.5) * 0.03
    ))
    space fine = warp(by: (
        (fbm(uv * 12.0 + (time * 0.2, 0.0), octaves: 3) - 0.5) * 0.01,
        (fbm(uv * 12.0 + (5.0, time * 0.2), octaves: 3) - 0.5) * 0.01
    ))

    in space coarse . fine {
        compose {
            circle(
                at: center + ((uv.x - 0.5) * 0.02, (uv.y - 0.5) * 0.02),
                radius: 0.14 + (uv.x * 0.02)
            ) |> fill(#79a7ff)
        }
    }
}
"#;

    let path = unique_temp_path("warp_nested_coord_shape_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected nested coord-driven warp chains to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn warp_rejects_non_vec2_displacement_with_clear_diagnostic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    in space warp(by: 0.03) {
        compose {
            circle(at: center, radius: 0.2) |> fill(#ffffff)
        }
    }
}
"#;

    let path = unique_temp_path("warp_bad_by_scalar");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected warp(by: scalar) to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("expected a vec2 like `(x, y)`, found a scalar"),
        "expected vec2 diagnostic for warp displacement\nstderr:\n{stderr}"
    );
}

#[test]
fn showcase_basic_wave_shapes_example_compiles() {
    let input = repo_root()
        .join("examples")
        .join("10) fundamentals")
        .join("wave_saw_square.fr");
    let output = common::run_example(&input);

    assert!(
        output.status.success(),
        "expected showcase wave_saw_square example to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn showcase_basic_scatter_index01_example_compiles() {
    let input = repo_root()
        .join("examples")
        .join("10) fundamentals")
        .join("scatter_index_phase.fr");
    let output = common::run_example(&input);

    assert!(
        output.status.success(),
        "expected showcase scatter_index_phase example to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn showcase_basic_wave_triangle_example_compiles() {
    let input = repo_root()
        .join("examples")
        .join("10) fundamentals")
        .join("wave_triangle_motion.fr");
    let output = common::run_example(&input);

    assert!(
        output.status.success(),
        "expected showcase wave_triangle_motion example to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn showcase_basic_sine_phase_example_compiles() {
    let input = repo_root()
        .join("examples")
        .join("10) fundamentals")
        .join("sine_phase_offsets.fr");
    let output = common::run_example(&input);

    assert!(
        output.status.success(),
        "expected showcase sine_phase_offsets example to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn field_layer_forms_compile_with_grey_bridge() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let wobble = field (time * 0.25)
  let fog = layer grey(wobble)

  compose {
    fog
  }
}
"#;

    let path = unique_temp_path("field_layer_grey_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected field/layer forms to compile with grey bridge\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn color_params_and_optional_scalar_ranges_compile_and_emit_manifest_defaults() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  param accent: color = rgb(0.1, 0.2, 0.3)
  param spread: f32 = 0.35

  compose {
    circle(at: center, radius: 0.1 + spread * 0.05)
    |> fill(accent)
  }
}
"#;

    let path = unique_temp_path("color_param_manifest");
    fs::write(&path, input).expect("failed to write temporary test source");

    let output = run_fresco(&path);
    assert!(
        output.status.success(),
        "expected color params to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let _ = fs::remove_file(&path);

    let manifest = driver::compile_source(input, common::TEST_SOURCE_PATH, "manifest", false)
        .expect("expected manifest emit for color params to compile");
    let stdout = manifest.emitted;
    let parsed: Value =
        serde_json::from_str(&stdout).expect("expected manifest JSON to deserialize");
    let params = parsed["canvases"][0]["params"]
        .as_array()
        .expect("expected canvas params array");
    let accent = params
        .iter()
        .find(|p| p["name"] == "accent")
        .expect("expected accent param");
    let spread = params
        .iter()
        .find(|p| p["name"] == "spread")
        .expect("expected spread param");
    assert!(
        accent["type"] == "color",
        "expected color param type metadata in manifest\nstdout:\n{stdout}"
    );
    assert!(
        accent["default"] == serde_json::json!([0.1, 0.2, 0.3, 1.0]),
        "expected color default array in manifest\nstdout:\n{stdout}"
    );
    assert!(
        spread["min"].is_null() && spread["max"].is_null(),
        "expected optional scalar range omission to serialize as null\nstdout:\n{stdout}"
    );
}

#[test]
fn glow_and_shadow_accept_dynamic_color_expressions() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    param tint: color = rgb(0.12, 0.56, 0.92)
    let pulse = wave(period: 2s, shape: sine, range: 0.55 .. 1.0)

    compose {
        circle(at: center, radius: 0.16)
        |> shadow(
            offset: (0.02, 0.025),
            soften: 0.04,
            color: tint * rgba(0.4, 0.45, 0.5, 0.65)
        )

        circle(at: center, radius: 0.16)
        |> fill(tint)
        |> glow(reach: 0.08, strength: pulse, color: tint * rgb(1.0, 0.9, 0.8))
    }
}
"#;

    let path = unique_temp_path("dynamic_effect_color_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected dynamic effect colors to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn typed_scalar_and_bool_params_emit_manifest_defaults() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    param steps: i32 = 4 in 1 .. 9
    param bands: u32 = 6 in 1 .. 12
    param enabled: bool = true

    let gate = select(0.0, 1.0, enabled)
    let radius = 0.08 + f32(steps) * 0.003 + f32(bands) * 0.001 + gate * 0.02

    compose {
        circle(at: center, radius: radius) |> fill(#99bbff)
    }
}
"#;

    let path = unique_temp_path("typed_param_manifest");
    fs::write(&path, input).expect("failed to write temporary test source");

    let output = run_fresco(&path);
    assert!(
        output.status.success(),
        "expected typed scalar params to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let _ = fs::remove_file(&path);

    let manifest = driver::compile_source(input, common::TEST_SOURCE_PATH, "manifest", false)
        .expect("expected manifest emit for typed params to compile");
    let stdout = manifest.emitted;
    let parsed: Value =
        serde_json::from_str(&stdout).expect("expected manifest JSON to deserialize");
    let params = parsed["canvases"][0]["params"]
        .as_array()
        .expect("expected canvas params array");
    let steps = params
        .iter()
        .find(|p| p["name"] == "steps")
        .expect("expected steps param");
    let bands = params
        .iter()
        .find(|p| p["name"] == "bands")
        .expect("expected bands param");
    let enabled = params
        .iter()
        .find(|p| p["name"] == "enabled")
        .expect("expected enabled param");
    assert!(
        steps["type"] == "i32"
            && steps["default"] == 4
            && steps["min"] == 1.0
            && steps["max"] == 9.0,
        "expected i32 param metadata in manifest\nstdout:\n{stdout}"
    );
    assert!(
        bands["type"] == "u32"
            && bands["default"] == 6
            && bands["min"] == 1.0
            && bands["max"] == 12.0,
        "expected u32 param metadata in manifest\nstdout:\n{stdout}"
    );
    assert!(
        enabled["type"] == "bool"
            && enabled["default"] == true
            && enabled["min"].is_null()
            && enabled["max"].is_null(),
        "expected bool param metadata in manifest\nstdout:\n{stdout}"
    );
}

#[test]
fn param_type_info_is_present_in_manifest() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    param speed: f32 = 1.0
    param tint: color = rgb(1.0, 0.5, 0.0)
    param stops: array<f32, 3> = [0.2, 0.5, 0.8]

    compose {
        circle(at: center, radius: 0.1 + speed * 0.01) |> fill(tint)
    }
}
"#;

    let manifest = driver::compile_source(input, common::TEST_SOURCE_PATH, "manifest", false)
        .expect("expected manifest emit to succeed");
    let stdout = manifest.emitted;
    let parsed: Value =
        serde_json::from_str(&stdout).expect("expected manifest JSON to deserialize");
    let params = parsed["canvases"][0]["params"]
        .as_array()
        .expect("expected canvas params array");

    let speed = params
        .iter()
        .find(|p| p["name"] == "speed")
        .expect("expected speed param");
    let tint = params
        .iter()
        .find(|p| p["name"] == "tint")
        .expect("expected tint param");
    let stops = params
        .iter()
        .find(|p| p["name"] == "stops")
        .expect("expected stops param");

    assert_eq!(
        speed["param_type"]["name"], "f32",
        "expected f32 param_type name\nstdout:\n{stdout}"
    );
    assert!(
        speed["param_type"]["params"].is_null(),
        "expected no params for scalar type\nstdout:\n{stdout}"
    );
    assert_eq!(
        tint["param_type"]["name"], "color",
        "expected color param_type name\nstdout:\n{stdout}"
    );
    assert_eq!(
        stops["param_type"]["name"], "array",
        "expected array param_type name\nstdout:\n{stdout}"
    );
    assert_eq!(
        stops["param_type"]["params"][0], "f32",
        "expected array element type to be f32\nstdout:\n{stdout}"
    );
    assert_eq!(
        stops["param_type"]["size"], 3,
        "expected fixed array size in param_type\nstdout:\n{stdout}"
    );
}

#[test]
fn dynamic_array_param_type_has_no_size() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    param weights: array<f32> = []

    compose {
        circle(at: center, radius: 0.1) |> fill(#ff0000)
    }
}
"#;

    let manifest = driver::compile_source(input, common::TEST_SOURCE_PATH, "manifest", false)
        .expect("expected manifest emit to succeed");
    let stdout = manifest.emitted;
    let parsed: Value =
        serde_json::from_str(&stdout).expect("expected manifest JSON to deserialize");

    let params = parsed["canvases"][0]["params"]
        .as_array()
        .expect("expected canvas params array");
    let storage_params = parsed["canvases"][0]["storage_params"]
        .as_array()
        .expect("expected storage_params array");

    let param = params
        .iter()
        .find(|p| p["name"] == "weights")
        .expect("expected weights param");
    let storage = storage_params
        .iter()
        .find(|p| p["name"] == "weights")
        .expect("expected weights storage param");

    assert_eq!(
        param["param_type"]["name"], "array",
        "param_type.name should be array"
    );
    assert_eq!(
        param["param_type"]["params"][0], "f32",
        "param_type.params[0] should be f32"
    );
    assert!(
        param["param_type"]["size"].is_null(),
        "dynamic array should have no size in param_type\nstdout:\n{stdout}"
    );
    assert_eq!(
        storage["param_type"]["name"], "array",
        "storage param_type.name should be array"
    );
    assert_eq!(
        storage["param_type"]["params"][0], "f32",
        "storage param_type.params[0] should be f32"
    );
    assert!(
        storage["param_type"]["size"].is_null(),
        "dynamic storage array should have no size in param_type\nstdout:\n{stdout}"
    );
}

#[test]
fn dynamic_array_example_emits_readonly_storage_buffer_access() {
    let input = repo_root()
        .join("examples")
        .join("20) techniques")
        .join("dynamic_array_equalizer.fr");

    let output = common::run_example_with_args(&input, &[]);

    assert!(
        output.status.success(),
        "expected dynamic array example to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("fresco_param_dynamic_array_equalizer_bands["),
        "expected WGSL to contain direct storage buffer indexing for `bands`\nwgsl:\n{wgsl}"
    );
    assert!(
        wgsl.contains("var<storage, read> fresco_param_dynamic_array_equalizer_bands"),
        "expected dynamic array storage param to be read-only in WGSL\nwgsl:\n{wgsl}"
    );
}

#[test]
fn dynamic_array_non_empty_default_populates_manifest() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    param weights: array<f32> = [0.25, 0.5, 0.75]

    compose {
        circle(at: center, radius: 0.1) |> fill(#ff0000)
    }
}
"#;

    let manifest = driver::compile_source(input, common::TEST_SOURCE_PATH, "manifest", false)
        .expect("expected manifest emit to succeed with non-empty default");
    let parsed: Value =
        serde_json::from_str(&manifest.emitted).expect("expected manifest JSON to deserialize");

    let params = parsed["canvases"][0]["params"]
        .as_array()
        .expect("expected canvas params array");
    let param = params
        .iter()
        .find(|p| p["name"] == "weights")
        .expect("expected weights param");

    let default_values = param["default"]["values"]
        .as_array()
        .expect("expected default.values to be an array");

    assert_eq!(
        default_values.len(),
        3,
        "expected 3 default values\nparam:\n{param}"
    );
    assert!(
        (default_values[0].as_f64().unwrap() - 0.25).abs() < 1e-5,
        "expected default[0] ≈ 0.25"
    );
    assert!(
        (default_values[1].as_f64().unwrap() - 0.5).abs() < 1e-5,
        "expected default[1] ≈ 0.5"
    );
    assert!(
        (default_values[2].as_f64().unwrap() - 0.75).abs() < 1e-5,
        "expected default[2] ≈ 0.75"
    );
}

#[test]
fn layer_form_rejects_scalar_without_bridge() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let bad = layer 0.5
  compose {
    bad
  }
}
"#;

    let path = unique_temp_path("layer_scalar_reject");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected scalar layer form to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`layer` expects a color/layer expression, found scalar"),
        "expected layer coercion diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn wave_saw_with_reversed_range_compiles_with_normalization_warning() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let drift = wave(period: 4s, shape: saw, phase: 0.25, range: 0.3 .. -0.3)
  compose {
    circle(at: center + (drift, 0), radius: 18px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("wave_saw_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected saw wave with reversed range to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("warning") && stderr.contains("range is inverted; normalizing endpoints"),
        "expected inverted-range normalization warning\nstderr:\n{stderr}"
    );
}

#[test]
fn rand_with_reversed_range_is_rejected_outside_scatter() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let jitter = rand(range: 1.0 .. -1.0)
  compose {
    circle(at: center + (jitter * 0.05, 0), radius: 14px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("rand_reversed_range_warn");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected rand outside scatter to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`rand(...)` is only supported inside `scatter` for now"),
        "expected outside-scatter rand diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn wrap_with_reversed_range_compiles_with_normalization_warning() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let x = wrap(x: time, range: 0.8 .. -0.2)
  compose {
    circle(at: (x, 0.5), radius: 10px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("wrap_reversed_range_warn");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected wrap with reversed range to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("warning") && stderr.contains("range is inverted; normalizing endpoints"),
        "expected inverted-range normalization warning\nstderr:\n{stderr}"
    );
}

#[test]
fn cross_kind_distance_and_coverage_arithmetic_is_rejected() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let d = field(0.2)
    let c = wave(period: 2s, shape: sine, range: 0.0 .. 1.0)
    let bad = d + c
    compose {
        circle(at: center + (bad * 0.0, 0.0), radius: 14px) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("cross_kind_distance_coverage_reject");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unsafe cross-kind arithmetic to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unsafe cross-kind arithmetic")
            && stderr.contains("distance")
            && stderr.contains("coverage"),
        "expected targeted cross-kind diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn scalar_kind_promotion_allows_scalar_with_mask_arithmetic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let c = wave(period: 2s, shape: triangle, range: 0.0 .. 1.0)
    let biased = c * 0.4 + 0.2
    compose {
        circle(at: center + ((biased - 0.5) * 0.08, 0), radius: 14px) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("scalar_kind_promotion_mask_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scalar/mask promotion arithmetic to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn rand_dynamic_range_is_rejected_outside_scatter() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let low = 0.2 + time * 0.0
    let high = 0.8 - time * 0.0
    let jitter = rand(range: low .. high)
    compose {
        circle(at: center + ((jitter - 0.5) * 0.1, 0), radius: 12px) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("rand_dynamic_range_preserve");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected dynamic rand range outside scatter to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`rand(...)` is only supported inside `scatter` for now"),
        "expected outside-scatter rand diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn wave_dynamic_range_endpoints_are_preserved() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let low = 0.15 + time * 0.0
    let high = 0.85 - time * 0.0
    let drift = wave(period: 3s, shape: saw, phase: 0.1, range: low .. high)
    compose {
        circle(at: center + ((drift - 0.5) * 0.2, 0), radius: 12px) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("wave_dynamic_range_preserve");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected dynamic wave range to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let combined = format!(
        "{}\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
    assert!(
        combined.contains(
            "`wave(range: ...)` range kept as-authored (dynamic endpoints are preserved at runtime)"
        ),
        "expected dynamic wave range preservation note\noutput:\n{combined}"
    );
}

#[test]
fn wrap_dynamic_range_endpoints_are_preserved() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let low = 0.2 + time * 0.0
    let high = 0.8 - time * 0.0
    let x = wrap(x: time * 0.25, range: low .. high)
    compose {
        circle(at: (x, 0.5), radius: 10px) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("wrap_dynamic_range_preserve");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected dynamic wrap range to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let combined = format!(
        "{}\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
    assert!(
            combined.contains("`wrap(x, range: ...)` range kept as-authored (dynamic endpoints are preserved at runtime)"),
            "expected dynamic wrap range preservation note\noutput:\n{combined}"
        );
}

#[test]
fn wave_accepts_explicit_numeric_list_argument() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let drift = wave(period: 4s, shape: saw, phase: 0.25, range: (0.3, -0.3))
  compose {
    circle(at: center + (drift, 0), radius: 18px) |> fill(#ffffff)
  }
}
"#;

    let path = unique_temp_path("wave_range_list_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explicit numeric list wave range to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_unknown_names_do_not_emit_block_level_fallback_error() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  compose {
        reflection_fill |> blend(add)
        reflection_rim |> blend(add)
        reflection_glow |> blend(add)
  }
}
"#;

    let path = unique_temp_path("compose_unknown_name_span");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unknown compose names to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unknown name `reflection_fill`"),
        "expected missing compose name diagnostic\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("unknown name `reflection_rim`"),
        "expected missing compose name diagnostic\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("unknown name `reflection_glow`"),
        "expected missing compose name diagnostic\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("this block produces no layer"),
        "did not expect fallback block-level diagnostic when concrete compose errors already exist\nstderr:\n{stderr}"
    );
}

#[test]
fn top_level_no_layer_diagnostic_anchors_to_canvas_name_span() {
    let input = r#"canvas anchor_test(uv: coord, time: signal) -> color {
  let pulse = wave(period: 1s, shape: sine, range: 0.2 .. 0.8)
}
"#;

    let result = driver::compile_source(input, common::TEST_SOURCE_PATH, "wgsl", false);
    let diags = result.expect_err("expected canvas-without-layer source to fail compilation");
    let no_layer = diags
        .iter()
        .find(|d| d.message == "this block produces no layer")
        .expect("expected top-level no-layer diagnostic");

    let expected_start = input
        .find("anchor_test")
        .expect("test source must include canvas name token");
    let expected_end = expected_start + "anchor_test".len();

    assert_eq!(
        no_layer.span_start, expected_start,
        "expected top-level no-layer diagnostic to start at canvas name"
    );
    assert_eq!(
        no_layer.span_end, expected_end,
        "expected top-level no-layer diagnostic to end at canvas name"
    );
}

#[test]
fn scatter_region_lifecycle_and_instance_index01_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let meteors = scatter 8 within region((0, 0) .. (1, 0.55)) seed 7 strategy compact
                      lifetime meteor: 0.7s respawn every rand(5s .. 9s) {
        let drift = wave(period: 8s, shape: saw, phase: meteor.index01, range: 0.2 .. -0.2)
        capsule(from: meteor.pos,
                        to: meteor.pos + (0.08 + drift, 0),
            radius: 0.01) |> fill(#ffffff)
  }

  compose {
    meteors
  }
}
"#;

    let path = unique_temp_path("scatter_index01_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter lifecycle/region/index01 semantics to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn centered_space_and_rotate_around_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    space stage = centered(aspect: preserve)
    let spin = wave(period: 3s, shape: sine, range: -20deg .. 20deg)

    compose {
        in space stage . rotate(angle: spin, around: center) {
            box(at: center, size: (0.35, 0.2)) |> fill(#ffffff)
        }
    }
}
"#;

    let path = unique_temp_path("centered_rotate_around_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected centered + rotate(around:) to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn field_stdlib_noise_fbm_wrap_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let p = center + (time * 0.05, 0.0)
    let n = field noise2(p)
    let f = field fbm(p, octaves: 5)
    let w = field wrap(x: n + f, range: -1.0 .. 1.0)
    let fog = layer grey(w)

    compose {
        fog
    }
}
"#;

    let path = unique_temp_path("field_stdlib_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected noise2/fbm/wrap field stdlib forms to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn layer_color_expression_math_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let ripple = field wave(period: 5s, shape: saw, range: 0.2 .. 0.9)
    let blow = layer (#224466 / (0.1 - ripple))

    compose {
        blow
    }
}
"#;

    let path = unique_temp_path("layer_color_expr_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected layer color-expression math to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn gradient_supports_vector_direction() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let hills = (circle(at: (0.3, 0.1), radius: 0.4) | circle(at: (0.75, 0.05), radius: 0.45))
              |> smooth(0.08)

    compose {
        hills |> fill(gradient(
            along: (1.0, 1.0),
            stops: [
                stop(at: 0.0, color: #0f2518),
                stop(at: 1.0, color: #4f8f63)
            ]
        ))
    }
}
"#;

    let path = unique_temp_path("gradient_vector_direction_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected vector-direction gradient to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn gradient_supports_multiple_stops() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let hills = (circle(at: (0.3, 0.1), radius: 0.4) | circle(at: (0.75, 0.05), radius: 0.45))
              |> smooth(0.08)

    compose {
        hills |> fill(gradient(
            along: y,
            stops: [
                stop(at: 0.0, color: #102117),
                stop(at: 0.35, color: #1f4a31),
                stop(at: 0.7, color: #3f7e58),
                stop(at: 1.0, color: #70b087),
            ]
        ))
    }
}
"#;

    let path = unique_temp_path("gradient_multi_stop_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected multi-stop gradient to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn gradient_supports_radial_kind() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let orb = circle(at: center, radius: 0.35)

    compose {
        orb |> fill(gradient(
            kind: radial,
            center: center,
            radius: 0.35,
            stops: [
                stop(at: 0.0, color: #fff3bf),
                stop(at: 0.45, color: #ffb347),
                stop(at: 1.0, color: #9a3412)
            ]
        ))
    }
}
"#;

    let path = unique_temp_path("gradient_radial_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected radial gradient to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn gradient_supports_paramized_stops_and_colors() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    param split: f32 = 0.42 in 0.0 .. 1.0
    param warm: f32 = 0.78 in 0.0 .. 1.0
    let orb = circle(at: center, radius: 0.35)

    compose {
        orb |> fill(gradient(
            kind: radial,
            center: center,
            radius: 0.35,
            stops: [
                stop(at: 0.0, color: rgb(r: 1.0, g: 0.95, b: 0.75)),
                stop(at: split, color: rgb(r: warm, g: 0.55, b: 0.28)),
                stop(at: 1.0, color: #6a281f)
            ]
        ))
    }
}
"#;

    let path = unique_temp_path("gradient_param_stops_colors_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected gradient paramized stops/colors to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn gradient_supports_legacy_start_end_syntax() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let orb = circle(at: center, radius: 0.3)

    compose {
        orb |> fill(gradient(along: y, start: #102117, end: #70b087))
    }
}
"#;

    let path = unique_temp_path("gradient_legacy_start_end_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected legacy gradient syntax to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn dynamic_shape_fill_color_expression_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let pulse = wave(period: 2s, shape: triangle, range: 0.2 .. 1.0)
    compose {
        circle(at: center, radius: 0.22) |> fill(rgb(r: pulse, g: 0.25, b: 0.8))
    }
}
"#;

    let path = unique_temp_path("dynamic_fill_expr_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected dynamic shape fill expression to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn local_function_can_shadow_stdlib_noise2_wrapper() {
    let input = r#"fn noise2(p: vec2) -> f32 {
    return 0.25
}

canvas t(uv: coord, time: signal) -> color {
    let n = noise2(uv)
    compose {
        circle(at: center, radius: 0.12 + n * 0.08) |> fill(#66aaff)
    }
}
"#;

    let path = unique_temp_path("shadow_stdlib_noise2_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected local noise2 function to shadow stdlib wrapper\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn function_overloads_by_arity_compile() {
    let input = r#"fn wobble(x: f32) -> f32 {
    return x * 0.5
}

fn wobble(x: f32, gain: f32) -> f32 {
    return x * gain
}

canvas t(uv: coord, time: signal) -> color {
    let a = wobble(0.2)
    let b = wobble(0.2, 0.8)
    compose {
        circle(at: center, radius: 0.1 + a + b) |> fill(#88bbff)
    }
}
"#;

    let path = unique_temp_path("overload_arity_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected overload-by-arity functions to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn wave_triangle_and_square_shapes_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let tri = wave(period: 3s, shape: triangle, range: -0.2 .. 0.2)
    let sq = wave(period: 2s, shape: square, range: 0.1 .. 0.3)

    compose {
        circle(at: center + (tri, 0), radius: 18px + sq) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("wave_triangle_square_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected wave triangle/square shapes to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn wave_supports_transition_and_mode_phase_shaping() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let pulse_a = wave(period: 2.2s, shape: sine, transition: quad, mode: in_out, range: 0.05 .. 0.18)
    let pulse_b = wave(period: 2.2s, shape: triangle, ease: out_in_cubic, range: 0.02 .. 0.08)

    compose {
        circle(at: center, radius: pulse_a + pulse_b) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("wave_transition_mode_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected wave transition/mode sample to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn wave_rejects_mixing_ease_with_transition_mode() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let bad = wave(period: 2s, shape: sine, ease: out_quad, transition: quad, mode: out, range: 0.05 .. 0.16)
    compose {
        circle(at: center, radius: bad) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("wave_mixed_ease_mode_error");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected wave shorthand+structured easing mix to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("either `ease: ...` or `transition: ...`/`mode: ...`, not both"),
        "expected explicit mixed-easing diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn centered_fit_and_fill_modes_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    space framed_fit = centered(aspect: fit)
    space framed_fill = centered(aspect: fill)

    compose {
        in space framed_fit {
            box(at: center, size: (0.5, 0.3)) |> fill(#22ccff)
        }

        in space framed_fill {
            box(at: center, size: (0.2, 0.8)) |> fill(#ff8844)
        }
    }
}
"#;

    let path = unique_temp_path("centered_modes_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected centered fit/fill modes to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn color_component_selectors_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let base = rgb(r: 0.2, g: 0.4, b: 0.8)
    let drift = wave(period: 2s, shape: triangle, range: 0.1 .. 0.9)

    compose {
        circle(at: center, radius: 0.22)
            |> fill(rgba(r: base.r + drift * 0.1,
                                     g: base.g,
                                     b: base.b,
                                     a: 1.0))
    }
}
"#;

    let path = unique_temp_path("color_component_selectors_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected color component selectors to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn noise2_and_fbm_value_noise_paths_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let p = center + (time * 0.07, 0.0)
    let n = field noise2(p)
    let f = field fbm(p, octaves: 6, lacunarity: 2.1, gain: 0.52)

    compose {
        layer grey((n + f) * 0.5)
    }
}
"#;

    let path = unique_temp_path("value_noise_paths_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected value-noise-based noise2/fbm paths to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn selector_chain_reports_scalar_terminal_diagnostic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
      let base = rgb(r: 0.2, g: 0.4, b: 0.8)
      let bad = base.r.x

      compose {
        layer grey(bad)
      }
    }
    "#;

    let path = unique_temp_path("selector_chain_bad");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected invalid selector chain to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`base.r` is a scalar, so `base.r.x` is invalid"),
        "expected scalar terminal selector diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn selector_unknown_component_reports_diagnostic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
      let base = rgb(r: 0.2, g: 0.4, b: 0.8)
      let bad = base.q

      compose {
        layer grey(bad)
      }
    }
    "#;

    let path = unique_temp_path("selector_unknown_component_bad");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unknown selector component to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unknown color component `q` on `base`"),
        "expected unknown component diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn unknown_name_short_circuits_dependent_diagnostics() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let a = missing_name
    let b = a + 1.0

    compose {
      layer grey(b)
    }
}
"#;

    let path = unique_temp_path("unknown_name_short_circuit");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected missing-name sample to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unknown name `missing_name`"),
        "expected root unknown-name diagnostic\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("expected a scalar"),
        "did not expect a dependent scalar coercion diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn compose_pipe_blend_syntax_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let a = circle(at: (0.42, 0.50), radius: 0.18) |> fill(#ff7dbb)
    let b = circle(at: (0.58, 0.50), radius: 0.18) |> fill(#6ba6ff)

    compose {
        fill(#0b1324)
        a
        b |> blend(add)
    }
}
"#;

    let path = unique_temp_path("compose_pipe_blend_syntax");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compose pipe blend syntax to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn compose_legacy_blend_suffix_is_rejected() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let a = circle(at: (0.42, 0.50), radius: 0.18) |> fill(#ff7dbb)

    compose {
        fill(#0b1324)
        a blend: add
    }
}
"#;

    let path = unique_temp_path("compose_legacy_blend_suffix");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected legacy compose blend suffix to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unexpected `blend`"),
        "expected parser to reject legacy `blend: mode` syntax\nstderr:\n{stderr}"
    );
}

#[test]
fn color_arithmetic_invalid_operand_mix_reports_diagnostic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let c = rgb(r: 0.2, g: 0.4, b: 0.8)
    let bad = c + center

    compose {
      layer bad
    }
}
"#;

    let path = unique_temp_path("color_arithmetic_bad_mix");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected invalid color arithmetic operand mix to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr
            .contains("color arithmetic `Add` only supports color/color or color/scalar operands"),
        "expected color arithmetic operand diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn explain_includes_centered_and_noise_receipts() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    space stage = centered(aspect: fit)
    let n = field noise2(center + (time * 0.03, 0.0))

    compose {
        in space stage {
            circle(at: center, radius: 0.2 + n * 0.01) |> fill(#ffffff)
        }
    }
}
"#;

    let path = unique_temp_path("explain_centered_noise_receipts");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explain receipt scenario to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain
            .contains("space: centered(aspect: fit) scales to fit entirely within viewport bounds"),
        "expected centered explain note\nexplain:\n{explain}"
    );
    assert!(
        explain.contains(
            "field: __intrinsic_noise2 lowered as deterministic smoothed value noise in v0"
        ) || explain.contains("field: noise2 lowered as deterministic smoothed value noise in v0"),
        "expected noise2 explain note\nexplain:\n{explain}"
    );
}

#[test]
fn ambient_selector_reports_component_diagnostic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let bad = center.z

    compose {
        layer grey(bad)
    }
}
"#;

    let path = unique_temp_path("ambient_selector_bad_component");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected ambient selector with invalid component to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unknown vec2 component `z` on `center`"),
        "expected ambient selector component diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn ambient_selector_chain_reports_scalar_terminal_diagnostic() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let bad = center.x.r

    compose {
        layer grey(bad)
    }
}
"#;

    let path = unique_temp_path("ambient_selector_chain_bad");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected chained ambient selector misuse to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("`center.x` is a scalar, so `center.x.r` is invalid"),
        "expected chained ambient selector terminal diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn conformance_file_has_no_confusable_grave_or_smart_quotes() {
    let me = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("showcase_conformance.rs");
    let text = fs::read_to_string(&me).expect("failed to read showcase_conformance.rs");
    let bad = [
        '\u{02CB}', // modifier letter grave accent (confusable with `)
        '\u{2018}', // left single quote
        '\u{2019}', // right single quote
        '\u{201C}', // left double quote
        '\u{201D}', // right double quote
    ];
    let found = text
        .char_indices()
        .find(|(_, ch)| bad.contains(ch))
        .map(|(idx, ch)| (idx, ch as u32));
    assert!(
        found.is_none(),
        "expected showcase_conformance.rs to avoid confusable quote characters; found {:?}",
        found
    );
}

#[test]
fn new_effects_soften_dilate_erode_inner_glow_bevel_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let core = circle(at: center, radius: 0.11)
    let ring = core |> dilate(radius: 0.03) |> erode(radius: 0.01)

    compose {
        ring |> soften(radius: 0.02, color: #9fd4ffff)
        ring |> inner_glow(reach: 0.06, strength: 0.8, color: #7dd3fcaa) |> blend(add)
        ring |> bevel(width: 3px, strength: 0.7) |> blend(add)
    }
}
"#;

    let path = unique_temp_path("new_effects_compile_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected new effects to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn blur_merge_explain_reports_exact_sqrt_radius() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let haze = circle(at: center, radius: 0.2)
        |> fill(#dbeafe)
        |> blur(radius: 0.01)
        |> blur(radius: 0.02)

    compose {
        haze
    }
}
"#;

    let path = unique_temp_path("blur_merge_exact_explain");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected blur merge explain scenario to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("rewrite: blur ∘ blur merged with exact radius sqrt(r1^2 + r2^2)"),
        "expected exact sqrt blur-merge explain note\nexplain:\n{explain}"
    );
}

#[test]
fn wide_effects_emit_non_fatal_warnings() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let a = circle(at: (0.45, 0.5), radius: 0.16)
    let b = box(at: (0.55, 0.5), size: (0.26, 0.18)) |> round(radius: 0.05)
    let joined = (a | b) |> smooth(radius: 0.04)

    compose {
        joined |> shadow(offset: (0.01, -0.01), soften: 0.03, color: #00000088)
        joined |> glow(reach: 0.06, strength: 0.7, color: #7dd3fc66) |> blend(add)
        joined |> fill(#93c5fd)
    }
}
"#;

    let path = unique_temp_path("wide_effect_warning_ok");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected wide-effect warning scenario to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("warning") && stderr.contains("may show approximation artifacts"),
        "expected non-fatal warning diagnostics for wide effects\nstderr:\n{stderr}"
    );
}

#[test]
fn math_builtins_compile_and_produce_valid_wgsl() {
    // Exercises all newly added math builtins in a single canvas.
    let input = r#"
canvas math_builtins(uv: coord, time: signal) -> color {
    let sample_value = sin(uv.x)
    let c = cos(uv.y)
    let at = atan(1.0)
    let at2 = atan2(y: uv.y, x: uv.x)
    let fl = floor(uv.x)
    let ce = ceil(uv.y)
    let ro = round(uv.x)
    let tr = trunc(uv.y)
    let p = pow(x: uv.x, e: 2.0)
    let e = exp(uv.x)
    let e2 = exp2(uv.y)
    let l = log(0.5)
    let l2 = log2(0.5)
    let mn = min(uv.x, uv.y)
    let mx = max(uv.x, uv.y)
    let cl = clamp(uv.x, lo: 0.0, hi: 1.0)
    let mi = mix(uv.x, uv.y, t: 0.5)
    let st = step(edge: 0.5, x: uv.x)
    let ss = smoothstep(lo: 0.0, hi: 1.0, x: uv.x)
    let sg = sign(uv.x - 0.5)
    let isq = inversesqrt(0.25)
    let lv = length((uv.x - 0.5, uv.y - 0.5))
    let dv = dot((uv.x, uv.y), (0.5, 0.5))
    let nv = normalize((uv.x - 0.5, uv.y - 0.5))
    let brightness = sample_value + c + at + at2 + fl + ce + ro + tr + p + e + e2 + l + l2 + mn + mx + cl + mi + st + ss + sg + isq + lv + dv + nv.x
    circle(at: center, radius: 0.3)
        |> fill(rgb(r: brightness, g: brightness, b: brightness))
}
"#;

    let path = unique_temp_path("math_builtins");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "math_builtins canvas failed to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    // Spot-check that key functions appear in the emitted WGSL.
    for fn_name in &[
        "sin(",
        "cos(",
        "atan2(",
        "floor(",
        "ceil(",
        "pow(",
        "exp(",
        "log(",
        "min(",
        "max(",
        "clamp(",
        "mix(",
        "step(",
        "smoothstep(",
        "inverseSqrt(",
    ] {
        assert!(
            wgsl.contains(fn_name),
            "expected `{fn_name}` in emitted WGSL\n{wgsl}"
        );
    }
}

#[test]
fn angle_constructor_supports_turn_units() {
    let input = r#"canvas t(uv: coord) -> color {
    let dir = angle(0.25turn)

    compose {
        circle(at: center + dir * 0.1, radius: 0.03) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("showcase_angle_turn_constructor");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected angle constructor with turn units to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn param_defaults_accept_px_literals() {
    let input = r#"canvas t(uv: coord) -> color {
    param stroke_w: f32 = 3px

    compose {
        circle(at: center, radius: stroke_w) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("showcase_param_px_default");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected px literal defaults in params to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn layer_mask_receiver_compiles() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let base = circle(at: center, radius: 0.15) |> fill(#ffffff)
    let fade = smoothstep(lo: 0.2, hi: 0.8, x: time)

    compose {
        base |> mask(fade)
    }
}
"#;

    let path = unique_temp_path("showcase_layer_mask_receiver");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected layer mask receiver usage to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn ramp_accepts_in_out_ease_alias() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let draw = ramp(from: 0s, over: 1s, ease: in_out)

    compose {
        circle(at: center, radius: 0.05 + draw * 0.05) |> fill(#ffffff)
    }
}
"#;

    let path = unique_temp_path("showcase_ramp_in_out_alias");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected ramp in_out alias to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn motion_blur_rejects_velocity_and_accepts_shutter_only() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let base = circle(at: center + (time * 0.05, 0.0), radius: 0.06) |> fill(#ffffff)

    compose {
        base |> motion_blur(shutter: 8ms, velocity: (0.18, 0.0))
    }
}
"#;

    let path = unique_temp_path("showcase_motion_blur_velocity_shutter");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected motion_blur shutter+velocity form to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("no longer accepts `velocity:`")
            && stderr.contains("motion_blur(shutter: 8ms)"),
        "expected migration diagnostic for velocity removal\nstderr:\n{stderr}"
    );

    let ok_input = r#"canvas t(uv: coord, time: signal) -> color {
    let base = circle(at: center + (time * 0.05, 0.0), radius: 0.06) |> fill(#ffffff)

    compose {
        base |> motion_blur(shutter: 8ms)
    }
}
"#;

    let ok_path = unique_temp_path("showcase_motion_blur_shutter_only");
    fs::write(&ok_path, ok_input).expect("failed to write temporary test source");
    let ok_output = run_fresco(&ok_path);
    let _ = fs::remove_file(&ok_path);

    assert!(
        ok_output.status.success(),
        "expected shutter-only motion_blur form to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&ok_output.stdout),
        normalize(&ok_output.stderr)
    );
}

#[test]
fn explain_reports_instruction_attribution_for_features() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
      space warped = rotate(12deg) . scale(1.0 + uv.y * 1.5)
      let ring = circle(at: (0.5, 0.5), radius: 0.18) |> stroke(2px)
        compose {
            in space warped {
                ring |> fill(#ffffff)
            }
        } |> motion_blur(shutter: 8ms)
    }
    "#;

    let path = unique_temp_path("explain_instruction_attribution");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explain attribution scenario to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("instructions: total emitted=")
            && explain.contains("motion_blur: +")
            && explain.contains("shape_aa: +")
            && explain.contains("instruction attribution by locality:")
            && explain.contains("local:"),
        "expected explain to include per-feature instruction attribution\nexplain:\n{explain}"
    );
}

#[test]
fn explain_feature_label_pragma_renames_attribution_rows() {
    let input = r#"#pragma explain.label.motion_blur = camera_smear

    canvas t(uv: coord, time: signal) -> color {
      let base = circle(at: center, radius: 0.2) |> fill(#ffffff)

      compose {
                base |> motion_blur(shutter: 8ms)
      }
    }
    "#;

    let path = unique_temp_path("explain_feature_label_pragma");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected explain label pragma scenario to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("camera_smear [motion_blur]: +"),
        "expected custom feature label in explain attribution\nexplain:\n{explain}"
    );
}

#[test]
fn explain_reports_scatter_and_pattern_filtering_attribution() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let guide = lines(along: x, every: 0.08uv)

    let stars = scatter 24 within screen seed 7 strategy compact {
        circle(at: instance.pos, radius: 1px)
            |> fill(#ffffff)
            |> opacity(0.5)
    }

    compose {
        guide |> stroke(1px) |> fill(#334455)
        stars
    }
}
"#;

    let path = unique_temp_path("explain_scatter_pattern_filtering_attribution");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter/pattern explain scenario to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("scatter: +") && explain.contains("pattern_filtering: +"),
        "expected explain to include scatter and pattern_filtering attribution\nexplain:\n{explain}"
    );
}

#[test]
fn integer_parameter_metadata_preserves_exact_defaults() {
    let input = r#"canvas t(uv: coord) -> color {
        param identity: u32 = 16777217
        param signed: i32 = -16777217
        param counts: array<u32, 2> = [16777217, 4294967295]
        compose { fill(#ffffff) }
    }"#;
    let output =
        driver::compile_source(input, common::TEST_SOURCE_PATH, "manifest", false).unwrap();
    let manifest: Value = serde_json::from_str(&output.emitted).unwrap();
    let params = manifest["canvases"][0]["params"].as_array().unwrap();
    let find = |name| params.iter().find(|param| param["name"] == name).unwrap();
    assert_eq!(find("identity")["default"], 16777217u32);
    assert_eq!(find("signed")["default"], -16777217i32);
    assert_eq!(
        find("counts")["default"]["values"],
        serde_json::json!([16777217u32, u32::MAX])
    );
}
