use fresco_artifact::{ComputeScalar, ManifestComputeArgument, ManifestRoot};
use fresco_example_engine::runtime::{
    RuntimeError,
    compute_plan::{ComputeInvocationPlan, ComputeLimits, OwnedAllocation},
};

fn compile() -> ManifestRoot {
    compile_source(include_str!("fixtures/owned-compute.fr"))
}

#[test]
fn shading_sampler_presets_are_typed_resources_with_explicit_sampling() {
    use fresco_artifact::types::SamplerPreset;
    let source = include_str!("fixtures/shading-sampler.fr");
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), source.into());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
        let pass = manifest.surfaces[0]
            .mesh_passes
            .iter()
            .find(|p| {
                p.pass
                    == if renderer == "deferred" {
                        "lighting_resolve"
                    } else {
                        "preview_mesh"
                    }
            })
            .unwrap();
        let sampler = pass.bindings.iter().find(|b| b.sampler.is_some()).unwrap();
        assert_eq!(sampler.sampler, Some(SamplerPreset::NearestRepeat));
        assert_eq!(sampler.signature.as_deref(), Some("sampler"));
        assert!(!pass.shading_inputs.contains_key(&sampler.name));
        assert!(output.wgsl.contains("textureSampleLevel"));
        assert!(output.wgsl.contains("textureSampleGrad"));
        for (from, to, diagnostic) in [
            (
                "nearest_repeat",
                "unknown_sampler",
                "standard sampler value",
            ),
            (
                "bind shading.filtering = nearest_repeat",
                "",
                "missing binding for shading input `filtering`",
            ),
            (
                "bind shading.filtering = nearest_repeat",
                "bind shading.filtering = image",
                "standard sampler value",
            ),
            (
                "requires SurfaceUV, DrawShadingResources",
                "requires SurfaceUV, DrawShadingResources\n param nearest_repeat: f32 = 0.0",
                "unshadowed standard sampler value",
            ),
        ] {
            files.insert("main.fr".into(), source.replace(from, to));
            let errors =
                fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
            assert!(
                format!("{errors:?}").contains(diagnostic),
                "{renderer}: {errors:?}"
            );
        }
    }
}

#[test]
fn draw_instance_binding_is_typed_and_separate_from_material_table_sources() {
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    files.insert(
        "main.fr".into(),
        "surface item(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| f.name == "preview_static")
        .unwrap();
    let binding = factory
        .bindings
        .iter()
        .find(|b| b.draw_data.is_some())
        .unwrap();
    assert_eq!(
        binding.draw_data,
        Some(fresco_artifact::ManifestDrawData::InstanceId)
    );
    assert_eq!(binding.signature.as_deref(), Some("uniform<u32>"));
    assert!(binding.source.is_none());
    for (from, to) in [
        ("@draw_data(instance_id)", "@draw_data(material_id)"),
        (
            "shading_instance_id: uniform<u32>",
            "shading_instance_id: uniform<f32>",
        ),
        (
            "@draw_data(instance_id)",
            "@draw_data(instance_id) @source(DrawRecord)",
        ),
    ] {
        let mut invalid = files.clone();
        let engine = invalid.get_mut("engine/core/05_mesh_contract.fr").unwrap();
        *engine = engine.replace(from, to);
        let error = fresco::driver::compile_bundle_virtual(&invalid, "main.fr", false).unwrap_err();
        assert!(format!("{error:?}").contains("@draw_data(instance_id) requires uniform<u32>"));
    }
}

#[test]
fn unused_sampler_bindings_validate_presets_types_and_exclusive_sources() {
    for declaration in [
        "@sampler(unknown) unused_sampler: sampler",
        "@sampler(nearest_repeat) unused_sampler: uniform<u32>",
        "@sampler(nearest_repeat) @sampler(linear_repeat) unused_sampler: sampler",
        "@sampler(nearest_repeat) @source(DrawRecord) unused_sampler: sampler",
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe("forward");
        let engine = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
        *engine = engine.replace(
            "@group(draw) scene: uniform<PreviewScene>",
            &format!("@group(draw) scene: uniform<PreviewScene>\n @group(draw) {declaration}"),
        );
        files.insert(
            "main.fr".into(),
            "surface item(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
        );
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            format!("{errors:?}").contains("@sampler requires"),
            "{errors:?}"
        );
    }
}

