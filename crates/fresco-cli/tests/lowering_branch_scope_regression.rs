#[path = "support/common.rs"]
mod common;

use common::{normalize, repo_root, run_example as run_fresco};

#[test]
fn if_branch_cache_scope_does_not_emit_out_of_scope_identifiers() {
    let root = repo_root();
    let file = root
        .join("examples")
        .join("90) gallery")
        .join("perspective_flip_card.fr");

    let output = run_fresco(&file);
    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = String::from_utf8(output.stdout).expect("expected utf-8 WGSL output");

    let module = naga::front::wgsl::parse_str(&wgsl).unwrap_or_else(|e| {
        panic!(
            "expected WGSL parse to succeed\nerror:\n{e}\n\nWGSL (first 120 lines):\n{}",
            wgsl.lines().take(120).collect::<Vec<_>>().join("\n")
        )
    });

    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    validator.validate(&module).unwrap_or_else(|e| {
        panic!(
            "expected WGSL validation to succeed\nerror:\n{e:#?}\n\nWGSL (first 120 lines):\n{}",
            wgsl.lines().take(120).collect::<Vec<_>>().join("\n")
        )
    });
}
