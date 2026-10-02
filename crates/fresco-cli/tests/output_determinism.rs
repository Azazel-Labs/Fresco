//! Generated output must not depend on hash iteration order. Each CLI run is a
//! fresh process with fresh `HashMap` seeds, so repeated runs of the same input
//! expose any output that leaks iteration order.

#[path = "support/common.rs"]
mod common;

use common::{normalize, repo_root, run_example_with_args};
use std::collections::HashMap;

const RUNS: usize = 4;

fn assert_stable(relative: &str, args: &[&str]) {
    let file = repo_root().join(relative);
    let mut first: Option<String> = None;
    for run in 0..RUNS {
        let output = run_example_with_args(&file, args);
        assert!(
            output.status.success(),
            "expected {relative} {args:?} to compile\nstderr:\n{}",
            normalize(&output.stderr)
        );
        let text = format!(
            "{}\n--- stderr ---\n{}",
            normalize(&output.stdout),
            normalize(&output.stderr)
        );
        match &first {
            None => first = Some(text),
            Some(expected) => assert!(
                *expected == text,
                "{relative} {args:?} changed between runs (run {run})\nfirst:\n{expected}\nthis run:\n{text}"
            ),
        }
    }
}

#[test]
fn surface_param_globals_lower_in_stable_order() {
    // Material params were bound by iterating a HashMap.
    assert_stable(
        "examples/40) surface shaders/style_sample.fr",
        &["--emit", "ir"],
    );
    assert_stable(
        "examples/40) surface shaders/vertex-wave.fr",
        &["--emit", "ir"],
    );
}

#[test]
fn helper_materialization_and_eager_validation_are_ordered() {
    // Pending helpers and eager overload validation iterated HashMaps, which
    // reordered let numbering (and so commutative operand order) and the
    // explain loop receipts.
    let file = "examples/90) gallery/iq_distance_gallery_original_direct.fr";
    assert_stable(file, &[]);
    assert_stable(file, &["--explain"]);
}

#[test]
fn explain_does_not_report_loops_from_eager_validation() {
    let output = run_example_with_args(
        repo_root().join("examples/90) gallery/iq_distance_gallery_original_direct.fr"),
        &["--explain"],
    );
    let explain = normalize(&output.stderr);
    let loops: Vec<&str> = explain
        .lines()
        .filter(|line| line.trim_start().starts_with("loop: unrolled"))
        .collect();
    let mut unique = loops.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        loops.len(),
        unique.len(),
        "each unrolled loop should be reported once\nexplain:\n{explain}"
    );
}

#[test]
fn missing_virtual_module_lists_available_files_sorted() {
    let engine_files: HashMap<String, String> = [
        "engine/engine.fr",
        "engine/zeta.fr",
        "engine/alpha.fr",
        "engine/mid.fr",
        "engine/beta.fr",
    ]
    .into_iter()
    .map(|path| {
        let source = if path == "engine/engine.fr" {
            "import \"missing.fr\"\n".to_string()
        } else {
            String::new()
        };
        (path.to_string(), source)
    })
    .collect();

    let mut messages = Vec::new();
    for _ in 0..RUNS {
        let diagnostics = fresco::driver::compile_source_bundle_with_engine_files(
            "canvas t(ctx: CanvasContext) -> color { compose { fill(#ffffff) } }\n",
            "main.fr",
            false,
            &fresco::driver::CompileContext::default(),
            &engine_files,
            "engine/engine.fr",
        )
        .expect_err("missing engine import must fail");
        let message = diagnostics
            .iter()
            .map(|diag| diag.message.clone())
            .find(|message| message.contains("not found in virtual file system"))
            .unwrap_or_else(|| panic!("expected a missing-module diagnostic: {diagnostics:?}"));
        messages.push(message);
    }

    assert!(
        messages[0].contains(
            "(available: engine/alpha.fr, engine/beta.fr, engine/engine.fr, engine/mid.fr, engine/zeta.fr)"
        ),
        "expected sorted available list: {}",
        messages[0]
    );
    assert!(
        messages.iter().all(|message| *message == messages[0]),
        "missing-module diagnostic changed between compiles: {messages:?}"
    );
}
