//! Native lifecycle adapter for the shared canvas and mesh runtimes.
use fresco_example_engine::{
    profile::{FrameInputs, camera::OrbitCamera, preview::PreviewShape},
    runtime::{
        canvas::{CanvasInputs, CanvasRenderer},
        depth::DepthTarget,
        mesh::{MeshRenderer, MeshResources},
        mesh_geometry::MeshGeometry,
        particles::{ParticleInputs, ParticleRenderer},
    },
};

use super::{EntryKind, PreparedArtifact};

pub(super) enum Renderer {
    Canvas(Box<CanvasRenderer>),
    Particles {
        renderer: Box<ParticleRenderer>,
        depth: Option<DepthTarget>,
    },
    Mesh {
        shape: PreviewShape,
        renderer: Box<MeshRenderer>,
        depth: Option<DepthTarget>,
    },
}

impl Renderer {
    pub async fn prepare(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        program: &PreparedArtifact,
    ) -> Result<Self, String> {
        match program.kind {
            EntryKind::Particles => {
                let mut renderer = ParticleRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &program.artifact.wgsl,
                    &program.artifact.manifest,
                    &program.entry,
                    ParticleInputs {
                        textures: &program.textures,
                        parameters: Some(&program.parameters),
                    },
                    format,
                )
                .await
                .map_err(|e| e.to_string())?;
                if let Some(environment) = crate::scene::prepare(
                    device,
                    queue,
                    &program.artifact.wgsl,
                    &program.artifact.manifest,
                    format,
                )
                .await
                .map_err(|e| e.to_string())?
                {
                    renderer.set_environment(environment, crate::scene::GROUND_MODEL);
                }
                Ok(Self::Particles {
                    renderer: Box::new(renderer),
                    depth: None,
                })
            }
            EntryKind::Canvas => {
                let mut renderer = CanvasRenderer::prepare_with_inputs(
                    device.clone(),
                    queue.clone(),
                    &program.artifact.wgsl,
                    &program.artifact.manifest,
                    &program.entry,
                    format,
                    CanvasInputs {
                        textures: &program.textures,
                        variant: program.variant.as_ref(),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
                let update = renderer
                    .begin_parameter_update(&program.parameters)
                    .map_err(|e| e.to_string())?
                    .prepare()
                    .await
                    .map_err(|e| e.to_string())?;
                renderer
                    .apply_parameter_update(update)
                    .map_err(|e| e.to_string())?;
                Ok(Self::Canvas(Box::new(renderer)))
            }
            EntryKind::Mesh => {
                let manifest = &program.artifact.manifest;
                let surface = manifest
                    .surfaces
                    .iter()
                    .find(|s| s.name == program.entry)
                    .ok_or("surface entry is missing")?;
                let pass = surface.mesh_passes.first().ok_or("mesh pass is missing")?;
                let factory = manifest
                    .vertex_factories
                    .iter()
                    .find(|f| f.name == pass.factory)
                    .ok_or("mesh factory is missing")?;
                let mesh = program
                    .mesh
                    .geometry_for_surface(surface)
                    .map_err(|e| e.to_string())?;
                let geometry = MeshGeometry::prepare(
                    device,
                    queue,
                    factory,
                    mesh.vertex_count,
                    &mesh.streams(),
                    Some(&mesh.indices),
                )
                .await
                .map_err(|e| e.to_string())?;
                let mut renderer = MeshRenderer::prepare_with_resources(
                    device.clone(),
                    queue.clone(),
                    &program.artifact.wgsl,
                    manifest,
                    &program.entry,
                    MeshResources {
                        factory: None,
                        geometry,
                        textures: &program.textures,
                        parameters: Some(&program.parameters),
                    },
                    format,
                )
                .await
                .map_err(|e| e.to_string())?;
                crate::scene::populate(
                    &mut renderer,
                    device,
                    queue,
                    &program.artifact.wgsl,
                    manifest,
                    format,
                )
                .await
                .map_err(|e| e.to_string())?;
                Ok(Self::Mesh {
                    shape: program.mesh,
                    renderer: Box::new(renderer),
                    depth: None,
                })
            }
        }
    }

    pub fn reset(&mut self) -> Result<(), String> {
        if let Self::Particles { renderer, .. } = self {
            renderer.reset().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        frame: FrameInputs,
        camera: OrbitCamera,
        target: &wgpu::TextureView,
    ) -> Result<bool, String> {
        if frame.physical_size.contains(&0) {
            return Ok(false);
        }
        match self {
            Self::Particles { renderer, depth } => {
                if depth
                    .as_ref()
                    .is_none_or(|target| target.size() != frame.physical_size)
                {
                    *depth = pollster::block_on(DepthTarget::prepare(device, frame.physical_size))
                        .map_err(|e| e.to_string())?;
                }
                pollster::block_on(renderer.render(
                    &camera.scene(PreviewShape::Sphere, frame),
                    target,
                    depth.as_ref().expect("prepared depth").view(),
                ))
                .map_err(|e| e.to_string())
            }
            Self::Canvas(renderer) => renderer.render(frame, target).map_err(|e| e.to_string()),
            Self::Mesh {
                renderer,
                depth,
                shape,
            } => {
                if depth
                    .as_ref()
                    .is_none_or(|target| target.size() != frame.physical_size)
                {
                    // Native resize work is infrequent. Retain the old target if
                    // allocating/validating its replacement fails.
                    let candidate =
                        pollster::block_on(DepthTarget::prepare(device, frame.physical_size))
                            .map_err(|e| e.to_string())?;
                    *depth = candidate;
                }
                renderer
                    .render(
                        &camera.scene(*shape, frame),
                        target,
                        depth.as_ref().expect("prepared depth").view(),
                    )
                    .map_err(|e| e.to_string())
            }
        }
    }
}
