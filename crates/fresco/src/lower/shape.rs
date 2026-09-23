use super::*;

impl<'h> FnCtx<'h> {
    pub(super) fn rendering_policy(&self) -> crate::hir::RenderingPolicy {
        self.hir
            .rendering_policy
            .expect("rendering entry points require validated engine policy before lowering")
    }

    pub(super) fn clear_point_sensitive_caches(&mut self) {
        self.sx_cache.clear();
        self.contour_cache.clear();
        self.sdf_cache.clear();
        self.local_pixel_span_cache.clear();
        self.shape_anchor_cache.clear();
        self.shape_half_extent_cache.clear();
        self.shape_local_point_cache.clear();
        self.shape_anchor_space_point_cache.clear();
        self.shape_glow_warp_cache.clear();
    }

    fn adaptive_aa_width(&mut self, d: Handle<Ex>) -> Handle<Ex> {
        let prev_feature_tag = self.ir.set_feature_tag(Some("shape_aa"));
        // Chain rule in screen space: these are the local distance gradient
        // projected onto the two pixel-footprint axes. A unit local SDF gradient
        // does not justify using the longest footprint axis for every edge.
        let grad_x = self.ir.dpdx(d);
        self.ir.name(grad_x, "grad_d_x");
        let grad_y = self.ir.dpdy(d);
        self.ir.name(grad_y, "grad_d_y");
        let gx2 = self.ir.mul(grad_x, grad_x);
        let gy2 = self.ir.mul(grad_y, grad_y);
        let length2 = self.ir.addx(gx2, gy2);
        let gradient_width = self.ir.m1(Mf::Sqrt, length2);
        self.ir.name(gradient_width, "aa_width_gradient");

        // Preserve the existing seam-safe policy in discontinuous coordinate
        // domains. Ordinary/projective spaces use the directional footprint.
        let directional_px =
            if self.discontinuity_space_depth > 0 || !self.repeat_cell_ctx.is_empty() {
                self.ir.px
            } else {
                let cap = self
                    .ir
                    .lit(self.rendering_policy().projective_footprint_max_px);
                let cap = self.ir.mul(cap, self.ir.px);
                let bounded = self.ir.m2(Mf::Min, gradient_width, cap);
                self.clamp_positive(bounded)
            };
        self.ir.name(directional_px, "aa_directional_px");
        let width_est = match self.rendering_policy().shape_aa_style {
            crate::hir::ShapeAaStyle::Gradient => gradient_width,
            crate::hir::ShapeAaStyle::Fwidth | crate::hir::ShapeAaStyle::Conservative => {
                // L1 is always at least L2, so conservative selects this too.
                let abs_dx = self.ir.m1(Mf::Abs, grad_x);
                let abs_dy = self.ir.m1(Mf::Abs, grad_y);
                self.ir.addx(abs_dx, abs_dy)
            }
        };
        let min_band = self.ir.lit(self.rendering_policy().shape_aa_min_px);
        let max_band = self.ir.lit(self.rendering_policy().shape_aa_max_px);
        let min_width = self.ir.mul(min_band, directional_px);
        let max_width = self.ir.mul(max_band, directional_px);
        let clamped = self.ir.m2(Mf::Max, width_est, min_width);
        let aa = self.ir.m2(Mf::Min, clamped, max_width);
        self.ir.name(aa, "aa_adaptive");
        self.ir.restore_feature_tag(prev_feature_tag);
        aa
    }

    fn coverage_from_width(&mut self, d: Handle<Ex>, aa: Handle<Ex>) -> Handle<Ex> {
        // Standard coverage formula: clamp(0.5 - d/aa, 0, 1).
        let half = self.ir.lit(0.5);
        let t = self.ir.div(d, aa);
        let v = self.ir.sub(half, t);
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        self.ir.m3(Mf::Clamp, v, zero, one)
    }

    fn saturate_coverage_by_mean(
        &mut self,
        coverage: Handle<Ex>,
        feature_width: Handle<Ex>,
        aa: Handle<Ex>,
        feature_name: &str,
        cap_name: &str,
        saturated_name: &str,
    ) -> Handle<Ex> {
        let feature_width = self.ir.m1(Mf::Abs, feature_width);
        self.ir.name(feature_width, feature_name);
        let mean_raw = self.safe_div_abs_nonzero(feature_width, aa);
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let mean_cap = self.ir.m3(Mf::Clamp, mean_raw, zero, one);
        self.ir.name(mean_cap, cap_name);
        let saturated = self.ir.m2(Mf::Min, coverage, mean_cap);
        self.ir.name(saturated, saturated_name);
        saturated
    }

    fn line_family_analytic_coverage(
        &mut self,
        axis: u8,
        spacing: Sx,
        offset: Sx,
        width: Sx,
        p: Handle<Ex>,
    ) -> Handle<Ex> {
        let prev_feature_tag = self.ir.set_feature_tag(Some("pattern_filtering"));
        let coord = if axis == 0 { Sx::CoordX } else { Sx::CoordY };
        let rel = Sx::Add(
            Box::new(Sx::Div(Box::new(coord), Box::new(spacing.clone()))),
            Box::new(offset),
        );

        let (j11, j12, j21, j22) = (
            Sx::FootprintJ11,
            Sx::FootprintJ12,
            Sx::FootprintJ21,
            Sx::FootprintJ22,
        );
        let (span_x, span_y) = crate::deriv::footprint_spans(&j11, &j12, &j21, &j22);
        let span = if axis == 0 { span_x } else { span_y };
        let pixel_span = Sx::Mul(Box::new(span), Box::new(Sx::PxLit(1.0)));
        let filter_width_cells = Sx::Div(Box::new(pixel_span), Box::new(spacing.clone()));

        let duty = Sx::Div(Box::new(Sx::Abs(Box::new(width))), Box::new(spacing));
        let cov_sx = crate::deriv::filtered_pulse_train(rel, filter_width_cells, duty);
        let out = self.sx_at(&cov_sx, p);
        self.ir.restore_feature_tag(prev_feature_tag);
        out
    }

