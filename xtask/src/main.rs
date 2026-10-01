use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::time::{Duration, Instant};
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

mod help;
mod readme;
mod repo_guard;
use help::print_help;

const RUSTFMT_EDITION: &str = "2024";
const RUSTFMT_CONFIG_EDITION: &str = "edition=2024";
const HOOK_EXPECTED_RUSTFMT: &str = "rustfmt --edition 2024";
const LANGUAGE_DOCS_ARTIFACT_PATH: &str = "docs/generated/language-reference.v1.json";
const LANGUAGE_DOCS_MARKDOWN_ARTIFACT_PATH: &str = "docs/generated/language-reference.v1.md";
const FMT_CHECK: &[&str] = &[
    "fmt",
    "--all",
    "--",
    "--check",
    "--config",
    RUSTFMT_CONFIG_EDITION,
];

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        print_help();
        return ExitCode::from(0);
    };

    let extra: Vec<String> = args.collect();
    let root = workspace_root();

    let result = match cmd.as_str() {
        "check" => run_cargo(&root, &["check", "--workspace", "--all-targets"]),
        "check-fast" => run_cargo(&root, &["check", "--profile", "dev-fast"]),
        "check-timing" => run_check_timing(&root, &extra),
        "timed-run" => run_timed_run(&root, &extra),
        "test" => run_workspace_tests(&root, false),
        "repo-guard" => repo_guard::run(&root),
        "readme-sync" => match extra.as_slice() {
            [] => readme::run(&root, false),
            [flag] if flag == "--check" => readme::run(&root, true),
            _ => {
                eprintln!("readme-sync only accepts optional --check");
                Err(1)
            }
        },
        "fmt" => run_cargo(
            &root,
            &["fmt", "--all", "--", "--config", RUSTFMT_CONFIG_EDITION],
        ),
        "install-hooks" => run_install_hooks(&root),
        "fmt-guard" => run_fmt_guard(&root),
        "clippy" => run_cargo(&root, &["clippy", "--workspace", "--all-targets"]),
        "ci" => run_ci(&root),
        "ci-strict" => run_ci_strict(&root),
        "lang-docs" => run_lang_docs(&root, &extra),
        "gate-status" => run_gate_status(&root),
        "dist" => run_dist(&root),
        "clean-all" => run_clean_all(&root),
        "run" => run_cargo_with_extra(&root, &["run"], &extra),
        "run-fast" => run_cargo_with_extra(&root, &["run", "--profile", "dev-fast"], &extra),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("unknown xtask command: {other}");
            print_help();
            Err(1)
        }
    };

    match result {
        Ok(()) => ExitCode::from(0),
        Err(code) => ExitCode::from(code as u8),
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask should live directly under the workspace root")
        .to_path_buf()
}

fn default_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |cpus| cpus.get().div_ceil(2))
}

/// `CARGO_BUILD_JOBS`, defaulting to [`default_workers`].
fn build_jobs() -> String {
    env::var("CARGO_BUILD_JOBS").unwrap_or_else(|_| default_workers().to_string())
}

fn run_workspace_tests(root: &Path, locked: bool) -> Result<(), i32> {
    let build_jobs = build_jobs();
    let test_threads =
        env::var("RUST_TEST_THREADS").unwrap_or_else(|_| default_workers().to_string());
    let mut args = vec!["test", "--workspace"];
    if locked {
        args.push("--locked");
    }
    // Pass the harness flag too: libtest-mimic does not read RUST_TEST_THREADS.
    args.extend(["--", "--test-threads", test_threads.as_str()]);
    run_cargo_with_env(
        root,
        &args,
        &[
            ("CARGO_BUILD_JOBS", build_jobs.as_str()),
            ("RUST_TEST_THREADS", test_threads.as_str()),
        ],
    )
}

/// Source-hygiene checks shared by `ci` and `ci-strict`.
fn run_ci_prelude(root: &Path) -> Result<(), i32> {
    repo_guard::run(root)?;
    readme::run(root, false)?;
    run_fmt_guard(root)
}

