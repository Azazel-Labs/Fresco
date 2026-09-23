//! Numerical regressions for the inline evaluator. Valid WGSL can still compute
//! the wrong answer, so sample the emitted scene at both sides of each branch.
use std::fs;

use naga::{BinaryOperator as B, Expression as E, Literal as L, MathFunction as M};

#[path = "support/common.rs"]
mod common;

fn compile(body: &str, prelude: &str) -> naga::Module {
    let source = format!(
        "{prelude}\nfn choose(x: f32) -> color {{\n{body}\n}}\n\
         canvas t(uv: coord, time: signal) -> color {{ compose {{ choose(uv.x) }} }}"
    );
    let path = common::unique_temp_path("inline_semantics");
    fs::write(&path, &source).unwrap();
    let output = common::run_fresco(&path);
    fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{source}\n{}",
        common::normalize(&output.stderr)
    );
    let wgsl = String::from_utf8(output.stdout).unwrap();
    let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap();
    module
}

// A deliberately small, strict evaluator for the pure expressions these tests
// emit. Unsupported IR fails loudly instead of substituting a plausible value.
// This avoids requiring a GPU and avoids depending on WGSL names/formatting.
fn eval_cached(
    function: &naga::Function,
    handle: naga::Handle<E>,
    arguments: &[Vec<f64>],
    cache: &std::cell::RefCell<std::collections::HashMap<naga::Handle<E>, Vec<f64>>>,
) -> Vec<f64> {
    if let Some(value) = cache.borrow().get(&handle) {
        return value.clone();
    }
    let value = |h| eval_cached(function, h, arguments, cache);
    let zip = |a: Vec<f64>, b: Vec<f64>, op: fn(f64, f64) -> f64| {
        assert_eq!(a.len(), b.len());
        a.into_iter()
            .zip(b)
            .map(|(a, b)| op(a, b))
            .collect::<Vec<_>>()
    };
    let result = match &function.expressions[handle] {
        E::Literal(L::F32(v)) => vec![f64::from(*v)],
        E::Literal(L::Bool(v)) => vec![if *v { 1.0 } else { 0.0 }],
        E::FunctionArgument(index) => arguments[*index as usize].clone(),
        E::Compose { components, .. } => components.iter().flat_map(|h| value(*h)).collect(),
        E::Splat { size, value: h } => vec![value(*h)[0]; *size as usize],
        E::AccessIndex { base, index } => vec![value(*base)[*index as usize]],
        E::Load { pointer } => value(*pointer),
        E::LocalVariable(local) => value(
            function.local_variables[*local]
                .init
                .expect("local must be initialized or stored before reading"),
        ),
        E::Binary { op, left, right } => zip(
            value(*left),
            value(*right),
            match op {
                B::Add => |a, b| a + b,
                B::Subtract => |a, b| a - b,
                B::Multiply => |a, b| a * b,
                B::Divide => |a, b| a / b,
                B::Greater => |a, b| if a > b { 1.0 } else { 0.0 },
                B::Less => |a, b| if a < b { 1.0 } else { 0.0 },
                B::Equal => |a, b| if a == b { 1.0 } else { 0.0 },
                B::NotEqual => |a, b| if a != b { 1.0 } else { 0.0 },
                B::LessEqual => |a, b| if a <= b { 1.0 } else { 0.0 },
                B::GreaterEqual => |a, b| if a >= b { 1.0 } else { 0.0 },
                B::LogicalAnd => |a, b| if a != 0.0 && b != 0.0 { 1.0 } else { 0.0 },
                B::LogicalOr => |a, b| if a != 0.0 || b != 0.0 { 1.0 } else { 0.0 },
                other => panic!("unsupported binary operator: {other:?}"),
            },
        ),
        E::Math {
            fun: M::Min,
            arg,
            arg1: Some(b),
            ..
        } => zip(value(*arg), value(*b), f64::min),
        E::Math {
            fun: M::Max,
            arg,
            arg1: Some(b),
            ..
        } => zip(value(*arg), value(*b), f64::max),
        E::Math {
            fun: M::Abs, arg, ..
        } => value(*arg).into_iter().map(f64::abs).collect(),
        E::Math {
            fun: M::InverseSqrt,
            arg,
            ..
        } => value(*arg).into_iter().map(|v| 1.0 / v.sqrt()).collect(),
        E::Math {
            fun: M::Dot,
            arg,
            arg1: Some(b),
            ..
        } => vec![zip(value(*arg), value(*b), |a, b| a * b).into_iter().sum()],
        E::Math {
            fun: M::Sign, arg, ..
        } => value(*arg)
            .into_iter()
            .map(|v| {
                if v > 0.0 {
                    1.0
                } else if v < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            })
            .collect(),
        E::Select {
            condition,
            accept,
            reject,
        } => {
            let condition = value(*condition);
            let accept = value(*accept);
            let reject = value(*reject);
            assert_eq!(accept.len(), reject.len());
            accept
                .into_iter()
                .zip(reject)
                .enumerate()
                .map(|(i, (a, b))| {
                    if condition[if condition.len() == 1 { 0 } else { i }] != 0.0 {
                        a
                    } else {
                        b
                    }
                })
                .collect()
        }
        E::Math {
            fun: M::Mix,
            arg,
            arg1: Some(b),
            arg2: Some(t),
            ..
        } => {
            let a = value(*arg);
            let b = value(*b);
            let t = value(*t);
            assert_eq!(a.len(), b.len());
            a.into_iter()
                .zip(b)
                .enumerate()
                .map(|(i, (a, b))| a + (b - a) * t[if t.len() == 1 { 0 } else { i }])
                .collect()
        }
        other => panic!("unsupported expression in semantic test: {other:?}"),
    };
    cache.borrow_mut().insert(handle, result.clone());
    result
}

