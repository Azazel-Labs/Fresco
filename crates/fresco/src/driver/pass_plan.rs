use crate::hir::{EffectDef, Hir, Layer, Locality, Sx};
use std::collections::{BTreeMap, BTreeSet};

fn effective_render_root(hir: &Hir) -> usize {
    match &hir.layers[hir.root] {
        Layer::MotionBlur { inner, .. } => *inner,
        _ => hir.root,
    }
}

fn is_synthetic_root_motion_blur(hir: &Hir, layer_id: usize) -> bool {
    layer_id == hir.root && matches!(hir.layers[hir.root], Layer::MotionBlur { .. })
}

/// Radius threshold below which a local effect is inlined as an N-tap loop
/// with no pass cut (§11 step 3 rung 1).
pub const INLINE_TAPS_THRESHOLD_PX: f32 = 4.0;

/// Radius threshold separating separable H+V passes from a downsample chain
/// (§11 step 3 rung 2 vs rung 3).
pub const SEPARABLE_HV_THRESHOLD_PX: f32 = 24.0;

/// Number of taps on each axis for the inline-taps rung (produces a
/// `(2*HALF_TAPS+1)²` grid filter).
pub const INLINE_HALF_TAPS: u32 = 2;

/// How many mip levels to use when `r > SEPARABLE_HV_THRESHOLD_PX` and no
/// explicit override is provided.
fn downsample_levels(radius_px: f32) -> u32 {
    ((radius_px / 16.0).ceil() as u32).max(2)
}

fn estimate_user_effect_radius_px(effect: &EffectDef, args: &[Sx]) -> Option<f32> {
    let radius = effect.locality_radius.clone()?;
    let vars = effect
        .param_names
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect::<std::collections::HashMap<_, _>>();
    let radius = radius.subst_vars(&vars);
    estimate_px_radius(&radius)
}

/// Kernel strategy selected by the §11 step-3 ladder for a pass.
#[derive(Debug, Clone, Copy)]
pub enum KernelStrategy {
    /// Point-locality: fused into any consuming pass, no cut needed.
    Fused,
    /// Local, estimated radius ≤ [`INLINE_TAPS_THRESHOLD_PX`]: inline
    /// `(2*half_taps+1)²`-sample box filter inside the consuming pass; the
    /// locality mark is retracted and no intermediate target is allocated.
    InlineTaps { radius_px: f32, half_taps: u32 },
    /// Local, `INLINE_TAPS_THRESHOLD_PX < r ≤ SEPARABLE_HV_THRESHOLD_PX`:
    /// separable horizontal + vertical passes at full resolution.
    SeparableHV { radius_px: f32 },
    /// Local, `r > SEPARABLE_HV_THRESHOLD_PX`: mip-ping-pong downsample chain
    /// (bloom topology — downsample, blur-while-resampling, upsample-accumulate).
    DownsampleChain { radius_px: f32, levels: u32 },
    /// Global: reduction tree / compute dispatch.
    GlobalReduction,
}

impl KernelStrategy {
    /// Serialisable label for manifest and explain output.
    pub fn label(self) -> &'static str {
        match self {
            KernelStrategy::Fused => "fused",
            KernelStrategy::InlineTaps { .. } => "inline-taps",
            KernelStrategy::SeparableHV { .. } => "separable-hv",
            KernelStrategy::DownsampleChain { .. } => "downsample-chain",
            KernelStrategy::GlobalReduction => "global-reduction",
        }
    }
}

/// Lifetime of an intermediate render target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "Frame lifetime reserved for feedback / ping-pong targets (§11 step 4)"
)]
pub enum TargetLifetime {
    /// Target is read exactly once by the immediately following pass, then
    /// released; a transient allocator can reuse it within the frame.
    Transient,
    /// Target survives for the whole frame (e.g. feedback ping-pong).
    Frame,
}

impl TargetLifetime {
    pub fn label(self) -> &'static str {
        match self {
            TargetLifetime::Transient => "transient",
            TargetLifetime::Frame => "frame",
        }
    }
}

/// An intermediate render target allocated between two passes.
#[derive(Debug, Clone)]
pub struct IntermediateTarget {
    pub id: usize,
    /// Image-format registry name (e.g. `"rgba16float"`).
    pub format: &'static str,
    /// Resolution scale relative to the canvas output (1.0 = full res).
    pub scale: f32,
    pub lifetime: TargetLifetime,
}

