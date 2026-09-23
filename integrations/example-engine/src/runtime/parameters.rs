//! The current fullscreen instance ABI stores 64 tightly packed f32 components.
//! Integer values must be exactly representable by that transport; silently
//! rounding a requested integer would change the authored program's inputs.
use std::collections::BTreeSet;

use fresco_artifact::{ManifestCanvas, ManifestParam};
use serde_json::{Map, Value};

use super::RuntimeError;
use super::storage::{StorageLimits, StorageSet};

pub const PARAMETER_COMPONENTS: usize = 64;
pub const PARAMETER_BYTES: usize = PARAMETER_COMPONENTS * 4;
pub const PARAMETER_OFFSET: usize = 48;
pub const INSTANCE_BYTES: usize = PARAMETER_OFFSET + PARAMETER_BYTES;

#[derive(Clone, Copy)]
pub(crate) enum Scalar {
    Float,
    Int,
    UInt,
    Bool,
}

#[derive(Clone)]
struct Layout {
    scalar: Scalar,
    width: usize,
    matrix: Option<usize>,
    length: Option<usize>,
    offset: usize,
}

/// CPU-owned input state. A failed batch changes neither values nor packed bytes.
#[derive(Clone)]
pub struct ParameterSet {
    definitions: Vec<ManifestParam>,
    layouts: Vec<Layout>,
    values: Map<String, Value>,
    bytes: [u8; PARAMETER_BYTES],
}

/// Uniform and storage input values committed as one batch. GPU hosts prepare
/// resources from a cloned candidate before installing it.
#[derive(Clone)]
pub struct CanvasParameters {
    pub(crate) instance: ParameterSet,
    pub(crate) storage: StorageSet,
}

impl CanvasParameters {
    pub fn new(canvas: &ManifestCanvas, limits: StorageLimits) -> Result<Self, RuntimeError> {
        super::storage::validate_canvas_storage_count(canvas, limits)?;
        Ok(Self {
            instance: ParameterSet::for_canvas(canvas)?,
            storage: StorageSet::new(
                &canvas.params,
                canvas.storage_params.clone(),
                limits,
                canvas
                    .engine_pass
                    .iter()
                    .map(|pass| (pass.instance_uniform_group, pass.instance_uniform_binding))
                    .chain(
                        canvas
                            .global_uniforms
                            .iter()
                            .map(|def| (def.group, def.binding)),
                    )
                    .chain(canvas.textures.iter().map(|def| (def.group, def.binding)))
                    .chain(canvas.sampler.iter().map(|def| (def.group, def.binding)))
                    .chain(
                        canvas
                            .path_buffers
                            .iter()
                            .map(|def| (def.group, def.binding)),
                    ),
            )?,
        })
    }

    pub fn values(&self) -> Map<String, Value> {
        let mut values = self.instance.values().clone();
        values.extend(self.storage.values().clone());
        values
    }

    pub fn update(&mut self, updates: &Map<String, Value>) -> Result<(), RuntimeError> {
        let mut instance = self.instance.clone();
        let mut storage = self.storage.clone();
        let (storage_updates, instance_updates) = updates
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .partition(|(name, _)| storage.values().contains_key(name));
        instance.update(&instance_updates)?;
        storage.update(&storage_updates)?;
        self.instance = instance;
        self.storage = storage;
        Ok(())
    }
}

fn error(name: &str, reason: impl Into<String>) -> RuntimeError {
    RuntimeError::Parameter {
        name: name.into(),
        reason: reason.into(),
    }
}

fn element(ty: &str) -> Option<(Scalar, usize)> {
    Some(match ty {
        "f32" | "float" | "scalar" => (Scalar::Float, 1),
        "i32" | "int" => (Scalar::Int, 1),
        "u32" => (Scalar::UInt, 1),
        "bool" => (Scalar::Bool, 1),
        "vec2" => (Scalar::Float, 2),
        "vec3" => (Scalar::Float, 3),
        "vec4" | "color" | "mat2" => (Scalar::Float, 4),
        "mat3" => (Scalar::Float, 9),
        "mat4" => (Scalar::Float, 16),
        _ => return None,
    })
}

