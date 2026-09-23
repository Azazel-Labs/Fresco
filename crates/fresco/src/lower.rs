//! Lowering: typed HIR -> naga IR (`naga::Module`).
//!
//! We build the IR directly (no WGSL string round-trip), which gets us every
//! naga backend — WGSL, HLSL, MSL, SPIR-V, GLSL — plus naga's validator.
//!
//! naga mechanics worth knowing if you're reading this for the first time:
//! - Function bodies are an expression *arena* plus a statement list. Every
//!   runtime expression must be covered by a `Statement::Emit(range)` before
//!   anything uses it; literals and function arguments must NOT be emitted.
//!   `FnCtx::add` handles that invariant in one place.
//! - `named_expressions` entries become `let name = ...;` in the WGSL
//!   backend, which is how we keep the emitted code readable (design doc
//!   §12: "emitted code is meant to be read").
//!
//! What this pass implements from the design doc:
//! - shape lowering (§10.1): fill/stroke/booleans as distance math
//! - effect-on-shape rewrites' final form (§10.2): shadow/glow/soften
//! - space inversion (§3): the sample point gets the inverse transform
//! - blend folding (§10.3): the compose stack becomes nested mix/adds on a
//!   vec3 accumulator (alpha channel elided — v0 assumes an opaque result)
//! - shape CSE (§10.3): SDFs are memoized per (shape, coordinate) pair

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::num::NonZeroU32;

use naga::{
    BinaryOperator as Bo, DerivativeAxis as Da, DerivativeControl as Dc, Expression as Ex, Handle,
    MathFunction as Mf, UnaryOperator as Uo, VectorSize,
};

use crate::hir::{
    Blend, CenteredMode, GlowColorSpace, GlowReach, Hir, Layer, LayerId, Locality, Shape, ShapeId,
    Sx, UserFnHelper, V2, VerticalAxis, Xform,
};
use crate::material_hir::MaterialHir;

// Internal resource classes: parameters, textures, storage/path buffers, and globals.
// ResourceLayout relocates these classes to engine-declared groups after lowering.
pub(crate) const CANVAS_STORAGE_GROUP: u32 = 2;

mod canvas;
mod cellular;
mod contour;
use contour::ContourResult;
pub(crate) mod compute;
mod ir;
mod layer;
mod numeric;
mod scalar;
mod shape;
mod space;
mod surface;

use canvas::{
    lower_canvas_entry_fn, lower_canvas_pass_entry_fn, lower_path_arc_sample_fn,
    lower_path_nearest_fn, lower_scatter_body_fn, lower_scene_fn, lower_scene_pass_fn,
    lower_user_helper_fn,
};

pub(crate) fn pass_target_texture_name(canvas_name: &str, target_id: usize) -> String {
    format!("__fresco_pass_target_{canvas_name}_{target_id}")
}

fn effective_render_root(hir: &Hir) -> LayerId {
    match &hir.layers[hir.root] {
        Layer::MotionBlur { inner, .. } => *inner,
        _ => hir.root,
    }
}

fn layer_inputs(layer: &Layer) -> Vec<LayerId> {
    match layer {
        Layer::Solid(_)
        | Layer::ColorExpr { .. }
        | Layer::Grey { .. }
        | Layer::Fill { .. }
        | Layer::FillExpr { .. }
        | Layer::FillGradient { .. }
        | Layer::Shadow { .. }
        | Layer::Glow { .. }
        | Layer::InnerGlow { .. }
        | Layer::Bevel { .. }
        | Layer::Soften { .. }
        | Layer::Image { .. }
        | Layer::ImageAt { .. } => Vec::new(),
        Layer::Blur { inner, .. }
        | Layer::MotionBlur { inner, .. }
        | Layer::GlowFx { inner, .. }
        | Layer::Opacity { inner, .. }
        | Layer::Tint { inner, .. }
        | Layer::PostProcess { inner, .. }
        | Layer::InSpace { inner, .. } => vec![*inner],
        Layer::If {
            then_layer,
            else_layer,
            ..
        } => vec![*then_layer, *else_layer],
        Layer::Compose(entries) => entries.iter().map(|(id, _)| *id).collect(),
        Layer::ScatterBins { body, .. } => vec![*body],
        Layer::UserEffect { inner, .. } => inner.map(|id| vec![id]).unwrap_or_default(),
    }
}

fn subgraph_stays_in_pass(
    hir: &Hir,
    render_root: LayerId,
    pass_id: usize,
    layer_to_pass: &[usize],
) -> bool {
    let mut stack = vec![render_root];
    let mut visited = HashSet::new();

    while let Some(layer_id) = stack.pop() {
        if !visited.insert(layer_id) {
            continue;
        }
        if layer_to_pass.get(layer_id).copied() != Some(pass_id) {
            return false;
        }
        stack.extend(layer_inputs(&hir.layers[layer_id]));
    }

    true
}

fn scene_function_dedup_key(function: &naga::Function) -> String {
    let mut canonical = function.clone();
    canonical.name = None;
    canonical.named_expressions.clear();
    for arg in &mut canonical.arguments {
        arg.name = None;
        arg.binding = None;
    }
    if let Some(result) = &mut canonical.result {
        result.binding = None;
    }
    for (_, local) in canonical.local_variables.iter_mut() {
        local.name = None;
    }
    format!("{canonical:#?}")
}

fn collect_helper_ids_from_sx(sx: &Sx, out: &mut BTreeSet<String>) {
    sx.walk_preorder(&mut |node| {
        if let Sx::UserCall { call, .. } = node {
            out.insert(call.helper_id.clone());
        }
    });
}

