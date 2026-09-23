//! One 16-byte float lane per reflected style parameter, addressed per material.
use super::{RuntimeError, parameters::ParameterSet};
use fresco_artifact::{ManifestParam, ManifestRoot, ManifestSurface};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct StyleParameters {
    lanes: Vec<(u32, String, Lane)>,
}

#[derive(Clone)]
enum Lane {
    Float(Box<ParameterSet>),
    Exact {
        definition: Box<ManifestParam>,
        values: Map<String, Value>,
        bytes: [u8; 16],
    },
}

impl Lane {
    fn new(definition: ManifestParam) -> Result<Self, RuntimeError> {
        if !matches!(definition.ty.as_str(), "u32" | "i32" | "bool") {
            return Ok(Self::Float(Box::new(ParameterSet::for_style(definition)?)));
        }
        if definition.min.is_some_and(|v| !v.is_finite())
            || definition.max.is_some_and(|v| !v.is_finite())
            || matches!((definition.min, definition.max), (Some(min), Some(max)) if min > max)
            || (definition.ty == "bool" && (definition.min.is_some() || definition.max.is_some()))
        {
            return Err(invalid("invalid declared style setting range"));
        }
        let updates = Map::from_iter([(definition.name.clone(), definition.default.clone())]);
        let mut lane = Self::Exact {
            definition: Box::new(definition),
            values: Map::new(),
            bytes: [0; 16],
        };
        lane.update(&updates)?;
        Ok(lane)
    }

    fn values(&self) -> &Map<String, Value> {
        match self {
            Self::Float(state) => state.values(),
            Self::Exact { values, .. } => values,
        }
    }

    fn bytes(&self) -> &[u8] {
        match self {
            Self::Float(state) => &state.bytes()[..16],
            Self::Exact { bytes, .. } => bytes,
        }
    }

    fn update(&mut self, updates: &Map<String, Value>) -> Result<(), RuntimeError> {
        let Self::Exact {
            definition,
            values,
            bytes,
        } = self
        else {
            let Self::Float(state) = self else {
                unreachable!()
            };
            return state.update(updates);
        };
        let value = &updates[&definition.name];
        let bits = match definition.ty.as_str() {
            "u32" => value
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| invalid("expected u32 style setting"))?,
            "i32" => {
                let signed = value
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or_else(|| invalid("expected i32 style setting"))?;
                u32::from_le_bytes(signed.to_le_bytes())
            }
            "bool" => u32::from(
                value
                    .as_bool()
                    .ok_or_else(|| invalid("expected boolean style setting"))?,
            ),
            _ => unreachable!("exact setting type"),
        };
        if let Some(number) = value.as_f64()
            && (definition.min.is_some_and(|min| number < min)
                || definition.max.is_some_and(|max| number > max))
        {
            return Err(invalid("style setting value is outside its declared range"));
        }
        // Two exactly representable 16-bit halves preserve every integer bit.
        bytes[..4].copy_from_slice(
            &(f32::from(u16::try_from(bits & 0xffff).expect("masked half"))).to_le_bytes(),
        );
        bytes[4..8].copy_from_slice(
            &(f32::from(u16::try_from(bits >> 16).expect("upper half"))).to_le_bytes(),
        );
        values.insert(definition.name.clone(), value.clone());
        Ok(())
    }
}

fn invalid(reason: &str) -> RuntimeError {
    RuntimeError::Parameter {
        name: "style".into(),
        reason: reason.into(),
    }
}

impl StyleParameters {
    pub fn new(surface: &ManifestSurface) -> Result<Self, RuntimeError> {
        let mut lanes = Vec::new();
        let mut names = BTreeSet::new();
        if let Some(settings) = &surface.settings {
            for selection in &settings.implementations {
                for (index, definition) in selection.parameters.iter().enumerate() {
                    if !matches!(
                        definition.ty.as_str(),
                        "f32" | "vec2" | "vec3" | "vec4" | "color" | "u32" | "i32" | "bool"
                    ) {
                        return Err(invalid("unsupported runtime style setting type"));
                    }
                    let offset = u32::try_from(index)
                        .ok()
                        .and_then(|i| selection.settings_offset.checked_add(i))
                        .ok_or_else(|| invalid("style settings offset exceeds u32"))?;
                    let mut definition = definition.clone();
                    definition.name = format!("{}.{}", selection.name, definition.name);
                    if !names.insert(definition.name.clone()) {
                        return Err(invalid("duplicate style parameter"));
                    }
                    lanes.push((offset, definition.name.clone(), Lane::new(definition)?));
                }
            }
        }
        Ok(Self { lanes })
    }

    pub fn values(&self) -> Map<String, Value> {
        self.lanes
            .iter()
            .flat_map(|(_, _, state)| state.values().clone())
            .collect()
    }

    #[cfg(feature = "runtime")]
    pub(crate) fn value_at(&self, offset: u32) -> Option<&Value> {
        self.lanes
            .iter()
            .find(|(lane, _, _)| *lane == offset)
            .and_then(|(_, name, state)| state.values().get(name))
    }

    pub fn update(&mut self, updates: &Map<String, Value>) -> Result<(), RuntimeError> {
        for name in updates.keys() {
            if !self.lanes.iter().any(|(_, n, _)| n == name) {
                return Err(invalid("unknown style parameter"));
            }
        }
        let mut candidate = self.clone();
        for (_, name, state) in &mut candidate.lanes {
            if let Some(value) = updates.get(name) {
                state.update(&Map::from_iter([(name.clone(), value.clone())]))?;
            }
        }
        *self = candidate;
        Ok(())
    }

    pub fn uploads(&self) -> impl Iterator<Item = (u32, &[u8])> {
        self.lanes
            .iter()
            .map(|(offset, _, state)| (*offset, &state.bytes()[..16]))
    }

    pub fn initial_bytes(manifest: &ManifestRoot, max_bytes: u64) -> Result<Vec<u8>, RuntimeError> {
        let mut lanes = BTreeMap::new();
        for surface in &manifest.surfaces {
            let state = Self::new(surface)?;
            for (offset, bytes) in state.uploads() {
                if lanes.insert(offset, bytes.to_vec()).is_some() {
                    return Err(invalid("overlapping material style settings records"));
                }
            }
        }
        let count = lanes
            .last_key_value()
            .map_or(1, |(offset, _)| u64::from(*offset) + 1);
        let size = count
            .checked_mul(16)
            .filter(|size| *size <= max_bytes)
            .and_then(|size| usize::try_from(size).ok())
            .ok_or_else(|| invalid("style settings buffer exceeds device limits"))?;
        let mut result = vec![0; size];
        for (offset, bytes) in lanes {
            let start = usize::try_from(u64::from(offset) * 16).expect("validated size");
            result[start..start + 16].copy_from_slice(&bytes);
        }
        Ok(result)
    }
}
