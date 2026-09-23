use fresco_example_engine::{ENTRYPOINT, SOURCES, source_files};

#[test]
fn embedded_profile_is_complete_and_each_module_compiles() {
    let files = source_files();
    assert_eq!(files.len(), SOURCES.len(), "duplicate virtual source paths");
    assert!(files.contains_key(ENTRYPOINT));
    assert!(SOURCES.windows(2).all(|pair| pair[0].path < pair[1].path));
    for source in SOURCES {
        assert!(source.path.starts_with("engine/"));
        fresco::driver::compile_bundle_virtual(&files, source.path, false)
            .unwrap_or_else(|errors| panic!("{}: {errors:#?}", source.path));
    }
}

#[test]
fn embedded_profile_compiles_a_canvas_without_filesystem_discovery() {
    let mut files = source_files();
    files.insert(
        "main.fr".into(),
        "canvas probe(ctx: CanvasContext) -> color { rgba(ctx.uv.x, ctx.uv.y, 0.0, 1.0) }".into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .expect("embedded canvas contract");
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(manifest["canvases"][0]["name"], "probe");
    assert!(output.wgsl.contains("@vertex"));
    assert!(output.wgsl.contains("@fragment"));
    assert!(!output.wgsl.contains("pbr_direct_response"));
}

#[test]
fn complete_artifacts_round_trip_through_shared_runtime_schema() {
    use fresco_artifact::ManifestRoot;
    use serde_json::Value;

    let sources = [
        "canvas probe(ctx: CanvasContext) -> color { param gain: f32 = 0.1 in 0.1 .. 0.9; rgba(gain, ctx.uv.y, 0.0, 1.0) }",
        include_str!("../examples/material_parameters.fr"),
        include_str!("../../../examples/50) particles/drifting_sparks.fr"),
    ];
    for source in sources {
        let mut files = source_files();
        files.insert("main.fr".into(), source.into());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .expect("example artifact compiles");
        let emitted: Value = serde_json::from_str(&output.manifest).unwrap();
        let shared: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
        let encoded = serde_json::to_string(&shared).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), emitted);
        assert!(shared.build_profile.is_some());
        for canvas in &shared.canvases {
            assert!(canvas.engine_pass.is_some());
            let record = &emitted["canvases"][0];
            for absent in ["textures", "sampler", "path_buffers", "storage_params"] {
                assert!(record.get(absent).is_none(), "omit unused {absent}");
            }
            assert_eq!(canvas.params[0].default, serde_json::json!(0.1));
            assert_eq!(canvas.params[0].min, Some(0.1));
        }
        for surface in &shared.surfaces {
            assert!(surface.surface_shader.is_some());
            assert!(surface.contract_requirements.is_some());
            assert!(
                !surface.mesh_passes.is_empty()
                    || fresco_example_engine::runtime::particle_contract::uses_particles(
                        &shared,
                        &surface.name
                    )
            );
        }
    }
}

#[test]
fn factory_group_symbols_can_be_renamed_without_changing_executable_layouts() {
    let mut files = source_files();
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let compile = |files: &std::collections::HashMap<String, String>| {
        fresco::driver::compile_bundle_virtual(files, "main.fr", false).expect("declared groups")
    };
    let before = compile(&files);
    for source in files.values_mut() {
        *source = source
            .replace("group draw", "group object_inputs")
            .replace("@group(draw)", "@group(object_inputs)");
    }
    let after = compile(&files);
    assert_eq!(
        before.wgsl, after.wgsl,
        "group spelling must not affect shader layout"
    );
    let before: fresco_artifact::ManifestRoot = serde_json::from_str(&before.manifest).unwrap();
    let after: fresco_artifact::ManifestRoot = serde_json::from_str(&after.manifest).unwrap();
    for (before, after) in before.vertex_factories.iter().zip(&after.vertex_factories) {
        for (a, b) in before.bindings.iter().zip(&after.bindings) {
            assert_eq!((a.group_index, a.binding), (b.group_index, b.binding));
            if a.group.as_deref() == Some("draw") {
                assert_eq!(b.group.as_deref(), Some("object_inputs"));
            }
        }
    }
}

