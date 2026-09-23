//! Typed, explicitly bound raster functions. Engine code owns evaluation order.
use super::*;
use fresco_artifact::{ManifestRasterEntry, ManifestStageOutput};

fn canonical(name: &str) -> String {
    match crate::check::strip_spatial_type_suffix(name) {
        "color" | "clip_position" | "vec4<f32>" => "vec4",
        "vec2<f32>" => "vec2",
        "vec3<f32>" => "vec3",
        "mat2x2<f32>" => "mat2",
        "mat3x3<f32>" => "mat3",
        "mat4x4<f32>" => "mat4",
        "float" | "signal" => "f32",
        other => other,
    }
    .into()
}

pub(in crate::driver) fn field_type(
    structs: &[StructDecl],
    ty: &str,
    field: &str,
) -> Result<String, String> {
    let ty = canonical(ty);
    if let Some(record) = find_struct(structs, &ty) {
        return record
            .fields
            .iter()
            .find(|f| f.name == field)
            .map(|f| canonical(&f.ty_name))
            .ok_or_else(|| format!("unknown `{ty}` field `{field}`"));
    }
    let (prefix, vector) = if let Some(vector) = ty.strip_prefix('u') {
        ("u", vector)
    } else if let Some(vector) = ty.strip_prefix('i') {
        ("i", vector)
    } else {
        ("", ty.as_str())
    };
    if let Some(width) = vector
        .strip_prefix("vec")
        .and_then(|n| n.parse::<usize>().ok())
        && (2..=4).contains(&width)
        && (1..=4).contains(&field.len())
        && (field.chars().all(|c| "xyzw"[..width].contains(c))
            || field.chars().all(|c| "rgba"[..width].contains(c)))
    {
        return Ok(if field.len() == 1 {
            format!("{prefix}{}", if prefix.is_empty() { "f32" } else { "32" })
        } else {
            format!("{prefix}vec{}", field.len())
        });
    }
    Err(format!("cannot access `{field}` on `{ty}`"))
}

struct Functions<'a> {
    pass: &'a PassDecl,
    imports: &'a [crate::ast::FnDecl],
    material: Option<&'a MaterialHir>,
    expected_return: Option<String>,
    structs: &'a [StructDecl],
    names: Vec<String>,
    needed: BTreeSet<usize>,
    needed_imports: BTreeSet<usize>,
    links: Option<FunctionLinks>,
    services: &'a [crate::ast::InterfaceDecl],
    service_calls: Vec<ServiceCall>,
    iterator_serial: u32,
    iterator_expansions: u32,
    resources: HashMap<String, String>,
    input_fields: HashMap<String, String>,
    input_name: String,
}

struct FunctionLinks {
    prefix: String,
    tokens: std::collections::BTreeMap<usize, String>,
    resources: BTreeSet<String>,
}

pub(in crate::driver) struct ServiceLink {
    pub hooks: Vec<crate::ast::PassFnHookDecl>,
    pub resources: BTreeSet<String>,
}

pub(in crate::driver) struct ServiceCall {
    pub input: String,
    pub method: String,
    pub receiver: crate::ast::Span,
    pub method_span: crate::ast::Span,
}

#[derive(Default)]
pub(in crate::driver) struct OperationUse {
    pub reachable: BTreeSet<usize>,
    pub service_calls: std::collections::BTreeMap<usize, Vec<ServiceCall>>,
}

