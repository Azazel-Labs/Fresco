//! Engine-owned, source-backed static surface configuration.
use crate::ast::{
    EntryProperty, Expr, MaterialReturnTy, Program, SExpr, Spanned, Stmt, SurfaceSettings,
};
use crate::diag::Diag;
use std::collections::{HashMap, HashSet};

pub(super) fn resolve(program: &mut Program) -> Result<(), Vec<Diag>> {
    resolve_with_overrides(program, &Default::default(), None)
}

pub(super) fn resolve_with_overrides(
    program: &mut Program,
    overrides: &std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, serde_json::Value>,
    >,
    entry_surfaces: Option<&HashSet<String>>,
) -> Result<(), Vec<Diag>> {
    for name in overrides.keys() {
        if !program.surfaces.iter().any(|surface| &surface.name == name) {
            return Err(vec![Diag::error(
                0..0,
                format!("unknown property override entry `{name}`"),
            )]);
        }
    }
    resolve_properties(program, overrides, entry_surfaces)?;
    super::render_state::resolve(program)
}

fn resolve_properties(
    program: &mut Program,
    overrides: &std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, serde_json::Value>,
    >,
    entry_surfaces: Option<&HashSet<String>>,
) -> Result<(), Vec<Diag>> {
    let schemas: Vec<_> = program
        .templates
        .iter()
        .filter(|schema| schema.property_block.is_some())
        .collect();
    let Some(_) = schemas.first() else {
        if !overrides.is_empty() {
            return Err(vec![Diag::error(
                0..0,
                "property overrides require an engine property contract",
            )]);
        }
        if let Some(contract) = program
            .templates
            .iter()
            .find(|contract| !contract.surface_defaults.is_empty())
        {
            return Err(vec![Diag::error(
                contract.span.clone(),
                "@surface_defaults requires an engine @surface_properties declaration",
            )]);
        }
        if let Some(block) = program
            .surfaces
            .iter()
            .flat_map(|surface| &surface.property_blocks)
            .next()
        {
            return Err(vec![Diag::error(
                block.span.clone(),
                "surface property blocks require an engine @surface_properties declaration",
            )]);
        }
        return Ok(());
    };
    let schemas: Vec<_> = schemas.into_iter().cloned().collect();
    let mut associations = HashSet::new();
    for schema in &schemas {
        let targets = if schema.property_targets.is_empty() {
            vec!["*"]
        } else {
            schema.property_targets.iter().map(String::as_str).collect()
        };
        for target in targets {
            if target != "*" && !program.material_properties.iter().any(|m| m.name == target) {
                return Err(vec![Diag::error(
                    schema.span.clone(),
                    format!("unknown surface property contract `{target}`"),
                )]);
            }
            if !associations.insert(target.to_string()) {
                return Err(vec![Diag::error(
                    schema.span.clone(),
                    format!("conflicting surface property association for `{target}`"),
                )]);
            }
        }
    }
    for contract in &program.templates {
        if contract.surface_defaults.is_empty() {
            continue;
        }
        if let Some(block) = &contract.surface_defaults_block {
            if !schemas
                .iter()
                .any(|s| s.property_block.as_ref() == Some(block))
            {
                return Err(vec![Diag::error(
                    contract.span.clone(),
                    format!("unknown surface defaults block `{block}`"),
                )]);
            }
        } else if schemas.len() != 1 {
            return Err(vec![Diag::error(
                contract.span.clone(),
                "multiple property schemas require @surface_defaults(block, property: value)",
            )]);
        }
    }
    let context = program.clone();
    for index in 0..program.surfaces.len() {
        let surface = &program.surfaces[index];
        let mut actual = match &surface.material_ty {
            MaterialReturnTy::Named(name) => Some(name.as_str()),
            MaterialReturnTy::Default => context
                .material_properties
                .iter()
                .find(|m| m.is_default)
                .map(|m| m.name.as_str()),
        };
        let mut selected = None;
        for _ in 0..=context.material_properties.len() {
            let Some(name) = actual else {
                break;
            };
            selected = schemas
                .iter()
                .find(|schema| schema.property_targets.iter().any(|target| target == name));
            if selected.is_some() {
                break;
            }
            actual = context
                .material_properties
                .iter()
                .find(|m| m.name == name)
                .and_then(|m| m.extends_name.as_deref());
        }
        let selected = selected.or_else(|| {
            schemas
                .iter()
                .find(|schema| schema.property_targets.is_empty())
        });
        if let Some(schema) = selected {
            let mut isolated = context.clone();
            isolated.surfaces = vec![surface.clone()];
            // Entry contracts configure the root source's surfaces. Imported scene
            // materials keep their own schema defaults, even in an emitter bundle.
            if entry_surfaces.is_some_and(|names| !names.contains(&surface.name)) {
                isolated.authored_entries.clear();
            }
            resolve_schema(&mut isolated, schema, overrides)?;
            program.surfaces[index] = isolated.surfaces.remove(0);
        } else if !surface.property_blocks.is_empty() || overrides.contains_key(&surface.name) {
            return Err(vec![Diag::error(
                surface.name_span.clone(),
                "no property schema is associated with this surface contract",
            )]);
        }
    }
    Ok(())
}

