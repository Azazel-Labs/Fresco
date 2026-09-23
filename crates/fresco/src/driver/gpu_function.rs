//! Shared ordinary GPU expression and statement emission.
use crate::ast::{BinOp, Expr, SExpr, Stmt, UnOp};
use crate::material_hir::MaterialHir;
use std::collections::HashMap;

/// Preserve source identifiers across WGSL's larger reserved-word namespace.
/// Escape the prefix itself too, so authored names cannot collide with escapes.
pub(super) fn shader_identifier(name: &str) -> String {
    if naga::keywords::wgsl::RESERVED.contains(&name) || name.starts_with("fresco_reserved_") {
        format!("fresco_reserved_{name}")
    } else {
        name.into()
    }
}

fn shader_path(name: &str) -> String {
    name.split('.')
        .map(shader_identifier)
        .collect::<Vec<_>>()
        .join(".")
}

#[derive(Clone)]
pub(super) struct ResourceSpecialization {
    pub root: usize,
    pub bindings: std::collections::BTreeMap<String, String>,
    pub name: String,
}

#[derive(Default)]
pub(super) struct ResourceSpecializations(std::cell::RefCell<Vec<ResourceSpecialization>>);

impl ResourceSpecializations {
    pub fn register(
        &self,
        root: usize,
        bindings: std::collections::BTreeMap<String, String>,
        base: &str,
    ) -> String {
        let mut entries = self.0.borrow_mut();
        if let Some(existing) = entries
            .iter()
            .find(|entry| entry.root == root && entry.bindings == bindings)
        {
            return existing.name.clone();
        }
        let name = format!("{base}_resources_{}", entries.len());
        entries.push(ResourceSpecialization {
            root,
            bindings,
            name: name.clone(),
        });
        name
    }
}

pub(super) fn texture_integer_argument(name: &str, index: usize) -> bool {
    (name == "textureLoad" && index >= 2) || (name == "textureDimensions" && index == 1)
}

pub(super) fn integer_literal(value: &SExpr) -> Result<Option<i32>, String> {
    let value = match &value.node {
        Expr::Num(value, crate::lexer::Unit::None) => *value,
        Expr::Unary(UnOp::Neg, inner) => match &inner.node {
            Expr::Num(value, crate::lexer::Unit::None) => -*value,
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    if value.fract() != 0.0 || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) {
        return Err("texture integer argument requires a representable integer literal".into());
    }
    Ok(Some(value as i32))
}

#[derive(Clone)]
pub(super) struct ExprContext<'a> {
    pub resource_specializations: Option<&'a ResourceSpecializations>,
    pub material: Option<&'a MaterialHir>,
    pub imported_functions: Option<&'a [crate::ast::FnDecl]>,
    pub imported_records: Option<&'a [crate::ast::StructDecl]>,
    pub referenced_calls: Option<&'a std::cell::RefCell<std::collections::BTreeSet<String>>>,
    pub input_var: &'a str,
    pub varying_source_ty: &'a str,
    pub varying_ty: &'a str,
    pub factory_entry: &'a str,
    pub binding_vars: &'a HashMap<&'a str, String>,
    pub type_aliases: Option<&'a HashMap<String, String>>,
}