#[test]
fn explicit_raster_functions_and_outputs_are_name_independent() {
    let mut files = source_files();
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let code = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    *code = code
        .replace("fn vertex(", "fn project_object(")
        .replace("prepare_fragment(", "orient_sample(")
        .replace("material_context(", "make_sample(")
        .replace("finish_fragment(", "apply_coverage(")
        .replace("evaluate_surface(", "sample_layers(")
        .replace("pack_pixels(", "encode_channels(")
        .replace("fn geometry(", "fn store_channels(")
        .replace("fn raster(", "fn paint(")
        .replace("@location(3) identity", "@location(6) identity");
    let recipe = files.get_mut("engine/config/renderer.fr").unwrap();
    *recipe = recipe
        .replace("@draw(vertex, raster,", "@draw(project_object, paint,")
        .replace(
            "@draw(vertex, geometry,",
            "@draw(project_object, store_channels,",
        )
        .replace("@color(3, identity)", "@color(6, identity)");
    *recipe = recipe
        .replace("preview_mesh.vertex.uv", "preview_mesh.project_object.uv")
        .replace(
            "opaque.geometry.surface_uv",
            "opaque.store_channels.surface_uv",
        );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let stages = &manifest.surfaces[0].mesh_passes.first().unwrap().entries;
    assert!(
        stages
            .iter()
            .any(|e| e.function == "project_object" && e.stage == "vertex")
    );
    let outputs = &stages
        .iter()
        .find(|e| e.function == "store_channels")
        .unwrap()
        .outputs;
    assert_eq!(
        outputs.iter().find(|o| o.ty == "uvec2").unwrap().location,
        6
    );
    assert!(stages.iter().any(|e| e.function == "paint"));
    let code = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    *code = code.replace("@location(6) identity", "@location(0) identity");
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("duplicate output location"))
    );
}

#[test]
fn a_fourth_renderer_is_selected_from_engine_metadata_and_unknown_ids_fail() {
    let mut files = fresco_example_engine::source_files_for_recipe("studio");
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    files
        .get_mut("engine/config/renderer.fr")
        .unwrap()
        .push_str(
            r#"
        @renderer("studio", "Studio light")
        @configure(preview_forward_plus, true) @configure(preview_deferred, false)
        @external(output, presentation) @external(depth, depth)
        @external(environment_input, lighting_environment) @external(frame, scene) @external(lamps, point_lights) @buffer(bins, 2097152)
        pipeline(postprocess) studio_recipe {
            @dispatch(cull, 16, 16) @bind(scene, frame) @bind(lights, lamps) @bind(tiles, bins) tile_culling
            @after(tile_culling) @draw(vertex, raster, mesh) @color(0, output) @depth(depth)
            @bind(environment, environment_input) @bind(scene, frame) @bind(point_lights, lamps) @bind(light_tiles, bins) preview_mesh
        }
    "#,
        );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let mut manifest: fresco_artifact::ManifestRoot =
        serde_json::from_str(&output.manifest).unwrap();
    let selected: Vec<_> = manifest.renderers.iter().filter(|r| r.selected).collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        (
            selected[0].id.as_str(),
            selected[0].label.as_str(),
            selected[0].pipeline.as_str()
        ),
        ("studio", "Studio light", "studio_recipe")
    );
    assert_eq!(manifest.renderers.len(), 4);
    let settings = manifest.surfaces[0].settings.as_ref().unwrap();
    assert_eq!(
        settings
            .properties
            .iter()
            .find(|p| p.name == "forward_plus")
            .unwrap()
            .value,
        1.0
    );
    fresco_example_engine::runtime::validate_artifact(&manifest).unwrap();
    manifest.schema_version = 1;
    assert!(fresco_example_engine::runtime::validate_artifact(&manifest).is_err());
    files.insert(
        "fresco.config.json".into(),
        r#"{"renderer":"missing"}"#.into(),
    );
    let error = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(format!("{error:?}").contains("unknown renderer"));
}

#[test]
fn compiler_managed_resources_follow_declared_allocation_groups() {
    let mut files = source_files();
    let core = files.get_mut("engine/core/01_core.fr").unwrap();
    *core = core
        .replace("@group(0) group material", "@group(1) group material")
        .replace("@group(1) group textures", "@group(0) group textures")
        .replace("@group(2) group draw", "@group(3) group draw")
        .replace("@group(3) group frame", "@group(2) group frame");
    files.insert(
        "main.fr".into(),
        r#"
        struct Clock { time: f32 }
        param clock: Clock
        surface probe(sp: surf) -> material(unlit) {
            param gain: f32 = 0.5
            param paint: texture<AlbedoRGB> = "paint.png"
            compose { base(albedo: paint.at(sp.uv) * gain * clock.time) }
        }
    "#
        .into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let surface = &manifest.surfaces[0];
    assert_eq!(surface.params[0].group, 1);
    assert_eq!(surface.textures[0].group, 0);
    assert_eq!(surface.sampler.as_ref().unwrap().group, 0);
    assert_eq!(surface.global_uniforms[0].group, 2);
    assert_eq!(
        manifest.vertex_factories[0].bindings[0].group_index,
        Some(3)
    );
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    for (_, resource) in module.global_variables.iter() {
        if resource.name.as_deref() == Some("clock") {
            assert_eq!(resource.binding.as_ref().unwrap().group, 2);
        }
    }
}

#[test]
fn independent_engine_has_two_targets_and_no_builtin_material_policy() {
    let files = std::collections::HashMap::from([
        ("engine/engine.fr".into(), include_str!("fixtures/tiny-engine.fr").into()),
        ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: rgba(0.2, 0.4, 0.6, 1.0)) } }".into()),
    ]);
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    assert!(manifest.tables.is_empty());
    assert!(
        manifest.surfaces[0]
            .settings
            .as_ref()
            .unwrap()
            .properties
            .is_empty()
    );
    assert_eq!(manifest.renderers[0].id, "ink");
    let entry = &manifest.surfaces[0]
        .mesh_passes
        .first()
        .unwrap()
        .entries
        .iter()
        .find(|e| e.function == "pack_pixels")
        .unwrap();
    assert_eq!(
        entry
            .outputs
            .iter()
            .map(|o| (o.location, o.ty.as_str()))
            .collect::<Vec<_>>(),
        [(0, "vec4"), (6, "u32")]
    );
}

