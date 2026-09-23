use super::*;
use crate::hir::{ColorSource, GlowFalloff, GradientAnchor, GradientKind, GradientStop, Sx};
use std::rc::Rc;

impl<'h> FnCtx<'h> {
    pub(super) fn native_scalar(&mut self, value: &Sx, p: Handle<Ex>) -> Handle<Ex> {
        match value {
            Sx::Typed(value) => self.typed_scalar(value, p),
            value => self.sx_at(value, p),
        }
    }

    pub(super) fn typed_scalar(
        &mut self,
        value: &crate::typed_scalar::Scalar,
        p: Handle<Ex>,
    ) -> Handle<Ex> {
        let args = value
            .args
            .iter()
            .map(|arg| match arg {
                Sx::Typed(value) => self.typed_scalar(value, p),
                value => self.sx_at(value, p),
            })
            .collect::<Vec<_>>();
        value
            .emit(&args, |e| Ok::<_, std::convert::Infallible>(self.ir.add(e)))
            .unwrap()
    }

    fn sx_vec_is_sample_point_invariant(&mut self, v: &crate::hir::SxVec) -> bool {
        match v {
            crate::hir::SxVec::V2(v) => {
                self.sx_is_sample_point_invariant(&v.0) && self.sx_is_sample_point_invariant(&v.1)
            }
            crate::hir::SxVec::V3(v) => {
                self.sx_is_sample_point_invariant(&v.0)
                    && self.sx_is_sample_point_invariant(&v.1)
                    && self.sx_is_sample_point_invariant(&v.2)
            }
            crate::hir::SxVec::V4(v) => {
                self.sx_is_sample_point_invariant(&v.0)
                    && self.sx_is_sample_point_invariant(&v.1)
                    && self.sx_is_sample_point_invariant(&v.2)
                    && self.sx_is_sample_point_invariant(&v.3)
            }
        }
    }

    fn user_helper_is_sample_point_invariant(&mut self, helper_id: &str) -> bool {
        if let Some(cached) = self.invariant_user_helper_cache.get(helper_id).copied() {
            return cached;
        }
        let invariant = self
            .hir
            .user_helpers
            .get(helper_id)
            .map(|helper| helper.sample_point_invariant)
            .unwrap_or(false);
        self.invariant_user_helper_cache
            .insert(helper_id.to_string(), invariant);
        invariant
    }

    pub(super) fn sx_is_sample_point_invariant(&mut self, s: &Sx) -> bool {
        self.sx_is_sample_point_invariant_inner(s)
    }

