//! Validate engine graph contracts before lowering their explicit integration points.
use crate::{ast::*, diag::Diag};
use fresco_artifact::{ManifestRecipeResource, ManifestRecipeStep};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn path(expr: &SExpr) -> Result<String, String> {
    match &expr.node {
        Expr::Var(name) => Ok(name.clone()),
        Expr::Member(base, member) => Ok(format!("{}.{}", path(base)?, member)),
        _ => Err("expected a declared resource or node name".into()),
    }
}
fn one(field: &StyleGraphField) -> Result<&SExpr, String> {
    match field.values.as_slice() {
        [value] => Ok(value),
        _ => Err(format!("`{}` requires one value", field.name)),
    }
}
/// A semantic port is an engine-declared name for the attachment versions at a
/// checked integration boundary. It is not a physical resource name.
pub(super) fn port(point: &StylePointDecl) -> Result<Option<&str>, String> {
    let mut declarations = point.fields.iter().filter(|field| field.name == "port");
    let Some(field) = declarations.next() else {
        return Ok(None);
    };
    if declarations.next().is_some() {
        return Err("duplicate integration policy `port`".into());
    }
    match &one(field)?.node {
        Expr::Var(name) if !matches!(name.as_str(), "self" | "material") && !name.contains('.') => {
            Ok(Some(name.as_str()))
        }
        _ => Err("resource port requires a single binding name".into()),
    }
}
fn fields(values: &[StyleGraphField]) -> Result<BTreeMap<&str, &StyleGraphField>, String> {
    let mut result = BTreeMap::new();
    for field in values {
        if result.insert(field.name.as_str(), field).is_some() {
            return Err(format!(
                "duplicate field `{}` at {:?}",
                field.name, field.span
            ));
        }
    }
    Ok(result)
}
fn get<'a>(
    fields: &BTreeMap<&str, &'a StyleGraphField>,
    name: &str,
) -> Result<&'a StyleGraphField, String> {
    fields
        .get(name)
        .copied()
        .ok_or_else(|| format!("missing integration field `{name}`"))
}
fn names(field: &StyleGraphField) -> Result<Vec<String>, String> {
    field.values.iter().map(path).collect()
}
fn all(field: &StyleGraphField) -> Result<Vec<String>, String> {
    match &one(field)?.node {
        Expr::Call {
            name,
            args,
            const_args,
            ..
        } if name == "all" && !args.is_empty() && const_args.is_empty() => args
            .iter()
            .map(|arg| {
                if arg.name.is_some() {
                    return Err("all() takes positional node names".into());
                }
                path(&arg.value)
            })
            .collect(),
        _ => Err(format!(
            "`{}` must bind all(node, ...) view completion",
            field.name
        )),
    }
}
fn compact(ty: &str) -> String {
    ty.chars().filter(|c| !c.is_whitespace()).collect()
}
pub(super) fn attachment(ty: &str) -> Result<(String, String), String> {
    let ty = compact(ty);
    let parts: Vec<_> = ty
        .strip_prefix("attachment<")
        .and_then(|s| s.strip_suffix('>'))
        .ok_or_else(|| {
            format!("integration targets require attachment<format, access>, found `{ty}`")
        })?
        .split(',')
        .collect();
    let [format, access, ..] = parts.as_slice() else {
        return Err("attachment requires format and access".into());
    };
    if parts.len() > 3 || parts.get(2).is_some_and(|samples| *samples != "1") {
        return Err("integration attachments currently require sample count 1".into());
    }
    let info = fresco_artifact::types::ImageFormat::parse(format)
        .ok_or_else(|| format!("unknown attachment format `{format}`"))?
        .info();
    let depth = info.supports(fresco_artifact::types::ImageUse::DepthStencilAttachment);
    if (depth && *access != "test_only") || (!depth && *access != "preserve_update") {
        return Err(format!(
            "unsupported attachment access `{access}` for `{format}`"
        ));
    }
    Ok(((*format).into(), (*access).into()))
}
fn resource_type(program: &Program, ty: &str) -> Result<(), String> {
    let ty = compact(ty);
    if ty == "draw_instance_id" {
        return Ok(());
    }
    if program.resource_types.iter().any(|r| r.name == ty) {
        return Ok(());
    }
    if program
        .interfaces
        .iter()
        .any(|i| i.name == ty && i.entry.is_none())
    {
        return Ok(());
    }
    let inner = ty
        .strip_prefix("uniform<")
        .or_else(|| ty.strip_prefix("buffer<"))
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(&ty);
    if matches!(
        inner,
        "f32" | "u32" | "i32" | "vec2" | "vec3" | "vec4" | "mat4"
    ) || program.structs.iter().any(|s| s.name == inner)
    {
        Ok(())
    } else {
        Err(format!(
            "unknown or unsupported contract resource type `{ty}`"
        ))
    }
}
pub(super) fn precedes(steps: &[ManifestRecipeStep], before: &str, after: &str) -> bool {
    let mut pending = vec![after];
    let mut seen = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if name == before {
            return true;
        }
        if seen.insert(name)
            && let Some(step) = steps.iter().find(|s| s.name == name)
        {
            pending.extend(step.after.iter().map(String::as_str));
        }
    }
    false
}
fn attr(name: &str, args: Vec<String>) -> PipelineAttribute {
    PipelineAttribute {
        name: name.into(),
        args,
        expressions: vec![],
        name_span: 0..0,
        args_span: None,
        span: 0..0,
    }
}
pub(super) fn bindings<'a>(
    program: &'a Program,
    step: &ManifestRecipeStep,
    factory_override: Option<&str>,
) -> Vec<&'a PassBindingDecl> {
    let Some(pass) = program.passes.iter().find(|p| p.name == step.pass) else {
        return vec![];
    };
    let mut result: Vec<_> = pass.bindings.iter().collect();
    let declared = pass
        .attrs
        .iter()
        .find(|a| a.name == "factory")
        .and_then(|a| a.args.first())
        .map(String::as_str);
    let selected = if step.is_draw_scoped() {
        factory_override.or(declared)
    } else {
        declared
    };
    if let Some(factory) =
        selected.and_then(|name| program.vertex_factories.iter().find(|f| f.name == name))
    {
        result.extend(&factory.bindings);
    }
    result
}

fn storage_writes(
    program: &Program,
    step: &ManifestRecipeStep,
    resource: &str,
    factory: Option<&str>,
) -> bool {
    bindings(program, step, factory).iter().any(|binding| {
        step.bindings
            .get(&binding.name)
            .is_some_and(|bound| bound == resource)
            && binding.attrs.iter().any(|attribute| {
                attribute.name == "access"
                    && attribute
                        .args
                        .iter()
                        .any(|access| matches!(access.as_str(), "write" | "read_write"))
            })
    })
}

