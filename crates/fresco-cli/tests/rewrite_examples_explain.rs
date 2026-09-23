use std::{fs, path::PathBuf, process::Command};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("failed to resolve workspace root")
}

fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .replace("\\\\?\\", "")
        .replace('\\', "/")
}

fn run_explain(input_rel: &str) -> String {
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");
    let output = Command::new(bin)
        .arg(repo_root().join(input_rel))
        .arg("--engine-dir")
        .arg(repo_root().join("integrations/example-engine/engine"))
        .arg("--emit")
        .arg("wgsl")
        .arg("--explain")
        .output()
        .expect("failed to run fresco binary");
    assert!(
        output.status.success(),
        "fresco failed for {input_rel}\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
    normalize(&output.stderr)
}

#[test]
fn rewrite_guard_true_example_reports_fired_rule() {
    let explain = run_explain("examples/16) user-effects/rewrite_guard_true.fr");
    assert!(
        explain.contains("rewrite: grain(a) ∘ grain(b) ⇒ grain(a) — user-defined rule fired"),
        "expected fired rewrite note\nexplain:\n{explain}"
    );
}

#[test]
fn rewrite_guard_false_example_reports_no_fired_rule() {
    let explain = run_explain("examples/16) user-effects/rewrite_guard_false.fr");
    assert!(
        !explain.contains("user-defined rule fired"),
        "expected no fired rewrite note\nexplain:\n{explain}"
    );
}

#[test]
fn rewrite_locality_example_reports_rule_and_locality_line() {
    let explain = run_explain("examples/16) user-effects/rewrite_locality_receipt.fr");
    assert!(
        explain.contains(
            "rewrite: neighborhood(a) ∘ neighborhood(b) ⇒ neighborhood(a) — user-defined rule fired"
        ),
        "expected locality example rewrite note\nexplain:\n{explain}"
    );
    assert!(
        explain.contains("locality: point="),
        "expected locality receipt line\nexplain:\n{explain}"
    );
}

#[test]
fn rewrite_non_match_example_reports_no_fired_rule() {
    let explain = run_explain("examples/16) user-effects/rewrite_non_match.fr");
    assert!(
        !explain.contains("user-defined rule fired"),
        "expected non-match example to skip rewrite\nexplain:\n{explain}"
    );
}

#[test]
fn rewrite_roadmap_preserves_example_references() {
    let guide = fs::read_to_string(repo_root().join("TODO.md"))
        .expect("failed to read roadmap containing rewrite contracts");
    for entry in [
        "rewrite_guard_true.fr",
        "rewrite_guard_false.fr",
        "rewrite_locality_receipt.fr",
        "rewrite_non_match.fr",
    ] {
        assert!(
            guide.contains(entry),
            "expected guide to mention {entry}\ncontent:\n{guide}"
        );
    }
}
