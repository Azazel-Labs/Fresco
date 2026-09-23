//! Engine-independent contracts for reusable GPU work.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestTechnique {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub name: String,
    pub resources: Vec<ManifestTechniqueResource>,
    pub steps: Vec<ManifestTechniqueStep>,
    pub outputs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestResourceType {
    Buffer {
        element: String,
        bytes: u32,
    },
    Image {
        format: String,
        extent: ManifestImageExtent,
    },
    Uniform {
        element: String,
        bytes: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestResourceSource {
    Allocate {
        pool: Option<String>,
    },
    External {
        provider: String,
    },
    Asset {
        asset: String,
    },
    /// The host binds this instance slot to a concrete invocation of the producer.
    TechniqueOutput {
        instance: String,
        technique: String,
        output: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestTechniqueResource {
    pub name: String,
    pub descriptor: ManifestResourceType,
    pub source: ManifestResourceSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestTechniqueStep {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attachments: BTreeMap<String, ManifestAttachmentOps>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<String>,
    pub name: String,
    pub pass: String,
    pub operation: ManifestTechniqueOperation,
    pub bindings: BTreeMap<String, String>,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestTechniqueOperation {
    Compute {
        entry: String,
        /// Logical invocation extent; the engine rounds up by workgroup size.
        extent: ManifestDispatchExtent,
        workgroup_size: [u32; 3],
    },
    Draw {
        vertex: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fragment: Option<String>,
        vertices: u32,
        instances: ManifestDrawCount,
        #[serde(with = "color_slots")]
        #[cfg_attr(feature = "typescript", ts(as = "BTreeMap<u32, String>"))]
        colors: BTreeMap<u32, String>,
        depth: Option<String>,
    },
}

impl ManifestTechniqueOperation {
    /// Checked dispatch dimensions without addition overflow at the u32 boundary.
    pub fn workgroups(&self) -> Result<Option<[u32; 3]>, &'static str> {
        match self {
            Self::Compute {
                extent,
                workgroup_size,
                ..
            } => {
                let ManifestDispatchExtent::Fixed(extent) = extent else {
                    return Err("dispatch requires an invocation extent");
                };
                let mut groups = [0; 3];
                for axis in 0..3 {
                    if workgroup_size[axis] == 0 {
                        return Err("workgroup dimensions must be positive");
                    }
                    groups[axis] = extent[axis].div_ceil(workgroup_size[axis]);
                }
                Ok(Some(groups))
            }
            Self::Draw { .. } => Ok(None),
        }
    }
}

/// Image sizing is resolved for each invocation, never inferred from shader names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestImageExtent {
    Viewport,
    Fixed { width: u32, height: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(untagged)]
pub enum ManifestDispatchExtent {
    Fixed([u32; 3]),
    Parameter { parameter: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(untagged)]
pub enum ManifestDrawCount {
    Fixed(u32),
    Parameter { parameter: String },
}

// Internally tagged enum content does not use JSON's numeric map-key coercion.
// Keep attachment locations numeric in Rust and canonical JSON object keys on wire.
pub(crate) mod color_slots {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        values: &BTreeMap<u32, String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(values.len()))?;
        for (slot, value) in values {
            map.serialize_entry(&slot.to_string(), value)?;
        }
        map.end()
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<u32, String>, D::Error> {
        let values = BTreeMap::<String, String>::deserialize(deserializer)?;
        values
            .into_iter()
            .map(|(key, value)| {
                key.parse()
                    .map(|slot| (slot, value))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

/// Explicit attachment preservation for a draw. Absent entries clear and store.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestAttachmentOps {
    pub load: bool,
    pub store: bool,
}