pub(super) fn bound_resource(
    program: &Program,
    resources: &[ManifestRecipeResource],
    steps: &[ManifestRecipeStep],
    expression: &SExpr,
    expected: &str,
    factory: Option<&str>,
) -> Result<String, String> {
    if program.interfaces.iter().any(|i| i.name == expected) {
        return super::shader_services::source(
            program, resources, steps, expression, expected, factory,
        )
        .map(|source| source.node);
    }
    if let Expr::Call {
        name,
        args,
        const_args,
        ..
    } = &expression.node
    {
        if !matches!(name.as_str(), "shader_output" | "draw_data")
            || !const_args.is_empty()
            || args.len() != 1
            || args[0].name.is_some()
        {
            return Err("capability sources require shader_output(node.entry.member) or draw_data(node.binding)".into());
        }
        let endpoint = path(&args[0].value)?;
        let parts: Vec<_> = endpoint.split('.').collect();
        let step = steps
            .iter()
            .find(|s| Some(&s.name.as_str()) == parts.first())
            .ok_or_else(|| format!("unknown capability producer `{endpoint}`"))?;
        if name == "draw_data" {
            let [_, binding_name] = parts.as_slice() else {
                return Err("draw_data requires node.binding".into());
            };
            if !step.is_draw_scoped() {
                return Err("draw_data requires a draw-scoped recipe node".into());
            }
            let binding = bindings(program, step, factory)
                .into_iter()
                .find(|b| b.name == *binding_name)
                .ok_or_else(|| format!("unknown draw data binding `{endpoint}`"))?;
            if !binding
                .attrs
                .iter()
                .any(|a| a.name == "draw_data" && a.args.as_slice() == ["instance_id"])
                || binding.attrs.iter().any(|a| a.name == "source")
                || binding.value_signature.as_deref().map(compact).as_deref()
                    != Some("uniform<u32>")
                || compact(expected) != "draw_instance_id"
                || step.bindings.contains_key(*binding_name)
            {
                return Err(format!(
                    "`{endpoint}` must provide typed per-draw instance data, not a material table or engine resource"
                ));
            }
        } else {
            let [_, entry, member] = parts.as_slice() else {
                return Err("shader_output requires node.entry.member".into());
            };
            let vertex = step.vertex.as_deref() == Some(*entry);
            let fragment = step.domain != "compute" && step.entry == *entry;
            if !vertex && !fragment {
                return Err(format!("`{endpoint}` is not an invoked raster entry"));
            }
            let pass = program
                .passes
                .iter()
                .find(|p| p.name == step.pass)
                .ok_or_else(|| format!("unknown capability pass `{}`", step.pass))?;
            let hook = pass
                .hooks
                .iter()
                .find(|h| h.name == *entry)
                .ok_or_else(|| format!("unknown capability entry `{endpoint}`"))?;
            let stage = if vertex { "vertex" } else { "fragment" };
            if !hook.attrs.iter().any(|a| a.name == stage) {
                return Err(format!("`{endpoint}` is not a {stage} output"));
            }
            let record = hook
                .return_ty
                .as_ref()
                .and_then(|ty| program.structs.iter().find(|s| s.name == ty.node))
                .ok_or_else(|| format!("`{endpoint}` requires a record output"))?;
            let field = record
                .fields
                .iter()
                .find(|f| f.name == *member)
                .ok_or_else(|| format!("unknown shader output `{endpoint}`"))?;
            if compact(&field.ty_name) != compact(expected) {
                return Err(format!(
                    "shader output `{endpoint}` requires `{expected}`, found `{}`",
                    field.ty_name
                ));
            }
            if fragment {
                let location = field
                    .attrs
                    .iter()
                    .find(|(name, _)| name == "location")
                    .and_then(|(_, args)| args.first())
                    .and_then(|s| s.parse::<u32>().ok())
                    .ok_or("fragment capability source requires an attachment location")?;
                let target = step
                    .colors
                    .get(&location)
                    .and_then(|name| resources.iter().find(|r| r.name == *name))
                    .ok_or("shader output capability has no attachment")?;
                if step
                    .attachments
                    .get(&target.name)
                    .is_some_and(|ops| !ops.store)
                {
                    return Err("shader output capability discards its attachment".into());
                }
                if !target
                    .format
                    .as_deref()
                    .and_then(fresco_artifact::types::ImageFormatRequirement::parse)
                    .is_some_and(|format| format.accepts_shader_output(expected))
                {
                    return Err("shader output capability has an incompatible attachment".into());
                }
            }
        }
        return Ok(endpoint);
    }
    let name = path(expression)?;
    if program.resource_types.iter().any(|r| r.name == expected) {
        super::prepared_geometry::producer(program, steps, &name, expected, factory)?;
        return Ok(name);
    }
    let (endpoint, binding_name) = name
        .split_once('.')
        .map_or((None, None), |(s, b)| (Some(s), Some(b)));
    let resource = if let (Some(node), Some(binding)) = (endpoint, binding_name) {
        steps
            .iter()
            .find(|s| s.name == node)
            .and_then(|s| s.bindings.get(binding))
            .cloned()
            .ok_or_else(|| format!("unknown provider endpoint `{name}`"))?
    } else {
        name.clone()
    };
    let declaration = resources
        .iter()
        .find(|r| r.name == resource)
        .ok_or_else(|| format!("unknown provider resource `{resource}`"))?;
    let expected = compact(expected);
    let mut found = false;
    for step in steps
        .iter()
        .filter(|s| endpoint.is_none_or(|node| s.name == node))
    {
        for (name, value) in &step.bindings {
            if *value != resource || binding_name.is_some_and(|b| b != name) {
                continue;
            }
            let signature = bindings(program, step, factory)
                .into_iter()
                .find(|b| b.name == *name)
                .and_then(|b| b.value_signature.as_ref())
                .ok_or_else(|| format!("untyped provider endpoint `{}.{name}`", step.name))?;
            let actual = compact(signature);
            if actual != expected && actual != format!("uniform<{expected}>") {
                return Err(format!(
                    "provider resource `{resource}` requires `{expected}`, found `{actual}` at `{}.{name}`",
                    step.name
                ));
            }
            found = true;
        }
    }
    if !found {
        return Err(format!(
            "provider resource `{resource}` has no typed consumer"
        ));
    }
    // Host resources have an external producer. Internal buffers need an ordered,
    // unconditional writer; allocation alone does not establish valid contents.
    if !matches!(declaration.kind.as_str(), "external" | "table_data") {
        let writers: Vec<_> = steps
            .iter()
            .filter(|step| {
                step.colors.values().any(|r| *r == resource)
                    || step.depth.as_ref() == Some(&resource)
                    || storage_writes(program, step, &resource, factory)
            })
            .collect();
        if !writers.iter().any(|writer| {
            writer.condition.is_none() && writer.attachments.get(&resource).is_none_or(|a| a.store)
        }) {
            return Err(format!(
                "provider resource `{resource}` has no unconditional producer"
            ));
        }
        for consumer in steps.iter().filter(|s| {
            s.bindings.values().any(|r| *r == resource) && !writers.iter().any(|w| w.name == s.name)
        }) {
            if !writers.iter().any(|w| {
                w.condition.is_none()
                    && w.attachments.get(&resource).is_none_or(|a| a.store)
                    && precedes(steps, &w.name, &consumer.name)
            }) {
                return Err(format!(
                    "provider resource `{resource}` has no ordered producer before `{}`",
                    consumer.name
                ));
            }
        }
    }
    Ok(resource)
}

