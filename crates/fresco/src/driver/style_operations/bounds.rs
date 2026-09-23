//! Host spatial metadata uses the engine's declared geometry member roles.
use super::*;
use fresco_artifact::{ComputeScalarType, ManifestDrawBounds};

fn geometry(
    program: &Program,
    operation: &StyleOperation,
    value: &SExpr,
) -> Result<String, Vec<Diag>> {
    let name = path(value)
        .ok_or_else(|| fail(&value.span, "expected a prepared geometry bounds member"))?;
    let (parameter, member) = name
        .split_once('.')
        .ok_or_else(|| fail(&value.span, "expected a prepared geometry bounds member"))?;
    let resource = operation
        .inputs
        .iter()
        .find(|input| input.name == parameter)
        .and_then(|input| {
            program
                .resource_types
                .iter()
                .find(|resource| resource.name == input.ty)
        });
    if !resource.is_some_and(|resource| {
        resource
            .attrs
            .iter()
            .any(|a| a.name == "geometry" && a.args.get(4).is_some_and(|role| role == member))
    }) {
        return Err(fail(
            &value.span,
            "bounds must use the engine-declared prepared geometry bounds member",
        ));
    }
    Ok(parameter.into())
}

pub(super) fn metadata(
    program: &Program,
    operation: &StyleOperation,
) -> Result<(Option<ManifestDrawBounds>, Option<String>), Vec<Diag>> {
    let bounds = match &operation.visibility {
        None => None,
        Some(value) if path(value).as_deref() == Some("uncullable") => None,
        Some(value) => {
            let Expr::Pipe {
                recv, name, args, ..
            } = &value.node
            else {
                return Err(fail(
                    &value.span,
                    "visibility requires uncullable or geometry.bounds.expand_world(amount)",
                ));
            };
            if name != "expand_world" || args.len() != 1 || args[0].name.is_some() {
                return Err(fail(
                    &value.span,
                    "bounds expansion requires one positional world-space distance",
                ));
            }
            let parameter = geometry(program, operation, recv)?;
            let amount = &args[0].value;
            if !matches!(
                super::super::compute_operations::host_type(program, operation, amount)?.as_str(),
                "f32" | "number"
            ) {
                return Err(fail(
                    &amount.span,
                    "bounds expansion requires a host f32 distance",
                ));
            }
            Some(ManifestDrawBounds {
                geometry: parameter,
                expansion: super::super::compute_host::lower(
                    program,
                    operation,
                    amount,
                    Some(ComputeScalarType::F32),
                )?,
            })
        }
    };
    let sort_geometry = operation
        .sort_position
        .as_ref()
        .map(|value| {
            let member = path(value)
                .and_then(|name| name.strip_suffix(".center").map(str::to_owned))
                .ok_or_else(|| {
                    fail(&value.span, "sort_position requires geometry.bounds.center")
                })?;
            geometry(
                program,
                operation,
                &Spanned {
                    node: Expr::Var(member),
                    span: value.span.clone(),
                },
            )
        })
        .transpose()?;
    Ok((bounds, sort_geometry))
}