    pub(super) fn shape_coverage(&mut self, id: ShapeId, p: Handle<Ex>) -> Handle<Ex> {
        if self.cellular_sample_depth > 0 {
            let d = self.sdf(id, p);
            return self.coverage(id, d, p);
        }
        if let Shape::RBox {
            center,
            half,
            round: Sx::Lit(0.0),
        } = self.hir.shapes[id].clone()
            && self.sx_is_sample_point_invariant(&center.0)
            && self.sx_is_sample_point_invariant(&center.1)
            && self.sx_is_sample_point_invariant(&half.0)
            && self.sx_is_sample_point_invariant(&half.1)
        {
            return self.box_filtered_coverage(&center, &half, p);
        }
        if let Shape::Outline { inner, width } = self.hir.shapes[id].clone()
            && let Shape::LineFamily {
                axis,
                spacing,
                offset,
            } = self.hir.shapes[inner].clone()
        {
            return self.line_family_analytic_coverage(axis, spacing, offset, width, p);
        }

        if let Shape::Outline { width, .. } = self.hir.shapes[id].clone() {
            self.stats.outline_thin_fade_count += 1;
            let Shape::Outline { inner, .. } = self.hir.shapes[id].clone() else {
                unreachable!()
            };
            let inner_d = self.sdf(inner, p);
            let inner_abs = self.ir.m1(Mf::Abs, inner_d);
            self.ir.name(inner_abs, &format!("outline_abs_s{id}"));
            let stroke_width_raw = self.sx_at(&width, p);
            let half = self.ir.lit(0.5);
            let stroke_half_w = self.ir.mul(stroke_width_raw, half);
            self.ir
                .name(stroke_half_w, &format!("outline_half_w_s{id}"));
            let d = self.ir.sub(inner_abs, stroke_half_w);
            self.ir.name(d, &format!("d_s{id}"));

            let aa = self.adaptive_aa_width(d);

            // For subpixel strokes, lock the visible width to one display
            // pixel and convert the deficit into opacity instead of widening
            // the stroke into a blurred band.
            let stroke_width = self.ir.m1(Mf::Abs, stroke_width_raw);
            let locked_width = self.ir.m2(Mf::Max, stroke_width, aa);
            self.ir
                .name(locked_width, &format!("outline_locked_w_s{id}"));
            let locked_half_w = self.ir.mul(locked_width, half);
            let locked_d = self.ir.sub(inner_abs, locked_half_w);
            let base_cov = self.coverage_from_width(locked_d, aa);
            let thin_fade = self.safe_div_positive(stroke_width, locked_width);
            let zero = self.ir.lit(0.0);
            let one = self.ir.lit(1.0);
            let thin_fade = self.ir.m3(Mf::Clamp, thin_fade, zero, one);
            self.ir.name(thin_fade, &format!("outline_thin_fade_s{id}"));
            let cov = self.ir.mul(base_cov, thin_fade);
            self.ir.name(cov, &format!("outline_cov_thin_fade_s{id}"));
            return cov;
        }

        let d = self.sdf(id, p);
        self.coverage(id, d, p)
    }

    fn box_filtered_coverage(&mut self, center: &V2, half: &V2, p: Handle<Ex>) -> Handle<Ex> {
        let prev_feature_tag = self.ir.set_feature_tag(Some("shape_aa"));
        let domain = self.box_filter_domains.get(&p).copied();
        // Separable box filter: retain the two axis spans instead of widening
        // both axes by a single isotropic estimate (especially bad in polar).
        let spans = if domain.is_none()
            && (self.discontinuity_space_depth > 0 || !self.repeat_cell_ctx.is_empty())
        {
            // Other remaps retain their existing seam-safe AA estimate until
            // they also provide an analytic domain footprint.
            let span = self.local_pixel_span(p);
            self.ir.splat2(span)
        } else {
            let (dx, dy) = domain.map_or_else(
                || (self.ir.dpdx(p), self.ir.dpdy(p)),
                |domain| (domain.dx, domain.dy),
            );
            let dx = self.ir.m1(Mf::Abs, dx);
            let dy = self.ir.m1(Mf::Abs, dy);
            self.ir.addx(dx, dy)
        };
        let aa_scale = self.ir.lit(1.0_f32.clamp(
            self.rendering_policy().shape_aa_min_px,
            self.rendering_policy().shape_aa_max_px,
        ));
        let aa_scale = self.ir.splat2(aa_scale);
        let spans = self.ir.mul(spans, aa_scale);
        self.ir.name(spans, "box_filter_spans");
        let center = self.v2x_at(center, p);
        let half = self.v2x_at(half, p);
        let lo = self.ir.sub(center, half);
        let hi = self.ir.addx(center, half);
        let mut x = self.ir.x_of(p);
        let mut wx = self.ir.x_of(spans);
        let mut lx = self.ir.x_of(lo);
        let mut hx = self.ir.x_of(hi);
        if let Some(domain) = domain {
            let period = self.ir.sub(domain.x_bounds.1, domain.x_bounds.0);
            let period = self.clamp_positive(period);
            x = self.ir.sub(x, domain.x_bounds.0);
            x = self.ir.div(x, period);
            wx = self.ir.div(wx, period);
            lx = self.ir.sub(lx, domain.x_bounds.0);
            lx = self.ir.div(lx, period);
            hx = self.ir.sub(hx, domain.x_bounds.0);
            hx = self.ir.div(hx, period);
        }
        let cx = self.lower_interval_filter([x, wx, lx, hx], p, domain.is_some());
        self.ir.name(
            cx,
            if domain.is_some() {
                "box_periodic_coverage"
            } else {
                "box_x_coverage"
            },
        );
        let y = self.ir.y_of(p);
        let wy = self.ir.y_of(spans);
        let ly = self.ir.y_of(lo);
        let hy = self.ir.y_of(hi);
        let cy = self.lower_interval_filter([y, wy, ly, hy], p, false);
        let coverage = self.ir.mul(cx, cy);
        self.ir.name(coverage, "box_filtered_coverage");
        self.ir.restore_feature_tag(prev_feature_tag);
        coverage
    }

    fn lower_interval_filter(
        &mut self,
        args: [Handle<Ex>; 4],
        p: Handle<Ex>,
        periodic: bool,
    ) -> Handle<Ex> {
        // Bind immutable handles to the shared, numerically tested HIR formula.
        // Handle-specific names keep expression-cache entries valid across calls.
        let names = args.map(|h| format!("$filter_{}", h.index()));
        for (name, handle) in names.iter().zip(args) {
            self.push_var_override(name.clone(), handle);
        }
        let [x, width, lo, hi] = names.clone().map(Sx::Var);
        let expr = if periodic {
            crate::deriv::filtered_periodic_interval(x, width, lo, hi)
        } else {
            crate::deriv::filtered_interval(x, width, lo, hi)
        };
        let result = self.sx_at(&expr, p);
        for name in names.iter().rev() {
            self.pop_var_override(name);
        }
        result
    }