fn declarations(program: &Program) -> Result<(), Vec<Diag>> {
    let mut capability_names = BTreeSet::new();
    for capability in &program.style_capabilities {
        let mut check = || -> Result<(), String> {
            if !capability_names.insert(&capability.name) {
                return Err(format!("duplicate capability `{}`", capability.name));
            }
            let mut names = BTreeSet::new();
            for member in &capability.members {
                if !names.insert(&member.name) {
                    return Err(format!(
                        "duplicate capability member `{}` at {:?}",
                        member.name, member.span
                    ));
                }
                resource_type(program, &member.ty)?;
            }
            Ok(())
        };
        check().map_err(|m| {
            vec![Diag::error(capability.span.clone(), m).with_file(&capability.source_file)]
        })?;
    }
    for contract in &program.style_contracts {
        let check = || -> Result<(), String> {
            let mut names = BTreeSet::new();
            for hook in &contract.hooks {
                names.insert(&hook.signature.name);
            }
            for input in &contract.inputs {
                if !names.insert(&input.name) {
                    return Err(format!(
                        "duplicate contract member `{}` at {:?}",
                        input.name, input.span
                    ));
                }
                resource_type(program, &input.ty)?;
            }
            for capability in &contract.capabilities {
                if !names.insert(&capability.name) || !capability_names.contains(&capability.name) {
                    return Err(format!(
                        "duplicate or unknown capability `{}` at {:?}",
                        capability.name, capability.span
                    ));
                }
            }
            for point in &contract.points {
                if !names.insert(&point.name) {
                    return Err(format!(
                        "duplicate contract member `{}` at {:?}",
                        point.name, point.span
                    ));
                }
                let target = program
                    .structs
                    .iter()
                    .find(|s| s.name == point.ty)
                    .ok_or_else(|| format!("unknown integration target `{}`", point.ty))?;
                if target.fields.is_empty() {
                    return Err("integration targets require at least one attachment".into());
                }
                let mut members = BTreeSet::new();
                for field in &target.fields {
                    if !members.insert(&field.name) {
                        return Err(format!("duplicate target member `{}`", field.name));
                    }
                    attachment(&field.ty_name)?;
                }
                let f = fields(&point.fields)?;
                for key in f.keys() {
                    if !matches!(
                        *key,
                        "scope" | "accepts" | "composition" | "after" | "before" | "port"
                    ) {
                        return Err(format!("unknown integration policy `{key}`"));
                    }
                }
                if path(one(get(&f, "scope")?)?)? != "view" {
                    return Err("integration scope currently requires `view`".into());
                }
                if path(one(get(&f, "accepts")?)?)? != "raster_draws" {
                    return Err("integration point currently accepts only raster_draws".into());
                }
                match &one(get(&f,"composition")?)?.node {
                    Expr::Var(name) if name == "global_transparent_queue" => {}
                    Expr::Call { name, args, const_args, .. } if name == "ordered_draws" && args.len() == 1 && args[0].name.is_none() && const_args.is_empty() && path(&args[0].value)? == "engine.stable_draw_order" => {}
                    _ => return Err("unsupported writer composition; expected ordered_draws(engine.stable_draw_order) or global_transparent_queue".into()),
                }
                let mut phases = BTreeSet::new();
                for phase in names_from_policies(&f)? {
                    if !phases.insert(phase.clone()) || members.contains(&phase) {
                        return Err(format!(
                            "duplicate integration phase or attachment `{phase}`"
                        ));
                    }
                }
            }
            let mut ports = BTreeSet::new();
            for point in &contract.points {
                if let Some(port) = port(point)? {
                    if names.iter().any(|name| name.as_str() == port) || !ports.insert(port) {
                        return Err(format!("duplicate contract resource port `{port}`"));
                    }
                    for used in &contract.capabilities {
                        if program.style_capabilities.iter().any(|capability| {
                            capability.name == used.name
                                && capability
                                    .members
                                    .iter()
                                    .any(|member| member.name.split('.').next() == Some(port))
                        }) {
                            return Err(format!(
                                "resource port `{port}` shadows a capability binding"
                            ));
                        }
                    }
                    for style in program
                        .styles
                        .iter()
                        .filter(|style| style.contract == contract.name)
                    {
                        if style
                            .params
                            .iter()
                            .chain(&style.static_params)
                            .any(|parameter| parameter.name == port)
                            || style.shading_inputs.iter().any(|input| input.name == port)
                        {
                            return Err(format!(
                                "style `{}` shadows resource port `{port}`",
                                style.name
                            ));
                        }
                    }
                }
            }
            Ok(())
        };
        check().map_err(|m| {
            vec![Diag::error(contract.name_span.clone(), m).with_file(&contract.source_file)]
        })?;
    }
    Ok(())
}
fn names_from_policies(f: &BTreeMap<&str, &StyleGraphField>) -> Result<Vec<String>, String> {
    let mut result = names(get(f, "after")?)?;
    result.extend(names(get(f, "before")?)?);
    Ok(result)
}

struct CheckedProvider {
    attrs: Vec<PipelineAttribute>,
    ports: Vec<fresco_artifact::ManifestResourcePort>,
}

