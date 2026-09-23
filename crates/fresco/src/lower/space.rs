use super::*;

impl<'h> FnCtx<'h> {
    /// Carry a periodic cell through a separable affine sample map. A general
    /// rotation/warp does not leave an axis-aligned cell, so it cannot use the
    /// interval specialization and deliberately does not inherit this metadata.
    fn remap_box_filter_domain(
        &mut self,
        input: Handle<Ex>,
        output: Handle<Ex>,
        scale: Handle<Ex>,
        offset: Handle<Ex>,
    ) -> Handle<Ex> {
        if let Some(domain) = self.box_filter_domains.get(&input).copied() {
            let sx = self.ir.x_of(scale);
            let ox = self.ir.x_of(offset);
            let a = self.ir.mul(domain.x_bounds.0, sx);
            let a = self.ir.addx(a, ox);
            let b = self.ir.mul(domain.x_bounds.1, sx);
            let b = self.ir.addx(b, ox);
            let lo = self.ir.m2(Mf::Min, a, b);
            let hi = self.ir.m2(Mf::Max, a, b);
            let dx = self.ir.mul(domain.dx, scale);
            let dy = self.ir.mul(domain.dy, scale);
            self.box_filter_domains.insert(
                output,
                BoxFilterDomain {
                    x_bounds: (lo, hi),
                    dx,
                    dy,
                },
            );
        }
        output
    }

    fn polar_box_filter_domain(
        &mut self,
        input: Handle<Ex>,
        output: Handle<Ex>,
        q: Handle<Ex>,
        xf: &Xform,
    ) {
        let prev_feature_tag = self.ir.set_feature_tag(Some("shape_aa"));
        let (a, b, c, d) = crate::deriv::jacobian(std::slice::from_ref(xf));
        let a = self.sx_at(&a, input);
        let b = self.sx_at(&b, input);
        let c = self.sx_at(&c, input);
        let d = self.sx_at(&d, input);
        let row_x = self.ir.vec2(a, b);
        let row_y = self.ir.vec2(c, d);
        let (dx, dy) = if let Some(domain) = self.box_filter_domains.get(&input).copied() {
            (domain.dx, domain.dy)
        } else {
            (self.ir.dpdx(input), self.ir.dpdy(input))
        };
        let xx = self.ir.m2(Mf::Dot, row_x, dx);
        let xy = self.ir.m2(Mf::Dot, row_y, dx);
        let yx = self.ir.m2(Mf::Dot, row_x, dy);
        let yy = self.ir.m2(Mf::Dot, row_y, dy);
        let mapped_dx = self.ir.vec2(xx, xy);
        let mapped_dy = self.ir.vec2(yx, yy);
        // The angle is undefined at the center. A pixel containing it sees
        // every direction; use one whole turn instead of a zero/NaN footprint.
        let span_x = self.ir.m1(Mf::Length, dx);
        let span_y = self.ir.m1(Mf::Length, dy);
        let span = self.ir.addx(span_x, span_y);
        let half = self.ir.lit(0.5);
        let half_span = self.ir.mul(span, half);
        let radius = self.ir.m1(Mf::Length, q);
        let at_center = self.ir.bin(Bo::LessEqual, radius, half_span);
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let center_dx = self.ir.vec2(one, span);
        let center_dy = self.ir.vec2(zero, zero);
        let dx = self.ir.add(Ex::Select {
            condition: at_center,
            accept: center_dx,
            reject: mapped_dx,
        });
        let dy = self.ir.add(Ex::Select {
            condition: at_center,
            accept: center_dy,
            reject: mapped_dy,
        });
        self.ir.name(dx, "polar_filter_dx");
        self.ir.name(dy, "polar_filter_dy");
        self.box_filter_domains.insert(
            output,
            BoxFilterDomain {
                x_bounds: (zero, one),
                dx,
                dy,
            },
        );
        self.ir.restore_feature_tag(prev_feature_tag);
    }

