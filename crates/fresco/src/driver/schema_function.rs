//! Typed instantiation of schema-local ordinary functions.
//!
//! The GPU frontend infers the output type; no response shape is prescribed here.
use super::gpu_function::{
    ExprContext, context_type, emit_expr, emit_statements, shader_identifier,
};
use crate::ast::{Arg, Expr, FnDecl, SExpr, Stmt, StructDecl};
use crate::context::EntryContext;
use crate::material_hir::MaterialChannel;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Resolve the format through the shared resource model before using its shader
/// image type. Format support and device sampling features remain distinct.
fn sampled_type(ty: &str) -> Result<String, String> {
    if ty.starts_with("texture2d<") {
        let resource = super::compute_operations::resource_type(ty, "read")?;
        Ok(super::compute_operations::resource_signature(
            &resource, false,
        ))
    } else {
        Ok(ty.into())
    }
}

pub(super) fn resource_argument_matches(expected: &str, actual: &str) -> Result<bool, String> {
    let expected_type =
        crate::resource_type::shader_resource_type(expected).ok_or("expected resource type")??;
    if let Some(actual) = crate::resource_type::shader_resource_type(actual) {
        return Ok(expected_type == actual?);
    }
    // Engine bindings may expose a sampled image's scalar class without a
    // concrete format. Authored format-qualified handles retain exact identity.
    if matches!(
        expected_type,
        crate::resource_type::ShaderResourceType::Data(crate::resource_type::ResourceType::Image(
            _
        ))
    ) {
        return Ok(sampled_type(expected)? == actual);
    }
    Ok(false)
}

#[derive(Debug, Clone)]
pub(crate) struct SchemaFunction {
    pub functions: Vec<FnDecl>,
    pub imports: Vec<FnDecl>,
    pub structs: Vec<StructDecl>,
    pub output: SExpr,
    pub parameters: Vec<(String, String)>,
    pub bindings: Vec<(String, String)>,
}

pub(crate) struct Instance {
    pub source: String,
    pub result_type: String,
    pub type_definitions: BTreeMap<String, String>,
    pub type_aliases: HashMap<String, String>,
}

fn add_type(
    name: &str,
    program: &SchemaFunction,
    aliases: &HashMap<String, String>,
    cx: &ExprContext<'_>,
    used: &mut HashSet<String>,
    active: &mut HashSet<String>,
    definitions: &mut String,
) -> Result<(), String> {
    if let Some(Ok(crate::resource_type::ShaderResourceType::Data(
        crate::resource_type::ResourceType::Buffer(element),
    ))) = crate::resource_type::shader_resource_type(name)
    {
        add_type(&element, program, aliases, cx, used, active, definitions)?;
    }
    if let Some(inner) = name
        .strip_prefix("array<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let (element, _) = inner
            .rsplit_once(',')
            .ok_or("array requires concrete length")?;
        add_type(
            element.trim(),
            program,
            aliases,
            cx,
            used,
            active,
            definitions,
        )?;
    }
    if used.contains(name) {
        return Ok(());
    }
    if let Some(record) = program.structs.iter().find(|s| s.name == name) {
        if !active.insert(name.into()) {
            return Err(format!("recursive record `{name}`"));
        }
        let mut fields = String::new();
        for field in &record.fields {
            add_type(
                &field.ty_name,
                program,
                aliases,
                cx,
                used,
                active,
                definitions,
            )?;
            fields.push_str(&format!(
                "  {}: {},\n",
                shader_identifier(&field.name),
                context_type(&field.ty_name, cx)?
            ));
        }
        active.remove(name);
        definitions.push_str(&format!("struct {} {{\n{fields}}}\n", aliases[name]));
        used.insert(name.into());
    }
    Ok(())
}

pub(super) fn library_symbol(index: usize) -> String {
    format!("fresco_library_import_{index}")
}

/// Resource arguments are opaque to the signal evaluator. Check their complete
/// ordinary GPU bodies here, including unused declarations, using the same
/// linker and Naga validation as executable shader calls.
pub(super) fn validate_resource_functions(
    program: &crate::ast::Program,
) -> Result<(), Vec<crate::diag::Diag>> {
    for (root, function) in program
        .functions
        .iter()
        .enumerate()
        .filter(|(_, function)| {
            function
                .params
                .iter()
                .map(|p| p.ty_name.as_str())
                .chain(function.ret_ty.iter().map(|(ty, _)| ty.as_str()))
                .any(|ty| crate::resource_type::shader_resource_type(ty).is_some())
        })
    {
        let validate = || -> Result<(), String> {
            let aliases: HashMap<_, _> = program
                .functions
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    (
                        library_symbol(index),
                        format!("fresco_validation_function_{index}"),
                    )
                })
                .chain(program.structs.iter().enumerate().map(|(index, record)| {
                    (record.name.clone(), format!("FrescoValidationType{index}"))
                }))
                .collect();
            let library = SchemaFunction {
                functions: Vec::new(),
                imports: Vec::new(),
                structs: program.structs.clone(),
                output: SExpr {
                    node: Expr::Num(0.0, crate::ast::Unit::None),
                    span: 0..0,
                },
                parameters: Vec::new(),
                bindings: Vec::new(),
            };
            let empty = HashMap::new();
            let context = ExprContext {
                resource_specializations: None,
                material: None,
                imported_functions: None,
                imported_records: Some(&program.structs),
                referenced_calls: None,
                binding_vars: &empty,
                type_aliases: Some(&aliases),
                input_var: "",
                varying_source_ty: "",
                varying_ty: "",
                factory_entry: "",
            };
            let resources = super::gpu_function::ResourceSpecializations::default();
            let mut bindings = BTreeMap::new();
            let mut globals = String::new();
            let mut resource_types = BTreeSet::new();
            for (binding, parameter) in function.params.iter().enumerate() {
                if parameter.ty_name.starts_with("buffer<") {
                    let crate::resource_type::ResourceType::Buffer(element) =
                        crate::resource_type::resource_type(&parameter.ty_name, "read")?
                    else {
                        unreachable!("buffer spelling")
                    };
                    let name = format!("fresco_validation_resource_{binding}");
                    globals.push_str(&format!(
                        "@group(0) @binding({binding}) var<storage, read> {name}: array<{}>;\n",
                        context_type(&element, &context)?
                    ));
                    resource_types.insert(element);
                    bindings.insert(parameter.name.clone(), name);
                }
            }
            let pending = if bindings.is_empty() {
                BTreeSet::from([library_symbol(root)])
            } else {
                resources.register(root, bindings, &aliases[&library_symbol(root)]);
                BTreeSet::new()
            };
            let (mut source, mut types) = super::gpu_function::link_imports(
                &program.functions,
                &program.structs,
                &aliases,
                pending,
                &resources,
            )?;
            source.push_str(&globals);
            types.extend(resource_types);
            let mut used = HashSet::new();
            for ty in types {
                add_type(
                    &ty,
                    &library,
                    &aliases,
                    &context,
                    &mut used,
                    &mut HashSet::new(),
                    &mut source,
                )?;
            }
            let module = naga::front::wgsl::parse_str(&source)
                .map_err(|error| error.emit_to_string(&source))?;
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .map_err(|error| error.emit_to_string(&source))?;
            Ok(())
        };
        validate().map_err(|message| {
            vec![
                crate::diag::Diag::error(
                    function.span.clone(),
                    format!("invalid resource function `{}`: {message}", function.name),
                )
                .with_file(&function.source_file),
            ]
        })?;
    }
    Ok(())
}

