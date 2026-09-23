//! Orbit policy shared by hosts; input events and scheduling belong to the shell.
use super::{FrameInputs, mesh::MeshSceneInputs, preview::PreviewShape};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            distance: 3.0,
        }
    }
}

impl OrbitCamera {
    /// Logical-pixel movement, matching the existing preview's drag sensitivity.
    pub fn drag(&mut self, delta: [f32; 2]) -> Result<(), &'static str> {
        if delta.iter().any(|v| !v.is_finite()) {
            return Err("camera drag must be finite");
        }
        self.yaw = (self.yaw - delta[0] * 0.01).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + delta[1] * 0.01).clamp(-1.35, 1.35);
        Ok(())
    }

    /// Wheel displacement in logical pixels; positive values move away.
    pub fn zoom(&mut self, delta: f32) -> Result<(), &'static str> {
        if !delta.is_finite() {
            return Err("camera zoom must be finite");
        }
        self.distance = (self.distance * (delta * 0.0015).exp()).clamp(1.1, 8.0);
        Ok(())
    }

    pub fn scene(self, shape: PreviewShape, frame: FrameInputs) -> MeshSceneInputs {
        let mut scene = shape.scene(frame);
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let backward = [sy * cp, sp, cy * cp];
        let right = [cy, 0.0, -sy];
        let up = [-sy * sp, cp, -cy * sp];
        scene.camera_position = backward.map(|v| v * self.distance);
        scene.view = [
            right[0],
            up[0],
            backward[0],
            0.0,
            right[1],
            up[1],
            backward[1],
            0.0,
            right[2],
            up[2],
            backward[2],
            0.0,
            0.0,
            0.0,
            -self.distance,
            1.0,
        ];
        scene
    }
}
