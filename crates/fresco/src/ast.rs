//! Untyped AST, straight out of the parser. Every node carries a byte span
//! so the checker can produce precise ariadne diagnostics.

pub use crate::lexer::Unit;

pub type Span = std::ops::Range<usize>;

#[derive(Debug, Clone)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

pub type SExpr = Spanned<Expr>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    LogicalAnd,
    LogicalOr,
    Add,
    Sub, // numeric subtract OR shape subtract; the checker disambiguates by type
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    BitXor,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    Union,     // `|` on shapes
    Intersect, // `&` on shapes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Option<String>,
    pub value: SExpr,
}

#[derive(Debug, Clone)]
pub struct PathCommand {
    pub kind: PathCommandKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum PathCommandKind {
    Move {
        to: SExpr,
    },
    Line {
        to: SExpr,
    },
    Cubic {
        c1: SExpr,
        c2: SExpr,
        to: SExpr,
    },
    Arc {
        center: SExpr,
        radius: SExpr,
        sweep: SExpr,
    },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub enum_name: Option<String>,
    pub enum_name_span: Option<Span>,
    pub variant_name: String,
    pub variant_span: Span,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstTemplateKind {
    U32,
    I32,
}

#[derive(Debug, Clone)]
pub struct ConstTemplateArg {
    pub value: SExpr,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub enum Expr {
    /// Numeric literal with unit suffix (`0.5`, `12px`, `20deg`, `2s`, `100ms`).
    Num(f64, Unit),
    /// `#rrggbbaa` etc., unpacked to linear-ish [0,1] RGBA.
    Color([f32; 4]),
    /// String literal.
    Str(String),
    Var(String),
    /// `(a, b)` — a vec2 literal.
    Vec2(Box<SExpr>, Box<SExpr>),
    /// `(a, b, c)` — a vec3 literal.
    Vec3(Box<SExpr>, Box<SExpr>, Box<SExpr>),
    /// `(a, b, c, d)` — a vec4 literal.
    Vec4(Box<SExpr>, Box<SExpr>, Box<SExpr>, Box<SExpr>),
    /// `[a, b, ...]` — array literal.
    Array(Vec<SExpr>),
    /// `[for i in items => expr]` — compile-time array comprehension.
    ArrayComp {
        name: String,
        iterable: Box<SExpr>,
        body: Box<SExpr>,
    },
    /// `|a, b| expr` — inline lambda expression.
    Lambda {
        params: Vec<Spanned<String>>,
        body: LambdaBody,
    },
    Unary(UnOp, Box<SExpr>),
    Binary(BinOp, Box<SExpr>, Box<SExpr>),
    /// `a .. b` — scalar range literal used by signal builtins.
    Range(Box<SExpr>, Box<SExpr>),
    /// `expr.field` — postfix member/swizzle access.
    Member(Box<SExpr>, String),
    /// `field <expr>` — explicit scalar field expression.
    Field(Box<SExpr>),
    /// `field <expr> at <coord-expr>` — re-sample a field at a different coordinate.
    FieldAt {
        inner: Box<SExpr>,
        coord: Box<SExpr>,
    },
    /// `layer <expr>` — explicit layer expression.
    Layer(Box<SExpr>),
    /// `path { ... }` — path DSL block parsed as structured commands.
    PathFuture {
        commands: Vec<PathCommand>,
    },
    /// `name(args...)`
    Call {
        name: String,
        name_span: Span,
        const_args: Vec<ConstTemplateArg>,
        args: Vec<Arg>,
    },
    /// `recv |> name(args...)`
    Pipe {
        recv: Box<SExpr>,
        name: String,
        name_span: Span,
        args: Vec<Arg>,
    },
    /// `<layer-expr> through space <chain>` — evaluate a layer at the S-mapped coordinate (§23.6).
    Through {
        layer: Box<SExpr>,
        chain: Vec<SpaceItem>,
    },
    /// `array[index]` — element access on a compile-time array value.
    Index {
        array: Box<SExpr>,
        index: Box<SExpr>,
    },
}

#[derive(Debug, Clone)]
pub enum LambdaBody {
    Expr(Box<SExpr>),
    BlockReturn(Box<SExpr>),
}

#[derive(Debug, Clone)]
pub struct SpaceCall {
    pub name: String,
    pub name_span: Span,
    pub args: Vec<Arg>,
}

#[derive(Debug, Clone)]
pub enum SpaceItem {
    Call(SpaceCall),
    Ref { name: String, name_span: Span },
}

#[derive(Debug, Clone)]
pub enum ComposeEntry {
    Expr {
        expr: SExpr,
        blend: Option<Spanned<String>>,
    },
    Block {
        body: Vec<Stmt>,
        blend: Option<Spanned<String>>,
        span: Span,
    },
    If {
        cond: SExpr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        blend: Option<Spanned<String>>,
        span: Span,
    },
    InSpace {
        chain: Vec<SpaceItem>,
        body: Vec<Stmt>,
        blend: Option<Spanned<String>>,
        span: Span,
    },
    For {
        name: String,
        iterable: SExpr,
        body: Vec<Stmt>,
        blend: Option<Spanned<String>>,
        /// Optional index variable bound to the iteration counter (from `each (v, i) in ...`).
        index_name: Option<(String, Span)>,
        span: Span,
    },
}

#[allow(
    dead_code,
    reason = "AST v0 includes staged fields consumed by planned passes"
)]
#[derive(Debug, Clone)]
pub struct ScatterLifecycle {
    pub instance_name: Spanned<String>,
    pub lifetime: SExpr,
    pub respawn_every: SExpr,
}

#[allow(
    dead_code,
    reason = "AST v0 includes staged fields consumed by planned passes"
)]
#[derive(Debug, Clone)]
pub struct ScatterDecl {
    pub count: SExpr,
    pub region: SExpr,
    pub seed: SExpr,
    pub strategy: Spanned<String>,
    pub lifecycle: Option<ScatterLifecycle>,
    pub body: Vec<Stmt>,
    pub post_pipes: Vec<ScatterPipeCall>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "AST v0 includes staged fields consumed by planned passes"
)]
#[derive(Debug, Clone)]
pub struct ScatterPipeCall {
    pub name: String,
    pub name_span: Span,
    pub args: Vec<Arg>,
}

