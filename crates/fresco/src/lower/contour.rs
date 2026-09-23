use super::*;
use crate::hir::CellLayout;

#[derive(Clone, Copy)]
pub(super) struct ContourResult {
    values: [Handle<Ex>; 6],
    edges: Option<Handle<Ex>>,
    lengths: Option<Handle<Ex>>,
    count: u32,
}

impl<'h> FnCtx<'h> {
    fn contour_select(
        &mut self,
        condition: Handle<Ex>,
        accept: Handle<Ex>,
        reject: Handle<Ex>,
    ) -> Handle<Ex> {
        self.ir.add(Ex::Select {
            condition,
            accept,
            reject,
        })
    }

    fn contour_slot(&mut self, array: Handle<Ex>, index: Handle<Ex>) -> Handle<Ex> {
        let index = self.ir.add(Ex::As {
            expr: index,
            kind: naga::ScalarKind::Uint,
            convert: Some(4),
        });
        self.ir.add(Ex::Access { base: array, index })
    }

    // Inverse of the continuous incoming Jacobian: normalized cell vector -> pixels.
    fn contour_pixels(&mut self, v: Handle<Ex>, geometry: &CellGeometryCtx) -> Handle<Ex> {
        let a = self.ir.x_of(geometry.dx);
        let b = self.ir.x_of(geometry.dy);
        let c = self.ir.y_of(geometry.dx);
        let d = self.ir.y_of(geometry.dy);
        let ad = self.ir.mul(a, d);
        let bc = self.ir.mul(b, c);
        let det = self.ir.sub(ad, bc);
        let x = self.ir.x_of(v);
        let y = self.ir.y_of(v);
        let dx = self.ir.mul(d, x);
        let by = self.ir.mul(b, y);
        let ay = self.ir.mul(a, y);
        let cx = self.ir.mul(c, x);
        let vx = self.ir.sub(dx, by);
        let vy = self.ir.sub(ay, cx);
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let valid = self.ir.bin(Bo::NotEqual, det, zero);
        let safe = self.contour_select(valid, det, one);
        let vx = self.ir.div(vx, safe);
        let vy = self.ir.div(vy, safe);
        self.ir.vec2(vx, vy)
    }

