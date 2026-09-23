pub(crate) fn print_help() {
    eprintln!(
        "xtask - lightweight workspace automation\n\n\
Usage:\n\
    cargo xtask <command> [args]\n\n\
Commands:\n\
  check              Run cargo check --workspace --all-targets\n\
    check-fast         Run cargo check --profile dev-fast\n\
    check-timing [runs] [profile]\n\
                                         Time repeated cargo check runs (default: 3 runs, dev profile)\n\
    timed-run <input.fr> [options] [-- <extra fresco-cli args>]\n\
                                                                                 Run fresco-cli with trace env + optional backend feature\n\
                                                                                 Options: --trace <off|fmt|json|chrome|tracy>\n\
                                                                                                    --log <rust_log_filter> | --no-log\n\
  test               Run cargo test --workspace\n\
    repo-guard         Verify LF text and reject mod.rs files\n\
    readme-sync [--check] Sync README samples and compile their source programs\n\
    fmt                Run cargo fmt --all with edition guard config\n\
    install-hooks      Configure core.hooksPath=.githooks (enables repo pre-commit hook)\n\
    fmt-guard          Verify rustfmt edition settings in rustfmt.toml and pre-commit hook\n\
  clippy             Run cargo clippy --workspace --all-targets\n\
    lang-docs [--check] Generate docs artifacts (JSON + Markdown) or verify they are up to date\n\
    ci                 Run CI gate: fmt/clippy warn only, check/test blocking\n\
    ci-strict          Run source hygiene, fmt(check), check, clippy(-D warnings), tests, rustdoc\n\
    gate-status        Show baseline/strict gate mode and promotion criteria\n\
    dist               Run cargo build --release\n\
    clean-all          Run cargo clean and remove ./dist if present\n\
  run [args...]      Forward args to cargo run\n\
    run-fast [args...] Run cargo run --profile dev-fast with forwarded args\n\
  help               Show this help\n\n\
Concurrency: test/ci/ci-strict default to half the available logical CPUs (rounded up, minimum 1).\n\
Override build jobs with CARGO_BUILD_JOBS and test workers with RUST_TEST_THREADS.\n"
    );
}