#[derive(Debug, Clone)]
pub struct StyleStage {
    pub name: String,
    pub name_span: Span,
    pub args: Vec<Arg>,
}

#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "style parameter metadata is preserved for diagnostics and future type checks"
)]
pub struct StyleParam {
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub default: SExpr,
}

#[cfg_attr(
    target_pointer_width = "64",
    expect(
        clippy::large_enum_variant,
        reason = "Keep parsed syntax nodes inline; changing their representation is a separate AST design change."
    )
)]
#[allow(
    dead_code,
    reason = "AST statement variants are staged for future language features"
)]
#[derive(Debug, Clone)]
pub enum Stmt {
    TextureBinding {
        name: String,
        name_span: Span,
        ty_name: String,
        default_asset: Option<String>,
        span: Span,
    },
    Param {
        name: String,
        name_span: Span,
        ty_name: String,
        default: SExpr,
        range: Option<(SExpr, SExpr)>,
        span: Span,
    },
    Let {
        /// `var` (or a legacy type-first local) permits assignment; `let` does not.
        mutable: bool,
        name: String,
        name_span: Span,
        declared_ty_name: Option<String>,
        declared_ty_span: Option<Span>,
        value: SExpr,
    },
    Const {
        name: String,
        name_span: Span,
        ty_name: String,
        ty_span: Span,
        value: SExpr,
    },
    Store {
        target: SExpr,
        value: SExpr,
        span: Span,
    },
    Assign {
        name: String,
        name_span: Span,
        field_path: Option<String>,
        value: SExpr,
        span: Span,
    },
    For {
        name: String,
        name_span: Span,
        iterable: SExpr,
        body: Vec<Stmt>,
        /// Optional index variable bound to the iteration counter (from `each (v, i) in ...`).
        index_name: Option<(String, Span)>,
        span: Span,
    },
    If {
        cond: SExpr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        span: Span,
    },
    Match {
        value: SExpr,
        arms: Vec<MatchArm>,
        default_body: Option<Vec<Stmt>>,
        default_span: Option<Span>,
        span: Span,
    },
    LetScatter {
        name: String,
        name_span: Span,
        scatter: ScatterDecl,
    },
    SpaceDecl {
        name: String,
        name_span: Span,
        chain: Vec<SpaceItem>,
        span: Span,
    },
    StyleDecl {
        name: String,
        name_span: Span,
        params: Vec<StyleParam>,
        stages: Vec<StyleStage>,
        span: Span,
    },
    /// `canvas_space = a(...) . b(...)` — sets a default space conversion
    /// for the whole canvas.
    CanvasSpace {
        chain: Vec<SpaceItem>,
        span: Span,
    },
    /// `in space a(...) . b(...) { ... }` — the block's final value, re-hosted
    /// in the transformed space.
    InSpace {
        chain: Vec<SpaceItem>,
        body: Vec<Stmt>,
        span: Span,
    },
    /// Establish an explicitly scoped semantic context.
    InContext {
        value: SExpr,
        body: Vec<Stmt>,
        span: Span,
    },
    /// `{ ... }` — grouped statements as a single statement value.
    Block {
        body: Vec<Stmt>,
        span: Span,
    },
    /// Parser-desugared statement sequence that executes in the current scope.
    Seq {
        body: Vec<Stmt>,
        span: Span,
    },
    /// `compose { entry (blend: mode)? ... }` — the painter's stack.
    Compose {
        entries: Vec<ComposeEntry>,
        span: Span,
    },
    /// `compose { ... } |> effect(...)` — compose result with follow-up layer effects.
    ComposePiped {
        entries: Vec<ComposeEntry>,
        pipes: Vec<ScatterPipeCall>,
        span: Span,
    },
    /// `vertex { position: ..., normal: ... }` — optional surface vertex-stage overrides.
    SurfaceVertex {
        fields: Vec<(Spanned<String>, SExpr)>,
        span: Span,
    },
    ReturnVoid {
        span: Span,
    },
    Return {
        value: SExpr,
        span: Span,
    },
    Break {
        span: Span,
    },
    /// `fn name(params) -> ty { body }` declared inside a canvas or fn body.
    LocalFnDecl(FnDecl),
    Expr(SExpr),
}

#[allow(
    dead_code,
    reason = "Canvas metadata fields are staged for ABI evolution"
)]
#[derive(Debug, Clone)]
pub struct CanvasParam {
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
}

/// The semantic class of a normalized root entry.
#[allow(
    dead_code,
    reason = "Root-entry metadata is staged for explicit compiler IR"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootEntryKind {
    Canvas,
    Surface,
}

/// The semantic role of a root-entry parameter after normalization.
#[allow(
    dead_code,
    reason = "Root-entry metadata is staged for explicit compiler IR"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootEntryParamRole {
    Primary,
    Signal,
    Delta,
    Resolution,
    Other,
}

/// One normalized root-entry parameter with an explicit semantic role.
#[allow(
    dead_code,
    reason = "Root-entry metadata is staged for explicit compiler IR"
)]
#[derive(Debug, Clone)]
pub struct NormalizedRootEntryParam {
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub role: RootEntryParamRole,
}

