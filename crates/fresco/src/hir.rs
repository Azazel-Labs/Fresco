//! Typed HIR: the layer / shape DAG described in the design doc (§3, §10).
//!
//! Shapes and layers live in flat arenas (`Vec` + index ids) so the rewrite
//! pass can edit nodes in place and the lowering pass can memoize SDF
//! evaluations per (shape, coordinate) pair — the "shape CSE" of §10.3.
//!
//! Unit resolution happens on the way *into* HIR:
//!   `deg` literals are folded to radians,
//!   `px`  literals become `Sx::PxLit`, which lowering multiplies by the
//!         runtime `px = 1.0 / res.y` value (§7: pixel-true at any size),
//!   `uv` / unitless / `s` literals are plain floats.
//!
//! ## Pattern Filtering
//!
//! The `FilteringState` controls whether procedural patterns (checker, stripe)
//! use analytic prefiltering to eliminate aliasing. The state can be set via
//! the `|> filtering(on/off)` effect and defaults to `Auto`, which enables
//! filtering when a footprint is available (via canvas_space Jacobian).

use crate::ast::Span;
use std::collections::HashMap;
use std::rc::Rc;

pub type ShapeId = usize;
pub type LayerId = usize;
pub type Color = [f32; 4];
pub type ColorExpr = [Sx; 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStorageDecision {
    ConstModuleEmbedded,
    BufferExceedsConstThreshold,
}

#[derive(Debug, Clone, Copy)]
#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
pub struct PathSegmentRow {
    pub p0: (f32, f32),
    pub p1: (f32, f32),
    pub p2: (f32, f32),
    pub p3: (f32, f32),
    pub s0: f32,
    pub len: f32,
    pub kind: u32,
    pub mid_u: f32,
}

pub const PATH_SEG_KIND_QUADRATIC: u32 = 0;
pub const PATH_SEG_KIND_CUBIC: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub enum PathPrimitive {
    Line {
        from: (f32, f32),
        to: (f32, f32),
    },
    Quadratic {
        p0: (f32, f32),
        p1: (f32, f32),
        p2: (f32, f32),
    },
    Cubic {
        p0: (f32, f32),
        p1: (f32, f32),
        p2: (f32, f32),
        p3: (f32, f32),
    },
    Arc {
        from: (f32, f32),
        center: (f32, f32),
        radius: f32,
        sweep: f32,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum CubicSamplingPolicy {
    FixedSamples(usize),
    Adaptive { tolerance: f32, max_depth: usize },
}

#[derive(Debug, Clone, Copy)]
pub struct PathSamplingPolicy {
    pub cubic: CubicSamplingPolicy,
    pub arc_samples_per_radius_radian: f32,
    pub arc_min_samples: usize,
    pub preserve_cubics: bool,
}

impl Default for PathSamplingPolicy {
    fn default() -> Self {
        Self {
            cubic: CubicSamplingPolicy::FixedSamples(48),
            arc_samples_per_radius_radian: 24.0,
            arc_min_samples: 16,
            preserve_cubics: false,
        }
    }
}

fn point_line_distance(point: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let abx = b.0 - a.0;
    let aby = b.1 - a.1;
    let len2 = abx * abx + aby * aby;
    if len2 <= f32::EPSILON {
        let dx = point.0 - a.0;
        let dy = point.1 - a.1;
        return (dx * dx + dy * dy).sqrt();
    }
    let apx = point.0 - a.0;
    let apy = point.1 - a.1;
    let cross = (apx * aby - apy * abx).abs();
    cross / len2.sqrt()
}

fn lerp2(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

#[expect(
    clippy::type_complexity,
    reason = "The tuple represents the fixed components of a compiler or geometry operation."
)]
fn split_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    t: f32,
) -> (
    ((f32, f32), (f32, f32), (f32, f32), (f32, f32)),
    ((f32, f32), (f32, f32), (f32, f32), (f32, f32)),
) {
    let p01 = lerp2(p0, p1, t);
    let p12 = lerp2(p1, p2, t);
    let p23 = lerp2(p2, p3, t);
    let p012 = lerp2(p01, p12, t);
    let p123 = lerp2(p12, p23, t);
    let p0123 = lerp2(p012, p123, t);
    ((p0, p01, p012, p0123), (p0123, p123, p23, p3))
}

fn cubic_to_quadratic_fit(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
) -> ((f32, f32), (f32, f32), (f32, f32)) {
    let q1 = (
        (3.0 * (p1.0 + p2.0) - p0.0 - p3.0) * 0.25,
        (3.0 * (p1.1 + p2.1) - p0.1 - p3.1) * 0.25,
    );
    (p0, q1, p3)
}

fn eval_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let omt = 1.0 - t;
    let omt2 = omt * omt;
    let t2 = t * t;
    (
        omt2 * omt * p0.0 + 3.0 * omt2 * t * p1.0 + 3.0 * omt * t2 * p2.0 + t2 * t * p3.0,
        omt2 * omt * p0.1 + 3.0 * omt2 * t * p1.1 + 3.0 * omt * t2 * p2.1 + t2 * t * p3.1,
    )
}

fn eval_quadratic(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), t: f32) -> (f32, f32) {
    let omt = 1.0 - t;
    let omt2 = omt * omt;
    let t2 = t * t;
    (
        omt2 * p0.0 + 2.0 * omt * t * p1.0 + t2 * p2.0,
        omt2 * p0.1 + 2.0 * omt * t * p1.1 + t2 * p2.1,
    )
}

fn gauss8_integrate_unit_interval<F>(t0: f32, t1: f32, speed: F) -> f32
where
    F: Fn(f32) -> f32,
{
    const NODES: [f32; 8] = [
        -0.960_289_84,
        -0.796_666_5,
        -0.525_532_4,
        -0.183_434_64,
        0.183_434_64,
        0.525_532_4,
        0.796_666_5,
        0.960_289_84,
    ];
    const WEIGHTS: [f32; 8] = [
        0.101_228_54,
        0.222_381_04,
        0.313_706_64,
        0.362_683_77,
        0.362_683_77,
        0.313_706_64,
        0.222_381_04,
        0.101_228_54,
    ];

    let half = 0.5 * (t1 - t0);
    let center = f32::midpoint(t1, t0);
    let mut integral = 0.0_f32;
    for i in 0..8 {
        let t = center + half * NODES[i];
        integral += WEIGHTS[i] * speed(t);
    }
    half * integral
}

fn quadratic_arc_length_range(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    t0: f32,
    t1: f32,
) -> f32 {
    let ax = p2.0 - 2.0 * p1.0 + p0.0;
    let ay = p2.1 - 2.0 * p1.1 + p0.1;
    let bx = p1.0 - p0.0;
    let by = p1.1 - p0.1;

    gauss8_integrate_unit_interval(t0, t1, |t| {
        let dx = ax * t + bx;
        let dy = ay * t + by;
        2.0 * (dx * dx + dy * dy).sqrt()
    })
}

fn quadratic_arc_length(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)) -> f32 {
    let len = quadratic_arc_length_range(p0, p1, p2, 0.0, 1.0);

    let chord_dx = p2.0 - p0.0;
    let chord_dy = p2.1 - p0.1;
    let chord = (chord_dx * chord_dx + chord_dy * chord_dy).sqrt();
    len.max(chord)
}

fn cubic_arc_length_range(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    t0: f32,
    t1: f32,
) -> f32 {
    let ax = -p0.0 + 3.0 * p1.0 - 3.0 * p2.0 + p3.0;
    let ay = -p0.1 + 3.0 * p1.1 - 3.0 * p2.1 + p3.1;
    let bx = 3.0 * p0.0 - 6.0 * p1.0 + 3.0 * p2.0;
    let by = 3.0 * p0.1 - 6.0 * p1.1 + 3.0 * p2.1;
    let cx = -3.0 * p0.0 + 3.0 * p1.0;
    let cy = -3.0 * p0.1 + 3.0 * p1.1;

    gauss8_integrate_unit_interval(t0, t1, |t| {
        let qdx = (3.0 * ax * t + 2.0 * bx) * t + cx;
        let qdy = (3.0 * ay * t + 2.0 * by) * t + cy;
        (qdx * qdx + qdy * qdy).sqrt()
    })
}

fn cubic_arc_length(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32)) -> f32 {
    let len = cubic_arc_length_range(p0, p1, p2, p3, 0.0, 1.0);

    let chord_dx = p3.0 - p0.0;
    let chord_dy = p3.1 - p0.1;
    let chord = (chord_dx * chord_dx + chord_dy * chord_dy).sqrt();
    len.max(chord)
}

fn cubic_quadratic_fit_error(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    q0: (f32, f32),
    q1: (f32, f32),
    q2: (f32, f32),
) -> f32 {
    let mut max_error = 0.0_f32;
    for t in [0.25_f32, 0.5_f32, 0.75_f32] {
        let c = eval_cubic(p0, p1, p2, p3, t);
        let q = eval_quadratic(q0, q1, q2, t);
        let dx = c.0 - q.0;
        let dy = c.1 - q.1;
        max_error = max_error.max((dx * dx + dy * dy).sqrt());
    }
    max_error
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
fn append_cubic_quadratics_adaptive(
    out: &mut Vec<PathPrimitive>,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    tolerance: f32,
    max_depth: usize,
    depth: usize,
) {
    let (q0, q1, q2) = cubic_to_quadratic_fit(p0, p1, p2, p3);
    let fit_error = cubic_quadratic_fit_error(p0, p1, p2, p3, q0, q1, q2);
    if depth >= max_depth || fit_error <= tolerance.max(1.0e-6) {
        out.push(PathPrimitive::Quadratic {
            p0: q0,
            p1: q1,
            p2: q2,
        });
        return;
    }
    let (left, right) = split_cubic_half(p0, p1, p2, p3);
    append_cubic_quadratics_adaptive(
        out,
        left.0,
        left.1,
        left.2,
        left.3,
        tolerance,
        max_depth,
        depth + 1,
    );
    append_cubic_quadratics_adaptive(
        out,
        right.0,
        right.1,
        right.2,
        right.3,
        tolerance,
        max_depth,
        depth + 1,
    );
}

#[expect(
    clippy::type_complexity,
    reason = "The tuple represents the fixed components of a compiler or geometry operation."
)]
fn split_cubic_uniform(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    pieces: usize,
) -> Vec<((f32, f32), (f32, f32), (f32, f32), (f32, f32))> {
    let pieces = pieces.max(1);
    let mut result = Vec::with_capacity(pieces);
    let mut cur = (p0, p1, p2, p3);
    for i in 0..pieces {
        let remaining = pieces - i;
        if remaining == 1 {
            result.push(cur);
            break;
        }
        let t = 1.0_f32 / (remaining as f32);
        let (left, right) = split_cubic(cur.0, cur.1, cur.2, cur.3, t);
        result.push(left);
        cur = right;
    }
    result
}

#[expect(
    clippy::type_complexity,
    reason = "The tuple represents the fixed components of a compiler or geometry operation."
)]
fn split_cubic_half(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
) -> (
    ((f32, f32), (f32, f32), (f32, f32), (f32, f32)),
    ((f32, f32), (f32, f32), (f32, f32), (f32, f32)),
) {
    let p01 = (f32::midpoint(p0.0, p1.0), f32::midpoint(p0.1, p1.1));
    let p12 = (f32::midpoint(p1.0, p2.0), f32::midpoint(p1.1, p2.1));
    let p23 = (f32::midpoint(p2.0, p3.0), f32::midpoint(p2.1, p3.1));
    let p012 = (f32::midpoint(p01.0, p12.0), f32::midpoint(p01.1, p12.1));
    let p123 = (f32::midpoint(p12.0, p23.0), f32::midpoint(p12.1, p23.1));
    let p0123 = (f32::midpoint(p012.0, p123.0), f32::midpoint(p012.1, p123.1));

    ((p0, p01, p012, p0123), (p0123, p123, p23, p3))
}

