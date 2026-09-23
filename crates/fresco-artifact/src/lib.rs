//! Shared, GPU-independent Fresco compiler artifact types.
//!
//! The optional `typescript` feature supports the browser contract generator.

use serde::{Deserialize, Serialize};

mod technique;
pub use technique::*;
mod compute;
pub use compute::*;
mod transparent;
pub mod types;
pub use transparent::validate_transparent_queues;

pub const SCHEMA_VERSION: u32 = 12;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
#[cfg_attr(feature = "typescript", ts(concrete(D = serde_json::Value, N = f64), bound = "N: ts_rs::TS"))]
#[serde(bound(deserialize = "D: Deserialize<'de>, N: Deserialize<'de>"))]
pub struct ManifestRoot<D = serde_json::Value, N = f64> {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub techniques: Vec<ManifestTechnique>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gpu_programs: Vec<ManifestGpuProgram>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renderers: Vec<ManifestRenderer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<ManifestTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_profile: Option<String>,
    pub canvases: Vec<ManifestCanvas<D, N>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<ManifestSurface<D, N>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pipelines: Vec<ManifestPipeline>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vertex_factories: Vec<ManifestVertexFactory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_axes: Vec<ManifestConfigAxis>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestGpuProgram {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_invocation: Option<ManifestComputeInvocation>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub compute_bindings: std::collections::BTreeMap<String, ManifestComputeBindingSource>,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub properties: Vec<ManifestEntryProperty>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    pub pass: String,
    pub entries: Vec<ManifestRasterEntry>,
    pub bindings: Vec<ManifestGpuBinding>,
    pub workgroup_size: Option<[u32; 3]>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestGpuBinding {
    /// Entries that query this resource's shape without reading or writing data.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub query_only_entries: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub fields: Vec<ManifestBufferField>,
    #[serde(default)]
    pub entry_access: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_stride: Option<u32>,
    pub name: String,
    pub group: u32,
    pub binding: u32,
    pub kind: String,
    pub ty: String,
    pub access: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestRenderer {
    pub id: String,
    pub label: String,
    pub pipeline: String,
    pub selected: bool,
    pub default: bool,
    pub passes: Vec<String>,
    #[serde(default)]
    pub resources: Vec<ManifestRecipeResource>,
    #[serde(default)]
    pub steps: Vec<ManifestRecipeStep>,
    /// Checked semantic attachment versions at engine integration boundaries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_ports: Vec<ManifestResourcePort>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestResourcePort {
    pub point: String,
    pub name: String,
    pub member: String,
    pub resource: String,
    pub access: String,
    pub queue: Option<String>,
    /// Engine nodes that update this resource through writable storage bindings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub storage_writers: Vec<String>,
    pub incoming: Vec<String>,
    pub outgoing: Vec<String>,
    pub versions: Vec<ManifestResourceVersion>,
}

/// Versions describe recipe execution, including its per-range expansion. A
/// conditional writer that does not execute preserves its input version.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestResourceVersion {
    pub id: u32,
    pub previous: Option<u32>,
    /// Nodes making this version available: completion gates for the initial
    /// version, then ordered operations or one runtime-sorted queue group.
    pub producers: Vec<String>,
    pub readers: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestRecipeResource {
    pub name: String,
    pub kind: String,
    pub format: Option<String>,
    pub bytes: Option<u32>,
    pub source: Option<String>,
    pub table: Option<String>,
    pub column: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestRecipeStep {
    /// Draws in one queue share attachment state and sort across recipe nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparent_queue: Option<String>,
    /// A draw instantiated for each occurrence of this material's selected range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<ManifestStyleInvocation>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub attachments: std::collections::BTreeMap<String, ManifestAttachmentOps>,
    pub name: String,
    pub pass: String,
    pub domain: String,
    pub vertex: Option<String>,
    pub entry: String,
    pub bindings: std::collections::BTreeMap<String, String>,
    #[serde(with = "technique::color_slots")]
    #[cfg_attr(
        feature = "typescript",
        ts(as = "std::collections::BTreeMap<u32, String>")
    )]
    pub colors: std::collections::BTreeMap<u32, String>,
    pub depth: Option<String>,
    pub dispatch_scale: [u32; 2],
    #[serde(default)]
    pub capacity: std::collections::BTreeMap<String, u32>,
    pub vertex_count: u32,
    pub after: Vec<String>,
    pub condition: Option<String>,
}

impl ManifestRecipeStep {
    /// Both domains bind resources and execute independently for each draw range.
    /// Instance draws generate vertices procedurally rather than reading the mesh.
    pub fn is_draw_scoped(&self) -> bool {
        matches!(self.domain.as_str(), "mesh" | "instance")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestStyleInvocation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<ManifestDrawBounds>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_geometry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<ManifestDrawHost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_vertices: Option<ManifestGeneratedVertices>,
    /// Per-invocation owned compute outputs, keyed by the draw's shader binding.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub compute_inputs: std::collections::BTreeMap<String, ManifestDrawComputeBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<String>,
    pub point: String,
    pub material: String,
    pub operation: String,
    pub ordinal: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestDrawBounds {
    pub geometry: String,
    pub expansion: ManifestComputeExpression,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestGeneratedVertices {
    pub binding: String,
    pub base_vertex: ManifestComputeExpression,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestDrawHost {
    pub arguments: std::collections::BTreeMap<String, ManifestComputeArgument>,
    pub geometry: std::collections::BTreeMap<String, ManifestDrawGeometry>,
    pub requirements: Vec<ManifestComputeExpression>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestDrawGeometry {
    pub producer: String,
    pub vertex_count_member: String,
    pub index_count_member: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestDrawComputeBinding {
    pub producer: String,
    pub ty: String,
    /// None binds the resource; Some binds its CPU-known logical dimension.
    pub dimension: Option<ManifestComputeDimension>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestTable {
    pub name: String,
    pub index_field: String,
    pub first_index: u32,
    pub fields: Vec<String>,
    pub records: Vec<ManifestTableRecord>,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestTableRecord {
    pub key: String,
    pub index: u32,
    pub values: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestVertexFactory {
    pub name: String,
    pub vertex_format: String,
    #[serde(default)]
    pub bindings: Vec<ManifestVertexFactoryBinding>,
    #[serde(default)]
    pub attributes: Vec<ManifestVertexAttribute>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub satisfies_interfaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub array_stride: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestVertexFactoryBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<types::SamplerPreset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw_data: Option<ManifestDrawData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<ManifestGeometryBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub name: String,
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<u32>,
    pub signature: Option<String>,
}

/// Runtime-owned data scoped to one draw invocation in one rendered view.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ManifestDrawData {
    /// Nonzero index identifying this object/material range in the current view.
    InstanceId,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestVertexAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub shader_location: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_format: Option<String>,
    pub required: bool,
    pub defaulted: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestConfigAxis {
    pub pipeline: String,
    pub pass: String,
    pub axis: String,
    pub known_mode: String,
    pub axis_class: String,
    pub inclusion: String,
    pub included_in_build: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPipeline {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(rename = "type")]
    pub pipeline_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_summary: Option<ManifestPipelineSemanticSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pass_semantics: Vec<ManifestPipelinePassSemantics>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPipelineSemanticSummary {
    pub raster_passes: usize,
    pub compute_passes: usize,
    pub additive_passes: usize,
    pub resource_flow_edges: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_layout_signature_uses: Vec<ManifestSharedLayoutSignatureUse>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPipelinePassSemantics {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
#[cfg_attr(feature = "typescript", ts(concrete(D = serde_json::Value, N = f64), bound = "N: ts_rs::TS"))]
#[serde(bound(deserialize = "D: Deserialize<'de>, N: Deserialize<'de>"))]
pub struct ManifestSurface<D = serde_json::Value, N = f64> {
    #[serde(default)]
    pub settings: Option<ManifestSurfaceSettings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_uniforms: Vec<ManifestGlobalUniform>,
    pub name: String,
    /// `"pbr"` or `"unlit"`.
    pub material_ty: String,
    #[serde(default)]
    pub surface_requirements: ManifestSurfaceRequirements,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ManifestSurfaceParam<D, N>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<ManifestTexture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<ManifestSampler>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_properties: Option<String>,
    pub surface_shader: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_shader_entry: Option<String>,
    pub render_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_evaluator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_shader_entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_shader_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_contract: Option<ManifestSurfaceEvaluationContract>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluation_variants: Vec<ManifestSurfaceEvaluationVariant>,
    pub contract_requirements: Option<ManifestSurfaceContractRequirements>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_channels: Vec<ManifestSurfaceCustomChannel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mesh_passes: Vec<ManifestMeshPass>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestStageOutput {
    pub location: u32,
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestRasterEntry {
    pub function: String,
    pub entry: String,
    pub stage: String,
    pub outputs: Vec<ManifestStageOutput>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestMeshVariant {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ManifestRasterEntry>,
    pub factory: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestMeshPass {
    /// A procedural pass runs in the instance domain without the range's vertex stream.
    #[serde(default)]
    pub procedural: bool,
    /// Per-draw owned outputs captured by base shading, distinct from material settings.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub shading_inputs: std::collections::BTreeMap<String, ManifestDrawComputeBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<ManifestMeshPreparation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared_source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<ManifestVertexFactoryBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ManifestRasterEntry>,
    #[serde(default)]
    pub variants: Vec<ManifestMeshVariant>,
    pub pass: String,
    pub factory: String,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestRasterState {
    pub cull: Option<String>,
    pub blend: Option<String>,
    pub depth_write: Option<bool>,
    pub depth_compare: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceSettings {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<ManifestImplementationSelection>,
    #[serde(default)]
    pub recipe_conditions: std::collections::BTreeMap<String, bool>,
    #[serde(default)]
    pub pass_states: std::collections::BTreeMap<String, ManifestRasterState>,
    #[serde(default)]
    pub evaluation_axes: std::collections::BTreeMap<String, String>,
    pub properties: Vec<ManifestEntryProperty>,
    pub usages: Vec<String>,
}
/// A symbolic implementation selection. IDs are local to this artifact.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestImplementationSelection {
    /// First 16-byte settings lane in the bundle-wide style parameter buffer.
    #[serde(default)]
    pub settings_offset: u32,
    /// Declaration order determines each parameter's relative 16-byte lane.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ManifestParam>,
    /// Compile-time settings; changing these requires a new artifact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub static_parameters: Vec<ManifestParam>,
    pub name: String,
    pub contract: String,
    pub symbol: String,
    pub id: u32,
    /// Declared symbols, including unsupported choices. Never a compatibility filter.
    pub available: Vec<String>,
    /// Compatibility of each declared symbol in this material/renderer context.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub availability: Vec<ManifestImplementationAvailability>,
    pub editable: bool,
}
/// A candidate is checked with selected settings when selected, otherwise its
/// defaults. This describes graph compatibility, not device allocation limits.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestImplementationAvailability {
    pub symbol: String,
    pub renderer: Option<String>,
    pub schema: String,
    pub factories: Vec<String>,
    pub provider: bool,
    pub capabilities: Vec<ManifestCapabilityAvailability>,
    pub integration_points: Vec<ManifestCapabilityAvailability>,
    pub static_parameters: Vec<ManifestParam>,
    pub supported: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestCapabilityAvailability {
    pub name: String,
    pub provided: bool,
    pub supported: bool,
    pub reasons: Vec<String>,
}

fn default_editable() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestEntryProperty {
    #[serde(default = "default_editable")]
    pub editable: bool,
    #[serde(default)]
    pub block: Option<String>,
    #[serde(default)]
    pub block_present: bool,
    pub entry: String,
    pub name: String,
    pub ty: String,
    pub value: f32,
    pub choices: Vec<(String, f32)>,
    pub permutation: bool,
    pub value_span: Option<ManifestSourceRange>,
    pub insert_at: usize,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSourceRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceEvaluationContract {
    pub inputs: Vec<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub runtime: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceEvaluationVariant {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_type: Option<String>,
    pub entry: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<ManifestSurfaceEvaluationVariantBinding>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceEvaluationVariantBinding {
    pub axis: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceContractRequirements {
    pub required_textures: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceCustomChannel {
    pub name: String,
    pub slot: u32,
    pub field: String,
    /// Scalar/vector lane count; absent for structured values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<u8>,
    /// Authored logical type, independent of any storage encoding.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceRequirements {
    #[serde(default)]
    pub context_type: Option<String>,
    #[serde(default)]
    pub context_fields: Vec<ManifestSurfaceContextField>,
    #[serde(default)]
    pub uv_channels: Vec<ManifestSurfaceUvChannelRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertex_stage: Option<ManifestSurfaceVertexStage>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceContextField {
    pub name: String,
    pub ty: String,
    pub semantic: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceVertexStage {
    pub entry_point: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSurfaceUvChannelRequirement {
    pub selector: String,
    pub stream_index: u32,
    pub semantic: String,
    pub components: u32,
    pub required: bool,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
#[cfg_attr(feature = "typescript", ts(concrete(D = serde_json::Value, N = f64), bound = "N: ts_rs::TS"))]
// Producers retain typed f32 defaults/ranges; consumers use JSON and f64.
// Both representations share this wire schema and one browser declaration.
pub struct ManifestSurfaceParam<D = serde_json::Value, N = f64> {
    pub group: u32,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// Uniform binding index at `@group(0)`.
    pub binding: u32,
    #[cfg_attr(feature = "typescript", ts(type = "unknown"))]
    pub default: D,
    pub min: Option<N>,
    pub max: Option<N>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
#[cfg_attr(feature = "typescript", ts(concrete(D = serde_json::Value, N = f64), bound = "N: ts_rs::TS"))]
#[serde(bound(deserialize = "D: Deserialize<'de>, N: Deserialize<'de>"))]
pub struct ManifestCanvas<D = serde_json::Value, N = f64> {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_uniforms: Vec<ManifestGlobalUniform>,
    pub name: String,
    #[serde(default)]
    pub params: Vec<ManifestParam<D, N>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub storage_params: Vec<ManifestStorageParam>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<ManifestTexture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<ManifestSampler>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_buffers: Vec<ManifestPathBuffer>,
    pub pass_plan: ManifestPassPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_pass: Option<ManifestEnginePass>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestEnginePass {
    pub pipeline: String,
    pub pass: String,
    pub interface: String,
    pub vertex_entry: String,
    pub fragment_entry: String,
    pub vertex_count: u32,
    pub instance_uniform_group: u32,
    pub instance_uniform_binding: u32,
    pub variants: Vec<ManifestEnginePassVariant>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestEnginePassVariant {
    pub key: String,
    pub bindings: Vec<ManifestEnginePassVariantBinding>,
    pub vertex_entry: String,
    pub fragment_entry: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestEnginePassVariantBinding {
    pub axis: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestGlobalUniform {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub group: u32,
    pub binding: u32,
    pub byte_size: u32,
    pub fields: Vec<ManifestGlobalUniformField>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestGlobalUniformField {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub offset: u32,
    pub components: u32,
    pub scalar_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestParamTypeInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
#[cfg_attr(feature = "typescript", ts(concrete(D = serde_json::Value, N = f64), bound = "N: ts_rs::TS"))]
// Producers retain typed f32 defaults/ranges; consumers use JSON and f64.
// Both representations share this wire schema and one browser declaration.
pub struct ManifestParam<D = serde_json::Value, N = f64> {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub param_type: Option<ManifestParamTypeInfo>,
    #[cfg_attr(feature = "typescript", ts(type = "unknown"))]
    pub default: D,
    pub min: Option<N>,
    pub max: Option<N>,
}

/// Typed defaults for artifact producers. Readers use the parameter's declared
/// type to interpret its JSON `default` value; an untagged number alone cannot
/// distinguish an f32, i32, and u32.
///
/// Keep scalar and color lanes as f32 until serialization so emitting a default
/// does not widen its decimal representation through `serde_json::Value`.
#[derive(Debug, Serialize, Clone)]
#[serde(untagged)]
pub enum ManifestParamDefault {
    Scalar(f32),
    Array(ManifestArrayDefault),
    Int(i32),
    UInt(u32),
    Bool(bool),
    Color([f32; 4]),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ManifestArrayDefault {
    #[serde(rename = "type")]
    pub elem_type: String,
    pub values: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestStorageParam {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub param_type: Option<ManifestParamTypeInfo>,
    pub group: u32,
    pub binding: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestTexture {
    pub name: String,
    pub group: u32,
    pub binding: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ManifestTextureMetadata>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestTextureMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<ManifestTextureChannel>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestTextureChannel {
    pub channel: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSampler {
    pub group: u32,
    pub binding: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPathBuffer {
    pub name: String,
    pub group: u32,
    pub binding: u32,
    pub segments: usize,
    /// Absent in older artifacts, which cannot initialize a standalone GPU host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ManifestPathData>,
}

pub const PATH_SEGMENT_LAYOUT: &str = "fresco-path-segment-v1";
pub const PATH_SEGMENT_STRIDE: u32 = 56;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPathData {
    pub layout: String,
    pub stride: u32,
    pub rows: Vec<ManifestPathSegment>,
}

/// V1 GPU record: p0/p1/p2/p3 at bytes 0/8/16/24, s0/len at 32/36,
/// kind (u32) at 40, mid_u at 44, followed by zero padding through byte 55.
/// Coordinates and lengths are f32; kind 0 is quadratic (also used for lines),
/// kind 1 is cubic. Geometry is already sampled according to compiler policy.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPathSegment {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub p2: [f32; 2],
    pub p3: [f32; 2],
    pub s0: f32,
    pub len: f32,
    pub kind: u32,
    pub mid_u: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPassPlan {
    #[serde(default)]
    pub passes: Vec<ManifestPass>,
    #[serde(default)]
    pub edges: Vec<ManifestEdge>,
    #[serde(default)]
    pub targets: Vec<ManifestIntermediateTarget>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPass {
    pub id: usize,
    pub stage: usize,
    pub locality: String,
    pub start_layer: usize,
    pub end_layer: usize,
    pub count: usize,
    pub kernel_strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_radius_px: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_levels: Option<u32>,
    pub entry_point: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ManifestPassInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_target: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestPassInput {
    pub from_pass: usize,
    pub target_id: usize,
    pub binding: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestIntermediateTarget {
    pub id: usize,
    pub format: String,
    pub scale: f32,
    pub lifetime: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestEdge {
    pub from: usize,
    pub to: usize,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export_to = "../web/src/generated/wasm-contracts/")
)]
pub struct ManifestSharedLayoutSignatureUse {
    pub signature_id: String,
    pub producer_pass: String,
    pub consumer_pass: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestBufferField {
    pub name: String,
    pub offset: u32,
    pub ty: String,
}

/// An object-owned buffer used by geometry preparation or its consumers.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestGeometryBinding {
    pub producer: String,
    /// raw_vertices, raw_indices, counts, bounds, vertices, or indices.
    pub role: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ManifestMeshPreparation {
    pub node: String,
    pub resource_type: String,
    pub entry: String,
    pub vertex_stride: u32,
    pub attributes: Vec<ManifestVertexAttribute>,
    pub workgroup_size: u32,
}