/// A first-class normalized root-entry node.
#[allow(
    dead_code,
    reason = "Root-entry metadata is staged for explicit compiler IR"
)]
#[derive(Debug, Clone)]
pub struct NormalizedRootEntry {
    pub entry_kind: String,
    pub kind: RootEntryKind,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<NormalizedRootEntryParam>,
    pub material_ty: Option<MaterialReturnTy>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Classifies a root-entry parameter's role from its declared type alone, so
/// role is a function of what the parameter *is*, never of its position.
/// Validation elsewhere in the checker treats this as the source of truth
/// for "is this the primary/signal/delta/resolution parameter?" and only
/// falls back to `ty_name` to distinguish `coord` from `surf` for messages.
fn normalized_root_entry_param_role(ty_name: &str) -> RootEntryParamRole {
    match ty_name.trim() {
        "coord" | "surf" => RootEntryParamRole::Primary,
        "signal" => RootEntryParamRole::Signal,
        "delta" => RootEntryParamRole::Delta,
        "resolution" => RootEntryParamRole::Resolution,
        _ => RootEntryParamRole::Other,
    }
}

fn normalized_root_entry_params(params: &[CanvasParam]) -> Vec<NormalizedRootEntryParam> {
    params
        .iter()
        .map(|param| NormalizedRootEntryParam {
            name: param.name.clone(),
            name_span: param.name_span.clone(),
            ty_name: param.ty_name.clone(),
            ty_span: param.ty_span.clone(),
            role: normalized_root_entry_param_role(&param.ty_name),
        })
        .collect()
}

/// A shared view over entry-point declarations that execute as a single body.
#[allow(
    dead_code,
    reason = "EntryTemplate remains as staged syntax-only glue for parser-side entry bodies"
)]
///
/// This is the first step of a broader restructuring that will let canvases,
/// surfaces, and eventually other entry-style declarations share the same
/// template contract while preserving their existing syntax and lowering hooks.
#[derive(Debug, Clone)]
pub struct EntryTemplate {
    pub name: String,
    pub name_span: Span,
    pub params: Vec<CanvasParam>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Template metadata is staged for engine-facing lowering and instantiation"
)]
#[derive(Debug, Clone)]
pub struct TemplatePlugDecl {
    /// Engine-visible callable alias for a bound entry method.
    pub binding_name: Option<Spanned<String>>,
    /// Ordered module calls thread this first parameter through each result.
    pub compose_param: Option<Spanned<String>>,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<FnParam>,
    pub return_ty: Option<Spanned<String>>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Template metadata is staged for engine-facing lowering and instantiation"
)]
#[derive(Debug, Clone)]
pub struct TemplateConfigParamDecl {
    pub attrs: Vec<(String, Vec<String>)>,
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub default: Option<SExpr>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Template declarations are staged for engine-facing lowering and instantiation"
)]
#[derive(Debug, Clone)]
pub struct TemplateDecl {
    pub surface_defaults: Vec<(Spanned<String>, SExpr)>,
    pub surface_defaults_block: Option<String>,
    pub property_block: Option<String>,
    pub property_targets: Vec<String>,
    pub entry: Option<(String, String)>,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<CanvasParam>,
    pub plugs: Vec<TemplatePlugDecl>,
    pub config_params: Vec<TemplateConfigParamDecl>,
    pub span: Span,
}

impl TemplateDecl {}

/// Engine-registered declaration, resolved after its imports are loaded.
#[derive(Debug, Clone)]
pub struct AuthoredEntry {
    /// Compile-time settings checked against the engine interface schema.
    pub config: Vec<(Spanned<String>, SExpr)>,
    pub kind: String,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<CanvasParam>,
    pub return_ty: Option<Spanned<String>>,
    pub body: Vec<Stmt>,
    pub blocks: Vec<AuthoredEntryBlock>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AuthoredEntryBlock {
    pub params: Option<Vec<FnParam>>,
    pub return_ty: Option<Spanned<String>>,
    pub name: String,
    pub name_span: Span,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Canvas metadata fields are staged for ABI evolution"
)]
#[derive(Debug, Clone)]
pub struct Canvas {
    pub entry_kind: String,
    pub declared_return_ty: Option<Spanned<String>>,
    pub name: String,
    pub name_span: Span,
    /// Declared canvas parameters as `name: type` pairs — types are
    /// documentation in v0;
    /// the ABI is fixed: (uv: vec2f, time: f32, res: vec2f) -> vec4f.
    pub params: Vec<CanvasParam>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

impl NormalizedRootEntry {
    pub fn into_canvas(self) -> Canvas {
        Canvas {
            entry_kind: self.entry_kind,
            declared_return_ty: None,
            name: self.name,
            name_span: self.name_span,
            params: self
                .params
                .into_iter()
                .map(|param| CanvasParam {
                    name: param.name,
                    name_span: param.name_span,
                    ty_name: param.ty_name,
                    ty_span: param.ty_span,
                })
                .collect(),
            body: self.body,
            span: self.span,
        }
    }
}

impl Canvas {
    pub fn as_normalized_root_entry(&self) -> NormalizedRootEntry {
        NormalizedRootEntry {
            entry_kind: self.entry_kind.clone(),
            kind: RootEntryKind::Canvas,
            name: self.name.clone(),
            name_span: self.name_span.clone(),
            params: normalized_root_entry_params(&self.params),
            material_ty: None,
            body: self.body.clone(),
            span: self.span.clone(),
        }
    }
}

impl SurfaceDecl {
    pub fn as_normalized_root_entry(&self) -> NormalizedRootEntry {
        NormalizedRootEntry {
            entry_kind: "surface".into(),
            kind: RootEntryKind::Surface,
            name: self.name.clone(),
            name_span: self.name_span.clone(),
            params: normalized_root_entry_params(&self.params),
            material_ty: Some(self.material_ty.clone()),
            body: self.body.clone(),
            span: self.span.clone(),
        }
    }
}

/// The material variant declared in a `surface ... -> material(...)` return type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MaterialReturnTy {
    /// Use the engine-declared default material profile.
    #[default]
    Default,
    /// An engine-declared material profile.
    Named(String),
}

/// One typed channel declaration inside a `material_model` block.
#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub struct MaterialChannelDecl {
    pub compose: Option<String>,
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub default: Option<SExpr>,
    pub span: Span,
}

/// Top-level `material_properties Name [extends Parent] { ... }` declaration.
#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub struct MaterialPropertiesDecl {
    pub context_parameter: Option<String>,
    pub context: Option<String>,
    pub composition: Option<[String; 3]>,
    pub is_default: bool,
    pub evaluator: Option<String>,
    pub name: String,
    pub name_span: Span,
    pub extends_name: Option<String>,
    pub extends_span: Option<Span>,
    pub channels: Vec<MaterialChannelDecl>,
    pub span: Span,
}

/// Top-level `schema_expression Name for Properties { shade: <expr> }` declaration.
#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub struct SchemaExpressionDecl {
    pub name: String,
    pub name_span: Span,
    pub properties_name: String,
    pub properties_span: Span,
    pub shade: SExpr,
    pub span: Span,
}

