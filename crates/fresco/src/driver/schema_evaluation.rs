//! Bridge engine raster hooks to checked material evaluation helpers.
use crate::ast::{Arg, Expr};
use crate::material_hir::MaterialHir;

pub(super) fn emit(
    material: &MaterialHir,
    args: &[Arg],
    emit_expr: impl Fn(&crate::ast::SExpr) -> Result<String, String>,
) -> Result<String, String> {
    // Engine context and evaluated material; renderer inputs follow by name.
    if args.len() < 2 || args[..2].iter().any(|arg| arg.name.is_some()) {
        return Err("evaluate_schema requires an engine context and material; evaluation inputs follow by name".into());
    }
    let mut supplied = std::collections::HashMap::new();
    for arg in &args[2..] {
        let name = arg
            .name
            .as_deref()
            .ok_or("evaluation contract inputs must be named")?;
        if supplied.insert(name, &arg.value).is_some() {
            return Err(format!("duplicate evaluation input `{name}`"));
        }
    }
    let mat = emit_expr(&args[1].value)?;
    let variant = select(material, supplied.remove("variant"))?;
    let mut actual = args[..1]
        .iter()
        .map(|arg| emit_expr(&arg.value))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(function) = &variant.function {
        for (name, _) in &function.parameters {
            let value=supplied.remove(name.as_str()).ok_or_else(||format!("missing evaluation input `{name}` for surface `{}`; the engine draw hook must supply its renderer resources", material.name))?;
            actual.push(emit_expr(value)?);
        }
    }
    if let Some(name) = supplied.keys().next() {
        return Err(format!(
            "unknown evaluation input `{name}` for surface `{}`",
            material.name
        ));
    }
    actual.push(mat);
    Ok(format!("{}({})", variant.entry, actual.join(", ")))
}

fn select<'a>(
    material: &'a MaterialHir,
    explicit_variant: Option<&crate::ast::SExpr>,
) -> Result<&'a crate::material_hir::EvaluationVariant, String> {
    if material.evaluation_variant_bodies.is_empty() {
        return Err(format!(
            "surface `{}` has no executable response; declare a schema_program in the engine",
            material.name
        ));
    }
    let requested = match explicit_variant {
        Some(value) => match &value.node {
            Expr::Str(name) => Some(name.as_str()),
            _ => return Err("evaluation variant must be a literal entry name".into()),
        },
        None => material.evaluation_shader_entry.as_deref(),
    };
    let variants = &material.evaluation_variant_bodies;
    let axes = material
        .settings
        .as_ref()
        .map(|settings| &settings.evaluation_axes);
    let variant = if let Some(axes) = axes.filter(|axes| !axes.is_empty()) {
        if explicit_variant.is_some() {
            return Err("evaluation variant must be selected either by surface properties or by the draw hook, not both".into());
        }
        let matches: Vec<_> = variants
            .iter()
            .filter(|variant| {
                axes.iter().all(|(axis, value)| {
                    variant
                        .bindings
                        .iter()
                        .any(|binding| &binding.axis == axis && &binding.value == value)
                })
            })
            .collect();
        match matches.as_slice() {
            [variant] => *variant,
            [] => {
                return Err(format!(
                    "unsupported evaluation permutation {axes:?} for surface `{}`",
                    material.name
                ));
            }
            _ => {
                return Err(format!(
                    "ambiguous evaluation permutation {axes:?}; the engine must specify the remaining axes"
                ));
            }
        }
    } else {
        let requested = requested
            .ok_or("specialized schema evaluation requires explicit axes or a variant entry")?;
        variants
            .iter()
            .find(|variant| variant.entry == requested)
            .ok_or_else(|| {
                format!(
                    "unknown evaluation variant `{requested}` for surface `{}`",
                    material.name
                )
            })?
    };
    Ok(variant)
}

pub(super) fn result_type(material: &MaterialHir, args: &[Arg]) -> Result<String, String> {
    let explicit = args
        .iter()
        .find(|a| a.name.as_deref() == Some("variant"))
        .map(|a| &a.value);
    select(material, explicit)?
        .result_type
        .clone()
        .ok_or("schema variant has no checked result type".into())
}
