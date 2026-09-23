use std::{fs, path::PathBuf, process::Command};

fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[path = "support/common.rs"]
mod common;
use common::unique_temp_path;

fn run_fresco(input_path: &PathBuf, extra: &[&str]) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    Command::new(bin)
        .arg(input_path)
        .args(extra)
        .output()
        .expect("failed to run fresco binary")
}

fn parse_scatter_lowering_value(explain: &str, key: &str) -> usize {
    let line = explain
        .lines()
        .find(|line| line.contains("scatter lowering:"))
        .expect("missing scatter lowering line in explain output");
    let needle = format!("{key}=");
    let value_text = line
        .split(&needle)
        .nth(1)
        .expect("missing metric key in scatter lowering line")
        .split(',')
        .next()
        .expect("missing metric value after key")
        .trim();
    value_text
        .parse::<usize>()
        .expect("scatter lowering metric must parse as usize")
}

#[test]
fn scatter_binning_note_and_caps_are_reported() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars = scatter 1024 within screen seed 13 strategy compact {
    circle(at: instance.pos, radius: 1px) |> fill(#ffffff)
  }

  compose {
    stars
  }
}
"#;

    let path = unique_temp_path("scatter_binning_explain");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path, &["--emit", "wgsl", "--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter-binned program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("scatter: binned 1024 instance(s)"),
        "expected explain output to include scatter binning note\nexplain:\n{explain}"
    );
    assert!(
        (explain.contains("adaptive cap") && explain.contains("total cap 256"))
            || explain.contains("no candidate truncation"),
        "expected explain output to include cap diagnostics or explicit no-truncation note\nexplain:\n{explain}"
    );
}

#[test]
fn scatter_output_is_deterministic_for_same_seed() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars = scatter 320 within screen seed 7 strategy compact {
    circle(at: instance.pos, radius: rand(0.8px .. 1.2px)) |> fill(#ffffff)
  }

  compose {
    stars
  }
}
"#;

    let path = unique_temp_path("scatter_determinism");
    fs::write(&path, input).expect("failed to write temporary test source");

    let run1 = run_fresco(&path, &["--emit", "wgsl"]);
    let run2 = run_fresco(&path, &["--emit", "wgsl"]);

    let _ = fs::remove_file(&path);

    assert!(
        run1.status.success(),
        "first run failed\nstderr:\n{}",
        normalize(&run1.stderr)
    );
    assert!(
        run2.status.success(),
        "second run failed\nstderr:\n{}",
        normalize(&run2.stderr)
    );

    let out1 = normalize(&run1.stdout);
    let out2 = normalize(&run2.stdout);
    assert_eq!(
        out1, out2,
        "WGSL output should be deterministic for same seed"
    );
}

#[test]
fn scatter_region_reports_invalid_bounds() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars = scatter 16 within region((0.8, 0.2) .. (0.1, 0.9)) seed 1 strategy compact {
    circle(at: instance.pos, radius: 1px) |> fill(#ffffff)
  }

  compose {
    stars
  }
}
"#;

    let path = unique_temp_path("scatter_bad_region");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path, &["--emit", "wgsl"]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected invalid region bounds to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("scatter region must have positive area"),
        "expected invalid bounds diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn scatter_body_is_extracted_as_helper_function() {
    // A scatter with a body-heavy shape (circle + glow) should produce a
    // named helper function rather than inlining the body once per instance.
    // This verifies the O(body + instances × call) codegen path.
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let dots = scatter 64 within screen seed 42 strategy compact {
    circle(at: instance.pos, radius: rand(0.4px .. 1.2px))
    |> fill(#ffcc88)
    |> glow(reach: rand(2px .. 5px), strength: 0.8)
  }

  compose {
    dots
  }
}
"#;

    let path = unique_temp_path("scatter_extraction");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path, &["--emit", "wgsl"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter with body-heavy shape to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);

    // Helper function must appear exactly once (body extracted, not inlined per instance)
    let helper_count = wgsl
        .lines()
        .filter(|l| l.starts_with("fn fresco_t_scatter_l"))
        .count();
    assert_eq!(
        helper_count, 1,
        "expected exactly one scatter helper function definition\nwgsl:\n{wgsl}"
    );

    // Call sites must replace inline body — verify the helper is called
    assert!(
        wgsl.contains("fresco_t_scatter_l"),
        "expected scatter helper to be called from canvas function\nwgsl:\n{wgsl}"
    );

    // Total line count should be far below full-body inlining (which would be
    // much larger than helper-call codegen even with dynamic position mapping).
    let line_count = wgsl.lines().count();
    assert!(
        line_count < 800,
        "expected extracted scatter WGSL to be compact (< 800 lines), got {line_count}\nwgsl:\n{wgsl}"
    );
}

