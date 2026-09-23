use std::{collections::HashMap, fs, process::Command};

use fresco::driver::{CompileBundleOutput, compile_bundle_virtual};
use naga::{Module, ShaderStage, Statement, TypeInner};
use serde_json::Value;

fn files() -> HashMap<String, String> {
    HashMap::from([
        (
            "engine/engine.fr".into(),
            include_str!("../../../tests/canaries/engine.fr").into(),
        ),
        (
            "engine/frame.fr".into(),
            include_str!("../../../tests/canaries/frame.fr").into(),
        ),
        (
            "palette.fr".into(),
            include_str!("../../../tests/canaries/palette.fr").into(),
        ),
        (
            "main.fr".into(),
            include_str!("../../../tests/canaries/main.fr").into(),
        ),
        // Discovery must follow imports, not eagerly parse every engine file.
        (
            "engine/unreachable.fr".into(),
            "deliberately invalid unused source".into(),
        ),
    ])
}

fn compile(files: &HashMap<String, String>) -> (CompileBundleOutput, Module, Value) {
    let output = compile_bundle_virtual(files, "main.fr", true)
        .unwrap_or_else(|errors| panic!("canary compilation failed: {errors:#?}"));
    let module = naga::front::wgsl::parse_str(&output.wgsl)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&output.wgsl)));
    let manifest = serde_json::from_str(&output.manifest).expect("manifest JSON");
    (output, module, manifest)
}

// Shared oracle: mutation tests must defeat these same checks, not a second,
// deliberately weaker set of assertions written specifically for the mutants.
fn contracts(module: &Module, manifest: &Value) -> Result<(), String> {
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(module)
    .map_err(|error| format!("invalid shader: {error:#?}"))?;
    let require = |condition, message: &str| {
        if condition {
            Ok(())
        } else {
            Err(message.to_string())
        }
    };
    let canvas = &manifest["canvases"][0];
    let pass = &canvas["engine_pass"];
    let root_name = format!(
        "fresco_{}",
        canvas["name"].as_str().ok_or("canvas name missing")?
    );
    let (root_handle, root) = module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some(&root_name))
        .ok_or("manifest canvas entry missing from module")?;
    let argument = root
        .arguments
        .first()
        .ok_or("canvas context argument missing")?;
    require(
        matches!(&module.types[argument.ty].inner, TypeInner::Struct { members, .. } if members.len() == 2),
        "canvas must receive the real context struct",
    )?;
    require(
        root.arguments.len() == 5,
        "context plus gain and three array components expected",
    )?;
    require(
        canvas["params"]
            .as_array()
            .is_some_and(|params| params.len() == 2),
        "internal context inputs leaked into controls",
    )?;
    require(
        canvas["params"][0]["name"] == "gain" && canvas["params"][1]["name"] == "levels",
        "authored parameters missing or reordered",
    )?;
    for (key, stage) in [
        ("vertex_entry", ShaderStage::Vertex),
        ("fragment_entry", ShaderStage::Fragment),
    ] {
        let name = pass[key].as_str().ok_or("stage metadata missing")?;
        let entry = module
            .entry_points
            .iter()
            .find(|entry| entry.name == name && entry.stage == stage)
            .ok_or_else(|| format!("manifest {key} has no matching executable stage"))?;
        if stage == ShaderStage::Fragment {
            require(entry.function.body.iter().any(|statement| matches!(statement, Statement::Call { function, .. } if *function == root_handle)), "fragment does not call the canvas implementation")?;
        }
    }
    let uniforms = canvas["global_uniforms"]
        .as_array()
        .ok_or("uniform metadata missing")?;
    require(!uniforms.is_empty(), "frame binding missing")?;
    for uniform in uniforms {
        let group = uniform["group"].as_u64().ok_or("uniform group missing")?;
        let binding = uniform["binding"]
            .as_u64()
            .ok_or("uniform binding missing")?;
        require(
            module.global_variables.iter().any(|(_, global)| {
                global.binding.as_ref().is_some_and(|slot| {
                    u64::from(slot.group) == group && u64::from(slot.binding) == binding
                })
            }),
            "manifest uniform has no shader resource",
        )?;
    }
    Ok(())
}

