#[path = "support/common.rs"]
mod common;

use common::wgsl::{Capabilities, assert_valid_wgsl};

use fresco::driver::{compile_bundle_virtual, compile_source_bundle};
use serde_json::{Value, json};
use std::collections::HashMap;

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
canvas sample(ctx: CanvasContext) -> color { let uv = context(coord);
    compose { circle(at: center, radius: (scene.position.x + scene.weight + scene.viewport.y) * scene.gain) |> fill(#fff) }
}

"#;
    let output = compile_source_bundle(source, common::TEST_SOURCE_PATH, false)
        .unwrap_or_else(|diags| panic!("{diags:?}"));
    let manifest: Value = serde_json::from_str(&output.manifest).unwrap();
    let uniform = &manifest["raster_entries"][0]["global_uniforms"][0];
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

    let module = assert_valid_wgsl(&output.wgsl, Capabilities::all());
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

/// An engine whose `canvas` context supplies runtime roles through custom
/// buffer and field names; `time_ty` lets a test mistype the time role.
fn clock_engine(source: &str, time_ty: &str) -> HashMap<String, String> {
    let policy = include_str!("../../fresco/tests/fixtures/engines/policy/engine/engine.fr");
    HashMap::from([
        (
            "engine/engine.fr".into(),
            format!(
                r#"{policy}
struct Clock {{
    @semantic(time) seconds: {time_ty}
    @semantic(delta_time) step: f32
    @semantic(resolution) viewport: vec2
}}
struct CanvasContext {{
    @semantic(coord) uv: vec2
    clock: Clock
}}
@entry(canvas, draw)
interface Canvas {{
    fn draw(@context ctx: CanvasContext) -> color
}}
"#
            ),
        ),
        ("main.fr".into(), source.into()),
    ])
}

#[test]
fn engine_context_roles_can_use_custom_buffer_and_field_names() {
    let source = r#"
canvas sample(ctx: CanvasContext) -> color { let t = context(time);
    compose { circle(at: center, radius: t * 0.01 + context(delta_time) + context(resolution).x) |> fill(#fff) }
}
"#;
    let output = compile_bundle_virtual(&clock_engine(source, "f32"), "main.fr", false)
        .unwrap_or_else(|diags| panic!("{diags:?}"));
    for field in [
        "ctx_1.clock.seconds",
        "ctx_1.clock.step",
        "ctx_1.clock.viewport",
    ] {
        assert!(
            output.wgsl.contains(field),
            "context role must resolve through `{field}`: {}",
            output.wgsl
        );
    }
    let manifest: Value = serde_json::from_str(&output.manifest).unwrap();
    assert!(manifest["raster_entries"][0]["global_uniforms"].is_null());
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
fn canvas_reads_runtime_context_instead_of_zero() {
    let output = compile_source_bundle(
        r#"
canvas animated(ctx: CanvasContext) -> color { let uv = context(coord); let t = context(time); let dt = context(delta_time); let viewport = context(resolution);
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
    assert!(manifest["raster_entries"][0]["global_uniforms"].is_null());
}

#[test]
fn incorrectly_typed_engine_context_role_is_rejected() {
    let source = r#"
canvas broken(ctx: CanvasContext) -> color { let t = context(time);
    compose { circle(at: center, radius: t) |> fill(#fff) }
}
"#;
    let errors =
        compile_bundle_virtual(&clock_engine(source, "vec2"), "main.fr", false).unwrap_err();
    assert!(
        errors.iter().any(|diag| {
            diag.message
                .contains("unknown or incorrectly typed context semantic `time`")
        }),
        "{errors:?}"
    );
}

#[test]
fn helpers_receive_canvas_context_inputs_explicitly() {
    let output = compile_source_bundle(
        r#"
fn animated(x: f32, time: f32, dt: f32, viewport: vec2, coord: vec2) -> f32 { return x * sin(time) * dt + viewport.x / viewport.y + coord.x }
fn outer(x: f32, time: f32, dt: f32, viewport: vec2, coord: vec2) -> f32 { return animated(x, time, dt, viewport, coord) + 0.01 }
canvas t(ctx: CanvasContext) -> color { let uv = context(coord);
    compose { circle(at: center, radius: outer(uv.x, context(time), context(delta_time), context(resolution), uv)) |> fill(#fff) }
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
            "{name} must take coordinate, time, delta, and resolution explicitly"
        );
    }
    assert!(output.wgsl.contains("sin(arg_time)"), "{}", output.wgsl);
    assert!(!output.wgsl.contains("sin(0f)"));
    assert!(output.wgsl.contains("ctx.delta"));
    assert!(output.wgsl.contains("ctx.res"));
}

#[test]
fn helpers_do_not_capture_entry_bindings_implicitly() {
    let errors = compile_source_bundle(
        r#"
fn animated(x: f32) -> f32 { return x * sin(time) }
canvas t(ctx: CanvasContext) -> color { let time = context(time);
    compose { circle(at: center, radius: animated(0.1)) |> fill(#fff) }
}
"#,
        common::TEST_SOURCE_PATH,
        false,
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|diag| diag.message.contains("unknown name `time`")),
        "{errors:?}"
    );
}