    pub(super) fn shape_anchor(&mut self, id: ShapeId, p: Handle<Ex>) -> Handle<Ex> {
        if let Some(cached) = self.shape_anchor_cache.get(&(id, p)).copied() {
            return cached;
        }

        let anchor = match self.hir.shapes[id].clone() {
            Shape::Circle { center, .. }
            | Shape::RBox { center, .. }
            | Shape::Ellipse { center, .. }
            | Shape::Star { center, .. } => self.v2x_at(&center, p),
            Shape::Capsule { from, to, .. } => {
                let a = self.v2x_at(&from, p);
                let b = self.v2x_at(&to, p);
                let half = self.ir.lit(0.5);
                let ab = self.ir.addx(a, b);
                let half2 = self.ir.splat2(half);
                self.ir.mul(ab, half2)
            }
            Shape::Triangle { a, b, c } => {
                let va = self.v2x_at(&a, p);
                let vb = self.v2x_at(&b, p);
                let vc = self.v2x_at(&c, p);
                let sum_ab = self.ir.addx(va, vb);
                let sum = self.ir.addx(sum_ab, vc);
                let third = self.ir.lit(1.0 / 3.0);
                let third2 = self.ir.splat2(third);
                self.ir.mul(sum, third2)
            }
            Shape::Polygon { contours, .. } => {
                let mut count = 0usize;
                let mut sum: Option<Handle<Ex>> = None;
                for contour in contours {
                    for vertex in contour {
                        let v = self.v2x_at(&vertex, p);
                        sum = Some(match sum {
                            Some(acc) => self.ir.addx(acc, v),
                            None => v,
                        });
                        count += 1;
                    }
                }
                let sum = sum.unwrap_or_else(|| {
                    let half_x = self.ir.lit(0.5);
                    let half_y = self.ir.lit(0.5);
                    self.ir.vec2(half_x, half_y)
                });
                let inv = self.ir.lit(1.0 / (count.max(1) as f32));
                let inv2 = self.ir.splat2(inv);
                self.ir.mul(sum, inv2)
            }
            Shape::LineFamily { .. } | Shape::GridLine { .. } => {
                // Line families are infinite; use canvas center as anchor.
                let half = self.ir.lit(0.5);
                let half_y = self.ir.lit(0.5);
                self.ir.vec2(half, half_y)
            }
            Shape::Outline { inner, .. } | Shape::Offset { inner, .. } => {
                self.shape_anchor(inner, p)
            }
            Shape::Rotate { inner, .. } => self.shape_anchor(inner, p),
            Shape::Mix(a, b, _) => {
                let aa = self.shape_anchor(a, p);
                let bb = self.shape_anchor(b, p);
                let sum = self.ir.addx(aa, bb);
                let half = self.ir.lit(0.5);
                let half2 = self.ir.splat2(half);
                self.ir.mul(sum, half2)
            }
            Shape::Union(a, b)
            | Shape::SmoothUnion(a, b, _)
            | Shape::Intersect(a, b)
            | Shape::Subtract(a, b) => {
                let aa = self.shape_anchor(a, p);
                let bb = self.shape_anchor(b, p);
                let sum = self.ir.addx(aa, bb);
                let half = self.ir.lit(0.5);
                let half2 = self.ir.splat2(half);
                self.ir.mul(sum, half2)
            }
        };
        self.shape_anchor_cache.insert((id, p), anchor);
        anchor
    }

    pub(super) fn shape_half_extent(&mut self, id: ShapeId, p: Handle<Ex>) -> Handle<Ex> {
        if let Some(cached) = self.shape_half_extent_cache.get(&(id, p)).copied() {
            return cached;
        }

        let extent = match self.hir.shapes[id].clone() {
            Shape::Circle { radius, .. } => {
                let r = self.sx_at(&radius, p);
                self.ir.splat2(r)
            }
            Shape::Capsule { from, to, radius } => {
                let a = self.v2x_at(&from, p);
                let b = self.v2x_at(&to, p);
                let delta = self.ir.sub(b, a);
                let abs_delta = self.ir.m1(Mf::Abs, delta);
                let half = self.ir.lit(0.5);
                let half2 = self.ir.splat2(half);
                let half_delta = self.ir.mul(abs_delta, half2);
                let r = self.sx_at(&radius, p);
                let r2 = self.ir.splat2(r);
                self.ir.addx(half_delta, r2)
            }
            Shape::RBox { half, .. } => self.v2x_at(&half, p),
            Shape::Ellipse { radii, .. } => self.v2x_at(&radii, p),
            Shape::Star { outer, .. } => {
                let r = self.sx_at(&outer, p);
                self.ir.splat2(r)
            }
            Shape::Triangle { a, b, c } => {
                let anchor = self.shape_anchor(id, p);
                let va = self.v2x_at(&a, p);
                let vb = self.v2x_at(&b, p);
                let vc = self.v2x_at(&c, p);
                let va_delta = self.ir.sub(va, anchor);
                let vb_delta = self.ir.sub(vb, anchor);
                let vc_delta = self.ir.sub(vc, anchor);
                let da = self.ir.m1(Mf::Abs, va_delta);
                let db = self.ir.m1(Mf::Abs, vb_delta);
                let dc = self.ir.m1(Mf::Abs, vc_delta);
                let ab = self.ir.m2(Mf::Max, da, db);
                self.ir.m2(Mf::Max, ab, dc)
            }
            Shape::Polygon { contours, .. } => {
                let anchor = self.shape_anchor(id, p);
                let zero = self.ir.lit(0.0);
                let mut extent = self.ir.vec2(zero, zero);
                for contour in contours {
                    for vertex in contour {
                        let v = self.v2x_at(&vertex, p);
                        let v_delta = self.ir.sub(v, anchor);
                        let d = self.ir.m1(Mf::Abs, v_delta);
                        extent = self.ir.m2(Mf::Max, extent, d);
                    }
                }
                extent
            }
            Shape::LineFamily { .. } | Shape::GridLine { .. } => {
                // Line families are infinite; return a large extent.
                let big = self.ir.lit(1e6);
                self.ir.splat2(big)
            }
            Shape::Outline { inner, width } => {
                let inner_extent = self.shape_half_extent(inner, p);
                let half = self.ir.lit(0.5);
                let w = self.sx_at(&width, p);
                let pad = self.ir.mul(w, half);
                let pad2 = self.ir.splat2(pad);
                self.ir.addx(inner_extent, pad2)
            }
            Shape::Offset { inner, delta } => {
                let inner_extent = self.shape_half_extent(inner, p);
                let delta = self.sx_at(&delta, p);
                let delta_abs = self.ir.m1(Mf::Abs, delta);
                let delta2 = self.ir.splat2(delta_abs);
                self.ir.addx(inner_extent, delta2)
            }
            Shape::Rotate { inner, .. } => {
                let inner_extent = self.shape_half_extent(inner, p);
                let ex = self.ir.x_of(inner_extent);
                let ey = self.ir.y_of(inner_extent);
                let max_extent = self.ir.m2(Mf::Max, ex, ey);
                self.ir.splat2(max_extent)
            }
            Shape::Mix(a, b, _)
            | Shape::Union(a, b)
            | Shape::SmoothUnion(a, b, _)
            | Shape::Intersect(a, b)
            | Shape::Subtract(a, b) => {
                let ea = self.shape_half_extent(a, p);
                let eb = self.shape_half_extent(b, p);
                self.ir.m2(Mf::Max, ea, eb)
            }
        };
        self.shape_half_extent_cache.insert((id, p), extent);
        extent
    }