fn provider(program: &Program, provider: &StyleProviderDecl) -> Result<CheckedProvider, String> {
    let contract = program
        .style_contracts
        .iter()
        .find(|c| c.name == provider.contract)
        .ok_or_else(|| format!("unknown provider contract `{}`", provider.contract))?;
    let pipeline = program
        .pipelines
        .iter()
        .find(|p| p.name == provider.renderer && p.attrs.iter().any(|a| a.name == "renderer"))
        .ok_or_else(|| {
            format!(
                "provider requires renderer pipeline `{}`",
                provider.renderer
            )
        })?;
    let (resources, steps) = super::recipes::reflect(pipeline)?;
    let mut inputs = BTreeMap::new();
    for (name, expression) in &provider.inputs {
        let declaration = contract
            .inputs
            .iter()
            .find(|i| i.name == *name)
            .ok_or_else(|| format!("unknown provider input `{name}`"))?;
        let resource = bound_resource(
            program,
            &resources,
            &steps,
            expression,
            &declaration.ty,
            None,
        )?;
        if inputs.insert(name, resource).is_some() {
            return Err(format!("duplicate provider input `{name}`"));
        }
    }
    for input in &contract.inputs {
        if !inputs.contains_key(&input.name) {
            return Err(format!("missing provider input `{}`", input.name));
        }
    }
    let mut blocks = BTreeMap::new();
    for block in &provider.blocks {
        if blocks.insert(&block.name, block).is_some() {
            return Err(format!(
                "duplicate provider block `{}` at {:?}",
                block.name, block.span
            ));
        }
        if !contract.points.iter().any(|p| p.name == block.name)
            && !contract.capabilities.iter().any(|c| c.name == block.name)
        {
            return Err(format!("unknown provider block `{}`", block.name));
        }
    }
    for capability in &contract.capabilities {
        let Some(block) = blocks.get(&capability.name) else {
            if capability.optional {
                continue;
            }
            return Err(format!("missing required capability `{}`", capability.name));
        };
        let declaration = program
            .style_capabilities
            .iter()
            .find(|c| c.name == capability.name)
            .expect("validated capability");
        let f = fields(&block.fields)?;
        for name in f.keys() {
            if *name != "factories" && !declaration.members.iter().any(|m| m.name == *name) {
                return Err(format!("unknown capability member `{name}`"));
            }
        }
        if let Some(factory_field) = f.get("factories") {
            for factory in names(factory_field)? {
                if !program.vertex_factories.iter().any(|f| f.name == factory) {
                    return Err(format!("unknown capability factory `{factory}`"));
                }
            }
        }
        for member in &declaration.members {
            bound_resource(
                program,
                &resources,
                &steps,
                one(get(&f, &member.name)?)?,
                &member.ty,
                None,
            )?;
        }
    }
    let mut lowered = Vec::new();
    let mut ports = Vec::new();
    for point in &contract.points {
        let Some(block) = blocks.get(&point.name) else {
            if point.optional {
                continue;
            }
            return Err(format!(
                "missing required integration point `{}`",
                point.name
            ));
        };
        let f = fields(&block.fields)?;
        let policy = fields(&point.fields)?;
        let transparent = matches!(&one(get(&policy, "composition")?)?.node, Expr::Var(name) if name == "global_transparent_queue");
        let phases = names_from_policies(&policy)?;
        let target = program
            .structs
            .iter()
            .find(|s| s.name == point.ty)
            .expect("validated target");
        for key in f.keys() {
            if *key != if transparent { "queue" } else { "order" }
                && !phases.iter().any(|p| p == key)
                && !target.fields.iter().any(|m| m.name == *key)
            {
                return Err(format!("unknown point binding `{key}`"));
            }
        }
        if !transparent && path(one(get(&f, "order")?)?)? != "stable_draw_order" {
            return Err("ordered_draws requires stable_draw_order provider".into());
        }
        let resolve = |role: &str| -> Result<Vec<String>, String> {
            let mut result = Vec::new();
            for phase in names(get(&policy, role)?)? {
                result.extend(all(get(&f, &phase)?)?);
            }
            for node in &result {
                if !steps.iter().any(|s| s.name == *node) {
                    return Err(format!("unknown integration node `{node}`"));
                }
            }
            Ok(result)
        };
        let incoming = resolve("after")?;
        let outgoing = resolve("before")?;
        let queue = transparent
            .then(|| path(one(get(&f, "queue")?)?))
            .transpose()?;
        let queued: Vec<_> = steps
            .iter()
            .filter(|step| queue.is_some() && step.transparent_queue == queue)
            .collect();
        if transparent && queued.is_empty() {
            return Err("transparent provider requires a declared renderer queue".into());
        }
        for member in &queued {
            if incoming
                .iter()
                .any(|node| *node == member.name || !precedes(&steps, node, &member.name))
                || outgoing
                    .iter()
                    .any(|node| *node == member.name || !precedes(&steps, &member.name, node))
            {
                return Err("transparent queue must lie between every incoming and outgoing integration boundary".into());
            }
        }
        for before in &incoming {
            for after in &outgoing {
                if before == after || !precedes(&steps, before, after) {
                    return Err(format!(
                        "integration point `{}` requires an existing path from `{before}` to `{after}`",
                        point.name
                    ));
                }
            }
        }
        let key = format!("{}::{}", contract.name, point.name);
        lowered.push(attr("__style_point", vec![key.clone()]));
        if let Some(queue) = &queue {
            lowered.push(attr("__style_queue", vec![key.clone(), queue.clone()]));
        }
        let mut join = vec![key.clone()];
        join.extend(incoming.clone());
        lowered.push(attr("__style_incoming", join));
        let mut next = vec![key.clone()];
        next.extend(outgoing.clone());
        lowered.push(attr("__style_outgoing", next));
        for input in &contract.inputs {
            if program.interfaces.iter().any(|i| i.name == input.ty) {
                continue;
            }
            lowered.push(attr(
                "__style_input",
                vec![
                    key.clone(),
                    input.name.clone(),
                    inputs[&input.name].clone(),
                    compact(&input.ty),
                ],
            ));
        }
        let mut attachment_resources = BTreeSet::new();
        for member in &target.fields {
            let resource = path(one(get(&f, &member.name)?)?)?;
            if !attachment_resources.insert(resource.clone()) {
                return Err(format!(
                    "integration attachments alias resource `{resource}`"
                ));
            }
            let (format, access) = attachment(&member.ty_name)?;
            let declaration = resources
                .iter()
                .find(|r| r.name == resource)
                .ok_or_else(|| format!("unknown integration attachment `{resource}`"))?;
            if declaration.kind != "image" || declaration.format.as_ref() != Some(&format) {
                return Err(format!(
                    "attachment `{resource}` requires format `{format}` and sample count 1"
                ));
            }
            let writers: Vec<_> = steps
                .iter()
                .filter(|s| {
                    s.colors.values().any(|r| *r == resource)
                        || s.depth.as_ref() == Some(&resource)
                        || storage_writes(program, s, &resource, None)
                })
                .collect();
            if let Some(port) = port(point)? {
                for (index, left) in writers.iter().enumerate() {
                    for right in writers.iter().skip(index + 1) {
                        let same_queue = left.transparent_queue.is_some()
                            && left.transparent_queue == right.transparent_queue;
                        if !same_queue
                            && !precedes(&steps, &left.name, &right.name)
                            && !precedes(&steps, &right.name, &left.name)
                        {
                            return Err(format!(
                                "resource port `{port}` has ambiguous writers `{}` and `{}` for `{resource}`",
                                left.name, right.name
                            ));
                        }
                    }
                }
                for consumer in steps.iter().filter(|step| {
                    step.bindings.values().any(|binding| binding == &resource)
                        || step.colors.values().any(|binding| binding == &resource)
                        || step.depth.as_ref() == Some(&resource)
                }) {
                    let before = incoming
                        .iter()
                        .any(|node| precedes(&steps, &consumer.name, node));
                    let after = outgoing
                        .iter()
                        .any(|node| precedes(&steps, node, &consumer.name));
                    let in_queue = queued.iter().any(|member| member.name == consumer.name);
                    if !before && !after && !in_queue {
                        return Err(format!(
                            "resource port `{port}` cannot place consumer `{}` of `{resource}` before or after its boundary",
                            consumer.name
                        ));
                    }
                    if before && after {
                        return Err(format!(
                            "resource port `{port}` has cyclic resource flow through `{}`",
                            consumer.name
                        ));
                    }
                    for writer in &writers {
                        let same_queue = consumer.transparent_queue.is_some()
                            && consumer.transparent_queue == writer.transparent_queue;
                        if !same_queue
                            && !precedes(&steps, &writer.name, &consumer.name)
                            && !precedes(&steps, &consumer.name, &writer.name)
                        {
                            return Err(format!(
                                "resource port `{port}` has unordered resource use between writer `{}` and consumer `{}`",
                                writer.name, consumer.name
                            ));
                        }
                    }
                }
            }
            if !writers.iter().any(|s| {
                s.condition.is_none()
                    && incoming.iter().any(|n| precedes(&steps, &s.name, n))
                    && (s.colors.values().any(|bound| *bound == resource)
                        || s.depth.as_ref() == Some(&resource))
                    && s.attachments
                        .get(&resource)
                        .is_none_or(|a| a.store && !a.load)
            }) {
                return Err(format!(
                    "attachment `{resource}` needs an unconditional stored producer before the integration point"
                ));
            }
            for writer in &writers {
                if outgoing.iter().any(|n| precedes(&steps, n, &writer.name))
                    && (writer.colors.values().any(|bound| *bound == resource)
                        || writer.depth.as_ref() == Some(&resource))
                    && !writer
                        .attachments
                        .get(&resource)
                        .is_some_and(|a| a.load && a.store)
                {
                    return Err(format!(
                        "downstream writer `{}` must preserve attachment `{resource}`",
                        writer.name
                    ));
                }
                if !queued.iter().any(|member| member.name == writer.name)
                    && !incoming.iter().any(|n| precedes(&steps, &writer.name, n))
                    && !outgoing.iter().any(|n| precedes(&steps, n, &writer.name))
                {
                    return Err(format!(
                        "attachment `{resource}` has unordered writer `{}`",
                        writer.name
                    ));
                }
                if incoming.iter().any(|n| precedes(&steps, &writer.name, n))
                    && writer.attachments.get(&resource).is_some_and(|a| !a.store)
                {
                    return Err(format!(
                        "attachment `{resource}` is discarded before integration"
                    ));
                }
            }
            if !(access == "test_only"
                && queued.iter().any(|s| s.depth.as_ref() == Some(&resource)))
                && !steps.iter().any(|s| {
                    outgoing.iter().any(|n| precedes(&steps, n, &s.name))
                        && (s.bindings.values().any(|r| *r == resource)
                            || s.colors.values().any(|r| *r == resource)
                            || s.depth.as_ref() == Some(&resource))
                })
            {
                return Err(format!(
                    "attachment `{resource}` has no downstream consumer"
                ));
            }
            lowered.push(attr(
                "__style_input",
                vec![
                    key.clone(),
                    member.name.clone(),
                    resource.clone(),
                    compact(&member.ty_name),
                ],
            ));
            if let Some(port) = port(point)? {
                let storage_writers = steps
                    .iter()
                    .filter(|step| storage_writes(program, step, &resource, None))
                    .map(|step| step.name.clone())
                    .collect();
                ports.push(fresco_artifact::ManifestResourcePort {
                    storage_writers,
                    point: key.clone(),
                    name: port.into(),
                    member: member.name.clone(),
                    resource,
                    access,
                    incoming: incoming.clone(),
                    outgoing: outgoing.clone(),
                    queue: queue.clone(),
                    versions: Vec::new(),
                });
            }
        }
        for member in &queued {
            let actual: BTreeSet<_> = member
                .colors
                .values()
                .chain(member.depth.iter())
                .cloned()
                .collect();
            if actual != attachment_resources {
                return Err("transparent provider target must match the queue attachments".into());
            }
        }
    }
    Ok(CheckedProvider {
        attrs: lowered,
        ports,
    })
}