#[test]
fn procedural_instance_draws_are_engine_defined_and_validate_vertex_sources() {
    let engine = include_str!("fixtures/tiny-engine.fr")
        .replace("@shader pass display", "@factory(plain) pass display")
        .replace(
            "@draw(cover, show, fullscreen)",
            "@draw(cover, show, instance)",
        );
    let mut files = std::collections::HashMap::from([
        ("engine/engine.fr".into(), engine.clone()),
        ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: #fff) } }".into()),
    ]);
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let display = manifest.surfaces[0]
        .mesh_passes
        .iter()
        .find(|p| p.pass == "display")
        .unwrap();
    assert!(display.procedural);
    assert!(
        display
            .entries
            .iter()
            .any(|e| e.function == "cover" && e.stage == "vertex")
    );
    assert!(!manifest.gpu_programs.iter().any(|p| p.pass == "display"));
    assert!(manifest.renderers[0].steps[1].is_draw_scoped());
    for (from, to) in [
        ("show, instance", "show, mesh"),
        ("pack_pixels, mesh", "pack_pixels, instance"),
    ] {
        files.insert("engine/engine.fr".into(), engine.replace(from, to));
        let error = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(format!("{error:?}").contains("does not match pass"));
    }
}

#[test]
fn executable_recipes_reject_invalid_contracts() {
    let engine = include_str!("fixtures/tiny-engine.fr");
    for (from, to, expected) in [
        ("@after(ink)", "@after(display)", "cycle"),
        ("@after(ink)", "@after(missing)", "dependency"),
        (
            "@draw(project, pack_pixels, mesh)",
            "@draw(project, missing, mesh)",
            "unknown fragment",
        ),
        (
            "@color(6, stamps)",
            "@color(6, absent)",
            "undefined resource",
        ),
        (
            "@image(stamps, r32uint)",
            "@image(stamps, rgba8unorm)",
            "incompatible",
        ),
        (
            "@bind(paint, paint)",
            "@bind(paint, screen)",
            "active attachment",
        ),
        ("@color(0, screen)", "@color(0, paint)", "active attachment"),
        ("cull: none", "cull: 42", "invalid value"),
        (
            "@location(6) tag",
            "@location(0) tag",
            "duplicate output location",
        ),
        (
            "return Interpolated(vec4(v.point, 1.0))",
            "discard_fragment(); return Interpolated(vec4(v.point, 1.0))",
            "stage",
        ),
    ] {
        assert!(engine.contains(from));
        let files = std::collections::HashMap::from([
            ("engine/engine.fr".into(), engine.replace(from, to)),
            ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: rgba(0.2, 0.4, 0.6, 1.0)) } }".into()),
        ]);
        let errors =
            fresco::driver::compile_bundle_virtual(&files, "main.fr", false).expect_err(to);
        assert!(
            errors
                .iter()
                .any(|e| e.message.to_lowercase().contains(expected)),
            "{to}: {errors:?}"
        );
    }
}

#[test]
fn renderer_selection_instantiates_its_own_mesh_pass() {
    let engine = include_str!("fixtures/tiny-engine.fr");
    let pass =
        &engine[engine.find("@factory(plain)").unwrap()..engine.find("struct Screen").unwrap()];
    let recipe = &engine[engine.find("@renderer").unwrap()..];
    let engine = format!(
        "{engine}\n{}\n{}",
        pass.replace("pass ink", "pass drawing"),
        recipe
            .replace(
                "@renderer(\"ink\", \"Ink on paper\") @default",
                "@renderer(\"alternate\", \"Alternate ink\")"
            )
            .replace(
                "pipeline(postprocess) print",
                "pipeline(postprocess) alternate"
            )
            .replace(" stamps) ink", " stamps) drawing")
            .replace("@after(ink)", "@after(drawing)")
    );
    for (renderer, pass) in [("ink", "ink"), ("alternate", "drawing")] {
        let files = std::collections::HashMap::from([
            ("engine/engine.fr".into(), engine.clone()),
            ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: #fff) } }".into()),
            ("fresco.config.json".into(), serde_json::json!({"renderer":renderer}).to_string()),
        ]);
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        assert_eq!(manifest.surfaces[0].mesh_passes.first().unwrap().pass, pass);
        assert_eq!(manifest.renderers.iter().filter(|r| r.selected).count(), 1);
    }
}

