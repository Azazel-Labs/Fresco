use std::collections::HashMap;
fn files(source: &str) -> HashMap<String, String> {
    let mut result = fresco_example_engine::source_files();
    result.insert("main.fr".into(), source.into());
    result
}
fn source(properties: &str) -> String {
    format!(
        "surface leaf(sp: surf) -> material(unlit) {{ {properties}\n compose {{ base(albedo: rgba(1.0, 0.2, 0.1, 0.25)) }} }}"
    )
}
fn compile(files: &HashMap<String, String>) -> fresco::driver::CompileBundleOutput {
    let output = fresco::driver::compile_bundle_virtual(files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    output
}
#[test]
fn surface_properties_resolve_render_state_and_source_ranges() {
    let authored =
        source("properties { blend: SurfaceBlend.Masked\n two_sided: true\n mask_cutoff: 0.3 }");
    let output = compile(&files(&authored));
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let surface = &manifest["surfaces"][0];
    let shared: fresco_artifact::ManifestSurfaceSettings =
        serde_json::from_value(surface["settings"].clone()).expect("shared surface settings");
    // Exercise the wire serializer: to_value widens f32 directly to f64 and
    // does not use JSON's shortest-round-tripping f32 number formatting.
    let round_trip: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&shared).unwrap()).unwrap();
    assert_eq!(round_trip, surface["settings"]);
    assert_eq!(
        surface["settings"]["pass_states"]["preview_mesh"]["blend"],
        "replace"
    );
    assert_eq!(
        surface["settings"]["pass_states"]["preview_mesh"]["cull"],
        "none"
    );
    assert_eq!(surface["material_ty"], "unlit");
    assert!(output.wgsl.contains("discard"));
    assert!(output.wgsl.contains("front_facing"));
    let property = surface["settings"]["properties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "two_sided")
        .unwrap();
    let start = property["value_span"]["start"].as_u64().unwrap() as usize;
    let end = property["value_span"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&authored[start..end], "true");
}
#[test]
fn engine_owns_property_vocabulary_and_profiles() {
    let mut inputs = files(&source(
        "options { coverage: SurfaceBlend.Translucent\n profile: SurfaceProfile.Lit }",
    ));
    let schema = inputs
        .get_mut("engine/core/07_surface_properties.fr")
        .unwrap();
    *schema = schema
        .replace(
            "@surface_properties(properties)",
            "@surface_properties(options)",
        )
        .replace(" blend:", " coverage:");
    let emitter = inputs
        .get_mut("engine/core/06_particle_contract.fr")
        .unwrap();
    *emitter = emitter.replace("@surface_defaults(blend:", "@surface_defaults(coverage:");
    for path in [
        "engine/core/05_mesh_contract.fr",
        "engine/particles/02_camera_billboard.fr",
        "engine/config/renderer.fr",
    ] {
        let renderer = inputs.get_mut(path).unwrap();
        *renderer = renderer
            .replace("blend ==", "coverage ==")
            .replace("blend !=", "coverage !=");
    }
    let output = compile(&inputs);
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        manifest["surfaces"][0]["settings"]["pass_states"]["preview_mesh"]["blend"],
        "premultiplied"
    );
    assert_eq!(manifest["surfaces"][0]["material_ty"], "standard");
    assert!(!output.wgsl.contains("discard"));
}
#[test]
fn invalid_properties_and_unsupported_usages_fail_closed() {
    for (properties, expected) in [
        ("properties { nonsense: true }", "unknown surface property"),
        (
            "properties { two_sided: true\n two_sided: false }",
            "duplicate surface property",
        ),
        ("properties { mask_cutoff: 2.0 }", "0..1"),
        (
            "properties { used_with_skinning: true }",
            "unsupported vertex factory",
        ),
        (
            "options { two_sided: true }",
            "unknown surface property block",
        ),
    ] {
        let errors =
            fresco::driver::compile_bundle_virtual(&files(&source(properties)), "main.fr", false)
                .unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{errors:?}"
        );
    }
}
#[test]
fn usage_flags_emit_additional_real_factory_variants() {
    let mut inputs = files(&source("properties { used_with_wind: true }"));
    let mesh = inputs.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    let start = mesh.find("vertex_factory preview_static").unwrap();
    // Copy only the factory, independent of declarations/attributes that follow it.
    let end = start + mesh[start..].find("\n}\n").unwrap() + "\n}\n".len();
    let factory = mesh[start..end].replace("preview_static", "preview_wind");
    mesh.push_str(&factory);
    let schema = inputs
        .get_mut("engine/core/07_surface_properties.fr")
        .unwrap();
    *schema = schema.replace("interface SurfaceOptions {", "interface SurfaceOptions {\nparam @config(editor) @usage(preview_wind) used_with_wind: bool = false");
    let output = compile(&inputs);
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let variants = manifest["surfaces"][0]["mesh_passes"][0]["variants"]
        .as_array()
        .unwrap();
    assert_eq!(variants.len(), 2);
    assert_ne!(
        variants[0]["entries"][0]["entry"],
        variants[1]["entries"][0]["entry"]
    );
    for variant in variants {
        assert!(output.wgsl.contains(&format!(
            "fn {}(",
            variant["entries"][0]["entry"].as_str().unwrap()
        )));
    }
}

