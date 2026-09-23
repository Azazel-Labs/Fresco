#[path = "support/common.rs"]
mod common;

use common::{
    ExampleCompilePolicy, collect_example_files_with_policies, normalize, repo_root,
    run_example as run_fresco,
};
use libtest_mimic::{Arguments, Failed, Trial};
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
    let excerpt = || wgsl.lines().take(80).collect::<Vec<_>>().join("\n");
    let module = naga::front::wgsl::parse_str(&wgsl).map_err(|error| {
        Failed::from(format!(
            "WGSL parse failed:\n{error}\n\nWGSL (first 80 lines):\n{}",
            excerpt()
        ))
    })?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    validator.validate(&module).map_err(|error| {
        Failed::from(format!(
            "WGSL validation failed:\n{error:#?}\n\nWGSL (first 80 lines):\n{}",
            excerpt()
        ))
    })?;
    Ok(())
}

fn main() {
    let mut args = Arguments::from_args();
    if args.test_threads.is_none() {
        args.test_threads = Some(common::test_threads());
    }
    let root = repo_root();
    let mut all_files = Vec::new();
    let mut files_with_policy = Vec::new();
    collect_example_files_with_policies(
        &root.join("examples"),
        &mut all_files,
        &mut files_with_policy,
    );
    assert!(
        !all_files.is_empty(),
        "expected .fr examples under {}",
        root.display()
    );
    files_with_policy.sort_by(|left, right| left.0.cmp(&right.0));
    let trials = files_with_policy
        .into_iter()
        .filter(|(_, policy)| *policy == ExampleCompilePolicy::MustCompile)
        .map(|(file, _)| {
            let name = file
                .strip_prefix(&root)
                .expect("example inside workspace")
                .to_string_lossy()
                .replace('\\', "/");
            Trial::test(name, move || validate_example(&file))
        })
        .collect();
    libtest_mimic::run(&args, trials).exit();
}