fn returned(
    module: &naga::Module,
    function: &naga::Function,
    block: &naga::Block,
    arguments: &[Vec<f64>],
    cache: &std::cell::RefCell<std::collections::HashMap<naga::Handle<E>, Vec<f64>>>,
) -> Option<Vec<f64>> {
    for (handle, expression) in function.expressions.iter() {
        if let E::LocalVariable(local) = expression {
            let local = &function.local_variables[*local];
            if local.init.is_none() && !cache.borrow().contains_key(&handle) {
                let width = match module.types[local.ty].inner {
                    naga::TypeInner::Scalar(_) => 1,
                    naga::TypeInner::Vector { size, .. } => size as usize,
                    ref other => panic!("unsupported zero-initialized local: {other:?}"),
                };
                cache.borrow_mut().insert(handle, vec![0.0; width]);
            }
        }
    }
    for statement in block.iter() {
        match statement {
            naga::Statement::Emit(range) => {
                for handle in range.clone() {
                    cache.borrow_mut().remove(&handle);
                }
            }
            naga::Statement::Store { pointer, value } => {
                let E::LocalVariable(local) = function.expressions[*pointer] else {
                    panic!("semantic test supports whole-local stores only");
                };
                let value = eval_cached(function, *value, arguments, cache);
                for (handle, expression) in function.expressions.iter() {
                    if matches!(expression, E::LocalVariable(other) if *other == local) {
                        cache.borrow_mut().insert(handle, value.clone());
                    }
                }
            }
            naga::Statement::Return { value: Some(h) } => {
                return Some(eval_cached(function, *h, arguments, cache));
            }
            naga::Statement::Call {
                function: callee,
                arguments: inputs,
                result: Some(result),
            } => {
                let inputs: Vec<_> = inputs
                    .iter()
                    .map(|h| eval_cached(function, *h, arguments, cache))
                    .collect();
                let callee = &module.functions[*callee];
                let callee_cache = std::cell::RefCell::new(std::collections::HashMap::new());
                let value = returned(module, callee, &callee.body, &inputs, &callee_cache)
                    .expect("numeric helper must return a value");
                cache.borrow_mut().insert(*result, value);
            }
            naga::Statement::If {
                condition,
                accept,
                reject,
            } => {
                let branch = if eval_cached(function, *condition, arguments, cache)[0] != 0.0 {
                    accept
                } else {
                    reject
                };
                if let Some(value) = returned(module, function, branch, arguments, cache) {
                    return Some(value);
                }
            }
            other => panic!("unsupported statement in semantic test: {other:?}"),
        }
    }
    None
}

