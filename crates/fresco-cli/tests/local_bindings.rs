#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, unique_temp_path};
use std::fs;

#[test]
fn mutable_locals_compile_through_helpers_and_canvas_control_flow() {
    let source = r#"
fn accumulate(x: f32) -> f32 {
    var result: f32 = x;
    for i in 0..3 { result += 0.05; }
    return result;
}
canvas test(uv: coord) -> color {
    var point: vec2 = uv;
    point.x += accumulate(0.1);
    let shade: f32 = point.x;
    compose { fill(rgba(shade, point.y, 0.0, 1.0)) }
}
"#;
    let path = unique_temp_path("mutable_locals");
    fs::write(&path, source).unwrap();
    let output = run_fresco(&path);
    fs::remove_file(path).unwrap();
    assert!(output.status.success(), "{}", normalize(&output.stderr));
}

#[test]
fn immutable_assignment_diagnostics_survive_the_full_pipeline() {
    for body in [
        "let x = 0.1; x = 0.2;",
        "let x = 0.1; x += 0.2;",
        "let x = vec2(0.1); x.y = 0.2;",
        "let x = 0.1; if false { x = 0.2; }",
        "let x = 0.1; { var x = 0.2; x += 0.1; } x = 0.3;",
    ] {
        let source =
            format!("canvas test(uv: coord) -> color {{ {body} compose {{ fill(#fff) }} }}");
        let path = unique_temp_path("immutable_local");
        fs::write(&path, source).unwrap();
        let output = run_fresco(&path);
        fs::remove_file(path).unwrap();
        let stderr = normalize(&output.stderr);
        assert!(!output.status.success(), "{body}");
        assert!(
            stderr.contains("cannot assign to immutable binding `x`"),
            "{stderr}"
        );
        assert!(
            stderr.contains("immutable binding declared here"),
            "{stderr}"
        );
    }
}