#[test]
fn shading_capabilities_validate_actual_shader_outputs_and_instance_bindings() {
    let source = r#"
style Coordinates for standard : StandardStyle {
    requires SurfaceUV, DrawShadingResources
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 { return vec3(0.0) }
    fn finish(surface: StandardSurface, context: ShadingContext, lighting: LightingResult) -> vec3 { return vec3(surface.uv, 0.0) }
}
surface item(sp: surf) -> material(standard) {
    properties { style: Coordinates }
    compose { base(albedo: #fff) }
}
"#;
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), source.into());
        fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let producer = if renderer == "deferred" {
            "lighting_resolve"
        } else {
            "preview_mesh"
        };
        let uv = if renderer == "deferred" {
            "opaque.geometry.surface_uv"
        } else {
            "preview_mesh.vertex.uv"
        };
        for (from, to, diagnostic) in [
            (
                format!("draw_data({producer}.shading_instance_id)"),
                format!("draw_data({producer}.draw_record)"),
                "typed per-draw instance data",
            ),
            (
                format!("shader_output({uv})"),
                "shader_output(missing.vertex.uv)".into(),
                "unknown capability producer",
            ),
            (
                format!("shader_output({uv})"),
                if renderer == "deferred" {
                    "shader_output(opaque.geometry.identity)".into()
                } else {
                    "shader_output(preview_mesh.vertex.world_pos)".into()
                },
                "requires `vec2`, found",
            ),
        ] {
            let mut invalid = files.clone();
            let config = invalid.get_mut("engine/config/renderer.fr").unwrap();
            assert!(config.contains(&from));
            *config = config.replace(&from, &to);
            let errors =
                fresco::driver::compile_bundle_virtual(&invalid, "main.fr", false).unwrap_err();
            assert!(
                format!("{errors:?}").contains(diagnostic),
                "{renderer}: {errors:?}"
            );
        }
        for capability in ["SurfaceUV", "DrawShadingResources"] {
            let mut missing = files.clone();
            let config = missing.get_mut("engine/config/renderer.fr").unwrap();
            *config = config
                .lines()
                .filter(|line| !line.trim_start().starts_with(&format!("{capability} {{")))
                .collect::<Vec<_>>()
                .join("\n");
            let errors =
                fresco::driver::compile_bundle_virtual(&missing, "main.fr", false).unwrap_err();
            assert!(format!("{errors:?}").contains(&format!("does not provide `{capability}`")));
        }
    }
}

#[test]
fn draw_scoped_shading_captures_owned_compute_output_in_all_renderers() {
    let source = r#"
compute ColorField() -> buffer<vec4, read> {
    output colors: buffer<vec4, write>(1u)
    workgroup_size: (1, 1, 1)
    dispatch threads(1u, 1u, 1u)
    @compute fn main(id: uvec3) { colors[id.x] = vec4(0.2, 0.7, 0.4, 1.0) }
    return colors
}
style Captured for standard : StandardStyle {
    requires SurfaceUV, DrawShadingResources
    shading_input colors: buffer<vec4, read> scope draw
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 { return vec3(0.0) }
    fn finish(surface: StandardSurface, context: ShadingContext, lighting: LightingResult) -> vec3 {
        return colors[0u].xyz
    }
    for self { let field = ColorField(); bind shading.colors = field }
}

surface item(sp: surf) -> material(standard) {
    properties { style: Captured }
    compose { base(albedo: #fff) }
}
"#;
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), source.into());
        let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let manifest: ManifestRoot = serde_json::from_str(&result.manifest).unwrap();
        let producer = manifest
            .gpu_programs
            .iter()
            .find(|p| p.compute_invocation.is_some())
            .unwrap();
        let captures: Vec<_> = manifest.surfaces[0]
            .mesh_passes
            .iter()
            .flat_map(|pass| {
                pass.shading_inputs
                    .iter()
                    .map(move |(name, input)| (pass, name, input))
            })
            .collect();
        assert!(!captures.is_empty(), "{renderer}");
        let mut reads_capture = false;
        let mut deferred_reads_capture = false;
        for (pass, name, input) in captures {
            assert_eq!(input.producer, producer.pass);
            assert_eq!(input.ty.replace(' ', ""), "buffer<vec4,read>");
            let binding = pass
                .bindings
                .iter()
                .find(|binding| &binding.name == name)
                .unwrap();
            for (index, entry) in module.entry_points.iter().enumerate() {
                if pass
                    .entries
                    .iter()
                    .any(|e| e.entry == entry.name && e.stage == "fragment")
                {
                    let reads = module.global_variables.iter().any(|(handle, global)| {
                        global.binding.as_ref().is_some_and(|slot| {
                            Some(slot.group) == binding.group_index
                                && Some(slot.binding) == binding.binding
                        }) && info.get_entry_point(index)[handle]
                            .contains(naga::valid::GlobalUse::READ)
                    });
                    reads_capture |= reads;
                    deferred_reads_capture |= reads && pass.pass == "lighting_resolve";
                }
            }
        }
        assert!(
            reads_capture,
            "{renderer}: shading must actually read the captured output"
        );
        if renderer == "deferred" {
            assert!(
                deferred_reads_capture,
                "Deferred resolve itself must consume the output"
            );
            let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
            let resolve = recipe
                .steps
                .iter()
                .find(|s| s.pass == "lighting_resolve")
                .unwrap();
            assert_eq!(resolve.domain, "instance");
            assert!(resolve.after.iter().any(|n| n == "opaque"));
            assert!(resolve.after.iter().any(|n| n == "clear_lighting"));
            assert!(resolve.attachments["lit_color"].load);
            assert!(resolve.attachments["lit_color"].store);
        }
        assert!(
            producer
                .compute_invocation
                .as_ref()
                .unwrap()
                .engine_dependencies
                .is_empty()
        );
    }
}
fn compile_source(source: &str) -> ManifestRoot {
    compile_renderer(source, "forward")
}
fn compile_renderer(source: &str, renderer: &str) -> ManifestRoot {
    let mut files = fresco_example_engine::source_files_for_recipe(renderer);
    files.insert("main.fr".into(), source.into());
    let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    serde_json::from_str(&result.manifest).unwrap()
}

