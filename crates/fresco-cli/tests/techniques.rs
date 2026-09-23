use serde_json::Value;
use std::collections::HashMap;

const GRAPH: &str = r#"
@shader pass positions {
    stage: compute
    binding { @group(0) @binding(0) @access(write) values: buffer<vec4> }
    @compute @workgroup_size(8)
    fn build(@builtin(global_invocation_id) id: uvec3) {
        if id.x < u32(16) { values[id.x] = vec4(f32(id.x), 0.0, 0.0, 1.0) }
    }
}
@shader pass visibility {
    stage: compute
    binding { @group(0) @binding(0) @access(write) values: buffer<u32> }
    @compute @workgroup_size(4)
    fn build(@builtin(global_invocation_id) id: uvec3) {
        if id.x < u32(16) { values[id.x] = u32(1) }
    }
}
struct Screen { @builtin(position) position: vec4 }
@shader pass composite {
    stage: raster
    draw: fullscreen
    binding {
        @group(0) @binding(0) positions: buffer<vec4>
        @group(0) @binding(1) visibility: buffer<u32>
    }
    @vertex fn vertex(@builtin(vertex_index) id: u32) -> Screen {
        return Screen(positions[id])
    }
    @fragment fn fragment() -> vec4 {
        return vec4(f32(visibility[u32(0)]))
    }
}
@technique
@buffer(points, 256) @buffer(mask, 64) @image(color, rgba16float)
@pool(points, geometry_scratch)
@output(result, color)
pipeline(compute) prepared_mesh {
    @draw(vertex, fragment, fullscreen, 3)
    @instances(16) @bind(positions, points) @bind(visibility, mask)
    @color(0, color) composite
    @dispatch(build, 16, 1, 1) @bind(values, points) positions
    @dispatch(build, 16, 1, 1) @bind(values, mask) visibility
}
"#;

fn compile(
    source: &str,
) -> Result<fresco::driver::CompileBundleOutput, Vec<fresco::driver::DiagnosticRecord>> {
    let files = HashMap::from([
        (
            "engine/engine.fr".into(),
            include_str!("../../../tests/render-policy/engine/engine.fr").into(),
        ),
        ("main.fr".into(), source.into()),
    ]);
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
}

#[test]
fn two_independent_compute_outputs_join_at_a_draw_without_a_material() {
    let output = compile(GRAPH).unwrap();
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    let graph = &root["techniques"][0];
    assert_eq!(graph["name"], "prepared_mesh");
    assert_eq!(graph["steps"][0]["name"], "positions");
    assert_eq!(graph["steps"][1]["name"], "visibility");
    assert_eq!(graph["steps"][0]["after"], serde_json::json!([]));
    assert_eq!(graph["steps"][1]["after"], serde_json::json!([]));
    assert_eq!(
        graph["steps"][2]["after"],
        serde_json::json!(["positions", "visibility"])
    );
    assert_eq!(graph["steps"][2]["operation"]["instances"], 16);
    assert_eq!(
        graph["steps"][0]["operation"]["extent"],
        serde_json::json!([16, 1, 1])
    );
    assert_eq!(graph["outputs"]["result"], "color");
    assert_eq!(graph["resources"][0]["source"]["pool"], "geometry_scratch");
}

#[test]
fn integer_vector_attachments_and_sampled_images_use_registry_types() {
    let source = r#"
struct Screen { @builtin(position) position: vec4 }
@shader pass write_values {
    stage: raster
    draw: fullscreen
    @vertex fn vertex(@builtin(vertex_index) id: u32) -> Screen { return Screen(vec4(0.0)) }
    @fragment fn fragment() -> uvec2 { return uvec2(3u, 5u) }
}
@shader pass read_values {
    stage: raster
    draw: fullscreen
    binding { @group(0) @binding(0) values: texture_2d<u32> }
    @vertex fn vertex(@builtin(vertex_index) id: u32) -> Screen { return Screen(vec4(0.0)) }
    @fragment fn fragment() -> vec4 { return vec4(f32(textureLoad(values, ivec2(0), 0).y)) }
}
@technique @image(data, rg32uint) @image(color, rgba8unorm) @output(result, color)
pipeline(postprocess) integer_image {
    @draw(vertex, fragment, fullscreen, 3) @color(0, data) write_values
    @draw(vertex, fragment, fullscreen, 3) @bind(values, data) @color(0, color) read_values
}
"#;
    for source in [
        source.to_string(),
        source
            .replace("rg32uint", "rgba32uint")
            .replace("uvec2", "uvec4")
            .replace("3u, 5u", "3u, 5u, 0u, 1u"),
    ] {
        let output = compile(&source).unwrap();
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let errors = compile(&source.replace("uint)", "float)")).unwrap_err();
        assert!(
            format!("{errors:?}")
                .contains("fragment output and attachment format are incompatible")
        );
    }
}

