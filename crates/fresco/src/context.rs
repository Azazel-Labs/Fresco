//! Typed entry contracts shared by checking, lowering, and stage emission.

use crate::ast::{InterfaceDecl, Span, StructDecl};
use crate::diag::Diag;

#[derive(Debug, Clone)]
pub struct ContextStruct {
    pub name: String,
    pub fields: Vec<ContextField>,
}

#[derive(Debug, Clone)]
pub struct ContextField {
    pub name: String,
    pub ty: ContextType,
    pub semantic: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ContextType {
    Scalar(crate::typed_scalar::Kind),
    Float,
    Vector(u8),
    Struct(ContextStruct),
}

#[derive(Debug, Clone)]
pub struct ContextComponent {
    pub name: String,
    pub path: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct EntryContext {
    pub parameter: String,
    pub interface: String,
    pub method: String,
    pub ty: ContextStruct,
    pub components: Vec<ContextComponent>,
    pub roles: std::collections::HashMap<String, Vec<u32>>,
}

impl EntryContext {
    pub(crate) fn contains_record(&self, name: &str) -> bool {
        fn contains(record: &ContextStruct, name: &str) -> bool {
            record.name == name
                || record.fields.iter().any(|field| match &field.ty {
                    ContextType::Struct(child) => contains(child, name),
                    ContextType::Float | ContextType::Scalar(_) | ContextType::Vector(_) => false,
                })
        }
        contains(&self.ty, name)
    }

    pub fn is_component(&self, name: &str) -> bool {
        self.components
            .iter()
            .any(|component| component.name == name)
    }
}

pub fn raster_contract(
    entry_kind: &str,
    interfaces: &[InterfaceDecl],
    structs: &[StructDecl],
) -> Result<Option<EntryContext>, Vec<Diag>> {
    let registrations = interfaces
        .iter()
        .filter(|interface| {
            interface
                .entry
                .as_ref()
                .is_some_and(|(kind, _)| kind == entry_kind)
        })
        .collect::<Vec<_>>();
    let Some(interface) = registrations.first() else {
        return Ok(None);
    };
    if registrations.len() != 1 {
        return Err(vec![Diag::error(
            interface.name_span.clone(),
            format!("duplicate @entry({entry_kind}, ...) registration"),
        )]);
    }
    let (_, method_name) = interface.entry.as_ref().expect("registered entry");
    let method = interface
        .methods
        .iter()
        .find(|method| method.name == *method_name)
        .ok_or_else(|| {
            vec![Diag::error(
                interface.span.clone(),
                format!(
                    "entry method `{method_name}` is not declared by `{}`",
                    interface.name
                ),
            )]
        })?;
    if method.params.len() != 1
        || !method.params[0].is_context
        || method.ret_ty.as_ref().is_none_or(|(ty, _)| ty != "color")
    {
        return Err(vec![Diag::error(
            method.span.clone(),
            "raster entry method requires one @context struct parameter and must return color",
        )]);
    }
    let ty = resolve_struct(
        &method.params[0].ty_name,
        structs,
        &mut Vec::new(),
        &method.params[0].ty_span,
    )?;
    let mut context = EntryContext {
        parameter: method.params[0].name.clone(),
        interface: interface.name.clone(),
        method: method_name.clone(),
        ty,
        components: Vec::new(),
        roles: std::collections::HashMap::new(),
    };
    collect_fields(
        &context.ty,
        &[],
        &mut context.components,
        &mut context.roles,
    )?;
    Ok(Some(context))
}

pub fn struct_context(
    name: &str,
    parameter: &str,
    structs: &[StructDecl],
    span: &Span,
) -> Result<EntryContext, Vec<Diag>> {
    let ty = resolve_struct(name, structs, &mut Vec::new(), span)?;
    let mut context = EntryContext {
        parameter: parameter.into(),
        interface: name.into(),
        method: "evaluate".into(),
        ty,
        components: Vec::new(),
        roles: std::collections::HashMap::new(),
    };
    collect_fields(
        &context.ty,
        &[],
        &mut context.components,
        &mut context.roles,
    )?;
    Ok(context)
}

fn resolve_struct(
    name: &str,
    structs: &[StructDecl],
    active: &mut Vec<String>,
    span: &Span,
) -> Result<ContextStruct, Vec<Diag>> {
    if active.iter().any(|entry| entry == name) {
        return Err(vec![Diag::error(
            span.clone(),
            "recursive context structs are not supported",
        )]);
    }
    let declaration = structs
        .iter()
        .find(|declaration| declaration.name == name)
        .ok_or_else(|| {
            vec![Diag::error(
                span.clone(),
                format!("unknown context struct `{name}`"),
            )]
        })?;
    active.push(name.to_string());
    let mut fields = Vec::new();
    for field in &declaration.fields {
        let ty = match field.ty_name.as_str() {
            "f32" | "scalar" => ContextType::Float,
            "i32" | "u32" | "bool" => ContextType::Scalar(
                crate::typed_scalar::Kind::parse(&field.ty_name).expect("scalar context type"),
            ),
            "vec2" => ContextType::Vector(2),
            "vec3" => ContextType::Vector(3),
            "vec4" | "color" => ContextType::Vector(4),
            other => ContextType::Struct(resolve_struct(other, structs, active, &field.ty_span)?),
        };
        fields.push(ContextField {
            name: field.name.clone(),
            ty,
            semantic: field.semantic.clone(),
            span: field.span.clone(),
        });
    }
    active.pop();
    Ok(ContextStruct {
        name: name.to_string(),
        fields,
    })
}

fn collect_fields(
    ty: &ContextStruct,
    prefix: &[u32],
    components: &mut Vec<ContextComponent>,
    roles: &mut std::collections::HashMap<String, Vec<u32>>,
) -> Result<(), Vec<Diag>> {
    for (index, field) in ty.fields.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(u32::try_from(index).expect("context field index fits u32"));
        if let Some(role) = &field.semantic {
            let valid = match role.as_str() {
                "coord" | "resolution" => matches!(field.ty, ContextType::Vector(2)),
                "time" | "delta_time" => matches!(field.ty, ContextType::Float),
                _ => false,
            };
            if !valid {
                return Err(vec![Diag::error(
                    field.span.clone(),
                    format!("unknown or incorrectly typed context semantic `{role}`"),
                )]);
            }
            if roles.insert(role.clone(), path.clone()).is_some() {
                return Err(vec![Diag::error(
                    field.span.clone(),
                    format!("duplicate context semantic `{role}`"),
                )]);
            }
        }
        match &field.ty {
            ContextType::Struct(nested) => collect_fields(nested, &path, components, roles)?,
            ContextType::Float | ContextType::Scalar(_) => components.push(ContextComponent {
                name: format!("fresco_context_input_{}", components.len()),
                path,
            }),
            ContextType::Vector(width) => {
                for component in 0..*width {
                    let mut path = path.clone();
                    path.push(u32::from(component));
                    components.push(ContextComponent {
                        name: format!("fresco_context_input_{}", components.len()),
                        path,
                    });
                }
            }
        }
    }
    Ok(())
}
