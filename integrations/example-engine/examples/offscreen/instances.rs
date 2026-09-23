//! Local-only proof that draw identity is independent of the material table.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{
        FrameInputs,
        preview::{PreviewGeometry, sphere_scene},
    },
    runtime::{depth::DepthTarget, mesh::MeshRenderer, mesh_geometry::MeshGeometry},
};
use std::error::Error;

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    let engine = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    *engine = engine.replace("    fn shade(sp: PreviewVarying, m: base)", r#"
    fn shade(sp: PreviewVarying, m: unlit) -> color {
        return rgba(select(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), shading_instance_id == 2u), 1.0)
    }
    fn shade(sp: PreviewVarying, m: base)"#);
    files.insert(
        "main.fr".into(),
        "surface shared(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("{errors:?}"))?;
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
    assert_eq!(manifest.surfaces.len(), 1);
    let shape = PreviewGeometry::sphere();
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| f.name == "preview_static")
        .unwrap();
    let geometry = MeshGeometry::prepare(
        device,
        queue,
        factory,
        shape.vertex_count,
        &shape.streams(),
        Some(&shape.indices),
    )
    .await?;
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.displacement.enabled = false;
    for value in &mut scene.model[..12] {
        *value *= 0.45;
    }
    scene.model[12] -= 0.45;
    let mut other_model = scene.model;
    other_model[12] += 0.9;
    let mut first = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "shared",
        geometry.clone(),
        FORMAT,
    )
    .await?;
    let second = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "shared",
        geometry.clone(),
        FORMAT,
    )
    .await?;
    first.add_object(second, other_model)?;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    first.render(&scene, view, depth.view())?;
    let both = readback(device, queue, output)?;
    let count = |pixels: &[u8], channel: usize| {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[channel] > 100 && p[1 - channel] < 30 && p[2] < 30)
            .count()
    };
    assert!(
        count(&both, 0) > 10 && count(&both, 1) > 10,
        "two draws sharing one material must carry distinct nonzero IDs"
    );
    // A separate rendered view starts its own ID namespace and owns its uniforms.
    let mut other_view = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "shared",
        geometry,
        FORMAT,
    )
    .await?;
    let other_output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("independent shading view"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let other_target = other_output.create_view(&Default::default());
    let other_depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    // Both views have work in flight before either output is inspected.
    first.render(&scene, view, depth.view())?;
    other_view.render(&scene, &other_target, other_depth.view())?;
    let single = readback(device, queue, &other_output)?;
    assert!(count(&single, 0) > 10);
    assert_eq!(count(&single, 1), 0);
    assert_eq!(both, readback(device, queue, output)?);
    println!(
        "Draw identity: shared material has independent instance IDs and separate views do not overwrite them."
    );
    Ok(())
}
