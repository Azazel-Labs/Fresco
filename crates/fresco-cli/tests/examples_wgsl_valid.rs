#[path = "support/common.rs"]
mod common;

use common::wgsl::{Capabilities, validate_wgsl};
use common::{normalize, repo_root, run_example as run_fresco};
use libtest_mimic::Failed;
use std::path::Path;

fn validate_example(file: &Path) -> Result<(), Failed> {
    let output = run_fresco(file);
    if !output.status.success() {
        return Err(Failed::from(format!(
            "compiler failed before WGSL validation\nstdout:\n{}\nstderr:\n{}",
            normalize(&output.stdout),
            normalize(&output.stderr)
        )));
    }
    let wgsl = String::from_utf8(output.stdout)
        .map_err(|error| Failed::from(format!("invalid utf-8 WGSL output: {error}")))?;
    validate_wgsl(&wgsl, Capabilities::default()).map_err(|error| {
        Failed::from(format!(
            "{error}\n\nWGSL (first 80 lines):\n{}",
            wgsl.lines().take(80).collect::<Vec<_>>().join("\n")
        ))
    })?;
    Ok(())
}

fn main() {
    let args = common::harness_args();
    let root = repo_root();
    let trials = common::must_compile_example_trials(&root, validate_example);
    assert!(
        !trials.is_empty(),
        "expected .fr examples under {}",
        root.display()
    );
    libtest_mimic::run(&args, trials).exit();
}
