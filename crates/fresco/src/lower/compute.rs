//! Pure compute values lower through the same scalar/vector IR as surface functions.
use super::*;
use crate::check::compute::ComputeHook;

pub(crate) fn hooks_wgsl(
    state_declaration: &str,
    state_name: &str,
    context_name: &str,
    hooks: &[(&str, &ComputeHook)],
) -> Result<String, String> {
    let mut builder = ModuleBuilder::new();
    builder.module = naga::front::wgsl::parse_str(state_declaration)
        .map_err(|e| format!("invalid compute state: {e}"))?;
    let state_ty = builder
        .module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some(state_name))
        .map(|(handle, _)| handle)
        .ok_or("missing compute state type")?;
    let t = builder.register_core_types_named(context_name);
    for (name, hook) in hooks {
        builder.push_function(lower_hook(name, hook, state_ty, &t));
    }
    let module = builder.finish();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("invalid typed compute IR: {e:?}"))?;
    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| format!("compute WGSL emission failed: {e}"))
}

fn lower_hook(
    name: &str,
    hook: &ComputeHook,
    state_ty: Handle<naga::Type>,
    t: &TypeHandles,
) -> naga::Function {
    let mut function = FunctionBuilder::new(name.to_string(), LoweringPolicy::from_env());
    let mut var_overrides = Vec::new();
    for (name, _) in &hook.inputs {
        let argument = function.arg(name, t.f32_);
        var_overrides.push((name.clone(), argument));
    }
    function.set_result(state_ty);
    let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
    let one = function.expr(Ex::Literal(naga::Literal::F32(1.0)));
    let uv = function.expr(Ex::Compose {
        ty: t.v2,
        components: vec![zero, zero],
    });
    let mut cx = FnCtx {
        hir: &hook.hir,
        ir: IrBuilder {
            function,
            types: *t,
            uv,
            time: zero,
            time_override: None,
            delta: zero,
            res: uv,
            px: one,
            aa: one,
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
        },
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
        var_overrides,
        stats: Stats::default(),
    };

    let mut fields = Vec::new();
    for output in &hook.outputs {
        let components: Vec<_> = output.iter().map(|sx| cx.sx_at(sx, uv)).collect();
        let field = match components.len() {
            1 => components[0],
            2 => cx.ir.add(Ex::Compose {
                ty: t.v2,
                components,
            }),
            3 => cx.ir.add(Ex::Compose {
                ty: t.v3,
                components,
            }),
            4 => cx.ir.add(Ex::Compose {
                ty: t.v4,
                components,
            }),
            _ => unreachable!("checker emits scalar or vector fields"),
        };
        fields.push(field);
    }
    let result = cx.ir.add(Ex::Compose {
        ty: state_ty,
        components: fields,
    });
    cx.ir.return_value(result);
    cx.ir.finish()
}