fn run_ci(root: &Path) -> Result<(), i32> {
    let build_jobs = build_jobs();
    let ci_build_jobs = [("CARGO_BUILD_JOBS", build_jobs.as_str())];

    run_ci_prelude(root)?;
    warn_only(
        run_cargo(root, FMT_CHECK),
        "cargo fmt --check reported formatting drift; continuing without failing `cargo xtask ci`",
    );
    run_cargo_with_env(
        root,
        &["check", "--workspace", "--all-targets", "--locked"],
        &ci_build_jobs,
    )?;
    run_workspace_tests(root, true)?;
    warn_only(
        run_cargo_with_env(
            root,
            &["clippy", "--workspace", "--all-targets", "--locked"],
            &ci_build_jobs,
        ),
        "cargo clippy reported lints; continuing without failing `cargo xtask ci`",
    );
    Ok(())
}

fn run_timed_run(root: &Path, extra: &[String]) -> Result<(), i32> {
    if extra.is_empty() {
        eprintln!(
            "timed-run requires an input .fr path\n\
help: cargo xtask timed-run \"examples/90) gallery/floating_boxes.fr\" [--trace <off|fmt|json|chrome|tracy>] [--log <filter>] [--timings] [--explain] [--output <path>] [-- <extra fresco-cli args>]"
        );
        return Err(1);
    }

    let input = extra[0].clone();
    let mut trace_mode = "fmt".to_string();
    let mut log_filter = Some("fresco::driver::pipeline=info".to_string());
    let mut cli_args: Vec<String> = vec![input];

    let mut i = 1usize;
    let mut passthrough = false;
    while i < extra.len() {
        let token = &extra[i];
        if passthrough {
            cli_args.push(token.clone());
            i += 1;
            continue;
        }

        match token.as_str() {
            "--" => {
                passthrough = true;
                i += 1;
            }
            "--trace" => {
                let Some(mode) = extra.get(i + 1) else {
                    eprintln!("--trace expects a value");
                    return Err(1);
                };
                trace_mode = mode.to_ascii_lowercase();
                i += 2;
            }
            "--log" => {
                let Some(filter) = extra.get(i + 1) else {
                    eprintln!("--log expects a value");
                    return Err(1);
                };
                log_filter = Some(filter.clone());
                i += 2;
            }
            "--no-log" => {
                log_filter = None;
                i += 1;
            }
            other => {
                cli_args.push(other.to_string());
                i += 1;
            }
        }
    }

    let mut cargo_args = vec![
        "run".to_string(),
        "-q".to_string(),
        "-p".to_string(),
        "fresco-cli".to_string(),
    ];

    match trace_mode.as_str() {
        "chrome" => {
            cargo_args.push("--features".to_string());
            cargo_args.push("fresco/trace-chrome".to_string());
        }
        "tracy" => {
            cargo_args.push("--features".to_string());
            cargo_args.push("fresco/trace-tracy".to_string());
        }
        _ => {}
    }

    cargo_args.push("--".to_string());
    cargo_args.extend(cli_args.iter().cloned());

    let mut envs = vec![("FRESCO_TRACE", trace_mode.as_str())];
    if let Some(filter) = &log_filter {
        envs.push(("RUST_LOG", filter.as_str()));
    }

    run_cargo_with_env(root, &cargo_args, &envs)
}

fn run_check_timing(root: &Path, extra: &[String]) -> Result<(), i32> {
    let mut runs = 3usize;
    let mut profile = "dev";

    if let Some(first) = extra.first() {
        if let Ok(parsed) = first.parse::<usize>() {
            runs = parsed.max(1);
            if let Some(second) = extra.get(1) {
                profile = second;
            }
        } else {
            profile = first;
        }
    }

    let mut args = vec!["check".to_string()];
    if profile != "dev" {
        args.push("--profile".to_string());
        args.push(profile.to_string());
    }

    eprintln!("Timing cargo check");
    eprintln!("- runs: {runs}");
    eprintln!("- profile: {profile}");

    let mut durations: Vec<Duration> = Vec::with_capacity(runs);
    for i in 1..=runs {
        let start = Instant::now();
        run_cargo(root, &args)?;
        let elapsed = start.elapsed();
        eprintln!("  run {i}/{runs}: {:.2} ms", elapsed.as_secs_f64() * 1000.0);
        durations.push(elapsed);
    }

    let total_secs: f64 = durations.iter().map(Duration::as_secs_f64).sum();
    let avg_ms = (total_secs * 1000.0) / runs as f64;
    let warm_ms = if runs > 1 {
        (durations
            .iter()
            .skip(1)
            .map(Duration::as_secs_f64)
            .sum::<f64>()
            * 1000.0)
            / (runs - 1) as f64
    } else {
        avg_ms
    };

    let best = durations
        .iter()
        .map(Duration::as_secs_f64)
        .fold(f64::INFINITY, f64::min)
        * 1000.0;

    eprintln!("Summary");
    eprintln!("- average: {:.2} ms", avg_ms);
    eprintln!("- warm average (excluding first run): {:.2} ms", warm_ms);
    eprintln!("- best: {:.2} ms", best);

    Ok(())
}