#[test]
fn custom_static_fields_specialize_shader_code_without_uniforms() {
    let mut inputs = files(
        "surface leaf(sp: surf) -> material(unlit) { compose { base(albedo: feature ? rgba(1.0,0.0,0.0,1.0) : rgba(0.0,1.0,0.0,1.0)) } }",
    );
    let schema = inputs
        .get_mut("engine/core/07_surface_properties.fr")
        .unwrap();
    *schema = schema.replace(
        "interface SurfaceOptions {",
        "interface SurfaceOptions {\nparam @config(editor) @permutation feature: bool = false",
    );
    let disabled = compile(&inputs);
    let source = inputs.get_mut("main.fr").unwrap();
    *source = source.replace("{ compose", "{ properties { feature: true }\n compose");
    let enabled = compile(&inputs);
    assert_ne!(disabled.wgsl, enabled.wgsl);
    let manifest: serde_json::Value = serde_json::from_str(&enabled.manifest).unwrap();
    // Empty runtime parameter lists are omitted from the manifest.
    assert!(manifest["surfaces"][0].get("params").is_none());
}

#[test]
fn particles_and_meshes_call_the_selected_evaluation_shader() {
    let sample = include_str!("../../../examples/50) particles/drifting_sparks.fr");
    let mut inputs = files(&sample.replace("material(unlit)", "material(standard)"));
    // This fixture explicitly selects schema evaluation instead of the engine's
    // standard PBR overload, exercising the custom response extension.
    let mesh = inputs.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    *mesh = mesh.replace(
        "fn shade(sp: PreviewVarying, m: standard)",
        "fn unused_standard_response(sp: PreviewVarying, m: standard)",
    );
    let output = compile(&inputs);
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let surface = &manifest["surfaces"][0];
    let helper = surface["evaluation_shader_entry"].as_str().unwrap();
    // The helper must be invoked from both executable raster paths, not just emitted.
    assert!(output.wgsl.matches(&format!("{helper}(")).count() >= 3);
    assert!(
        manifest["techniques"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["surface"] == surface["name"] && t["metadata"]["engine"] == "particle")
    );
    assert!(output.wgsl.contains("normal = vec3<f32>(context.normal"));
}

#[test]
fn engine_can_rename_material_channels_profiles_and_context() {
    let mut inputs = files(&source("properties { profile: SurfaceProfile.Lit }"));
    for text in inputs.values_mut() {
        *text = text
            .replace("albedo", "pigment_tint")
            .replace("emissive", "glow")
            .replace("standard", "thermal")
            .replace("unlit", "flat")
            .replace("struct surf {", "struct SampleContext {")
            .replace(": surf", ": SampleContext")
            .replace("-> surf", "-> SampleContext")
            .replace("surf(", "SampleContext(")
            .replace("@context(surf, sp)", "@context(SampleContext, sp)");
    }
    let output = compile(&inputs);
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let surface = &manifest["surfaces"][0];
    assert_eq!(surface["material_properties"], "thermal");
    assert_eq!(
        surface["surface_requirements"]["context_type"],
        "SampleContext"
    );
    assert!(output.wgsl.contains("pigment_tint: vec4<f32>"));
    assert!(output.wgsl.contains("glow: vec3<f32>"));
    assert!(!output.wgsl.contains("albedo"));
    assert!(!output.wgsl.contains("emissive"));
}