pub(super) fn prepare(program: &mut Program) -> Result<(), Vec<Diag>> {
    declarations(program)?;
    let mut providers = BTreeSet::new();
    let mut lowered = Vec::new();
    for declaration in &program.style_providers {
        if !providers.insert((&declaration.contract, &declaration.renderer)) {
            return Err(vec![
                Diag::error(
                    declaration.span.clone(),
                    "duplicate contract provider for renderer",
                )
                .with_file(&declaration.source_file),
            ]);
        }
        let attrs = provider(program, declaration).map_err(|m| {
            vec![Diag::error(declaration.span.clone(), m).with_file(&declaration.source_file)]
        })?;
        lowered.push((declaration.renderer.clone(), attrs));
    }
    requirements(program)?;
    operation_inputs(program)?;
    for pipeline in &program.pipelines {
        let attrs: Vec<_> = lowered
            .iter()
            .filter(|(name, _)| *name == pipeline.name)
            .flat_map(|(_, provider)| &provider.attrs)
            .collect();
        let points: Vec<_> = attrs
            .iter()
            .filter(|a| a.name == "__style_point")
            .map(|a| &a.args[0])
            .collect();
        if points.len() < 2 {
            continue;
        }
        let (_, steps) = super::recipes::reflect(pipeline)
            .map_err(|m| vec![Diag::error(pipeline.span.clone(), m)])?;
        for (index, a) in points.iter().enumerate() {
            for b in points.iter().skip(index + 1) {
                let targets = |point: &String| -> BTreeSet<&String> {
                    attrs
                        .iter()
                        .filter(|p| {
                            p.name == "__style_input"
                                && p.args[0] == *point
                                && p.args[3].starts_with("attachment<")
                        })
                        .map(|p| &p.args[2])
                        .collect()
                };
                if targets(a).is_disjoint(&targets(b)) {
                    continue;
                }
                let queue = |point: &String| {
                    attrs
                        .iter()
                        .find(|p| p.name == "__style_queue" && p.args[0] == *point)
                        .map(|p| &p.args[1])
                };
                // Both providers already validated their attachments and phase
                // boundaries against this actual queue. Its global ordering is
                // the shared writer policy, irrespective of point names.
                if queue(a).is_some() && queue(a) == queue(b) {
                    continue;
                }
                let ordered = |first: &String, second: &String| {
                    let outgoing = attrs
                        .iter()
                        .find(|p| p.name == "__style_outgoing" && p.args[0] == *first)
                        .expect("lowered point");
                    let incoming = attrs
                        .iter()
                        .find(|p| p.name == "__style_incoming" && p.args[0] == *second)
                        .expect("lowered point");
                    if let Some(queue) = attrs
                        .iter()
                        .find(|p| p.name == "__style_queue" && p.args[0] == *second)
                    {
                        let members: Vec<_> = steps
                            .iter()
                            .filter(|step| step.transparent_queue.as_ref() == Some(&queue.args[1]))
                            .collect();
                        if !members.is_empty()
                            && members.iter().all(|member| {
                                outgoing.args[1..]
                                    .iter()
                                    .any(|node| precedes(&steps, node, &member.name))
                            })
                        {
                            return true;
                        }
                    }
                    outgoing.args[1..]
                        .iter()
                        .any(|n| incoming.args[1..].iter().all(|m| precedes(&steps, n, m)))
                };
                if !ordered(a, b) && !ordered(b, a) {
                    return Err(vec![Diag::error(
                        pipeline.span.clone(),
                        format!(
                            "integration points `{a}` and `{b}` share attachments without ordered boundaries"
                        ),
                    )]);
                }
            }
        }
    }
    for pipeline in &mut program.pipelines {
        for (_, provider) in lowered.iter().filter(|(name, _)| *name == pipeline.name) {
            pipeline.attrs.extend(provider.attrs.clone());
            pipeline.resource_ports.extend(provider.ports.clone());
        }
    }
    Ok(())
}

fn graph_requirement(expression: &SExpr) -> bool {
    matches!(&expression.node, Expr::Var(name) if !name.contains('.') && name != "true" && name != "false")
}

pub(super) fn requirements(program: &Program) -> Result<(), Vec<Diag>> {
    for style in &program.styles {
        let contract = program
            .style_contracts
            .iter()
            .find(|c| c.name == style.contract)
            .expect("validated style");
        for requirement in &style.requirements {
            if !graph_requirement(requirement) {
                validate_material_requirement(program, requirement)?;
                continue;
            }
            let name = path(requirement).map_err(|m| {
                vec![Diag::error(requirement.span.clone(), m).with_file(&style.source_file)]
            })?;
            if !contract.capabilities.iter().any(|c| c.name == name)
                && !contract.points.iter().any(|p| p.name == name)
            {
                return Err(vec![
                    Diag::error(
                        requirement.span.clone(),
                        format!("unknown style requirement `{name}`"),
                    )
                    .with_file(&style.source_file),
                ]);
            }
        }
        for surface in &program.surfaces {
            let Some(settings) = &surface.settings else {
                continue;
            };
            if !settings
                .implementations
                .iter()
                .any(|s| s.contract == style.contract && s.symbol == style.name)
            {
                continue;
            }
            for requirement in &style.requirements {
                if !graph_requirement(requirement) {
                    let bindings: Vec<_> = surface
                        .body
                        .iter()
                        .filter(|s| matches!(s, Stmt::Const { .. }))
                        .cloned()
                        .collect();
                    if !crate::check::compute::constant_bool(
                        program,
                        &bindings,
                        &predicate(requirement)?,
                    )? {
                        return Err(vec![
                            Diag::error(
                                requirement.span.clone(),
                                format!(
                                    "style `{}` precondition is not satisfied for surface `{}`",
                                    style.name, surface.name
                                ),
                            )
                            .with_file(&style.source_file),
                        ]);
                    }
                }
            }
            if contract.inputs.is_empty()
                && contract.capabilities.is_empty()
                && contract.points.is_empty()
            {
                continue;
            }
            let renderer = program
                .pipelines
                .iter()
                .find(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"))
                .ok_or_else(|| {
                    vec![
                        Diag::error(
                            style.name_span.clone(),
                            format!(
                                "style `{}` requires a `{}` provider and an executable renderer",
                                style.name, style.contract
                            ),
                        )
                        .with_file(&style.source_file),
                    ]
                })?;
            let provider = program
                .style_providers
                .iter()
                .find(|p| p.contract == style.contract && p.renderer == renderer.name)
                .ok_or_else(|| {
                    vec![
                        Diag::error(
                            style.name_span.clone(),
                            format!(
                                "style `{}` requires a `{}` provider for renderer `{}`",
                                style.name, style.contract, renderer.name
                            ),
                        )
                        .with_file(&style.source_file),
                    ]
                })?;
            let mut required: Vec<_> = style
                .requirements
                .iter()
                .filter(|r| graph_requirement(r))
                .cloned()
                .collect();
            required.extend(
                contract
                    .capabilities
                    .iter()
                    .filter(|c| !c.optional)
                    .map(|c| Spanned {
                        node: Expr::Var(c.name.clone()),
                        span: c.span.clone(),
                    }),
            );
            for requirement in &required {
                check_capability(
                    program,
                    &style.name,
                    &style.source_file,
                    surface,
                    renderer,
                    provider,
                    requirement,
                )?;
            }
        }
    }
    Ok(())
}

/// The factory domain used for both requirement checks and compatibility metadata.
pub(super) fn selected_factories<'a>(
    program: &'a Program,
    surface: &'a SurfaceDecl,
    renderer: Option<&'a PipelineDecl>,
) -> BTreeSet<&'a String> {
    let settings = surface.settings.as_ref().expect("selected settings");
    if settings.usages.is_empty() {
        renderer
            .into_iter()
            .flat_map(|r| &r.pass_refs)
            .filter_map(|r| program.passes.iter().find(|p| p.name == r.name))
            .flat_map(|p| &p.attrs)
            .filter(|a| a.name == "factory")
            .filter_map(|a| a.args.first())
            .collect()
    } else {
        settings.usages.iter().collect()
    }
}

