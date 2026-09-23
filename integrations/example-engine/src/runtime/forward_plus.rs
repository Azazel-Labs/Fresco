//! Basic tiled forward lighting. GPU culling is engine code; the material
//! response lives in the authored mesh contract. There is no depth prepass.
use super::RuntimeError;
pub use crate::profile::lighting::{LightingEnvironment, demo_lights};

pub const MAX_LIGHTS: usize = 64;
pub const TILE_SIZE: u32 = 16;
pub const MAX_VIEWPORT: u32 = 8192;

/// Ordinary scene illumination. Preset names never cross the GPU interface.
#[derive(Debug, Clone, Copy)]
pub struct SceneLighting {
    pub unlit: bool,
    pub sky: [f32; 3],
    pub ground: [f32; 3],
    pub specular_fill: f32,
    pub direction: [f32; 3],
    pub directional_radiance: [f32; 3],
    pub shadows: bool,
}
impl SceneLighting {
    pub fn bytes(self) -> Result<[u8; 80], RuntimeError> {
        let values = [
            if self.unlit { 0.0 } else { 1.0 },
            self.specular_fill,
            0.0,
            0.0,
            self.sky[0],
            self.sky[1],
            self.sky[2],
            0.0,
            self.ground[0],
            self.ground[1],
            self.ground[2],
            0.0,
            self.direction[0],
            self.direction[1],
            self.direction[2],
            if self.shadows { 1.0 } else { 0.0 },
            self.directional_radiance[0],
            self.directional_radiance[1],
            self.directional_radiance[2],
            0.0,
        ];
        if values.iter().any(|v| !v.is_finite())
            || self
                .sky
                .iter()
                .chain(&self.ground)
                .chain(&self.directional_radiance)
                .any(|v| *v < 0.0)
            || self.specular_fill < 0.0
            || !(0.00001..=f32::MAX).contains(&self.direction.iter().map(|v| v * v).sum::<f32>())
        {
            return Err(invalid("invalid scene lighting or zero directional vector"));
        }
        let mut bytes = [0; 80];
        for (value, lane) in values.iter().zip(bytes.as_chunks_mut::<4>().0) {
            lane.copy_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }
}
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointLight {
    pub position: [f32; 3],
    pub radius: f32,
    pub color: [f32; 3],
    pub intensity: f32,
}

fn invalid(reason: &str) -> RuntimeError {
    RuntimeError::ForwardPlus(reason.into())
}

/// Validate the whole edit before publishing any bytes. Unused slots are zero.
pub fn pack_lights(lights: &[PointLight]) -> Result<Vec<u8>, RuntimeError> {
    if lights.len() > MAX_LIGHTS {
        return Err(invalid("at most 64 point lights are supported"));
    }
    let mut bytes = vec![0; MAX_LIGHTS * 32];
    for (light, chunk) in lights.iter().zip(bytes.as_chunks_mut::<32>().0) {
        let values = [
            light.position[0],
            light.position[1],
            light.position[2],
            light.radius,
            light.color[0],
            light.color[1],
            light.color[2],
            light.intensity,
        ];
        if values.iter().any(|v| !v.is_finite())
            || light.radius <= 0.0
            || light.intensity < 0.0
            || light.color.iter().any(|v| *v < 0.0)
        {
            return Err(invalid(
                "light values must be finite, radius positive, and color/intensity nonnegative",
            ));
        }
        for (value, lane) in values.iter().zip(chunk.as_chunks_mut::<4>().0) {
            lane.copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(bytes)
}

pub fn tile_grid(size: [u32; 2]) -> Result<[u32; 2], RuntimeError> {
    if size.iter().any(|v| *v > MAX_VIEWPORT) {
        return Err(invalid("viewport exceeds 8192 pixels per dimension"));
    }
    Ok(size.map(|v| v.div_ceil(TILE_SIZE)))
}

/// Stable directional-light projection centered on the preview object's bounds.
/// The same world-space matrix is consumed by every raster path.
pub(super) fn shadow_camera_bytes(
    scene: &crate::profile::mesh::MeshSceneInputs,
    direction: [f32; 3],
) -> [u8; 64] {
    let normalize = |v: [f32; 3]| {
        let d = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.map(|x| x / d)
    };
    let light = normalize(direction);
    let right = if light[0].abs() + light[2].abs() < 0.0001 {
        [1.0, 0.0, 0.0]
    } else {
        normalize([light[2], 0.0, -light[0]])
    };
    let up = [
        light[1] * right[2] - light[2] * right[1],
        light[2] * right[0] - light[0] * right[2],
        light[0] * right[1] - light[1] * right[0],
    ];
    let center = [scene.model[12], scene.model[13], scene.model[14]];
    let scale = (0..3)
        .map(|i| {
            scene.model[i * 4..i * 4 + 3]
                .iter()
                .map(|v| v * v)
                .sum::<f32>()
                .sqrt()
        })
        .fold(0.0_f32, f32::max);
    let displacement = if scene.displacement.enabled {
        scene.displacement.amplitude.abs()
    } else {
        0.0
    };
    let radius = ((2.0 + displacement) * scale).max(0.001);
    let depth_span = 2.0 * radius;
    let dot = |a: [f32; 3], b: [f32; 3]| a.into_iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let mut matrix = [0.0; 16];
    for i in 0..3 {
        matrix[i * 4] = right[i] / radius;
        matrix[i * 4 + 1] = up[i] / radius;
        matrix[i * 4 + 2] = -light[i] / depth_span;
    }
    matrix[12] = -dot(right, center) / radius;
    matrix[13] = -dot(up, center) / radius;
    matrix[14] = (radius + dot(light, center)) / depth_span;
    matrix[15] = 1.0;
    let mut bytes = [0; 64];
    for (value, output) in matrix.iter().zip(bytes.as_chunks_mut::<4>().0) {
        output.copy_from_slice(&value.to_le_bytes());
    }
    bytes
}