#[test]
fn conflicting_writes_and_undefined_reads_are_diagnosed() {
    for (source, expected) in [
        (GRAPH.replace("@bind(values, mask) visibility", "@bind(values, mask) visibility\n@node(second) @dispatch(build, 16, 1, 1) @bind(values, mask) visibility"), "conflicting writes"),
        (GRAPH.replace("@dispatch(build, 16, 1, 1) @bind(values, mask) visibility", ""), "without a producer"),
        (GRAPH.replace("@bind(values, mask) visibility", "@after(composite) @bind(values, mask) visibility"), "cycle"),
        (GRAPH.replace("@buffer(points, 256)", "@image(points, rgba16float)"), "types disagree"),
    ] {
        let errors = compile(&source).unwrap_err();
        assert!(errors.iter().any(|e| e.message.contains(expected)), "{expected}: {errors:?}");
    }
}

#[test]
fn write_only_is_checked_against_actual_shader_reads() {
    let source = GRAPH.replace(
        "values[id.x] = u32(1)",
        "values[id.x] = values[id.x] + u32(1)",
    );
    let errors = compile(&source).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("write-only resource")),
        "{errors:?}"
    );
}

#[test]
fn engine_providers_replace_producers_and_assets_keep_their_identity() {
    let source = GRAPH
        .replace("@dispatch(build, 16, 1, 1) @bind(values, mask) visibility", "")
        .replace("@output(result, color)", "@output(result, color) @provider(mask, engine_visibility) @asset(color, authored_target)");
    let output = compile(&source).unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    let resources = root["techniques"][0]["resources"].as_array().unwrap();
    assert_eq!(
        resources[1]["source"],
        serde_json::json!({"kind": "external", "provider": "engine_visibility"})
    );
    assert_eq!(
        resources[2]["source"],
        serde_json::json!({"kind": "asset", "asset": "authored_target"})
    );
}

#[test]
fn compute_only_techniques_have_no_required_draw_or_surface() {
    let source = GRAPH[..GRAPH.find("struct Screen").unwrap()].to_owned()
        + r#"
@technique @buffer(data, 256) @output(result, data)
pipeline(compute) bake {
    @dispatch(build, 16, 1, 1) @bind(values, data) positions
}
"#;
    let output = compile(&source).unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(root["techniques"][0]["steps"].as_array().unwrap().len(), 1);
    assert_eq!(root["techniques"][0]["outputs"]["result"], "data");
}

#[test]
fn importing_a_technique_output_preserves_instance_identity_and_checks_types() {
    let source = GRAPH
        .replace(
            "@dispatch(build, 16, 1, 1) @bind(values, mask) visibility",
            "",
        )
        .replace(
            "@output(result, color)",
            "@output(result, color) @from(mask, scene_cull, bake_visibility, result)",
        )
        + r#"
@technique @buffer(bits, 64) @output(result, bits)
pipeline(compute) bake_visibility {
    @dispatch(build, 16, 1, 1) @bind(values, bits) visibility
}
"#;
    let output = compile(&source).unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        root["techniques"][0]["resources"][1]["source"],
        serde_json::json!({"kind": "technique_output", "instance": "scene_cull", "technique": "bake_visibility", "output": "result"})
    );
    for (changed, expected) in [
        (
            source.replace(
                "scene_cull, bake_visibility, result",
                "scene_cull, missing, result",
            ),
            "unknown producer",
        ),
        (
            source.replace(
                "scene_cull, bake_visibility, result",
                "scene_cull, bake_visibility, missing",
            ),
            "unknown technique output",
        ),
        (
            source.replace("@buffer(bits, 64)", "@buffer(bits, 128)"),
            "incompatible imported output",
        ),
    ] {
        let errors = compile(&changed).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{expected}: {errors:?}"
        );
    }
}