fn assert_samples(body: &str, samples: &[(f64, [f64; 4])]) {
    assert_samples_with_prelude(body, "", samples);
}

fn assert_samples_with_prelude(body: &str, prelude: &str, samples: &[(f64, [f64; 4])]) {
    let module = compile(body, prelude);
    let (_, function) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("fresco_scene_t"))
        .expect("scene function missing");
    for (x, expected) in samples {
        let cache = std::cell::RefCell::new(std::collections::HashMap::new());
        let actual = returned(&module, function, &function.body, &[vec![*x, 0.5]], &cache)
            .expect("scene must return");
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < 1e-6),
            "x={x}: expected {expected:?}, got {actual:?}\n{body}"
        );
    }
}

const BLUE: [f64; 4] = [0.0, 0.0, 1.0, 1.0];
const RED: [f64; 4] = [1.0, 0.0, 0.0, 1.0];

const NORMALIZATION_PRELUDE: &str =
    include_str!("../../../integrations/example-engine/engine/core/00_prelude.fr");

#[test]
fn inverse_sqrt_aliases_preserve_scalar_and_vector_results() {
    for name in ["inverse_sqrt", "inverseSqrt", "inversesqrt", "rsqrt"] {
        assert_samples(
            &format!(
                "let v = {name}(vec3(x, x * 4.0, x * 16.0))\nreturn rgba({name}(x), v.y, v.z, 1.0)"
            ),
            &[
                (4.0, [0.5, 0.25, 0.125, 1.0]),
                (16.0, [0.25, 0.125, 0.0625, 1.0]),
            ],
        );
    }
}

#[test]
fn normalization_overloads_reject_incompatible_arguments() {
    for expression in [
        "normalize_or(vec2(0.0), vec3(1.0))",
        "normalize_or_zero(1.0)",
    ] {
        let path = common::unique_temp_path("normalization_invalid");
        fs::write(
            &path,
            format!(
                "{NORMALIZATION_PRELUDE}\ncanvas t(uv: coord, time: signal) -> color {{\n\
             let invalid = {expression}\nrgba(1.0, 1.0, 1.0, 1.0)\n}}"
            ),
        )
        .unwrap();
        let output = common::run_fresco(&path);
        fs::remove_file(path).unwrap();
        assert!(!output.status.success(), "must reject {expression}");
        let error = common::normalize(&output.stderr);
        assert!(
            error.contains("normalize_or") && error.contains("overload"),
            "{error}"
        );
    }
}

#[test]
fn normalization_fallbacks_preserve_threshold_and_fallback_values() {
    assert_samples_with_prelude(
        "let v = normalize_or(vec2(x, 0.0), vec2(2.0, 3.0))\nreturn rgba(v.x, v.y, 0.0, 1.0)",
        NORMALIZATION_PRELUDE,
        &[
            (0.0, [2.0, 3.0, 0.0, 1.0]),
            (0.00005, [2.0, 3.0, 0.0, 1.0]),
            (f64::from(0.0001_f32), [2.0, 3.0, 0.0, 1.0]),
            (0.00011, [1.0, 0.0, 0.0, 1.0]),
            (-1.0, [-1.0, 0.0, 0.0, 1.0]),
        ],
    );
    assert_samples_with_prelude(
        "let v = normalize_or_zero(vec2(3.0 * x, 4.0 * x))\nreturn rgba(v.x, v.y, 0.0, 1.0)",
        NORMALIZATION_PRELUDE,
        &[
            (0.0, [0.0, 0.0, 0.0, 1.0]),
            (0.00001, [0.0, 0.0, 0.0, 1.0]),
            (0.00003, [0.6, 0.8, 0.0, 1.0]),
            (1e30, [0.6, 0.8, 0.0, 1.0]),
        ],
    );
}