#[test]
fn scatter_footprint_support_combines_shape_and_effect_extent() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars = scatter 64 within screen seed 9 strategy compact {
    circle(at: instance.pos, radius: 2px)
    |> glow(reach: 3px, strength: 1.0)
  }

  compose {
    stars
  }
}
"#;

    let path = unique_temp_path("scatter_footprint_support");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path, &["--emit", "wgsl", "--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("footprint≈5.000"),
        "expected footprint support note to include shape radius + glow reach\nexplain:\n{explain}"
    );
}

#[test]
fn scatter_rand_uses_instance_id_in_generated_wgsl() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let dots = scatter 8 within screen seed 17 strategy compact {
    circle(at: instance.pos, radius: rand(0.8px .. 1.2px)) |> fill(#ffffff)
  }

  compose {
    dots
  }
}
"#;

    let path = unique_temp_path("scatter_rand_inst_id");
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco(&path, &["--emit", "wgsl"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected scatter rand program to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    let inst_id_mentions = wgsl.matches("inst_id").count();
    assert!(
        inst_id_mentions >= 2,
        "expected generated WGSL to use inst_id inside scatter body rand lowering\nwgsl:\n{wgsl}"
    );
}

#[test]
fn scatter_procedural_emitted_instances_scale_with_count_and_exceed_base_cap() {
    let screen_input = |count: usize| {
        format!(
            r#"canvas t(uv: coord, time: signal) -> color {{
  let stars = scatter {count} within screen seed 19 strategy procedural {{
    circle(at: instance.pos, radius: 0.8px) |> fill(#ffffff)
  }}

  compose {{
    stars
  }}
}}
"#
        )
    };

    let bounded_footprint_input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars = scatter 60000 within region((0.45, 0.45) .. (0.55, 0.55)) seed 19 strategy procedural {
    circle(at: instance.pos, radius: 24px) |> fill(#ffffff)
  }

  compose {
    stars
  }
}
"#;

    let low_path = unique_temp_path("scatter_procedural_low");
    fs::write(&low_path, screen_input(18000)).expect("failed to write low-count source");
    let low_output = run_fresco(&low_path, &["--emit", "wgsl", "--explain"]);
    let _ = fs::remove_file(&low_path);

    assert!(
        low_output.status.success(),
        "expected low-count procedural compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&low_output.stdout),
        normalize(&low_output.stderr)
    );

    let high_path = unique_temp_path("scatter_procedural_high");
    fs::write(&high_path, screen_input(60000)).expect("failed to write high-count source");
    let high_output = run_fresco(&high_path, &["--emit", "wgsl", "--explain"]);
    let _ = fs::remove_file(&high_path);

    assert!(
        high_output.status.success(),
        "expected high-count procedural compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&high_output.stdout),
        normalize(&high_output.stderr)
    );

    let low_explain = normalize(&low_output.stderr);
    let high_explain = normalize(&high_output.stderr);
    let low_emitted = parse_scatter_lowering_value(&low_explain, "emitted_instances");
    let high_emitted = parse_scatter_lowering_value(&high_explain, "emitted_instances");
    let low_samples = parse_scatter_lowering_value(&low_explain, "procedural_samples");
    let high_samples = parse_scatter_lowering_value(&high_explain, "procedural_samples");
    let low_cap = parse_scatter_lowering_value(&low_explain, "procedural_cap");
    let high_cap = parse_scatter_lowering_value(&high_explain, "procedural_cap");
    assert!(
        high_emitted > low_emitted,
        "expected procedural emitted_instances to grow with higher scatter count\nlow explain:\n{low_explain}\nhigh explain:\n{high_explain}"
    );
    assert!(
        high_samples > low_samples,
        "expected procedural samples to grow with higher scatter count\nlow explain:\n{low_explain}\nhigh explain:\n{high_explain}"
    );
    assert!(
        low_cap >= 128 && high_cap >= 128,
        "expected procedural cap metrics to be present and at least base cap\nlow explain:\n{low_explain}\nhigh explain:\n{high_explain}"
    );

    let bounded_path = unique_temp_path("scatter_procedural_bounded_footprint");
    fs::write(&bounded_path, bounded_footprint_input)
        .expect("failed to write bounded-footprint source");
    let bounded_output = run_fresco(&bounded_path, &["--emit", "wgsl", "--explain"]);
    let _ = fs::remove_file(&bounded_path);

    assert!(
        bounded_output.status.success(),
        "expected bounded-footprint procedural compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&bounded_output.stdout),
        normalize(&bounded_output.stderr)
    );

    let bounded_explain = normalize(&bounded_output.stderr);
    let bounded_occupied = parse_scatter_lowering_value(&bounded_explain, "occupied_bins");
    let bounded_emitted = parse_scatter_lowering_value(&bounded_explain, "emitted_instances");
    assert!(
        bounded_occupied > 0,
        "expected bounded-footprint procedural scatter to occupy bins\nexplain:\n{bounded_explain}"
    );
    assert!(
        bounded_emitted > bounded_occupied * 128,
        "expected adaptive procedural cap to exceed 128 samples/bin for bounded-footprint scene\nexplain:\n{bounded_explain}"
    );
}

