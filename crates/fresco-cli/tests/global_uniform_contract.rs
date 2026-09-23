#[path = "support/common.rs"]
mod common;

use fresco::driver::compile_source_bundle;
use serde_json::{Value, json};

#[test]
fn spatial_uniform_fields_keep_their_layout_and_following_fields() {
    let source = r#"
struct SceneData {
    gain: f32
    position: vec3 in world
    weight: f32
    viewport: vec2 in screen
}
param scene: SceneData
canvas sample(uv: coord) -> color {
    compose { circle(at: center, radius: (scene.position.x + scene.weight + scene.viewport.y) * scene.gain) |> fill(#fff) }
}

"#;
    let output = compile_source_bundle(source, common::TEST_SOURCE_PATH, false)
        .unwrap_or_else(|diags| panic!("{diags:?}"));
    let manifest: Value = serde_json::from_str(&output.manifest).unwrap();
    let uniform = &manifest["canvases"][0]["global_uniforms"][0];
    assert_eq!(uniform["byte_size"], 48);
    let fields = uniform["fields"].as_array().unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|field| (&field["offset"], &field["components"]))
            .map(|(offset, components)| json!([offset, components]))
            .collect::<Vec<_>>(),
        vec![
            json!([0, 1]),
            json!([16, 3]),
            json!([28, 1]),
            json!([32, 2])
        ]
    );

    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let (_, global) = module
        .global_variables
        .iter()
        .find(|(_, global)| global.name.as_deref() == Some("scene"))
        .unwrap();
    assert_eq!(global.binding.as_ref().unwrap().group, 3);
    if let naga::TypeInner::Struct { members, span } = &module.types[global.ty].inner {
        assert_eq!(*span, 48);
        assert_eq!(
            members
                .iter()
                .map(|member| member.offset)
                .collect::<Vec<_>>(),
            vec![0, 16, 28, 32]
        );
    } else {
        panic!("global uniform must lower to a struct");
    }
}

#[test]
fn engine_accessors_can_use_custom_buffer_and_field_names() {
    let source = r#"
struct Clock {
    seconds: f32
    step: f32
    viewport: vec2
}

param clock: Clock
fn time() -> f32 { return clock.seconds * 2.0 }
fn delta_time() -> f32 { return clock.step }
fn resolution() -> vec2 { return clock.viewport }
canvas sample(uv: coord, t: signal) -> color {
    compose { circle(at: center, radius: t * 0.01) |> fill(#fff) }
}
"#;
    let output = compile_source_bundle(source, common::TEST_SOURCE_PATH, false)
        .unwrap_or_else(|diags| panic!("{diags:?}"));
    assert!(output.wgsl.contains("clock.seconds"));
    let manifest: Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        manifest["canvases"][0]["global_uniforms"][0]["name"],
        "clock"
    );
}

#[test]
fn named_surface_response_uses_its_declared_material_properties_and_shader() {
    let source = r#"
material_properties pigment {
    channel albedo: color = #fff
    channel opacity: f32 = 1.0
    channel normal: vec3 in world = sp.normal
}
schema_expression ink for pigment {
    shade: (albedo.r * 0.123, albedo.g, albedo.b, opacity)
}
surface sample(sp: surf) -> material(ink) {
    compose { base(albedo: #ff0000) }
}
"#;
    let output = compile_source_bundle(source, common::TEST_SOURCE_PATH, false)
        .unwrap_or_else(|diags| panic!("{diags:?}"));
    let manifest: Value = serde_json::from_str(&output.manifest).unwrap();
    let surface = &manifest["surfaces"][0];
    assert_eq!(surface["material_properties"], "pigment");
    assert_eq!(surface["surface_shader"], "ink");
    assert_eq!(
        surface["evaluation_shader_entry"],
        "fresco_evaluation_shader_sample"
    );
    assert!(
        output.wgsl.contains("0.123"),
        "authored response must reach WGSL"
    );
    let changed = compile_source_bundle(
        &source.replace("0.123", "0.456"),
        common::TEST_SOURCE_PATH,
        false,
    )
    .unwrap();
    assert_ne!(output.wgsl, changed.wgsl);
}

#[test]
fn standalone_canvas_reads_runtime_arguments_instead_of_zero() {
    let output = compile_source_bundle(
        r#"
canvas animated(uv: coord, t: signal, dt: delta, viewport: resolution) -> color {
    compose { circle(at: center, radius: sin(t) * dt + viewport.x / viewport.y) |> fill(#fff) }
}
"#,
        common::TEST_SOURCE_PATH,
        false,
    )
    .unwrap();
    assert!(
        output.wgsl.contains("sin(t)"),
        "time must stay dynamic: {}",
        output.wgsl
    );
    assert!(
        output.wgsl.contains("ctx.delta"),
        "delta must come from the entry context"
    );
    assert!(
        output.wgsl.contains("ctx.res"),
        "resolution must come from the entry context"
    );
    let manifest: Value = serde_json::from_str(&output.manifest).unwrap();
    assert!(manifest["canvases"][0]["global_uniforms"].is_null());
}

#[test]
fn incorrectly_typed_engine_accessor_is_not_replaced_by_an_entry_argument() {
    let errors = compile_source_bundle(
        r#"
fn time() -> vec2 { return (1.0, 2.0) }
canvas broken(uv: coord, t: signal) -> color {
    compose { circle(at: center, radius: t) |> fill(#fff) }
}
"#,
        common::TEST_SOURCE_PATH,
        false,
    )
    .unwrap_err();
    assert!(errors.iter().any(|diag| {
        diag.message
            .contains("runtime accessor `time` must return scalar")
    }));
}

#[test]
fn standalone_helpers_capture_runtime_inputs_transitively() {
    let output = compile_source_bundle(
        r#"
fn animated(x: f32) -> f32 { return x * sin(time) * dt + viewport.x / viewport.y + coord.x }
fn outer(x: f32) -> f32 { return animated(x) + 0.01 }
canvas t(uv: coord, time: signal, dt: delta, viewport: resolution) -> color {
    compose { circle(at: center, radius: outer(uv.x)) |> fill(#fff) }
}
"#,
        common::TEST_SOURCE_PATH,
        false,
    )
    .unwrap();
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    for name in ["animated", "outer"] {
        let (_, helper) = module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref().is_some_and(|n| n.contains(name)))
            .unwrap();
        assert_eq!(
            helper.arguments.len(),
            5,
            "{name} must capture coordinate, time, delta, and resolution"
        );
    }
    assert!(output.wgsl.contains("sin(entry_time)"), "{}", output.wgsl);
    assert!(!output.wgsl.contains("sin(0f)"));
    assert!(output.wgsl.contains("entry_coord.x"));
    assert!(output.wgsl.contains("ctx.delta"));
}
