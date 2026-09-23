//! Concrete GPU state reflected from authored pass expressions.
use super::RuntimeError;
use fresco_artifact::ManifestSurfaceSettings;

pub(crate) struct RasterState {
    pub cull: Option<wgpu::Face>,
    pub blend: Option<wgpu::BlendState>,
    pub depth_write: bool,
    pub depth_compare: wgpu::CompareFunction,
}
pub(crate) fn resolve(
    settings: &ManifestSurfaceSettings,
    pass: &str,
) -> Result<RasterState, RuntimeError> {
    resolve_targets(settings, pass, true)
}

pub(crate) fn resolve_targets(
    settings: &ManifestSurfaceSettings,
    pass: &str,
    has_color: bool,
) -> Result<RasterState, RuntimeError> {
    let invalid = |s: &str| RuntimeError::MeshContract {
        entry: pass.into(),
        reason: s.into(),
    };
    let state = settings
        .pass_states
        .get(pass)
        .ok_or_else(|| invalid("missing authored raster state"))?;
    let cull = match state.cull.as_deref() {
        Some("none") => None,
        Some("front") => Some(wgpu::Face::Front),
        Some("back") => Some(wgpu::Face::Back),
        _ => return Err(invalid("invalid cull state")),
    };
    let blend = match state.blend.as_deref() {
        None if !has_color => None,
        Some("replace") => None,
        Some("alpha") => Some(wgpu::BlendState::ALPHA_BLENDING),
        Some("premultiplied") => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        Some("additive") => Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        }),
        _ => return Err(invalid("invalid blend state")),
    };
    let depth_compare = match state.depth_compare.as_deref() {
        Some("never") => wgpu::CompareFunction::Never,
        Some("less") => wgpu::CompareFunction::Less,
        Some("equal") => wgpu::CompareFunction::Equal,
        Some("less_equal") => wgpu::CompareFunction::LessEqual,
        Some("greater") => wgpu::CompareFunction::Greater,
        Some("not_equal") => wgpu::CompareFunction::NotEqual,
        Some("greater_equal") => wgpu::CompareFunction::GreaterEqual,
        Some("always") => wgpu::CompareFunction::Always,
        _ => return Err(invalid("invalid depth comparison")),
    };
    Ok(RasterState {
        cull,
        blend,
        depth_write: state
            .depth_write
            .ok_or_else(|| invalid("missing depth-write state"))?,
        depth_compare,
    })
}