#[test]
fn image_dimensions_and_buffer_layouts_are_explicit_and_checked() {
    let output = compile(&GRAPH.replace(
        "@output(result, color)",
        "@output(result, color) @dimensions(color, 128, 64)",
    ))
    .unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        root["techniques"][0]["resources"][2]["descriptor"]["extent"],
        serde_json::json!({"kind": "fixed", "width": 128, "height": 64})
    );
    for (source, expected) in [
        (
            GRAPH.replace("@buffer(points, 256)", "@buffer(points, 12)"),
            "element stride",
        ),
        (
            GRAPH.replace(
                "@output(result, color)",
                "@output(result, color) @dimensions(color, 0, 64)",
            ),
            "dimensions must be positive",
        ),
        (
            GRAPH.replace(
                "@output(result, color)",
                "@output(result, color) @dimensions(mask, 128, 64)",
            ),
            "declared image",
        ),
        (
            GRAPH.replace(
                "@pool(points, geometry_scratch)",
                "@pool(missing, geometry_scratch)",
            ),
            "undefined resource",
        ),
        (
            GRAPH.replace(
                "@output(result, color)",
                "@output(result, color) @provider(points, supplied)",
            ),
            "cannot request a pool",
        ),
    ] {
        let errors = compile(&source).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{expected}: {errors:?}"
        );
    }
}

#[test]
fn renderer_selection_does_not_strip_independent_technique_programs() {
    let source = GRAPH.to_owned()
        + r#"
@renderer("plain", "Plain") @default
@image(target, rgba16float)
@external(points, external_positions) @external(mask, external_mask)
pipeline(postprocess) renderer {
    @draw(vertex, fragment, fullscreen) @color(0, target)
    @bind(positions, points) @bind(visibility, mask) composite
}
"#;
    let output = compile(&source).unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    let programs = root["gpu_programs"].as_array().unwrap();
    assert!(programs.iter().any(|p| p["pass"] == "positions"));
    assert!(programs.iter().any(|p| p["pass"] == "visibility"));
    assert_eq!(root["techniques"][0]["steps"].as_array().unwrap().len(), 3);
}

#[test]
fn only_selected_shader_entries_contribute_resource_dependencies() {
    let source = GRAPH
        .replace(
            "@fragment fn fragment()",
            "@fragment fn plain() -> vec4 { return vec4(1.0) }\n    @fragment fn fragment()",
        )
        .replace(
            "@draw(vertex, fragment, fullscreen, 3)",
            "@draw(vertex, plain, fullscreen, 3)",
        )
        .replace(
            "@dispatch(build, 16, 1, 1) @bind(values, mask) visibility",
            "",
        );
    let output = compile(&source).unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        root["techniques"][0]["steps"][1]["after"],
        serde_json::json!(["positions"])
    );
    assert_eq!(
        root["techniques"][0]["steps"][1]["reads"],
        serde_json::json!(["points"])
    );
}

#[test]
fn inter_technique_cycles_are_rejected() {
    let source = r#"
@shader pass copy {
    stage: compute
    binding {
        @group(0) @binding(0) input: buffer<u32>
        @group(0) @binding(1) @access(write) output: buffer<u32>
    }
    @compute @workgroup_size(1)
    fn run(@builtin(global_invocation_id) id: uvec3) { output[id.x] = input[id.x] }
}
@technique @buffer(input, 4) @buffer(output, 4) @output(result, output)
@from(input, other, second, result)
pipeline(compute) first { @dispatch(run, 1, 1, 1) @bind(input, input) @bind(output, output) copy }
@technique @buffer(input, 4) @buffer(output, 4) @output(result, output)
@from(input, other, first, result)
pipeline(compute) second { @dispatch(run, 1, 1, 1) @bind(input, input) @bind(output, output) copy }
"#;
    let errors = compile(source).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("cycle between technique")),
        "{errors:?}"
    );
}