#[test]
fn raster_lighting_requires_explicit_renderer_inputs_and_valid_variants() {
    let sample = include_str!("../../../examples/50) particles/drifting_sparks.fr");
    let mut inputs = files(&sample.replace("material(unlit)", "material(standard)"));
    let core = inputs.get_mut("engine/core/01_core.fr").unwrap();
    *core = core.replace(
        "material_properties standard",
        "@evaluator(visibility_response)\nmaterial_properties standard",
    );
    let mesh = inputs.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    mesh.truncate(mesh.find("schema_program preview_lit").unwrap());
    mesh.push_str(
        r#"
schema_evaluator visibility_response for standard {
    contract { inputs: visibility: f32 }
    shade: (albedo.r * visibility, albedo.g * visibility, albedo.b * visibility, albedo.a * opacity)
}
"#,
    );
    let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap_err();
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("missing evaluation input `visibility`")),
        "{errors:?}"
    );
    for path in [
        "engine/core/05_mesh_contract.fr",
        "engine/particles/02_camera_billboard.fr",
        "engine/config/renderer.fr",
    ] {
        let hook = inputs.get_mut(path).unwrap();
        *hook = hook.replace("), m)", "), m, visibility: 0.25)");
    }
    let output = compile(&inputs);
    assert!(output.wgsl.contains("0.25, m)"));
    let hook = inputs
        .get_mut("engine/particles/02_camera_billboard.fr")
        .unwrap();
    *hook = hook.replace(
        "visibility: 0.25",
        "visibility: 0.25, variant: \"missing_variant\"",
    );
    let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("unknown evaluation variant")),
        "{errors:?}"
    );
}

#[test]
fn lit_profile_without_an_authored_response_is_an_error() {
    let mut inputs = files(&source("properties { profile: SurfaceProfile.Lit }"));
    let mesh = inputs.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    // Select the schema response explicitly before removing its implementation.
    *mesh = mesh.replace(
        "fn shade(sp: PreviewVarying, m: standard)",
        "fn unused_standard_response(sp: PreviewVarying, m: standard)",
    );
    mesh.truncate(mesh.find("schema_program preview_lit").unwrap());
    let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("no executable response")),
        "{errors:?}"
    );
}

#[test]
fn source_properties_select_renderer_evaluation_permutations() {
    let mut inputs = files(&source(
        "properties { profile: SurfaceProfile.Lit\n receive_shadows: true }",
    ));
    let schema = inputs
        .get_mut("engine/core/07_surface_properties.fr")
        .unwrap();
    *schema = schema.replace("interface SurfaceOptions {", "interface SurfaceOptions {\nparam @config(editor) @evaluation_axis(shadow, off, on) receive_shadows: bool = false");
    let core = inputs.get_mut("engine/core/01_core.fr").unwrap();
    *core = core.replace(
        "material_properties standard",
        "@evaluator(visibility_response)\nmaterial_properties standard",
    );
    let mesh = inputs.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    *mesh = mesh.replace(
        "fn shade(sp: PreviewVarying, m: standard)",
        "fn unused_standard_response(sp: PreviewVarying, m: standard)",
    );
    mesh.truncate(mesh.find("schema_program preview_lit").unwrap());
    mesh.push_str(
        r#"
schema_evaluator visibility_response for standard {
    contract { inputs: visibility: f32 }
    permutations { shadow: off|on }
    specialize as "preview_shadow_${shadow}"
    variant off when shadow == off: (albedo.r, albedo.g, albedo.b, albedo.a)
    variant on when shadow == on: (albedo.r * visibility, albedo.g * visibility, albedo.b * visibility, albedo.a)
}
"#,
    );
    *mesh = mesh.replace("), m)", "), m, visibility: 0.25)");
    let output = compile(&inputs);
    assert!(output.wgsl.contains("return preview_shadow_on(surf(sp.uv"));
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        manifest["surfaces"][0]["settings"]["evaluation_axes"]["shadow"],
        "on"
    );
    let authored = inputs.get_mut("main.fr").unwrap();
    *authored = authored.replace("receive_shadows: true", "receive_shadows: false");
    assert!(
        compile(&inputs)
            .wgsl
            .contains("return preview_shadow_off(surf(sp.uv")
    );
    let schema = inputs
        .get_mut("engine/core/07_surface_properties.fr")
        .unwrap();
    *schema = schema.replace("shadow, off, on", "shadow, unavailable, on");
    let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("unsupported evaluation permutation")),
        "{errors:?}"
    );
}

