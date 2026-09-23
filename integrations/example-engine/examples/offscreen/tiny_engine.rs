//! Independent engine exercises sparse MRT locations and an empty factory layout.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{
        depth::DepthTarget, mesh::MeshRenderer, mesh_geometry::MeshGeometry, vertices::VertexValues,
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
};
pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let files = HashMap::from([
        ("engine/engine.fr".into(), include_str!("../../tests/fixtures/tiny-engine.fr").into()),
        ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: rgba(0.2, 0.4, 0.6, 1.0)) } }".into()),
    ]);
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
    let points = [-1.0, -1.0, 0.0, 3.0, -1.0, 0.0, -1.0, 3.0, 0.0];
    let geometry = MeshGeometry::prepare(
        device,
        queue,
        &manifest.vertex_factories[0],
        3,
        &BTreeMap::from([("point".into(), VertexValues::F32(&points))]),
        None,
    )
    .await?;
    let mut renderer = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "sample",
        geometry,
        FORMAT,
    )
    .await?;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    renderer.render(&scene, view, depth.view())?;
    let pixels = readback(device, queue, output)?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [51, 102, 153, 255])
    );
    let identities = readback(device, queue, renderer.material_id_texture().unwrap())?;
    assert!(
        identities
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| u32::from_le_bytes(*pixel) == 37)
    );
    println!(
        "Independent engine GPU checks: two targets, integer location 6, no scene buffer or built-in material policy passed."
    );
    Ok(())
}