/// Check the resolved call graph independently of the emission cache. A helper
/// already emitted for an unrestricted caller must still satisfy a hook's
/// effect contract when reached through dynamic dispatch.
pub(super) fn validate_library_effects(
    root: usize,
    functions: &[FnDecl],
    structs: &[StructDecl],
) -> Result<(), String> {
    let program = SchemaFunction {
        functions: Vec::new(),
        imports: functions.to_vec(),
        structs: structs.to_vec(),
        output: SExpr {
            node: Expr::Num(0.0, crate::ast::Unit::None),
            span: 0..0,
        },
        parameters: Vec::new(),
        bindings: Vec::new(),
    };
    let mut resolver = Instantiator {
        derivative_free: true,
        strict_variables: false,
        program: &program,
        prefix: "fresco_library",
        captures: Vec::new(),
        variables: HashMap::new(),
        aliases: HashMap::new(),
        pending: vec![(false, root)],
        seen: HashSet::from([(false, root)]),
        referenced_types: BTreeSet::new(),
        value_types: HashMap::new(),
        call_types: HashMap::new(),
        inference_errors: HashMap::new(),
    };
    while let Some((_, index)) = resolver.pending.pop() {
        let function = &functions[index];
        resolver.value_types = function
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty_name.clone()))
            .collect();
        resolver.inference_errors.clear();
        let mut body = function
            .typed_body
            .as_ref()
            .unwrap_or(&function.body)
            .clone();
        resolver.statements(&mut body, false).map_err(|error| {
            format!(
                "shading hook `{}` reaches function `{}`: {error}",
                functions[root].name, function.name
            )
        })?;
    }
    Ok(())
}

pub(super) fn resolve_library_body(
    body: &mut [Stmt],
    value_types: HashMap<String, String>,
    functions: &[FnDecl],
    structs: &[StructDecl],
    shadowed: &BTreeSet<String>,
) -> Result<(), String> {
    let mut imports = functions.to_vec();
    for function in &mut imports {
        if shadowed.contains(&function.name) {
            function.is_internal = true;
        }
    }
    let program = SchemaFunction {
        functions: Vec::new(),
        imports,
        structs: structs.to_vec(),
        output: SExpr {
            node: Expr::Num(0.0, crate::ast::Unit::None),
            span: 0..0,
        },
        parameters: Vec::new(),
        bindings: Vec::new(),
    };
    let mut resolver = Instantiator {
        derivative_free: false,
        strict_variables: false,
        program: &program,
        prefix: "fresco_library",
        captures: Vec::new(),
        variables: HashMap::new(),
        aliases: HashMap::new(),
        pending: Vec::new(),
        seen: HashSet::new(),
        referenced_types: BTreeSet::new(),
        value_types,
        call_types: HashMap::new(),
        inference_errors: HashMap::new(),
    };
    resolver.statements(body, false)
}