#[test]
fn shader_helpers_use_the_shared_typed_function_checker() {
    let source = r#"
struct State { position: vec4, heat: f32 }
fn change<T>(value: T, dt: f32) -> T { var next = value; next.heat = value.heat + dt; return next }
@shader pass evolve {
    stage: compute
    binding { @group(0) @binding(0) @access(read_write) state: buffer<State> }
    @pure fn advance(value: State, dt: f32) -> State { return change(value, dt) }
    @compute @workgroup_size(8)
    fn run(@builtin(global_invocation_id) id: uvec3) { state[id.x] = advance(state[id.x], 0.25) }
}
@technique @external(values, supplied)
pipeline(compute) update { @dispatch(run, 8, 1, 1) @bind(state, values) evolve }
"#;
    let output = compile(source).unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(root["gpu_programs"][0]["bindings"][0]["element_stride"], 32);
    assert_eq!(
        root["gpu_programs"][0]["bindings"][0]["fields"][1]["offset"],
        16
    );
    assert_eq!(
        root["techniques"][0]["steps"][0]["reads"],
        serde_json::json!(["values"])
    );
    assert_eq!(
        root["techniques"][0]["steps"][0]["writes"],
        serde_json::json!(["values"])
    );
}

#[test]
fn invocation_parameters_ordered_updates_and_void_returns_are_generic() {
    let source = r#"
@shader @meta(reservation, 64) pass update {
    stage: compute
    binding { @group(0) @binding(0) @access(read_write) values: buffer<u32> }
    @compute @workgroup_size(8)
    fn run(@builtin(global_invocation_id) id: uvec3) {
        if id.x >= u32(64) { return }
        values[id.x] = values[id.x] + u32(1)
    }
}
@technique @meta(owner, "application") @external(state, persistent)
pipeline(compute) evolve {
    @node(first) @enabled(reset) @dispatch(run, count) @bind(values, state) update
    @node(second) @after(first) @dispatch(run, count) @bind(values, state) update
}
"#;
    let output = compile(source).unwrap();
    let root: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let graph = &root.techniques[0];
    assert_eq!(graph.metadata["owner"], "application");
    assert_eq!(graph.steps[0].enabled.as_deref(), Some("reset"));
    assert_eq!(graph.steps[1].after, ["first"]);
    assert!(graph.steps[0].operation.workgroups().is_err());
    let roundtrip: fresco_artifact::ManifestRoot =
        serde_json::from_value(serde_json::to_value(&root).unwrap()).unwrap();
    assert_eq!(roundtrip.techniques[0].steps.len(), 2);
    for invalid in [
        source.replace("@after(first)", ""),
        source.replace("@dispatch(run, count)", "@dispatch(run, -1)"),
    ] {
        assert!(compile(&invalid).is_err());
    }
}

#[test]
fn generic_draw_contract_roundtrips_numeric_attachment_slots() {
    let output = compile(GRAPH).unwrap();
    let root: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let value = serde_json::to_value(&root).unwrap();
    let decoded: fresco_artifact::ManifestRoot = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}

#[test]
fn attachment_descriptors_do_not_depend_on_provider_names() {
    let source = GRAPH.replace(
        "@image(color, rgba16float)",
        "@image(color, float_color) @provider(color, arbitrary_target)",
    );
    let output = compile(&source).unwrap();
    let root: Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        root["techniques"][0]["resources"][2]["descriptor"]["format"],
        "float_color"
    );
    assert_eq!(
        root["techniques"][0]["resources"][2]["source"]["provider"],
        "arbitrary_target"
    );
    for replacement in [
        "@external(color, presentation)",
        "@external(color, depth)",
        "@image(color, float_color)",
    ] {
        assert!(
            compile(&GRAPH.replace("@image(color, rgba16float)", replacement)).is_err(),
            "{replacement}"
        );
    }
}

#[test]
fn depth_only_technique_has_no_fragment_entry_and_accepts_arbitrary_provider() {
    let source = r#"
struct Vertex { @builtin(position) position: vec4 }
@shader pass geometry {
    stage: raster
    @vertex fn vertex(@builtin(vertex_index) id: u32) -> Vertex {
        return Vertex(vec4(f32(id), 0.0, 0.5, 1.0))
    }
}
@technique @image(z, depth32float) @provider(z, arbitrary_depth) @output(result, z)
pipeline(compute) depth_pass {
    @draw_depth(vertex, 3) @depth(z) geometry
}
"#;
    let output = compile(source).unwrap();
    let root: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    assert!(matches!(&root.techniques[0].steps[0].operation,
        fresco_artifact::ManifestTechniqueOperation::Draw { fragment: None, depth: Some(depth), colors, .. }
        if depth == "z" && colors.is_empty()));
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    for invalid in [
        source.replace("@depth(z)", ""),
        source.replace("@depth(z)", "@depth(z) @color(0, z)"),
        source.replace("depth32float", "rgba16float"),
    ] {
        assert!(compile(&invalid).is_err());
    }
}