/// Top-level `schema_program Name for MaterialProperties { ... }` declaration.
#[derive(Debug, Clone)]
pub struct SchemaProgramDecl {
    pub name: String,
    pub name_span: Span,
    pub material_model_name: String,
    pub material_model_span: Span,
    pub functions: Vec<FnDecl>,
    pub output: Vec<SExpr>,
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub struct EvaluationContractBindingDecl {
    pub name: String,
    pub name_span: Span,
    pub ty_name: Option<String>,
    pub ty_span: Option<Span>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub struct SchemaEvaluatorContractDecl {
    pub inputs: Vec<EvaluationContractBindingDecl>,
    pub runtime: Vec<EvaluationContractBindingDecl>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationPermutationAtom {
    Ident(String),
    Number(u32),
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationPermutationSpec {
    Single(EvaluationPermutationAtom),
    Range { start: u32, end: u32 },
    Set(Vec<EvaluationPermutationAtom>),
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub struct SchemaEvaluatorPermutationDecl {
    pub name: String,
    pub name_span: Span,
    pub spec: EvaluationPermutationSpec,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub struct SchemaEvaluatorVariantDecl {
    pub predicate: Option<SExpr>,
    pub name: String,
    pub name_span: Span,
    pub shade: SExpr,
    pub span: Span,
}

/// Top-level `schema_evaluator Name for MaterialProperties { ... }` declaration.
#[derive(Debug, Clone)]
pub struct SchemaEvaluatorDecl {
    pub name: String,
    pub name_span: Span,
    pub material_model_name: String,
    pub material_model_span: Span,
    pub contract: Option<SchemaEvaluatorContractDecl>,
    pub permutations: Vec<SchemaEvaluatorPermutationDecl>,
    pub specialize_pattern: Option<String>,
    pub specialize_pattern_span: Option<Span>,
    pub shade: SExpr,
    pub shader_source: Option<String>,
    pub variants: Vec<SchemaEvaluatorVariantDecl>,
}

/// A top-level `surface` entry point declaration.
///
/// `surface name(sp: surf, ...) -> material { ... }`
///
#[derive(Debug, Clone)]
pub struct SurfacePropertyBlock {
    pub name: String,
    pub values: Vec<(Spanned<String>, SExpr)>,
    pub span: Span,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SurfaceSettings {
    pub shading_samplers: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, fresco_artifact::types::SamplerPreset>,
    >,
    /// Owned shading captures indexed by contract, then authored slot name.
    pub shading_inputs: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, fresco_artifact::ManifestDrawComputeBinding>,
    >,
    pub implementation_slots: std::collections::BTreeSet<String>,
    pub implementations: Vec<fresco_artifact::ManifestImplementationSelection>,
    pub property_u32: std::collections::BTreeMap<String, u32>,
    pub recipe_conditions: std::collections::BTreeMap<String, bool>,
    pub pass_states: std::collections::BTreeMap<String, fresco_artifact::ManifestRasterState>,
    pub evaluation_axes: std::collections::BTreeMap<String, String>,
    pub properties: Vec<EntryProperty>,
    pub usages: Vec<String>,
}

/// Surface entries are a rung-8 planned feature (§25 of the design doc).
/// The parser accepts the declaration so aspirational north-star source files
/// can be loaded and receive a clear "not yet supported" diagnostic rather than
/// a raw parse error.
#[derive(Debug, Clone)]
pub struct SurfaceDecl {
    pub body_start: usize,
    pub property_blocks: Vec<SurfacePropertyBlock>,
    pub settings: Option<SurfaceSettings>,

    pub name: String,
    pub name_span: Span,
    /// Declared surface parameters (`sp: surf`, `time: signal`, etc.).
    pub params: Vec<CanvasParam>,
    /// The material variant declared after `->`.
    pub material_ty: MaterialReturnTy,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Function parameter metadata is preserved for diagnostics and tooling"
)]
#[derive(Debug, Clone)]
pub struct FnParam {
    pub is_context: bool,
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    /// True when this parameter appears after a `*` keyword-only separator.
    pub keyword_only: bool,
    pub default: Option<SExpr>,
}

#[allow(
    dead_code,
    reason = "Type parameter metadata is staged for generic function support"
)]
#[derive(Debug, Clone)]
pub struct TypeParam {
    pub name: String,
    pub name_span: Span,
    /// Interface names this type parameter must conform to.
    pub bounds: Vec<String>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Const template parameter metadata is staged for compile-time value specialization"
)]
#[derive(Debug, Clone)]
pub struct ConstTemplateParam {
    pub name: String,
    pub name_span: Span,
    pub kind: ConstTemplateKind,
    pub kind_span: Span,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Vertex interface declarations are staged for future lowering support"
)]
#[derive(Debug, Clone)]
pub struct VertexInterfaceDecl {
    pub name: String,
    pub name_span: Span,
    pub members: Vec<VertexInterfaceMemberDecl>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Vertex interface member declarations are staged for future lowering support"
)]
#[derive(Debug, Clone)]
pub struct VertexInterfaceMemberDecl {
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub optional: bool,
    pub default: Option<SExpr>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Vertex format declarations are staged for future lowering support"
)]
#[derive(Debug, Clone)]
pub struct VertexFormatDecl {
    pub name: String,
    pub name_span: Span,
    pub parent: Option<String>,
    pub parent_span: Option<Span>,
    pub members: Vec<VertexInterfaceMemberDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VertexFactoryDecl {
    pub name: String,
    pub name_span: Span,
    pub target_format: String,
    pub target_format_span: Span,
    pub bindings: Vec<PassBindingDecl>,
    pub hooks: Vec<PassFnHookDecl>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Function declaration metadata is staged for future lowering paths"
)]
#[derive(Debug, Clone)]
pub struct FnDecl {
    /// Compiler-assigned effect contract for dynamically dispatched shader hooks.
    pub derivative_free: bool,
    /// Original typed body, retaining conversions erased by legacy signal desugaring.
    pub typed_body: Option<Vec<Stmt>>,
    pub name: String,
    pub name_span: Span,
    pub docs: Option<String>,
    pub type_params: Vec<TypeParam>,
    pub const_params: Vec<ConstTemplateParam>,
    pub params: Vec<FnParam>,
    pub ret_ty: Option<(String, Span)>,
    pub is_internal: bool,
    pub is_builtin: bool,
    pub source_file: String,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Engine-authored shading and graph contract.
#[derive(Debug, Clone)]
pub struct StyleContractDecl {
    pub source_file: String,
    pub name: String,
    pub name_span: Span,
    pub schema: String,
    pub schema_span: Span,
    pub hooks: Vec<StyleHookDecl>,
    pub inputs: Vec<StyleInputDecl>,
    pub capabilities: Vec<StyleCapabilityUse>,
    pub points: Vec<StylePointDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StyleHookDecl {
    pub signature: InterfaceMethodDecl,
    pub default: Option<FnDecl>,
}

/// Typed engine data exposed by a contract or capability.
#[derive(Debug, Clone)]
pub struct StyleInputDecl {
    pub name: String,
    pub ty: String,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub struct StyleCapabilityUse {
    pub name: String,
    pub optional: bool,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub struct StyleGraphField {
    pub name: String,
    pub values: Vec<SExpr>,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub struct StylePointDecl {
    pub name: String,
    pub ty: String,
    pub optional: bool,
    pub fields: Vec<StyleGraphField>,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub struct StyleCapabilityDecl {
    pub source_file: String,
    pub name: String,
    pub members: Vec<StyleInputDecl>,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub struct StyleProviderBlock {
    pub name: String,
    pub fields: Vec<StyleGraphField>,
    pub span: Span,
}
#[derive(Debug, Clone)]
pub struct StyleProviderDecl {
    pub source_file: String,
    pub contract: String,
    pub renderer: String,
    pub inputs: Vec<(String, SExpr)>,
    pub blocks: Vec<StyleProviderBlock>,
    pub span: Span,
}

/// A schema-targeted implementation, preserved separately from its lowering.
#[derive(Debug, Clone)]
pub struct StyleDecl {
    pub source_file: String,
    pub name: String,
    pub name_span: Span,
    pub schema: String,
    pub schema_span: Span,
    pub contract: String,
    pub contract_span: Span,
    pub params: Vec<GlobalParamDecl>,
    pub static_params: Vec<GlobalParamDecl>,
    pub shading_inputs: Vec<StyleShadingInputDecl>,
    pub requirements: Vec<SExpr>,
    pub graph: Vec<Spanned<StyleGraphNode>>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

/// Explicit resource captures of shading hooks, separate from material settings.
#[derive(Debug, Clone)]
pub struct StyleShadingInputDecl {
    pub name: String,
    pub ty: String,
    pub scope: String,
    pub span: Span,
}

/// A selected engine geometry producer captured by an indexed draw operation.
#[derive(Debug, Clone)]
pub struct PreparedDraw {
    pub producer_pass: String,
    pub producer_hook: String,
    pub producer_node: String,
    pub resource: StructDecl,
    pub parameter: String,
}

/// Graph construction is distinct from shader statements and shader control flow.
#[derive(Debug, Clone)]
pub enum StyleGraphNode {
    BindShading {
        name: String,
        value: SExpr,
    },
    Require(SExpr),
    ForSelf(Vec<Spanned<StyleGraphNode>>),
    At {
        point: String,
        target: String,
        body: Vec<Spanned<StyleGraphNode>>,
    },
    StaticIf {
        condition: SExpr,
        then_body: Vec<Spanned<StyleGraphNode>>,
        else_body: Vec<Spanned<StyleGraphNode>>,
    },
    StaticFor {
        variable: String,
        range: SExpr,
        body: Vec<Spanned<StyleGraphNode>>,
    },
    Call(SExpr),
    Let {
        name: String,
        value: SExpr,
    },
}

/// Reusable operation metadata. The retained pass owns its shader functions.
#[derive(Debug, Clone)]
pub struct StyleOperation {
    pub inputs: Vec<StyleInputDecl>,
    pub raster: Option<SExpr>,
    pub generated_vertices: Option<(SExpr, SExpr)>,
    pub requirements: Vec<SExpr>,
    pub attachments: Vec<StyleGraphField>,
    pub visibility: Option<SExpr>,
    pub sort_position: Option<SExpr>,
    pub compute: Option<ComputeOperation>,
}

#[derive(Debug, Clone)]
pub struct ComputeOperation {
    pub return_ty: Spanned<String>,
    pub output: Option<ComputeOutput>,
    pub workgroup_size: Option<Vec<SExpr>>,
    pub threads: Option<Vec<SExpr>>,
    pub returned: Option<Spanned<String>>,
}

#[derive(Debug, Clone)]
pub struct ComputeOutput {
    pub name: String,
    pub ty: String,
    pub extents: Vec<SExpr>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Interface method declarations are staged for interface type checking"
)]
#[derive(Debug, Clone)]
pub struct InterfaceMethodDecl {
    pub name: String,
    pub name_span: Span,
    /// Parameters as (name, type_name) pairs.
    pub params: Vec<FnParam>,
    pub ret_ty: Option<(String, Span)>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Interface declarations are staged for generic bound checking"
)]
#[derive(Debug, Clone)]
pub struct InterfaceDecl {
    pub entry: Option<(String, String)>,
    pub name: String,
    pub name_span: Span,
    pub methods: Vec<InterfaceMethodDecl>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Conformance declarations are staged for generic bound checking"
)]
#[derive(Debug, Clone)]
pub struct ConformanceDecl {
    /// The concrete type name conforming to the interface.
    pub type_name: String,
    pub type_name_span: Span,
    pub interface_name: String,
    pub interface_name_span: Span,
    /// Method implementations provided by this conformance.
    pub methods: Vec<FnDecl>,
    /// Contract-style root instances validate method bodies in their owning
    /// entry scope so instance parameters remain available as captures.
    pub body_checked_with_instance: bool,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Enum variants keep source spans for diagnostics and tooling"
)]
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Enum declaration metadata is staged for semantic expansion"
)]
#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub name: String,
    pub name_span: Span,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Struct declarations are staged for semantic expansion"
)]
#[derive(Debug, Clone)]
pub struct StructFieldDecl {
    pub attrs: Vec<(String, Vec<String>)>,
    pub semantic: Option<String>,
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub default: Option<SExpr>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Struct declarations are staged for semantic expansion"
)]
#[derive(Debug, Clone)]
pub struct StructDecl {
    pub attrs: Vec<PipelineAttribute>,
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<StructFieldDecl>,
    pub span: Span,
}

