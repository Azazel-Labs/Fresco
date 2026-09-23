//! Definition-side contracts for owned compute operations.
use crate::{ast::*, diag::Diag};
use fresco_artifact::{
    ManifestComputeBindingSource as BindingSource, ManifestComputeDimension as Dimension,
    ManifestComputeGeometryRole as GeometryRole, ManifestGpuProgram,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct KernelDefinition {
    pub pass: PassDecl,
    pub bindings: BTreeMap<String, BindingSource>,
}

impl KernelDefinition {
    pub fn emit(&self, program: &Program) -> Result<(String, ManifestGpuProgram), String> {
        let library = crate::check::compute::ComputeLibrary::new(program, Default::default());
        let (code, mut reflection) =
            super::mesh_pass::shader::emit(&self.pass, &program.structs, &library, None, "")?;
        let reflected: BTreeSet<_> = reflection.bindings.iter().map(|b| &b.name).collect();
        let declared: BTreeSet<_> = self.bindings.keys().collect();
        if reflected != declared {
            return Err(
                "compute binding reflection does not match lowered binding provenance".into(),
            );
        }
        reflection.compute_bindings.clone_from(&self.bindings);
        Ok((code, reflection))
    }
}

fn fail(span: &Span, message: impl Into<String>) -> Vec<Diag> {
    vec![Diag::error(span.clone(), message)]
}

pub(super) use crate::resource_type::{ResourceType, resource_type};

pub(super) fn resource_signature(resource: &ResourceType, write: bool) -> String {
    match resource {
        ResourceType::Buffer(element) => format!("buffer<{element}>"),
        ResourceType::Image(format) if write => {
            format!("texture_storage_2d<{}, write>", format.info().name)
        }
        ResourceType::Image(format) => format
            .info()
            .sampled_shader_type()
            .expect("resource type checked for sampled use"),
    }
}

pub(super) fn resource_dimensions(resource: &ResourceType) -> &'static [(&'static str, Dimension)] {
    match resource {
        ResourceType::Buffer(_) => &[("count", Dimension::Count)],
        ResourceType::Image(_) => &[("width", Dimension::Width), ("height", Dimension::Height)],
    }
}

fn path(value: &SExpr) -> Option<String> {
    match &value.node {
        Expr::Var(name) => Some(name.clone()),
        Expr::Member(base, name) => Some(format!("{}.{name}", path(base)?)),
        _ => None,
    }
}

