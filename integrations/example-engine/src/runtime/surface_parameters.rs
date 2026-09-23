//! Current material parameter ABI: one group-0 uniform per parameter, carrying
//! either a scalar f32 lane or four color lanes. Buffers are padded to 16 bytes.
use std::collections::BTreeSet;

use fresco_artifact::{ManifestParam, ManifestSurfaceParam};
use serde_json::{Map, Value};

use super::{RuntimeError, parameters::ParameterSet, uniforms::UniformLimits};

#[derive(Clone)]
struct Block {
    group: u32,
    name: String,
    binding: u32,
    state: ParameterSet,
}

#[derive(Clone)]
pub struct SurfaceParameters {
    blocks: Vec<Block>,
}

pub struct SurfaceParameterUpload<'a> {
    pub group: u32,
    pub binding: u32,
    pub bytes: &'a [u8],
}

impl SurfaceParameters {
    pub fn new(
        definitions: &[ManifestSurfaceParam],
        limits: UniformLimits,
        occupied: impl IntoIterator<Item = (u32, u32)>,
    ) -> Result<Self, RuntimeError> {
        let mut bindings: BTreeSet<_> = occupied.into_iter().collect();
        let mut names = BTreeSet::new();
        let mut blocks = Vec::new();
        for def in definitions {
            let invalid = |reason: &str| RuntimeError::Parameter {
                name: def.name.clone(),
                reason: reason.into(),
            };
            if !matches!(def.ty.as_str(), "f32" | "i32" | "u32" | "bool" | "color") {
                return Err(invalid("unsupported material uniform type"));
            }
            if def.group >= limits.max_bind_groups
                || def.binding >= limits.max_bindings_per_bind_group
                || limits.max_uniform_buffer_binding_size < 16
            {
                return Err(invalid("material uniform exceeds device limits"));
            }
            if !names.insert(&def.name) {
                return Err(invalid("duplicate parameter name"));
            }
            if !bindings.insert((def.group, def.binding)) {
                return Err(invalid(&format!(
                    "occupied uniform binding at group {} binding {}",
                    def.group, def.binding
                )));
            }
            // Reuse the shared f32 transport/range/default validation. Each
            // material has independent buffers rather than the canvas's shared
            // 64-lane block, so do not impose a canvas-wide parameter limit.
            let state = ParameterSet::new(vec![ManifestParam {
                name: def.name.clone(),
                ty: def.ty.clone(),
                param_type: None,
                default: def.default.clone(),
                min: def.min,
                max: def.max,
            }])?;
            blocks.push(Block {
                group: def.group,
                name: def.name.clone(),
                binding: def.binding,
                state,
            });
        }
        Ok(Self { blocks })
    }

    pub fn values(&self) -> Map<String, Value> {
        self.blocks
            .iter()
            .flat_map(|block| block.state.values().clone())
            .collect()
    }

    pub fn uploads(&self) -> impl Iterator<Item = SurfaceParameterUpload<'_>> {
        self.blocks.iter().map(|block| SurfaceParameterUpload {
            group: block.group,
            binding: block.binding,
            bytes: &block.state.bytes()[..16],
        })
    }

    /// Validate the complete batch before replacing any CPU values or bytes.
    pub fn update(&mut self, updates: &Map<String, Value>) -> Result<(), RuntimeError> {
        for name in updates.keys() {
            if !self.blocks.iter().any(|block| &block.name == name) {
                return Err(RuntimeError::Parameter {
                    name: name.clone(),
                    reason: "unknown material parameter".into(),
                });
            }
        }
        let mut staged = self.blocks.clone();
        for block in &mut staged {
            if let Some(value) = updates.get(&block.name) {
                block
                    .state
                    .update(&Map::from_iter([(block.name.clone(), value.clone())]))?;
            }
        }
        self.blocks = staged;
        Ok(())
    }
}
