use super::*;
use std::collections::HashSet;

pub(super) fn lower_path_nearest_fn(
    canvas_name: &str,
    path_id: usize,
    table: &PathTableBinding,
    t: &TypeHandles,
    policy: LoweringPolicy,
) -> naga::Function {
    let mut function = FunctionBuilder::new(
        format!("fresco_path_sample_{canvas_name}_p{path_id}"),
        policy,
    );
    let p = function.arg("p", t.v2);
    function.set_result(t.v4);

    let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
    let zero_vec2 = function.expr(Ex::Compose {
        ty: t.v2,
        components: vec![zero, zero],
    });

    let mut ir = IrBuilder {
        function,
        types: *t,
        uv: p,
        time: zero,
        time_override: None,
        delta: zero,
        res: zero_vec2,
        px: zero,
        aa: zero,
        jacobian_j11: None,
        jacobian_j12: None,
        jacobian_j21: None,
        jacobian_j22: None,
        param_scalars: HashMap::new(),
        param_scalar_ptrs: HashSet::new(),
        tex_globals: HashMap::new(),
        sampler_global: None,
        param_storage_globals: HashMap::new(),
        global_uniform_globals: HashMap::new(),
    };
    let px = ir.x_of(p);
    let py = ir.y_of(p);
    let eps = ir.lit(1.0e-6);
    let huge = ir.lit(1.0e9);
    let one = ir.lit(1.0);
    let best_dist_ptr = ir.local("path_best_dist", t.f32_, Some(huge));
    let best_along_ptr = if table.demand.along {
        Some(ir.local("path_best_along", t.f32_, Some(zero)))
    } else {
        None
    };
    let best_tx_ptr = if table.demand.tangent {
        Some(ir.local("path_best_tx", t.f32_, Some(one)))
    } else {
        None
    };
    let best_ty_ptr = if table.demand.tangent {
        Some(ir.local("path_best_ty", t.f32_, Some(zero)))
    } else {
        None
    };
    let zero_u32 = ir.lit_u32(0);
    let idx_ptr = ir.local("path_idx", t.u32_, Some(zero_u32));
    let seg_ptr = ir.local("path_seg", t.path_seg, None);

    ir.begin_block();
    let idx = ir.load(idx_ptr);
    match table.storage {
        PathTableStorage::Buffer { global } => {
            let table_ptr = ir.add(Ex::GlobalVariable(global));
            let seg_ref = ir.add(Ex::Access {
                base: table_ptr,
                index: idx,
            });
            let seg_val = ir.load(seg_ref);
            ir.store(seg_ptr, seg_val);
        }
        PathTableStorage::Const { constant } => {
            let const_array = ir.add(Ex::Constant(constant));
            let seg_value = ir.add(Ex::Access {
                base: const_array,
                index: idx,
            });
            ir.store(seg_ptr, seg_value);
        }
    }
    let seg = ir.load(seg_ptr);
    let p0 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 0,
    });
    let p1 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 1,
    });
    let p2 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 2,
    });
    let p3 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 3,
    });
    let s0 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 4,
    });
    let seg_len = ir.add(Ex::AccessIndex {
        base: seg,
        index: 5,
    });
    let kind = ir.add(Ex::AccessIndex {
        base: seg,
        index: 6,
    });
    let mid_u = ir.add(Ex::AccessIndex {
        base: seg,
        index: 7,
    });

    let one_u32 = ir.lit_u32(1);
    let is_cubic = ir.bin(Bo::Equal, kind, one_u32);

    let p0x = ir.x_of(p0);
    let p0y = ir.y_of(p0);
    let p1x = ir.x_of(p1);
    let p1y = ir.y_of(p1);
    let p2x = ir.x_of(p2);
    let p2y = ir.y_of(p2);
    let p3x = ir.x_of(p3);
    let p3y = ir.y_of(p3);

    let two = ir.lit(2.0);
    let three = ir.lit(3.0);
    let six = ir.lit(6.0);

    // Quadratic coefficients for q(t) = a3*t^3 + a2*t^2 + a1*t + a0.
    let q_a3x = zero;
    let q_a3y = zero;
    let p1_twice_x = ir.mul(two, p1x);
    let p1_twice_y = ir.mul(two, p1y);
    let q_a2x_l = ir.sub(p2x, p1_twice_x);
    let q_a2y_l = ir.sub(p2y, p1_twice_y);
    let q_a2x = ir.addx(q_a2x_l, p0x);
    let q_a2y = ir.addx(q_a2y_l, p0y);
    let q_d1x = ir.sub(p1x, p0x);
    let q_d1y = ir.sub(p1y, p0y);
    let q_a1x = ir.mul(two, q_d1x);
    let q_a1y = ir.mul(two, q_d1y);
    let q_a0x = p0x;
    let q_a0y = p0y;

    // Cubic coefficients for q(t) = a3*t^3 + a2*t^2 + a1*t + a0.
    let c_p1_3x = ir.mul(three, p1x);
    let c_p2_3x = ir.mul(three, p2x);
    let c_a3_l = ir.sub(c_p1_3x, c_p2_3x);
    let c_a3_m = ir.addx(c_a3_l, p3x);
    let c_a3x = ir.sub(c_a3_m, p0x);
    let c_p1_3y = ir.mul(three, p1y);
    let c_p2_3y = ir.mul(three, p2y);
    let c_a3_ly = ir.sub(c_p1_3y, c_p2_3y);
    let c_a3_my = ir.addx(c_a3_ly, p3y);
    let c_a3y = ir.sub(c_a3_my, p0y);

    let c_p0_3x = ir.mul(three, p0x);
    let c_p1_6x = ir.mul(six, p1x);
    let c_a2_lx = ir.sub(c_p0_3x, c_p1_6x);
    let c_p2_3x_for_a2 = ir.mul(three, p2x);
    let c_a2x = ir.addx(c_a2_lx, c_p2_3x_for_a2);
    let c_p0_3y = ir.mul(three, p0y);
    let c_p1_6y = ir.mul(six, p1y);
    let c_a2_ly = ir.sub(c_p0_3y, c_p1_6y);
    let c_p2_3y_for_a2 = ir.mul(three, p2y);
    let c_a2y = ir.addx(c_a2_ly, c_p2_3y_for_a2);

    let c_p0_3nx = ir.mul(three, p0x);
    let c_a1_lx = ir.sub(c_p1_3x, c_p0_3nx);
    let c_a1x = c_a1_lx;
    let c_p0_3ny = ir.mul(three, p0y);
    let c_a1_ly = ir.sub(c_p1_3y, c_p0_3ny);
    let c_a1y = c_a1_ly;
    let c_a0x = p0x;
    let c_a0y = p0y;

    let a3x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a3x,
        reject: q_a3x,
    });
    let a3y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a3y,
        reject: q_a3y,
    });
    let a2x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a2x,
        reject: q_a2x,
    });
    let a2y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a2y,
        reject: q_a2y,
    });
    let a1x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a1x,
        reject: q_a1x,
    });
    let a1y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a1y,
        reject: q_a1y,
    });
    let a0x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a0x,
        reject: q_a0x,
    });
    let a0y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a0y,
        reject: q_a0y,
    });

    let end_x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: p3x,
        reject: p2x,
    });
    let end_y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: p3y,
        reject: p2y,
    });

    // Seed t from endpoint chord projection, then apply Newton updates on
    // g(t)=dot(q(t)-p, q'(t)) with g'(t)=dot(q',q')+dot(q-p,q'').
    let chord_x = ir.sub(end_x, p0x);
    let chord_y = ir.sub(end_y, p0y);
    let chord_len2_x = ir.mul(chord_x, chord_x);
    let chord_len2_y = ir.mul(chord_y, chord_y);
    let chord_len2_raw = ir.addx(chord_len2_x, chord_len2_y);
    let chord_len2 = ir.m2(Mf::Max, chord_len2_raw, eps);
    let wx = ir.sub(px, p0x);
    let wy = ir.sub(py, p0y);
    let proj_x = ir.mul(wx, chord_x);
    let proj_y = ir.mul(wy, chord_y);
    let proj = ir.addx(proj_x, proj_y);
    let t_seed_raw = ir.div(proj, chord_len2);
    let t_seed = ir.m3(Mf::Clamp, t_seed_raw, zero, one);
    let t_ptr = ir.local("path_t", t.f32_, None);
    ir.store(t_ptr, t_seed);

    for _ in 0..3 {
        let tt = ir.load(t_ptr);
        let tt2 = ir.mul(tt, tt);
        let tt3 = ir.mul(tt2, tt);

        let a3x_t3 = ir.mul(a3x, tt3);
        let a2x_t2 = ir.mul(a2x, tt2);
        let a1x_t = ir.mul(a1x, tt);
        let qx_01 = ir.addx(a3x_t3, a2x_t2);
        let qx_02 = ir.addx(qx_01, a1x_t);
        let qx = ir.addx(qx_02, a0x);

        let a3y_t3 = ir.mul(a3y, tt3);
        let a2y_t2 = ir.mul(a2y, tt2);
        let a1y_t = ir.mul(a1y, tt);
        let qy_01 = ir.addx(a3y_t3, a2y_t2);
        let qy_02 = ir.addx(qy_01, a1y_t);
        let qy = ir.addx(qy_02, a0y);

        let a3x_3 = ir.mul(three, a3x);
        let a2x_2 = ir.mul(two, a2x);
        let qdx_0 = ir.mul(a3x_3, tt2);
        let qdx_1 = ir.mul(a2x_2, tt);
        let qdx_2 = ir.addx(qdx_0, qdx_1);
        let qdx = ir.addx(qdx_2, a1x);

        let a3y_3 = ir.mul(three, a3y);
        let a2y_2 = ir.mul(two, a2y);
        let qdy_0 = ir.mul(a3y_3, tt2);
        let qdy_1 = ir.mul(a2y_2, tt);
        let qdy_2 = ir.addx(qdy_0, qdy_1);
        let qdy = ir.addx(qdy_2, a1y);

        let a3x_6 = ir.mul(six, a3x);
        let a3y_6 = ir.mul(six, a3y);
        let qddx_0 = ir.mul(a3x_6, tt);
        let qddy_0 = ir.mul(a3y_6, tt);
        let qddx = ir.addx(qddx_0, a2x_2);
        let qddy = ir.addx(qddy_0, a2y_2);

        let dx = ir.sub(qx, px);
        let dy = ir.sub(qy, py);
        let dx_qdx = ir.mul(dx, qdx);
        let dy_qdy = ir.mul(dy, qdy);
        let g = ir.addx(dx_qdx, dy_qdy);

        let qdx2 = ir.mul(qdx, qdx);
        let qdy2 = ir.mul(qdy, qdy);
        let qd2 = ir.addx(qdx2, qdy2);

        let dx_ddx = ir.mul(dx, qddx);
        let dy_ddy = ir.mul(dy, qddy);
        let d_dot_dd = ir.addx(dx_ddx, dy_ddy);
        let gp_raw = ir.addx(qd2, d_dot_dd);
        let gp_abs = ir.m1(Mf::Abs, gp_raw);
        let gp_safe_mag = ir.m2(Mf::Max, gp_abs, eps);
        let gp_is_neg = ir.bin(Bo::Less, gp_raw, zero);
        let neg_one = ir.lit(-1.0);
        let gp_sign = ir.add(Ex::Select {
            condition: gp_is_neg,
            accept: neg_one,
            reject: one,
        });
        let gp_safe = ir.mul(gp_sign, gp_safe_mag);
        let step = ir.div(g, gp_safe);
        let t_unclamped = ir.sub(tt, step);
        let next_t = ir.m3(Mf::Clamp, t_unclamped, zero, one);
        ir.store(t_ptr, next_t);
    }

    let t_newton = ir.load(t_ptr);
    let t_newton2 = ir.mul(t_newton, t_newton);
    let t_newton3 = ir.mul(t_newton2, t_newton);
    let n_a3x_t3 = ir.mul(a3x, t_newton3);
    let n_a2x_t2 = ir.mul(a2x, t_newton2);
    let n_a1x_t = ir.mul(a1x, t_newton);
    let n_qx0 = ir.addx(n_a3x_t3, n_a2x_t2);
    let n_qx1 = ir.addx(n_qx0, n_a1x_t);
    let qx_newton = ir.addx(n_qx1, a0x);
    let n_a3y_t3 = ir.mul(a3y, t_newton3);
    let n_a2y_t2 = ir.mul(a2y, t_newton2);
    let n_a1y_t = ir.mul(a1y, t_newton);
    let n_qy0 = ir.addx(n_a3y_t3, n_a2y_t2);
    let n_qy1 = ir.addx(n_qy0, n_a1y_t);
    let qy_newton = ir.addx(n_qy1, a0y);
    let dx_newton = ir.sub(px, qx_newton);
    let dy_newton = ir.sub(py, qy_newton);
    let dx_newton2 = ir.mul(dx_newton, dx_newton);
    let dy_newton2 = ir.mul(dy_newton, dy_newton);
    let d2_newton = ir.addx(dx_newton2, dy_newton2);

    let dx_p0 = ir.sub(px, p0x);
    let dy_p0 = ir.sub(py, p0y);
    let dx_p02 = ir.mul(dx_p0, dx_p0);
    let dy_p02 = ir.mul(dy_p0, dy_p0);
    let d2_p0 = ir.addx(dx_p02, dy_p02);
    let dx_p2 = ir.sub(px, end_x);
    let dy_p2 = ir.sub(py, end_y);
    let dx_p22 = ir.mul(dx_p2, dx_p2);
    let dy_p22 = ir.mul(dy_p2, dy_p2);
    let d2_p2 = ir.addx(dx_p22, dy_p22);

    let use_p0 = ir.bin(Bo::Less, d2_p0, d2_newton);
    let best_t_after_p0 = ir.add(Ex::Select {
        condition: use_p0,
        accept: zero,
        reject: t_newton,
    });
    let best_d2_after_p0 = ir.add(Ex::Select {
        condition: use_p0,
        accept: d2_p0,
        reject: d2_newton,
    });
    let use_p2 = ir.bin(Bo::Less, d2_p2, best_d2_after_p0);
    let t_seg = ir.add(Ex::Select {
        condition: use_p2,
        accept: one,
        reject: best_t_after_p0,
    });
    let best_d2 = ir.add(Ex::Select {
        condition: use_p2,
        accept: d2_p2,
        reject: best_d2_after_p0,
    });
    let dist = ir.m1(Mf::Sqrt, best_d2);

    let t2 = ir.mul(t_seg, t_seg);
    let a3x_3 = ir.mul(three, a3x);
    let a2x_2 = ir.mul(two, a2x);
    let qdx0 = ir.mul(a3x_3, t2);
    let qdx1 = ir.mul(a2x_2, t_seg);
    let qdx2s = ir.addx(qdx0, qdx1);
    let qdx = ir.addx(qdx2s, a1x);
    let a3y_3 = ir.mul(three, a3y);
    let a2y_2 = ir.mul(two, a2y);
    let qdy0 = ir.mul(a3y_3, t2);
    let qdy1 = ir.mul(a2y_2, t_seg);
    let qdy2s = ir.addx(qdy0, qdy1);
    let qdy = ir.addx(qdy2s, a1y);
    let qdx2 = ir.mul(qdx, qdx);
    let qdy2 = ir.mul(qdy, qdy);
    let tan_len2_raw = ir.addx(qdx2, qdy2);
    let tan_len2 = ir.m2(Mf::Max, tan_len2_raw, eps);
    let tan_len = ir.m1(Mf::Sqrt, tan_len2);
    let tx = ir.div(qdx, tan_len);
    let ty = ir.div(qdy, tan_len);

    let t3 = ir.mul(t2, t_seg);
    let a3x_t3 = ir.mul(a3x, t3);
    let a2x_t2 = ir.mul(a2x, t2);
    let a1x_t = ir.mul(a1x, t_seg);
    let qx0 = ir.addx(a3x_t3, a2x_t2);
    let qx1 = ir.addx(qx0, a1x_t);
    let qx = ir.addx(qx1, a0x);
    let a3y_t3 = ir.mul(a3y, t3);
    let a2y_t2 = ir.mul(a2y, t2);
    let a1y_t = ir.mul(a1y, t_seg);
    let qy0 = ir.addx(a3y_t3, a2y_t2);
    let qy1 = ir.addx(qy0, a1y_t);
    let qy = ir.addx(qy1, a0y);
    let _dx = ir.sub(px, qx);
    let _dy = ir.sub(py, qy);
    let best_dist = ir.load(best_dist_ptr);
    let better = ir.bin(Bo::Less, dist, best_dist);
    let next_best = ir.add(Ex::Select {
        condition: better,
        accept: dist,
        reject: best_dist,
    });
    ir.store(best_dist_ptr, next_best);

    if let Some(ptr) = best_along_ptr {
        // Reparameterize geometric t -> arc fraction u using midpoint metadata.
        // u(t)=a*t^2+b*t with u(0)=0, u(0.5)=mid_u, u(1)=1.
        let four = ir.lit(4.0);
        let four_mid = ir.mul(four, mid_u);
        let a_reparam = ir.sub(two, four_mid);
        let b_reparam = ir.sub(four_mid, one);
        let t2_reparam = ir.mul(t_seg, t_seg);
        let u_a = ir.mul(a_reparam, t2_reparam);
        let u_b = ir.mul(b_reparam, t_seg);
        let u_seg_raw = ir.addx(u_a, u_b);
        let u_seg = ir.m3(Mf::Clamp, u_seg_raw, zero, one);
        let along_delta = ir.mul(u_seg, seg_len);
        let along = ir.addx(s0, along_delta);
        let cur = ir.load(ptr);
        let next = ir.add(Ex::Select {
            condition: better,
            accept: along,
            reject: cur,
        });
        ir.store(ptr, next);
    }
    if let (Some(tx_ptr), Some(ty_ptr)) = (best_tx_ptr, best_ty_ptr) {
        let cur_tx = ir.load(tx_ptr);
        let cur_ty = ir.load(ty_ptr);
        let next_tx = ir.add(Ex::Select {
            condition: better,
            accept: tx,
            reject: cur_tx,
        });
        let next_ty = ir.add(Ex::Select {
            condition: better,
            accept: ty,
            reject: cur_ty,
        });
        ir.store(tx_ptr, next_tx);
        ir.store(ty_ptr, next_ty);
    }
    let body = ir.end_block();

    ir.begin_block();
    let idx_cur = ir.load(idx_ptr);
    let one_u32 = ir.lit_u32(1);
    let idx_next = ir.addx(idx_cur, one_u32);
    ir.store(idx_ptr, idx_next);
    let continuing = ir.end_block();

    let seg_count = ir.lit_u32(table.seg_count);
    let break_if = ir.bin(Bo::GreaterEqual, idx_next, seg_count);
    ir.push_statement(naga::Statement::Loop {
        body,
        continuing,
        break_if: Some(break_if),
    });

    let out_dist = ir.load(best_dist_ptr);
    let out_along = best_along_ptr.map(|ptr| ir.load(ptr)).unwrap_or(zero);
    let out_tx = best_tx_ptr.map(|ptr| ir.load(ptr)).unwrap_or(one);
    let out_ty = best_ty_ptr.map(|ptr| ir.load(ptr)).unwrap_or(zero);
    let out = ir.add(Ex::Compose {
        ty: t.v4,
        components: vec![out_dist, out_along, out_tx, out_ty],
    });
    ir.return_value(out);
    ir.finish()
}

