//! Resolve engine-declared axis domains before checking pass contracts.
use super::*;
use crate::ast::{AxisDecl, AxisDomain, PipelineAttribute};

pub(crate) fn resolve(program: &mut Program) -> Result<(), Vec<Diag>> {
    let mut diags = Vec::new();
    let mut axes = HashMap::new();
    for axis in &program.axes {
        if axes.insert(axis.name.as_str(), axis).is_some() {
            diags.push(Diag::error(
                axis.name_span.clone(),
                format!("duplicate axis `{}`", axis.name),
            ));
        }
        if !matches!(axis.known_mode.as_str(), "compile" | "pipeline" | "draw") {
            diags.push(Diag::error(
                axis.known_mode_span.clone(),
                "axis binding time must be compile, pipeline, or draw",
            ));
        }
        if axis.known_mode == "draw" && !matches!(axis.domain, AxisDomain::SymbolicType { .. }) {
            diags.push(Diag::error(
                axis.name_span.clone(),
                format!(
                    "draw-known axis `{}` must declare a symbolic enum domain",
                    axis.name
                ),
            ));
        }
        if let AxisDomain::SymbolicType { name, span } = &axis.domain
            && !program.enums.iter().any(|decl| decl.name == *name)
        {
            diags.push(Diag::error(
                span.clone(),
                format!("axis `{}` references unknown enum `{name}`", axis.name),
            ));
        }
    }
    for pass in &mut program.passes {
        for permutation in &mut pass.permutations {
            if let Some(axis) = axes.get(permutation.name.as_str()) {
                if let Some(attr) = permutation.attrs.iter().find(|a| a.name == "known") {
                    if attr.args.first() != Some(&axis.known_mode) {
                        diags.push(Diag::error(
                            attr.span.clone(),
                            format!(
                                "axis `{}` binding time conflicts with its declaration",
                                axis.name
                            ),
                        ));
                    }
                } else {
                    permutation.attrs.push(PipelineAttribute {
                        expressions: Vec::new(),
                        name: "known".into(),
                        name_span: axis.known_mode_span.clone(),
                        args: vec![axis.known_mode.clone()],
                        args_span: Some(axis.known_mode_span.clone()),
                        span: axis.span.clone(),
                    });
                }
                let values: Vec<&str> = match &axis.domain {
                    AxisDomain::ValueSet(values) => {
                        values.iter().map(|v| v.name.node.as_str()).collect()
                    }
                    AxisDomain::SymbolicType { name, .. } => program
                        .enums
                        .iter()
                        .find(|e| e.name == *name)
                        .map(|e| e.variants.iter().map(|v| v.name.as_str()).collect())
                        .unwrap_or_default(),
                };
                for value in permutation
                    .values
                    .iter()
                    .chain(permutation.else_value.iter())
                {
                    if !values.contains(&value.node.as_str()) {
                        diags.push(Diag::error(value.span.clone(), format!("axis `{}` selects `{}`, but that value is not declared for the axis", axis.name, value.node)));
                    }
                }
            } else if !program.axes.is_empty()
                && !permutation.attrs.iter().any(|a| a.name == "known")
            {
                diags.push(Diag::error(
                    permutation.name_span.clone(),
                    format!(
                        "pass `{}` references undeclared axis `{}`",
                        pass.name, permutation.name
                    ),
                ));
            }
        }
        // Local declarations remain valid without a global registry.
        let local_axes: Vec<AxisDecl> = pass
            .permutations
            .iter()
            .filter(|p| !axes.contains_key(p.name.as_str()))
            .map(|p| AxisDecl {
                name: p.name.clone(),
                name_span: p.name_span.clone(),
                known_mode: p
                    .attrs
                    .iter()
                    .find(|a| a.name == "known")
                    .and_then(|a| a.args.first())
                    .cloned()
                    .unwrap_or_default(),
                known_mode_span: p.name_span.clone(),
                span: p.span.clone(),
                domain: AxisDomain::ValueSet(
                    p.values
                        .iter()
                        .map(|v| AxisValueDecl {
                            name: v.clone(),
                            sub_axes: vec![],
                            span: v.span.clone(),
                        })
                        .collect(),
                ),
            })
            .collect();
        let mut scope = axes.clone();
        scope.extend(local_axes.iter().map(|a| (a.name.as_str(), a)));
        for permutation in &pass.permutations {
            if let Some(guard) = &permutation.when_guard {
                validate_expr(guard, "permutation guard", &scope, &mut diags);
            }
        }
        for requirement in &pass.requirements {
            if let Some(guard) = requirement.clause.guard_expr() {
                validate_expr(guard, "require guard", &scope, &mut diags);
            }
            validate_expr(
                requirement.clause.constraint_expr(),
                "require constraint",
                &scope,
                &mut diags,
            );
        }
        for binding in &pass.bindings {
            if let Some(signature) = &binding.value_signature {
                for axis in scope.values() {
                    if signature.contains(&format!("[{}]", axis.name))
                        && axis.known_mode != "compile"
                    {
                        diags.push(Diag::error(binding.span.clone(), format!("binding `{}` uses axis `{}` as an array length, but that axis is @known({})", binding.name, axis.name, axis.known_mode)));
                    }
                }
            }
        }
    }
    if diags.is_empty() { Ok(()) } else { Err(diags) }
}

