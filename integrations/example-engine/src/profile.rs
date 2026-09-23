//! Host-value adapters for this authored engine profile, separate from GPU packing.
use fresco_artifact::{ManifestGlobalUniform, ManifestGlobalUniformField};

use crate::runtime::uniforms::UniformValue;

pub mod camera;
#[cfg(feature = "runtime")]
pub mod lighting;
pub mod mesh;
pub mod preview;

#[derive(Debug, Clone, Copy)]
pub struct FrameInputs {
    pub time: f32,
    pub delta_time: f32,
    pub physical_size: [u32; 2],
}

/// Resolve the bundled `frame: FrameGlobals` contract. Unknown fields must be
/// handled explicitly by an extending host; the generic packer reports omissions.
pub fn frame_uniform(
    uniform: &ManifestGlobalUniform,
    field: &ManifestGlobalUniformField,
    frame: FrameInputs,
) -> Option<UniformValue> {
    if uniform.name != "frame" || uniform.ty != "FrameGlobals" {
        return None;
    }
    let values = match field.name.as_str() {
        "time" => vec![frame.time],
        "delta_time" => vec![frame.delta_time],
        "resolution" => vec![frame.physical_size[0] as f32, frame.physical_size[1] as f32],
        _ => return None,
    };
    Some(UniformValue::F32(values))
}
