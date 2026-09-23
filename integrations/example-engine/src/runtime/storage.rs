//! Dynamic canvas arrays, packed according to the compiler's storage ABI.
//! Unlike the instance parameter block, vec3 elements have a 16-byte stride.
use std::collections::BTreeSet;

use fresco_artifact::{ManifestCanvas, ManifestParam, ManifestStorageParam};
use serde_json::{Map, Value};

use super::RuntimeError;
use super::parameters::{Scalar, encode_scalar};

#[derive(Debug, Clone, Copy)]
pub struct StorageLimits {
    pub max_bind_groups: u32,
    pub max_bindings_per_group: u32,
    pub max_buffers_per_stage: u32,
    pub max_buffer_bytes: u64,
}

pub(crate) fn validate_canvas_storage_count(
    canvas: &ManifestCanvas,
    limits: StorageLimits,
) -> Result<(), RuntimeError> {
    let count = canvas
        .storage_params
        .len()
        .checked_add(canvas.path_buffers.len())
        .and_then(|count| u32::try_from(count).ok());
    if count.is_none_or(|count| count > limits.max_buffers_per_stage) {
        return Err(invalid(
            "storage",
            "combined array/path buffer count exceeds device limit",
        ));
    }
    Ok(())
}

#[cfg(feature = "runtime")]
impl From<&wgpu::Limits> for StorageLimits {
    fn from(limits: &wgpu::Limits) -> Self {
        Self {
            max_bind_groups: limits.max_bind_groups,
            max_bindings_per_group: limits.max_bindings_per_bind_group,
            max_buffers_per_stage: limits.max_storage_buffers_per_shader_stage,
            max_buffer_bytes: limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size),
        }
    }
}

#[derive(Clone)]
struct Block {
    binding: ManifestStorageParam,
    parameter: ManifestParam,
    scalar: Scalar,
    width: usize,
    stride: usize,
    bytes: Vec<u8>,
    count: usize,
}

/// A complete CPU candidate. Hosts allocate and validate its GPU resources before
/// replacing an installed set. Failed edits preserve every existing value/byte.
#[derive(Clone)]
pub struct StorageSet {
    blocks: Vec<Block>,
    values: Map<String, Value>,
    max_buffer_bytes: u64,
}

pub struct StorageUpload<'a> {
    pub definition: &'a ManifestStorageParam,
    pub bytes: &'a [u8],
    pub element_count: usize,
    pub stride: usize,
}

fn invalid(name: &str, reason: impl Into<String>) -> RuntimeError {
    RuntimeError::Parameter {
        name: name.into(),
        reason: reason.into(),
    }
}

fn element(ty: &str) -> Option<(Scalar, usize)> {
    Some(match ty {
        "f32" => (Scalar::Float, 1),
        "i32" => (Scalar::Int, 1),
        "u32" => (Scalar::UInt, 1),
        "bool" => (Scalar::Bool, 1),
        "vec2" => (Scalar::Float, 2),
        "vec3" => (Scalar::Float, 3),
        "vec4" => (Scalar::Float, 4),
        _ => return None,
    })
}