#[test]
fn semantic_canary_checks_imports_lowering_resources_and_executable_contracts() {
    let (output, module, manifest) = compile(&files());
    contracts(&module, &manifest).expect("end-to-end compiler contracts");
    let mut nested = files()
        .into_iter()
        .map(|(path, source)| {
            let prefix = if path.starts_with("engine/") {
                "project/examples"
            } else {
                "project/examples/sketches"
            };
            (format!("{prefix}/{path}"), source)
        })
        .collect::<HashMap<_, _>>();
    nested.insert(
        "engine/engine.fr".into(),
        "invalid distant engine must not override the nearest engine".into(),
    );
    let nested = compile_bundle_virtual(&nested, "project/examples/sketches/main.fr", false)
        .expect("nested virtual examples discover the nearest engine like filesystem examples");
    let nested_module = naga::front::wgsl::parse_str(&nested.wgsl).expect("nested WGSL");
    let nested_manifest = serde_json::from_str(&nested.manifest).expect("nested manifest");
    contracts(&nested_module, &nested_manifest).expect("nested engine contracts");
    assert!(
        !output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == "error")
    );
    assert!(
        output
            .explain
            .as_ref()
            .is_some_and(|explain| !explain.is_empty())
    );
}

fn without_debug_names(mut module: Module) -> String {
    for (_, function) in module.functions.iter_mut() {
        function.name = None;
        function.named_expressions.clear();
        for argument in &mut function.arguments {
            argument.name = None;
        }
        for (_, local) in function.local_variables.iter_mut() {
            local.name = None;
        }
    }
    format!("{module:#?}")
}

#[test]
fn conditional_derivative_canary_preserves_aa_without_divergent_derivatives() {
    fn inspect(block: &naga::Block, function: &naga::Function, conditional: bool) -> usize {
        let mut derivatives = 0;
        for statement in block {
            match statement {
                Statement::Emit(range) => {
                    for handle in range.clone() {
                        if matches!(
                            function.expressions[handle],
                            naga::Expression::Derivative { .. }
                        ) {
                            assert!(
                                !conditional,
                                "derivative remained inside conditional control flow"
                            );
                            derivatives += 1;
                        }
                    }
                }
                Statement::Block(body) => derivatives += inspect(body, function, conditional),
                Statement::If { accept, reject, .. } => {
                    derivatives += inspect(accept, function, true);
                    derivatives += inspect(reject, function, true);
                }
                Statement::Loop {
                    body, continuing, ..
                } => {
                    derivatives += inspect(body, function, conditional);
                    derivatives += inspect(continuing, function, conditional);
                }
                Statement::Switch { cases, .. } => {
                    for case in cases {
                        derivatives += inspect(&case.body, function, true);
                    }
                }
                _ => {}
            }
        }
        derivatives
    }

    let example = include_str!("../../../examples/90) gallery/perspective_flip_card.fr");
    for source in [
        example.to_string(),
        example.replace("cos(flip) >= 0.0", "context(coord).x >= 0.5"),
    ] {
        let mut sources = files();
        sources.insert("main.fr".into(), source);
        let (_, module, _) = compile(&sources);
        let scene = module
            .functions
            .iter()
            .find(|(_, function)| {
                function.name.as_deref() == Some("fresco_scene_perspective_flip_card")
            })
            .expect("scene function")
            .1;
        assert!(
            inspect(&scene.body, scene, false) > 0,
            "AA derivatives were removed instead of scheduled safely"
        );
        assert!(
            scene
                .expressions
                .iter()
                .any(|(_, expression)| matches!(expression, naga::Expression::Select { .. })),
            "face selection missing"
        );
    }

    let mut sources = files();
    sources.insert("main.fr".into(), "canvas plain(ctx: CanvasContext) -> color { compose { if context(coord).x > 0.5 { fill(#ff0000) } else { fill(#0000ff) } } }".into());
    let (_, module, _) = compile(&sources);
    let scene = module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some("fresco_scene_plain"))
        .expect("plain scene")
        .1;
    assert!(
        scene
            .body
            .iter()
            .any(|statement| matches!(statement, Statement::If { .. })),
        "derivative-free branches should remain conditional"
    );
}