fn collect_helper_ids_from_stmt(stmt: &crate::hir::UserFnStmt, out: &mut BTreeSet<String>) {
    stmt.walk_sx(&mut |sx| collect_helper_ids_from_sx(sx, out));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoweringPolicy {
    Readable,
    Compact,
}

#[derive(Debug, Default, Clone)]
pub struct ScatterLayerStats {
    pub layer_id: usize,
    pub strategy: String,
    pub occupied_bins: usize,
    pub emitted_instances: usize,
    pub procedural_samples: usize,
    pub procedural_cap: usize,
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub emitted_instructions: usize,
    pub feature_instruction_counts: BTreeMap<String, usize>,
    pub feature_instruction_counts_by_locality: BTreeMap<String, BTreeMap<String, usize>>,
    pub sdf_evals: usize,
    pub sdf_cache_hits: usize,
    pub outline_thin_fade_count: usize,
    pub scatter_procedural_paths: usize,
    pub scatter_branch_tree_paths: usize,
    pub scatter_compact_paths: usize,
    pub scatter_compact_reason_bins: usize,
    pub scatter_compact_reason_instances: usize,
    pub scatter_occupied_bins: usize,
    pub scatter_emitted_instances: usize,
    pub scatter_procedural_samples: usize,
    pub scatter_procedural_cap: usize,
    pub scatter_layers: Vec<ScatterLayerStats>,
    /// Number of iteration constructs (e.g. motion_blur sample loops) lowered as GPU loops.
    /// Populated in Phase 1; zero until then.
    pub iter_loops: usize,
    /// Number of iteration constructs unrolled as repeated inline code rather than loops.
    /// Any non-zero value here is a codegen quality regression; Phase 1 eliminates this.
    pub unrolled_iters: usize,
}

type GlowWarpCache =
    HashMap<(ShapeId, Handle<Ex>, Handle<Ex>, Handle<Ex>), (Handle<Ex>, Handle<Ex>)>;

/// Output of the lowering pass: the naga module, per-canvas stats, and a map
/// from texture name to its binding index in the texture allocation class (sampler is binding 0).
pub struct LoweringResult {
    pub module: naga::Module,
    pub stats: Vec<Stats>,
    /// `name -> binding_index` in `@group(1)`. The sampler is always at
    /// `@group(1) @binding(0)`; the first texture is at binding 1, etc.
    pub tex_bindings: HashMap<String, u32>,
    /// `canvas_name -> (target_id -> binding_index)` for compiler-internal
    /// pass-target textures used by multi-pass replay.
    pub pass_target_bindings: HashMap<String, HashMap<usize, u32>>,
    /// `canvas_name:path_index -> binding_index` in `@group(2)` for path
    /// segment tables lowered as storage buffers.
    pub path_bindings: HashMap<String, u32>,
    /// `canvas_name:param_name -> binding_index` in `@group(0)` for dynamic
    /// array params lowered as storage buffers.
    pub param_bindings: HashMap<String, u32>,
    /// `global_param_name -> binding_index` in `@group(3)` for struct-typed
    /// global `param` uniform buffers (e.g. `frame`).
    pub global_uniform_bindings: HashMap<String, u32>,
}

#[derive(Copy, Clone)]
struct TypeHandles {
    entry_context: Option<Handle<naga::Type>>,
    f32_: Handle<naga::Type>,
    u32_: Handle<naga::Type>,
    i32_: Handle<naga::Type>,
    bool_: Handle<naga::Type>,
    native_vectors: [[Handle<naga::Type>; 3]; 4],
    v2: Handle<naga::Type>,
    v3: Handle<naga::Type>,
    v4: Handle<naga::Type>,
    ctx: Handle<naga::Type>,
    m2: Handle<naga::Type>,
    m3: Handle<naga::Type>,
    m4: Handle<naga::Type>,
    path_seg: Handle<naga::Type>,
    contour_arrays: [Handle<naga::Type>; 3],
}

struct ModuleBuilder {
    module: naga::Module,
    function_dedup_cache: HashMap<String, Handle<naga::Function>>,
}

struct FunctionBuilder {
    function: naga::Function,
    block_stack: Vec<naga::Block>,
    policy: LoweringPolicy,
    emitted_instructions: usize,
    feature_instruction_counts: BTreeMap<String, usize>,
    feature_instruction_counts_by_locality: BTreeMap<String, BTreeMap<String, usize>>,
    active_feature_tag: Option<String>,
    active_locality: Option<Locality>,
}

struct IrBuilder {
    function: FunctionBuilder,
    types: TypeHandles,
    uv: Handle<Ex>,
    time: Handle<Ex>,
    time_override: Option<Handle<Ex>>,
    delta: Handle<Ex>,
    res: Handle<Ex>,
    px: Handle<Ex>,
    aa: Handle<Ex>,
    /// Footprint Jacobian matrix elements for gradient-driven filtering (§3.8, §24.1).
    /// These are computed from the canvas_space chain and represent the derivative
    /// of the sample mapping. The parallelogram spanned by the columns `[j11, j21]`
    /// and `[j12, j22]` is the footprint — the region of local space one output pixel covers.
    jacobian_j11: Option<Handle<Ex>>,
    jacobian_j12: Option<Handle<Ex>>,
    jacobian_j21: Option<Handle<Ex>>,
    jacobian_j22: Option<Handle<Ex>>,
    param_scalars: HashMap<String, Handle<Ex>>,
    param_scalar_ptrs: HashSet<Handle<Ex>>,
    tex_globals: HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    /// Storage-buffer globals for dynamic array params (`array<T>`), keyed by param name.
    param_storage_globals: HashMap<String, Handle<naga::GlobalVariable>>,
    /// Struct-typed global uniform buffers (e.g. `frame`), keyed by binding name.
    global_uniform_globals: HashMap<String, Handle<naga::GlobalVariable>>,
}

/// An axis-aligned periodic sample domain and its seam-free screen derivatives.
/// Bounds describe the canonical cell, not permission to repeat arbitrary shapes.
#[derive(Clone, Copy)]
struct BoxFilterDomain {
    x_bounds: (Handle<Ex>, Handle<Ex>),
    dx: Handle<Ex>,
    dy: Handle<Ex>,
}

struct FnCtx<'h> {
    hir: &'h Hir,
    ir: IrBuilder,
    /// True when Sx::Param resolves to immutable canvas/runtime parameters.
    /// Helper and scatter-lowering contexts set this false because their
    /// param slots include mutable locals.
    params_are_immutable: bool,
    current_pass_id: Option<usize>,
    layer_to_pass: Vec<usize>,
    pass_output_target_names: HashMap<usize, String>,
    /// Shape CSE (§10.3): one SDF evaluation per (shape, coordinate).
    sdf_cache: HashMap<(ShapeId, Handle<Ex>), Handle<Ex>>,
    /// Scalar CSE: reuse lowered scalar subexpressions at the same sample point.
    sx_cache: HashMap<(Sx, Handle<Ex>), Handle<Ex>>,
    contour_cache: HashMap<(u32, Sx, Handle<Ex>), ContourResult>,
    /// Reuse lowered scalar subexpressions that are invariant to the sample point.
    invariant_sx_cache: HashMap<Sx, Handle<Ex>>,
    /// Memoized sample-point invariance for user helper bodies.
    invariant_user_helper_cache: HashMap<String, bool>,
    /// Reuse local pixel span computations at the same sample point.
    local_pixel_span_cache: HashMap<Handle<Ex>, Handle<Ex>>,
    /// Reuse shape anchor computations at the same sample point.
    shape_anchor_cache: HashMap<(ShapeId, Handle<Ex>), Handle<Ex>>,
    /// Reuse shape half-extents at the same sample point.
    shape_half_extent_cache: HashMap<(ShapeId, Handle<Ex>), Handle<Ex>>,
    /// Reuse shape-local normalized coordinates at the same sample point.
    shape_local_point_cache: HashMap<(ShapeId, Handle<Ex>), Handle<Ex>>,
    /// Reuse anchor-centered shape-space points keyed by anchor sample point and target sample point.
    shape_anchor_space_point_cache: HashMap<(ShapeId, Handle<Ex>, Handle<Ex>), Handle<Ex>>,
    /// Reuse GlowReach::Vec2 warp results keyed by shape, sample point, and reach handles.
    shape_glow_warp_cache: GlowWarpCache,
    source_color_ctx: Vec<SourceColorCtx>,
    effect_input_layer_ctx: Vec<LayerId>,
    scatter_instance_ctx: Vec<ScatterInstanceCtx>,
    repeat_cell_ctx: Vec<RepeatCellCtx>,
    /// Outer cellular quadrature evaluates occupancy before filtering.
    cellular_sample_depth: u32,
    discontinuity_space_depth: usize,
    box_filter_domains: HashMap<Handle<Ex>, BoxFilterDomain>,
    /// Pre-built helper functions for scatter body lowering (keyed by scatter
    /// layer id). When populated, scatter instances use `Statement::Call` to
    /// the helper instead of inlining the full body per instance.
    scatter_body_fns: HashMap<LayerId, Handle<naga::Function>>,
    /// Pre-built user helper functions indexed by helper id.
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    /// Runtime call-result cache to avoid emitting duplicate call statements
    /// when multiple components read from the same vector-returning call.
    user_call_cache: HashMap<(crate::hir::UserFnCall, Option<Handle<Ex>>), Handle<Ex>>,
    /// Shared path nearest-search helper handles by path profile id.
    path_helper_fns: HashMap<usize, PathHelperFns>,
    /// Cache nearest-search helper results per path and sample coordinate.
    path_sample_cache: HashMap<(usize, Handle<Ex>), Handle<Ex>>,
    /// Cache arc-length helper results per path and scalar sample parameter.
    path_param_sample_cache: HashMap<(usize, Handle<Ex>), Handle<Ex>>,
    /// Reuse texture samples for repeated channel reads from the same texture at the same coordinate.
    texture_sample_cache:
        HashMap<(Handle<naga::GlobalVariable>, TextureSampleCoordKey), Handle<Ex>>,
    /// Reuse the per-pixel gradient dither vector across multiple gradient fills.
    gradient_dither3_cache: Option<Handle<Ex>>,
    color_shape: Option<ShapeId>,
    cell_inset_pixel_span: Option<Handle<Ex>>,
    /// Var-override stack for `Sx::Var` resolution inside `Layer::UserEffect` lowering.
    ///
    /// When lowering a `UserEffect` node, actual argument `Handle<Ex>` values are pushed
    /// for each parameter name; `sx_at(Sx::Var(name), p)` looks up the top of the stack.
    /// After the body is lowered, the overrides are popped in reverse order.
    var_overrides: Vec<(String, Handle<Ex>)>,
    stats: Stats,
}