/// Resolve ordinary library overloads using the same typed call traversal as
/// schema functions, retaining the caller's record aliases and library scope.
pub(super) fn link_library_function(
    root: usize,
    functions: &[FnDecl],
    structs: &[StructDecl],
    aliases: &HashMap<String, String>,
    emitted: &mut BTreeSet<usize>,
    specialization: Option<&super::gpu_function::ResourceSpecialization>,
    resources: &super::gpu_function::ResourceSpecializations,
) -> Result<(String, BTreeSet<String>), String> {
    if specialization.is_none()
        && functions[root]
            .params
            .iter()
            .any(|parameter| parameter.ty_name.starts_with("buffer<"))
    {
        return Err("buffer function requires explicit resource bindings before emission".into());
    }
    if functions[root].derivative_free {
        validate_library_effects(root, functions, structs)?;
    }
    let program = SchemaFunction {
        functions: Vec::new(),
        imports: functions.to_vec(),
        structs: structs.to_vec(),
        output: SExpr {
            node: Expr::Num(0.0, crate::ast::Unit::None),
            span: 0..0,
        },
        parameters: Vec::new(),
        bindings: Vec::new(),
    };
    let mut resolver = Instantiator {
        derivative_free: false,
        strict_variables: functions[root]
            .params
            .iter()
            .any(|p| crate::resource_type::shader_resource_type(&p.ty_name).is_some()),
        program: &program,
        prefix: "fresco_library",
        captures: Vec::new(),
        variables: HashMap::new(),
        aliases: aliases.clone(),
        pending: vec![(false, root)],
        seen: HashSet::from([(false, root)]),
        referenced_types: BTreeSet::new(),
        value_types: HashMap::new(),
        call_types: HashMap::new(),
        inference_errors: HashMap::new(),
    };
    let empty = HashMap::new();
    let context = ExprContext {
        resource_specializations: Some(resources),
        material: None,
        imported_functions: Some(functions),
        imported_records: Some(structs),
        referenced_calls: None,
        binding_vars: &empty,
        type_aliases: Some(aliases),
        input_var: "",
        varying_source_ty: "",
        varying_ty: "",
        factory_entry: "",
    };
    let mut code = String::new();
    while let Some((_, index)) = resolver.pending.pop() {
        let specialized_root = index == root && specialization.is_some();
        if !specialized_root
            && functions[index]
                .params
                .iter()
                .any(|p| p.ty_name.starts_with("buffer<"))
        {
            continue;
        }
        if !specialized_root && !emitted.insert(index) {
            continue;
        }
        let function = &functions[index];
        if index != root && function.derivative_free {
            validate_library_effects(index, functions, structs)?;
        }
        if !function.type_params.is_empty() || !function.const_params.is_empty() {
            return Err(format!(
                "GPU imported function `{}` requires concrete types",
                function.name
            ));
        }
        if function
            .params
            .iter()
            .any(|p| p.default.is_some() || p.keyword_only)
        {
            return Err(format!(
                "GPU imported function `{}` requires positional parameters without defaults",
                function.name
            ));
        }
        resolver.value_types = function
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty_name.clone()))
            .collect();
        resolver.inference_errors.clear();
        let mut body = function
            .typed_body
            .as_ref()
            .unwrap_or(&function.body)
            .clone();
        resolver.statements(&mut body, false)?;
        let buffers = specialization
            .filter(|_| specialized_root)
            .into_iter()
            .flat_map(|specialization| &specialization.bindings)
            .map(|(name, value)| (name.as_str(), value.clone()))
            .collect();
        let context = ExprContext {
            binding_vars: &buffers,
            ..context.clone()
        };
        for parameter in &function.params {
            if parameter.ty_name.starts_with("buffer<") {
                let crate::resource_type::ResourceType::Buffer(element) =
                    crate::resource_type::resource_type(&parameter.ty_name, "read")?
                else {
                    unreachable!("buffer spelling")
                };
                resolver.referenced_types.insert(element);
            }
        }
        let parameters = function
            .params
            .iter()
            .filter(|parameter| !specialized_root || !parameter.ty_name.starts_with("buffer<"))
            .map(|p| {
                resolver.referenced_types.insert(p.ty_name.clone());
                if let Some(Ok(crate::resource_type::ShaderResourceType::Data(
                    crate::resource_type::ResourceType::Buffer(element),
                ))) = crate::resource_type::shader_resource_type(&p.ty_name)
                {
                    resolver.referenced_types.insert(element);
                }
                context_type(&p.ty_name, &context)
                    .map(|ty| format!("{}: {ty}", shader_identifier(&p.name)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let result = function
            .ret_ty
            .as_ref()
            .map(|(ty, _)| {
                resolver.referenced_types.insert(ty.clone());
                context_type(ty, &context).map(|ty| format!(" -> {ty}"))
            })
            .transpose()?
            .unwrap_or_default();
        let symbol = library_symbol(index);
        let name = aliases
            .get(&symbol)
            .ok_or_else(|| format!("missing GPU library symbol `{symbol}`"))?;
        let name = specialization
            .filter(|_| specialized_root)
            .map_or(name.as_str(), |value| value.name.as_str());
        code.push_str(&format!(
            "fn {name}({}){result} {{\n{}\n}}\n",
            parameters.join(", "),
            emit_statements(&body, &context)?
        ));
    }
    Ok((code, resolver.referenced_types))
}

struct Instantiator<'a> {
    derivative_free: bool,
    strict_variables: bool,
    program: &'a SchemaFunction,
    prefix: &'a str,
    captures: Vec<(String, String)>,
    variables: HashMap<String, String>,
    aliases: HashMap<String, String>,
    pending: Vec<(bool, usize)>,
    seen: HashSet<(bool, usize)>,
    referenced_types: BTreeSet<String>,
    value_types: HashMap<String, String>,
    call_types: HashMap<String, String>,
    inference_errors: HashMap<String, String>,
}

impl Instantiator<'_> {
    fn value_type(&self, expression: &SExpr) -> Result<String, String> {
        fn canonical(ty: &str) -> String {
            match crate::check::strip_spatial_type_suffix(ty) {
                "color" => "vec4",
                "float" | "half" => "f32",
                other => other,
            }
            .replace(' ', "")
        }
        let field = |ty: &str, name: &str| -> Result<String, String> {
            if let Some(record) = self.program.structs.iter().find(|s| s.name == ty) {
                return record
                    .fields
                    .iter()
                    .find(|f| f.name == name)
                    .map(|f| canonical(&f.ty_name))
                    .ok_or_else(|| format!("unknown field `{ty}.{name}`"));
            }
            if (ty.starts_with("vec") || ty.starts_with("uvec") || ty.starts_with("ivec"))
                && !name.is_empty()
                && name.len() <= 4
                && name.chars().all(|c| "xyzwrgba".contains(c))
            {
                let (scalar, prefix) = if ty.starts_with("uvec") {
                    ("u32", "uvec")
                } else if ty.starts_with("ivec") {
                    ("i32", "ivec")
                } else {
                    ("f32", "vec")
                };
                return Ok(if name.len() == 1 {
                    scalar.into()
                } else {
                    format!("{prefix}{}", name.len())
                });
            }
            Err(format!("cannot infer field `{ty}.{name}`"))
        };
        match &expression.node {
            Expr::Num(..) => Ok("f32".into()),
            Expr::Color(..) | Expr::Vec4(..) => Ok("vec4".into()),
            Expr::Vec2(..) => Ok("vec2".into()),
            Expr::Vec3(..) => Ok("vec3".into()),
            Expr::Var(name) => {
                if matches!(name.as_str(), "true" | "false") {
                    return Ok("bool".into());
                }
                if let Some(ty) = self.value_types.get(name) {
                    return Ok(canonical(ty));
                }
                if let Some(error) = self.inference_errors.get(name) {
                    return Err(format!(
                        "{error}; annotate `{name}` to resolve this overload"
                    ));
                }
                if let Some((root, member)) = name.rsplit_once('.') {
                    let ty = self.value_type(&SExpr {
                        node: Expr::Var(root.into()),
                        span: expression.span.clone(),
                    })?;
                    return field(&ty, member);
                }
                Err(format!(
                    "cannot infer type of `{name}` for function overload"
                ))
            }
            Expr::Member(base, member) => field(&self.value_type(base)?, member),
            Expr::Unary(_, value) => self.value_type(value),
            Expr::Binary(op, a, b) => {
                if matches!(
                    op,
                    crate::ast::BinOp::Eq
                        | crate::ast::BinOp::Ne
                        | crate::ast::BinOp::Lt
                        | crate::ast::BinOp::Le
                        | crate::ast::BinOp::Gt
                        | crate::ast::BinOp::Ge
                ) {
                    return Ok("bool".into());
                }
                let a = self.value_type(a)?;
                let b = self.value_type(b)?;
                Ok(
                    if a == "f32" || (a.starts_with("mat") && b.starts_with("vec")) {
                        b
                    } else {
                        a
                    },
                )
            }
            Expr::Call { name, args, .. } => {
                if let Some(ty) = self.call_types.get(name) {
                    return Ok(canonical(ty));
                }
                if name == "textureLoad" {
                    let texture = self
                        .value_type(&args.first().ok_or("textureLoad requires an image")?.value)?;
                    let texture = sampled_type(&texture)?;
                    return match texture.as_str() {
                        "texture_2d<f32>" => Ok("vec4".into()),
                        "texture_2d<u32>" => Ok("uvec4".into()),
                        "texture_2d<i32>" => Ok("ivec4".into()),
                        "texture_depth_2d" => Ok("f32".into()),
                        _ => Err("textureLoad requires a typed sampled image".into()),
                    };
                }
                if matches!(name.as_str(), "textureSampleLevel" | "textureSampleGrad") {
                    let texture =
                        self.value_type(&args.first().ok_or("sampling requires an image")?.value)?;
                    return match sampled_type(&texture)?.as_str() {
                        "texture_2d<f32>" => Ok("vec4".into()),
                        _ => {
                            Err("explicit sampling requires a floating-point sampled image".into())
                        }
                    };
                }
                if self.program.structs.iter().any(|s| s.name == *name)
                    || super::gpu_function::wgsl_type(name).is_ok()
                {
                    return Ok(canonical(name));
                }
                if matches!(name.as_str(), "length" | "distance" | "dot" | "determinant") {
                    return Ok("f32".into());
                }
                if matches!(
                    name.as_str(),
                    "select"
                        | "normalize"
                        | "abs"
                        | "min"
                        | "max"
                        | "clamp"
                        | "sin"
                        | "cos"
                        | "tan"
                        | "ceil"
                        | "floor"
                        | "fract"
                        | "sqrt"
                        | "inverse_sqrt"
                        | "inverseSqrt"
                        | "inversesqrt"
                        | "rsqrt"
                        | "exp"
                        | "pow"
                        | "mix"
                        | "cross"
                        | "reflect"
                        | "sign"
                        | "smoothstep"
                        | "step"
                ) {
                    return self
                        .value_type(&args.first().ok_or("function requires an argument")?.value);
                }
                Err(format!(
                    "cannot infer result of `{name}` for function overload"
                ))
            }
            Expr::Array(items) => Ok(format!(
                "array<{},{}>",
                self.value_type(items.first().ok_or("empty array")?)?,
                items.len()
            )),
            Expr::Index { array, .. } => {
                let ty = self.value_type(array)?;
                if ty.starts_with("buffer<") {
                    let crate::resource_type::ResourceType::Buffer(element) =
                        crate::resource_type::resource_type(&ty, "read")?
                    else {
                        unreachable!("buffer spelling")
                    };
                    return Ok(element);
                }
                if let Some(inner) = ty.strip_prefix("array<").and_then(|s| s.strip_suffix('>')) {
                    return Ok(inner
                        .rsplit_once(',')
                        .map_or(inner, |(element, _)| element)
                        .into());
                }
                if ty.starts_with("mat") {
                    return Ok(ty.replacen("mat", "vec", 1));
                }
                field(&ty, "x")
            }
            other => Err(format!("cannot infer ordinary argument type for {other:?}")),
        }
    }

    fn call(
        &mut self,
        name: &str,
        args: &mut Vec<Arg>,
        local: bool,
    ) -> Result<Option<String>, String> {
        let local_candidates: Vec<_> = self
            .program
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.name == name)
            .collect();
        let use_local = local && !local_candidates.is_empty();
        let candidates: Vec<_> = if use_local {
            local_candidates
        } else {
            self.program
                .imports
                .iter()
                .enumerate()
                .filter(|(_, f)| f.name == name && !f.is_builtin && !f.is_internal)
                .collect()
        };
        if candidates.is_empty() {
            if self.derivative_free
                && matches!(
                    name,
                    "ddx"
                        | "ddy"
                        | "fwidth"
                        | "dpdx"
                        | "dpdy"
                        | "dpdxCoarse"
                        | "dpdyCoarse"
                        | "dpdxFine"
                        | "dpdyFine"
                        | "fwidthCoarse"
                        | "fwidthFine"
                        | "textureSample"
                        | "textureSampleBias"
                        | "textureSampleCompare"
                )
            {
                return Err(format!(
                    "dynamically dispatched shading hooks cannot use `{name}`; use explicit LOD or supplied gradients"
                ));
            }
            return Ok(None);
        }
        let mut matching: Vec<_> = candidates
            .into_iter()
            .filter(|(_, f)| {
                args.len() <= f.params.len()
                    && f.params
                        .iter()
                        .skip(args.len())
                        .all(|p| p.default.is_some())
            })
            .collect();
        if matching.len() > 1 {
            let types = args
                .iter()
                .map(|arg| self.value_type(&arg.value))
                .collect::<Result<Vec<_>, _>>()?;
            matching.retain(|(_, function)| {
                args.iter()
                    .zip(&types)
                    .enumerate()
                    .all(|(index, (arg, ty))| {
                        let param = arg
                            .name
                            .as_ref()
                            .and_then(|name| function.params.iter().find(|p| &p.name == name))
                            .or_else(|| function.params.get(index));
                        param.is_some_and(|p| {
                            let expected = match crate::check::strip_spatial_type_suffix(&p.ty_name)
                            {
                                "color" => "vec4",
                                "float" | "half" => "f32",
                                other => other,
                            };
                            if crate::resource_type::shader_resource_type(expected).is_some() {
                                resource_argument_matches(expected, ty).unwrap_or(false)
                            } else {
                                expected.replace(' ', "") == ty.replace(' ', "")
                            }
                        })
                    })
            });
        }
        let [(index, function)] = matching.as_slice() else {
            return Err(format!(
                "schema call `{name}` has no unique ordinary function signature"
            ));
        };
        if !function.type_params.is_empty() || !function.const_params.is_empty() {
            return Err(format!(
                "schema call `{name}` requires concrete type and const arguments"
            ));
        }
        let mut positional = Vec::new();
        let mut named = HashMap::new();
        for arg in std::mem::take(args) {
            if let Some(name) = arg.name.clone() {
                if named.insert(name.clone(), arg).is_some() {
                    return Err(format!("duplicate argument to `{name}`"));
                }
            } else if named.is_empty() {
                positional.push(arg);
            } else {
                return Err("positional argument after a named argument".into());
            }
        }
        let mut positional = positional.into_iter();
        for param in &function.params {
            let arg = positional
                .next()
                .or_else(|| named.remove(&param.name))
                .or_else(|| param.default.clone().map(|value| Arg { name: None, value }))
                .ok_or_else(|| format!("missing argument `{}` to `{name}`", param.name))?;
            if crate::resource_type::shader_resource_type(&param.ty_name).is_some() {
                let actual = self.value_type(&arg.value)?;
                if !resource_argument_matches(&param.ty_name, &actual)? {
                    return Err(format!(
                        "resource argument `{}` requires `{}`, found `{actual}`",
                        param.name, param.ty_name
                    ));
                }
            }
            args.push(arg);
        }
        if positional.next().is_some() || !named.is_empty() {
            return Err(format!("unexpected argument to `{name}`"));
        }
        if let Some((ty, _)) = &function.ret_ty {
            self.call_types.insert(
                format!(
                    "{}_{}_{}",
                    self.prefix,
                    if use_local { "local" } else { "import" },
                    index
                ),
                ty.clone(),
            );
        }
        let key = (use_local, *index);
        if self.seen.insert(key) {
            self.pending.push(key);
        }
        if use_local {
            for (capture, _) in &self.captures {
                args.push(Arg {
                    name: None,
                    value: SExpr {
                        node: Expr::Var(capture.clone()),
                        span: 0..0,
                    },
                });
            }
        }
        Ok(Some(format!(
            "{}_{}_{}",
            self.prefix,
            if use_local { "local" } else { "import" },
            index
        )))
    }

    fn expression(&mut self, expr: &mut SExpr, local: bool) -> Result<(), String> {
        match &mut expr.node {
            Expr::Pipe {
                recv, name, args, ..
            } => {
                self.expression(recv, local)?;
                for arg in args.iter_mut() {
                    self.expression(&mut arg.value, local)?;
                }
                let ty = sampled_type(&self.value_type(recv)?)?;
                let span = expr.span.clone();
                let call = |name: &str, values: Vec<SExpr>| SExpr {
                    span: span.clone(),
                    node: Expr::Call {
                        name: name.into(),
                        name_span: span.clone(),
                        const_args: vec![],
                        args: values
                            .into_iter()
                            .map(|value| Arg { name: None, value })
                            .collect(),
                    },
                };
                if args.iter().any(|a| a.name.is_some()) {
                    return Err("image methods require positional arguments".into());
                }
                if matches!(name.as_str(), "sample_level" | "sample_grad") {
                    let gradients = name == "sample_grad";
                    if ty != "texture_2d<f32>" {
                        return Err(
                            "explicit sampling requires a floating-point sampled image".into()
                        );
                    }
                    let expected: &[&str] = if gradients {
                        &["sampler", "vec2", "vec2", "vec2"]
                    } else {
                        &["sampler", "vec2", "f32"]
                    };
                    if args.len() != expected.len() {
                        return Err(format!(
                            "{name} requires {} positional arguments",
                            expected.len()
                        ));
                    }
                    for (argument, expected) in args.iter().zip(expected) {
                        let actual = self.value_type(&argument.value)?;
                        if actual != *expected {
                            return Err(format!("{name} requires `{expected}`, found `{actual}`"));
                        }
                    }
                    let mut values = vec![*recv.clone()];
                    values.extend(args.iter().map(|argument| argument.value.clone()));
                    expr.node = call(
                        if gradients {
                            "textureSampleGrad"
                        } else {
                            "textureSampleLevel"
                        },
                        values,
                    )
                    .node;
                    self.expression(expr, local)?;
                    return Ok(());
                }
                let store = name == "store" && ty.starts_with("texture_storage_2d<");
                let load = name == "load"
                    && matches!(
                        ty.as_str(),
                        "texture_2d<f32>"
                            | "texture_2d<u32>"
                            | "texture_2d<i32>"
                            | "texture_depth_2d"
                    );
                if !store && !load || args.len() != if store { 2 } else { 1 } {
                    return Err(format!("unsupported resource method `{ty}.{name}`"));
                }
                if !matches!(self.value_type(&args[0].value)?.as_str(), "uvec2" | "ivec2") {
                    return Err("image coordinates require uvec2 or ivec2".into());
                }
                let mut values = vec![*recv.clone(), call("ivec2", vec![args[0].value.clone()])];
                if store {
                    let format = ty
                        .strip_prefix("texture_storage_2d<")
                        .and_then(|ty| ty.split_once(','))
                        .and_then(|(name, _)| fresco_artifact::types::ImageFormat::parse(name))
                        .ok_or("storage image format is not registered")?;
                    let scalar = format.info().scalar.name();
                    let vector = format.info().scalar.vector();
                    let actual = self.value_type(&args[1].value)?;
                    if actual != scalar && actual != vector {
                        return Err(format!(
                            "image store requires `{scalar}` or `{vector}`, found `{actual}`"
                        ));
                    }
                    values.push(call(vector, vec![args[1].value.clone()]));
                } else {
                    values.push(call(
                        "i32",
                        vec![SExpr {
                            node: Expr::Num(0.0, crate::lexer::Unit::None),
                            span: span.clone(),
                        }],
                    ));
                }
                expr.node = call(if store { "textureStore" } else { "textureLoad" }, values).node;
                self.expression(expr, local)?;
            }
            Expr::Call {
                name,
                args,
                const_args,
                ..
            } => {
                if !const_args.is_empty() {
                    return Err("schema calls require concrete function declarations".into());
                }
                for arg in args.iter_mut() {
                    self.expression(&mut arg.value, local)?;
                }
                if let Some(target) = self.call(name, args, local)? {
                    *name = target;
                } else if let Some(record) = self.program.structs.iter().find(|s| s.name == *name) {
                    self.referenced_types.insert(record.name.clone());
                    if args.iter().any(|a| a.name.is_some()) {
                        let mut supplied = BTreeMap::new();
                        for arg in std::mem::take(args) {
                            let key = arg.name.clone().ok_or(
                                "record construction cannot mix named and positional fields",
                            )?;
                            if supplied.insert(key.clone(), arg).is_some() {
                                return Err(format!("duplicate record field `{key}`"));
                            }
                        }
                        for field in &record.fields {
                            args.push(
                                supplied.remove(&field.name).ok_or_else(|| {
                                    format!("missing record field `{}`", field.name)
                                })?,
                            );
                        }
                        if !supplied.is_empty() {
                            return Err("unknown record field".into());
                        }
                    }
                }
            }
            Expr::Unary(_, a) | Expr::Member(a, _) => self.expression(a, local)?,
            Expr::Binary(_, a, b) | Expr::Vec2(a, b) | Expr::Range(a, b) => {
                self.expression(a, local)?;
                self.expression(b, local)?;
            }
            Expr::Vec3(a, b, c) => {
                self.expression(a, local)?;
                self.expression(b, local)?;
                self.expression(c, local)?;
            }
            Expr::Vec4(a, b, c, d) => {
                self.expression(a, local)?;
                self.expression(b, local)?;
                self.expression(c, local)?;
                self.expression(d, local)?;
            }
            Expr::Array(items) => {
                for item in items {
                    self.expression(item, local)?;
                }
            }
            Expr::Index { array, index } => {
                self.expression(array, local)?;
                self.expression(index, local)?;
            }
            // String arguments belong to compile-time selectors such as schema
            // evaluation variants; their owning emitter validates them.
            Expr::Var(name) if self.strict_variables => {
                let root = name.split('.').next().expect("variable root");
                if !matches!(name.as_str(), "true" | "false")
                    && !self.value_types.contains_key(root)
                    && !self.inference_errors.contains_key(root)
                {
                    return Err(format!(
                        "implicit shader resource capture `{name}` is not allowed; pass it explicitly"
                    ));
                }
            }
            Expr::Num(..) | Expr::Color(_) | Expr::Var(_) | Expr::Str(_) => {}
            other => return Err(format!("unsupported ordinary GPU expression: {other:?}")),
        }
        Ok(())
    }

    fn statements(&mut self, body: &mut [Stmt], local: bool) -> Result<(), String> {
        let outer_types = self.value_types.clone();
        let outer_errors = self.inference_errors.clone();
        for stmt in body {
            match stmt {
                Stmt::Let {
                    value,
                    declared_ty_name,
                    name,
                    ..
                } => {
                    if let Some(ty) = declared_ty_name {
                        self.referenced_types.insert(ty.clone());
                    }
                    self.expression(value, local)?;
                    // Naga checks every expression. This prepass needs local types only
                    // to select overloaded source functions; retain an unresolved inference
                    // diagnostic instead of constraining all WGSL builtin expressions.
                    let inferred = match declared_ty_name {
                        Some(ty) => Ok(ty.clone()),
                        None => self.value_type(value),
                    };
                    match inferred {
                        Ok(ty) => {
                            self.value_types.insert(name.clone(), ty);
                            self.inference_errors.remove(name);
                        }
                        Err(error) => {
                            self.value_types.remove(name);
                            self.inference_errors.insert(name.clone(), error);
                        }
                    }
                }
                Stmt::Const {
                    name,
                    value,
                    ty_name,
                    ..
                } => {
                    self.referenced_types.insert(ty_name.clone());
                    self.expression(value, local)?;
                    self.value_types.insert(name.clone(), ty_name.clone());
                }
                Stmt::Assign { value, .. } | Stmt::Return { value, .. } | Stmt::Expr(value) => {
                    self.expression(value, local)?;
                }
                Stmt::Store { target, value, .. } => {
                    self.expression(target, local)?;
                    self.expression(value, local)?;
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    self.expression(cond, local)?;
                    self.statements(then_body, local)?;
                    if let Some(body) = else_body {
                        self.statements(body, local)?;
                    }
                }
                Stmt::For {
                    name,
                    iterable,
                    body,
                    ..
                } => {
                    self.expression(iterable, local)?;
                    let previous = self.value_types.insert(name.clone(), "u32".into());
                    self.statements(body, local)?;
                    if let Some(ty) = previous {
                        self.value_types.insert(name.clone(), ty);
                    } else {
                        self.value_types.remove(name);
                    }
                }
                Stmt::Block { body, .. } => self.statements(body, local)?,
                Stmt::Break { .. } | Stmt::ReturnVoid { .. } => {}
                other => return Err(format!("unsupported ordinary GPU statement: {other:?}")),
            }
        }
        self.value_types = outer_types;
        self.inference_errors = outer_errors;
        Ok(())
    }
}

