//! Specialization and WGSL emission for an engine-authored fullscreen pass.

use std::collections::{HashMap, HashSet};

use crate::ast::{BinOp, Expr, PassDecl, PipelineDecl, SExpr, Stmt, UnOp};
use crate::hir::Hir;

#[derive(Debug, Clone)]
pub(super) struct FullscreenVariant {
    pub key: String,
    pub bindings: Vec<(String, String)>,
    pub vertex_entry: String,
    pub fragment_entry: String,
}

#[derive(Debug, Clone)]
pub(super) struct FullscreenStage {
    pub pipeline: String,
    pub pass: String,
    pub interface: String,
    pub vertex_entry: String,
    pub fragment_entry: String,
    pub variants: Vec<FullscreenVariant>,
    pub wgsl: String,
    pub validation_wgsl: String,
}

pub(super) fn for_canvas(
    hir: &Hir,
    passes: &[PassDecl],
    pipelines: &[PipelineDecl],
) -> Result<Option<FullscreenStage>, String> {
    let Some(entry) = &hir.entry_context else {
        return Ok(None);
    };
    let Some((pipeline, pass)) = pipelines.iter().find_map(|pipeline| {
        let interface = pipeline.material_name.as_ref()?;
        if interface != &entry.interface {
            return None;
        }
        pipeline.passes.iter().find_map(|pass_ref| {
            let pass = passes.iter().find(|pass| {
                pass.name == pass_ref.node
                    && pass.material_name.as_ref() == Some(interface)
                    && pass
                        .stage
                        .as_ref()
                        .is_some_and(|stage| stage.node == "raster")
                    && pass
                        .draw
                        .as_ref()
                        .is_some_and(|draw| draw.node == "fullscreen")
                    && !pass.hooks.is_empty()
            })?;
            Some((pipeline, pass))
        })
    }) else {
        return Ok(None);
    };

    let vertex = pass
        .hooks
        .iter()
        .find(|hook| hook.name == "vertex")
        .ok_or_else(|| format!("fullscreen pass `{}` requires a `vertex` hook", pass.name))?;
    let shade = pass
        .hooks
        .iter()
        .find(|hook| hook.name == "shade")
        .ok_or_else(|| format!("fullscreen pass `{}` requires a `shade` hook", pass.name))?;
    let vertex_body = crate::parser::pass_hook_body(vertex)
        .map_err(|errors| format!("invalid vertex hook in `{}`: {errors:?}", pass.name))?;
    let shade_body = crate::parser::pass_hook_body(shade)
        .map_err(|errors| format!("invalid shade hook in `{}`: {errors:?}", pass.name))?;

    let vertex_param = vertex.params.first().ok_or_else(|| {
        format!(
            "fullscreen pass `{}` vertex hook requires `vertex_id: u32`",
            pass.name
        )
    })?;
    if vertex.params.len() != 1 || vertex_param.ty_name != "u32" {
        return Err(format!(
            "fullscreen pass `{}` vertex hook must have signature `fn vertex(vertex_id: u32) -> ScreenVarying`",
            pass.name
        ));
    }
    let authored_varying = vertex
        .return_ty
        .as_ref()
        .map(|ty| ty.node.as_str())
        .ok_or_else(|| {
            format!(
                "fullscreen pass `{}` vertex hook requires a varying return type",
                pass.name
            )
        })?;
    let interface = pipeline.material_name.as_ref().expect("selected interface");
    if shade.params.len() != 2
        || shade.params[0].ty_name != authored_varying
        || shade.params[1].ty_name != *interface
    {
        return Err(format!(
            "fullscreen pass `{}` shade hook must accept a varying and `{interface}` instance",
            pass.name
        ));
    }

    for permutation in &pass.permutations {
        let mode = permutation
            .attrs
            .iter()
            .find(|attr| attr.name == "known")
            .and_then(|attr| attr.args.first())
            .map(|mode| mode.trim().to_ascii_lowercase());
        if mode.as_deref() != Some("compile") {
            return Err(format!(
                "executable fullscreen pass `{}` axis `{}` must use `@known(compile)`; pipeline- and draw-known lowering is not implemented yet",
                pass.name, permutation.name
            ));
        }
    }
    let compile_variants = if pass.permutations.is_empty() {
        vec![HashMap::new()]
    } else {
        crate::check::active_pass_variants(pass)
    };

    let suffix = sanitize(&hir.name);
    let varying = format!("FrescoFullscreenVarying_{suffix}");
    let entry_base = format!("fresco_pass_{}_{}", sanitize(&pass.name), suffix);
    let helper = format!("fresco_{}", hir.name);
    let helper_args = helper_arguments(hir)?;
    let helper_params = helper_parameters(hir)?;
    let position_helper = format!("fresco_fullscreen_triangle_position_{suffix}");
    let interface_param = shade.params[1].name.as_str();
    let mut wgsl = format!(
        r#"
struct {varying} {{
  @builtin(position) clip_pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
}}

fn {position_helper}(vertex_id: u32) -> vec2<f32> {{
  let x = f32((vertex_id << 1u) & 2u);
  let y = f32(vertex_id & 2u);
  return vec2<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0);
}}
"#,
    );
    let mut variants = Vec::new();
    let mut entry_suffixes = HashSet::new();
    for values in &compile_variants {
        let bindings = pass
            .permutations
            .iter()
            .filter_map(|axis| {
                values
                    .get(&axis.name)
                    .map(|value| (axis.name.clone(), value.clone()))
            })
            .collect::<Vec<_>>();
        let key = variant_key(&bindings);
        let entry_suffix = if bindings.is_empty() {
            String::new()
        } else {
            format!("_{}", sanitize(&key))
        };
        if !entry_suffixes.insert(entry_suffix.clone()) {
            return Err(format!(
                "fullscreen pass `{}` has compile-time variants whose keys collide after WGSL name sanitization near `{key}`",
                pass.name
            ));
        }
        let vertex_entry = format!("{entry_base}{entry_suffix}_vertex");
        let fragment_entry = format!("{entry_base}{entry_suffix}_fragment");
        let context = EmitContext {
            entry,
            helper: &helper,
            interface_param,
            varying: &varying,
            authored_varying,
            position_helper: &position_helper,
            helper_args: &helper_args,
            compile_values: values,
        };
        let vertex_code = emit_statements(&vertex_body, context)?;
        let shade_code = emit_statements(&shade_body, context)?;
        wgsl.push_str(&format!(
            r#"
@vertex
fn {vertex_entry}(@builtin(vertex_index) {vertex_param}: u32) -> {varying} {{
{vertex_code}}}

@fragment
fn {fragment_entry}({shade_param}: {varying}) -> @location(0) vec4<f32> {{
{shade_code}}}
"#,
            vertex_param = vertex_param.name,
            shade_param = shade.params[0].name,
        ));
        variants.push(FullscreenVariant {
            key,
            bindings,
            vertex_entry,
            fragment_entry,
        });
    }
    let first_variant = variants
        .first()
        .ok_or_else(|| format!("fullscreen pass `{}` has no active variants", pass.name))?;
    let validation_wgsl = format!(
        r#"
struct FrameGlobals {{
  time: f32,
  delta_time: f32,
  resolution: vec2<f32>,
}}
@group(3) @binding(0)
var<uniform> frame: FrameGlobals;

struct FrescoFullscreenUniforms {{
  time: f32,
  _pad0: vec3<f32>,
  res: vec2<f32>,
  _pad1: vec2<f32>,
  params: array<vec4<f32>, 16>,
}}
@group(0) @binding(0)
var<uniform> fresco_fullscreen_uniforms: FrescoFullscreenUniforms;

fn {helper}({helper_params}) -> vec4<f32> {{
  return vec4<f32>(0.0);
}}
{wgsl}
"#,
    );

    Ok(Some(FullscreenStage {
        pipeline: pipeline.name.clone(),
        pass: pass.name.clone(),
        interface: interface.clone(),
        vertex_entry: first_variant.vertex_entry.clone(),
        fragment_entry: first_variant.fragment_entry.clone(),
        variants,
        wgsl,
        validation_wgsl,
    }))
}

