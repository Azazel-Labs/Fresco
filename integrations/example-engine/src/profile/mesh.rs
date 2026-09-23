//! Host inputs for the bundled `PreviewScene` contract in `05_mesh_contract.fr`.
//! Matrix values are column-major, matching WGSL's matrix constructors.

use super::FrameInputs;
use crate::runtime::RuntimeError;

const SCENE_FLOATS: usize = 60;
pub const SCENE_BYTES: usize = SCENE_FLOATS * size_of::<f32>();

#[derive(Debug, Clone, Copy)]
pub struct DisplacementInputs {
    pub enabled: bool,
    pub amplitude: f32,
    pub frequency: f32,
    pub speed: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct MeshSceneInputs {
    pub model: [f32; 16],
    pub view: [f32; 16],
    pub projection: [f32; 16],
    pub camera_position: [f32; 3],
    pub frame: FrameInputs,
    pub displacement: DisplacementInputs,
}

impl MeshSceneInputs {
    /// Pack the sample profile's scene buffer. This is profile policy, not a
    /// general reflection-based uniform encoder. An integration with another
    /// scene contract must supply its own adapter.
    ///
    /// Returns owned bytes only after validating every input, allowing the host
    /// to preserve its installed GPU buffer on failure.
    pub fn pack(&self) -> Result<[u8; SCENE_BYTES], RuntimeError> {
        let mut values = [0.0_f32; SCENE_FLOATS];
        values[..16].copy_from_slice(&self.model);
        values[16..32].copy_from_slice(&self.view);
        values[32..48].copy_from_slice(&self.projection);
        values[48..51].copy_from_slice(&self.camera_position);
        values[51] = self.frame.time;
        values[52] = self.frame.physical_size[0] as f32;
        values[53] = self.frame.physical_size[1] as f32;
        // Lanes 54 and 55 are the authored padding field.
        values[56] = if self.displacement.enabled { 1.0 } else { 0.0 };
        values[57] = self.displacement.amplitude;
        values[58] = self.displacement.frequency;
        values[59] = self.displacement.speed;
        if values.iter().any(|value| !value.is_finite()) {
            return Err(RuntimeError::UniformValue {
                uniform: "scene".into(),
                field: "PreviewScene".into(),
                reason: "matrix, camera, time, and displacement inputs must be finite".into(),
            });
        }
        let mut bytes = [0; SCENE_BYTES];
        for (value, destination) in values.iter().zip(bytes.as_chunks_mut::<4>().0) {
            destination.copy_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }
}

/// Conservative bounds for the bundled static factory's authored preparation.
/// Other factories must provide their own world bounds; this adapter does not
/// assume their skinning or procedural transforms match PreviewScene.model.
pub fn prepared_bounds(
    factory: &str,
    local: Option<crate::runtime::bounds::Bounds>,
    scene: &MeshSceneInputs,
) -> Result<Option<crate::runtime::bounds::Bounds>, RuntimeError> {
    if factory != "preview_static" {
        return Ok(None);
    }
    local
        .map(|bounds| {
            bounds
                .expand(if scene.displacement.enabled {
                    scene.displacement.amplitude.abs()
                } else {
                    0.0
                })?
                .transform(&scene.model)
        })
        .transpose()
}