impl ParameterSet {
    /// Instance parameters only; reflected storage arrays use StorageSet.
    pub fn for_canvas(canvas: &ManifestCanvas) -> Result<Self, RuntimeError> {
        Self::new(
            canvas
                .params
                .iter()
                .filter(|param| {
                    !canvas
                        .storage_params
                        .iter()
                        .any(|storage| storage.name == param.name)
                })
                .cloned()
                .collect(),
        )
    }

    pub fn new(definitions: Vec<ManifestParam>) -> Result<Self, RuntimeError> {
        Self::with_vectors(definitions, false)
    }

    pub(crate) fn for_style(definition: ManifestParam) -> Result<Self, RuntimeError> {
        Self::with_vectors(vec![definition], true)
    }

    fn with_vectors(definitions: Vec<ManifestParam>, vectors: bool) -> Result<Self, RuntimeError> {
        let mut layouts = Vec::new();
        let mut offset = 0usize;
        let mut names = BTreeSet::new();
        let mut defaults = Map::new();
        for def in &definitions {
            if !names.insert(&def.name) {
                return Err(error(&def.name, "duplicate declaration"));
            }
            if def.min.is_some_and(|v| !v.is_finite())
                || def.max.is_some_and(|v| !v.is_finite())
                || matches!((def.min, def.max), (Some(min), Some(max)) if min > max)
            {
                return Err(error(&def.name, "invalid declared range"));
            }
            let ty = def.ty.trim();
            let (scalar, width, length, matrix) =
                if let Some(inner) = ty.strip_prefix("array<").and_then(|s| s.strip_suffix('>')) {
                    let (ty, count) = inner.split_once(',').ok_or_else(|| {
                        error(&def.name, "dynamic arrays require storage bindings")
                    })?;
                    let count = count
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| error(&def.name, "invalid array length"))?;
                    if count == 0 {
                        return Err(error(&def.name, "array length must be positive"));
                    }
                    let (scalar, width) = element(ty.trim())
                        .ok_or_else(|| error(&def.name, "unsupported array element type"))?;
                    let matrix = match ty.trim() {
                        "mat2" => Some(2),
                        "mat3" => Some(3),
                        "mat4" => Some(4),
                        _ => None,
                    };
                    (scalar, width, Some(count), matrix)
                } else {
                    // These are the standalone parameter types accepted by the
                    // compiler's executable fullscreen entry ABI.
                    if !matches!(
                        ty,
                        "f32" | "float" | "scalar" | "i32" | "int" | "u32" | "bool" | "color"
                    ) && !(vectors && matches!(ty, "vec2" | "vec3" | "vec4"))
                    {
                        return Err(error(
                            &def.name,
                            format!("unsupported fullscreen parameter type `{ty}`"),
                        ));
                    }
                    let (scalar, width) = element(ty).expect("supported type");
                    (scalar, width, None, None)
                };
            let count = width
                .checked_mul(length.unwrap_or(1))
                .and_then(|n| offset.checked_add(n))
                .filter(|end| *end <= PARAMETER_COMPONENTS)
                .ok_or_else(|| error(&def.name, "fullscreen parameters exceed 64 components"))?;
            layouts.push(Layout {
                scalar,
                width,
                matrix,
                length,
                offset,
            });
            offset = count;
            // Array defaults carry an envelope in the compiler artifact. Host
            // updates and the public value map use ordinary nested JSON arrays.
            let default = if length.is_some() {
                def.default.get("values").unwrap_or(&def.default).clone()
            } else {
                def.default.clone()
            };
            defaults.insert(def.name.clone(), default);
        }
        let mut result = Self {
            definitions,
            layouts,
            values: Map::new(),
            bytes: [0; PARAMETER_BYTES],
        };
        result.update(&defaults)?;
        Ok(result)
    }

    pub fn definitions(&self) -> &[ManifestParam] {
        &self.definitions
    }
    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }
    pub fn bytes(&self) -> &[u8; PARAMETER_BYTES] {
        &self.bytes
    }

    pub fn update(&mut self, updates: &Map<String, Value>) -> Result<(), RuntimeError> {
        let mut bytes = self.bytes;
        for (name, value) in updates {
            let index = self
                .definitions
                .iter()
                .position(|def| def.name == *name)
                .ok_or_else(|| error(name, "unknown parameter"))?;
            let def = &self.definitions[index];
            let layout = &self.layouts[index];
            let mut components = Vec::new();
            if let Some(length) = layout.length {
                let elements = value
                    .as_array()
                    .filter(|v| v.len() == length)
                    .ok_or_else(|| {
                        error(name, format!("expected an array of {length} elements"))
                    })?;
                for value in elements {
                    encode_element(def, layout, value, &mut components)?;
                }
            } else {
                encode_element(def, layout, value, &mut components)?;
            }
            for (i, component) in components.iter().enumerate() {
                let offset = (layout.offset + i) * 4;
                bytes[offset..offset + 4].copy_from_slice(&component.to_le_bytes());
            }
        }
        self.bytes = bytes;
        for (name, value) in updates {
            self.values.insert(name.clone(), value.clone());
        }
        Ok(())
    }
}

