//! Raster entry wrappers: the `fresco_<name>` (and per-pass
//! `fresco_<name>_pass<N>_`) functions that build the shared scene context
//! from the entry ABI and call the lowered scene function, including
//! temporal motion-blur resampling.
use super::*;

fn lower_raster_entry_fn_named(
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

    let ir = IrBuilder::new(
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
    );
    let mut cx = FnCtx::new(hir, ir);

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

pub(super) fn lower_raster_entry_fn(
    hir: &Hir,
    t: &TypeHandles,
    policy: LoweringPolicy,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    scene_fn: Handle<naga::Function>,
) -> naga::Function {
    lower_raster_entry_fn_named(
        hir,
        &format!("fresco_{}", hir.name),
        t,
        policy,
        tex_globals,
        sampler_global,
        scene_fn,
    )
}

pub(super) fn lower_raster_pass_entry_fn(
    hir: &Hir,
    pass_id: usize,
    t: &TypeHandles,
    policy: LoweringPolicy,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    scene_fn: Handle<naga::Function>,
) -> naga::Function {
    lower_raster_entry_fn_named(
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

    fn compile_wgsl(src: &str) -> String {
        let out = compile_source(src, "raster_entry.fr", "wgsl", false).unwrap_or_else(|diags| {
            panic!("expected compile success, got diagnostics: {diags:#?}")
        });
        out.emitted
    }

    #[test]
    fn single_pass_raster_entry_omits_redundant_pass_entry_wrapper() {
        let src = r#"canvas t(ctx: CanvasContext) -> color { let uv = context(coord); let time = context(time);
    compose {
        fill(#0e1420)
        circle(at: (0.5, 0.5), radius: 0.2) |> fill(#ffffff)
    }
}
"#;

        let wgsl = compile_wgsl(src);
        assert!(
            !wgsl.contains("fn fresco_t_pass0_("),
            "expected single-pass fused raster entries to omit redundant pass wrapper\nwgsl:\n{wgsl}"
        );
        assert!(
            !wgsl.contains("fn fresco_scene_t_pass0_("),
            "expected single-pass raster entries to reuse the base scene helper instead of emitting a duplicate pass helper\nwgsl:\n{wgsl}"
        );
    }
}
