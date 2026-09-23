//! Pack reflected buffers without assuming engine field names or fixed offsets.
use std::collections::HashSet;

use fresco_artifact::{ManifestGlobalUniform, ManifestGlobalUniformField};

use super::RuntimeError;

/// Supply the limits of the device on which these resources will be installed.
#[derive(Debug, Clone, Copy)]
pub struct UniformLimits {
    pub max_bind_groups: u32,
    pub max_bindings_per_bind_group: u32,
    pub max_uniform_buffer_binding_size: u64,
}

/// Values retain their scalar encoding; integer values are never converted to floats.
#[derive(Debug, Clone)]
pub enum UniformValue {
    F32(Vec<f32>),
    I32(Vec<i32>),
    U32(Vec<u32>),
}

#[derive(Debug)]
struct Block {
    definition: ManifestGlobalUniform,
    data: Vec<u8>,
    staged: Vec<u8>,
}

/// Validated, reusable CPU buffers. Uploads become visible only after a complete
/// successful update, so a host can enqueue all GPU writes transactionally.
#[derive(Debug)]
pub struct UniformSet {
    blocks: Vec<Block>,
    initialized: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct UniformUpload<'a> {
    pub group: u32,
    pub binding: u32,
    pub bytes: &'a [u8],
}

fn layout_error(def: &ManifestGlobalUniform, reason: impl Into<String>) -> RuntimeError {
    RuntimeError::UniformLayout {
        uniform: def.name.clone(),
        reason: reason.into(),
    }
}

fn validate(def: &ManifestGlobalUniform, limits: UniformLimits) -> Result<(), RuntimeError> {
    if def.group >= limits.max_bind_groups || def.binding >= limits.max_bindings_per_bind_group {
        return Err(layout_error(def, "binding exceeds device limits"));
    }
    if def.byte_size == 0
        || !def.byte_size.is_multiple_of(16)
        || u64::from(def.byte_size) > limits.max_uniform_buffer_binding_size
    {
        return Err(layout_error(
            def,
            "buffer size must be a nonzero multiple of 16 within the device limit",
        ));
    }
    let mut names = HashSet::new();
    let mut ranges = Vec::new();
    for field in &def.fields {
        if !names.insert(&field.name) {
            return Err(layout_error(
                def,
                format!("duplicate field `{}`", field.name),
            ));
        }
        if !matches!(field.scalar_type.as_str(), "f32" | "i32" | "u32") {
            return Err(layout_error(
                def,
                format!(
                    "unsupported scalar type for `{}`: {}",
                    field.name, field.scalar_type
                ),
            ));
        }
        let alignment = match field.components {
            1 => 4,
            2 => 8,
            3 | 4 => 16,
            _ => {
                return Err(layout_error(
                    def,
                    format!("`{}` must have 1 to 4 components", field.name),
                ));
            }
        };
        let end = field
            .offset
            .checked_add(field.components * 4)
            .ok_or_else(|| layout_error(def, format!("field `{}` range overflows", field.name)))?;
        if !field.offset.is_multiple_of(alignment) || end > def.byte_size {
            return Err(layout_error(
                def,
                format!("field `{}` is misaligned or outside its buffer", field.name),
            ));
        }
        ranges.push((field.offset, end));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(layout_error(def, "fields overlap"));
    }
    Ok(())
}

fn zeroed_buffer(def: &ManifestGlobalUniform) -> Result<Vec<u8>, RuntimeError> {
    let size = usize::try_from(def.byte_size)
        .map_err(|_| layout_error(def, "buffer size exceeds host address space"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|e| layout_error(def, format!("cannot allocate buffer: {e}")))?;
    bytes.resize(size, 0);
    Ok(bytes)
}

impl UniformSet {
    pub fn new(
        definitions: Vec<ManifestGlobalUniform>,
        limits: UniformLimits,
    ) -> Result<Self, RuntimeError> {
        let mut bindings = HashSet::new();
        let mut names = HashSet::new();
        for def in &definitions {
            validate(def, limits)?;
            if !bindings.insert((def.group, def.binding)) {
                return Err(layout_error(def, "duplicate group/binding pair"));
            }
            if !names.insert(&def.name) {
                return Err(layout_error(def, "duplicate uniform name"));
            }
        }
        let blocks = definitions
            .into_iter()
            .map(|definition| {
                Ok(Block {
                    data: zeroed_buffer(&definition)?,
                    staged: zeroed_buffer(&definition)?,
                    definition,
                })
            })
            .collect::<Result<_, RuntimeError>>()?;
        Ok(Self {
            blocks,
            initialized: false,
        })
    }

    pub fn definitions(&self) -> impl Iterator<Item = &ManifestGlobalUniform> {
        self.blocks.iter().map(|block| &block.definition)
    }

    /// The resolver supplies every reflected field. No missing value defaults to zero.
    /// Failure leaves all previously committed bytes intact, including other buffers.
    pub fn update(
        &mut self,
        mut resolve: impl FnMut(
            &ManifestGlobalUniform,
            &ManifestGlobalUniformField,
        ) -> Option<UniformValue>,
    ) -> Result<(), RuntimeError> {
        for block in &mut self.blocks {
            block.staged.fill(0);
            for field in &block.definition.fields {
                let value = resolve(&block.definition, field).ok_or_else(|| {
                    RuntimeError::MissingUniformValue {
                        uniform: block.definition.name.clone(),
                        field: field.name.clone(),
                    }
                })?;
                let invalid = |reason: &str| RuntimeError::UniformValue {
                    uniform: block.definition.name.clone(),
                    field: field.name.clone(),
                    reason: reason.into(),
                };
                let (kind, count) = match &value {
                    UniformValue::F32(values) => {
                        if values.iter().any(|v| !v.is_finite()) {
                            return Err(invalid("components must be finite"));
                        }
                        ("f32", values.len())
                    }
                    UniformValue::I32(values) => ("i32", values.len()),
                    UniformValue::U32(values) => ("u32", values.len()),
                };
                if kind != field.scalar_type || u32::try_from(count).ok() != Some(field.components)
                {
                    return Err(invalid(
                        "scalar type or component count does not match reflection",
                    ));
                }
                let offset = usize::try_from(field.offset)
                    .map_err(|_| invalid("offset exceeds host address space"))?;
                for (index, destination) in block.staged[offset..offset + count * 4]
                    .as_chunks_mut::<4>()
                    .0
                    .iter_mut()
                    .enumerate()
                {
                    let bytes = match &value {
                        UniformValue::F32(values) => values[index].to_le_bytes(),
                        UniformValue::I32(values) => values[index].to_le_bytes(),
                        UniformValue::U32(values) => values[index].to_le_bytes(),
                    };
                    destination.copy_from_slice(&bytes);
                }
            }
        }
        for block in &mut self.blocks {
            std::mem::swap(&mut block.data, &mut block.staged);
        }
        self.initialized = true;
        Ok(())
    }

    /// `None` until the first successful update prevents uploading uninitialized host data.
    pub fn uploads(&self) -> Option<impl Iterator<Item = UniformUpload<'_>>> {
        self.initialized.then(|| {
            self.blocks.iter().map(|block| UniformUpload {
                group: block.definition.group,
                binding: block.definition.binding,
                bytes: &block.data,
            })
        })
    }
}