/// Check a provider member for this exact material/factory context. Used by
/// selected requirements and reflection of optional, currently unused members.
pub(super) fn check_capability(
    program: &Program,
    style_name: &str,
    source_file: &str,
    surface: &SurfaceDecl,
    renderer: &PipelineDecl,
    provider: &StyleProviderDecl,
    requirement: &SExpr,
) -> Result<(), Vec<Diag>> {
    let name = path(requirement).expect("validated requirement");
    let block = provider
        .blocks
        .iter()
        .find(|b| b.name == name)
        .ok_or_else(|| {
            vec![
                Diag::error(
                    requirement.span.clone(),
                    format!(
                        "renderer `{}` does not provide `{name}` required by style `{}`",
                        renderer.name, style_name
                    ),
                )
                .with_file(source_file),
            ]
        })?;
    let allowed = block
        .fields
        .iter()
        .find(|f| f.name == "factories")
        .map(names)
        .transpose()
        .map_err(|m| vec![Diag::error(block.span.clone(), m).with_file(&provider.source_file)])?;
    let factories = selected_factories(program, surface, Some(renderer));
    for factory in factories {
        if allowed
            .as_ref()
            .is_some_and(|allowed| !allowed.contains(factory))
        {
            return Err(vec![Diag::error(requirement.span.clone(),format!("capability `{name}` unavailable for vertex factory `{factory}` on surface `{}`",surface.name)).with_file(source_file)]);
        }
        if let Some(capability) = program.style_capabilities.iter().find(|c| c.name == name) {
            let (resources, steps) =
                super::recipes::reflect(renderer).expect("validated renderer provider");
            let values = fields(&block.fields).expect("validated capability fields");
            for member in &capability.members {
                let expression = one(get(&values, &member.name).expect("validated member"))
                    .expect("validated member value");
                bound_resource(program,&resources,&steps,expression,&member.ty,Some(factory))
                .map_err(|m| vec![Diag::error(requirement.span.clone(),format!("capability `{name}` unavailable for vertex factory `{factory}`: {m}")).with_file(source_file)])?;
            }
        }
    }
    Ok(())
}

pub(super) fn validate_material_requirement(
    program: &Program,
    requirement: &SExpr,
) -> Result<(), Vec<Diag>> {
    let bindings: Vec<_> = program
        .templates
        .iter()
        .filter(|t| t.property_block.is_some())
        .flat_map(|t| &t.config_params)
        .filter(|p| !p.ty_name.starts_with("implementation<"))
        .filter_map(|p| {
            p.default.as_ref().map(|value| Stmt::Const {
                name: p.name.clone(),
                name_span: p.name_span.clone(),
                ty_name: p.ty_name.clone(),
                ty_span: p.ty_span.clone(),
                value: value.clone(),
            })
        })
        .collect();
    crate::check::compute::constant_bool(program, &bindings, &predicate(requirement)?)?;
    Ok(())
}

fn predicate(expression: &SExpr) -> Result<SExpr, Vec<Diag>> {
    let mut result = expression.clone();
    result.node = match &expression.node {
        Expr::Var(name) => Expr::Var(name.strip_prefix("material.").unwrap_or(name).into()),
        Expr::Member(base, name) if matches!(&base.node, Expr::Var(v) if v == "material") => {
            Expr::Var(name.clone())
        }
        Expr::Member(base, name) => Expr::Member(Box::new(predicate(base)?), name.clone()),
        Expr::Binary(op, a, b) => {
            Expr::Binary(*op, Box::new(predicate(a)?), Box::new(predicate(b)?))
        }
        Expr::Unary(op, a) => Expr::Unary(*op, Box::new(predicate(a)?)),
        Expr::Num(..) => expression.node.clone(),
        _ => {
            return Err(vec![Diag::error(
                expression.span.clone(),
                "style preconditions require a boolean expression over material properties",
            )]);
        }
    };
    Ok(result)
}

pub(super) fn operation_inputs(program: &Program) -> Result<(), Vec<Diag>> {
    for pipeline in &program.pipelines {
        for marker in pipeline
            .attrs
            .iter()
            .filter(|a| a.name == "__style_operation")
        {
            let [contract, _, point] = marker.args.as_slice() else {
                return Err(vec![Diag::error(
                    marker.span.clone(),
                    "invalid checked operation identity",
                )]);
            };
            let Some(contract) = program.style_contracts.iter().find(|c| c.name == *contract)
            else {
                return Err(vec![Diag::error(
                    marker.span.clone(),
                    "operation references an unknown typed contract",
                )]);
            };
            let check = || -> Result<(), String> {
                let point = contract
                    .points
                    .iter()
                    .find(|p| {
                        p.name == *point || format!("{}::{}", contract.name, p.name) == *point
                    })
                    .ok_or_else(|| format!("unknown contract integration point `{point}`"))?;
                let target = program
                    .structs
                    .iter()
                    .find(|s| s.name == point.ty)
                    .expect("validated target");
                let mut types: BTreeMap<_, _> = target
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), compact(&f.ty_name)))
                    .collect();
                types.extend(
                    contract
                        .inputs
                        .iter()
                        .map(|i| (i.name.clone(), compact(&i.ty))),
                );
                if pipeline.pass_refs.iter().any(|r| r.invocation.is_some()) {
                    types.insert(
                        format!("__settings_{}", contract.name),
                        "buffer<vec4>".into(),
                    );
                    for reference in &pipeline.pass_refs {
                        if let Some(pass) = program.passes.iter().find(|p| p.name == reference.name)
                        {
                            for capture in &pass.service_captures {
                                if types
                                    .insert(capture.name.clone(), compact(&capture.ty))
                                    .is_some()
                                {
                                    return Err("shader service capture conflicts with a contribution input".into());
                                }
                            }
                        }
                    }
                }
                let mut validation = pipeline.clone();
                validation.attrs.clear();
                for input in pipeline.attrs.iter().filter(|a| a.name == "input") {
                    let [name, ty] = input.args.as_slice() else {
                        return Err("contribution input requires name and type".into());
                    };
                    if types.get(name) != Some(&compact(ty)) {
                        return Err(format!(
                            "incompatible contribution input `{name}` of type `{ty}`"
                        ));
                    }
                    validation
                        .attrs
                        .push(attr("external", vec![name.clone(), ty.clone()]));
                }
                let (_, steps) = super::recipes::reflect(&validation)?;
                for step in &steps {
                    if step.domain != "mesh" {
                        return Err("integration point accepts raster_draws only".into());
                    }
                    for (binding, resource) in &step.bindings {
                        let expected = types
                            .get(resource)
                            .ok_or_else(|| format!("unknown contribution resource `{resource}`"))?;
                        let actual = bindings(program, step, None)
                            .into_iter()
                            .find(|b| b.name == *binding)
                            .and_then(|b| b.value_signature.as_ref())
                            .ok_or_else(|| {
                                format!(
                                    "unknown typed contribution binding `{}.{binding}`",
                                    step.pass
                                )
                            })?;
                        if compact(actual) != *expected
                            && compact(actual) != format!("uniform<{expected}>")
                        {
                            return Err(format!(
                                "contribution binding `{}.{binding}` requires `{expected}`, found `{actual}`",
                                step.pass
                            ));
                        }
                    }
                    for resource in step.colors.values().chain(step.depth.iter()) {
                        let ty = types.get(resource).ok_or_else(|| {
                            format!("unknown integration attachment `{resource}`")
                        })?;
                        let (format, access) = attachment(ty)?;
                        if (format == "depth32float") != (step.depth.as_ref() == Some(resource)) {
                            return Err(format!(
                                "attachment `{resource}` has incompatible draw access"
                            ));
                        }
                        if !step
                            .attachments
                            .get(resource)
                            .is_some_and(|ops| ops.load && ops.store)
                        {
                            return Err(format!(
                                "attachment `{resource}` must preserve and store its contents"
                            ));
                        }
                        if access == "test_only" {
                            let pass = program
                                .passes
                                .iter()
                                .find(|p| p.name == step.pass)
                                .ok_or_else(|| format!("unknown pass `{}`", step.pass))?;
                            let value = pass
                                .state
                                .iter()
                                .find(|(name, _)| name == "depth_write")
                                .map(|(_, v)| v);
                            if !value.is_some_and(|v| matches!(&v.node, Expr::Var(name) if matches!(name.as_str(),"false" | "off"))) { return Err(format!("test_only attachment requires pass `{}` to disable depth writes",step.pass)); }
                        }
                    }
                }
                Ok(())
            };
            check().map_err(|m| vec![Diag::error(marker.span.clone(), m)])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;
    const GRAPH: &str = r#"
