//! Instantiate authored draw operations from a statically constructed style graph.
use crate::{ast::*, diag::Diag};
use fresco_artifact::{ManifestImplementationSelection, ManifestStyleInvocation};
use std::collections::{BTreeMap, BTreeSet};
mod bounds;
mod captures;
mod compute;

const MAX_NODES: usize = 1024;
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
fn fail(span: &Span, message: impl Into<String>) -> Vec<Diag> {
    vec![Diag::error(span.clone(), message)]
}
fn path(value: &SExpr) -> Option<String> {
    match &value.node {
        Expr::Var(name) => Some(name.clone()),
        Expr::Member(base, member) => Some(format!("{}.{member}", path(base)?)),
        _ => None,
    }
}
fn compact(ty: &str) -> String {
    ty.chars().filter(|c| !c.is_whitespace()).collect()
}
fn scalar(ty: &str) -> bool {
    matches!(
        ty,
        "f32" | "u32" | "i32" | "bool" | "vec2" | "vec3" | "vec4" | "color"
    )
}
fn literal(value: &serde_json::Value, ty: &str) -> Result<String, String> {
    if let Some(values) = value.as_array() {
        return Ok(format!(
            "{}({})",
            if ty == "color" { "vec4" } else { ty },
            values
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if value.is_boolean() {
        return Ok(value.to_string());
    }
    if value.is_number() {
        return Ok(format!("{ty}({value})"));
    }
    Err(format!("unsupported operation constant `{value}`"))
}
fn expression(source: &str) -> Result<SExpr, Vec<Diag>> {
    use chumsky::Parser;
    let text = format!("const value: f32 = {source}");
    let tokens = crate::lexer::lex_spanned(&text);
    let program = crate::parser::program()
        .parse(crate::parser::input(&tokens, text.len()..text.len()))
        .into_result()
        .map_err(|e| {
            fail(
                &(0..0),
                format!("invalid generated operation expression: {e:?}"),
            )
        })?;
    Ok(program.consts[0].value.clone())
}

fn constants(
    program: &Program,
    style: &StyleDecl,
    selection: Option<&ManifestImplementationSelection>,
) -> Result<Vec<Stmt>, Vec<Diag>> {
    let defaults;
    let parameters = if let Some(selection) = selection {
        &selection.static_parameters
    } else {
        defaults = super::styles::parameters(program, &style.name, &[])?.1;
        &defaults
    };
    parameters
        .iter()
        .map(|p| {
            let declaration = style
                .static_params
                .iter()
                .find(|s| s.name == p.name)
                .expect("checked static setting");
            let source = literal(&p.default, &declaration.ty_name)
                .map_err(|m| fail(&declaration.span, m))?;
            Ok(Stmt::Const {
                name: p.name.clone(),
                name_span: declaration.name_span.clone(),
                ty_name: declaration.ty_name.clone(),
                ty_span: declaration.ty_span.clone(),
                value: expression(&source)?,
            })
        })
        .collect()
}
fn constant_program(program: &Program, bindings: &[Stmt]) -> Program {
    let mut context = program.clone();
    for binding in bindings {
        if let Stmt::Const {
            name,
            name_span,
            ty_name,
            ty_span,
            value,
        } = binding
        {
            context.consts.push(ConstDecl {
                name: name.clone(),
                name_span: name_span.clone(),
                ty_name: ty_name.clone(),
                ty_span: ty_span.clone(),
                value: value.clone(),
                span: value.span.clone(),
            });
        }
    }
    context
}

#[derive(Clone)]
struct ResourceHandle {
    ty: String,
    /// Index in this material's active invocation graph. Inactive branches
    /// retain the type for checking but never acquire a producer identity.
    producer: Option<usize>,
}

#[derive(Clone)]
struct Scope {
    per_self: bool,
    point: Option<(String, String)>,
    bindings: Vec<Stmt>,
    handles: BTreeMap<String, ResourceHandle>,
}
struct Call {
    operation: String,
    point: String,
    target: String,
    args: Vec<Arg>,
    bindings: Vec<Stmt>,
    /// Operation parameter -> producing invocation, resolved in lexical scope.
    /// These are resource bindings; shader reflection determines execution edges.
    resources: BTreeMap<String, usize>,
    span: Span,
}
struct Graph<'a> {
    program: &'a Program,
    style: &'a StyleDecl,
    surface: Option<&'a SurfaceDecl>,
    calls: Vec<Call>,
    shading_bindings: BTreeMap<String, usize>,
    shading_samplers: BTreeMap<String, fresco_artifact::types::SamplerPreset>,
    visited: usize,
}
impl Graph<'_> {
    fn port_binding(&self, name: &str) -> bool {
        self.program
            .style_contracts
            .iter()
            .find(|contract| contract.name == self.style.contract)
            .expect("checked contract")
            .points
            .iter()
            .any(|point| {
                super::style_graph::port(point).expect("checked resource port") == Some(name)
            })
    }
    fn sampler_value(
        &self,
        value: &SExpr,
        scope: &Scope,
    ) -> Result<fresco_artifact::types::SamplerPreset, Vec<Diag>> {
        let symbol = path(value).ok_or_else(|| {
            fail(
                &value.span,
                "sampler binding requires an immutable standard sampler",
            )
        })?;
        let shadowed = scope.handles.contains_key(&symbol)
            || self.program.consts.iter().any(|value| value.name == symbol)
            || self.program.params.iter().any(|value| value.name == symbol)
            || self
                .style
                .params
                .iter()
                .chain(&self.style.static_params)
                .any(|p| p.name == symbol)
            || scope
                .bindings
                .iter()
                .any(|b| matches!(b, Stmt::Const { name, .. } if name == &symbol));
        (!shadowed)
            .then(|| fresco_artifact::types::SamplerPreset::parse(&symbol))
            .flatten()
            .ok_or_else(|| {
                fail(
                    &value.span,
                    "sampler binding requires an unshadowed standard sampler value",
                )
            })
    }

    fn walk(
        &mut self,
        nodes: &[Spanned<StyleGraphNode>],
        scope: &Scope,
        active: bool,
    ) -> Result<(), Vec<Diag>> {
        let mut local = scope.clone();
        let scope = &mut local;
        for node in nodes {
            self.visited += 1;
            if self.visited > MAX_NODES {
                return Err(fail(
                    &node.span,
                    "style graph exceeds the 1024-node construction limit",
                ));
            }
            match &node.node {
                StyleGraphNode::BindShading { name, value } => {
                    if !scope.per_self {
                        return Err(fail(
                            &node.span,
                            "shading bindings require for self invocation identity",
                        ));
                    }
                    let input = self
                        .style
                        .shading_inputs
                        .iter()
                        .find(|input| input.name == *name)
                        .ok_or_else(|| {
                            fail(&node.span, format!("unknown shading input `{name}`"))
                        })?;
                    if compact(&input.ty) == "sampler" {
                        let preset = self.sampler_value(value, scope)?;
                        if active && self.shading_samplers.insert(name.clone(), preset).is_some() {
                            return Err(fail(
                                &node.span,
                                format!("shading input `{name}` is bound more than once"),
                            ));
                        }
                        continue;
                    }
                    let handle = path(value)
                        .and_then(|name| scope.handles.get(&name))
                        .ok_or_else(|| {
                            fail(
                                &value.span,
                                "shading binding requires a resource handle in lexical scope",
                            )
                        })?;
                    let expected = super::compute_operations::resource_type(&input.ty, "read")
                        .map_err(|message| fail(&input.span, message))?;
                    let actual = super::compute_operations::resource_type(&handle.ty, "read")
                        .map_err(|message| fail(&value.span, message))?;
                    if actual != expected {
                        return Err(fail(
                            &value.span,
                            format!("shading input `{name}` requires `{}`", input.ty),
                        ));
                    }
                    if active
                        && self
                            .shading_bindings
                            .insert(
                                name.clone(),
                                handle.producer.expect("active resource producer"),
                            )
                            .is_some()
                    {
                        return Err(fail(
                            &node.span,
                            format!("shading input `{name}` is bound more than once"),
                        ));
                    }
                }
                StyleGraphNode::ForSelf(body) => {
                    if scope.per_self {
                        return Err(fail(
                            &node.span,
                            "nested for self would duplicate invocation identity",
                        ));
                    }
                    let mut inner = scope.clone();
                    inner.per_self = true;
                    self.walk(body, &inner, active)?;
                }
                StyleGraphNode::At {
                    point,
                    target,
                    body,
                } => {
                    if !scope.per_self || scope.point.is_some() {
                        return Err(fail(
                            &node.span,
                            "at requires for self and cannot nest integration points",
                        ));
                    }
                    let contract = self
                        .program
                        .style_contracts
                        .iter()
                        .find(|c| c.name == self.style.contract)
                        .expect("checked style contract");
                    if target == "self"
                        || target == "material"
                        || self.port_binding(target)
                        || self.style.params.iter().any(|p| p.name == *target)
                        || self.style.shading_inputs.iter().any(|p| p.name == *target)
                        || scope
                            .bindings
                            .iter()
                            .any(|b| matches!(b,Stmt::Const {name,..} if name==target))
                        || contract.inputs.iter().any(|i| i.name == *target)
                        || scope.handles.contains_key(target)
                    {
                        return Err(fail(
                            &node.span,
                            "integration target shadows an invocation input or setting",
                        ));
                    }
                    if !contract.points.iter().any(|p| p.name == *point) {
                        return Err(fail(
                            &node.span,
                            format!("unknown integration point `{point}`"),
                        ));
                    }
                    if active {
                        self.require(
                            &Spanned {
                                node: Expr::Var(point.clone()),
                                span: node.span.clone(),
                            },
                            scope,
                            true,
                        )?;
                    }
                    let mut inner = scope.clone();
                    inner.point = Some((point.clone(), target.clone()));
                    self.walk(body, &inner, active)?;
                }
                StyleGraphNode::Require(requirement) => self.require(requirement, scope, active)?,
                StyleGraphNode::StaticIf {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let selected = crate::check::compute::constant_bool(
                        self.program,
                        &scope.bindings,
                        condition,
                    )?;
                    self.walk(then_body, scope, active && selected)?;
                    self.walk(else_body, scope, active && !selected)?;
                }
                StyleGraphNode::StaticFor {
                    variable,
                    range,
                    body,
                } => {
                    let Expr::Range(start, end) = &range.node else {
                        return Err(fail(
                            &range.span,
                            "static for requires a bounded start..end range",
                        ));
                    };
                    let start = crate::check::compute::constant_number(
                        self.program,
                        &scope.bindings,
                        start,
                    )?;
                    let end =
                        crate::check::compute::constant_number(self.program, &scope.bindings, end)?;
                    if !start.is_finite()
                        || !end.is_finite()
                        || start.fract() != 0.0
                        || end.fract() != 0.0
                        || start < 0.0
                        || end < start
                        || end > f64::from(u32::MAX)
                        || end - start > MAX_NODES as f64
                    {
                        return Err(fail(
                            &range.span,
                            "static for requires an ascending u32 range of at most 1024 iterations",
                        ));
                    }
                    if scope
                        .bindings
                        .iter()
                        .any(|b| matches!(b,Stmt::Const {name,..} if name==variable))
                        || self.style.params.iter().any(|p| p.name == *variable)
                        || self
                            .style
                            .shading_inputs
                            .iter()
                            .any(|p| p.name == *variable)
                        || scope.handles.contains_key(variable)
                        || self.port_binding(variable)
                    {
                        return Err(fail(
                            &node.span,
                            "static loop variable shadows a setting, loop, or resource handle",
                        ));
                    }
                    // A zero-trip loop still checks its body, without activating requirements.
                    let values: Vec<_> = if start == end {
                        vec![start as u32]
                    } else {
                        (start as u32..end as u32).collect()
                    };
                    for value in values {
                        let mut inner = scope.clone();
                        inner.bindings.push(Stmt::Const {
                            name: variable.clone(),
                            name_span: node.span.clone(),
                            ty_name: "u32".into(),
                            ty_span: node.span.clone(),
                            value: expression(&format!("u32({value})"))?,
                        });
                        self.walk(body, &inner, active && start != end)?;
                    }
                }
                StyleGraphNode::Let { name, value } if !matches!(&value.node, Expr::Call { name, .. } if self.program.passes.iter().any(|pass| pass.name == *name && pass.operation.is_some())) =>
                {
                    let contract = self
                        .program
                        .style_contracts
                        .iter()
                        .find(|c| c.name == self.style.contract)
                        .expect("contract");
                    if matches!(name.as_str(), "self" | "material")
                        || self.port_binding(name)
                        || scope.handles.contains_key(name)
                        || scope
                            .point
                            .as_ref()
                            .is_some_and(|(_, target)| target == name)
                        || scope.bindings.iter().any(
                            |b| matches!(b, Stmt::Const {name: existing, ..} if existing == name),
                        )
                        || self.style.params.iter().any(|p| p.name == *name)
                        || self.style.shading_inputs.iter().any(|p| p.name == *name)
                        || contract.inputs.iter().any(|p| p.name == *name)
                        || contract.capabilities.iter().any(|used| {
                            self.program.style_capabilities.iter().any(|cap| {
                                cap.name == used.name
                                    && cap.members.iter().any(|member| {
                                        member.name.split('.').next() == Some(name.as_str())
                                    })
                            })
                        })
                    {
                        return Err(fail(
                            &node.span,
                            "graph constant shadows an existing graph binding",
                        ));
                    }
                    let ty = crate::check::compute::constant_binding_type(
                        self.program,
                        &scope.bindings,
                        value,
                    )?;
                    scope.bindings.push(Stmt::Const {
                        name: name.clone(),
                        name_span: node.span.clone(),
                        ty_name: ty,
                        ty_span: value.span.clone(),
                        value: value.clone(),
                    });
                }
                StyleGraphNode::Call(value) | StyleGraphNode::Let { value, .. } => {
                    let Expr::Call {
                        name,
                        args,
                        const_args,
                        ..
                    } = &value.node
                    else {
                        unreachable!("graph parser checks calls");
                    };
                    if !const_args.is_empty() {
                        return Err(fail(
                            &value.span,
                            "operations use named value arguments, not template arguments",
                        ));
                    }
                    let operation = self
                        .program
                        .passes
                        .iter()
                        .find(|p| p.name == *name && p.operation.is_some())
                        .ok_or_else(|| fail(&value.span, format!("unknown operation `{name}`")))?;
                    let definition = operation.operation.as_ref().expect("operation");
                    let result_name = match &node.node {
                        StyleGraphNode::Let { name, .. } => Some(name),
                        StyleGraphNode::Call(_) => None,
                        _ => unreachable!("operation call"),
                    };
                    let (point, target) = if definition.compute.is_some() {
                        if !scope.per_self {
                            return Err(fail(
                                &node.span,
                                "compute calls require for self invocation identity",
                            ));
                        }
                        let name = result_name.ok_or_else(|| {
                            fail(
                                &node.span,
                                "compute calls return an owned resource; bind the result with let",
                            )
                        })?;
                        let contract = self
                            .program
                            .style_contracts
                            .iter()
                            .find(|c| c.name == self.style.contract)
                            .expect("contract");
                        if matches!(name.as_str(), "self" | "material")
                            || self.port_binding(name)
                            || scope.handles.contains_key(name)
                            || scope
                                .point
                                .as_ref()
                                .is_some_and(|(_, target)| target == name)
                            || scope
                                .bindings
                                .iter()
                                .any(|b| matches!(b, Stmt::Const { name: n, .. } if n == name))
                            || self.style.params.iter().any(|p| &p.name == name)
                            || self.style.shading_inputs.iter().any(|p| &p.name == name)
                            || contract.inputs.iter().any(|p| &p.name == name)
                            || contract.capabilities.iter().any(|used| {
                                self.program.style_capabilities.iter().any(|capability| {
                                    capability.name == used.name
                                        && capability.members.iter().any(|member| {
                                            member.name.split('.').next() == Some(name.as_str())
                                        })
                                })
                            })
                        {
                            return Err(fail(
                                &node.span,
                                "compute result shadows an existing graph binding",
                            ));
                        }
                        self.arguments(operation, args, scope, active)?;
                        (String::new(), String::new())
                    } else {
                        if result_name.is_some() {
                            return Err(fail(
                                &node.span,
                                "draw operations do not return resource handles",
                            ));
                        }
                        if !scope.per_self {
                            return Err(fail(
                                &node.span,
                                "raster operation calls require for self invocation identity",
                            ));
                        }
                        let mut invocation_scope = scope.clone();
                        if invocation_scope.point.is_none() {
                            let contract = self
                                .program
                                .style_contracts
                                .iter()
                                .find(|contract| contract.name == self.style.contract)
                                .expect("checked contract");
                            let mut selected = None;
                            for point in &contract.points {
                                let port = super::style_graph::port(point)
                                    .map_err(|message| fail(&node.span, message))?;
                                if let Some(port) = port
                                    && args.iter().any(|arg| {
                                        path(&arg.value).is_some_and(|name| {
                                            name.starts_with(&format!("{port}."))
                                        })
                                    })
                                {
                                    if selected.is_some() {
                                        return Err(fail(
                                            &node.span,
                                            "operation resources select multiple integration boundaries; use one resource port",
                                        ));
                                    }
                                    selected = Some((point.name.clone(), port.to_owned()));
                                }
                            }
                            invocation_scope.point = Some(selected.ok_or_else(|| fail(&node.span,
                                "raster operation calls require for self and at point as target, or attachments from a declared resource port"))?);
                            self.require(
                                &Spanned {
                                    node: Expr::Var(
                                        invocation_scope
                                            .point
                                            .as_ref()
                                            .expect("selected point")
                                            .0
                                            .clone(),
                                    ),
                                    span: node.span.clone(),
                                },
                                &invocation_scope,
                                active,
                            )?;
                        }
                        self.arguments(operation, args, &invocation_scope, active)?;
                        invocation_scope.point.expect("checked placement")
                    };
                    if definition.compute.is_none() {
                        for requirement in &definition.requirements {
                            if !host_requirement(definition, requirement) {
                                self.require(requirement, scope, active)?;
                            }
                        }
                    }
                    let resources = if active {
                        args.iter()
                            .filter_map(|arg| {
                                let handle = scope.handles.get(&path(&arg.value)?)?;
                                Some((
                                    arg.name.clone().expect("checked named operation argument"),
                                    handle
                                        .producer
                                        .expect("active handles have active producers"),
                                ))
                            })
                            .collect()
                    } else {
                        BTreeMap::new()
                    };
                    if let (Some(name), Some(compute)) = (result_name, &definition.compute) {
                        scope.handles.insert(
                            name.clone(),
                            ResourceHandle {
                                ty: compute.return_ty.node.clone(),
                                producer: active.then_some(self.calls.len()),
                            },
                        );
                    }
                    if active {
                        self.calls.push(Call {
                            operation: name.clone(),
                            point: point.clone(),
                            target: target.clone(),
                            args: args.clone(),
                            bindings: scope.bindings.clone(),
                            resources,
                            span: node.span.clone(),
                        });
                    }
                }
            }
        }
        Ok(())
    }
    fn require(&self, requirement: &SExpr, _scope: &Scope, active: bool) -> Result<(), Vec<Diag>> {
        let mut context = self.program.clone();
        let mut style = self.style.clone();
        style.graph.clear();
        style.requirements = vec![requirement.clone()];
        context.styles = vec![style];
        context.surfaces = if active {
            self.surface.cloned().into_iter().collect()
        } else {
            vec![]
        };
        super::style_graph::requirements(&context)
    }
    fn arguments(
        &self,
        pass: &PassDecl,
        args: &[Arg],
        scope: &Scope,
        active: bool,
    ) -> Result<(), Vec<Diag>> {
        let operation = pass.operation.as_ref().expect("operation");
        if let Some(required) = &pass.material_name {
            let mut schema = Some(self.style.schema.as_str());
            let mut compatible = false;
            for _ in 0..=self.program.material_properties.len() {
                let Some(name) = schema else {
                    break;
                };
                if name == required {
                    compatible = true;
                    break;
                }
                schema = self
                    .program
                    .material_properties
                    .iter()
                    .find(|m| m.name == name)
                    .and_then(|m| m.extends_name.as_deref());
            }
            if !compatible {
                return Err(fail(
                    &pass.span,
                    format!(
                        "draw operation `{}` requires material schema `{required}`",
                        pass.name
                    ),
                ));
            }
        }
        let mut names = BTreeSet::new();
        let mut attachments = BTreeSet::new();
        for arg in args {
            let name = arg
                .name
                .as_ref()
                .ok_or_else(|| fail(&arg.value.span, "draw operation arguments must be named"))?;
            if !names.insert(name) {
                return Err(fail(
                    &arg.value.span,
                    format!("duplicate operation argument `{name}`"),
                ));
            }
            let parameter = operation
                .inputs
                .iter()
                .find(|p| p.name == *name)
                .ok_or_else(|| {
                    fail(
                        &arg.value.span,
                        format!("unknown operation argument `{name}`"),
                    )
                })?;
            let ty = compact(&parameter.ty);
            if ty == "DrawRange" {
                if !scope.per_self || path(&arg.value).as_deref() != Some("self") {
                    return Err(fail(
                        &arg.value.span,
                        "DrawRange must be the current for self invocation",
                    ));
                }
            } else if ty == "sampler" && operation.compute.is_none() {
                self.sampler_value(&arg.value, scope)?;
            } else if scalar(&ty) {
                if let Some(projection) =
                    captures::projection(self.program, self.style, &arg.value)?
                {
                    let expected = if ty == "color" { "vec4" } else { &ty };
                    if projection.ty != expected {
                        return Err(fail(
                            &arg.value.span,
                            format!(
                                "operation argument `{name}` requires `{ty}`, found `{}`",
                                projection.ty
                            ),
                        ));
                    }
                } else if let Some(setting) =
                    path(&arg.value).and_then(|n| self.style.params.iter().find(|p| p.name == n))
                {
                    if setting.ty_name != ty {
                        return Err(fail(
                            &arg.value.span,
                            format!(
                                "operation argument `{name}` requires `{ty}`, found `{}`",
                                setting.ty_name
                            ),
                        ));
                    }
                } else {
                    let context = constant_program(self.program, &scope.bindings);
                    let parameter = GlobalParamDecl {
                        name: format!("__operation_{name}"),
                        name_span: parameter.span.clone(),
                        ty_name: ty.clone(),
                        ty_span: parameter.span.clone(),
                        default: None,
                        range: None,
                        span: parameter.span.clone(),
                    };
                    crate::check::compute::style_parameter_value(&context, &parameter, &arg.value)?;
                }
            } else {
                let contract = self
                    .program
                    .style_contracts
                    .iter()
                    .find(|c| c.name == self.style.contract)
                    .expect("contract");
                let name = path(&arg.value).ok_or_else(|| {
                    fail(
                        &arg.value.span,
                        "resource arguments must name contract inputs or target attachments",
                    )
                })?;
                let port_member = super::resource_ports::member(contract, &name)
                    .map_err(|message| fail(&arg.value.span, message))?;
                let attachment_identity = port_member
                    .map(|(point, member)| (Some(point.name.clone()), member.to_owned()))
                    .or_else(|| {
                        scope.point.as_ref().and_then(|(point, target)| {
                            name.strip_prefix(&format!("{target}."))
                                .map(|member| (Some(point.clone()), member.to_owned()))
                        })
                    })
                    .unwrap_or_else(|| (None, name.clone()));
                if ty.starts_with("attachment<") && !attachments.insert(attachment_identity) {
                    return Err(fail(
                        &arg.value.span,
                        "operation attachment arguments must not alias",
                    ));
                }
                let capability = super::prepared_geometry::member(self.program, contract, &name)
                    .map_err(|m| fail(&arg.value.span, m))?;
                let actual = if let Some((point, member)) = port_member {
                    if !scope
                        .point
                        .as_ref()
                        .is_some_and(|(selected, _)| *selected == point.name)
                    {
                        return Err(fail(
                            &arg.value.span,
                            "resource port does not match the operation's integration boundary",
                        ));
                    }
                    self.program
                        .structs
                        .iter()
                        .find(|target| target.name == point.ty)
                        .and_then(|target| target.fields.iter().find(|field| field.name == member))
                        .map(|field| field.ty_name.clone())
                } else if let Some(handle) = scope.handles.get(&name) {
                    Some(handle.ty.clone())
                } else if let Some((capability, member)) = capability {
                    self.require(
                        &Spanned {
                            node: Expr::Var(capability.name.clone()),
                            span: arg.value.span.clone(),
                        },
                        scope,
                        active,
                    )?;
                    Some(member.ty.clone())
                } else if let Some((point, target)) = &scope.point {
                    if let Some(member) = name.strip_prefix(&format!("{target}.")) {
                        let point = contract
                            .points
                            .iter()
                            .find(|p| p.name == *point)
                            .expect("point");
                        self.program
                            .structs
                            .iter()
                            .find(|s| s.name == point.ty)
                            .and_then(|s| s.fields.iter().find(|f| f.name == member))
                            .map(|f| f.ty_name.clone())
                    } else {
                        contract
                            .inputs
                            .iter()
                            .find(|i| i.name == name)
                            .map(|i| i.ty.clone())
                    }
                } else {
                    contract
                        .inputs
                        .iter()
                        .find(|i| i.name == name)
                        .map(|i| i.ty.clone())
                };
                let same_handle = actual.as_ref().is_some_and(|actual| {
                    matches!((super::compute_operations::resource_type(actual, "read"), super::compute_operations::resource_type(&ty, "read")), (Ok(a), Ok(b)) if a == b)
                });
                if actual.as_deref().map(compact) != Some(ty.clone()) && !same_handle {
                    return Err(fail(
                        &arg.value.span,
                        format!("operation resource `{name}` does not provide `{ty}`"),
                    ));
                }
            }
        }
        let mut projections = BTreeMap::new();
        for argument in args {
            if let Some(projection) =
                captures::projection(self.program, self.style, &argument.value)?
            {
                projections.insert(argument.name.clone().expect("checked name"), projection);
            }
        }
        captures::validate_host(operation, &projections)?;
        for parameter in &operation.inputs {
            if !names.contains(&parameter.name) {
                return Err(fail(
                    &pass.span,
                    format!("missing operation argument `{}`", parameter.name),
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn prepare(program: &mut Program) -> Result<(), Vec<Diag>> {
    for pass in &program.passes {
        let mut diagnostics = Vec::new();
        crate::check::shader_services::validate(pass, program, &mut diagnostics);
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        super::mesh_pass::validate_services(program, pass).map_err(|message| {
            vec![Diag::error(pass.span.clone(), message).with_file(&pass.source_file)]
        })?;
    }
    let definitions: Vec<_> = program
        .passes
        .iter()
        .filter(|p| p.operation.is_some())
        .cloned()
        .collect();
    validate(program, &definitions)?;
    let context = program.clone();
    for style in &context.styles {
        let scope = Scope {
            per_self: false,
            point: None,
            bindings: constants(&context, style, None)?,
            handles: BTreeMap::new(),
        };
        let mut validation = Graph {
            program: &context,
            style,
            surface: None,
            calls: vec![],
            shading_bindings: Default::default(),
            shading_samplers: Default::default(),
            visited: 0,
        };
        validation.walk(&style.graph, &scope, false)?;
        for surface in &context.surfaces {
            let Some(selection) = surface.settings.as_ref().and_then(|s| {
                s.implementations
                    .iter()
                    .find(|s| s.symbol == style.name && s.contract == style.contract)
            }) else {
                continue;
            };
            prepare_selection(program, &context, style, surface, selection)?;
        }
    }
    super::style_graph::operation_inputs(program)?;
    program.passes.retain(|p| p.operation.is_none());
    super::prepared_geometry::install(program)?;
    super::style_availability::reflect(program, &context);
    Ok(())
}

/// Shared specialization path for compilation and candidate reflection. The caller
/// owns an isolated program when probing an unselected implementation.
pub(super) fn prepare_selection(
    program: &mut Program,
    context: &Program,
    style: &StyleDecl,
    surface: &SurfaceDecl,
    selection: &ManifestImplementationSelection,
) -> Result<(), Vec<Diag>> {
    let scope = Scope {
        bindings: constants(context, style, Some(selection))?,
        per_self: false,
        point: None,
        handles: BTreeMap::new(),
    };
    let mut graph = Graph {
        program: context,
        style,
        surface: Some(surface),
        calls: vec![],
        shading_bindings: Default::default(),
        shading_samplers: Default::default(),
        visited: 0,
    };
    graph.walk(&style.graph, &scope, true)?;
    for input in &style.shading_inputs {
        if !graph.shading_bindings.contains_key(&input.name)
            && !graph.shading_samplers.contains_key(&input.name)
        {
            return Err(fail(
                &input.span,
                format!("missing binding for shading input `{}`", input.name),
            ));
        }
    }
    if !graph.shading_bindings.is_empty() {
        let inputs = style
            .shading_inputs
            .iter()
            .filter(|input| compact(&input.ty) != "sampler")
            .map(|input| {
                let producer = graph.shading_bindings[&input.name];
                (
                    input.name.clone(),
                    fresco_artifact::ManifestDrawComputeBinding {
                        producer: compute::invocation_name(surface, style, producer),
                        ty: input.ty.clone(),
                        dimension: None,
                    },
                )
            })
            .collect();
        program
            .surfaces
            .iter_mut()
            .find(|s| s.name == surface.name)
            .expect("selected surface")
            .settings
            .as_mut()
            .expect("selected settings")
            .shading_inputs
            .insert(style.contract.clone(), inputs);
    }
    if !graph.shading_samplers.is_empty() {
        program
            .surfaces
            .iter_mut()
            .find(|s| s.name == surface.name)
            .expect("selected surface")
            .settings
            .as_mut()
            .expect("selected settings")
            .shading_samplers
            .insert(style.contract.clone(), graph.shading_samplers.clone());
    }
    for (ordinal, call) in graph.calls.iter().enumerate() {
        if call.resources.values().any(|producer| *producer >= ordinal) {
            return Err(fail(
                &call.span,
                "operation resource dependency must name an earlier producer",
            ));
        }
        instantiate(
            program,
            context,
            style,
            surface,
            selection,
            call,
            u32::try_from(ordinal).expect("bounded graph"),
        )?;
    }
    Ok(())
}

// A requirement referring to an operation input is evaluated from captured host
// values. Capability/material requirements remain compile-time contract checks.
fn host_requirement(operation: &StyleOperation, value: &SExpr) -> bool {
    match &value.node {
        Expr::Var(name) => operation
            .inputs
            .iter()
            .any(|input| input.name == name.split('.').next().expect("variable path")),
        Expr::Member(value, _) | Expr::Unary(_, value) => host_requirement(operation, value),
        Expr::Binary(_, left, right) => {
            host_requirement(operation, left) || host_requirement(operation, right)
        }
        Expr::Call { args, .. } => args
            .iter()
            .any(|arg| host_requirement(operation, &arg.value)),
        Expr::Index { array, index } => {
            host_requirement(operation, array) || host_requirement(operation, index)
        }
        _ => false,
    }
}

fn validate(program: &Program, definitions: &[PassDecl]) -> Result<(), Vec<Diag>> {
    let mut names = BTreeSet::new();
    for pass in definitions {
        if !names.insert(&pass.name)
            || program
                .passes
                .iter()
                .filter(|p| p.name == pass.name)
                .count()
                != 1
        {
            return Err(fail(&pass.span, "duplicate draw operation name"));
        }
        let operation = pass.operation.as_ref().expect("definition");
        if pass
            .material_name
            .as_ref()
            .is_some_and(|name| !program.material_properties.iter().any(|m| &m.name == name))
        {
            return Err(fail(&pass.span, "unknown draw operation material schema"));
        }
        let mut inputs = BTreeSet::new();
        for parameter in &operation.inputs {
            if parameter.name.starts_with("__fresco_") {
                return Err(fail(
                    &parameter.span,
                    "__fresco_ is reserved for compiler-owned operation bindings",
                ));
            }
            if parameter.name.contains('.') {
                return Err(fail(
                    &parameter.span,
                    "draw parameter names must be simple identifiers",
                ));
            }
            if !inputs.insert(&parameter.name) {
                return Err(fail(&parameter.span, "duplicate operation parameter"));
            }
            let ty = compact(&parameter.ty);
            if ty.starts_with("attachment<") {
                super::style_graph::attachment(&ty)
                    .map_err(|message| fail(&parameter.span, message))?;
            }
            if !scalar(&ty)
                && ty != "DrawRange"
                && !ty.starts_with("attachment<")
                && !program.structs.iter().any(|s| s.name == ty)
                && !program.resource_types.iter().any(|s| s.name == ty)
                && !(operation.compute.is_none()
                    && program
                        .interfaces
                        .iter()
                        .any(|i| i.name == ty && i.entry.is_none()))
                && !(ty == "sampler" && operation.compute.is_none())
                && super::compute_operations::resource_type(&ty, "read").is_err()
            {
                return Err(fail(
                    &parameter.span,
                    format!("unsupported draw operation input type `{ty}`"),
                ));
            }
        }
        if operation.compute.is_some() {
            super::compute_operations::validate(program, pass)?;
            continue;
        }
        let source = operation.raster.as_ref().and_then(path).ok_or_else(|| {
            fail(
                &pass.span,
                "draw operation requires raster geometry_parameter",
            )
        })?;
        if !operation.inputs.iter().any(|p| {
            p.name == source
                && (p.ty == "DrawRange" || program.resource_types.iter().any(|r| r.name == p.ty))
        }) {
            return Err(fail(
                &pass.span,
                "raster source must be a DrawRange or a prepared geometry resource parameter",
            ));
        }
        if let Some((vertices, offset)) = &operation.generated_vertices {
            if operation
                .inputs
                .iter()
                .any(|p| p.name == source && p.ty == "DrawRange")
            {
                return Err(fail(
                    &vertices.span,
                    "generated vertices require prepared geometry",
                ));
            }
            let input =
                path(vertices).and_then(|name| operation.inputs.iter().find(|p| p.name == name));
            if !input.is_some_and(|p| {
                matches!(
                    super::compute_operations::resource_type(&p.ty, "read"),
                    Ok(super::compute_operations::ResourceType::Buffer(_))
                )
            }) {
                return Err(fail(
                    &vertices.span,
                    "generated vertices must name a read-only buffer parameter",
                ));
            }
            if !matches!(
                super::compute_operations::host_type(program, operation, offset)?.as_str(),
                "u32" | "number"
            ) {
                return Err(fail(&offset.span, "generated vertex offset must be u32"));
            }
            super::compute_host::lower(
                program,
                operation,
                offset,
                Some(fresco_artifact::ComputeScalarType::U32),
            )?;
        }
        bounds::metadata(program, operation)?;
        if operation
            .inputs
            .iter()
            .any(|p| p.name == source && p.ty == "DrawRange")
        {
            if !pass
                .attrs
                .iter()
                .any(|a| a.name == "factory" && a.args.len() == 1)
            {
                return Err(fail(
                    &pass.span,
                    "draw operation requires an explicit vertex factory",
                ));
            }
            let factory_name = pass
                .attrs
                .iter()
                .find(|a| a.name == "factory")
                .and_then(|a| a.args.first())
                .expect("checked factory attribute");
            let factory = program
                .vertex_factories
                .iter()
                .find(|f| &f.name == factory_name)
                .ok_or_else(|| fail(&pass.span, "unknown operation vertex factory"))?;
            if !factory.hooks.iter().any(|h| h.name == "transform") {
                return Err(fail(
                    &pass.span,
                    "operation factory requires a transform hook",
                ));
            }
        } else if pass.attrs.iter().any(|a| a.name == "factory") {
            return Err(fail(
                &pass.span,
                "prepared draws obtain their factory from the geometry producer",
            ));
        }
        for kind in ["vertex", "fragment"] {
            if pass
                .hooks
                .iter()
                .filter(|h| h.attrs.iter().any(|a| a.name == kind))
                .count()
                != 1
            {
                return Err(fail(
                    &pass.span,
                    format!("draw operation requires exactly one @{kind} shader"),
                ));
            }
        }
        for requirement in &operation.requirements {
            if host_requirement(operation, requirement) {
                if super::compute_operations::host_type(program, operation, requirement)? != "bool"
                {
                    return Err(fail(
                        &requirement.span,
                        "draw preconditions require bool expressions",
                    ));
                }
                super::compute_host::lower(
                    program,
                    operation,
                    requirement,
                    Some(fresco_artifact::ComputeScalarType::Bool),
                )?;
                continue;
            }
            if let Expr::Var(name) = &requirement.node
                && !name.contains('.')
                && name != "true"
                && name != "false"
            {
                if !program.style_contracts.iter().any(|c| {
                    c.capabilities.iter().any(|cap| cap.name == *name)
                        || c.points.iter().any(|p| p.name == *name)
                }) {
                    return Err(fail(
                        &requirement.span,
                        format!("unknown operation requirement `{name}`"),
                    ));
                }
            } else {
                super::style_graph::validate_material_requirement(program, requirement)?;
            }
        }
        let mut attached = BTreeSet::new();
        for field in &operation.attachments {
            if !attached.insert(&field.name)
                || !operation
                    .inputs
                    .iter()
                    .any(|p| p.name == field.name && p.ty.starts_with("attachment<"))
                || field.values.len() != 1
                || path(&field.values[0]).as_deref() != Some("load_store")
            {
                return Err(fail(
                    &field.span,
                    "operation attachments require unique attachment parameters with load_store",
                ));
            }
        }
        for parameter in operation
            .inputs
            .iter()
            .filter(|p| p.ty.starts_with("attachment<"))
        {
            if !attached.contains(&parameter.name) {
                return Err(fail(
                    &parameter.span,
                    "missing explicit attachment preservation policy",
                ));
            }
        }
        if operation.inputs.iter().any(|p| compact(&p.ty).ends_with(",test_only>"))
            && pass.state.iter().find(|(name, _)| name == "depth_write").is_some_and(|(_, value)| !matches!(&value.node, Expr::Var(name) if matches!(name.as_str(), "false" | "off"))) {
            return Err(fail(&pass.span, "test_only attachments require operations to disable depth writes"));
        }
        super::mesh_pass::validate_operation(program, pass).map_err(|m| fail(&pass.span, m))?;
        // Retained bodies must parse even when no call activates this declaration.
        for hook in &pass.hooks {
            crate::parser::executable_pass_hook_body(hook)
                .map_err(|e| fail(&hook.span, format!("invalid draw shader body: {e:?}")))?;
        }
    }
    Ok(())
}

fn instantiate(
    program: &mut Program,
    context: &Program,
    style: &StyleDecl,
    surface: &SurfaceDecl,
    selection: &ManifestImplementationSelection,
    call: &Call,
    ordinal: u32,
) -> Result<(), Vec<Diag>> {
    let definition = context
        .passes
        .iter()
        .find(|p| p.name == call.operation && p.operation.is_some())
        .expect("checked call");
    let operation = definition.operation.as_ref().expect("operation");
    if operation.compute.is_some() {
        return compute::instantiate(
            program, context, style, surface, selection, call, ordinal, definition,
        );
    }
    let point = format!("{}::{}", style.contract, call.point);
    let renderer = program
        .pipelines
        .iter_mut()
        .find(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"))
        .ok_or_else(|| fail(&call.span, "draw invocation requires a selected renderer"))?;
    if !renderer
        .attrs
        .iter()
        .any(|a| a.name == "__style_point" && a.args.first() == Some(&point))
    {
        return Err(fail(
            &call.span,
            format!("selected renderer does not provide `{point}`"),
        ));
    }
    let name = format!("__style_{}_{}_{ordinal:04}", surface.name, style.name);
    let service_uses = if operation
        .inputs
        .iter()
        .any(|p| context.interfaces.iter().any(|i| i.name == p.ty))
    {
        super::mesh_pass::validate_operation(context, definition)
            .map_err(|m| fail(&definition.span, m))?
    } else {
        super::mesh_pass::OperationUse::default()
    };
    let mut pass = definition.clone();
    pass.operation = None;
    pass.prepared_draw =
        super::prepared_geometry::select_draw(context, style, operation, &call.args)?;
    if let Some(geometry) = &pass.prepared_draw {
        let source = context
            .passes
            .iter()
            .find(|p| p.name == geometry.producer_pass)
            .expect("checked preparation pass");
        pass.attrs
            .extend(source.attrs.iter().filter(|a| a.name == "factory").cloned());
    }
    // Reusable operations have concrete defaults independent of material state.
    for (key, value) in [
        ("cull", "none"),
        ("blend_all", "replace"),
        ("depth_write", "off"),
        ("depth_compare", "less_equal"),
    ] {
        if !pass
            .state
            .iter()
            .any(|(name, _)| name == key || key == "depth_compare" && name == "depth_test")
        {
            pass.state.push((
                key.into(),
                Spanned {
                    node: Expr::Var(value.into()),
                    span: call.span.clone(),
                },
            ));
        }
    }
    pass.name.clone_from(&name);
    pass.material_name = Some(style.schema.clone());
    pass.stage = Some(Spanned {
        node: "raster".into(),
        span: call.span.clone(),
    });
    pass.draw = Some(Spanned {
        node: "per_object".into(),
        span: call.span.clone(),
    });
    let mut reference = PipelinePassRef {
        name: name.clone(),
        name_span: call.span.clone(),
        span: call.span.clone(),
        attrs: vec![],
        invocation: Some(Box::new(ManifestStyleInvocation {
            bounds: None,
            sort_geometry: None,
            host: None,
            generated_vertices: None,
            compute_inputs: BTreeMap::new(),
            preparation: pass.prepared_draw.as_ref().map(|g| g.producer_pass.clone()),
            point: point.clone(),
            material: surface.name.clone(),
            operation: call.operation.clone(),
            ordinal,
        })),
    };
    let vertex = pass
        .hooks
        .iter()
        .find(|h| h.attrs.iter().any(|a| a.name == "vertex"))
        .expect("vertex")
        .name
        .clone();
    let fragment = pass
        .hooks
        .iter()
        .find(|h| h.attrs.iter().any(|a| a.name == "fragment"))
        .expect("fragment")
        .name
        .clone();
    reference
        .attrs
        .push(attr("draw", vec![vertex, fragment, "mesh".into()]));
    let mut attributes = vec![attr(
        "__style_operation",
        vec![style.contract.clone(), style.name.clone(), point.clone()],
    )];
    let mut prelude = super::prepared_geometry::bind_draw(context, &mut pass)?;
    let projections = captures::arguments(context, style, call)?;
    for value in operation
        .requirements
        .iter()
        .chain(operation.visibility.iter())
        .chain(operation.sort_position.iter())
        .chain(
            operation
                .generated_vertices
                .iter()
                .map(|(_, offset)| offset),
        )
    {
        captures::reject_host(value, &projections)?;
    }
    let mut service_hooks = Vec::new();
    let mut color = 0u32;
    for parameter in &operation.inputs {
        let value = &call
            .args
            .iter()
            .find(|a| a.name.as_ref() == Some(&parameter.name))
            .expect("checked argument")
            .value;
        let ty = compact(&parameter.ty);
        if ty == "DrawRange" || context.resource_types.iter().any(|r| r.name == ty) {
            continue;
        }
        if context.interfaces.iter().any(|i| i.name == ty) {
            service_hooks.extend(
                super::shader_services::link(
                    context,
                    style,
                    parameter,
                    value,
                    &point,
                    super::shader_services::DrawLink {
                        renderer,
                        pass: &mut pass,
                        reference: &mut reference,
                        attributes: &mut attributes,
                        uses: &service_uses,
                    },
                )
                .map_err(|m| fail(&parameter.span, m))?,
            );
            continue;
        }
        if ty == "sampler" {
            let preset = path(value)
                .and_then(|symbol| fresco_artifact::types::SamplerPreset::parse(&symbol))
                .ok_or_else(|| fail(&value.span, "invalid checked sampler argument"))?;
            ensure_binding(&mut pass, context, &parameter.name, "sampler")?;
            let binding = pass
                .bindings
                .iter_mut()
                .find(|binding| binding.name == parameter.name)
                .ok_or_else(|| {
                    fail(
                        &parameter.span,
                        "operation sampler conflicts with a factory binding",
                    )
                })?;
            if binding
                .attrs
                .iter()
                .any(|attribute| !matches!(attribute.name.as_str(), "group" | "binding"))
            {
                return Err(fail(
                    &parameter.span,
                    "operation sampler conflicts with an authored binding source",
                ));
            }
            binding
                .attrs
                .push(attr("sampler", vec![preset.name().into()]));
            continue;
        }
        if let Some(producer) = call.resources.get(&parameter.name) {
            let producer = compute::invocation_name(surface, style, *producer);
            let resource = super::compute_operations::resource_type(&ty, "read")
                .map_err(|m| fail(&parameter.span, m))?;
            ensure_binding(
                &mut pass,
                context,
                &parameter.name,
                &super::compute_operations::resource_signature(&resource, false),
            )?;
            let inputs = &mut reference
                .invocation
                .as_mut()
                .expect("draw invocation")
                .compute_inputs;
            inputs.insert(
                parameter.name.clone(),
                fresco_artifact::ManifestDrawComputeBinding {
                    producer: producer.clone(),
                    ty: ty.clone(),
                    dimension: None,
                },
            );
            for (member, dimension) in super::compute_operations::resource_dimensions(&resource) {
                let binding = format!("__fresco_{}_{}", parameter.name, member);
                ensure_binding(&mut pass, context, &binding, "uniform<u32>")?;
                inputs.insert(
                    binding.clone(),
                    fresco_artifact::ManifestDrawComputeBinding {
                        producer: producer.clone(),
                        ty: ty.clone(),
                        dimension: Some(*dimension),
                    },
                );
                pass.bindings
                    .iter_mut()
                    .find(|b| b.name == binding)
                    .ok_or_else(|| {
                        fail(
                            &parameter.span,
                            "reserved operation dimension binding conflicts with factory",
                        )
                    })?
                    .operation_alias = Some(format!("{}.{member}", parameter.name));
            }
            continue;
        }
        if scalar(&ty) {
            let projection = projections.get(&parameter.name);
            let root_value = projection.map(|projection| Spanned {
                node: Expr::Var(projection.root.clone()),
                span: value.span.clone(),
            });
            let value = root_value.as_ref().unwrap_or(value);
            let source = if let Some((lane, setting)) =
                path(value).and_then(|n| style.params.iter().enumerate().find(|(_, p)| p.name == n))
            {
                ensure_settings_binding(&mut pass, context)?;
                let lane = selection
                    .settings_offset
                    .checked_add(
                        u32::try_from(lane)
                            .map_err(|_| fail(&call.span, "setting offset overflow"))?,
                    )
                    .ok_or_else(|| fail(&call.span, "setting offset overflow"))?;
                let value = format!("__operation_settings[u32({lane})]");
                match setting.ty_name.as_str() {
                    "f32" => format!("{value}.x"),
                    "vec2" => format!("{value}.xy"),
                    "vec3" => format!("{value}.xyz"),
                    "vec4" | "color" => value,
                    "u32" => format!("(u32({value}.x) | (u32({value}.y) << u32(16)))"),
                    "i32" => format!("i32(u32({value}.x) | (u32({value}.y) << u32(16)))"),
                    "bool" => format!("({value}.x != 0.0)"),
                    _ => unreachable!("setting type"),
                }
            } else if let Some(projection) = projection {
                let binding = format!("__fresco_projection_{}", parameter.name);
                ensure_binding(
                    &mut pass,
                    context,
                    &binding,
                    &format!("uniform<{}>", projection.root_ty),
                )?;
                if !attributes
                    .iter()
                    .any(|a| a.name == "input" && a.args.first() == Some(&projection.root))
                {
                    attributes.push(attr(
                        "input",
                        vec![projection.root.clone(), projection.root_ty.clone()],
                    ));
                }
                reference
                    .attrs
                    .push(attr("bind", vec![binding.clone(), projection.root.clone()]));
                binding
            } else {
                let context = constant_program(context, &call.bindings);
                let decl = GlobalParamDecl {
                    name: format!("__arg_{}", parameter.name),
                    name_span: parameter.span.clone(),
                    ty_name: ty.clone(),
                    ty_span: parameter.span.clone(),
                    default: None,
                    range: None,
                    span: parameter.span.clone(),
                };
                literal(
                    &crate::check::compute::style_parameter_value(&context, &decl, value)?,
                    &ty,
                )
                .map_err(|m| fail(&value.span, m))?
            };
            let source = projection.map_or_else(
                || source.clone(),
                |projection| format!("({source}).{}", projection.member),
            );
            prelude.push_str(&format!(
                "let {}: {} = {source};\n",
                parameter.name,
                if ty == "color" { "vec4" } else { &ty }
            ));
        } else {
            let argument = path(value).expect("checked resource");
            let contract = context
                .style_contracts
                .iter()
                .find(|contract| contract.name == style.contract)
                .expect("checked contract");
            let port_member = super::resource_ports::member(contract, &argument)
                .map_err(|message| fail(&value.span, message))?;
            let input = port_member
                .map(|(_, member)| member)
                .or_else(|| argument.strip_prefix(&format!("{}.", call.target)))
                .unwrap_or(&argument)
                .to_string();
            if !attributes
                .iter()
                .any(|a| a.name == "input" && a.args.first() == Some(&input))
            {
                attributes.push(attr("input", vec![input.clone(), ty.clone()]));
            }
            if ty.starts_with("attachment<") {
                if ty.starts_with("attachment<depth") {
                    reference.attrs.push(attr("depth", vec![input.clone()]));
                } else {
                    reference
                        .attrs
                        .push(attr("color", vec![color.to_string(), input.clone()]));
                    color += 1;
                }
                reference.attrs.push(attr(
                    "attachment",
                    vec![input, "load".into(), "store".into()],
                ));
            } else {
                ensure_binding(
                    &mut pass,
                    context,
                    &parameter.name,
                    &format!("uniform<{ty}>"),
                )?;
                reference
                    .attrs
                    .push(attr("bind", vec![parameter.name.clone(), input]));
            }
        }
    }
    let requirements = operation
        .requirements
        .iter()
        .filter(|r| host_requirement(operation, r))
        .map(|r| {
            super::compute_host::lower(
                context,
                operation,
                r,
                Some(fresco_artifact::ComputeScalarType::Bool),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (bounds, sort_geometry) = bounds::metadata(context, operation)?;
    let has_spatial = bounds.is_some() || sort_geometry.is_some();
    {
        let invocation = reference.invocation.as_mut().expect("draw invocation");
        invocation.bounds = bounds;
        invocation.sort_geometry = sort_geometry;
    }
    if operation.generated_vertices.is_some() || !requirements.is_empty() || has_spatial {
        let mut arguments = BTreeMap::new();
        let mut geometry = BTreeMap::new();
        for parameter in &operation.inputs {
            let ty = compact(&parameter.ty);
            if scalar(&ty) {
                if projections.contains_key(&parameter.name) {
                    // Shader-only projections were excluded from all host expressions above.
                    continue;
                }
                let value = &call
                    .args
                    .iter()
                    .find(|a| a.name.as_ref() == Some(&parameter.name))
                    .expect("checked argument")
                    .value;
                arguments.insert(
                    parameter.name.clone(),
                    compute::scalar_argument(context, style, selection, call, &ty, value)?,
                );
            } else if context.resource_types.iter().any(|r| r.name == ty) {
                let prepared = super::prepared_geometry::select_resource(
                    context, style, parameter, &call.args,
                )?;
                let roles = prepared
                    .resource
                    .attrs
                    .iter()
                    .find(|a| a.name == "geometry")
                    .expect("validated geometry roles");
                geometry.insert(
                    parameter.name.clone(),
                    fresco_artifact::ManifestDrawGeometry {
                        producer: prepared.producer_pass,
                        vertex_count_member: roles.args[2].clone(),
                        index_count_member: roles.args[3].clone(),
                    },
                );
            }
        }
        reference.invocation.as_mut().expect("draw invocation").host =
            Some(fresco_artifact::ManifestDrawHost {
                arguments,
                geometry,
                requirements,
            });
    }
    if let Some((vertices, offset)) = &operation.generated_vertices {
        let binding = path(vertices).expect("validated generated vertex parameter");
        let invocation = reference.invocation.as_mut().expect("draw invocation");
        if !invocation.compute_inputs.contains_key(&binding) {
            return Err(fail(
                &vertices.span,
                "generated vertices require an owned compute output",
            ));
        }
        invocation.generated_vertices = Some(fresco_artifact::ManifestGeneratedVertices {
            binding,
            base_vertex: super::compute_host::lower(
                context,
                operation,
                offset,
                Some(fresco_artifact::ComputeScalarType::U32),
            )?,
        });
    }
    if pass
        .bindings
        .iter()
        .any(|b| b.name == "__operation_settings")
    {
        let input = format!("__settings_{}", style.contract);
        let resource = if let Some(existing) = renderer.attrs.iter().find(|a| {
            a.name == "external" && a.args.get(1).is_some_and(|s| s == "style_parameters")
        }) {
            existing.args[0].clone()
        } else {
            renderer.attrs.push(attr(
                "external",
                vec![input.clone(), "style_parameters".into()],
            ));
            input.clone()
        };
        if !renderer.attrs.iter().any(|a| {
            a.name == "__style_input"
                && a.args.first() == Some(&point)
                && a.args.get(1) == Some(&input)
        }) {
            renderer.attrs.push(attr(
                "__style_input",
                vec![point, input.clone(), resource, "buffer<vec4>".into()],
            ));
        }
        attributes.push(attr("input", vec![input.clone(), "buffer<vec4>".into()]));
        reference
            .attrs
            .push(attr("bind", vec!["__operation_settings".into(), input]));
    }
    for hook in &mut pass.hooks {
        let tokens = crate::lexer::lex_spanned(&prelude)
            .into_iter()
            .map(|(t, _)| (t, call.span.clone()));
        hook.body.splice(1..1, tokens);
    }
    pass.hooks.extend(service_hooks);
    program.passes.push(pass);
    program.pipelines.push(PipelineDecl {
        resource_ports: Vec::new(),
        name: name.clone(),
        name_span: call.span.clone(),
        material_name: Some(style.schema.clone()),
        material_span: Some(style.schema_span.clone()),
        pipeline_type: "postprocess".into(),
        pipeline_type_span: call.span.clone(),
        passes: vec![Spanned {
            node: name,
            span: call.span.clone(),
        }],
        attrs: attributes,
        pass_refs: vec![reference],
        span: call.span.clone(),
    });
    Ok(())
}

fn ensure_settings_binding(pass: &mut PassDecl, program: &Program) -> Result<(), Vec<Diag>> {
    ensure_binding(pass, program, "__operation_settings", "buffer<vec4>")
}
pub(super) fn ensure_binding(
    pass: &mut PassDecl,
    program: &Program,
    name: &str,
    ty: &str,
) -> Result<(), Vec<Diag>> {
    let factory = pass
        .attrs
        .iter()
        .find(|a| a.name == "factory")
        .and_then(|a| a.args.first())
        .and_then(|n| program.vertex_factories.iter().find(|f| f.name == *n))
        .ok_or_else(|| fail(&pass.span, "unknown operation vertex factory"))?;
    if let Some(binding) = pass
        .bindings
        .iter()
        .chain(&factory.bindings)
        .find(|b| b.name == name)
    {
        if binding.value_signature.as_deref().map(compact) != Some(compact(ty)) {
            return Err(fail(
                &pass.span,
                format!("operation binding `{name}` conflicts with factory type"),
            ));
        }
        return Ok(());
    }
    // Reuse the factory's authored group, extending it with collision-free slots.
    let group = factory
        .bindings
        .iter()
        .find_map(|b| b.group_index)
        .unwrap_or(0);
    let occupied: BTreeSet<_> = factory
        .bindings
        .iter()
        .chain(&pass.bindings)
        .filter(|b| b.group_index == Some(group))
        .filter_map(|b| b.binding_index)
        .collect();
    let index = (0..u32::MAX)
        .find(|i| !occupied.contains(i))
        .ok_or_else(|| fail(&pass.span, "operation binding space exhausted"))?;
    pass.bindings.push(PassBindingDecl {
        operation_alias: None,
        group_index: Some(group),
        binding_index: Some(index),
        name: name.into(),
        name_span: pass.span.clone(),
        attrs: vec![
            attr("group", vec![group.to_string()]),
            attr("binding", vec![index.to_string()]),
        ],
        value_signature: Some(ty.into()),
        span: pass.span.clone(),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    #[test]
    fn shading_input_declarations_reject_invalid_scope_access_and_missing_bindings() {
        for (declaration, expected) in [
            (
                "shading_input data: buffer<vec4, write> scope draw",
                "requires `read` access",
            ),
            (
                "shading_input data: buffer<vec4, read> scope material",
                "require scope draw",
            ),
            (
                "shading_input ink: buffer<vec4, read> scope draw",
                "duplicate or reserved",
            ),
            (
                "shading_input data: buffer<vec4, read> scope draw",
                "missing binding for shading input",
            ),
        ] {
            let mut input = files();
            let engine = input.get_mut("engine/engine.fr").unwrap();
            *engine = engine.replace(
                "style Extra for Value : Action {",
                &format!("style Extra for Value : Action {{ {declaration}; "),
            );
            let errors =
                super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err();
            assert!(
                format!("{errors:?}").contains(expected),
                "{declaration}: {errors:?}"
            );
        }
    }
    #[test]
    fn shading_bindings_retain_producers_and_check_inactive_branches() {
        use chumsky::Parser;
        for (body, error) in [
            (
                "for self { let values = Build(count: 4u); bind shading.data = values }",
                None,
            ),
            (
                "for self { let values = Build(count: 4u); static if false { bind shading.data = values }; bind shading.data = values }",
                None,
            ),
            ("bind shading.data = missing", Some("require for self")),
            (
                "for self { bind shading.data = missing }",
                Some("lexical scope"),
            ),
            (
                "for self { let values = Build(count: 4u); bind shading.unknown = values }",
                Some("unknown shading input"),
            ),
            (
                "for self { let values = Build(count: 4u); bind shading.data = values; bind shading.data = values }",
                Some("more than once"),
            ),
            (
                "for self { static if false { bind shading.data = missing } }",
                Some("lexical scope"),
            ),
            (
                "for self { static if true { let values = Build(count: 4u) }; bind shading.data = values }",
                Some("lexical scope"),
            ),
            (
                "for self { let data = Build(count: 4u) }",
                Some("shadows an existing graph binding"),
            ),
            (
                "for self { static for data in 0u..1u { let values = Build(count: data) } }",
                Some("shadows a setting, loop, or resource handle"),
            ),
            (
                "for self { at missing as data { } }",
                Some("integration target shadows"),
            ),
        ] {
            let source = format!(
                "{COMPUTE_DEFINITIONS}\ncontract Action for Value {{ fn shade(x: f32) -> f32 }}\nstyle Only for Value : Action {{ shading_input data: buffer<vec4, read> scope draw; {body} }}"
            );
            let tokens = crate::lexer::lex_spanned(&source);
            let program = crate::parser::program()
                .parse(crate::parser::input(&tokens, source.len()..source.len()))
                .into_result()
                .unwrap();
            let style = &program.styles[0];
            let scope = super::Scope {
                per_self: false,
                point: None,
                bindings: vec![],
                handles: Default::default(),
            };
            let mut graph = super::Graph {
                program: &program,
                style,
                surface: None,
                calls: vec![],
                shading_bindings: Default::default(),
                shading_samplers: Default::default(),
                visited: 0,
            };
            let result = graph.walk(&style.graph, &scope, true);
            if let Some(error) = error {
                assert!(
                    format!("{:?}", result.unwrap_err()).contains(error),
                    "{body}"
                );
            } else {
                result.unwrap();
                assert_eq!(graph.shading_bindings["data"], 0);
                assert_eq!(graph.calls.len(), 1);
            }
        }
    }
    #[test]
    fn compute_handles_retain_producers_across_reuse_and_static_expansion() {
        use chumsky::Parser;
        let input = files();
        let source = input["engine/engine.fr"].replace(
            "for self { at finish",
            r#"for self {
            let root = Build(count: root)
            static if false { let absent = Build(count: 99u) }
            static for iteration in 0u..2u {
                let values = Build(count: iteration)
                let copied = Copy(source: values)
                let copied_root = Copy(source: root)
            }
            at finish"#,
        ) + COMPUTE_DEFINITIONS
            + "\nconst root: u32 = 1u\n";
        let tokens = crate::lexer::lex_spanned(&source);
        let program = crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap();
        let style = program.styles.iter().find(|s| s.name == "Extra").unwrap();
        let scope = super::Scope {
            per_self: false,
            point: None,
            bindings: super::constants(&program, style, None).unwrap(),
            handles: Default::default(),
        };
        let mut graph = super::Graph {
            program: &program,
            style,
            surface: None,
            calls: vec![],
            visited: 0,
            shading_bindings: Default::default(),
            shading_samplers: Default::default(),
        };
        graph.walk(&style.graph, &scope, true).unwrap();
        assert_eq!(graph.calls.len(), 9);
        assert!(
            graph.calls[0].resources.is_empty(),
            "call arguments resolve before introducing the result handle"
        );
        for (consumer, producer) in [(2, 1), (3, 0), (5, 4), (6, 0)] {
            assert_eq!(
                graph.calls[consumer].resources.get("source"),
                Some(&producer)
            );
        }
        assert!(graph.calls[..7].iter().all(|call| call.point.is_empty()));
        assert!(graph.calls[7..].iter().all(|call| call.point == "finish"));
        assert!(graph.calls.iter().all(|call| {
            call.resources
                .values()
                .all(|producer| graph.calls[*producer].operation == "Build")
        }));
    }
    const COMPUTE_DEFINITIONS: &str = r#"
compute Build(count: u32) -> buffer<vec4, read> {
    requires count <= 4096u
    output values: buffer<vec4, write>(count)
    workgroup_size: (64, 1, 1)
    dispatch threads(count, 1, 1)
    @compute fn main(id: uvec3) {
        if id.x >= count { return }
        values[id.x] = vec4(1.0)
    }
    return values
}
compute Copy(source: buffer<vec4, read>) -> buffer<vec4, read> {
    output copied: buffer<vec4, write>(source.count)
    workgroup_size: (64, 1, 1)
    dispatch threads(source.count, 1, 1)
    @compute fn main(id: uvec3) {
        if id.x >= source.count { return }
        copied[id.x] = source[id.x]
    }
    return copied
}
"#;

    #[test]
    fn compute_selected_calls_emit_distinct_owned_programs_and_dependencies() {
        use fresco_artifact::{
            ComputeScalar, ManifestComputeArgument as Argument,
            ManifestComputeExpression as Expression,
        };
        let mut input = files();
        let engine = input.get_mut("engine/engine.fr").unwrap();
        *engine = engine.replace("for self { at finish", "for self { let source = Build(count: count); let copied = Copy(source: source); let again = Copy(source: source); at finish");
        engine.push_str(COMPUTE_DEFINITIONS);
        input.get_mut("main.fr").unwrap().push_str("\nsurface other(sp: SamplePoint) -> material(Value) { properties { action: Extra }; compose { Value(tint: #fff) } }");
        let result = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&result.manifest).unwrap();
        let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let kernels: Vec<_> = manifest
            .gpu_programs
            .iter()
            .filter(|p| p.compute_invocation.is_some())
            .collect();
        assert_eq!(kernels.len(), 6);
        let names: std::collections::BTreeSet<_> = kernels.iter().map(|p| &p.pass).collect();
        assert_eq!(names.len(), kernels.len());
        for material in ["item", "other"] {
            let mut own: Vec<_> = kernels
                .iter()
                .filter(|p| p.compute_invocation.as_ref().unwrap().material == material)
                .collect();
            own.sort_by_key(|p| p.compute_invocation.as_ref().unwrap().ordinal);
            let first = own[0].compute_invocation.as_ref().unwrap();
            assert!(
                matches!(&first.arguments["count"], Argument::Constant { value, ty } if value == &serde_json::json!(2) && ty == "u32")
            );
            assert_eq!(first.requirements.len(), 1);
            assert!(
                matches!(&first.output.extents[0], Expression::Input { parameter, .. } if parameter == "count")
            );
            assert!(matches!(
                &first.threads[1],
                Expression::Constant {
                    value: ComputeScalar::U32(1)
                }
            ));
            assert!(first.dependencies.is_empty());
            for consumer in &own[1..] {
                let invocation = consumer.compute_invocation.as_ref().unwrap();
                assert_eq!(invocation.dependencies, [own[0].pass.clone()]);
                assert_eq!(invocation.metadata_dependencies, [own[0].pass.clone()]);
                assert!(
                    matches!(&invocation.arguments["source"], Argument::Output { producer } if producer == &own[0].pass)
                );
                assert!(
                    consumer
                        .bindings
                        .iter()
                        .any(|b| b.access == "write" && b.element_stride == Some(16))
                );
            }
            assert!(
                own.iter().all(|p| p.surface.as_deref() == Some(material)
                    && p.workgroup_size == Some([64, 1, 1]))
            );
        }
        let renderer = manifest.renderers.iter().find(|r| r.selected).unwrap();
        assert!(
            renderer
                .steps
                .iter()
                .all(|step| !names.contains(&step.pass)),
            "compute calls must not become after-opaque raster contributions"
        );
    }

    #[test]
    fn compute_definitions_and_inactive_calls_do_not_instantiate_work() {
        let input = files();
        let baseline = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
        for graph in [
            "",
            "static if false { let values = Build(count: 16u); let copied = Copy(source: values) };",
        ] {
            let mut input = input.clone();
            let engine = input.get_mut("engine/engine.fr").unwrap();
            *engine = engine.replace(
                "for self { at finish",
                &format!("for self {{ {graph} at finish"),
            );
            engine.push_str(COMPUTE_DEFINITIONS);
            let result = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
            assert_eq!(baseline.wgsl, result.wgsl);
            assert_eq!(baseline.manifest, result.manifest);
        }
    }

    #[test]
    fn compute_definitions_validate_host_contracts_even_without_calls() {
        for (from, to, expected) in [
            ("return values", "return missing", "return its owned output"),
            (
                "output values: buffer<vec4, write>(count)",
                "output values: buffer<vec4, read>(count)",
                "requires `write` access",
            ),
            (
                "output values: buffer<vec4, write>(count)",
                "output values: buffer<u32, write>(count)",
                "does not match",
            ),
            (
                "output values: buffer<vec4, write>(count)",
                "output values: buffer<vec4, write>(count, count)",
                "requires 1 logical extents",
            ),
            ("(source.count)", "(source[0u].x)", "cannot read GPU values"),
            (
                "workgroup_size: (64, 1, 1)",
                "workgroup_size: (0, 1, 1)",
                "positive u32 constants",
            ),
            ("requires count <= 4096u", "requires count", "require bool"),
            (
                "workgroup_size: (64, 1, 1)",
                "workgroup_size: (64, 1)",
                "three constant dimensions",
            ),
            (
                "dispatch threads(count, 1, 1)",
                "dispatch threads(count, 1)",
                "three logical extents",
            ),
            (
                "@compute fn main(id: uvec3)",
                "@compute fn main(id: vec3)",
                "one uvec3 invocation ID",
            ),
        ] {
            let mut input = files();
            input
                .get_mut("engine/engine.fr")
                .unwrap()
                .push_str(&COMPUTE_DEFINITIONS.replace(from, to));
            let error = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err();
            assert!(format!("{error:?}").contains(expected), "{from}: {error:?}");
        }
    }

    #[test]
    fn compute_graph_results_enforce_type_and_lexical_scope_in_inactive_branches() {
        for (graph, expected) in [
            ("Build(count: 1u)", "bind the result with let"),
            (
                "let ink = Build(count: 1u)",
                "shadows an existing graph binding",
            ),
            ("let values = Copy(source: values)", "does not provide"),
            (
                "static if false { let values = Build(count: 1u) }; let copied = Copy(source: values)",
                "does not provide",
            ),
            (
                "let values = Build(count: 1u); let values = Build(count: 2u)",
                "shadows an existing graph binding",
            ),
            (
                "let values = Build(count: 1u); let copied = Copy(source: ink)",
                "does not provide",
            ),
            (
                "let values = Build(count: 1u); static for values in 0u..1u { let copied = Copy(source: values) }",
                "shadows a setting, loop, or resource handle",
            ),
        ] {
            let mut input = files();
            let engine = input.get_mut("engine/engine.fr").unwrap();
            *engine = engine.replace(
                "for self { at finish",
                &format!("for self {{ static if false {{ {graph} }}; at finish"),
            );
            engine.push_str(COMPUTE_DEFINITIONS);
            let error = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err();
            assert!(
                format!("{error:?}").contains(expected),
                "{graph}: {error:?}"
            );
        }
    }
    fn files() -> HashMap<String, String> {
        let original = include_str!("../../tests/fixtures/engines/operation-engine.fr");
        let source=original
            .replace("interface Action {", "contract Action for Value { point finish: Target { scope: view; accepts: raster_draws; composition: ordered_draws(engine.stable_draw_order); after: complete; before: presentation }; ")
            .replace("@implementation(Action) struct Plain {}\nconform Plain : Action", "style Plain for Value : Action")
            .replace("@implementation(Action) struct Extra {}\nconform Extra : Action", "style Extra for Value : Action")
            .replace("style Extra for Value : Action {", "style Extra for Value : Action { param ink: color = #00ff00; static param count: u32 = 2u; for self { at finish as target { static for i in 0u..count { Paint(geometry: self, tint: ink, target: target.target) } } }; ")
            .replace("@external(result, presentation)", "@image(result, rgba16float)")
            .replace("@draw(project, paint, mesh) @color(0, result) first", "@draw(project, paint, mesh) @color(0, result) first\n @node(display) @after(first) @draw(project, paint, mesh) @color(0, result) @attachment(result, load, store) first");
        let source = format!(
            "{source}\n{}",
            r#"
struct Target { target: attachment<rgba16float, preserve_update> }
provide Action for main_plan { finish { complete: all(first); presentation: all(display); target: result; order: stable_draw_order } }
@factory(plain) draw Paint(geometry: DrawRange, tint: color, target: attachment<rgba16float, preserve_update>) {
 raster geometry
 visibility: uncullable
 cull: none
 attachments { target: load_store }
 @vertex fn project(v: Corner) -> Projected { return Projected(vec4(v.point, 1.0)) }
 @fragment fn paint(v: Projected) -> vec4 { return tint }
}
"#
        );
        HashMap::from([
            ("engine/engine.fr".into(),source),
            ("main.fr".into(),"surface item(sp: SamplePoint) -> material(Value) { properties { action: Extra }; compose { Value(tint: #fff) } }".into()),
        ])
    }
    #[test]
    fn style_operations_compile_reusable_draws() {
        let output = super::super::compile_bundle_virtual(&files(), "main.fr", false).unwrap();
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let renderer = manifest.renderers.iter().find(|r| r.selected).unwrap();
        let invocations: Vec<_> = renderer
            .steps
            .iter()
            .filter_map(|s| s.invocation.as_ref())
            .collect();
        assert_eq!(invocations.len(), 2);
        assert_eq!(invocations[0].ordinal, 0);
        assert_eq!(invocations[1].ordinal, 1);
        assert!(
            invocations
                .iter()
                .all(|i| i.material == "item" && i.operation == "Paint")
        );
    }
    #[test]
    fn draw_sampler_arguments_are_explicit_and_independent_per_invocation() {
        let mut input = files();
        let source = input.get_mut("engine/engine.fr").unwrap();
        *source = source
            .replace(
                "tint: color, target:",
                "tint: color, filtering: sampler, target:",
            )
            .replace(
                "tint: ink, target:",
                "tint: ink, filtering: nearest_repeat, target:",
            )
            .replace(
                "Paint(geometry: self, tint: ink, filtering: nearest_repeat, target: target.target)",
                "static if i == 0u { Paint(geometry: self, tint: ink, filtering: nearest_repeat, target: target.target) } else { Paint(geometry: self, tint: ink, filtering: linear_clamp, target: target.target) }",
            );
        let output = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let samplers: Vec<_> = manifest
            .surfaces
            .iter()
            .flat_map(|surface| &surface.mesh_passes)
            .flat_map(|pass| &pass.bindings)
            .filter_map(|binding| binding.sampler)
            .collect();
        assert_eq!(
            samplers,
            [
                fresco_artifact::types::SamplerPreset::NearestRepeat,
                fresco_artifact::types::SamplerPreset::LinearClamp
            ]
        );
        for (from, to, diagnostic) in [
            (
                "filtering: nearest_repeat",
                "filtering: missing",
                "standard sampler value",
            ),
            (
                "filtering: nearest_repeat",
                "filtering: ink",
                "standard sampler value",
            ),
            (
                "filtering: nearest_repeat",
                "filtering: 1u",
                "immutable standard sampler",
            ),
            (
                "param ink: color",
                "param nearest_repeat: f32 = 0.0; param ink: color",
                "unshadowed standard sampler",
            ),
        ] {
            let mut invalid = input.clone();
            let source = invalid.get_mut("engine/engine.fr").unwrap();
            *source = source.replace(from, to);
            let errors =
                super::super::compile_bundle_virtual(&invalid, "main.fr", false).unwrap_err();
            assert!(format!("{errors:?}").contains(diagnostic), "{errors:?}");
        }
    }

    #[test]
    fn style_operations_reject_invalid_calls_and_inactive_definitions() {
        for (from, to, expected) in [
            ("tint: ink, target:", "tint: true, target:", "color"),
            ("geometry: self", "geometry: missing", "current for self"),
            (
                "tint: ink, target:",
                "target:",
                "missing operation argument `tint`",
            ),
            (
                "tint: ink, target:",
                "tint: ink, tint: ink, target:",
                "duplicate operation argument",
            ),
            (
                "target: target.target",
                "target: target.missing",
                "does not provide",
            ),
            ("count: u32 = 2u", "count: u32 = 2048u", "at most 1024"),
            ("0u..count", "0u..ink", "constant"),
            (
                "return tint",
                "return implicit_frame",
                "unknown raster value",
            ),
            ("return tint", "return 1.0", "draw shader return"),
            ("target: load_store", "target: discard", "load_store"),
        ] {
            let mut input = files();
            let engine = input.get_mut("engine/engine.fr").unwrap();
            assert!(engine.contains(from));
            *engine = engine.replace(from, to);
            let error = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err();
            assert!(format!("{error:?}").contains(expected), "{from}: {error:?}");
        }
        let mut input = files();
        let engine = input.get_mut("engine/engine.fr").unwrap();
        *engine = engine
            .replace("count: u32 = 2u", "count: u32 = 0u")
            .replace("return tint", "return missing_capture");
        assert!(
            format!(
                "{:?}",
                super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err()
            )
            .contains("missing_capture")
        );
    }
    #[test]
    fn style_operations_static_branches_activate_only_selected_requirements() {
        let mut input = files();
        let engine = input.get_mut("engine/engine.fr").unwrap();
        *engine = engine
            .replace(
                "contract Action for Value {",
                "capability Absent {}\ncontract Action for Value { optional capability Absent; ",
            )
            .replace(
                "for self {",
                "static if false { requires Absent }\nelse { for self {",
            )
            .replace("} } }; ", "} } } }; ");
        // Close the static else block independently of formatting in the fixture.

        super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
        let engine = input.get_mut("engine/engine.fr").unwrap();
        *engine = engine.replace("static if false", "static if true");
        let error = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err();
        assert!(
            format!("{error:?}").contains("does not provide `Absent`"),
            "{error:?}"
        );
    }
    #[test]
    fn style_operations_are_per_material_and_declarations_do_not_execute() {
        let mut input = files();
        input.get_mut("main.fr").unwrap().push_str("\nsurface other(sp: SamplePoint) -> material(Value) { properties { action: Extra }; compose { Value(tint: #fff) } }");
        let output = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
        for surface in &manifest.surfaces {
            let own = recipe
                .steps
                .iter()
                .filter(|s| {
                    s.invocation
                        .as_ref()
                        .is_some_and(|i| i.material == surface.name)
                })
                .count();
            assert_eq!(own, 2);
            assert_eq!(
                surface
                    .mesh_passes
                    .iter()
                    .filter(|p| p.pass.starts_with("__style_"))
                    .count(),
                2
            );
        }
        *input.get_mut("main.fr").unwrap() =
            input["main.fr"].replace("action: Extra", "action: Plain");
        let output = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        assert!(
            manifest
                .renderers
                .iter()
                .flat_map(|r| &r.steps)
                .all(|s| s.invocation.is_none())
        );
        assert!(!output.wgsl.contains("__style_"));
    }
    #[test]
    fn style_operations_preconditions_are_checked_but_only_activated_by_calls() {
        let mut input = files();
        let engine = input.get_mut("engine/engine.fr").unwrap();
        *engine = engine
            .replace(
                "interface Controls {",
                "interface Controls { param enabled: bool = false\n",
            )
            .replace(
                " raster geometry",
                " requires material.enabled == true\n raster geometry",
            );
        let error = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err();
        assert!(
            format!("{error:?}").contains("precondition is not satisfied"),
            "{error:?}"
        );
        let engine = input.get_mut("engine/engine.fr").unwrap();
        *engine = engine.replace("count: u32 = 2u", "count: u32 = 0u");
        super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap();
        let engine = input.get_mut("engine/engine.fr").unwrap();
        *engine = engine.replace("material.enabled", "material.missing");
        let error = super::super::compile_bundle_virtual(&input, "main.fr", false).unwrap_err();
        assert!(format!("{error:?}").contains("missing"), "{error:?}");
    }
}