/// Only CPU-known operation inputs and resource dimensions can construct work.
/// Values read out of shader buffers or arbitrary view fields cannot size an
/// allocation without GPU readback, which is outside this operation model.
pub(super) fn host_type(
    program: &Program,
    operation: &StyleOperation,
    value: &SExpr,
) -> Result<String, Vec<Diag>> {
    if let Some(name) = path(value) {
        if matches!(name.as_str(), "true" | "false") {
            return Ok("bool".into());
        }
        if let Some(input) = operation.inputs.iter().find(|i| i.name == name) {
            if matches!(input.ty.as_str(), "u32" | "i32" | "f32" | "bool") {
                return Ok(input.ty.clone());
            }
        } else if let Some((root, field)) = name.split_once('.')
            && let Some(input) = operation.inputs.iter().find(|i| i.name == root)
        {
            if let Some(resource) = program.resource_types.iter().find(|r| r.name == input.ty) {
                let roles = resource.attrs.iter().find(|a| a.name == "geometry");
                if roles.is_some_and(|a| {
                    a.args.get(2).is_some_and(|n| n == field)
                        || a.args.get(3).is_some_and(|n| n == field)
                }) {
                    return Ok("u32".into());
                }
            } else if let Ok(resource) = resource_type(&input.ty, "read")
                && matches!(
                    (&resource, field),
                    (ResourceType::Buffer(_), "count")
                        | (ResourceType::Image(_), "width" | "height")
                )
            {
                return Ok("u32".into());
            }
        }
        return Err(fail(
            &value.span,
            format!("`{name}` is not a CPU-known scalar or resource extent"),
        ));
    }
    let numeric = |ty: &str| matches!(ty, "number" | "u32" | "i32" | "f32");
    match &value.node {
        Expr::Num(number, Unit::None) if number.is_finite() => Ok(if number.fract() == 0.0 {
            "number"
        } else {
            "f32"
        }
        .into()),
        Expr::Unary(UnOp::Neg, inner) => {
            let ty = host_type(program, operation, inner)?;
            if matches!(ty.as_str(), "number" | "f32" | "i32") {
                Ok(ty)
            } else {
                Err(fail(&value.span, "host negation requires a signed scalar"))
            }
        }
        Expr::Binary(op, left, right) => {
            let left = host_type(program, operation, left)?;
            let right = host_type(program, operation, right)?;
            let common = if left == "number" && numeric(&right) {
                &right
            } else {
                &left
            };
            if left != right
                && !(left == "number" && numeric(&right) || right == "number" && numeric(&left))
            {
                return Err(fail(
                    &value.span,
                    "host expression operands have incompatible types",
                ));
            }
            match op {
                BinOp::LogicalAnd | BinOp::LogicalOr if common == "bool" => Ok("bool".into()),
                BinOp::Eq | BinOp::Ne => Ok("bool".into()),
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge if numeric(common) => {
                    Ok("bool".into())
                }
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
                    if numeric(common) =>
                {
                    Ok(common.clone())
                }
                _ => Err(fail(&value.span, "unsupported host expression operator")),
            }
        }
        Expr::Call {
            name,
            args,
            const_args,
            ..
        } if const_args.is_empty() && args.iter().all(|a| a.name.is_none()) => {
            if matches!(name.as_str(), "checked_mul" | "checked_add") && args.len() == 2 {
                for arg in args {
                    extent(program, operation, &arg.value)?;
                }
                Ok("u32".into())
            } else if matches!(name.as_str(), "u32" | "i32" | "f32")
                && args.len() == 1
                && numeric(&host_type(program, operation, &args[0].value)?)
            {
                Ok(name.clone())
            } else {
                Err(fail(
                    &value.span,
                    "host expressions require scalar conversions or checked arithmetic",
                ))
            }
        }
        _ => Err(fail(
            &value.span,
            "operation extents and preconditions cannot read GPU values",
        )),
    }
}

fn extent(program: &Program, operation: &StyleOperation, value: &SExpr) -> Result<(), Vec<Diag>> {
    let ty = host_type(program, operation, value)?;
    if ty == "number" {
        // Check constant expressions through the ordinary typed constant checker.
        let parameter = GlobalParamDecl {
            name: "__compute_extent".into(),
            name_span: value.span.clone(),
            ty_name: "u32".into(),
            ty_span: value.span.clone(),
            default: None,
            range: None,
            span: value.span.clone(),
        };
        crate::check::compute::style_parameter_value(program, &parameter, value)?;
    } else if ty != "u32" {
        return Err(fail(&value.span, "logical extents require u32 values"));
    }
    Ok(())
}

/// Build a fully bound kernel independently of whether a graph instantiates it.
/// Invocation lowering supplies these same bindings with concrete resources.
pub(super) fn shader_definition(
    program: &Program,
    definition: &PassDecl,
) -> Result<KernelDefinition, Vec<Diag>> {
    shader_definition_projected(program, definition, &BTreeMap::new())
}