impl Functions<'_> {
    fn material_distance(&self, actual: &str, expected: &str) -> Option<usize> {
        let material = self.material?;
        let mut name = actual;
        for distance in 0..=material.material_schemas.len() {
            if name == expected {
                return Some(distance);
            }
            name = material.material_schemas.get(name)?.as_deref()?;
        }
        None
    }
    fn field(&self, ty: &str, field: &str) -> Result<String, String> {
        if ty == self.input_name {
            return self
                .input_fields
                .get(field)
                .cloned()
                .ok_or_else(|| format!("unknown vertex field `{field}`"));
        }
        if let Some(material) = self
            .material
            .filter(|m| m.material_schemas.contains_key(ty))
        {
            return material
                .material_channels
                .iter()
                .find(|c| c.name == field)
                .map(|c| canonical(&c.ty_name))
                .ok_or_else(|| format!("unknown material field `{field}`"));
        }
        field_type(self.structs, ty, field)
    }
    fn variable(&self, name: &str, vars: &HashMap<String, String>) -> Result<String, String> {
        if matches!(name, "true" | "false") {
            return Ok("bool".into());
        }
        if let Some(ty) = self.resources.get(name) {
            return Ok(ty.clone());
        }
        if let Some((base, field)) = name.rsplit_once('.') {
            return self.field(&self.variable(base, vars)?, field);
        }
        if let Some(ty) = vars.get(name).or_else(|| self.resources.get(name)) {
            return Ok(ty.clone());
        }
        if let Some(value) = self
            .material
            .and_then(|m| super::super::surface_properties::static_value(m, name))
        {
            return Ok(if matches!(value.as_str(), "true" | "false") {
                "bool"
            } else if value.ends_with('u') {
                "u32"
            } else {
                "f32"
            }
            .into());
        }
        Err(format!("unknown raster value `{name}`"))
    }
    fn expr(&mut self, e: &mut SExpr, vars: &HashMap<String, String>) -> Result<String, String> {
        match &mut e.node {
            Expr::Num(..) => Ok("f32".into()),
            Expr::Color(..) => Ok("vec4".into()),
            Expr::Var(name) => {
                let ty = self.variable(name, vars)?;
                let root = name.split('.').next().expect("variable name");
                if self.resources.contains_key(root)
                    && let Some(links) = &mut self.links
                {
                    links
                        .tokens
                        .insert(e.span.start, format!("{}{root}", links.prefix));
                    links.resources.insert(root.into());
                }
                Ok(ty)
            }
            Expr::Vec2(a, b) => {
                self.expr(a, vars)?;
                self.expr(b, vars)?;
                Ok("vec2".into())
            }
            Expr::Vec3(a, b, c) => {
                self.expr(a, vars)?;
                self.expr(b, vars)?;
                self.expr(c, vars)?;
                Ok("vec3".into())
            }
            Expr::Vec4(a, b, c, d) => {
                self.expr(a, vars)?;
                self.expr(b, vars)?;
                self.expr(c, vars)?;
                self.expr(d, vars)?;
                Ok("vec4".into())
            }
            Expr::Array(items) => {
                let mut ty = None;
                for item in items.iter_mut() {
                    let next = self.expr(item, vars)?;
                    if ty.as_ref().is_some_and(|ty| ty != &next) {
                        return Err("array elements must have the same type".into());
                    }
                    ty = Some(next);
                }
                Ok(format!(
                    "array<{},{}>",
                    ty.ok_or("array values cannot be empty")?,
                    items.len()
                ))
            }
            Expr::Member(value, field) => {
                if let Expr::Var(name) = &value.node
                    && let Some(ty) = self.resources.get(&format!("{name}.{field}"))
                {
                    return Ok(ty.clone());
                }
                let ty = self.expr(value, vars)?;
                self.field(&ty, field)
            }
            Expr::Index { array, index } => {
                let ty = self.expr(array, vars)?;
                self.expr(index, vars)?;
                if let Some(ty) = ty.strip_prefix("buffer<").and_then(|s| s.strip_suffix('>')) {
                    return Ok(canonical(ty));
                }
                if let Some(inner) = ty.strip_prefix("array<").and_then(|s| s.strip_suffix('>')) {
                    let (element, _) = inner
                        .rsplit_once(',')
                        .ok_or("schema array requires a concrete size")?;
                    return Ok(canonical(element.trim()));
                }
                if ty.starts_with("mat") {
                    return Ok(ty.replacen("mat", "vec", 1));
                }
                self.field(&ty, "x")
            }
            Expr::Unary(_, value) => self.expr(value, vars),
            Expr::Binary(op, a, b) => {
                let left = self.expr(a, vars)?;
                let right = self.expr(b, vars)?;
                if self.expected_return.is_some() {
                    if matches!(op, BinOp::LogicalAnd | BinOp::LogicalOr) {
                        if left != "bool" || right != "bool" {
                            return Err("logical operands must be boolean".into());
                        }
                        return Ok("bool".into());
                    }
                    let product = matches!(op, BinOp::Mul | BinOp::Div)
                        && ((left == "f32" && right.starts_with("vec"))
                            || (right == "f32" && left.starts_with("vec"))
                            || (left.starts_with("mat") && right.starts_with("vec")));
                    if left != right && !product {
                        return Err(format!(
                            "incompatible draw shader operands `{left}` and `{right}`"
                        ));
                    }
                }
                if matches!(
                    op,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                ) {
                    return Ok("bool".into());
                }
                if left.starts_with("mat") && right.starts_with("vec") {
                    return Ok(right);
                }
                Ok(if left == "f32" && right.starts_with("vec") {
                    right
                } else {
                    left
                })
            }
            Expr::Pipe {
                recv,
                name,
                name_span,
                args,
            } if matches!(&recv.node, Expr::Var(input) if self.resources.get(input).is_some_and(|ty| self.services.iter().any(|i| i.name == *ty))) =>
            {
                let receiver = self.expr(recv, vars)?;
                let interface = self
                    .services
                    .iter()
                    .find(|i| i.name == receiver)
                    .expect("service receiver");
                let method = interface
                    .methods
                    .iter()
                    .find(|m| m.name == *name)
                    .ok_or_else(|| format!("unknown shader service method `{receiver}.{name}`"))?;
                if args.iter().any(|a| a.name.is_some()) || args.len() != method.params.len() {
                    return Err(format!(
                        "shader service method `{receiver}.{name}` requires its explicit positional arguments"
                    ));
                }
                for (argument, parameter) in args.iter_mut().zip(&method.params) {
                    let ty = self.expr(&mut argument.value, vars)?;
                    if ty != canonical(&parameter.ty_name) {
                        return Err(format!(
                            "shader service method `{receiver}.{name}` requires `{}`, found `{ty}`",
                            parameter.ty_name
                        ));
                    }
                }
                let Expr::Var(input) = &recv.node else {
                    unreachable!("checked service receiver");
                };
                self.service_calls.push(ServiceCall {
                    input: input.clone(),
                    method: name.clone(),
                    receiver: recv.span.clone(),
                    method_span: name_span.clone(),
                });
                Ok(method
                    .ret_ty
                    .as_ref()
                    .map_or_else(|| "void".into(), |(ty, _)| canonical(ty)))
            }
            Expr::Pipe {
                recv, name, args, ..
            } if matches!(&recv.node,Expr::Var(s) if s=="factory") && name == "transform" => {
                for arg in args {
                    self.expr(&mut arg.value, vars)?;
                }
                Ok("mat4".into())
            }
            Expr::Pipe {
                recv, name, args, ..
            } if matches!(name.as_str(), "sample_level" | "sample_grad") => {
                let receiver = self.expr(recv, vars)?;
                let types = args
                    .iter_mut()
                    .map(|arg| self.expr(&mut arg.value, vars))
                    .collect::<Result<Vec<_>, _>>()?;
                let expected: &[&str] = if name == "sample_level" {
                    &["sampler", "vec2", "f32"]
                } else {
                    &["sampler", "vec2", "vec2", "vec2"]
                };
                if receiver != "texture_2d<f32>"
                    || args.iter().any(|arg| arg.name.is_some())
                    || types != expected
                {
                    return Err(format!(
                        "{name} requires a floating-point sampled image and positional arguments {expected:?}"
                    ));
                }
                let function = if name == "sample_level" {
                    "textureSampleLevel"
                } else {
                    "textureSampleGrad"
                };
                let mut lowered = vec![crate::ast::Arg {
                    name: None,
                    value: *recv.clone(),
                }];
                lowered.extend(args.iter().cloned());
                e.node = Expr::Call {
                    name: function.into(),
                    name_span: e.span.clone(),
                    const_args: vec![],
                    args: lowered,
                };
                Ok("vec4".into())
            }
            Expr::Call {
                name,
                name_span,
                args,
                ..
            } => {
                if name == "evaluate_schema" {
                    for arg in args
                        .iter_mut()
                        .filter(|a| a.name.as_deref() != Some("variant"))
                    {
                        self.expr(&mut arg.value, vars)?;
                    }
                    return super::super::schema_evaluation::result_type(
                        self.material
                            .ok_or("operation requires an explicit material evaluator argument")?,
                        args,
                    )
                    .map(|ty| canonical(&ty));
                }

                let types = args
                    .iter_mut()
                    .map(|a| self.expr(&mut a.value, vars))
                    .collect::<Result<Vec<_>, _>>()?;
                for (index, (arg, ty)) in args.iter().zip(&types).enumerate() {
                    if texture_integer_argument(name, index)
                        && integer_literal(&arg.value)?.is_none()
                        && !matches!(ty.as_str(), "i32" | "u32")
                    {
                        return Err("texture integer argument requires i32 or u32".into());
                    }
                }
                let candidates: Vec<_> = self
                    .pass
                    .hooks
                    .iter()
                    .enumerate()
                    .filter(|(_, h)| h.name == *name)
                    .collect();
                if !candidates.is_empty() {
                    let mut matching = Vec::new();
                    for (index, hook) in candidates {
                        if hook.params.len() != types.len() {
                            continue;
                        }
                        let mut score = 0;
                        let mut valid = true;
                        for (param, actual) in hook.params.iter().zip(&types) {
                            let expected = canonical(&param.ty_name);
                            if &expected == actual {
                                continue;
                            }
                            if let Some(distance) = self.material_distance(actual, &expected) {
                                score += distance;
                            } else {
                                valid = false;
                                break;
                            }
                        }
                        if valid {
                            matching.push((score, index));
                        }
                    }
                    matching.sort();
                    let Some(&(score, index)) = matching.first() else {
                        return Err(format!(
                            "no overload of `{name}` accepts ({})",
                            types.join(", ")
                        ));
                    };
                    if matching.get(1).is_some_and(|(other, _)| *other == score) {
                        return Err(format!("ambiguous raster call `{name}`"));
                    }
                    self.needed.insert(index);
                    if let Some(links) = &mut self.links {
                        links
                            .tokens
                            .insert(name_span.start, self.names[index].clone());
                    }
                    name.clone_from(&self.names[index]);
                    let hook = &self.pass.hooks[index];
                    if let Some(plan) = &hook.dispatch
                        && plan.draw_scoped
                        && plan.cases.is_empty()
                    {
                        return Err(format!(
                            "draw-scoped dispatch requires a selected `{}` implementation",
                            plan.contract
                        ));
                    }
                    let result = hook
                        .return_ty
                        .as_ref()
                        .map_or("void", |ty| ty.node.as_str());
                    if hook
                        .attrs
                        .iter()
                        .any(|a| a.name == "evaluate" && a.args.as_slice() == ["surface"])
                    {
                        return self
                            .material
                            .ok_or("operation cannot capture the current material")?
                            .material_properties_name
                            .clone()
                            .ok_or("surface evaluator requires material properties".into());
                    }
                    return Ok(canonical(result));
                }
                let imported: Vec<_> = self
                    .imports
                    .iter()
                    .enumerate()
                    .filter(|(_, function)| {
                        function.name == *name && !function.is_builtin && !function.is_internal
                    })
                    .collect();
                if !imported.is_empty() {
                    if args.iter().any(|argument| argument.name.is_some()) {
                        return Err(format!(
                            "imported GPU function `{name}` requires positional arguments"
                        ));
                    }
                    let mut matching = Vec::new();
                    for candidate @ (_, function) in imported {
                        if function.params.len() != types.len() {
                            continue;
                        }
                        let mut matches = true;
                        for (param, ty) in function.params.iter().zip(&types) {
                            let compatible =
                                if crate::resource_type::shader_resource_type(&param.ty_name)
                                    .is_some()
                                {
                                    super::super::schema_function::resource_argument_matches(
                                        &param.ty_name,
                                        ty,
                                    )?
                                } else {
                                    canonical(&param.ty_name) == *ty
                                };
                            if !compatible {
                                matches = false;
                                break;
                            }
                        }
                        if matches {
                            matching.push(candidate);
                        }
                    }
                    let [(index, function)] = matching.as_slice() else {
                        return Err(format!(
                            "no unique imported raster overload `{name}` for ({})",
                            types.join(", ")
                        ));
                    };
                    *name = super::super::schema_function::library_symbol(*index);
                    self.needed_imports.insert(*index);
                    return Ok(canonical(
                        function.ret_ty.as_ref().map_or("void", |(ty, _)| ty),
                    ));
                }
                if let Some(record) = find_struct(self.structs, name) {
                    if self.expected_return.is_some()
                        && (record.fields.len() != types.len()
                            || record
                                .fields
                                .iter()
                                .zip(&types)
                                .any(|(f, t)| canonical(&f.ty_name) != *t))
                    {
                        return Err(format!(
                            "draw constructor `{name}` requires its declared field types"
                        ));
                    }
                    return Ok(name.clone());
                }
                if *name == self.input_name {
                    return Ok(name.clone());
                }
                if wgsl_type(name).is_ok() {
                    return Ok(canonical(name));
                }
                if name == "rgba" {
                    return Ok("vec4".into());
                }
                if name == "discard_fragment" && args.is_empty() {
                    return Ok("void".into());
                }
                match name.as_str() {
                    "textureDimensions"
                        if types.first().is_some_and(|ty| {
                            ty.starts_with("texture_2d<") || ty == "texture_depth_2d"
                        }) =>
                    {
                        Ok("uvec2".into())
                    }
                    "textureLoad" => match types.first().map(String::as_str) {
                        Some("texture_2d<f32>") => Ok("vec4".into()),
                        Some("texture_2d<u32>") => Ok("uvec4".into()),
                        Some("texture_2d<i32>") => Ok("ivec4".into()),
                        Some("texture_depth_2d") => Ok("f32".into()),
                        _ => Err("textureLoad requires a supported sampled texture".into()),
                    },
                    "length" | "distance" | "dot" | "determinant" => Ok("f32".into()),
                    "select" => types
                        .first()
                        .cloned()
                        .ok_or("select requires arguments".into()),
                    "normalize" | "abs" | "max" | "min" | "clamp" | "sin" | "cos" | "tan"
                    | "ceil" | "floor" | "fract" | "sqrt" | "inverse_sqrt" | "inverseSqrt"
                    | "exp" | "pow" | "mix" | "cross" | "reflect" | "sign" | "smoothstep"
                    | "step" | "dpdx" | "dpdy" => types
                        .first()
                        .cloned()
                        .ok_or_else(|| format!("{name} requires arguments")),
                    _ => Err(format!("unresolved raster function `{name}`")),
                }
            }
            other => Err(format!("unsupported typed raster expression: {other:?}")),
        }
    }
    fn statements(
        &mut self,
        body: &mut [Stmt],
        vars: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        for stmt in body {
            if self.expected_return.is_some() {
                let declared = match &*stmt {
                    Stmt::Let { name, .. } | Stmt::Const { name, .. } | Stmt::For { name, .. } => {
                        Some(name)
                    }
                    _ => None,
                };
                if declared.is_some_and(|name| {
                    self.resources.contains_key(name) || name.starts_with("__fresco_")
                }) {
                    return Err("draw shader local shadows an operation input or uses a reserved binding name".into());
                }
            }
            match stmt {
                Stmt::Let {
                    name,
                    value,
                    declared_ty_name,
                    ..
                } => {
                    let ty = self.expr(value, vars)?;
                    if crate::driver::shader_iterators::element(&ty).is_some() {
                        return Err("shader iterators must be consumed directly by for".into());
                    }
                    if self.expected_return.is_some()
                        && declared_ty_name
                            .as_deref()
                            .is_some_and(|declared| canonical(declared) != ty)
                    {
                        return Err(format!(
                            "draw shader local `{name}` has incompatible initializer `{ty}`"
                        ));
                    }
                    vars.insert(
                        name.clone(),
                        declared_ty_name.as_deref().map(canonical).unwrap_or(ty),
                    );
                }
                Stmt::Const {
                    name,
                    ty_name,
                    value,
                    ..
                } => {
                    let ty = self.expr(value, vars)?;
                    if crate::driver::shader_iterators::element(&ty).is_some() {
                        return Err("shader iterators must be consumed directly by for".into());
                    }
                    if self.expected_return.is_some() && canonical(ty_name) != ty {
                        return Err(format!(
                            "draw constant `{name}` has incompatible initializer"
                        ));
                    }
                    vars.insert(name.clone(), canonical(ty_name));
                }
                Stmt::Return { .. } | Stmt::ReturnVoid { .. } | Stmt::Break { .. }
                    if self
                        .expected_return
                        .as_deref()
                        .and_then(crate::driver::shader_iterators::element)
                        .is_some() =>
                {
                    return Err("shader iterators complete by falling through; early return and break are not supported".into());
                }
                Stmt::Expr(value) if matches!(&value.node, Expr::Call { name, .. } if name == "yield") =>
                {
                    let element = self
                        .expected_return
                        .as_deref()
                        .and_then(crate::driver::shader_iterators::element)
                        .map(canonical)
                        .ok_or("yield is only valid inside a shader iterator")?;
                    let Expr::Call {
                        args, const_args, ..
                    } = &mut value.node
                    else {
                        unreachable!()
                    };
                    if args.len() != 1 || args[0].name.is_some() || !const_args.is_empty() {
                        return Err("yield requires one positional value".into());
                    }
                    let actual = self.expr(&mut args[0].value, vars)?;
                    if actual != element {
                        return Err(format!(
                            "shader iterator yields `{element}`, found `{actual}`"
                        ));
                    }
                }
                Stmt::Block { body, .. } => self.statements(body, &mut vars.clone())?,
                Stmt::Return { value, .. } if self.expected_return.is_some() => {
                    let actual = self.expr(value, vars)?;
                    if self.expected_return.as_ref() != Some(&actual) {
                        return Err(format!(
                            "draw shader return requires `{}`, found `{actual}`",
                            self.expected_return.as_ref().expect("checked return")
                        ));
                    }
                }
                Stmt::Assign {
                    name,
                    field_path,
                    value,
                    ..
                } if self.expected_return.is_some() => {
                    if self.links.is_some() && self.resources.contains_key(name) {
                        return Err("shader services cannot mutate captured bindings".into());
                    }
                    let target = field_path
                        .as_ref()
                        .map_or_else(|| name.clone(), |p| format!("{name}.{p}"));
                    let expected = self.variable(&target, vars)?;
                    let actual = self.expr(value, vars)?;
                    if expected != actual {
                        return Err(format!(
                            "draw assignment to `{target}` requires `{expected}`, found `{actual}`"
                        ));
                    }
                }
                Stmt::Assign { value, .. } | Stmt::Return { value, .. } | Stmt::Expr(value) => {
                    self.expr(value, vars)?;
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    // Configuration is already resolved by the shared compile-time evaluator.
                    if let Some(selected) = self
                        .material
                        .and_then(|m| super::super::surface_properties::static_condition(m, cond))
                    {
                        if selected {
                            self.statements(then_body, &mut vars.clone())?;
                        } else if let Some(body) = else_body {
                            self.statements(body, &mut vars.clone())?;
                        }
                    } else {
                        let ty = self.expr(cond, vars)?;
                        if self.expected_return.is_some() && ty != "bool" {
                            return Err("draw shader condition must be boolean".into());
                        }
                        self.statements(then_body, &mut vars.clone())?;
                        if let Some(body) = else_body {
                            self.statements(body, &mut vars.clone())?;
                        }
                    }
                }
                Stmt::For {
                    name,
                    iterable,
                    body,
                    index_name,
                    ..
                } => {
                    if index_name.is_some() {
                        return Err("indexed raster iteration is not supported".into());
                    }
                    if !matches!(iterable.node, Expr::Range(..)) {
                        let original = iterable.clone();
                        let ty = self.expr(iterable, vars)?;
                        let element = crate::driver::shader_iterators::element(&ty)
                            .ok_or("raster loop requires a range or shader iterator")?
                            .to_owned();
                        if self.material.is_none() {
                            let mut inner = vars.clone();
                            inner.insert(name.clone(), canonical(&element));
                            self.statements(body, &mut inner)?;
                            continue;
                        }
                        let Expr::Call { name: callable, .. } = &iterable.node else {
                            return Err(
                                "shader iterator must resolve to a pass-local helper".into()
                            );
                        };
                        let index = self
                            .names
                            .iter()
                            .position(|name| name == callable)
                            .ok_or("shader iterator must resolve to a pass-local helper")?;
                        let Expr::Call { args, .. } = &original.node else {
                            return Err("shader iterator must be called directly".into());
                        };
                        self.iterator_expansions = self
                            .iterator_expansions
                            .checked_add(1)
                            .filter(|count| *count <= 256)
                            .ok_or(
                                "shader iterator expansion exceeds 256 calls; check for recursion",
                            )?;
                        self.needed.remove(&index);
                        let mut occupied: BTreeSet<String> =
                            vars.keys().chain(self.resources.keys()).cloned().collect();
                        for hook in &self.pass.hooks {
                            occupied.extend(hook.params.iter().map(|p| p.name.clone()));
                            occupied.extend(hook.body.iter().filter_map(|(token, _)| {
                                crate::parser::identifier_token(token).map(str::to_owned)
                            }));
                        }
                        let mut expanded = crate::driver::shader_iterators::Expansion {
                            occupied: &mut occupied,
                            serial: &mut self.iterator_serial,
                            variable: name,
                            consumer: body,
                            element: &element,
                            span: &original.span,
                        }
                        .expand(&self.pass.hooks[index], args)?;
                        self.statements(std::slice::from_mut(&mut expanded), vars)?;
                        *stmt = expanded;
                        continue;
                    }
                    let Expr::Range(a, b) = &mut iterable.node else {
                        return Err("raster loop requires a range".into());
                    };
                    self.expr(a, vars)?;
                    self.expr(b, vars)?;
                    let mut inner = vars.clone();
                    inner.insert(name.clone(), "u32".into());
                    self.statements(body, &mut inner)?;
                }
                other => return Err(format!("unsupported typed raster statement: {other:?}")),
            }
        }
        Ok(())
    }
}