#[test]
fn compute_geometry_demand_is_material_scoped_in_every_renderer() {
    let source = format!(
        "{}\nsurface untouched(sp: surf) -> material(standard) {{ compose {{ base(albedo: #fff) }} }}",
        include_str!("fixtures/owned-compute.fr")
    );
    for renderer in ["forward", "forward-plus", "deferred"] {
        let manifest = compile_renderer(&source, renderer);
        let owner = manifest.surfaces.iter().find(|s| s.name == "item").unwrap();
        let other = manifest
            .surfaces
            .iter()
            .find(|s| s.name == "untouched")
            .unwrap();
        assert!(
            owner.mesh_passes.iter().any(|p| p.preparation.is_some()),
            "{renderer}"
        );
        assert!(
            other.mesh_passes.iter().all(|p| p.preparation.is_none()),
            "{renderer}"
        );
        assert!(
            manifest
                .gpu_programs
                .iter()
                .filter_map(|p| p.compute_invocation.as_ref())
                .all(|i| i.material == "item" && i.engine_dependencies.is_empty())
        );
    }
}
fn limits() -> ComputeLimits {
    ComputeLimits {
        max_workgroup_size: [256, 256, 64],
        max_workgroup_invocations: 256,
        max_workgroups: 65535,
        max_buffer_bytes: 1 << 30,
        max_storage_binding_bytes: 1 << 27,
        max_texture_dimension_2d: 8192,
    }
}

const COMPUTED_DRAW: &str = r#"
struct ComputedVertex { @semantic(position) position: vec4 }
@factory(preview_static) draw Show(geometry: DrawRange, scene: PreviewScene,
    values: buffer<vec4, read>, field: texture2d<rg32float, read>,
    target: attachment<rgba16float, preserve_update>) {
    raster geometry
    attachments { target: load_store }
    @vertex fn project(v: PreviewShaded) -> ComputedVertex {
        return ComputedVertex(scene.proj * scene.view * factory.transform(v) * vec4(v.position, 1.0))
    }
    @fragment fn paint(v: ComputedVertex) -> vec4 {
        if (values).count == 0u || (field).width == 0u { return vec4(0.0) }
        return values[0u] + textureLoad(field, ivec2(0), 0)
    }
}
"#;