/// The three locality classes for user-defined effects (§16.1).
///
/// `Point` effects are pure per-pixel functions that may not read neighbors.
/// `Local(r)` effects may sample within a bounded radius.
/// `Global` effects may read arbitrary coordinates (e.g. image-wide statistics).
#[allow(
    dead_code,
    reason = "Locality class metadata is staged for effect type checking"
)]
#[derive(Debug, Clone)]
pub enum LocalityClass {
    Point,
    Local {
        /// Optional radius-bound expression — e.g. `local(spread)`.
        constraint: Option<Box<SExpr>>,
    },
    Global,
}

/// One side of a rewrite composition rule: a single named effect call with
/// named parameter holes that bind values from the LHS and carry them to the
/// result pattern.
#[allow(
    dead_code,
    reason = "Rewrite pattern nodes are staged for the rewrite rule engine"
)]
#[derive(Debug, Clone)]
pub struct RewritePattern {
    pub name: String,
    pub name_span: Span,
    pub args: Vec<SExpr>,
    pub span: Span,
}

/// The `lhs ∘ rhs` composition pattern on the left-hand side of a rewrite
/// rule.  Written `rewrite <outer>(<args>) compose <inner>(<args>)`.
#[allow(
    dead_code,
    reason = "Rewrite composition nodes are staged for the rewrite rule engine"
)]
#[derive(Debug, Clone)]
pub struct RewriteComposition {
    /// The outermost effect (the one applied last, i.e., `outer(inner(...))`).
    pub outer: RewritePattern,
    /// The innermost effect (the one applied first).
    pub inner: RewritePattern,
    pub span: Span,
}

