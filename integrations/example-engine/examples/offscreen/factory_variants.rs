//! Factory resource layouts are per variant, with no compiler preview ABI.
use super::{FORMAT, SIZE, mesh::geometry, readback};
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{
        depth::DepthTarget,
        mesh::{FactoryInputs, MeshRenderer, MeshResources},
        textures::TextureInputs,
    },
};
use std::{collections::BTreeMap, error::Error};

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    let contract = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    contract.push_str(r#"
vertex_factory preview_skinned for PreviewMesh {
    binding {
        @group(draw) scene: uniform<PreviewScene>
        @group(draw) environment: uniform<PreviewLighting>
        @group(draw) shadow_camera: uniform<PreviewShadow>
        @group(draw) shadow_map: texture_depth_2d
        @group(draw) point_lights: buffer<vec4>
        @group(draw) light_tiles: buffer<u32>
        @group(draw) style_settings: buffer<vec4>
        @group(draw) @source(DrawRecord) draw_record: uniform<DrawRecord>
        @group(draw) @draw_data(instance_id) shading_instance_id: uniform<u32>
        @group(draw) offsets: buffer<vec4>
    }
    fn transform(v: PreviewMesh) -> mat4 from object to world {
        return scene.model * mat4(vec4(1.0, 0.0, 0.0, 0.0), vec4(0.0, 1.0, 0.0, 0.0), vec4(0.0, 0.0, 1.0, 0.0), vec4(offsets[0].xyz, 1.0))
    }
}
"#);
    files.insert("main.fr".into(), "surface probe(sp: surf) -> material(unlit) {\n properties { used_with_skinning: true }\n compose { base(albedo: rgba(0.8, 0.3, 0.1, 1.0)) } }".into());
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest)?;
    let pass = manifest.surfaces[0].mesh_passes.first().unwrap();
    assert_eq!(pass.variants.len(), 2);
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let mut baseline = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "probe",
        geometry(device, queue, &manifest).await?,
        FORMAT,
    )
    .await?;
    baseline.render(&scene, view, depth.view())?;
    let before = readback(device, queue, texture)?;
    let offsets = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("variant translation"),
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bytes: Vec<u8> = [0.7f32, 0.0, 0.0, 0.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    queue.write_buffer(&offsets, 0, &bytes);
    let buffers = BTreeMap::from([("offsets".into(), offsets)]);
    let mut variant = MeshRenderer::prepare_with_resources(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "probe",
        MeshResources {
            factory: Some(FactoryInputs {
                name: "preview_skinned",
                buffers: &buffers,
            }),
            geometry: geometry(device, queue, &manifest).await?,
            textures: &TextureInputs::new(),
            parameters: None,
        },
        FORMAT,
    )
    .await?;
    variant.render(&scene, view, depth.view())?;
    assert_ne!(
        before,
        readback(device, queue, texture)?,
        "the extra factory buffer must affect executed vertex positions"
    );
    let failed = MeshRenderer::prepare_with_resources(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "probe",
        MeshResources {
            factory: Some(FactoryInputs {
                name: "preview_skinned",
                buffers: &BTreeMap::new(),
            }),
            geometry: geometry(device, queue, &manifest).await?,
            textures: &TextureInputs::new(),
            parameters: None,
        },
        FORMAT,
    )
    .await;
    assert!(
        matches!(failed, Err(e) if e.to_string().contains("missing factory resource `offsets`"))
    );
    println!(
        "Factory variants: independent resource layouts, extra-buffer vertex execution, and missing-resource diagnostics passed."
    );
    Ok(())
}