#[test]
fn only_selected_recipe_gpu_programs_are_emitted() {
    for (renderer, expected) in [
        ("forward", vec!["clear_opaque", "scene_background"]),
        (
            "forward-plus",
            vec![
                "clear_opaque",
                "preview_buffer_view",
                "scene_background",
                "tile_culling",
            ],
        ),
        (
            "deferred",
            vec![
                "clear_gbuffer",
                "clear_opaque",
                "deferred_buffer_view",
                "scene_background",
                "tile_culling",
            ],
        ),
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert(
            "main.fr".into(),
            "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let mut actual: Vec<_> = manifest
            .gpu_programs
            .iter()
            .map(|p| p.pass.as_str())
            .collect();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        assert_eq!(
            manifest.surfaces[0]
                .mesh_passes
                .iter()
                .any(|p| p.pass == "lighting_resolve" && p.procedural),
            renderer == "deferred",
            "Deferred lighting must be specialized per draw instance"
        );
        assert_eq!(
            output.wgsl.contains("fn fresco_gpu_lighting_resolve_"),
            renderer == "deferred"
        );
        assert_eq!(
            output.wgsl.contains("fn fresco_gpu_tile_culling_"),
            renderer != "forward"
        );
    }
}

#[test]
fn one_surface_has_independent_mesh_pass_programs_and_resource_wiring() {
    let engine = include_str!("fixtures/multi-pass-engine.fr");
    let files = std::collections::HashMap::from([
        ("engine/engine.fr".into(), engine.into()),
        ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: rgba(0.2, 0.4, 0.6, 1.0)) } }".into()),
    ]);
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest).unwrap();
    let passes = &manifest.surfaces[0].mesh_passes;
    assert_eq!(passes.len(), 2);
    let depth = passes.iter().find(|p| p.pass == "measure").unwrap();
    let color = passes.iter().find(|p| p.pass == "ink").unwrap();
    assert_eq!(depth.entries.len(), 1);
    assert_eq!(depth.entries[0].stage, "vertex");
    assert_ne!(
        depth.entries[0].entry,
        color
            .entries
            .iter()
            .find(|e| e.stage == "vertex")
            .unwrap()
            .entry
    );
    let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
    assert_eq!(recipe.steps[0].bindings["view"], "first_view");
    assert_eq!(recipe.steps[1].bindings["view"], "second_view");
    assert_eq!(recipe.steps[0].pass, recipe.steps[1].pass);
    assert_eq!(recipe.steps[2].bindings["view"], "second_view");
    let module = naga::front::wgsl::parse_str(&compiled.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    for invalid in [
        engine.replace(
            "@draw_depth(project, mesh, 3)",
            "@draw_depth(missing, mesh, 3)",
        ),
        engine.replace("@depth(measured)", ""),
    ] {
        let mut changed = files.clone();
        changed.insert("engine/engine.fr".into(), invalid);
        assert!(fresco::driver::compile_bundle_virtual(&changed, "main.fr", false).is_err());
    }
}