#[test]
fn unused_draw_resource_inputs_are_checked_and_cannot_be_shadowed() {
    let source = format!(
        "{}\n{COMPUTED_DRAW}",
        include_str!("fixtures/owned-compute.fr")
    );
    compile_source(&source);
    for (from, to, message) in [
        (
            "textureLoad(field, ivec2(0), 0)",
            "textureLoad(field, ivec2(0), 0.5)",
            "texture integer argument",
        ),
        (
            "values: buffer<vec4, read>",
            "values: buffer<vec3, read>",
            "incompatible draw shader operands",
        ),
        (
            "if (values).count",
            "let values = vec4(0.0); if (values).count",
            "shadows an operation input",
        ),
        (
            "if (values).count",
            "let __fresco_values_count = 0u; if (values).count",
            "reserved binding name",
        ),
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe("forward");
        files.insert("main.fr".into(), source.replace(from, to));
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(format!("{errors:?}").contains(message), "{errors:?}");
    }
}

#[test]
fn contributed_draw_samples_owned_image_with_an_explicit_sampler_in_every_renderer() {
    let source = format!("{}\n{}", include_str!("fixtures/owned-compute.fr").replace(
        "let dimensions = FieldDimensions(source: field)",
        "let dimensions = FieldDimensions(source: field)\nShow(geometry: self, scene: frame, values: copied, field: field, filtering: nearest_repeat, target: target.color)"),
        COMPUTED_DRAW.replace("target: attachment", "filtering: sampler, target: attachment")
            .replace("textureLoad(field, ivec2(0), 0)", "field.sample_level(filtering, vec2(0.25, 0.5), 0.0) + field.sample_grad(filtering, vec2(0.25, 0.5), vec2(0.0), vec2(0.0))"));
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), source.clone());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert!(output.wgsl.contains("textureSampleLevel"), "{renderer}");
        assert!(output.wgsl.contains("textureSampleGrad"), "{renderer}");
        let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
        let pass = manifest.surfaces[0]
            .mesh_passes
            .iter()
            .find(|pass| {
                pass.bindings
                    .iter()
                    .any(|binding| binding.name == "filtering")
            })
            .unwrap();
        let sampler = pass
            .bindings
            .iter()
            .find(|binding| binding.name == "filtering")
            .unwrap();
        assert_eq!(
            sampler.sampler,
            Some(fresco_artifact::types::SamplerPreset::NearestRepeat)
        );
        let invocation = manifest
            .renderers
            .iter()
            .filter(|renderer| renderer.selected)
            .flat_map(|renderer| &renderer.steps)
            .filter_map(|step| step.invocation.as_ref())
            .find(|invocation| invocation.operation == "Show")
            .unwrap();
        assert!(invocation.compute_inputs.contains_key("field"));
        assert!(!invocation.compute_inputs.contains_key("filtering"));
    }
}

#[test]
fn draw_calls_retain_typed_compute_handles_and_logical_dimensions() {
    let source = format!("{}\n{COMPUTED_DRAW}", include_str!("fixtures/owned-compute.fr").replace(
        "let dimensions = FieldDimensions(source: field)",
        "let dimensions = FieldDimensions(source: field)\nShow(geometry: self, scene: frame, values: copied, field: field, target: target.color)\nShow(geometry: self, scene: frame, values: copied, field: field, target: target.color)"));
    for renderer in ["forward", "forward-plus", "deferred"] {
        let manifest = compile_renderer(&source, renderer);
        let inferred_source = source
            .replace("at after_opaque as target {", "")
            .replace("target: target.color", "target: opaque.color")
            .replace("        }\n    }\n}", "    }\n}");
        assert_ne!(source, inferred_source);
        let inferred = compile_renderer(&inferred_source, renderer);
        assert_eq!(
            serde_json::to_value(&manifest.renderers).unwrap(),
            serde_json::to_value(&inferred.renderers).unwrap(),
            "explicit and resource-inferred compute/draw recipes: {renderer}"
        );
        assert_eq!(
            serde_json::to_value(&manifest.gpu_programs).unwrap(),
            serde_json::to_value(&inferred.gpu_programs).unwrap(),
            "placement must not change independent compute dependencies: {renderer}"
        );
        let copied = manifest
            .gpu_programs
            .iter()
            .find(|p| {
                p.compute_invocation
                    .as_ref()
                    .is_some_and(|i| i.operation == "CopyPoints")
            })
            .unwrap();
        let field = manifest
            .gpu_programs
            .iter()
            .find(|p| {
                p.compute_invocation
                    .as_ref()
                    .is_some_and(|i| i.operation == "MakeField")
            })
            .unwrap();
        let steps: Vec<_> = manifest
            .renderers
            .iter()
            .filter(|r| r.selected)
            .flat_map(|r| &r.steps)
            .filter(|s| s.invocation.as_ref().is_some_and(|i| i.operation == "Show"))
            .collect();
        assert_eq!(steps.len(), 2, "{renderer}");
        assert_ne!(steps[0].name, steps[1].name);
        for step in steps {
            let inputs = &step.invocation.as_ref().unwrap().compute_inputs;
            assert_eq!(inputs["values"].producer, copied.pass);
            assert_eq!(inputs["field"].producer, field.pass);
            assert_eq!(
                inputs["__fresco_values_count"].dimension,
                Some(fresco_artifact::ManifestComputeDimension::Count)
            );
            assert_eq!(
                inputs["__fresco_field_width"].dimension,
                Some(fresco_artifact::ManifestComputeDimension::Width)
            );
            assert!(
                !step.bindings.contains_key("values"),
                "owned handles are not renderer-global resource names"
            );
        }
    }
}