    pub(super) fn shape_local_point(&mut self, id: ShapeId, p: Handle<Ex>) -> Handle<Ex> {
        if let Some(cached) = self.shape_local_point_cache.get(&(id, p)).copied() {
            return cached;
        }

        let anchor = self.shape_anchor(id, p);
        let extent = self.shape_half_extent(id, p);
        let eps = self.eps();
        let eps2 = self.ir.splat2(eps);
        let safe_extent = self.ir.m2(Mf::Max, extent, eps2);
        let two = self.ir.lit(2.0);
        let two2 = self.ir.splat2(two);
        let denom = self.ir.mul(safe_extent, two2);
        let delta = self.ir.sub(p, anchor);
        let unit = self.ir.div(delta, denom);
        let half = self.ir.lit(0.5);
        let half2 = self.ir.splat2(half);
        let local_point = self.ir.addx(half2, unit);
        self.shape_local_point_cache.insert((id, p), local_point);
        local_point
    }

    pub(super) fn shape_anchor_space_point(
        &mut self,
        id: ShapeId,
        anchor_sample_p: Handle<Ex>,
        sample_p: Handle<Ex>,
    ) -> Handle<Ex> {
        let key = (id, anchor_sample_p, sample_p);
        if let Some(cached) = self.shape_anchor_space_point_cache.get(&key).copied() {
            return cached;
        }

        let anchor = self.shape_anchor(id, anchor_sample_p);
        let half_x = self.ir.lit(0.5);
        let half_y = self.ir.lit(0.5);
        let center = self.ir.vec2(half_x, half_y);
        let delta = self.ir.sub(sample_p, anchor);
        let point = self.ir.addx(center, delta);
        self.shape_anchor_space_point_cache.insert(key, point);
        point
    }

    pub(super) fn shape_glow_warp(
        &mut self,
        id: ShapeId,
        p: Handle<Ex>,
        rx: Handle<Ex>,
        ry: Handle<Ex>,
    ) -> (Handle<Ex>, Handle<Ex>) {
        let key = (id, p, rx, ry);
        if let Some(cached) = self.shape_glow_warp_cache.get(&key).copied() {
            return cached;
        }

        let anchor = self.shape_anchor(id, p);
        let q = self.ir.sub(p, anchor);
        let qx = self.ir.x_of(q);
        let qy = self.ir.y_of(q);
        let eps = self.ir.lit(1.0e-6);
        let abs_rx = self.ir.m1(Mf::Abs, rx);
        let abs_ry = self.ir.m1(Mf::Abs, ry);
        let safe_rx = self.ir.m2(Mf::Max, abs_rx, eps);
        let safe_ry = self.ir.m2(Mf::Max, abs_ry, eps);
        let max_r = self.ir.m2(Mf::Max, safe_rx, safe_ry);
        let sx = self.ir.div(max_r, safe_rx);
        let sy = self.ir.div(max_r, safe_ry);
        let qx_scaled = self.ir.mul(qx, sx);
        let qy_scaled = self.ir.mul(qy, sy);
        let scaled = self.ir.vec2(qx_scaled, qy_scaled);
        let glow_p = self.ir.addx(anchor, scaled);
        let out = (glow_p, max_r);
        self.shape_glow_warp_cache.insert(key, out);
        out
    }

