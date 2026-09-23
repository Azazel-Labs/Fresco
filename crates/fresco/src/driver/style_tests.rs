use super::{compile_bundle_virtual, tests::executable_mesh_files};
use std::collections::HashMap;

fn files() -> HashMap<String, String> {
    let mut files = executable_mesh_files(
        "1.0",
        "finish_response(style, apply_response(style, m.albedo))",
    );
    files.insert(
        "engine/render_policy.fr".into(),
        crate::test_support::POLICY.into(),
    );
    let engine = files.get_mut("engine/mesh.fr").unwrap();
    *engine = engine.replace("  fn shade(sp:", "  @dispatch(Response, apply) fn apply_response(id: u32, value: vec4) -> vec4 { return vec4(0.0) }\n  @dispatch(Response, finish) fn finish_response(id: u32, value: vec4) -> vec4 { return vec4(0.0) }\n  fn shade(sp:");
    *engine = engine.replace(
        "  fn shade(sp:",
        "  fn contract_finish(value: vec4) -> vec4 { return value * 0.125 }\n  fn shade(sp:",
    );
    engine.push_str(
        r#"
contract Response for base {
    fn apply(value: vec4) -> vec4
    fn finish(value: vec4) -> vec4 { return contract_finish(value) }
}
fn contract_finish(value: vec4) -> vec4 { return value * 0.75 }
@surface_properties(properties) interface Options {
    param @config(editor) style: implementation<Response> = Plain
}
"#,
    );
    files.insert(
        "plain.fr".into(),
        r#"
style Plain for base : Response {
    fn apply(value: vec4) -> vec4 { return value }
}
"#
        .into(),
    );
    files.insert(
        "half.fr".into(),
        r#"
style Half for base : Response {
    fn apply(value: vec4) -> vec4 { return value * 0.5 }
    fn finish(value: vec4) -> vec4 { return value * 0.25 }
}
"#
        .into(),
    );
    let surface = files["main.fr"].clone();
    files.insert(
        "main.fr".into(),
        format!(
            "import \"plain.fr\"\nimport \"half.fr\"\n{}\n{}",
            surface,
            surface
                .replace("surface sample", "surface second")
                .replace("  compose", "  properties { style: Half }\n  compose")
        ),
    );
    files
}