/// A complete rewrite rule: `rewrite <lhs> => <result> [when <guard>]`.
#[allow(
    dead_code,
    reason = "Rewrite rule nodes are staged for the rewrite rule engine"
)]
#[derive(Debug, Clone)]
pub struct RewriteRule {
    pub lhs: RewriteComposition,
    pub result: RewritePattern,
    /// Optional numeric tolerance (`within ε`) used for rewrite
    /// self-verification. Must be a positive compile-time scalar.
    pub tolerance: Option<Box<SExpr>>,
    /// Optional boolean guard expression evaluated over bound parameter values.
    pub guard: Option<Box<SExpr>>,
    pub span: Span,
}

/// A user-defined effect declaration:
/// `effect <name>(<params>) <locality> { <body> [<rewrite_rules>] }`
#[allow(
    dead_code,
    reason = "Effect declaration metadata is staged for semantic expansion"
)]
#[derive(Debug, Clone)]
pub struct EffectDecl {
    pub name: String,
    pub name_span: Span,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<FnParam>,
    pub locality: LocalityClass,
    pub rewrites: Vec<RewriteRule>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// Top-level `tags Domain { value, ... }` declaration.
#[allow(
    dead_code,
    reason = "Tag declarations are parsed as staged type-system vocabulary"
)]
#[derive(Debug, Clone)]
pub struct TagsDecl {
    pub domain: String,
    pub domain_span: Span,
    pub values: Vec<Spanned<String>>,
    pub span: Span,
}

/// Top-level `axis_default: static|uniform` declaration.
#[allow(
    dead_code,
    reason = "Axis default mode is parsed for staged pipeline DSL support"
)]
#[derive(Debug, Clone)]
pub struct AxisDefaultDecl {
    pub mode: String,
    pub mode_span: Span,
    pub span: Span,
}

/// A single value in an axis domain, optionally carrying nested sub-axes.
#[allow(
    dead_code,
    reason = "Axis values are parsed for staged pipeline variant validation"
)]
#[derive(Debug, Clone)]
pub struct AxisValueDecl {
    pub name: Spanned<String>,
    pub sub_axes: Vec<AxisSubAxisDecl>,
    pub span: Span,
}

/// A nested sub-axis declared under a specific parent value.
#[allow(
    dead_code,
    reason = "Sub-axes are parsed for staged pipeline variant validation"
)]
#[derive(Debug, Clone)]
pub struct AxisSubAxisDecl {
    pub name: String,
    pub name_span: Span,
    pub domain: AxisDomain,
    pub span: Span,
}

/// Top-level `axis @known(mode) name: ...` declaration.
#[allow(
    dead_code,
    reason = "Axis declarations are parsed for staged pipeline variant validation"
)]
#[derive(Debug, Clone)]
pub struct AxisDecl {
    pub name: String,
    pub name_span: Span,
    pub known_mode: String,
    pub known_mode_span: Span,
    pub domain: AxisDomain,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Axis domains are parsed for staged pipeline variant validation"
)]
#[derive(Debug, Clone)]
pub enum AxisDomain {
    ValueSet(Vec<AxisValueDecl>),
    SymbolicType { name: String, span: Span },
}

/// Generic metadata attribute parsed from `@name(...)` or `@name`.
#[allow(
    dead_code,
    reason = "Pipeline attribute metadata is parsed for staged checker/lowering support"
)]
#[derive(Debug, Clone)]
pub struct PipelineAttribute {
    /// Checked expression arguments for resource-policy attributes.
    pub expressions: Vec<SExpr>,
    pub name: String,
    pub name_span: Span,
    pub args: Vec<String>,
    pub args_span: Option<Span>,
    pub span: Span,
}

/// A staged binding-like declaration in a pass body.
#[allow(
    dead_code,
    reason = "Pass bindings are parsed for staged pipeline attribute validation"
)]
#[derive(Debug, Clone)]
pub struct PassBindingDecl {
    /// Compiler-owned logical input path, such as a resource's `values.count`.
    /// Source bindings cannot declare or override this alias.
    pub operation_alias: Option<String>,
    pub group_index: Option<u32>,
    pub binding_index: Option<u32>,
    pub name: String,
    pub name_span: Span,
    pub attrs: Vec<PipelineAttribute>,
    pub value_signature: Option<String>,
    pub span: Span,
}

/// A staged pass permutation declaration inside `permutations { ... }`.
#[allow(
    dead_code,
    reason = "Pass permutation declarations are parsed for staged pipeline variant validation"
)]
#[derive(Debug, Clone)]
pub struct PassPermutationDecl {
    pub name: String,
    pub name_span: Span,
    pub attrs: Vec<PipelineAttribute>,
    pub values: Vec<Spanned<String>>,
    pub when_guard: Option<SExpr>,
    pub else_value: Option<Spanned<String>>,
    pub value_signature: Option<String>,
    pub span: Span,
}

