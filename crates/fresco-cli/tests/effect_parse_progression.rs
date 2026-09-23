/// Integration tests for §16.1 user-defined effects.
///
/// Tests are organized into phases matching the implementation plan:
///   Phase 1 — AST & Parser (locality syntax + effect declarations)
///   Phase 2 — Effect Symbol Table & Checker Integration
///   Phase 5 — Rewrite rule engine (rules fire at compile time)
///   Phase 6 — Effect references as callable parameters
///
/// Phase 1 tests verify that `effect` declarations parse and round-trip without
/// semantic meaning.  Phase 2 tests verify that effects type-check, that
/// calling them in a canvas compiles to valid WGSL, and that semantic errors
/// (bad locality, arity mismatches) are reported correctly.  Phase 6 tests
/// verify that an effect declaration name can be passed as a callable argument
/// to a function that accepts `fn(…)->layer`, dispatched correctly inside the
/// function body, and that arity errors are caught at the call site.
use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco_with_args, unique_temp_path};

// ─── helpers ──────────────────────────────────────────────────────────────────

fn assert_compiles(suffix: &str, input: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

fn assert_compiles_valid_wgsl(suffix: &str, input: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = String::from_utf8(output.stdout).expect("expected utf-8 WGSL output");
    let module = naga::front::wgsl::parse_str(&wgsl)
        .unwrap_or_else(|e| panic!("expected WGSL parse to succeed\nerror:\n{e}\nwgsl:\n{wgsl}"));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    validator.validate(&module).unwrap_or_else(|e| {
        panic!("expected WGSL validation to succeed\nerror:\n{e:#?}\nwgsl:\n{wgsl}")
    });
}

fn compile_to_wgsl(suffix: &str, input: &str) -> String {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("expected utf-8 WGSL output")
}

fn assert_compile_fails_with(suffix: &str, input: &str, expected_in_stderr: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &[]);
    let _ = fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected compile to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains(expected_in_stderr),
        "expected stderr to contain `{expected_in_stderr}`\nstderr:\n{stderr}"
    );
}

// ─── Phase 1: Parser — locality keyword variants ──────────────────────────────

#[test]
fn point_effect_parses() {
    // A `point` locality effect with a single scalar param and a fill body.
    assert_compiles(
        "point_effect_parses",
        r#"
effect tint_red(amount: f32) point {
    fill(rgba(r: amount, g: 0.0, b: 0.0, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        circle(at: center, radius: 50px) |> fill(#ffffff)
    }
}
"#,
    );
}

#[test]
fn local_effect_bare_parses() {
    // A bare `local` locality effect (no radius bound).
    assert_compiles(
        "local_effect_bare_parses",
        r#"
effect edge_glow(strength: f32) local {
    fill(rgba(r: strength, g: strength, b: 0.0, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000)
    }
}
"#,
    );
}

#[test]
fn local_effect_with_radius_parses() {
    // A `local(r)` effect carrying an explicit radius constraint expression.
    assert_compiles(
        "local_effect_with_radius_parses",
        r#"
effect neighborhood(radius: f32) local(radius * 2.0) {
    fill(rgba(r: 0.0, g: radius, b: 0.5, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000)
    }
}
"#,
    );
}

#[test]
fn global_effect_parses() {
    // A `global` locality effect.
    assert_compiles(
        "global_effect_parses",
        r#"
effect full_pass(strength: f32) global {
    fill(rgba(r: strength, g: 0.0, b: 0.5, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000)
    }
}
"#,
    );
}

#[test]
fn effect_with_rewrite_rule_parses() {
    // An effect carrying a rewrite rule: `grain(a) compose grain(b) => grain(a)`
    assert_compiles(
        "effect_with_rewrite_rule_parses",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a)
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000)
    }
}
"#,
    );
}

#[test]
fn effect_with_when_guard_parses() {
    // A rewrite rule with a `when` guard expression.
    assert_compiles(
        "effect_with_when_guard_parses",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a) when a > b
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000)
    }
}
"#,
    );
}