#[derive(Clone, Copy)]
struct EmitContext<'a> {
    entry: &'a crate::context::EntryContext,
    helper: &'a str,
    interface_param: &'a str,
    varying: &'a str,
    authored_varying: &'a str,
    position_helper: &'a str,
    helper_args: &'a str,
    compile_values: &'a HashMap<String, String>,
}

fn emit_statements(statements: &[Stmt], context: EmitContext<'_>) -> Result<String, String> {
    let mut output = String::new();
    for statement in statements {
        match statement {
            Stmt::Let {
                name,
                value,
                mutable,
                declared_ty_name,
                ..
            } => {
                let keyword = if *mutable { "var" } else { "let" };
                let annotation = match declared_ty_name {
                    Some(ty) => format!(": {}", wgsl_type(ty)),
                    None => String::new(),
                };
                output.push_str(&format!(
                    "  {keyword} {name}{annotation} = {};\n",
                    emit_expr(value, context)?
                ));
            }
            Stmt::Assign {
                name,
                field_path,
                value,
                ..
            } => {
                let target = match field_path {
                    Some(field) => format!("{name}.{field}"),
                    None => name.clone(),
                };
                output.push_str(&format!("  {target} = {};\n", emit_expr(value, context)?));
            }
            Stmt::Const {
                name,
                ty_name,
                value,
                ..
            } => output.push_str(&format!(
                "  const {name}: {} = {};\n",
                wgsl_type(ty_name),
                emit_expr(value, context)?
            )),
            Stmt::Return { value, .. } => {
                output.push_str(&format!("  return {};\n", emit_expr(value, context)?));
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                if let Some(selected) = eval_compile_bool(cond, context.compile_values) {
                    let body = if selected {
                        then_body.as_slice()
                    } else {
                        else_body.as_deref().unwrap_or_default()
                    };
                    output.push_str(&emit_statements(body, context)?);
                } else {
                    output.push_str(&format!("  if ({}) {{\n", emit_expr(cond, context)?));
                    output.push_str(&indent(&emit_statements(then_body, context)?));
                    output.push_str("  }");
                    if let Some(else_body) = else_body {
                        output.push_str(" else {\n");
                        output.push_str(&indent(&emit_statements(else_body, context)?));
                        output.push_str("  }");
                    }
                    output.push('\n');
                }
            }
            other => {
                return Err(format!(
                    "unsupported executable fullscreen statement: {other:?}"
                ));
            }
        }
    }
    Ok(output)
}