#[test]
fn normalization_fallbacks_support_three_and_four_lanes() {
    for (body, expected) in [
        (
            "let v = normalize_or_zero(vec3(x, 2.0*x, 2.0*x))\nreturn rgba(v.x, v.y, v.z, 1.0)",
            [1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 1.0],
        ),
        (
            "let v = normalize_or_zero(vec4(x, x, x, x))\nreturn rgba(v.x, v.y, v.w, 1.0)",
            [0.5, 0.5, 0.5, 1.0],
        ),
    ] {
        assert_samples_with_prelude(
            body,
            NORMALIZATION_PRELUDE,
            &[(1.0, expected), (1e30, expected)],
        );
    }
    assert_samples_with_prelude(
        "let a = normalize_or(vec3(x), vec3(2.0, 3.0, 4.0))\nlet b = normalize_or(vec4(x), vec4(5.0))\nreturn rgba(a.x, a.y, a.z + b.w, 1.0)",
        NORMALIZATION_PRELUDE,
        &[(0.0, [2.0, 3.0, 9.0, 1.0])],
    );
}

#[test]
fn normalization_overloads_follow_expression_and_function_result_types() {
    let prelude = format!("{NORMALIZATION_PRELUDE}\nfn direction(v: vec3) -> vec3 {{ return v }}");
    assert_samples_with_prelude(
        "let v = vec3(x, 2.0*x, 2.0*x)\n\
         let unit = normalize_or_zero(direction(v + vec3(0.0)))\n\
         let result = normalize_or_zero(abs(unit))\nreturn rgba(result.x, result.y, result.z, 1.0)",
        &prelude,
        &[
            (0.0, [0.0, 0.0, 0.0, 1.0]),
            (1.0, [1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, 1.0]),
        ],
    );
}

#[test]
fn control_arithmetic_color() {
    assert_samples(
        "return rgb(x * 0.5, 0.0, 1.0 - x)",
        &[(0.0, BLUE), (1.0, [0.5, 0.0, 0.0, 1.0])],
    );
}

#[test]
fn control_both_branches_return() {
    assert_samples(
        "if x > 0.5 { return #ff0000 } else { return #0000ff }",
        &[(0.25, BLUE), (0.5, BLUE), (0.75, RED)],
    );
}

#[test]
fn conditional_early_return_preserves_taken_path() {
    assert_samples(
        "if x > 0.5 { return #ff0000 }\nreturn #0000ff",
        &[(0.75, RED), (0.25, BLUE)],
    );
}

#[test]
fn conditional_assignment_preserves_untaken_path() {
    assert_samples(
        "var c = #0000ff\nif x > 0.5 { c = #ff0000 }\nreturn c",
        &[(0.25, BLUE), (0.75, RED)],
    );
}

#[test]
fn branch_assignments_merge_without_leaking_between_branches() {
    assert_samples(
        "var n = 0.0\nif x > 0.5 { n = 1.0 } else { n += 0.25 }\nreturn rgb(n, 0.0, 0.0)",
        &[(0.25, [0.25, 0.0, 0.0, 1.0]), (0.75, RED)],
    );
}

#[test]
fn conditional_break_preserves_runtime_condition() {
    assert_samples(
        "var n = 0.0\nfor i in 0 .. 4 {\nif x > 0.5 {} else { break }\nn += 1.0\n}\nreturn rgb(n / 4.0, 0.0, 0.0)",
        &[(0.25, [0.0, 0.0, 0.0, 1.0]), (0.75, RED)],
    );
}

#[test]
fn guarded_loop_does_not_truncate_at_64_iterations() {
    assert_samples(
        "var n = 0.0\nfor i in 0 .. 100 {\nif x > 0.5 {} else { break }\nn += 1.0\n}\nreturn rgb(n / 100.0, 0.0, 0.0)",
        &[(0.75, RED)],
    );
}

