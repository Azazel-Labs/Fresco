use super::*;
use crate::hir::{CellLayout, CellularSpace};

struct CellPlaneQuery<'a> {
    point: [Handle<Ex>; 2],
    ray: Option<Handle<Ex>>,
    inset: &'a Sx,
    p: Handle<Ex>,
}

impl<'h> FnCtx<'h> {
    fn cell_plane_distance(
        &mut self,
        geometry: &CellGeometryCtx,
        query: &CellPlaneQuery<'_>,
        normal: [Handle<Ex>; 2],
        height: Handle<Ex>,
    ) -> Handle<Ex> {
        let n = self.ir.vec2(normal[0], normal[1]);
        if let Some(ray) = query.ray {
            let dot = self.ir.m2(Mf::Dot, n, ray);
            let zero = self.ir.lit(0.0);
            let one = self.ir.lit(1.0);
            let positive = self.ir.bin(Bo::Greater, dot, zero);
            let safe = self.ir.add(Ex::Select {
                condition: positive,
                accept: dot,
                reject: one,
            });
            let distance = self.ir.div(height, safe);
            let bound = self.ir.lit(2.0);
            return self.ir.add(Ex::Select {
                condition: positive,
                accept: distance,
                reject: bound,
            });
        }
        let point = self.ir.vec2(query.point[0], query.point[1]);
        let projection = self.ir.m2(Mf::Dot, n, point);
        let distance = self.ir.sub(height, projection);
        if matches!(query.inset, Sx::Lit(0.0)) {
            return distance;
        }
        // Project the continuous incoming coordinate Jacobian onto this face's
        // normal. Never differentiate an owner ID or wrapped cell coordinate.
        let dx = self.ir.m2(Mf::Dot, n, geometry.dx);
        let dy = self.ir.m2(Mf::Dot, n, geometry.dy);
        let gradient = self.ir.vec2(dx, dy);
        let pixel_span = self.ir.m1(Mf::Length, gradient);
        let previous = self.cell_inset_pixel_span.replace(pixel_span);
        let inset = self.sx_at(query.inset, query.p);
        self.cell_inset_pixel_span = previous;
        self.ir.sub(distance, inset)
    }