pub(super) fn lower_path_arc_sample_fn(
    canvas_name: &str,
    path_id: usize,
    table: &PathTableBinding,
    t: &TypeHandles,
    policy: LoweringPolicy,
) -> naga::Function {
    let mut function = FunctionBuilder::new(
        format!("fresco_path_arc_sample_{canvas_name}_p{path_id}"),
        policy,
    );
    let s = function.arg("s", t.f32_);
    function.set_result(t.v4);

    let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
    let zero_vec2 = function.expr(Ex::Compose {
        ty: t.v2,
        components: vec![zero, zero],
    });

    let mut ir = IrBuilder {
        function,
        types: *t,
        uv: zero_vec2,
        time: zero,
        time_override: None,
        delta: zero,
        res: zero_vec2,
        px: zero,
        aa: zero,
        jacobian_j11: None,
        jacobian_j12: None,
        jacobian_j21: None,
        jacobian_j22: None,
        param_scalars: HashMap::new(),
        param_scalar_ptrs: HashSet::new(),
        tex_globals: HashMap::new(),
        sampler_global: None,
        param_storage_globals: HashMap::new(),
        global_uniform_globals: HashMap::new(),
    };

    let eps = ir.lit(1.0e-6);
    let one = ir.lit(1.0);
    let s_nonneg = ir.m2(Mf::Max, s, zero);

    let out_x_ptr = if table.demand.point_at {
        Some(ir.local("path_out_x", t.f32_, Some(zero)))
    } else {
        None
    };
    let out_y_ptr = if table.demand.point_at {
        Some(ir.local("path_out_y", t.f32_, Some(zero)))
    } else {
        None
    };
    let out_tx_ptr = if table.demand.tangent_at {
        Some(ir.local("path_out_tx", t.f32_, Some(zero)))
    } else {
        None
    };
    let out_ty_ptr = if table.demand.tangent_at {
        Some(ir.local("path_out_ty", t.f32_, Some(zero)))
    } else {
        None
    };

    let zero_u32 = ir.lit_u32(0);
    let idx_ptr = ir.local("path_idx", t.u32_, Some(zero_u32));
    let seg_ptr = ir.local("path_seg", t.path_seg, None);

    ir.begin_block();
    let idx = ir.load(idx_ptr);
    match table.storage {
        PathTableStorage::Buffer { global } => {
            let table_ptr = ir.add(Ex::GlobalVariable(global));
            let seg_ref = ir.add(Ex::Access {
                base: table_ptr,
                index: idx,
            });
            let seg_val = ir.load(seg_ref);
            ir.store(seg_ptr, seg_val);
        }
        PathTableStorage::Const { constant } => {
            let const_array = ir.add(Ex::Constant(constant));
            let seg_value = ir.add(Ex::Access {
                base: const_array,
                index: idx,
            });
            ir.store(seg_ptr, seg_value);
        }
    }

    let seg = ir.load(seg_ptr);
    let p0 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 0,
    });
    let p1 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 1,
    });
    let p2 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 2,
    });
    let p3 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 3,
    });
    let s0 = ir.add(Ex::AccessIndex {
        base: seg,
        index: 4,
    });
    let seg_len = ir.add(Ex::AccessIndex {
        base: seg,
        index: 5,
    });
    let kind = ir.add(Ex::AccessIndex {
        base: seg,
        index: 6,
    });
    let mid_u = ir.add(Ex::AccessIndex {
        base: seg,
        index: 7,
    });
    let s1 = ir.addx(s0, seg_len);
    let safe_len = ir.m2(Mf::Max, seg_len, eps);

    let s_delta = ir.sub(s_nonneg, s0);
    let u_raw = ir.div(s_delta, safe_len);
    let u_seg = ir.m3(Mf::Clamp, u_raw, zero, one);

    // Invert the same midpoint-based reparameterization used by nearest.
    // u(t)=a*t^2+b*t, solved with a short Newton refinement from t=u.
    let two_reparam = ir.lit(2.0);
    let four = ir.lit(4.0);
    let four_mid = ir.mul(four, mid_u);
    let a_reparam = ir.sub(two_reparam, four_mid);
    let b_reparam = ir.sub(four_mid, one);
    let t_ptr = ir.local("path_t_reparam", t.f32_, None);
    ir.store(t_ptr, u_seg);
    for _ in 0..2 {
        let tt = ir.load(t_ptr);
        let tt2 = ir.mul(tt, tt);
        let f_a = ir.mul(a_reparam, tt2);
        let f_b = ir.mul(b_reparam, tt);
        let f_sum = ir.addx(f_a, f_b);
        let f = ir.sub(f_sum, u_seg);

        let fp_a = ir.mul(two_reparam, a_reparam);
        let fp_a_t = ir.mul(fp_a, tt);
        let fp = ir.addx(fp_a_t, b_reparam);
        let fp_abs = ir.m1(Mf::Abs, fp);
        let fp_safe_mag = ir.m2(Mf::Max, fp_abs, eps);
        let fp_is_neg = ir.bin(Bo::Less, fp, zero);
        let neg_one = ir.lit(-1.0);
        let fp_sign = ir.add(Ex::Select {
            condition: fp_is_neg,
            accept: neg_one,
            reject: one,
        });
        let fp_safe = ir.mul(fp_sign, fp_safe_mag);
        let step = ir.div(f, fp_safe);
        let t_next_unclamped = ir.sub(tt, step);
        let t_next = ir.m3(Mf::Clamp, t_next_unclamped, zero, one);
        ir.store(t_ptr, t_next);
    }
    let t_seg = ir.load(t_ptr);

    let gate_lo = ir.m2(Mf::Step, s0, s_nonneg);
    let gate_hi = ir.m2(Mf::Step, s1, s_nonneg);
    let one_minus_hi = ir.sub(one, gate_hi);
    let gate_nonlast = ir.mul(gate_lo, one_minus_hi);
    let last_idx = ir.lit_u32(table.seg_count.saturating_sub(1));
    let is_last = ir.bin(Bo::Equal, idx, last_idx);
    let gate = ir.add(Ex::Select {
        condition: is_last,
        accept: gate_lo,
        reject: gate_nonlast,
    });

    let one_u32 = ir.lit_u32(1);
    let is_cubic = ir.bin(Bo::Equal, kind, one_u32);

    let p0x = ir.x_of(p0);
    let p0y = ir.y_of(p0);
    let p1x = ir.x_of(p1);
    let p1y = ir.y_of(p1);
    let p2x = ir.x_of(p2);
    let p2y = ir.y_of(p2);
    let p3x = ir.x_of(p3);
    let p3y = ir.y_of(p3);

    let two = ir.lit(2.0);
    let three = ir.lit(3.0);
    let six = ir.lit(6.0);
    let t2 = ir.mul(t_seg, t_seg);
    let t3 = ir.mul(t2, t_seg);

    // Quadratic coefficients
    let q_a3x = zero;
    let q_a3y = zero;
    let p1_twice_x = ir.mul(two, p1x);
    let p1_twice_y = ir.mul(two, p1y);
    let q_a2x_l = ir.sub(p2x, p1_twice_x);
    let q_a2y_l = ir.sub(p2y, p1_twice_y);
    let q_a2x = ir.addx(q_a2x_l, p0x);
    let q_a2y = ir.addx(q_a2y_l, p0y);
    let q_d1x = ir.sub(p1x, p0x);
    let q_d1y = ir.sub(p1y, p0y);
    let q_a1x = ir.mul(two, q_d1x);
    let q_a1y = ir.mul(two, q_d1y);
    let q_a0x = p0x;
    let q_a0y = p0y;

    // Cubic coefficients
    let c_p1_3x = ir.mul(three, p1x);
    let c_p2_3x = ir.mul(three, p2x);
    let c_a3_lx = ir.sub(c_p1_3x, c_p2_3x);
    let c_a3_mx = ir.addx(c_a3_lx, p3x);
    let c_a3x = ir.sub(c_a3_mx, p0x);
    let c_p1_3y = ir.mul(three, p1y);
    let c_p2_3y = ir.mul(three, p2y);
    let c_a3_ly = ir.sub(c_p1_3y, c_p2_3y);
    let c_a3_my = ir.addx(c_a3_ly, p3y);
    let c_a3y = ir.sub(c_a3_my, p0y);

    let c_p0_3x = ir.mul(three, p0x);
    let c_p1_6x = ir.mul(six, p1x);
    let c_a2_lx = ir.sub(c_p0_3x, c_p1_6x);
    let c_p2_3x_for_a2 = ir.mul(three, p2x);
    let c_a2x = ir.addx(c_a2_lx, c_p2_3x_for_a2);
    let c_p0_3y = ir.mul(three, p0y);
    let c_p1_6y = ir.mul(six, p1y);
    let c_a2_ly = ir.sub(c_p0_3y, c_p1_6y);
    let c_p2_3y_for_a2 = ir.mul(three, p2y);
    let c_a2y = ir.addx(c_a2_ly, c_p2_3y_for_a2);

    let c_p0_3nx = ir.mul(three, p0x);
    let c_a1x = ir.sub(c_p1_3x, c_p0_3nx);
    let c_p0_3ny = ir.mul(three, p0y);
    let c_a1y = ir.sub(c_p1_3y, c_p0_3ny);
    let c_a0x = p0x;
    let c_a0y = p0y;

    let a3x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a3x,
        reject: q_a3x,
    });
    let a3y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a3y,
        reject: q_a3y,
    });
    let a2x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a2x,
        reject: q_a2x,
    });
    let a2y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a2y,
        reject: q_a2y,
    });
    let a1x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a1x,
        reject: q_a1x,
    });
    let a1y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a1y,
        reject: q_a1y,
    });
    let a0x = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a0x,
        reject: q_a0x,
    });
    let a0y = ir.add(Ex::Select {
        condition: is_cubic,
        accept: c_a0y,
        reject: q_a0y,
    });

    let a3x_t3 = ir.mul(a3x, t3);
    let a2x_t2 = ir.mul(a2x, t2);
    let a1x_t = ir.mul(a1x, t_seg);
    let cand_x_0 = ir.addx(a3x_t3, a2x_t2);
    let cand_x_1 = ir.addx(cand_x_0, a1x_t);
    let cand_x = ir.addx(cand_x_1, a0x);

    let a3y_t3 = ir.mul(a3y, t3);
    let a2y_t2 = ir.mul(a2y, t2);
    let a1y_t = ir.mul(a1y, t_seg);
    let cand_y_0 = ir.addx(a3y_t3, a2y_t2);
    let cand_y_1 = ir.addx(cand_y_0, a1y_t);
    let cand_y = ir.addx(cand_y_1, a0y);

    let a3x_3 = ir.mul(three, a3x);
    let a2x_2 = ir.mul(two, a2x);
    let deriv_x_0 = ir.mul(a3x_3, t2);
    let deriv_x_1 = ir.mul(a2x_2, t_seg);
    let deriv_x_2 = ir.addx(deriv_x_0, deriv_x_1);
    let deriv_x = ir.addx(deriv_x_2, a1x);
    let a3y_3 = ir.mul(three, a3y);
    let a2y_2 = ir.mul(two, a2y);
    let deriv_y_0 = ir.mul(a3y_3, t2);
    let deriv_y_1 = ir.mul(a2y_2, t_seg);
    let deriv_y_2 = ir.addx(deriv_y_0, deriv_y_1);
    let deriv_y = ir.addx(deriv_y_2, a1y);

    let deriv_x2 = ir.mul(deriv_x, deriv_x);
    let deriv_y2 = ir.mul(deriv_y, deriv_y);
    let deriv_len2_raw = ir.addx(deriv_x2, deriv_y2);
    let deriv_len2 = ir.m2(Mf::Max, deriv_len2_raw, eps);
    let deriv_len = ir.m1(Mf::Sqrt, deriv_len2);
    let tx = ir.div(deriv_x, deriv_len);
    let ty = ir.div(deriv_y, deriv_len);

    if let (Some(x_ptr), Some(y_ptr)) = (out_x_ptr, out_y_ptr) {
        let cur_x = ir.load(x_ptr);
        let cur_y = ir.load(y_ptr);
        let weighted_x = ir.mul(cand_x, gate);
        let weighted_y = ir.mul(cand_y, gate);
        let next_x = ir.addx(cur_x, weighted_x);
        let next_y = ir.addx(cur_y, weighted_y);
        ir.store(x_ptr, next_x);
        ir.store(y_ptr, next_y);
    }

    if let (Some(tx_ptr), Some(ty_ptr)) = (out_tx_ptr, out_ty_ptr) {
        let cur_tx = ir.load(tx_ptr);
        let cur_ty = ir.load(ty_ptr);
        let weighted_tx = ir.mul(tx, gate);
        let weighted_ty = ir.mul(ty, gate);
        let next_tx = ir.addx(cur_tx, weighted_tx);
        let next_ty = ir.addx(cur_ty, weighted_ty);
        ir.store(tx_ptr, next_tx);
        ir.store(ty_ptr, next_ty);
    }

    let body = ir.end_block();

    ir.begin_block();
    let idx_cur = ir.load(idx_ptr);
    let one_u32 = ir.lit_u32(1);
    let idx_next = ir.addx(idx_cur, one_u32);
    ir.store(idx_ptr, idx_next);
    let continuing = ir.end_block();

    let seg_count = ir.lit_u32(table.seg_count);
    let break_if = ir.bin(Bo::GreaterEqual, idx_next, seg_count);
    ir.push_statement(naga::Statement::Loop {
        body,
        continuing,
        break_if: Some(break_if),
    });

    let out_x = out_x_ptr.map(|ptr| ir.load(ptr)).unwrap_or(zero);
    let out_y = out_y_ptr.map(|ptr| ir.load(ptr)).unwrap_or(zero);
    let out_tx = out_tx_ptr.map(|ptr| ir.load(ptr)).unwrap_or(one);
    let out_ty = out_ty_ptr.map(|ptr| ir.load(ptr)).unwrap_or(zero);
    let out = ir.add(Ex::Compose {
        ty: t.v4,
        components: vec![out_x, out_y, out_tx, out_ty],
    });
    ir.return_value(out);
    ir.finish()
}

