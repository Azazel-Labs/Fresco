#![allow(
    dead_code,
    reason = "Shared integration-test support: each test crate includes this module but uses only a subset of its helpers."
)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "wgsl.rs"]
pub mod wgsl;

const EXAMPLE_POLICY_FILE: &str = "example-test-policy.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExampleCompilePolicy {
    MustCompile,
    AllowCompileFailures,
}

pub fn test_threads() -> usize {
    match std::env::var("RUST_TEST_THREADS") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|count| *count > 0)
            .expect("RUST_TEST_THREADS must be a positive integer"),
        Err(std::env::VarError::NotPresent) => {
            std::thread::available_parallelism().map_or(1, |cpus| cpus.get().div_ceil(2))
        }
        Err(error) => panic!("cannot read RUST_TEST_THREADS: {error}"),
    }
}

/// libtest-mimic arguments with the shared default test thread count applied.
pub fn harness_args() -> libtest_mimic::Arguments {
    let mut args = libtest_mimic::Arguments::from_args();
    if args.test_threads.is_none() {
        args.test_threads = Some(test_threads());
    }
    args
}

/// Repo-relative, `/`-separated path of an example file, used for trial names
/// and failure messages.
pub fn example_rel_path(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// One libtest-mimic trial per `must-compile` example under `root/examples`,
/// named by its repo-relative path.
pub fn must_compile_example_trials(
    root: &Path,
    check: fn(&Path) -> Result<(), libtest_mimic::Failed>,
) -> Vec<libtest_mimic::Trial> {
    collect_example_files_with_policies(&root.join("examples"))
        .into_iter()
        .filter(|(_, policy)| *policy == ExampleCompilePolicy::MustCompile)
        .map(|(file, _)| {
            libtest_mimic::Trial::test(example_rel_path(root, &file), move || check(&file))
        })
        .collect()
}

pub const TEST_SOURCE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/render-policy/test.fr"
);

pub fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("failed to resolve workspace root")
}

fn test_root() -> &'static Path {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "fresco_policy_tests_{}_{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("engine")).expect("create test engine directory");
        fs::write(
            root.join("engine/engine.fr"),
            include_str!("../../../../tests/render-policy/engine/engine.fr"),
        )
        .expect("write explicit test engine rendering policy");
        root
    })
}

pub fn unique_temp_path(suffix: &str) -> PathBuf {
    test_root().join(format!("fresco_{suffix}_{}.fr", next_temp_id()))
}

pub fn unique_temp_dir(suffix: &str) -> PathBuf {
    let dir = test_root().join(format!("fresco_{suffix}_{}", next_temp_id()));
    fs::create_dir_all(&dir).expect("failed to create temporary directory");
    dir
}

fn next_temp_id() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1)
    })
    .expect("test temporary path counter exhausted")
}

/// Repository examples exercise the selected sample integration explicitly.
pub fn run_example(input_path: impl AsRef<Path>) -> Output {
    run_example_with_args(input_path, &[])
}

pub fn run_example_with_args(input_path: impl AsRef<Path>, extra_args: &[&str]) -> Output {
    let engine = repo_root().join("integrations/example-engine/engine");
    let engine = engine.to_string_lossy();
    let mut args = vec!["--engine-dir", &engine];
    args.extend_from_slice(extra_args);
    run_fresco_with_args(input_path, &args)
}

pub fn compile_example(
    source: &str,
    filename: &str,
    explain: bool,
) -> Result<fresco::driver::CompileBundleOutput, Vec<fresco::driver::DiagnosticRecord>> {
    fresco::driver::compile_source_bundle_with_engine_dir(
        source,
        filename,
        explain,
        &fresco::driver::CompileContext::default(),
        Some(&repo_root().join("integrations/example-engine/engine")),
    )
}

pub fn run_fresco(input_path: impl AsRef<Path>) -> Output {
    run_fresco_with_args(input_path, &[])
}

/// A `fresco` CLI invocation with no arguments yet, for tests that need to set
/// environment variables or otherwise customize the process.
pub fn fresco_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fresco"))
}

/// Runs the CLI on `input_path`. The CLI emits WGSL unless `extra_args`
/// selects another `--emit` target.
pub fn run_fresco_with_args(input_path: impl AsRef<Path>, extra_args: &[&str]) -> Output {
    fresco_command()
        .arg(input_path.as_ref())
        .args(extra_args)
        .output()
        .expect("failed to run fresco binary")
}

/// Writes `source` to a unique temporary `.fr` file, runs the CLI on it with
/// `extra_args`, and removes the file again.
pub fn run_source(suffix: &str, source: &str, extra_args: &[&str]) -> Output {
    let path = unique_temp_path(suffix);
    fs::write(&path, source).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, extra_args);
    let _ = fs::remove_file(&path);
    output
}