    pub(super) fn cell_geometry_query(
        &mut self,
        scope_id: u32,
        angle: Option<&Sx>,
        inset: &Sx,
        p: Handle<Ex>,
    ) -> Handle<Ex> {
        let ctx = self.repeat_cell_ctx(scope_id);
        let geometry = ctx.geometry.expect("cell geometry checked before lowering");
        let half = self.ir.lit(0.5);
        let point = [self.ir.sub(ctx.uv_x, half), self.ir.sub(ctx.uv_y, half)];
        let ray = angle.map(|angle| {
            let angle = self.sx_at(angle, p);
            let x = self.ir.m1(Mf::Cos, angle);
            let y = self.ir.m1(Mf::Sin, angle);
            self.ir.vec2(x, y)
        });
        let query = CellPlaneQuery {
            point,
            ray,
            inset,
            p,
        };
        let mut pixel_units = false;
        inset.walk_preorder(&mut |node| pixel_units |= matches!(node, Sx::PxLit(_)));
        if ray.is_none()
            && !pixel_units
            && matches!(
                geometry.cells.layout,
                CellLayout::Square | CellLayout::Brick | CellLayout::Hex
            )
        {
            let x = self.ir.m1(Mf::Abs, point[0]);
            let y = self.ir.m1(Mf::Abs, point[1]);
            let extent = if geometry.cells.layout == CellLayout::Hex {
                let hx = self.ir.mul(x, half);
                let slope = self.ir.lit(0.866_025_4);
                let hy = self.ir.mul(y, slope);
                let diagonal = self.ir.addx(hx, hy);
                self.ir.m2(Mf::Max, x, diagonal)
            } else {
                self.ir.m2(Mf::Max, x, y)
            };
            let edge = self.ir.sub(half, extent);
            let amount = self.sx_at(inset, p);
            return self.ir.sub(edge, amount);
        }
        let mut nearest = self.ir.lit(2.0);
        if geometry.cells.layout == CellLayout::Voronoi {
            // Every point has a site within sqrt(2). A face neighbor is at most
            // 2*sqrt(2) from the owner. Generating cells >=4 indices apart have
            // site separation >=3, so the owner's 7x7 window contains all faces.
            let result = self.ir.local("cell_boundary", self.ir.types.f32_, None);
            let counter = self.ir.local("cell_face", self.ir.types.f32_, None);
            let zero = self.ir.lit(0.0);
            self.ir.store(result, nearest);
            self.ir.store(counter, zero);
            self.ir.begin_block();
            let index = self.ir.load(counter);
            let seven = self.ir.lit(7.0);
            let three = self.ir.lit(3.0);
            let row = self.ir.div(index, seven);
            let row = self.ir.m1(Mf::Floor, row);
            let column = self.ir.bin(Bo::Modulo, index, seven);
            let row = self.ir.sub(row, three);
            let column = self.ir.sub(column, three);
            let x = self.ir.addx(ctx.id_x, column);
            let y = self.ir.addx(ctx.id_y, row);
            let site = self.cell_site([x, y], &geometry.cells);
            let x = self.ir.sub(site[0], geometry.site[0]);
            let y = self.ir.sub(site[1], geometry.site[1]);
            let delta = self.ir.vec2(x, y);
            let length = self.ir.m1(Mf::Length, delta);
            let one = self.ir.lit(1.0);
            let valid = self.ir.bin(Bo::Greater, length, zero);
            let safe = self.ir.add(Ex::Select {
                condition: valid,
                accept: length,
                reject: one,
            });
            let normal = [self.ir.div(x, safe), self.ir.div(y, safe)];
            let height = self.ir.mul(length, half);
            let distance = self.cell_plane_distance(&geometry, &query, normal, height);
            let previous = self.ir.load(result);
            let minimum = self.ir.m2(Mf::Min, previous, distance);
            let first = self.ir.bin(Bo::Equal, index, zero);
            let distance = self.ir.add(Ex::Select {
                condition: first,
                accept: distance,
                reject: minimum,
            });
            let distance = self.ir.add(Ex::Select {
                condition: valid,
                accept: distance,
                reject: previous,
            });
            self.ir.store(result, distance);
            let body = self.ir.end_block();
            self.finish_cellular_loop(counter, 49, body);
            // Expressions emitted within this loop cannot escape via a cache.
            self.clear_point_sensitive_caches();
            self.user_call_cache.clear();
            self.path_sample_cache.clear();
            self.path_param_sample_cache.clear();
            self.gradient_dither3_cache = None;
            self.texture_sample_cache.clear();
            return self.ir.load(result);
        }
        let normals: &[[f32; 2]] = if geometry.cells.layout == CellLayout::Hex {
            &[
                [1.0, 0.0],
                [-1.0, 0.0],
                [0.5, 0.866_025_4],
                [-0.5, 0.866_025_4],
                [0.5, -0.866_025_4],
                [-0.5, -0.866_025_4],
            ]
        } else {
            &[[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]]
        };
        for (face, &[nx, ny]) in normals.iter().enumerate() {
            let mut height = half;
            // Jitter moves the site, not the square ownership boundary.
            if geometry.cells.layout == CellLayout::Jittered {
                let axis = usize::from(ny != 0.0);
                let id = if axis == 0 { ctx.id_x } else { ctx.id_y };
                let center = self.ir.addx(id, half);
                let offset = self.ir.sub(center, geometry.site[axis]);
                let sign = self.ir.lit(nx + ny);
                let offset = self.ir.mul(offset, sign);
                height = self.ir.addx(height, offset);
            }
            let normal = [self.ir.lit(nx), self.ir.lit(ny)];
            let distance = self.cell_plane_distance(&geometry, &query, normal, height);
            nearest = if face == 0 {
                distance
            } else {
                self.ir.m2(Mf::Min, nearest, distance)
            };
        }
        nearest
    }