struct ViewData { scale: f32 }
struct Target { color: attachment<rgba16float, preserve_update> }
capability Camera { view: ViewData }
contract Response for Value {
    input view: ViewData
    optional capability Camera
    point finish: Target {
        scope: view
        accepts: raster_draws
        composition: ordered_draws(engine.stable_draw_order)
        after: completed
        before: display
    }
    fn shade(x: f32) -> f32
}
@shader pass draw {
    stage: raster
    draw: fullscreen
    binding { @group(0) @binding(0) view: uniform<ViewData> }
}
@renderer("test", "Test") @image(target, rgba16float) @external(camera, camera)
pipeline(postprocess) renderer {
    @node(left) @draw(vertex, fragment, fullscreen, 3) @color(0, target) @bind(view, camera) draw
    @node(right) @after(left) @draw(vertex, fragment, fullscreen, 3) @color(0, target) @attachment(target, load, store) draw
    @node(display) @after(right) @draw(vertex, fragment, fullscreen, 3) @color(0, target) @attachment(target, load, store) draw
}
provide Response for renderer {
    view = camera
    Camera { view: left.view }
    finish {
        completed: all(left, right)
        display: all(display)
        color: target
        order: stable_draw_order
    }
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
    fn resource_ports_are_explicit_unique_contract_bindings() {
        let source = GRAPH.replace("scope: view", "port: opaque; scope: view");
        let mut program = parse(&source);
        prepare(&mut program).unwrap();
        assert_eq!(
            port(&program.style_contracts[0].points[0]).unwrap(),
            Some("opaque")
        );
        for (replacement, expected) in [
            ("port: view", "duplicate contract resource port"),
            ("port: finish", "duplicate contract resource port"),
            ("port: self", "single binding name"),
            ("port: opaque.color", "single binding name"),
            ("port: opaque; port: other", "duplicate field"),
        ] {
            let mut invalid = parse(&source.replace("port: opaque", replacement));
            let errors = prepare(&mut invalid).unwrap_err();
            assert!(format!("{errors:?}").contains(expected), "{errors:?}");
        }
    }
    #[test]
    fn resource_ports_require_ordered_readers_even_without_style_calls() {
        let ambiguous = GRAPH.replace("scope: view", "port: opaque; scope: view")
            .replace("    @node(display)", "    @node(other) @after(left) @draw(vertex, fragment, fullscreen, 3) @color(0, target) @attachment(target, load, store) draw\n    @node(display)");
        let errors = prepare(&mut parse(&ambiguous)).unwrap_err();
        assert!(
            format!("{errors:?}").contains("ambiguous writers"),
            "{errors:?}"
        );
        let reader = "@node(observer) @after(left) @draw(vertex, fragment, fullscreen, 3) @bind(view, target) draw\n";
        let source = GRAPH.replace(
            "    @node(display)",
            &format!("    {reader}    @node(display)"),
        );
        // An explicit integration point remains available when this extra reader
        // has no proven relationship to the boundary.
        prepare(&mut parse(&source)).unwrap();
        let inferred = source.replace("scope: view", "port: opaque; scope: view");
        let errors = prepare(&mut parse(&inferred)).unwrap_err();
        assert!(
            format!("{errors:?}").contains("cannot place consumer `observer`"),
            "{errors:?}"
        );
        let ordered = inferred.replace(
            "@node(observer) @after(left)",
            "@node(observer) @after(display)",
        );
        prepare(&mut parse(&ordered)).unwrap();
        let missing_consumer = GRAPH.replace("scope: view", "port: opaque; scope: view")
            .replace("@node(display) @after(right) @draw(vertex, fragment, fullscreen, 3) @color(0, target) @attachment(target, load, store)",
                "@node(display) @after(right) @draw(vertex, fragment, fullscreen, 3)");
        let errors = prepare(&mut parse(&missing_consumer)).unwrap_err();
        assert!(
            format!("{errors:?}").contains("no downstream consumer"),
            "{errors:?}"
        );
    }
    #[test]
    fn resource_ports_track_storage_writers_by_binding_access() {
        let source = GRAPH.replace("scope: view", "port: opaque; scope: view")
            .replace("@node(display) @after(right)", "@node(write_image) @after(right) @dispatch(fill, 1, 1) @bind(output, target) store_image\n    @node(display) @after(write_image)")
            .replace("completed: all(left, right)", "completed: all(write_image)");
        let source = format!(
            "{source}\n@shader pass store_image {{
    stage: compute
    binding {{ @group(0) @binding(0) @access(write) output: texture_storage_2d<rgba16float, write> }}
}}"
        );
        let mut program = parse(&source);
        prepare(&mut program).unwrap();
        assert_eq!(
            program.pipelines[0].resource_ports[0].storage_writers,
            ["write_image"]
        );
        let unordered = source.replace(
            "@node(write_image) @after(right)",
            "@node(write_image) @after(left)",
        );
        let errors = prepare(&mut parse(&unordered)).unwrap_err();
        assert!(
            format!("{errors:?}").contains("ambiguous writers"),
            "{errors:?}"
        );
    }
    #[test]
    fn typed_provider_binds_real_resources_and_all_boundary_nodes() {
        let mut program = parse(GRAPH);
        prepare(&mut program).unwrap();
        let attrs = &program.pipelines[0].attrs;
        assert!(attrs.iter().any(
            |a| a.name == "__style_incoming" && a.args == ["Response::finish", "left", "right"]
        ));
        assert!(
            attrs
                .iter()
                .any(|a| a.name == "__style_outgoing" && a.args == ["Response::finish", "display"])
        );
        assert!(attrs.iter().any(|a| a.name == "__style_input"
            && a.args
                == [
                    "Response::finish",
                    "color",
                    "target",
                    "attachment<rgba16float,preserve_update>"
                ]));
    }
    #[test]
    fn independent_engine_composes_typed_boundaries_and_serializes_writers() {
        let source = include_str!("../../tests/fixtures/engines/operation-engine.fr")
            .replace("interface Action {", "contract Action for Value { point finish: Target { port: opaque; scope: view; accepts: raster_draws; composition: ordered_draws(engine.stable_draw_order); after: complete; before: presentation }; ")
            .replace("@implementation(Action) struct Plain {}\nconform Plain : Action", "style Plain for Value : Action")
            .replace("@implementation(Action) struct Extra {}\nconform Extra : Action", "style Extra for Value : Action")
            .replace("@external(result, presentation)", "@image(result, rgba16float)")
            .replace("@draw(project, paint, mesh) @color(0, result) first", "@draw(project, paint, mesh) @color(0, result) first\n @node(display) @after(first) @draw(project, paint, mesh) @color(0, result) @attachment(result, load, store) first")
            .replace("style Extra for Value : Action {", "style Extra for Value : Action { for self { at finish as target { Paint(geometry: self, target: target.target); Paint(geometry: self, target: target.target) } }; ");
        let source = format!(
            "{source}\nstruct Target {{ target: attachment<rgba16float, preserve_update> }}\nprovide Action for main_plan {{ finish {{ complete: all(first); presentation: all(display); target: result; order: stable_draw_order }} }}\n@factory(plain) draw Paint(geometry: DrawRange, target: attachment<rgba16float, preserve_update>) {{ raster geometry; visibility: uncullable; cull: none; attachments {{ target: load_store }}; @vertex fn project(v: Corner) -> Projected {{ return Projected(vec4(v.point, 1.0)) }}; @fragment fn paint(v: Projected) -> vec4 {{ return vec4(0.0, 1.0, 0.0, 1.0) }} }}"
        );
        let files = std::collections::HashMap::from([
            ("engine/engine.fr".into(),source),
            ("main.fr".into(),"surface item(sp: SamplePoint) -> material(Value) { properties { action: Extra }; compose { Value(tint: #fff) } }".into()),
        ]);
        let output = super::super::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
        let [port] = recipe.resource_ports.as_slice() else {
            panic!("expected a reflected attachment port");
        };
        assert_eq!(port.name, "opaque");
        assert_eq!(port.versions.len(), 3);
        assert_eq!(port.versions[0].producers, ["first"]);
        assert_eq!(port.versions[1].previous, Some(0));
        assert_eq!(port.versions[2].previous, Some(1));
        assert_eq!(port.versions[2].readers, ["display"]);
        assert_eq!(port.versions[0].readers, port.versions[1].producers);
        assert_eq!(port.versions[1].readers, port.versions[2].producers);
        let contributions: Vec<_> = recipe
            .steps
            .iter()
            .filter(|s| {
                s.invocation
                    .as_ref()
                    .is_some_and(|i| i.operation == "Paint")
            })
            .collect();
        assert_eq!(contributions.len(), 2);
        assert!(contributions[1].after.contains(&contributions[0].name));
        let display = recipe.steps.iter().find(|s| s.name == "display").unwrap();
        for contribution in contributions {
            assert!(display.after.contains(&contribution.name));
        }
        let mut inferred = files.clone();
        let engine = inferred.get_mut("engine/engine.fr").unwrap();
        *engine = engine
            .replace("at finish as target { Paint(geometry: self, target: target.target); Paint(geometry: self, target: target.target) }",
                "Paint(geometry: self, target: opaque.target); Paint(geometry: self, target: opaque.target)");
        let inferred_output =
            super::super::compile_bundle_virtual(&inferred, "main.fr", false).unwrap();
        let inferred_manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&inferred_output.manifest).unwrap();
        let inferred_recipe = inferred_manifest
            .renderers
            .iter()
            .find(|r| r.selected)
            .unwrap();
        assert_eq!(
            serde_json::to_value(recipe).unwrap(),
            serde_json::to_value(inferred_recipe).unwrap()
        );
        let mut files = files;
        let engine = files.get_mut("engine/engine.fr").unwrap();
        engine.push_str("\ncapability Special {}\nvertex_factory other for Triangle {}\n");
        *engine = engine
            .replace(
                "contract Action for Value {",
                "contract Action for Value { optional capability Special; ",
            )
            .replace(
                "style Extra for Value : Action {",
                "style Extra for Value : Action { requires Special; ",
            )
            .replace(
                "provide Action for main_plan {",
                "provide Action for main_plan { Special { factories: plain }; ",
            );
        super::super::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        for (replacement, expected) in [
            (
                "Special { factories: other };",
                "unavailable for vertex factory `plain`",
            ),
            ("", "does not provide `Special`"),
        ] {
            let mut rejected = files.clone();
            let engine = rejected.get_mut("engine/engine.fr").unwrap();
            *engine = engine.replace("Special { factories: plain };", replacement);
            let error =
                super::super::compile_bundle_virtual(&rejected, "main.fr", false).unwrap_err();
            assert!(format!("{error:?}").contains(expected), "{error:?}");
        }
    }
    #[test]
    fn unused_providers_and_policies_are_checked() {
        for (from, to, expected) in [
            (
                "@image(target, rgba16float)",
                "@image(target, rgba8unorm)",
                "requires format",
            ),
            (
                "view = camera",
                "view = missing",
                "unknown provider resource",
            ),
            ("uniform<ViewData>", "uniform<vec4>", "requires `ViewData`"),
            ("@after(right)", "@after(left)", "existing path"),
            (
                "all(left, right)",
                "all(missing)",
                "unknown integration node",
            ),
            (
                "ordered_draws(engine.stable_draw_order)",
                "global_transparent_queue(engine.stable_draw_order)",
                "unsupported writer composition",
            ),
            ("scope: view", "scope: object", "scope currently requires"),
            (
                "accepts: raster_draws",
                "accepts: dispatches",
                "accepts only",
            ),
            ("preserve_update>", "preserve_update, 4>", "sample count 1"),
            (
                "Camera { view: left.view }",
                "Camera { view: missing.view }",
                "unknown provider endpoint",
            ),
            (
                "view = camera",
                "view = camera\nview = camera",
                "duplicate provider input",
            ),
            (
                "@node(left)",
                "@when(true) @node(left)",
                "unconditional stored producer",
            ),
        ] {
            let source = GRAPH.replace(from, to);
            let result = prepare(&mut parse(&source));
            let text = format!("{:?}", result.unwrap_err());
            assert!(text.contains(expected), "{from} => {to}: {text}");
        }
    }

    #[test]
    fn overlapping_points_do_not_create_unordered_writers() {
        let mut program = parse(GRAPH);
        let mut point = program.style_contracts[0].points[0].clone();
        point.name = "another".into();
        program.style_contracts[0].points.push(point);
        let mut block = program.style_providers[0]
            .blocks
            .iter()
            .find(|b| b.name == "finish")
            .unwrap()
            .clone();
        block.name = "another".into();
        program.style_providers[0].blocks.push(block);
        for point in &mut program.style_contracts[0].points {
            point.fields.push(StyleGraphField {
                name: "port".into(),
                values: vec![Spanned {
                    node: Expr::Var(format!("{}_resources", point.name)),
                    span: 0..0,
                }],
                span: 0..0,
            });
        }
        let errors = prepare(&mut program).unwrap_err();
        assert!(format!("{errors:?}").contains("share attachments without ordered boundaries"));
    }

    #[test]
    fn resource_capability_checks_the_selected_factory_signature() {
        let source = GRAPH.replace(
            "@node(left) @draw(vertex, fragment, fullscreen, 3)",
            "@node(left) @draw(vertex, fragment, mesh)",
        );
        let mut program = parse(&source);
        let mut factory = parse(include_str!(
            "../../tests/fixtures/engines/operation-engine.fr"
        ))
        .vertex_factories
        .remove(0);
        factory.bindings = std::mem::take(&mut program.passes[0].bindings);
        program.passes[0]
            .attrs
            .push(attr("factory", vec![factory.name.clone()]));
        let mut incompatible = factory.clone();
        incompatible.name = "different_layout".into();
        incompatible.bindings[0].value_signature = Some("uniform<vec4>".into());
        program.vertex_factories.extend([factory, incompatible]);
        let (resources, steps) = super::super::recipes::reflect(&program.pipelines[0]).unwrap();
        let value = &program.style_providers[0].inputs[0].1;
        bound_resource(
            &program,
            &resources,
            &steps,
            value,
            "ViewData",
            Some("plain"),
        )
        .unwrap();
        let error = bound_resource(
            &program,
            &resources,
            &steps,
            value,
            "ViewData",
            Some("different_layout"),
        )
        .unwrap_err();
        assert!(
            error.contains("requires `ViewData`, found `uniform<vec4>`"),
            "{error}"
        );
    }

    #[test]
    fn internal_resource_requires_initialization_without_an_existing_recipe_reader() {
        // A style compute call can be the first consumer outside the recipe.
        // Its provider must not infer initialization just from allocation or a
        // conditional writer, even when there is no existing reader to inspect.
        let source = GRAPH
            .replace("@external(camera, camera)", "@buffer(camera, 16)")
            .replace(
                "view: uniform<ViewData>",
                "@access(write) view: buffer<ViewData>",
            );
        for conditional in [false, true] {
            let source = if conditional {
                source.replace("@node(left)", "@when(true) @node(left)")
            } else {
                source.clone()
            };
            let program = parse(&source);
            let (resources, steps) = super::super::recipes::reflect(&program.pipelines[0]).unwrap();
            let expression = &program.style_providers[0].inputs[0].1;
            let result = bound_resource(
                &program,
                &resources,
                &steps,
                expression,
                "buffer<ViewData>",
                None,
            );
            if conditional {
                assert!(result.unwrap_err().contains("no unconditional producer"));
            } else {
                assert_eq!(result.unwrap(), "camera");
            }
        }
    }
}
