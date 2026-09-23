use std::path::PathBuf;
use std::process::Command;
use std::{fs, time::Duration};

#[path = "support/common.rs"]
mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("failed to resolve workspace root")
}

fn compile_wgsl(input_path: &PathBuf) -> String {
    compile_wgsl_with_policy(input_path, None)
}

fn compile_wgsl_with_policy(input_path: &PathBuf, policy: Option<&str>) -> String {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    let mut cmd = Command::new(bin);
    cmd.arg(input_path).arg("--emit").arg("wgsl");
    cmd.arg("--engine-dir")
        .arg(repo_root().join("integrations/example-engine/engine"));
    if let Some(policy) = policy {
        cmd.env("FRESCO_LOWERING_POLICY", policy);
    }

    let output = cmd.output().expect("failed to run fresco binary");

    assert!(
        output.status.success(),
        "expected {} to compile\nstdout:\n{}\nstderr:\n{}",
        input_path.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("wgsl output was not valid utf-8")
}

fn wgsl_metrics(wgsl: &str) -> (usize, usize) {
    let bytes = wgsl.len();
    let let_count = wgsl
        .lines()
        .filter(|line| line.trim_start().starts_with("let "))
        .count();
    (bytes, let_count)
}

fn canvas_lowering_wgsl(wgsl: &str) -> &str {
    wgsl.split_once("struct FrescoFullscreenUniforms")
        .map_or(wgsl, |(lowered, _engine_stages)| lowered)
}

#[test]
fn floating_boxes_codegen_size_regression_guard() {
    let input = repo_root()
        .join("examples")
        .join("90) gallery")
        .join("floating_boxes.fr");

    let wgsl = compile_wgsl(&input);
    // Keep this guard focused on the scene/canvas lowering. Compiler-owned
    // fullscreen stages are fixed integration ABI and have separate coverage.
    let (bytes, let_count) = wgsl_metrics(canvas_lowering_wgsl(&wgsl));

    const MAX_BYTES: usize = 190_000;
    const MAX_LETS: usize = 2_800;

    assert!(
        bytes <= MAX_BYTES,
        "floating_boxes WGSL grew beyond threshold: bytes={} (max={})",
        bytes,
        MAX_BYTES
    );
    assert!(
        let_count <= MAX_LETS,
        "floating_boxes WGSL let-count grew beyond threshold: lets={} (max={})",
        let_count,
        MAX_LETS
    );
}

#[test]
fn wave_saw_square_codegen_size_regression_guard() {
    let input = repo_root()
        .join("examples")
        .join("10) fundamentals")
        .join("wave_saw_square.fr");

    let wgsl = compile_wgsl(&input);
    let (bytes, let_count) = wgsl_metrics(canvas_lowering_wgsl(&wgsl));

    // This budget protects the lowered scene and shared canvas helper. The
    // fixed compiler-owned stage ABI is covered by engine-pass tests.
    const MAX_BYTES: usize = 4_600;
    const MAX_LETS: usize = 60;

    assert!(
        bytes <= MAX_BYTES,
        "wave_saw_square WGSL grew beyond threshold: bytes={} (max={})",
        bytes,
        MAX_BYTES
    );
    assert!(
        let_count <= MAX_LETS,
        "wave_saw_square WGSL let-count grew beyond threshold: lets={} (max={})",
        let_count,
        MAX_LETS
    );
}

#[test]
fn compact_lowering_policy_reduces_floating_boxes_output_size() {
    let input = repo_root()
        .join("examples")
        .join("90) gallery")
        .join("floating_boxes.fr");

    let readable = compile_wgsl_with_policy(&input, Some("readable"));
    let compact = compile_wgsl_with_policy(&input, Some("compact"));

    let (readable_bytes, readable_lets) = wgsl_metrics(&readable);
    let (compact_bytes, compact_lets) = wgsl_metrics(&compact);

    assert!(
        compact_bytes < readable_bytes,
        "expected compact policy to reduce bytes: compact={} readable={}",
        compact_bytes,
        readable_bytes
    );
    assert!(
        compact_lets <= readable_lets,
        "expected compact policy to not increase let-count: compact={} readable={}",
        compact_lets,
        readable_lets
    );
}

#[test]
fn floating_boxes_scatter_compile_timing_regression_guard() {
    let input = repo_root()
        .join("examples")
        .join("90) gallery")
        .join("floating_boxes.fr");
    let src = fs::read_to_string(&input).expect("failed to read floating_boxes source");
    let filename = input.display().to_string();

    // Run on a dedicated thread with a larger stack, matching the CLI binary
    // (see `fresco-cli/src/main.rs`): the checker/lowering pipeline can nest
    // deeply enough for scatter-heavy scenes to overflow a default test
    // thread's stack even though the resulting WGSL stays well within the
    // codegen size budget checked above.
    let scatter_compile_ms = std::thread::Builder::new()
        .name("floating-boxes-scatter-timing".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let _warmup = common::compile_example(&src, &filename, false)
                .expect("warmup compile should succeed");
            std::thread::sleep(Duration::from_millis(10));
            let measured = common::compile_example(&src, &filename, false)
                .expect("timed compile should succeed");
            measured.timings.pipeline.check_rewrite_ms + measured.timings.pipeline.lower_validate_ms
        })
        .expect("failed to start compile thread")
        .join()
        .expect("compile thread panicked");

    let default_budget_ms = if cfg!(target_os = "windows") {
        45_000.0
    } else {
        25_000.0
    };
    let budget_ms = std::env::var("FRESCO_SCATTER_COMPILE_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(default_budget_ms);

    assert!(
        scatter_compile_ms <= budget_ms,
        "floating_boxes scatter compile exceeded budget: {:.2}ms (budget {:.2}ms)",
        scatter_compile_ms,
        budget_ms,
    );
}