#[test]
fn preview_has_typed_shadow_dataflow_without_a_compiler_shadow_contract() {
    let mut files = source_files();
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(standard) { compose { base(albedo: #fff) } }".into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let entries = &manifest.surfaces[0].mesh_passes[0].entries;
    assert_eq!(entries.iter().filter(|e| e.stage == "vertex").count(), 2);
    let depth = entries
        .iter()
        .find(|e| e.function == "shadow_fragment")
        .unwrap();
    assert!(depth.outputs.is_empty());
    assert!(manifest.vertex_factories.iter().any(|f| {
        f.bindings
            .iter()
            .any(|b| b.signature.as_deref() == Some("texture_depth_2d"))
    }));
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let code = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    *code = code.replace("shadow_vertex(v: PreparedVertex)", "shadow_vertex(v: vec3)");
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| { e.message.contains("mesh vertex entries must share") }),
        "{errors:?}"
    );
}

#[test]
fn custom_schema_retains_its_material_and_response_in_every_renderer() {
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = if path == "deferred" {
            fresco_example_engine::source_files_for_deferred()
        } else {
            fresco_example_engine::source_files_for_renderer(path == "forward-plus")
        };
        files.insert(
            "main.fr".into(),
            include_str!("fixtures/custom-response.fr").into(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|errors| panic!("{path}: {errors:?}"));
        let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
        assert_eq!(manifest["surfaces"][0]["material_properties"], "toon");
        let entry = manifest["surfaces"][0]["evaluation_shader_entry"]
            .as_str()
            .unwrap();
        assert!(
            output.wgsl.matches(&format!("{entry}(")).count() >= 2,
            "custom response must be invoked"
        );
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
fn standard_material_uses_the_authored_standard_ggx_response_in_every_renderer() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = source_files();
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer": renderer}).to_string(),
        );
        files.insert(
            "main.fr".into(),
            "surface probe(sp: surf) -> material(standard) { compose { base(albedo: #fff) } }"
                .into(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|errors| panic!("{renderer}: {errors:?}"));
        let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
        assert_eq!(manifest["surfaces"][0]["material_properties"], "standard");
        assert_eq!(
            manifest["surfaces"][0]["settings"]["implementations"][0]["symbol"],
            "StandardGGX"
        );
        assert!(output.wgsl.contains("import_pbr_direct_response"));
        assert!(output.wgsl.contains("import_pbr_indirect_response"));
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
fn external_style_assignments_share_standard_data_and_generate_distinct_dispatch_ids() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    let (declarations, material) = sample.split_once("surface style_sample").unwrap();
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = source_files();
        files.insert("styles/external.fr".into(), declarations.into());
        files.insert("main.fr".into(), format!("import \"styles/external.fr\"\nsurface style_sample{material}\nsurface ordinary(sp: surf) -> material(standard) {{ compose {{ base(albedo: #fff) }} }}"));
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer": renderer}).to_string(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let assignments: std::collections::BTreeMap<_, _> = manifest
            .surfaces
            .iter()
            .map(|surface| {
                assert_eq!(surface.material_properties.as_deref(), Some("standard"));
                let selected = &surface.settings.as_ref().unwrap().implementations[0];
                assert_eq!(selected.name, "style");
                assert_eq!(selected.contract, "StandardStyle");
                (
                    surface.name.as_str(),
                    (selected.symbol.as_str(), selected.id),
                )
            })
            .collect();
        assert_eq!(assignments["ordinary"], ("StandardGGX", 1));
        assert_eq!(assignments["style_sample"], ("Toon", 2));
        assert!(
            output
                .wgsl
                .contains("fresco_implementation_StandardGGX_direct")
        );
        assert!(output.wgsl.contains("fresco_implementation_Toon_direct"));
    }
}

#[test]
fn contributed_style_draws_are_selected_per_material_and_require_compatible_coverage() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    for blend in ["Masked", "Translucent"] {
        let mut files = source_files();
        files.insert(
            "main.fr".into(),
            sample.replace(
                "style: Toon",
                &format!("style: Toon\nblend: SurfaceBlend.{blend}"),
            ),
        );
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("precondition is not satisfied")),
            "{errors:?}"
        );
    }
    let mut files = source_files();
    files.insert("main.fr".into(), format!("{sample}\nsurface ordinary(sp: surf) -> material(standard) {{ compose {{ base(albedo: #fff) }} }}"));
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
    let shell = recipe
        .steps
        .iter()
        .find(|s| {
            s.invocation
                .as_ref()
                .is_some_and(|i| i.operation == "InvertedHull")
        })
        .unwrap();
    for surface in &manifest.surfaces {
        let enabled = surface.name == "style_sample";
        assert_eq!(
            surface.settings.as_ref().unwrap().recipe_conditions[shell.condition.as_ref().unwrap()],
            enabled
        );
        assert_eq!(
            surface.mesh_passes.iter().any(|p| p.pass == shell.pass),
            enabled
        );
    }
}