#[test]
fn rate_canary_checks_units_context_division_and_invalid_spellings() {
    use fresco::lexer::{Token, lex_spanned};

    let mut sources = files();
    let program = |expression: &str| {
        format!(
            "canvas rate_canary(ctx: CanvasContext) -> color {{\nlet interval = 2s\nlet value = {expression}\ncompose {{ fill(rgba(value, 0.0, 0.0, 1.0)) }}\n}}"
        )
    };
    for unit in [
        "", "px", "uv", "vw", "vh", "vmin", "vmax", "deg", "turn", "s", "ms",
    ] {
        for (denominator, factor) in [("s", 1.0), ("ms", 1000.0)] {
            let literal = format!("2{unit}/{denominator}");
            let tokens = lex_spanned(&literal);
            assert!(
                matches!(tokens.as_slice(), [(Token::Rate(_), span)] if *span == (0..literal.len()))
            );
            sources.insert("main.fr".into(), program(&literal));
            let (_, actual, _) = compile(&sources);
            sources.insert(
                "main.fr".into(),
                program(&format!("{}{} * context(time)", 2.0 * factor, unit)),
            );
            let (_, expected, _) = compile(&sources);
            assert_eq!(
                without_debug_names(actual),
                without_debug_names(expected),
                "{literal}"
            );
        }
    }
    // Seconds remain ordinary numeric values once assigned, not dimensioned types.
    sources.insert("main.fr".into(), program("20deg / interval"));
    compile(&sources);
    sources.insert("main.fr".into(), program("-20deg/s / 2"));
    compile(&sources);

    // The literal follows semantic roles, including a nested replacement clock.
    let scoped = |expression: &str| {
        format!(
            "canvas rate_canary(ctx: CanvasContext) -> color {{\nin context FrameGlobals(time: 3.0, delta_time: 0.0, resolution: (320.0, 200.0)) {{\ncompose {{ fill(rgba({expression}, 0.0, 0.0, 1.0)) }}\n}}\n}}"
        )
    };
    sources.insert("main.fr".into(), scoped("2/s"));
    let (_, actual, _) = compile(&sources);
    sources.insert("main.fr".into(), scoped("2 * context(time)"));
    let (_, expected, _) = compile(&sources);
    assert_eq!(without_debug_names(actual), without_debug_names(expected));

    for invalid in [
        "20deg /s",
        "20deg/ s",
        "20deg / s",
        "20deg / 1s",
        "20deg / 1ms",
        "20deg / ms",
        "20deg/second",
        "20deg/ms_extra",
    ] {
        sources.insert("main.fr".into(), program(invalid));
        assert!(
            compile_bundle_virtual(&sources, "main.fr", false).is_err(),
            "accepted {invalid}"
        );
    }
    for name in ["s", "ms"] {
        sources.insert(
            "main.fr".into(),
            program("0.0").replace("let interval =", &format!("let {name} =")),
        );
        assert!(
            compile_bundle_virtual(&sources, "main.fr", false).is_err(),
            "reserved name {name} accepted"
        );
    }
    sources.insert("main.fr".into(), program("1px/s"));
    sources.get_mut("engine/frame.fr").unwrap().replace_range(
        ..,
        include_str!("../../../tests/canaries/frame.fr")
            .replace("@semantic(time)", "")
            .as_str(),
    );
    let errors =
        compile_bundle_virtual(&sources, "main.fr", false).expect_err("missing clock accepted");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("time") && error.message.contains("not available")),
        "{errors:#?}"
    );
}

