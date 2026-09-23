//! Built-in preview geometry and camera policy for inspecting the bundled mesh profile.
use std::collections::BTreeMap;

use super::{
    FrameInputs,
    mesh::{DisplacementInputs, MeshSceneInputs},
};
use crate::runtime::vertices::VertexValues;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewShape {
    #[default]
    Sphere,
    Plane,
    Box,
}

impl std::str::FromStr for PreviewShape {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sphere" => Ok(Self::Sphere),
            "plane" => Ok(Self::Plane),
            "box" => Ok(Self::Box),
            _ => Err("mesh must be sphere, plane, or box"),
        }
    }
}

impl PreviewShape {
    pub fn geometry(self) -> PreviewGeometry {
        match self {
            Self::Sphere => PreviewGeometry::sphere(),
            Self::Plane => PreviewGeometry::plane(),
            Self::Box => PreviewGeometry::cube(),
        }
    }

    /// Check the surface contract against this profile's two supplied UV streams
    /// before allocating geometry. Selector spelling is authored, not host policy.
    pub fn geometry_for_surface(
        self,
        surface: &fresco_artifact::ManifestSurface,
    ) -> Result<PreviewGeometry, crate::runtime::RuntimeError> {
        for channel in &surface.surface_requirements.uv_channels {
            if channel.required && (channel.stream_index > 1 || channel.components != 2) {
                return Err(crate::runtime::RuntimeError::MeshContract {
                    entry: surface.name.clone(),
                    reason: format!(
                        "missing required UV streams: {} -> stream {} ({}, {} components); preview supplies two-component streams 0 and 1. See surface_requirements.uv_channels",
                        channel.selector,
                        channel.stream_index,
                        channel.semantic,
                        channel.components,
                    ),
                });
            }
        }
        Ok(self.geometry())
    }

    pub fn scene(self, frame: FrameInputs) -> MeshSceneInputs {
        let mut scene = sphere_scene(frame);
        if self == Self::Plane {
            // Tilt the +Y-facing plane toward the camera, retaining depth and UV perspective.
            let (sin, cos) = std::f32::consts::FRAC_PI_4.sin_cos();
            scene.model = [
                0.9,
                0.0,
                0.0,
                0.0,
                0.0,
                cos * 0.9,
                sin * 0.9,
                0.0,
                0.0,
                -sin * 0.9,
                cos * 0.9,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ];
        }
        scene
    }
}

pub struct PreviewGeometry {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub tangents: Vec<f32>,
    pub uv: Vec<f32>,
    pub uv2: Vec<f32>,
    pub indices: Vec<u32>,
    pub vertex_count: u32,
}