#[test]
fn typed_style_boundaries_finish_before_all_downstream_phases() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = source_files();
        files.insert("main.fr".into(),format!("{sample}\nsurface glass(sp: surf) -> material(standard) {{ properties {{ blend: SurfaceBlend.Translucent }}; compose {{ base(albedo: #fff) }} }}"));
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":renderer}).to_string(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
        let shell = recipe
            .steps
            .iter()
            .find(|s| {
                s.invocation
                    .as_ref()
                    .is_some_and(|i| i.operation == "InvertedHull")
            })
            .unwrap();
        let opaque = if renderer == "deferred" {
            "lighting_resolve"
        } else {
            "preview_mesh"
        };
        assert!(shell.after.iter().any(|n| n == opaque));
        for name in [
            "transparent",
            if renderer == "forward" {
                "scene_background"
            } else if renderer == "deferred" {
                "deferred_buffer_view"
            } else {
                "preview_buffer_view"
            },
            "scene_background",
        ] {
            let step = recipe.steps.iter().find(|s| s.name == name).unwrap();
            assert!(
                step.after.contains(&shell.name),
                "{renderer}/{name} must await all outlines"
            );
        }
        let clear = recipe
            .steps
            .iter()
            .find(|s| {
                s.name
                    == if renderer == "deferred" {
                        "clear_geometry"
                    } else {
                        "clear_opaque"
                    }
            })
            .unwrap();
        assert!(clear.condition.is_none());
        assert_eq!(clear.domain, "fullscreen");
        let glass = manifest
            .surfaces
            .iter()
            .find(|s| s.name == "glass")
            .unwrap();
        let settings = glass.settings.as_ref().unwrap();
        assert!(!settings.recipe_conditions[shell.condition.as_ref().unwrap()]);
        let transparent = recipe
            .steps
            .iter()
            .find(|s| s.name == "transparent")
            .unwrap();
        assert!(settings.recipe_conditions[transparent.condition.as_ref().unwrap()]);
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
fn unused_style_operations_cannot_violate_attachment_or_binding_contracts() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr")
        .replace("style: Toon", "style: StandardGGX");
    for (from, to, expected) in [
        (
            "depth { write: false",
            "depth { write: true",
            "disable depth writes",
        ),
        (
            "color: load_store",
            "color: clear_store",
            "operation attachments require unique attachment parameters with load_store",
        ),
        (
            "view: frame",
            "view: opaque.depth",
            "operation resource `opaque.depth` does not provide `PreviewScene`",
        ),
        (
            "view: frame",
            "missing: frame",
            "unknown operation argument `missing`",
        ),
    ] {
        assert!(sample.contains(from));
        let mut files = source_files();
        files.insert("main.fr".into(), sample.replace(from, to));
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{from}: {errors:?}"
        );
    }
}

#[test]
fn preview_scene_content_is_opt_in_and_uses_ordinary_material_contracts() {
    for scene in [false, true] {
        let mut files = if scene {
            fresco_example_engine::preview_source_files()
        } else {
            source_files()
        };
        files.insert(
            "main.fr".into(),
            "canvas probe(ctx: CanvasContext) -> color { #fff }".into(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let ground = manifest
            .surfaces
            .iter()
            .find(|s| s.name == "fresco_scene_ground");
        assert_eq!(ground.is_some(), scene);
        assert_eq!(manifest.canvases.len(), 1);
        if let Some(ground) = ground {
            assert_eq!(ground.material_ty, "standard");
            assert!(!ground.mesh_passes.is_empty());
        }
    }
}

#[test]
fn structured_styles_and_finish_overrides_validate_in_every_renderer() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        for symbol in ["Accounting", "FinishAccounting"] {
            let mut files = source_files();
            files.insert(
                "main.fr".into(),
                include_str!("fixtures/style-accounting.fr")
                    .replace("style: Accounting", &format!("style: {symbol}")),
            );
            files.insert(
                "fresco.config.json".into(),
                serde_json::json!({"renderer": renderer}).to_string(),
            );
            let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                .unwrap_or_else(|e| panic!("{renderer}/{symbol}: {e:?}"));
            let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            for hook in ["direct", "indirect", "finish"] {
                assert!(
                    output
                        .wgsl
                        .contains(&format!("fresco_implementation_{symbol}_{hook}")),
                    "{renderer}/{symbol}/{hook}"
                );
            }
            assert!(!output.wgsl.contains("style_ambient"));
        }
    }
}

#[test]
fn runtime_style_settings_are_per_material_with_shared_dispatch() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = source_files();
        files.insert(
            "main.fr".into(),
            include_str!("fixtures/style-settings.fr").into(),
        );
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":renderer}).to_string(),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let selections: Vec<_> = manifest
            .surfaces
            .iter()
            .map(|s| &s.settings.as_ref().unwrap().implementations[0])
            .collect();
        assert_eq!(selections[0].id, selections[1].id);
        assert_ne!(selections[0].settings_offset, selections[1].settings_offset);
        assert_eq!(selections[0].parameters[0].default, 0.25);
        assert_eq!(selections[1].parameters[0].default, 0.75);
        assert_eq!(selections[0].parameters[0].min, Some(0.0));
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert!(
            output
                .wgsl
                .contains("fresco_implementation_Adjustable_finish")
        );
    }
}

#[test]
fn runtime_style_settings_reject_invalid_declarations_and_assignments() {
    let source = include_str!("fixtures/style-settings.fr");
    for (from, to, message) in [
        ("gain: 0.25", "missing: 0.25", "unknown style setting"),
        (
            "gain: 0.25",
            "gain: 0.25, gain: 0.5",
            "duplicate style setting",
        ),
        ("gain: 0.25", "0.25", "named arguments"),
        ("gain: 0.25", "gain: 2.0", "outside its declared range"),
        (
            "= 0.5 in 0.0 .. 1.0",
            "= 2.0 in 0.0 .. 1.0",
            "outside its declared range",
        ),
        (
            "param gain: f32",
            "param gain: mat4",
            "style settings support",
        ),
        ("gain: 0.25", "gain: vec3(0.0)", "expected scalar"),
    ] {
        let mut files = source_files();
        files.insert("main.fr".into(), source.replace(from, to));
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(message)),
            "{to}: {errors:?}"
        );
    }
}