#[test]
fn compute_graph_separates_allocation_order_from_gpu_readiness() {
    use fresco_example_engine::runtime::compute_graph::{
        ComputeGraph, ComputePrerequisite as Dependency,
    };
    use std::collections::BTreeSet;
    let mut manifest = compile();
    // Serialization order must not supply producer/consumer edges.
    manifest.gpu_programs.reverse();
    let graph = ComputeGraph::new(&manifest, "item").unwrap();
    let mut completed = BTreeSet::new();
    let ready = |completed: &BTreeSet<_>| {
        graph
            .ready(completed)
            .map(|n| {
                n.program
                    .compute_invocation
                    .as_ref()
                    .unwrap()
                    .operation
                    .as_str()
            })
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        ready(&completed),
        BTreeSet::from(["MakeField", "FieldDimensions"])
    );
    let project = graph
        .allocation_order()
        .iter()
        .find(|n| n.program.compute_invocation.as_ref().unwrap().operation == "ProjectPoints")
        .unwrap();
    assert_eq!(project.prerequisites.len(), 1);
    assert!(matches!(
        project.prerequisites.first(),
        Some(Dependency::PreparedGeometry(_))
    ));
    completed.extend(project.prerequisites.clone());
    assert!(ready(&completed).contains("ProjectPoints"));
    assert!(!ready(&completed).contains("CopyPoints"));
    completed.insert(Dependency::Operation(project.program.pass.clone()));
    assert!(ready(&completed).contains("CopyPoints"));
    assert!(!ready(&completed).contains("ProjectPoints"));
    let ordered: Vec<_> = graph
        .allocation_order()
        .iter()
        .map(|n| {
            n.program
                .compute_invocation
                .as_ref()
                .unwrap()
                .operation
                .as_str()
        })
        .collect();
    assert!(
        ordered.iter().position(|n| *n == "MakeField")
            < ordered.iter().position(|n| *n == "FieldDimensions")
    );
}

#[test]
fn scene_compute_schedule_releases_early_work_and_keeps_object_identity() {
    use fresco_example_engine::runtime::compute_graph::{
        ComputeGraph, ComputePrerequisite as Dependency, ComputeSchedule, ScheduledCompute,
    };
    let manifest = compile();
    let graph = ComputeGraph::new(&manifest, "item").unwrap();
    let nodes = graph.allocation_order();
    let project = nodes
        .iter()
        .position(|n| n.program.compute_invocation.as_ref().unwrap().operation == "ProjectPoints")
        .unwrap();
    let copy = nodes
        .iter()
        .position(|n| n.program.compute_invocation.as_ref().unwrap().operation == "CopyPoints")
        .unwrap();
    let geometry = nodes[project]
        .prerequisites
        .iter()
        .find_map(|p| {
            if let Dependency::PreparedGeometry(name) = p {
                Some(name.as_str())
            } else {
                None
            }
        })
        .unwrap();
    let make_schedule = || {
        ComputeSchedule::new(
            (0..2)
                .map(|_| {
                    nodes
                        .iter()
                        .map(|node| ScheduledCompute {
                            name: &node.program.pass,
                            prerequisites: &node.prerequisites,
                        })
                        .collect()
                })
                .collect(),
        )
    };
    let mut schedule = make_schedule();
    let initial = schedule.ready();
    assert_eq!(
        initial.len(),
        4,
        "image producer and metadata query need no geometry or opaque completion"
    );
    assert!(initial.iter().all(|(_, n)| *n != project && *n != copy));
    assert!(schedule.finish().is_err());
    schedule.geometry_completed(0, geometry);
    assert_eq!(
        schedule.ready(),
        [(0, project), (0, copy)],
        "prepared compute and its data consumer must run before any opaque boundary"
    );
    assert!(
        !schedule.is_complete(1, &nodes[copy].program.pass),
        "same operation symbol must not release another object's consumer"
    );
    schedule.geometry_completed(1, geometry);
    assert_eq!(schedule.ready(), [(1, project), (1, copy)]);
    schedule.finish().unwrap();
    assert!(schedule.ready().is_empty(), "each invocation executes once");
    let mut next_view = make_schedule();
    assert_eq!(
        next_view.ready(),
        initial,
        "completion cannot leak into another frame/view"
    );

    let late = std::collections::BTreeSet::from([Dependency::EngineNode("opaque_finished".into())]);
    let mut schedule = ComputeSchedule::new(vec![vec![ScheduledCompute {
        name: "read_color",
        prerequisites: &late,
    }]]);
    schedule.engine_completed("opaque_started");
    assert!(schedule.ready().is_empty());
    assert!(schedule.finish().is_err());
    schedule.engine_completed("opaque_finished");
    assert_eq!(schedule.ready(), [(0, 0)]);
    schedule.finish().unwrap();
}

