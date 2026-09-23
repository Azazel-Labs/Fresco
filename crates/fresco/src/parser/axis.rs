use super::*;
use crate::ast::{AxisDecl, AxisDomain, AxisSubAxisDecl, AxisValueDecl};

// In axis constraints, `axis == a | b` compares against a value set.
// Ordinary expression parsing gives shape union a lower precedence than `==`.
pub(super) fn constraint(mut expr: SExpr) -> SExpr {
    if let Expr::Binary(op, lhs, rhs) = expr.node {
        let lhs = constraint(*lhs);
        let rhs = constraint(*rhs);
        expr.node = match (op, lhs.node) {
            (BinOp::Union, Expr::Binary(cmp @ (BinOp::Eq | BinOp::Ne), axis, value)) => {
                Expr::Binary(cmp, axis, Box::new(binx(BinOp::Union, *value, rhs)))
            }
            (op, node) => Expr::Binary(
                op,
                Box::new(AstSpanned {
                    node,
                    span: lhs.span,
                }),
                Box::new(rhs),
            ),
        };
    }
    expr
}

pub(super) fn declaration<'src>() -> PBox<'src, AxisDecl> {
    let atom = choice((
        ident_sp().map(|(node, span)| AstSpanned { node, span }),
        select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) }.map_with(|n, e| {
            AstSpanned {
                node: n.to_string(),
                span: e.span(),
            }
        }),
    ))
    .boxed();
    let domain = recursive(|domain| {
        let child = ident_sp()
            .then_ignore(just(Token::Colon))
            .then(domain)
            .map_with(|((name, name_span), domain), e| AxisSubAxisDecl {
                name,
                name_span,
                domain,
                span: e.span(),
            });
        atom.clone()
            .then(
                child
                    .separated_by(comma_sep())
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(
                        just(Token::LBrace).then_ignore(nl()),
                        nl().ignore_then(just(Token::RBrace)),
                    )
                    .or_not(),
            )
            .map_with(|(name, children), e| AxisValueDecl {
                name,
                sub_axes: children.unwrap_or_default(),
                span: e.span(),
            })
            .separated_by(nl().ignore_then(just(Token::Bar)).then_ignore(nl()))
            .at_least(1)
            .collect::<Vec<_>>()
            .map(AxisDomain::ValueSet)
    });
    select! { Token::Ident(s) if s == "axis" => () }
        .ignore_then(select! { Token::Attribute(s) if s == "known" => () })
        .ignore_then(
            ident_sp()
                .or(just(Token::Pipeline).map_with(|_, e| ("pipeline".to_string(), e.span())))
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .then(ident_sp())
        .then_ignore(just(Token::Colon))
        .then(domain)
        .map_with(
            |(((known_mode, known_mode_span), (name, name_span)), domain), e| {
                let domain = match domain {
                    AxisDomain::ValueSet(values)
                        if values.len() == 1
                            && values[0].sub_axes.is_empty()
                            && known_mode == "draw"
                            && values[0].name.node.parse::<f64>().is_err() =>
                    {
                        AxisDomain::SymbolicType {
                            name: values[0].name.node.clone(),
                            span: values[0].name.span.clone(),
                        }
                    }
                    other => other,
                };
                AxisDecl {
                    name,
                    name_span,
                    known_mode,
                    known_mode_span,
                    domain,
                    span: e.span(),
                }
            },
        )
        .boxed()
}
