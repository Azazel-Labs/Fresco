//! Decoded texture inputs and reflected binding validation, independent of a GPU.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::RuntimeError;
use fresco_artifact::{ManifestSampler, ManifestTexture};

/// Row-major RGBA8, top row first, straight alpha, with no implicit sRGB decode.
#[derive(Clone, Debug)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

pub type TextureInputs = BTreeMap<String, TextureImage>;

pub(crate) fn error(name: &str, reason: impl Into<String>) -> RuntimeError {
    RuntimeError::Texture {
        name: name.into(),
        reason: reason.into(),
    }
}

impl TextureImage {
    pub fn validate(&self, name: &str, max_dimension: u32) -> Result<(), RuntimeError> {
        if self.width == 0
            || self.height == 0
            || self.width > max_dimension
            || self.height > max_dimension
        {
            return Err(error(
                name,
                format!("dimensions must be between 1 and {max_dimension}"),
            ));
        }
        let expected = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| error(name, "image byte length overflow"))?;
        if self.pixels.len() != expected {
            return Err(error(
                name,
                format!(
                    "expected {expected} RGBA8 bytes, received {}",
                    self.pixels.len()
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct TextureLimits {
    pub max_dimension: u32,
    pub max_bind_groups: u32,
    pub max_bindings_per_group: u32,
    pub max_textures_per_stage: u32,
    pub max_samplers_per_stage: u32,
}

/// Validate every resource before allocating or replacing GPU state.
pub fn validate_bindings(
    definitions: &[ManifestTexture],
    sampler: Option<&ManifestSampler>,
    inputs: &TextureInputs,
    occupied: impl IntoIterator<Item = (u32, u32)>,
    limits: TextureLimits,
) -> Result<(), RuntimeError> {
    if definitions.len() > limits.max_textures_per_stage as usize {
        return Err(error("bindings", "texture count exceeds the device limit"));
    }
    if !definitions.is_empty() && sampler.is_none() {
        return Err(error(
            "bindings",
            "sampled textures require a reflected sampler",
        ));
    }
    let mut locations: BTreeSet<_> = occupied.into_iter().collect();
    let mut names = BTreeSet::new();
    let mut reserve = |name: &str, group, binding| {
        if group >= limits.max_bind_groups || binding >= limits.max_bindings_per_group {
            return Err(error(name, "binding location exceeds the device limit"));
        }
        if !locations.insert((group, binding)) {
            return Err(error(name, "binding collides with another resource"));
        }
        Ok(())
    };
    for def in definitions {
        if !names.insert(&def.name) {
            return Err(error(&def.name, "duplicate texture name"));
        }
        reserve(&def.name, def.group, def.binding)?;
        inputs
            .get(&def.name)
            .ok_or_else(|| error(&def.name, "host did not supply this texture"))?
            .validate(&def.name, limits.max_dimension)?;
    }
    if let Some(sampler) = sampler {
        if limits.max_samplers_per_stage == 0 {
            return Err(error("sampler", "device cannot bind a sampler"));
        }
        reserve("sampler", sampler.group, sampler.binding)?;
    }
    for name in inputs.keys() {
        if !names.contains(name) {
            return Err(error(name, "no such texture in the selected entry"));
        }
    }
    Ok(())
}