fn compile(files: &HashMap<String, String>) -> (naga::Module, fresco_artifact::ManifestRoot) {
    let output = compile_bundle_virtual(files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap();
    (module, serde_json::from_str(&output.manifest).unwrap())
}

fn assert_error(files: &HashMap<String, String>, expected: &str) {
    let error = compile_bundle_virtual(files, "main.fr", false).unwrap_err();
    let text = format!("{error:?}");
    assert!(text.contains(expected), "expected {expected:?}: {text}");
}

#[test]
fn draw_dispatch_checks_missing_selection_only_on_reachable_calls() {
    let mut input = files();
    let engine = input.get_mut("engine/mesh.fr").unwrap();
    *engine = engine
        .replace(
            "@dispatch(Response, apply)",
            "@dispatch(Other, apply, unused_settings, draw)",
        )
        .replace(
            "fn apply_response(id: u32,",
            "fn apply_response(id: u32, offset: u32,",
        )
        .replace("apply_response(style,", "apply_response(style, 0u,");
    engine.push_str("\ninterface Other { fn apply(value: vec4) -> vec4 }\n");
    assert_error(
        &input,
        "draw-scoped dispatch requires a selected `Other` implementation",
    );
    let engine = input.get_mut("engine/mesh.fr").unwrap();
    *engine = engine.replace("apply_response(style, 0u, m.albedo)", "m.albedo");
    compile(&input);
}

#[test]
fn unused_shading_slot_layouts_are_checked_when_all_hooks_are_inherited() {
    let mut input = files();
    let engine = input.get_mut("engine/mesh.fr").unwrap();
    *engine = engine.replace(
        "    fn apply(value: vec4) -> vec4\n",
        "    fn apply(value: vec4) -> vec4 { return value }\n",
    );
    input.get_mut("plain.fr").unwrap().push_str(
        r#"
style Unselected for base : Response {
    shading_input unused: buffer<array<vec4, 2>, read> scope draw
}
"#,
    );
    compile(&input);
    let invalid = input.get_mut("plain.fr").unwrap();
    *invalid = invalid.replace(
        "buffer<array<vec4, 2>, read>",
        "buffer<UnknownElement, read>",
    );
    assert_error(&input, "unsupported executable mesh field type");
}

#[test]
fn unused_shading_slots_typecheck_hook_bodies_and_explicit_helpers() {
    let mut input = files();
    input.get_mut("plain.fr").unwrap().push_str(
        r#"
fn slot_value(values: buffer<vec4, read>, index: u32) -> vec4 { return values[index] }
style Unselected for base : Response {
    shading_input palette: buffer<vec4, read> scope draw
    shading_input density: texture2d<r32float, read> scope draw
    fn apply(value: vec4) -> vec4 {
        return slot_value(palette, 0u) * density.load(ivec2(0)) * value
    }
}
"#,
    );
    compile(&input);
    for (from, to, expected) in [
        (
            "slot_value(palette, 0u)",
            "slot_value(vec4(1.0), 0u)",
            "resource argument",
        ),
        (
            "density.load(ivec2(0))",
            "density.store(ivec2(0), vec4(1.0))",
            "unsupported resource method",
        ),
        (
            "return values[index]",
            "return palette[index]",
            "implicit shader resource capture",
        ),
    ] {
        let mut invalid = input.clone();
        let source = invalid.get_mut("plain.fr").unwrap();
        *source = source.replace(from, to);
        assert_error(&invalid, expected);
    }
}

#[test]
fn shading_hook_effects_reject_derivatives_through_helpers_and_unused_styles() {
    for expression in [
        "ddx(value.x)",
        "ddy(value.y)",
        "fwidth(value.z)",
        "hidden(value.x)",
    ] {
        let mut input = files();
        input.get_mut("plain.fr").unwrap().push_str(&format!(
            r#"
fn hidden(value: f32) -> f32 {{ return ddx(value) }}
style Unselected for base : Response {{
    fn apply(value: vec4) -> vec4 {{ return vec4({expression}) }}
}}
"#
        ));
        assert_error(&input, "dynamically dispatched shading hooks cannot use");
    }
    let mut input = files();
    let engine = input.get_mut("engine/mesh.fr").unwrap();
    *engine = engine.replace("return contract_finish(value)", "return vec4(ddx(value.x))");
    assert_error(&input, "dynamically dispatched shading hooks cannot use");
}

#[test]
fn imported_styles_dispatch_defaults_and_overrides_with_stable_selection() {
    let mut files = files();
    let (module, manifest) = compile(&files);
    let selections = |manifest: &fresco_artifact::ManifestRoot| {
        manifest
            .surfaces
            .iter()
            .map(|s| {
                let selected = &s.settings.as_ref().unwrap().implementations[0];
                (
                    s.name.clone(),
                    selected.symbol.clone(),
                    selected.id,
                    selected.available.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let original = selections(&manifest);
    assert_eq!(original.len(), 2);
    assert_eq!(original[0].1, "Plain");
    assert_eq!(original[1].1, "Half");
    assert_ne!(original[0].2, original[1].2);
    assert_eq!(original[0].3, ["Half", "Plain"]);
    let function = |name: &str| {
        module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_ref().is_some_and(|n| n.contains(name)))
            .unwrap()
            .1
    };
    let plain_finish = function("fresco_implementation_Plain_finish");
    assert!(plain_finish.body.iter().any(|s| matches!(s, naga::Statement::Call { function, .. }
        if module.functions[*function].name.as_ref().is_some_and(|n| n.contains("contract_finish")))));
    let library_finish = plain_finish
        .body
        .iter()
        .find_map(|s| match s {
            naga::Statement::Call { function, .. } => Some(&module.functions[*function]),
            _ => None,
        })
        .unwrap();
    assert!(
        library_finish.expressions.iter().any(
            |(_, e)| matches!(e, naga::Expression::Literal(naga::Literal::F32(v)) if *v == 0.75)
        ),
        "contract default must use its library helper, not the pass-local namesake"
    );
    let half_finish = function("fresco_implementation_Half_finish");
    assert!(
        !half_finish
            .body
            .iter()
            .any(|s| matches!(s, naga::Statement::Call { .. }))
    );
    assert!(
        half_finish.expressions.iter().any(
            |(_, e)| matches!(e, naga::Expression::Literal(naga::Literal::F32(v)) if *v == 0.25)
        )
    );
    assert!(
        function("fresco_implementation_Half_apply")
            .expressions
            .iter()
            .any(
                |(_, e)| matches!(e, naga::Expression::Literal(naga::Literal::F32(v)) if *v == 0.5)
            )
    );
    let root = files.get_mut("main.fr").unwrap();
    *root = root.replace(
        "import \"plain.fr\"\nimport \"half.fr\"",
        "import \"half.fr\"\nimport \"plain.fr\"",
    );
    assert_eq!(selections(&compile(&files).1), original);
}

#[test]
fn style_contracts_reject_invalid_declarations_and_defaults() {
    let original = files();
    for (from, to, message) in [
        ("for base", "for missing", "material schema `missing`"),
        (": Response", ": Missing", "unknown shading contract"),
        (
            "fn apply(value: vec4) -> vec4 { return value }",
            "",
            "missing required hook `apply`",
        ),
        ("fn apply", "fn unknown", "unknown style hook"),
        ("value: vec4", "value: vec3", "exact contract signature"),
        ("return value }", "return vec3(0.0) }", "return"),
    ] {
        let mut files = original.clone();
        let style = files.get_mut("plain.fr").unwrap();
        assert!(style.contains(from));
        *style = style.replace(from, to);
        assert_error(&files, message);
    }
    let mut files = original.clone();
    files
        .get_mut("engine/mesh.fr")
        .unwrap()
        .push_str("\nmaterial_properties other { channel albedo: color }\n");
    *files.get_mut("plain.fr").unwrap() = original["plain.fr"].replace("for base", "for other");
    assert_error(&files, "but contract `Response` requires `base`");

    // Defaults must be checked even when every style overrides them.
    let mut files = original.clone();
    *files.get_mut("plain.fr").unwrap() = "style Plain for base : Response { fn apply(value: vec4) -> vec4 { return value }\nfn finish(value: vec4) -> vec4 { return value } }".into();
    *files.get_mut("engine/mesh.fr").unwrap() = original["engine/mesh.fr"]
        .replace("return contract_finish(value)", "return missing_capture");
    assert_error(&files, "missing_capture");
}

#[test]
fn style_selection_checks_schema_even_without_property_schema_annotation() {
    let mut files = files();
    files
        .get_mut("engine/mesh.fr")
        .unwrap()
        .push_str("\nmaterial_properties other { channel albedo: color }\n");
    *files.get_mut("main.fr").unwrap() = "import \"plain.fr\"\nimport \"half.fr\"\nsurface sample(sp: surf) -> material(other) { properties { style: Plain }\n compose { other(albedo: #ffffff) } }".into();
    assert_error(&files, "requires material schema `base`");
}

#[test]
fn style_graph_scopes_and_resource_declarations_are_checked() {
    for member in ["static if true {}", "for self {}"] {
        let mut sources = files();
        let plain = sources.get_mut("plain.fr").unwrap();
        *plain = plain.replace(
            "style Plain for base : Response {",
            &format!("style Plain for base : Response {{ {member};"),
        );
        compile(&sources);
    }
    let mut sources = files();
    let plain = sources.get_mut("plain.fr").unwrap();
    *plain = plain.replace(
        "style Plain for base : Response {",
        "style Plain for base : Response { at missing as target {}; ",
    );
    assert_error(&sources, "at requires for self");
    for (member, diagnostic) in [
        (
            "input view: View",
            "unknown or unsupported contract resource type",
        ),
        ("optional capability Geometry", "unknown capability"),
        (
            "point after_opaque: Target {}",
            "unknown integration target",
        ),
    ] {
        let mut files = files();
        files
            .get_mut("engine/mesh.fr")
            .unwrap()
            .push_str(&format!("\ncontract Extra for base {{ {member} }}"));
        assert_error(&files, diagnostic);
    }
}

#[test]
fn style_preconditions_are_typed_checked_unused_and_evaluated_per_material() {
    let mut sources = files();
    let engine = sources.get_mut("engine/mesh.fr").unwrap();
    *engine = engine.replace(
        "interface Options {",
        "interface Options { param enabled: bool = true\n",
    );
    let plain = sources.get_mut("plain.fr").unwrap();
    *plain = plain.replace(
        "style Plain for base : Response {",
        "style Plain for base : Response { requires material.enabled == true\n",
    );
    compile(&sources);
    let mut rejected = sources.clone();
    let engine = rejected.get_mut("engine/mesh.fr").unwrap();
    *engine = engine.replace("enabled: bool = true", "enabled: bool = false");
    assert_error(&rejected, "precondition is not satisfied");
    let plain = sources.get_mut("plain.fr").unwrap();
    *plain = plain.replace("material.enabled == true", "material.missing == true");
    assert_error(&sources, "missing");
}

#[test]
fn optional_capability_becomes_required_when_a_selected_style_requests_it() {
    let mut sources = files();
    let engine = sources.get_mut("engine/mesh.fr").unwrap();
    engine.push_str("\ncapability CameraData { scene: vec4 }\n");
    *engine = engine.replace(
        "contract Response for base {",
        "contract Response for base { optional capability CameraData\n",
    );
    // Selecting a graph contract requires a renderer provider, including when all
    // capabilities are optional: absence must not masquerade as a valid provider.
    assert_error(&sources, "requires a `Response` provider");
    let plain = sources.get_mut("plain.fr").unwrap();
    *plain = plain.replace(
        "style Plain for base : Response {",
        "style Plain for base : Response { requires MissingCapability\n",
    );
    assert_error(&sources, "unknown style requirement");
}

#[test]
fn unused_styles_and_duplicate_declarations_are_checked_in_their_source_file() {
    let original = files();
    for (file, addition, message) in [
        (
            "plain.fr",
            "style Unused for base : Response {}",
            "missing required hook",
        ),
        (
            "plain.fr",
            "style Plain for base : Response {}",
            "duplicate style name",
        ),
        (
            "plain.fr",
            "style Unused for base : Response { fn apply(v: vec4) -> vec4 { return v }\nfn apply(v: vec4) -> vec4 { return v } }",
            "duplicate style hook",
        ),
        (
            "engine/mesh.fr",
            "contract Response for base {}",
            "duplicate shading contract name",
        ),
        (
            "engine/mesh.fr",
            "contract Extra for base { fn a() -> f32\nfn a() -> f32 }",
            "duplicate contract hook",
        ),
        (
            "engine/mesh.fr",
            "contract Extra for base { fn a() -> f32 { return missing_default } }",
            "missing_default",
        ),
        (
            "engine/mesh.fr",
            "contract Extra for base { fn a(v: MissingType) -> f32 }",
            "MissingType",
        ),
        (
            "engine/mesh.fr",
            "contract Extra for base { fn a(v: f32, v: f32) -> f32 }",
            "duplicate contract hook parameter",
        ),
    ] {
        let mut files = original.clone();
        files
            .get_mut(file)
            .unwrap()
            .push_str(&format!("\n{addition}\n"));
        let errors = compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains(message) && e.file == file),
            "{errors:?}"
        );
    }
}

#[test]
fn config_can_select_styles_and_unused_implementations_are_not_emitted() {
    let mut files = files();
    files.insert(
        "fresco.config.json".into(),
        serde_json::json!({
            "property_overrides": {"sample": {"style": "Half"}}
        })
        .to_string(),
    );
    let (module, manifest) = compile(&files);
    assert!(
        manifest
            .surfaces
            .iter()
            .all(|s| s.settings.as_ref().unwrap().implementations[0].symbol == "Half")
    );
    assert!(!module.functions.iter().any(|(_, f)| {
        f.name
            .as_ref()
            .is_some_and(|n| n.contains("fresco_implementation_Plain"))
    }));
}

#[test]
fn styles_accept_materials_extending_the_contract_schema() {
    let mut files = files();
    files
        .get_mut("engine/mesh.fr")
        .unwrap()
        .push_str("\nmaterial_properties derived extends base { channel extra: f32 = 0.0 }\n");
    let root = files.get_mut("main.fr").unwrap();
    *root = root.replace("material(base)", "material(derived)");
    let (_, manifest) = compile(&files);
    assert_eq!(manifest.surfaces.len(), 2);
    assert!(
        manifest
            .surfaces
            .iter()
            .all(|s| !s.settings.as_ref().unwrap().implementations.is_empty())
    );
}

#[test]
fn static_settings_specialize_dispatch_without_runtime_storage() {
    let mut files = files();
    files.insert("plain.fr".into(), "style Plain for base : Response { static param gain: f32 in [0.0, 1.0] = 0.25\nstatic param enabled: bool = true\nstatic param seed: u32 = u32(4294967295)\nfn apply(value: vec4) -> vec4 { if enabled { return value * gain + vec4(f32(seed & u32(1))) } return value } }".into());
    let main = files["main.fr"].replace("style: Half", "style: Plain(gain: 0.75)");
    files.insert(
        "main.fr".into(),
        format!(
            "{}\n{}",
            main,
            main.split("surface second")
                .nth(1)
                .map(|body| format!("surface third{body}"))
                .unwrap()
        ),
    );
    let (_, manifest) = compile(&files);
    let select = |name: &str| {
        manifest
            .surfaces
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .settings
            .as_ref()
            .unwrap()
            .implementations[0]
            .clone()
    };
    let first = select("sample");
    let second = select("second");
    let third = select("third");
    assert_ne!(first.id, second.id);
    assert_eq!(second.id, third.id);
    assert!(first.parameters.is_empty());
    assert_eq!(
        first.static_parameters[2].default,
        serde_json::json!(u32::MAX)
    );
    assert_eq!(first.static_parameters[1].default, serde_json::json!(true));
    assert_eq!(second.static_parameters[0].default, serde_json::json!(0.75));
    let source = files["main.fr"].clone();
    files.insert("main.fr".into(), source.replace("gain: 0.75", "gain: 2.0"));
    assert_error(&files, "outside its declared range");
}

#[test]
fn integer_setting_ranges_are_exact_and_boolean_ranges_are_rejected() {
    let original = files();
    for (parameter, expected) in [
        (
            "param seed: u32 in [16777217, 16777218] = u32(16777216)",
            "outside its declared range",
        ),
        (
            "static param enabled: bool in [0, 1] = true",
            "boolean settings cannot have numeric ranges",
        ),
        ("static param seed: u32 = 1.5", "u32"),
        ("param seed: u32 = -1", "u32"),
        ("param enabled: bool = 1.0", "bool"),
    ] {
        let mut files = original.clone();
        files.insert("plain.fr".into(), format!("style Plain for base : Response {{ {parameter}\nfn apply(value: vec4) -> vec4 {{ return value }} }}"));
        assert_error(&files, expected);
    }
}
