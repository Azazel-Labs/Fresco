fn source() -> String {
    include_str!("fixtures/generated-shells.fr")
        .replace("layers: u32) ->", "layers: u32, clock: f32, red: f32) ->")
        .replace("layers: 3u)", "layers: 3u, clock: frame.time, red: tint.r)")
        .replace(
            "v.world_position +",
            "v.world_position + vec3(clock + red, 0.0, 0.0) +",
        )
        .replace("expansion: f32,", "expansion: f32, clock: f32, tint: vec3,")
        .replace(
            "expansion: expansion, view: frame",
            "expansion: expansion, clock: frame.time, tint: tint.rgb, view: frame",
        )
        .replace("param limit:", "param tint: color = #fff\n    param limit:")
        .replace(
            "select(vec4(0.0, 0.0, 0.1, 0.1), vec4(0.1, 0.0, 0.0, 0.1), geometry.bounds.valid)",
            "vec4(tint * clock, 1.0)",
        )
}

#[test]
fn projected_shader_arguments_keep_their_complete_typed_bindings() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), source());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let compute = manifest
            .gpu_programs
            .iter()
            .find_map(|p| p.compute_invocation.as_ref())
            .unwrap();
        assert!(
            matches!(&compute.arguments["clock"], fresco_artifact::ManifestComputeArgument::External { ty, .. } if ty == "PreviewScene")
        );
        assert!(
            matches!(&compute.arguments["red"], fresco_artifact::ManifestComputeArgument::Setting { ty, name, .. } if ty == "color" && name == "tint")
        );
        let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
        let draw = recipe
            .steps
            .iter()
            .find_map(|s| s.invocation.as_ref())
            .unwrap();
        let host = draw.host.as_ref().unwrap();
        assert!(!host.arguments.contains_key("clock"));
        assert!(!host.arguments.contains_key("tint"));
    }
}

#[test]
fn invalid_projections_and_host_uses_fail_before_gpu_submission() {
    for (from, to, diagnostic) in [
        (
            "clock: frame.time",
            "clock: frame.missing",
            "unknown `PreviewScene` field",
        ),
        (
            "clock: frame.time",
            "clock: frame.res.z",
            "cannot access `z`",
        ),
        (
            "tint: tint.rgb",
            "tint: tint.rgbaaa",
            "cannot access `rgbaaa`",
        ),
        (
            "tint: tint.rgb",
            "tint: tint.r",
            "requires `vec3`, found `f32`",
        ),
        (
            "requires layer < limit",
            "requires clock > 0.0",
            "projected shader values cannot determine host",
        ),
        (
            "geometry.bounds.expand_world(expansion)",
            "geometry.bounds.expand_world(clock)",
            "projected shader values cannot determine host",
        ),
        (
            "checked_mul(geometry.vertex_count, layers)",
            "u32(clock)",
            "projected shader values cannot determine host",
        ),
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe("forward");
        files.insert("main.fr".into(), source().replace(from, to));
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(diagnostic)),
            "{to}: {errors:?}"
        );
    }
}

#[test]
fn graph_constants_keep_integer_types_lexical_scope_and_iteration_values() {
    let source = source().replace(
        "static for layer in 0..3 {",
        "static for ordinal in 0..3 {\nlet layer = 2u - ordinal",
    );
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    files.insert("main.fr".into(), source.clone());
    let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&result.manifest).unwrap();
    let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
    let values: Vec<_> = recipe
        .steps
        .iter()
        .filter_map(|s| s.invocation.as_ref())
        .map(|draw| {
            let fresco_artifact::ManifestComputeArgument::Constant { ty, value } =
                &draw.host.as_ref().unwrap().arguments["layer"]
            else {
                panic!("constant layer")
            };
            assert_eq!(ty, "u32");
            value.as_u64().unwrap()
        })
        .collect();
    assert_eq!(values, [2, 1, 0]);
    for replacement in [
        "let layer = frame.time",
        "let layer = limit",
        "let ordinal = 0u; let layer = 0u",
        "static if false { let layer = 0u }",
    ] {
        files.insert(
            "main.fr".into(),
            source.replace("let layer = 2u - ordinal", replacement),
        );
        assert!(
            fresco::driver::compile_bundle_virtual(&files, "main.fr", false).is_err(),
            "{replacement}"
        );
    }
}