#[test]
fn effect_with_within_tolerance_parses() {
    // A rewrite rule may declare a numeric tolerance with `within ε`.
    assert_compiles(
        "effect_with_within_tolerance_parses",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a) within 0.5 / 255.0
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000)
    }
}
"#,
    );
}

#[test]
fn effect_with_no_params_parses() {
    // An effect with zero parameters still parses correctly.
    assert_compiles(
        "effect_with_no_params_parses",
        r#"
effect white_fill() point {
    fill(#ffffff)
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000)
    }
}
"#,
    );
}

// ─── Phase 1: Parser — error cases ───────────────────────────────────────────

#[test]
fn missing_locality_keyword_is_rejected() {
    // An effect missing the locality clause should produce a parse error.
    assert_compile_fails_with(
        "missing_locality_keyword",
        r#"
effect bad_effect(x: f32) {
    fill(#ffffff)
}

canvas t(uv: coord) -> color {
    compose { fill(#000000) }
}
"#,
        "unexpected",
    );
}

// ─── Phase 2: Checker — effect callable from canvas ──────────────────────────

#[test]
fn point_effect_callable_from_canvas() {
    // A point effect can be called as a standalone layer in a canvas compose block.
    assert_compiles_valid_wgsl(
        "point_effect_callable_from_canvas",
        r#"
effect bright(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        bright(0.5)
    }
}
"#,
    );
}

#[test]
fn point_effect_piped_from_layer() {
    // A point effect can receive a piped-in layer.
    assert_compiles_valid_wgsl(
        "point_effect_piped_from_layer",
        r#"
effect tint_blue(amount: f32) point {
    fill(rgba(r: 0.0, g: 0.0, b: amount, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        circle(at: center, radius: 50px) |> fill(#ff0000) |> tint_blue(0.8)
    }
}
"#,
    );
}

#[test]
fn point_effect_can_read_self_channels() {
    assert_compiles_valid_wgsl(
        "point_effect_reads_self_channels",
        r#"
effect tint_blue(amount: f32) point {
    layer rgba(r: self.r, g: self.g, b: amount, a: self.a)
}

canvas t(uv: coord) -> color {
    compose {
        circle(at: center, radius: 50px) |> fill(rgba(r: 0.125, g: 0.25, b: 0.5, a: 1.0)) |> tint_blue(0.8)
    }
}
"#,
    );
}

#[test]
fn local_effect_can_resample_self_at_offset() {
    let wgsl = compile_to_wgsl(
        "local_effect_resamples_self_at_offset",
        r#"
effect chromatic_split(spread: f32) local(spread) {
    let left = self at (coord - (spread, 0.0))
    let right = self at (coord + (spread, 0.0))
    layer rgba(r: left.r, g: self.g, b: right.b, a: self.a)
}

canvas t(uv: coord) -> color {
    compose {
        circle(at: center, radius: 50px) |> fill(#e8d5b7) |> chromatic_split(0.03)
    }
}
"#,
    );

    assert!(
        wgsl.contains("0.03") || wgsl.contains("3e-2"),
        "expected the chromatic split offset to appear in the emitted WGSL\nwgsl:\n{wgsl}"
    );
}

#[test]
fn local_effect_with_radius_callable() {
    // A local(r) effect is callable from a canvas and produces valid WGSL.
    assert_compiles_valid_wgsl(
        "local_effect_with_radius_callable",
        r#"
effect soft_edge(r: f32) local(r) {
    fill(rgba(r: 0.5, g: r, b: 0.5, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        soft_edge(0.1)
    }
}
"#,
    );
}

#[test]
fn effect_with_multiple_params_callable() {
    // An effect with multiple scalar params accepts arguments at the call site.
    assert_compiles_valid_wgsl(
        "effect_with_multiple_params_callable",
        r#"
effect tinted(r: f32, g: f32, b: f32) point {
    fill(rgba(r: r, g: g, b: b, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        tinted(1.0, 0.5, 0.0)
    }
}
"#,
    );
}

// ─── Phase 2: Checker — semantic errors ──────────────────────────────────────

#[test]
fn effect_arity_mismatch_is_rejected() {
    // Calling an effect with the wrong number of arguments is an error.
    assert_compile_fails_with(
        "effect_arity_mismatch",
        r#"
effect two_arg(x: f32, y: f32) point {
    fill(rgba(r: x, g: y, b: 0.0, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        two_arg(0.5)
    }
}
"#,
        "expects 2 argument(s), found 1",
    );
}

#[test]
fn builtin_effect_name_reservation_is_enforced() {
    // Declaring an effect with a builtin name like `blur` is rejected.
    assert_compile_fails_with(
        "builtin_name_reservation",
        r#"
effect blur(radius: f32) local(radius) {
    fill(rgba(r: 0.5, g: 0.5, b: 0.5, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose { fill(#000000) }
}
"#,
        "reserved for a builtin",
    );
}

#[test]
fn duplicate_effect_name_is_rejected() {
    // Declaring the same effect name twice is rejected.
    assert_compile_fails_with(
        "duplicate_effect_name",
        r#"
effect my_fx(x: f32) point {
    fill(rgba(r: x, g: 0.0, b: 0.0, a: 1.0))
}

effect my_fx(y: f32) point {
    fill(rgba(r: 0.0, g: y, b: 0.0, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose { fill(#000000) }
}
"#,
        "duplicate effect declaration",
    );
}

#[test]
fn point_effect_calling_local_builtin_is_rejected() {
    // A `point` effect may not contain `local` operations (cheapen-only law).
    // `blur` has Local locality; embedding it inside a point effect is illegal.
    assert_compile_fails_with(
        "cheapen_only_law_point_vs_local",
        r#"
effect bad_point(r: f32) point {
    circle(at: center, radius: 20px) |> fill(#ffffff) |> blur(r * 2.0)
}

canvas t(uv: coord) -> color {
    compose { fill(#000000) }
}
"#,
        "cheapen-only law",
    );
}

// ─── Phase 2: Checker — rewrite rule validation ───────────────────────────────

#[test]
fn rewrite_unbound_hole_is_rejected() {
    // A rewrite rule result that uses a hole not bound in the LHS is rejected.
    assert_compile_fails_with(
        "rewrite_unbound_hole",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(c)
}

canvas t(uv: coord) -> color {
    compose { fill(#000000) }
}
"#,
        "unbound hole",
    );
}

#[test]
fn rewrite_within_non_constant_is_rejected() {
    // `within` must be a compile-time constant.
    assert_compile_fails_with(
        "rewrite_within_non_constant",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a) within a
}

canvas t(uv: coord) -> color {
    compose { fill(#000000) }
}
"#,
        "rewrite tolerance `within` must be compile-time constant",
    );
}

#[test]
fn rewrite_within_non_positive_is_rejected() {
    // `within` must be positive.
    assert_compile_fails_with(
        "rewrite_within_non_positive",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a) within 0.0
}

canvas t(uv: coord) -> color {
    compose { fill(#000000) }
}
"#,
        "rewrite tolerance `within` must be a positive finite constant",
    );
}

// ─── Phase 2: Regression — builtins unaffected by user effects ────────────────

#[test]
fn blur_compose_fill_regression() {
    // The hardcoded `blur ∘ fill => soften` rewrite still fires unchanged.
    // This is a compilation regression guard, not an assertion of rewrite choice.
    assert_compiles_valid_wgsl(
        "blur_compose_fill_regression",
        r#"
canvas t(uv: coord) -> color {
    let sample_value = circle(at: center, radius: 50px)
    compose {
        sample_value |> fill(#ff0000) |> blur(4.0)
    }
}
"#,
    );
}

// ─── Phase 5: Rewrite rule engine — rules fire at compile time ────────────────

fn assert_explain_contains(suffix: &str, input: &str, expected_note: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        explain.contains(expected_note),
        "expected explain output to contain `{expected_note}`\nexplain:\n{explain}"
    );
}

fn assert_explain_does_not_contain(suffix: &str, input: &str, absent_note: &str) {
    let path = unique_temp_path(suffix);
    fs::write(&path, input).expect("failed to write temporary test source");
    let output = run_fresco_with_args(&path, &["--explain"]);
    let _ = fs::remove_file(&path);

    assert!(
        output.status.success(),
        "expected compile to succeed\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let explain = normalize(&output.stderr);
    assert!(
        !explain.contains(absent_note),
        "expected explain output NOT to contain `{absent_note}`\nexplain:\n{explain}"
    );
}

#[test]
fn user_defined_rewrite_rule_fires_for_composed_effects() {
    // `grain(0.1) |> grain(0.2)` matches `grain(a) compose grain(b) => grain(a)`.
    // The rule should fire and the explain output should record it.
    assert_explain_contains(
        "rewrite_rule_fires",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a)
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000) |> grain(0.1) |> grain(0.2)
    }
}
"#,
        "rewrite: grain(a) ∘ grain(b) ⇒ grain(a) — user-defined rule fired",
    );
}

#[test]
fn user_defined_rewrite_rule_with_true_guard_fires() {
    // Rule: `grain(a) compose grain(b) => grain(a) when a > b`.
    // With outer=grain(0.8) and inner=grain(0.2): a=0.8, b=0.2, guard 0.8>0.2 is true → fires.
    assert_explain_contains(
        "rewrite_guard_true_fires",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a) when a > b
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000) |> grain(0.2) |> grain(0.8)
    }
}
"#,
        "rewrite: grain(a) ∘ grain(b) ⇒ grain(a) — user-defined rule fired",
    );
}

#[test]
fn user_defined_rewrite_rule_with_false_guard_does_not_fire() {
    // Rule: `grain(a) compose grain(b) => grain(a) when a > b`.
    // With outer=grain(0.2) and inner=grain(0.8): a=0.2, b=0.8, guard 0.2>0.8 is false → skipped.
    assert_explain_does_not_contain(
        "rewrite_guard_false_skipped",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a) when a > b
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000) |> grain(0.8) |> grain(0.2)
    }
}
"#,
        "user-defined rule fired",
    );
}

#[test]
fn user_defined_rewrite_result_is_valid_wgsl() {
    // After a rewrite rule fires, the resulting program must still produce valid WGSL.
    assert_compiles_valid_wgsl(
        "rewrite_rule_wgsl_valid",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a)
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000) |> grain(0.1) |> grain(0.5)
    }
}
"#,
    );
}

#[test]
fn unmatched_effects_do_not_trigger_rewrite() {
    // Two different effects composed should not match the grain ∘ grain rule.
    assert_explain_does_not_contain(
        "rewrite_no_match_different_effects",
        r#"
effect grain(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))

    rewrite grain(a) compose grain(b) => grain(a)
}

effect bright(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))
}

canvas t(uv: coord) -> color {
    compose {
        fill(#000000) |> grain(0.1) |> bright(0.9)
    }
}
"#,
        "user-defined rule fired",
    );
}

// ─── Phase 6: Effect references as callable parameters ────────────────────────

#[test]
fn effect_ref_as_callable_dispatches_correctly() {
    // An effect name can be passed as a `fn(f32)->layer` callable argument.
    // The call inside the function body is routed through the effect path and
    // emits a `UserEffect` layer — verified by producing valid WGSL.
    assert_compiles_valid_wgsl(
        "effect_ref_callable_dispatch",
        r#"
effect shimmer(amount: f32) point {
    fill(rgba(r: amount, g: amount, b: amount, a: 1.0))
}

fn apply_effect(f: fn(f32)->layer, v: f32) -> layer {
    f(v)
}

canvas t(uv: coord) -> color {
    compose {
        apply_effect(shimmer, 0.5)
    }
}
"#,
    );
}

#[test]
fn effect_ref_arity_mismatch_at_call_site_is_rejected() {
    // Passing a 2-parameter effect where the callable expects 1 parameter must
    // produce an arity-mismatch diagnostic at the call site.
    assert_compile_fails_with(
        "effect_ref_arity_mismatch",
        r#"
effect shimmer(amount: f32, hue: f32) point {
    fill(rgba(r: amount, g: hue, b: amount, a: 1.0))
}

fn apply_effect(f: fn(f32)->layer, v: f32) -> layer {
    f(v)
}

canvas t(uv: coord) -> color {
    compose {
        apply_effect(shimmer, 0.5)
    }
}
"#,
        "arity mismatch",
    );
}
