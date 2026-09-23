//! Explicit scalar/vector projections retain their complete typed source binding.
use super::*;

pub(super) struct Projection {
    pub root: String,
    pub root_ty: String,
    pub member: String,
    pub ty: String,
}

pub(super) fn projection(
    program: &Program,
    style: &StyleDecl,
    value: &SExpr,
) -> Result<Option<Projection>, Vec<Diag>> {
    let Some(name) = path(value) else {
        return Ok(None);
    };
    let Some((root, member)) = name.split_once('.') else {
        return Ok(None);
    };
    let root_ty = style
        .params
        .iter()
        .find(|p| p.name == root)
        .map(|p| &p.ty_name)
        .or_else(|| {
            program
                .style_contracts
                .iter()
                .find(|c| c.name == style.contract)
                .and_then(|c| c.inputs.iter().find(|i| i.name == root))
                .map(|i| &i.ty)
        });
    let Some(root_ty) = root_ty else {
        return Ok(None);
    };
    let mut ty = root_ty.clone();
    for field in member.split('.') {
        ty = super::super::mesh_pass::field_type(&program.structs, &ty, field)
            .map_err(|message| fail(&value.span, message))?;
    }
    if !scalar(&ty) {
        return Err(fail(
            &value.span,
            "operation projections must select scalar or vector shader values",
        ));
    }
    Ok(Some(Projection {
        root: root.into(),
        root_ty: root_ty.clone(),
        member: member.into(),
        ty,
    }))
}

/// A uniform's shader value is not available to CPU allocation/precondition code.
/// Keep projected settings under the same explicit shader-only rule initially.
pub(super) fn reject_host(
    value: &SExpr,
    projections: &BTreeMap<String, Projection>,
) -> Result<(), Vec<Diag>> {
    if projections.is_empty() {
        return Ok(());
    }
    if path(value)
        .is_some_and(|name| projections.contains_key(name.split('.').next().expect("path")))
    {
        return Err(fail(
            &value.span,
            "projected shader values cannot determine host allocation, dispatch, bounds, or preconditions",
        ));
    }
    match &value.node {
        Expr::Unary(_, value) | Expr::Member(value, _) => reject_host(value, projections)?,
        Expr::Binary(_, a, b) => {
            reject_host(a, projections)?;
            reject_host(b, projections)?;
        }
        Expr::Call {
            args, const_args, ..
        } => {
            for argument in args
                .iter()
                .map(|a| &a.value)
                .chain(const_args.iter().map(|a| &a.value))
            {
                reject_host(argument, projections)?;
            }
        }
        Expr::Pipe { recv, args, .. } => {
            reject_host(recv, projections)?;
            for argument in args {
                reject_host(&argument.value, projections)?;
            }
        }
        Expr::Num(..) | Expr::Var(..) => {}
        _ => {
            return Err(fail(
                &value.span,
                "unsupported host expression for a projected shader value",
            ));
        }
    }
    Ok(())
}

pub(super) fn arguments(
    program: &Program,
    style: &StyleDecl,
    call: &Call,
) -> Result<BTreeMap<String, Projection>, Vec<Diag>> {
    call.args
        .iter()
        .filter_map(
            |argument| match projection(program, style, &argument.value) {
                Ok(Some(projection)) => Some(Ok((
                    argument.name.clone().expect("named argument"),
                    projection,
                ))),
                Ok(None) => None,
                Err(errors) => Some(Err(errors)),
            },
        )
        .collect()
}

pub(super) fn validate_host(
    operation: &StyleOperation,
    projections: &BTreeMap<String, Projection>,
) -> Result<(), Vec<Diag>> {
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
        reject_host(value, projections)?;
    }
    if let Some(compute) = &operation.compute {
        for value in compute
            .output
            .iter()
            .flat_map(|output| &output.extents)
            .chain(compute.threads.iter().flatten())
        {
            reject_host(value, projections)?;
        }
    }
    Ok(())
}
