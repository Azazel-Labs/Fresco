use fresco_artifact::ManifestRoot;

#[test]
fn one_material_can_submit_ordinary_and_contributed_draws_to_the_same_queue() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert(
            "main.fr".into(),
            include_str!("fixtures/transparent-queue.fr").into(),
        );
        let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|errors| panic!("{renderer}: {errors:?}"));
        let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

fn mixed_scene() -> String {
    let toon = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    let veil = toon_at("transparent")
        .replace("Toon", "Veil")
        .replace("InvertedHull", "TransparentShell")
        .replace("style_sample", "veil")
        .replace("blend { all: replace }", "blend { all: premultiplied }")
        .replace("return ink", "return vec4(ink.rgb * ink.a, ink.a)");
    format!(
        "{toon}\n{veil}\nsurface glass(sp: surf) -> material(standard) {{ properties {{ blend: SurfaceBlend.Translucent }}; compose {{ base(albedo: rgba(0.2, 0.4, 0.6, 0.5)) }} }}"
    )
}

fn toon_at(point: &str) -> String {
    let source = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    assert!(source.contains("color: opaque.color, depth: opaque.depth)"));
    source
        .replace(
            "            InvertedHull(",
            &format!("            at {point} as target {{\n                InvertedHull("),
        )
        .replace(
            "color: opaque.color, depth: opaque.depth)",
            "color: target.color, depth: target.depth)\n            }",
        )
}

#[test]
fn transparent_style_draws_join_ordinary_transparency_after_opaque_contributions() {
    let source = mixed_scene();
    for (renderer, reverse_declarations) in [
        ("forward", false),
        ("forward-plus", false),
        ("deferred", false),
        ("forward", true),
        ("forward-plus", true),
        ("deferred", true),
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert(
            "main.fr".into(),
            if reverse_declarations {
                source.replace("Toon", "Zulu").replace("Veil", "Alpha")
            } else {
                source.clone()
            },
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|errors| panic!("{renderer}: {errors:?}"));
        let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
        let recipe = manifest
            .renderers
            .iter()
            .find(|recipe| recipe.selected)
            .unwrap();
        let queued: Vec<_> = recipe
            .steps
            .iter()
            .filter(|step| step.transparent_queue.as_deref() == Some("transparency"))
            .collect();
        assert_eq!(queued.len(), 2, "{renderer}");
        assert!(queued.iter().any(|step| step.name == "transparent"));
        assert!(queued.iter().any(|step| {
            step.invocation
                .as_ref()
                .is_some_and(|i| i.operation == "TransparentShell")
        }));
        let outline = recipe
            .steps
            .iter()
            .find(|step| {
                step.invocation
                    .as_ref()
                    .is_some_and(|i| i.operation == "InvertedHull")
            })
            .unwrap();
        assert!(outline.transparent_queue.is_none());
        for step in &queued {
            assert!(
                step.after.contains(&outline.name),
                "{renderer}: queue must wait for the complete opaque contribution"
            );
            assert!(
                step.after
                    .iter()
                    .all(|dependency| !queued.iter().any(|member| member.name == *dependency))
            );
        }
        let last_queue = recipe
            .steps
            .iter()
            .rposition(|step| step.transparent_queue.is_some())
            .unwrap();
        assert!(recipe.steps[last_queue + 1..].iter().any(|step| {
            queued
                .iter()
                .all(|member| step.after.contains(&member.name))
        }));
        fresco_artifact::validate_transparent_queues(&recipe.steps).unwrap();
    }
}

#[test]
fn invalid_transparent_providers_and_draw_states_are_rejected() {
    for (file, from, to, expected) in [
        (
            "engine/config/renderer.fr",
            "queue: transparency",
            "queue: missing",
            "declared renderer queue",
        ),
        (
            "engine/config/renderer.fr",
            "queue: transparency\n        complete_opaque: all(preview_mesh)",
            "queue: transparency\n        complete_opaque: all(transparent)",
            "between every incoming and outgoing",
        ),
        (
            "main.fr",
            "blend { all: premultiplied }",
            "blend { all: replace }",
            "premultiplied blending",
        ),
        (
            "main.fr",
            "depth { write: false; compare: less_equal }",
            "depth { write: false; compare: always }",
            "read-only less/less_equal",
        ),
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe("forward");
        files.insert("main.fr".into(), mixed_scene());
        let source = files.get_mut(file).unwrap();
        assert!(source.contains(from));
        *source = source.replace(from, to);
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(format!("{errors:?}").contains(expected), "{errors:?}");
    }
}

#[test]
fn compatible_points_share_one_global_queue_without_an_artificial_order() {
    fn alias_blocks(source: &str, declaration: &str) -> String {
        let mut result = source.to_owned();
        let blocks: Vec<_> = source
            .match_indices(declaration)
            .map(|(start, _)| {
                let end = start + source[start..].find("\n    }").unwrap() + "\n    }".len();
                source[start..end].to_owned()
            })
            .collect();
        assert!(!blocks.is_empty());
        for block in blocks {
            result = result.replace(
                &block,
                &format!(
                    "{block}\n{}",
                    block.replacen("transparent", "transparent_extra", 1)
                ),
            );
        }
        result
    }
    let extra = toon_at("transparent_extra")
        .replace("Toon", "VeilTwo")
        .replace("InvertedHull", "TransparentShellTwo")
        .replace("style_sample", "veil_two")
        .replace("blend { all: replace }", "blend { all: premultiplied }")
        .replace("return ink", "return vec4(ink.rgb * ink.a, ink.a)");
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), format!("{}\n{extra}", mixed_scene()));
        let contract = files.get_mut("engine/styles/contract.fr").unwrap();
        *contract = alias_blocks(contract, "    optional point transparent:");
        let provider = files.get_mut("engine/config/renderer.fr").unwrap();
        *provider = alias_blocks(provider, "    transparent {");
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
        let queued: Vec<_> = manifest
            .renderers
            .iter()
            .find(|r| r.selected)
            .unwrap()
            .steps
            .iter()
            .filter(|s| s.transparent_queue.as_deref() == Some("transparency"))
            .collect();
        assert_eq!(queued.len(), 3);
        assert!(queued.iter().any(|s| {
            s.invocation
                .as_ref()
                .is_some_and(|i| i.point.ends_with("::transparent_extra"))
        }));
        assert!(queued.iter().any(|s| {
            s.invocation
                .as_ref()
                .is_some_and(|i| i.point.ends_with("::transparent"))
        }));
        assert!(
            queued
                .iter()
                .all(|a| queued.iter().all(|b| !a.after.contains(&b.name)))
        );
    }
}