    pub(super) fn enter_space(&mut self, p: Handle<Ex>, xforms: &[Xform]) -> Handle<Ex> {
        let mut p = p;
        let mut perspective: Option<(Handle<Ex>, Handle<Ex>)> = None;
        for (index, xf) in xforms.iter().enumerate() {
            p = match xf {
                Xform::Cellular(cells) => self.enter_cellular(p, cells),
                Xform::Rotate { angle, around } => {
                    // p' = C + R(-θ)(p - C), C = canvas center
                    let theta = self.sx_at(angle, p);
                    let ang = self.ir.neg(theta);
                    self.ir.name(ang, &format!("space_ang_t{index}"));
                    let c = self.ir.m1(Mf::Cos, ang);
                    self.ir.name(c, &format!("space_cos_t{index}"));
                    let s = self.ir.m1(Mf::Sin, ang);
                    self.ir.name(s, &format!("space_sin_t{index}"));
                    let center = self.v2x_at(around, p);
                    self.ir.name(center, &format!("space_center_t{index}"));
                    let q = self.ir.sub(p, center);
                    self.ir.name(q, &format!("space_q_t{index}"));
                    let qx = self.ir.x_of(q);
                    let qy = self.ir.y_of(q);
                    // R(-θ) = [c s; -s c] applied to q
                    let cx = self.ir.mul(c, qx);
                    let sy = self.ir.mul(s, qy);
                    let nx = self.ir.addx(cx, sy);
                    self.ir.name(nx, &format!("space_rot_x_t{index}"));
                    let sx_ = self.ir.mul(s, qx);
                    let cy = self.ir.mul(c, qy);
                    let ny = self.ir.sub(cy, sx_);
                    self.ir.name(ny, &format!("space_rot_y_t{index}"));
                    let rq = self.ir.vec2(nx, ny);
                    self.ir.name(rq, &format!("space_rot_t{index}"));
                    self.ir.addx(center, rq)
                }
                Xform::Translate(t) => {
                    let invariant = self.sx_is_sample_point_invariant(&t.0)
                        && self.sx_is_sample_point_invariant(&t.1);
                    let t = self.v2x_at(t, p);
                    self.ir.name(t, &format!("space_offset_t{index}"));
                    let out = self.ir.sub(p, t);
                    if invariant && self.box_filter_domains.contains_key(&p) {
                        let one = self.ir.lit(1.0);
                        let scale = self.ir.splat2(one);
                        let offset = self.ir.neg(t);
                        self.remap_box_filter_domain(p, out, scale, offset);
                    }
                    out
                }
                Xform::Translate3 { by, z } => {
                    let offset = self.v2x_at(by, p);
                    self.ir.name(offset, &format!("space_offset3_t{index}"));
                    let shifted = self.ir.sub(p, offset);
                    let z = self.sx_at(z, p);

                    let one = self.ir.lit(1.0);
                    let half = self.ir.lit(0.5);
                    let center = self.ir.vec2(half, half);

                    let (focal, cam_origin) = match perspective {
                        Some((f, o)) => (f, o),
                        None => (one, center),
                    };

                    let q = self.ir.sub(shifted, cam_origin);
                    let denom = self.ir.addx(focal, z);
                    let factor = self.safe_div_positive(denom, focal);
                    let factor2 = self.ir.splat2(factor);
                    let scaled = self.ir.mul(q, factor2);
                    self.ir.addx(cam_origin, scaled)
                }
                Xform::RotateX { angle, around } => {
                    let center = self.v2x_at(around, p);
                    let q = self.ir.sub(p, center);
                    let qx = self.ir.x_of(q);
                    let qy = self.ir.y_of(q);
                    let angle = self.sx_at(angle, p);
                    let c = self.ir.m1(Mf::Cos, angle);
                    let s = self.ir.m1(Mf::Sin, angle);
                    let (u, v) = match perspective {
                        Some((focal, _)) => {
                            // Flat projective inverse for a plane rotated around X.
                            let focal_c = self.ir.mul(focal, c);
                            let qy_s = self.ir.mul(qy, s);
                            let denom = self.ir.sub(focal_c, qy_s);
                            let guard_span = self.local_pixel_span(p);
                            self.ir
                                .name(guard_span, &format!("space_projective_guard_span_t{index}"));

                            let qy_f = self.ir.mul(qy, focal);
                            let v = self.projective_div_signed_or_far(qy_f, denom, guard_span);
                            let z = self.ir.mul(v, s);
                            let fz = self.ir.addx(focal, z);
                            let qx_fz = self.ir.mul(qx, fz);
                            let u_raw = self.safe_div_positive(qx_fz, focal);
                            let u = self.guard_projective_visible(u_raw, denom, guard_span);
                            (u, v)
                        }
                        None => {
                            // Fallback without explicit perspective: axis stretch.
                            let v = self.safe_div_abs_nonzero(qy, c);
                            (qx, v)
                        }
                    };

                    let qr = self.ir.vec2(u, v);
                    self.ir.addx(center, qr)
                }
                Xform::RotateY { angle, around } => {
                    let center = self.v2x_at(around, p);
                    let q = self.ir.sub(p, center);
                    let qx = self.ir.x_of(q);
                    let qy = self.ir.y_of(q);
                    let angle = self.sx_at(angle, p);
                    let c = self.ir.m1(Mf::Cos, angle);
                    let s = self.ir.m1(Mf::Sin, angle);
                    let (u, v) = match perspective {
                        Some((focal, _)) => {
                            // Flat projective inverse for a plane rotated around Y.
                            let focal_c = self.ir.mul(focal, c);
                            let qx_s = self.ir.mul(qx, s);
                            let denom = self.ir.addx(focal_c, qx_s);
                            let guard_span = self.local_pixel_span(p);
                            self.ir
                                .name(guard_span, &format!("space_projective_guard_span_t{index}"));

                            let qx_f = self.ir.mul(qx, focal);
                            let u = self.projective_div_signed_or_far(qx_f, denom, guard_span);
                            let us = self.ir.mul(u, s);
                            let z = self.ir.neg(us);
                            let fz = self.ir.addx(focal, z);
                            let qy_fz = self.ir.mul(qy, fz);
                            let v_raw = self.safe_div_positive(qy_fz, focal);
                            let v = self.guard_projective_visible(v_raw, denom, guard_span);
                            (u, v)
                        }
                        None => {
                            // Fallback without explicit perspective: axis stretch.
                            let u = self.safe_div_abs_nonzero(qx, c);
                            (u, qy)
                        }
                    };

                    let qr = self.ir.vec2(u, v);
                    self.ir.addx(center, qr)
                }
                Xform::Scale {
                    factor,
                    around: (ax, ay),
                } => {
                    let invariant = self.sx_is_sample_point_invariant(factor)
                        && self.sx_is_sample_point_invariant(ax)
                        && self.sx_is_sample_point_invariant(ay);
                    let k_raw = self.sx_at(factor, p);
                    self.ir.name(k_raw, &format!("space_scale_t{index}"));
                    let k = self.clamp_signed_nonzero(k_raw);
                    let cx = self.sx_at(ax, p);
                    let cy = self.sx_at(ay, p);
                    let center = self.ir.vec2(cx, cy);
                    self.ir.name(center, &format!("space_center_t{index}"));
                    let q = self.ir.sub(p, center);
                    self.ir.name(q, &format!("space_q_t{index}"));
                    let ks = self.ir.splat2(k);
                    self.ir.name(ks, &format!("space_scale2_t{index}"));
                    let qx = self.ir.x_of(q);
                    let qy = self.ir.y_of(q);
                    let kx = self.ir.x_of(ks);
                    let ky = self.ir.y_of(ks);
                    let qsx = self.safe_div_signed_nonzero(qx, kx);
                    let qsy = self.safe_div_signed_nonzero(qy, ky);
                    let qs = self.ir.vec2(qsx, qsy);
                    self.ir.name(qs, &format!("space_scaled_q_t{index}"));
                    let out = self.ir.addx(center, qs);
                    if invariant && self.box_filter_domains.contains_key(&p) {
                        let one = self.ir.lit(1.0);
                        let inv = self.ir.div(one, k);
                        let scale = self.ir.splat2(inv);
                        let scaled_center = self.ir.mul(center, scale);
                        let offset = self.ir.sub(center, scaled_center);
                        self.remap_box_filter_domain(p, out, scale, offset);
                    }
                    out
                }
                Xform::RepeatX(every) => {
                    let every = self.sx_at(every, p);
                    self.ir.name(every, &format!("space_repeat_every_t{index}"));
                    let safe_every = self.clamp_positive(every);
                    self.ir
                        .name(safe_every, &format!("space_repeat_safe_every_t{index}"));

                    let x = self.ir.x_of(p);
                    let y = self.ir.y_of(p);
                    let turns = self.ir.div(x, safe_every);
                    let frac = self.ir.m1(Mf::Fract, turns);
                    let x_wrapped = self.ir.mul(frac, safe_every);
                    self.ir.name(x_wrapped, &format!("space_repeat_x_t{index}"));
                    self.ir.vec2(x_wrapped, y)
                }
                Xform::RepeatY(every) => {
                    let every = self.sx_at(every, p);
                    self.ir.name(every, &format!("space_repeat_every_t{index}"));
                    let safe_every = self.clamp_positive(every);
                    self.ir
                        .name(safe_every, &format!("space_repeat_safe_every_t{index}"));

                    let x = self.ir.x_of(p);
                    let y = self.ir.y_of(p);
                    let turns = self.ir.div(y, safe_every);
                    let frac = self.ir.m1(Mf::Fract, turns);
                    let y_wrapped = self.ir.mul(frac, safe_every);
                    self.ir.name(y_wrapped, &format!("space_repeat_y_t{index}"));
                    self.ir.vec2(x, y_wrapped)
                }
                Xform::Repeat2D { every, cell_scope } => {
                    let ex = self.sx_at(&every.0, p);
                    let ey = self.sx_at(&every.1, p);
                    self.ir.name(ex, &format!("space_repeat_x_every_t{index}"));
                    self.ir.name(ey, &format!("space_repeat_y_every_t{index}"));
                    let safe_ex = self.clamp_positive(ex);
                    let safe_ey = self.clamp_positive(ey);
                    self.ir
                        .name(safe_ex, &format!("space_repeat_x_safe_every_t{index}"));
                    self.ir
                        .name(safe_ey, &format!("space_repeat_y_safe_every_t{index}"));

                    let x = self.ir.x_of(p);
                    let y = self.ir.y_of(p);
                    let turns_x = self.ir.div(x, safe_ex);
                    let turns_y = self.ir.div(y, safe_ey);
                    let cell_x = self.ir.m1(Mf::Floor, turns_x);
                    let cell_y = self.ir.m1(Mf::Floor, turns_y);
                    let uv_x = self.ir.m1(Mf::Fract, turns_x);
                    let uv_y = self.ir.m1(Mf::Fract, turns_y);
                    let x_wrapped = self.ir.mul(uv_x, safe_ex);
                    let y_wrapped = self.ir.mul(uv_y, safe_ey);
                    self.ir
                        .name(x_wrapped, &format!("space_repeat2_x_t{index}"));
                    self.ir
                        .name(y_wrapped, &format!("space_repeat2_y_t{index}"));

                    if let Some(scope_id) = cell_scope {
                        let half = self.ir.lit(0.5);
                        let center_x = self.ir.mul(half, safe_ex);
                        let center_y = self.ir.mul(half, safe_ey);
                        let rand = self.cell_random(cell_x, cell_y, 0);
                        self.repeat_cell_ctx.push(RepeatCellCtx {
                            geometry: None,
                            scope_id: *scope_id,
                            id_x: cell_x,
                            id_y: cell_y,
                            center_x,
                            center_y,
                            uv_x,
                            uv_y,
                            rand,
                        });
                    }

                    self.ir.vec2(x_wrapped, y_wrapped)
                }
                Xform::RepeatRadial {
                    count,
                    around,
                    from,
                    to,
                    angles,
                } => {
                    let center = self.v2x_at(around, p);
                    self.ir
                        .name(center, &format!("space_repeat_radial_center_t{index}"));
                    let q = self.ir.sub(p, center);
                    self.ir.name(q, &format!("space_repeat_radial_q_t{index}"));

                    let qx = self.ir.x_of(q);
                    let qy = self.ir.y_of(q);
                    let theta = self.ir.m2(Mf::Atan2, qy, qx);
                    self.ir
                        .name(theta, &format!("space_repeat_radial_theta_t{index}"));
                    let r = self.ir.m1(Mf::Length, q);
                    self.ir.name(r, &format!("space_repeat_radial_r_t{index}"));

                    let theta_wrapped = if let Some(angles) = angles {
                        let from_h = self.ir.lit(angles[0]);
                        let to_h = self.ir.lit(angles[angles.len() - 1]);
                        let span = self.ir.sub(to_h, from_h);
                        let rel = self.ir.sub(theta, from_h);
                        let turns = self.ir.div(rel, span);
                        let frac = self.ir.m1(Mf::Fract, turns);
                        let wrapped = self.ir.mul(frac, span);
                        let theta_periodic = self.ir.addx(from_h, wrapped);

                        let zero = self.ir.lit(0.0);
                        let mut offset = zero;
                        for pair in angles.windows(2) {
                            let ai = self.ir.lit(pair[0]);
                            let aj = self.ir.lit(pair[1]);
                            let ge_ai = self.ir.m2(Mf::Step, ai, theta_periodic);
                            let ge_aj = self.ir.m2(Mf::Step, aj, theta_periodic);
                            let in_sector = self.ir.sub(ge_ai, ge_aj);
                            let rel_sector = self.ir.sub(theta_periodic, ai);
                            let contribution = self.ir.mul(in_sector, rel_sector);
                            offset = self.ir.addx(offset, contribution);
                        }
                        self.ir.addx(from_h, offset)
                    } else {
                        let from = self.sx_at(from, p);
                        let to = self.sx_at(to, p);
                        let count = self.sx_at(count, p);

                        let one = self.ir.lit(1.0);
                        let safe_count = self.ir.m2(Mf::Max, count, one);
                        let span = self.ir.sub(to, from);
                        let safe_abs_span = self.clamp_abs_nonzero(span);
                        let span_sign = self.ir.m1(Mf::Sign, span);
                        let step_abs = self.ir.div(safe_abs_span, safe_count);
                        let step = self.ir.mul(step_abs, span_sign);

                        let rel = self.ir.sub(theta, from);
                        let turns = self.ir.div(rel, step);
                        let frac = self.ir.m1(Mf::Fract, turns);
                        let offset = self.ir.mul(frac, step);
                        self.ir.addx(from, offset)
                    };
                    self.ir.name(
                        theta_wrapped,
                        &format!("space_repeat_radial_theta_wrapped_t{index}"),
                    );

                    let c = self.ir.m1(Mf::Cos, theta_wrapped);
                    let s = self.ir.m1(Mf::Sin, theta_wrapped);
                    let x = self.ir.mul(r, c);
                    let y = self.ir.mul(r, s);
                    let qr = self.ir.vec2(x, y);
                    self.ir
                        .name(qr, &format!("space_repeat_radial_qr_t{index}"));
                    self.ir.addx(center, qr)
                }
                Xform::Perspective {
                    fov,
                    near,
                    far,
                    origin,
                } => {
                    // Flat-only v1 camera model: establish focal length and
                    // camera origin for pseudo-3D transforms in this chain.
                    let fov = self.sx_at(fov, p);
                    let _near = self.sx_at(near, p);
                    let _far = self.sx_at(far, p);
                    let cam_origin = self.v2x_at(origin, p);

                    let half = self.ir.lit(0.5);
                    let hfov = self.ir.mul(fov, half);
                    let tan_half = self.ir.m1(Mf::Tan, hfov);
                    let safe_tan = self.clamp_abs_nonzero(tan_half);
                    let focal = self.ir.div(half, safe_tan);
                    perspective = Some((focal, cam_origin));
                    p
                }
                Xform::Aspect(ratio) => {
                    let invariant = self.sx_is_sample_point_invariant(ratio);
                    // Keep authored coordinates in a target width/height ratio by
                    // inversely scaling sample-space around center.
                    let ratio = self.sx_at(ratio, p);
                    self.ir.name(ratio, &format!("space_aspect_ratio_t{index}"));

                    let eps = self.ir.lit(1.0e-6);
                    let one = self.ir.lit(1.0);

                    let safe_ratio = self.ir.m2(Mf::Max, ratio, eps);
                    self.ir
                        .name(safe_ratio, &format!("space_aspect_safe_ratio_t{index}"));

                    let res_x = self.ir.x_of(self.ir.res);
                    let res_y = self.ir.y_of(self.ir.res);
                    let safe_res_y = self.ir.m2(Mf::Max, res_y, eps);
                    let screen_aspect = self.ir.div(res_x, safe_res_y);
                    self.ir.name(
                        screen_aspect,
                        &format!("space_aspect_screen_ratio_t{index}"),
                    );
                    let safe_screen_aspect = self.ir.m2(Mf::Max, screen_aspect, eps);

                    let x_ratio = self.ir.div(screen_aspect, safe_ratio);
                    let y_ratio = self.ir.div(safe_ratio, safe_screen_aspect);
                    let inv_x = self.ir.m2(Mf::Max, x_ratio, one);
                    let inv_y = self.ir.m2(Mf::Max, y_ratio, one);
                    let inv = self.ir.vec2(inv_x, inv_y);
                    self.ir.name(inv, &format!("space_aspect_inv_t{index}"));

                    let half = self.ir.lit(0.5);
                    let center = self.ir.vec2(half, half);
                    let q = self.ir.sub(p, center);
                    let qs = self.ir.mul(q, inv);
                    let out = self.ir.addx(center, qs);
                    if invariant && self.box_filter_domains.contains_key(&p) {
                        let scaled_center = self.ir.mul(center, inv);
                        let offset = self.ir.sub(center, scaled_center);
                        self.remap_box_filter_domain(p, out, inv, offset);
                    }
                    out
                }
                Xform::Centered { mode } => {
                    // centered(...) anchors content around the canonical center.
                    // Preserve/Fit keep authored bounds visible; Fill intentionally
                    // crops to fill the viewport.
                    let eps = self.ir.lit(1.0e-6);
                    let one = self.ir.lit(1.0);

                    let res_x = self.ir.x_of(self.ir.res);
                    let res_y = self.ir.y_of(self.ir.res);
                    let safe_res_y = self.ir.m2(Mf::Max, res_y, eps);
                    let screen_aspect = self.ir.div(res_x, safe_res_y);
                    self.ir.name(
                        screen_aspect,
                        &format!("space_centered_screen_ratio_t{index}"),
                    );
                    let safe_screen_aspect = self.ir.m2(Mf::Max, screen_aspect, eps);

                    let x_ratio = screen_aspect;
                    let y_ratio = self.ir.div(one, safe_screen_aspect);
                    self.ir
                        .name(x_ratio, &format!("space_centered_x_ratio_t{index}"));
                    self.ir
                        .name(y_ratio, &format!("space_centered_y_ratio_t{index}"));
                    let (inv_x, inv_y) = match mode {
                        CenteredMode::Fill => {
                            let ix = self.ir.m2(Mf::Min, x_ratio, one);
                            let iy = self.ir.m2(Mf::Min, y_ratio, one);
                            (ix, iy)
                        }
                        CenteredMode::Preserve | CenteredMode::Fit => {
                            let ix = self.ir.m2(Mf::Max, x_ratio, one);
                            let iy = self.ir.m2(Mf::Max, y_ratio, one);
                            (ix, iy)
                        }
                    };
                    let inv = self.ir.vec2(inv_x, inv_y);
                    self.ir.name(inv, &format!("space_centered_inv_t{index}"));

                    let half = self.ir.lit(0.5);
                    let center = self.ir.vec2(half, half);
                    let q = self.ir.sub(p, center);
                    let qs = self.ir.mul(q, inv);
                    let out = self.ir.addx(center, qs);
                    if self.box_filter_domains.contains_key(&p) {
                        let scaled_center = self.ir.mul(center, inv);
                        let offset = self.ir.sub(center, scaled_center);
                        self.remap_box_filter_domain(p, out, inv, offset);
                    }
                    out
                }
                Xform::Orientation { y } => match y {
                    VerticalAxis::Up => p,
                    VerticalAxis::Down => {
                        // Convert from canonical Fresco y-up coordinates to
                        // top-left style y-down authored coordinates.
                        let x = self.ir.x_of(p);
                        let py = self.ir.y_of(p);
                        let one = self.ir.lit(1.0);
                        let y_down = self.ir.sub(one, py);
                        self.ir
                            .name(y_down, &format!("space_orientation_y_t{index}"));
                        let out = self.ir.vec2(x, y_down);
                        if self.box_filter_domains.contains_key(&p) {
                            let minus_one = self.ir.lit(-1.0);
                            let zero = self.ir.lit(0.0);
                            let scale = self.ir.vec2(one, minus_one);
                            let offset = self.ir.vec2(zero, one);
                            self.remap_box_filter_domain(p, out, scale, offset);
                        }
                        out
                    }
                },
                Xform::Polar {
                    center,
                    from,
                    clockwise,
                } => {
                    // Inverse of polar warp: cartesian sample -> straight-space
                    // coordinates where x is turn fraction [0,1) and y is radius.
                    let center = self.v2x_at(center, p);
                    self.ir
                        .name(center, &format!("space_polar_center_t{index}"));
                    let q = self.ir.sub(p, center);
                    self.ir.name(q, &format!("space_polar_q_t{index}"));

                    let qx = self.ir.x_of(q);
                    let qy = self.ir.y_of(q);
                    let theta = self.ir.m2(Mf::Atan2, qy, qx);
                    self.ir.name(theta, &format!("space_polar_theta_t{index}"));

                    let from = self.sx_at(from, p);
                    self.ir.name(from, &format!("space_polar_from_t{index}"));
                    let tau = self.ir.lit(std::f32::consts::TAU);
                    let turns = if *clockwise {
                        let phase = self.ir.sub(theta, from);
                        self.ir.div(phase, tau)
                    } else {
                        let phase = self.ir.sub(from, theta);
                        self.ir.div(phase, tau)
                    };
                    let x = self.ir.m1(Mf::Fract, turns);
                    self.ir.name(x, &format!("space_polar_x_t{index}"));

                    let y = self.ir.m1(Mf::Length, q);
                    self.ir.name(y, &format!("space_polar_y_t{index}"));
                    let out = self.ir.vec2(x, y);
                    self.polar_box_filter_domain(p, out, q, xf);
                    out
                }
                Xform::Warp { by } => {
                    let by = self.v2x_at(by, p);
                    self.ir.name(by, &format!("space_warp_by_t{index}"));
                    self.ir.sub(p, by)
                }
            };
        }
        self.ir.name(p, "p_space");
        p
    }
}