#[test]
fn style_selection_overrides_persist_named_values_not_numeric_ids() {
    let mut files = source_files();
    files.insert(
        "main.fr".into(),
        include_str!("fixtures/style-settings.fr").into(),
    );
    files.insert("fresco.config.json".into(),serde_json::json!({"property_overrides":{"first":{"style":{"symbol":"Adjustable","settings":{"gain":0.125,"tint":[0,1,0,1]}}}}}).to_string());
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let selected = &manifest.surfaces[0]
        .settings
        .as_ref()
        .unwrap()
        .implementations[0];
    assert_eq!(selected.parameters[0].default, 0.125);
    assert_eq!(
        selected.parameters[1].default,
        serde_json::json!([0.0, 1.0, 0.0, 1.0])
    );
}

#[test]
fn integer_and_boolean_style_settings_compile_exactly_for_all_renderers() {
    let source = include_str!("fixtures/style-settings.fr")
        .replace("param gain: f32", "param seed: u32 = u32(4294967295)\nparam offset: i32 = i32(-2147483648)\nparam enabled: bool = true\nstatic param layers: u32 in [1, 32] = u32(12)\nparam gain: f32")
        .replace("return tint.rgb * gain", "if enabled == false { return vec3(0.0) }\nreturn tint.rgb * gain * f32((seed & u32(255)) + u32(layers)) + vec3(f32(offset))");
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = source_files();
        files.insert("main.fr".into(), source.clone());
        files.insert("fresco.config.json".into(), serde_json::json!({"renderer":renderer,"property_overrides":{"first":{"style":{"symbol":"Adjustable","settings":{"seed":16777217,"layers":24,"enabled":false}}}}}).to_string());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let first = manifest
            .surfaces
            .iter()
            .find(|s| s.name == "first")
            .unwrap();
        let selected = &first.settings.as_ref().unwrap().implementations[0];
        assert_eq!(selected.parameters[0].default, serde_json::json!(16777217));
        assert_eq!(selected.parameters[1].default, serde_json::json!(i32::MIN));
        assert_eq!(selected.parameters[2].default, serde_json::json!(false));
        assert_eq!(selected.static_parameters[0].default, serde_json::json!(24));
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
fn disabled_outline_removes_preparation_and_capability_demand() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr")
        .replace(
            "outline_enabled: bool = true",
            "outline_enabled: bool = false",
        )
        .replace("style: Toon", "style: Toon; blend: SurfaceBlend.Masked");
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = source_files();
        files.insert("main.fr".into(), sample.clone());
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":renderer}).to_string(),
        );
        let providers = files.get_mut("engine/config/renderer.fr").unwrap();
        *providers = providers
            .lines()
            .filter(|line| !line.contains("PreparedGeometry {"))
            .collect::<Vec<_>>()
            .join("\n");
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        assert!(
            manifest.surfaces[0]
                .mesh_passes
                .iter()
                .all(|p| p.preparation.is_none() && p.prepared_source.is_none())
        );
        assert!(
            manifest
                .renderers
                .iter()
                .find(|r| r.selected)
                .unwrap()
                .steps
                .iter()
                .all(|s| s.invocation.is_none())
        );
    }
}

#[test]
fn unused_prepared_operations_check_attachment_and_capture_contracts() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr")
        .replace("style: Toon", "style: StandardGGX");
    for (from, to, expected) in [
        (
            "depth { write: false",
            "depth { write: true",
            "disable depth writes",
        ),
        ("color: load_store", "color: clear_store", "load_store"),
        (
            "view: PreviewScene",
            "view: PreviewLighting",
            "PreviewLighting",
        ),
        ("view.proj", "missing.proj", "missing"),
    ] {
        assert!(sample.contains(from));
        let mut files = source_files();
        files.insert("main.fr".into(), sample.replace(from, to));
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{from}: {errors:?}"
        );
    }
}