#[derive(Clone, Hash, PartialEq, Eq)]
enum TextureSampleCoordKey {
    /// Cache key for implicit sampling at the current sample point.
    Current,
    /// Cache key for explicit sampling coordinates.
    Explicit(Sx, Sx),
}

#[derive(Clone, Copy)]
enum PathTableStorage {
    Const {
        constant: Handle<naga::Constant>,
    },
    Buffer {
        global: Handle<naga::GlobalVariable>,
    },
}

#[derive(Clone, Copy)]
struct PathTableBinding {
    seg_count: u32,
    storage: PathTableStorage,
    demand: crate::hir::PathChannelDemand,
}

#[derive(Clone, Copy, Default)]
struct PathHelperFns {
    nearest_sample: Option<Handle<naga::Function>>,
    arc_sample: Option<Handle<naga::Function>>,
}

#[derive(Clone, Copy)]
struct SourceColorCtx {
    r: Handle<Ex>,
    g: Handle<Ex>,
    b: Handle<Ex>,
    a: Handle<Ex>,
}

#[derive(Clone, Copy)]
struct ScatterInstanceCtx {
    pos_x: Handle<Ex>,
    pos_y: Handle<Ex>,
    id: Handle<Ex>,
    index01: Handle<Ex>,
    age_norm: Handle<Ex>,
}

#[derive(Clone, Copy)]
struct RepeatCellCtx {
    geometry: Option<CellGeometryCtx>,
    scope_id: u32,
    id_x: Handle<Ex>,
    id_y: Handle<Ex>,
    center_x: Handle<Ex>,
    center_y: Handle<Ex>,
    uv_x: Handle<Ex>,
    uv_y: Handle<Ex>,
    rand: Handle<Ex>,
}

#[derive(Clone, Copy)]
struct CellGeometryCtx {
    cells: crate::hir::CellularSpace,
    site: [Handle<Ex>; 2],
    dx: Handle<Ex>,
    dy: Handle<Ex>,
}

