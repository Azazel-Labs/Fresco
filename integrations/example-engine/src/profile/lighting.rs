//! Authored sample scene presets; the renderer receives only packed light values.
use crate::runtime::forward_plus::{PointLight, SceneLighting};

/// Scene presets shared by sample hosts. They produce ordinary light data.
#[derive(Debug, Default, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LightingEnvironment {
    #[default]
    Preview,
    Unlit,
    Directional,
    ThreeLights,
}

impl LightingEnvironment {
    pub fn lighting(self) -> SceneLighting {
        let preview = matches!(self, Self::Preview);
        SceneLighting {
            unlit: matches!(self, Self::Unlit),
            sky: if preview {
                [0.40, 0.46, 0.55]
            } else {
                [0.03; 3]
            },
            ground: if preview {
                [0.25, 0.23, 0.22]
            } else {
                [0.03; 3]
            },
            specular_fill: if preview { 0.3 } else { 0.0 },
            direction: [0.4, 0.8, 0.6],
            directional_radiance: match self {
                Self::Preview => [2.8, 2.65, 2.4],
                Self::Directional => [3.0; 3],
                _ => [0.0; 3],
            },
            shadows: preview,
        }
    }
    pub fn bytes(self) -> [u8; 80] {
        self.lighting().bytes().expect("valid lighting preset")
    }
    pub fn point_lights(self) -> Vec<PointLight> {
        if matches!(self, Self::ThreeLights) {
            demo_lights().to_vec()
        } else {
            Vec::new()
        }
    }
}
pub fn demo_lights() -> [PointLight; 3] {
    [
        PointLight {
            position: [-1.4, 0.8, 1.4],
            radius: 3.0,
            color: [1.0, 0.12, 0.04],
            intensity: 3.0,
        },
        PointLight {
            position: [1.4, 0.6, 1.0],
            radius: 3.0,
            color: [0.05, 0.25, 1.0],
            intensity: 3.0,
        },
        PointLight {
            position: [0.0, -1.2, 1.6],
            radius: 2.8,
            color: [0.08, 1.0, 0.2],
            intensity: 2.0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_create_ordinary_lights_and_allow_arbitrary_replacements() {
        assert!(LightingEnvironment::Preview.point_lights().is_empty());
        let mut lights = LightingEnvironment::ThreeLights.point_lights();
        assert_eq!(lights.len(), 3);
        lights.push(PointLight {
            position: [2.0, 3.0, 1.0],
            radius: 5.0,
            color: [0.4, 0.7, 0.2],
            intensity: 4.0,
        });
        assert!(crate::runtime::forward_plus::pack_lights(&lights).is_ok());
        let mut lighting = LightingEnvironment::Preview.lighting();
        lighting.direction = [-1.0, 0.3, 0.0];
        assert!(lighting.bytes().is_ok());
        lighting.direction = [0.0; 3];
        assert!(lighting.bytes().is_err());
        lighting = LightingEnvironment::Preview.lighting();
        lighting.sky[0] = -1.0;
        assert!(lighting.bytes().is_err());
    }
}
