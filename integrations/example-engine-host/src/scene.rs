//! Sample scene content shared by the native and browser hosts.
use fresco_artifact::ManifestRoot;
use fresco_example_engine::{
    profile::preview::PreviewGeometry,
    runtime::{RuntimeError, mesh::MeshRenderer, mesh_geometry::MeshGeometry},
};

pub async fn prepare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    wgsl: &str,
    manifest: &ManifestRoot,
    format: wgpu::TextureFormat,
) -> Result<Option<MeshRenderer>, RuntimeError> {
    let Some(surface) = manifest
        .surfaces
        .iter()
        .find(|s| s.name == "fresco_scene_ground")
    else {
        // An independent engine owns its own scene setup.
        return Ok(None);
    };
    let factory_name = &surface
        .mesh_passes
        .first()
        .ok_or_else(|| RuntimeError::PassPlan("ground material has no mesh program".into()))?
        .factory;
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| &f.name == factory_name)
        .ok_or_else(|| RuntimeError::PassPlan("ground vertex factory is missing".into()))?;
    let mesh = PreviewGeometry::plane();
    let geometry = MeshGeometry::prepare(
        device,
        queue,
        factory,
        mesh.vertex_count,
        &mesh.streams(),
        Some(&mesh.indices),
    )
    .await?;
    let floor = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        wgsl,
        manifest,
        &surface.name,
        geometry,
        format,
    )
    .await?;
    floor.set_background([0.075, 0.11, 0.17, 1.0], [0.23, 0.26, 0.30, 1.0])?;
    Ok(Some(floor))
}

pub const GROUND_MODEL: [f32; 16] = [
    4.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, -0.92, 0.0, 1.0,
];

pub async fn populate(
    renderer: &mut MeshRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    wgsl: &str,
    manifest: &ManifestRoot,
    format: wgpu::TextureFormat,
) -> Result<(), RuntimeError> {
    if let Some(floor) = prepare(device, queue, wgsl, manifest, format).await? {
        renderer.add_object(floor, GROUND_MODEL)?;
        renderer.set_background([0.075, 0.11, 0.17, 1.0], [0.23, 0.26, 0.30, 1.0])?;
    }
    Ok(())
}