struct UserHelperRuntimeCtx<'a, 'h> {
    cx: &'a mut FnCtx<'h>,
    helper: &'a UserFnHelper,
    scopes: Vec<HashMap<String, Vec<String>>>,
    vec_ptr_scopes: Vec<HashMap<String, Handle<Ex>>>,
    mutated_names: HashSet<String>,
}

impl<'a, 'h> UserHelperRuntimeCtx<'a, 'h> {
    fn param_base_from_slot(slot: &str) -> Option<String> {
        let stripped = slot.strip_prefix("arg__")?;
        Some(stripped.split("__").next()?.to_string())
    }

    fn collect_mutated_names(stmts: &[crate::hir::UserFnStmt], out: &mut HashSet<String>) {
        for stmt in stmts {
            match stmt {
                crate::hir::UserFnStmt::Assign { name, .. } => {
                    out.insert(name.clone());
                }
                crate::hir::UserFnStmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    Self::collect_mutated_names(then_body, out);
                    Self::collect_mutated_names(else_body, out);
                }
                crate::hir::UserFnStmt::For { body, .. } => {
                    Self::collect_mutated_names(body, out);
                }
                crate::hir::UserFnStmt::Let { .. }
                | crate::hir::UserFnStmt::Expr { .. }
                | crate::hir::UserFnStmt::Return { .. }
                | crate::hir::UserFnStmt::Break => {}
            }
        }
    }

    fn new(cx: &'a mut FnCtx<'h>, helper: &'a UserFnHelper) -> Self {
        let mut this = Self {
            cx,
            helper,
            scopes: vec![HashMap::new()],
            vec_ptr_scopes: vec![HashMap::new()],
            mutated_names: HashSet::new(),
        };

        // Parameters that are assigned in helper bodies must be mirrored into
        // mutable local slots; read-only parameters stay as function args.
        Self::collect_mutated_names(&helper.body_stmts, &mut this.mutated_names);

        let mut vec_param_ptrs: HashMap<String, Handle<Ex>> = HashMap::new();
        for param in &helper.params {
            let is_mutated = this.mutated_names.contains(&param.name);
            if !is_mutated {
                continue;
            }

            let vec_ty = match param.ty {
                crate::hir::UserFnParamTy::Vec2 => {
                    Some(vector_type_handle(param.scalar_kind, 2, &this.cx.ir.types))
                }
                crate::hir::UserFnParamTy::Vec3 => {
                    Some(vector_type_handle(param.scalar_kind, 3, &this.cx.ir.types))
                }
                crate::hir::UserFnParamTy::Vec4 => {
                    Some(vector_type_handle(param.scalar_kind, 4, &this.cx.ir.types))
                }
                _ => None,
            };
            let Some(vec_ty) = vec_ty else {
                continue;
            };

            let Some(first_slot) = param.scalar_slots.first() else {
                continue;
            };
            let Some(&first_handle) = this.cx.ir.param_scalars.get(first_slot) else {
                continue;
            };
            let Some((vec_arg, _)) = this.cx.ir.access_index_of(first_handle) else {
                continue;
            };

            let vec_ptr = this
                .cx
                .ir
                .local(&format!("arg_{}_var", param.name), vec_ty, None);
            this.cx.ir.store(vec_ptr, vec_arg);

            for slot in &param.scalar_slots {
                let Some(&slot_handle) = this.cx.ir.param_scalars.get(slot) else {
                    continue;
                };
                let Some((_, index)) = this.cx.ir.access_index_of(slot_handle) else {
                    continue;
                };
                let comp_ptr = this.cx.ir.add(Ex::AccessIndex {
                    base: vec_ptr,
                    index,
                });
                this.cx.ir.bind_scalar_slot_ptr(slot.clone(), comp_ptr);
            }

            vec_param_ptrs.insert(param.name.clone(), vec_ptr);
        }

        let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
        for slot in &helper.param_scalars {
            let base = Self::param_base_from_slot(slot);
            if let Some(base_name) = base.as_ref() {
                let base = base_name.clone();
                grouped.entry(base).or_default().push(slot.clone());
            }

            let needs_mirror = base
                .as_ref()
                .is_some_and(|base_name| this.mutated_names.contains(base_name));
            if needs_mirror
                && !base
                    .as_ref()
                    .is_some_and(|name| vec_param_ptrs.contains_key(name))
            {
                let init = *this
                    .cx
                    .ir
                    .param_scalars
                    .get(slot)
                    .expect("checker emitted unknown helper parameter slot");
                let ptr_name = format!("{slot}_var");
                let kind = helper
                    .params
                    .iter()
                    .find(|param| param.scalar_slots.contains(slot))
                    .map(|param| param.scalar_kind)
                    .expect("helper parameter slot has a declared type");
                let ty = scalar_type_handle(kind, &this.cx.ir.types);
                let ptr = this.cx.ir.local(&ptr_name, ty, None);
                this.cx.ir.bind_scalar_slot_ptr(slot.clone(), ptr);
                this.cx.ir.store(ptr, init);
            }
        }

        for (name, slots) in grouped {
            let vec_ptr = vec_param_ptrs.get(&name).copied();
            this.bind_slots(name, slots, vec_ptr);
        }

        this
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.vec_ptr_scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
        self.vec_ptr_scopes.pop();
    }

    fn bind_slots(&mut self, name: String, slots: Vec<String>, vec_ptr: Option<Handle<Ex>>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.clone(), slots);
        }
        if let Some(vec_ptr) = vec_ptr
            && let Some(scope) = self.vec_ptr_scopes.last_mut()
        {
            scope.insert(name, vec_ptr);
        }
    }

    fn lookup_slots(&self, name: &str) -> Option<Vec<String>> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }

    fn lookup_vec_ptr(&self, name: &str) -> Option<Handle<Ex>> {
        self.vec_ptr_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn vec_components(&mut self, base: Handle<Ex>, width: usize) -> Vec<Handle<Ex>> {
        (0..width)
            .map(|index| {
                self.cx.ir.add(Ex::AccessIndex {
                    base,
                    index: index as u32,
                })
            })
            .collect()
    }

    fn compose_vec2_from_sx(&mut self, v: &(Sx, Sx), p: Handle<Ex>) -> Handle<Ex> {
        let (x, y) = v;
        let xh = self.cx.native_scalar(x, p);
        let yh = self.cx.native_scalar(y, p);
        self.cx.ir.add(Ex::Compose {
            ty: vector_type_handle(x.scalar_kind(), 2, &self.cx.ir.types),
            components: vec![xh, yh],
        })
    }

    fn compose_vec3_from_sx(&mut self, v: &(Sx, Sx, Sx), p: Handle<Ex>) -> Handle<Ex> {
        let (x, y, z) = v;
        let xh = self.cx.native_scalar(x, p);
        let yh = self.cx.native_scalar(y, p);
        let zh = self.cx.native_scalar(z, p);
        self.cx.ir.add(Ex::Compose {
            ty: vector_type_handle(x.scalar_kind(), 3, &self.cx.ir.types),
            components: vec![xh, yh, zh],
        })
    }

    fn compose_vec4_from_sx(&mut self, v: &(Sx, Sx, Sx, Sx), p: Handle<Ex>) -> Handle<Ex> {
        let (x, y, z, w) = v;
        let xh = self.cx.native_scalar(x, p);
        let yh = self.cx.native_scalar(y, p);
        let zh = self.cx.native_scalar(z, p);
        let wh = self.cx.native_scalar(w, p);
        self.cx.ir.add(Ex::Compose {
            ty: vector_type_handle(x.scalar_kind(), 4, &self.cx.ir.types),
            components: vec![xh, yh, zh, wh],
        })
    }

    fn expression_kind(value: &crate::hir::UserFnExpr) -> crate::typed_scalar::Kind {
        use crate::hir::{UserFnExpr, UserFnValue};
        match value {
            UserFnExpr::Value(
                UserFnValue::Scalar(s)
                | UserFnValue::Vec2((s, _))
                | UserFnValue::Vec3((s, _, _))
                | UserFnValue::Vec4((s, _, _, _)),
            ) => s.scalar_kind(),
            _ => crate::typed_scalar::Kind::F32,
        }
    }

    fn value_components(&mut self, value: &crate::hir::UserFnExpr) -> Vec<Handle<Ex>> {
        let p = self.cx.ir.uv;
        match value {
            crate::hir::UserFnExpr::Value(crate::hir::UserFnValue::Scalar(s)) => {
                vec![self.cx.native_scalar(s, p)]
            }
            crate::hir::UserFnExpr::Value(crate::hir::UserFnValue::Vec2((x, y))) => {
                vec![self.cx.native_scalar(x, p), self.cx.native_scalar(y, p)]
            }
            crate::hir::UserFnExpr::Value(crate::hir::UserFnValue::Vec3((x, y, z))) => {
                vec![
                    self.cx.native_scalar(x, p),
                    self.cx.native_scalar(y, p),
                    self.cx.native_scalar(z, p),
                ]
            }
            crate::hir::UserFnExpr::Value(crate::hir::UserFnValue::Vec4((x, y, z, w))) => {
                vec![
                    self.cx.native_scalar(x, p),
                    self.cx.native_scalar(y, p),
                    self.cx.native_scalar(z, p),
                    self.cx.native_scalar(w, p),
                ]
            }
            crate::hir::UserFnExpr::SlotSwizzle2(slots) => {
                vec![
                    self.cx.sx_at(&Sx::Param(slots[0].clone()), p),
                    self.cx.sx_at(&Sx::Param(slots[1].clone()), p),
                ]
            }
            crate::hir::UserFnExpr::SlotSwizzle3(slots) => {
                vec![
                    self.cx.sx_at(&Sx::Param(slots[0].clone()), p),
                    self.cx.sx_at(&Sx::Param(slots[1].clone()), p),
                    self.cx.sx_at(&Sx::Param(slots[2].clone()), p),
                ]
            }
            crate::hir::UserFnExpr::SlotSwizzle4(slots) => {
                vec![
                    self.cx.sx_at(&Sx::Param(slots[0].clone()), p),
                    self.cx.sx_at(&Sx::Param(slots[1].clone()), p),
                    self.cx.sx_at(&Sx::Param(slots[2].clone()), p),
                    self.cx.sx_at(&Sx::Param(slots[3].clone()), p),
                ]
            }
            crate::hir::UserFnExpr::Normalize2(v) => {
                let vec = self.compose_vec2_from_sx(v, p);
                let normalized = self.cx.ir.m1(Mf::Normalize, vec);
                self.vec_components(normalized, 2)
            }
            crate::hir::UserFnExpr::Normalize3(v) => {
                let vec = self.compose_vec3_from_sx(v, p);
                let normalized = self.cx.ir.m1(Mf::Normalize, vec);
                self.vec_components(normalized, 3)
            }
            crate::hir::UserFnExpr::Normalize4(v) => {
                let vec = self.compose_vec4_from_sx(v, p);
                let normalized = self.cx.ir.m1(Mf::Normalize, vec);
                self.vec_components(normalized, 4)
            }
            crate::hir::UserFnExpr::Min2 { a, b } => {
                let av = self.compose_vec2_from_sx(a, p);
                let bv = self.compose_vec2_from_sx(b, p);
                let out = self.cx.ir.m2(Mf::Min, av, bv);
                self.vec_components(out, 2)
            }
            crate::hir::UserFnExpr::Min3 { a, b } => {
                let av = self.compose_vec3_from_sx(a, p);
                let bv = self.compose_vec3_from_sx(b, p);
                let out = self.cx.ir.m2(Mf::Min, av, bv);
                self.vec_components(out, 3)
            }
            crate::hir::UserFnExpr::Min4 { a, b } => {
                let av = self.compose_vec4_from_sx(a, p);
                let bv = self.compose_vec4_from_sx(b, p);
                let out = self.cx.ir.m2(Mf::Min, av, bv);
                self.vec_components(out, 4)
            }
            crate::hir::UserFnExpr::Max2 { a, b } => {
                let av = self.compose_vec2_from_sx(a, p);
                let bv = self.compose_vec2_from_sx(b, p);
                let out = self.cx.ir.m2(Mf::Max, av, bv);
                self.vec_components(out, 2)
            }
            crate::hir::UserFnExpr::Max3 { a, b } => {
                let av = self.compose_vec3_from_sx(a, p);
                let bv = self.compose_vec3_from_sx(b, p);
                let out = self.cx.ir.m2(Mf::Max, av, bv);
                self.vec_components(out, 3)
            }
            crate::hir::UserFnExpr::Max4 { a, b } => {
                let av = self.compose_vec4_from_sx(a, p);
                let bv = self.compose_vec4_from_sx(b, p);
                let out = self.cx.ir.m2(Mf::Max, av, bv);
                self.vec_components(out, 4)
            }
            crate::hir::UserFnExpr::Clamp2 { x, lo, hi } => {
                let xv = self.compose_vec2_from_sx(x, p);
                let lv = self.compose_vec2_from_sx(lo, p);
                let hv = self.compose_vec2_from_sx(hi, p);
                let out = self.cx.ir.m3(Mf::Clamp, xv, lv, hv);
                self.vec_components(out, 2)
            }
            crate::hir::UserFnExpr::Clamp3 { x, lo, hi } => {
                let xv = self.compose_vec3_from_sx(x, p);
                let lv = self.compose_vec3_from_sx(lo, p);
                let hv = self.compose_vec3_from_sx(hi, p);
                let out = self.cx.ir.m3(Mf::Clamp, xv, lv, hv);
                self.vec_components(out, 3)
            }
            crate::hir::UserFnExpr::Clamp4 { x, lo, hi } => {
                let xv = self.compose_vec4_from_sx(x, p);
                let lv = self.compose_vec4_from_sx(lo, p);
                let hv = self.compose_vec4_from_sx(hi, p);
                let out = self.cx.ir.m3(Mf::Clamp, xv, lv, hv);
                self.vec_components(out, 4)
            }
        }
    }

    fn value_vector(&mut self, value: &crate::hir::UserFnExpr, width: usize) -> Option<Handle<Ex>> {
        let p = self.cx.ir.uv;
        match (width, value) {
            (2, crate::hir::UserFnExpr::Value(crate::hir::UserFnValue::Vec2(v))) => {
                Some(self.compose_vec2_from_sx(v, p))
            }
            (3, crate::hir::UserFnExpr::Value(crate::hir::UserFnValue::Vec3(v))) => {
                Some(self.compose_vec3_from_sx(v, p))
            }
            (4, crate::hir::UserFnExpr::Value(crate::hir::UserFnValue::Vec4(v))) => {
                Some(self.compose_vec4_from_sx(v, p))
            }
            (2, crate::hir::UserFnExpr::SlotSwizzle2(slots)) => Some(self.compose_vec2_from_sx(
                &(Sx::Param(slots[0].clone()), Sx::Param(slots[1].clone())),
                p,
            )),
            (3, crate::hir::UserFnExpr::SlotSwizzle3(slots)) => Some(self.compose_vec3_from_sx(
                &(
                    Sx::Param(slots[0].clone()),
                    Sx::Param(slots[1].clone()),
                    Sx::Param(slots[2].clone()),
                ),
                p,
            )),
            (4, crate::hir::UserFnExpr::SlotSwizzle4(slots)) => Some(self.compose_vec4_from_sx(
                &(
                    Sx::Param(slots[0].clone()),
                    Sx::Param(slots[1].clone()),
                    Sx::Param(slots[2].clone()),
                    Sx::Param(slots[3].clone()),
                ),
                p,
            )),
            (2, crate::hir::UserFnExpr::Normalize2(v)) => {
                let vec = self.compose_vec2_from_sx(v, p);
                Some(self.cx.ir.m1(Mf::Normalize, vec))
            }
            (3, crate::hir::UserFnExpr::Normalize3(v)) => {
                let vec = self.compose_vec3_from_sx(v, p);
                Some(self.cx.ir.m1(Mf::Normalize, vec))
            }
            (4, crate::hir::UserFnExpr::Normalize4(v)) => {
                let vec = self.compose_vec4_from_sx(v, p);
                Some(self.cx.ir.m1(Mf::Normalize, vec))
            }
            (2, crate::hir::UserFnExpr::Min2 { a, b }) => {
                let av = self.compose_vec2_from_sx(a, p);
                let bv = self.compose_vec2_from_sx(b, p);
                Some(self.cx.ir.m2(Mf::Min, av, bv))
            }
            (3, crate::hir::UserFnExpr::Min3 { a, b }) => {
                let av = self.compose_vec3_from_sx(a, p);
                let bv = self.compose_vec3_from_sx(b, p);
                Some(self.cx.ir.m2(Mf::Min, av, bv))
            }
            (4, crate::hir::UserFnExpr::Min4 { a, b }) => {
                let av = self.compose_vec4_from_sx(a, p);
                let bv = self.compose_vec4_from_sx(b, p);
                Some(self.cx.ir.m2(Mf::Min, av, bv))
            }
            (2, crate::hir::UserFnExpr::Max2 { a, b }) => {
                let av = self.compose_vec2_from_sx(a, p);
                let bv = self.compose_vec2_from_sx(b, p);
                Some(self.cx.ir.m2(Mf::Max, av, bv))
            }
            (3, crate::hir::UserFnExpr::Max3 { a, b }) => {
                let av = self.compose_vec3_from_sx(a, p);
                let bv = self.compose_vec3_from_sx(b, p);
                Some(self.cx.ir.m2(Mf::Max, av, bv))
            }
            (4, crate::hir::UserFnExpr::Max4 { a, b }) => {
                let av = self.compose_vec4_from_sx(a, p);
                let bv = self.compose_vec4_from_sx(b, p);
                Some(self.cx.ir.m2(Mf::Max, av, bv))
            }
            (2, crate::hir::UserFnExpr::Clamp2 { x, lo, hi }) => {
                let xv = self.compose_vec2_from_sx(x, p);
                let lv = self.compose_vec2_from_sx(lo, p);
                let hv = self.compose_vec2_from_sx(hi, p);
                Some(self.cx.ir.m3(Mf::Clamp, xv, lv, hv))
            }
            (3, crate::hir::UserFnExpr::Clamp3 { x, lo, hi }) => {
                let xv = self.compose_vec3_from_sx(x, p);
                let lv = self.compose_vec3_from_sx(lo, p);
                let hv = self.compose_vec3_from_sx(hi, p);
                Some(self.cx.ir.m3(Mf::Clamp, xv, lv, hv))
            }
            (4, crate::hir::UserFnExpr::Clamp4 { x, lo, hi }) => {
                let xv = self.compose_vec4_from_sx(x, p);
                let lv = self.compose_vec4_from_sx(lo, p);
                let hv = self.compose_vec4_from_sx(hi, p);
                Some(self.cx.ir.m3(Mf::Clamp, xv, lv, hv))
            }
            _ => None,
        }
    }

    fn slot_ptr(&self, slot: &str) -> Handle<Ex> {
        *self
            .cx
            .ir
            .param_scalars
            .get(slot)
            .expect("checker emitted unknown helper local slot")
    }

    fn lower_return_value(&mut self, value: &crate::hir::UserFnExpr) {
        let out = if self.helper.ret_components == 1 {
            self.value_components(value)[0]
        } else if let Some(vec) = self.value_vector(value, self.helper.ret_components as usize) {
            vec
        } else {
            let comps = self.value_components(value);
            match self.helper.ret_components {
                2 => self.cx.ir.vec2(comps[0], comps[1]),
                3 => self.cx.ir.add(Ex::Compose {
                    ty: self.cx.ir.types.v3,
                    components: vec![comps[0], comps[1], comps[2]],
                }),
                4 => self.cx.ir.add(Ex::Compose {
                    ty: self.cx.ir.types.v4,
                    components: vec![comps[0], comps[1], comps[2], comps[3]],
                }),
                _ => unreachable!("unsupported helper return arity"),
            }
        };
        self.cx
            .ir
            .push_statement(naga::Statement::Return { value: Some(out) });
    }

    fn lower_stmt_list(
        &mut self,
        stmts: &[crate::hir::UserFnStmt],
        in_loop: bool,
        implicit_tail_return: bool,
    ) {
        for (index, stmt) in stmts.iter().enumerate() {
            let is_tail = implicit_tail_return && index + 1 == stmts.len();
            self.cx.user_call_cache.clear();
            self.cx.clear_point_sensitive_caches();
            match stmt {
                crate::hir::UserFnStmt::Let { name, slots, init } => {
                    let init_values = self.value_components(init);
                    let is_mutated = self.mutated_names.contains(name);
                    if (2..=4).contains(&slots.len()) {
                        let vec_ty = vector_type_handle(
                            Self::expression_kind(init),
                            slots.len(),
                            &self.cx.ir.types,
                        );
                        let init_vec = self.cx.ir.add(Ex::Compose {
                            ty: vec_ty,
                            components: init_values,
                        });
                        if is_mutated {
                            let vec_ptr = self.cx.ir.local(&format!("loc_{name}"), vec_ty, None);
                            self.cx.ir.store(vec_ptr, init_vec);
                            for (index, slot) in slots.iter().enumerate() {
                                let comp_ptr = self.cx.ir.add(Ex::AccessIndex {
                                    base: vec_ptr,
                                    index: index as u32,
                                });
                                self.cx.ir.bind_scalar_slot_ptr(slot.clone(), comp_ptr);
                            }
                            self.bind_slots(name.clone(), slots.clone(), Some(vec_ptr));
                        } else {
                            for (index, slot) in slots.iter().enumerate() {
                                let comp = self.cx.ir.add(Ex::AccessIndex {
                                    base: init_vec,
                                    index: index as u32,
                                });
                                self.cx.ir.bind_scalar_slot_value(slot.clone(), comp);
                            }
                            self.bind_slots(name.clone(), slots.clone(), None);
                        }
                    } else {
                        for (slot, init) in slots.iter().zip(init_values) {
                            if is_mutated {
                                let kind = match &stmt {
                                    crate::hir::UserFnStmt::Let {
                                        init:
                                            crate::hir::UserFnExpr::Value(
                                                crate::hir::UserFnValue::Scalar(value),
                                            ),
                                        ..
                                    } => value.scalar_kind(),
                                    _ => crate::typed_scalar::Kind::F32,
                                };
                                let ty = scalar_type_handle(kind, &self.cx.ir.types);
                                let ptr = self.cx.ir.local(slot, ty, None);
                                self.cx.ir.bind_scalar_slot_ptr(slot.clone(), ptr);
                                self.cx.ir.store(ptr, init);
                            } else {
                                if slots.len() == 1 {
                                    self.cx.ir.name(init, &format!("loc_{name}"));
                                }
                                self.cx.ir.bind_scalar_slot_value(slot.clone(), init);
                            }
                        }
                        self.bind_slots(name.clone(), slots.clone(), None);
                    }
                }
                crate::hir::UserFnStmt::Assign {
                    name,
                    field_path,
                    value,
                } => {
                    let rhs = self.value_components(value);
                    let slots = self
                        .lookup_slots(name)
                        .expect("checker emitted assignment to unknown helper local");
                    if let Some(path) = field_path {
                        let indices: Vec<usize> = path
                            .chars()
                            .map(|c| match c {
                                'x' | 'r' => 0,
                                'y' | 'g' => 1,
                                'z' | 'b' => 2,
                                'w' | 'a' => 3,
                                _ => panic!("unsupported helper swizzle assignment component"),
                            })
                            .collect();
                        if let Some(vec_ptr) = self.lookup_vec_ptr(name) {
                            for (rhs_idx, target_idx) in indices.into_iter().enumerate() {
                                let ptr = self.cx.ir.add(Ex::AccessIndex {
                                    base: vec_ptr,
                                    index: target_idx as u32,
                                });
                                self.cx.ir.store(ptr, rhs[rhs_idx]);
                            }
                        } else {
                            for (rhs_idx, target_idx) in indices.into_iter().enumerate() {
                                let ptr = self.slot_ptr(&slots[target_idx]);
                                self.cx.ir.store(ptr, rhs[rhs_idx]);
                            }
                        }
                    } else {
                        if let Some(vec_ptr) = self.lookup_vec_ptr(name)
                            && (2..=4).contains(&slots.len())
                        {
                            let vec_ty = vector_type_handle(
                                Self::expression_kind(value),
                                slots.len(),
                                &self.cx.ir.types,
                            );
                            let rhs_vec = self.cx.ir.add(Ex::Compose {
                                ty: vec_ty,
                                components: rhs,
                            });
                            self.cx.ir.store(vec_ptr, rhs_vec);
                        } else {
                            for (slot, rhs_value) in slots.iter().zip(rhs) {
                                let ptr = self.slot_ptr(slot);
                                self.cx.ir.store(ptr, rhs_value);
                            }
                        }
                    }
                }
                crate::hir::UserFnStmt::Expr { value } => {
                    if is_tail {
                        self.lower_return_value(value);
                    } else {
                        let _ = self.value_components(value);
                    }
                }
                crate::hir::UserFnStmt::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    let p = self.cx.ir.uv;
                    let cond_s = self.cx.sx_at(cond, p);
                    let zero = self.cx.ir.lit(0.0);
                    let cond_bool = self.cx.ir.bin(Bo::NotEqual, cond_s, zero);

                    self.cx.ir.begin_block();
                    self.cx.user_call_cache.clear();
                    self.cx.clear_point_sensitive_caches();
                    self.push_scope();
                    self.lower_stmt_list(then_body, in_loop, is_tail);
                    self.pop_scope();
                    let accept = self.cx.ir.end_block();

                    self.cx.ir.begin_block();
                    self.cx.user_call_cache.clear();
                    self.cx.clear_point_sensitive_caches();
                    self.push_scope();
                    self.lower_stmt_list(else_body, in_loop, is_tail);
                    self.pop_scope();
                    let reject = self.cx.ir.end_block();

                    self.cx.user_call_cache.clear();
                    self.cx.clear_point_sensitive_caches();

                    self.cx.ir.push_statement(naga::Statement::If {
                        condition: cond_bool,
                        accept,
                        reject,
                    });
                }
                crate::hir::UserFnStmt::For {
                    name,
                    slots,
                    values,
                    body,
                    index_name,
                } => {
                    self.cx.user_call_cache.clear();
                    self.cx.clear_point_sensitive_caches();
                    for slot in slots {
                        if !self.cx.ir.param_scalars.contains_key(slot) {
                            let zero = self.cx.ir.lit(0.0);
                            let ptr = self.cx.ir.local(slot, self.cx.ir.types.f32_, Some(zero));
                            self.cx.ir.bind_scalar_slot_ptr(slot.clone(), ptr);
                        }
                    }
                    let zero = self.cx.ir.lit(0.0);
                    let idx_ptr = self
                        .cx
                        .ir
                        .local("loop_idx", self.cx.ir.types.f32_, Some(zero));
                    // If an index variable is requested, allocate a slot for it and
                    // bind it so that body expressions can reference it.
                    let idx_var_ptr = if let Some(idx_name) = index_name {
                        let zero2 = self.cx.ir.lit(0.0);
                        let ptr = self
                            .cx
                            .ir
                            .local(idx_name, self.cx.ir.types.f32_, Some(zero2));
                        self.cx.ir.bind_scalar_slot_ptr(idx_name.clone(), ptr);
                        Some(ptr)
                    } else {
                        None
                    };
                    self.bind_slots(name.clone(), slots.clone(), None);
                    let iter_ptr = self.slot_ptr(&slots[0]);

                    self.cx.ir.begin_block();
                    let idx = self.cx.ir.load(idx_ptr);
                    let start = values.first().cloned().unwrap_or(Sx::Lit(0.0));
                    let step = if values.len() >= 2 {
                        match (&values[0], &values[1]) {
                            (Sx::Lit(a), Sx::Lit(b)) => Sx::Lit(b - a),
                            _ => Sx::Lit(1.0),
                        }
                    } else {
                        Sx::Lit(1.0)
                    };
                    let p = self.cx.ir.uv;
                    let start_h = self.cx.sx_at(&start, p);
                    let step_h = self.cx.sx_at(&step, p);
                    let scaled = self.cx.ir.mul(idx, step_h);
                    let iter_value = self.cx.ir.addx(start_h, scaled);
                    self.cx.ir.store(iter_ptr, iter_value);
                    // Keep the index variable in sync with the loop counter.
                    if let Some(iptr) = idx_var_ptr {
                        self.cx.ir.store(iptr, idx);
                    }

                    self.push_scope();
                    self.lower_stmt_list(body, true, false);
                    self.pop_scope();
                    let body_block = self.cx.ir.end_block();

                    self.cx.ir.begin_block();
                    let one = self.cx.ir.lit(1.0);
                    let idx_cur = self.cx.ir.load(idx_ptr);
                    let idx_next = self.cx.ir.addx(idx_cur, one);
                    self.cx.ir.store(idx_ptr, idx_next);
                    let limit = self.cx.ir.lit(values.len() as f32);
                    let idx_live = self.cx.ir.load(idx_ptr);
                    let break_if = self.cx.ir.bin(Bo::GreaterEqual, idx_live, limit);
                    let continuing = self.cx.ir.end_block();

                    self.cx.ir.push_statement(naga::Statement::Loop {
                        body: body_block,
                        continuing,
                        break_if: Some(break_if),
                    });
                }
                crate::hir::UserFnStmt::Return { value } => {
                    self.lower_return_value(value);
                }
                crate::hir::UserFnStmt::Break => {
                    if in_loop {
                        self.cx.ir.push_statement(naga::Statement::Break);
                    }
                }
            }
        }
    }
}