    pub(super) fn cell_contour_query(
        &mut self,
        scope_id: u32,
        inset: &Sx,
        at: Option<&Sx>,
        channel: u8,
        p: Handle<Ex>,
    ) -> Handle<Ex> {
        let geometry = self
            .repeat_cell_ctx(scope_id)
            .geometry
            .expect("checked contour");
        let mut pixel_inset = false;
        inset.walk_preorder(&mut |node| pixel_inset |= matches!(node, Sx::PxLit(_)));
        let regular = !pixel_inset
            && matches!(
                geometry.cells.layout,
                CellLayout::Square | CellLayout::Brick | CellLayout::Hex
            );
        if regular && channel == 2 {
            let inset = self.sx_at(inset, p);
            let zero = self.ir.lit(0.0);
            let half = self.ir.lit(0.5);
            let inset = self.ir.m2(Mf::Max, inset, zero);
            let radius = self.ir.sub(half, inset);
            let radius = self.ir.m2(Mf::Max, radius, zero);
            let n = if geometry.cells.layout == CellLayout::Hex {
                6.0
            } else {
                4.0
            };
            let factor = self.ir.lit(2.0 * n * (std::f32::consts::PI / n).tan());
            return self.ir.mul(radius, factor);
        }
        let key = (scope_id, inset.clone(), p);
        let contour = if let Some(result) = self
            .contour_cache
            .get(&key)
            .filter(|r| at.is_none() || r.edges.is_some())
        {
            *result
        } else {
            let result = if regular && at.is_none() {
                self.build_regular_contour(scope_id, inset, p)
            } else {
                self.build_cell_contour(scope_id, inset, p)
            };
            self.contour_cache.insert(key, result);
            result
        };
        if channel < 6 {
            return contour.values[usize::from(channel)];
        }
        let at = self.sx_at(at.expect("point query has a parameter"), p);
        let at = self.ir.m1(Mf::Fract, at);
        let target = self.ir.mul(at, contour.values[2]);
        let out = self.ir.local("contour_point", self.ir.types.v2, None);
        let zero = self.ir.lit(0.0);
        let origin = self.ir.vec2(zero, zero);
        self.ir.store(out, origin);
        let i = self
            .ir
            .local("contour_point_edge", self.ir.types.f32_, None);
        self.ir.store(i, zero);
        self.ir.begin_block();
        let index = self.ir.load(i);
        let slot = self.contour_slot(contour.lengths.expect("point query retains lengths"), index);
        let lengths = self.ir.load(slot);
        let len = self.ir.x_of(lengths);
        let angle = self.ir.z_of(lengths);
        // The candidate edge's start is the sum of all preceding edge lengths.
        let sum = self
            .ir
            .local("contour_point_start", self.ir.types.f32_, None);
        self.ir.store(sum, zero);
        let j = self
            .ir
            .local("contour_point_previous", self.ir.types.f32_, None);
        self.ir.store(j, zero);
        self.ir.begin_block();
        let other = self.ir.load(j);
        let slot = self.contour_slot(contour.lengths.expect("point query retains lengths"), other);
        let values = self.ir.load(slot);
        let other_angle = self.ir.z_of(values);
        let before = self.ir.bin(Bo::Less, other_angle, angle);
        let length = self.ir.x_of(values);
        let length = self.contour_select(before, length, zero);
        let previous = self.ir.load(sum);
        let next = self.ir.addx(previous, length);
        self.ir.store(sum, next);
        let body = self.ir.end_block();
        self.finish_cellular_loop(j, contour.count, body);
        let start = self.ir.load(sum);
        let end = self.ir.addx(start, len);
        let after = self.ir.bin(Bo::GreaterEqual, target, start);
        let before = self.ir.bin(Bo::Less, target, end);
        let choose = self.ir.bin(Bo::LogicalAnd, after, before);
        let slot = self.contour_slot(contour.edges.expect("point query retains edges"), index);
        let edge = self.ir.load(slot);
        let a = self.ir.xy_of(edge);
        let bx = self.ir.z_of(edge);
        let by = self.ir.w_of(edge);
        let b = self.ir.vec2(bx, by);
        let delta = self.ir.sub(b, a);
        let offset = self.ir.sub(target, start);
        let safe = self.clamp_positive(len);
        let fraction = self.ir.div(offset, safe);
        let fraction = self.ir.splat2(fraction);
        let offset = self.ir.mul(delta, fraction);
        let point = self.ir.addx(a, offset);
        let previous = self.ir.load(out);
        let point = self.contour_select(choose, point, previous);
        self.ir.store(out, point);
        let body = self.ir.end_block();
        self.finish_cellular_loop(i, contour.count, body);
        let point = self.ir.load(out);
        let ctx = self.repeat_cell_ctx(scope_id);
        let geometry = ctx.geometry.expect("checked cell contour");
        let component = if channel == 6 {
            self.ir.x_of(point)
        } else {
            self.ir.y_of(point)
        };
        let axis = usize::from(channel == 7);
        let scale = self.ir.lit(geometry.cells.every[axis]);
        let offset = self.ir.mul(component, scale);
        let center = if axis == 0 {
            ctx.center_x
        } else {
            ctx.center_y
        };
        self.ir.addx(center, offset)
    }