pub(super) fn shader_definition_projected(
    program: &Program,
    definition: &PassDecl,
    projections: &BTreeMap<String, (String, String)>,
) -> Result<KernelDefinition, Vec<Diag>> {
    let operation = definition.operation.as_ref().expect("operation");
    let compute = operation.compute.as_ref().expect("compute");
    let output = compute.output.as_ref().expect("validated output");
    let mut pass = definition.clone();
    pass.operation = None;
    pass.stage = Some(Spanned {
        node: "compute".into(),
        span: pass.span.clone(),
    });
    let mut replacements = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut prelude = String::new();
    fn binding(
        pass: &mut PassDecl,
        sources: &mut BTreeMap<String, BindingSource>,
        declaration: (&str, &str),
        write: bool,
        source: BindingSource,
    ) -> Result<(), Vec<Diag>> {
        let (name, ty) = declaration;
        if sources.insert(name.into(), source).is_some() {
            return Err(fail(&pass.span, "duplicate lowered compute binding"));
        }
        let index = u32::try_from(pass.bindings.len())
            .map_err(|_| fail(&pass.span, "compute binding count overflow"))?;
        let mut attrs = vec![
            attribute("group", vec!["0".into()], &pass.span),
            attribute("binding", vec![index.to_string()], &pass.span),
        ];
        if write {
            attrs.push(attribute("access", vec!["write".into()], &pass.span));
        }
        pass.bindings.push(PassBindingDecl {
            operation_alias: None,
            group_index: Some(0),
            binding_index: Some(index),
            name: name.into(),
            name_span: pass.span.clone(),
            attrs,
            value_signature: Some(ty.into()),
            span: pass.span.clone(),
        });
        Ok(())
    }
    for input in &operation.inputs {
        if input.name.starts_with("__fresco_") {
            return Err(fail(
                &input.span,
                "__fresco_ is reserved for compiler-owned operation bindings",
            ));
        }
        if let Some((root_ty, member)) = projections.get(&input.name) {
            let name = format!("__fresco_projection_{}", input.name);
            binding(
                &mut pass,
                &mut sources,
                (&name, &format!("uniform<{root_ty}>")),
                false,
                BindingSource::Value {
                    parameter: input.name.clone(),
                    ty: root_ty.clone(),
                },
            )?;
            prelude.push_str(&format!(
                "let {}: {} = {name}.{member};\n",
                input.name, input.ty
            ));
        } else if let Some(resource) = program.resource_types.iter().find(|r| r.name == input.ty) {
            let roles = resource
                .attrs
                .iter()
                .find(|a| a.name == "geometry")
                .expect("validated geometry roles");
            for field in &resource.fields {
                let name = format!("__fresco_{}_{}", input.name, field.name);
                let geometry_source = || -> Result<BindingSource, Vec<Diag>> {
                    let role = roles
                        .args
                        .iter()
                        .position(|name| name == &field.name)
                        .and_then(|index| {
                            [
                                GeometryRole::Vertices,
                                GeometryRole::Indices,
                                GeometryRole::VertexCount,
                                GeometryRole::IndexCount,
                                GeometryRole::Bounds,
                            ]
                            .get(index)
                            .copied()
                        })
                        .ok_or_else(|| {
                            fail(&field.span, "geometry field has no compute binding role")
                        })?;
                    Ok(BindingSource::Geometry {
                        parameter: input.name.clone(),
                        member: field.name.clone(),
                        role,
                    })
                };
                if roles.args.get(4) == Some(&field.name) {
                    let record = program
                        .structs
                        .iter()
                        .find(|r| r.name == field.ty_name)
                        .expect("bounds record");
                    if record.fields.len() != 3
                        || record.fields[0].ty_name != "vec3"
                        || record.fields[1].ty_name != "vec3"
                        || record.fields[2].ty_name != "bool"
                    {
                        return Err(fail(
                            &field.span,
                            "geometry bounds require two vec3 fields and a bool availability flag",
                        ));
                    }
                    let data = format!("__fresco_{}_bounds_data", input.name);
                    binding(
                        &mut pass,
                        &mut sources,
                        (&data, "uniform<__FrescoGeometryBounds>"),
                        false,
                        geometry_source()?,
                    )?;
                    // Construct on actual use, so an unused bounds member does
                    // not introduce an artificial metadata read/dependency.
                    replacements.insert(
                        format!("{}.{}", input.name, field.name),
                        format!(
                            "{}({data}.minimum.xyz, {data}.maximum.xyz, {data}.minimum.w != 0.0)",
                            field.ty_name
                        ),
                    );
                } else if let Ok(resource) = resource_type(&field.ty_name, "read") {
                    binding(
                        &mut pass,
                        &mut sources,
                        (&name, &resource_signature(&resource, false)),
                        false,
                        geometry_source()?,
                    )?;
                } else {
                    binding(
                        &mut pass,
                        &mut sources,
                        (&name, &format!("uniform<{}>", field.ty_name)),
                        false,
                        geometry_source()?,
                    )?;
                }
                if roles.args.get(4) != Some(&field.name) {
                    pass.bindings
                        .last_mut()
                        .expect("geometry binding")
                        .operation_alias = Some(format!("{}.{}", input.name, field.name));
                }
            }
        } else if let Ok(resource) = resource_type(&input.ty, "read") {
            binding(
                &mut pass,
                &mut sources,
                (&input.name, &resource_signature(&resource, false)),
                false,
                BindingSource::Resource {
                    parameter: input.name.clone(),
                },
            )?;
            let dimensions = resource_dimensions(&resource);
            for (dimension, axis) in dimensions {
                let name = format!("__fresco_{}_{}", input.name, dimension);
                binding(
                    &mut pass,
                    &mut sources,
                    (&name, "uniform<u32>"),
                    false,
                    BindingSource::Dimension {
                        parameter: input.name.clone(),
                        axis: *axis,
                    },
                )?;
                pass.bindings
                    .last_mut()
                    .expect("dimension binding")
                    .operation_alias = Some(format!("{}.{dimension}", input.name));
            }
        } else if input.ty == "bool" {
            let name = format!("__fresco_bool_{}", input.name);
            binding(
                &mut pass,
                &mut sources,
                (&name, "uniform<u32>"),
                false,
                BindingSource::Value {
                    parameter: input.name.clone(),
                    ty: input.ty.clone(),
                },
            )?;
            prelude.push_str(&format!("let {} = {name} != 0u;\n", input.name));
        } else {
            let ty = if input.ty == "color" {
                "vec4"
            } else {
                &input.ty
            };
            binding(
                &mut pass,
                &mut sources,
                (&input.name, &format!("uniform<{ty}>")),
                false,
                BindingSource::Value {
                    parameter: input.name.clone(),
                    ty: input.ty.clone(),
                },
            )?;
        }
    }
    if output.name.starts_with("__fresco_") {
        return Err(fail(
            &output.span,
            "__fresco_ is reserved for compiler-owned operation bindings",
        ));
    }
    binding(
        &mut pass,
        &mut sources,
        (
            &output.name,
            &resource_signature(
                &resource_type(&output.ty, "write").map_err(|m| fail(&output.span, m))?,
                true,
            ),
        ),
        true,
        BindingSource::Output,
    )?;
    binding(
        &mut pass,
        &mut sources,
        ("__fresco_dispatch", "uniform<uvec3>"),
        false,
        BindingSource::Dispatch,
    )?;
    let workgroup = compute
        .workgroup_size
        .as_ref()
        .expect("validated workgroup")
        .iter()
        .map(|value| {
            let parameter = GlobalParamDecl {
                name: "__compute_workgroup".into(),
                name_span: value.span.clone(),
                ty_name: "u32".into(),
                ty_span: value.span.clone(),
                default: None,
                range: None,
                span: value.span.clone(),
            };
            let evaluated =
                crate::check::compute::style_parameter_value(program, &parameter, value)?;
            Ok(evaluated.as_u64().expect("validated dimension").to_string())
        })
        .collect::<Result<Vec<_>, Vec<Diag>>>()?;
    for hook in &mut pass.hooks {
        // Match authored token sequences, retaining original spans. Longest
        // resource paths come first so nested aggregate members stay unambiguous.
        let patterns: Vec<_> = replacements
            .iter()
            .map(|(from, to)| {
                (
                    crate::lexer::lex_spanned(from)
                        .into_iter()
                        .map(|(token, _)| token)
                        .collect::<Vec<_>>(),
                    to,
                )
            })
            .collect();
        let mut body = Vec::new();
        let mut cursor = 0;
        while cursor < hook.body.len() {
            if let Some((pattern, replacement)) = patterns.iter().find(|(pattern, _)| {
                hook.body[cursor..]
                    .iter()
                    .take(pattern.len())
                    .map(|(token, _)| token)
                    .eq(pattern.iter())
            }) {
                let span = hook.body[cursor].1.clone();
                body.extend(
                    crate::lexer::lex_spanned(replacement)
                        .into_iter()
                        .map(|(token, _)| (token, span.clone())),
                );
                cursor += pattern.len();
            } else {
                body.push(hook.body[cursor].clone());
                cursor += 1;
            }
        }
        hook.body = body;
        let mut capture = prelude.clone();
        if hook.attrs.iter().any(|a| a.name == "compute") {
            let parameter = &mut hook.params[0];
            parameter.attrs.push(attribute(
                "builtin",
                vec!["global_invocation_id".into()],
                &parameter.name_span,
            ));
            hook.attrs
                .push(attribute("workgroup_size", workgroup.clone(), &hook.span));
            let id = &parameter.name;
            capture.push_str(&format!("if {id}.x >= __fresco_dispatch.x || {id}.y >= __fresco_dispatch.y || {id}.z >= __fresco_dispatch.z {{ return }}\n"));
        } else if !hook.attrs.is_empty() {
            return Err(fail(
                &hook.span,
                "compute helpers cannot declare other shader stages or attributes",
            ));
        }
        hook.body.splice(
            1..1,
            crate::lexer::lex_spanned(&capture)
                .into_iter()
                .map(|(token, _)| (token, hook.span.clone())),
        );
    }
    Ok(KernelDefinition {
        pass,
        bindings: sources,
    })
}