/// Asserts that the CLI succeeded and returns its normalized stdout.
pub fn assert_success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
    normalize(&output.stdout)
}

/// Asserts that the CLI failed with `needle` in stderr and returns the
/// normalized stderr.
pub fn assert_fails_with(output: &Output, needle: &str) -> String {
    assert!(
        !output.status.success(),
        "expected compile to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(needle),
        "expected stderr to contain `{needle}`\nstderr:\n{stderr}"
    );
    stderr
}

/// Compiles `source` to WGSL, panicking with the compiler output on failure.
pub fn compile_to_wgsl(suffix: &str, source: &str) -> String {
    assert_success(&run_source(suffix, source, &[]))
}

/// Compiles `source` to WGSL and asserts naga validates it under the default
/// capabilities.
pub fn assert_compiles_valid_wgsl(suffix: &str, source: &str) {
    wgsl::assert_valid_wgsl(
        &compile_to_wgsl(suffix, source),
        wgsl::Capabilities::default(),
    );
}

pub fn assert_compiles(suffix: &str, source: &str) {
    assert_success(&run_source(suffix, source, &[]));
}

pub fn assert_compile_fails_with(suffix: &str, source: &str, needle: &str) {
    assert_fails_with(&run_source(suffix, source, &[]), needle);
}

/// Compiles `source` with `--explain` and asserts the receipt (stderr)
/// contains `needle`.
pub fn assert_explain_contains(suffix: &str, source: &str, needle: &str) {
    let output = run_source(suffix, source, &["--explain"]);
    assert_success(&output);
    let explain = normalize(&output.stderr);
    assert!(
        explain.contains(needle),
        "expected explain output to contain `{needle}`\nexplain:\n{explain}"
    );
}

/// Reads `key=<usize>` from the `scatter lowering:` line of an explain receipt.
pub fn parse_scatter_lowering_value(explain: &str, key: &str) -> usize {
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

/// Collects every standalone example entrypoint under `dir`, sorted by path,
/// with the compile policy inherited from the nearest policy file.
pub fn collect_example_files_with_policies(dir: &Path) -> Vec<(PathBuf, ExampleCompilePolicy)> {
    let mut files = Vec::new();
    collect_example_files_with_policy_recursive(dir, ExampleCompilePolicy::MustCompile, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn collect_example_files_with_policy_recursive(
    dir: &Path,
    inherited_policy: ExampleCompilePolicy,
    files_out: &mut Vec<(PathBuf, ExampleCompilePolicy)>,
) {
    let active_policy = if dir.join(EXAMPLE_POLICY_FILE).exists() {
        load_example_policy(dir)
    } else {
        inherited_policy
    };

    let entries = fs::read_dir(dir).expect("failed to read examples directory");
    let mut fr_files: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();

    for entry in entries {
        let entry = entry.expect("failed to read examples entry");
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.extension().is_some_and(|ext| ext == "fr") {
            fr_files.push(path);
        }
    }

    // If this directory has a `main.fr` alongside other `.fr` files it is a
    // multi-file example bundle.  Only add `main.fr` so library sibling files
    // are not compiled standalone (they have no canvas block).
    let is_main = |path: &PathBuf| path.file_name().is_some_and(|name| name == "main.fr");
    let has_main = fr_files.iter().any(is_main);
    if has_main && fr_files.len() > 1 {
        fr_files.retain(is_main);
    }
    files_out.extend(fr_files.into_iter().map(|path| (path, active_policy)));

    for subdir in subdirs {
        collect_example_files_with_policy_recursive(&subdir, active_policy, files_out);
    }
}

fn load_example_policy(dir: &Path) -> ExampleCompilePolicy {
    let policy_path = dir.join(EXAMPLE_POLICY_FILE);
    let raw = fs::read_to_string(&policy_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", policy_path.display()));

    let value = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or_else(|| {
            panic!(
                "{} is empty; expected one of: must-compile, allow-compile-failures",
                policy_path.display()
            )
        });

    let normalized = value
        .strip_prefix("compile_policy=")
        .or_else(|| value.strip_prefix("compile_policy ="))
        .unwrap_or(value)
        .trim();

    match normalized {
        "must-compile" => ExampleCompilePolicy::MustCompile,
        "allow-compile-failures" => ExampleCompilePolicy::AllowCompileFailures,
        other => panic!(
            "{} has invalid policy `{other}`; expected `must-compile` or `allow-compile-failures`",
            policy_path.display()
        ),
    }
}