    pub(super) fn sdf(&mut self, id: ShapeId, p: Handle<Ex>) -> Handle<Ex> {
        if let Some(h) = self.sdf_cache.get(&(id, p)) {
            self.stats.sdf_cache_hits += 1;
            return *h;
        }
        self.stats.sdf_evals += 1;
        let d = match self.hir.shapes[id].clone() {
            Shape::Circle { center, radius } => {
                let c = self.v2x_at(&center, p);
                let q = self.ir.sub(p, c);
                let len = self.ir.m1(Mf::Length, q);
                let r = self.sx_at(&radius, p);
                self.ir.sub(len, r)
            }
            Shape::Capsule { from, to, radius } => {
                // Segment SDF: |pa - ba * clamp(dot(pa,ba)/dot(ba,ba), 0,1)| - r
                let a = self.v2x_at(&from, p);
                let b = self.v2x_at(&to, p);
                let pa = self.ir.sub(p, a);
                let ba = self.ir.sub(b, a);

                let pax = self.ir.x_of(pa);
                let pay = self.ir.y_of(pa);
                let bax = self.ir.x_of(ba);
                let bay = self.ir.y_of(ba);

                let pax_bax = self.ir.mul(pax, bax);
                let pay_bay = self.ir.mul(pay, bay);
                let dot_p = self.ir.addx(pax_bax, pay_bay);
                let bax_bax = self.ir.mul(bax, bax);
                let bay_bay = self.ir.mul(bay, bay);
                let dot_b = self.ir.addx(bax_bax, bay_bay);
                let t_raw = self.safe_div_positive(dot_p, dot_b);
                let zero = self.ir.lit(0.0);
                let one = self.ir.lit(1.0);
                let t = self.ir.m3(Mf::Clamp, t_raw, zero, one);

                let tbax = self.ir.mul(t, bax);
                let tbay = self.ir.mul(t, bay);
                let qx = self.ir.sub(pax, tbax);
                let qy = self.ir.sub(pay, tbay);
                let q = self.ir.vec2(qx, qy);
                let len = self.ir.m1(Mf::Length, q);
                let r = self.sx_at(&radius, p);
                self.ir.sub(len, r)
            }
            Shape::RBox {
                center,
                half,
                round,
            } => {
                // q = abs(p - c) - half + r;  d = length(max(q,0)) + min(max(q.x,q.y),0) - r
                let c = self.v2x_at(&center, p);
                let half = self.v2x_at(&half, p);
                let r = self.sx_at(&round, p);
                let pc = self.ir.sub(p, c);
                let apc = self.ir.m1(Mf::Abs, pc);
                let q0 = self.ir.sub(apc, half);
                let rs = self.ir.splat2(r);
                let q = self.ir.addx(q0, rs);
                self.ir.name(q, &format!("box_q_s{id}"));
                let zero = self.ir.lit(0.0);
                let z2 = self.ir.splat2(zero);
                let mx2 = self.ir.m2(Mf::Max, q, z2);
                self.ir.name(mx2, &format!("box_q_pos_s{id}"));
                let outer = self.ir.m1(Mf::Length, mx2);
                let qx = self.ir.x_of(q);
                let qy = self.ir.y_of(q);
                let corner = self.ir.m2(Mf::Max, qx, qy);
                let inner = self.ir.m2(Mf::Min, corner, zero);
                self.ir.name(inner, &format!("box_q_neg_s{id}"));
                let dsum = self.ir.addx(outer, inner);
                self.ir.sub(dsum, r)
            }
            Shape::Ellipse { center, radii } => {
                // Ellipse SDF approximation
                let c = self.v2x_at(&center, p);
                let ab = self.v2x_at(&radii, p);
                let pc = self.ir.sub(p, c);

                let px = self.ir.x_of(pc);
                let py = self.ir.y_of(pc);
                let ax = self.ir.x_of(ab);
                let ay = self.ir.y_of(ab);

                // Normalize by radii: (p/ab)
                let nx = self.safe_div_positive(px, ax);
                let ny = self.safe_div_positive(py, ay);
                let n = self.ir.vec2(nx, ny);
                let len_n = self.ir.m1(Mf::Length, n);

                // Distance approximation: (len(p/ab) - 1) * min(ab)
                let one = self.ir.lit(1.0);
                let len_minus_1 = self.ir.sub(len_n, one);
                let min_ab = self.ir.m2(Mf::Min, ax, ay);
                let d = self.ir.mul(len_minus_1, min_ab);
                self.ir.name(d, &format!("ellipse_d_s{id}"));
                d
            }
            Shape::Star {
                center,
                outer,
                inner,
                points,
            } => {
                // Star SDF: use polar coordinates and modulo to repeat around center
                let c = self.v2x_at(&center, p);
                let pc = self.ir.sub(p, c);
                let px = self.ir.x_of(pc);
                let py = self.ir.y_of(pc);

                // Polar angle phase-shifted so the first lobe points upward.
                let angle_raw = self.ir.m2(Mf::Atan2, py, px);
                let up_phase = self.ir.lit(std::f32::consts::FRAC_PI_2);
                let angle = self.ir.sub(angle_raw, up_phase);

                // Sector angle for one star point (full rotation / points)
                let sector = std::f32::consts::TAU / points as f32;
                let half_sector = sector * 0.5;

                // Fold angle into one sector: angle_mod = angle - floor(angle / sector) * sector
                let sector_lit = self.ir.lit(sector);
                let half_lit = self.ir.lit(half_sector);
                let angle_norm = self.ir.addx(angle, half_lit);
                let div = self.ir.div(angle_norm, sector_lit);
                let floored = self.ir.m1(Mf::Floor, div);
                let mult = self.ir.mul(floored, sector_lit);
                let angle_mod = self.ir.sub(angle_norm, mult);
                let folded = self.ir.sub(angle_mod, half_lit);

                // Compute radius at this angle (lerp between inner and outer)
                let abs_folded = self.ir.m1(Mf::Abs, folded);
                let t = self.ir.div(abs_folded, half_lit);
                let outer_r = self.sx_at(&outer, p);
                let inner_r = self.sx_at(&inner, p);
                let r = self.ir.m3(Mf::Mix, outer_r, inner_r, t);

                // Distance from this radius
                let len_p = self.ir.m1(Mf::Length, pc);
                let d = self.ir.sub(len_p, r);
                self.ir.name(d, &format!("star{}_d_s{id}", points));
                d
            }
            Shape::Triangle { a, b, c } => {
                // Triangle SDF using edge distances with winding-invariant sign.
                let va = self.v2x_at(&a, p);
                let vb = self.v2x_at(&b, p);
                let vc = self.v2x_at(&c, p);

                let pa = self.ir.sub(p, va);
                let ba = self.ir.sub(vb, va);
                let pb = self.ir.sub(p, vb);
                let cb = self.ir.sub(vc, vb);
                let pc_vec = self.ir.sub(p, vc);
                let ac = self.ir.sub(va, vc);

                // Compute distance to first edge (AB)
                let pa_x = self.ir.x_of(pa);
                let pa_y = self.ir.y_of(pa);
                let ba_x = self.ir.x_of(ba);
                let ba_y = self.ir.y_of(ba);
                let pa_ba_x = self.ir.mul(pa_x, ba_x);
                let pa_ba_y = self.ir.mul(pa_y, ba_y);
                let dot_pa_ba = self.ir.addx(pa_ba_x, pa_ba_y);
                let ba_ba_x = self.ir.mul(ba_x, ba_x);
                let ba_ba_y = self.ir.mul(ba_y, ba_y);
                let dot_ba_ba = self.ir.addx(ba_ba_x, ba_ba_y);
                let t1_raw = self.safe_div_positive(dot_pa_ba, dot_ba_ba);
                let zero = self.ir.lit(0.0);
                let one = self.ir.lit(1.0);
                let t1 = self.ir.m3(Mf::Clamp, t1_raw, zero, one);
                let t1_ba_x = self.ir.mul(t1, ba_x);
                let t1_ba_y = self.ir.mul(t1, ba_y);
                let q1_x = self.ir.sub(pa_x, t1_ba_x);
                let q1_y = self.ir.sub(pa_y, t1_ba_y);
                let q1_x2 = self.ir.mul(q1_x, q1_x);
                let q1_y2 = self.ir.mul(q1_y, q1_y);
                let d1 = self.ir.addx(q1_x2, q1_y2);

                // Compute distance to second edge (BC)
                let pb_x = self.ir.x_of(pb);
                let pb_y = self.ir.y_of(pb);
                let cb_x = self.ir.x_of(cb);
                let cb_y = self.ir.y_of(cb);
                let pb_cb_x = self.ir.mul(pb_x, cb_x);
                let pb_cb_y = self.ir.mul(pb_y, cb_y);
                let dot_pb_cb = self.ir.addx(pb_cb_x, pb_cb_y);
                let cb_cb_x = self.ir.mul(cb_x, cb_x);
                let cb_cb_y = self.ir.mul(cb_y, cb_y);
                let dot_cb_cb = self.ir.addx(cb_cb_x, cb_cb_y);
                let t2_raw = self.safe_div_positive(dot_pb_cb, dot_cb_cb);
                let t2 = self.ir.m3(Mf::Clamp, t2_raw, zero, one);
                let t2_cb_x = self.ir.mul(t2, cb_x);
                let t2_cb_y = self.ir.mul(t2, cb_y);
                let q2_x = self.ir.sub(pb_x, t2_cb_x);
                let q2_y = self.ir.sub(pb_y, t2_cb_y);
                let q2_x2 = self.ir.mul(q2_x, q2_x);
                let q2_y2 = self.ir.mul(q2_y, q2_y);
                let d2 = self.ir.addx(q2_x2, q2_y2);

                // Compute distance to third edge (CA)
                let pc_x = self.ir.x_of(pc_vec);
                let pc_y = self.ir.y_of(pc_vec);
                let ac_x = self.ir.x_of(ac);
                let ac_y = self.ir.y_of(ac);
                let pc_ac_x = self.ir.mul(pc_x, ac_x);
                let pc_ac_y = self.ir.mul(pc_y, ac_y);
                let dot_pc_ac = self.ir.addx(pc_ac_x, pc_ac_y);
                let ac_ac_x = self.ir.mul(ac_x, ac_x);
                let ac_ac_y = self.ir.mul(ac_y, ac_y);
                let dot_ac_ac = self.ir.addx(ac_ac_x, ac_ac_y);
                let t3_raw = self.safe_div_positive(dot_pc_ac, dot_ac_ac);
                let t3 = self.ir.m3(Mf::Clamp, t3_raw, zero, one);
                let t3_ac_x = self.ir.mul(t3, ac_x);
                let t3_ac_y = self.ir.mul(t3, ac_y);
                let q3_x = self.ir.sub(pc_x, t3_ac_x);
                let q3_y = self.ir.sub(pc_y, t3_ac_y);
                let q3_x2 = self.ir.mul(q3_x, q3_x);
                let q3_y2 = self.ir.mul(q3_y, q3_y);
                let d3 = self.ir.addx(q3_x2, q3_y2);

                let min12 = self.ir.m2(Mf::Min, d1, d2);
                let min_all = self.ir.m2(Mf::Min, min12, d3);
                let d_unsigned = self.ir.m1(Mf::Sqrt, min_all);

                // Use signed edge orientation from Inigo Quilez's robust triangle SDF.
                // Negative inside, positive outside, independent of CW/CCW vertex order.
                let ba_ac_x = self.ir.mul(ba_x, ac_y);
                let ba_ac_y = self.ir.mul(ba_y, ac_x);
                let tri_orient = self.ir.sub(ba_ac_x, ba_ac_y);
                let tri_sign = self.ir.m1(Mf::Sign, tri_orient);

                let pa_ba_cross_x = self.ir.mul(pa_x, ba_y);
                let pa_ba_cross_y = self.ir.mul(pa_y, ba_x);
                let edge0_cross = self.ir.sub(pa_ba_cross_x, pa_ba_cross_y);
                let edge0_signed = self.ir.mul(tri_sign, edge0_cross);

                let pb_cb_cross_x = self.ir.mul(pb_x, cb_y);
                let pb_cb_cross_y = self.ir.mul(pb_y, cb_x);
                let edge1_cross = self.ir.sub(pb_cb_cross_x, pb_cb_cross_y);
                let edge1_signed = self.ir.mul(tri_sign, edge1_cross);

                let pc_ac_cross_x = self.ir.mul(pc_x, ac_y);
                let pc_ac_cross_y = self.ir.mul(pc_y, ac_x);
                let edge2_cross = self.ir.sub(pc_ac_cross_x, pc_ac_cross_y);
                let edge2_signed = self.ir.mul(tri_sign, edge2_cross);

                let min01 = self.ir.m2(Mf::Min, edge0_signed, edge1_signed);
                let min_edge = self.ir.m2(Mf::Min, min01, edge2_signed);
                let edge_sign = self.ir.m1(Mf::Sign, min_edge);
                let neg_dist = self.ir.neg(d_unsigned);
                let signed_d = self.ir.mul(neg_dist, edge_sign);
                self.ir.name(signed_d, &format!("tri_d_s{id}"));
                signed_d
            }
            Shape::Polygon {
                contours,
                fill_rule,
            } => {
                let big = self.ir.lit(1.0e9);
                let mut min_d2 = big;
                let zero = self.ir.lit(0.0);
                let one = self.ir.lit(1.0);
                let half = self.ir.lit(0.5);
                let quarter = self.ir.lit(0.25);
                let mut winding = zero;
                let mut crossings = zero;

                let px = self.ir.x_of(p);
                let py = self.ir.y_of(p);

                for contour in &contours {
                    if contour.len() < 2 {
                        continue;
                    }
                    for i in 0..contour.len() {
                        let a = self.v2x_at(&contour[i], p);
                        let b = self.v2x_at(&contour[(i + 1) % contour.len()], p);

                        // Unsigned distance to segment a->b.
                        let pa = self.ir.sub(p, a);
                        let ba = self.ir.sub(b, a);
                        let pa_x = self.ir.x_of(pa);
                        let pa_y = self.ir.y_of(pa);
                        let ba_x = self.ir.x_of(ba);
                        let ba_y = self.ir.y_of(ba);

                        let dot_pa_ba_x = self.ir.mul(pa_x, ba_x);
                        let dot_pa_ba_y = self.ir.mul(pa_y, ba_y);
                        let dot_pa_ba = self.ir.addx(dot_pa_ba_x, dot_pa_ba_y);
                        let dot_ba_ba_x = self.ir.mul(ba_x, ba_x);
                        let dot_ba_ba_y = self.ir.mul(ba_y, ba_y);
                        let dot_ba_ba = self.ir.addx(dot_ba_ba_x, dot_ba_ba_y);
                        let t_raw = self.safe_div_positive(dot_pa_ba, dot_ba_ba);
                        let t = self.ir.m3(Mf::Clamp, t_raw, zero, one);
                        let proj_x = self.ir.mul(t, ba_x);
                        let proj_y = self.ir.mul(t, ba_y);
                        let q_x = self.ir.sub(pa_x, proj_x);
                        let q_y = self.ir.sub(pa_y, proj_y);
                        let q_x2 = self.ir.mul(q_x, q_x);
                        let q_y2 = self.ir.mul(q_y, q_y);
                        let d2 = self.ir.addx(q_x2, q_y2);
                        min_d2 = self.ir.m2(Mf::Min, min_d2, d2);

                        // Inside tests by horizontal ray crossings.
                        let a_x = self.ir.x_of(a);
                        let a_y = self.ir.y_of(a);
                        let b_x = self.ir.x_of(b);
                        let b_y = self.ir.y_of(b);

                        let a_le_py = self.ir.bin(Bo::LessEqual, a_y, py);
                        let b_gt_py = self.ir.bin(Bo::Greater, b_y, py);
                        let up_cross = self.ir.bin(Bo::LogicalAnd, a_le_py, b_gt_py);

                        let a_gt_py = self.ir.bin(Bo::Greater, a_y, py);
                        let b_le_py = self.ir.bin(Bo::LessEqual, b_y, py);
                        let down_cross = self.ir.bin(Bo::LogicalAnd, a_gt_py, b_le_py);

                        let dy = self.ir.sub(b_y, a_y);
                        let denom = self.clamp_signed_nonzero(dy);

                        let py_minus_ay = self.ir.sub(py, a_y);
                        let t_ray = self.ir.div(py_minus_ay, denom);
                        let bx_minus_ax = self.ir.sub(b_x, a_x);
                        let ray_step = self.ir.mul(t_ray, bx_minus_ax);
                        let x_inter = self.ir.addx(a_x, ray_step);
                        let ray_hit_left = self.ir.bin(Bo::Less, px, x_inter);

                        let one_or_zero_hit = self.ir.add(Ex::Select {
                            condition: ray_hit_left,
                            accept: one,
                            reject: zero,
                        });

                        let up_hit = self.ir.add(Ex::Select {
                            condition: up_cross,
                            accept: one_or_zero_hit,
                            reject: zero,
                        });
                        let down_hit = self.ir.add(Ex::Select {
                            condition: down_cross,
                            accept: one_or_zero_hit,
                            reject: zero,
                        });
                        crossings = self.ir.addx(crossings, up_hit);
                        crossings = self.ir.addx(crossings, down_hit);

                        // Non-zero winding contribution.
                        let p_minus_a_x = self.ir.sub(px, a_x);
                        let p_minus_a_y = self.ir.sub(py, a_y);
                        let cross_x = self.ir.mul(bx_minus_ax, p_minus_a_y);
                        let by_minus_ay = self.ir.sub(b_y, a_y);
                        let cross_y = self.ir.mul(by_minus_ay, p_minus_a_x);
                        let edge_cross = self.ir.sub(cross_x, cross_y);

                        let cross_pos = self.ir.bin(Bo::Greater, edge_cross, zero);
                        let cross_neg = self.ir.bin(Bo::Less, edge_cross, zero);
                        let up_unit = self.ir.add(Ex::Select {
                            condition: cross_pos,
                            accept: one,
                            reject: zero,
                        });
                        let down_unit = self.ir.add(Ex::Select {
                            condition: cross_neg,
                            accept: one,
                            reject: zero,
                        });
                        let up_wind = self.ir.add(Ex::Select {
                            condition: up_cross,
                            accept: up_unit,
                            reject: zero,
                        });
                        let down_wind = self.ir.add(Ex::Select {
                            condition: down_cross,
                            accept: down_unit,
                            reject: zero,
                        });
                        winding = self.ir.addx(winding, up_wind);
                        winding = self.ir.sub(winding, down_wind);
                    }
                }

                let d_unsigned = self.ir.m1(Mf::Sqrt, min_d2);
                let inside = match fill_rule {
                    crate::hir::FillRule::NonZero => self.ir.bin(Bo::NotEqual, winding, zero),
                    crate::hir::FillRule::EvenOdd => {
                        let half_cross = self.ir.mul(crossings, half);
                        let frac = self.ir.m1(Mf::Fract, half_cross);
                        self.ir.bin(Bo::Greater, frac, quarter)
                    }
                };

                let neg_one = self.ir.neg(one);
                let inside_sign = self.ir.add(Ex::Select {
                    condition: inside,
                    accept: neg_one,
                    reject: one,
                });
                let signed_d = self.ir.mul(d_unsigned, inside_sign);
                self.ir.name(signed_d, &format!("poly_d_s{id}"));
                signed_d
            }
            Shape::LineFamily {
                axis,
                spacing,
                offset,
            } => {
                // Periodic line family SDF: d(p) = (|fract(p[axis]/spacing + offset + 0.5) - 0.5|) * spacing
                // This is 1-Lipschitz exact (§10.1).
                let coord = if axis == 0 {
                    self.ir.x_of(p)
                } else {
                    self.ir.y_of(p)
                };
                let spacing_raw = self.sx_at(&spacing, p);
                // Keep periodic spacing positive and non-zero to avoid
                // unstable fract/divide behavior when authored spacing is
                // zero, negative, or extremely small.
                let spacing_val = self.clamp_abs_nonzero(spacing_raw);
                let offset_val = self.sx_at(&offset, p);

                // coord / spacing
                let normalized = self.ir.div(coord, spacing_val);
                // normalized + offset + 0.5
                let half = self.ir.lit(0.5);
                let phase = self.ir.addx(normalized, offset_val);
                let phase_shifted = self.ir.addx(phase, half);
                // fract(phase_shifted)
                let frac = self.ir.m1(Mf::Fract, phase_shifted);
                // frac - 0.5
                let centered = self.ir.sub(frac, half);
                // abs(centered)
                let abs_centered = self.ir.m1(Mf::Abs, centered);
                // abs_centered * spacing
                let d = self.ir.mul(abs_centered, spacing_val);
                self.ir.name(d, &format!("lines_d_s{id}"));
                d
            }
            Shape::GridLine { axis, at } => {
                // Single infinite line SDF: d(p) = |p[axis] - at|
                // Avoids the precision loss from emulating a single line
                // via an extremely large periodic spacing.
                let coord = if axis == 0 {
                    self.ir.x_of(p)
                } else {
                    self.ir.y_of(p)
                };
                let at = self.sx_at(&at, p);
                let delta = self.ir.sub(coord, at);
                let d = self.ir.m1(Mf::Abs, delta);
                self.ir.name(d, &format!("gridline_d_s{id}"));
                d
            }
            Shape::Outline { inner, width } => {
                // stroke(w) ⇒ |d| - w/2  (§10.1)
                let d = self.sdf(inner, p);
                let ad = self.ir.m1(Mf::Abs, d);
                self.ir.name(ad, &format!("outline_abs_s{id}"));
                let w = self.sx_at(&width, p);
                let half = self.ir.lit(0.5);
                let hw = self.ir.mul(w, half);
                self.ir.name(hw, &format!("outline_half_w_s{id}"));
                self.ir.sub(ad, hw)
            }
            Shape::Offset { inner, delta } => {
                // d' = d - delta
                let d = self.sdf(inner, p);
                let delta = self.sx_at(&delta, p);
                self.ir.sub(d, delta)
            }
            Shape::Rotate { inner, angle } => {
                // Rotate query point into the inner shape's local frame.
                // d_rot(p) = d_inner(anchor + R(-a) * (p - anchor))
                let anchor = self.shape_anchor(inner, p);
                let delta = self.ir.sub(p, anchor);
                let dx = self.ir.x_of(delta);
                let dy = self.ir.y_of(delta);

                let a = self.sx_at(&angle, p);
                let ca = self.ir.m1(Mf::Cos, a);
                let sa = self.ir.m1(Mf::Sin, a);

                let dx_ca = self.ir.mul(dx, ca);
                let dy_sa = self.ir.mul(dy, sa);
                let local_x = self.ir.addx(dx_ca, dy_sa);

                let dx_sa = self.ir.mul(dx, sa);
                let dy_ca = self.ir.mul(dy, ca);
                let local_y = self.ir.sub(dy_ca, dx_sa);

                let local = self.ir.vec2(local_x, local_y);
                let rotated_p = self.ir.addx(anchor, local);
                self.sdf(inner, rotated_p)
            }
            Shape::Mix(a, b, t) => {
                let da = self.sdf(a, p);
                let db = self.sdf(b, p);
                let t = self.sx_at(&t, p);
                self.ir.m3(Mf::Mix, da, db, t)
            }
            Shape::Union(a, b) => {
                let da = self.sdf(a, p);
                let db = self.sdf(b, p);
                self.ir.m2(Mf::Min, da, db)
            }
            Shape::Intersect(a, b) => {
                let da = self.sdf(a, p);
                let db = self.sdf(b, p);
                self.ir.m2(Mf::Max, da, db)
            }
            Shape::Subtract(a, b) => {
                let da = self.sdf(a, p);
                let db = self.sdf(b, p);
                let nb = self.ir.neg(db);
                self.ir.m2(Mf::Max, da, nb)
            }
            Shape::SmoothUnion(a, b, k) => {
                // polynomial smin: h = clamp(0.5 + 0.5*(db-da)/k, 0, 1)
                //                  d = mix(db, da, h) - k*h*(1-h)
                let da = self.sdf(a, p);
                let db = self.sdf(b, p);
                let k_raw = self.sx_at(&k, p);
                // Smooth radius behaves as a magnitude; clamp away from zero
                // so (db-da)/k remains numerically stable.
                let k = self.clamp_abs_nonzero(k_raw);
                let half = self.ir.lit(0.5);
                let diff = self.ir.sub(db, da);
                let t0 = self.ir.mul(half, diff);
                let t1 = self.ir.div(t0, k);
                let t2 = self.ir.addx(half, t1);
                let zero = self.ir.lit(0.0);
                let one = self.ir.lit(1.0);
                let h = self.ir.m3(Mf::Clamp, t2, zero, one);
                self.ir.name(h, &format!("smin_h_s{id}"));
                let m = self.ir.m3(Mf::Mix, db, da, h);
                self.ir.name(m, &format!("smin_mix_s{id}"));
                let omh = self.ir.sub(one, h);
                let kh = self.ir.mul(k, h);
                let corr = self.ir.mul(kh, omh);
                self.ir.name(corr, &format!("smin_corr_s{id}"));
                self.ir.sub(m, corr)
            }
        };
        self.ir.name(d, &format!("d_s{id}"));
        self.sdf_cache.insert((id, p), d);
        d
    }

