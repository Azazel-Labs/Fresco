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

#[test]
fn scatter_parses_and_compiles_with_mvp_semantics() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let stars = scatter 12 within screen seed 9 strategy compact
                            lifetime star: 0.7s respawn every 2s {
    circle(at: center, radius: 0.1) |> fill(#ffffff)
    } |> opacity(0.5)
  compose {
    stars
  }
}
"#;

    let path = unique_temp_path("scatter_parse");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter MVP semantics to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn scatter_lifecycle_rand_lowers_to_instance_driven_wgsl() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let sparks = scatter 6 within screen seed 13 strategy compact
                                                    lifetime spark: rand(0.3s .. 0.9s) respawn every rand(2s .. 5s) {
    circle(at: spark.pos, radius: 1px) |> fill(#ffffff)
  }

  compose {
    sparks
  }
}
"#;

    let path = unique_temp_path("scatter_lifecycle_rand");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter lifecycle rand program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("inst_id"),
        "expected lifecycle rand lowering to reference instance id\nwgsl:\n{wgsl}"
    );
    assert!(
        wgsl.contains("43758.547") || wgsl.contains("43758.55"),
        "expected lifecycle rand lowering to emit hash-style noise math\nwgsl:\n{wgsl}"
    );
}

#[test]
fn scatter_requires_explicit_strategy_clause() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let stars = scatter 8 within screen seed 5 {
        circle(at: instance.pos, radius: 1px) |> fill(#ffffff)
    }

    compose {
        stars
    }
}
"#;

    let path = unique_temp_path("scatter_missing_strategy");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected scatter without strategy clause to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.to_ascii_lowercase().contains("strategy"),
        "expected missing-strategy diagnostic to mention strategy\nstderr:\n{stderr}"
    );
}

#[test]
fn scatter_reports_unknown_strategy_value() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let stars = scatter 8 within screen seed 5 strategy turbo {
        circle(at: instance.pos, radius: 1px) |> fill(#ffffff)
    }

    compose {
        stars
    }
}
"#;

    let path = unique_temp_path("scatter_unknown_strategy");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected unknown scatter strategy to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("unknown scatter strategy"),
        "expected unknown strategy diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn scatter_procedural_strategy_rejects_lifecycle_bindings() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let stars = scatter 8 within screen seed 5 strategy procedural
                                                    lifetime star: 0.7s respawn every 2s {
        circle(at: star.pos, radius: 1px) |> fill(#ffffff)
    }

    compose {
        stars
    }
}
"#;

    let path = unique_temp_path("scatter_procedural_lifecycle_conflict");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected procedural strategy with lifecycle to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("does not support lifecycle bindings"),
        "expected procedural+lifecycle compatibility diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn scatter_accepts_newlines_between_required_clauses() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let stars = scatter 180
                    within region((0.0, 0.0) .. (1.0, 1.0))
                    seed 19
                    strategy procedural {
        circle(at: instance.pos, radius: rand(0.12px .. 0.30px)) |> fill(#ffffff)
    }

    compose {
        stars
    }
}
"#;

    let path = unique_temp_path("scatter_multiline_required_clauses");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter with newline-separated required clauses to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn scatter_procedural_allows_counts_above_v0_nonprocedural_bound() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let stars = scatter 18000
                    within region((0.0, 0.0) .. (1.0, 1.0))
                    seed 19
                    strategy procedural {
        circle(at: instance.pos, radius: 0.2px) |> fill(#ffffff)
    }

    compose {
        stars
    }
}
"#;

    let path = unique_temp_path("scatter_procedural_high_count");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected high-count procedural scatter to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn scatter_instance_aliases_compile() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
    let sparks = scatter 16 within screen seed 17 strategy compact {
        let age = wrap(x: time + instance.rand, range: 0 .. 1)
        circle(at: instance.pos + instance.jitter * 0.02, radius: 0.01 + instance.rand2 * 0.01)
            |> fill(#ffffff)
            |> opacity(1.0 - age)
    }

    compose {
        sparks
    }
}
"#;

    let path = unique_temp_path("scatter_instance_aliases");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter instance aliases to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}
