#![cfg(feature = "runtime")]
use fresco_example_engine::runtime::forward_plus::{
    MAX_LIGHTS, PointLight, demo_lights, pack_lights, tile_grid,
};

#[test]
fn light_upload_is_bounded_validated_and_zero_padded() {
    let light = demo_lights()[0];
    let bytes = pack_lights(&[light]).unwrap();
    assert_eq!(bytes.len(), MAX_LIGHTS * 32);
    assert_eq!(
        f32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        light.radius
    );
    assert!(bytes[32..].iter().all(|v| *v == 0));
    assert!(pack_lights(&[]).unwrap().iter().all(|v| *v == 0));
    assert!(pack_lights(&[light; MAX_LIGHTS]).is_ok());
    assert!(pack_lights(&[light; MAX_LIGHTS + 1]).is_err());
    for invalid in [
        PointLight {
            radius: 0.0,
            ..light
        },
        PointLight {
            radius: -1.0,
            ..light
        },
        PointLight {
            radius: f32::INFINITY,
            ..light
        },
        PointLight {
            position: [f32::NAN, 0.0, 0.0],
            ..light
        },
        PointLight {
            intensity: -1.0,
            ..light
        },
        PointLight {
            color: [-1.0, 0.0, 0.0],
            ..light
        },
    ] {
        assert!(pack_lights(&[invalid]).is_err());
    }
}

#[test]
fn tiles_include_partial_edges_and_reject_oversized_frames() {
    assert_eq!(tile_grid([0, 0]).unwrap(), [0, 0]);
    assert_eq!(tile_grid([1, 17]).unwrap(), [1, 2]);
    assert_eq!(tile_grid([8192, 8192]).unwrap(), [512, 512]);
    assert!(tile_grid([8193, 1]).is_err());
}

#[test]
fn authored_forward_plus_material_and_compute_are_valid_wgsl() {
    let mut files = fresco_example_engine::source_files_for_renderer(true);
    files.insert(
        "main.fr".into(),
        include_str!("fixtures/forward-plus.fr").into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let source = output.wgsl.as_str();
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    for binding in ["_point_lights", "_light_tiles"] {
        let buffers: Vec<_> = module
            .global_variables
            .iter()
            .filter(|(_, global)| {
                global.name.as_deref().is_some_and(|name| {
                    name.starts_with("fresco_resource_") && name.ends_with(binding)
                })
            })
            .collect();
        assert_eq!(buffers.len(), 1, "mesh binding {binding}");
        assert_eq!(
            buffers[0].1.space,
            naga::AddressSpace::Storage {
                access: naga::StorageAccess::LOAD
            }
        );
    }
    assert!(output.wgsl.contains("for (var i"));
}

#[test]
fn renderer_configuration_changes_shader_without_material_edits() {
    let source = include_str!("fixtures/forward-plus.fr");
    let mut shaders = Vec::new();
    for enabled in [false, true] {
        let mut files = fresco_example_engine::source_files_for_renderer(enabled);
        files.insert("main.fr".into(), source.into());
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
        let property = manifest["surfaces"][0]["settings"]["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "forward_plus")
            .unwrap();
        assert_eq!(property["value"], if enabled { 1.0 } else { 0.0 });
        assert_eq!(property["editable"], false);
        assert_eq!(files["main.fr"], source);
        shaders.push(output.wgsl);
        files.insert("main.fr".into(), "surface bad(sp: surf) -> material(standard) { properties { forward_plus: true }; compose { base(albedo: rgba(1.0, 1.0, 1.0, 1.0)) } }".into());
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            format!("{errors:?}").contains("engine-owned"),
            "{errors:#?}"
        );
    }
    assert_ne!(shaders[0], shaders[1]);
}