impl PreviewGeometry {
    fn empty(vertex_count: u32) -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uv: Vec::new(),
            uv2: Vec::new(),
            indices: Vec::new(),
            vertex_count,
        }
    }

    /// The preview's 16-by-16 subdivided XZ plane, facing +Y.
    pub fn plane() -> Self {
        const BANDS: u32 = 16;
        let mut mesh = Self::empty((BANDS + 1) * (BANDS + 1));
        for row in 0..=BANDS {
            for column in 0..=BANDS {
                let u = column as f32 / BANDS as f32;
                let v = row as f32 / BANDS as f32;
                mesh.positions
                    .extend_from_slice(&[u * 2.0 - 1.0, 0.0, v * 2.0 - 1.0]);
                mesh.normals.extend_from_slice(&[0.0, 1.0, 0.0]);
                mesh.tangents.extend_from_slice(&[1.0, 0.0, 0.0]);
                mesh.uv.extend_from_slice(&[u, v]);
                mesh.uv2.extend_from_slice(&[u * 2.0, v * 2.0]);
            }
        }
        for row in 0..BANDS {
            for column in 0..BANDS {
                let a = row * (BANDS + 1) + column;
                let b = a + BANDS + 1;
                mesh.indices
                    .extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        mesh
    }

    /// Six separate box faces preserve hard normals and independent UV seams.
    pub fn cube() -> Self {
        let mut mesh = Self::empty(24);
        for (normal, right, up) in [
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ] {
            let base = u32::try_from(mesh.positions.as_chunks::<3>().0.len())
                .expect("bounded preview geometry");
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let position: [f32; 3] =
                    std::array::from_fn(|i| normal[i] + right[i] * su + up[i] * sv);
                mesh.positions.extend_from_slice(&position);
                mesh.normals.extend_from_slice(&normal);
                mesh.tangents.extend_from_slice(&right);
                mesh.uv
                    .extend_from_slice(&[f32::midpoint(su, 1.0), f32::midpoint(sv, 1.0)]);
                mesh.uv2.extend_from_slice(&[su + 1.0, sv + 1.0]);
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        mesh
    }

    /// Match the existing preview's 64 latitude bands, 64 longitude columns,
    /// outward CCW winding, and distinct second UV projection.
    pub fn sphere() -> Self {
        const BANDS: u32 = 64;
        let mut mesh = Self {
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            uv: Vec::new(),
            uv2: Vec::new(),
            indices: Vec::new(),
            vertex_count: (BANDS + 1) * (BANDS + 1),
        };
        for row in 0..=BANDS {
            let v = row as f32 / BANDS as f32;
            let (sin_phi, cos_phi) = (std::f32::consts::PI * v).sin_cos();
            for column in 0..=BANDS {
                let u = column as f32 / BANDS as f32;
                let (sin_theta, cos_theta) = (std::f32::consts::TAU * u).sin_cos();
                let position = [cos_theta * sin_phi, cos_phi, sin_theta * sin_phi];
                mesh.positions.extend_from_slice(&position);
                mesh.normals.extend_from_slice(&position);
                mesh.tangents.extend_from_slice(&if sin_phi < 1.0e-6 {
                    [1.0, 0.0, 0.0]
                } else {
                    [-sin_theta, 0.0, cos_theta]
                });
                mesh.uv.extend_from_slice(&[u, v]);
                mesh.uv2
                    .extend_from_slice(&[position[0] * 0.5 + 0.5, position[2] * 0.5 + 0.5]);
            }
        }
        for row in 0..BANDS {
            for column in 0..BANDS {
                let a = row * (BANDS + 1) + column;
                let b = a + BANDS + 1;
                mesh.indices
                    .extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        mesh
    }

    pub fn streams(&self) -> BTreeMap<String, VertexValues<'_>> {
        BTreeMap::from([
            ("position".into(), VertexValues::F32(&self.positions)),
            ("normal".into(), VertexValues::F32(&self.normals)),
            ("tangent".into(), VertexValues::F32(&self.tangents)),
            ("uv".into(), VertexValues::F32(&self.uv)),
            ("uv2".into(), VertexValues::F32(&self.uv2)),
        ])
    }
}

/// Fixed initial camera, with a WebGPU zero-to-one depth projection. Hosts can
/// supply another MeshSceneInputs value when adding camera controls.
pub fn sphere_scene(frame: FrameInputs) -> MeshSceneInputs {
    let aspect = frame.physical_size[0].max(1) as f32 / frame.physical_size[1].max(1) as f32;
    let f = 1.0 / (std::f32::consts::FRAC_PI_4 * 0.5).tan();
    let near = 0.1;
    let far = 100.0;
    let range = 1.0 / (near - far);
    MeshSceneInputs {
        model: [
            0.9, 0.0, 0.0, 0.0, 0.0, 0.9, 0.0, 0.0, 0.0, 0.0, 0.9, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        view: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -3.0, 1.0,
        ],
        projection: [
            f / aspect,
            0.0,
            0.0,
            0.0,
            0.0,
            f,
            0.0,
            0.0,
            0.0,
            0.0,
            far * range,
            -1.0,
            0.0,
            0.0,
            far * near * range,
            0.0,
        ],
        camera_position: [0.0, 0.0, 3.0],
        frame,
        displacement: DisplacementInputs {
            enabled: false,
            amplitude: 0.0,
            frequency: 0.0,
            speed: 0.0,
        },
    }
}
