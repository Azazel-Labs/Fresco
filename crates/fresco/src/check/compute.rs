//! Pure compute hooks use the ordinary function checker and scalar/vector HIR.
use super::*;
use crate::ast::{PassDecl, PassFnHookDecl, Program};

pub struct ComputeLibrary {
    program: Program,
    options: CheckOptions,
}

pub(crate) struct ComputeHook {
    pub hir: Hir,
    /// Scalar ABI arguments, with their authored wrapper access expressions.
    pub inputs: Vec<(String, String)>,
    /// Components of each output field, in declaration order.
    pub outputs: Vec<Vec<Sx>>,
}

impl ComputeLibrary {
    pub(crate) fn functions(&self) -> &[FnDecl] {
        &self.program.functions
    }
    pub fn new(program: &Program, options: CheckOptions) -> Self {
        Self {
            program: program.clone(),
            options,
        }
    }

    pub(crate) fn check_hook(
        &self,
        pass: &PassDecl,
        hook: &PassFnHookDecl,
        state: &StructDecl,
    ) -> Result<ComputeHook, String> {
        let mut functions = self.program.functions.clone();
        // Use ordinary lexical local functions: hooks can call one another without
        // exporting their names into the imported module namespace.
        let mut locals = Vec::new();
        for local in &pass.hooks {
            locals.push(FnDecl {
                typed_body: None,
                name: local.name.clone(),
                name_span: local.name_span.clone(),
                docs: None,
                type_params: Vec::new(),
                const_params: Vec::new(),
                params: local
                    .params
                    .iter()
                    .map(|p| FnParam {
                        is_context: false,
                        name: p.name.clone(),
                        name_span: p.name_span.clone(),
                        ty_name: p.ty_name.clone(),
                        ty_span: p.ty_span.clone(),
                        keyword_only: false,
                        default: None,
                    })
                    .collect(),
                ret_ty: local
                    .return_ty
                    .as_ref()
                    .map(|t| (t.node.clone(), t.span.clone())),
                is_internal: false,
                derivative_free: false,
                is_builtin: false,
                source_file: pass.source_file.clone(),
                body: crate::parser::pass_hook_body(local).map_err(|errors| {
                    format!("invalid compute function `{}`: {errors:?}", local.name)
                })?,
                span: local.span.clone(),
            });
        }
        let mut entry = locals
            .iter()
            .find(|f| f.name == hook.name)
            .ok_or("selected compute hook is not part of its pass")?
            .clone();
        entry.name = "fresco_compute_entry".to_string();
        entry.body.clone_from(&pass.entry_bindings);
        entry.body.extend(locals.into_iter().map(Stmt::LocalFnDecl));
        entry.body.push(Stmt::Return {
            value: Spanned {
                node: Expr::Call {
                    name: hook.name.clone(),
                    name_span: hook.name_span.clone(),
                    const_args: Vec::new(),
                    args: hook
                        .params
                        .iter()
                        .map(|p| Arg {
                            name: None,
                            value: Spanned {
                                node: Expr::Var(p.name.clone()),
                                span: p.name_span.clone(),
                            },
                        })
                        .collect(),
                },
                span: hook.span.clone(),
            },
            span: hook.span.clone(),
        });
        functions.push(entry);
        let p = &self.program;
        let (mut c, prep) = declarations::create_checker_for_surface(
            format!("compute_{}", hook.name),
            &functions,
            &p.consts,
            &p.enums,
            &p.structs,
            &[],
            &[],
            &p.interfaces,
            &p.conformances,
            &[],
            &self.options,
        );
        c.diags.extend(prep);
        // Inline through the shared typed evaluator; compute does not build a second
        // helper-expression emitter or inherit raster-only resources.
        c.emit_user_helper_calls = false;
        let mut inputs = Vec::new();
        let mut args = Vec::new();
        for param in &hook.params {
            let value = input_value(&param.ty_name, &param.name, &p.structs, &mut inputs, 0)?;
            c.bind(param.name.clone(), value);
            args.push(Arg {
                name: None,
                value: Spanned {
                    node: Expr::Var(param.name.clone()),
                    span: param.name_span.clone(),
                },
            });
        }
        let result = c.eval_user_fn_call(
            "fresco_compute_entry",
            &hook.name_span,
            &hook.span,
            &[],
            &args,
        );
        if c.diags.iter().any(|d| d.severity == Severity::Error) {
            return Err(c
                .diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; "));
        }
        let Some(Value::Struct {
            ty_name,
            mut fields,
        }) = result
        else {
            return Err(format!(
                "compute hook `{}` must return initialized `{}` state",
                hook.name, state.name
            ));
        };
        if ty_name != state.name {
            return Err(format!(
                "compute hook `{}` returned `{ty_name}`, expected `{}`",
                hook.name, state.name
            ));
        }
        let mut outputs = Vec::new();
        for field in &state.fields {
            let value = fields.remove(&field.name).ok_or_else(|| {
                format!("compute state field `{}` is not initialized", field.name)
            })?;
            let components = components(value)?;
            for sx in &components {
                validate_compute_expression(sx)?;
            }
            outputs.push(components);
        }
        Ok(ComputeHook {
            hir: c.hir,
            inputs,
            outputs,
        })
    }
}

fn input_value(
    ty: &str,
    access: &str,
    structs: &[StructDecl],
    inputs: &mut Vec<(String, String)>,
    depth: usize,
) -> Result<Value, String> {
    if depth > structs.len() {
        return Err(format!("recursive compute state type `{ty}`"));
    }
    let mut scalar = |access: String| {
        let name = format!("fresco_arg_{}", inputs.len());
        inputs.push((name.clone(), access));
        Sx::Var(name)
    };
    Ok(match strip_spatial_type_suffix(ty) {
        "f32" | "float" | "signal" => Value::Scalar(scalar(access.into())),
        "vec2" => Value::Vec2((scalar(format!("{access}.x")), scalar(format!("{access}.y")))),
        "vec3" => Value::Vec3((
            scalar(format!("{access}.x")),
            scalar(format!("{access}.y")),
            scalar(format!("{access}.z")),
        )),
        "vec4" => Value::Vec4((
            scalar(format!("{access}.x")),
            scalar(format!("{access}.y")),
            scalar(format!("{access}.z")),
            scalar(format!("{access}.w")),
        )),
        other => {
            let st = structs.iter().find(|s| s.name == other)
                .ok_or_else(|| format!("typed compute input `{access}` uses unsupported type `{ty}`; use f32 or float vectors"))?;
            let mut fields = HashMap::new();
            for f in &st.fields {
                fields.insert(
                    f.name.clone(),
                    input_value(
                        &f.ty_name,
                        &format!("{access}.{}", f.name),
                        structs,
                        inputs,
                        depth + 1,
                    )?,
                );
            }
            Value::Struct {
                ty_name: st.name.clone(),
                fields,
            }
        }
    })
}

fn components(value: Value) -> Result<Vec<Sx>, String> {
    match value {
        Value::Scalar(s) | Value::Distance(s) | Value::Coverage(s) | Value::Mask(s) => Ok(vec![s]),
        Value::Vec2((x, y)) => Ok(vec![x, y]),
        Value::Vec3((x, y, z)) => Ok(vec![x, y, z]),
        Value::Vec4((x, y, z, w)) => Ok(vec![x, y, z, w]),
        Value::ColorField { rgba, .. } => Ok(rgba.to_vec()),
        Value::Color { rgba, .. } => Ok(rgba.into_iter().map(Sx::Lit).collect()),
        other => Err(format!(
            "unsupported compute output field type {}",
            other.kind()
        )),
    }
}

fn validate_compute_expression(expression: &Sx) -> Result<(), String> {
    let mut unavailable = None;
    expression.walk_preorder(&mut |sx| {
        let capability = match sx {
            Sx::Typed(_) => None,
            Sx::Ddx(_)
            | Sx::Ddy(_)
            | Sx::Fwidth(_)
            | Sx::PxLit(_)
            | Sx::FootprintJ11
            | Sx::FootprintJ12
            | Sx::FootprintJ21
            | Sx::FootprintJ22 => Some("screen-space derivatives/footprints"),
            Sx::CellQuery { .. }
            | Sx::CellContour { .. }
            | Sx::GradientChannel { .. }
            | Sx::CoordX
            | Sx::CoordY => Some("raster coordinates"),
            Sx::EntryInput(_)
            | Sx::UniformField { .. }
            | Sx::TexChannel { .. }
            | Sx::Param(_)
            | Sx::DynamicArrayIndex { .. }
            | Sx::EffectInputChannel { .. }
            | Sx::PostColorR
            | Sx::PostColorG
            | Sx::PostColorB
            | Sx::PostColorA => Some("implicit raster resources"),
            Sx::ScatterInstanceId
            | Sx::ScatterInstanceIndex01
            | Sx::ScatterInstanceAgeNorm
            | Sx::ScatterInstancePosX
            | Sx::ScatterInstancePosY
            | Sx::RepeatCellIdX(_)
            | Sx::RepeatCellIdY(_)
            | Sx::RepeatCellCenterX(_)
            | Sx::RepeatCellCenterY(_)
            | Sx::RepeatCellUvX(_)
            | Sx::RepeatCellUvY(_)
            | Sx::RepeatCellRand(_) => Some("raster instance/cell context"),
            Sx::PathDist { .. }
            | Sx::PathAlong { .. }
            | Sx::PathTangentComponent { .. }
            | Sx::PathPointAtComponent { .. }
            | Sx::PathTangentAtComponent { .. } => Some("path resources"),
            Sx::UserCall { .. } => Some("non-inlined runtime helper calls"),
            Sx::Lit(_)
            | Sx::Var(_)
            | Sx::Let { .. }
            | Sx::Neg(_)
            | Sx::Add(..)
            | Sx::Sub(..)
            | Sx::Mul(..)
            | Sx::Div(..)
            | Sx::Lt(..)
            | Sx::Le(..)
            | Sx::Gt(..)
            | Sx::Ge(..)
            | Sx::Eq(..)
            | Sx::Ne(..)
            | Sx::Sin(_)
            | Sx::Cos(_)
            | Sx::Tan(_)
            | Sx::Asin(_)
            | Sx::Acos(_)
            | Sx::Atan(_)
            | Sx::Sqrt(_)
            | Sx::InverseSqrt(_)
            | Sx::Fract(_)
            | Sx::Abs(_)
            | Sx::Sign(_)
            | Sx::Floor(_)
            | Sx::Ceil(_)
            | Sx::Round(_)
            | Sx::Trunc(_)
            | Sx::Exp(_)
            | Sx::Exp2(_)
            | Sx::Log(_)
            | Sx::Log2(_)
            | Sx::Atan2(..)
            | Sx::Pow(..)
            | Sx::Min(..)
            | Sx::Max(..)
            | Sx::Step(..)
            | Sx::Dot { .. }
            | Sx::NormalizeComponent { .. }
            | Sx::MinComponent { .. }
            | Sx::MaxComponent { .. }
            | Sx::ClampVecComponent { .. }
            | Sx::Length(_)
            | Sx::Clamp(..)
            | Sx::Mix(..)
            | Sx::Select(..)
            | Sx::SmoothStep(..)
            | Sx::SrgbToLinear(_)
            | Sx::LinearToSrgb(_) => None,
        };
        if capability.is_some() {
            unavailable = capability;
        }
    });
    match unavailable {
        Some(capability) => Err(format!(
            "compute hooks cannot use {capability}; pass explicit simulation inputs"
        )),
        None => Ok(()),
    }
}

/// Use the ordinary typed constant evaluator for engine resource expressions.
fn constant_value(
    program: &Program,
    bindings: &[Stmt],
    expression: &SExpr,
) -> Result<Value, Vec<Diag>> {
    let options = CheckOptions::default();
    let (mut checker, preparation) = declarations::create_checker_for_surface(
        "resource_policy".into(),
        &program.functions,
        &program.consts,
        &program.enums,
        &program.structs,
        &[],
        &[],
        &program.interfaces,
        &program.conformances,
        &[],
        &options,
    );
    checker.diags.extend(preparation);
    checker.emit_user_helper_calls = false;
    let mut body = bindings.to_vec();
    body.push(Stmt::Return {
        value: expression.clone(),
        span: expression.span.clone(),
    });
    let result = checker.eval_standalone_stmt_block("resource_policy", "", &expression.span, &body);
    let folded = result.and_then(|value| {
        checker.eval_compile_time_const_value(&value, &expression.span, "resource policy")
    });
    if checker
        .diags
        .iter()
        .any(|diag| diag.severity == Severity::Error)
    {
        return Err(checker.diags);
    }
    folded.ok_or_else(|| {
        vec![Diag::error(
            expression.span.clone(),
            "resource policy requires a compile-time value",
        )]
    })
}

pub(crate) fn constant_binding_type(
    program: &Program,
    bindings: &[Stmt],
    expression: &SExpr,
) -> Result<String, Vec<Diag>> {
    let value = constant_value(program, bindings, expression)?;
    if matches!(value, Value::Vec2(_) | Value::Vec3(_) | Value::Vec4(_)) {
        let prefix = match Checker::value_element_kind(&value).expect("vector element kind") {
            crate::typed_scalar::Kind::F32 => "",
            crate::typed_scalar::Kind::I32 => "i",
            crate::typed_scalar::Kind::U32 => "u",
            crate::typed_scalar::Kind::Bool => "b",
        };
        return Ok(format!("{prefix}{}", value.kind()));
    }
    let ty = match &value {
        Value::Scalar(Sx::Typed(value)) => value.kind.name(),
        Value::Scalar(_) => "f32",
        Value::Color { .. } | Value::ColorField { .. } => value.kind(),
        _ => {
            return Err(vec![Diag::error(
                expression.span.clone(),
                "graph bindings require a compile-time scalar, vector, or color",
            )]);
        }
    };
    Ok(ty.into())
}

pub(crate) fn constant_number(
    program: &Program,
    bindings: &[Stmt],
    expression: &SExpr,
) -> Result<f64, Vec<Diag>> {
    let scalar = match Some(constant_value(program, bindings, expression)?) {
        Some(Value::Scalar(Sx::Typed(value))) => match value.evaluate(&HashMap::new()) {
            Ok(naga::Literal::U32(v)) => Some(f64::from(v)),
            Ok(naga::Literal::I32(v)) => Some(f64::from(v)),
            Ok(naga::Literal::F32(v)) => Some(f64::from(v)),
            Ok(naga::Literal::Bool(v)) => Some(if v { 1.0 } else { 0.0 }),
            _ => None,
        },
        Some(Value::Scalar(value)) => Checker::try_eval_static_scalar(&value).map(f64::from),
        _ => None,
    };
    match scalar {
        Some(value) if value.is_finite() => Ok(value),
        _ => Err(vec![Diag::error(
            expression.span.clone(),
            "resource policy requires a finite compile-time scalar",
        )]),
    }
}

pub(crate) fn constant_scalar(
    program: &Program,
    bindings: &[Stmt],
    expression: &SExpr,
) -> Result<f32, Vec<Diag>> {
    constant_number(program, bindings, expression).map(|value| value as f32)
}

/// Check a precondition as a boolean, rather than accepting numeric truthiness.
pub(crate) fn constant_bool(
    program: &Program,
    bindings: &[Stmt],
    expression: &SExpr,
) -> Result<bool, Vec<Diag>> {
    let mut typed = bindings.to_vec();
    typed.push(Stmt::Const {
        name: "__style_precondition".into(),
        name_span: expression.span.clone(),
        ty_name: "bool".into(),
        ty_span: expression.span.clone(),
        value: expression.clone(),
    });
    constant_number(
        program,
        &typed,
        &Spanned {
            node: Expr::Var("__style_precondition".into()),
            span: expression.span.clone(),
        },
    )
    .map(|value| value == 1.0)
}

/// Typed constant defaults for runtime style inputs, evaluated by the ordinary checker.
pub(crate) fn style_parameter_value(
    program: &Program,
    parameter: &GlobalParamDecl,
    expression: &SExpr,
) -> Result<serde_json::Value, Vec<Diag>> {
    let (mut checker, preparation) = declarations::create_checker_for_surface(
        "style_parameter".into(),
        &program.functions,
        &program.consts,
        &program.enums,
        &program.structs,
        &[],
        &[],
        &program.interfaces,
        &[],
        &[],
        &CheckOptions::default(),
    );
    checker.diags.extend(preparation);
    checker.emit_user_helper_calls = false;
    let body = vec![
        Stmt::Const {
            name: parameter.name.clone(),
            name_span: expression.span.clone(),
            ty_name: parameter.ty_name.clone(),
            ty_span: expression.span.clone(),
            value: expression.clone(),
        },
        Stmt::Return {
            value: Spanned {
                node: Expr::Var(parameter.name.clone()),
                span: expression.span.clone(),
            },
            span: expression.span.clone(),
        },
    ];
    let result = checker.eval_standalone_stmt_block("style parameter", "", &expression.span, &body);
    let folded = result.and_then(|v| {
        checker.eval_compile_time_const_value(&v, &expression.span, "style parameter")
    });
    if checker.diags.iter().any(|d| d.severity == Severity::Error) {
        return Err(checker.diags);
    }
    if matches!(parameter.ty_name.as_str(), "u32" | "i32" | "bool") {
        let literal = match &folded {
            Some(Value::Scalar(Sx::Typed(value))) => value.evaluate(&HashMap::new()).ok(),
            _ => None,
        };
        return match (parameter.ty_name.as_str(), literal) {
            ("u32", Some(naga::Literal::U32(v))) => Ok(serde_json::json!(v)),
            ("i32", Some(naga::Literal::I32(v))) => Ok(serde_json::json!(v)),
            ("bool", Some(naga::Literal::Bool(v))) => Ok(serde_json::json!(v)),
            _ => Err(vec![Diag::error(
                expression.span.clone(),
                format!("style setting requires a constant {}", parameter.ty_name),
            )]),
        };
    }
    let values = match folded {
        Some(Value::Scalar(v)) => vec![v],
        Some(Value::Vec2((x, y))) => vec![x, y],
        Some(Value::Vec3((x, y, z))) => vec![x, y, z],
        Some(Value::Vec4((x, y, z, w))) => vec![x, y, z, w],
        Some(Value::Color { rgba, .. }) => return Ok(serde_json::json!(rgba)),
        Some(Value::ColorField { rgba, .. }) => rgba.into(),
        _ => {
            return Err(vec![Diag::error(
                expression.span.clone(),
                "style parameter requires a constant scalar, vector, or color",
            )]);
        }
    };
    let values: Option<Vec<f32>> = values.iter().map(Checker::try_eval_static_scalar).collect();
    let Some(values) = values.filter(|vs| vs.iter().all(|v| v.is_finite())) else {
        return Err(vec![Diag::error(
            expression.span.clone(),
            "style parameter requires finite constant components",
        )]);
    };
    Ok(match parameter.ty_name.as_str() {
        "f32" => serde_json::json!(values[0]),
        _ => serde_json::json!(values),
    })
}