/// Lower the body of a scatter layer as a standalone helper function.
///
/// The helper signature is:
/// `(p: vec2, px: f32, aa: f32, time: f32,
///   inst_pos_x: f32, inst_pos_y: f32, inst_id: f32, inst_index01: f32,
///   …canvas_params…) -> vec4<f32>`
///
/// This lets the main canvas call the helper once per instance (a tiny
/// call site) rather than inlining the full body for every instance.
#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(super) fn lower_scatter_body_fn(
    hir: &Hir,
    scatter_id: LayerId,
    body: LayerId,
    t: &TypeHandles,
    policy: LoweringPolicy,
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    path_helper_fns: HashMap<usize, PathHelperFns>,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
) -> naga::Function {
    let (ir, instance_ctx) = IrBuilder::new_scatter_body(
        format!("fresco_{}_scatter_l{scatter_id}", hir.name),
        t,
        &hir.params,
        policy,
        tex_globals,
        sampler_global,
        global_uniform_globals,
    );
    let p = ir.uv;

    let mut cx = FnCtx {
        hir,
        ir,
        params_are_immutable: false,
        current_pass_id: None,
        layer_to_pass: Vec::new(),
        pass_output_target_names: HashMap::new(),
        sdf_cache: HashMap::new(),
        sx_cache: HashMap::new(),
        contour_cache: HashMap::new(),
        invariant_sx_cache: HashMap::new(),
        invariant_user_helper_cache: HashMap::new(),
        local_pixel_span_cache: HashMap::new(),
        shape_anchor_cache: HashMap::new(),
        shape_half_extent_cache: HashMap::new(),
        shape_local_point_cache: HashMap::new(),
        shape_anchor_space_point_cache: HashMap::new(),
        shape_glow_warp_cache: HashMap::new(),
        source_color_ctx: Vec::new(),
        effect_input_layer_ctx: Vec::new(),
        scatter_instance_ctx: vec![instance_ctx],
        repeat_cell_ctx: Vec::new(),
        cellular_sample_depth: 0,
        discontinuity_space_depth: 0,
        box_filter_domains: HashMap::new(),
        // Scatter bodies may contain intrinsic helper calls (e.g. hoisted
        // noise helpers), so carry the prebuilt helper function table.
        scatter_body_fns: HashMap::new(),
        user_helper_fns,
        user_call_cache: HashMap::new(),
        path_helper_fns,
        path_sample_cache: HashMap::new(),
        path_param_sample_cache: HashMap::new(),
        texture_sample_cache: HashMap::new(),
        gradient_dither3_cache: None,
        color_shape: None,
        cell_inset_pixel_span: None,
        var_overrides: Vec::new(),
        stats: Stats {
            ..Default::default()
        },
    };

    let (rgb, a) = cx.layer_color(body, p);
    let rgba = cx.ir.add(Ex::Compose {
        ty: cx.ir.types.v4,
        components: vec![rgb, a],
    });
    cx.ir.return_value(rgba);
    cx.ir.finish()
}

