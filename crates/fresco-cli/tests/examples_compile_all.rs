#[path = "support/common.rs"]
mod common;

use std::io::Write;

use common::{
    ExampleCompilePolicy, collect_example_files_with_policies, normalize, repo_root,
    run_example as run_fresco,
};
use libtest_mimic::{Arguments, Failed, Trial};

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
fn test_name_for(root: &std::path::Path, file: &std::path::Path) -> String {
    let rel = file
        .strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| file.display().to_string());

    rel.chars()
        .map(|c| match c {
            '/' | '\\' | ' ' | '(' | ')' | '.' => '_',
            other => other,
        })
        .collect()
}

fn main() {
    let mut args = Arguments::from_args();
    if args.test_threads.is_none() {
        args.test_threads = Some(common::test_threads());
    }
    let root = repo_root();
    let examples_dir = root.join("examples");

    let mut all_files = Vec::new();
    let mut files_with_policy = Vec::new();
    collect_example_files_with_policies(&examples_dir, &mut all_files, &mut files_with_policy);

    assert!(
        !all_files.is_empty(),
        "expected at least one .fr example under {}",
        examples_dir.display()
    );

    let mut trials = Vec::with_capacity(files_with_policy.len());
    for (file, policy) in files_with_policy {
        let name = test_name_for(&root, &file);
        let root_for_trial = root.clone();
        let trial = match policy {
            ExampleCompilePolicy::MustCompile => {
                let announce_name = name.clone();
                Trial::test(name, move || {
                    announce_compiling(&announce_name);
                    let output = run_fresco(&file);
                    if output.status.success() {
                        Ok(())
                    } else {
                        let rel = file
                            .strip_prefix(&root_for_trial)
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| file.display().to_string());
                        Err(Failed::from(format!(
                            "{rel} failed to compile\nstdout:\n{}\nstderr:\n{}",
                            normalize(&output.stdout),
                            normalize(&output.stderr)
                        )))
                    }
                })
            }
            ExampleCompilePolicy::AllowCompileFailures => {
                let announce_name = name.clone();
                Trial::test(name, move || {
                    announce_compiling(&announce_name);
                    let output = run_fresco(&file);
                    if output.status.success() {
                        let rel = file
                            .strip_prefix(&root_for_trial)
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| file.display().to_string());
                        Err(Failed::from(format!(
                            "{rel} is marked allow-compile-failures but now compiles successfully — \
                         consider promoting it to must-compile policy"
                        )))
                    } else {
                        Ok(())
                    }
                })
            }
        };
        trials.push(trial);
    }

    libtest_mimic::run(&args, trials).exit();
}
