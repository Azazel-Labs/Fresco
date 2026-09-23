//! Entry selection and shared-renderer dispatch; no browser or compiler logic.
use fresco_artifact::{ManifestCanvas, ManifestRoot, ManifestSurface, ManifestTexture};
use fresco_example_engine::{
    profile::{FrameInputs, camera::OrbitCamera, preview::PreviewShape},
    runtime::{
        RuntimeError,
        canvas::{CanvasInputs, CanvasRenderer},
        depth::DepthTarget,
        mesh::{MeshRenderer, MeshResources},
        mesh_geometry::MeshGeometry,
        particles::{ParticleInputs, ParticleRenderer},
        textures::TextureInputs,
    },
};
use serde_json::{Map, Value};

pub(super) enum Renderer {
    Particles(Box<ParticleRenderer>),
    Canvas(Box<CanvasRenderer>),
    Mesh(Box<MeshRenderer>, PreviewShape),
}

enum Entry<'a> {
    Canvas(&'a ManifestCanvas),
    Mesh(&'a ManifestSurface),
}

fn select<'a>(manifest: &'a ManifestRoot, name: &str) -> Result<Entry<'a>, String> {
    let mut entries = manifest
        .canvases
        .iter()
        .filter(|entry| entry.name == name)
        .map(Entry::Canvas)
        .chain(
            manifest
                .surfaces
                .iter()
                .filter(|entry| entry.name == name)
                .map(Entry::Mesh),
        );
    let entry = entries
        .next()
        .ok_or_else(|| format!("entry `{name}` does not exist"))?;
    if entries.next().is_some() {
        return Err(format!("entry `{name}` is ambiguous"));
    }
    Ok(entry)
}

pub(super) fn textures<'a>(
    manifest: &'a ManifestRoot,
    name: &str,
) -> Result<&'a [ManifestTexture], String> {
    Ok(match select(manifest, name)? {
        Entry::Canvas(entry) => &entry.textures,
        Entry::Mesh(entry) => &entry.textures,
    })
}

pub(super) struct Request<'a> {
    pub shape: PreviewShape,
    pub wgsl: &'a str,
    pub manifest: &'a ManifestRoot,
    pub entry: &'a str,
    pub textures: &'a TextureInputs,
    pub parameters: Option<&'a Map<String, Value>>,
    pub variant: Option<&'a fresco_example_engine::runtime::canvas_variant::VariantSelection>,
}

impl Renderer {
    pub async fn prepare(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        request: Request<'_>,
    ) -> Result<Self, String> {
        match select(request.manifest, request.entry)? {
            Entry::Canvas(_) => {
                let mut renderer = CanvasRenderer::prepare_with_inputs(
                    device,
                    queue,
                    request.wgsl,
                    request.manifest,
                    request.entry,
                    format,
                    CanvasInputs {
                        textures: request.textures,
                        variant: request.variant,
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
                if let Some(parameters) = request.parameters {
                    let pending = renderer
                        .begin_parameter_update(parameters)
                        .map_err(|e| e.to_string())?
                        .prepare()
                        .await
                        .map_err(|e| e.to_string())?;
                    renderer
                        .apply_parameter_update(pending)
                        .map_err(|e| e.to_string())?;
                }
                Ok(Self::Canvas(Box::new(renderer)))
            }
            Entry::Mesh(surface) => {
                if request.variant.is_some() {
                    return Err("explicit variant bindings currently require a canvas entry".into());
                }
                if fresco_example_engine::runtime::particle_contract::uses_particles(
                    request.manifest,
                    &surface.name,
                ) {
                    let mut renderer = ParticleRenderer::prepare(
                        device.clone(),
                        queue.clone(),
                        request.wgsl,
                        request.manifest,
                        request.entry,
                        ParticleInputs {
                            textures: request.textures,
                            parameters: request.parameters,
                        },
                        format,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    if let Some(environment) = crate::scene::prepare(
                        &device,
                        &queue,
                        request.wgsl,
                        request.manifest,
                        format,
                    )
                    .await
                    .map_err(|e| e.to_string())?
                    {
                        renderer.set_environment(environment, crate::scene::GROUND_MODEL);
                    }
                    return Ok(Self::Particles(Box::new(renderer)));
                }
                let pass = surface
                    .mesh_passes
                    .first()
                    .ok_or("surface has no executable mesh pass")?;
                let factory = request
                    .manifest
                    .vertex_factories
                    .iter()
                    .find(|factory| factory.name == pass.factory)
                    .ok_or("mesh factory is missing")?;
                let mesh = request
                    .shape
                    .geometry_for_surface(surface)
                    .map_err(|e| e.to_string())?;
                let geometry = MeshGeometry::prepare(
                    &device,
                    &queue,
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
                    request.wgsl,
                    request.manifest,
                    request.entry,
                    MeshResources {
                        factory: None,
                        geometry,
                        textures: request.textures,
                        parameters: request.parameters,
                    },
                    format,
                )
                .await
                .map_err(|e| e.to_string())?;
                crate::scene::populate(
                    &mut renderer,
                    &device,
                    &queue,
                    request.wgsl,
                    request.manifest,
                    format,
                )
                .await
                .map_err(|e| e.to_string())?;
                Ok(Self::Mesh(Box::new(renderer), request.shape))
            }
        }
    }

    pub fn is_mesh(&self) -> bool {
        matches!(self, Self::Mesh(..) | Self::Particles(..))
    }
    pub fn values(&self) -> Map<String, Value> {
        match self {
            Self::Particles(renderer) => renderer.parameter_values(),
            Self::Canvas(renderer) => renderer.parameters().values(),
            Self::Mesh(renderer, _) => renderer.parameter_values(),
        }
    }
    pub fn render(
        &mut self,
        frame: FrameInputs,
        camera: OrbitCamera,
        target: &wgpu::TextureView,
        depth: Option<&DepthTarget>,
    ) -> Result<bool, RuntimeError> {
        match self {
            Self::Particles(_) => Err(RuntimeError::ParticleResources(
                "particle frames require render_async".into(),
            )),
            Self::Canvas(renderer) => renderer.render(frame, target),
            Self::Mesh(renderer, shape) => {
                let depth = depth
                    .filter(|depth| depth.size() == frame.physical_size)
                    .ok_or_else(|| {
                        RuntimeError::DepthTarget(
                            "installed depth dimensions do not match the frame".into(),
                        )
                    })?;
                renderer.render(&camera.scene(*shape, frame), target, depth.view())
            }
        }
    }
}