impl SchemaFunction {
    pub fn instantiate(
        &self,
        channels: &[MaterialChannel],
        context: &EntryContext,
        entry: &str,
        material_type: &str,
    ) -> Result<Instance, String> {
        self.instantiate_with_result(channels, context, entry, material_type, None)
    }

    pub(crate) fn instantiate_with_result(
        &self,
        channels: &[MaterialChannel],
        context: &EntryContext,
        entry: &str,
        material_type: &str,
        expected_result: Option<&str>,
    ) -> Result<Instance, String> {
        let mut signatures = HashSet::new();
        for function in &self.functions {
            let signature = (
                function.name.clone(),
                function
                    .params
                    .iter()
                    .map(|p| p.ty_name.clone())
                    .collect::<Vec<_>>(),
            );
            if !signatures.insert(signature) {
                return Err(format!(
                    "duplicate schema function signature `{}`",
                    function.name
                ));
            }
        }
        let mut aliases = HashMap::new();
        let mut definitions = String::new();
        // Only types reachable from the functions/output are materialized by WGSL;
        // preserve the registered context ABI, namespace other authored records.
        let mut context_names = HashSet::new();
        fn context_types(ty: &crate::context::ContextStruct, names: &mut HashSet<String>) {
            names.insert(ty.name.clone());
            for field in &ty.fields {
                if let crate::context::ContextType::Struct(child) = &field.ty {
                    context_types(child, names);
                }
            }
        }
        context_types(&context.ty, &mut context_names);
        for record in &self.structs {
            aliases.insert(
                record.name.clone(),
                if context_names.contains(&record.name) {
                    record.name.clone()
                } else {
                    format!(
                        "{material_type}_type_{}",
                        self.structs
                            .iter()
                            .position(|r| r.name == record.name)
                            .expect("declared record")
                    )
                },
            );
        }
        let empty = HashMap::new();
        let type_context = ExprContext {
            resource_specializations: None,
            imported_functions: None,
            imported_records: None,
            referenced_calls: None,
            material: None,
            input_var: "",
            varying_source_ty: "",
            varying_ty: "",
            factory_entry: "",
            binding_vars: &empty,
            type_aliases: Some(&aliases),
        };
        let mut used = HashSet::new();
        let mut schema = format!("struct {material_type} {{\n");
        for channel in channels {
            schema.push_str(&format!(
                " {}: {},\n",
                channel.name,
                context_type(&channel.ty_name, &type_context)?
            ));
        }
        schema.push_str("}\n");
        let mut captures = vec![("fresco_schema_context".into(), context.ty.name.clone())];
        captures.extend(
            self.parameters
                .iter()
                .map(|(name, ty)| (format!("fresco_schema_input_{name}"), ty.clone())),
        );
        captures.push(("fresco_schema_value".into(), material_type.into()));
        let mut variables: HashMap<String, String> = channels
            .iter()
            .map(|channel| {
                (
                    channel.name.clone(),
                    format!("fresco_schema_value.{}", channel.name),
                )
            })
            .collect();
        for (name, value) in &self.bindings {
            if let Ok(number) = value.parse::<u32>() {
                variables.insert(name.clone(), format!("{number}.0"));
            }
        }
        for (name, _) in &self.parameters {
            variables.insert(name.clone(), format!("fresco_schema_input_{name}"));
        }
        variables.insert("self".into(), "fresco_schema_value".into());
        variables.insert(context.parameter.clone(), "fresco_schema_context".into());
        let mut scope_types: HashMap<String, String> = self.parameters.iter().cloned().collect();
        for channel in channels {
            scope_types.insert(channel.name.clone(), channel.ty_name.clone());
            scope_types.insert(format!("self.{}", channel.name), channel.ty_name.clone());
        }
        scope_types.insert(context.parameter.clone(), context.ty.name.clone());
        let mut emitter = Instantiator {
            derivative_free: false,
            strict_variables: false,
            program: self,
            prefix: entry,
            captures,
            variables,
            aliases: aliases.clone(),
            pending: Vec::new(),
            seen: HashSet::new(),
            value_types: scope_types.clone(),
            call_types: HashMap::new(),
            inference_errors: HashMap::new(),
            referenced_types: context_names
                .iter()
                .cloned()
                .chain(self.parameters.iter().map(|(_, ty)| ty.clone()))
                .chain(channels.iter().map(|channel| channel.ty_name.clone()))
                .collect(),
        };
        let mut output = self.output.clone();
        emitter.expression(&mut output, true)?;

        let signature = emitter
            .captures
            .iter()
            .map(|(name, ty)| {
                Ok(format!(
                    "{name}: {}",
                    if ty == material_type {
                        ty.clone()
                    } else {
                        context_type(ty, &type_context)?
                    }
                ))
            })
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        let mut functions = String::new();
        let mut executable_functions = None;
        loop {
            if emitter.pending.is_empty() && executable_functions.is_none() {
                executable_functions = Some(functions.clone());
                for index in 0..self.functions.len() {
                    if emitter.seen.insert((true, index)) {
                        emitter.pending.push((true, index));
                    }
                }
            }
            let Some((local, index)) = emitter.pending.pop() else {
                break;
            };
            let function = if local {
                &self.functions[index]
            } else {
                &self.imports[index]
            };
            for p in &function.params {
                emitter.referenced_types.insert(p.ty_name.clone());
            }
            if let Some((ty, _)) = &function.ret_ty {
                emitter.referenced_types.insert(ty.clone());
            }
            let mut body = function
                .typed_body
                .as_ref()
                .unwrap_or(&function.body)
                .clone();
            emitter.inference_errors.clear();
            emitter.value_types = if local {
                scope_types.clone()
            } else {
                HashMap::new()
            };
            for p in &function.params {
                emitter
                    .value_types
                    .insert(p.name.clone(), p.ty_name.clone());
            }
            emitter.statements(&mut body, local)?;
            let mut vars = if local {
                emitter.variables.clone()
            } else {
                HashMap::new()
            };
            for param in &function.params {
                vars.remove(&param.name);
            }
            let vars = vars.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            let cx = ExprContext {
                resource_specializations: None,
                imported_functions: None,
                imported_records: None,
                referenced_calls: None,
                material: None,
                input_var: "",
                varying_source_ty: "",
                varying_ty: "",
                factory_entry: "",
                binding_vars: &vars,
                type_aliases: Some(&emitter.aliases),
            };
            let mut params = function
                .params
                .iter()
                .map(|p| {
                    Ok(format!(
                        "{}: {}",
                        shader_identifier(&p.name),
                        context_type(&p.ty_name, &cx)?
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            if local {
                params.push(signature.clone());
            }
            let result = function.ret_ty.as_ref().ok_or_else(|| {
                format!("schema function `{}` requires a return type", function.name)
            })?;
            let result = context_type(&result.0, &cx)?;
            functions.push_str(&format!(
                "fn {entry}_{}_{index}({}) -> {result} {{\n{}\n}}\n",
                if local { "local" } else { "import" },
                params.join(", "),
                emit_statements(&body, &cx)?
            ));
        }
        let vars = emitter
            .variables
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        let cx = ExprContext {
            resource_specializations: None,
            imported_functions: None,
            imported_records: None,
            referenced_calls: None,
            material: None,
            input_var: "",
            varying_source_ty: "",
            varying_ty: "",
            factory_entry: "",
            binding_vars: &vars,
            type_aliases: Some(&emitter.aliases),
        };
        let output = emit_expr(&output, &cx)?;
        for name in &emitter.referenced_types {
            add_type(
                name,
                self,
                &emitter.aliases,
                &cx,
                &mut used,
                &mut HashSet::new(),
                &mut definitions,
            )?;
        }
        let probe = format!(
            "{definitions}{schema}{functions}fn {entry}({signature}) {{ let fresco_schema_result = {output}; }}\n"
        );
        let module = naga::front::wgsl::parse_str(&probe).map_err(|e| e.emit_to_string(&probe))?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|e| e.emit_to_string(&probe))?;
        let (handle, function) = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some(entry))
            .ok_or("missing schema result probe")?;
        let value = function
            .named_expressions
            .iter()
            .find(|(_, name)| name.as_str() == "fresco_schema_result")
            .map(|(handle, _)| *handle)
            .ok_or("missing schema result value")?;
        let ty = info[handle][value].ty.inner_with(&module.types);
        fn type_text(ty: &naga::TypeInner, module: &naga::Module) -> Result<String, String> {
            use naga::{ScalarKind as K, TypeInner as T};
            let scalar = |s: naga::Scalar| match s.kind {
                K::Float => Ok("f32"),
                K::Sint => Ok("i32"),
                K::Uint => Ok("u32"),
                K::Bool => Ok("bool"),
                _ => Err("abstract result requires a concrete type".to_string()),
            };
            match ty {
                T::Scalar(s) => Ok(scalar(*s)?.into()),
                T::Vector { size, scalar: s } => Ok(format!("vec{}<{}>", *size as u8, scalar(*s)?)),
                T::Matrix {
                    columns,
                    rows,
                    scalar: s,
                } => Ok(format!(
                    "mat{}x{}<{}>",
                    *columns as u8,
                    *rows as u8,
                    scalar(*s)?
                )),
                T::Array {
                    base,
                    size: naga::ArraySize::Constant(size),
                    ..
                } => Ok(format!(
                    "array<{}, {}>",
                    type_text(&module.types[*base].inner, module)?,
                    size
                )),
                T::Struct { .. } => module
                    .types
                    .iter()
                    .find(|(_, t)| &t.inner == ty)
                    .and_then(|(_, t)| t.name.clone())
                    .ok_or("unnamed result record".into()),
                _ => Err("schema output must be an ordinary concrete value type".into()),
            }
        }
        let result_type = type_text(ty, &module)?;
        let functions = executable_functions.expect("completed reachable function emission");
        let return_type = match expected_result {
            Some(expected) => context_type(expected, &cx)?,
            None => result_type.clone(),
        };
        let source =
            format!("{functions}fn {entry}({signature}) -> {return_type} {{ return {output}; }}\n");
        let validation = format!("{definitions}{schema}{source}");
        let module =
            naga::front::wgsl::parse_str(&validation).map_err(|e| e.emit_to_string(&validation))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|e| e.emit_to_string(&validation))?;
        let mut type_definitions = BTreeMap::new();
        for definition in definitions.split_inclusive("}\n") {
            let name = definition
                .strip_prefix("struct ")
                .and_then(|s| s.split_once(" {"))
                .map(|(name, _)| name)
                .ok_or("invalid emitted record definition")?;
            if !context_names.contains(name) {
                type_definitions.insert(name.to_string(), definition.to_string());
            }
        }
        let type_aliases = emitter
            .aliases
            .iter()
            .filter(|(name, _)| used.contains(*name) && !context_names.contains(*name))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let result_type = emitter
            .aliases
            .iter()
            .find(|(_, target)| *target == &result_type)
            .map_or(result_type.clone(), |(name, _)| name.clone());
        Ok(Instance {
            source,
            result_type,
            type_definitions,
            type_aliases,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resource_helpers_compile_through_the_public_pipeline() {
        let source = r#"
fn read_value(values: buffer<vec4, read>, index: u32) -> vec4 { return values[index] }
fn forward(values: buffer<vec4, read>, index: u32) -> vec4 { return read_value(values, index) }
fn lookup(image: texture2d<r32float, read>, filtering: sampler, uv: vec2) -> vec4 {
    return image.sample_level(filtering, uv, 0.0)
}
@shader pass probe {
    stage: raster
    draw: fullscreen
    binding {
        @group(0) @binding(0) image: texture_2d<f32>
        @group(0) @binding(1) filtering: sampler
        @group(0) @binding(2) values: buffer<vec4>
        @group(0) @binding(3) other: buffer<vec4>
    }
    @fragment fn shade(@builtin(position) pixel: vec4) -> vec4 {
        return lookup(image, filtering, pixel.xy) * (forward(values, u32(0)) + forward(other, u32(0)))
    }
}
"#;
        let error =
            compile(&source.replace("forward(values, u32(0))", "forward(vec4(1.0), u32(0))"))
                .expect_err("values cannot substitute for buffer resources");
        assert!(
            format!("{error:?}").contains("resource argument"),
            "{error:?}"
        );
        let source = compile(source).expect("resource helpers and explicit bindings");
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let (index, _) = module
            .entry_points
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.stage == naga::ShaderStage::Fragment)
            .unwrap();
        let read_slots: std::collections::BTreeSet<_> = module
            .global_variables
            .iter()
            .filter_map(|(handle, global)| {
                let binding = global.binding.as_ref()?;
                info.get_entry_point(index)[handle]
                    .contains(naga::valid::GlobalUse::READ)
                    .then_some(binding.binding)
            })
            .collect();
        assert_eq!(read_slots, [0, 1, 2, 3].into());
        let buffer_reads: std::collections::BTreeSet<_> = module
            .functions
            .iter()
            .filter_map(|(_, function)| {
                let slots: std::collections::BTreeSet<_> = function
                    .expressions
                    .iter()
                    .filter_map(|(_, expression)| {
                        let naga::Expression::GlobalVariable(handle) = expression else {
                            return None;
                        };
                        let binding = module.global_variables[*handle].binding.as_ref()?;
                        (binding.binding >= 2).then_some(binding.binding)
                    })
                    .collect();
                (!slots.is_empty()).then_some(slots)
            })
            .collect();
        assert_eq!(
            buffer_reads,
            [
                std::collections::BTreeSet::from([2]),
                std::collections::BTreeSet::from([3])
            ]
            .into()
        );
    }

    #[test]
    fn unused_resource_helpers_are_typechecked_without_implicit_captures() {
        for (body, expected) in [
            ("return missing", "missing"),
            ("return 1.0", "return"),
            (
                "return image.sample_level(filtering, uv, u32(0))",
                "requires `f32`",
            ),
        ] {
            let source = format!(
                "fn unused(image: texture2d<r32float, read>, filtering: sampler, uv: vec2) -> vec4 {{ {body} }}"
            );
            let error = compile(&source).expect_err("unused resource function must be checked");
            assert!(format!("{error:?}").contains(expected), "{error:?}");
        }
        let error = compile("fn unused(values: buffer<vec4, read>) -> vec4 { return fresco_validation_resource_0[0u] }")
            .expect_err("even validation bindings must not become implicit captures");
        assert!(
            format!("{error:?}").contains("implicit shader resource capture"),
            "{error:?}"
        );
    }

    fn sampling_library(ty: &str, expression: &str) -> Result<String, String> {
        use chumsky::Parser;
        let source = format!(
            "fn lookup(image: {ty}, filtering: sampler, uv: vec2, dx: vec2, dy: vec2) -> vec4 {{ return {expression} }}"
        );
        let tokens = crate::lexer::lex_spanned(&source);
        let mut program = crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .expect("typed image function");
        program.functions[0].derivative_free = true;
        let aliases = [(super::library_symbol(0), "lookup".into())].into();
        super::link_library_function(
            0,
            &program.functions,
            &[],
            &aliases,
            &mut Default::default(),
            None,
            &Default::default(),
        )
        .map(|(source, _)| source)
    }

    #[test]
    fn explicit_sampling_preserves_lod_and_gradients_in_naga() {
        for (method, arguments, gradients) in [
            ("sample_level", "filtering, uv, 0.0", false),
            ("sample_grad", "filtering, uv, dx, dy", true),
        ] {
            let source = sampling_library(
                "texture2d<r32float, read>",
                &format!("image.{method}({arguments})"),
            )
            .expect("typed resource method");
            let module = naga::front::wgsl::parse_str(&source).expect("valid shader syntax");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .expect("portable typed resource function");
            let function = &module.functions.iter().next().unwrap().1;
            let levels: Vec<_> = function
                .expressions
                .iter()
                .filter_map(|(_, expression)| {
                    if let naga::Expression::ImageSample { level, .. } = expression {
                        Some(level)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(levels.len(), 1);
            if gradients {
                assert!(matches!(levels[0], naga::SampleLevel::Gradient { .. }));
            } else {
                assert!(matches!(levels[0], naga::SampleLevel::Exact(_)));
            }
        }
    }

    #[test]
    fn explicit_sampling_rejects_invalid_resource_access_and_arguments() {
        for (ty, expression, message) in [
            (
                "texture2d<r32uint, read>",
                "image.sample_level(filtering, uv, 0.0)",
                "floating-point",
            ),
            (
                "texture2d<r32float, write>",
                "image.sample_level(filtering, uv, 0.0)",
                "read",
            ),
            (
                "texture2d<r32float, read>",
                "image.sample_level(uv, uv, 0.0)",
                "sampler",
            ),
            (
                "texture2d<r32float, read>",
                "image.sample_level(filtering, uv, u32(0))",
                "f32",
            ),
            (
                "texture2d<r32float, read>",
                "image.sample_grad(filtering, uv, 0.0, dy)",
                "vec2",
            ),
            (
                "texture2d<r32float, read>",
                "image.sample_level(filtering, uv)",
                "3 positional",
            ),
            (
                "texture2d<r32float, read>",
                "image.sample(filtering, uv)",
                "unsupported resource method",
            ),
        ] {
            let error = sampling_library(ty, expression).expect_err(expression);
            assert!(error.contains(message), "{expression}: {error}");
        }
    }

    #[test]
    fn hook_effect_checks_revisit_previously_emitted_helpers() {
        use chumsky::Parser;
        for intrinsic in [
            "dpdx",
            "dpdy",
            "dpdxCoarse",
            "dpdyCoarse",
            "dpdxFine",
            "dpdyFine",
            "fwidth",
            "fwidthCoarse",
            "fwidthFine",
            "textureSample",
            "textureSampleBias",
            "textureSampleCompare",
        ] {
            let source = format!(
                "fn helper(value: f32) -> f32 {{ return {intrinsic}(value) }}\nfn hook(value: f32) -> f32 {{ return helper(value) }}"
            );
            let tokens = crate::lexer::lex_spanned(&source);
            let mut program = crate::parser::program()
                .parse(crate::parser::input(&tokens, source.len()..source.len()))
                .into_result()
                .unwrap();
            program.functions[1].derivative_free = true;
            let aliases = [
                (super::library_symbol(0), "helper".into()),
                (super::library_symbol(1), "hook".into()),
            ]
            .into();
            // The first use of the helper has no effect restriction. Its presence
            // in the emission cache cannot authorize a later dynamic hook call.
            let mut emitted = Default::default();
            super::link_library_function(
                0,
                &program.functions,
                &[],
                &aliases,
                &mut emitted,
                None,
                &Default::default(),
            )
            .unwrap();
            let error = super::link_library_function(
                1,
                &program.functions,
                &[],
                &aliases,
                &mut emitted,
                None,
                &Default::default(),
            )
            .unwrap_err();
            assert!(
                error.contains("reaches function `helper`") && error.contains(intrinsic),
                "{error}"
            );
        }
    }

    #[test]
    fn hook_effect_checks_resolve_user_functions_before_intrinsics() {
        use chumsky::Parser;
        let source = "fn dpdx(value: f32) -> f32 { return value }\nfn hook(value: f32) -> f32 { return dpdx(value) }";
        let tokens = crate::lexer::lex_spanned(source);
        let program = crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap();
        super::validate_library_effects(1, &program.functions, &[]).unwrap();
    }

    fn compile(source: &str) -> Result<String, Vec<crate::driver::DiagnosticRecord>> {
        crate::test_support::compile_source(source, "main.fr", "wgsl", false).map(|o| o.emitted)
    }
    fn source(program: &str) -> String {
        format!(
            "material_properties data {{ channel power: f32 = 2.0 }}\n{program}\nsurface probe(sp: surf) -> material(data) {{ compose {{ base() }} }}"
        )
    }
    #[test]
    fn preserves_typed_record_matrix_array_and_integer_results() {
        let wgsl = compile(&source(
            r#"
struct Packet { count: u32, signed: i32, enabled: bool, transform: mat3, samples: array<vec2, 2> }
schema_program transfer for data {
    fn transform(packet: Packet) -> Packet { return packet }
    output: transform
}
"#,
        ))
        .expect("typed record program");
        let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
        let (_, entry) = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some("fresco_evaluation_shader_probe"))
            .unwrap();
        let output = entry.result.as_ref().unwrap().ty;
        assert_eq!(
            entry.arguments[1].ty, output,
            "record input and output must retain one nominal type"
        );
        let naga::TypeInner::Struct { members, .. } = &module.types[output].inner else {
            panic!("record output")
        };
        for (index, kind) in [
            (0, naga::ScalarKind::Uint),
            (1, naga::ScalarKind::Sint),
            (2, naga::ScalarKind::Bool),
        ] {
            assert!(
                matches!(module.types[members[index].ty].inner,naga::TypeInner::Scalar(s) if s.kind==kind)
            );
        }
        assert!(matches!(
            module.types[members[3].ty].inner,
            naga::TypeInner::Matrix {
                columns: naga::VectorSize::Tri,
                rows: naga::VectorSize::Tri,
                ..
            }
        ));
        assert!(
            matches!(module.types[members[4].ty].inner,naga::TypeInner::Array {size:naga::ArraySize::Constant(n),..} if n.get()==2)
        );
    }
    #[test]
    fn local_functions_shadow_imports_without_capturing_imported_bodies() {
        let wgsl = compile(&source(
            r#"
fn offset(x: f32) -> f32 { return x + 7.0 }
fn imported(x: f32) -> f32 { return offset(x) }
schema_program transfer for data {
    fn offset(x: f32) -> f32 { return x + 19.0 }
    fn run(x: f32) -> vec2 { return vec2(offset(x), imported(x)) }
    output: run
}
"#,
        ))
        .expect("lexical function scope");
        assert!(
            wgsl.contains("+ 7.0)") && wgsl.contains("+ 19.0)"),
            "{wgsl}"
        );
        assert!(
            wgsl.contains("import_") && wgsl.contains("local_"),
            "{wgsl}"
        );
    }
    #[test]
    fn specialization_preserves_runtime_arguments_and_uses_explicit_predicates() {
        let wgsl = compile(&source(
            r#"
schema_evaluator transfer for data {
    contract { inputs: count: u32, enabled: bool, gain: f32 }
    permutations { mode: low|high }
    specialize as "transfer_${mode}"
    variant misleading_name when mode == low: select(0.0, f32(count) * gain, enabled)
    variant unrelated_name when mode == high: select(0.0, f32(count) * gain * 3.0, enabled)
}
"#,
        ))
        .expect("explicit typed specialization");
        assert!(
            !wgsl.contains("fresco_evaluation_shader_probe("),
            "no invented unspecialized fallback"
        );
        let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
        for name in ["transfer_low", "transfer_high"] {
            let (_, function) = module
                .functions
                .iter()
                .find(|(_, f)| f.name.as_deref() == Some(name))
                .unwrap();
            assert!(
                matches!(module.types[function.arguments[1].ty].inner,naga::TypeInner::Scalar(s) if s.kind==naga::ScalarKind::Uint)
            );
            assert!(
                matches!(module.types[function.arguments[2].ty].inner,naga::TypeInner::Scalar(s) if s.kind==naga::ScalarKind::Bool)
            );
            for index in 1..=3 {
                assert!(function.expressions.iter().any(|(_,expr)|matches!(expr,naga::Expression::FunctionArgument(i) if *i==index)),"{name} must read runtime argument {index}");
            }
        }
    }
    #[test]
    fn unknown_types_and_nonconcrete_array_sizes_fail_closed() {
        for ty in ["UnknownType", "array<f32, missing>", "array<f32, 0>"] {
            let errors=compile(&source(&format!("schema_evaluator transfer for data {{ contract {{ inputs: value: {ty} }}\n shade: power }}"))).expect_err(ty);
            assert!(errors.iter().any(|e| e.severity == "error"), "{errors:?}");
        }
    }
    #[test]
    fn overloads_use_parameter_types_and_duplicate_signatures_are_errors() {
        let program = r#"
schema_program transfer for data {
    fn choose(value: f32) -> vec2 { return vec2(value, self.power) }
    fn choose(value: vec2) -> vec2 { return value }
    fn run(input: vec2) -> vec4 {
        let first=choose(input)
        let second=choose(2.0)
        return vec4(first, second)
    }
    output: run
}
"#;
        let wgsl = compile(&source(program)).expect("typed overloads");
        assert!(
            wgsl.contains("local_0(") && wgsl.contains("local_1("),
            "{wgsl}"
        );
        let duplicate = program.replace(
            "fn choose(value: vec2) -> vec2 { return value }",
            "fn choose(value: f32) -> vec2 { return vec2(value) }",
        );
        let errors = compile(&source(&duplicate)).expect_err("duplicate overload");
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("duplicate schema function signature")),
            "{errors:?}"
        );
    }
    #[test]
    fn inherited_program_reflects_its_typed_runtime_inputs() {
        let source = r#"
material_properties parent { channel power: f32 = 1.0 }
material_properties child extends parent { channel extra: f32 = 2.0 }
schema_program transfer for parent {
    fn run(gain: f32) -> f32 { return self.power * gain }
    output: run
}
surface probe(sp: surf) -> material(child) { compose { base() } }
"#;
        let output = crate::test_support::compile_source(source, "main.fr", "manifest", false)
            .expect("inherited program");
        let manifest: serde_json::Value = serde_json::from_str(&output.emitted).unwrap();
        assert_eq!(
            manifest["surfaces"][0]["evaluation_contract"]["inputs"],
            serde_json::json!(["gain"])
        );
        assert_eq!(
            manifest["surfaces"][0]["evaluation_variants"][0]["result_type"],
            "f32"
        );
    }
    #[test]
    fn predicates_reject_missing_and_ambiguous_selections() {
        for branches in [
            "variant first when mode == low: power",
            "variant first when mode == low: power\nvariant second when mode == low: power",
        ] {
            let errors=compile(&source(&format!("schema_evaluator transfer for data {{ contract {{ inputs: gain: f32 }}\n permutations {{ mode: low|high }}\n specialize as \"transfer_${{mode}}\"\n{branches} }}"))).expect_err("incomplete/ambiguous predicate");
            assert!(
                errors.iter().any(|e| e.message.contains("exactly one")),
                "{errors:?}"
            );
        }
    }
}
