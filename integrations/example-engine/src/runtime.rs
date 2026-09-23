//! Platform-independent resource preparation shared by native and browser hosts.

pub mod bounds;
pub mod buffer_views;
#[cfg(feature = "runtime")]
pub mod canvas;
pub mod canvas_variant;
pub mod compute_expression;
#[cfg(feature = "runtime")]
mod compute_frame;
pub mod compute_graph;
pub mod compute_plan;
#[cfg(feature = "runtime")]
pub mod deferred;
#[cfg(feature = "runtime")]
pub mod depth;
#[cfg(feature = "runtime")]
pub mod device;
#[cfg(feature = "runtime")]
pub mod forward_plus;
#[cfg(feature = "runtime")]
pub mod material_resources;
#[cfg(feature = "runtime")]
pub mod mesh;
#[cfg(feature = "runtime")]
pub mod mesh_geometry;
#[cfg(feature = "runtime")]
pub mod owned_compute;
pub mod parameters;
#[cfg(feature = "runtime")]
pub mod particle_bindings;
#[cfg(feature = "runtime")]
pub mod particle_buffers;
#[cfg(feature = "runtime")]
pub mod particle_compute;
pub mod particle_contract;
#[cfg(feature = "runtime")]
pub mod particle_draw;
pub mod particle_layout;
pub mod particle_playback;
pub mod particle_pool;
#[cfg(feature = "runtime")]
pub mod particles;
pub mod pass_plan;
pub mod paths;
#[cfg(feature = "runtime")]
mod raster_state;
#[cfg(feature = "runtime")]
mod recipe;
#[cfg(feature = "runtime")]
mod resources;
pub mod storage;
pub mod style_parameters;
pub mod surface_parameters;
pub mod tables;
#[cfg(feature = "runtime")]
pub mod targets;
#[cfg(feature = "runtime")]
pub mod technique;
pub mod textures;
pub mod uniforms;
pub mod vertices;

pub fn validate_artifact(manifest: &fresco_artifact::ManifestRoot) -> Result<(), RuntimeError> {
    if manifest.schema_version != fresco_artifact::SCHEMA_VERSION {
        return Err(RuntimeError::ArtifactVersion {
            actual: manifest.schema_version,
            expected: fresco_artifact::SCHEMA_VERSION,
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("unsupported artifact version {actual}; expected {expected}")]
    ArtifactVersion { actual: u32, expected: u32 },
    #[error("deferred renderer: {0}")]
    Deferred(String),
    #[error("forward+ lighting: {0}")]
    ForwardPlus(String),
    #[error("invalid particle resources: {0}")]
    ParticleResources(String),
    #[error("invalid mesh depth target: {0}")]
    DepthTarget(String),
    #[error("mesh `{entry}`: {reason}")]
    MeshContract { entry: String, reason: String },
    #[error("vertex factory `{factory}`: {reason}")]
    VertexLayout { factory: String, reason: String },
    #[error("invalid pass plan: {0}")]
    PassPlan(String),
    #[error("path buffer `{name}`: {reason}")]
    PathBuffer { name: String, reason: String },
    #[error("texture `{name}`: {reason}")]
    Texture { name: String, reason: String },
    #[error("parameter `{name}`: {reason}")]
    Parameter { name: String, reason: String },
    #[error("canvas `{entry}`: {reason}")]
    CanvasContract { entry: String, reason: String },
    #[error("GPU preparation failed: {0}")]
    GpuValidation(String),
    #[error("invalid uniform layout `{uniform}`: {reason}")]
    UniformLayout { uniform: String, reason: String },
    #[error("host must supply `{uniform}.{field}`")]
    MissingUniformValue { uniform: String, field: String },
    #[error("invalid value for `{uniform}.{field}`: {reason}")]
    UniformValue {
        uniform: String,
        field: String,
        reason: String,
    },
}