#[test]
fn entry_contract_surface_defaults_are_overridable_and_keep_source_spans_absent() {
    let emitter = include_str!("../../../examples/50) particles/drifting_sparks.fr");
    for (properties, expected) in [
        ("", "premultiplied"),
        ("properties { blend: SurfaceBlend.Opaque }", "replace"),
        ("properties { blend: SurfaceBlend.Masked }", "replace"),
    ] {
        let authored = emitter.replace(
            "    let radius",
            &format!("    {properties}\n    let radius"),
        );
        let output = compile(&files(&authored));
        let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
        let settings = &manifest["surfaces"][0]["settings"];
        assert_eq!(settings["pass_states"]["preview_mesh"]["blend"], expected);
        let blend = settings["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "blend")
            .unwrap();
        if properties.is_empty() {
            assert!(
                blend["value_span"].is_null(),
                "engine default must not pretend to be authored source"
            );
        } else {
            assert!(blend["value_span"].is_object());
        }
    }
    let output = compile(&files(&source("")));
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        manifest["surfaces"][0]["settings"]["pass_states"]["preview_mesh"]["blend"],
        "replace"
    );
}

#[test]
fn entry_surface_defaults_follow_engine_vocabulary() {
    let authored = include_str!("../../../examples/50) particles/drifting_sparks.fr")
        .replace("emitter drifting", "sparks drifting");
    let mut inputs = files(&authored);
    let contract = inputs
        .get_mut("engine/core/06_particle_contract.fr")
        .unwrap();
    *contract = contract
        .replace("@entry(emitter, update)", "@entry(sparks, update)")
        .replace(
            "blend: SurfaceBlend.Translucent",
            "coverage: SurfaceBlend.Masked",
        );
    let schema = inputs
        .get_mut("engine/core/07_surface_properties.fr")
        .unwrap();
    *schema = schema.replace(" blend:", " coverage:");
    for path in [
        "engine/core/05_mesh_contract.fr",
        "engine/particles/02_camera_billboard.fr",
        "engine/config/renderer.fr",
    ] {
        let renderer = inputs.get_mut(path).unwrap();
        *renderer = renderer
            .replace("blend ==", "coverage ==")
            .replace("blend !=", "coverage !=");
    }
    let output = compile(&inputs);
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        manifest["surfaces"][0]["settings"]["pass_states"]["preview_mesh"]["blend"],
        "replace"
    );
}

#[test]
fn invalid_entry_surface_defaults_report_errors() {
    for (defaults, expected) in [
        ("missing: true", "unknown surface default property"),
        (
            "blend: SurfaceBlend.Opaque, blend: SurfaceBlend.Masked",
            "duplicate surface default",
        ),
        ("blend: 42", "invalid enum value"),
    ] {
        let mut inputs = files(include_str!(
            "../../../examples/50) particles/drifting_sparks.fr"
        ));
        let contract = inputs
            .get_mut("engine/core/06_particle_contract.fr")
            .unwrap();
        *contract = contract.replace("blend: SurfaceBlend.Translucent", defaults);
        let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
            .expect_err("invalid default must fail");
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{errors:?}"
        );
    }
}

#[test]
fn competing_entry_surface_defaults_are_diagnosed() {
    let authored = format!(
        "{}\nother_system alternate {{}}",
        include_str!("../../../examples/50) particles/drifting_sparks.fr")
    );
    let mut inputs = files(&authored);
    inputs
        .get_mut("engine/core/06_particle_contract.fr")
        .unwrap()
        .push_str(
            r#"
@entry(other_system, update)
@surface_defaults(blend: SurfaceBlend.Opaque)
interface OtherSystem {
    fn update() -> f32
}
"#,
        );
    let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect_err("conflicting entry defaults must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("ambiguous surface default `blend`")),
        "{errors:?}"
    );
}