    /// Integer mixing keeps cell identity/randomness independent of floating
    /// transcendental implementations. Signed lattice IDs are bitcast to u32.
    pub(super) fn cell_random(&mut self, x: Handle<Ex>, y: Handle<Ex>, seed: u32) -> Handle<Ex> {
        let x = self.ir.add(Ex::As {
            expr: x,
            kind: naga::ScalarKind::Sint,
            convert: Some(4),
        });
        let x = self.ir.add(Ex::As {
            expr: x,
            kind: naga::ScalarKind::Uint,
            convert: None,
        });
        let y = self.ir.add(Ex::As {
            expr: y,
            kind: naga::ScalarKind::Sint,
            convert: Some(4),
        });
        let y = self.ir.add(Ex::As {
            expr: y,
            kind: naga::ScalarKind::Uint,
            convert: None,
        });
        let k = self.ir.lit_u32(0x9e37_79b9);
        let y = self.ir.mul(y, k);
        let h = self.ir.bin(Bo::ExclusiveOr, x, y);
        let seed = self.ir.lit_u32(seed);
        let mut h = self.ir.bin(Bo::ExclusiveOr, h, seed);
        for (shift, multiplier) in [(16, 0x7feb_352d), (15, 0x846c_a68b)] {
            let shift = self.ir.lit_u32(shift);
            let shifted = self.ir.bin(Bo::ShiftRight, h, shift);
            h = self.ir.bin(Bo::ExclusiveOr, h, shifted);
            let multiplier = self.ir.lit_u32(multiplier);
            h = self.ir.mul(h, multiplier);
        }
        let shift = self.ir.lit_u32(16);
        let shifted = self.ir.bin(Bo::ShiftRight, h, shift);
        h = self.ir.bin(Bo::ExclusiveOr, h, shifted);
        let shift = self.ir.lit_u32(8);
        let bits = self.ir.bin(Bo::ShiftRight, h, shift);
        let f = self.ir.add(Ex::As {
            expr: bits,
            kind: naga::ScalarKind::Float,
            convert: Some(4),
        });
        let scale = self.ir.lit(1.0 / 16_777_216.0);
        self.ir.mul(f, scale)
    }

    pub(super) fn cell_site(
        &mut self,
        id: [Handle<Ex>; 2],
        cells: &CellularSpace,
    ) -> [Handle<Ex>; 2] {
        let half = self.ir.lit(0.5);
        let mut x = self.ir.addx(id[0], half);
        let mut y = self.ir.addx(id[1], half);
        if matches!(cells.layout, CellLayout::Brick | CellLayout::Hex) {
            let rows = self.ir.mul(id[1], half);
            let offset = self.ir.m1(Mf::Fract, rows);
            x = self.ir.addx(x, offset);
        }
        if cells.layout == CellLayout::Hex {
            let pitch = self.ir.lit(3.0_f32.sqrt() * 0.5);
            y = self.ir.mul(y, pitch);
        }
        if matches!(cells.layout, CellLayout::Jittered | CellLayout::Voronoi) {
            let rx = self.cell_random(id[0], id[1], cells.seed);
            let ry = self.cell_random(id[0], id[1], cells.seed ^ 0xa511_e9b3);
            let dx = self.ir.sub(rx, half);
            let dy = self.ir.sub(ry, half);
            let jitter = self.ir.lit(cells.jitter);
            let dx = self.ir.mul(dx, jitter);
            let dy = self.ir.mul(dy, jitter);
            x = self.ir.addx(x, dx);
            y = self.ir.addx(y, dy);
        }
        [x, y]
    }

