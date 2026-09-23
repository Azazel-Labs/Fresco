#[path = "support/common.rs"]
mod common;

use common::{normalize, repo_root, run_example_with_args as run_fresco_with_args};

fn parse_scatter_lowering_value(explain: &str, key: &str) -> usize {
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

#[test]
fn scatter_index_phase_keeps_all_instances_in_wgsl() {
    let root = repo_root();
    let file = root
        .join("examples")
        .join("10) fundamentals")
        .join("scatter_index_phase.fr");

    let output = run_fresco_with_args(&file, &["--explain"]);
    assert!(
        output.status.success(),
        "expected scatter_index_phase to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    let emitted_instances = parse_scatter_lowering_value(&explain, "emitted_instances");
    assert_eq!(
        emitted_instances, 12,
        "expected 12 emitted scatter instances in explain output\nexplain:\n{explain}"
    );
}