fn variant_key(bindings: &[(String, String)]) -> String {
    if bindings.is_empty() {
        "default".to_string()
    } else {
        bindings
            .iter()
            .map(|(axis, value)| format!("{axis}={value}"))
            .collect::<Vec<_>>()
            .join(";")
    }
}

fn compile_value(expression: &SExpr, values: &HashMap<String, String>) -> Option<String> {
    match &expression.node {
        Expr::Var(name) => values.get(name).cloned(),
        Expr::Str(value) => Some(value.clone()),
        Expr::Num(value, _) => Some(value.to_string()),
        _ => None,
    }
}

fn eval_compile_bool(expression: &SExpr, values: &HashMap<String, String>) -> Option<bool> {
    let Expr::Binary(op, left, right) = &expression.node else {
        return None;
    };
    match op {
        BinOp::Eq => Some(compile_value(left, values)? == compile_value(right, values)?),
        BinOp::Ne => Some(compile_value(left, values)? != compile_value(right, values)?),
        BinOp::Intersect | BinOp::LogicalAnd => {
            Some(eval_compile_bool(left, values)? && eval_compile_bool(right, values)?)
        }
        BinOp::Union | BinOp::LogicalOr => {
            Some(eval_compile_bool(left, values)? || eval_compile_bool(right, values)?)
        }
        _ => None,
    }
}

