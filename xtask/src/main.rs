use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
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
        "run" => run_cargo_with_extra(&root, "run", &extra),
        "run-fast" => run_cargo_with_profile_extra(&root, "run", "dev-fast", &extra),
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

fn run_workspace_tests(root: &PathBuf, locked: bool) -> Result<(), i32> {
    let workers = default_workers().to_string();
    let build_jobs = env::var("CARGO_BUILD_JOBS").unwrap_or_else(|_| workers.clone());
    let test_threads = env::var("RUST_TEST_THREADS").unwrap_or(workers);
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

fn run_ci(root: &PathBuf) -> Result<(), i32> {
    let build_jobs = env::var("CARGO_BUILD_JOBS").unwrap_or_else(|_| default_workers().to_string());
    let ci_build_jobs = [("CARGO_BUILD_JOBS", build_jobs.as_str())];

    repo_guard::run(root)?;
    readme::run(root, false)?;
    run_fmt_guard(root)?;
    warn_only(
        run_cargo(
            root,
            &[
                "fmt",
                "--all",
                "--",
                "--check",
                "--config",
                RUSTFMT_CONFIG_EDITION,
            ],
        ),
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

fn run_timed_run(root: &PathBuf, extra: &[String]) -> Result<(), i32> {
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

    let mut envs: Vec<(String, String)> = vec![("FRESCO_TRACE".to_string(), trace_mode)];
    if let Some(filter) = log_filter {
        envs.push(("RUST_LOG".to_string(), filter));
    }

    let env_display = envs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!("> {env_display} cargo {}", cargo_args.join(" "));

    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).args(&cargo_args);
    for (k, v) in &envs {
        cmd.env(k, v);
    }

    let status = cmd.status().map_err(|e| {
        eprintln!("failed to run cargo: {e}");
        1
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(status.code().unwrap_or(1))
    }
}

fn run_check_timing(root: &PathBuf, extra: &[String]) -> Result<(), i32> {
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
        run_cargo_owned(root, &args)?;
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

fn run_ci_strict(root: &PathBuf) -> Result<(), i32> {
    let build_jobs = env::var("CARGO_BUILD_JOBS").unwrap_or_else(|_| default_workers().to_string());
    let ci_build_jobs = [("CARGO_BUILD_JOBS", build_jobs.as_str())];

    repo_guard::run(root)?;
    readme::run(root, false)?;
    run_fmt_guard(root)?;
    run_cargo(
        root,
        &[
            "fmt",
            "--all",
            "--",
            "--check",
            "--config",
            RUSTFMT_CONFIG_EDITION,
        ],
    )?;
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

    let output_path = root.join(LANGUAGE_DOCS_ARTIFACT_PATH);
    let markdown_output_path = root.join(LANGUAGE_DOCS_MARKDOWN_ARTIFACT_PATH);
    let model = fresco::language_docs::build_language_docs_model_with_prelude(
        fresco_example_engine::PRELUDE,
    )
    .map_err(|errors| {
        eprintln!("invalid example prelude: {errors:?}");
        1
    })?;
    let json = serde_json::to_string_pretty(&model).expect("language docs serialize");
    let markdown = fresco::language_docs::render_language_docs_markdown(&model);
    let payload = format!("{json}\n");
    let markdown_payload = format!("{markdown}\n");

    if check_mode {
        let existing_json = fs::read_to_string(&output_path).map_err(|e| {
            eprintln!(
                "failed to read {} (run `cargo xtask lang-docs`): {e}",
                output_path.display()
            );
            1
        })?;
        let existing_markdown = fs::read_to_string(&markdown_output_path).map_err(|e| {
            eprintln!(
                "failed to read {} (run `cargo xtask lang-docs`): {e}",
                markdown_output_path.display()
            );
            1
        })?;
        if existing_json != payload || existing_markdown != markdown_payload {
            eprintln!(
                "language docs artifacts are out of date; run `cargo xtask lang-docs` and commit the results\n- {}\n- {}",
                output_path.display(),
                markdown_output_path.display()
            );
            return Err(1);
        }
        println!(
            "language docs artifacts are up to date\n- {}\n- {}",
            output_path.display(),
            markdown_output_path.display()
        );
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            eprintln!("failed to create {}: {e}", parent.display());
            1
        })?;
    }
    if let Some(parent) = markdown_output_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            eprintln!(
                "failed to create output directory {}: {e}",
                parent.display()
            );
            1
        })?;
    }

    fs::write(&output_path, payload).map_err(|e| {
        eprintln!("failed to write {}: {e}", output_path.display());
        1
    })?;
    fs::write(&markdown_output_path, markdown_payload).map_err(|e| {
        eprintln!(
            "failed to write language docs markdown artifact {}: {e}",
            markdown_output_path.display()
        );
        1
    })?;

    println!("wrote {}", output_path.display());
    println!("wrote {}", markdown_output_path.display());
    Ok(())
}

fn run_dist(root: &PathBuf) -> Result<(), i32> {
    run_cargo(root, &["build", "--release"])
}

fn run_clean_all(root: &PathBuf) -> Result<(), i32> {
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
    let status = Command::new("git")
        .current_dir(root)
        .args(["config", "core.hooksPath", ".githooks"])
        .status()
        .map_err(|e| {
            eprintln!("failed to run git config: {e}");
            1
        })?;

    if !status.success() {
        return Err(status.code().unwrap_or(1));
    }

    eprintln!(
        "Installed git hook path. Staged Rust files will now be rustfmt'ed by .githooks/pre-commit."
    );
    Ok(())
}

fn run_cargo_with_extra(root: &PathBuf, subcommand: &str, extra: &[String]) -> Result<(), i32> {
    let mut args = vec![subcommand.to_string()];
    args.extend(extra.iter().cloned());
    run_cargo_owned(root, &args)
}

fn run_cargo_with_profile_extra(
    root: &PathBuf,
    subcommand: &str,
    profile: &str,
    extra: &[String],
) -> Result<(), i32> {
    let mut args = vec![
        subcommand.to_string(),
        "--profile".to_string(),
        profile.to_string(),
    ];
    args.extend(extra.iter().cloned());
    run_cargo_owned(root, &args)
}

fn run_cargo(root: &PathBuf, args: &[&str]) -> Result<(), i32> {
    let owned: Vec<String> = args.iter().map(ToString::to_string).collect();
    run_cargo_owned(root, &owned)
}

fn run_cargo_owned(root: &PathBuf, args: &[String]) -> Result<(), i32> {
    eprintln!("> cargo {}", args.join(" "));
    let status = Command::new("cargo")
        .current_dir(root)
        .args(args)
        .status()
        .map_err(|e| {
            eprintln!("failed to run cargo: {e}");
            1
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(status.code().unwrap_or(1))
    }
}

fn run_cargo_with_env(root: &PathBuf, args: &[&str], envs: &[(&str, &str)]) -> Result<(), i32> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }

    eprintln!(
        "> {} cargo {}",
        envs.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" "),
        args.join(" ")
    );

    let status = cmd.status().map_err(|e| {
        eprintln!("failed to run cargo: {e}");
        1
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(status.code().unwrap_or(1))
    }
}