fn cubic_is_flat_enough(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    tolerance: f32,
) -> bool {
    let d1 = point_line_distance(p1, p0, p3);
    let d2 = point_line_distance(p2, p0, p3);
    d1.max(d2) <= tolerance.max(1.0e-6)
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
fn append_cubic_points_adaptive(
    points: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    tolerance: f32,
    max_depth: usize,
    depth: usize,
) {
    if depth >= max_depth || cubic_is_flat_enough(p0, p1, p2, p3, tolerance) {
        points.push(p3);
        return;
    }
    let (left, right) = split_cubic_half(p0, p1, p2, p3);
    append_cubic_points_adaptive(
        points,
        left.0,
        left.1,
        left.2,
        left.3,
        tolerance,
        max_depth,
        depth + 1,
    );
    append_cubic_points_adaptive(
        points,
        right.0,
        right.1,
        right.2,
        right.3,
        tolerance,
        max_depth,
        depth + 1,
    );
}

pub fn preprocess_path_primitives(
    primitives: &[PathPrimitive],
    sampling: PathSamplingPolicy,
) -> Vec<PathPrimitive> {
    if sampling.preserve_cubics {
        return primitives.to_vec();
    }

    let mut out = Vec::with_capacity(primitives.len());
    for primitive in primitives {
        match *primitive {
            PathPrimitive::Line { from, to } => out.push(PathPrimitive::Line { from, to }),
            PathPrimitive::Quadratic { p0, p1, p2 } => {
                out.push(PathPrimitive::Quadratic { p0, p1, p2 });
            }
            PathPrimitive::Arc {
                from,
                center,
                radius,
                sweep,
            } => out.push(PathPrimitive::Arc {
                from,
                center,
                radius,
                sweep,
            }),
            PathPrimitive::Cubic { p0, p1, p2, p3 } => match sampling.cubic {
                CubicSamplingPolicy::FixedSamples(samples) => {
                    let pieces = split_cubic_uniform(p0, p1, p2, p3, samples.max(1));
                    for piece in pieces {
                        let (q0, q1, q2) =
                            cubic_to_quadratic_fit(piece.0, piece.1, piece.2, piece.3);
                        out.push(PathPrimitive::Quadratic {
                            p0: q0,
                            p1: q1,
                            p2: q2,
                        });
                    }
                }
                CubicSamplingPolicy::Adaptive {
                    tolerance,
                    max_depth,
                } => {
                    append_cubic_quadratics_adaptive(
                        &mut out,
                        p0,
                        p1,
                        p2,
                        p3,
                        tolerance,
                        max_depth.max(1),
                        0,
                    );
                }
            },
        }
    }
    out
}

impl PathPrimitive {
    fn append_sampled_points(self, points: &mut Vec<(f32, f32)>, sampling: PathSamplingPolicy) {
        match self {
            Self::Line { to, .. } => {
                points.push(to);
            }
            Self::Quadratic { p2, .. } => {
                // Keep one segment per quadratic for now; later lowering can consume
                // quadratic primitives directly for exact closest-point evaluation.
                points.push(p2);
            }
            Self::Cubic { p0, p1, p2, p3 } => match sampling.cubic {
                CubicSamplingPolicy::FixedSamples(samples) => {
                    let samples = samples.max(1);
                    for i in 1..=samples {
                        let t = (i as f32) / (samples as f32);
                        let omt = 1.0 - t;
                        let omt2 = omt * omt;
                        let t2 = t * t;
                        let x = omt2 * omt * p0.0
                            + 3.0 * omt2 * t * p1.0
                            + 3.0 * omt * t2 * p2.0
                            + t2 * t * p3.0;
                        let y = omt2 * omt * p0.1
                            + 3.0 * omt2 * t * p1.1
                            + 3.0 * omt * t2 * p2.1
                            + t2 * t * p3.1;
                        points.push((x, y));
                    }
                }
                CubicSamplingPolicy::Adaptive {
                    tolerance,
                    max_depth,
                } => {
                    append_cubic_points_adaptive(
                        points,
                        p0,
                        p1,
                        p2,
                        p3,
                        tolerance,
                        max_depth.max(1),
                        0,
                    );
                }
            },
            Self::Arc {
                from,
                center,
                radius,
                sweep,
            } => {
                let r = radius.abs().max(f32::EPSILON);
                let start = (from.1 - center.1).atan2(from.0 - center.0);
                let samples = ((sweep.abs() * r * sampling.arc_samples_per_radius_radian).ceil()
                    as usize)
                    .max(sampling.arc_min_samples.max(1));
                for i in 1..=samples {
                    let t = (i as f32) / (samples as f32);
                    let a = start + sweep * t;
                    points.push((center.0 + r * a.cos(), center.1 + r * a.sin()));
                }
            }
        }
    }

    fn start_point(self) -> (f32, f32) {
        match self {
            Self::Line { from, .. } | Self::Arc { from, .. } => from,
            Self::Quadratic { p0, .. } => p0,
            Self::Cubic { p0, .. } => p0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathChannelDemand {
    pub dist: bool,
    pub along: bool,
    pub tangent: bool,
    pub point_at: bool,
    pub tangent_at: bool,
}

impl PathChannelDemand {
    pub fn record_channel(&mut self, channel: &'static str) {
        match channel {
            "dist" => self.dist = true,
            "along" => self.along = true,
            "tangent" => self.tangent = true,
            _ => {}
        }
    }

    pub fn record_eval(&mut self, eval: &'static str) {
        match eval {
            "point_at" => self.point_at = true,
            "tangent_at" => self.tangent_at = true,
            _ => {}
        }
    }

    pub fn nearest_channels(self) -> Vec<&'static str> {
        let mut channels = Vec::new();
        if self.dist {
            channels.push("dist");
        }
        if self.along {
            channels.push("along");
        }
        if self.tangent {
            channels.push("tangent");
        }
        channels
    }

    pub const fn needs_nearest_sample(self) -> bool {
        self.dist || self.along || self.tangent
    }

    pub const fn needs_arc_sample(self) -> bool {
        self.point_at || self.tangent_at
    }

    pub const fn needs_path_geometry(self) -> bool {
        self.needs_nearest_sample() || self.needs_arc_sample()
    }
}

#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
pub struct PathProfile {
    pub primitives: Vec<PathPrimitive>,
    pub sampling: PathSamplingPolicy,
    pub flattened_segment_count: usize,
    pub total_length: f32,
    pub storage: PathStorageDecision,
    pub demand: PathChannelDemand,
}

impl PathProfile {
    pub fn flattened_rows(&self) -> Vec<PathSegmentRow> {
        fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            (dx * dx + dy * dy).sqrt()
        }

        fn midpoint(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
            (f32::midpoint(a.0, b.0), f32::midpoint(a.1, b.1))
        }

        let mut rows = Vec::new();
        let mut s0 = 0.0_f32;

        for primitive in &self.primitives {
            match *primitive {
                PathPrimitive::Line { from, to } => {
                    let len = distance(from, to);
                    rows.push(PathSegmentRow {
                        p0: from,
                        p1: midpoint(from, to),
                        p2: to,
                        p3: to,
                        s0,
                        len,
                        kind: PATH_SEG_KIND_QUADRATIC,
                        mid_u: 0.5,
                    });
                    s0 += len;
                }
                PathPrimitive::Quadratic { p0, p1, p2 } => {
                    let len = quadratic_arc_length(p0, p1, p2);
                    let mid_len = quadratic_arc_length_range(p0, p1, p2, 0.0, 0.5);
                    let mid_u = if len > f32::EPSILON {
                        (mid_len / len).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    rows.push(PathSegmentRow {
                        p0,
                        p1,
                        p2,
                        p3: p2,
                        s0,
                        len,
                        kind: PATH_SEG_KIND_QUADRATIC,
                        mid_u,
                    });
                    s0 += len;
                }
                PathPrimitive::Cubic { p0, p1, p2, p3 } => {
                    let len = cubic_arc_length(p0, p1, p2, p3);
                    let mid_len = cubic_arc_length_range(p0, p1, p2, p3, 0.0, 0.5);
                    let mid_u = if len > f32::EPSILON {
                        (mid_len / len).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    rows.push(PathSegmentRow {
                        p0,
                        p1,
                        p2,
                        p3,
                        s0,
                        len,
                        kind: PATH_SEG_KIND_CUBIC,
                        mid_u,
                    });
                    s0 += len;
                }
                PathPrimitive::Arc { .. } => {
                    // Fallback path: approximate with short linear chords, but still
                    // emit rows in quadratic-native storage form.
                    let mut points = vec![primitive.start_point()];
                    primitive.append_sampled_points(&mut points, self.sampling);
                    for window in points.windows(2) {
                        let a = window[0];
                        let b = window[1];
                        let len = distance(a, b);
                        rows.push(PathSegmentRow {
                            p0: a,
                            p1: midpoint(a, b),
                            p2: b,
                            p3: b,
                            s0,
                            len,
                            kind: PATH_SEG_KIND_QUADRATIC,
                            mid_u: 0.5,
                        });
                        s0 += len;
                    }
                }
            }
        }

        rows
    }
}

/// Filtering state for procedural patterns (§3, analytic prefiltering).
///
/// Controls whether patterns use analytic prefiltering (via box-filtered
/// antiderivatives) or unfiltered point-sampling.
///
/// - `Auto`: Enable filtering when footprint is available (default).
/// - `ForceOn`: Always use filtering (fails if footprint unavailable).
/// - `ForceOff`: Always use unfiltered point-sampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilteringState {
    /// Automatically enable filtering when footprint is available (default).
    Auto,
    /// Force filtering on (requires footprint, errors if unavailable).
    ForceOn,
    /// Force filtering off (always use unfiltered point-sampling).
    ForceOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeAaStyle {
    Gradient,
    Fwidth,
    Conservative,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserFnCall {
    pub helper_id: String,
    pub args: Vec<Sx>,
    pub ret_components: u8,
}

#[derive(Debug, Clone)]
pub struct UserFnHelper {
    pub ret_kind: crate::typed_scalar::Kind,
    pub id: String,
    pub params: Vec<UserFnParam>,
    pub param_scalars: Vec<String>,
    pub ret_components: u8,
    pub sample_point_invariant: bool,
    pub needs_entry_inputs: bool,
    pub body_stmts: Vec<UserFnStmt>,
}

#[derive(Debug, Clone)]
pub struct UserFnParam {
    pub scalar_kind: crate::typed_scalar::Kind,
    pub name: String,
    pub ty: UserFnParamTy,
    pub scalar_slots: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserFnParamTy {
    Scalar,
    Vec2,
    Vec3,
    Vec4,
    Mat2,
    Mat3,
    Mat4,
}

#[derive(Debug, Clone)]
pub enum UserFnValue {
    Scalar(Sx),
    Vec2((Sx, Sx)),
    Vec3((Sx, Sx, Sx)),
    Vec4((Sx, Sx, Sx, Sx)),
}

#[derive(Debug, Clone)]
pub enum UserFnExpr {
    Value(UserFnValue),
    SlotSwizzle2([String; 2]),
    SlotSwizzle3([String; 3]),
    SlotSwizzle4([String; 4]),
    Normalize2((Sx, Sx)),
    Normalize3((Sx, Sx, Sx)),
    Normalize4((Sx, Sx, Sx, Sx)),
    Min2 {
        a: (Sx, Sx),
        b: (Sx, Sx),
    },
    Min3 {
        a: (Sx, Sx, Sx),
        b: (Sx, Sx, Sx),
    },
    Min4 {
        a: (Sx, Sx, Sx, Sx),
        b: (Sx, Sx, Sx, Sx),
    },
    Max2 {
        a: (Sx, Sx),
        b: (Sx, Sx),
    },
    Max3 {
        a: (Sx, Sx, Sx),
        b: (Sx, Sx, Sx),
    },
    Max4 {
        a: (Sx, Sx, Sx, Sx),
        b: (Sx, Sx, Sx, Sx),
    },
    Clamp2 {
        x: (Sx, Sx),
        lo: (Sx, Sx),
        hi: (Sx, Sx),
    },
    Clamp3 {
        x: (Sx, Sx, Sx),
        lo: (Sx, Sx, Sx),
        hi: (Sx, Sx, Sx),
    },
    Clamp4 {
        x: (Sx, Sx, Sx, Sx),
        lo: (Sx, Sx, Sx, Sx),
        hi: (Sx, Sx, Sx, Sx),
    },
}

#[derive(Debug, Clone)]
pub enum UserFnStmt {
    Let {
        name: String,
        slots: Vec<String>,
        init: UserFnExpr,
    },
    Assign {
        name: String,
        field_path: Option<String>,
        value: UserFnExpr,
    },
    Expr {
        value: UserFnExpr,
    },
    If {
        cond: Sx,
        then_body: Vec<UserFnStmt>,
        else_body: Vec<UserFnStmt>,
    },
    For {
        name: String,
        slots: Vec<String>,
        values: Vec<Sx>,
        body: Vec<UserFnStmt>,
        /// Optional index variable name bound to the loop counter (from `each (v, i) in ...`).
        index_name: Option<String>,
    },
    Return {
        value: UserFnExpr,
    },
    Break,
}

impl UserFnExpr {
    /// Calls `f` on every `Sx` leaf contained in this expression.
    pub fn for_each_sx(&self, f: &mut impl FnMut(&Sx)) {
        match self {
            UserFnExpr::Value(UserFnValue::Scalar(s)) => f(s),
            UserFnExpr::Value(UserFnValue::Vec2((x, y))) => {
                f(x);
                f(y);
            }
            UserFnExpr::Value(UserFnValue::Vec3((x, y, z))) => {
                f(x);
                f(y);
                f(z);
            }
            UserFnExpr::Value(UserFnValue::Vec4((x, y, z, w))) => {
                f(x);
                f(y);
                f(z);
                f(w);
            }
            UserFnExpr::SlotSwizzle2(_)
            | UserFnExpr::SlotSwizzle3(_)
            | UserFnExpr::SlotSwizzle4(_) => {}
            UserFnExpr::Normalize2((x, y)) => {
                f(x);
                f(y);
            }
            UserFnExpr::Normalize3((x, y, z)) => {
                f(x);
                f(y);
                f(z);
            }
            UserFnExpr::Normalize4((x, y, z, w)) => {
                f(x);
                f(y);
                f(z);
                f(w);
            }
            UserFnExpr::Min2 {
                a: (ax, ay),
                b: (bx, by),
            }
            | UserFnExpr::Max2 {
                a: (ax, ay),
                b: (bx, by),
            } => {
                f(ax);
                f(ay);
                f(bx);
                f(by);
            }
            UserFnExpr::Min3 {
                a: (ax, ay, az),
                b: (bx, by, bz),
            }
            | UserFnExpr::Max3 {
                a: (ax, ay, az),
                b: (bx, by, bz),
            } => {
                f(ax);
                f(ay);
                f(az);
                f(bx);
                f(by);
                f(bz);
            }
            UserFnExpr::Min4 {
                a: (ax, ay, az, aw),
                b: (bx, by, bz, bw),
            }
            | UserFnExpr::Max4 {
                a: (ax, ay, az, aw),
                b: (bx, by, bz, bw),
            } => {
                f(ax);
                f(ay);
                f(az);
                f(aw);
                f(bx);
                f(by);
                f(bz);
                f(bw);
            }
            UserFnExpr::Clamp2 {
                x: (x0, x1),
                lo: (l0, l1),
                hi: (h0, h1),
            } => {
                f(x0);
                f(x1);
                f(l0);
                f(l1);
                f(h0);
                f(h1);
            }
            UserFnExpr::Clamp3 {
                x: (x0, x1, x2),
                lo: (l0, l1, l2),
                hi: (h0, h1, h2),
            } => {
                f(x0);
                f(x1);
                f(x2);
                f(l0);
                f(l1);
                f(l2);
                f(h0);
                f(h1);
                f(h2);
            }
            UserFnExpr::Clamp4 {
                x: (x0, x1, x2, x3),
                lo: (l0, l1, l2, l3),
                hi: (h0, h1, h2, h3),
            } => {
                f(x0);
                f(x1);
                f(x2);
                f(x3);
                f(l0);
                f(l1);
                f(l2);
                f(l3);
                f(h0);
                f(h1);
                f(h2);
                f(h3);
            }
        }
    }
}

impl UserFnStmt {
    /// Calls `f` on every `Sx` contained anywhere within this statement
    /// (including recursively into nested statement lists).
    pub fn walk_sx(&self, f: &mut impl FnMut(&Sx)) {
        match self {
            UserFnStmt::Let { init, .. }
            | UserFnStmt::Assign { value: init, .. }
            | UserFnStmt::Expr { value: init }
            | UserFnStmt::Return { value: init } => init.for_each_sx(f),
            UserFnStmt::If {
                cond,
                then_body,
                else_body,
            } => {
                f(cond);
                for stmt in then_body {
                    stmt.walk_sx(f);
                }
                for stmt in else_body {
                    stmt.walk_sx(f);
                }
            }
            UserFnStmt::For { values, body, .. } => {
                for sx in values {
                    f(sx);
                }
                for stmt in body {
                    stmt.walk_sx(f);
                }
            }
            UserFnStmt::Break => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// Element type for array parameters
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayElemType {
    F32,
    I32,
    U32,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    Mat2,
    Mat3,
    Mat4,
    Color,
}

impl ArrayElemType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "f32" => Some(Self::F32),
            "i32" => Some(Self::I32),
            "u32" => Some(Self::U32),
            "bool" => Some(Self::Bool),
            "vec2" => Some(Self::Vec2),
            "vec3" => Some(Self::Vec3),
            "vec4" => Some(Self::Vec4),
            "mat2" => Some(Self::Mat2),
            "mat3" => Some(Self::Mat3),
            "mat4" => Some(Self::Mat4),
            "color" => Some(Self::Color),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::Bool => "bool",
            Self::Vec2 => "vec2",
            Self::Vec3 => "vec3",
            Self::Vec4 => "vec4",
            Self::Mat2 => "mat2",
            Self::Mat3 => "mat3",
            Self::Mat4 => "mat4",
            Self::Color => "color",
        }
    }
}

/// Individual array element value
#[derive(Debug, Clone)]
pub enum ArrayElemValue {
    F32(f32),
    I32(i32),
    U32(u32),
    Bool(bool),
    Vec2((f32, f32)),
    Vec3((f32, f32, f32)),
    Vec4((f32, f32, f32, f32)),
    Mat2([[f32; 2]; 2]),
    Mat3([[f32; 3]; 3]),
    Mat4([[f32; 4]; 4]),
    Color(Color),
}

impl ArrayElemValue {
    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn matches_type(&self, elem_type: ArrayElemType) -> bool {
        matches!(
            (elem_type, self),
            (ArrayElemType::F32, ArrayElemValue::F32(_))
                | (ArrayElemType::I32, ArrayElemValue::I32(_))
                | (ArrayElemType::U32, ArrayElemValue::U32(_))
                | (ArrayElemType::Bool, ArrayElemValue::Bool(_))
                | (ArrayElemType::Vec2, ArrayElemValue::Vec2(_))
                | (ArrayElemType::Vec3, ArrayElemValue::Vec3(_))
                | (ArrayElemType::Vec4, ArrayElemValue::Vec4(_))
                | (ArrayElemType::Mat2, ArrayElemValue::Mat2(_))
                | (ArrayElemType::Mat3, ArrayElemValue::Mat3(_))
                | (ArrayElemType::Mat4, ArrayElemValue::Mat4(_))
                | (ArrayElemType::Color, ArrayElemValue::Color(_))
        )
    }
}

/// Generic array parameter container
#[derive(Debug, Clone)]
pub struct ArrayParam {
    pub elem_type: ArrayElemType,
    pub values: Vec<ArrayElemValue>,
}

impl ArrayParam {
    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_f32(&self, index: usize) -> Option<f32> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::F32, ArrayElemValue::F32(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_i32(&self, index: usize) -> Option<i32> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::I32, ArrayElemValue::I32(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_u32(&self, index: usize) -> Option<u32> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::U32, ArrayElemValue::U32(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_bool(&self, index: usize) -> Option<bool> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Bool, ArrayElemValue::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_vec2(&self, index: usize) -> Option<(f32, f32)> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Vec2, ArrayElemValue::Vec2(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_vec3(&self, index: usize) -> Option<(f32, f32, f32)> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Vec3, ArrayElemValue::Vec3(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_vec4(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Vec4, ArrayElemValue::Vec4(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_mat2(&self, index: usize) -> Option<[[f32; 2]; 2]> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Mat2, ArrayElemValue::Mat2(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_mat3(&self, index: usize) -> Option<[[f32; 3]; 3]> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Mat3, ArrayElemValue::Mat3(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_mat4(&self, index: usize) -> Option<[[f32; 4]; 4]> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Mat4, ArrayElemValue::Mat4(v)) => Some(*v),
            _ => None,
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub fn get_color(&self, index: usize) -> Option<Color> {
        match (&self.elem_type, self.values.get(index)?) {
            (ArrayElemType::Color, ArrayElemValue::Color(v)) => Some(*v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ParamDefault {
    Scalar(f32),
    Array(ArrayParam),
    Int(i32),
    UInt(u32),
    Bool(bool),
    Color(Color),
}

/// A vector of scalar expressions, representing a 2-, 3-, or 4-component vector.
/// Used as a compact carrier for vector operands in the dimension-collapsed `Sx` variants
/// (`Dot`, `NormalizeComponent`, `MinComponent`, `MaxComponent`, `ClampVecComponent`, `Length`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SxVec {
    V2(Box<(Sx, Sx)>),
    V3(Box<(Sx, Sx, Sx)>),
    V4(Box<(Sx, Sx, Sx, Sx)>),
}

impl SxVec {
    /// Apply coordinate substitution to every `Sx` leaf in this vector.
    pub fn subst_coord(self, new_x: &Sx, new_y: &Sx) -> SxVec {
        match self {
            SxVec::V2(v) => {
                let (a, b) = *v;
                SxVec::V2(Box::new((
                    a.subst_coord(new_x, new_y),
                    b.subst_coord(new_x, new_y),
                )))
            }
            SxVec::V3(v) => {
                let (a, b, c) = *v;
                SxVec::V3(Box::new((
                    a.subst_coord(new_x, new_y),
                    b.subst_coord(new_x, new_y),
                    c.subst_coord(new_x, new_y),
                )))
            }
            SxVec::V4(v) => {
                let (a, b, c, d) = *v;
                SxVec::V4(Box::new((
                    a.subst_coord(new_x, new_y),
                    b.subst_coord(new_x, new_y),
                    c.subst_coord(new_x, new_y),
                    d.subst_coord(new_x, new_y),
                )))
            }
        }
    }

    /// Apply symbolic variable substitution to every `Sx` leaf in this vector.
    pub fn subst_vars(self, vars: &HashMap<String, Sx>) -> SxVec {
        match self {
            SxVec::V2(v) => {
                let (a, b) = *v;
                SxVec::V2(Box::new((a.subst_vars(vars), b.subst_vars(vars))))
            }
            SxVec::V3(v) => {
                let (a, b, c) = *v;
                SxVec::V3(Box::new((
                    a.subst_vars(vars),
                    b.subst_vars(vars),
                    c.subst_vars(vars),
                )))
            }
            SxVec::V4(v) => {
                let (a, b, c, d) = *v;
                SxVec::V4(Box::new((
                    a.subst_vars(vars),
                    b.subst_vars(vars),
                    c.subst_vars(vars),
                    d.subst_vars(vars),
                )))
            }
        }
    }

    /// Visit every `Sx` leaf in this vector.
    pub fn for_each_sx(&self, visit: &mut impl FnMut(&Sx)) {
        match self {
            SxVec::V2(v) => {
                visit(&v.0);
                visit(&v.1);
            }
            SxVec::V3(v) => {
                visit(&v.0);
                visit(&v.1);
                visit(&v.2);
            }
            SxVec::V4(v) => {
                visit(&v.0);
                visit(&v.1);
                visit(&v.2);
                visit(&v.3);
            }
        }
    }
}

/// Inputs provided by the existing canvas/surface entry ABI when no engine accessor is declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryInput {
    Time,
    Delta,
    ResolutionX,
    ResolutionY,
}

/// Scalar expression tree (time-varying floats).
#[derive(Debug, Clone)]
pub enum Sx {
    Typed(Box<crate::typed_scalar::Scalar>),
    /// Geometry of a closed cell contour. Channels: distance, progress, length,
    /// pixel distance, pixel progress, pixel length, point x, point y.
    CellContour {
        scope_id: u32,
        inset: Box<Sx>,
        at: Option<Box<Sx>>,
        channel: u8,
    },
    /// Cell half-plane distance, or ray radius when angle is present.
    CellQuery {
        scope_id: u32,
        angle: Option<Box<Sx>>,
        inset: Box<Sx>,
    },
    /// A channel of a gradient evaluated at the active color sample coordinate.
    GradientChannel {
        sample: Rc<GradientSample>,
        channel: u8,
    },
    Lit(f32),
    EntryInput(EntryInput),
    /// A literal in `px`; lowered as `lit * px_var`.
    PxLit(f32),
    /// A field read from a struct-typed global `param`'s bound
    /// `var<uniform>` buffer (e.g. `frame.time` where
    /// `param frame: FrameGlobals` is declared at file scope). `field_index`
    /// is the field's position in the struct's declared field order, used
    /// directly by lowering to build a naga `AccessIndex` read against the
    /// buffer's global variable. `component` selects a vector component
    /// (0=x/r, 1=y/g, 2=z/b, 3=w/a) with a second `AccessIndex` when the
    /// field itself is a vec2/vec3/vec4, or is `None` for scalar fields.
    UniformField {
        binding_name: Rc<str>,
        field_name: Rc<str>,
        field_index: u32,
        component: Option<u8>,
    },
    /// The active sample coordinate in the current space.
    CoordX,
    /// The active sample coordinate in the current space.
    CoordY,
    /// Footprint matrix element [0,0]: ∂x/∂output_x (Jacobian J₁₁).
    /// The x-component of the right-arrow: how local x changes per output pixel-step right.
    /// NOTE: Computed and wired up from canvas_space in lowering.
    /// These variants are constructed by the checker for filtering and gradient-driven effects.
    #[allow(dead_code, reason = "Constructed by checker for filtering")]
    FootprintJ11,
    /// Footprint matrix element [0,1]: ∂x/∂output_y (Jacobian J₁₂).
    /// The x-component of the up-arrow: how local x changes per output pixel-step up.
    #[allow(dead_code, reason = "Constructed by checker for filtering")]
    FootprintJ12,
    /// Footprint matrix element [1,0]: ∂y/∂output_x (Jacobian J₂₁).
    /// The y-component of the right-arrow: how local y changes per output pixel-step right.
    #[allow(dead_code, reason = "Constructed by checker for filtering")]
    FootprintJ21,
    /// Footprint matrix element [1,1]: ∂y/∂output_y (Jacobian J₂₂).
    /// The y-component of the up-arrow: how local y changes per output pixel-step up.
    #[allow(dead_code, reason = "Constructed by checker for filtering")]
    FootprintJ22,
    /// The composed source color red channel for a `postprocess(...)` layer effect.
    PostColorR,
    /// The composed source color green channel for a `postprocess(...)` layer effect.
    PostColorG,
    /// The composed source color blue channel for a `postprocess(...)` layer effect.
    PostColorB,
    /// The composed source color alpha channel for a `postprocess(...)` layer effect.
    PostColorA,
    /// User-exposed scalar parameter declared with `param`.
    Param(String),
    /// Scatter instance id (0..count-1) for template-lowered scatter bodies.
    ScatterInstanceId,
    /// Scatter instance normalized index in [0,1] for template-lowered scatter bodies.
    ScatterInstanceIndex01,
    /// Scatter instance normalized lifecycle phase in [0,1] for template-lowered scatter bodies.
    ScatterInstanceAgeNorm,
    /// Scatter instance x position for template-lowered scatter bodies.
    ScatterInstancePosX,
    /// Scatter instance y position for template-lowered scatter bodies.
    ScatterInstancePosY,
    /// Repeat-cell lattice x id for named `repeat(..., cell: name)` bindings.
    RepeatCellIdX(u32),
    /// Repeat-cell lattice y id for named `repeat(..., cell: name)` bindings.
    RepeatCellIdY(u32),
    /// Repeat-cell center x in the wrapped local repeat cell.
    RepeatCellCenterX(u32),
    /// Repeat-cell center y in the wrapped local repeat cell.
    RepeatCellCenterY(u32),
    /// Repeat-cell local x coordinate in 0..1 within the wrapped cell.
    RepeatCellUvX(u32),
    /// Repeat-cell local y coordinate in 0..1 within the wrapped cell.
    RepeatCellUvY(u32),
    /// Seeded random value constant throughout one cellular owner.
    RepeatCellRand(u32),
    Neg(Box<Sx>),
    Add(Box<Sx>, Box<Sx>),
    Sub(Box<Sx>, Box<Sx>),
    Mul(Box<Sx>, Box<Sx>),
    Div(Box<Sx>, Box<Sx>),
    Lt(Box<Sx>, Box<Sx>),
    Le(Box<Sx>, Box<Sx>),
    Gt(Box<Sx>, Box<Sx>),
    Ge(Box<Sx>, Box<Sx>),
    Eq(Box<Sx>, Box<Sx>),
    Ne(Box<Sx>, Box<Sx>),
    Sin(Box<Sx>),
    Cos(Box<Sx>),
    Tan(Box<Sx>),
    Asin(Box<Sx>),
    Acos(Box<Sx>),
    Atan(Box<Sx>),
    Sqrt(Box<Sx>),
    InverseSqrt(Box<Sx>),
    Fract(Box<Sx>),
    Abs(Box<Sx>),
    Sign(Box<Sx>),
    Floor(Box<Sx>),
    Ceil(Box<Sx>),
    Round(Box<Sx>),
    Trunc(Box<Sx>),
    Exp(Box<Sx>),
    Exp2(Box<Sx>),
    Log(Box<Sx>),
    Log2(Box<Sx>),
    // Binary scalar math
    Atan2(Box<Sx>, Box<Sx>),
    Pow(Box<Sx>, Box<Sx>),
    Min(Box<Sx>, Box<Sx>),
    Max(Box<Sx>, Box<Sx>),
    Step(Box<Sx>, Box<Sx>),
    /// Dot product of two equal-dimension vectors.
    Dot {
        a: SxVec,
        b: SxVec,
    },
    /// One component of a normalized vector. `index` selects the output channel (0-based).
    NormalizeComponent {
        v: SxVec,
        index: u8,
    },
    /// One component of a per-lane vector min. `index` selects the output channel (0-based).
    MinComponent {
        a: SxVec,
        b: SxVec,
        index: u8,
    },
    /// One component of a per-lane vector max. `index` selects the output channel (0-based).
    MaxComponent {
        a: SxVec,
        b: SxVec,
        index: u8,
    },
    /// One component of a per-lane vector clamp. `index` selects the output channel (0-based).
    ClampVecComponent {
        x: SxVec,
        lo: SxVec,
        hi: SxVec,
        index: u8,
    },
    /// Length (magnitude) of a vector.
    Length(SxVec),
    // Ternary scalar math
    Clamp(Box<Sx>, Box<Sx>, Box<Sx>),
    Mix(Box<Sx>, Box<Sx>, Box<Sx>),
    /// WGSL-style selection: false value, true value, condition.
    Select(Box<Sx>, Box<Sx>, Box<Sx>),
    SmoothStep(Box<Sx>, Box<Sx>, Box<Sx>),
    Ddx(Box<Sx>),
    Ddy(Box<Sx>),
    Fwidth(Box<Sx>),
    /// Explicit color-space channel conversion from sRGB to linear.
    SrgbToLinear(Box<Sx>),
    /// Explicit color-space channel conversion from linear to sRGB.
    #[allow(
        dead_code,
        reason = "reserved for planned linear->sRGB channel conversion lowering"
    )]
    LinearToSrgb(Box<Sx>),
    /// Scalar result of a user helper-function call, or a scalar component
    /// extracted from a vector-valued user helper-function call.
    /// `component` is `None` for a scalar-returning call; `Some(index)` to
    /// extract component `index` from a vector-returning call.
    UserCall {
        call: Rc<UserFnCall>,
        component: Option<u32>,
    },
    /// Single channel extracted from a typed texture sample.
    /// `channel` is the RGBA index (0=r, 1=g, 2=b, 3=a).
    /// Lowered by sampling the texture and extracting the indicated component.
    /// `sample_at` is an explicit coordinate override when present; otherwise
    /// the current coordinate is used.
    TexChannel {
        tex_name: String,
        channel: u8,
        sample_at: Option<Box<V2>>,
        decode_mul: f32,
        decode_add: f32,
        /// Optional rich decode expression evaluated with `raw` bound to sampled channel.
        decode_expr: Option<Box<Sx>>,
    },
    /// One channel sampled from the current user-effect input layer.
    EffectInputChannel {
        sample_x: Box<Sx>,
        sample_y: Box<Sx>,
        channel: u8,
    },
    /// Distance from the active sample point to the nearest point on a lowered path profile.
    PathDist {
        path_id: usize,
    },
    /// Arc-length at the nearest point on a lowered path profile.
    PathAlong {
        path_id: usize,
    },
    /// One component of the tangent at the nearest point on a lowered path profile.
    PathTangentComponent {
        path_id: usize,
        component: u8,
    },
    /// One component of `point_at(path, s)` sampled along arc-length `s`.
    PathPointAtComponent {
        path_id: usize,
        s: Box<Sx>,
        component: u8,
    },
    /// One component of `tangent_at(path, s)` sampled along arc-length `s`.
    PathTangentAtComponent {
        path_id: usize,
        s: Box<Sx>,
        component: u8,
    },
    /// A named symbolic variable introduced by a user-defined effect parameter.
    ///
    /// `Sx::Var(name)` appears in effect body layer trees when the checker
    /// evaluates the body with symbolic parameter bindings.  During lowering
    /// of `Layer::UserEffect`, actual argument `Handle<Ex>` values are pushed
    /// onto the IR's var-override stack; `sx_at` then resolves each `Var` to
    /// the override handle instead of emitting a raw variable access.
    Var(String),
    /// Runtime element access on a dynamic array storage-buffer parameter.
    ///
    /// `param_name` identifies the `array<T>` param; `index` is the runtime
    /// scalar index expression; `component` selects a lane for vector element
    /// types (`None` for scalars, `Some(0..n)` for vec2/vec3/vec4 components).
    DynamicArrayIndex {
        param_name: String,
        index: Box<Sx>,
        /// `None` for scalar element types (f32/i32/u32/bool).
        /// `Some(k)` extracts component `k` from vector element types.
        component: Option<u8>,
    },
    /// Bind `value` once under `name`, then evaluate `body` with `Sx::Var(name)`
    /// resolving to that single bound value.
    ///
    /// Exists so a value referenced multiple times within one construction
    /// (e.g. a ternary desugared into `else + cond*(then-else)`, or a
    /// parameter referenced more than once in a function body) is only ever
    /// duplicated as a cheap `Var` leaf, not as a full clone of its
    /// (potentially large) expression tree. Without this, sequential
    /// accumulator chains (`res = opU(res, x)` repeated many times) roughly
    /// double the tree size on every step, which is exponential in the
    /// chain length.
    Let {
        name: String,
        value: Rc<Sx>,
        body: Box<Sx>,
    },
}

impl Sx {
    /// Substitute `CoordX` and `CoordY` with arbitrary expressions throughout
    /// this expression tree.  Used to implement `field <expr> at <coord>` (§ext 23.7):
    /// the field expression is re-evaluated with a different sample point.
    pub fn subst_coord(self, new_x: &Sx, new_y: &Sx) -> Sx {
        let sub = |s: Sx| s.subst_coord(new_x, new_y);
        match self {
            Sx::Typed(value) => Sx::Typed(Box::new(value.map(sub))),
            Sx::CellContour {
                scope_id,
                channel,
                at,
                inset,
            } => Sx::CellContour {
                scope_id,
                channel,
                at: at.map(|a| Box::new(sub(*a))),
                inset: Box::new(sub(*inset)),
            },
            Sx::CellQuery {
                scope_id,
                angle,
                inset,
            } => Sx::CellQuery {
                scope_id,
                angle: angle.map(|a| Box::new(sub(*a))),
                inset: Box::new(sub(*inset)),
            },
            Sx::GradientChannel { sample, channel } => Sx::GradientChannel {
                sample: Rc::new(sample.map_sx(sub)),
                channel,
            },
            // Coordinate leaves — these are the substitution targets.
            Sx::CoordX => new_x.clone(),
            Sx::CoordY => new_y.clone(),
            // Pure leaves that don't depend on the sample coordinate.
            leaf @ (Sx::Lit(_)
            | Sx::PxLit(_)
            | Sx::EntryInput(_)
            | Sx::UniformField { .. }
            | Sx::FootprintJ11
            | Sx::FootprintJ12
            | Sx::FootprintJ21
            | Sx::FootprintJ22
            | Sx::PostColorR
            | Sx::PostColorG
            | Sx::PostColorB
            | Sx::PostColorA
            | Sx::Param(_)
            | Sx::ScatterInstanceId
            | Sx::ScatterInstanceIndex01
            | Sx::ScatterInstanceAgeNorm
            | Sx::ScatterInstancePosX
            | Sx::ScatterInstancePosY
            | Sx::RepeatCellIdX(_)
            | Sx::RepeatCellIdY(_)
            | Sx::RepeatCellCenterX(_)
            | Sx::RepeatCellCenterY(_)
            | Sx::RepeatCellUvX(_)
            | Sx::RepeatCellUvY(_)
            | Sx::RepeatCellRand(_)
            | Sx::PathDist { .. }
            | Sx::PathAlong { .. }
            | Sx::PathTangentComponent { .. }
            | Sx::Var(_)) => leaf,
            Sx::TexChannel {
                tex_name,
                channel,
                sample_at,
                decode_mul,
                decode_add,
                decode_expr,
            } => Sx::TexChannel {
                tex_name,
                channel,
                sample_at: sample_at.map(|sample_at| {
                    let (sx, sy) = *sample_at;
                    Box::new((sub(sx), sub(sy)))
                }),
                decode_mul,
                decode_add,
                decode_expr: decode_expr.map(|expr| Box::new(sub(*expr))),
            },
            Sx::DynamicArrayIndex {
                param_name,
                index,
                component,
            } => Sx::DynamicArrayIndex {
                param_name,
                index: Box::new(sub(*index)),
                component,
            },
            Sx::EffectInputChannel {
                sample_x,
                sample_y,
                channel,
            } => Sx::EffectInputChannel {
                sample_x: Box::new(sub(*sample_x)),
                sample_y: Box::new(sub(*sample_y)),
                channel,
            },
            Sx::PathPointAtComponent {
                path_id,
                s,
                component,
            } => Sx::PathPointAtComponent {
                path_id,
                s: Box::new(sub(*s)),
                component,
            },
            Sx::PathTangentAtComponent {
                path_id,
                s,
                component,
            } => Sx::PathTangentAtComponent {
                path_id,
                s: Box::new(sub(*s)),
                component,
            },
            // Unary
            Sx::Neg(a) => Sx::Neg(Box::new(sub(*a))),
            Sx::Sin(a) => Sx::Sin(Box::new(sub(*a))),
            Sx::Cos(a) => Sx::Cos(Box::new(sub(*a))),
            Sx::Tan(a) => Sx::Tan(Box::new(sub(*a))),
            Sx::Asin(a) => Sx::Asin(Box::new(sub(*a))),
            Sx::Acos(a) => Sx::Acos(Box::new(sub(*a))),
            Sx::Atan(a) => Sx::Atan(Box::new(sub(*a))),
            Sx::Sqrt(a) => Sx::Sqrt(Box::new(sub(*a))),
            Sx::InverseSqrt(a) => Sx::InverseSqrt(Box::new(sub(*a))),
            Sx::Fract(a) => Sx::Fract(Box::new(sub(*a))),
            Sx::Abs(a) => Sx::Abs(Box::new(sub(*a))),
            Sx::Sign(a) => Sx::Sign(Box::new(sub(*a))),
            Sx::Floor(a) => Sx::Floor(Box::new(sub(*a))),
            Sx::Ceil(a) => Sx::Ceil(Box::new(sub(*a))),
            Sx::Round(a) => Sx::Round(Box::new(sub(*a))),
            Sx::Trunc(a) => Sx::Trunc(Box::new(sub(*a))),
            Sx::Exp(a) => Sx::Exp(Box::new(sub(*a))),
            Sx::Exp2(a) => Sx::Exp2(Box::new(sub(*a))),
            Sx::Log(a) => Sx::Log(Box::new(sub(*a))),
            Sx::Log2(a) => Sx::Log2(Box::new(sub(*a))),
            Sx::Ddx(a) => Sx::Ddx(Box::new(sub(*a))),
            Sx::Ddy(a) => Sx::Ddy(Box::new(sub(*a))),
            Sx::Fwidth(a) => Sx::Fwidth(Box::new(sub(*a))),
            Sx::SrgbToLinear(a) => Sx::SrgbToLinear(Box::new(sub(*a))),
            Sx::LinearToSrgb(a) => Sx::LinearToSrgb(Box::new(sub(*a))),
            // Binary
            Sx::Add(a, b) => Sx::Add(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Sub(a, b) => Sx::Sub(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Mul(a, b) => Sx::Mul(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Div(a, b) => Sx::Div(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Lt(a, b) => Sx::Lt(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Le(a, b) => Sx::Le(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Gt(a, b) => Sx::Gt(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Ge(a, b) => Sx::Ge(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Eq(a, b) => Sx::Eq(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Ne(a, b) => Sx::Ne(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Atan2(a, b) => Sx::Atan2(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Pow(a, b) => Sx::Pow(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Min(a, b) => Sx::Min(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Max(a, b) => Sx::Max(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Step(a, b) => Sx::Step(Box::new(sub(*a)), Box::new(sub(*b))),
            // Vector operations (dimension-collapsed)
            Sx::Dot { a, b } => Sx::Dot {
                a: a.subst_coord(new_x, new_y),
                b: b.subst_coord(new_x, new_y),
            },
            Sx::NormalizeComponent { v, index } => Sx::NormalizeComponent {
                v: v.subst_coord(new_x, new_y),
                index,
            },
            Sx::MinComponent { a, b, index } => Sx::MinComponent {
                a: a.subst_coord(new_x, new_y),
                b: b.subst_coord(new_x, new_y),
                index,
            },
            Sx::MaxComponent { a, b, index } => Sx::MaxComponent {
                a: a.subst_coord(new_x, new_y),
                b: b.subst_coord(new_x, new_y),
                index,
            },
            Sx::ClampVecComponent { x, lo, hi, index } => Sx::ClampVecComponent {
                x: x.subst_coord(new_x, new_y),
                lo: lo.subst_coord(new_x, new_y),
                hi: hi.subst_coord(new_x, new_y),
                index,
            },
            Sx::Length(v) => Sx::Length(v.subst_coord(new_x, new_y)),
            // Ternary
            Sx::Clamp(a, lo, hi) => {
                Sx::Clamp(Box::new(sub(*a)), Box::new(sub(*lo)), Box::new(sub(*hi)))
            }
            Sx::Mix(a, b, t) => Sx::Mix(Box::new(sub(*a)), Box::new(sub(*b)), Box::new(sub(*t))),
            Sx::Select(a, b, t) => {
                Sx::Select(Box::new(sub(*a)), Box::new(sub(*b)), Box::new(sub(*t)))
            }
            Sx::SmoothStep(lo, hi, x) => {
                Sx::SmoothStep(Box::new(sub(*lo)), Box::new(sub(*hi)), Box::new(sub(*x)))
            }
            Sx::UserCall { call, component } => {
                let call = Rc::try_unwrap(call).unwrap_or_else(|rc| (*rc).clone());
                Sx::UserCall {
                    call: Rc::new(UserFnCall {
                        helper_id: call.helper_id,
                        args: call.args.into_iter().map(sub).collect(),
                        ret_components: call.ret_components,
                    }),
                    component,
                }
            }
            Sx::Let { name, value, body } => Sx::Let {
                name,
                value: Rc::new(sub(Rc::unwrap_or_clone(value))),
                body: Box::new(sub(*body)),
            },
        }
    }

    /// Substitute symbolic `Sx::Var(name)` leaves with concrete scalar
    /// expressions.
    pub fn subst_vars(self, vars: &HashMap<String, Sx>) -> Sx {
        let sub = |s: Sx| s.subst_vars(vars);
        match self {
            Sx::Typed(value) => Sx::Typed(Box::new(value.map(sub))),
            Sx::CellContour {
                scope_id,
                channel,
                at,
                inset,
            } => Sx::CellContour {
                scope_id,
                channel,
                at: at.map(|a| Box::new(sub(*a))),
                inset: Box::new(sub(*inset)),
            },
            Sx::CellQuery {
                scope_id,
                angle,
                inset,
            } => Sx::CellQuery {
                scope_id,
                angle: angle.map(|a| Box::new(sub(*a))),
                inset: Box::new(sub(*inset)),
            },
            Sx::GradientChannel { sample, channel } => Sx::GradientChannel {
                sample: Rc::new(sample.map_sx(sub)),
                channel,
            },
            Sx::Var(name) => vars.get(&name).cloned().unwrap_or(Sx::Var(name)),
            leaf @ (Sx::Lit(_)
            | Sx::PxLit(_)
            | Sx::EntryInput(_)
            | Sx::UniformField { .. }
            | Sx::CoordX
            | Sx::CoordY
            | Sx::FootprintJ11
            | Sx::FootprintJ12
            | Sx::FootprintJ21
            | Sx::FootprintJ22
            | Sx::PostColorR
            | Sx::PostColorG
            | Sx::PostColorB
            | Sx::PostColorA
            | Sx::Param(_)
            | Sx::ScatterInstanceId
            | Sx::ScatterInstanceIndex01
            | Sx::ScatterInstanceAgeNorm
            | Sx::ScatterInstancePosX
            | Sx::ScatterInstancePosY
            | Sx::RepeatCellIdX(_)
            | Sx::RepeatCellIdY(_)
            | Sx::RepeatCellCenterX(_)
            | Sx::RepeatCellCenterY(_)
            | Sx::RepeatCellUvX(_)
            | Sx::RepeatCellUvY(_)
            | Sx::RepeatCellRand(_)
            | Sx::PathDist { .. }
            | Sx::PathAlong { .. }
            | Sx::PathTangentComponent { .. }) => leaf,
            Sx::TexChannel {
                tex_name,
                channel,
                sample_at,
                decode_mul,
                decode_add,
                decode_expr,
            } => Sx::TexChannel {
                tex_name,
                channel,
                sample_at: sample_at.map(|sample_at| {
                    let (sx, sy) = *sample_at;
                    Box::new((sub(sx), sub(sy)))
                }),
                decode_mul,
                decode_add,
                decode_expr: decode_expr.map(|expr| Box::new(sub(*expr))),
            },
            Sx::DynamicArrayIndex {
                param_name,
                index,
                component,
            } => Sx::DynamicArrayIndex {
                param_name,
                index: Box::new(sub(*index)),
                component,
            },
            Sx::EffectInputChannel {
                sample_x,
                sample_y,
                channel,
            } => Sx::EffectInputChannel {
                sample_x: Box::new(sub(*sample_x)),
                sample_y: Box::new(sub(*sample_y)),
                channel,
            },
            Sx::PathPointAtComponent {
                path_id,
                s,
                component,
            } => Sx::PathPointAtComponent {
                path_id,
                s: Box::new(sub(*s)),
                component,
            },
            Sx::PathTangentAtComponent {
                path_id,
                s,
                component,
            } => Sx::PathTangentAtComponent {
                path_id,
                s: Box::new(sub(*s)),
                component,
            },
            Sx::Neg(a) => Sx::Neg(Box::new(sub(*a))),
            Sx::Sin(a) => Sx::Sin(Box::new(sub(*a))),
            Sx::Cos(a) => Sx::Cos(Box::new(sub(*a))),
            Sx::Tan(a) => Sx::Tan(Box::new(sub(*a))),
            Sx::Asin(a) => Sx::Asin(Box::new(sub(*a))),
            Sx::Acos(a) => Sx::Acos(Box::new(sub(*a))),
            Sx::Atan(a) => Sx::Atan(Box::new(sub(*a))),
            Sx::Sqrt(a) => Sx::Sqrt(Box::new(sub(*a))),
            Sx::InverseSqrt(a) => Sx::InverseSqrt(Box::new(sub(*a))),
            Sx::Fract(a) => Sx::Fract(Box::new(sub(*a))),
            Sx::Abs(a) => Sx::Abs(Box::new(sub(*a))),
            Sx::Sign(a) => Sx::Sign(Box::new(sub(*a))),
            Sx::Floor(a) => Sx::Floor(Box::new(sub(*a))),
            Sx::Ceil(a) => Sx::Ceil(Box::new(sub(*a))),
            Sx::Round(a) => Sx::Round(Box::new(sub(*a))),
            Sx::Trunc(a) => Sx::Trunc(Box::new(sub(*a))),
            Sx::Exp(a) => Sx::Exp(Box::new(sub(*a))),
            Sx::Exp2(a) => Sx::Exp2(Box::new(sub(*a))),
            Sx::Log(a) => Sx::Log(Box::new(sub(*a))),
            Sx::Log2(a) => Sx::Log2(Box::new(sub(*a))),
            Sx::Ddx(a) => Sx::Ddx(Box::new(sub(*a))),
            Sx::Ddy(a) => Sx::Ddy(Box::new(sub(*a))),
            Sx::Fwidth(a) => Sx::Fwidth(Box::new(sub(*a))),
            Sx::SrgbToLinear(a) => Sx::SrgbToLinear(Box::new(sub(*a))),
            Sx::LinearToSrgb(a) => Sx::LinearToSrgb(Box::new(sub(*a))),
            Sx::Add(a, b) => Sx::Add(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Sub(a, b) => Sx::Sub(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Mul(a, b) => Sx::Mul(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Div(a, b) => Sx::Div(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Lt(a, b) => Sx::Lt(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Le(a, b) => Sx::Le(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Gt(a, b) => Sx::Gt(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Ge(a, b) => Sx::Ge(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Eq(a, b) => Sx::Eq(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Ne(a, b) => Sx::Ne(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Atan2(a, b) => Sx::Atan2(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Pow(a, b) => Sx::Pow(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Min(a, b) => Sx::Min(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Max(a, b) => Sx::Max(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Step(a, b) => Sx::Step(Box::new(sub(*a)), Box::new(sub(*b))),
            Sx::Dot { a, b } => Sx::Dot {
                a: a.subst_vars(vars),
                b: b.subst_vars(vars),
            },
            Sx::NormalizeComponent { v, index } => Sx::NormalizeComponent {
                v: v.subst_vars(vars),
                index,
            },
            Sx::MinComponent { a, b, index } => Sx::MinComponent {
                a: a.subst_vars(vars),
                b: b.subst_vars(vars),
                index,
            },
            Sx::MaxComponent { a, b, index } => Sx::MaxComponent {
                a: a.subst_vars(vars),
                b: b.subst_vars(vars),
                index,
            },
            Sx::ClampVecComponent { x, lo, hi, index } => Sx::ClampVecComponent {
                x: x.subst_vars(vars),
                lo: lo.subst_vars(vars),
                hi: hi.subst_vars(vars),
                index,
            },
            Sx::Length(v) => Sx::Length(v.subst_vars(vars)),
            Sx::Clamp(a, lo, hi) => {
                Sx::Clamp(Box::new(sub(*a)), Box::new(sub(*lo)), Box::new(sub(*hi)))
            }
            Sx::Mix(a, b, t) => Sx::Mix(Box::new(sub(*a)), Box::new(sub(*b)), Box::new(sub(*t))),
            Sx::Select(a, b, t) => {
                Sx::Select(Box::new(sub(*a)), Box::new(sub(*b)), Box::new(sub(*t)))
            }
            Sx::SmoothStep(lo, hi, x) => {
                Sx::SmoothStep(Box::new(sub(*lo)), Box::new(sub(*hi)), Box::new(sub(*x)))
            }
            Sx::UserCall { call, component } => {
                let call = Rc::try_unwrap(call).unwrap_or_else(|rc| (*rc).clone());
                Sx::UserCall {
                    call: Rc::new(UserFnCall {
                        helper_id: call.helper_id,
                        args: call.args.into_iter().map(sub).collect(),
                        ret_components: call.ret_components,
                    }),
                    component,
                }
            }
            Sx::Let { name, value, body } => Sx::Let {
                name,
                value: Rc::new(sub(Rc::unwrap_or_clone(value))),
                body: Box::new(sub(*body)),
            },
        }
    }

    /// Visit each direct scalar-expression child.
    pub fn for_each_child(&self, mut visit: impl FnMut(&Sx)) {
        match self {
            Sx::Typed(value) => value.args.iter().for_each(&mut visit),
            Sx::CellContour {
                at: angle, inset, ..
            }
            | Sx::CellQuery { angle, inset, .. } => {
                if let Some(angle) = angle {
                    visit(angle);
                }
                visit(inset);
            }
            Sx::GradientChannel { sample, .. } => sample.for_each_sx(&mut visit),
            Sx::UserCall { call, .. } => {
                for arg in &call.args {
                    visit(arg);
                }
            }
            Sx::EffectInputChannel {
                sample_x, sample_y, ..
            } => {
                visit(sample_x);
                visit(sample_y);
            }
            Sx::Neg(a)
            | Sx::Sin(a)
            | Sx::Cos(a)
            | Sx::Tan(a)
            | Sx::Asin(a)
            | Sx::Acos(a)
            | Sx::Atan(a)
            | Sx::Sqrt(a)
            | Sx::InverseSqrt(a)
            | Sx::Fract(a)
            | Sx::Abs(a)
            | Sx::Sign(a)
            | Sx::Floor(a)
            | Sx::Ceil(a)
            | Sx::Round(a)
            | Sx::Trunc(a)
            | Sx::Exp(a)
            | Sx::Exp2(a)
            | Sx::Log(a)
            | Sx::Log2(a)
            | Sx::Ddx(a)
            | Sx::Ddy(a)
            | Sx::Fwidth(a)
            | Sx::SrgbToLinear(a)
            | Sx::LinearToSrgb(a) => visit(a),
            Sx::Add(a, b)
            | Sx::Sub(a, b)
            | Sx::Mul(a, b)
            | Sx::Div(a, b)
            | Sx::Lt(a, b)
            | Sx::Le(a, b)
            | Sx::Gt(a, b)
            | Sx::Ge(a, b)
            | Sx::Eq(a, b)
            | Sx::Ne(a, b)
            | Sx::Atan2(a, b)
            | Sx::Pow(a, b)
            | Sx::Min(a, b)
            | Sx::Max(a, b)
            | Sx::Step(a, b) => {
                visit(a);
                visit(b);
            }
            Sx::Clamp(a, b, c)
            | Sx::Mix(a, b, c)
            | Sx::Select(a, b, c)
            | Sx::SmoothStep(a, b, c) => {
                visit(a);
                visit(b);
                visit(c);
            }
            Sx::Dot { a, b } => {
                a.for_each_sx(&mut visit);
                b.for_each_sx(&mut visit);
            }
            Sx::Length(v) | Sx::NormalizeComponent { v, .. } => {
                v.for_each_sx(&mut visit);
            }
            Sx::MinComponent { a, b, .. } | Sx::MaxComponent { a, b, .. } => {
                a.for_each_sx(&mut visit);
                b.for_each_sx(&mut visit);
            }
            Sx::ClampVecComponent { x, lo, hi, .. } => {
                x.for_each_sx(&mut visit);
                lo.for_each_sx(&mut visit);
                hi.for_each_sx(&mut visit);
            }
            Sx::PathPointAtComponent { s, .. } | Sx::PathTangentAtComponent { s, .. } => {
                visit(s);
            }
            Sx::Lit(_)
            | Sx::PxLit(_)
            | Sx::EntryInput(_)
            | Sx::UniformField { .. }
            | Sx::CoordX
            | Sx::CoordY
            | Sx::FootprintJ11
            | Sx::FootprintJ12
            | Sx::FootprintJ21
            | Sx::FootprintJ22
            | Sx::PostColorR
            | Sx::PostColorG
            | Sx::PostColorB
            | Sx::PostColorA
            | Sx::Param(_)
            | Sx::ScatterInstanceId
            | Sx::ScatterInstanceIndex01
            | Sx::ScatterInstanceAgeNorm
            | Sx::ScatterInstancePosX
            | Sx::ScatterInstancePosY
            | Sx::RepeatCellIdX(_)
            | Sx::RepeatCellIdY(_)
            | Sx::RepeatCellCenterX(_)
            | Sx::RepeatCellCenterY(_)
            | Sx::RepeatCellUvX(_)
            | Sx::RepeatCellUvY(_)
            | Sx::RepeatCellRand(_)
            | Sx::PathDist { .. }
            | Sx::PathAlong { .. }
            | Sx::PathTangentComponent { .. }
            | Sx::Var(_) => {}
            Sx::TexChannel {
                sample_at: Some(sample_at),
                ..
            } => {
                let (sample_x, sample_y) = sample_at.as_ref();
                visit(sample_x);
                visit(sample_y);
            }
            Sx::TexChannel {
                sample_at: None, ..
            } => {}
            Sx::DynamicArrayIndex { index, .. } => {
                visit(index);
            }
            Sx::Let { value, body, .. } => {
                visit(value);
                visit(body);
            }
        }
    }

    /// Depth-first traversal visiting this node before children.
    pub fn walk_preorder(&self, visit: &mut impl FnMut(&Sx)) {
        visit(self);
        self.for_each_child(|child| child.walk_preorder(visit));
    }

    /// Return the semantic name of a history-bound temporal source, when this
    /// node directly represents one.
    ///
    /// This is intentionally sparse today. As first-class frame-indexed or
    /// remembered-state scalar nodes are added, wire them here so temporal-purity
    /// validation can pick them up without checker refactors.
    pub const fn temporal_history_source_name(&self) -> Option<&'static str> {
        None
    }

    pub fn requires_shape_color_frame(&self) -> bool {
        let mut required = false;
        self.walk_preorder(&mut |node| {
            if let Sx::GradientChannel { sample, .. } = node {
                required |= matches!(
                    sample.kind,
                    GradientKind::Linear {
                        anchor: GradientAnchor::Shape,
                        ..
                    }
                );
            }
        });
        required
    }

    /// Whether lowering should memoize this expression at a sample point.
    pub const fn is_cacheable_at_point(&self) -> bool {
        matches!(
            self,
            Self::Param(_)
                | Self::CellQuery { .. }
                | Self::CellContour { .. }
                | Self::GradientChannel { .. }
                | Self::Add(_, _)
                | Self::Sub(_, _)
                | Self::Mul(_, _)
                | Self::Div(_, _)
                | Self::Lt(_, _)
                | Self::Le(_, _)
                | Self::Gt(_, _)
                | Self::Ge(_, _)
                | Self::Eq(_, _)
                | Self::Ne(_, _)
                | Self::Neg(_)
                | Self::Sin(_)
                | Self::Cos(_)
                | Self::Tan(_)
                | Self::Asin(_)
                | Self::Acos(_)
                | Self::Atan(_)
                | Self::Sqrt(_)
                | Self::InverseSqrt(_)
                | Self::Fract(_)
                | Self::Abs(_)
                | Self::Sign(_)
                | Self::Floor(_)
                | Self::Ceil(_)
                | Self::Round(_)
                | Self::Trunc(_)
                | Self::Exp(_)
                | Self::Exp2(_)
                | Self::Log(_)
                | Self::Log2(_)
                | Self::Atan2(_, _)
                | Self::Pow(_, _)
                | Self::Min(_, _)
                | Self::Max(_, _)
                | Self::Step(_, _)
                | Self::Dot { .. }
                | Self::NormalizeComponent { .. }
                | Self::MinComponent { .. }
                | Self::MaxComponent { .. }
                | Self::ClampVecComponent { .. }
                | Self::Length(_)
                | Self::Clamp(_, _, _)
                | Self::Mix(_, _, _)
                | Self::Select(_, _, _)
                | Self::SmoothStep(_, _, _)
                | Self::Ddx(_)
                | Self::Ddy(_)
                | Self::Fwidth(_)
                | Self::SrgbToLinear(_)
                | Self::LinearToSrgb(_)
                | Self::UserCall { .. }
                | Self::ScatterInstanceAgeNorm
                | Self::RepeatCellIdX(_)
                | Self::RepeatCellIdY(_)
                | Self::RepeatCellCenterX(_)
                | Self::RepeatCellCenterY(_)
                | Self::RepeatCellUvX(_)
                | Self::RepeatCellUvY(_)
                | Self::RepeatCellRand(_)
                | Self::TexChannel { .. }
                | Self::EffectInputChannel { .. }
                | Self::PathDist { .. }
                | Self::PathAlong { .. }
                | Self::PathTangentComponent { .. }
                | Self::PathPointAtComponent { .. }
                | Self::PathTangentAtComponent { .. }
                | Self::DynamicArrayIndex { .. }
                | Self::Let { .. }
        )
    }

    /// Evaluate this expression as a constant `f32`, substituting any
    /// `Sx::Var(name)` leaves from `vars`.  Returns `None` if any leaf cannot
    /// be reduced to a compile-time constant (e.g. `Sx::Param`, `Sx::CoordX`,
    /// `Sx::Time`, or a `Sx::Var` whose name is absent from `vars`).
    ///
    /// Used by the rewrite rule engine (§16.1) to evaluate `when` guard
    /// expressions after substituting the constant argument values bound to
    /// each pattern hole.  A rule is skipped conservatively when the guard
    /// cannot be fully evaluated at compile time.
    pub fn try_eval_with_vars(&self, vars: &HashMap<&str, f32>) -> Option<f32> {
        crate::signal_eval::evaluate(self, vars).ok()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Structural equality and hashing for Sx (used as CSE cache keys in lowering).
//
// `f32` does not implement `Eq` or `Hash`, so we provide manual impls that
// use bit-level comparison for `Lit` and `PxLit` (treating NaN as equal to
// itself, which is correct for expression-identity purposes) and delegate
// structurally for every other variant.
// ──────────────────────────────────────────────────────────────────────────────

impl PartialEq for Sx {
    fn eq(&self, other: &Self) -> bool {
        use Sx::*;
        match (self, other) {
            (Typed(a), Typed(b)) => a == b,
            (
                GradientChannel {
                    sample: a,
                    channel: ac,
                },
                GradientChannel {
                    sample: b,
                    channel: bc,
                },
            ) => a == b && ac == bc,
            (
                CellContour {
                    scope_id: a,
                    inset: ai,
                    at: aa,
                    channel: ac,
                },
                CellContour {
                    scope_id: b,
                    inset: bi,
                    at: ba,
                    channel: bc,
                },
            ) => a == b && ai == bi && aa == ba && ac == bc,
            (
                CellQuery {
                    scope_id: a,
                    angle: aa,
                    inset: ai,
                },
                CellQuery {
                    scope_id: b,
                    angle: ba,
                    inset: bi,
                },
            ) => a == b && aa == ba && ai == bi,
            (Lit(a), Lit(b)) => a.to_bits() == b.to_bits(),
            (PxLit(a), PxLit(b)) => a.to_bits() == b.to_bits(),
            (Param(a), Param(b)) => a == b,
            (Neg(a), Neg(b))
            | (Sin(a), Sin(b))
            | (Cos(a), Cos(b))
            | (Tan(a), Tan(b))
            | (Asin(a), Asin(b))
            | (Acos(a), Acos(b))
            | (Atan(a), Atan(b))
            | (Sqrt(a), Sqrt(b))
            | (InverseSqrt(a), InverseSqrt(b))
            | (Fract(a), Fract(b))
            | (Abs(a), Abs(b))
            | (Sign(a), Sign(b))
            | (Floor(a), Floor(b))
            | (Ceil(a), Ceil(b))
            | (Round(a), Round(b))
            | (Trunc(a), Trunc(b))
            | (Exp(a), Exp(b))
            | (Exp2(a), Exp2(b))
            | (Log(a), Log(b))
            | (Log2(a), Log2(b))
            | (Ddx(a), Ddx(b))
            | (Ddy(a), Ddy(b))
            | (Fwidth(a), Fwidth(b))
            | (SrgbToLinear(a), SrgbToLinear(b))
            | (LinearToSrgb(a), LinearToSrgb(b)) => a == b,
            (Add(a1, a2), Add(b1, b2))
            | (Sub(a1, a2), Sub(b1, b2))
            | (Mul(a1, a2), Mul(b1, b2))
            | (Div(a1, a2), Div(b1, b2))
            | (Lt(a1, a2), Lt(b1, b2))
            | (Le(a1, a2), Le(b1, b2))
            | (Gt(a1, a2), Gt(b1, b2))
            | (Ge(a1, a2), Ge(b1, b2))
            | (Eq(a1, a2), Eq(b1, b2))
            | (Ne(a1, a2), Ne(b1, b2))
            | (Atan2(a1, a2), Atan2(b1, b2))
            | (Pow(a1, a2), Pow(b1, b2))
            | (Min(a1, a2), Min(b1, b2))
            | (Max(a1, a2), Max(b1, b2))
            | (Step(a1, a2), Step(b1, b2)) => a1 == b1 && a2 == b2,
            (Dot { a: a1, b: a2 }, Dot { a: b1, b: b2 }) => a1 == b1 && a2 == b2,
            (Length(a), Length(b)) => a == b,
            (NormalizeComponent { v: va, index: ia }, NormalizeComponent { v: vb, index: ib }) => {
                ia == ib && va == vb
            }
            (
                MinComponent {
                    a: aa,
                    b: ab,
                    index: ia,
                },
                MinComponent {
                    a: ba,
                    b: bb,
                    index: ib,
                },
            )
            | (
                MaxComponent {
                    a: aa,
                    b: ab,
                    index: ia,
                },
                MaxComponent {
                    a: ba,
                    b: bb,
                    index: ib,
                },
            ) => ia == ib && aa == ba && ab == bb,
            (
                ClampVecComponent {
                    x: xa,
                    lo: la,
                    hi: ha,
                    index: ia,
                },
                ClampVecComponent {
                    x: xb,
                    lo: lb,
                    hi: hb,
                    index: ib,
                },
            ) => ia == ib && xa == xb && la == lb && ha == hb,
            (Clamp(a1, a2, a3), Clamp(b1, b2, b3))
            | (Mix(a1, a2, a3), Mix(b1, b2, b3))
            | (Select(a1, a2, a3), Select(b1, b2, b3))
            | (SmoothStep(a1, a2, a3), SmoothStep(b1, b2, b3)) => a1 == b1 && a2 == b2 && a3 == b3,
            (
                UserCall {
                    call: ca,
                    component: ia,
                },
                UserCall {
                    call: cb,
                    component: ib,
                },
            ) => ia == ib && ca == cb,
            (
                TexChannel {
                    tex_name: ta,
                    channel: ca,
                    sample_at: sa,
                    decode_mul: ma,
                    decode_add: aa,
                    decode_expr: ea,
                },
                TexChannel {
                    tex_name: tb,
                    channel: cb,
                    sample_at: sb,
                    decode_mul: mb,
                    decode_add: ab,
                    decode_expr: eb,
                },
            ) => {
                ta == tb
                    && ca == cb
                    && sa == sb
                    && ma.to_bits() == mb.to_bits()
                    && aa.to_bits() == ab.to_bits()
                    && ea == eb
            }
            (
                EffectInputChannel {
                    sample_x: ax,
                    sample_y: ay,
                    channel: ac,
                },
                EffectInputChannel {
                    sample_x: bx,
                    sample_y: by,
                    channel: bc,
                },
            ) => ax == bx && ay == by && ac == bc,
            (PathDist { path_id: a }, PathDist { path_id: b })
            | (PathAlong { path_id: a }, PathAlong { path_id: b }) => a == b,
            (
                PathTangentComponent {
                    path_id: pa,
                    component: ca,
                },
                PathTangentComponent {
                    path_id: pb,
                    component: cb,
                },
            ) => pa == pb && ca == cb,
            (
                PathPointAtComponent {
                    path_id: pa,
                    s: sa,
                    component: ca,
                },
                PathPointAtComponent {
                    path_id: pb,
                    s: sb,
                    component: cb,
                },
            )
            | (
                PathTangentAtComponent {
                    path_id: pa,
                    s: sa,
                    component: ca,
                },
                PathTangentAtComponent {
                    path_id: pb,
                    s: sb,
                    component: cb,
                },
            ) => pa == pb && ca == cb && sa == sb,
            (
                UniformField {
                    binding_name: ba,
                    field_name: fa,
                    component: ca,
                    ..
                },
                UniformField {
                    binding_name: bb,
                    field_name: fb,
                    component: cb,
                    ..
                },
            ) => ba == bb && fa == fb && ca == cb,
            (EntryInput(a), EntryInput(b)) => a == b,
            // Unit variants (no fields)
            (CoordX, CoordX)
            | (CoordY, CoordY)
            | (FootprintJ11, FootprintJ11)
            | (FootprintJ12, FootprintJ12)
            | (FootprintJ21, FootprintJ21)
            | (FootprintJ22, FootprintJ22)
            | (PostColorR, PostColorR)
            | (PostColorG, PostColorG)
            | (PostColorB, PostColorB)
            | (PostColorA, PostColorA)
            | (ScatterInstanceId, ScatterInstanceId)
            | (ScatterInstanceIndex01, ScatterInstanceIndex01)
            | (ScatterInstanceAgeNorm, ScatterInstanceAgeNorm)
            | (ScatterInstancePosX, ScatterInstancePosX)
            | (ScatterInstancePosY, ScatterInstancePosY) => true,
            (RepeatCellIdX(a), RepeatCellIdX(b))
            | (RepeatCellIdY(a), RepeatCellIdY(b))
            | (RepeatCellCenterX(a), RepeatCellCenterX(b))
            | (RepeatCellCenterY(a), RepeatCellCenterY(b))
            | (RepeatCellUvX(a), RepeatCellUvX(b))
            | (RepeatCellUvY(a), RepeatCellUvY(b))
            | (RepeatCellRand(a), RepeatCellRand(b)) => a == b,
            (
                DynamicArrayIndex {
                    param_name: pa,
                    index: ia,
                    component: ca,
                },
                DynamicArrayIndex {
                    param_name: pb,
                    index: ib,
                    component: cb,
                },
            ) => pa == pb && ca == cb && ia == ib,
            (
                Let {
                    name: na,
                    value: va,
                    body: ba,
                },
                Let {
                    name: nb,
                    value: vb,
                    body: bb,
                },
            ) => na == nb && va == vb && ba == bb,
            _ => false,
        }
    }
}

impl Eq for Sx {}

impl std::hash::Hash for Sx {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use Sx::*;
        // Hash the variant discriminant first, then fields.
        std::mem::discriminant(self).hash(state);
        match self {
            Typed(value) => value.hash(state),
            CellContour {
                scope_id,
                inset,
                at,
                channel,
            } => {
                scope_id.hash(state);
                inset.hash(state);
                at.hash(state);
                channel.hash(state);
            }
            CellQuery {
                scope_id,
                angle,
                inset,
            } => {
                scope_id.hash(state);
                angle.hash(state);
                inset.hash(state);
            }
            GradientChannel { sample, channel } => {
                sample.hash(state);
                channel.hash(state);
            }
            EntryInput(input) => input.hash(state),
            Lit(f) => f.to_bits().hash(state),
            PxLit(f) => f.to_bits().hash(state),
            Param(s) => s.hash(state),
            Neg(a) | Sin(a) | Cos(a) | Tan(a) | Asin(a) | Acos(a) | Atan(a) | Sqrt(a)
            | InverseSqrt(a) | Fract(a) | Abs(a) | Sign(a) | Floor(a) | Ceil(a) | Round(a)
            | Trunc(a) | Exp(a) | Exp2(a) | Log(a) | Log2(a) | Ddx(a) | Ddy(a) | Fwidth(a)
            | SrgbToLinear(a) | LinearToSrgb(a) => a.hash(state),
            Add(a, b)
            | Sub(a, b)
            | Mul(a, b)
            | Div(a, b)
            | Lt(a, b)
            | Le(a, b)
            | Gt(a, b)
            | Ge(a, b)
            | Eq(a, b)
            | Ne(a, b)
            | Atan2(a, b)
            | Pow(a, b)
            | Min(a, b)
            | Max(a, b)
            | Step(a, b) => {
                a.hash(state);
                b.hash(state);
            }
            Dot { a, b } => {
                a.hash(state);
                b.hash(state);
            }
            Length(v) => v.hash(state),
            NormalizeComponent { v, index } => {
                index.hash(state);
                v.hash(state);
            }
            MinComponent { a, b, index } | MaxComponent { a, b, index } => {
                index.hash(state);
                a.hash(state);
                b.hash(state);
            }
            ClampVecComponent { x, lo, hi, index } => {
                index.hash(state);
                x.hash(state);
                lo.hash(state);
                hi.hash(state);
            }
            Clamp(a, b, c) | Mix(a, b, c) | Select(a, b, c) | SmoothStep(a, b, c) => {
                a.hash(state);
                b.hash(state);
                c.hash(state);
            }
            UserCall { call, component } => {
                call.hash(state);
                component.hash(state);
            }
            TexChannel {
                tex_name,
                channel,
                sample_at,
                decode_mul,
                decode_add,
                decode_expr,
            } => {
                tex_name.hash(state);
                channel.hash(state);
                sample_at.hash(state);
                decode_mul.to_bits().hash(state);
                decode_add.to_bits().hash(state);
                decode_expr.hash(state);
            }
            EffectInputChannel {
                sample_x,
                sample_y,
                channel,
            } => {
                sample_x.hash(state);
                sample_y.hash(state);
                channel.hash(state);
            }
            PathDist { path_id } | PathAlong { path_id } => path_id.hash(state),
            PathTangentComponent { path_id, component } => {
                path_id.hash(state);
                component.hash(state);
            }
            PathPointAtComponent {
                path_id,
                s,
                component,
            }
            | PathTangentAtComponent {
                path_id,
                s,
                component,
            } => {
                path_id.hash(state);
                s.hash(state);
                component.hash(state);
            }
            UniformField {
                binding_name,
                field_name,
                component,
                ..
            } => {
                binding_name.hash(state);
                field_name.hash(state);
                component.hash(state);
            }
            // Unit variants — discriminant already hashed above.
            CoordX
            | CoordY
            | FootprintJ11
            | FootprintJ12
            | FootprintJ21
            | FootprintJ22
            | PostColorR
            | PostColorG
            | PostColorB
            | PostColorA
            | ScatterInstanceId
            | ScatterInstanceIndex01
            | ScatterInstanceAgeNorm
            | ScatterInstancePosX
            | ScatterInstancePosY => {}
            RepeatCellIdX(scope_id)
            | RepeatCellIdY(scope_id)
            | RepeatCellCenterX(scope_id)
            | RepeatCellCenterY(scope_id)
            | RepeatCellUvX(scope_id)
            | RepeatCellUvY(scope_id)
            | RepeatCellRand(scope_id) => scope_id.hash(state),
            Var(name) => name.hash(state),
            DynamicArrayIndex {
                param_name,
                index,
                component,
            } => {
                param_name.hash(state);
                index.hash(state);
                component.hash(state);
            }
            Let { name, value, body } => {
                name.hash(state);
                value.hash(state);
                body.hash(state);
            }
        }
    }
}

pub type V2 = (Sx, Sx);

#[derive(Debug, Clone)]
pub enum Shape {
    Circle {
        center: V2,
        radius: Sx,
    },
    Capsule {
        from: V2,
        to: V2,
        radius: Sx,
    },
    /// Rounded box; `round` is `Sx::Lit(0.0)` for sharp corners.
    RBox {
        center: V2,
        half: V2,
        round: Sx,
    },
    /// Ellipse with separate x and y radii.
    Ellipse {
        center: V2,
        radii: V2,
    },
    /// Star shape with configurable points, outer and inner radii.
    Star {
        center: V2,
        outer: Sx,
        inner: Sx,
        points: u32,
    },
    /// Triangle defined by three arbitrary vertices.
    Triangle {
        a: V2,
        b: V2,
        c: V2,
    },
    /// Generic polygon/path contours with a fill rule.
    ///
    /// Each contour is a closed ring of vertices in authored order.
    Polygon {
        contours: Vec<Vec<V2>>,
        fill_rule: FillRule,
    },
    /// Periodic line family for gridlines.
    ///
    /// `axis = 0` for X-axis (vertical lines), `axis = 1` for Y-axis (horizontal lines).
    /// SDF: d(p) = (|fract(p[axis]/spacing + offset + 0.5) - 0.5|) * spacing
    /// This is 1-Lipschitz exact (§10.1).
    LineFamily {
        axis: u8,
        spacing: Sx,
        offset: Sx,
    },
    /// Single infinite line.
    ///
    /// `axis = 0` uses x-coordinate distance (`|p.x - at|`) and therefore
    /// yields a vertical line. `axis = 1` uses y-coordinate distance and
    /// yields a horizontal line.
    GridLine {
        axis: u8,
        at: Sx,
    },
    /// `stroke(w)` as a *shape*: the outline band `|d| - w/2` (§10.1).
    Outline {
        inner: ShapeId,
        width: Sx,
    },
    /// Signed distance offset: `d' = d - delta`.
    /// Positive `delta` dilates (expands); negative `delta` erodes (shrinks).
    Offset {
        inner: ShapeId,
        delta: Sx,
    },
    /// Rotate a shape around its own anchor by `angle` (radians).
    Rotate {
        inner: ShapeId,
        angle: Sx,
    },
    Mix(ShapeId, ShapeId, Sx),
    Union(ShapeId, ShapeId),
    SmoothUnion(ShapeId, ShapeId, Sx),
    Intersect(ShapeId, ShapeId),
    Subtract(ShapeId, ShapeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    Over,
    Add,
    Screen,
    Multiply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locality {
    Point,
    Local,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CenteredMode {
    Preserve,
    Fit,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAxis {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellLayout {
    Square,
    Brick,
    Hex,
    Jittered,
    Voronoi,
}

#[derive(Debug, Clone, Copy)]
pub struct CellularSpace {
    pub layout: CellLayout,
    pub every: [f32; 2],
    pub seed: u32,
    pub jitter: f32,
    /// Explicit square quadrature grid width (1 through 4).
    pub samples_axis: u32,
    pub cell_scope: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum Xform {
    /// A partition of the plane, not overlapping scatter. Content is clipped
    /// to the selected owner; the enclosing layer is filtered across owners.
    Cellular(CellularSpace),
    /// Radians; content rotates by +angle around canvas center, so the
    /// sample point is rotated by -angle (the inversion of §3 / §12).
    Rotate {
        angle: Sx,
        around: V2,
    },
    Translate(V2),
    /// Pseudo-3D translate in flat perspective space.
    Translate3 {
        by: V2,
        z: Sx,
    },
    /// Uniform scale around an optional pivot (defaults to canvas center `(0.5, 0.5)`).
    Scale {
        factor: Sx,
        around: V2,
    },
    /// Pseudo-3D rotate around X axis in flat perspective space.
    RotateX {
        angle: Sx,
        around: V2,
    },
    /// Pseudo-3D rotate around Y axis in flat perspective space.
    RotateY {
        angle: Sx,
        around: V2,
    },
    /// Repeat along x with period `every` in the current space.
    RepeatX(Sx),
    /// Repeat along y with period `every` in the current space.
    RepeatY(Sx),
    /// Repeat authored space on a 2-D lattice, optionally exposing a named
    /// repeat-cell binding within the immediate `in space` body.
    Repeat2D {
        every: V2,
        cell_scope: Option<u32>,
    },
    /// Repeat around `around` in angle space, splitting into `count` sectors.
    RepeatRadial {
        count: Sx,
        around: V2,
        from: Sx,
        to: Sx,
        /// Optional compile-time sector boundary list used for non-uniform
        /// repeat sectors. Values are radians in ascending order.
        angles: Option<Vec<f32>>,
    },
    /// Flat-only projective camera transform in v1.
    Perspective {
        fov: Sx,
        near: Sx,
        far: Sx,
        origin: V2,
    },
    /// Keep content authored in a target width/height ratio while adapting
    /// to the runtime viewport ratio.
    Aspect(Sx),
    /// Convenience constructor for centered framing. In v0 this maps to
    /// explicit aspect handling.
    Centered {
        mode: CenteredMode,
    },
    /// Declare authored vertical axis orientation for this space.
    ///
    /// Fresco's canonical authored space is `y: up` (origin at bottom-left).
    /// `orientation(y: down)` provides an explicit conversion layer so content
    /// authored in top-left systems remains portable.
    Orientation {
        y: VerticalAxis,
    },
    /// Map straight content coordinates into a polar space around `center`.
    /// `from` is the angle (radians) for x=0, and `clockwise` controls
    /// whether increasing x advances clockwise or counterclockwise.
    Polar {
        center: V2,
        from: Sx,
        clockwise: bool,
    },
    /// Non-linear point-local displacement field, sampled as an inverse warp.
    Warp {
        by: V2,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GradientStop {
    pub at: Sx,
    pub color: ColorExpr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GradientAnchor {
    Scene,
    Shape,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GradientKind {
    Linear { along: V2, anchor: GradientAnchor },
    Radial { center: V2, radius: Sx },
}

/// Shared gradient payload keeps channel expressions compact and visitable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GradientSample {
    pub kind: GradientKind,
    pub stops: Vec<GradientStop>,
    pub at: V2,
}

impl GradientSample {
    pub fn channels(kind: GradientKind, stops: Vec<GradientStop>) -> ColorExpr {
        let sample = Rc::new(Self {
            kind,
            stops,
            at: (Sx::CoordX, Sx::CoordY),
        });
        std::array::from_fn(|channel| Sx::GradientChannel {
            sample: Rc::clone(&sample),
            channel: u8::try_from(channel).expect("RGBA channel fits u8"),
        })
    }

    pub fn map_sx(&self, mut map: impl FnMut(Sx) -> Sx) -> Self {
        let kind = match &self.kind {
            GradientKind::Linear { along, anchor } => GradientKind::Linear {
                along: (map(along.0.clone()), map(along.1.clone())),
                anchor: *anchor,
            },
            GradientKind::Radial { center, radius } => GradientKind::Radial {
                center: (map(center.0.clone()), map(center.1.clone())),
                radius: map(radius.clone()),
            },
        };
        let stops = self
            .stops
            .iter()
            .map(|stop| GradientStop {
                at: map(stop.at.clone()),
                color: stop.color.clone().map(&mut map),
            })
            .collect();
        Self {
            kind,
            stops,
            at: (map(self.at.0.clone()), map(self.at.1.clone())),
        }
    }

    pub fn for_each_sx(&self, visit: &mut impl FnMut(&Sx)) {
        visit(&self.at.0);
        visit(&self.at.1);
        match &self.kind {
            GradientKind::Linear { along, .. } => {
                visit(&along.0);
                visit(&along.1);
            }
            GradientKind::Radial { center, radius } => {
                visit(&center.0);
                visit(&center.1);
                visit(radius);
            }
        }
        for stop in &self.stops {
            visit(&stop.at);
            for channel in &stop.color {
                visit(channel);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ColorSource {
    Solid(ColorExpr),
    Gradient {
        kind: GradientKind,
        stops: Vec<GradientStop>,
    },
}

#[derive(Debug, Clone)]
pub enum GlowReach {
    Scalar(Sx),
    Vec2(V2),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlowFalloff {
    Exp,
    Gaussian,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlowColorSpace {
    Scene,
    Shape,
    Glow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScatterLoweringStrategy {
    Compact,
    BranchTree,
    Procedural,
}

#[derive(Debug, Clone)]
pub enum Layer {
    Solid(Color),
    Fill {
        shape: ShapeId,
        color: Color,
    },
    FillExpr {
        shape: ShapeId,
        r: Sx,
        g: Sx,
        b: Sx,
        a: Sx,
    },
    FillGradient {
        shape: ShapeId,
        kind: GradientKind,
        /// Gradient stops in author order.
        stops: Vec<GradientStop>,
    },
    /// Analytic shadow (§10.2): soften(fill(translate(shape, off)), soften).
    Shadow {
        shape: ShapeId,
        off: V2,
        soften: Sx,
        color: ColorExpr,
    },
    /// Analytic glow (§10.2): strength * exp(-max(d,0)/(reach/3)).
    Glow {
        shape: ShapeId,
        reach: GlowReach,
        strength: Sx,
        color: ColorSource,
        falloff: GlowFalloff,
        color_space: GlowColorSpace,
    },
    /// Edge glow constrained to the shape interior.
    InnerGlow {
        shape: ShapeId,
        reach: GlowReach,
        strength: Sx,
        color: ColorSource,
        falloff: GlowFalloff,
        color_space: GlowColorSpace,
    },
    /// Simple analytic bevel shading around the edge band.
    Bevel {
        shape: ShapeId,
        width: Sx,
        light: V2,
        strength: Sx,
        highlight: ColorExpr,
        shadow: Box<ColorExpr>,
    },
    /// Approximate glow applied to an already-rendered layer in one pass.
    GlowFx {
        inner: LayerId,
        reach: GlowReach,
        strength: Sx,
        color: ColorSource,
        falloff: GlowFalloff,
    },
    /// Soft-edged fill; the *target* of the `blur ∘ fill ⇒ soften` rewrite.
    Soften {
        shape: ShapeId,
        radius: Sx,
        color: ColorExpr,
    },
    /// Layer-level blur; analytic rewrites are preferred (§10.2/§10.3).
    ///
    /// If no analytic rewrite fires in `rewrite::run`, this node survives with
    /// `Locality::Local` so the pass partitioner selects a kernel strategy
    /// (§11 step 3).  Lowering emits an inline N-tap approximation; for large
    /// radii the manifest describes the ideal separable-HV or downsample-chain
    /// schedule for a runtime shim.
    Blur {
        inner: LayerId,
        radius: Sx,
        /// Source span retained for future diagnostics (pass-cut warnings etc.).
        #[allow(
            dead_code,
            reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
        )]
        span: Span,
    },
    /// Directional motion blur modeled as line integration over `offset`.
    ///
    /// Lowering samples `inner` along the shutter segment centered at the
    /// current point: `p + t * offset`, where `t in [-0.5, 0.5]`.
    MotionBlur {
        inner: LayerId,
        shutter: Sx,
        offset: V2,
        /// Source span retained for future diagnostics (pass-cut warnings etc.).
        #[allow(
            dead_code,
            reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
        )]
        span: Span,
    },
    Opacity {
        inner: LayerId,
        alpha: Sx,
    },
    ColorExpr {
        r: Sx,
        g: Sx,
        b: Sx,
        a: Sx,
    },
    Grey {
        value: Sx,
    },
    /// Sample a 2D texture at the current coordinate.
    /// The texture name is registered in `Hir::textures`.
    Image {
        tex_name: String,
    },
    /// Sample a 2D texture at an explicit coordinate expression.
    /// The texture name is registered in `Hir::textures`.
    ImageAt {
        tex_name: String,
        sample_x: Sx,
        sample_y: Sx,
    },
    /// Color tint: blends a layer towards a solid color by `amount` ∈ [0,1].
    /// At amount=0 the layer is unchanged; at amount=1 it becomes the tint color.
    Tint {
        inner: LayerId,
        color: ColorExpr,
        amount: Sx,
    },
    /// Run a point-local color function over the composed result of `inner`.
    PostProcess {
        inner: LayerId,
        rgba: ColorExpr,
    },
    ScatterBins {
        min: (Sx, Sx),
        max: (Sx, Sx),
        bins_x: usize,
        bins_y: usize,
        strategy: ScatterLoweringStrategy,
        lifecycle: Option<ScatterLifecycleParams>,
        bins: Vec<Vec<ScatterInstance>>,
        body: LayerId,
    },
    InSpace {
        xforms: Vec<Xform>,
        inner: LayerId,
    },
    If {
        cond: Sx,
        then_layer: LayerId,
        else_layer: LayerId,
    },
    Compose(Vec<(LayerId, Blend)>),
    /// A user-defined effect application (§16.1).
    ///
    /// `def_idx` indexes into `Hir::effects`.  `inner` is the piped-in layer
    /// (the layer the effect is applied to), if any.  `args` are the evaluated
    /// scalar parameter values passed at the call site.
    UserEffect {
        def_idx: usize,
        inner: Option<LayerId>,
        args: Vec<Sx>,
        /// Source span retained for diagnostics.
        #[allow(
            dead_code,
            reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
        )]
        span: Span,
    },
}

/// Borrowed metadata for a layer node that imposes a temporal-purity constraint
/// on its input subgraph.
#[derive(Debug, Clone, Copy)]
pub struct TemporalPurityRequirementRef<'a> {
    pub root: LayerId,
    pub span: &'a Span,
    pub owner_name: &'static str,
}

impl Hir {
    /// The single shape that defines an effect's local color frame, if any.
    pub fn color_shape(&self, id: LayerId) -> Option<ShapeId> {
        match &self.layers[id] {
            Layer::Fill { shape, .. }
            | Layer::FillExpr { shape, .. }
            | Layer::FillGradient { shape, .. }
            | Layer::Soften { shape, .. }
            | Layer::Shadow { shape, .. }
            | Layer::Glow { shape, .. }
            | Layer::InnerGlow { shape, .. }
            | Layer::Bevel { shape, .. } => Some(*shape),
            Layer::Tint { inner, .. }
            | Layer::Opacity { inner, .. }
            | Layer::Blur { inner, .. }
            | Layer::MotionBlur { inner, .. }
            | Layer::PostProcess { inner, .. } => self.color_shape(*inner),
            _ => None,
        }
    }
}

impl Layer {
    /// Visit color expressions owned by this node (not its input layers).
    pub fn for_each_color_sx(&self, mut visit: impl FnMut(&Sx)) {
        match self {
            Layer::FillExpr { r, g, b, a, .. } | Layer::ColorExpr { r, g, b, a } => {
                for channel in [r, g, b, a] {
                    visit(channel);
                }
            }
            Layer::Shadow { color, .. }
            | Layer::Soften { color, .. }
            | Layer::Tint { color, .. }
            | Layer::PostProcess { rgba: color, .. } => {
                for channel in color {
                    visit(channel);
                }
            }
            Layer::Bevel {
                highlight, shadow, ..
            } => {
                for channel in highlight.iter().chain(shadow.iter()) {
                    visit(channel);
                }
            }
            Layer::Glow { color, .. }
            | Layer::InnerGlow { color, .. }
            | Layer::GlowFx { color, .. } => match color {
                ColorSource::Solid(color) => {
                    for channel in color {
                        visit(channel);
                    }
                }
                ColorSource::Gradient { stops, .. } => {
                    for stop in stops {
                        for channel in &stop.color {
                            visit(channel);
                        }
                    }
                }
            },
            Layer::FillGradient { stops, .. } => {
                for stop in stops {
                    for channel in &stop.color {
                        visit(channel);
                    }
                }
            }
            Layer::Solid(_)
            | Layer::Fill { .. }
            | Layer::Blur { .. }
            | Layer::MotionBlur { .. }
            | Layer::Opacity { .. }
            | Layer::Grey { .. }
            | Layer::Image { .. }
            | Layer::ImageAt { .. }
            | Layer::ScatterBins { .. }
            | Layer::InSpace { .. }
            | Layer::If { .. }
            | Layer::Compose(_)
            | Layer::UserEffect { .. } => {}
        }
    }

    /// Return temporal-purity requirement metadata for layers that must evaluate
    /// correctly under time-shifted sampling.
    pub fn temporal_purity_requirement(&self) -> Option<TemporalPurityRequirementRef<'_>> {
        match self {
            Layer::MotionBlur { inner, span, .. } => Some(TemporalPurityRequirementRef {
                root: *inner,
                span,
                owner_name: "motion_blur",
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ScatterInstance {
    pub id: u32,
    pub index01: f32,
    pub pos: (f32, f32),
    pub footprint: f32,
}

#[derive(Debug, Clone)]
pub struct ScatterLifecycleParams {
    pub lifetime: Sx,
    pub respawn_every: Sx,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty_name: String,
    pub default: ParamDefault,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

/// Result of parsing an array parameter type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayParamSize {
    /// Fixed-size array with compile-time known length: `array<f32, 5>`
    Fixed(usize),
    /// Dynamic array with runtime length: `array<f32>`
    Dynamic,
}

/// Parse an array parameter type string.
/// Returns the element type name and size (Fixed or Dynamic).
///
/// Examples:
/// - `array<f32, 5>` -> `Some(("f32", ArrayParamSize::Fixed(5)))`
/// - `array<f32>` -> `Some(("f32", ArrayParamSize::Dynamic))`
pub fn parse_array_param_type_ex(ty_name: &str) -> Option<(&str, ArrayParamSize)> {
    let inner = ty_name.strip_prefix("array<")?.strip_suffix('>')?;

    // Commas in nested type arguments belong to the element type.
    let mut depth = 0usize;
    let mut separator = None;
    for (index, ch) in inner.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                separator = Some(index);
                break;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    if let Some(index) = separator {
        let (elem_ty, suffix) = inner.split_at(index);
        let len_text = &suffix[1..];
        // Fixed-size array: array<T, N>
        let elem_ty = elem_ty.trim();
        let len = len_text.trim().parse::<usize>().ok()?;
        Some((elem_ty, ArrayParamSize::Fixed(len)))
    } else {
        // Dynamic array: array<T>
        let elem_ty = inner.trim();
        Some((elem_ty, ArrayParamSize::Dynamic))
    }
}

/// Legacy helper for fixed-size arrays only. Returns None for dynamic arrays.
/// Prefer `parse_array_param_type_ex` for new code that needs to handle both.
pub fn parse_array_param_type(ty_name: &str) -> Option<(&str, usize)> {
    match parse_array_param_type_ex(ty_name)? {
        (elem_ty, ArrayParamSize::Fixed(len)) => Some((elem_ty, len)),
        (_, ArrayParamSize::Dynamic) => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextureMetadata {
    pub default_asset: Option<String>,
    /// Name of the `texture_type` definition that describes this texture's channel layout.
    /// `None` means an untyped (plain) texture.
    pub texture_type_name: Option<String>,
}

/// One channel in a `texture_type` definition, after semantic analysis.
#[derive(Debug, Clone)]
pub struct TextureChannelDef {
    /// RGBA channel index: 0=r, 1=g, 2=b, 3=a.
    pub channel_idx: u8,
    /// Raw channel letter from the texture_type declaration (`r`, `g`, `b`, or `a`).
    pub channel_name: String,
    /// User-defined semantic name (e.g. `"roughness"`, `"metallic"`).
    pub semantic_name: String,
    /// Affine decode applied to the sampled channel: `decoded = raw * mul + add`.
    pub decode_mul: f32,
    pub decode_add: f32,
    /// Optional rich decode expression using `Sx::Var("raw")` as sampled channel input.
    pub decode_expr: Option<Sx>,
}

/// A compiled `texture_type` definition stored in the HIR.
#[derive(Debug, Clone)]
pub struct TextureTypeDef {
    pub channels: Vec<TextureChannelDef>,
    pub result_expr: Option<crate::ast::SExpr>,
}

impl TextureTypeDef {
    /// Find the channel definition for a given semantic name, if any.
    pub fn channel_def_for(&self, name: &str) -> Option<&TextureChannelDef> {
        self.channels.iter().find(|c| c.semantic_name == name)
    }

    /// Find the channel index for a given semantic name, if any.
    pub fn channel_for(&self, name: &str) -> Option<u8> {
        self.channel_def_for(name).map(|c| c.channel_idx)
    }
}

/// A compiled rewrite rule stored in the HIR after semantic analysis.
/// Both sides reference effects by name (resolved at check time).
#[derive(Debug, Clone)]
pub struct CompiledRewriteRule {
    pub outer_name: String,
    /// Hole names bound from outer pattern.
    pub outer_param_names: Vec<String>,
    pub inner_name: String,
    /// Hole names bound from inner pattern.
    pub inner_param_names: Vec<String>,
    pub result_name: String,
    /// Hole names used in result pattern (must be subset of outer ∪ inner).
    pub result_param_names: Vec<String>,
    /// Optional guard: a compiled `Sx` expression with `Sx::Var(name)` leaves
    /// representing pattern holes.  `None` means the rule fires unconditionally
    /// when the pattern matches.  `Some(sx)` is evaluated at rewrite time after
    /// substituting actual argument constant values; the rule fires only when
    /// the guard evaluates to a non-zero value.  If the arguments are not
    /// statically known constants, the rule is skipped conservatively.
    pub guard_expr: Option<Sx>,
    /// Optional numeric rewrite tolerance declared as `within ε`.
    /// This is checker-validated to be a positive compile-time constant.
    pub tolerance: Option<f32>,
}

/// The HIR-level representation of a user-defined effect after checking.
#[derive(Debug, Clone)]
pub struct EffectDef {
    pub name: String,
    pub locality: Locality,
    /// Optional local radius expression from `local(expr)` after effect-body
    /// checking, with parameters represented as `Sx::Var(name)`.
    pub locality_radius: Option<Sx>,
    pub param_names: Vec<String>,
    #[allow(
        dead_code,
        reason = "param_types retained for future type-checking diagnostics"
    )]
    pub param_types: Vec<String>,
    /// The `LayerId` of the checked body expression (the effect's layer tree).
    pub body_layer: LayerId,
    pub rewrites: Vec<CompiledRewriteRule>,
    /// Source span for diagnostic messages.
    #[allow(dead_code, reason = "span retained for future per-site diagnostics")]
    pub span: Span,
}

/// Engine-authored filtering choices. Deliberately has no Default implementation.
#[derive(Debug, Clone, Copy)]
pub struct RenderingPolicy {
    pub shape_aa_min_px: f32,
    pub shape_aa_max_px: f32,
    pub shape_aa_style: ShapeAaStyle,
    pub projective_footprint_max_px: f32,
}

#[derive(Debug)]
pub struct Hir {
    pub entry_context: Option<crate::context::EntryContext>,
    pub name: String,
    pub params: Vec<Param>,
    /// Absent only in helper-only checking contexts that do not render.
    pub rendering_policy: Option<RenderingPolicy>,
    pub canvas_space: Option<Vec<Xform>>,
    /// Jacobian matrix from canvas_space chain for footprint computation.
    /// Components are `(j11, j12, j21, j22)` representing the 2×2 derivative matrix.
    pub canvas_jacobian: Option<(Sx, Sx, Sx, Sx)>,
    pub shapes: Vec<Shape>,
    pub layers: Vec<Layer>,
    pub layer_locality: Vec<Locality>,
    pub root: LayerId,
    /// Human-readable notes accumulated by rewrite/lowering for `--explain`.
    pub notes: Vec<String>,
    /// Specialization notes for generic function instantiations, for `--explain`.
    pub specialization_notes: Vec<String>,
    /// Unique texture names referenced by `Layer::Image` in declaration order.
    /// The runtime must bind a `texture_2d<f32>` + `sampler` for each entry.
    pub textures: Vec<String>,
    /// Reverse index for O(1) lookup in [`register_texture`].
    /// Kept in sync with `textures`: `texture_index[name] == textures.iter().position(name)`.
    pub(crate) texture_index: HashMap<String, usize>,
    /// Optional metadata for named textures, keyed by texture name.
    pub texture_metadata: HashMap<String, TextureMetadata>,
    /// All `texture_type` definitions declared in this program, keyed by type name.
    pub texture_type_defs: HashMap<String, TextureTypeDef>,
    /// User helper functions emitted as reusable WGSL functions and invoked via
    /// first-class call expressions in scalar lowering.
    pub user_helpers: HashMap<String, UserFnHelper>,
    /// Path segment tables captured at check time for lowering-side backend
    /// materialization (module constants vs buffers).
    pub path_profiles: Vec<PathProfile>,
    /// User-defined effect definitions in declaration order.
    pub effects: Vec<EffectDef>,
    /// Index from effect name to position in `effects` for O(1) lookup.
    pub effect_by_name: HashMap<String, usize>,
    /// Struct-typed global `param` declarations visible to this program, in
    /// declaration order: `(name, ty_name, fields)`. Carried for lowering
    /// (real `var<uniform>` buffer creation, shared across every
    /// canvas/surface in the compile) and manifest emission.
    pub global_uniforms: Vec<crate::check::GlobalUniformDef>,
}

impl Hir {
    pub fn shape(&mut self, s: Shape) -> ShapeId {
        self.shapes.push(s);
        self.shapes.len() - 1
    }
    pub fn layer(&mut self, l: Layer) -> LayerId {
        self.layers.push(l);
        self.layer_locality.push(Locality::Point);
        self.layers.len() - 1
    }

    pub fn register_path_profile(&mut self, profile: PathProfile) -> usize {
        let id = self.path_profiles.len();
        self.path_profiles.push(profile);
        id
    }
    /// Register a texture name and return its 0-based index in `self.textures`.
    /// If the name is already registered, returns the existing index.
    /// O(1) via the companion `texture_index` map.
    pub fn register_texture(&mut self, name: &str) -> usize {
        if let Some(&idx) = self.texture_index.get(name) {
            return idx;
        }
        let idx = self.textures.len();
        self.textures.push(name.to_string());
        self.texture_index.insert(name.to_string(), idx);
        idx
    }

    pub fn set_texture_default_asset(&mut self, name: &str, default_asset: String) {
        self.register_texture(name);
        let meta = self.texture_metadata.entry(name.to_string()).or_default();
        meta.default_asset = Some(default_asset);
    }

    pub fn texture_default_asset(&self, name: &str) -> Option<&str> {
        self.texture_metadata
            .get(name)
            .and_then(|meta| meta.default_asset.as_deref())
    }
}

#[cfg(test)]
mod numeric_tests {
    #[test]
    fn selection_does_not_arithmetically_mix_the_rejected_value() {
        use super::Sx;
        let selected = Sx::Select(
            Box::new(Sx::Lit(f32::NAN)),
            Box::new(Sx::Lit(7.0)),
            Box::new(Sx::Lit(1.0)),
        );
        assert_eq!(
            selected.try_eval_with_vars(&std::collections::HashMap::new()),
            Some(7.0)
        );
        let selected = Sx::Select(
            Box::new(Sx::Lit(9.0)),
            Box::new(Sx::Lit(f32::INFINITY)),
            Box::new(Sx::Lit(0.0)),
        );
        assert_eq!(
            selected.try_eval_with_vars(&std::collections::HashMap::new()),
            Some(9.0)
        );
    }

    #[test]
    fn array_type_sizes_do_not_consume_nested_element_arguments() {
        assert_eq!(
            super::parse_array_param_type("array<array<f32,2>,3>"),
            Some(("array<f32,2>", 3))
        );
        assert!(matches!(
            super::parse_array_param_type_ex("array<array<f32,2>>"),
            Some(("array<f32,2>", super::ArrayParamSize::Dynamic))
        ));
        assert!(super::parse_array_param_type_ex("array<array<f32,2>").is_none());
    }

    use super::split_cubic_half;

    #[test]
    fn subdivision_preserves_a_degenerate_curve_at_large_coordinates() {
        let point = (f32::MAX, -f32::MAX);
        let (left, right) = split_cubic_half(point, point, point, point);
        assert_eq!(left, (point, point, point, point));
        assert_eq!(right, (point, point, point, point));
    }
}