fn emit_expr(expression: &SExpr, context: EmitContext<'_>) -> Result<String, String> {
    match &expression.node {
        Expr::Num(value, _) => Ok(format_float(*value)),
        Expr::Color(value) => Ok(format!(
            "vec4<f32>({}, {}, {}, {})",
            format_float(value[0].into()),
            format_float(value[1].into()),
            format_float(value[2].into()),
            format_float(value[3].into())
        )),
        Expr::Var(name) => Ok(name.clone()),
        Expr::Vec2(a, b) => Ok(format!(
            "vec2<f32>({}, {})",
            emit_expr(a, context)?,
            emit_expr(b, context)?
        )),
        Expr::Vec3(a, b, c) => Ok(format!(
            "vec3<f32>({}, {}, {})",
            emit_expr(a, context)?,
            emit_expr(b, context)?,
            emit_expr(c, context)?
        )),
        Expr::Vec4(a, b, c, d) => Ok(format!(
            "vec4<f32>({}, {}, {}, {})",
            emit_expr(a, context)?,
            emit_expr(b, context)?,
            emit_expr(c, context)?,
            emit_expr(d, context)?
        )),
        Expr::Unary(op, value) => Ok(format!(
            "({}{})",
            match op {
                UnOp::Neg => "-",
            },
            emit_expr(value, context)?
        )),
        Expr::Binary(op, left, right) => Ok(format!(
            "({} {} {})",
            emit_expr(left, context)?,
            binary_op(*op),
            emit_expr(right, context)?
        )),
        Expr::Member(value, field) => Ok(format!("{}.{}", emit_expr(value, context)?, field)),
        Expr::Call { name, args, .. } => {
            let emitted = args
                .iter()
                .map(|arg| emit_expr(&arg.value, context))
                .collect::<Result<Vec<_>, _>>()?;
            let target = match name.as_str() {
                "vec2" => "vec2<f32>",
                "vec3" => "vec3<f32>",
                "vec4" | "rgba" => "vec4<f32>",
                "fullscreen_triangle_position" => context.position_helper,
                other if other == context.authored_varying => context.varying,
                other => other,
            };
            Ok(format!("{target}({})", emitted.join(", ")))
        }
        Expr::Pipe {
            recv, name, args, ..
        } if matches!(&recv.node, Expr::Var(receiver) if receiver == context.interface_param)
            && name == &context.entry.method =>
        {
            if args.len() != 1 {
                return Err("canvas entry requires exactly one context argument".to_string());
            }
            let argument = args.first().expect("checked argument count");
            Ok(format!(
                "{}({}{})",
                context.helper,
                emit_expr(&argument.value, context)?,
                context.helper_args,
            ))
        }
        other => Err(format!(
            "unsupported executable fullscreen expression: {other:?}"
        )),
    }
}

fn binary_op(op: BinOp) -> &'static str {
    super::gpu_function::binary_op(op)
}

fn wgsl_type(name: &str) -> &str {
    match name {
        "vec2" => "vec2<f32>",
        "vec3" => "vec3<f32>",
        "vec4" | "color" => "vec4<f32>",
        other => other,
    }
}

fn format_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("  {line}\n")).collect()
}

