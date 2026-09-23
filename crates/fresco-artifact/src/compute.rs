//! Binding provenance for authored compute operations. These roles are emitted
//! by lowering; hosts must not recover them from generated shader identifiers.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestComputeInvocation {
    pub material: String,
    pub operation: String,
    pub ordinal: u32,
    pub arguments: std::collections::BTreeMap<String, ManifestComputeArgument>,
    pub bindings: std::collections::BTreeMap<String, ManifestComputeBindingSource>,
    pub output: ManifestComputeOutput,
    pub threads: Vec<ManifestComputeExpression>,
    pub requirements: Vec<ManifestComputeExpression>,
    /// Producer program IDs whose GPU data is read by this kernel.
    pub dependencies: Vec<String>,
    /// Renderer recipe nodes that produce engine resources read by this kernel.
    pub engine_dependencies: Vec<String>,
    /// Producer allocation metadata needed before preparing this invocation.
    pub metadata_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestComputeArgument {
    Constant {
        ty: String,
        #[cfg_attr(feature = "typescript", ts(type = "unknown"))]
        value: serde_json::Value,
    },
    Setting {
        ty: String,
        name: String,
        offset: u32,
    },
    Output {
        producer: String,
    },
    Geometry {
        producer: String,
        hook: String,
        node: String,
        resource_type: String,
    },
    External {
        resource: String,
        ty: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestComputeOutput {
    pub binding: String,
    pub ty: String,
    pub layout: ManifestComputeOutputLayout,
    pub extents: Vec<ManifestComputeExpression>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestComputeOutputLayout {
    Buffer { element: String, stride: u32 },
    Image { format: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ComputeScalarType {
    Bool,
    U32,
    I32,
    F32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ComputeScalar {
    Bool(bool),
    U32(u32),
    I32(i32),
    F32(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ComputeBinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestComputeExpression {
    Constant {
        value: ComputeScalar,
    },
    Input {
        parameter: String,
        member: Option<String>,
        ty: ComputeScalarType,
    },
    Negate {
        value: Box<Self>,
    },
    Binary {
        op: ComputeBinaryOp,
        left: Box<Self>,
        right: Box<Self>,
    },
    Convert {
        ty: ComputeScalarType,
        value: Box<Self>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManifestComputeBindingSource {
    Value {
        parameter: String,
        ty: String,
    },
    Resource {
        parameter: String,
    },
    Geometry {
        parameter: String,
        member: String,
        role: ManifestComputeGeometryRole,
    },
    Dimension {
        parameter: String,
        axis: ManifestComputeDimension,
    },
    Output,
    Dispatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ManifestComputeGeometryRole {
    Vertices,
    Indices,
    VertexCount,
    IndexCount,
    Bounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ManifestComputeDimension {
    Count,
    Width,
    Height,
}

/// Shader reads, before parameter names are resolved to invocation resources.
/// Reading a logical dimension requires allocation metadata, not completion of
/// the producer's GPU work. Host allocation/dispatch expressions add their own
/// metadata dependencies separately.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ComputeShaderReads {
    pub data: BTreeSet<String>,
    pub dimensions: BTreeSet<String>,
    pub values: BTreeSet<String>,
}

impl crate::ManifestGpuProgram {
    pub fn compute_shader_reads(&self, entry: &str) -> Result<ComputeShaderReads, String> {
        if !self
            .entries
            .iter()
            .any(|e| e.entry == entry && e.stage == "compute")
        {
            return Err(format!("unknown compute entry `{entry}`"));
        }
        let names: BTreeSet<_> = self.bindings.iter().map(|b| &b.name).collect();
        if names.len() != self.bindings.len() || names != self.compute_bindings.keys().collect() {
            return Err("compute binding provenance is incomplete or ambiguous".into());
        }
        let mut reads = ComputeShaderReads::default();
        for binding in &self.bindings {
            let Some(access) = binding.entry_access.get(entry) else {
                continue;
            };
            let source = &self.compute_bindings[&binding.name];
            if matches!(source, ManifestComputeBindingSource::Output) {
                if access != "write" {
                    return Err("owned compute output must be write-only".into());
                }
                continue;
            }
            if access != "read" {
                return Err("compute inputs must be read-only".into());
            }
            match source {
                ManifestComputeBindingSource::Value { parameter, .. } => {
                    reads.values.insert(parameter.clone());
                }
                ManifestComputeBindingSource::Resource { parameter } => {
                    if binding.query_only_entries.contains(entry) {
                        reads.dimensions.insert(parameter.clone());
                    } else {
                        reads.data.insert(parameter.clone());
                    }
                }
                ManifestComputeBindingSource::Geometry {
                    parameter,
                    role:
                        ManifestComputeGeometryRole::Vertices | ManifestComputeGeometryRole::Indices,
                    ..
                } => {
                    if binding.query_only_entries.contains(entry) {
                        reads.dimensions.insert(parameter.clone());
                    } else {
                        reads.data.insert(parameter.clone());
                    }
                }
                ManifestComputeBindingSource::Geometry {
                    parameter,
                    role:
                        ManifestComputeGeometryRole::VertexCount
                        | ManifestComputeGeometryRole::IndexCount
                        | ManifestComputeGeometryRole::Bounds,
                    ..
                }
                | ManifestComputeBindingSource::Dimension { parameter, .. } => {
                    reads.dimensions.insert(parameter.clone());
                }
                ManifestComputeBindingSource::Dispatch => {}
                ManifestComputeBindingSource::Output => unreachable!("output handled above"),
            }
        }
        Ok(reads)
    }
}
