#![allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]

use naga::{BinaryOperator as Bo, Expression as Ex, Handle, Span};

use super::uniforms;

const SP: Span = Span::UNDEFINED;

pub(super) struct BridgeHandles {
    pub function: Handle<naga::Function>,
    pub uv_arg: Handle<Ex>,
    pub time_arg: Handle<Ex>,
    pub dt_arg: Handle<Ex>,
    pub res_arg: Handle<Ex>,
}

pub(super) fn append_bridge_function(
    module: &mut naga::Module,
    user_fn: Handle<naga::Function>,
    user_param_types: &[String],
    has_delta: bool,
    common: &uniforms::CommonHandles,
) -> BridgeHandles {
    let mut function = naga::Function {
        name: Some("__fresco_user_expr".to_string()),
        ..Default::default()
    };

    function.arguments.push(naga::FunctionArgument {
        name: Some("uv".to_string()),
        ty: common.v2_ty,
        binding: None,
    });
    let uv_arg = function.expressions.append(Ex::FunctionArgument(0), SP);

    function.arguments.push(naga::FunctionArgument {
        name: Some("time".to_string()),
        ty: common.f32_ty,
        binding: None,
    });
    let time_arg = function.expressions.append(Ex::FunctionArgument(1), SP);

    function.arguments.push(naga::FunctionArgument {
        name: Some("dt".to_string()),
        ty: common.f32_ty,
        binding: None,
    });
    let dt_arg = function.expressions.append(Ex::FunctionArgument(2), SP);

    function.arguments.push(naga::FunctionArgument {
        name: Some("res".to_string()),
        ty: common.v2_ty,
        binding: None,
    });
    let res_arg = function.expressions.append(Ex::FunctionArgument(3), SP);

    function.result = Some(naga::FunctionResult {
        ty: common.v4_ty,
        binding: None,
    });

    let mut call_args = vec![uv_arg, time_arg];
    if has_delta {
        call_args.push(dt_arg);
    }
    call_args.push(res_arg);

    let mut slot = 0usize;
    for ty in user_param_types {
        let normalized = ty
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        match normalized.as_str() {
            "vec4<f32>" | "color" => {
                if slot + 3 >= 16 {
                    break;
                }
                let c0 = uniforms::read_param_scalar(&mut function, common.uniform_global, slot);
                let c1 =
                    uniforms::read_param_scalar(&mut function, common.uniform_global, slot + 1);
                let c2 =
                    uniforms::read_param_scalar(&mut function, common.uniform_global, slot + 2);
                let c3 =
                    uniforms::read_param_scalar(&mut function, common.uniform_global, slot + 3);
                let value = uniforms::vec4(&mut function, common.v4_ty, c0, c1, c2, c3);
                call_args.push(value);
                slot += 4;
            }
            "bool" => {
                let scalar =
                    uniforms::read_param_scalar(&mut function, common.uniform_global, slot);
                let threshold = uniforms::lit_f32(&mut function, 0.5);
                let value = function.expressions.append(
                    Ex::Binary {
                        op: Bo::GreaterEqual,
                        left: scalar,
                        right: threshold,
                    },
                    SP,
                );
                call_args.push(value);
                slot += 1;
            }
            "u32" => {
                let scalar =
                    uniforms::read_param_scalar(&mut function, common.uniform_global, slot);
                let value = function.expressions.append(
                    Ex::As {
                        expr: scalar,
                        kind: naga::ScalarKind::Uint,
                        convert: Some(4),
                    },
                    SP,
                );
                call_args.push(value);
                slot += 1;
            }
            "i32" => {
                let scalar =
                    uniforms::read_param_scalar(&mut function, common.uniform_global, slot);
                let value = function.expressions.append(
                    Ex::As {
                        expr: scalar,
                        kind: naga::ScalarKind::Sint,
                        convert: Some(4),
                    },
                    SP,
                );
                call_args.push(value);
                slot += 1;
            }
            _ => {
                let scalar =
                    uniforms::read_param_scalar(&mut function, common.uniform_global, slot);
                call_args.push(scalar);
                slot += 1;
            }
        }
    }

    let call_result = function.expressions.append(Ex::CallResult(user_fn), SP);
    function.body.push(
        naga::Statement::Call {
            function: user_fn,
            arguments: call_args,
            result: Some(call_result),
        },
        SP,
    );

    let ret_value = call_result;
    function.body.push(
        naga::Statement::Return {
            value: Some(ret_value),
        },
        SP,
    );

    let handle = module.functions.append(function, SP);
    BridgeHandles {
        function: handle,
        uv_arg,
        time_arg,
        dt_arg,
        res_arg,
    }
}

pub(super) fn call_bridge(
    function: &mut naga::Function,
    bridge_fn: Handle<naga::Function>,
    uv: Handle<Ex>,
    time: Handle<Ex>,
    dt: Handle<Ex>,
    res: Handle<Ex>,
) -> Handle<Ex> {
    let result = function.expressions.append(Ex::CallResult(bridge_fn), SP);
    function.body.push(
        naga::Statement::Call {
            function: bridge_fn,
            arguments: vec![uv, time, dt, res],
            result: Some(result),
        },
        SP,
    );
    result
}