pub(super) fn emit_statements(
    statements: &[Stmt],
    context: &ExprContext<'_>,
) -> Result<String, String> {
    let mut output = String::new();
    let mut bindings = context.binding_vars.clone();
    for statement in statements {
        let scoped = ExprContext {
            binding_vars: &bindings,
            ..context.clone()
        };
        let context = &scoped;
        match statement {
            Stmt::Block { body, .. } => {
                output.push_str("  {\n");
                output.push_str(&emit_statements(body, context)?);
                output.push_str("  }\n");
            }
            Stmt::Let {
                name,
                value,
                mutable,
                declared_ty_name,
                ..
            } => {
                let keyword = if *mutable { "var" } else { "let" };
                let annotation = match declared_ty_name {
                    Some(ty) => format!(": {}", context_type(ty, context)?),
                    None => String::new(),
                };
                output.push_str(&format!(
                    "  {keyword} {name}{annotation} = {};\n",
                    emit_expr(value, context)?,
                    name = shader_identifier(name)
                ));
            }
            Stmt::Store { target, value, .. } => {
                output.push_str(&format!(
                    "  {} = {};\n",
                    emit_expr(target, context)?,
                    emit_expr(value, context)?
                ));
            }
            Stmt::Break { .. } => output.push_str("  break;\n"),
            Stmt::Assign {
                name,
                field_path,
                value,
                ..
            } => {
                let target = match field_path {
                    Some(field) => format!("{}.{}", shader_identifier(name), shader_path(field)),
                    None => shader_identifier(name),
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
                context_type(ty_name, context)?,
                emit_expr(value, context)?,
                name = shader_identifier(name)
            )),
            Stmt::For {
                name,
                iterable,
                body,
                index_name: None,
                ..
            } => {
                let Expr::Range(start, end) = &iterable.node else {
                    return Err("executable mesh loops require an integer range".into());
                };
                let mut loop_bindings = context.binding_vars.clone();
                loop_bindings.remove(name.as_str());
                let loop_context = ExprContext {
                    binding_vars: &loop_bindings,
                    ..context.clone()
                };
                output.push_str(&format!(
                    "  for (var {name} = u32({}); {name} < u32({}); {name} = {name} + 1u) {{\n{}  }}\n",
                    emit_expr(start, context)?, emit_expr(end, context)?, emit_statements(body, &loop_context)?, name = shader_identifier(name)
                ));
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                if let Some(selected) = context
                    .material
                    .and_then(|m| super::surface_properties::static_condition(m, cond))
                {
                    if selected {
                        output.push_str(&emit_statements(then_body, context)?);
                    } else if let Some(body) = else_body {
                        output.push_str(&emit_statements(body, context)?);
                    }
                    continue;
                }
                output.push_str(&format!(
                    "  if ({}) {{\n{}  }}\n",
                    emit_expr(cond, context)?,
                    emit_statements(then_body, context)?
                ));
                if let Some(body) = else_body {
                    output.push_str(&format!(
                        "  else {{\n{}  }}\n",
                        emit_statements(body, context)?
                    ));
                }
            }
            Stmt::Expr(value) if matches!(&value.node, Expr::Call { name, args, .. } if name == "discard_fragment" && args.is_empty()) =>
            {
                output.push_str("  discard;\n");
            }
            Stmt::Expr(value) => output.push_str(&format!("  {};\n", emit_expr(value, context)?)),
            Stmt::ReturnVoid { .. } => output.push_str("  return;\n"),
            Stmt::Return { value, .. } => {
                output.push_str(&format!("  return {};\n", emit_expr(value, context)?));
            }
            other => {
                return Err(format!("unsupported executable mesh statement: {other:?}"));
            }
        }
        if let Stmt::Let { name, .. } | Stmt::Const { name, .. } = statement {
            bindings.remove(name.as_str());
        }
    }
    Ok(output)
}