/// A typed `require` clause in a staged pass invariant block.
#[allow(
    dead_code,
    reason = "Pass requirement clauses are parsed for staged pipeline invariant validation"
)]
#[derive(Debug, Clone)]
pub enum PassRequirementClause {
    Implication { guard: SExpr, constraint: SExpr },
    Plain { constraint: SExpr },
}

impl PassRequirementClause {
    pub fn guard_expr(&self) -> Option<&SExpr> {
        match self {
            Self::Implication { guard, .. } => Some(guard),
            Self::Plain { .. } => None,
        }
    }

    pub fn constraint_expr(&self) -> &SExpr {
        match self {
            Self::Implication { constraint, .. } | Self::Plain { constraint } => constraint,
        }
    }
}

/// A staged `require { ... }` pass invariant clause.
#[allow(
    dead_code,
    reason = "Pass requirement declarations are parsed for staged pipeline invariant validation"
)]
#[derive(Debug, Clone)]
pub struct PassRequirementDecl {
    pub clause: PassRequirementClause,
    pub span: Span,
}

/// Parameter declaration captured from a staged pass hook signature.
#[allow(
    dead_code,
    reason = "Pass hook signatures are parsed for staged vertex contract validation"
)]
#[derive(Debug, Clone)]
pub struct PassFnHookParamDecl {
    pub attrs: Vec<PipelineAttribute>,
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub span: Span,
}

/// Top-level pass hook declaration, for example `fn vertex(v: Shaded) -> Surf { ... }`.
#[allow(
    dead_code,
    reason = "Pass hook declarations are parsed for staged pass contract validation"
)]
#[derive(Debug, Clone)]
pub struct PassFnHookDecl {
    pub attrs: Vec<PipelineAttribute>,
    /// Resolved dispatch plan retained for resource-aware pass specialization.
    pub dispatch: Option<PassDispatchPlan>,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<PassFnHookParamDecl>,
    pub return_ty: Option<Spanned<String>>,
    /// Complete body tokens, including braces and original source spans. Parse
    /// with `parser::pass_hook_body` when selecting a pass for checking/lowering.
    /// Unselected staged engine declarations retain their authored code.
    pub body: Vec<(crate::lexer::Token, Span)>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct PassDispatchPlan {
    pub contract: String,
    /// Engine opt-in: only the current draw's selected implementation is callable.
    pub draw_scoped: bool,
    pub cases: Vec<PassDispatchCase>,
    /// Authored fallback, before generated selector branches are inserted.
    pub fallback: Vec<(crate::lexer::Token, Span)>,
}

#[derive(Debug, Clone)]
pub struct PassDispatchCase {
    pub shading_inputs: Vec<StyleShadingInputDecl>,
    pub selector: u32,
    pub function: String,
    pub arguments: String,
}

/// A staged pass reference entry in a pipeline block.
#[allow(
    dead_code,
    reason = "Pipeline pass refs are parsed for staged attribute placement validation"
)]
#[derive(Debug, Clone)]
pub struct PipelinePassRef {
    pub invocation: Option<Box<fresco_artifact::ManifestStyleInvocation>>,
    pub name: String,
    pub name_span: Span,
    pub attrs: Vec<PipelineAttribute>,
    pub span: Span,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EntryProperty {
    pub editable: bool,
    pub block: Option<String>,
    pub block_present: bool,
    pub entry: String,
    pub name: String,
    pub ty: String,
    pub value: f32,
    pub choices: Vec<(String, f32)>,
    pub permutation: bool,
    pub value_span: Option<Span>,
    pub insert_at: usize,
}

/// Top-level `pass Name [for Material] { ... }` declaration.
#[allow(
    dead_code,
    reason = "Pass declarations are parsed as staged pipeline DSL support"
)]
#[derive(Debug, Clone)]
pub struct PassDecl {
    /// Compiler-linked service captures, validated against their renderer sources.
    pub service_captures: Vec<StyleInputDecl>,
    pub compute_invocation: Option<Box<fresco_artifact::ManifestComputeInvocation>>,
    pub prepared_draw: Option<Box<PreparedDraw>>,
    pub preparation: Option<Box<PreparedDraw>>,
    pub operation: Option<Box<StyleOperation>>,
    pub state: Vec<(String, SExpr)>,
    pub entry_properties: Vec<EntryProperty>,
    /// Typed lexical bindings supplied by an instantiated engine entry contract.
    pub entry_bindings: Vec<Stmt>,
    pub name: String,
    pub name_span: Span,
    pub source_file: String,
    pub material_name: Option<String>,
    pub material_span: Option<Span>,
    pub attrs: Vec<PipelineAttribute>,
    pub stage: Option<Spanned<String>>,
    pub draw: Option<Spanned<String>>,
    pub blend: Option<Spanned<String>>,
    pub reads: Vec<Spanned<String>>,
    pub writes: Vec<Spanned<String>>,
    pub permutations: Vec<PassPermutationDecl>,
    pub requirements: Vec<PassRequirementDecl>,
    pub bindings: Vec<PassBindingDecl>,
    pub hooks: Vec<PassFnHookDecl>,
    pub vertex_interface: Option<Spanned<String>>,
    pub span: Span,
}

/// Top-level `pipeline Name [for Material] { pass_a pass_b ... }` declaration.
#[allow(
    dead_code,
    reason = "Pipeline declarations are parsed as staged pipeline DSL support"
)]
#[derive(Debug, Clone)]
pub struct PipelineDecl {
    pub name: String,
    pub name_span: Span,
    pub material_name: Option<String>,
    pub material_span: Option<Span>,
    pub pipeline_type: String,
    pub pipeline_type_span: Span,
    pub passes: Vec<Spanned<String>>,
    pub attrs: Vec<PipelineAttribute>,
    pub pass_refs: Vec<PipelinePassRef>,
    /// Checked engine completion ports, populated after provider validation.
    /// Source syntax cannot construct this compiler-owned metadata directly.
    pub resource_ports: Vec<fresco_artifact::ManifestResourcePort>,
    pub span: Span,
}

