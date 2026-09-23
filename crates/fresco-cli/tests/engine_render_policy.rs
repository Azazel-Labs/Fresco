use std::collections::HashMap;

use fresco::driver::{
    CompileContext, ContextFile, compile_bundle_virtual, compile_bundle_virtual_with_context,
};

const POLICY: &str = "#pragma check.shape_aa_min_px = 1.5\n\
#pragma check.shape_aa_max_px = 3.0\n\
#pragma check.shape_aa_style = gradient\n\
#pragma check.projective_footprint_max_px = 64.0\n";

fn files() -> HashMap<String, String> {
    HashMap::from([
        ("engine/engine.fr".into(), POLICY.into()),
        (
            "main.fr".into(),
            r#"
canvas probe(uv: coord) -> color {
    in space scale(0.2) {
        star(at: (0.5, 0.5), outer: 0.2, inner: 0.1, points: 5) |> fill(#ffffff)
    }
}
"#
            .into(),
        ),
    ])
}

#[test]
fn each_engine_setting_is_required_with_an_actionable_suggestion() {
    for line in POLICY.lines() {
        let key = line.split_whitespace().nth(1).unwrap();
        let mut sources = files();
        sources.insert("engine/engine.fr".into(), POLICY.replace(line, ""));
        let errors = compile_bundle_virtual(&sources, "main.fr", false).unwrap_err();
        let missing = errors
            .iter()
            .find(|diag| diag.message.contains(key))
            .unwrap();
        assert!(missing.message.contains("missing required engine setting"));
        assert!(
            missing
                .help
                .as_ref()
                .is_some_and(|help| help.contains(line))
        );
    }
}

#[test]
fn entry_pragmas_and_api_values_cannot_replace_engine_declarations() {
    let mut sources = files();
    sources.remove("engine/engine.fr");
    let source = sources.get_mut("main.fr").unwrap();
    source.insert_str(0, POLICY);
    let mut context = CompileContext::default();
    assert!(context.check.shape_aa_min_px.is_none());
    assert!(context.check.shape_aa_max_px.is_none());
    assert!(context.check.shape_aa_style.is_none());
    assert!(context.check.projective_footprint_max_px.is_none());
    ContextFile::from_json_str(
        r#"{"check":{"shape_aa_min_px":1.5,"shape_aa_max_px":3.0,"shape_aa_style":"gradient"}}"#,
    )
    .unwrap()
    .apply_to(&mut context);
    context.check.projective_footprint_max_px = Some(64.0);
    let errors =
        compile_bundle_virtual_with_context(&sources, "main.fr", false, &context).unwrap_err();
    assert_eq!(
        errors
            .iter()
            .filter(|diag| diag.message.contains("missing required engine setting"))
            .count(),
        4
    );
}

#[test]
fn imported_engine_policy_changes_lowering() {
    let mut sources = files();
    sources.insert("engine/engine.fr".into(), "import \"policy.fr\"".into());
    sources.insert("engine/policy.fr".into(), POLICY.into());
    let initial = compile_bundle_virtual(&sources, "main.fr", false).unwrap();
    for (before, after) in [
        ("1.5", "0.875"),
        ("3.0", "4.25"),
        ("64.0", "17.0"),
        ("gradient", "fwidth"),
    ] {
        sources.insert("engine/policy.fr".into(), POLICY.replace(before, after));
        let changed = compile_bundle_virtual(&sources, "main.fr", false).unwrap();
        assert_ne!(
            initial.wgsl, changed.wgsl,
            "engine setting {before} did not reach lowering"
        );
    }
}

#[test]
fn invalid_and_duplicate_engine_settings_are_rejected() {
    for (before, after) in [
        ("1.5", "0.0"),
        ("3.0", "1.0"),
        ("64.0", "0.0"),
        ("gradient", "potato"),
    ] {
        let mut sources = files();
        sources.insert("engine/engine.fr".into(), POLICY.replace(before, after));
        let errors = compile_bundle_virtual(&sources, "main.fr", false).unwrap_err();
        assert!(errors.iter().any(|diag| diag.file == "engine/engine.fr"));
    }
    let mut sources = files();
    sources
        .get_mut("engine/engine.fr")
        .unwrap()
        .push_str("\nimport \"duplicate.fr\"\n");
    sources.insert(
        "engine/duplicate.fr".into(),
        "#pragma check.shape_aa_min_px = 2.0".into(),
    );
    let errors = compile_bundle_virtual(&sources, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|diag| diag.message.contains("duplicate engine setting"))
    );
}