#[test]
fn compute_graph_rejects_missing_edges_foreign_handles_and_cycles() {
    use fresco_example_engine::runtime::compute_graph::ComputeGraph;
    let original = compile();
    for failure in [
        "missing edge",
        "missing argument",
        "foreign handle",
        "cycle",
    ] {
        let mut manifest = original.clone();
        let program = manifest
            .gpu_programs
            .iter_mut()
            .find(|p| {
                p.compute_invocation
                    .as_ref()
                    .is_some_and(|i| i.operation == "CopyPoints")
            })
            .unwrap();
        let invocation = program.compute_invocation.as_mut().unwrap();
        match failure {
            "missing edge" => invocation.dependencies.clear(),
            "missing argument" => {
                invocation.arguments.remove("source");
            }
            "foreign handle" => {
                invocation.arguments.insert(
                    "source".into(),
                    ManifestComputeArgument::Output {
                        producer: "another_material_output".into(),
                    },
                );
            }
            "cycle" => {
                invocation.arguments.insert(
                    "source".into(),
                    ManifestComputeArgument::Output {
                        producer: program.pass.clone(),
                    },
                );
                invocation.dependencies = vec![program.pass.clone()];
                invocation.metadata_dependencies = vec![program.pass.clone()];
            }
            _ => unreachable!(),
        }
        assert!(ComputeGraph::new(&manifest, "item").is_err(), "{failure}");
    }
}

#[test]
fn compute_settings_cannot_capture_an_unrelated_lane_or_type() {
    use fresco_example_engine::runtime::compute_graph::ComputeGraph;
    let original = compile();
    for failure in ["offset", "name", "type"] {
        let mut manifest = original.clone();
        let invocation = manifest
            .gpu_programs
            .iter_mut()
            .filter_map(|p| p.compute_invocation.as_mut())
            .find(|i| i.operation == "ProjectPoints")
            .unwrap();
        let ManifestComputeArgument::Setting { ty, name, offset } =
            invocation.arguments.get_mut("gain").unwrap()
        else {
            panic!("runtime setting capture");
        };
        match failure {
            "offset" => *offset += 1,
            "name" => *name = "another_setting".into(),
            "type" => *ty = "u32".into(),
            _ => unreachable!(),
        }
        assert!(ComputeGraph::new(&manifest, "item").is_err(), "{failure}");
    }
}

#[test]
fn compute_graph_plans_logical_handles_with_one_shared_budget() {
    use fresco_example_engine::runtime::compute_graph::ComputeGraph;
    let manifest = compile();
    let graph = ComputeGraph::new(&manifest, "item").unwrap();
    for size in [65, 0] {
        let read = |_program: &fresco_artifact::ManifestGpuProgram,
                    parameter: &str,
                    member: Option<&str>| {
            Ok(match (parameter, member) {
                ("mesh", Some("vertex_count")) => ComputeScalar::U32(7),
                ("size", None) => ComputeScalar::U32(size),
                ("gain", None) => ComputeScalar::F32(1.25),
                _ => panic!("owned output dimensions must resolve from producer metadata"),
            })
        };
        let budget = if size == 0 { 236 } else { 1524 };
        let plans = graph.plan_allocations(limits(), budget, &read).unwrap();
        assert_eq!(
            plans.values().map(|p| p.output.bytes()).sum::<u64>(),
            budget
        );
        assert!(graph.plan_allocations(limits(), budget - 1, &read).is_err());
        let dimensions = graph
            .allocation_order()
            .iter()
            .find(|n| n.program.compute_invocation.as_ref().unwrap().operation == "FieldDimensions")
            .unwrap();
        let OwnedAllocation::Buffer(buffer) = plans[dimensions.program.pass.as_str()].output else {
            panic!("buffer result")
        };
        assert_eq!(
            buffer.elements(),
            size,
            "logical zero must not become physical sentinel width one"
        );
    }
}