#[test]
fn schema_evaluator_require_pruning_reports_actionable_diagnostic() {
    let source = r#"import "../core/01_core.fr"
import "../core/02_functions.fr"

pass pruned_forward for standard {
    stage: raster
    draw:  per_object
    blend: opaque

    permutations {
        @known(compile) ambient:      "flat"|"sh2"
    }

    require {
        ambient == "flat" => disable(ambient)
        ambient != "flat" => disable(ambient)
    }

    binding {
        @group(frame)                                                   camera:        uniform<CameraData>
        @group(frame)                                                   fog_data:      uniform<FogData>
        @group(frame) @expect(ambient == "flat")                        ambient_color: uniform<vec3>
        @group(draw)  @expect(ambient == "sh2")                         ambient_sh:    uniform<SH2>
    }

    fn shade(sp: Surf, m: standard) -> color {
        return rgba(m.albedo.rgb, 1.0)
    }
}

pipeline(lighting) forward for standard {
    pruned_forward
}
"#;
    let mut inputs = fresco_example_engine::source_files();
    let entry = "engine/pipelines/pruned_forward_require_diag.fr";
    inputs.insert(entry.into(), source.into());
    let diagnostics = fresco::driver::compile_bundle_virtual(&inputs, entry, false)
        .expect_err("expected pruning all variants to fail");
    let output = diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{} {}",
                diagnostic.message,
                diagnostic.help.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.contains("no surviving permutation variants")
            && output.contains("matching `disable(...)` clause pruned the last one"),
        "expected actionable pruning diagnostic:\n{output}"
    );
}

#[test]
fn engine_hooks_consume_typed_schema_records_through_ordinary_functions() {
    let mut engine = include_str!("fixtures/tiny-engine.fr").replace(
        "return Pixels(sample(sp).tint, u32(37))",
        "return consume(evaluate_schema(sp, sample(sp), identity: u32(37)))",
    ).replace(
        "@fragment fn pack_pixels",
        "fn consume(value: Report) -> Pixels { return Pixels(value.paint * value.weights[1].x * value.matrix[1][0], value.identity) }\n    @fragment fn pack_pixels",
    );
    engine.push_str(r#"
struct Report { paint: vec4, identity: u32, weights: array<vec2, 2>, matrix: mat2 }
schema_program transfer for Pigment {
    fn apply(identity: u32) -> Report { return Report(paint: self.tint, identity: identity, weights: [vec2(1.0), vec2(2.0)], matrix: mat2(vec2(1.0, 0.0), vec2(1.0, 1.0))) }
    output: apply
}
"#);
    let inputs = HashMap::from([
        ("engine/engine.fr".into(), engine),
        ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: #ffffff) } }".into()),
    ]);
    let output = compile(&inputs);
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        manifest["surfaces"][0]["evaluation_variants"][0]["result_type"],
        "Report"
    );
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    let (_, entry) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("fresco_evaluation_shader_sample"))
        .unwrap();
    let result = entry.result.as_ref().unwrap().ty;
    assert!(
        module
            .functions
            .iter()
            .any(|(_, f)| f.arguments.iter().any(|a| a.ty == result)),
        "the engine helper must receive the same record type"
    );
}

#[test]
fn emitter_defaults_do_not_change_imported_scene_materials() {
    let mut inputs = fresco_example_engine::preview_source_files();
    inputs.insert(
        "main.fr".into(),
        include_str!("../../../examples/50) particles/drifting_sparks.fr").into(),
    );
    let output = compile(&inputs);
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let surfaces = manifest["surfaces"].as_array().unwrap();
    for (name, blend, writes_depth) in [
        ("drifting_sparks", "premultiplied", false),
        ("fresco_scene_ground", "replace", true),
    ] {
        let surface = surfaces.iter().find(|s| s["name"] == name).unwrap();
        let state = &surface["settings"]["pass_states"]["preview_mesh"];
        assert_eq!(state["blend"], blend, "{name}");
        assert_eq!(state["depth_write"], writes_depth, "{name}");
    }
}