/// Lower all HIR canvases in one naga module, reading `FRESCO_LOWERING_POLICY`
/// from the environment to choose between `Readable` (default) and `Compact`
/// output. Prefer [`lower_all_with_policy`] when you need deterministic
/// behaviour in tests or host-controlled rendering.
/// Build one real `var<uniform>` struct-typed global per unique struct-typed
/// global `param` (e.g. `param frame: FrameGlobals`), shared across every
/// canvas/surface in the compile so `frame.time`-style field reads all
/// resolve to the same naga global.
///
/// Placed at `@group(3)`, one binding index per unique global name, to avoid
/// any collision with the existing `@group(0)` param/storage bindings,
/// `@group(1)` texture bindings, and `@group(2)` path-segment storage
/// buffers in this legacy allocation path. General host layout obligations
/// are documented in `LANGUAGE.md`.
fn ensure_global_uniform_buffers<'a>(
    module: &mut naga::Module,
    t: &TypeHandles,
    defs: impl Iterator<Item = &'a crate::check::GlobalUniformDef>,
) -> (
    HashMap<String, Handle<naga::GlobalVariable>>,
    HashMap<String, u32>,
) {
    let mut globals = HashMap::new();
    let mut bindings = HashMap::new();
    let mut next_binding: u32 = 0;
    let mut seen = HashSet::new();

    for def in defs {
        if !seen.insert(def.name.clone()) {
            continue;
        }

        let mut members = Vec::with_capacity(def.fields.len());
        for ((field_name, _), (offset, components)) in def.fields.iter().zip(def.field_layouts()) {
            let ty = match components {
                1 => t.f32_,
                2 => t.v2,
                3 => t.v3,
                4 => t.v4,
                _ => unreachable!("validated global uniform component count"),
            };
            members.push(naga::StructMember {
                name: Some(field_name.clone()),
                ty,
                binding: None,
                offset,
            });
        }
        let span = def.byte_size();

        let struct_ty = module.types.insert(
            naga::Type {
                name: Some(def.ty_name.clone()),
                inner: naga::TypeInner::Struct { members, span },
            },
            naga::Span::UNDEFINED,
        );

        let global = module.global_variables.append(
            naga::GlobalVariable {
                name: Some(def.name.clone()),
                space: naga::AddressSpace::Uniform,
                binding: Some(naga::ResourceBinding {
                    // These are internal allocation classes, relocated through ResourceLayout.
                    group: 3,
                    binding: next_binding,
                }),
                ty: struct_ty,
                init: None,
                memory_decorations: naga::MemoryDecorations::empty(),
            },
            naga::Span::UNDEFINED,
        );
        bindings.insert(def.name.clone(), next_binding);
        next_binding += 1;
        globals.insert(def.name.clone(), global);
    }

    (globals, bindings)
}

pub fn lower_all(hirs: &[Hir], material_hirs: &[MaterialHir]) -> LoweringResult {
    lower_all_with_policy(hirs, material_hirs, LoweringPolicy::from_env())
}

