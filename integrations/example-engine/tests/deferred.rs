#![cfg(feature = "runtime")]
use fresco_artifact::ManifestRoot;
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::deferred::inverse_view_projection,
};

#[test]
fn deferred_contract_has_integer_ids_and_valid_wgsl() {
    let mut files = fresco_example_engine::source_files_for_deferred();
    files.insert("main.fr".into(), "surface zebra(sp: surf) -> material(standard) { compose { base(albedo: rgba(0.7, 0.3, 0.1, 1.0), roughness: 0.3, metallic: 0.8) } }\nsurface alpha(sp: surf) -> material(unlit) { compose { base(albedo: rgba(0.2, 0.4, 0.8, 1.0)) } }".into());
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let mut manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(
        fresco_example_engine::runtime::tables::column_bytes(
            &manifest,
            "DrawRecord",
            "shading_model",
            1024
        )
        .unwrap(),
        [0u32, 2, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    for surface in &manifest.surfaces {
        let pass = surface.mesh_passes.first().unwrap();
        assert_eq!(
            manifest.tables[0]
                .records
                .iter()
                .find(|r| r.key == surface.name)
                .unwrap()
                .index,
            if surface.name == "alpha" { 1 } else { 2 }
        );
        let geometry = pass
            .entries
            .iter()
            .find(|e| e.function == "geometry")
            .unwrap();
        assert!(output.wgsl.contains(&geometry.entry));
        assert_eq!(
            geometry
                .outputs
                .iter()
                .map(|o| o.ty.as_str())
                .collect::<Vec<_>>(),
            ["vec4", "vec4", "vec4", "uvec2", "vec2"]
        );
    }
    {
        let source = output.wgsl.as_str();
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
    manifest.tables[0].records[0].index = manifest.tables[0].records[1].index;
    assert!(
        fresco_example_engine::runtime::tables::column_bytes(
            &manifest,
            "DrawRecord",
            "shading_model",
            1024
        )
        .is_err()
    );
    manifest.tables[0].records[0].index = u32::MAX;
    assert!(
        fresco_example_engine::runtime::tables::column_bytes(
            &manifest,
            "DrawRecord",
            "shading_model",
            1024
        )
        .is_err()
    );
}

#[test]
fn depth_reconstruction_inverts_view_projection_and_rejects_singular_cameras() {
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [257, 193],
    });
    let inverse = inverse_view_projection(&scene).unwrap();
    for position in [[0.0, 0.0, 0.0, 1.0], [0.3, -0.7, 0.2, 1.0]] {
        let mul = |m: &[f32; 16], v: [f32; 4]| -> [f32; 4] {
            std::array::from_fn(|row| (0..4).map(|col| m[col * 4 + row] * v[col]).sum())
        };
        let clip = mul(&scene.projection, mul(&scene.view, position));
        let reconstructed = mul(&inverse, clip);
        for i in 0..3 {
            assert!((reconstructed[i] / reconstructed[3] - position[i]).abs() < 0.0001);
        }
    }
    scene.projection = [0.0; 16];
    assert!(inverse_view_projection(&scene).is_err());
}