fn encode_element(
    def: &ManifestParam,
    layout: &Layout,
    value: &Value,
    output: &mut Vec<f32>,
) -> Result<(), RuntimeError> {
    if let Some(size) = layout.matrix {
        let columns = value
            .as_array()
            .filter(|v| v.len() == size)
            .ok_or_else(|| error(&def.name, format!("expected {size} matrix columns")))?;
        let column = Layout {
            scalar: layout.scalar,
            width: size,
            matrix: None,
            length: None,
            offset: 0,
        };
        for value in columns {
            encode_element(def, &column, value, output)?;
        }
        return Ok(());
    }
    let values = if layout.width == 1 {
        std::slice::from_ref(value)
    } else {
        value
            .as_array()
            .filter(|v| v.len() == layout.width)
            .map(Vec::as_slice)
            .ok_or_else(|| error(&def.name, format!("expected {} components", layout.width)))?
    };
    for value in values {
        output.push(encode_scalar(def, layout.scalar, value)?);
    }
    Ok(())
}

// The current canvas and surface transports encode scalar lanes as f32, including integers
// and booleans. Keep their validation identical until the compiler ABI changes.
pub(crate) fn encode_scalar(
    def: &ManifestParam,
    scalar: Scalar,
    value: &Value,
) -> Result<f32, RuntimeError> {
    let number = match scalar {
        Scalar::Bool => {
            if value
                .as_bool()
                .ok_or_else(|| error(&def.name, "expected a boolean"))?
            {
                1.0
            } else {
                0.0
            }
        }
        _ => value
            .as_f64()
            .ok_or_else(|| error(&def.name, "expected a number"))?,
    };
    let packed = number as f32;
    if !number.is_finite() || !packed.is_finite() {
        return Err(error(&def.name, "value must be finite and fit f32"));
    }
    let bounds = match scalar {
        Scalar::Int => Some((f64::from(i32::MIN), f64::from(i32::MAX))),
        Scalar::UInt => Some((0.0, f64::from(u32::MAX))),
        _ => None,
    };
    if let Some((min, max)) = bounds
        && (number.fract() != 0.0 || number < min || number > max || f64::from(packed) != number)
    {
        return Err(error(
            &def.name,
            "integer must fit its declared type and be exactly representable by the shader f32 transport",
        ));
    }
    // Bounds constrain shader values. Decimal host inputs which round to the
    // same f32 endpoint must not be rejected because JSON carries f64 numbers.
    let bounded_number = if matches!(scalar, Scalar::Float) {
        f64::from(packed)
    } else {
        number
    };
    let bound = |value: f64| {
        if matches!(scalar, Scalar::Float) {
            f64::from(value as f32)
        } else {
            value
        }
    };
    if def.min.is_some_and(|min| bounded_number < bound(min))
        || def.max.is_some_and(|max| bounded_number > bound(max))
    {
        return Err(error(&def.name, "value is outside the declared range"));
    }
    Ok(packed)
}