pub fn lower_all_with_policy(
    hirs: &[Hir],
    material_hirs: &[MaterialHir],
    policy: LoweringPolicy,
) -> LoweringResult {
    let mut builder = ModuleBuilder::new();
    let mut t = builder.register_core_types();

    let need_path_geometry = hirs.iter().any(|hir| {
        hir.path_profiles.iter().any(|profile| {
            profile.demand.needs_path_geometry() && profile.flattened_segment_count > 0
        })
    });

    let mut path_seg_ty = t.path_seg;
    if need_path_geometry {
        path_seg_ty = builder.module.types.insert(
            naga::Type {
                name: Some("FrescoPathSeg".to_string()),
                inner: naga::TypeInner::Struct {
                    members: vec![
                        naga::StructMember {
                            name: Some("p0".to_string()),
                            ty: t.v2,
                            binding: None,
                            offset: 0,
                        },
                        naga::StructMember {
                            name: Some("p1".to_string()),
                            ty: t.v2,
                            binding: None,
                            offset: 8,
                        },
                        naga::StructMember {
                            name: Some("p2".to_string()),
                            ty: t.v2,
                            binding: None,
                            offset: 16,
                        },
                        naga::StructMember {
                            name: Some("p3".to_string()),
                            ty: t.v2,
                            binding: None,
                            offset: 24,
                        },
                        naga::StructMember {
                            name: Some("s0".to_string()),
                            ty: t.f32_,
                            binding: None,
                            offset: 32,
                        },
                        naga::StructMember {
                            name: Some("len".to_string()),
                            ty: t.f32_,
                            binding: None,
                            offset: 36,
                        },
                        naga::StructMember {
                            name: Some("kind".to_string()),
                            ty: t.u32_,
                            binding: None,
                            offset: 40,
                        },
                        naga::StructMember {
                            name: Some("mid_u".to_string()),
                            ty: t.f32_,
                            binding: None,
                            offset: 44,
                        },
                        naga::StructMember {
                            name: Some("_pad".to_string()),
                            ty: t.f32_,
                            binding: None,
                            offset: 48,
                        },
                    ],
                    span: fresco_artifact::PATH_SEGMENT_STRIDE,
                },
            },
            naga::Span::UNDEFINED,
        );
        t.path_seg = path_seg_ty;
    }

    let pass_plans: Vec<_> = hirs
        .iter()
        .map(crate::driver::pass_plan::build_for_hir)
        .collect();

    let mut all_tex_names: Vec<String> = Vec::new();
    {
        let mut seen = HashSet::new();
        for (hir, pass_plan) in hirs.iter().zip(pass_plans.iter()) {
            for name in &hir.textures {
                if seen.insert(name.clone()) {
                    all_tex_names.push(name.clone());
                }
            }
            for pass in &pass_plan.passes {
                if let Some(target_id) = pass.output_target {
                    let name = pass_target_texture_name(&hir.name, target_id);
                    if seen.insert(name.clone()) {
                        all_tex_names.push(name);
                    }
                }
            }
        }
        for material_hir in material_hirs {
            for name in &material_hir.textures {
                if seen.insert(name.clone()) {
                    all_tex_names.push(name.clone());
                }
            }
        }
    }

    let (sampler_global, tex_globals, tex_bindings) = if all_tex_names.is_empty() {
        (None, HashMap::new(), HashMap::new())
    } else {
        let sampler_ty = builder.module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Sampler { comparison: false },
            },
            naga::Span::UNDEFINED,
        );
        let sampler_global = builder.module.global_variables.append(
            naga::GlobalVariable {
                name: Some("fresco_sampler".to_string()),
                space: naga::AddressSpace::Handle,
                binding: Some(naga::ResourceBinding {
                    group: 1,
                    binding: 0,
                }),
                ty: sampler_ty,
                init: None,
                memory_decorations: naga::MemoryDecorations::empty(),
            },
            naga::Span::UNDEFINED,
        );

        let tex_ty = builder.module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Image {
                    dim: naga::ImageDimension::D2,
                    arrayed: false,
                    class: naga::ImageClass::Sampled {
                        kind: naga::ScalarKind::Float,
                        multi: false,
                    },
                },
            },
            naga::Span::UNDEFINED,
        );

        let mut tex_globals: HashMap<String, Handle<naga::GlobalVariable>> = HashMap::new();
        let mut tex_bindings: HashMap<String, u32> = HashMap::new();

        for (i, name) in all_tex_names.iter().enumerate() {
            let binding_idx = (i + 1) as u32;
            let global = builder.module.global_variables.append(
                naga::GlobalVariable {
                    name: Some(format!("t_{name}")),
                    space: naga::AddressSpace::Handle,
                    binding: Some(naga::ResourceBinding {
                        group: 1,
                        binding: binding_idx,
                    }),
                    ty: tex_ty,
                    init: None,
                    memory_decorations: naga::MemoryDecorations::empty(),
                },
                naga::Span::UNDEFINED,
            );
            tex_globals.insert(name.clone(), global);
            tex_bindings.insert(name.clone(), binding_idx);
        }

        (Some(sampler_global), tex_globals, tex_bindings)
    };

    let (global_uniform_globals, global_uniform_bindings) = ensure_global_uniform_buffers(
        &mut builder.module,
        &t,
        hirs.iter()
            .flat_map(|hir| hir.global_uniforms.iter())
            .chain(material_hirs.iter().flat_map(|m| m.global_uniforms.iter())),
    );

    let mut all_stats = Vec::new();
    let mut path_bindings: HashMap<String, u32> = HashMap::new();
    let mut param_bindings: HashMap<String, u32> = HashMap::new();
    let mut next_storage_binding: u32 = 0;

    let pass_target_bindings: HashMap<String, HashMap<usize, u32>> = hirs
        .iter()
        .zip(pass_plans.iter())
        .map(|(hir, pass_plan)| {
            let bindings = pass_plan
                .passes
                .iter()
                .filter_map(|pass| {
                    pass.output_target.map(|target_id| {
                        let tex_name = pass_target_texture_name(&hir.name, target_id);
                        let binding = tex_bindings.get(&tex_name).copied().unwrap_or(0);
                        (target_id, binding)
                    })
                })
                .collect();
            (hir.name.clone(), bindings)
        })
        .collect();

    for (canvas_idx, (hir, pass_plan)) in hirs.iter().zip(pass_plans.iter()).enumerate() {
        let mut t = t;
        if let Some(context) = &hir.entry_context {
            t.entry_context = Some(builder.register_context_type(&context.ty, &t).0);
        }
        let mut param_storage_globals: HashMap<String, Handle<naga::GlobalVariable>> =
            HashMap::new();

        for param in &hir.params {
            use crate::hir::{ArrayParamSize, parse_array_param_type_ex};

            if let Some((elem_ty_str, ArrayParamSize::Dynamic)) =
                parse_array_param_type_ex(&param.ty_name)
            {
                let elem_ty_handle = match elem_ty_str {
                    "f32" | "i32" | "u32" | "bool" => t.f32_,
                    "vec2" => t.v2,
                    "vec3" => t.v3,
                    "vec4" => t.v4,
                    _ => {
                        eprintln!(
                            "Warning: dynamic array element type `{elem_ty_str}` not yet supported for storage buffers"
                        );
                        continue;
                    }
                };

                let array_ty = builder.module.types.insert(
                    naga::Type {
                        name: Some(format!("FrescoParam_{}_{}", hir.name, param.name)),
                        inner: naga::TypeInner::Array {
                            base: elem_ty_handle,
                            size: naga::ArraySize::Dynamic,
                            stride: match elem_ty_str {
                                "f32" | "i32" | "u32" | "bool" => 4,
                                "vec2" => 8,
                                "vec3" => 16,
                                "vec4" => 16,
                                _ => 16,
                            },
                        },
                    },
                    naga::Span::UNDEFINED,
                );

                let global = builder.module.global_variables.append(
                    naga::GlobalVariable {
                        name: Some(format!("fresco_param_{}_{}", hir.name, param.name)),
                        space: naga::AddressSpace::Storage {
                            access: naga::StorageAccess::LOAD,
                        },
                        binding: Some(naga::ResourceBinding {
                            group: CANVAS_STORAGE_GROUP,
                            binding: next_storage_binding,
                        }),
                        ty: array_ty,
                        init: None,
                        memory_decorations: naga::MemoryDecorations::empty(),
                    },
                    naga::Span::UNDEFINED,
                );

                param_storage_globals.insert(param.name.clone(), global);
                param_bindings.insert(format!("{}:{}", hir.name, param.name), next_storage_binding);
                next_storage_binding = next_storage_binding
                    .checked_add(1)
                    .expect("storage bindings must fit u32");
            }
        }

        let mut path_tables: HashMap<usize, PathTableBinding> = HashMap::new();
        for (path_idx, profile) in hir.path_profiles.iter().enumerate() {
            if !profile.demand.needs_path_geometry() {
                continue;
            }
            let flat_segments = profile.flattened_rows();
            let Some(seg_count) = NonZeroU32::new(flat_segments.len() as u32) else {
                continue;
            };

            let array_ty = builder.module.types.insert(
                naga::Type {
                    name: Some(format!("FrescoPathSegArray_c{canvas_idx}_p{path_idx}")),
                    inner: naga::TypeInner::Array {
                        base: path_seg_ty,
                        size: naga::ArraySize::Constant(seg_count),
                        stride: fresco_artifact::PATH_SEGMENT_STRIDE,
                    },
                },
                naga::Span::UNDEFINED,
            );

            let mut seg_components = Vec::with_capacity(flat_segments.len());
            for seg in &flat_segments {
                let ax = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p0.0)),
                    naga::Span::UNDEFINED,
                );
                let ay = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p0.1)),
                    naga::Span::UNDEFINED,
                );
                let cx = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p1.0)),
                    naga::Span::UNDEFINED,
                );
                let cy = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p1.1)),
                    naga::Span::UNDEFINED,
                );
                let bx = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p2.0)),
                    naga::Span::UNDEFINED,
                );
                let by = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p2.1)),
                    naga::Span::UNDEFINED,
                );
                let dx = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p3.0)),
                    naga::Span::UNDEFINED,
                );
                let dy = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.p3.1)),
                    naga::Span::UNDEFINED,
                );
                let s0 = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.s0)),
                    naga::Span::UNDEFINED,
                );
                let len = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.len)),
                    naga::Span::UNDEFINED,
                );
                let kind = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::U32(seg.kind)),
                    naga::Span::UNDEFINED,
                );
                let mid_u = builder.module.global_expressions.append(
                    Ex::Literal(naga::Literal::F32(seg.mid_u)),
                    naga::Span::UNDEFINED,
                );
                let pad = builder
                    .module
                    .global_expressions
                    .append(Ex::Literal(naga::Literal::F32(0.0)), naga::Span::UNDEFINED);

                let a_vec = builder.module.global_expressions.append(
                    Ex::Compose {
                        ty: t.v2,
                        components: vec![ax, ay],
                    },
                    naga::Span::UNDEFINED,
                );
                let b_vec = builder.module.global_expressions.append(
                    Ex::Compose {
                        ty: t.v2,
                        components: vec![bx, by],
                    },
                    naga::Span::UNDEFINED,
                );
                let c_vec = builder.module.global_expressions.append(
                    Ex::Compose {
                        ty: t.v2,
                        components: vec![cx, cy],
                    },
                    naga::Span::UNDEFINED,
                );
                let d_vec = builder.module.global_expressions.append(
                    Ex::Compose {
                        ty: t.v2,
                        components: vec![dx, dy],
                    },
                    naga::Span::UNDEFINED,
                );
                let seg_expr = builder.module.global_expressions.append(
                    Ex::Compose {
                        ty: path_seg_ty,
                        components: vec![a_vec, c_vec, b_vec, d_vec, s0, len, kind, mid_u, pad],
                    },
                    naga::Span::UNDEFINED,
                );
                seg_components.push(seg_expr);
            }

            match profile.storage {
                crate::hir::PathStorageDecision::ConstModuleEmbedded => {
                    let array_init = builder.module.global_expressions.append(
                        Ex::Compose {
                            ty: array_ty,
                            components: seg_components,
                        },
                        naga::Span::UNDEFINED,
                    );

                    let constant = builder.module.constants.append(
                        naga::Constant {
                            name: Some(format!("fresco_path_c{canvas_idx}_p{path_idx}")),
                            ty: array_ty,
                            init: array_init,
                        },
                        naga::Span::UNDEFINED,
                    );
                    path_tables.insert(
                        path_idx,
                        PathTableBinding {
                            seg_count: seg_count.get(),
                            storage: PathTableStorage::Const { constant },
                            demand: profile.demand,
                        },
                    );
                }
                crate::hir::PathStorageDecision::BufferExceedsConstThreshold => {
                    let global = builder.module.global_variables.append(
                        naga::GlobalVariable {
                            name: Some(format!("fresco_path_buf_c{canvas_idx}_p{path_idx}")),
                            space: naga::AddressSpace::Storage {
                                access: naga::StorageAccess::LOAD,
                            },
                            binding: Some(naga::ResourceBinding {
                                group: CANVAS_STORAGE_GROUP,
                                binding: next_storage_binding,
                            }),
                            ty: array_ty,
                            init: None,
                            memory_decorations: naga::MemoryDecorations::empty(),
                        },
                        naga::Span::UNDEFINED,
                    );
                    path_tables.insert(
                        path_idx,
                        PathTableBinding {
                            seg_count: seg_count.get(),
                            storage: PathTableStorage::Buffer { global },
                            demand: profile.demand,
                        },
                    );
                    path_bindings
                        .insert(format!("{}:{}", hir.name, path_idx), next_storage_binding);
                    next_storage_binding = next_storage_binding
                        .checked_add(1)
                        .expect("storage bindings must fit u32");
                }
            }
        }

        let mut path_helper_fns: HashMap<usize, PathHelperFns> = HashMap::new();
        for (&path_idx, table) in &path_tables {
            let mut helper_set = PathHelperFns::default();
            if table.demand.needs_nearest_sample() {
                let f = lower_path_nearest_fn(&hir.name, path_idx, table, &t, policy);
                helper_set.nearest_sample = Some(builder.push_function_dedup(f));
            }
            if table.demand.needs_arc_sample() {
                let f = lower_path_arc_sample_fn(&hir.name, path_idx, table, &t, policy);
                helper_set.arc_sample = Some(builder.push_function_dedup(f));
            }
            path_helper_fns.insert(path_idx, helper_set);
        }

        let mut user_helper_fns: HashMap<String, Handle<naga::Function>> = HashMap::new();
        let mut pending: BTreeMap<String, &UserFnHelper> = hir
            .user_helpers
            .iter()
            .map(|(id, helper)| (id.clone(), helper))
            .collect();

        while !pending.is_empty() {
            let ids: Vec<String> = pending.keys().cloned().collect();
            let mut progressed = false;

            for id in ids {
                let helper = *pending
                    .get(&id)
                    .expect("pending helper id vanished during lowering");

                let mut deps = BTreeSet::new();
                for stmt in &helper.body_stmts {
                    collect_helper_ids_from_stmt(stmt, &mut deps);
                }

                let ready = deps
                    .iter()
                    .all(|dep| dep == &id || !pending.contains_key(dep));
                if !ready {
                    continue;
                }

                let f = lower_user_helper_fn(
                    hir,
                    helper,
                    &t,
                    policy,
                    user_helper_fns.clone(),
                    path_helper_fns.clone(),
                    &global_uniform_globals,
                );
                let handle = builder.push_function_dedup(f);
                user_helper_fns.insert(id.clone(), handle);
                pending.remove(&id);
                progressed = true;
            }

            if !progressed {
                let id = pending
                    .keys()
                    .next()
                    .cloned()
                    .expect("pending helper set unexpectedly empty");
                let helper = pending
                    .remove(&id)
                    .expect("pending helper id vanished during cycle fallback");
                let f = lower_user_helper_fn(
                    hir,
                    helper,
                    &t,
                    policy,
                    user_helper_fns.clone(),
                    path_helper_fns.clone(),
                    &global_uniform_globals,
                );
                let handle = builder.push_function_dedup(f);
                user_helper_fns.insert(id, handle);
            }
        }

        let scatter_body_fns: HashMap<LayerId, Handle<naga::Function>> = hir
            .layers
            .iter()
            .enumerate()
            .filter_map(|(id, layer)| {
                if let Layer::ScatterBins { body, .. } = layer {
                    Some((id, *body))
                } else {
                    None
                }
            })
            .map(|(scatter_id, body_id)| {
                let f = lower_scatter_body_fn(
                    hir,
                    scatter_id,
                    body_id,
                    &t,
                    policy,
                    user_helper_fns.clone(),
                    path_helper_fns.clone(),
                    &tex_globals,
                    sampler_global,
                    &global_uniform_globals,
                );
                let handle = builder.push_function_dedup(f);
                (scatter_id, handle)
            })
            .collect();

        let scene_root = effective_render_root(hir);
        let root_pass_id = pass_plan.layer_to_pass[scene_root];

        let (scene_fn, stats) = lower_scene_fn(
            hir,
            &t,
            policy,
            scatter_body_fns.clone(),
            user_helper_fns.clone(),
            path_helper_fns.clone(),
            &tex_globals,
            sampler_global,
            &param_storage_globals,
            &global_uniform_globals,
        );
        let base_scene_key = scene_function_dedup_key(&scene_fn);
        let base_scene_handle = builder.push_function(scene_fn);
        let mut scene_helper_cache = vec![(base_scene_key, base_scene_handle)];
        let entry_fn = lower_canvas_entry_fn(
            hir,
            &t,
            policy,
            &tex_globals,
            sampler_global,
            base_scene_handle,
        );
        builder.push_function(entry_fn);

        let pass_output_target_names: HashMap<usize, String> = pass_plan
            .passes
            .iter()
            .filter_map(|pass| {
                pass.output_target
                    .map(|target_id| (pass.id, pass_target_texture_name(&hir.name, target_id)))
            })
            .collect();
        let omit_single_pass_wrapper = pass_plan.passes.len() == 1
            && matches!(
                pass_plan.passes[0].kernel_strategy,
                crate::driver::pass_plan::KernelStrategy::Fused
            );

        for pass in &pass_plan.passes {
            if omit_single_pass_wrapper {
                continue;
            }
            let scene_handle = if pass.id == root_pass_id
                && pass.render_root == scene_root
                && subgraph_stays_in_pass(hir, scene_root, pass.id, &pass_plan.layer_to_pass)
            {
                base_scene_handle
            } else {
                let scene_fn = lower_scene_pass_fn(
                    hir,
                    pass.id,
                    pass.render_root,
                    pass_plan.layer_to_pass.clone(),
                    pass_output_target_names.clone(),
                    &t,
                    policy,
                    scatter_body_fns.clone(),
                    user_helper_fns.clone(),
                    path_helper_fns.clone(),
                    &tex_globals,
                    sampler_global,
                    &param_storage_globals,
                    &global_uniform_globals,
                );
                let scene_key = scene_function_dedup_key(&scene_fn);
                if let Some((_, handle)) = scene_helper_cache
                    .iter()
                    .find(|(existing_key, _)| *existing_key == scene_key)
                {
                    *handle
                } else {
                    let handle = builder.push_function(scene_fn);
                    scene_helper_cache.push((scene_key, handle));
                    handle
                }
            };
            let entry_fn = lower_canvas_pass_entry_fn(
                hir,
                pass.id,
                &t,
                policy,
                &tex_globals,
                sampler_global,
                scene_handle,
            );
            builder.push_function(entry_fn);
        }
        all_stats.push(stats);
    }
    all_stats.extend(surface::lower_surfaces_with_policy(
        material_hirs,
        &mut builder.module,
        &t,
        &tex_bindings,
        &mut param_bindings,
        policy,
        &global_uniform_globals,
    ));
    LoweringResult {
        module: builder.finish(),
        stats: all_stats,
        tex_bindings,
        pass_target_bindings,
        path_bindings,
        param_bindings,
        global_uniform_bindings,
    }
}