pub(super) fn validate(program: &Program, pass: &PassDecl) -> Result<(), Vec<Diag>> {
    let operation = pass.operation.as_ref().expect("operation");
    let compute = operation.compute.as_ref().expect("compute definition");
    if operation.raster.is_some()
        || operation.visibility.is_some()
        || operation.sort_position.is_some()
        || !operation.attachments.is_empty()
        || !pass.state.is_empty()
        || pass.draw.is_some()
        || pass.blend.is_some()
    {
        return Err(fail(
            &pass.span,
            "compute operations cannot declare raster state or attachments",
        ));
    }
    if !pass.bindings.is_empty()
        || !pass.attrs.is_empty()
        || pass.stage.is_some()
        || !pass.reads.is_empty()
        || !pass.writes.is_empty()
        || !pass.permutations.is_empty()
        || !pass.requirements.is_empty()
    {
        return Err(fail(
            &pass.span,
            "compute resources must be explicit operation inputs and owned outputs",
        ));
    }
    let output = compute.output.as_ref().ok_or_else(|| {
        fail(
            &pass.span,
            "compute operation requires one typed output allocation",
        )
    })?;
    if output.name.contains('.') || operation.inputs.iter().any(|p| p.name == output.name) {
        return Err(fail(&output.span, "output requires a unique simple name"));
    }
    let returned = resource_type(&compute.return_ty.node, "read")
        .map_err(|m| fail(&compute.return_ty.span, m))?;
    let written = resource_type(&output.ty, "write").map_err(|m| fail(&output.span, m))?;
    if returned != written {
        return Err(fail(
            &output.span,
            "returned resource type does not match the owned output",
        ));
    }
    if compute
        .returned
        .as_ref()
        .is_none_or(|r| r.node != output.name)
    {
        return Err(fail(
            &pass.span,
            "compute must return its owned output by name",
        ));
    }
    let dimensions = match written {
        ResourceType::Buffer(_) => 1,
        ResourceType::Image(_) => 2,
    };
    if output.extents.len() != dimensions {
        return Err(fail(
            &output.span,
            format!("output allocation requires {dimensions} logical extents"),
        ));
    }
    let threads = compute.threads.as_ref().ok_or_else(|| {
        fail(
            &pass.span,
            "compute operation requires dispatch threads(x, y, z)",
        )
    })?;
    if threads.len() != 3 {
        return Err(fail(
            &pass.span,
            "dispatch threads requires three logical extents",
        ));
    }
    for value in output.extents.iter().chain(threads) {
        extent(program, operation, value)?;
    }
    let workgroup = compute
        .workgroup_size
        .as_ref()
        .ok_or_else(|| fail(&pass.span, "compute operation requires workgroup_size"))?;
    if workgroup.len() != 3 {
        return Err(fail(
            &pass.span,
            "workgroup_size requires three constant dimensions",
        ));
    }
    let mut product = 1u32;
    for value in workgroup {
        let parameter = GlobalParamDecl {
            name: "__compute_workgroup".into(),
            name_span: value.span.clone(),
            ty_name: "u32".into(),
            ty_span: value.span.clone(),
            default: None,
            range: None,
            span: value.span.clone(),
        };
        let evaluated = crate::check::compute::style_parameter_value(program, &parameter, value)?;
        let dimension = evaluated
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| {
                fail(
                    &value.span,
                    "workgroup dimensions must be positive u32 constants",
                )
            })?;
        product = product
            .checked_mul(dimension)
            .ok_or_else(|| fail(&value.span, "workgroup size product exceeds u32"))?;
    }
    for requirement in &operation.requirements {
        if host_type(program, operation, requirement)? != "bool" {
            return Err(fail(
                &requirement.span,
                "compute preconditions require bool expressions",
            ));
        }
    }
    let entries: Vec<_> = pass
        .hooks
        .iter()
        .filter(|h| h.attrs.iter().any(|a| a.name == "compute"))
        .collect();
    let [entry] = entries.as_slice() else {
        return Err(fail(
            &pass.span,
            "compute operation requires exactly one @compute entry",
        ));
    };
    if entry.params.len() != 1
        || entry.params[0].ty_name != "uvec3"
        || entry.return_ty.is_some()
        || entry.attrs.len() != 1
        || !entry.attrs[0].args.is_empty()
        || !entry.params[0].attrs.is_empty()
    {
        return Err(fail(
            &entry.span,
            "compute entry requires one uvec3 invocation ID and no return value or additional attributes",
        ));
    }
    for hook in &pass.hooks {
        if hook.params.iter().any(|p| {
            p.name.starts_with("__fresco_")
                || p.name == output.name
                || operation.inputs.iter().any(|i| i.name == p.name)
        }) {
            return Err(fail(
                &hook.span,
                "compute shader parameters cannot shadow operation resources or inputs",
            ));
        }
        let body = crate::parser::executable_pass_hook_body(hook)
            .map_err(|e| fail(&hook.span, format!("invalid compute shader body: {e:?}")))?;
        fn captures(
            body: &[Stmt],
            operation: &StyleOperation,
            output: &ComputeOutput,
        ) -> Result<(), String> {
            for statement in body {
                let name = match statement {
                    Stmt::Let { name, .. } | Stmt::Const { name, .. } | Stmt::For { name, .. } => {
                        Some(name)
                    }
                    _ => None,
                };
                if name.is_some_and(|name| {
                    name.starts_with("__fresco_")
                        || name == &output.name
                        || operation.inputs.iter().any(|i| &i.name == name)
                }) {
                    return Err("compute shader local shadows an operation input or compiler-owned resource".into());
                }
                match statement {
                    Stmt::If {
                        then_body,
                        else_body,
                        ..
                    } => {
                        captures(then_body, operation, output)?;
                        if let Some(body) = else_body {
                            captures(body, operation, output)?;
                        }
                    }
                    Stmt::For { body, .. } | Stmt::Block { body, .. } => {
                        captures(body, operation, output)?;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        captures(&body, operation, output).map_err(|m| fail(&hook.span, m))?;
    }
    let shader = shader_definition(program, pass)?;
    shader.emit(program).map_err(|m| fail(&pass.span, m))?;
    Ok(())
}

fn attribute(name: &str, args: Vec<String>, span: &Span) -> PipelineAttribute {
    PipelineAttribute {
        name: name.into(),
        args,
        expressions: vec![],
        name_span: span.clone(),
        args_span: None,
        span: span.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;

    const BUFFERS: &str = r#"
struct ResultVertex { position: vec3, weight: f32 }
compute Generate(count: u32, enabled: bool) -> buffer<ResultVertex, read> {
    output vertices: buffer<ResultVertex, write>(count)
    workgroup_size: (64, 1, 1)
    dispatch threads(count, 1, 1)
    @compute fn main(id: uvec3) {
        let weight = select(0.0, 1.0, enabled)
        vertices[id.x] = ResultVertex(vec3(f32(id.x)), weight)
    }
    return vertices
}
compute Copy(source: buffer<u32, read>) -> buffer<u32, read> {
    output values: buffer<u32, write>(source.count)
    workgroup_size: (64, 1, 1)
    dispatch threads(source.count, 1, 1)
    @compute fn main(id: uvec3) { values[id.x] = source[id.x] + 1u }
    return values
}
"#;
    fn parse(source: &str) -> Program {
        let tokens = crate::lexer::lex_spanned(source);
        crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap()
    }

    #[test]
    fn compute_image_queries_depend_on_metadata_without_reading_texels() {
        for (value, data) in [
            ("textureDimensions(source).x", false),
            (
                "u32(source.load(id.xy).r) + textureDimensions(source).x",
                true,
            ),
        ] {
            let source = format!(
                r#"
compute Query(source: texture2d<r32float, read>) -> buffer<u32, read> {{
    output result: buffer<u32, write>(source.width)
    workgroup_size: (64, 1, 1)
    dispatch threads(source.width, 1, 1)
    @compute fn main(id: uvec3) {{ result[id.x] = {value} }}
    return result
}}
"#
            );
            let program = parse(&source);
            validate(&program, &program.passes[0]).unwrap();
            let (_, reflected) = shader_definition(&program, &program.passes[0])
                .unwrap()
                .emit(&program)
                .unwrap();
            let entry = &reflected.entries[0].entry;
            let reads = reflected.compute_shader_reads(entry).unwrap();
            assert_eq!(reads.data.contains("source"), data);
            assert_eq!(reads.dimensions.contains("source"), !data);
            let binding = reflected
                .bindings
                .iter()
                .find(|b| b.name == "source")
                .unwrap();
            assert_eq!(binding.query_only_entries.contains(entry), !data);
            assert_eq!(
                binding.entry_access.get(entry).map(String::as_str),
                Some("read")
            );
        }
    }

    #[test]
    fn compute_geometry_binding_roles_follow_declared_members() {
        let source = r#"
struct Vertex { position: vec4 }
struct Bounds { low: vec3, high: vec3, available: bool }
@geometry(points, lookup, total, index_total, box)
resource Shape {
    total: u32,
    box: Bounds,
    points: buffer<Vertex, read>,
    index_total: u32,
    lookup: buffer<u32, read>
}
compute Project(mesh: Shape, enabled: bool) -> buffer<vec4, read> {
    output result: buffer<vec4, write>(mesh.total)
    workgroup_size: (64, 1, 1)
    dispatch threads(mesh.total, 1, 1)
    @compute fn main(id: uvec3) {
        let fresco_bounds_mesh = vec4(0.0)
        result[id.x] = select(fresco_bounds_mesh, mesh.points[id.x].position, enabled)
    }
    return result
}
"#;
        let mut program = parse(source);
        super::super::prepared_geometry::collect(&mut program).unwrap();
        validate(&program, &program.passes[0]).unwrap();
        let kernel = shader_definition(&program, &program.passes[0]).unwrap();
        let (_, reflected) = kernel.emit(&program).unwrap();
        let reads = reflected
            .compute_shader_reads(&reflected.entries[0].entry)
            .unwrap();
        assert_eq!(reads.data, BTreeSet::from(["mesh".into()]));
        assert_eq!(reads.values, BTreeSet::from(["enabled".into()]));
        assert!(reads.dimensions.is_empty());
        assert!(
            reflected
                .compute_bindings
                .values()
                .any(|source| matches!(source,
            BindingSource::Geometry { parameter, member, role: GeometryRole::Vertices }
            if parameter == "mesh" && member == "points"))
        );
        assert!(
            reflected
                .compute_bindings
                .values()
                .any(|source| matches!(source,
            BindingSource::Value { parameter, ty } if parameter == "enabled" && ty == "bool"))
        );
    }

    #[test]
    fn compute_binding_provenance_distinguishes_data_dimensions_and_unused_inputs() {
        for (body, helper, data, dimensions) in [
            ("source[id.x]", "", true, false),
            ("vec4(f32(source.count))", "", false, true),
            ("vec4(f32((source).count))", "", false, true),
            ("vec4(1.0)", "", false, false),
            (
                "fetch(id.x)",
                "fn fetch(index: u32) -> vec4 { return source[index] }",
                true,
                false,
            ),
        ] {
            let source = format!(
                r#"
compute Build(source: buffer<vec4, read>, unused: buffer<vec4, read>) -> buffer<vec4, read> {{
    output result: buffer<vec4, write>(source.count)
    workgroup_size: (64, 1, 1)
    dispatch threads(source.count, 1, 1)
    @compute fn main(id: uvec3) {{ result[id.x] = {body} }}
    {helper}
    return result
}}
"#
            );
            let program = parse(&source);
            validate(&program, &program.passes[0]).unwrap();
            let kernel = shader_definition(&program, &program.passes[0]).unwrap();
            let (_, reflected) = kernel.emit(&program).unwrap();
            let entry = &reflected.entries[0].entry;
            let reads = reflected.compute_shader_reads(entry).unwrap();
            assert_eq!(reads.data.contains("source"), data, "{body}");
            assert_eq!(reads.dimensions.contains("source"), dimensions, "{body}");
            assert!(!reads.data.contains("unused"));
            assert!(!reads.dimensions.contains("unused"));
            let serialized = serde_json::to_string(&reflected).unwrap();
            let mut restored: ManifestGpuProgram = serde_json::from_str(&serialized).unwrap();
            assert_eq!(restored.compute_shader_reads(entry).unwrap(), reads);
            // The consumer follows explicit roles even if every emitted binding
            // identifier changes. No prefix/suffix convention carries semantics.
            for (index, binding) in restored.bindings.iter_mut().enumerate() {
                let role = restored.compute_bindings.remove(&binding.name).unwrap();
                binding.name = format!("arbitrary_slot_{index}");
                restored.compute_bindings.insert(binding.name.clone(), role);
            }
            assert_eq!(restored.compute_shader_reads(entry).unwrap(), reads);
            restored.compute_bindings.pop_first();
            assert!(restored.compute_shader_reads(entry).is_err());
        }
    }

    #[test]
    fn compute_kernels_are_typed_without_calls_and_always_guard_logical_dispatch() {
        let program = parse(BUFFERS);
        for definition in &program.passes {
            validate(&program, definition).unwrap();
            let pass = shader_definition(&program, definition).unwrap();
            let (code, reflection) = pass.emit(&program).unwrap();
            let module = naga::front::wgsl::parse_str(&code).unwrap();
            let body = &module.entry_points[0].function.body;
            let guard = body.iter().position(|s| matches!(s, naga::Statement::If { accept, .. } if accept.iter().any(|s| matches!(s, naga::Statement::Return { value: None })))).unwrap();
            let store = body
                .iter()
                .position(|s| matches!(s, naga::Statement::Store { .. }))
                .unwrap();
            assert!(guard < store, "guard must precede authored writes");
            assert_eq!(reflection.workgroup_size, Some([64, 1, 1]));
            let output = reflection
                .bindings
                .iter()
                .find(|b| b.access == "write")
                .unwrap();
            assert_eq!(
                output.element_stride,
                Some(if definition.name == "Generate" { 16 } else { 4 })
            );
            assert!(output.entry_access.values().all(|a| a == "write"));
            assert!(
                reflection
                    .bindings
                    .iter()
                    .any(|b| b.name == "__fresco_dispatch"
                        && b.entry_access.values().any(|a| a == "read"))
            );
        }
    }

    #[test]
    fn compute_resource_formats_follow_shared_registry() {
        use fresco_artifact::types::{ImageFormat, ImageUse};
        for format in ImageFormat::ALL {
            let info = format.info();
            for (access, usage) in [("read", ImageUse::Sampled), ("write", ImageUse::Storage)] {
                let result = resource_type(&format!("texture2d<{}, {access}>", info.name), access);
                assert_eq!(
                    result.is_ok(),
                    info.supports(usage),
                    "{} {access}",
                    info.name
                );
            }
        }
    }

    #[test]
    fn compute_image_methods_preserve_format_and_access() {
        for (format, scalar) in [
            ("r32float", "f32"),
            ("r32uint", "u32"),
            ("r32sint", "i32"),
            ("rg32float", "f32"),
            ("rgba16uint", "u32"),
            ("r8unorm", "f32"),
        ] {
            let source = format!(
                r#"
compute Filter(source: texture2d<{format}, read>, gain: {scalar}) -> texture2d<{format}, read> {{
    output field: texture2d<{format}, write>(source.width, source.height)
    workgroup_size: (8, 8, 1)
    dispatch threads(source.width, source.height, 1)
    @compute fn main(id: uvec3) {{ field.store(id.xy, source.load(id.xy).r * gain) }}
    return field
}}
"#
            );
            let program = parse(&source);
            validate(&program, &program.passes[0]).unwrap();
            let bad = parse(&source.replace(&format!("gain: {scalar}"), "gain: bool"));
            assert!(validate(&bad, &bad.passes[0]).is_err());
        }
    }

    #[test]
    fn compute_kernel_validation_rejects_captures_and_access_hazards() {
        for (from, to) in [
            ("source[id.x] + 1u", "undeclared_capture"),
            ("source[id.x] + 1u", "values[id.x]"),
            (
                "values[id.x] = source[id.x] + 1u",
                "source[id.x] = 0u; values[id.x] = 0u",
            ),
            (
                "values[id.x] = source[id.x] + 1u",
                "let source = 1u; values[id.x] = source",
            ),
            ("source[id.x] + 1u", "vec3(1.0)"),
            (
                "ResultVertex(vec3(f32(id.x)), weight)",
                "ResultVertex(vec3(f32(id.x)), true)",
            ),
        ] {
            let program = parse(&BUFFERS.replace(from, to));
            assert!(
                program
                    .passes
                    .iter()
                    .any(|pass| validate(&program, pass).is_err()),
                "accepted {to}"
            );
        }
    }
}
