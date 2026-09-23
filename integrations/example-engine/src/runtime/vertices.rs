//! Interleaved vertex uploads from explicit host streams and reflected layouts.
use std::collections::{BTreeMap, HashSet};

use fresco_artifact::ManifestVertexFactory;

use super::RuntimeError;

#[derive(Debug, Clone, Copy)]
pub struct VertexLimits {
    pub max_attributes: u32,
    pub max_stride: u32,
    pub max_buffer_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub enum VertexValues<'a> {
    F32(&'a [f32]),
    I32(&'a [i32]),
    U32(&'a [u32]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scalar {
    Float,
    Signed,
    Unsigned,
}

impl VertexValues<'_> {
    fn layout(self) -> (Scalar, usize) {
        match self {
            Self::F32(values) => (Scalar::Float, values.len()),
            Self::I32(values) => (Scalar::Signed, values.len()),
            Self::U32(values) => (Scalar::Unsigned, values.len()),
        }
    }

    fn bytes(self, index: usize) -> [u8; 4] {
        match self {
            Self::F32(values) => values[index].to_le_bytes(),
            Self::I32(values) => values[index].to_le_bytes(),
            Self::U32(values) => values[index].to_le_bytes(),
        }
    }
}

fn format(name: &str) -> Option<(Scalar, u32)> {
    match name {
        "float32" => Some((Scalar::Float, 1)),
        "float32x2" => Some((Scalar::Float, 2)),
        "float32x3" => Some((Scalar::Float, 3)),
        "float32x4" => Some((Scalar::Float, 4)),
        "sint32" => Some((Scalar::Signed, 1)),
        "sint32x2" => Some((Scalar::Signed, 2)),
        "sint32x3" => Some((Scalar::Signed, 3)),
        "sint32x4" => Some((Scalar::Signed, 4)),
        "uint32" => Some((Scalar::Unsigned, 1)),
        "uint32x2" => Some((Scalar::Unsigned, 2)),
        "uint32x3" => Some((Scalar::Unsigned, 3)),
        "uint32x4" => Some((Scalar::Unsigned, 4)),
        _ => None,
    }
}

/// Produces an owned candidate; a failed update cannot mutate installed geometry.
/// Every reflected attribute needs a stream, including optional/defaulted fields:
/// the current manifest advertises defaults but does not carry their values.
pub fn pack_vertices(
    factory: &ManifestVertexFactory,
    vertex_count: u32,
    streams: &BTreeMap<String, VertexValues<'_>>,
    limits: VertexLimits,
) -> Result<Vec<u8>, RuntimeError> {
    let invalid = |reason: String| RuntimeError::VertexLayout {
        factory: factory.name.clone(),
        reason,
    };
    let stride = factory
        .array_stride
        .filter(|stride| *stride > 0 && stride.is_multiple_of(4) && *stride <= limits.max_stride)
        .ok_or_else(|| invalid("missing, misaligned, or unsupported vertex stride".into()))?;
    if factory.attributes.is_empty()
        || u32::try_from(factory.attributes.len())
            .ok()
            .is_none_or(|count| count > limits.max_attributes)
    {
        return Err(invalid("attribute count exceeds supported layout".into()));
    }
    let mut names = HashSet::new();
    let mut locations = HashSet::new();
    let mut ranges = Vec::new();
    let mut fields = Vec::new();
    for attribute in &factory.attributes {
        if !names.insert(attribute.name.as_str())
            || !locations.insert(attribute.shader_location)
            || attribute.shader_location >= limits.max_attributes
        {
            return Err(invalid(
                "duplicate attribute name/location or location exceeds device limit".into(),
            ));
        }
        let (scalar, components) = attribute
            .gpu_format
            .as_deref()
            .and_then(format)
            .ok_or_else(|| invalid(format!("unsupported GPU format for `{}`", attribute.name)))?;
        let offset = attribute
            .offset
            .filter(|offset| offset.is_multiple_of(4))
            .ok_or_else(|| {
                invalid(format!(
                    "missing or misaligned offset for `{}`",
                    attribute.name
                ))
            })?;
        let end = offset
            .checked_add(components * 4)
            .filter(|end| *end <= stride)
            .ok_or_else(|| {
                invalid(format!(
                    "attribute `{}` exceeds vertex stride",
                    attribute.name
                ))
            })?;
        ranges.push((offset, end));
        let values = *streams
            .get(&attribute.name)
            .ok_or_else(|| invalid(format!("missing vertex stream `{}`", attribute.name)))?;
        let expected = u64::from(vertex_count) * u64::from(components);
        let (supplied_type, count) = values.layout();
        if supplied_type != scalar || u64::try_from(count).ok() != Some(expected) {
            return Err(invalid(format!(
                "stream `{}` has the wrong scalar type or lane count",
                attribute.name
            )));
        }
        if let VertexValues::F32(values) = values
            && values.iter().any(|value| !value.is_finite())
        {
            return Err(invalid(format!(
                "stream `{}` contains non-finite values",
                attribute.name
            )));
        }
        fields.push((offset, components, values));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(invalid("vertex attributes overlap".into()));
    }
    if let Some(unknown) = streams.keys().find(|name| !names.contains(name.as_str())) {
        return Err(invalid(format!("unknown vertex stream `{unknown}`")));
    }
    let size = u64::from(vertex_count) * u64::from(stride);
    let size = usize::try_from(size)
        .ok()
        .filter(|_| size <= limits.max_buffer_bytes)
        .ok_or_else(|| invalid("vertex buffer exceeds device or host limits".into()))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|error| invalid(format!("cannot allocate vertex buffer: {error}")))?;
    bytes.resize(size, 0);
    // The allocation above proves all per-vertex offsets fit the host address space.
    let stride =
        usize::try_from(stride).map_err(|_| invalid("stride exceeds host address space".into()))?;
    for (vertex, destination) in bytes.chunks_exact_mut(stride).enumerate() {
        for &(offset, components, values) in &fields {
            let offset = offset as usize;
            let components = components as usize;
            for (lane, target) in destination[offset..offset + components * 4]
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .enumerate()
            {
                target.copy_from_slice(&values.bytes(vertex * components + lane));
            }
        }
    }
    Ok(bytes)
}