    pub(super) fn enter_cellular(&mut self, p: Handle<Ex>, cells: &CellularSpace) -> Handle<Ex> {
        let period_x = self.ir.lit(cells.every[0]);
        let period_y = self.ir.lit(cells.every[1]);
        let px = self.ir.x_of(p);
        let py = self.ir.y_of(p);
        let qx = self.ir.div(px, period_x);
        let qy = self.ir.div(py, period_y);
        let normalized = self.ir.vec2(qx, qy);
        let geometry_dx = self.ir.dpdx(normalized);
        let geometry_dy = self.ir.dpdy(normalized);
        let row = if cells.layout == CellLayout::Hex {
            let pitch = self.ir.lit(3.0_f32.sqrt() * 0.5);
            self.ir.div(qy, pitch)
        } else {
            qy
        };
        let iy = self.ir.m1(Mf::Floor, row);
        let column = if cells.layout == CellLayout::Brick {
            let half = self.ir.lit(0.5);
            let parity = self.ir.mul(iy, half);
            let offset = self.ir.m1(Mf::Fract, parity);
            self.ir.sub(qx, offset)
        } else {
            qx
        };
        let ix = self.ir.m1(Mf::Floor, column);
        let mut id = [ix, iy];
        let mut site = self.cell_site(id, cells);
        if matches!(cells.layout, CellLayout::Hex | CellLayout::Voronoi) {
            // Sites lie in their generating unit cells (jitter <= 1). The own
            // square's site is <= sqrt(2) away; any excluded square is >= 2
            // away. A 5x5 search therefore contains the true nearest site.
            // The staggered hex lattice is covered by the same bound. Strict
            // comparisons and row-major traversal give stable ties.
            let zero = self.ir.lit(0.0);
            let index_ptr = self.ir.local("cell_candidate", self.ir.types.f32_, None);
            let best_ptr = self.ir.local("cell_distance", self.ir.types.f32_, None);
            let id_ptr = self.ir.local("cell_owner", self.ir.types.v2, None);
            let site_ptr = self.ir.local("cell_site", self.ir.types.v2, None);
            // Explicit stores reset the search on every coverage sample, including
            // when this block is itself inside another generated loop.
            self.ir.store(index_ptr, zero);
            self.ir.store(best_ptr, zero);
            self.ir.begin_block();
            let index = self.ir.load(index_ptr);
            let five = self.ir.lit(5.0);
            let two = self.ir.lit(2.0);
            let row = self.ir.div(index, five);
            let row = self.ir.m1(Mf::Floor, row);
            let column = self.ir.bin(Bo::Modulo, index, five);
            let dx = self.ir.sub(column, two);
            let dy = self.ir.sub(row, two);
            let cx = self.ir.addx(ix, dx);
            let cy = self.ir.addx(iy, dy);
            let candidate = self.cell_site([cx, cy], cells);
            let x = self.ir.sub(qx, candidate[0]);
            let y = self.ir.sub(qy, candidate[1]);
            let xx = self.ir.mul(x, x);
            let yy = self.ir.mul(y, y);
            let distance = self.ir.addx(xx, yy);
            let previous = self.ir.load(best_ptr);
            let closer = self.ir.bin(Bo::Less, distance, previous);
            let first = self.ir.bin(Bo::Equal, index, zero);
            let choose = self.ir.bin(Bo::LogicalOr, first, closer);
            let best = self.ir.add(Ex::Select {
                condition: choose,
                accept: distance,
                reject: previous,
            });
            self.ir.store(best_ptr, best);
            let candidate_id = self.ir.vec2(cx, cy);
            let candidate_site = self.ir.vec2(candidate[0], candidate[1]);
            for (pointer, value) in [(id_ptr, candidate_id), (site_ptr, candidate_site)] {
                let previous = self.ir.load(pointer);
                let selected = self.ir.add(Ex::Select {
                    condition: choose,
                    accept: value,
                    reject: previous,
                });
                self.ir.store(pointer, selected);
            }
            let body = self.ir.end_block();
            self.finish_cellular_loop(index_ptr, 25, body);
            let owner = self.ir.load(id_ptr);
            let nearest = self.ir.load(site_ptr);
            id = [self.ir.x_of(owner), self.ir.y_of(owner)];
            site = [self.ir.x_of(nearest), self.ir.y_of(nearest)];
        }
        let half = self.ir.lit(0.5);
        let ux = self.ir.sub(qx, site[0]);
        let uy = self.ir.sub(qy, site[1]);
        let ux = self.ir.addx(ux, half);
        let uy = self.ir.addx(uy, half);
        let x = self.ir.mul(ux, period_x);
        let y = self.ir.mul(uy, period_y);
        if let Some(scope_id) = cells.cell_scope {
            let center_x = self.ir.mul(half, period_x);
            let center_y = self.ir.mul(half, period_y);
            let rand = self.cell_random(id[0], id[1], cells.seed);
            self.repeat_cell_ctx.push(RepeatCellCtx {
                geometry: Some(CellGeometryCtx {
                    cells: *cells,
                    site,
                    dx: geometry_dx,
                    dy: geometry_dy,
                }),
                scope_id,
                id_x: id[0],
                id_y: id[1],
                center_x,
                center_y,
                uv_x: ux,
                uv_y: uy,
                rand,
            });
        }
        let result = self.ir.vec2(x, y);
        self.ir.name(result, "cellular_local");
        result
    }