pub(super) fn output_fields(record: &StructDecl) -> Result<Vec<ManifestStageOutput>, String> {
    let mut outputs = Vec::new();
    let mut locations = BTreeSet::new();
    for field in &record.fields {
        let attrs: Vec<_> = field
            .attrs
            .iter()
            .filter(|(name, _)| name == "location")
            .collect();
        let location = match attrs.as_slice() {
            [(_, args)] if args.len() == 1 => args[0]
                .parse::<u32>()
                .map_err(|_| "output location must be a u32")?,
            _ => {
                return Err(format!(
                    "output `{}.{}` requires one @location(index)",
                    record.name, field.name
                ));
            }
        };
        if !locations.insert(location) {
            return Err(format!("duplicate output location {location}"));
        }
        let ty = canonical(&field.ty_name);
        if !matches!(
            ty.as_str(),
            "f32" | "i32" | "u32" | "vec2" | "vec3" | "vec4" | "uvec2" | "uvec3" | "uvec4"
        ) {
            return Err(format!("invalid fragment output type `{ty}`"));
        }
        outputs.push(ManifestStageOutput {
            location,
            name: field.name.clone(),
            ty,
        });
    }
    outputs.sort_by_key(|o| o.location);
    Ok(outputs)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Threads independent typed mesh emission inputs."
)]
pub(super) fn emit(
    pass: &PassDecl,
    library: &crate::check::compute::ComputeLibrary,
    factory: &VertexFactoryDecl,
    material: &MaterialHir,
    structs: &[StructDecl],
    members: &[&VertexInterfaceMemberDecl],
    input_ty: &str,
    varying_ty: &str,
    entry_base: &str,
    transform_entry: &str,
    resource_decls: &str,
    base: &ExprContext<'_>,
) -> Result<MeshStage, String> {
    for hook in &pass.hooks {
        for attr in &hook.attrs {
            if !matches!(
                attr.name.as_str(),
                "vertex" | "fragment" | "evaluate" | "prepare"
            ) {
                return Err(format!(
                    "unsupported raster function attribute `@{}`",
                    attr.name
                ));
            }
        }
        if hook.attrs.iter().any(|a| a.name == "evaluate")
            && hook
                .attrs
                .iter()
                .any(|a| matches!(a.name.as_str(), "vertex" | "fragment"))
        {
            return Err("an evaluator binding cannot also be a GPU entry".into());
        }
    }
    let vertex_indices: Vec<_> = pass
        .hooks
        .iter()
        .enumerate()
        .filter(|(_, h)| h.attrs.iter().any(|a| a.name == "vertex"))
        .map(|(i, _)| i)
        .collect();
    let Some(vertex_index) = vertex_indices.first() else {
        return Err("explicit raster pass requires a @vertex function".into());
    };
    let vertex = &pass.hooks[*vertex_index];
    for index in &vertex_indices {
        let entry = &pass.hooks[*index];
        if entry.params.len() != 1
            || entry.params[0].ty_name != vertex.params[0].ty_name
            || entry.return_ty.as_ref().map(|ty| &ty.node)
                != vertex.return_ty.as_ref().map(|ty| &ty.node)
        {
            return Err(
                "mesh vertex entries must share the factory input and varying types".into(),
            );
        }
    }
    let preparations: Vec<_> = pass
        .hooks
        .iter()
        .enumerate()
        .filter(|(_, h)| h.attrs.iter().any(|a| a.name == "prepare"))
        .collect();
    if preparations.len() > 1 {
        return Err("mesh pass accepts one preparation function".into());
    }
    let preparation = preparations.first().copied();
    if let Some((_, hook)) = preparation
        && (hook.params.len() != 1
            || hook
                .return_ty
                .as_ref()
                .is_none_or(|t| t.node != vertex.params[0].ty_name))
    {
        return Err(
            "preparation must accept one raw vertex and return the vertex entry's input record"
                .into(),
        );
    }
    let input_name = canonical(&preparation.map_or(vertex, |(_, h)| h).params[0].ty_name);
    let mut aliases = HashMap::from([
        (input_name.clone(), input_ty.into()),
        (canonical(base.varying_source_ty), varying_ty.into()),
    ]);
    aliases.insert(
        material.context.ty.name.clone(),
        material.context.ty.name.clone(),
    );
    for name in material.material_schemas.keys() {
        aliases.insert(name.clone(), format!("FrescoMaterial_{}", material.name));
    }
    aliases.extend(material.evaluation_type_aliases.clone());
    // Uniform records passed to ordinary functions retain their declared identity.
    if let Some(resources) = base.type_aliases {
        aliases.extend(resources.clone());
    }
    for (index, function) in library.functions().iter().enumerate() {
        if !function.is_builtin && !function.is_internal {
            aliases.insert(
                function.name.clone(),
                format!("{entry_base}_import_{}", function.name),
            );
            aliases.insert(
                super::super::schema_function::library_symbol(index),
                if library
                    .functions()
                    .iter()
                    .filter(|f| f.name == function.name)
                    .count()
                    > 1
                {
                    format!("{entry_base}_import_{}_{index}", function.name)
                } else {
                    format!("{entry_base}_import_{}", function.name)
                },
            );
        }
    }
    let referenced_calls = std::cell::RefCell::new(BTreeSet::new());
    let resource_specializations = ResourceSpecializations::default();
    let mut resources = HashMap::new();
    for binding in factory.bindings.iter().chain(&pass.bindings) {
        let signature = binding
            .value_signature
            .as_ref()
            .ok_or("resource signature missing")?;
        resources.insert(
            binding.name.clone(),
            uniform_type(signature)
                .map(canonical)
                .unwrap_or_else(|| signature.clone()),
        );
        if let Some(name) = &binding.operation_alias
            && resources
                .insert(
                    name.clone(),
                    uniform_type(signature)
                        .map(canonical)
                        .unwrap_or_else(|| signature.clone()),
                )
                .is_some()
        {
            return Err("operation binding alias is ambiguous".into());
        }
    }
    let mut functions = Functions {
        pass,
        imports: library.functions(),
        material: Some(material),
        expected_return: None,
        structs,
        names: pass
            .hooks
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{entry_base}_fn_{i}_{}", sanitize(&h.name)))
            .collect(),
        needed: BTreeSet::new(),
        needed_imports: BTreeSet::new(),
        links: None,
        services: &[],
        service_calls: Vec::new(),
        iterator_serial: 0,
        iterator_expansions: 0,
        resources,
        input_fields: members
            .iter()
            .map(|m| (m.name.clone(), canonical(&m.ty_name)))
            .collect(),
        input_name,
    };
    if let Some((index, hook)) = preparation {
        functions.needed.insert(index);
        let name = &hook.return_ty.as_ref().expect("checked preparation").node;
        aliases
            .entry(name.clone())
            .or_insert_with(|| format!("{entry_base}_import_type_{}", sanitize(name)));
    }
    let mut entries = Vec::new();
    let mut wrappers = String::new();
    let mut declarations = String::new();
    for (index, hook) in pass.hooks.iter().enumerate() {
        let stages: Vec<_> = hook
            .attrs
            .iter()
            .filter(|a| matches!(a.name.as_str(), "vertex" | "fragment"))
            .collect();
        if stages.is_empty() {
            continue;
        }
        if stages.len() != 1 || !stages[0].args.is_empty() {
            return Err("stage function requires one argument-free stage attribute".into());
        }
        let stage = &stages[0].name;
        let result = hook
            .return_ty
            .as_ref()
            .map_or("void", |ty| ty.node.as_str());
        if stage == "vertex" && result == "void" {
            return Err("vertex stage requires a return type".into());
        }
        let outputs = if stage == "vertex" || result == "void" {
            Vec::new()
        } else if let Some(record) = find_struct(structs, result) {
            output_fields(record)?
        } else {
            vec![ManifestStageOutput {
                location: 0,
                name: "color".into(),
                ty: canonical(result),
            }]
        };
        if stage == "fragment" && !outputs.is_empty() && find_struct(structs, result).is_some() {
            let name = format!("{entry_base}_{}", sanitize(result));
            if aliases.insert(result.to_string(), name.clone()).is_none() {
                declarations.push_str(&format!("struct {name} {{\n"));
                // Preserve constructor field order while reflecting explicit locations.
                for field in &find_struct(structs, result)
                    .expect("selected output")
                    .fields
                {
                    let output = outputs
                        .iter()
                        .find(|o| o.name == field.name)
                        .expect("validated field");
                    declarations.push_str(&format!(
                        "@location({}) {}: {},\n",
                        output.location,
                        shader_identifier(&field.name),
                        wgsl_type(&field.ty_name)?
                    ));
                }
                declarations.push_str("}\n");
            }
        }
        let ty = if result == "void" {
            String::new()
        } else {
            aliases
                .get(result)
                .cloned()
                .map_or_else(|| wgsl_type(result), Ok)?
        };
        let mut params = Vec::new();
        let mut args = Vec::new();
        for param in &hook.params {
            let ty = aliases
                .get(&canonical(&param.ty_name))
                .cloned()
                .map_or_else(|| wgsl_type(&param.ty_name), Ok)?;
            let mut attrs = String::new();
            for attr in &param.attrs {
                if attr.name != "builtin" || attr.args.len() != 1 {
                    return Err(
                        "stage parameter requires @builtin(name) or a varying struct".into(),
                    );
                }
                attrs.push_str(&format!("@builtin({}) ", attr.args[0]));
            }
            if stage == "vertex"
                && let Some((prepare_index, _)) = preparation
            {
                if pass.preparation.is_some() {
                    params.push("@builtin(vertex_index) fresco_prepared_index: u32".into());
                    args.push(format!(
                        "{}[fresco_prepared_index]",
                        base.binding_vars["__prepare_vertices"]
                    ));
                } else {
                    params.push(format!("{}: {input_ty}", shader_identifier(&param.name)));
                    args.push(format!(
                        "{}({})",
                        functions.names[prepare_index],
                        shader_identifier(&param.name)
                    ));
                }
            } else {
                params.push(format!("{attrs}{}: {ty}", shader_identifier(&param.name)));
                args.push(shader_identifier(&param.name));
            }
        }
        let entry = format!("{entry_base}_{}", sanitize(&hook.name));
        let location = if stage == "fragment" && find_struct(structs, result).is_none() {
            "@location(0) "
        } else {
            ""
        };
        let (output, returning) = if result == "void" {
            (String::new(), "")
        } else {
            (format!(" -> {location}{ty}"), "return ")
        };
        wrappers.push_str(&format!(
            "@{stage} fn {entry}({}){output} {{ {returning}{}({}); }}\n",
            params.join(", "),
            functions.names[index],
            args.join(", ")
        ));
        entries.push(ManifestRasterEntry {
            function: hook.name.clone(),
            entry,
            stage: stage.clone(),
            outputs,
        });
        functions.needed.insert(index);
    }
    let existing_types: BTreeSet<_> = aliases.keys().cloned().collect();
    for record in structs {
        aliases
            .entry(record.name.clone())
            .or_insert_with(|| format!("{entry_base}_import_type_{}", sanitize(&record.name)));
    }
    let transform = factory
        .hooks
        .iter()
        .find(|h| h.name == "transform")
        .ok_or("factory transform missing")?;
    let body = crate::parser::executable_pass_hook_body(transform)
        .map_err(|e| format!("invalid transform: {e:?}"))?;
    let mut code = format!(
        "{resource_decls}{}{}\nfn {transform_entry}({}: {input_ty})->mat4x4<f32>{{\n{}}}\n",
        emit_vertex_input(input_ty, members)?,
        emit_varying(
            find_struct(structs, base.varying_source_ty).ok_or("varying type missing")?,
            varying_ty
        )?,
        transform.params[0].name,
        emit_statements(&body, base)?
    );
    let mut done = BTreeSet::new();
    while let Some(index) = functions
        .needed
        .iter()
        .find(|i| !done.contains(*i))
        .copied()
    {
        done.insert(index);
        let hook = &pass.hooks[index];
        let mut vars: HashMap<_, _> = hook
            .params
            .iter()
            .map(|p| (p.name.clone(), canonical(&p.ty_name)))
            .collect();
        // A material parameter is specialized to the full concrete schema without dropping channels.
        for ty in vars.values_mut() {
            if material.material_schemas.contains_key(ty) {
                *ty = material
                    .material_properties_name
                    .clone()
                    .ok_or("material schema missing")?;
            }
        }
        let result = hook
            .return_ty
            .as_ref()
            .map_or("void", |ty| ty.node.as_str());
        let context = ExprContext {
            resource_specializations: Some(&resource_specializations),
            imported_functions: Some(library.functions()),
            imported_records: None,
            referenced_calls: Some(&referenced_calls),
            type_aliases: Some(&aliases),
            ..base.clone()
        };
        let params = hook
            .params
            .iter()
            .map(|p| {
                Ok(format!(
                    "{}: {}",
                    shader_identifier(&p.name),
                    context_type(&p.ty_name, &context)?
                ))
            })
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        let evaluators: Vec<_> = hook.attrs.iter().filter(|a| a.name == "evaluate").collect();
        let body = if !evaluators.is_empty() {
            let [attr] = evaluators.as_slice() else {
                return Err("duplicate evaluator binding".into());
            };
            if hook.params.len() != 1
                || canonical(&hook.params[0].ty_name) != material.context.ty.name
            {
                return Err(
                    "surface evaluator requires the authored surface context parameter".into(),
                );
            }
            let parsed = crate::parser::executable_pass_hook_body(hook)
                .map_err(|e| format!("invalid evaluator binding: {e:?}"))?;
            if !parsed.is_empty() {
                return Err("bound evaluator declaration must have an empty body".into());
            }
            match attr.args.as_slice() {
                [kind] if kind == "surface" => {
                    if functions
                        .material_distance(
                            material
                                .material_properties_name
                                .as_deref()
                                .ok_or("material missing")?,
                            result,
                        )
                        .is_none()
                    {
                        return Err(
                            "evaluator return schema must accept the selected surface".into()
                        );
                    }
                    format!(
                        "return fresco_{}({});",
                        sanitize(&material.name),
                        hook.params[0].name
                    )
                }
                [kind] if kind == "vertex" && result == material.context.ty.name => {
                    if material.vertex_program.is_some() {
                        format!(
                            "return fresco_surface_vertex_{}({});",
                            sanitize(&material.name),
                            hook.params[0].name
                        )
                    } else {
                        format!("return {};", hook.params[0].name)
                    }
                }
                _ => {
                    return Err(
                        "@evaluate requires surface or vertex and a compatible result".into(),
                    );
                }
            }
        } else {
            let mut body = crate::parser::executable_pass_hook_body(hook)
                .map_err(|e| format!("invalid raster function: {e:?}"))?;
            functions.statements(&mut body, &mut vars)?;
            emit_statements(&body, &context)?
        };
        let output = if result == "void" {
            String::new()
        } else {
            format!("->{}", context_type(result, &context)?)
        };
        code.push_str(&format!(
            "fn {}({params}){output}{{\n{body}\n}}\n",
            functions.names[index]
        ));
    }
    let (imported, mut needed_types) = link_imports(
        library.functions(),
        structs,
        &aliases,
        referenced_calls.into_inner(),
        &resource_specializations,
    )?;
    let mut emitted_types = existing_types;
    if let Some((_, hook)) = preparation {
        let name = &hook.return_ty.as_ref().expect("checked preparation").node;
        if pass.preparation.is_none() {
            emitted_types.remove(name);
            needed_types.insert(name.clone());
        }
    }
    while let Some(name) = needed_types.pop_first() {
        if !emitted_types.insert(name.clone()) {
            continue;
        }
        let record = find_struct(structs, &name)
            .ok_or_else(|| format!("unknown imported GPU record `{name}`"))?;
        declarations.push_str(&format!("struct {} {{\n", aliases[&name]));
        let context = ExprContext {
            type_aliases: Some(&aliases),
            ..base.clone()
        };
        for field in &record.fields {
            if !field.attrs.is_empty() {
                return Err(format!(
                    "imported value record `{name}` cannot carry stage attributes"
                ));
            }
            if find_struct(structs, &field.ty_name).is_some() {
                needed_types.insert(field.ty_name.clone());
            }
            declarations.push_str(&format!(
                "{}: {},\n",
                shader_identifier(&field.name),
                context_type(&field.ty_name, &context)?
            ));
        }
        declarations.push_str("}\n");
    }
    code.push_str(&imported);
    code.push_str(&declarations);
    code.push_str(&wrappers);
    let prepared = if let Some(draw) = &pass.preparation {
        let (index, _) = preparation.ok_or("missing preparation function")?;
        let (compute, metadata) = preparation_shader::emit(
            draw,
            members,
            input_ty,
            &functions.names[index],
            base,
            structs,
        )?;
        code.push_str(&compute);
        Some(metadata)
    } else {
        None
    };
    Ok(MeshStage {
        procedural: false,
        shading_inputs: Default::default(),
        bindings: pass.bindings.clone(),
        entries,
        variants: Vec::new(),
        pass: pass.name.clone(),
        factory: factory.name.clone(),
        wgsl: code,
        preparation: prepared,
    })
}