/// Authored resource namespace; optional explicit GPU group assignment.
#[derive(Debug, Clone)]
pub struct ResourceGroupDecl {
    pub name: String,
    pub attrs: Vec<PipelineAttribute>,
    pub span: Span,
}

/// Physical GPU groups for compiler-managed parameter, texture, storage and global resources.
#[derive(Debug, Clone, Copy)]
pub struct ResourceLayout(pub [u32; 4]);
impl Default for ResourceLayout {
    fn default() -> Self {
        Self([0, 1, 2, 3])
    }
}

#[allow(dead_code, reason = "Program aggregates staged top-level declarations")]
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub resource_layout: ResourceLayout,
    pub groups: Vec<ResourceGroupDecl>,
    pub authored_entries: Vec<AuthoredEntry>,
    pub pragmas: Vec<PragmaDecl>,
    pub imports: Vec<ImportDecl>,
    pub consts: Vec<ConstDecl>,
    pub enums: Vec<EnumDecl>,
    pub structs: Vec<StructDecl>,
    /// Engine-defined resource aggregates, separated before ordinary value checking.
    pub resource_types: Vec<StructDecl>,
    pub params: Vec<GlobalParamDecl>,
    pub tags: Vec<TagsDecl>,
    pub axes: Vec<AxisDecl>,
    pub axis_defaults: Vec<AxisDefaultDecl>,
    pub passes: Vec<PassDecl>,
    pub pipelines: Vec<PipelineDecl>,
    pub material_properties: Vec<MaterialPropertiesDecl>,
    pub schema_programs: Vec<SchemaProgramDecl>,
    pub vertex_interfaces: Vec<VertexInterfaceDecl>,
    pub vertex_formats: Vec<VertexFormatDecl>,
    pub vertex_factories: Vec<VertexFactoryDecl>,
    pub schema_expressions: Vec<SchemaExpressionDecl>,
    pub schema_evaluators: Vec<SchemaEvaluatorDecl>,
    pub texture_types: Vec<TextureTypeDecl>,
    pub functions: Vec<FnDecl>,
    pub interfaces: Vec<InterfaceDecl>,
    pub conformances: Vec<ConformanceDecl>,
    pub style_contracts: Vec<StyleContractDecl>,
    pub style_capabilities: Vec<StyleCapabilityDecl>,
    pub style_providers: Vec<StyleProviderDecl>,
    pub styles: Vec<StyleDecl>,
    pub canvases: Vec<Canvas>,
    pub surfaces: Vec<SurfaceDecl>,
    pub templates: Vec<TemplateDecl>,
    pub effects: Vec<EffectDecl>,
}

#[derive(Debug, Clone, Copy)]
pub enum RootEntryRef<'a> {
    Canvas(&'a Canvas),
    Surface(&'a SurfaceDecl),
}

impl<'a> RootEntryRef<'a> {
    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn as_normalized_root_entry(self) -> NormalizedRootEntry {
        match self {
            Self::Canvas(canvas) => canvas.as_normalized_root_entry(),
            Self::Surface(surface) => surface.as_normalized_root_entry(),
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn name(self) -> &'a str {
        match self {
            Self::Canvas(canvas) => &canvas.name,
            Self::Surface(surface) => &surface.name,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn kind(self) -> &'static str {
        match self {
            Self::Canvas(_) => "canvas",
            Self::Surface(_) => "surface",
        }
    }
}

impl Program {
    pub fn root_entries(&self) -> impl Iterator<Item = RootEntryRef<'_>> {
        self.canvases
            .iter()
            .map(RootEntryRef::Canvas)
            .chain(self.surfaces.iter().map(RootEntryRef::Surface))
    }
}

#[allow(
    dead_code,
    reason = "Top-level constants are retained for checker-time constant binding"
)]
#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub value: SExpr,
    pub span: Span,
}

/// A file-scope `param` declaration: `param name: Type` or
/// `param name: Type = default (in min..max)?`.
///
/// Unlike the scoped `param` statement (see [`Stmt::Param`]), the default is
/// optional here: a struct-typed global param (e.g. `param frame: FrameGlobals`)
/// has no meaningful compile-time default since the host supplies every field
/// at runtime. Scalar/array/texture-typed global params behave exactly like
/// their canvas/surface-scoped counterparts and are declared into every
/// checking context that references them.
#[derive(Debug, Clone)]
pub struct GlobalParamDecl {
    pub name: String,
    pub name_span: Span,
    pub ty_name: String,
    pub ty_span: Span,
    pub default: Option<SExpr>,
    pub range: Option<(SExpr, SExpr)>,
    pub span: Span,
}

#[allow(
    dead_code,
    reason = "Pragma declarations are retained for compiler-context overrides"
)]
#[derive(Debug, Clone)]
pub enum PragmaValue {
    Number(f64),
    Ident(String),
}

#[allow(
    dead_code,
    reason = "Pragma declarations are retained for compiler-context overrides"
)]
#[derive(Debug, Clone)]
pub struct PragmaDecl {
    pub key: String,
    pub key_span: Span,
    pub value: PragmaValue,
    pub value_span: Span,
    pub span: Span,
}

#[allow(dead_code, reason = "Import spans are retained for richer diagnostics")]
#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub path: String,
    pub span: Span,
}

/// One channel→name mapping inside a `texture_type` body.
/// `channel` is one of `"r"`, `"g"`, `"b"`, or `"a"`.
#[derive(Debug, Clone)]
pub enum TextureChannelDecode {
    /// Affine decode applied after channel read: `decoded = raw * mul + add`.
    Affine { mul: f32, add: f32 },
    /// Expression decode evaluated with `raw` bound to the sampled channel.
    Expr(SExpr),
}

#[derive(Debug, Clone)]
pub struct TextureChannelDef {
    pub channel: String,
    pub channel_span: Span,
    pub semantic_name: String,
    pub decode: Option<TextureChannelDecode>,
    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub span: Span,
}

/// Top-level `texture_type Name { channel_defs }` declaration.
#[allow(
    dead_code,
    reason = "Texture type declarations are staged for semantic expansion"
)]
#[derive(Debug, Clone)]
pub struct TextureTypeDecl {
    pub name: String,
    pub name_span: Span,
    pub result_ty: Option<(String, Span)>,
    pub channels: Vec<TextureChannelDef>,
    pub result_expr: Option<SExpr>,
    pub span: Span,
}