#[test]
fn ordinary_imports_cannot_supply_engine_policy() {
    let mut sources = files();
    sources.remove("engine/engine.fr");
    sources.insert("policy.fr".into(), POLICY.into());
    sources
        .get_mut("main.fr")
        .unwrap()
        .insert_str(0, "import \"policy.fr\"\n");
    let errors = compile_bundle_virtual(&sources, "main.fr", false).unwrap_err();
    assert_eq!(
        errors
            .iter()
            .filter(|diag| diag.message.contains("missing required engine setting"))
            .count(),
        4
    );
}

#[test]
fn helper_only_modules_do_not_require_rendering_policy() {
    let sources = HashMap::from([(
        "main.fr".into(),
        "fn twice(x: f32) -> f32 { return x * 2.0; }".into(),
    )]);
    compile_bundle_virtual(&sources, "main.fr", false).unwrap();
}

#[test]
fn split_policy_is_validated_after_all_engine_imports() {
    let mut sources = files();
    sources.insert(
        "engine/engine.fr".into(),
        "import \"minimum.fr\"\nimport \"rest.fr\"".into(),
    );
    sources.insert(
        "engine/minimum.fr".into(),
        "#pragma check.shape_aa_min_px = 6.0".into(),
    );
    sources.insert("engine/rest.fr".into(), "#pragma check.shape_aa_max_px = 8.0\n#pragma check.shape_aa_style = gradient\n#pragma check.projective_footprint_max_px = 64.0".into());
    compile_bundle_virtual(&sources, "main.fr", false).unwrap();
}

#[test]
fn surface_entries_also_require_an_engine_policy() {
    let sources = HashMap::from([(
        "main.fr".into(),
        "surface probe(sp: surf) -> material { compose { base(albedo: #ffffff) } }".into(),
    )]);
    let errors = compile_bundle_virtual(&sources, "main.fr", false).unwrap_err();
    assert_eq!(
        errors
            .iter()
            .filter(|diag| diag.message.contains("missing required engine setting"))
            .count(),
        4
    );
}

#[test]
fn compiling_the_engine_entry_itself_retains_its_policy() {
    let mut sources = files();
    sources
        .get_mut("engine/engine.fr")
        .unwrap()
        .push_str(MATERIAL_SCHEMA);
    sources
        .get_mut("engine/engine.fr")
        .unwrap()
        .push_str("\nsurface probe(sp: surf) -> material { compose { base(albedo: #ffffff) } }");
    compile_bundle_virtual(&sources, "engine/engine.fr", false).unwrap();
}

#[test]
fn compiling_an_engine_entry_does_not_hide_import_cycles() {
    let mut sources = files();
    sources
        .get_mut("engine/engine.fr")
        .unwrap()
        .push_str("\nimport \"engine.fr\"\n");
    let errors = compile_bundle_virtual(&sources, "engine/engine.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|diag| diag.message.contains("cyclic import"))
    );
}

#[test]
fn compiling_a_policy_module_reached_by_engine_imports_retains_ownership() {
    let mut sources = files();
    sources.insert(
        "engine/engine.fr".into(),
        "import \"policy.fr\"\nimport \"material.fr\"".into(),
    );
    sources.insert("engine/policy.fr".into(), POLICY.into());
    sources.insert(
        "engine/material.fr".into(),
        format!("{MATERIAL_SCHEMA}\nsurface probe(sp: surf) -> material {{ compose {{ base(albedo: #ffffff) }} }}"),
    );
    compile_bundle_virtual(&sources, "engine/policy.fr", false).unwrap();
}

const MATERIAL_SCHEMA: &str = r#"
struct surf { @semantic(coord) uv: vec2 }
@context(surf, sp)
@composition(base, material, mask)
@default_material
material_properties test_material { channel albedo: color = #000 }
"#;