    fn sx_is_sample_point_invariant_inner(&mut self, s: &Sx) -> bool {
        match s {
            Sx::Typed(value) => value
                .args
                .iter()
                .all(|arg| self.sx_is_sample_point_invariant_inner(arg)),
            Sx::Lit(_) | Sx::EntryInput(_) | Sx::UniformField { .. } => true,
            Sx::Param(_) => self.params_are_immutable,
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
            | Sx::SrgbToLinear(a)
            | Sx::LinearToSrgb(a) => self.sx_is_sample_point_invariant_inner(a),
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
                self.sx_is_sample_point_invariant_inner(a)
                    && self.sx_is_sample_point_invariant_inner(b)
            }
            Sx::Clamp(x, lo, hi)
            | Sx::Mix(x, lo, hi)
            | Sx::Select(x, lo, hi)
            | Sx::SmoothStep(x, lo, hi) => {
                self.sx_is_sample_point_invariant_inner(x)
                    && self.sx_is_sample_point_invariant_inner(lo)
                    && self.sx_is_sample_point_invariant_inner(hi)
            }
            Sx::Dot { a, b } => {
                self.sx_vec_is_sample_point_invariant(a) && self.sx_vec_is_sample_point_invariant(b)
            }
            Sx::NormalizeComponent { v, .. } | Sx::Length(v) => {
                self.sx_vec_is_sample_point_invariant(v)
            }
            Sx::MinComponent { a, b, .. } | Sx::MaxComponent { a, b, .. } => {
                self.sx_vec_is_sample_point_invariant(a) && self.sx_vec_is_sample_point_invariant(b)
            }
            Sx::ClampVecComponent { x, lo, hi, .. } => {
                self.sx_vec_is_sample_point_invariant(x)
                    && self.sx_vec_is_sample_point_invariant(lo)
                    && self.sx_vec_is_sample_point_invariant(hi)
            }
            Sx::UserCall { call, .. } => {
                call.args
                    .iter()
                    .all(|arg| self.sx_is_sample_point_invariant_inner(arg))
                    && self.user_helper_is_sample_point_invariant(&call.helper_id)
            }
            // Dynamic array indexing: invariant if the index expression is invariant
            // (the array data itself lives in a storage buffer and never varies).
            Sx::DynamicArrayIndex { index, .. } => self.sx_is_sample_point_invariant_inner(index),
            _ => false,
        }
    }

    fn sx_lit_value(sx: &Sx) -> Option<f32> {
        match sx {
            Sx::Lit(value) => Some(*value),
            _ => None,
        }
    }

    fn is_sx_lit(sx: &Sx, expected: f32) -> bool {
        matches!(sx, Sx::Lit(value) if value.to_bits() == expected.to_bits())
    }

    fn rgb_expr_equal(a: &crate::hir::ColorExpr, b: &crate::hir::ColorExpr) -> bool {
        a[0] == b[0] && a[1] == b[1] && a[2] == b[2]
    }

    fn axis_aligned_linear_gradient_component(
        &mut self,
        along: &(Sx, Sx),
        p: Handle<Ex>,
    ) -> Option<Handle<Ex>> {
        if Self::is_sx_lit(&along.0, 1.0) && Self::is_sx_lit(&along.1, 0.0) {
            Some(self.ir.x_of(p))
        } else if Self::is_sx_lit(&along.0, 0.0) && Self::is_sx_lit(&along.1, 1.0) {
            Some(self.ir.y_of(p))
        } else {
            None
        }
    }

    pub(super) fn local_pixel_span(&mut self, p: Handle<Ex>) -> Handle<Ex> {
        // Periodic/remapped spaces (repeat, polar seam, repeat-cell scopes)
        // introduce coordinate discontinuities. Derivatives of remapped
        // coordinates can spike there and cause visible seams when used as
        // AA/pixel span estimates. In those scopes, fall back to screen-space px.
        if self.discontinuity_space_depth > 0 || !self.repeat_cell_ctx.is_empty() {
            return self.ir.px;
        }

        if let Some(cached) = self.local_pixel_span_cache.get(&p).copied() {
            return cached;
        }

        // Convert one screen pixel into local sample-space units at `p`.
        // This keeps `px` authored values stable under active `in space` transforms.
        let dpdx_p = self.ir.dpdx(p);
        let dpdy_p = self.ir.dpdy(p);
        let span_x = self.ir.m1(Mf::Length, dpdx_p);
        let span_y = self.ir.m1(Mf::Length, dpdy_p);
        let span = self.ir.m2(Mf::Max, span_x, span_y);
        let span_min = self.clamp_positive(span);

        // Perspective singularities can spike derivatives and cause sweeping
        // AA/px-width artifacts. Cap local pixel span to a finite multiple of
        // screen-space px so numeric blow-ups cannot flood coverage ramps.
        let cap_mul = self
            .ir
            .lit(self.rendering_policy().projective_footprint_max_px);
        let span_cap = self.ir.mul(cap_mul, self.ir.px);
        let local_px = self.ir.m2(Mf::Min, span_min, span_cap);
        self.local_pixel_span_cache.insert(p, local_px);
        local_px
    }

    pub(super) const SCATTER_BIN_RUNTIME_CAP: usize = 8;
    pub(super) const SCATTER_PROCEDURAL_SAMPLE_CAP_BASE: usize = 128;
    pub(super) const SCATTER_PROCEDURAL_SAMPLE_CAP_HARD_MAX: usize = 256;

    fn param_base_max_access_index(&self, base: Handle<Ex>) -> Option<u32> {
        self.ir
            .param_scalars
            .values()
            .filter_map(|h| {
                if self.ir.is_scalar_slot_ptr(*h) {
                    return None;
                }
                let (candidate_base, index) = self.ir.access_index_of(*h)?;
                (candidate_base == base).then_some(index)
            })
            .max()
    }

    fn try_param_vector2(&self, v: &(Sx, Sx)) -> Option<Handle<Ex>> {
        let (Sx::Param(x), Sx::Param(y)) = (&v.0, &v.1) else {
            return None;
        };
        let xh = *self.ir.param_scalars.get(x)?;
        let yh = *self.ir.param_scalars.get(y)?;
        if self.ir.is_scalar_slot_ptr(xh) || self.ir.is_scalar_slot_ptr(yh) {
            return None;
        }
        let (xb, xi) = self.ir.access_index_of(xh)?;
        let (yb, yi) = self.ir.access_index_of(yh)?;
        if xb == yb && xi == 0 && yi == 1 && self.param_base_max_access_index(xb) == Some(1) {
            Some(xb)
        } else {
            None
        }
    }

    fn try_param_vector3(&self, v: &(Sx, Sx, Sx)) -> Option<Handle<Ex>> {
        let (Sx::Param(x), Sx::Param(y), Sx::Param(z)) = (&v.0, &v.1, &v.2) else {
            return None;
        };
        let xh = *self.ir.param_scalars.get(x)?;
        let yh = *self.ir.param_scalars.get(y)?;
        let zh = *self.ir.param_scalars.get(z)?;
        if self.ir.is_scalar_slot_ptr(xh)
            || self.ir.is_scalar_slot_ptr(yh)
            || self.ir.is_scalar_slot_ptr(zh)
        {
            return None;
        }
        let (xb, xi) = self.ir.access_index_of(xh)?;
        let (yb, yi) = self.ir.access_index_of(yh)?;
        let (zb, zi) = self.ir.access_index_of(zh)?;
        if xb == yb
            && yb == zb
            && xi == 0
            && yi == 1
            && zi == 2
            && self.param_base_max_access_index(xb) == Some(2)
        {
            Some(xb)
        } else {
            None
        }
    }

    fn try_param_vector4(&self, v: &(Sx, Sx, Sx, Sx)) -> Option<Handle<Ex>> {
        let (Sx::Param(x), Sx::Param(y), Sx::Param(z), Sx::Param(w)) = (&v.0, &v.1, &v.2, &v.3)
        else {
            return None;
        };
        let xh = *self.ir.param_scalars.get(x)?;
        let yh = *self.ir.param_scalars.get(y)?;
        let zh = *self.ir.param_scalars.get(z)?;
        let wh = *self.ir.param_scalars.get(w)?;
        if self.ir.is_scalar_slot_ptr(xh)
            || self.ir.is_scalar_slot_ptr(yh)
            || self.ir.is_scalar_slot_ptr(zh)
            || self.ir.is_scalar_slot_ptr(wh)
        {
            return None;
        }
        let (xb, xi) = self.ir.access_index_of(xh)?;
        let (yb, yi) = self.ir.access_index_of(yh)?;
        let (zb, zi) = self.ir.access_index_of(zh)?;
        let (wb, wi) = self.ir.access_index_of(wh)?;
        if xb == yb
            && yb == zb
            && zb == wb
            && xi == 0
            && yi == 1
            && zi == 2
            && wi == 3
            && self.param_base_max_access_index(xb) == Some(3)
        {
            Some(xb)
        } else {
            None
        }
    }

    fn vec_from_sxvec(&mut self, v: &crate::hir::SxVec, p: Handle<Ex>) -> Handle<Ex> {
        use crate::hir::SxVec;
        match v {
            SxVec::V2(v) => {
                match (&v.0, &v.1) {
                    (
                        Sx::NormalizeComponent {
                            v: inner_v,
                            index: 0,
                        },
                        Sx::NormalizeComponent { index: 1, .. },
                    ) => {
                        let vec = self.vec_from_sxvec(inner_v, p);
                        return self.ir.m1(Mf::Normalize, vec);
                    }
                    (Sx::MinComponent { a, b, index: 0 }, Sx::MinComponent { index: 1, .. }) => {
                        let av = self.vec_from_sxvec(a, p);
                        let bv = self.vec_from_sxvec(b, p);
                        return self.ir.m2(Mf::Min, av, bv);
                    }
                    (Sx::MaxComponent { a, b, index: 0 }, Sx::MaxComponent { index: 1, .. }) => {
                        let av = self.vec_from_sxvec(a, p);
                        let bv = self.vec_from_sxvec(b, p);
                        return self.ir.m2(Mf::Max, av, bv);
                    }
                    (
                        Sx::ClampVecComponent {
                            x,
                            lo,
                            hi,
                            index: 0,
                        },
                        Sx::ClampVecComponent { index: 1, .. },
                    ) => {
                        let xv = self.vec_from_sxvec(x, p);
                        let lv = self.vec_from_sxvec(lo, p);
                        let hv = self.vec_from_sxvec(hi, p);
                        return self.ir.m3(Mf::Clamp, xv, lv, hv);
                    }
                    _ => {}
                }
                if let Some(base) = self.try_param_vector2(v) {
                    return base;
                }
                let (x, y) = &**v;
                let xh = self.sx_at(x, p);
                let yh = self.sx_at(y, p);
                self.ir.add(Ex::Compose {
                    ty: self.ir.types.v2,
                    components: vec![xh, yh],
                })
            }
            SxVec::V3(v) => {
                match (&v.0, &v.1, &v.2) {
                    (
                        Sx::NormalizeComponent {
                            v: inner_v,
                            index: 0,
                        },
                        Sx::NormalizeComponent { index: 1, .. },
                        Sx::NormalizeComponent { index: 2, .. },
                    ) => {
                        let vec = self.vec_from_sxvec(inner_v, p);
                        return self.ir.m1(Mf::Normalize, vec);
                    }
                    (
                        Sx::MinComponent { a, b, index: 0 },
                        Sx::MinComponent { index: 1, .. },
                        Sx::MinComponent { index: 2, .. },
                    ) => {
                        let av = self.vec_from_sxvec(a, p);
                        let bv = self.vec_from_sxvec(b, p);
                        return self.ir.m2(Mf::Min, av, bv);
                    }
                    (
                        Sx::MaxComponent { a, b, index: 0 },
                        Sx::MaxComponent { index: 1, .. },
                        Sx::MaxComponent { index: 2, .. },
                    ) => {
                        let av = self.vec_from_sxvec(a, p);
                        let bv = self.vec_from_sxvec(b, p);
                        return self.ir.m2(Mf::Max, av, bv);
                    }
                    (
                        Sx::ClampVecComponent {
                            x,
                            lo,
                            hi,
                            index: 0,
                        },
                        Sx::ClampVecComponent { index: 1, .. },
                        Sx::ClampVecComponent { index: 2, .. },
                    ) => {
                        let xv = self.vec_from_sxvec(x, p);
                        let lv = self.vec_from_sxvec(lo, p);
                        let hv = self.vec_from_sxvec(hi, p);
                        return self.ir.m3(Mf::Clamp, xv, lv, hv);
                    }
                    _ => {}
                }
                if let Some(base) = self.try_param_vector3(v) {
                    return base;
                }
                let (x, y, z) = &**v;
                let xh = self.sx_at(x, p);
                let yh = self.sx_at(y, p);
                let zh = self.sx_at(z, p);
                self.ir.add(Ex::Compose {
                    ty: self.ir.types.v3,
                    components: vec![xh, yh, zh],
                })
            }
            SxVec::V4(v) => {
                match (&v.0, &v.1, &v.2, &v.3) {
                    (
                        Sx::NormalizeComponent {
                            v: inner_v,
                            index: 0,
                        },
                        Sx::NormalizeComponent { index: 1, .. },
                        Sx::NormalizeComponent { index: 2, .. },
                        Sx::NormalizeComponent { index: 3, .. },
                    ) => {
                        let vec = self.vec_from_sxvec(inner_v, p);
                        return self.ir.m1(Mf::Normalize, vec);
                    }
                    (
                        Sx::MinComponent { a, b, index: 0 },
                        Sx::MinComponent { index: 1, .. },
                        Sx::MinComponent { index: 2, .. },
                        Sx::MinComponent { index: 3, .. },
                    ) => {
                        let av = self.vec_from_sxvec(a, p);
                        let bv = self.vec_from_sxvec(b, p);
                        return self.ir.m2(Mf::Min, av, bv);
                    }
                    (
                        Sx::MaxComponent { a, b, index: 0 },
                        Sx::MaxComponent { index: 1, .. },
                        Sx::MaxComponent { index: 2, .. },
                        Sx::MaxComponent { index: 3, .. },
                    ) => {
                        let av = self.vec_from_sxvec(a, p);
                        let bv = self.vec_from_sxvec(b, p);
                        return self.ir.m2(Mf::Max, av, bv);
                    }
                    (
                        Sx::ClampVecComponent {
                            x,
                            lo,
                            hi,
                            index: 0,
                        },
                        Sx::ClampVecComponent { index: 1, .. },
                        Sx::ClampVecComponent { index: 2, .. },
                        Sx::ClampVecComponent { index: 3, .. },
                    ) => {
                        let xv = self.vec_from_sxvec(x, p);
                        let lv = self.vec_from_sxvec(lo, p);
                        let hv = self.vec_from_sxvec(hi, p);
                        return self.ir.m3(Mf::Clamp, xv, lv, hv);
                    }
                    _ => {}
                }
                if let Some(base) = self.try_param_vector4(v) {
                    return base;
                }
                let (x, y, z, w) = &**v;
                let xh = self.sx_at(x, p);
                let yh = self.sx_at(y, p);
                let zh = self.sx_at(z, p);
                let wh = self.sx_at(w, p);
                self.ir.add(Ex::Compose {
                    ty: self.ir.types.v4,
                    components: vec![xh, yh, zh, wh],
                })
            }
        }
    }

    // ------------------------------------------------------- HIR scalars
    pub(super) fn repeat_cell_ctx(&self, scope_id: u32) -> RepeatCellCtx {
        self.repeat_cell_ctx
            .iter()
            .rev()
            .copied()
            .find(|ctx| ctx.scope_id == scope_id)
            .expect("repeat cell binding used outside repeat-cell lowering scope")
    }

    pub(super) fn sx_at(&mut self, s: &Sx, p: Handle<Ex>) -> Handle<Ex> {
        if self.cell_inset_pixel_span.is_some() {
            return self.sx_at_uncached(s, p);
        }
        if self.sx_is_sample_point_invariant(s) {
            if let Some(cached) = self.invariant_sx_cache.get(s).copied() {
                return cached;
            }
            let lowered = self.sx_at_uncached(s, p);
            self.invariant_sx_cache.insert(s.clone(), lowered);
            return lowered;
        }

        if s.is_cacheable_at_point() {
            let key = (s.clone(), p);
            if let Some(cached) = self.sx_cache.get(&key).copied() {
                return cached;
            }
            let lowered = self.sx_at_uncached(s, p);
            self.sx_cache.insert(key, lowered);
            return lowered;
        }

        self.sx_at_uncached(s, p)
    }

    pub(super) fn sx_at_uncached(&mut self, s: &Sx, p: Handle<Ex>) -> Handle<Ex> {
        match s {
            Sx::Typed(value) => {
                let scalar = self.typed_scalar(value, p);
                if value.kind == crate::typed_scalar::Kind::F32 {
                    scalar
                } else {
                    let conversion = crate::typed_scalar::Scalar {
                        kind: crate::typed_scalar::Kind::F32,
                        op: crate::typed_scalar::Op::Cast,
                        args: vec![s.clone()],
                    };
                    conversion
                        .emit(&[scalar], |e| {
                            Ok::<_, std::convert::Infallible>(self.ir.add(e))
                        })
                        .unwrap()
                }
            }
            Sx::CellContour {
                scope_id,
                inset,
                at,
                channel,
            } => self.cell_contour_query(*scope_id, inset, at.as_deref(), *channel, p),
            Sx::CellQuery {
                scope_id,
                angle,
                inset,
            } => self.cell_geometry_query(*scope_id, angle.as_deref(), inset, p),
            Sx::GradientChannel { sample, channel } => {
                let sample_p = self.v2x_at(&sample.at, p);
                let sample_p = if matches!(
                    sample.kind,
                    GradientKind::Linear {
                        anchor: GradientAnchor::Shape,
                        ..
                    }
                ) {
                    let shape = self
                        .color_shape
                        .expect("shape-anchored color validated before lowering");
                    self.shape_local_point(shape, sample_p)
                } else {
                    sample_p
                };
                let (rgb, alpha) = self.gradient_color_at(&sample.kind, &sample.stops, sample_p);
                let channels = [
                    self.ir.x_of(rgb),
                    self.ir.y_of(rgb),
                    self.ir.add(Ex::AccessIndex {
                        base: rgb,
                        index: 2,
                    }),
                    alpha,
                ];
                for (index, &value) in channels.iter().enumerate() {
                    let key = Sx::GradientChannel {
                        sample: Rc::clone(sample),
                        channel: u8::try_from(index).expect("RGBA channel fits u8"),
                    };
                    self.sx_cache.insert((key, p), value);
                }
                channels[usize::from(*channel)]
            }

            Sx::Lit(v) => self.ir.lit(*v),
            Sx::PxLit(v) => {
                let l = self.ir.lit(*v);
                let local_px = self
                    .cell_inset_pixel_span
                    .unwrap_or_else(|| self.local_pixel_span(p));
                self.ir.mul(l, local_px)
            }
            Sx::EntryInput(input) => match input {
                crate::hir::EntryInput::Time => self.ir.time_override.unwrap_or(self.ir.time),
                crate::hir::EntryInput::Delta => self.ir.delta,
                crate::hir::EntryInput::ResolutionX => self.ir.x_of(self.ir.res),
                crate::hir::EntryInput::ResolutionY => self.ir.y_of(self.ir.res),
            },
            Sx::UniformField {
                binding_name,
                field_index,
                component,
                ..
            } => {
                let global = *self
                    .ir
                    .global_uniform_globals
                    .get(binding_name.as_ref())
                    .unwrap_or_else(|| {
                        panic!(
                            "global uniform `{binding_name}` was not built before lowering \
                             (missing from global_uniform_globals)"
                        )
                    });
                let global_expr = self.ir.add(Ex::GlobalVariable(global));
                let field_ptr = self.ir.add(Ex::AccessIndex {
                    base: global_expr,
                    index: *field_index,
                });
                let value_ptr = match component {
                    Some(c) => self.ir.add(Ex::AccessIndex {
                        base: field_ptr,
                        index: u32::from(*c),
                    }),
                    None => field_ptr,
                };
                self.ir.load(value_ptr)
            }
            Sx::CoordX => self.ir.x_of(p),
            Sx::CoordY => self.ir.y_of(p),
            // Footprint matrix elements (§3.8, §24.1). Use computed Jacobian from canvas_space
            // when available, falling back to identity for contexts without canvas transforms
            // (e.g., user helper functions, scatter bodies).
            Sx::FootprintJ11 => {
                let prev_feature_tag = self.ir.set_feature_tag(Some("pattern_filtering"));
                let out = self.ir.jacobian_j11.unwrap_or_else(|| self.ir.lit(1.0));
                self.ir.restore_feature_tag(prev_feature_tag);
                out
            }
            Sx::FootprintJ12 => {
                let prev_feature_tag = self.ir.set_feature_tag(Some("pattern_filtering"));
                let out = self.ir.jacobian_j12.unwrap_or_else(|| self.ir.lit(0.0));
                self.ir.restore_feature_tag(prev_feature_tag);
                out
            }
            Sx::FootprintJ21 => {
                let prev_feature_tag = self.ir.set_feature_tag(Some("pattern_filtering"));
                let out = self.ir.jacobian_j21.unwrap_or_else(|| self.ir.lit(0.0));
                self.ir.restore_feature_tag(prev_feature_tag);
                out
            }
            Sx::FootprintJ22 => {
                let prev_feature_tag = self.ir.set_feature_tag(Some("pattern_filtering"));
                let out = self.ir.jacobian_j22.unwrap_or_else(|| self.ir.lit(1.0));
                self.ir.restore_feature_tag(prev_feature_tag);
                out
            }
            Sx::PostColorR => self
                .source_color_ctx
                .last()
                .map(|ctx| ctx.r)
                .expect("postprocess source red used outside postprocess lowering"),
            Sx::PostColorG => self
                .source_color_ctx
                .last()
                .map(|ctx| ctx.g)
                .expect("postprocess source green used outside postprocess lowering"),
            Sx::PostColorB => self
                .source_color_ctx
                .last()
                .map(|ctx| ctx.b)
                .expect("postprocess source blue used outside postprocess lowering"),
            Sx::PostColorA => self
                .source_color_ctx
                .last()
                .map(|ctx| ctx.a)
                .expect("postprocess source alpha used outside postprocess lowering"),
            Sx::Param(name) => self
                .ir
                .param_scalars
                .get(name)
                .copied()
                .map(|h| {
                    if self.ir.is_scalar_slot_ptr(h) {
                        self.ir.load(h)
                    } else {
                        h
                    }
                })
                .expect("checker emitted unknown parameter reference"),
            Sx::ScatterInstanceId => self
                .scatter_instance_ctx
                .last()
                .map(|ctx| ctx.id)
                .expect("scatter instance id used outside scatter lowering"),
            Sx::ScatterInstanceIndex01 => self
                .scatter_instance_ctx
                .last()
                .map(|ctx| ctx.index01)
                .expect("scatter instance index01 used outside scatter lowering"),
            Sx::ScatterInstanceAgeNorm => self
                .scatter_instance_ctx
                .last()
                .map(|ctx| ctx.age_norm)
                .expect("scatter instance age_norm used outside scatter lowering"),
            Sx::ScatterInstancePosX => self
                .scatter_instance_ctx
                .last()
                .map(|ctx| ctx.pos_x)
                .expect("scatter instance pos.x used outside scatter lowering"),
            Sx::ScatterInstancePosY => self
                .scatter_instance_ctx
                .last()
                .map(|ctx| ctx.pos_y)
                .expect("scatter instance pos.y used outside scatter lowering"),
            Sx::RepeatCellIdX(scope_id) => self.repeat_cell_ctx(*scope_id).id_x,
            Sx::RepeatCellIdY(scope_id) => self.repeat_cell_ctx(*scope_id).id_y,
            Sx::RepeatCellCenterX(scope_id) => self.repeat_cell_ctx(*scope_id).center_x,
            Sx::RepeatCellCenterY(scope_id) => self.repeat_cell_ctx(*scope_id).center_y,
            Sx::RepeatCellUvX(scope_id) => self.repeat_cell_ctx(*scope_id).uv_x,
            Sx::RepeatCellUvY(scope_id) => self.repeat_cell_ctx(*scope_id).uv_y,
            Sx::RepeatCellRand(scope_id) => self.repeat_cell_ctx(*scope_id).rand,
            Sx::Neg(a) => {
                let a = self.sx_at(a, p);
                self.ir.neg(a)
            }
            Sx::Add(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                self.ir.addx(a, b)
            }
            Sx::Sub(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                self.ir.sub(a, b)
            }
            Sx::Mul(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                self.ir.mul(a, b)
            }
            Sx::Div(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                self.ir.div(a, b)
            }
            Sx::Lt(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                let cond = self.ir.bin(Bo::Less, a, b);
                let one = self.ir.lit(1.0);
                let zero = self.ir.lit(0.0);
                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: one,
                    reject: zero,
                })
            }
            Sx::Le(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                let cond = self.ir.bin(Bo::LessEqual, a, b);
                let one = self.ir.lit(1.0);
                let zero = self.ir.lit(0.0);
                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: one,
                    reject: zero,
                })
            }
            Sx::Gt(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                let cond = self.ir.bin(Bo::Greater, a, b);
                let one = self.ir.lit(1.0);
                let zero = self.ir.lit(0.0);
                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: one,
                    reject: zero,
                })
            }
            Sx::Ge(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                let cond = self.ir.bin(Bo::GreaterEqual, a, b);
                let one = self.ir.lit(1.0);
                let zero = self.ir.lit(0.0);
                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: one,
                    reject: zero,
                })
            }
            Sx::Eq(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                let cond = self.ir.bin(Bo::Equal, a, b);
                let one = self.ir.lit(1.0);
                let zero = self.ir.lit(0.0);
                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: one,
                    reject: zero,
                })
            }
            Sx::Ne(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                let cond = self.ir.bin(Bo::NotEqual, a, b);
                let one = self.ir.lit(1.0);
                let zero = self.ir.lit(0.0);
                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: one,
                    reject: zero,
                })
            }
            Sx::Sin(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Sin, v)
            }
            Sx::Cos(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Cos, v)
            }
            Sx::Tan(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Tan, v)
            }
            Sx::Asin(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Asin, v)
            }
            Sx::Acos(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Acos, v)
            }
            Sx::Atan(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Atan, v)
            }
            Sx::Sqrt(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Sqrt, v)
            }
            Sx::InverseSqrt(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::InverseSqrt, v)
            }
            Sx::Fract(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Fract, v)
            }
            Sx::Abs(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Abs, v)
            }
            Sx::Sign(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Sign, v)
            }
            Sx::Floor(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Floor, v)
            }
            Sx::Ceil(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Ceil, v)
            }
            Sx::Round(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Round, v)
            }
            Sx::Trunc(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Trunc, v)
            }
            Sx::Exp(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Exp, v)
            }
            Sx::Exp2(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Exp2, v)
            }
            Sx::Log(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Log, v)
            }
            Sx::Log2(v) => {
                let v = self.sx_at(v, p);
                self.ir.m1(Mf::Log2, v)
            }
            Sx::Atan2(y, x) => {
                let y = self.sx_at(y, p);
                let x = self.sx_at(x, p);
                self.ir.m2(Mf::Atan2, y, x)
            }
            Sx::Pow(x, e) => {
                let x = self.sx_at(x, p);
                let e = self.sx_at(e, p);
                self.ir.m2(Mf::Pow, x, e)
            }
            Sx::Min(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                self.ir.m2(Mf::Min, a, b)
            }
            Sx::Max(a, b) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                self.ir.m2(Mf::Max, a, b)
            }
            Sx::Step(edge, x) => {
                let edge = self.sx_at(edge, p);
                let x = self.sx_at(x, p);
                self.ir.m2(Mf::Step, edge, x)
            }
            Sx::Dot { a, b } => {
                let av = self.vec_from_sxvec(a, p);
                let bv = self.vec_from_sxvec(b, p);
                self.ir.m2(Mf::Dot, av, bv)
            }
            Sx::NormalizeComponent { v, index } => {
                let vec = self.vec_from_sxvec(v, p);
                let norm = self.ir.m1(Mf::Normalize, vec);
                self.ir.add(Ex::AccessIndex {
                    base: norm,
                    index: u32::from(*index),
                })
            }
            Sx::MinComponent { a, b, index } => {
                let av = self.vec_from_sxvec(a, p);
                let bv = self.vec_from_sxvec(b, p);
                let v = self.ir.m2(Mf::Min, av, bv);
                self.ir.add(Ex::AccessIndex {
                    base: v,
                    index: u32::from(*index),
                })
            }
            Sx::MaxComponent { a, b, index } => {
                let av = self.vec_from_sxvec(a, p);
                let bv = self.vec_from_sxvec(b, p);
                let v = self.ir.m2(Mf::Max, av, bv);
                self.ir.add(Ex::AccessIndex {
                    base: v,
                    index: u32::from(*index),
                })
            }
            Sx::ClampVecComponent { x, lo, hi, index } => {
                let xv = self.vec_from_sxvec(x, p);
                let lv = self.vec_from_sxvec(lo, p);
                let hv = self.vec_from_sxvec(hi, p);
                let v = self.ir.m3(Mf::Clamp, xv, lv, hv);
                self.ir.add(Ex::AccessIndex {
                    base: v,
                    index: u32::from(*index),
                })
            }
            Sx::Length(v) => {
                let vec = self.vec_from_sxvec(v, p);
                self.ir.m1(Mf::Length, vec)
            }
            Sx::Clamp(x, lo, hi) => {
                let x = self.sx_at(x, p);
                let lo = self.sx_at(lo, p);
                let hi = self.sx_at(hi, p);
                self.ir.m3(Mf::Clamp, x, lo, hi)
            }
            Sx::Select(a, b, cond) => {
                let reject = self.sx_at(a, p);
                let accept = self.sx_at(b, p);
                let cond = self.sx_at(cond, p);
                let zero = self.ir.add(Ex::Literal(naga::Literal::F32(0.0)));
                let condition = self.ir.add(Ex::Binary {
                    op: naga::BinaryOperator::NotEqual,
                    left: cond,
                    right: zero,
                });
                self.ir.add(Ex::Select {
                    condition,
                    accept,
                    reject,
                })
            }
            Sx::Mix(a, b, t) => {
                let a = self.sx_at(a, p);
                let b = self.sx_at(b, p);
                let t = self.sx_at(t, p);
                self.ir.m3(Mf::Mix, a, b, t)
            }
            Sx::SmoothStep(lo, hi, x) => {
                let lo = self.sx_at(lo, p);
                let hi = self.sx_at(hi, p);
                let x = self.sx_at(x, p);
                self.ir.m3(Mf::SmoothStep, lo, hi, x)
            }
            Sx::Ddx(v) => {
                let v = self.sx_at(v, p);
                self.ir.add(Ex::Derivative {
                    axis: Da::X,
                    ctrl: Dc::None,
                    expr: v,
                })
            }
            Sx::Ddy(v) => {
                let v = self.sx_at(v, p);
                self.ir.add(Ex::Derivative {
                    axis: Da::Y,
                    ctrl: Dc::None,
                    expr: v,
                })
            }
            Sx::Fwidth(v) => {
                let v = self.sx_at(v, p);
                self.ir.add(Ex::Derivative {
                    axis: Da::Width,
                    ctrl: Dc::None,
                    expr: v,
                })
            }
            Sx::SrgbToLinear(v) => {
                let x = self.sx_at(v, p);
                let threshold = self.ir.lit(0.04045);
                let cond = self.ir.bin(Bo::LessEqual, x, threshold);

                let denom = self.ir.lit(12.92);
                let low = self.ir.div(x, denom);

                let offset = self.ir.lit(0.055);
                let one = self.ir.lit(1.0);
                let scale = self.ir.addx(one, offset);
                let shifted = self.ir.addx(x, offset);
                let normalized = self.ir.div(shifted, scale);
                let exponent = self.ir.lit(2.4);
                let high = self.ir.m2(Mf::Pow, normalized, exponent);

                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: low,
                    reject: high,
                })
            }
            Sx::LinearToSrgb(v) => {
                let x = self.sx_at(v, p);
                let threshold = self.ir.lit(0.003_130_8);
                let cond = self.ir.bin(Bo::LessEqual, x, threshold);

                let low_scale = self.ir.lit(12.92);
                let low = self.ir.mul(x, low_scale);

                let exponent = self.ir.lit(1.0 / 2.4);
                let powed = self.ir.m2(Mf::Pow, x, exponent);
                let high_scale = self.ir.lit(1.055);
                let scaled = self.ir.mul(powed, high_scale);
                let offset = self.ir.lit(0.055);
                let high = self.ir.sub(scaled, offset);

                self.ir.add(Ex::Select {
                    condition: cond,
                    accept: low,
                    reject: high,
                })
            }
            Sx::UserCall { call, component } => {
                let result = self.lower_user_call_result(call, p);
                match component {
                    None => {
                        if call.ret_components == 1 {
                            result
                        } else {
                            self.ir.add(Ex::AccessIndex {
                                base: result,
                                index: 0,
                            })
                        }
                    }
                    Some(index) => self.ir.add(Ex::AccessIndex {
                        base: result,
                        index: *index,
                    }),
                }
            }
            Sx::TexChannel {
                tex_name,
                channel,
                sample_at,
                decode_mul,
                decode_add,
                decode_expr,
            } => {
                let &tex_global = self
                    .ir
                    .tex_globals
                    .get(tex_name)
                    .expect("texture global must be registered before lowering");
                let (sample_p, sample_key) = if let Some(sample_at) = sample_at {
                    let (sample_x, sample_y) = sample_at.as_ref();
                    let sx = self.sx_at(sample_x, p);
                    let sy = self.sx_at(sample_y, p);
                    (
                        self.ir.vec2(sx, sy),
                        TextureSampleCoordKey::Explicit(sample_x.clone(), sample_y.clone()),
                    )
                } else {
                    (p, TextureSampleCoordKey::Current)
                };
                let sample = if let Some(&cached) = self
                    .texture_sample_cache
                    .get(&(tex_global, sample_key.clone()))
                {
                    cached
                } else {
                    let sampler_global = self
                        .ir
                        .sampler_global
                        .expect("sampler global must exist when textures are used");
                    let img = self.ir.add(Ex::GlobalVariable(tex_global));
                    let sampler = self.ir.add(Ex::GlobalVariable(sampler_global));
                    let sample = self.ir.add(Ex::ImageSample {
                        image: img,
                        sampler,
                        gather: None,
                        coordinate: sample_p,
                        array_index: None,
                        offset: None,
                        level: naga::SampleLevel::Auto,
                        depth_ref: None,
                        clamp_to_edge: false,
                    });
                    self.texture_sample_cache
                        .insert((tex_global, sample_key), sample);
                    sample
                };
                let raw = self.ir.add(Ex::AccessIndex {
                    base: sample,
                    index: u32::from(*channel),
                });
                if let Some(expr) = decode_expr {
                    let texel_r = self.ir.add(Ex::AccessIndex {
                        base: sample,
                        index: 0,
                    });
                    let texel_g = self.ir.add(Ex::AccessIndex {
                        base: sample,
                        index: 1,
                    });
                    let texel_b = self.ir.add(Ex::AccessIndex {
                        base: sample,
                        index: 2,
                    });
                    let texel_a = self.ir.add(Ex::AccessIndex {
                        base: sample,
                        index: 3,
                    });

                    self.push_var_override("raw".to_string(), raw);
                    self.push_var_override("texel_r".to_string(), texel_r);
                    self.push_var_override("texel_g".to_string(), texel_g);
                    self.push_var_override("texel_b".to_string(), texel_b);
                    self.push_var_override("texel_a".to_string(), texel_a);
                    let decoded = self.sx_at(expr, p);
                    self.pop_var_override("texel_a");
                    self.pop_var_override("texel_b");
                    self.pop_var_override("texel_g");
                    self.pop_var_override("texel_r");
                    self.pop_var_override("raw");
                    decoded
                } else {
                    let mul = self.ir.lit(*decode_mul);
                    let add = self.ir.lit(*decode_add);
                    let scaled = self.ir.mul(raw, mul);
                    self.ir.addx(scaled, add)
                }
            }
            Sx::EffectInputChannel {
                sample_x,
                sample_y,
                channel,
            } => {
                let inner_id = self
                    .effect_input_layer_ctx
                    .last()
                    .copied()
                    .expect("effect input used outside user-effect lowering");
                let sample_x = self.sx_at(sample_x, p);
                let sample_y = self.sx_at(sample_y, p);
                let sample_p = self.ir.vec2(sample_x, sample_y);
                let (rgb, a) = self.layer_color(inner_id, sample_p);
                match channel {
                    0 => self.ir.x_of(rgb),
                    1 => self.ir.y_of(rgb),
                    2 => self.ir.add(Ex::AccessIndex {
                        base: rgb,
                        index: 2,
                    }),
                    3 => a,
                    _ => unreachable!("rgba channel index must be in 0..=3"),
                }
            }
            Sx::PathDist { path_id } => {
                let sample = self.lower_path_sample_result(*path_id, p);
                self.ir.add(Ex::AccessIndex {
                    base: sample,
                    index: 0,
                })
            }
            Sx::PathAlong { path_id } => {
                let sample = self.lower_path_sample_result(*path_id, p);
                self.ir.add(Ex::AccessIndex {
                    base: sample,
                    index: 1,
                })
            }
            Sx::PathTangentComponent { path_id, component } => {
                let sample = self.lower_path_sample_result(*path_id, p);
                self.ir.add(Ex::AccessIndex {
                    base: sample,
                    index: 2 + u32::from(*component),
                })
            }
            Sx::PathPointAtComponent {
                path_id,
                s,
                component,
            } => {
                let s_handle = self.sx_at(s, p);
                let sample = self.lower_path_param_sample_result(*path_id, s_handle);
                self.ir.add(Ex::AccessIndex {
                    base: sample,
                    index: u32::from(*component),
                })
            }
            Sx::PathTangentAtComponent {
                path_id,
                s,
                component,
            } => {
                let s_handle = self.sx_at(s, p);
                let sample = self.lower_path_param_sample_result(*path_id, s_handle);
                self.ir.add(Ex::AccessIndex {
                    base: sample,
                    index: 2 + u32::from(*component),
                })
            }
            Sx::Var(name) => {
                // Resolved via the var-override stack pushed by `Layer::UserEffect` lowering.
                self.peek_var_override(name).unwrap_or_else(|| {
                    panic!("Sx::Var({name:?}) used outside user-effect lowering")
                })
            }
            Sx::DynamicArrayIndex {
                param_name,
                index,
                component,
            } => {
                let idx_h = self.native_scalar(index, p);
                // Dynamic indexing requires u32; cast the scalar index if needed.
                // Note: naga widths are in bytes, so 4 = 32-bit.
                let idx_u32 = self.ir.add(Ex::As {
                    kind: naga::ScalarKind::Uint,
                    expr: idx_h,
                    convert: Some(4),
                });

                if let Some(array_expr) = self.ir.param_scalars.get(param_name).copied() {
                    let elem = self.ir.add(Ex::Access {
                        base: array_expr,
                        index: idx_u32,
                    });
                    return match component {
                        None => elem,
                        Some(c) => self.ir.add(Ex::AccessIndex {
                            base: elem,
                            index: u32::from(*c),
                        }),
                    };
                }

                let global = self
                    .ir
                    .param_storage_globals
                    .get(param_name)
                    .copied()
                    .unwrap_or_else(|| {
                        panic!(
                            "Sx::DynamicArrayIndex references unknown param `{param_name}`; \
                             checker should have validated this"
                        )
                    });
                let global_ptr = self.ir.add(Ex::GlobalVariable(global));
                let elem_ptr = self.ir.add(Ex::Access {
                    base: global_ptr,
                    index: idx_u32,
                });
                match component {
                    None => {
                        // Scalar element type — load directly.
                        self.ir.load(elem_ptr)
                    }
                    Some(c) => {
                        // Vector element type — load the vector, then extract the lane.
                        let vec_val = self.ir.load(elem_ptr);
                        self.ir.add(Ex::AccessIndex {
                            base: vec_val,
                            index: u32::from(*c),
                        })
                    }
                }
            }
            Sx::Let { name, value, body } => {
                // Lower `value` exactly once and let every `Var(name)` reference inside
                // `body` resolve to that same handle, instead of re-lowering `value`
                // once per reference (which is what would happen if `Let` were
                // desugared away before reaching this point).
                let value_handle = self.sx_at(value, p);
                self.push_var_override(name.clone(), value_handle);
                let result = self.sx_at(body, p);
                self.pop_var_override(name);
                result
            }
        }
    }

    fn lower_path_sample_result(&mut self, path_id: usize, p: Handle<Ex>) -> Handle<Ex> {
        let key = (path_id, p);
        if let Some(handle) = self.path_sample_cache.get(&key).copied() {
            return handle;
        }

        let helper_fn = self
            .path_helper_fns
            .get(&path_id)
            .and_then(|fns| fns.nearest_sample)
            .expect("checker emitted path nearest-search demand without a lowering helper");
        let call_result = self.ir.add(Ex::CallResult(helper_fn));
        self.ir.push_statement(naga::Statement::Call {
            function: helper_fn,
            arguments: vec![p],
            result: Some(call_result),
        });
        self.path_sample_cache.insert(key, call_result);
        call_result
    }

    fn lower_path_param_sample_result(&mut self, path_id: usize, s: Handle<Ex>) -> Handle<Ex> {
        let key = (path_id, s);
        if let Some(handle) = self.path_param_sample_cache.get(&key).copied() {
            return handle;
        }

        let helper_fn = self
            .path_helper_fns
            .get(&path_id)
            .and_then(|fns| fns.arc_sample)
            .expect("checker emitted path arc-sample demand without a lowering helper");
        let call_result = self.ir.add(Ex::CallResult(helper_fn));
        self.ir.push_statement(naga::Statement::Call {
            function: helper_fn,
            arguments: vec![s],
            result: Some(call_result),
        });
        self.path_param_sample_cache.insert(key, call_result);
        call_result
    }

    fn lower_user_call_result(
        &mut self,
        call: &crate::hir::UserFnCall,
        p: Handle<Ex>,
    ) -> Handle<Ex> {
        // Arguments and captured coordinates may depend on the active sample point.
        // Invariant scalar expressions are already cached by sx_at.
        let key = (call.clone(), Some(p));
        if let Some(handle) = self.user_call_cache.get(&key).copied() {
            return handle;
        }

        let helper_fn = *self
            .user_helper_fns
            .get(&call.helper_id)
            .expect("checker emitted unknown user helper call target");
        let helper = self
            .hir
            .user_helpers
            .get(&call.helper_id)
            .expect("checker emitted unknown user helper metadata");

        let mut args = if helper.params.is_empty() {
            call.args
                .iter()
                .map(|arg| self.sx_at(arg, p))
                .collect::<Vec<_>>()
        } else {
            let expected_scalars = helper
                .params
                .iter()
                .map(|param| match param.ty {
                    crate::hir::UserFnParamTy::Scalar => 1usize,
                    crate::hir::UserFnParamTy::Vec2 => 2usize,
                    crate::hir::UserFnParamTy::Vec3 => 3usize,
                    crate::hir::UserFnParamTy::Vec4 => 4usize,
                    crate::hir::UserFnParamTy::Mat2 => 4usize,
                    crate::hir::UserFnParamTy::Mat3 => 9usize,
                    crate::hir::UserFnParamTy::Mat4 => 16usize,
                })
                .sum::<usize>();
            debug_assert_eq!(
                call.args.len(),
                expected_scalars,
                "helper call packing mismatch for {}: saw {}, expected {}",
                call.helper_id,
                call.args.len(),
                expected_scalars
            );

            let mut packed = Vec::with_capacity(helper.params.len());
            let mut cursor = 0usize;
            for param in &helper.params {
                match param.ty {
                    crate::hir::UserFnParamTy::Scalar => {
                        let scalar = call
                            .args
                            .get(cursor)
                            .map(|sx| self.native_scalar(sx, p))
                            .unwrap_or_else(|| self.ir.lit(0.0));
                        packed.push(scalar);
                        cursor += 1;
                    }
                    crate::hir::UserFnParamTy::Vec2
                    | crate::hir::UserFnParamTy::Vec3
                    | crate::hir::UserFnParamTy::Vec4 => {
                        let width = match param.ty {
                            crate::hir::UserFnParamTy::Vec2 => 2usize,
                            crate::hir::UserFnParamTy::Vec3 => 3usize,
                            crate::hir::UserFnParamTy::Vec4 => 4usize,
                            _ => unreachable!(),
                        };
                        let mut comps = Vec::with_capacity(width);
                        for _ in 0..width {
                            comps.push(
                                call.args
                                    .get(cursor)
                                    .map(|sx| self.native_scalar(sx, p))
                                    .unwrap_or_else(|| self.ir.lit(0.0)),
                            );
                            cursor += 1;
                        }
                        let ty = match param.ty {
                            crate::hir::UserFnParamTy::Vec2 => {
                                vector_type_handle(param.scalar_kind, 2, &self.ir.types)
                            }
                            crate::hir::UserFnParamTy::Vec3 => {
                                vector_type_handle(param.scalar_kind, 3, &self.ir.types)
                            }
                            crate::hir::UserFnParamTy::Vec4 => {
                                vector_type_handle(param.scalar_kind, 4, &self.ir.types)
                            }
                            _ => unreachable!(),
                        };
                        packed.push(self.ir.add(Ex::Compose {
                            ty,
                            components: comps,
                        }));
                    }
                    crate::hir::UserFnParamTy::Mat2
                    | crate::hir::UserFnParamTy::Mat3
                    | crate::hir::UserFnParamTy::Mat4 => {
                        let (rows, vec_ty, mat_ty) = match param.ty {
                            crate::hir::UserFnParamTy::Mat2 => {
                                (2usize, self.ir.types.v2, self.ir.types.m2)
                            }
                            crate::hir::UserFnParamTy::Mat3 => {
                                (3usize, self.ir.types.v3, self.ir.types.m3)
                            }
                            crate::hir::UserFnParamTy::Mat4 => {
                                (4usize, self.ir.types.v4, self.ir.types.m4)
                            }
                            _ => unreachable!(),
                        };
                        let mut cols = Vec::with_capacity(rows);
                        for _ in 0..rows {
                            let mut col_components = Vec::with_capacity(rows);
                            for _ in 0..rows {
                                col_components.push(
                                    call.args
                                        .get(cursor)
                                        .map(|sx| self.native_scalar(sx, p))
                                        .unwrap_or_else(|| self.ir.lit(0.0)),
                                );
                                cursor += 1;
                            }
                            cols.push(self.ir.add(Ex::Compose {
                                ty: vec_ty,
                                components: col_components,
                            }));
                        }
                        packed.push(self.ir.add(Ex::Compose {
                            ty: mat_ty,
                            components: cols,
                        }));
                    }
                }
            }
            packed
        };

        if helper.needs_entry_inputs {
            args.extend([
                p,
                self.ir.time_override.unwrap_or(self.ir.time),
                self.ir.delta,
                self.ir.res,
            ]);
        }

        let call_result = self.ir.add(Ex::CallResult(helper_fn));
        self.ir.push_statement(naga::Statement::Call {
            function: helper_fn,
            arguments: args,
            result: Some(call_result),
        });
        self.user_call_cache.insert(key, call_result);
        call_result
    }

    pub(super) fn v2x_at(&mut self, v: &V2, p: Handle<Ex>) -> Handle<Ex> {
        let x = self.sx_at(&v.0, p);
        let y = self.sx_at(&v.1, p);
        self.ir.vec2(x, y)
    }

    pub(super) fn color3(&mut self, c: [f32; 4]) -> Handle<Ex> {
        let r = self.ir.lit(c[0]);
        let g = self.ir.lit(c[1]);
        let b = self.ir.lit(c[2]);
        self.ir.add(Ex::Compose {
            ty: self.ir.types.v3,
            components: vec![r, g, b],
        })
    }

    pub(super) fn color_expr3_at(&mut self, c: &[Sx; 4], p: Handle<Ex>) -> Handle<Ex> {
        let r = self.sx_at(&c[0], p);
        let g = self.sx_at(&c[1], p);
        let b = self.sx_at(&c[2], p);
        self.ir.add(Ex::Compose {
            ty: self.ir.types.v3,
            components: vec![r, g, b],
        })
    }

    pub(super) fn color_source_at(
        &mut self,
        color: &ColorSource,
        p: Handle<Ex>,
    ) -> (Handle<Ex>, Handle<Ex>) {
        match color {
            ColorSource::Solid(rgba) => {
                let rgb = self.color_expr3_at(rgba, p);
                let a = self.sx_at(&rgba[3], p);
                (rgb, a)
            }
            ColorSource::Gradient { kind, stops } => self.gradient_color_at(kind, stops, p),
        }
    }

    pub(super) fn gradient_color_at(
        &mut self,
        kind: &GradientKind,
        stops: &[GradientStop],
        p: Handle<Ex>,
    ) -> (Handle<Ex>, Handle<Ex>) {
        let eps = self.ir.lit(1.0e-6);
        let raw_t = match kind {
            GradientKind::Linear { along, .. } => {
                if let Some(axis_component) = self.axis_aligned_linear_gradient_component(along, p)
                {
                    axis_component
                } else {
                    let dir = self.v2x_at(along, p);
                    let dx = self.ir.x_of(dir);
                    let dy = self.ir.y_of(dir);
                    let dir_len = self.ir.m1(Mf::Length, dir);
                    let safe_len = self.ir.m2(Mf::Max, dir_len, eps);
                    let nx = self.ir.div(dx, safe_len);
                    let ny = self.ir.div(dy, safe_len);
                    let px = self.ir.x_of(p);
                    let py = self.ir.y_of(p);
                    let dot_x = self.ir.mul(px, nx);
                    let dot_y = self.ir.mul(py, ny);
                    self.ir.addx(dot_x, dot_y)
                }
            }
            GradientKind::Radial { center, radius } => {
                let center = self.v2x_at(center, p);
                let delta = self.ir.sub(p, center);
                let dist = self.ir.m1(Mf::Length, delta);
                let radius = self.sx_at(radius, p);
                let safe_radius = self.ir.m2(Mf::Max, radius, eps);
                self.ir.div(dist, safe_radius)
            }
        };
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let t = self.ir.m3(Mf::Clamp, raw_t, zero, one);

        let first = &stops[0];
        let mut rgb = self.color_expr3_at(&first.color, p);
        let mut grad_a = self.sx_at(&first.color[3], p);
        let mut prev_at = self.sx_at(&first.at, p);
        let mut prev_stop = first;
        for stop in stops.iter().skip(1) {
            let seg_start = prev_at;
            let seg_end = self.sx_at(&stop.at, p);
            let local_t = match (
                Self::sx_lit_value(&prev_stop.at),
                Self::sx_lit_value(&stop.at),
            ) {
                (Some(0.0), Some(1.0)) => t,
                (Some(seg_start_lit), Some(seg_end_lit)) if seg_end_lit > seg_start_lit => {
                    let local_t_num = self.ir.sub(t, seg_start);
                    let seg_len = self.ir.lit(seg_end_lit - seg_start_lit);
                    let local_t_raw = self.ir.div(local_t_num, seg_len);
                    self.ir.m3(Mf::Clamp, local_t_raw, zero, one)
                }
                _ => {
                    let seg_raw = self.ir.sub(seg_end, seg_start);
                    let seg_len = self.ir.m2(Mf::Max, seg_raw, eps);
                    let local_t_num = self.ir.sub(t, seg_start);
                    let local_t_raw = self.ir.div(local_t_num, seg_len);
                    self.ir.m3(Mf::Clamp, local_t_raw, zero, one)
                }
            };
            let local_t3 = self.ir.splat3(local_t);

            if !Self::rgb_expr_equal(&prev_stop.color, &stop.color) {
                let next_rgb = self.color_expr3_at(&stop.color, p);
                rgb = self.ir.m3(Mf::Mix, rgb, next_rgb, local_t3);
            }

            if prev_stop.color[3] != stop.color[3] {
                let next_a = self.sx_at(&stop.color[3], p);
                grad_a = self.ir.m3(Mf::Mix, grad_a, next_a, local_t);
            }
            prev_at = seg_end;
            prev_stop = stop;
        }

        let dither3 = self.gradient_dither3();
        let rgb_noisy = self.ir.addx(rgb, dither3);
        let zero3 = self.ir.splat3(zero);
        let one3 = self.ir.splat3(one);
        let rgb_dithered = self.ir.m3(Mf::Clamp, rgb_noisy, zero3, one3);
        (rgb_dithered, grad_a)
    }

    pub(super) fn glow_falloff_at(
        &mut self,
        falloff: GlowFalloff,
        d_nonnegative: Handle<Ex>,
        reach: Handle<Ex>,
    ) -> Handle<Ex> {
        let third = self.ir.lit(1.0 / 3.0);
        let eps = self.ir.lit(1.0e-6);
        let reach_abs = self.ir.m1(Mf::Abs, reach);
        let denom_raw = self.ir.mul(reach_abs, third);
        let denom = self.ir.m2(Mf::Max, denom_raw, eps);
        let t = self.ir.div(d_nonnegative, denom);
        match falloff {
            GlowFalloff::Exp => {
                let nt = self.ir.neg(t);
                self.ir.m1(Mf::Exp, nt)
            }
            GlowFalloff::Gaussian => {
                let t2 = self.ir.mul(t, t);
                let nt2 = self.ir.neg(t2);
                self.ir.m1(Mf::Exp, nt2)
            }
            GlowFalloff::Linear => {
                let one = self.ir.lit(1.0);
                let raw = self.ir.sub(one, t);
                let zero = self.ir.lit(0.0);
                self.ir.m2(Mf::Max, raw, zero)
            }
        }
    }

    /// Deterministic per-channel gradient dither in approximately +/-0.5 LSB
    /// of 8-bit color. Applied in linear space to reduce visible banding.
    pub(super) fn gradient_dither3(&mut self) -> Handle<Ex> {
        if let Some(cached) = self.gradient_dither3_cache {
            return cached;
        }

        let x = self.ir.x_of(self.ir.uv);
        let y = self.ir.y_of(self.ir.uv);

        let half = self.ir.lit(0.5);
        // 1.0 / 255.0 gives a centered range of approximately +/-0.5 LSB.
        let amp = self.ir.lit(0.003_921_569);

        let r_cx = self.ir.lit(12.9898);
        let r_cy = self.ir.lit(78.233);
        let r_scale = self.ir.lit(43_758.547);

        let rx = self.ir.mul(x, r_cx);
        let ry = self.ir.mul(y, r_cy);
        let r_phase = self.ir.addx(rx, ry);
        let r_sin = self.ir.m1(Mf::Sin, r_phase);
        let r_hash_in = self.ir.mul(r_sin, r_scale);
        let r_hash = self.ir.m1(Mf::Fract, r_hash_in);
        let r_centered = self.ir.sub(r_hash, half);
        let r = self.ir.mul(r_centered, amp);

        let g_cx = self.ir.lit(39.3468);
        let g_cy = self.ir.lit(11.1351);
        let g_scale = self.ir.lit(24_634.635);

        let gx = self.ir.mul(x, g_cx);
        let gy = self.ir.mul(y, g_cy);
        let g_phase = self.ir.addx(gx, gy);
        let g_sin = self.ir.m1(Mf::Sin, g_phase);
        let g_hash_in = self.ir.mul(g_sin, g_scale);
        let g_hash = self.ir.m1(Mf::Fract, g_hash_in);
        let g_centered = self.ir.sub(g_hash, half);
        let g = self.ir.mul(g_centered, amp);

        let b_cx = self.ir.lit(73.1569);
        let b_cy = self.ir.lit(52.2358);
        let b_scale = self.ir.lit(19_642.35);

        let bx = self.ir.mul(x, b_cx);
        let by = self.ir.mul(y, b_cy);
        let b_phase = self.ir.addx(bx, by);
        let b_sin = self.ir.m1(Mf::Sin, b_phase);
        let b_hash_in = self.ir.mul(b_sin, b_scale);
        let b_hash = self.ir.m1(Mf::Fract, b_hash_in);
        let b_centered = self.ir.sub(b_hash, half);
        let b = self.ir.mul(b_centered, amp);

        let dither = self.ir.add(Ex::Compose {
            ty: self.ir.types.v3,
            components: vec![r, g, b],
        });
        self.gradient_dither3_cache = Some(dither);
        dither
    }

    /// Push a named scalar handle onto the var-override stack.
    /// Called by `Layer::UserEffect` lowering before evaluating the body.
    pub(super) fn push_var_override(&mut self, name: String, handle: Handle<Ex>) {
        self.var_overrides.push((name, handle));
    }

    /// Pop the most-recently-pushed override for `name`.
    /// Called by `Layer::UserEffect` lowering after the body is evaluated.
    pub(super) fn pop_var_override(&mut self, name: &str) {
        if let Some(pos) = self.var_overrides.iter().rposition(|(n, _)| n == name) {
            self.var_overrides.remove(pos);
        }
    }

    /// Look up the top override for `name`, if one exists.
    pub(super) fn peek_var_override(&self, name: &str) -> Option<Handle<Ex>> {
        self.var_overrides
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, h)| *h)
    }
}
