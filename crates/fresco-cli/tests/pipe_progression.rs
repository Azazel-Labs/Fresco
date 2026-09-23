use std::{fs, path::PathBuf, process::Command};

fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[path = "support/common.rs"]
mod common;
use common::unique_temp_path;

fn run_fresco(input_path: &PathBuf) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    Command::new(bin)
        .arg(input_path)
        .arg("--emit")
        .arg("wgsl")
        .output()
        .expect("failed to run fresco binary")
}

fn compile_to_wgsl(suffix: &str, input: &str) -> String {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    normalize(&output.stdout)
}

#[test]
fn pipe_first_arg_builtin_color_transforms_compile() {
    let wgsl = compile_to_wgsl(
        "pipe_builtin_color_transforms",
        r#"canvas t(uv: coord, time: signal) -> color {
  let accent = #5eead4
  let lo = accent |> darken(by: 0.35)
  let hi = accent |> lighten(by: 0.2)
  let flat = accent |> desaturate(by: 0.4)
  let loud = accent |> saturate(by: 0.15)

  compose {
    box(at: (0.2, 0.5), size: (0.15, 0.6)) |> fill(lo)
    box(at: (0.4, 0.5), size: (0.15, 0.6)) |> fill(hi)
    box(at: (0.6, 0.5), size: (0.15, 0.6)) |> fill(flat)
    box(at: (0.8, 0.5), size: (0.15, 0.6)) |> fill(loud)
  }
}
"#,
    );

    assert!(
        wgsl.contains("mix("),
        "expected color transform lowering to use mix\nwgsl:\n{wgsl}"
    );
}

#[test]
fn pipe_first_arg_builtin_math_and_vector_helpers_compile() {
    let wgsl = compile_to_wgsl(
        "pipe_builtin_math_and_vector_helpers",
        r#"canvas t(uv: coord, time: signal) -> color {
    let a = uv.x |> abs() |> clamp(lo: 0.0, hi: 1.0)
    let b = (uv - (0.5, 0.5)) |> length()
    let c = 0.25 |> mix(b, t: 0.5)
    let d = (0.0, 0.0) |> lerp((1.0, 1.0), t: 0.5)

    compose {
        box(at: (0.25, 0.5), size: (a, 0.2)) |> fill(#60a5fa)
        box(at: (0.75, 0.5), size: (c, 0.2)) |> fill(#f472b6)
        box(at: (0.5, 0.25), size: d) |> fill(#34d399)
        grey(b)
    }
}
"#,
    );

    assert!(
        wgsl.contains("clamp("),
        "expected pipeable math lowering to use clamp\nwgsl:\n{wgsl}"
    );
}

#[test]
fn pipe_first_arg_builtin_array_len_compile() {
    let wgsl = compile_to_wgsl(
        "pipe_builtin_array_len",
        r#"canvas t(uv: coord, time: signal) -> color {
    let values = [0.15, 0.4, 0.65, 0.9]
    let count = values |> len()
    let direct = len([1, 2, 3])

    compose {
        box(at: (0.5, 0.3 + count * 0.03), size: (0.2, 0.12)) |> fill(#60a5fa)
        box(at: (0.5, 0.7 + direct * 0.03), size: (0.2, 0.12)) |> fill(#f472b6)
    }
}
"#,
    );

    assert!(
        !wgsl.is_empty(),
        "expected len(array) example to compile\nwgsl:\n{wgsl}"
    );
}

#[test]
fn pipe_first_arg_builtin_distribute_compile() {
    let wgsl = compile_to_wgsl(
        "pipe_builtin_distribute",
        r#"canvas t(uv: coord, time: signal) -> color {
    let values = [0.3, 0.5, 0.42, 0.61]
    let slot = 2 |> distribute(count: len(values), start: 0.08, end: 0.92)
    let x = slot.center
    let w = slot.size

    compose {
        box(at: (x, 0.5), size: (w * 0.45, 0.2)) |> fill(#5eead4)
    }
}
"#,
    );

    assert!(
        !wgsl.is_empty(),
        "expected distribute example to compile\nwgsl:\n{wgsl}"
    );
}

#[test]
fn pipe_first_arg_builtin_distribute_slot_fields_compile() {
    let wgsl = compile_to_wgsl(
        "pipe_builtin_distribute_slot_fields",
        r#"canvas t(uv: coord, time: signal) -> color {
    let values = [0.3, 0.5, 0.42, 0.61]
    let slot = 2 |> distribute(count: len(values))
    let span = slot.right - slot.left
    let center = slot.center

    compose {
        box(at: (center, 0.5), size: (span * 0.45, 0.2)) |> fill(#5eead4)
    }
}
"#,
    );

    assert!(
        !wgsl.is_empty(),
        "expected distribute slot fields example to compile\nwgsl:\n{wgsl}"
    );
}

#[test]
fn pipe_first_arg_builtin_distribute_spacing_mode_compile() {
    let wgsl = compile_to_wgsl(
        "pipe_builtin_distribute_spacing_mode",
        r#"canvas t(uv: coord, time: signal) -> color {
    let values = [0.3, 0.5, 0.42, 0.61]
    let slot = 2 |> distribute(count: len(values), width: 0.12, gap: 0.02, anchor: 0.5)
    let x = slot.center
    let w = slot.size

    compose {
        box(at: (x, 0.5), size: (w * 0.8, 0.2)) |> fill(#5eead4)
    }
}
"#,
    );

    assert!(
        !wgsl.is_empty(),
        "expected distribute spacing mode to compile\nwgsl:\n{wgsl}"
    );
}

#[test]
fn pipe_first_arg_builtin_scalar_overload_compiles() {
    let wgsl = compile_to_wgsl(
        "pipe_builtin_scalar_saturate",
        r#"canvas t(uv: coord, time: signal) -> color {
  let x = (uv.x * 1.8 - 0.4) |> saturate()
  compose {
        grey(x)
  }
}
"#,
    );

    assert!(
        wgsl.contains("clamp("),
        "expected scalar saturate lowering to clamp\nwgsl:\n{wgsl}"
    );
}

#[test]
fn pipe_first_arg_user_function_compiles() {
    let wgsl = compile_to_wgsl(
        "pipe_user_function",
        r#"fn grade(c: color, amount: f32) -> color {
  darken(c, by: amount)
}

canvas t(uv: coord, time: signal) -> color {
  let accent = #5eead4
  let shaded = accent |> grade(amount: 0.25)
  compose {
    box(at: (0.5, 0.5), size: (0.4, 0.6)) |> fill(shaded)
  }
}
"#,
    );

    assert!(
        wgsl.contains("mix("),
        "expected piped user function to lower successfully\nwgsl:\n{wgsl}"
    );
}