impl StorageSet {
    pub fn new(
        parameters: &[ManifestParam],
        bindings: Vec<ManifestStorageParam>,
        limits: StorageLimits,
        occupied: impl IntoIterator<Item = (u32, u32)>,
    ) -> Result<Self, RuntimeError> {
        if u32::try_from(bindings.len())
            .ok()
            .is_none_or(|n| n > limits.max_buffers_per_stage)
        {
            return Err(invalid(
                "storage",
                "storage buffer count exceeds device limit",
            ));
        }
        let mut used: BTreeSet<_> = occupied.into_iter().collect();
        let mut names = BTreeSet::new();
        let mut blocks = Vec::new();
        let mut defaults = Map::new();
        for binding in bindings {
            let name = &binding.name;
            if !names.insert(name.clone()) {
                return Err(invalid(name, "duplicate storage parameter"));
            }
            if binding.group >= limits.max_bind_groups
                || binding.binding >= limits.max_bindings_per_group
            {
                return Err(invalid(name, "storage binding exceeds device limits"));
            }
            if !used.insert((binding.group, binding.binding)) {
                return Err(invalid(
                    name,
                    "storage binding collides with another resource",
                ));
            }
            let mut matches = parameters.iter().filter(|param| param.name == *name);
            let parameter = matches
                .next()
                .ok_or_else(|| invalid(name, "missing parameter declaration"))?;
            if matches.next().is_some() || parameter.ty != binding.ty {
                return Err(invalid(
                    name,
                    "ambiguous or mismatched parameter declaration",
                ));
            }
            if parameter.min.is_some_and(|v| !v.is_finite())
                || parameter.max.is_some_and(|v| !v.is_finite())
                || matches!((parameter.min, parameter.max), (Some(min), Some(max)) if min > max)
            {
                return Err(invalid(name, "invalid declared range"));
            }
            let ty = binding
                .ty
                .trim()
                .strip_prefix("array<")
                .and_then(|s| s.strip_suffix('>'))
                .map(str::trim)
                .ok_or_else(|| invalid(name, "expected a dynamic array type"))?;
            let (scalar, width) = element(ty)
                .ok_or_else(|| invalid(name, "unsupported dynamic array element type"))?;
            for info in [binding.param_type.as_ref(), parameter.param_type.as_ref()]
                .into_iter()
                .flatten()
            {
                if info.name != "array"
                    || info.size.is_some()
                    || info.params.as_deref() != Some(&[ty.to_string()])
                {
                    return Err(invalid(
                        name,
                        "array metadata does not match the declared type",
                    ));
                }
            }
            let default = parameter
                .default
                .get("values")
                .unwrap_or(&parameter.default)
                .clone();
            defaults.insert(name.clone(), default);
            blocks.push(Block {
                binding,
                parameter: parameter.clone(),
                scalar,
                width,
                stride: if width == 3 { 16 } else { width * 4 },
                bytes: Vec::new(),
                count: 0,
            });
        }
        let mut result = Self {
            blocks,
            values: Map::new(),
            max_buffer_bytes: limits.max_buffer_bytes,
        };
        result.update(&defaults)?;
        Ok(result)
    }

    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }

    pub fn uploads(&self) -> impl Iterator<Item = StorageUpload<'_>> {
        self.blocks.iter().map(|block| StorageUpload {
            definition: &block.binding,
            bytes: &block.bytes,
            element_count: block.count,
            stride: block.stride,
        })
    }

    pub fn update(&mut self, updates: &Map<String, Value>) -> Result<(), RuntimeError> {
        let mut staged = Vec::new();
        for (name, value) in updates {
            let (index, block) = self
                .blocks
                .iter()
                .enumerate()
                .find(|(_, b)| b.binding.name == *name)
                .ok_or_else(|| invalid(name, "unknown storage parameter"))?;
            let values = value
                .as_array()
                .ok_or_else(|| invalid(name, "expected an array"))?;
            // WebGPU requires at least one element in a runtime-array binding.
            // The logical host array remains empty; its backing element is zero.
            let size = values
                .len()
                .max(1)
                .checked_mul(block.stride)
                .filter(|size| u64::try_from(*size).is_ok_and(|size| size <= self.max_buffer_bytes))
                .ok_or_else(|| invalid(name, "storage buffer size exceeds device limit"))?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(size)
                .map_err(|e| invalid(name, format!("cannot allocate storage bytes: {e}")))?;
            bytes.resize(size, 0);
            for (item, value) in values.iter().enumerate() {
                let components = if block.width == 1 {
                    std::slice::from_ref(value)
                } else {
                    value
                        .as_array()
                        .filter(|v| v.len() == block.width)
                        .map(Vec::as_slice)
                        .ok_or_else(|| {
                            invalid(
                                name,
                                format!("expected {} components per element", block.width),
                            )
                        })?
                };
                for (lane, value) in components.iter().enumerate() {
                    let offset = item * block.stride + lane * 4;
                    let packed = encode_scalar(&block.parameter, block.scalar, value)?;
                    bytes[offset..offset + 4].copy_from_slice(&packed.to_le_bytes());
                }
            }
            staged.push((index, bytes, values.len()));
        }
        for (index, bytes, count) in staged {
            self.blocks[index].bytes = bytes;
            self.blocks[index].count = count;
        }
        self.values.extend(updates.clone());
        Ok(())
    }
}