#[test]
fn equivalence_canary_checks_formatting_renaming_and_repeat_compilation() {
    let original = files();
    let (_, baseline, manifest) = compile(&original);
    contracts(&baseline, &manifest).expect("baseline contracts");
    let baseline = without_debug_names(baseline);
    let (_, repeated, _) = compile(&original);
    assert_eq!(
        baseline,
        without_debug_names(repeated),
        "repeat compilation changed semantics"
    );

    let path = std::env::temp_dir().join(format!("fresco-canary-format-{}.fr", std::process::id()));
    let mut formatted = original.clone();
    for (name, source) in &original {
        if name.ends_with("unreachable.fr") {
            continue;
        }
        fs::write(&path, source).expect("write formatter input");
        let output = Command::new(env!("CARGO_BIN_EXE_fresco"))
            .args(["fmt"])
            .arg(&path)
            .arg("--stdout")
            .output()
            .expect("run formatter");
        assert!(
            output.status.success(),
            "formatter: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).expect("formatter UTF-8");
        fs::write(&path, &text).expect("write second formatter input");
        let twice = Command::new(env!("CARGO_BIN_EXE_fresco"))
            .args(["fmt"])
            .arg(&path)
            .arg("--stdout")
            .output()
            .expect("repeat formatter");
        assert!(twice.status.success());
        assert_eq!(
            text.as_bytes(),
            twice.stdout,
            "formatter is not idempotent for {name}"
        );
        formatted.insert(name.clone(), text);
    }
    fs::remove_file(path).expect("remove temporary formatter input");
    let (_, module, other_manifest) = compile(&formatted);
    contracts(&module, &other_manifest).expect("formatted contracts");
    assert_eq!(
        baseline,
        without_debug_names(module),
        "formatting changed program meaning"
    );

    for source in formatted.values_mut() {
        *source = source.replace("accent", "renamed_accent");
    }
    let (_, renamed, renamed_manifest) = compile(&formatted);
    contracts(&renamed, &renamed_manifest).expect("renamed contracts");
    assert_eq!(
        baseline,
        without_debug_names(renamed),
        "helper renaming changed program meaning"
    );
}

#[test]
fn diagnostic_canary_checks_import_type_name_and_context_failures() {
    // Each broken program starts from the same compiling, multi-feature control.
    compile(&files());
    for (file, from, to, expected) in [
        ("main.fr", "\"palette.fr\"", "\"missing.fr\"", "missing.fr"),
        (
            "main.fr",
            "accent(levels[2])",
            "unknown_canary_fn(levels[2])",
            "unknown_canary_fn",
        ),
        (
            "main.fr",
            "ctx: CanvasContext",
            "ctx: vec2",
            "CanvasContext",
        ),
        (
            "engine/frame.fr",
            "@semantic(delta_time)",
            "@semantic(time)",
            "duplicate context semantic",
        ),
    ] {
        let mut broken = files();
        let source = broken.get_mut(file).expect("fixture file");
        assert!(source.contains(from), "mutation anchor disappeared: {from}");
        *source = source.replace(from, to);
        let errors = compile_bundle_virtual(&broken, "main.fr", false)
            .expect_err("invalid program accepted");
        let matching = errors
            .iter()
            .find(|error| error.severity == "error" && error.message.contains(expected))
            .unwrap_or_else(|| panic!("missing {expected:?} diagnostic: {errors:#?}"));
        assert!(
            !matching.file.is_empty(),
            "diagnostic lost source attribution"
        );
        assert!(
            matching.span_start <= matching.span_end,
            "inverted diagnostic span"
        );
    }
}

#[test]
fn mutation_canary_proves_contract_checks_reject_corrupted_compiler_output() {
    let (_, module, manifest) = compile(&files());
    contracts(&module, &manifest).expect("positive control");
    let mut missing_stage = manifest.clone();
    missing_stage["canvases"][0]["engine_pass"]["fragment_entry"] =
        Value::String("nonexistent".into());
    assert!(
        contracts(&module, &missing_stage).is_err(),
        "surviving mutant: wrong stage metadata"
    );
    let mut missing_binding = manifest.clone();
    missing_binding["canvases"][0]["global_uniforms"][0]["binding"] = Value::from(999);
    assert!(
        contracts(&module, &missing_binding).is_err(),
        "surviving mutant: wrong resource binding"
    );
    let mut leaked_control = manifest.clone();
    leaked_control["canvases"][0]["params"]
        .as_array_mut()
        .expect("params")
        .push(serde_json::json!({"name": "hidden_context"}));
    assert!(
        contracts(&module, &leaked_control).is_err(),
        "surviving mutant: exposed context input"
    );
    let mut missing_fragment = module.clone();
    missing_fragment
        .entry_points
        .retain(|entry| entry.stage != ShaderStage::Fragment);
    assert!(
        contracts(&missing_fragment, &manifest).is_err(),
        "surviving mutant: missing executable fragment"
    );
    let mut wrong_abi = module;
    let scalar = wrong_abi
        .types
        .iter()
        .find(|(_, ty)| matches!(ty.inner, TypeInner::Scalar(_)))
        .expect("scalar type")
        .0;
    let root = wrong_abi
        .functions
        .iter_mut()
        .find(|(_, function)| function.name.as_deref() == Some("fresco_canary"))
        .expect("canvas entry")
        .1;
    root.arguments[0].ty = scalar;
    assert!(
        contracts(&wrong_abi, &manifest).is_err(),
        "surviving mutant: scalar in place of context"
    );
}