/// Check the complete callable closure, without instantiating a shader stage.
/// Service code may capture only the exporting pass/factory's declared bindings.
pub(in crate::driver) fn validate_services(
    program: &crate::ast::Program,
    pass: &PassDecl,
) -> Result<(), String> {
    service_closure(program, pass, None, "").map(|_| ())
}

pub(in crate::driver) fn service_closure(
    program: &crate::ast::Program,
    pass: &PassDecl,
    selection: Option<(&str, &BTreeSet<String>)>,
    prefix: &str,
) -> Result<ServiceLink, String> {
    let mut linked = ServiceLink {
        hooks: Vec::new(),
        resources: BTreeSet::new(),
    };
    let exports: Vec<_> = pass.attrs.iter().filter(|a| a.name == "service").collect();
    let iterator_roots: Vec<_> = pass
        .hooks
        .iter()
        .enumerate()
        .filter(|(_, hook)| {
            hook.return_ty
                .as_ref()
                .is_some_and(|ty| crate::driver::shader_iterators::element(&ty.node).is_some())
        })
        .map(|(index, _)| index)
        .collect();
    if exports.is_empty() && (iterator_roots.is_empty() || pass.operation.is_some()) {
        return Ok(linked);
    }
    let factory = pass
        .attrs
        .iter()
        .find(|a| a.name == "factory")
        .and_then(|a| a.args.first())
        .map(|name| {
            program
                .vertex_factories
                .iter()
                .find(|f| f.name == *name)
                .ok_or_else(|| format!("unknown shader service factory `{name}`"))
        })
        .transpose()?;
    let mut resources = HashMap::new();
    for binding in pass
        .bindings
        .iter()
        .chain(factory.into_iter().flat_map(|f| &f.bindings))
    {
        let signature = binding
            .value_signature
            .as_ref()
            .ok_or("service binding requires a type")?;
        let ty = uniform_type(signature)
            .map(canonical)
            .unwrap_or_else(|| signature.clone());
        if resources.insert(binding.name.clone(), ty).is_some() {
            return Err(format!(
                "ambiguous shader service binding `{}`",
                binding.name
            ));
        }
    }
    let mut pending = BTreeSet::new();
    if selection.is_none() && pass.operation.is_none() {
        pending.extend(iterator_roots.into_iter().map(|index| (false, index)));
    }
    for export in exports {
        if selection.is_some_and(|(name, _)| export.args.first().is_none_or(|n| n != name)) {
            continue;
        }
        let interface = program
            .interfaces
            .iter()
            .find(|i| export.args.first() == Some(&i.name))
            .ok_or("unknown shader service interface")?;
        for method in &interface.methods {
            if selection.is_some_and(|(_, methods)| !methods.contains(&method.name)) {
                continue;
            }
            let index = pass
                .hooks
                .iter()
                .position(|h| {
                    h.name == method.name
                        && h.params.len() == method.params.len()
                        && h.params
                            .iter()
                            .zip(&method.params)
                            .all(|(a, b)| canonical(&a.ty_name) == canonical(&b.ty_name))
                })
                .ok_or("missing shader service method")?;
            pending.insert((false, index));
        }
    }
    let mut import_scope = pass.clone();
    import_scope.hooks.clear();
    let mut functions = Functions {
        pass,
        imports: &program.functions,
        material: None,
        expected_return: None,
        structs: &program.structs,
        names: pass
            .hooks
            .iter()
            .map(|h| format!("{prefix}{}", h.name))
            .collect(),
        needed: BTreeSet::new(),
        needed_imports: BTreeSet::new(),
        links: None,
        services: &[],
        service_calls: Vec::new(),
        iterator_serial: 0,
        iterator_expansions: 0,
        resources: resources.clone(),
        input_fields: HashMap::new(),
        input_name: String::new(),
    };
    let mut edges = std::collections::BTreeMap::new();
    while let Some(node @ (imported, index)) = pending.pop_first() {
        if edges.contains_key(&node) {
            continue;
        }
        // Imported functions retain module scope. Their callers cannot supply
        // ambient pass helpers or resource bindings by a coincidentally equal name.
        functions.pass = if imported { &import_scope } else { pass };
        functions.resources = if imported {
            HashMap::new()
        } else {
            resources.clone()
        };
        functions.links = (!imported).then(|| FunctionLinks {
            prefix: prefix.into(),
            tokens: std::collections::BTreeMap::new(),
            resources: BTreeSet::new(),
        });
        let (name, params, result, mut body) = if imported {
            let function = &program.functions[index];
            if !function.type_params.is_empty() {
                return Err(
                    "shader service helpers cannot have unresolved generic parameters".into(),
                );
            }
            (
                &function.name,
                function
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), canonical(&p.ty_name)))
                    .collect::<HashMap<_, _>>(),
                function
                    .ret_ty
                    .as_ref()
                    .map_or_else(|| "void".into(), |(ty, _)| canonical(ty)),
                function.body.clone(),
            )
        } else {
            let hook = &pass.hooks[index];
            if !hook.attrs.is_empty()
                || hook.dispatch.is_some()
                || hook.params.iter().any(|p| !p.attrs.is_empty())
            {
                return Err(format!(
                    "shader service cannot call stage, evaluator, or dispatch hook `{}`",
                    hook.name
                ));
            }
            (
                &hook.name,
                hook.params
                    .iter()
                    .map(|p| (p.name.clone(), canonical(&p.ty_name)))
                    .collect::<HashMap<_, _>>(),
                hook.return_ty
                    .as_ref()
                    .map_or_else(|| "void".into(), |ty| canonical(&ty.node)),
                crate::parser::executable_pass_hook_body(hook)
                    .map_err(|e| format!("invalid shader service helper `{}`: {e:?}", hook.name))?,
            )
        };
        let iterator_element = crate::driver::shader_iterators::element(&result);
        if imported && iterator_element.is_some() {
            return Err("shader iterators must be pass-local helpers".into());
        }
        for ty in params
            .values()
            .map(String::as_str)
            .chain(std::iter::once(iterator_element.unwrap_or(&result)))
        {
            if ty != "void"
                && wgsl_type(ty).is_err()
                && find_struct(&program.structs, ty).is_none()
                && crate::resource_type::shader_resource_type(ty).is_none()
            {
                return Err(format!(
                    "shader service helper `{name}` uses unknown shader type `{ty}`"
                ));
            }
        }
        if params.keys().any(|p| functions.resources.contains_key(p)) {
            return Err(format!(
                "shader service helper `{name}` parameter shadows an exported binding"
            ));
        }
        functions.expected_return = Some(result.clone());
        functions.needed.clear();
        functions.needed_imports.clear();
        functions
            .statements(&mut body, &mut params.clone())
            .map_err(|e| format!("shader service helper `{name}`: {e}"))?;
        if result != "void" && iterator_element.is_none() && !returns(&body) {
            return Err(format!(
                "shader service helper `{name}` must return on every path"
            ));
        }
        if let Some(links) = functions.links.take() {
            let mut hook = pass.hooks[index].clone();
            hook.name = format!("{prefix}{}", hook.name);
            for (token, span) in &mut hook.body {
                if let Some(name) = links.tokens.get(&span.start) {
                    if crate::parser::identifier_token(token).is_none() {
                        return Err(
                            "service linker expected an identifier at its resolved reference"
                                .into(),
                        );
                    }
                    *token = crate::lexer::Token::Ident(name.clone().into());
                }
            }
            linked.hooks.push(hook);
            linked.resources.extend(links.resources);
        }
        let dependencies: BTreeSet<_> = functions
            .needed
            .iter()
            .map(|i| (false, *i))
            .chain(functions.needed_imports.iter().map(|i| (true, *i)))
            .collect();
        pending.extend(dependencies.iter().copied());
        edges.insert(node, dependencies);
    }
    // A reachable recursive helper cannot be lowered to any supported GPU stage.
    let mut completed = BTreeSet::new();
    while !edges.is_empty() {
        let ready: Vec<_> = edges
            .iter()
            .filter(|(_, deps)| deps.is_subset(&completed))
            .map(|(node, _)| *node)
            .collect();
        if ready.is_empty() {
            return Err(format!(
                "shader service exported by `{}` contains recursive helper calls",
                pass.name
            ));
        }
        for node in ready {
            edges.remove(&node);
            completed.insert(node);
        }
    }
    Ok(linked)
}

