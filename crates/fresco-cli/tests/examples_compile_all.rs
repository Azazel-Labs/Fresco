#[path = "support/common.rs"]
mod common;

use std::io::Write;

use common::{
    ExampleCompilePolicy, collect_example_files_with_policies, example_rel_path, normalize,
    repo_root, run_example as run_fresco,
};
use libtest_mimic::{Failed, Trial};

/// Print progress immediately (flushed) so a slow/stuck example is visible
/// while it's running rather than only after it finishes or times out.
fn announce_compiling(name: &str) {
    eprintln!("[examples] compiling {name} ...");
    let _ = std::io::stderr().flush();
}

/// Turn a repo-relative example path into a libtest-mimic-safe test name so
/// each example is its own individually filterable/parallel test instead of
/// one monolithic loop where a single stuck file hides which example is
/// actually the problem.
fn test_name_for(rel: &str) -> String {
    rel.chars()
        .map(|c| match c {
            '/' | ' ' | '(' | ')' | '.' => '_',
            other => other,
        })
        .collect()
}

fn main() {
    let args = common::harness_args();
    let root = repo_root();
    let examples_dir = root.join("examples");
    let files_with_policy = collect_example_files_with_policies(&examples_dir);

    assert!(
        !files_with_policy.is_empty(),
        "expected at least one .fr example under {}",
        examples_dir.display()
    );

    let trials = files_with_policy
        .into_iter()
        .map(|(file, policy)| {
            let rel = example_rel_path(&root, &file);
            Trial::test(test_name_for(&rel), move || {
                announce_compiling(&test_name_for(&rel));
                let compiled = run_fresco(&file);
                match (policy, compiled.status.success()) {
                    (ExampleCompilePolicy::MustCompile, false) => Err(Failed::from(format!(
                        "{rel} failed to compile\nstdout:\n{}\nstderr:\n{}",
                        normalize(&compiled.stdout),
                        normalize(&compiled.stderr)
                    ))),
                    (ExampleCompilePolicy::AllowCompileFailures, true) => Err(Failed::from(format!(
                        "{rel} is marked allow-compile-failures but now compiles successfully — \
                         consider promoting it to must-compile policy"
                    ))),
                    _ => Ok(()),
                }
            })
        })
        .collect();

    libtest_mimic::run(&args, trials).exit();
}
