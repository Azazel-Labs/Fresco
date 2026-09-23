#[test]
fn complete_meadow_fur_compiles_and_preserves_compute_and_draw_dependencies() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert(
            "main.fr".into(),
            include_str!("../../../examples/40) surface shaders/style_sample_fur.fr").into(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|errors| panic!("{renderer}: {errors:?}"));
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let computes: Vec<_> = manifest
            .gpu_programs
            .iter()
            .filter_map(|p| p.compute_invocation.as_ref())
            .collect();
        assert_eq!(computes.len(), 4);
        for compute in &computes {
            assert!(
                compute.dependencies.is_empty(),
                "density and shell generation are independent"
            );
            assert!(
                compute.engine_dependencies.is_empty(),
                "compute must not wait for opaque completion"
            );
        }
        let selected = manifest.renderers.iter().find(|r| r.selected).unwrap();
        let shells: Vec<_> = selected
            .steps
            .iter()
            .filter_map(|s| s.invocation.as_ref())
            .filter(|i| i.operation == "FurShell")
            .collect();
        assert_eq!(shells.len(), 32);
        assert_eq!(
            shells
                .iter()
                .filter(|i| i.material == "chestnut_fur")
                .count(),
            12
        );
        assert_eq!(
            shells.iter().filter(|i| i.material == "pale_fur").count(),
            20
        );
        for shell in shells {
            assert_eq!(shell.point, "StandardStyle::transparent");
            assert!(shell.generated_vertices.is_some());
            assert!(shell.bounds.is_some());
            assert!(shell.compute_inputs.contains_key("vertices"));
            assert!(shell.compute_inputs.contains_key("density"));
        }
        for surface in &manifest.surfaces {
            assert!(
                surface
                    .mesh_passes
                    .iter()
                    .any(|p| !p.shading_inputs.is_empty()),
                "density must feed root shading"
            );
        }
    }
}

#[test]
fn mixed_styles_preserve_material_parameters_and_pass_local_binding_scopes() {
    let source = format!(
        "{}\n{}\n{}",
        include_str!("../../../examples/40) surface shaders/style_sample.fr"),
        include_str!("../../../examples/40) surface shaders/style_sample_fur.fr"),
        include_str!("fixtures/mixed-style-materials.fr")
    );
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), source.clone());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|errors| panic!("{renderer}: {errors:?}"));
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let toon = manifest
            .surfaces
            .iter()
            .find(|surface| surface.name == "style_sample")
            .unwrap();
        let tint = toon
            .params
            .iter()
            .find(|param| param.name == "tint")
            .unwrap();
        assert_eq!(tint.ty, "color");
        if renderer == "deferred" {
            assert!(
                toon.mesh_passes.iter().any(|pass| {
                    pass.procedural
                        && pass.bindings.iter().any(|binding| {
                            binding.group_index == Some(tint.group)
                                && binding.binding == Some(tint.binding)
                                && binding.signature.as_deref() == Some("texture_2d<f32>")
                        })
                }),
                "fixture must exercise a material uniform and a distinct procedural pass sharing a slot"
            );
        }
        let recipe = manifest
            .renderers
            .iter()
            .find(|recipe| recipe.selected)
            .unwrap();
        let operations: Vec<_> = recipe
            .steps
            .iter()
            .filter_map(|step| step.invocation.as_ref())
            .collect();
        assert_eq!(
            operations
                .iter()
                .filter(|invocation| invocation.operation == "FurShell")
                .count(),
            32
        );
        assert_eq!(
            operations
                .iter()
                .filter(|invocation| invocation.operation == "InvertedHull")
                .count(),
            2
        );
        for name in ["chestnut_fur", "pale_fur", "blue_toon", "plain", "glass"] {
            assert!(manifest.surfaces.iter().any(|surface| surface.name == name));
        }
    }
}

#[test]
fn meadow_fur_rejects_unsupported_materials_and_shell_counts() {
    let source = include_str!("../../../examples/40) surface shaders/style_sample_fur.fr");
    for (from, to, expected) in [
        (
            "properties { style:",
            "properties { blend: SurfaceBlend.Translucent; style:",
            "precondition is not satisfied",
        ),
        (
            "properties { style:",
            "properties { two_sided: true; style:",
            "precondition is not satisfied",
        ),
        (
            "MeadowFur(layers: 12u",
            "MeadowFur(layers: 0u",
            "outside its declared range",
        ),
        (
            "MeadowFur(layers: 12u",
            "MeadowFur(layers: 33u",
            "outside its declared range",
        ),
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe("forward");
        assert!(source.contains(from));
        files.insert("main.fr".into(), source.replace(from, to));
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{to}: {errors:?}"
        );
    }
}

#[test]
fn unused_pass_binding_cannot_shadow_a_live_material_parameter() {
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    files.insert("main.fr".into(), "surface item(sp: surf) -> material(standard) { param tint: color = #f5ad69; compose { base(albedo: tint) } }".into());
    let factory = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    assert!(factory.contains("pass preview_mesh for base {"));
    *factory = factory.replace(
        "pass preview_mesh for base {",
        "pass preview_mesh for base { binding { @group(0) @binding(0) unused: uniform<vec4> }",
    );
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("shadows an external shader resource")),
        "{errors:?}"
    );
}
