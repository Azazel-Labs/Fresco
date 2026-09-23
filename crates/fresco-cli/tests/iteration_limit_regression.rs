//! Safe boundary tests. These do not attempt an allocation-exhaustion probe;
//! rejecting huge ranges before allocation needs separate resource accounting.
use std::fs;

#[path = "support/common.rs"]
mod common;

fn run_range(start: i32, end: i32) -> std::process::Output {
    let source = format!(
        "fn choose(x: f32) -> color {{\n\
         for i in {start} .. {end} {{ let unused = i }}\n\
         return #ff0000\n}}\n\
         canvas t(uv: coord) -> color {{ compose {{ choose(uv.x) }} }}"
    );
    let path = common::unique_temp_path("iteration_limit");
    fs::write(&path, source).unwrap();
    let output = common::run_fresco(&path);
    fs::remove_file(path).unwrap();
    output
}

#[test]
fn range_at_limit_is_accepted_in_both_directions() {
    for (start, end) in [(0, 4096), (4096, 0)] {
        let output = run_range(start, end);
        assert!(
            output.status.success(),
            "{start} .. {end}: {}",
            common::normalize(&output.stderr)
        );
    }
}

#[test]
fn range_over_limit_reports_iteration_count_in_both_directions() {
    for (start, end) in [(0, 4097), (4097, 0), (-2048, 2049)] {
        let output = run_range(start, end);
        let stderr = common::normalize(&output.stderr);
        assert!(!output.status.success(), "{start} .. {end} should fail");
        assert!(
            stderr.contains("4097 iterations") && stderr.contains("limit of 4096"),
            "expected range-limit diagnostic, not a crash or unrelated error: {stderr}"
        );
    }
}