fn axis_ref(
    expr: &SExpr,
    context: &str,
    ordered: bool,
    scope: &HashMap<&str, &AxisDecl>,
    diags: &mut Vec<Diag>,
) {
    let Expr::Var(name) = &expr.node else {
        diags.push(Diag::error(
            expr.span.clone(),
            format!("{context} must be axis-only"),
        ));
        return;
    };
    let Some(axis) = scope.get(name.as_str()) else {
        for parent in scope.values() {
            if let AxisDomain::ValueSet(values) = &parent.domain
                && values
                    .iter()
                    .any(|v| v.sub_axes.iter().any(|child| child.name == *name))
            {
                diags.push(Diag::error(
                    expr.span.clone(),
                    format!(
                        "sub-axis `{name}` is only valid under parent axis `{}`",
                        parent.name
                    ),
                ));
                return;
            }
        }
        let mut diag = Diag::error(
            expr.span.clone(),
            format!("{context} references unknown axis `{name}`"),
        );
        if let Some(candidate) = scope
            .keys()
            .find(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            diag = diag.with_help(format!("did you mean axis `{candidate}`?"));
        }
        diags.push(diag);
        return;
    };
    if axis.known_mode == "draw" {
        diags.push(
            Diag::error(
                expr.span.clone(),
                format!("{context} references draw-known axis `{name}`"),
            )
            .with_related_label(axis.name_span.clone(), "declared here as draw-known"),
        );
    }
    if ordered
        && !matches!(&axis.domain, AxisDomain::ValueSet(values) if values.iter().all(|v| v.name.node.parse::<i64>().is_ok()))
    {
        diags.push(
            Diag::error(
                expr.span.clone(),
                format!("ordered comparison on non-ordered axis `{name}`"),
            )
            .with_related_label(
                axis.name_span.clone(),
                "declared here as symbolic value-set domain",
            ),
        );
    }
}

fn value_set(expr: &SExpr) -> bool {
    match &expr.node {
        Expr::Var(_) | Expr::Num(_, _) | Expr::Str(_) => true,
        Expr::Binary(BinOp::Union, a, b) => value_set(a) && value_set(b),
        Expr::Unary(UnOp::Neg, a) => matches!(a.node, Expr::Num(_, _)),
        _ => false,
    }
}

fn validate_expr(
    expr: &SExpr,
    context: &str,
    scope: &HashMap<&str, &AxisDecl>,
    diags: &mut Vec<Diag>,
) {
    match &expr.node {
        Expr::Binary(BinOp::Eq | BinOp::Ne, lhs, rhs) => {
            axis_ref(lhs, context, false, scope, diags);
            if !value_set(rhs) {
                diags.push(Diag::error(
                    rhs.span.clone(),
                    format!("{context} must be axis-only"),
                ));
            }
        }
        Expr::Binary(BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, lhs, rhs) => {
            axis_ref(lhs, context, true, scope, diags);
            if !matches!(rhs.node, Expr::Num(_, _)) {
                axis_ref(rhs, context, true, scope, diags);
            }
        }
        Expr::Binary(BinOp::LogicalAnd | BinOp::LogicalOr | BinOp::Union, lhs, rhs) => {
            validate_expr(lhs, context, scope, diags);
            validate_expr(rhs, context, scope, diags);
        }
        Expr::Call { name, args, .. } if name == "disable" && !args.is_empty() => {
            for arg in args {
                axis_ref(&arg.value, context, false, scope, diags);
            }
        }
        _ => diags.push(Diag::error(
            expr.span.clone(),
            format!("{context} must be axis-only"),
        )),
    }
}