pub(super) fn emit_expr(expression: &SExpr, context: &ExprContext<'_>) -> Result<String, String> {
    let static_name = match &expression.node {
        Expr::Var(name) => Some(name.clone()),
        Expr::Member(base, member) => match &base.node {
            Expr::Var(name) => Some(format!("{name}.{member}")),
            _ => None,
        },
        _ => None,
    };
    if let Some(binding) = static_name
        .as_deref()
        .and_then(|name| context.binding_vars.get(name))
    {
        return Ok(binding.clone());
    }
    if let Some(value) = static_name.as_deref().and_then(|name| {
        context
            .material
            .and_then(|m| super::surface_properties::static_value(m, name))
    }) {
        return Ok(value);
    }
    match &expression.node {
        Expr::Num(value, crate::lexer::Unit::None) => Ok(format_float(*value)),
        Expr::Num(_, unit) => Err(format!(
            "unit `{unit:?}` requires explicit conversion in an ordinary GPU function"
        )),
        Expr::Array(items) => Ok(format!(
            "array({})",
            items
                .iter()
                .map(|item| emit_expr(item, context))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        Expr::Color(value) => Ok(format!(
            "vec4<f32>({}, {}, {}, {})",
            format_float(value[0].into()),
            format_float(value[1].into()),
            format_float(value[2].into()),
            format_float(value[3].into())
        )),
        Expr::Var(name) if matches!(name.as_str(), "true" | "false") => Ok(name.clone()),
        Expr::Var(name) => Ok(context
            .binding_vars
            .get(name.as_str())
            .cloned()
            .or_else(|| {
                name.split_once('.').and_then(|(root, path)| {
                    context
                        .binding_vars
                        .get(root)
                        .map(|resolved| format!("{resolved}.{}", shader_path(path)))
                })
            })
            .unwrap_or_else(|| shader_path(name))),
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
        Expr::Unary(UnOp::Neg, value) => Ok(format!("(-{})", emit_expr(value, context)?)),
        Expr::Binary(op, left, right) => Ok(format!(
            "({} {} {})",
            emit_expr(left, context)?,
            binary_op(*op),
            emit_expr(right, context)?
        )),
        Expr::Member(value, field) => Ok(format!(
            "{}.{}",
            emit_expr(value, context)?,
            shader_identifier(field)
        )),
        Expr::Index { array, index } => Ok(format!(
            "{}[u32({})]",
            emit_expr(array, context)?,
            emit_expr(index, context)?
        )),
        Expr::Call { name, args, .. } => {
            if name == "evaluate_schema" {
                return super::schema_evaluation::emit(
                    context
                        .material
                        .ok_or("schema evaluation requires a surface adapter")?,
                    args,
                    |value| emit_expr(value, context),
                );
            }
            if context.imported_functions.is_some_and(|functions| {
                functions.iter().any(|function| {
                    function.name == *name && !function.is_builtin && !function.is_internal
                })
            }) && args.iter().any(|argument| argument.name.is_some())
            {
                return Err(format!(
                    "imported GPU function `{name}` requires positional arguments"
                ));
            }
            let element = crate::typed_scalar::Kind::element(name);
            let mut ordered: Vec<_> = args.iter().collect();
            let mut specialized = None;
            if let Some(index) = name
                .strip_prefix("fresco_library_import_")
                .and_then(|index| index.parse::<usize>().ok())
                && let Some(function) = context
                    .imported_functions
                    .and_then(|functions| functions.get(index))
            {
                let mut bindings = std::collections::BTreeMap::new();
                let mut ordinary = Vec::new();
                for (parameter, argument) in function.params.iter().zip(&ordered) {
                    if parameter.ty_name.starts_with("buffer<") {
                        bindings
                            .insert(parameter.name.clone(), emit_expr(&argument.value, context)?);
                    } else {
                        ordinary.push(*argument);
                    }
                }
                if !bindings.is_empty() {
                    let registry = context.resource_specializations.ok_or(
                        "buffer resource specialization is unavailable in this shader context",
                    )?;
                    let base = context
                        .type_aliases
                        .and_then(|aliases| aliases.get(name))
                        .ok_or("resource function symbol is missing")?;
                    specialized = Some(registry.register(index, bindings, base));
                    ordered = ordinary;
                }
            }
            if specialized.is_none()
                && let Some(calls) = context.referenced_calls
            {
                calls.borrow_mut().insert(name.clone());
            }
            if let Some(record) = context
                .imported_records
                .and_then(|records| records.iter().find(|record| record.name == *name))
                && args.iter().any(|arg| arg.name.is_some())
            {
                if args.len() != record.fields.len() {
                    return Err(format!("record `{name}` requires every field exactly once"));
                }
                ordered = record
                    .fields
                    .iter()
                    .map(|field| {
                        let matches: Vec<_> = args
                            .iter()
                            .filter(|arg| arg.name.as_deref() == Some(field.name.as_str()))
                            .collect();
                        match matches.as_slice() {
                            [arg] => Ok(*arg),
                            _ => Err(format!(
                                "record `{name}` requires field `{}` exactly once",
                                field.name
                            )),
                        }
                    })
                    .collect::<Result<_, _>>()?;
            }
            let args = ordered.into_iter().enumerate()
                .map(|(index, arg)| {
                    if texture_integer_argument(name, index) && let Some(value) = integer_literal(&arg.value)? {
                        return Ok(format!("{value}i"));
                    }
                    let value = emit_expr(&arg.value, context)?;
                    let numeric_literal = matches!(arg.value.node, Expr::Num(..))
                        || matches!(&arg.value.node, Expr::Unary(UnOp::Neg, inner) if matches!(inner.node, Expr::Num(..)));
                    if name.starts_with(['u', 'i', 'b']) && name.contains("vec") && numeric_literal {
                        let kind = element.ok_or("unknown vector element type")?;
                        Ok(format!("{}({value})", kind.name()))
                    } else { Ok(value) }
                })
                .collect::<Result<Vec<_>, String>>()?;
            let target = match name.as_str() {
                "inverse_sqrt" | "inversesqrt" | "rsqrt" => "inverseSqrt",
                "float" | "half" => "f32",
                "int" => "i32",
                "mat2" => "mat2x2<f32>",
                "mat3" => "mat3x3<f32>",
                "mat4" => "mat4x4<f32>",
                "vec2" => "vec2<f32>",
                "uvec2" => "vec2<u32>",
                "uvec3" => "vec3<u32>",
                "uvec4" => "vec4<u32>",
                "ivec2" => "vec2<i32>",
                "ivec3" => "vec3<i32>",
                "ivec4" => "vec4<i32>",
                "bvec2" => "vec2<bool>",
                "bvec3" => "vec3<bool>",
                "bvec4" => "vec4<bool>",
                "vec3" => "vec3<f32>",
                "vec4" | "rgba" => "vec4<f32>",
                other if other == context.varying_source_ty => context.varying_ty,
                other => context
                    .type_aliases
                    .and_then(|aliases| aliases.get(other))
                    .map_or(other, String::as_str),
            };
            let target = specialized.as_deref().unwrap_or(target);
            Ok(format!("{target}({})", args.join(", ")))
        }
        Expr::Pipe {
            recv, name, args, ..
        } if matches!(&recv.node, Expr::Var(receiver) if receiver == "factory")
            && name == "transform" =>
        {
            let vertex = if let Some(arg) = args.first() {
                emit_expr(&arg.value, context)?
            } else {
                context.input_var.to_string()
            };
            Ok(format!("{}({vertex})", context.factory_entry))
        }
        other => Err(format!("unsupported executable mesh expression: {other:?}")),
    }
}

pub(super) fn context_type(name: &str, context: &ExprContext<'_>) -> Result<String, String> {
    if name.starts_with("buffer<") {
        return Err("buffer resources require binding specialization; they cannot be stored or returned as ordinary shader values".into());
    }
    if name.starts_with("texture2d<") {
        let resource = super::compute_operations::resource_type(name, "read")?;
        return Ok(super::compute_operations::resource_signature(
            &resource, false,
        ));
    }
    if matches!(
        name,
        "sampler" | "texture_2d<f32>" | "texture_2d<u32>" | "texture_2d<i32>" | "texture_depth_2d"
    ) {
        return Ok(name.into());
    }
    if let Some(inner) = name
        .strip_prefix("array<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let (element, count) = inner
            .rsplit_once(',')
            .ok_or("function arrays require a concrete positive length")?;
        let count = count
            .trim()
            .parse::<std::num::NonZeroU32>()
            .map_err(|_| "function arrays require a concrete positive length")?;
        return Ok(format!(
            "array<{}, {}>",
            context_type(element.trim(), context)?,
            count
        ));
    }
    context
        .type_aliases
        .and_then(|aliases| aliases.get(crate::check::strip_spatial_type_suffix(name)))
        .cloned()
        .map_or_else(|| wgsl_type(name), Ok)
}

pub(super) fn wgsl_type(name: &str) -> Result<String, String> {
    let base = crate::check::strip_spatial_type_suffix(name);
    let ty = match base.to_ascii_lowercase().as_str() {
        "f32" | "float" | "signal" | "mask" | "coverage" => "f32",
        "u32" => "u32",
        "i32" => "i32",
        "bool" => "bool",
        "half" => "f32",
        "int" => "i32",
        "vec2" => "vec2<f32>",
        "uvec2" => "vec2<u32>",
        "uvec3" => "vec3<u32>",
        "uvec4" => "vec4<u32>",
        "ivec2" => "vec2<i32>",
        "ivec3" => "vec3<i32>",
        "ivec4" => "vec4<i32>",
        "bvec2" => "vec2<bool>",
        "bvec3" => "vec3<bool>",
        "bvec4" => "vec4<bool>",
        "vec3" => "vec3<f32>",
        "vec4" | "color" => "vec4<f32>",
        "mat2" => "mat2x2<f32>",
        "mat3" => "mat3x3<f32>",
        "mat4" => "mat4x4<f32>",
        other => return Err(format!("unsupported executable mesh field type `{other}`")),
    };
    Ok(ty.to_string())
}

pub(super) fn binary_op(op: BinOp) -> &'static str {
    match op {
        BinOp::LogicalAnd => "&&",
        BinOp::LogicalOr => "||",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::BitXor => "^",
        BinOp::Union => "|",
        BinOp::Intersect => "&",
    }
}

pub(super) fn format_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

/// Link reachable concrete library functions in library scope, not caller resource scope.
pub(super) fn link_imports(
    functions: &[crate::ast::FnDecl],
    structs: &[crate::ast::StructDecl],
    aliases: &HashMap<String, String>,
    mut pending: std::collections::BTreeSet<String>,
    resources: &ResourceSpecializations,
) -> Result<(String, std::collections::BTreeSet<String>), String> {
    let mut emitted = std::collections::BTreeSet::new();
    let mut needed = std::collections::BTreeSet::new();
    let mut code = String::new();
    while let Some(name) = pending.pop_first() {
        if structs.iter().any(|record| record.name == name) {
            needed.insert(name.clone());
        }
        let candidates: Vec<_> = functions
            .iter()
            .enumerate()
            .filter(|(index, function)| {
                (function.name == name || super::schema_function::library_symbol(*index) == name)
                    && !function.is_builtin
                    && !function.is_internal
            })
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let [(index, _)] = candidates.as_slice() else {
            return Err(format!(
                "GPU imported function `{name}` requires an unambiguous concrete signature"
            ));
        };
        let (source, types) = super::schema_function::link_library_function(
            *index,
            functions,
            structs,
            aliases,
            &mut emitted,
            None,
            resources,
        )?;
        code.push_str(&source);
        needed.extend(types);
    }
    let mut specialized = std::collections::BTreeSet::new();
    loop {
        let next = resources
            .0
            .borrow()
            .iter()
            .find(|entry| !specialized.contains(&entry.name))
            .cloned();
        let Some(next) = next else { break };
        specialized.insert(next.name.clone());
        let (source, types) = super::schema_function::link_library_function(
            next.root,
            functions,
            structs,
            aliases,
            &mut emitted,
            Some(&next),
            resources,
        )?;
        code.push_str(&source);
        needed.extend(types);
    }
    needed.retain(|name| structs.iter().any(|record| &record.name == name));
    Ok((code, needed))
}