    /// fill(s) ⇒ clamp(0.5 - d/aa, 0, 1)  (§10.1)
    ///
    /// Shape SDFs are evaluated from the current sample point `p`, which already
    /// includes any active `in space` remapping. Therefore `dpdx(d)` / `dpdy(d)`
    /// already encode the local-to-screen footprint and should be used directly
    /// for adaptive AA width.
    pub(super) fn coverage(&mut self, _id: ShapeId, d: Handle<Ex>, _p: Handle<Ex>) -> Handle<Ex> {
        if self.cellular_sample_depth > 0 {
            let zero = self.ir.lit(0.0);
            let one = self.ir.lit(1.0);
            let inside = self.ir.bin(Bo::Less, d, zero);
            return self.ir.add(Ex::Select {
                condition: inside,
                accept: one,
                reject: zero,
            });
        }
        let aa = self.adaptive_aa_width(d);
        self.coverage_from_width(d, aa)
    }

    /// soften(fill(s), r) ⇒ 1 - smoothstep(-r/2, r/2, d)  (§10.2)
    pub(super) fn soft_coverage(
        &mut self,
        _id: ShapeId,
        d: Handle<Ex>,
        r: &Sx,
        p: Handle<Ex>,
    ) -> Handle<Ex> {
        let r = self.sx_at(r, p);
        let half = self.ir.lit(0.5);
        let hr = self.ir.mul(r, half);
        self.ir.name(hr, "soft_half_r");
        let nhr = self.ir.neg(hr);
        let ss = self.ir.m3(Mf::SmoothStep, nhr, hr, d);
        let one = self.ir.lit(1.0);
        let cov = self.ir.sub(one, ss);

        // Cellular quadrature already integrates this authored softness band.
        if self.cellular_sample_depth > 0 {
            return cov;
        }

        // Rung (b) widening is only valid while the soften band remains small
        // relative to the footprint crossing the feature. When the footprint
        // grows beyond the band, cap contribution by the feature mean instead
        // of letting smoothstep widening invent a fatter shape.
        let aa = self.adaptive_aa_width(d);
        self.saturate_coverage_by_mean(
            cov,
            r,
            aa,
            "soft_feature_w",
            "soft_mean_cap",
            "soft_cov_saturated",
        )
    }
}
