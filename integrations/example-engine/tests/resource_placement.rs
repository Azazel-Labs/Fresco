#[path = "fixtures/resource_placement.rs"]
mod fixture;

#[test]
fn toon_sample_migration_preserves_executable_work() {
    let inferred = include_str!("../../../examples/40) surface shaders/style_sample.fr");
    assert!(inferred.contains("color: opaque.color, depth: opaque.depth)"));
    let explicit = inferred
        .replace(
            "            InvertedHull(",
            "            at after_opaque as target {\n                InvertedHull(",
        )
        .replace(
            "color: opaque.color, depth: opaque.depth)",
            "color: target.color, depth: target.depth)\n            }",
        );
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut artifacts = Vec::new();
        for source in [inferred, &explicit] {
            let mut files = fresco_example_engine::source_files_for_recipe(renderer);
            files.insert("main.fr".into(), source.into());
            let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
            let manifest: fresco_artifact::ManifestRoot =
                serde_json::from_str(&output.manifest).unwrap();
            artifacts.push((
                output.wgsl,
                serde_json::to_value(&manifest.renderers).unwrap(),
            ));
        }
        assert_eq!(artifacts[0], artifacts[1], "{renderer}: Toon migration");
    }
}

#[test]
fn resource_placement_preserves_graph_shaders_and_early_compute_in_every_renderer() {
    use fresco_example_engine::runtime::compute_graph::ComputeGraph;
    use std::collections::BTreeSet;
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut artifacts = Vec::new();
        for mode in [0, 1, 2] {
            let inferred = mode == 1;
            let mut files = fresco_example_engine::source_files_for_recipe(renderer);
            let mut source = fixture::source(inferred);
            if mode == 2 {
                source = source
                    .replace("target.color", "opaque.color")
                    .replace("target.depth", "opaque.depth");
            }
            files.insert("main.fr".into(), source);
            let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                .unwrap_or_else(|errors| panic!("{renderer}/{inferred}: {errors:?}"));
            let manifest: fresco_artifact::ManifestRoot =
                serde_json::from_str(&output.manifest).unwrap();
            let recipe = manifest
                .renderers
                .iter()
                .find(|renderer| renderer.selected)
                .unwrap();
            let color = recipe
                .resource_ports
                .iter()
                .find(|port| port.member == "color")
                .unwrap();
            let depth = recipe
                .resource_ports
                .iter()
                .find(|port| port.member == "depth")
                .unwrap();
            assert_eq!(
                color.versions.len(),
                5,
                "two operations for each of two materials"
            );
            assert_eq!(
                depth.versions.len(),
                1,
                "test-only depth must not produce a version"
            );
            if renderer != "forward" {
                let inspector = if renderer == "deferred" {
                    "deferred_buffer_view"
                } else {
                    "preview_buffer_view"
                };
                assert!(
                    depth.versions[0]
                        .readers
                        .iter()
                        .any(|reader| reader == inspector),
                    "read-only transparency must not hide the depth version consumed by inspection"
                );
            }
            for material in ["first", "second"] {
                let graph = ComputeGraph::new(&manifest, material).unwrap();
                assert_eq!(
                    graph.ready(&BTreeSet::new()).count(),
                    1,
                    "ink compute needs no opaque completion"
                );
            }
            artifacts.push((
                output.wgsl,
                serde_json::to_value(&manifest.renderers).unwrap(),
                serde_json::to_value(&manifest.gpu_programs).unwrap(),
            ));
        }
        assert_eq!(
            artifacts[0], artifacts[1],
            "{renderer}: placement changed executable work"
        );
        assert_eq!(
            artifacts[0], artifacts[2],
            "{renderer}: explicit point with matching port arguments"
        );
    }
}

#[test]
fn transparent_port_versions_cover_the_whole_sorted_queue() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        let contract = files.get_mut("engine/styles/contract.fr").unwrap();
        *contract = contract.replace(
            "optional point transparent: OpaqueTarget {",
            "optional point transparent: OpaqueTarget { port: translucent;",
        );
        let source = include_str!("../../../examples/40) surface shaders/style_sample_fur.fr");
        files.insert("main.fr".into(), source.into());
        let explicit = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let source = source
            .replace("at transparent as target {", "")
            .replace("            }\n        }\n    }", "            }\n    }")
            .replace("target.color", "translucent.color")
            .replace("target.depth", "translucent.depth");
        files.insert("main.fr".into(), source);
        let inferred = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let first: fresco_artifact::ManifestRoot =
            serde_json::from_str(&explicit.manifest).unwrap();
        let second: fresco_artifact::ManifestRoot =
            serde_json::from_str(&inferred.manifest).unwrap();
        assert_eq!(explicit.wgsl, inferred.wgsl);
        assert_eq!(
            serde_json::to_value(&first.renderers).unwrap(),
            serde_json::to_value(&second.renderers).unwrap()
        );
        let recipe = first
            .renderers
            .iter()
            .find(|renderer| renderer.selected)
            .unwrap();
        let port = recipe
            .resource_ports
            .iter()
            .find(|port| port.name == "translucent" && port.member == "color")
            .unwrap();
        assert_eq!(port.versions.len(), 2);
        let queue = port.queue.as_ref().unwrap();
        let mut writers: Vec<_> = recipe
            .steps
            .iter()
            .filter(|step| step.transparent_queue.as_ref() == Some(queue))
            .map(|step| step.name.clone())
            .collect();
        writers.sort();
        assert_eq!(port.versions[1].producers, writers);
    }
}