fn run_ci_strict(root: &Path) -> Result<(), i32> {
    let build_jobs = build_jobs();
    let ci_build_jobs = [("CARGO_BUILD_JOBS", build_jobs.as_str())];

    run_ci_prelude(root)?;
    run_cargo(root, FMT_CHECK)?;
    run_cargo_with_env(
        root,
        &["check", "--workspace", "--all-targets", "--locked"],
        &ci_build_jobs,
    )?;
    run_cargo_with_env(
        root,
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        &ci_build_jobs,
    )?;
    run_cargo_with_env(
        root,
        &["doc", "--workspace", "--no-deps", "--locked"],
        &[
            ("RUSTDOCFLAGS", "-D warnings"),
            ("CARGO_BUILD_JOBS", build_jobs.as_str()),
        ],
    )?;
    run_workspace_tests(root, true)?;
    Ok(())
}

fn run_lang_docs(root: &Path, extra: &[String]) -> Result<(), i32> {
    let check_mode = extra.iter().any(|arg| arg == "--check");
    if extra.iter().any(|arg| arg != "--check") {
        eprintln!("lang-docs only accepts optional --check");
        return Err(1);
    }

    let model = fresco::language_docs::build_language_docs_model_with_prelude(
        fresco_example_engine::PRELUDE,
    )
    .map_err(|errors| {
        eprintln!("invalid example prelude: {errors:?}");
        1
    })?;
    let json = serde_json::to_string_pretty(&model).expect("language docs serialize");
    let markdown = fresco::language_docs::render_language_docs_markdown(&model);

    struct Artifact {
        path: PathBuf,
        payload: String,
        create_dir_error: &'static str,
        write_error: &'static str,
    }
    let artifacts = [
        Artifact {
            path: root.join(LANGUAGE_DOCS_ARTIFACT_PATH),
            payload: format!("{json}\n"),
            create_dir_error: "failed to create",
            write_error: "failed to write",
        },
        Artifact {
            path: root.join(LANGUAGE_DOCS_MARKDOWN_ARTIFACT_PATH),
            payload: format!("{markdown}\n"),
            create_dir_error: "failed to create output directory",
            write_error: "failed to write language docs markdown artifact",
        },
    ];
    let listing: String = artifacts
        .iter()
        .map(|artifact| format!("\n- {}", artifact.path.display()))
        .collect();

    if check_mode {
        let mut stale = false;
        for artifact in &artifacts {
            let existing = fs::read_to_string(&artifact.path).map_err(|e| {
                eprintln!(
                    "failed to read {} (run `cargo xtask lang-docs`): {e}",
                    artifact.path.display()
                );
                1
            })?;
            stale |= existing != artifact.payload;
        }
        if stale {
            eprintln!(
                "language docs artifacts are out of date; run `cargo xtask lang-docs` and commit the results{listing}"
            );
            return Err(1);
        }
        println!("language docs artifacts are up to date{listing}");
        return Ok(());
    }

    for artifact in &artifacts {
        if let Some(parent) = artifact.path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                eprintln!("{} {}: {e}", artifact.create_dir_error, parent.display());
                1
            })?;
        }
    }
    for artifact in &artifacts {
        fs::write(&artifact.path, &artifact.payload).map_err(|e| {
            eprintln!("{} {}: {e}", artifact.write_error, artifact.path.display());
            1
        })?;
    }
    for artifact in &artifacts {
        println!("wrote {}", artifact.path.display());
    }
    Ok(())
}

fn run_dist(root: &Path) -> Result<(), i32> {
    run_cargo(root, &["build", "--release"])
}

fn run_clean_all(root: &Path) -> Result<(), i32> {
    run_cargo(root, &["clean"])?;

    let dist_dir = root.join("dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir).map_err(|e| {
            eprintln!("failed to remove {}: {e}", dist_dir.display());
            1
        })?;
        eprintln!("> removed {}", dist_dir.display());
    }

    Ok(())
}