    /// Regular polygons need one ordered edge walk, not half-plane clipping or arrays.
    fn build_regular_contour(&mut self, scope_id: u32, inset: &Sx, p: Handle<Ex>) -> ContourResult {
        let ctx = self.repeat_cell_ctx(scope_id);
        let geometry = ctx.geometry.expect("checked contour");
        let count = if geometry.cells.layout == CellLayout::Hex {
            6
        } else {
            4
        };
        let inset = self.sx_at(inset, p);
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let half = self.ir.lit(0.5);
        let inset = self.ir.m2(Mf::Max, inset, zero);
        let height = self.ir.sub(half, inset);
        let height = self.ir.m2(Mf::Max, height, zero);
        let cosine = self.ir.lit((std::f32::consts::PI / count as f32).cos());
        let radius = self.ir.div(height, cosine);
        let radius = self.ir.splat2(radius);
        let valid = self.ir.bin(Bo::Greater, height, zero);
        let x = self.ir.sub(ctx.uv_x, half);
        let y = self.ir.sub(ctx.uv_y, half);
        let point = self.ir.vec2(x, y);
        let screen_point = self.contour_pixels(point, &geometry);
        let total = self.ir.local("contour_total", self.ir.types.v2, None);
        let empty = self.ir.vec2(zero, zero);
        self.ir.store(total, empty);
        let nearest = [
            self.ir.local("contour_nearest", self.ir.types.v2, None),
            self.ir
                .local("contour_pixel_nearest", self.ir.types.v2, None),
        ];
        let far = self.ir.lit(1.0e20);
        let initial = self.ir.vec2(far, zero);
        for ptr in nearest {
            self.ir.store(ptr, initial);
        }
        let counter = self.ir.local("contour_edge", self.ir.types.f32_, None);
        self.ir.store(counter, zero);
        self.ir.begin_block();
        let index = self.ir.load(counter);
        let step = self.ir.lit(std::f32::consts::TAU / count as f32);
        let angle = self.ir.mul(index, step);
        let offset = self.ir.lit(std::f32::consts::PI / count as f32);
        let start = self.ir.sub(angle, offset);
        let end = self.ir.addx(angle, offset);
        let ax = self.ir.m1(Mf::Cos, start);
        let ay = self.ir.m1(Mf::Sin, start);
        let bx = self.ir.m1(Mf::Cos, end);
        let by = self.ir.m1(Mf::Sin, end);
        let a = self.ir.vec2(ax, ay);
        let b = self.ir.vec2(bx, by);
        let a = self.ir.mul(a, radius);
        let b = self.ir.mul(b, radius);
        let screen_a = self.contour_pixels(a, &geometry);
        let screen_b = self.contour_pixels(b, &geometry);
        let previous_total = self.ir.load(total);
        let mut lengths = [zero; 2];
        for (metric, (a, b, point)) in [(a, b, point), (screen_a, screen_b, screen_point)]
            .into_iter()
            .enumerate()
        {
            let delta = self.ir.sub(b, a);
            let offset = self.ir.sub(point, a);
            let len2 = self.ir.m2(Mf::Dot, delta, delta);
            let len = self.ir.m1(Mf::Sqrt, len2);
            lengths[metric] = len;
            let safe = self.clamp_positive(len2);
            let along = self.ir.m2(Mf::Dot, offset, delta);
            let t = self.ir.div(along, safe);
            let t = self.ir.m3(Mf::Clamp, t, zero, one);
            let fraction = self.ir.splat2(t);
            let closest = self.ir.mul(delta, fraction);
            let residual = self.ir.sub(offset, closest);
            let distance = self.ir.m2(Mf::Dot, residual, residual);
            let previous = self.ir.load(nearest[metric]);
            let best = self.ir.x_of(previous);
            let closer = self.ir.bin(Bo::Less, distance, best);
            let choose = self.ir.bin(Bo::LogicalAnd, valid, closer);
            let arc = self.ir.mul(t, len);
            let start = if metric == 0 {
                self.ir.x_of(previous_total)
            } else {
                self.ir.y_of(previous_total)
            };
            let arc = self.ir.addx(start, arc);
            let candidate = self.ir.vec2(distance, arc);
            let selected = self.contour_select(choose, candidate, previous);
            self.ir.store(nearest[metric], selected);
        }
        let length = self.ir.vec2(lengths[0], lengths[1]);
        let sum = self.ir.addx(previous_total, length);
        self.ir.store(total, sum);
        let body = self.ir.end_block();
        self.finish_cellular_loop(counter, count, body);
        let total = self.ir.load(total);
        let mut values = [zero; 6];
        for metric in 0..2 {
            let best = self.ir.load(nearest[metric]);
            let distance = self.ir.x_of(best);
            let arc = self.ir.y_of(best);
            let length = if metric == 0 {
                self.ir.x_of(total)
            } else {
                self.ir.y_of(total)
            };
            let safe = self.clamp_positive(length);
            let progress = self.ir.div(arc, safe);
            values[3 * metric] = self.ir.m1(Mf::Sqrt, distance);
            values[3 * metric + 1] = self.ir.m1(Mf::Fract, progress);
            values[3 * metric + 2] = length;
        }
        ContourResult {
            values,
            edges: None,
            lengths: None,
            count,
        }
    }