#[test]
fn owned_compute_artifacts_connect_prepared_geometry_settings_and_resource_producers() {
    let manifest = compile();
    let mut programs: Vec<_> = manifest
        .gpu_programs
        .iter()
        .filter(|p| p.compute_invocation.is_some())
        .collect();
    programs.sort_by_key(|p| p.compute_invocation.as_ref().unwrap().ordinal);
    assert_eq!(programs.len(), 4);
    let project = programs[0].compute_invocation.as_ref().unwrap();
    assert!(
        project.engine_dependencies.is_empty(),
        "lexical after_opaque placement must not delay independent compute work"
    );
    let ManifestComputeArgument::Geometry { producer, .. } = &project.arguments["mesh"] else {
        panic!("geometry provider");
    };
    assert!(
        manifest.surfaces[0]
            .mesh_passes
            .iter()
            .any(|p| p.pass == *producer && p.preparation.is_some()),
        "compute-only geometry demand must activate preparation"
    );
    assert!(
        matches!(&project.arguments["scene"], ManifestComputeArgument::External { resource, .. } if resource == "frame")
    );
    assert!(
        matches!(&project.arguments["gain"], ManifestComputeArgument::Setting { name, .. } if name == "gain")
    );
    assert_eq!(
        programs[1]
            .compute_invocation
            .as_ref()
            .unwrap()
            .dependencies,
        [programs[0].pass.clone()]
    );
    let query = programs[3].compute_invocation.as_ref().unwrap();
    assert!(query.dependencies.is_empty());
    assert_eq!(query.metadata_dependencies, [programs[2].pass.clone()]);
    let reader = |name: &str, member: Option<&str>| -> Result<ComputeScalar, RuntimeError> {
        match (name, member) {
            ("mesh", Some("vertex_count")) | ("source", Some("count")) => Ok(ComputeScalar::U32(7)),
            ("source", Some("width")) | ("size", None) => Ok(ComputeScalar::U32(65)),
            ("gain", None) => Ok(ComputeScalar::F32(1.25)),
            _ => Err(RuntimeError::PassPlan("unexpected host input".into())),
        }
    };
    let plans: Vec<_> = programs
        .iter()
        .map(|p| ComputeInvocationPlan::new(p, limits(), 4096, &reader).unwrap())
        .collect();
    assert_eq!(plans[0].output.bytes(), 7 * 16);
    assert_eq!(plans[0].dispatch.groups(), [1, 1, 1]);
    assert_eq!(plans[2].output.bytes(), 65 * 2 * 8);
    assert_eq!(plans[2].dispatch.groups(), [9, 1, 1]);
    assert!(ComputeInvocationPlan::new(programs[2], limits(), 1000, &reader).is_err());
    let empty =
        ComputeInvocationPlan::new(programs[2], limits(), 8, &|_, _| Ok(ComputeScalar::U32(0)))
            .unwrap();
    assert!(empty.dispatch.is_empty());
    assert!(
        matches!(empty.output, OwnedAllocation::Image { allocation, bytes: 8, .. } if allocation.logical() == [0, 2] && allocation.physical() == [1, 1])
    );
    assert!(
        ComputeInvocationPlan::new(programs[2], limits(), u64::MAX, &|_, _| Ok(
            ComputeScalar::U32(4097)
        ))
        .is_err(),
        "preconditions run before allocation"
    );
}