fn helper_arguments(hir: &Hir) -> Result<String, String> {
    let mut slot = 0usize;
    let mut args = Vec::new();
    for param in &hir.params {
        if hir
            .entry_context
            .as_ref()
            .is_some_and(|context| context.is_component(&param.name))
        {
            continue;
        }
        let component = |index: usize| {
            let vector = index.div_euclid(4);
            let component = ["x", "y", "z", "w"][index.rem_euclid(4)];
            format!("fresco_fullscreen_uniforms.params[{vector}].{component}")
        };
        if let Some((element_name, length)) = crate::hir::parse_array_param_type(&param.ty_name) {
            let element = crate::hir::ArrayElemType::from_str(element_name)
                .ok_or_else(|| format!("unsupported array element type in `{}`", param.name))?;
            let width = match element {
                crate::hir::ArrayElemType::F32
                | crate::hir::ArrayElemType::I32
                | crate::hir::ArrayElemType::U32
                | crate::hir::ArrayElemType::Bool => 1,
                crate::hir::ArrayElemType::Vec2 => 2,
                crate::hir::ArrayElemType::Vec3 => 3,
                crate::hir::ArrayElemType::Vec4 | crate::hir::ArrayElemType::Color => 4,
                crate::hir::ArrayElemType::Mat2 => 4,
                crate::hir::ArrayElemType::Mat3 => 9,
                crate::hir::ArrayElemType::Mat4 => 16,
            };
            for _ in 0..length {
                if matches!(element, crate::hir::ArrayElemType::Color) {
                    args.push(format!(
                        "vec4<f32>({}, {}, {}, {})",
                        component(slot),
                        component(slot + 1),
                        component(slot + 2),
                        component(slot + 3)
                    ));
                    slot += 4;
                } else {
                    for _ in 0..width {
                        args.push(component(slot));
                        slot += 1;
                    }
                }
            }
        } else if matches!(
            crate::hir::parse_array_param_type_ex(&param.ty_name),
            Some((_, crate::hir::ArrayParamSize::Dynamic))
        ) {
            continue;
        } else {
            let (argument, width) = match param.ty_name.as_str() {
                "f32" | "float" | "scalar" | "i32" | "int" | "u32" | "bool" => (component(slot), 1),
                "color" => (
                    format!(
                        "vec4<f32>({}, {}, {}, {})",
                        component(slot),
                        component(slot + 1),
                        component(slot + 2),
                        component(slot + 3)
                    ),
                    4,
                ),
                other => {
                    return Err(format!(
                        "fullscreen pass instance parameter `{}` uses unsupported runtime type `{other}`",
                        param.name
                    ));
                }
            };
            slot += width;
            args.push(argument);
        }
        if slot > 64 {
            return Err(format!(
                "fullscreen pass instance `{}` exceeds 64 runtime parameter slots",
                hir.name
            ));
        }
    }
    if args.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!(", {}", args.join(", ")))
    }
}

fn helper_parameters(hir: &Hir) -> Result<String, String> {
    let entry = hir.entry_context.as_ref().expect("registered canvas entry");
    let mut params = vec![format!("ctx: {}", entry.ty.name)];
    let mut index = 0usize;
    for param in &hir.params {
        if entry.is_component(&param.name) {
            continue;
        }
        if let Some((element_name, length)) = crate::hir::parse_array_param_type(&param.ty_name) {
            let element = crate::hir::ArrayElemType::from_str(element_name)
                .ok_or_else(|| format!("unsupported array element type in `{}`", param.name))?;
            let width = match element {
                crate::hir::ArrayElemType::F32
                | crate::hir::ArrayElemType::I32
                | crate::hir::ArrayElemType::U32
                | crate::hir::ArrayElemType::Bool => 1,
                crate::hir::ArrayElemType::Vec2 => 2,
                crate::hir::ArrayElemType::Vec3 => 3,
                crate::hir::ArrayElemType::Vec4 | crate::hir::ArrayElemType::Color => 4,
                crate::hir::ArrayElemType::Mat2 => 4,
                crate::hir::ArrayElemType::Mat3 => 9,
                crate::hir::ArrayElemType::Mat4 => 16,
            };
            for _ in 0..length {
                if matches!(element, crate::hir::ArrayElemType::Color) {
                    params.push(format!("param_{index}: vec4<f32>"));
                    index += 1;
                } else {
                    for _ in 0..width {
                        params.push(format!("param_{index}: f32"));
                        index += 1;
                    }
                }
            }
        } else if crate::hir::parse_array_param_type_ex(&param.ty_name).is_none() {
            let ty = if param.ty_name == "color" {
                "vec4<f32>"
            } else {
                "f32"
            };
            params.push(format!("param_{index}: {ty}"));
            index += 1;
        }
    }
    Ok(params.join(", "))
}