/// Check every definition before graph activation and report stage-reachable hooks.
/// Factory and material resources are deliberately absent from lexical captures.
pub(in crate::driver) fn validate_operation(
    program: &crate::ast::Program,
    pass: &PassDecl,
) -> Result<OperationUse, String> {
    let operation = pass
        .operation
        .as_ref()
        .ok_or("expected operation definition")?;
    let vertex = pass
        .hooks
        .iter()
        .find(|h| h.attrs.iter().any(|a| a.name == "vertex"))
        .ok_or("missing vertex shader")?;
    let input = vertex
        .params
        .first()
        .ok_or("vertex shader requires a typed vertex parameter")?;
    let prepared = operation.inputs.iter().find_map(|p| {
        program
            .resource_types
            .iter()
            .find(|r| r.name == p.ty)
            .map(|r| (p, r))
    });
    let interface = if prepared.is_some() {
        if vertex.params.len() != 1 || input.ty_name != "u32" {
            return Err("prepared draw vertex shader requires one u32 vertex index".into());
        }
        None
    } else {
        Some(
            program
                .vertex_interfaces
                .iter()
                .find(|i| i.name == input.ty_name)
                .ok_or("unknown operation vertex interface")?,
        )
    };
    let mut functions = Functions {
        pass,
        imports: &program.functions,
        material: None,
        expected_return: None,
        structs: &program.structs,
        names: pass.hooks.iter().map(|h| h.name.clone()).collect(),
        needed: BTreeSet::new(),
        needed_imports: BTreeSet::new(),
        links: None,
        services: &program.interfaces,
        service_calls: Vec::new(),
        iterator_serial: 0,
        iterator_expansions: 0,
        resources: operation
            .inputs
            .iter()
            .filter(|p| p.ty != "DrawRange" && !p.ty.starts_with("attachment<"))
            .map(|p| (p.name.clone(), canonical(&p.ty)))
            .collect(),
        input_fields: interface
            .into_iter()
            .flat_map(|i| &i.members)
            .map(|m| (m.name.clone(), canonical(&m.ty_name)))
            .collect(),
        input_name: input.ty_name.clone(),
    };
    for parameter in &operation.inputs {
        if let Ok(resource) =
            crate::driver::compute_operations::resource_type(&parameter.ty, "read")
        {
            functions.resources.insert(
                parameter.name.clone(),
                crate::driver::compute_operations::resource_signature(&resource, false),
            );
            for (member, _) in crate::driver::compute_operations::resource_dimensions(&resource) {
                functions
                    .resources
                    .insert(format!("{}.{member}", parameter.name), "u32".into());
            }
        }
    }
    if let Some((parameter, resource)) = prepared {
        for field in &resource.fields {
            let ty = if let Some(inner) = field.ty_name.replace(' ', "").strip_prefix("buffer<") {
                format!(
                    "buffer<{}>",
                    inner
                        .strip_suffix(",read>")
                        .ok_or("unsupported prepared buffer access")?
                )
            } else {
                canonical(&field.ty_name)
            };
            functions
                .resources
                .insert(format!("{}.{}", parameter.name, field.name), ty);
        }
    }
    let mut edges = std::collections::BTreeMap::new();
    let mut service_calls = std::collections::BTreeMap::new();
    for (index, hook) in pass.hooks.iter().enumerate() {
        let iterator_element = hook
            .return_ty
            .as_ref()
            .and_then(|ty| crate::driver::shader_iterators::element(&ty.node));
        if iterator_element.is_some() && !hook.attrs.is_empty() {
            return Err(
                "shader iterators must be ordinary helpers, not shader entry points".into(),
            );
        }
        for ty in hook.params.iter().map(|p| p.ty_name.as_str()).chain(
            hook.return_ty
                .iter()
                .map(|t| iterator_element.unwrap_or(t.node.as_str())),
        ) {
            if ty != "clip_position"
                && wgsl_type(ty).is_err()
                && find_struct(&program.structs, ty).is_none()
                && ty != input.ty_name
            {
                return Err(format!("unknown draw shader type `{ty}`"));
            }
        }
        if hook.params.iter().any(|p| {
            p.name.starts_with("__fresco_") || operation.inputs.iter().any(|i| i.name == p.name)
        }) {
            return Err("draw shader parameter shadows an operation input".into());
        }
        let mut vars = hook
            .params
            .iter()
            .map(|p| (p.name.clone(), canonical(&p.ty_name)))
            .collect();
        let mut body = crate::parser::executable_pass_hook_body(hook)
            .map_err(|e| format!("invalid draw shader: {e:?}"))?;
        functions.expected_return = Some(
            hook.return_ty
                .as_ref()
                .map_or_else(|| "void".into(), |t| canonical(&t.node)),
        );
        functions.needed.clear();
        functions.statements(&mut body, &mut vars)?;
        edges.insert(index, functions.needed.clone());
        service_calls.insert(index, std::mem::take(&mut functions.service_calls));
        if hook.return_ty.is_some() && iterator_element.is_none() && !returns(&body) {
            return Err(format!(
                "draw shader `{}` must return on every path",
                hook.name
            ));
        }
    }
    let mut unchecked = edges.clone();
    let mut completed = BTreeSet::new();
    while !unchecked.is_empty() {
        let ready: Vec<_> = unchecked
            .iter()
            .filter(|(_, dependencies)| dependencies.is_subset(&completed))
            .map(|(index, _)| *index)
            .collect();
        if ready.is_empty() {
            return Err("draw shader contains recursive helper calls".into());
        }
        for index in ready {
            unchecked.remove(&index);
            completed.insert(index);
        }
    }
    let mut pending: Vec<_> = pass
        .hooks
        .iter()
        .enumerate()
        .filter(|(_, h)| {
            h.attrs
                .iter()
                .any(|a| matches!(a.name.as_str(), "vertex" | "fragment"))
        })
        .map(|(i, _)| i)
        .collect();
    let mut reachable = BTreeSet::new();
    while let Some(index) = pending.pop() {
        if reachable.insert(index) {
            pending.extend(&edges[&index]);
        }
    }
    Ok(OperationUse {
        reachable,
        service_calls,
    })
}
fn returns(body: &[Stmt]) -> bool {
    body.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If {
            then_body,
            else_body: Some(other),
            ..
        } => returns(then_body) && returns(other),
        _ => false,
    })
}