#[test]
fn control_unguarded_loop_runs_all_iterations() {
    assert_samples(
        "var n = 0.0\nfor i in 0 .. 100 { n += 1.0 }\nreturn rgb(n / 100.0, 0.0, 0.0)",
        &[(0.75, RED)],
    );
}

#[test]
fn nested_returns_preserve_each_path() {
    assert_samples(
        "if x > 0.25 { if x > 0.75 { return #ff0000 }\nreturn #00ff00 }\nreturn #0000ff",
        &[(0.0, BLUE), (0.5, [0.0, 1.0, 0.0, 1.0]), (1.0, RED)],
    );
}

#[test]
fn early_return_merges_with_trailing_expression() {
    assert_samples(
        "if x > 0.5 { return #ff0000 }\nrgb(0.0, 0.0, 1.0)",
        &[(0.25, BLUE), (0.75, RED)],
    );
}

#[test]
fn branch_return_can_mix_with_branch_trailing_expression() {
    for body in [
        "if x > 0.5 { return #ff0000 } else { rgb(0.0, 0.0, 1.0) }",
        "if x > 0.5 { rgb(1.0, 0.0, 0.0) } else { return #0000ff }",
    ] {
        assert_samples(body, &[(0.25, BLUE), (0.75, RED)]);
    }
}

#[test]
fn early_return_captures_state_before_later_assignment() {
    assert_samples(
        "var n = 0.25\nif x > 0.5 { return rgb(n, 0.0, 0.0) }\nn = 1.0\nreturn rgb(n, 0.0, 0.0)",
        &[(0.25, RED), (0.75, [0.25, 0.0, 0.0, 1.0])],
    );
}

#[test]
fn shadowed_branch_local_does_not_modify_outer_local() {
    assert_samples(
        "let n = 0.25\nif x > 0.5 { var n = 1.0\nn += 1.0 }\nreturn rgb(n, 0.0, 0.0)",
        &[(0.25, [0.25, 0.0, 0.0, 1.0]), (0.75, [0.25, 0.0, 0.0, 1.0])],
    );
}

#[test]
fn break_can_depend_on_mutated_loop_state() {
    assert_samples(
        "var n = 0.0\nfor i in 0 .. 4 { if n > x { break }\nn += 0.25 }\nreturn rgb(n, 0.0, 0.0)",
        &[(0.0, [0.25, 0.0, 0.0, 1.0]), (0.5, [0.75, 0.0, 0.0, 1.0])],
    );
}

#[test]
fn nested_loop_consumes_its_own_break() {
    assert_samples(
        "var n = 0.0\nfor i in 0 .. 2 { for j in 0 .. 2 { if x > 0.5 { break }\nn += 0.25 }\nn += 0.25 }\nreturn rgb(n, 0.0, 0.0)",
        &[(0.25, [1.5, 0.0, 0.0, 1.0]), (0.75, [0.5, 0.0, 0.0, 1.0])],
    );
}

#[test]
fn nested_loop_bounds_can_use_compile_time_mutated_state() {
    assert_samples(
        "var count = 1.0\nvar n = 0.0\nfor i in 0 .. 2 { for j in 0 .. count { n += 1.0 }\ncount += 1.0 }\nreturn rgb(n / 3.0, 0.0, 0.0)",
        &[(0.25, RED)],
    );
}

#[test]
fn exhaustive_match_keeps_each_arms_result() {
    assert_samples_with_prelude(
        "var kind = Kind.blue\nif x > 0.5 { kind = Kind.red }\nmatch kind { red: { return #ff0000 }\nblue: { return #0000ff } }",
        "enum Kind { red, blue }",
        &[(0.25, BLUE), (0.75, RED)],
    );
}

#[test]
fn conditional_struct_field_assignment_merges_fields() {
    assert_samples_with_prelude(
        "var value = Sample(r: 0.0, b: 1.0)\nif x > 0.5 { value.r = 1.0\nvalue.b = 0.0 }\nreturn rgb(value.r, 0.0, value.b)",
        "struct Sample { r: f32, b: f32 }",
        &[(0.25, BLUE), (0.75, RED)],
    );
}