#[derive(Debug, Clone)]
pub struct PassPlan {
    pub id: usize,
    pub stage: usize,
    pub locality: Locality,
    pub start_layer: usize,
    pub end_layer: usize,
    pub render_root: usize,
    pub count: usize,
    /// Kernel execution strategy selected by the §11 ladder.
    pub kernel_strategy: KernelStrategy,
    /// Intermediate target written by this pass; `None` means the pass writes
    /// to the final swapchain / framebuffer output.
    pub output_target: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PassBuild {
    pub passes: Vec<PassPlan>,
    pub edges: Vec<(usize, usize, &'static str)>,
    /// Intermediate targets allocated between passes.
    pub targets: Vec<IntermediateTarget>,
    pub layer_to_pass: Vec<usize>,
}

pub fn build_for_hir(hir: &Hir) -> PassBuild {
    if hir.layer_locality.is_empty() {
        return PassBuild {
            passes: vec![PassPlan {
                id: 0,
                stage: 0,
                locality: Locality::Point,
                start_layer: 0,
                end_layer: 0,
                render_root: 0,
                count: 0,
                kernel_strategy: KernelStrategy::Fused,
                output_target: None,
            }],
            edges: Vec::new(),
            targets: Vec::new(),
            layer_to_pass: vec![0],
        };
    }

    let reachable = reachable_layers_from_root(hir);

    let mut layer_stage = vec![0usize; hir.layer_locality.len()];
    for (idx, layer) in hir.layers.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        let mut stage = 0usize;
        for dep in layer_inputs(layer) {
            if !reachable[dep] {
                continue;
            }
            let dep_stage = layer_stage[dep];
            let dep_locality = hir.layer_locality[dep];
            let this_locality = hir.layer_locality[idx];
            let needed = dep_stage + usize::from(dep_locality != this_locality);
            stage = stage.max(needed);
        }
        layer_stage[idx] = stage;
    }

    let mut key_to_id = BTreeMap::<(usize, u8), usize>::new();
    let mut passes = Vec::<PassPlan>::new();
    let mut layer_to_pass = vec![usize::MAX; hir.layer_locality.len()];

    for idx in 0..hir.layer_locality.len() {
        if !reachable[idx] {
            continue;
        }
        if is_synthetic_root_motion_blur(hir, idx) {
            continue;
        }
        let locality = hir.layer_locality[idx];
        let stage = layer_stage[idx];
        let key = (stage, locality_order(locality));
        let pass_id = if let Some(id) = key_to_id.get(&key) {
            *id
        } else {
            let id = passes.len();
            key_to_id.insert(key, id);
            passes.push(PassPlan {
                id,
                stage,
                locality,
                start_layer: idx,
                end_layer: idx,
                render_root: idx,
                count: 0,
                kernel_strategy: KernelStrategy::Fused,
                output_target: None,
            });
            id
        };
        layer_to_pass[idx] = pass_id;
        let p = &mut passes[pass_id];
        p.start_layer = p.start_layer.min(idx);
        p.end_layer = p.end_layer.max(idx);
        p.count += 1;
    }

    let mut same_pass_consumer = vec![false; hir.layers.len()];
    for (idx, layer) in hir.layers.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        let pass_id = layer_to_pass[idx];
        if pass_id == usize::MAX {
            continue;
        }
        for dep in layer_inputs(layer) {
            if !reachable[dep] {
                continue;
            }
            if layer_to_pass[dep] == pass_id {
                same_pass_consumer[dep] = true;
            }
        }
    }

    for (idx, pass_id) in layer_to_pass.iter().copied().enumerate() {
        if !reachable[idx] || pass_id == usize::MAX {
            continue;
        }
        if !same_pass_consumer[idx] {
            passes[pass_id].render_root = idx;
        }
    }

    // --- Step 3: kernel strategy selection (§11) ---
    // Find the maximum blur radius (in px) for each pass, then apply the
    // documented threshold ladder.
    let mut pass_max_radius_px: Vec<Option<f32>> = vec![None; passes.len()];
    for (idx, layer) in hir.layers.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        let r = match layer {
            Layer::Blur { radius, .. } => estimate_px_radius(radius),
            Layer::MotionBlur { offset, .. } => {
                let rx = estimate_px_radius(&offset.0);
                let ry = estimate_px_radius(&offset.1);
                match (rx, ry) {
                    (Some(x), Some(y)) => Some(x.max(y)),
                    (Some(x), None) => Some(x),
                    (None, Some(y)) => Some(y),
                    (None, None) => None,
                }
            }
            Layer::UserEffect { def_idx, args, .. } => hir
                .effects
                .get(*def_idx)
                .filter(|effect| effect.locality == Locality::Local)
                .and_then(|effect| estimate_user_effect_radius_px(effect, args)),
            _ => None,
        };
        let contributes_local_radius = matches!(
            layer,
            Layer::Blur { .. } | Layer::MotionBlur { .. }
        ) || matches!(layer, Layer::UserEffect { def_idx, .. } if hir.effects.get(*def_idx).is_some_and(|effect| effect.locality == Locality::Local));
        if contributes_local_radius {
            let pass_id = layer_to_pass[idx];
            if pass_id == usize::MAX {
                continue;
            }
            let slot = &mut pass_max_radius_px[pass_id];
            *slot = Some(match (*slot, r) {
                (Some(cur), Some(new)) => cur.max(new),
                (Some(cur), None) => cur,
                (None, Some(new)) => new,
                (None, None) => 0.0,
            });
        }
    }

    for (i, pass) in passes.iter_mut().enumerate() {
        pass.kernel_strategy = match pass.locality {
            Locality::Point => KernelStrategy::Fused,
            Locality::Local => {
                let r = pass_max_radius_px[i].unwrap_or(0.0);
                if r <= INLINE_TAPS_THRESHOLD_PX {
                    KernelStrategy::InlineTaps {
                        radius_px: r,
                        half_taps: INLINE_HALF_TAPS,
                    }
                } else if r <= SEPARABLE_HV_THRESHOLD_PX {
                    KernelStrategy::SeparableHV { radius_px: r }
                } else {
                    KernelStrategy::DownsampleChain {
                        radius_px: r,
                        levels: downsample_levels(r),
                    }
                }
            }
            Locality::Global => KernelStrategy::GlobalReduction,
        };
    }

    // --- Intermediate target allocation ---
    // Every pass that is not the root-layer pass writes to an intermediate
    // target so downstream passes can sample it.  Inline-tap passes are
    // folded into their consumer (mark retracted); they still get a target id
    // in the plan to keep indices stable, but the runtime shim may skip them.
    let root_pass = layer_to_pass[effective_render_root(hir)];
    let mut targets: Vec<IntermediateTarget> = Vec::new();

    for pass in passes.iter_mut() {
        if pass.id == root_pass {
            // Final output → swapchain.
            pass.output_target = None;
        } else {
            let tid = targets.len();
            targets.push(IntermediateTarget {
                id: tid,
                format: "rgba16float",
                scale: 1.0,
                lifetime: TargetLifetime::Transient,
            });
            pass.output_target = Some(tid);
        }
    }

    let edges = build_pass_edges(hir, &layer_to_pass, &reachable);
    PassBuild {
        passes,
        edges,
        targets,
        layer_to_pass,
    }
}