fn resolve_schema(
    program: &mut Program,
    schema: &crate::ast::TemplateDecl,
    overrides: &std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, serde_json::Value>,
    >,
) -> Result<(), Vec<Diag>> {
    if schema.entry.is_some() || !schema.plugs.is_empty() {
        return Err(vec![Diag::error(
            schema.span.clone(),
            "surface property schemas contain configuration fields only",
        )]);
    }
    let block_name = schema.property_block.as_ref().expect("selected schema");
    // Defaults belong to active engine entry contracts, not particular entry names.
    // Defaults are explicitly scoped to a property block when multiple schemas exist.
    let active_kinds: HashSet<_> = program
        .authored_entries
        .iter()
        .map(|entry| &entry.kind)
        .collect();
    let mut entry_defaults = HashMap::new();
    for contract in &program.templates {
        if contract.surface_defaults.is_empty()
            || contract
                .surface_defaults_block
                .as_ref()
                .is_some_and(|block| block != block_name)
        {
            continue;
        }
        let Some((kind, _)) = &contract.entry else {
            return Err(vec![Diag::error(
                contract.span.clone(),
                "@surface_defaults requires an @entry contract",
            )]);
        };
        let mut names = HashSet::new();
        for (name, value) in &contract.surface_defaults {
            if !names.insert(&name.node) {
                return Err(vec![Diag::error(
                    name.span.clone(),
                    format!("duplicate surface default `{}`", name.node),
                )]);
            }
            if !schema
                .config_params
                .iter()
                .any(|field| field.name == name.node)
            {
                return Err(vec![Diag::error(
                    name.span.clone(),
                    format!("unknown surface default property `{}`", name.node),
                )]);
            }
            if active_kinds.contains(kind)
                && entry_defaults
                    .insert(name.node.clone(), value.clone())
                    .is_some()
            {
                return Err(vec![Diag::error(
                    name.span.clone(),
                    format!(
                        "ambiguous surface default `{}` from active entry contracts",
                        name.node
                    ),
                )]);
            }
        }
    }
    let context = program.clone();
    for surface in &mut program.surfaces {
        if surface.settings.is_some() {
            continue;
        }
        let fail = |message: String| vec![Diag::error(surface.name_span.clone(), message)];
        if surface.property_blocks.len() > 1 {
            return Err(fail("only one surface property block is allowed".into()));
        }
        let block = surface.property_blocks.first();
        if block.is_some_and(|block| &block.name != block_name) {
            return Err(fail(format!(
                "unknown surface property block; engine declares `{block_name}`"
            )));
        }
        let mut authored = HashMap::new();
        if let Some(block) = block {
            for (name, value) in &block.values {
                if authored.insert(name.node.clone(), value.clone()).is_some() {
                    return Err(fail(format!("duplicate surface property `{}`", name.node)));
                }
            }
        }
        if let Some(values) = overrides.get(&surface.name) {
            for (name, value) in values {
                let mut node = match value {
                    serde_json::Value::Object(value) => {
                        super::styles::selection_override(&context, value, &surface.name_span)
                            .map_err(&fail)?
                    }
                    serde_json::Value::String(value) => Expr::Var(value.clone()),
                    serde_json::Value::Bool(value) => Expr::Var(value.to_string()),
                    serde_json::Value::Number(value) => Expr::Num(
                        value
                            .as_f64()
                            .ok_or_else(|| fail("invalid numeric property override".into()))?,
                        crate::ast::Unit::None,
                    ),
                    _ => return Err(fail(format!("invalid property override `{name}`"))),
                };
                if let Some(field) = schema
                    .config_params
                    .iter()
                    .find(|field| &field.name == name)
                    && matches!(field.ty_name.as_str(), "u32" | "i32")
                    && value.is_number()
                {
                    let number = value
                        .as_f64()
                        .ok_or_else(|| fail("invalid integer override".into()))?;
                    let (minimum, maximum) = if field.ty_name == "u32" {
                        (0.0, f64::from(u32::MAX))
                    } else {
                        (f64::from(i32::MIN), f64::from(i32::MAX))
                    };
                    if number.fract() != 0.0 || !(minimum..=maximum).contains(&number) {
                        return Err(fail(format!(
                            "property override `{name}` is outside {}",
                            field.ty_name
                        )));
                    }
                    node = Expr::Call {
                        name: field.ty_name.clone(),
                        name_span: surface.name_span.clone(),
                        const_args: Vec::new(),
                        args: vec![crate::ast::Arg {
                            name: None,
                            value: Spanned {
                                node,
                                span: surface.name_span.clone(),
                            },
                        }],
                    };
                }
                authored.insert(
                    name.clone(),
                    Spanned {
                        node,
                        span: surface.name_span.clone(),
                    },
                );
            }
        }
        let mut property_u32 = std::collections::BTreeMap::new();
        let mut implementations = Vec::new();
        let mut implementation_slots = std::collections::BTreeSet::new();
        let mut bindings = Vec::new();
        let mut properties = Vec::new();
        let mut usages = Vec::new();
        let mut evaluation_axes = std::collections::BTreeMap::new();
        let mut profile_mapped = false;
        let mut names = HashSet::new();
        for field in &schema.config_params {
            if !names.insert(&field.name) {
                return Err(fail(format!(
                    "duplicate engine surface property `{}`",
                    field.name
                )));
            }
            let engine_owned = field
                .attrs
                .iter()
                .any(|(name, args)| name == "config" && args.iter().any(|arg| arg == "engine"));
            let authored_value = authored.remove(&field.name);
            if engine_owned && authored_value.is_some() {
                return Err(fail(format!(
                    "`{}` is engine-owned; configure it through the engine instead of setting a material property",
                    field.name
                )));
            }
            let value_span = block
                .and_then(|block| {
                    block
                        .values
                        .iter()
                        .find(|(name, _)| name.node == field.name)
                })
                .map(|(_, value)| value.span.clone());
            let value = authored_value.or_else(|| entry_defaults.get(&field.name).cloned()).or_else(|| field.default.clone()).ok_or_else(|| fail(format!("missing required surface property `{}`; provide a value or an explicit engine default",field.name)))?;
            if let Some(contract) = super::implementations::contract_type(&field.ty_name) {
                implementation_slots.insert(field.name.clone());
                let mut annotations = HashSet::new();
                for (attr, args) in &field.attrs {
                    if !annotations.insert(attr)
                        || !matches!(attr.as_str(), "config" | "schema")
                        || args.len() != 1
                    {
                        return Err(fail(format!(
                            "invalid implementation property annotation `@{attr}`"
                        )));
                    }
                }
                if let Some((_, targets)) = field.attrs.iter().find(|(a, _)| a == "schema") {
                    let target = &targets[0];
                    if !context
                        .material_properties
                        .iter()
                        .any(|s| &s.name == target)
                    {
                        return Err(fail(format!(
                            "unknown implementation property schema `{target}`"
                        )));
                    }
                    let mut actual = match &surface.material_ty {
                        MaterialReturnTy::Named(name) => Some(name.as_str()),
                        MaterialReturnTy::Default => context
                            .material_properties
                            .iter()
                            .find(|m| m.is_default)
                            .map(|m| m.name.as_str()),
                    };
                    let mut compatible = false;
                    for _ in 0..=context.material_properties.len() {
                        let Some(name) = actual else {
                            break;
                        };
                        if name == target {
                            compatible = true;
                            break;
                        }
                        actual = context
                            .material_properties
                            .iter()
                            .find(|m| m.name == name)
                            .and_then(|m| m.extends_name.as_deref());
                    }
                    if !compatible {
                        if value_span.is_some()
                            || overrides
                                .get(&surface.name)
                                .is_some_and(|values| values.contains_key(&field.name))
                        {
                            return Err(fail(format!(
                                "implementation property `{}` requires schema `{target}`",
                                field.name
                            )));
                        }
                        continue;
                    }
                }
                let (symbol, args) = match &value.node {
                    Expr::Var(symbol) => (symbol.clone(), Vec::new()),
                    Expr::Call {
                        name,
                        const_args,
                        args,
                        ..
                    } if const_args.is_empty() => (name.clone(), args.clone()),
                    _ => {
                        return Err(fail(format!(
                            "implementation property `{}` requires a declared symbol or named style settings",
                            field.name
                        )));
                    }
                };
                super::styles::check_selection(&context, contract, surface)?;
                let available = super::implementations::catalog(&context, contract)?;
                if !available.contains(&symbol) {
                    return Err(fail(format!(
                        "`{symbol}` is not a registered implementation of `{contract}`"
                    )));
                }
                let (parameters, static_parameters) =
                    super::styles::parameters(&context, &symbol, &args)?;
                implementations.push(fresco_artifact::ManifestImplementationSelection {
                    parameters,
                    static_parameters,
                    settings_offset: 0,
                    name: field.name.clone(),
                    contract: contract.into(),
                    symbol,
                    id: 0,
                    available,
                    availability: Vec::new(),
                    editable: !engine_owned
                        && field
                            .attrs
                            .iter()
                            .any(|(a, args)| a == "config" && args == &["editor"]),
                });
                continue;
            }
            bindings.push(Stmt::Const {
                name: field.name.clone(),
                name_span: field.name_span.clone(),
                ty_name: field.ty_name.clone(),
                ty_span: field.ty_span.clone(),
                value,
            });
            let expression = Spanned {
                node: Expr::Var(field.name.clone()),
                span: surface.name_span.clone(),
            };
            let exact_value =
                crate::check::compute::constant_number(&context, &bindings, &expression)?;
            let value = exact_value as f32;
            let enumeration = context
                .enums
                .iter()
                .find(|enumeration| enumeration.name == field.ty_name);
            if (field.ty_name == "u32" || enumeration.is_some())
                && exact_value.is_finite()
                && exact_value.fract() == 0.0
                && (0.0..=f64::from(u32::MAX)).contains(&exact_value)
            {
                property_u32.insert(field.name.clone(), exact_value as u32);
            }
            let choices = if field.ty_name == "bool" {
                vec![("false".into(), 0.0), ("true".into(), 1.0)]
            } else if let Some(enumeration) = enumeration {
                enumeration
                    .variants
                    .iter()
                    .map(|variant| {
                        let name = format!("{}.{}", enumeration.name, variant.name);
                        let expr = Spanned {
                            node: Expr::Var(name.clone()),
                            span: field.span.clone(),
                        };
                        crate::check::compute::constant_scalar(&context, &bindings, &expr)
                            .map(|value| (name, value))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            if !value.is_finite() {
                return Err(fail(format!("property `{}` must be finite", field.name)));
            }
            if !choices.is_empty() && !choices.iter().any(|(_, choice)| *choice == value) {
                return Err(fail(format!(
                    "invalid enum value for property `{}`",
                    field.name
                )));
            }
            for (attribute, args) in &field.attrs {
                match attribute.as_str() {
                    "evaluation_axis" => {
                        if choices.is_empty() || args.len() != choices.len() + 1 {
                            return Err(fail("@evaluation_axis requires an axis name and one target per boolean or enum choice".into()));
                        }
                        let selected = choices
                            .iter()
                            .position(|(_, choice)| *choice == value)
                            .ok_or_else(|| {
                                fail(format!("invalid evaluation choice for `{}`", field.name))
                            })?;
                        if evaluation_axes
                            .insert(args[0].clone(), args[selected + 1].clone())
                            .is_some()
                        {
                            return Err(fail(format!(
                                "duplicate evaluation axis mapping `{}`",
                                args[0]
                            )));
                        }
                    }
                    "profiles" => {
                        let Some(enumeration) = enumeration else {
                            return Err(fail(format!("@{attribute} requires an enum property")));
                        };
                        if args.len() != enumeration.variants.len() {
                            return Err(fail(format!(
                                "@{attribute} requires one target per enum variant"
                            )));
                        }
                        let selected = choices
                            .iter()
                            .position(|(_, choice)| *choice == value)
                            .ok_or_else(|| {
                                fail(format!("invalid enum value for `{}`", field.name))
                            })?;
                        let target = &args[selected];
                        if profile_mapped {
                            return Err(fail("duplicate @profiles mapping".into()));
                        }
                        profile_mapped = true;
                        surface.material_ty = match target.as_str() {
                            "inherit" => surface.material_ty.clone(),
                            name => MaterialReturnTy::Named(name.into()),
                        };
                    }
                    "range" => {
                        let bounds = args
                            .iter()
                            .map(|arg| arg.parse::<f32>())
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|_| fail("@range requires finite numeric bounds".into()))?;
                        if bounds.len() != 2
                            || !bounds.iter().all(|v| v.is_finite())
                            || bounds[0] > bounds[1]
                        {
                            return Err(fail(
                                "@range requires ordered finite (minimum, maximum) bounds".into(),
                            ));
                        }
                        if !value.is_finite() || !(bounds[0]..=bounds[1]).contains(&value) {
                            return Err(fail(format!(
                                "property `{}` must be in {}..{}",
                                field.name, bounds[0], bounds[1]
                            )));
                        }
                    }
                    "usage" => {
                        if field.ty_name != "bool" || args.len() != 1 {
                            return Err(fail("@usage(factory) requires a boolean property".into()));
                        }
                        if value != 0.0 {
                            if !context
                                .vertex_factories
                                .iter()
                                .any(|factory| factory.name == args[0])
                            {
                                return Err(fail(format!(
                                    "surface usage requests unsupported vertex factory `{}`",
                                    args[0]
                                )));
                            }
                            if usages.contains(&args[0]) {
                                return Err(fail(format!("duplicate surface usage `{}`", args[0])));
                            }
                            usages.push(args[0].clone());
                        }
                    }
                    "config" | "permutation" => {}
                    other => {
                        return Err(fail(format!(
                            "unsupported surface property annotation `@{other}`"
                        )));
                    }
                }
            }
            {
                let editor = field
                    .attrs
                    .iter()
                    .any(|(name, args)| name == "config" && args.iter().any(|a| a == "editor"));
                properties.push(EntryProperty {
                    editable: editor && !engine_owned,
                    block: Some(block_name.clone()),
                    block_present: block.is_some(),
                    entry: surface.name.clone(),
                    name: field.name.clone(),
                    ty: field.ty_name.clone(),
                    value,
                    choices,
                    permutation: true,
                    value_span,
                    insert_at: block.map_or(surface.body_start, |block| block.span.end - 1),
                });
            }
        }
        if let Some(name) = authored.keys().next() {
            return Err(fail(format!("unknown surface property `{name}`")));
        }
        if usages.is_empty()
            && schema
                .config_params
                .iter()
                .any(|field| field.attrs.iter().any(|(name, _)| name == "usage"))
        {
            return Err(fail(
                "surface properties must request at least one supported usage".into(),
            ));
        }
        surface.settings = Some(SurfaceSettings {
            shading_inputs: Default::default(),
            shading_samplers: Default::default(),
            implementation_slots,
            implementations,
            property_u32,
            recipe_conditions: Default::default(),
            pass_states: Default::default(),
            evaluation_axes,
            properties,
            usages,
        });
        bindings.append(&mut surface.body);
        surface.body = bindings;
    }
    Ok(())
}

/// Resolve engine-declared static property names and enum choices in raster hooks.
pub(super) fn static_value(
    material: &crate::material_hir::MaterialHir,
    name: &str,
) -> Option<String> {
    let settings = material.settings.as_ref()?;
    if let Some(selection) = settings.implementations.iter().find(|s| s.name == name) {
        return Some(format!("{}u", selection.id));
    }
    for property in &settings.properties {
        if property.name == name {
            return Some(if property.ty == "u32" {
                format!("{}u", settings.property_u32.get(name)?)
            } else if property.ty == "bool" {
                (property.value != 0.0).to_string()
            } else {
                format!("{:.9}f", property.value)
            });
        }
        if let Some((_, value)) = property
            .choices
            .iter()
            .find(|(choice, _)| property.ty != "bool" && choice == name)
        {
            return Some(format!("{value:.9}f"));
        }
    }
    None
}

/// Fold comparisons of engine static properties before emitting stage code.
pub(super) fn static_condition(
    material: &crate::material_hir::MaterialHir,
    expression: &SExpr,
) -> Option<bool> {
    fn scalar(material: &crate::material_hir::MaterialHir, expression: &SExpr) -> Option<f64> {
        let name = match &expression.node {
            Expr::Num(value, _) => return Some(*value),
            Expr::Var(name) if name == "true" => return Some(1.0),
            Expr::Var(name) if name == "false" => return Some(0.0),
            Expr::Var(name) => name.clone(),
            Expr::Member(base, field) => match &base.node {
                Expr::Var(name) => format!("{name}.{field}"),
                _ => return None,
            },
            _ => return None,
        };
        let value = static_value(material, &name)?;
        match value.as_str() {
            "true" => Some(1.0),
            "false" => Some(0.0),
            _ => value.trim_end_matches(['f', 'u']).parse().ok(),
        }
    }
    if let Expr::Binary(
        op @ (crate::ast::BinOp::LogicalAnd | crate::ast::BinOp::LogicalOr),
        left,
        right,
    ) = &expression.node
    {
        let left = static_condition(material, left)?;
        return if *op == crate::ast::BinOp::LogicalAnd {
            Some(left && static_condition(material, right)?)
        } else {
            Some(left || static_condition(material, right)?)
        };
    }
    if let Expr::Binary(op, left, right) = &expression.node {
        let left = scalar(material, left)?;
        let right = scalar(material, right)?;
        return match op {
            crate::ast::BinOp::Eq => Some(left == right),
            crate::ast::BinOp::Ne => Some(left != right),
            crate::ast::BinOp::Lt => Some(left < right),
            crate::ast::BinOp::Le => Some(left <= right),
            crate::ast::BinOp::Gt => Some(left > right),
            crate::ast::BinOp::Ge => Some(left >= right),
            _ => None,
        };
    }
    scalar(material, expression).map(|value| value != 0.0)
}

#[cfg(test)]
mod tests {
    use chumsky::Parser;
    #[test]
    fn independent_property_contracts_use_the_nearest_schema_and_generic_ranges() {
        let source = r#"
            material_properties cloth { channel albedo: color }
            material_properties velvet extends cloth { channel fuzz: f32 }
            material_properties glass { channel albedo: color }
            @surface_properties(fabric, cloth)
            interface FabricOptions { param @range(0, 4) density: f32 = 2.0 }
            @surface_properties(optics, glass)
            interface GlassOptions { param @range(1, 3) refraction: f32 = 1.5 }
            surface a(sp: surf) -> material(velvet) { fabric { density: 3.0 } }
            surface b(sp: surf) -> material(glass) { optics { refraction: 2.0 } }
        "#;
        let parse = |source: &str| {
            let tokens = crate::lexer::lex_spanned(source);
            crate::parser::program()
                .parse(crate::parser::input(&tokens, source.len()..source.len()))
                .into_result()
                .unwrap()
        };
        let mut program = parse(source);
        super::resolve(&mut program).unwrap();
        let a = &program.surfaces[0].settings.as_ref().unwrap().properties;
        let b = &program.surfaces[1].settings.as_ref().unwrap().properties;
        assert_eq!((a[0].name.as_str(), a[0].value), ("density", 3.0));
        assert_eq!((b[0].name.as_str(), b[0].value), ("refraction", 2.0));
        assert!(!a[0].editable);
        assert!(
            super::resolve(&mut parse(&source.replace("density: 3.0", "density: 5.0"))).is_err()
        );
        assert!(
            super::resolve(&mut parse(
                &source.replace("optics, glass", "optics, cloth")
            ))
            .is_err()
        );
    }

    #[test]
    fn engine_owned_properties_are_reflected_but_reject_authored_overrides() {
        let engine = r#"
            const enabled: bool = true
            @surface_properties(properties)
            interface Options {
                param @config(engine) @permutation tiled: bool = enabled
                param blend: Blend = Blend.Opaque
                param sided: bool = false
                param @range(0, 1) cutoff: f32 = 0.5
            }
            enum Blend: u32 { Opaque }
        "#;
        for (authored, valid) in [("", true), ("properties { tiled: false };", false)] {
            let source = format!(
                "{engine} surface example(sp: surf) -> material(standard) {{ {authored} compose {{ base(albedo: rgba(1.0, 1.0, 1.0, 1.0)) }} }}"
            );
            let tokens = crate::lexer::lex_spanned(&source);
            let mut program = crate::parser::program()
                .parse(crate::parser::input(&tokens, source.len()..source.len()))
                .into_result()
                .unwrap();
            let result = super::resolve(&mut program);
            if valid {
                result.unwrap();
                let property = &program.surfaces[0].settings.as_ref().unwrap().properties[0];
                assert!(!property.editable);
                assert_eq!(property.value, 1.0);
                assert!(property.permutation);
            } else {
                assert!(format!("{:?}", result.unwrap_err()).contains("engine-owned"));
            }
        }
    }
}
