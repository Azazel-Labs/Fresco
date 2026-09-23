#![allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

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
pub fn run_example(input_path: impl AsRef<Path>) -> std::process::Output {
    run_example_with_args(input_path, &[])
}

pub fn run_example_with_args(
    input_path: impl AsRef<Path>,
    extra_args: &[&str],
) -> std::process::Output {
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

pub fn run_fresco(input_path: impl AsRef<Path>) -> std::process::Output {
    run_fresco_with_args(input_path, &[])
}

pub fn run_fresco_with_args(
    input_path: impl AsRef<Path>,
    extra_args: &[&str],
) -> std::process::Output {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");

    let mut cmd = Command::new(bin);
    cmd.arg(input_path.as_ref()).arg("--emit").arg("wgsl");
    cmd.args(extra_args);
    cmd.output().expect("failed to run fresco binary")
}

pub fn collect_example_files(dir: &Path, out: &mut Vec<PathBuf>) {
    collect_example_files_with_policies(dir, out, &mut Vec::new());
}

pub fn collect_example_files_with_policies(
    dir: &Path,
    files_out: &mut Vec<PathBuf>,
    files_with_policy_out: &mut Vec<(PathBuf, ExampleCompilePolicy)>,
) {
    collect_example_files_with_policy_recursive(
        dir,
        ExampleCompilePolicy::MustCompile,
        files_out,
        files_with_policy_out,
    );
}

fn collect_example_files_with_policy_recursive(
    dir: &Path,
    inherited_policy: ExampleCompilePolicy,
    files_out: &mut Vec<PathBuf>,
    files_with_policy_out: &mut Vec<(PathBuf, ExampleCompilePolicy)>,
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
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("fr") {
            fr_files.push(path);
        }
    }

    // If this directory has a `main.fr` alongside other `.fr` files it is a
    // multi-file example bundle.  Only add `main.fr` so library sibling files
    // are not compiled standalone (they have no canvas block).
    let has_main = fr_files
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("main.fr"));
    if has_main && fr_files.len() > 1 {
        for path in fr_files {
            if path.file_name().and_then(|n| n.to_str()) == Some("main.fr") {
                files_out.push(path.clone());
                files_with_policy_out.push((path, active_policy));
            }
        }
    } else {
        for path in fr_files {
            files_out.push(path.clone());
            files_with_policy_out.push((path, active_policy));
        }
    }

    for subdir in subdirs {
        collect_example_files_with_policy_recursive(
            &subdir,
            active_policy,
            files_out,
            files_with_policy_out,
        );
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