fn scalar_type_handle(kind: crate::typed_scalar::Kind, types: &TypeHandles) -> Handle<naga::Type> {
    use crate::typed_scalar::Kind;
    match kind {
        Kind::F32 => types.f32_,
        Kind::I32 => types.i32_,
        Kind::U32 => types.u32_,
        Kind::Bool => types.bool_,
    }
}

fn vector_type_handle(
    kind: crate::typed_scalar::Kind,
    width: usize,
    types: &TypeHandles,
) -> Handle<naga::Type> {
    use crate::typed_scalar::Kind;
    let index = match kind {
        Kind::F32 => 0,
        Kind::I32 => 1,
        Kind::U32 => 2,
        Kind::Bool => 3,
    };
    types.native_vectors[index][width - 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser as _;
    use logos::Logos as _;

    #[test]
    fn scene_function_dedup_key_ignores_helper_names_for_equivalent_pass_lowering() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(#0e1420)
        circle(at: (0.5, 0.5), radius: 0.2) |> fill(#ffffff)
    }
}
"#;

        let mut tokens = Vec::new();
        for (tok, span) in crate::lexer::Token::lexer(src).spanned() {
            match tok {
                Ok(token) => tokens.push((token, span)),
                Err(()) => panic!("expected lex success"),
            }
        }
        let eoi = src.len()..src.len();
        let (program, parse_errs) = crate::parser::program()
            .parse(crate::parser::input(&tokens, eoi))
            .into_output_errors();
        assert!(
            parse_errs.is_empty(),
            "expected parse success: {parse_errs:#?}"
        );
        let program = program.unwrap_or_default();
        let canvas = &program.canvases[0];
        let (hir, diags) = crate::check::check(
            &canvas.as_normalized_root_entry(),
            &program.functions,
            &program.consts,
            &program.enums,
            &program.structs,
            &program.params,
            &program.texture_types,
            &program.interfaces,
            &program.conformances,
            &program.effects,
            &program.vertex_interfaces,
            &program.vertex_formats,
            &program.vertex_factories,
            &crate::check::CheckOptions {
                shape_aa_min_px: Some(1.5),
                shape_aa_max_px: Some(3.0),
                shape_aa_style: Some(crate::hir::ShapeAaStyle::Gradient),
                projective_footprint_max_px: Some(64.0),
                ..crate::check::CheckOptions::default()
            },
        )
        .unwrap_or_else(|diags| panic!("expected check success, got diagnostics: {diags:#?}"));
        assert!(
            diags.is_empty(),
            "expected no non-error diagnostics: {diags:#?}"
        );

        let scene_root = effective_render_root(&hir);
        let layer_to_pass = vec![0usize; hir.layers.len()];

        let mut builder = ModuleBuilder::new();
        let t = builder.register_core_types();
        let tex_globals = HashMap::new();
        let pass_output_target_names = HashMap::new();
        let scatter_body_fns = HashMap::new();
        let user_helper_fns = HashMap::new();
        let path_helper_fns = HashMap::new();
        let param_storage_globals = HashMap::new();

        let scene_pass0 = lower_scene_pass_fn(
            &hir,
            0,
            scene_root,
            layer_to_pass.clone(),
            pass_output_target_names.clone(),
            &t,
            LoweringPolicy::Readable,
            scatter_body_fns.clone(),
            user_helper_fns.clone(),
            path_helper_fns.clone(),
            &tex_globals,
            None,
            &param_storage_globals,
            &HashMap::new(),
        );
        let scene_pass7 = lower_scene_pass_fn(
            &hir,
            7,
            scene_root,
            layer_to_pass,
            pass_output_target_names,
            &t,
            LoweringPolicy::Readable,
            scatter_body_fns,
            user_helper_fns,
            path_helper_fns,
            &tex_globals,
            None,
            &param_storage_globals,
            &HashMap::new(),
        );

        assert_eq!(
            scene_function_dedup_key(&scene_pass0),
            scene_function_dedup_key(&scene_pass7),
            "expected equivalent pass helpers to share the same dedup key despite different helper names"
        );
    }

    #[test]
    fn scene_function_dedup_key_ignores_argument_local_and_named_expression_labels() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(#0e1420)
        circle(at: (0.5, 0.5), radius: 0.2) |> fill(#ffffff)
    }
}
"#;

        let mut tokens = Vec::new();
        for (tok, span) in crate::lexer::Token::lexer(src).spanned() {
            match tok {
                Ok(token) => tokens.push((token, span)),
                Err(()) => panic!("expected lex success"),
            }
        }
        let eoi = src.len()..src.len();
        let (program, parse_errs) = crate::parser::program()
            .parse(crate::parser::input(&tokens, eoi))
            .into_output_errors();
        assert!(
            parse_errs.is_empty(),
            "expected parse success: {parse_errs:#?}"
        );
        let program = program.unwrap_or_default();
        let canvas = &program.canvases[0];
        let (hir, diags) = crate::check::check(
            &canvas.as_normalized_root_entry(),
            &program.functions,
            &program.consts,
            &program.enums,
            &program.structs,
            &program.params,
            &program.texture_types,
            &program.interfaces,
            &program.conformances,
            &program.effects,
            &program.vertex_interfaces,
            &program.vertex_formats,
            &program.vertex_factories,
            &crate::check::CheckOptions {
                shape_aa_min_px: Some(1.5),
                shape_aa_max_px: Some(3.0),
                shape_aa_style: Some(crate::hir::ShapeAaStyle::Gradient),
                projective_footprint_max_px: Some(64.0),
                ..crate::check::CheckOptions::default()
            },
        )
        .unwrap_or_else(|diags| panic!("expected check success, got diagnostics: {diags:#?}"));
        assert!(
            diags.is_empty(),
            "expected no non-error diagnostics: {diags:#?}"
        );

        let scene_root = effective_render_root(&hir);
        let mut builder = ModuleBuilder::new();
        let t = builder.register_core_types();
        let original = lower_scene_pass_fn(
            &hir,
            0,
            scene_root,
            vec![0usize; hir.layers.len()],
            HashMap::new(),
            &t,
            LoweringPolicy::Readable,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            &HashMap::new(),
            None,
            &HashMap::new(),
            &HashMap::new(),
        );

        let mut renamed = original.clone();
        renamed.name = Some("different_scene_name".to_string());
        for (index, arg) in renamed.arguments.iter_mut().enumerate() {
            arg.name = Some(format!("arg_{index}"));
        }
        if let Some(result) = &mut renamed.result {
            result.binding = Some(naga::Binding::BuiltIn(naga::BuiltIn::Position {
                invariant: false,
            }));
        }
        for (index, (_, local)) in renamed.local_variables.iter_mut().enumerate() {
            local.name = Some(format!("local_{index}"));
        }
        renamed.named_expressions = renamed
            .expressions
            .iter()
            .map(|(handle, _)| (handle, "renamed".to_string()))
            .collect();

        assert_eq!(
            scene_function_dedup_key(&original),
            scene_function_dedup_key(&renamed),
            "expected helper dedup keys to ignore non-semantic helper naming metadata"
        );
    }
}
