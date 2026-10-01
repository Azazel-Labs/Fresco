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
    let ir = IrBuilder::from_parts(
        function,
        *t,
        SampleInputs {
            uv,
            time: zero,
            delta: zero,
            res: uv,
            px: one,
            aa: one,
        },
    );
    let mut cx = FnCtx {
        var_overrides,
        ..FnCtx::new(&hook.hir, ir)
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