fn run_gate_status(root: &Path) -> Result<(), i32> {
    let wf_path = root.join(".github").join("workflows").join("ci.yml");
    fs::metadata(&wf_path).map_err(|e| {
        eprintln!("failed to read {}: {e}", wf_path.display());
        1
    })?;

    eprintln!("Gate status");
    eprintln!("- CI gate (`cargo xtask ci-strict`): blocking build/test/fmt/clippy");
    eprintln!("- local gate (`cargo xtask ci`): blocking build/test, warning-only fmt/clippy");
    eprintln!();
    eprintln!("Promotion criteria");
    eprintln!("1. `cargo xtask ci-strict` passes (run via GitHub Actions on every PR)");
    eprintln!("2. dependency hygiene jobs are green on pull requests");
    Ok(())
}

fn warn_only(result: Result<(), i32>, message: &str) {
    if result.is_err() {
        emit_warning(message);
    }
}

fn emit_warning(message: &str) {
    eprintln!("warning: {message}");
    if env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        eprintln!("::warning::{message}");
    }
}

fn run_fmt_guard(root: &Path) -> Result<(), i32> {
    let rustfmt_toml_path = root.join("rustfmt.toml");
    let rustfmt_toml = fs::read_to_string(&rustfmt_toml_path).map_err(|e| {
        eprintln!("failed to read {}: {e}", rustfmt_toml_path.display());
        1
    })?;

    let expected_edition_line = format!("edition = \"{}\"", RUSTFMT_EDITION);
    if !rustfmt_toml.contains(&expected_edition_line) {
        eprintln!(
            "{} must contain `{}` to keep formatter edition stable",
            rustfmt_toml_path.display(),
            expected_edition_line
        );
        return Err(1);
    }

    let hook_path = root.join(".githooks").join("pre-commit");
    let hook = fs::read_to_string(&hook_path).map_err(|e| {
        eprintln!("failed to read {}: {e}", hook_path.display());
        1
    })?;

    if !hook.contains(HOOK_EXPECTED_RUSTFMT) {
        eprintln!(
            "{} must invoke `{}` to match workspace formatting policy",
            hook_path.display(),
            HOOK_EXPECTED_RUSTFMT
        );
        return Err(1);
    }

    Ok(())
}

fn run_install_hooks(root: &Path) -> Result<(), i32> {
    let hook_path = root.join(".githooks").join("pre-commit");
    fs::metadata(&hook_path).map_err(|e| {
        eprintln!("failed to read {}: {e}", hook_path.display());
        1
    })?;

    #[cfg(unix)]
    {
        fs::set_permissions(&hook_path, Permissions::from_mode(0o755)).map_err(|e| {
            eprintln!(
                "failed to set executable bit on {}: {e}",
                hook_path.display()
            );
            1
        })?;
    }

    eprintln!("> git config core.hooksPath .githooks");
    exit_status(
        "git config",
        Command::new("git")
            .current_dir(root)
            .args(["config", "core.hooksPath", ".githooks"])
            .status(),
    )?;

    eprintln!(
        "Installed git hook path. Staged Rust files will now be rustfmt'ed by .githooks/pre-commit."
    );
    Ok(())
}

fn run_cargo_with_extra(root: &Path, args: &[&str], extra: &[String]) -> Result<(), i32> {
    let args: Vec<&str> = args
        .iter()
        .copied()
        .chain(extra.iter().map(String::as_str))
        .collect();
    run_cargo(root, &args)
}

fn run_cargo(root: &Path, args: &[impl AsRef<str>]) -> Result<(), i32> {
    run_cargo_with_env(root, args, &[])
}

/// Runs `cargo <args>` in `root` with extra environment variables, echoing
/// the command line first.
fn run_cargo_with_env(
    root: &Path,
    args: &[impl AsRef<str>],
    envs: &[(&str, &str)],
) -> Result<(), i32> {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let env_display: String = envs.iter().map(|(k, v)| format!("{k}={v} ")).collect();
    eprintln!("> {env_display}cargo {}", args.join(" "));

    exit_status(
        "cargo",
        Command::new("cargo")
            .current_dir(root)
            .args(&args)
            .envs(envs.iter().copied())
            .status(),
    )
}

/// Maps a spawned command's status to the xtask result: a spawn failure is
/// reported and becomes exit code 1, a failing exit keeps its code.
fn exit_status(program: &str, status: io::Result<ExitStatus>) -> Result<(), i32> {
    let status = status.map_err(|e| {
        eprintln!("failed to run {program}: {e}");
        1
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(status.code().unwrap_or(1))
    }
}
