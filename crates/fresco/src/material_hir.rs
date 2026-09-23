//! HIR for `surface` entry points — the rung-8 material layer stack.

use crate::ast::{MaterialReturnTy, SchemaEvaluatorContractDecl, SchemaEvaluatorPermutationDecl};
use crate::hir::{Param, Sx, TextureMetadata, TextureTypeDef, UserFnHelper};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MaterialChannel {
    pub name: String,
    pub ty_name: String,
}

/// Structured signal payload; the channel declaration supplies its concrete GPU type.
#[derive(Debug, Clone)]
pub enum MaterialValue {
    Scalar(Sx),
    Vector(Vec<Sx>),
    Matrix(Vec<Vec<Sx>>),
    Array(Vec<MaterialValue>),
    Record(HashMap<String, MaterialValue>),
}

/// A single material layer in the surface compose stack.
#[derive(Debug, Clone)]
pub struct MaterialLayer {
    /// Distinguishes the engine initializer from subsequent layer operations.
    pub is_base: bool,
    /// Optional engine composition weight.
    pub weight: Option<Sx>,
    /// Values indexed by the fields of the selected engine schema.
    pub channels: HashMap<String, MaterialValue>,
}

/// Optional user-authored vertex-stage overrides for a surface.
#[derive(Debug, Clone, Default)]
pub struct MaterialVertexProgram {
    /// Typed context fields updated by this stage, in engine declaration order.
    pub fields: Vec<(String, Vec<Sx>)>,
}

/// High-level IR for a `surface` entry point.
#[derive(Debug, Clone)]
pub struct MaterialHir {
    pub material_schemas: HashMap<String, Option<String>>,
    pub context: crate::context::EntryContext,
    pub settings: Option<crate::ast::SurfaceSettings>,
    pub(crate) rendering_policy: Option<crate::hir::RenderingPolicy>,
    /// Surface entry name (used to derive the lowered function name `fresco_<name>`).
    pub name: String,
    /// Declarative material properties binding selected by the surface entry.
    pub material_properties_name: Option<String>,
    /// Surface shader identifier for the response function bound to this surface.
    pub surface_shader_name: String,
    /// Render policy identifier selected by the surface entry.
    pub render_policy_name: String,
    /// Resolved engine material profile reference.
    pub material_ty: MaterialReturnTy,
    /// User-exposed `param` declarations.
    pub params: Vec<Param>,
    /// User helper functions referenced from this surface.
    pub user_helpers: HashMap<String, UserFnHelper>,
    /// Texture names referenced (for bind-manifest generation).
    pub textures: Vec<String>,
    pub texture_index: HashMap<String, usize>,
    /// Optional metadata for named textures, keyed by texture name.
    pub texture_metadata: HashMap<String, TextureMetadata>,
    /// All `texture_type` definitions declared in this program, keyed by type name.
    pub texture_type_defs: HashMap<String, TextureTypeDef>,
    /// Layer stack — first entry should always be a base layer.
    pub layers: Vec<MaterialLayer>,
    /// Declared material channels for the selected schema in deterministic order.
    pub material_channels: Vec<MaterialChannel>,
    /// Declared per-channel defaults used when a surface doesn't author a value.
    pub channel_defaults: HashMap<String, MaterialValue>,
    pub record_types: Vec<crate::ast::StructDecl>,
    /// Optional linked evaluation model name.
    pub schema_evaluator_name: Option<String>,
    /// Optional selected program model name.
    pub schema_program_name: Option<String>,
    /// Optional authored evaluation contract metadata.
    pub evaluation_contract: Option<SchemaEvaluatorContractDecl>,
    /// Optional authored evaluation permutation metadata.
    pub evaluation_permutations: Vec<SchemaEvaluatorPermutationDecl>,
    /// Optional authored specialization pattern.
    pub evaluation_specialize_pattern: Option<String>,
    /// Instantiated ordinary GPU functions and their exact result/type identity.
    pub evaluation_typed_source: Option<String>,
    pub evaluation_type_aliases: HashMap<String, String>,
    /// Manifest/runtime-visible WGSL entrypoint for authored evaluation.
    pub evaluation_shader_entry: Option<String>,
    /// Optional authored WGSL source for the evaluation model itself.
    pub evaluation_shader_source: Option<String>,
    /// Optional authored evaluation variant declarations for branchy evaluation strategies.
    pub evaluation_variant_decls: Vec<crate::ast::SchemaEvaluatorVariantDecl>,
    /// Generated evaluation-variant bodies for specialized lowering.
    pub evaluation_variant_bodies: Vec<EvaluationVariant>,
    /// Optional user-authored vertex-stage logic.
    pub vertex_program: Option<MaterialVertexProgram>,
    /// Human-readable notes for `--explain` output.
    pub notes: Vec<String>,
    /// Struct-typed global `param` declarations visible to this surface,
    /// carried for lowering (real `var<uniform>` buffer creation, shared
    /// with canvas `Hir`s in the same compile) and manifest emission.
    pub global_uniforms: Vec<crate::check::GlobalUniformDef>,
}

#[derive(Debug, Clone)]
pub struct EvaluationVariantBinding {
    pub axis: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct EvaluationVariant {
    pub result_type: Option<String>,
    pub entry: String,
    pub bindings: Vec<EvaluationVariantBinding>,
    pub(crate) function: Option<crate::driver::schema_function::SchemaFunction>,
}

impl MaterialHir {
    pub fn texture_default_asset(&self, name: &str) -> Option<&str> {
        self.texture_metadata
            .get(name)
            .and_then(|meta| meta.default_asset.as_deref())
    }

    pub fn evaluation_variants(&self) -> Vec<EvaluationVariant> {
        self.evaluation_variant_bodies.clone()
    }
}

pub(crate) fn sanitize_wgsl_function_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        out.push_str("fresco_evaluation_variant");
    }

    if out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out.insert(0, '_');
    }

    if out.chars().last().is_some_and(|ch| ch.is_ascii_digit()) {
        out.push('_');
    }

    out
}