    fn build_cell_contour(&mut self, scope_id: u32, inset: &Sx, p: Handle<Ex>) -> ContourResult {
        let ctx = self.repeat_cell_ctx(scope_id);
        let geometry = ctx.geometry.expect("checked cell contour");
        let count = match geometry.cells.layout {
            CellLayout::Voronoi => 49,
            CellLayout::Hex => 6,
            _ => 4,
        };
        let array_type = self.ir.types.contour_arrays[match count {
            4 => 0,
            6 => 1,
            49 => 2,
            _ => unreachable!("cell contour capacity"),
        }];
        let planes = self.ir.local("contour_planes", array_type, None);
        let edges = self.ir.local("contour_edges", array_type, None);
        let lengths = self.ir.local("contour_lengths", array_type, None);
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let half = self.ir.lit(0.5);
        let tau = self.ir.lit(std::f32::consts::TAU);
        let px = self.ir.sub(ctx.uv_x, half);
        let py = self.ir.sub(ctx.uv_y, half);
        let point = self.ir.vec2(px, py);
        let pixel_point = self.contour_pixels(point, &geometry);
        let i = self.ir.local("contour_face", self.ir.types.f32_, None);
        self.ir.store(i, zero);
        self.ir.begin_block();
        let index = self.ir.load(i);
        let (normal, height) = if geometry.cells.layout == CellLayout::Voronoi {
            let seven = self.ir.lit(7.0);
            let three = self.ir.lit(3.0);
            let row = self.ir.div(index, seven);
            let row = self.ir.m1(Mf::Floor, row);
            let col = self.ir.bin(Bo::Modulo, index, seven);
            let row = self.ir.sub(row, three);
            let col = self.ir.sub(col, three);
            let x = self.ir.addx(ctx.id_x, col);
            let y = self.ir.addx(ctx.id_y, row);
            let site = self.cell_site([x, y], &geometry.cells);
            let x = self.ir.sub(site[0], geometry.site[0]);
            let y = self.ir.sub(site[1], geometry.site[1]);
            let delta = self.ir.vec2(x, y);
            let len = self.ir.m1(Mf::Length, delta);
            let valid = self.ir.bin(Bo::Greater, len, zero);
            let safe = self.contour_select(valid, len, one);
            let safe = self.ir.splat2(safe);
            let normal = self.ir.div(delta, safe);
            let height = self.ir.mul(len, half);
            (normal, height)
        } else {
            let step = self.ir.lit(std::f32::consts::TAU / count as f32);
            let angle = self.ir.mul(index, step);
            let nx = self.ir.m1(Mf::Cos, angle);
            let ny = self.ir.m1(Mf::Sin, angle);
            let normal = self.ir.vec2(nx, ny);
            let mut height = half;
            if geometry.cells.layout == CellLayout::Jittered {
                let x = self.ir.addx(ctx.id_x, half);
                let y = self.ir.addx(ctx.id_y, half);
                let x = self.ir.sub(x, geometry.site[0]);
                let y = self.ir.sub(y, geometry.site[1]);
                let delta = self.ir.vec2(x, y);
                let offset = self.ir.m2(Mf::Dot, normal, delta);
                height = self.ir.addx(height, offset);
            }
            (normal, height)
        };
        let nx = self.ir.x_of(normal);
        let ny = self.ir.y_of(normal);
        let angle = self.ir.m2(Mf::Atan2, ny, nx);
        let negative = self.ir.bin(Bo::Less, angle, zero);
        let wrapped = self.ir.addx(angle, tau);
        let angle = self.contour_select(negative, wrapped, angle);
        let dx = self.ir.m2(Mf::Dot, normal, geometry.dx);
        let dy = self.ir.m2(Mf::Dot, normal, geometry.dy);
        let gradient = self.ir.vec2(dx, dy);
        let span = self.ir.m1(Mf::Length, gradient);
        let previous = self.cell_inset_pixel_span.replace(span);
        let amount = self.sx_at(inset, p);
        self.cell_inset_pixel_span = previous;
        let amount = self.ir.m2(Mf::Max, amount, zero);
        let height = self.ir.sub(height, amount);
        // The owner and coincident sites do not define clipping planes.
        let norm = self.ir.m2(Mf::Dot, normal, normal);
        let valid = self.ir.bin(Bo::Greater, norm, half);
        let height = self.contour_select(valid, height, one);
        let plane = self.ir.vec4(nx, ny, height, angle);
        let slot = self.contour_slot(planes, index);
        self.ir.store(slot, plane);
        let body = self.ir.end_block();
        self.finish_cellular_loop(i, count, body);
        // No expression defined inside the face loop may escape through CSE.
        let contour_cache = std::mem::take(&mut self.contour_cache);
        self.clear_point_sensitive_caches();
        self.contour_cache = contour_cache;
        self.user_call_cache.clear();
        self.path_sample_cache.clear();
        self.path_param_sample_cache.clear();
        self.texture_sample_cache.clear();
        self.gradient_dither3_cache = None;

        let total = self.ir.local("contour_total", self.ir.types.v2, None);
        let empty = self.ir.vec2(zero, zero);
        self.ir.store(total, empty);
        let nearest = [
            self.ir.local("contour_nearest", self.ir.types.v4, None),
            self.ir
                .local("contour_pixel_nearest", self.ir.types.v4, None),
        ];
        let far = self.ir.lit(1.0e20);
        let initial = self.ir.vec4(far, zero, zero, zero);
        for ptr in nearest {
            self.ir.store(ptr, initial);
        }
        self.ir.store(i, zero);
        self.ir.begin_block();
        let index = self.ir.load(i);
        let slot = self.contour_slot(planes, index);
        let plane = self.ir.load(slot);
        let normal = self.ir.xy_of(plane);
        let nx = self.ir.x_of(normal);
        let ny = self.ir.y_of(normal);
        let minus_y = self.ir.neg(ny);
        let tangent = self.ir.vec2(minus_y, nx);
        let height = self.ir.z_of(plane);
        let angle = self.ir.w_of(plane);
        let height2 = self.ir.splat2(height);
        let origin = self.ir.mul(normal, height2);
        let bounds = self.ir.local("contour_interval", self.ir.types.v2, None);
        let lo = self.ir.lit(-4.0);
        let hi = self.ir.lit(4.0);
        let interval = self.ir.vec2(lo, hi);
        self.ir.store(bounds, interval);
        let feasible = self.ir.local("contour_feasible", self.ir.types.f32_, None);
        let norm = self.ir.m2(Mf::Dot, normal, normal);
        self.ir.store(feasible, norm);
        let j = self.ir.local("contour_clip", self.ir.types.f32_, None);
        self.ir.store(j, zero);
        self.ir.begin_block();
        let other = self.ir.load(j);
        let slot = self.contour_slot(planes, other);
        let plane2 = self.ir.load(slot);
        let n2 = self.ir.xy_of(plane2);
        let h2 = self.ir.z_of(plane2);
        let denom = self.ir.m2(Mf::Dot, n2, tangent);
        let projection = self.ir.m2(Mf::Dot, n2, origin);
        let rhs = self.ir.sub(h2, projection);
        let epsilon = self.ir.lit(1.0e-6);
        let abs = self.ir.m1(Mf::Abs, denom);
        let crossing = self.ir.bin(Bo::Greater, abs, epsilon);
        let safe = self.contour_select(crossing, denom, one);
        let t = self.ir.div(rhs, safe);
        let previous = self.ir.load(bounds);
        let lo = self.ir.x_of(previous);
        let hi = self.ir.y_of(previous);
        let negative = self.ir.bin(Bo::Less, denom, zero);
        let positive = self.ir.bin(Bo::Greater, denom, zero);
        let lower = self.ir.bin(Bo::LogicalAnd, crossing, negative);
        let upper = self.ir.bin(Bo::LogicalAnd, crossing, positive);
        let clipped_lo = self.ir.m2(Mf::Max, lo, t);
        let clipped_hi = self.ir.m2(Mf::Min, hi, t);
        let lo = self.contour_select(lower, clipped_lo, lo);
        let hi = self.contour_select(upper, clipped_hi, hi);
        let interval = self.ir.vec2(lo, hi);
        self.ir.store(bounds, interval);
        let neg_epsilon = self.ir.neg(epsilon);
        let outside = self.ir.bin(Bo::Less, rhs, neg_epsilon);
        let parallel = self.ir.bin(Bo::LessEqual, abs, epsilon);
        let invalid = self.ir.bin(Bo::LogicalAnd, parallel, outside);
        let previous = self.ir.load(feasible);
        let valid = self.contour_select(invalid, zero, previous);
        self.ir.store(feasible, valid);
        let body = self.ir.end_block();
        self.finish_cellular_loop(j, count, body);
        let interval = self.ir.load(bounds);
        let lo = self.ir.x_of(interval);
        let hi = self.ir.y_of(interval);
        let nonempty = self.ir.bin(Bo::Greater, hi, lo);
        let feasible = self.ir.load(feasible);
        let valid = self.ir.bin(Bo::Greater, feasible, half);
        let valid = self.ir.bin(Bo::LogicalAnd, valid, nonempty);
        let lo = self.ir.splat2(lo);
        let hi = self.ir.splat2(hi);
        let a = self.ir.mul(tangent, lo);
        let b = self.ir.mul(tangent, hi);
        let a = self.ir.addx(origin, a);
        let b = self.ir.addx(origin, b);
        let ax = self.ir.x_of(a);
        let ay = self.ir.y_of(a);
        let bx = self.ir.x_of(b);
        let by = self.ir.y_of(b);
        let edge = self.ir.vec4(ax, ay, bx, by);
        let slot = self.contour_slot(edges, index);
        self.ir.store(slot, edge);
        let pixel_a = self.contour_pixels(a, &geometry);
        let pixel_b = self.contour_pixels(b, &geometry);
        let mut lens = [zero; 2];
        for (metric, (a, b, point)) in [(a, b, point), (pixel_a, pixel_b, pixel_point)]
            .into_iter()
            .enumerate()
        {
            let delta = self.ir.sub(b, a);
            let offset = self.ir.sub(point, a);
            let len2 = self.ir.m2(Mf::Dot, delta, delta);
            let len = self.ir.m1(Mf::Sqrt, len2);
            lens[metric] = self.contour_select(valid, len, zero);
            let safe = self.clamp_positive(len2);
            let along = self.ir.m2(Mf::Dot, offset, delta);
            let t = self.ir.div(along, safe);
            let t = self.ir.m3(Mf::Clamp, t, zero, one);
            let fraction = self.ir.splat2(t);
            let closest = self.ir.mul(delta, fraction);
            let residual = self.ir.sub(offset, closest);
            let distance = self.ir.m2(Mf::Dot, residual, residual);
            let previous = self.ir.load(nearest[metric]);
            let best = self.ir.x_of(previous);
            let closer = self.ir.bin(Bo::Less, distance, best);
            let choose = self.ir.bin(Bo::LogicalAnd, valid, closer);
            let arc = self.ir.mul(t, len);
            let candidate = self.ir.vec4(distance, angle, arc, zero);
            let selected = self.contour_select(choose, candidate, previous);
            self.ir.store(nearest[metric], selected);
        }
        let values = self.ir.vec4(lens[0], lens[1], angle, zero);
        let slot = self.contour_slot(lengths, index);
        self.ir.store(slot, values);
        let len = self.ir.vec2(lens[0], lens[1]);
        let previous = self.ir.load(total);
        let sum = self.ir.addx(previous, len);
        self.ir.store(total, sum);
        let body = self.ir.end_block();
        self.finish_cellular_loop(i, count, body);
        let best = [self.ir.load(nearest[0]), self.ir.load(nearest[1])];
        let arc = self.ir.local("contour_arc", self.ir.types.v2, None);
        let a = self.ir.z_of(best[0]);
        let b = self.ir.z_of(best[1]);
        let initial = self.ir.vec2(a, b);
        self.ir.store(arc, initial);
        self.ir.store(i, zero);
        self.ir.begin_block();
        let index = self.ir.load(i);
        let slot = self.contour_slot(lengths, index);
        let lengths_value = self.ir.load(slot);
        let angle = self.ir.z_of(lengths_value);
        let mut additions = [zero; 2];
        for metric in 0..2 {
            let best_angle = self.ir.y_of(best[metric]);
            let before = self.ir.bin(Bo::Less, angle, best_angle);
            let len = if metric == 0 {
                self.ir.x_of(lengths_value)
            } else {
                self.ir.y_of(lengths_value)
            };
            additions[metric] = self.contour_select(before, len, zero);
        }
        let addition = self.ir.vec2(additions[0], additions[1]);
        let previous = self.ir.load(arc);
        let next = self.ir.addx(previous, addition);
        self.ir.store(arc, next);
        let body = self.ir.end_block();
        self.finish_cellular_loop(i, count, body);
        let total = self.ir.load(total);
        let arc = self.ir.load(arc);
        let mut values = [zero; 6];
        for metric in 0..2 {
            let len = if metric == 0 {
                self.ir.x_of(total)
            } else {
                self.ir.y_of(total)
            };
            let arc = if metric == 0 {
                self.ir.x_of(arc)
            } else {
                self.ir.y_of(arc)
            };
            let safe = self.clamp_positive(len);
            let progress = self.ir.div(arc, safe);
            let progress = self.ir.m1(Mf::Fract, progress);
            let distance2 = self.ir.x_of(best[metric]);
            values[metric * 3] = self.ir.m1(Mf::Sqrt, distance2);
            values[metric * 3 + 1] = progress;
            values[metric * 3 + 2] = len;
        }
        ContourResult {
            values,
            edges: Some(edges),
            lengths: Some(lengths),
            count,
        }
    }
}