#[test]
fn reflected_style_candidates_use_material_and_static_graph_checks() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    for (selected, missing_geometry, supported) in [
        ("StandardGGX", false, false),
        ("Toon(outline_enabled: false)", true, true),
        ("StandardGGX", true, false),
    ] {
        let mut files = source_files();
        files.insert(
            "main.fr".into(),
            sample.replace(
                "style: Toon",
                &format!("style: {selected}; blend: SurfaceBlend.Masked"),
            ),
        );
        if missing_geometry {
            let providers = files.get_mut("engine/config/renderer.fr").unwrap();
            *providers = providers
                .lines()
                .filter(|line| !line.contains("PreparedGeometry {"))
                .collect::<Vec<_>>()
                .join("\n");
        }
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let surface = manifest
            .surfaces
            .iter()
            .find(|s| s.name == "style_sample")
            .unwrap();
        let selection = &surface.settings.as_ref().unwrap().implementations[0];
        let toon = selection
            .availability
            .iter()
            .find(|c| c.symbol == "Toon")
            .unwrap();
        assert_eq!(toon.supported, supported, "{toon:?}");
        assert_eq!(toon.schema, "standard");
        assert!(toon.provider);
        assert!(toon.renderer.is_some());
        assert!(toon.factories.iter().any(|f| f == "preview_static"));
        assert_eq!(
            toon.capabilities
                .iter()
                .any(|c| c.name == "PreparedGeometry" && c.supported),
            !missing_geometry
        );
        assert!(
            toon.integration_points
                .iter()
                .any(|p| p.name == "after_opaque" && p.supported)
        );
        if supported {
            assert!(toon.reasons.is_empty());
            assert_eq!(toon.static_parameters[0].default, serde_json::json!(false));
        } else {
            assert!(!toon.reasons.is_empty());
            files.insert(
                "main.fr".into(),
                sample.replace("style: Toon", "style: Toon; blend: SurfaceBlend.Masked"),
            );
            let errors =
                fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
            assert!(
                errors.iter().any(|e| toon.reasons.contains(&e.message)),
                "{errors:?} vs {toon:?}"
            );
        }
        // Probing Toon must not add its hull to the selected PBR render graph.
        assert!(
            manifest
                .renderers
                .iter()
                .find(|r| r.selected)
                .unwrap()
                .steps
                .iter()
                .all(|s| s.invocation.is_none())
        );
    }
}

#[test]
fn imported_style_candidates_are_discovered_without_a_registry_entry() {
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    let (definitions, material) = sample.split_once("surface style_sample").unwrap();
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert(
            "external/response.fr".into(),
            definitions.replace("Toon", "ExternalBands"),
        );
        files.insert(
            "main.fr".into(),
            format!(
                "import \"external/response.fr\"\nsurface style_sample{}",
                material.replace("Toon", "ExternalBands")
            ),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let selection = &manifest
            .surfaces
            .iter()
            .find(|s| s.name == "style_sample")
            .unwrap()
            .settings
            .as_ref()
            .unwrap()
            .implementations[0];
        assert_eq!(selection.symbol, "ExternalBands");
        let candidate = selection
            .availability
            .iter()
            .find(|c| c.symbol == "ExternalBands")
            .unwrap();
        assert!(candidate.supported, "{candidate:?}");
        assert!(
            candidate
                .capabilities
                .iter()
                .any(|c| c.name == "PreparedGeometry" && c.supported)
        );
        assert!(
            manifest
                .renderers
                .iter()
                .find(|r| r.selected)
                .unwrap()
                .steps
                .iter()
                .any(|s| s
                    .invocation
                    .as_ref()
                    .is_some_and(|i| i.material == "style_sample"))
        );
    }
}

#[test]
fn reflected_capabilities_check_factory_restrictions_even_when_unused() {
    let mut files = source_files();
    let sample = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    files.insert(
        "main.fr".into(),
        sample.replace("style: Toon", "style: StandardGGX"),
    );
    let mesh = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    let start = mesh.find("vertex_factory preview_static").unwrap();
    let end = mesh[start..].find("struct PreviewDirectSample").unwrap() + start;
    let alternate = mesh[start..end].replace("preview_static", "alternate_factory");
    mesh.push_str(&alternate);
    let providers = files.get_mut("engine/config/renderer.fr").unwrap();
    *providers = providers
        .lines()
        .map(|line| {
            if line.contains("PreparedGeometry {") {
                line.replace("factories: preview_static", "factories: alternate_factory")
            } else {
                line.into()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let selection = &manifest
        .surfaces
        .iter()
        .find(|s| s.name == "style_sample")
        .unwrap()
        .settings
        .as_ref()
        .unwrap()
        .implementations[0];
    for candidate in &selection.availability {
        let capability = candidate
            .capabilities
            .iter()
            .find(|c| c.name == "PreparedGeometry")
            .unwrap();
        assert!(capability.provided);
        assert!(!capability.supported, "{capability:?}");
        assert!(
            capability
                .reasons
                .iter()
                .any(|r| r.contains("vertex factory `preview_static`"))
        );
        assert_eq!(candidate.supported, candidate.symbol == "StandardGGX");
    }
    files.insert("main.fr".into(), sample.into());
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    let toon = selection
        .availability
        .iter()
        .find(|c| c.symbol == "Toon")
        .unwrap();
    assert!(errors.iter().any(|e| toon.reasons.contains(&e.message)));
}