    /// Emit a fixed-trip loop with a uniform counter and row-major traversal.
    pub(super) fn finish_cellular_loop(
        &mut self,
        index_ptr: Handle<Ex>,
        count: u32,
        body: naga::Block,
    ) {
        self.ir.begin_block();
        let index = self.ir.load(index_ptr);
        let one = self.ir.lit(1.0);
        let next = self.ir.addx(index, one);
        self.ir.store(index_ptr, next);
        let limit = self.ir.lit(count as f32);
        let done = self.ir.bin(Bo::GreaterEqual, next, limit);
        let continuing = self.ir.end_block();
        self.ir.push_statement(naga::Statement::Loop {
            body,
            continuing,
            break_if: Some(done),
        });
    }

    /// Integrate owner selection AND content, so discontinuous cell IDs do
    /// not feed hardware derivatives across a seam. RGB is alpha-weighted.
    /// Nested cells share this outermost quadrature rather than multiplying it.
    pub(super) fn cellular_layer(
        &mut self,
        id: LayerId,
        p: Handle<Ex>,
        axis: u32,
    ) -> (Handle<Ex>, Handle<Ex>) {
        let dx = self.ir.dpdx(p);
        let dy = self.ir.dpdy(p);
        let zero = self.ir.lit(0.0);
        let rgb_ptr = self.ir.local("cell_rgb", self.ir.types.v3, None);
        let alpha_ptr = self.ir.local("cell_alpha", self.ir.types.f32_, None);
        let index_ptr = self.ir.local("cell_sample", self.ir.types.f32_, None);
        let black = self.ir.splat3(zero);
        self.ir.store(rgb_ptr, black);
        self.ir.store(alpha_ptr, zero);
        self.ir.store(index_ptr, zero);
        self.cellular_sample_depth += 1;
        self.clear_point_sensitive_caches();
        self.invariant_sx_cache.clear();
        self.user_call_cache.clear();
        self.path_sample_cache.clear();
        self.path_param_sample_cache.clear();
        self.gradient_dither3_cache = None;
        self.texture_sample_cache.clear();
        self.ir.begin_block();
        let index = self.ir.load(index_ptr);
        let width = self.ir.lit(axis as f32);
        let row = self.ir.div(index, width);
        let row = self.ir.m1(Mf::Floor, row);
        let column = self.ir.bin(Bo::Modulo, index, width);
        let half = self.ir.lit(0.5);
        let x = self.ir.addx(column, half);
        let y = self.ir.addx(row, half);
        let x = self.ir.div(x, width);
        let y = self.ir.div(y, width);
        let ox = self.ir.sub(x, half);
        let oy = self.ir.sub(y, half);
        let ox = self.ir.splat2(ox);
        let oy = self.ir.splat2(oy);
        let x = self.ir.mul(dx, ox);
        let y = self.ir.mul(dy, oy);
        let offset = self.ir.addx(x, y);
        let sample = self.ir.addx(p, offset);
        let (c, a) = self.layer_color(id, sample);
        let weight = self.ir.splat3(a);
        let c = self.ir.mul(c, weight);
        let rgb = self.ir.load(rgb_ptr);
        let alpha = self.ir.load(alpha_ptr);
        let rgb = self.ir.addx(rgb, c);
        let alpha = self.ir.addx(alpha, a);
        self.ir.store(rgb_ptr, rgb);
        self.ir.store(alpha_ptr, alpha);
        let body = self.ir.end_block();
        self.finish_cellular_loop(index_ptr, axis * axis, body);
        self.cellular_sample_depth -= 1;
        // Expressions emitted in the loop must never escape through a cache.
        self.clear_point_sensitive_caches();
        self.invariant_sx_cache.clear();
        self.user_call_cache.clear();
        self.path_sample_cache.clear();
        self.path_param_sample_cache.clear();
        self.gradient_dither3_cache = None;
        self.texture_sample_cache.clear();
        let rgb = self.ir.load(rgb_ptr);
        let alpha = self.ir.load(alpha_ptr);
        let safe_alpha = self.clamp_positive(alpha);
        let denominator = self.ir.splat3(safe_alpha);
        let rgb = self.ir.div(rgb, denominator);
        let weight = self.ir.lit(1.0 / (axis * axis) as f32);
        let alpha = self.ir.mul(alpha, weight);
        self.ir.name(alpha, "cellular_filtered_alpha");
        (rgb, alpha)
    }
}