pub(super) fn lower_user_helper_fn(
    hir: &Hir,
    helper: &UserFnHelper,
    t: &TypeHandles,
    policy: LoweringPolicy,
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    path_helper_fns: HashMap<usize, PathHelperFns>,
    global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
) -> naga::Function {
    let ir = IrBuilder::new_user_helper(helper, t, policy, global_uniform_globals);
    let mut cx = FnCtx {
        hir,
        ir,
        params_are_immutable: false,
        current_pass_id: None,
        layer_to_pass: Vec::new(),
        pass_output_target_names: HashMap::new(),
        sdf_cache: HashMap::new(),
        sx_cache: HashMap::new(),
        contour_cache: HashMap::new(),
        invariant_sx_cache: HashMap::new(),
        invariant_user_helper_cache: HashMap::new(),
        local_pixel_span_cache: HashMap::new(),
        shape_anchor_cache: HashMap::new(),
        shape_half_extent_cache: HashMap::new(),
        shape_local_point_cache: HashMap::new(),
        shape_anchor_space_point_cache: HashMap::new(),
        shape_glow_warp_cache: HashMap::new(),
        source_color_ctx: Vec::new(),
        effect_input_layer_ctx: Vec::new(),
        scatter_instance_ctx: Vec::new(),
        repeat_cell_ctx: Vec::new(),
        cellular_sample_depth: 0,
        discontinuity_space_depth: 0,
        box_filter_domains: HashMap::new(),
        scatter_body_fns: HashMap::new(),
        user_helper_fns,
        user_call_cache: HashMap::new(),
        path_helper_fns,
        path_sample_cache: HashMap::new(),
        path_param_sample_cache: HashMap::new(),
        texture_sample_cache: HashMap::new(),
        gradient_dither3_cache: None,
        color_shape: None,
        cell_inset_pixel_span: None,
        var_overrides: Vec::new(),
        stats: Stats {
            ..Default::default()
        },
    };

    let mut runtime = UserHelperRuntimeCtx::new(&mut cx, helper);
    runtime.lower_stmt_list(&helper.body_stmts, false, true);

    cx.ir.finish()
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
fn lower_scene_fn_with_root(
    hir: &Hir,
    scene_name: &str,
    scene_root: LayerId,
    current_pass_id: Option<usize>,
    layer_to_pass: Vec<usize>,
    pass_output_target_names: HashMap<usize, String>,
    t: &TypeHandles,
    policy: LoweringPolicy,
    scatter_body_fns: HashMap<LayerId, Handle<naga::Function>>,
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    path_helper_fns: HashMap<usize, PathHelperFns>,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    param_storage_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
) -> (naga::Function, Stats) {
    let mut cx = FnCtx {
        hir,
        ir: IrBuilder::new(
            scene_name,
            t,
            &hir.params,
            policy,
            tex_globals,
            sampler_global,
            param_storage_globals,
            global_uniform_globals,
            true,
            None,
        ),
        params_are_immutable: true,
        current_pass_id,
        layer_to_pass,
        pass_output_target_names,
        sdf_cache: HashMap::new(),
        sx_cache: HashMap::new(),
        contour_cache: HashMap::new(),
        invariant_sx_cache: HashMap::new(),
        invariant_user_helper_cache: HashMap::new(),
        local_pixel_span_cache: HashMap::new(),
        shape_anchor_cache: HashMap::new(),
        shape_half_extent_cache: HashMap::new(),
        shape_local_point_cache: HashMap::new(),
        shape_anchor_space_point_cache: HashMap::new(),
        shape_glow_warp_cache: HashMap::new(),
        source_color_ctx: Vec::new(),
        effect_input_layer_ctx: Vec::new(),
        scatter_instance_ctx: Vec::new(),
        repeat_cell_ctx: Vec::new(),
        cellular_sample_depth: 0,
        discontinuity_space_depth: 0,
        box_filter_domains: HashMap::new(),
        scatter_body_fns,
        user_helper_fns,
        user_call_cache: HashMap::new(),
        path_helper_fns,
        path_sample_cache: HashMap::new(),
        path_param_sample_cache: HashMap::new(),
        texture_sample_cache: HashMap::new(),
        gradient_dither3_cache: None,
        color_shape: None,
        cell_inset_pixel_span: None,
        var_overrides: Vec::new(),
        stats: Stats {
            ..Default::default()
        },
    };

    // `px` and `aa` are ambient values supplied through FrescoCtx.
    cx.ir.name(cx.ir.px, "px");
    cx.ir.name(cx.ir.aa, "aa");

    // Compute the footprint Jacobian matrix from the canvas_space chain (§3.8, §24.1).
    // The Jacobian represents the derivative of the composed sample mapping, and the
    // parallelogram spanned by its columns is the footprint — the region of local space
    // one output pixel covers. This drives gradient-driven antialiasing and filtering.
    if let Some(xforms) = &hir.canvas_space {
        let (j11_sx, j12_sx, j21_sx, j22_sx) = crate::deriv::jacobian(xforms);
        // Lower the Jacobian Sx expressions to IR handles and store them
        let j11_h = cx.sx_at(&j11_sx, cx.ir.uv);
        let j12_h = cx.sx_at(&j12_sx, cx.ir.uv);
        let j21_h = cx.sx_at(&j21_sx, cx.ir.uv);
        let j22_h = cx.sx_at(&j22_sx, cx.ir.uv);
        cx.ir.name(j11_h, "jacobian_j11");
        cx.ir.name(j12_h, "jacobian_j12");
        cx.ir.name(j21_h, "jacobian_j21");
        cx.ir.name(j22_h, "jacobian_j22");
        cx.ir.jacobian_j11 = Some(j11_h);
        cx.ir.jacobian_j12 = Some(j12_h);
        cx.ir.jacobian_j21 = Some(j21_h);
        cx.ir.jacobian_j22 = Some(j22_h);
    }

    // Fold the whole layer DAG down to a vec3 color at the root coordinate.
    // `canvas_space` provides footprint information, but geometry transforms
    // are explicit through `in space` blocks.
    let root_p = cx.ir.uv;
    cx.ir.name(root_p, "p_root");
    let rgb = match &hir.layers[scene_root] {
        Layer::Compose(entries) => cx.fold_compose_rgb_only(entries, root_p),
        _ => {
            // Canvas output is flattened over black, just like the compose
            // path above. A standalone layer still contributes its coverage.
            let (rgb, alpha) = cx.layer_color(scene_root, root_p);
            let coverage = cx.ir.splat3(alpha);
            cx.ir.mul(rgb, coverage)
        }
    };
    cx.ir.name(rgb, "col");

    let one = cx.ir.lit(1.0);
    let rgba = cx.ir.add(Ex::Compose {
        ty: cx.ir.types.v4,
        components: vec![rgb, one],
    });
    cx.ir.return_value(rgba);

    let mut stats = cx.stats.clone();
    stats.emitted_instructions = cx.ir.emitted_instructions();
    stats.feature_instruction_counts = cx.ir.feature_instruction_counts();
    stats.feature_instruction_counts_by_locality = cx.ir.feature_instruction_counts_by_locality();
    if matches!(hir.layers[hir.root], Layer::MotionBlur { .. }) {
        stats.iter_loops += 1;
        *stats
            .feature_instruction_counts
            .entry("motion_blur".to_string())
            .or_insert(0) += 1;
        *stats
            .feature_instruction_counts_by_locality
            .entry("local".to_string())
            .or_default()
            .entry("motion_blur".to_string())
            .or_insert(0) += 1;
    }
    (cx.ir.finish(), stats)
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(super) fn lower_scene_fn(
    hir: &Hir,
    t: &TypeHandles,
    policy: LoweringPolicy,
    scatter_body_fns: HashMap<LayerId, Handle<naga::Function>>,
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    path_helper_fns: HashMap<usize, PathHelperFns>,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    param_storage_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
) -> (naga::Function, Stats) {
    let scene_root = match &hir.layers[hir.root] {
        Layer::MotionBlur { inner, .. } => *inner,
        _ => hir.root,
    };
    lower_scene_fn_with_root(
        hir,
        &format!("fresco_scene_{}", hir.name),
        scene_root,
        None,
        Vec::new(),
        HashMap::new(),
        t,
        policy,
        scatter_body_fns,
        user_helper_fns,
        path_helper_fns,
        tex_globals,
        sampler_global,
        param_storage_globals,
        global_uniform_globals,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(super) fn lower_scene_pass_fn(
    hir: &Hir,
    pass_id: usize,
    scene_root: LayerId,
    layer_to_pass: Vec<usize>,
    pass_output_target_names: HashMap<usize, String>,
    t: &TypeHandles,
    policy: LoweringPolicy,
    scatter_body_fns: HashMap<LayerId, Handle<naga::Function>>,
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    path_helper_fns: HashMap<usize, PathHelperFns>,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    param_storage_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
) -> naga::Function {
    lower_scene_fn_with_root(
        hir,
        &format!("fresco_scene_{}_pass{pass_id}", hir.name),
        scene_root,
        Some(pass_id),
        layer_to_pass,
        pass_output_target_names,
        t,
        policy,
        scatter_body_fns,
        user_helper_fns,
        path_helper_fns,
        tex_globals,
        sampler_global,
        param_storage_globals,
        global_uniform_globals,
    )
    .0
}

fn lower_canvas_entry_fn_named(
    hir: &Hir,
    entry_name: &str,
    t: &TypeHandles,
    policy: LoweringPolicy,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    scene_fn: Handle<naga::Function>,
) -> naga::Function {
    fn call_scene_sample(
        cx: &mut FnCtx<'_>,
        scene_fn: Handle<naga::Function>,
        scene_ctx: Handle<Ex>,
        scene_params: &[Handle<Ex>],
        tau: Handle<Ex>,
    ) -> Handle<Ex> {
        let mut call_args = vec![cx.ir.uv, tau, scene_ctx];
        call_args.extend(scene_params.iter().copied());
        let call_result = cx.ir.add(Ex::CallResult(scene_fn));
        cx.ir.push_statement(naga::Statement::Call {
            function: scene_fn,
            arguments: call_args,
            result: Some(call_result),
        });
        call_result
    }

    let mut cx = FnCtx {
        hir,
        ir: IrBuilder::new(
            entry_name,
            t,
            &hir.params,
            policy,
            tex_globals,
            sampler_global,
            &HashMap::new(),
            &HashMap::new(),
            false,
            hir.entry_context.as_ref(),
        ),
        params_are_immutable: true,
        current_pass_id: None,
        layer_to_pass: Vec::new(),
        pass_output_target_names: HashMap::new(),
        sdf_cache: HashMap::new(),
        sx_cache: HashMap::new(),
        contour_cache: HashMap::new(),
        invariant_sx_cache: HashMap::new(),
        invariant_user_helper_cache: HashMap::new(),
        local_pixel_span_cache: HashMap::new(),
        shape_anchor_cache: HashMap::new(),
        shape_half_extent_cache: HashMap::new(),
        shape_local_point_cache: HashMap::new(),
        shape_anchor_space_point_cache: HashMap::new(),
        shape_glow_warp_cache: HashMap::new(),
        source_color_ctx: Vec::new(),
        effect_input_layer_ctx: Vec::new(),
        scatter_instance_ctx: Vec::new(),
        repeat_cell_ctx: Vec::new(),
        cellular_sample_depth: 0,
        discontinuity_space_depth: 0,
        box_filter_domains: HashMap::new(),
        scatter_body_fns: HashMap::new(),
        user_helper_fns: HashMap::new(),
        user_call_cache: HashMap::new(),
        path_helper_fns: HashMap::new(),
        path_sample_cache: HashMap::new(),
        path_param_sample_cache: HashMap::new(),
        texture_sample_cache: HashMap::new(),
        gradient_dither3_cache: None,
        color_shape: None,
        cell_inset_pixel_span: None,
        var_overrides: Vec::new(),
        stats: Stats {
            ..Default::default()
        },
    };

    // Build a context payload shared across scene re-evaluations.
    let res_y = cx.ir.add(Ex::AccessIndex {
        base: cx.ir.res,
        index: 1,
    });
    let eps = cx.ir.lit(1.0e-6);
    let safe_res_y = cx.ir.m2(Mf::Max, res_y, eps);
    let one = cx.ir.lit(1.0);
    let px = cx.ir.add(Ex::Binary {
        op: Bo::Divide,
        left: one,
        right: safe_res_y,
    });
    let k = cx.ir.lit(1.5);
    let aa = cx.ir.add(Ex::Binary {
        op: Bo::Multiply,
        left: k,
        right: px,
    });
    let scene_ctx = cx.ir.add(Ex::Compose {
        ty: t.ctx,
        components: vec![cx.ir.res, cx.ir.delta, px, aa],
    });

    let param_start = 4usize;
    let argc = cx.ir.function_arg_count();
    let scene_params = if let Some(context) = &hir.entry_context {
        let arguments = cx.ir.function.function.arguments.clone();
        let mut values = Vec::new();
        for param in &hir.params {
            if context.is_component(&param.name) {
                values.push(cx.ir.param_scalars[&param.name]);
            } else {
                let prefix = format!("{}__", param.name);
                for (index, arg) in arguments.iter().enumerate().skip(1) {
                    if arg
                        .name
                        .as_ref()
                        .is_some_and(|name| name == &param.name || name.starts_with(&prefix))
                    {
                        values.push(cx.ir.add(Ex::FunctionArgument(
                            u32::try_from(index).expect("argument index fits u32"),
                        )));
                    }
                }
            }
        }
        values
    } else {
        cx.ir.function_args_in_order_from(param_start, argc)
    };

    let out_rgba = if let Layer::MotionBlur { shutter, .. } = &hir.layers[hir.root] {
        const TEMPORAL_SAMPLES: usize = 5;
        let n = cx.ir.lit(TEMPORAL_SAMPLES as f32);
        let zero = cx.ir.lit(0.0);
        let one = cx.ir.lit(1.0);
        let eps = cx.ir.lit(1.0e-6);
        let safe_delta = cx.ir.m2(Mf::Max, cx.ir.delta, eps);
        let frame_div = cx.ir.div(cx.ir.time, safe_delta);
        let frame_index = cx.ir.m1(Mf::Floor, frame_div);
        let uvx = cx.ir.x_of(cx.ir.uv);
        let uvy = cx.ir.y_of(cx.ir.uv);
        let ign_kx = cx.ir.lit(0.06711056);
        let ign_ky = cx.ir.lit(0.00583715);
        let ign_mul = cx.ir.lit(52.982_918);
        let jitter_frame_weight = cx.ir.lit(0.754_877_7);
        let jitter_index_weight = cx.ir.lit(0.5698403);
        let zero4 = cx.ir.add(Ex::Compose {
            ty: cx.ir.types.v4,
            components: vec![zero, zero, zero, zero],
        });
        let acc_ptr = cx.ir.local("motion_blur_acc", cx.ir.types.v4, Some(zero4));
        let idx_ptr = cx.ir.local("motion_blur_i", cx.ir.types.f32_, Some(zero));

        let shutter_h = cx.sx_at(shutter, cx.ir.uv);

        cx.ir.begin_block();
        let i = cx.ir.load(idx_ptr);
        let ign_dot_x = cx.ir.mul(uvx, ign_kx);
        let ign_dot_y = cx.ir.mul(uvy, ign_ky);
        let ign_dot = cx.ir.addx(ign_dot_x, ign_dot_y);
        let ign_seed = cx.ir.m1(Mf::Fract, ign_dot);
        let jitter_frame_term = cx.ir.mul(frame_index, jitter_frame_weight);
        let jitter_index_term = cx.ir.mul(i, jitter_index_weight);
        let jitter_mix = cx.ir.addx(jitter_frame_term, jitter_index_term);
        let jitter_phase = cx.ir.addx(ign_seed, jitter_mix);
        let jitter_phase_frac = cx.ir.m1(Mf::Fract, jitter_phase);
        let jitter_scaled = cx.ir.mul(ign_mul, jitter_phase_frac);
        let jitter = cx.ir.m1(Mf::Fract, jitter_scaled);
        let i_jittered = cx.ir.addx(i, jitter);
        let frac = cx.ir.div(i_jittered, n);
        let dt = cx.ir.mul(shutter_h, frac);
        let tau = cx.ir.sub(cx.ir.time, dt);
        let sample = call_scene_sample(&mut cx, scene_fn, scene_ctx, &scene_params, tau);
        let acc_cur = cx.ir.load(acc_ptr);
        let acc_next = cx.ir.addx(acc_cur, sample);
        cx.ir.store(acc_ptr, acc_next);
        let body = cx.ir.end_block();

        cx.ir.begin_block();
        let i_cur = cx.ir.load(idx_ptr);
        let i_next = cx.ir.addx(i_cur, one);
        cx.ir.store(idx_ptr, i_next);
        let break_if = cx.ir.bin(Bo::GreaterEqual, i_next, n);
        let continuing = cx.ir.end_block();

        cx.ir.push_statement(naga::Statement::Loop {
            body,
            continuing,
            break_if: Some(break_if),
        });

        let acc = cx.ir.load(acc_ptr);
        let n4 = cx.ir.add(Ex::Compose {
            ty: cx.ir.types.v4,
            components: vec![n, n, n, n],
        });
        cx.ir.div(acc, n4)
    } else {
        let tau_now = cx.ir.time;
        call_scene_sample(&mut cx, scene_fn, scene_ctx, &scene_params, tau_now)
    };

    cx.ir.return_value(out_rgba);
    cx.ir.finish()
}

pub(super) fn lower_canvas_entry_fn(
    hir: &Hir,
    t: &TypeHandles,
    policy: LoweringPolicy,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    scene_fn: Handle<naga::Function>,
) -> naga::Function {
    lower_canvas_entry_fn_named(
        hir,
        &format!("fresco_{}", hir.name),
        t,
        policy,
        tex_globals,
        sampler_global,
        scene_fn,
    )
}

pub(super) fn lower_canvas_pass_entry_fn(
    hir: &Hir,
    pass_id: usize,
    t: &TypeHandles,
    policy: LoweringPolicy,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    scene_fn: Handle<naga::Function>,
) -> naga::Function {
    lower_canvas_entry_fn_named(
        hir,
        &format!("fresco_{}_pass{pass_id}_", hir.name),
        t,
        policy,
        tex_globals,
        sampler_global,
        scene_fn,
    )
}

#[cfg(test)]
mod tests {
    use crate::test_support::compile_source;

    const TEST_SOURCE_PATH: &str = "helper.fr";

    fn compile_ir(src: &str) -> String {
        let out = compile_source(src, TEST_SOURCE_PATH, "ir", false).unwrap_or_else(|diags| {
            panic!("expected compile success, got diagnostics: {diags:#?}")
        });
        out.emitted
    }

    fn compile_wgsl(src: &str) -> String {
        let out = compile_source(src, TEST_SOURCE_PATH, "wgsl", false).unwrap_or_else(|diags| {
            panic!("expected compile success, got diagnostics: {diags:#?}")
        });
        out.emitted
    }

    fn compile_explain(src: &str) -> String {
        let out = compile_source(src, TEST_SOURCE_PATH, "wgsl", true).unwrap_or_else(|diags| {
            panic!("expected compile success, got diagnostics: {diags:#?}")
        });
        out.explain
            .unwrap_or_else(|| panic!("expected explain output to be present"))
    }

    #[test]
    fn helper_vec_intrinsics_do_not_duplicate_math_nodes_in_ir() {
        let src = r#"fn vec_intrinsics(v: vec3) -> vec3 {
  let a = max(v, vec3(0.0));
  let b = min(a, vec3(1.0));
  return clamp(b, vec3(0.2), vec3(0.8));
}

canvas t(uv: coord, time: signal) -> color {
  let q = vec_intrinsics(vec3(uv.x, uv.y, sin(time) * 0.5 + 0.5));
  compose {
    fill(rgb(q.x, q.y, q.z))
  }
}
"#;

        let ir = compile_ir(src);
        let max_count = ir.matches("fun: Max,").count();
        let min_count = ir.matches("fun: Min,").count();
        let clamp_count = ir.matches("fun: Clamp,").count();

        assert!(
            max_count >= 1,
            "expected helper IR lowering to include Max math nodes\nir:\n{ir}"
        );
        assert!(
            min_count >= 1,
            "expected helper IR lowering to include Min math nodes\nir:\n{ir}"
        );
        assert_eq!(
            clamp_count, 1,
            "expected exactly one vec Clamp math node in helper IR lowering\nir:\n{ir}"
        );
    }

    #[test]
    fn gridline_lowering_avoids_fract_based_single_line_emulation() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
  let axes = gridline(along: x, at: 0.0) | gridline(along: y, at: 0.0)
  compose {
    axes |> stroke(2px) |> fill(#5a6d94)
  }
}
"#;

        let ir = compile_ir(src);
        assert!(
            !ir.contains("lines_d_s"),
            "expected gridline to lower as direct single-line distance, not periodic lines\nir:\n{ir}"
        );
        assert!(
            !ir.contains("fun: Fract"),
            "expected gridline lowering to avoid fract-based large-spacing emulation\nir:\n{ir}"
        );
    }

    #[test]
    fn fbm_lowering_reuses_a_helper_function() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
  let a = fbm(uv + (0.05, 0.03))
  let b = fbm(uv + (0.10, 0.06))
  compose {
    fill(rgb(a, b, 0.0))
  }
}
"#;

        let wgsl = compile_wgsl(src);
        let helper_name = "_builtin_fbm_4_";
        let helper_refs = wgsl.matches(helper_name).count();

        assert!(
            wgsl.contains(&format!("fn {helper_name}(")),
            "expected fbm lowering to emit a reusable helper function\nwgsl:\n{wgsl}"
        );
        assert!(
            helper_refs >= 3,
            "expected fbm helper to be defined once and called multiple times\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn noise2_lowering_reuses_a_helper_function() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
  let a = noise2(uv + (0.05, 0.03))
  let b = noise2(uv + (0.10, 0.06))
  compose {
    fill(rgb(a, b, 0.0))
  }
}
"#;

        let wgsl = compile_wgsl(src);
        let helper_name = "_builtin_noise2_";
        let helper_refs = wgsl.matches(helper_name).count();

        assert!(
            wgsl.contains(&format!("fn {helper_name}(")),
            "expected noise2 lowering to emit a reusable helper function\nwgsl:\n{wgsl}"
        );
        assert!(
            helper_refs >= 3,
            "expected noise2 helper to be defined once and called multiple times\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn noise1_lowering_reuses_a_helper_function() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
  let a = noise1(uv.x + time * 0.1)
  let b = noise1(uv.y + time * 0.2)
  compose {
    fill(rgb(a, b, 0.0))
  }
}
"#;

        let wgsl = compile_wgsl(src);
        let helper_name = "_builtin_noise1_";
        let helper_refs = wgsl.matches(helper_name).count();

        assert!(
            wgsl.contains(&format!("fn {helper_name}(")),
            "expected noise1 lowering to emit a reusable helper function\nwgsl:\n{wgsl}"
        );
        assert!(
            helper_refs >= 3,
            "expected noise1 helper to be defined once and called multiple times\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn noise3_lowering_reuses_a_helper_function() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
  let a = noise3((uv.x, uv.y, time * 0.1))
  let b = noise3((uv.x + 0.1, uv.y + 0.2, time * 0.2))
  compose {
    fill(rgb(a, b, 0.0))
  }
}
"#;

        let wgsl = compile_wgsl(src);
        let helper_name = "_builtin_noise3_";
        let helper_refs = wgsl.matches(helper_name).count();

        assert!(
            wgsl.contains(&format!("fn {helper_name}(")),
            "expected noise3 lowering to emit a reusable helper function\nwgsl:\n{wgsl}"
        );
        assert!(
            helper_refs >= 3,
            "expected noise3 helper to be defined once and called multiple times\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn filtered_checker_width_uses_pixel_footprint_units() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = scale(1.0)
  filtering(on);
  compose {
        in space warped {
            let c = checker(scale: 0.25uv, at: (0.5, 0.5))
            box(at: (0.5, 0.5), size: (1.0, 1.0)) |> fill(rgb(c, c, c))
        }
  }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("dpdx(p_space)")
                && wgsl.contains("dpdy(p_space)")
                && wgsl.contains("/ 0.25f), 0.00001f)"),
            "expected filtered checker width to use local footprint derivatives in pixel units\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn polar_dial_uses_periodic_finite_box_filter() {
        let src = r#"canvas dial(uv: coord) -> color {
            param progress: f32 = 0.72 in 0 .. 1
            space stage = centered(aspect: preserve)
            space dial_space = polar(center: center, from: -90deg, direction: clockwise)
            let track = box(at: (0.5, 0.32), size: (1.0, 0.045))
            let bar = box(at: (progress / 2, 0.32), size: (progress, 0.045))
            compose {
                fill(#0d1117)
                in space stage {
                    in space dial_space {
                        compose {
                            track |> fill(#ffffff20)
                            bar |> fill(#4facfe)
                        }
                    }
                }
            }
        }"#;
        let wgsl = compile_wgsl(src);
        assert!(wgsl.contains("box_periodic_coverage"), "{wgsl}");
        assert!(
            wgsl.contains("polar_filter_dx") && wgsl.contains("polar_filter_dy"),
            "{wgsl}"
        );
        assert!(
            !wgsl.contains("aa_adaptive"),
            "boxes must integrate finite intervals instead of isolated SDF edges: {wgsl}"
        );
    }

    #[test]
    fn polar_box_domain_survives_separable_nested_spaces_and_does_not_leak() {
        let src = r#"canvas t(uv: coord) -> color {
    param progress: f32 = 0.72 in 0 .. 1
    space dial = polar(center: center, from: -90deg, direction: clockwise)
    space move = translate(by: (0.1, 0.0)) . scale(2.0)
    compose {
        in space dial {
            in space move {
                box(at: (progress / 2, 0.32), size: (progress, 0.045)) |> fill(#4facfe)
            }
        }
        box(at: (0.5, 0.5), size: (progress, 0.1)) |> fill(#ffffff)
    }


}"#;
        let wgsl = compile_wgsl(src);
        assert!(wgsl.contains("box_periodic_coverage"), "{wgsl}");
        assert!(
            wgsl.contains("box_x_coverage"),
            "ordinary sibling must retain nonperiodic coverage: {wgsl}"
        );
    }

    #[test]
    fn polar_box_filter_handles_reflection_and_retains_rounded_shape_geometry() {
        for direction in ["clockwise", "counterclockwise"] {
            let src = format!(
                r#"canvas t(uv: coord) -> color {{
    param progress: f32 = 1.0 in 0 .. 1
    space dial = polar(center: center, from: 35deg, direction: {direction}) . scale(-2.0) . orientation(y: down)
    compose {{
        in space dial {{
            box(at: (0.5, 0.32), size: (progress, 0.045)) |> fill(#4facfe)
        }}
    }}
}}"#
            );
            let wgsl = compile_wgsl(&src);
            assert!(wgsl.contains("box_periodic_coverage"), "{wgsl}");
            let rounded = src.replace("|> fill", "|> round(0.01) |> fill");
            let wgsl = compile_wgsl(&rounded);
            assert!(
                !wgsl.contains("box_periodic_coverage"),
                "rounded boxes must retain their geometry: {wgsl}"
            );
            assert!(wgsl.contains("aa_adaptive"), "{wgsl}");
        }
    }

    #[test]
    fn stroked_line_family_uses_analytic_filtered_coverage() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(14deg) . scale(1.0 + uv.y * 2.0)
    let l = lines(along: x, every: 0.07uv, offset: 0.0) |> stroke(2px)
    compose {
        fill(#0b1220)
        in space warped {
            l |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("floor(") && wgsl.contains("min(fract("),
            "expected line-family analytic box-filter terms in WGSL\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("aa_adaptive") && !wgsl.contains("grad_d_x"),
            "expected stroked line-family coverage to bypass derivative AA ramp\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn exact_shape_coverage_uses_directional_distance_band() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(10deg) . scale(1.0 + uv.y * 2.0)
  let ring = circle(at: (0.5, 0.5), radius: 0.18) |> stroke(2px)
  compose {
    fill(#101820)
        in space warped {
            ring |> fill(#ffffff)
        }
  }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("aa_adaptive") && wgsl.contains("dpdx(p_") && wgsl.contains("dpdy(p_"),
            "expected exact-shape AA and authored pixel units in explicit space\nwgsl:\n{wgsl}"
        );
        assert!(
            wgsl.contains("grad_d_x")
                && wgsl.contains("aa_width_gradient")
                && wgsl.contains("aa_directional_px"),
            "expected exact-shape AA bounds to follow the distance gradient\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("max(aa_width_gradient, aa_width_fwidth)"),
            "expected default shape AA policy to avoid conservative widening selection\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("aa_width_fwidth"),
            "expected default shape AA policy to avoid emitting the unused fwidth branch\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn non_exact_shape_coverage_still_uses_sdf_derivative_band() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(14deg) . scale(1.0 + uv.y * 1.8)
    let sample_value = star(at: (0.5, 0.5), outer: 0.22, inner: 0.10, points: 5)
    compose {
        fill(#0b1220)
        in space warped {
            sample_value |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("aa_adaptive") && wgsl.contains("grad_d_x"),
            "expected non-exact shapes to keep derivative-driven adaptive AA markers\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("footprint_j11")
                && !wgsl.contains("footprint_j12")
                && !wgsl.contains("footprint_j21")
                && !wgsl.contains("footprint_j22"),
            "shape AA should not depend on footprint Jacobian symbol names\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn linear_gradient_builtin_dispatches_and_skips_axis_normalization() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(gradient(
            along: y,
            stops: [
                stop(at: 0.0, color: #17345c),
                stop(at: 1.0, color: #5eead4),
            ]
        ))
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            !wgsl.contains("vec2<f32>(0f, 1f)") && !wgsl.contains("length(vec2"),
            "expected axis-aligned linear gradients to avoid generic direction normalization\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn canonical_two_stop_gradient_skips_identity_remap_and_alpha_mix() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(gradient(
            along: y,
            stops: [
                stop(at: 0.0, color: #0b1324),
                stop(at: 1.0, color: #1d2f4f),
            ]
        ))
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            !wgsl.contains("max((1f - 0f), 0.000001f)") && !wgsl.contains("mix(1f, 1f,"),
            "expected canonical two-stop gradients to skip identity remap and alpha mix\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn literal_three_stop_gradient_skips_safe_denominator_clamps() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(gradient(
            along: y,
            stops: [
                stop(at: 0.0, color: #0b1324),
                stop(at: 0.25, color: #17345c),
                stop(at: 0.75, color: #5eead4),
            ]
        ))
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            !wgsl.contains("max((0.25f - 0f), 0.000001f)")
                && !wgsl.contains("max((0.75f - 0.25f), 0.000001f)"),
            "expected literal gradient segments to use folded constant denominators\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn adjacent_equal_literal_stop_colors_skip_redundant_rgb_mix() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(gradient(
            along: y,
            stops: [
                stop(at: 0.0, color: #0b1324),
                stop(at: 0.25, color: #0b1324),
                stop(at: 1.0, color: #5eead4),
            ]
        ))
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            !wgsl.contains(
                "mix(vec3<f32>(0.043137256f, 0.07450981f, 0.14117648f), vec3<f32>(0.043137256f, 0.07450981f, 0.14117648f)"
            ),
            "expected adjacent equal literal stop colors to skip redundant rgb mixes\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn reused_shape_anchor_gradients_share_one_local_point_normalization() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    let card = box(at: (0.5, 0.5), size: (0.4, 0.2))
    compose {
        card |> fill(gradient(
            along: y,
            anchor: shape,
            stops: [
                stop(at: 0.0, color: #0b1324),
                stop(at: 1.0, color: #5eead4),
            ]
        ))
        card |> fill(gradient(
            along: y,
            anchor: shape,
            stops: [
                stop(at: 0.0, color: #0b1324),
                stop(at: 1.0, color: #5eead4),
            ]
        )) |> blend(add)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert_eq!(
            wgsl.matches("vec2(0.5f) + ((p - vec2<f32>(0.5f, 0.5f)) / (max(vec2<f32>(")
                .count(),
            1,
            "expected reused shape-anchor gradients to share one local-point normalization\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn repeated_shape_anchor_gradients_share_time_only_stop_color_math() {
        let src = r#"struct FrameGlobals {
    time: f32
}

param frame: FrameGlobals

fn time() -> f32 { return frame.time }

canvas t(uv: coord, time: signal) -> color {
    let pulse = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)
    compose {
        box(at: (0.3, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                anchor: shape,
                stops: [
                    stop(at: 0.0, color: #0b1324),
                    stop(at: 1.0, color: rgb(pulse, pulse, pulse)),
                ]
            ))
        box(at: (0.7, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                anchor: shape,
                stops: [
                    stop(at: 0.0, color: #0b1324),
                    stop(at: 1.0, color: rgb(pulse, pulse, pulse)),
                ]
            )) |> blend(add)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert_eq!(
            wgsl.matches("6.2831855f * fract((").count(),
            1,
            "expected repeated shape-anchor gradients to share time-only stop color math\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn repeated_shape_anchor_gradients_share_length_based_stop_color_math() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    let pulse = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)
    let duo = length((pulse, pulse))
    compose {
        box(at: (0.3, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                anchor: shape,
                stops: [
                    stop(at: 0.0, color: #0b1324),
                    stop(at: 1.0, color: rgb(duo, duo, duo)),
                ]
            ))
        box(at: (0.7, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                anchor: shape,
                stops: [
                    stop(at: 0.0, color: #0b1324),
                    stop(at: 1.0, color: rgb(duo, duo, duo)),
                ]
            )) |> blend(add)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert_eq!(
            wgsl.matches("length(vec2<f32>(").count(),
            1,
            "expected repeated shape-anchor gradients to share length-based stop color math\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn glow_and_inner_glow_share_shape_color_space_anchor_transform() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    let card = box(at: (0.5, 0.5), size: (0.4, 0.2))
    compose {
        fill(#0e1420)
        card |> glow(
            reach: 0.05,
            strength: 0.7,
            color: gradient(along: y, stops: [
                stop(at: 0.0, color: #0b1324),
                stop(at: 1.0, color: #5eead4),
            ]),
            color_space: shape
        )
        card |> inner_glow(
            reach: 0.05,
            strength: 0.7,
            color: gradient(along: y, stops: [
                stop(at: 0.0, color: #0b1324),
                stop(at: 1.0, color: #5eead4),
            ]),
            color_space: shape
        ) |> blend(add)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert_eq!(
            wgsl.matches("vec2<f32>(0.5f, 0.5f) + (p - vec2<f32>(0.5f, 0.5f))")
                .count(),
            1,
            "expected glow and inner_glow to share the same shape color-space anchor transform\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn glow_and_inner_glow_share_vec2_reach_warp_math() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    let card = box(at: (0.5, 0.5), size: (0.4, 0.2))
    compose {
        fill(#0e1420)
        card |> glow(
            reach: (0.05, 0.08),
            strength: 0.7,
            color: #5eead4,
            color_space: glow
        )
        card |> inner_glow(
            reach: (0.05, 0.08),
            strength: 0.7,
            color: #5eead4,
            color_space: glow
        ) |> blend(add)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert_eq!(
            wgsl.matches("max(abs(0.05f), 0.000001f)").count(),
            1,
            "expected glow and inner_glow to share the vec2 glow-reach warp math\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn repeated_shape_anchor_gradients_share_pure_helper_stop_color_math() {
        let src = r#"struct FrameGlobals {
    time: f32
}

param frame: FrameGlobals

fn time() -> f32 { return frame.time }

fn pulse01(t: f32) -> f32 {
  return sin(t * 0.5) * 0.3 + 0.5
}

canvas t(uv: coord, time: signal) -> color {
    compose {
        box(at: (0.3, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                anchor: shape,
                stops: [
                    stop(at: 0.0, color: #0b1324),
                    stop(at: 1.0, color: rgb(pulse01(time), pulse01(time), pulse01(time))),
                ]
            ))
        box(at: (0.7, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                anchor: shape,
                stops: [
                    stop(at: 0.0, color: #0b1324),
                    stop(at: 1.0, color: rgb(pulse01(time), pulse01(time), pulse01(time))),
                ]
            )) |> blend(add)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert_eq!(
            wgsl.matches("_pulse01_1_(").count(),
            2,
            "expected pure helper stop color math to be lowered once and reused across shape-anchor gradients\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn repeated_gradient_fills_share_one_dither_hash_per_scene_function() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        box(at: (0.3, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                stops: [
                    stop(at: 0.0, color: #17345c),
                    stop(at: 1.0, color: #5eead4),
                ]
            ))

        box(at: (0.7, 0.5), size: (0.2, 0.4))
            |> fill(gradient(
                along: y,
                stops: [
                    stop(at: 0.0, color: #17345c),
                    stop(at: 1.0, color: #5eead4),
                ]
            ))
    }
}
"#;

        let wgsl = compile_wgsl(src);
        let first_scene = wgsl
            .split("fn fresco_t(")
            .next()
            .expect("scene function should precede the entry function");
        assert_eq!(
            first_scene.matches("43758.547f").count(),
            1,
            "expected repeated gradient fills to reuse one dither hash per scene function\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn single_pass_canvas_omits_redundant_pass_entry_wrapper() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(#0e1420)
        circle(at: (0.5, 0.5), radius: 0.2) |> fill(#ffffff)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            !wgsl.contains("fn fresco_t_pass0_("),
            "expected single-pass fused canvases to omit redundant pass wrapper\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("fn fresco_scene_t_pass0_("),
            "expected single-pass canvases to reuse the base scene helper instead of emitting a duplicate pass helper\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn standalone_layer_retains_dynamic_opacity_in_scene_output() {
        let wgsl = compile_wgsl(
            "canvas t(uv: coord, time: signal) -> color { param strength: f32 = 0.5; fill(#ff0000) |> opacity(strength) }",
        );
        let module = naga::front::wgsl::parse_str(&wgsl).expect("valid WGSL");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("valid standalone opacity shader");
        let scene = wgsl.split("fn fresco_t(").next().unwrap();
        assert!(
            scene.contains("strength") && scene.contains("opacity_a_"),
            "standalone opacity must remain live in the scene output: {wgsl}"
        );
    }

    #[test]
    fn top_level_compose_scene_skips_dead_alpha_accumulator_chain() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(#0e1420)
        circle(at: (0.5, 0.5), radius: 0.2) |> fill(#ffffff)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        let first_scene = wgsl
            .split("fn fresco_t(")
            .next()
            .expect("scene function should precede the entry function");
        assert!(
            !first_scene.contains("compose_over_a_")
                && !first_scene.contains("compose_add_a_")
                && !first_scene.contains("compose_screen_a_")
                && !first_scene.contains("compose_mul_a_"),
            "expected opaque top-level scene lowering to skip dead compose alpha accumulators\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn in_space_shape_coverage_uses_local_adaptive_aa() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(12deg) . scale(1.0 + uv.y * 1.5)
    let ring = circle(at: (0.5, 0.5), radius: 0.18) |> stroke(2px)
    compose {
        fill(#101820)
        in space warped {
            ring |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("aa_adaptive"),
            "expected shape coverage in `in space` to use adaptive AA\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn in_space_stroke_px_width_uses_local_pixel_span() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(8deg) . scale(0.25 + uv.y)
    let ring = circle(at: (0.5, 0.5), radius: 0.20) |> stroke(1px)
    compose {
        fill(#0b1220)
        in space warped {
            ring |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("outline_half_w") && wgsl.contains("dpdx(p_") && wgsl.contains("dpdy(p_"),
            "expected px stroke width lowering to use local sample-space derivatives\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn in_space_outline_stroke_emits_thin_fade_guard() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(10deg) . scale(0.12 + uv.y * 0.35)
    let ring = circle(at: (0.5, 0.5), radius: 0.20) |> stroke(1px)
    compose {
        fill(#0b1220)
        in space warped {
            ring |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("outline_locked_w")
                && wgsl.contains("outline_thin_fade")
                && wgsl.contains("outline_cov_thin_fade"),
            "expected thin outline coverage to emit thin-fade stroke guard\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn explain_reports_outline_thin_fade_receipt() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(10deg) . scale(0.12 + uv.y * 0.35)
    let ring = circle(at: (0.5, 0.5), radius: 0.20) |> stroke(1px)
    compose {
        fill(#0b1220)
        in space warped {
            ring |> fill(#ffffff)
        }
    }
}
"#;

        let explain = compile_explain(src);
        assert!(
            explain.contains("stroke aa: thin-fade enabled"),
            "expected explain output to report thin-fade stroke receipt\nexplain:\n{explain}"
        );
    }

    #[test]
    fn soften_emits_mean_coverage_saturation_guard() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(10deg) . scale(0.12 + uv.y * 0.35)
    let ring = circle(at: (0.5, 0.5), radius: 0.20)
    compose {
        fill(#0b1220)
        in space warped {
            ring |> soften(radius: 1px, color: #ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("soft_mean_cap") && wgsl.contains("soft_cov_saturated"),
            "expected soften coverage to emit mean-coverage saturation guard\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn shadow_emits_mean_coverage_saturation_guard() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(10deg) . scale(0.12 + uv.y * 0.35)
    let ring = circle(at: (0.5, 0.5), radius: 0.20)
    compose {
        fill(#0b1220)
        in space warped {
            ring |> shadow(offset: (1px, 1px), soften: 1px, color: #00000088)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("soft_mean_cap") && wgsl.contains("soft_cov_saturated"),
            "expected shadow soften path to emit mean-coverage saturation guard\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn glow_falloff_path_does_not_emit_soft_saturation_markers() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    let core = circle(at: center, radius: 0.09) |> fill(#ffffff)
    let haze = core |> glow(
        reach: (0.18, 0.08),
        strength: 0.65,
        color: gradient(
            along: y,
            stops: [
                stop(at: 0.0, color: #ff8bd6),
                stop(at: 1.0, color: #7dd3fc),
            ]
        ),
        falloff: linear
    )

    compose {
        fill(#070b15)
        haze |> blend(add)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("glow_falloff") || wgsl.contains("layer_glow_scalar"),
            "expected glow lowering to use its falloff-based path\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("soft_mean_cap") && !wgsl.contains("soft_cov_saturated"),
            "glow should not route through soft-coverage saturation markers\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn in_space_line_thickness_px_uses_local_pixel_span() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(12deg) . scale(0.15 + uv.y * 0.6)
    let l = line(from: (0.1, 0.8), to: (0.9, 0.2), thickness: 1px)
    compose {
        fill(#101820)
        in space warped {
            l |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("dpdx(p_") && wgsl.contains("dpdy(p_"),
            "expected px thickness lowering for line/capsule to use local sample-space derivatives\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn shape_aa_pragma_tunes_min_and_max_band() {
        let src = r#"#pragma check.shape_aa_min_px = 0.75
    #pragma check.shape_aa_max_px = 5.5

canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(18deg) . scale(1.0 + uv.y * 2.2)
    let ring = circle(at: (0.5, 0.5), radius: 0.2) |> stroke(2px)
    compose {
        fill(#111827)
        in space warped {
            ring |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("0.75f") && wgsl.contains("dpdx(p_") && wgsl.contains("dpdy(p_"),
            "expected pragma-tuned shape AA to preserve the lower clamp bound and derivative-backed width path in emitted WGSL\nwgsl:\n{wgsl}"
        );
        assert!(
            wgsl.contains("5.5f") && wgsl.contains("aa_directional_px"),
            "expected the upper band to clamp the directional footprint\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn shape_coverage_band_is_derivative_only_not_footprint_symbols() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    space warped = rotate(14deg) . scale(1.0 + uv.y * 1.8)
    let sample_value = star(at: (0.5, 0.5), outer: 0.22, inner: 0.10, points: 5)
    compose {
        fill(#0b1220)
        in space warped {
            sample_value |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("aa_adaptive") && wgsl.contains("grad_d_x"),
            "expected derivative-driven adaptive AA markers\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("footprint_j11")
                && !wgsl.contains("footprint_j12")
                && !wgsl.contains("footprint_j21")
                && !wgsl.contains("footprint_j22"),
            "shape AA should not depend on footprint Jacobian symbol names\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn repeat_zero_period_is_guarded_by_safe_every_clamps() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    compose {
        fill(#101820)
        in space repeat(every: (0.0000001, 0.0000001)) {
            circle(at: (0.5, 0.5), radius: 0.2) |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("space_repeat_x_safe_every")
                && wgsl.contains("space_repeat_y_safe_every"),
            "expected repeat lowering to keep non-zero safe period guards\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn perspective_rotate_uses_signed_denominator_guard() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    let flip = time * 90deg
    compose {
        fill(#101820)
        in space centered(aspect: preserve)
            .perspective(fov: 58deg, near: 0.02, far: 12.0, origin: center)
            .rotate_y(angle: flip, around: center) {
            box(at: center, size: (0.5, 0.6)) |> fill(#ffffff)
        }
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            wgsl.contains("sign(")
                && wgsl.contains("select(")
                && wgsl.contains(" > max(0.000001f")
                && wgsl.contains("space_projective_guard_span_t")
                && !wgsl.contains("* 0.001f"),
            "expected perspective rotate lowering to preserve sign and cull non-visible horizon samples using footprint-based guard span\nwgsl:\n{wgsl}"
        );
        assert!(
            wgsl.contains("64f * px"),
            "expected local pixel span cap to be emitted for perspective numeric stability\nwgsl:\n{wgsl}"
        );
    }

    #[test]
    fn canvas_space_statement_is_rejected() {
        let src = r#"canvas t(uv: coord, time: signal) -> color {
    canvas_space = rotate(45deg) . scale(0.8)
    let frame = box(at: (0.5, 0.5), size: (0.7, 0.5)) |> stroke(2px)
    compose {
        fill(#101820)
        frame |> fill(#ffffff)
    }
}
"#;

        let out = compile_source(src, "canvas_space_removed.fr", "wgsl", false);
        assert!(out.is_err(), "expected `canvas_space` to be rejected");
    }
}