#[test]
fn scatter_procedural_hard_max_clamps_near_extreme_counts() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars = scatter 1200000 within screen seed 21 strategy procedural {
    circle(at: instance.pos, radius: 0.8px) |> fill(#ffffff)
  }

  compose {
    stars
  }
}
"#;

    let path = unique_temp_path("scatter_procedural_hard_max");
    fs::write(&path, input).expect("failed to write hard-max source");
    let output = run_fresco(&path, &["--emit", "wgsl", "--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected hard-max procedural compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    let occupied = parse_scatter_lowering_value(&explain, "occupied_bins");
    let emitted = parse_scatter_lowering_value(&explain, "emitted_instances");
    let samples = parse_scatter_lowering_value(&explain, "procedural_samples");
    let cap = parse_scatter_lowering_value(&explain, "procedural_cap");

    assert!(
        occupied > 0,
        "expected hard-max procedural case to occupy bins\nexplain:\n{explain}"
    );
    assert_eq!(
        cap, 256,
        "expected procedural cap to clamp at hard max in extreme-count case\nexplain:\n{explain}"
    );
    assert_eq!(
        samples, 256,
        "expected procedural samples to clamp at hard max in extreme-count case\nexplain:\n{explain}"
    );
    assert_eq!(
        emitted,
        occupied * 256,
        "expected emitted_instances to reflect hard-max clamping\nexplain:\n{explain}"
    );
}

#[test]
fn scatter_explain_reports_per_layer_metrics_for_multiple_scatter_layers() {
    let input = r#"canvas t(uv: coord, time: signal) -> color {
  let stars_a = scatter 18000 within screen seed 19 strategy procedural {
    circle(at: instance.pos, radius: 0.8px) |> fill(#ffffff)
  }

  let stars_b = scatter 60000 within screen seed 21 strategy procedural {
    circle(at: instance.pos, radius: 0.8px) |> fill(#88ccff)
  }

  compose {
    stars_a
    stars_b |> blend(add)
  }
}
"#;

    let path = unique_temp_path("scatter_per_layer_metrics");
    fs::write(&path, input).expect("failed to write per-layer metrics source");
    let output = run_fresco(&path, &["--emit", "wgsl", "--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected multi-scatter procedural compile success\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains("scatter per-layer metrics:"),
        "expected explain output to include per-layer scatter metrics section\nexplain:\n{explain}"
    );
    let layer_lines = explain
        .lines()
        .filter(|line| line.trim_start().starts_with('l') && line.contains(": strategy="))
        .collect::<Vec<_>>();
    assert!(
        layer_lines.len() >= 2,
        "expected at least two per-layer metric lines for multi-scatter canvas\nexplain:\n{explain}"
    );
    assert!(
        layer_lines.iter().all(|line| {
            line.contains("strategy=procedural")
                && line.contains("procedural_samples=")
                && line.contains("procedural_cap=")
        }),
        "expected per-layer lines to include procedural strategy and metric fields\nexplain:\n{explain}"
    );
}