pub fn locality_counts(hir: &Hir) -> (usize, usize, usize) {
    let mut point = 0usize;
    let mut local = 0usize;
    let mut global = 0usize;
    for (idx, mark) in hir.layer_locality.iter().enumerate() {
        if is_synthetic_root_motion_blur(hir, idx) {
            continue;
        }
        match mark {
            Locality::Point => point += 1,
            Locality::Local => local += 1,
            Locality::Global => global += 1,
        }
    }
    (point, local, global)
}

pub fn estimated_passes(point_count: usize, local_count: usize, global_count: usize) -> usize {
    let mut passes = 0usize;
    if point_count > 0 {
        passes += 1;
    }
    if local_count > 0 {
        passes += 1;
    }
    if global_count > 0 {
        passes += 1;
    }
    passes.max(1)
}

pub fn locality_label(locality: Locality) -> &'static str {
    match locality {
        Locality::Point => "point",
        Locality::Local => "local",
        Locality::Global => "global",
    }
}

fn build_pass_edges(
    hir: &Hir,
    layer_to_pass: &[usize],
    reachable: &[bool],
) -> Vec<(usize, usize, &'static str)> {
    let mut seen = BTreeSet::<(usize, usize, &'static str)>::new();
    for (idx, layer) in hir.layers.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        if is_synthetic_root_motion_blur(hir, idx) {
            continue;
        }
        let to_pass = layer_to_pass[idx];
        if to_pass == usize::MAX {
            continue;
        }
        for dep in layer_inputs(layer) {
            if !reachable[dep] {
                continue;
            }
            if is_synthetic_root_motion_blur(hir, dep) {
                continue;
            }
            let from_pass = layer_to_pass[dep];
            if from_pass == usize::MAX {
                continue;
            }
            if from_pass == to_pass {
                continue;
            }
            let reason = if hir.layer_locality[dep] != hir.layer_locality[idx] {
                "locality-boundary"
            } else {
                "dependency-order"
            };
            seen.insert((from_pass, to_pass, reason));
        }
    }
    seen.into_iter().collect()
}

fn reachable_layers_from_root(hir: &Hir) -> Vec<bool> {
    let mut reachable = vec![false; hir.layers.len()];
    if hir.layers.is_empty() {
        return reachable;
    }

    let root = effective_render_root(hir);
    if root >= hir.layers.len() {
        return reachable;
    }

    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if id >= hir.layers.len() || reachable[id] {
            continue;
        }
        reachable[id] = true;
        for dep in layer_inputs(&hir.layers[id]) {
            stack.push(dep);
        }
    }

    reachable
}

fn layer_inputs(layer: &Layer) -> Vec<usize> {
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

fn locality_order(locality: Locality) -> u8 {
    match locality {
        Locality::Point => 0,
        Locality::Local => 1,
        Locality::Global => 2,
    }
}

/// Estimate a blur radius in physical pixels from a scalar HIR expression.
///
/// - `Sx::PxLit(r)` → `r` px exactly.
/// - `Sx::Lit(r)` → `r * 1080.0` (UV-space literal at a canonical 1080 p
///   height; used only for strategy selection, not for correctness-critical
///   math).
/// - Everything else → `None` (caller falls back to the inline-taps rung).
pub fn estimate_px_radius(sx: &Sx) -> Option<f32> {
    match sx {
        Sx::PxLit(r) => Some(*r),
        Sx::Lit(r) => Some(r * 1080.0),
        _ => None,
    }
}