#[test]
fn authored_allocation_arithmetic_keeps_native_integer_precision() {
    let source = include_str!("fixtures/owned-compute.fr").replace(
        "output points: buffer<vec4, write>(mesh.vertex_count)",
        "output points: buffer<vec4, write>(16777217 + 1)",
    );
    let manifest = compile_source(&source);
    let program = manifest
        .gpu_programs
        .iter()
        .find(|p| {
            p.compute_invocation
                .as_ref()
                .is_some_and(|i| i.operation == "ProjectPoints")
        })
        .unwrap();
    let mut limits = limits();
    limits.max_storage_binding_bytes = 1 << 30;
    let plan = ComputeInvocationPlan::new(program, limits, 1 << 30, &|name, _| {
        Ok(if name == "gain" {
            ComputeScalar::F32(1.0)
        } else {
            ComputeScalar::U32(7)
        })
    })
    .unwrap();
    assert!(
        matches!(plan.output, OwnedAllocation::Buffer(buffer) if buffer.elements() == 16_777_218)
    );
    assert_eq!(plan.output.bytes(), 16_777_218 * 16);
}

#[test]
fn floating_allocation_arithmetic_requires_an_explicit_conversion() {
    let source = include_str!("fixtures/owned-compute.fr").replace("2u", "u32(0.7 + 0.7)");
    let manifest = compile_source(&source);
    let program = manifest
        .gpu_programs
        .iter()
        .find(|p| {
            p.compute_invocation
                .as_ref()
                .is_some_and(|i| i.operation == "MakeField")
        })
        .unwrap();
    let plan =
        ComputeInvocationPlan::new(program, limits(), 4096, &|_, _| Ok(ComputeScalar::U32(65)))
            .unwrap();
    assert!(
        matches!(plan.output, OwnedAllocation::Image { allocation, .. } if allocation.logical() == [65, 1])
    );
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        source.replace("u32(0.7 + 0.7)", "(0.7 + 0.7)"),
    );
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(format!("{errors:?}").contains("logical extents require u32"));
}

#[test]
fn generated_shells_capture_offsets_and_owned_vertex_buffers_in_every_renderer() {
    let source = include_str!("fixtures/generated-shells.fr");
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), source.into());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let draws: Vec<_> = manifest
            .renderers
            .iter()
            .flat_map(|r| &r.steps)
            .filter_map(|s| s.invocation.as_ref())
            .filter(|i| i.generated_vertices.is_some())
            .collect();
        assert_eq!(draws.len(), 3);
        for (layer, invocation) in draws.iter().enumerate() {
            let generated = invocation.generated_vertices.as_ref().unwrap();
            assert_eq!(generated.binding, "vertices");
            assert_eq!(invocation.bounds.as_ref().unwrap().geometry, "geometry");
            assert_eq!(invocation.sort_geometry.as_deref(), Some("geometry"));
            assert_eq!(invocation.host.as_ref().unwrap().requirements.len(), 2);
            assert!(
                matches!(&invocation.host.as_ref().unwrap().arguments["limit"], ManifestComputeArgument::Setting {ty, name, ..} if ty == "u32" && name == "limit")
            );
            assert!(invocation.compute_inputs.contains_key(&generated.binding));
            assert!(invocation.preparation.is_some());
            assert!(
                matches!(&invocation.host.as_ref().unwrap().arguments["layer"], ManifestComputeArgument::Constant { value, .. } if value.as_u64() == Some(layer as u64))
            );
        }
        for (from, to, diagnostic) in [
            (
                "geometry.bounds.expand_world(expansion)",
                "geometry.vertices.expand_world(expansion)",
                "engine-declared prepared geometry bounds member",
            ),
            (
                "geometry.bounds.expand_world(expansion)",
                "geometry.bounds.expand_world(layer)",
                "host f32 distance",
            ),
            (
                "sort_position: geometry.bounds.center",
                "sort_position: view.camera_pos",
                "sort_position requires geometry.bounds.center",
            ),
            (
                "requires layer < limit",
                "requires layer",
                "draw preconditions require bool",
            ),
            (
                "requires layer < limit",
                "requires vertices[0u].x > 0.0",
                "cannot read GPU values",
            ),
            (
                "using vertices base_vertex",
                "using view base_vertex",
                "read-only buffer parameter",
            ),
            (
                "base_vertex checked_mul(layer, geometry.vertex_count)",
                "base_vertex 0.5",
                "offset must be u32",
            ),
            (
                "base_vertex checked_mul(layer, geometry.vertex_count)",
                "base_vertex vertices[0u].x",
                "cannot read GPU values",
            ),
        ] {
            files.insert("main.fr".into(), source.replace(from, to));
            let errors =
                fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
            assert!(format!("{errors:?}").contains(diagnostic), "{errors:?}");
        }
    }
}
