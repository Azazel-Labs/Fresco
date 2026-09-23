//! Engine graph declarations use expressions with source spans, not opaque tokens.
use super::*;
use crate::ast::{
    StyleCapabilityDecl, StyleGraphField, StyleInputDecl, StyleProviderBlock, StyleProviderDecl,
};

pub(super) fn graph<'src>() -> PBox<'src, crate::ast::Spanned<crate::ast::StyleGraphNode>> {
    use crate::ast::StyleGraphNode;
    recursive(|statement| {
        let body = statement
            .separated_by(sep())
            .allow_leading()
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace));
        choice((
            keyword("bind")
                .ignore_then(keyword("shading"))
                .ignore_then(just(Token::Dot))
                .ignore_then(ident())
                .then_ignore(just(Token::Eq))
                .then(expr::expr_parser())
                .map(|(name, value)| StyleGraphNode::BindShading { name, value }),
            just(Token::Let)
                .ignore_then(ident())
                .then_ignore(just(Token::Eq))
                .then(expr::expr_parser())
                .map(|(name, value)| StyleGraphNode::Let { name, value }),
            keyword("requires")
                .ignore_then(expr::expr_parser())
                .map(StyleGraphNode::Require),
            just(Token::For)
                .ignore_then(keyword("self"))
                .ignore_then(body.clone())
                .map(StyleGraphNode::ForSelf),
            just(Token::At)
                .ignore_then(ident())
                .then_ignore(keyword("as"))
                .then(ident())
                .then(body.clone())
                .map(|((point, target), body)| StyleGraphNode::At {
                    point,
                    target,
                    body,
                }),
            keyword("static")
                .ignore_then(just(Token::If))
                .ignore_then(expr::expr_parser())
                .then(body.clone())
                .then(
                    nl().ignore_then(just(Token::Else))
                        .ignore_then(body.clone())
                        .or_not(),
                )
                .map(
                    |((condition, then_body), else_body)| StyleGraphNode::StaticIf {
                        condition,
                        then_body,
                        else_body: else_body.unwrap_or_default(),
                    },
                ),
            keyword("static")
                .ignore_then(just(Token::For))
                .ignore_then(ident())
                .then_ignore(just(Token::In))
                .then(expr::expr_parser())
                .then(body)
                .map(|((variable, range), body)| StyleGraphNode::StaticFor {
                    variable,
                    range,
                    body,
                }),
            expr::expr_parser().try_map(|call, span| {
                if matches!(&call.node, Expr::Call { .. }) {
                    Ok(StyleGraphNode::Call(call))
                } else {
                    Err(Rich::custom(
                        span,
                        "style graph statements require operation calls or static control flow",
                    ))
                }
            }),
        ))
        .map_with(|node, e| crate::ast::Spanned {
            node,
            span: e.span(),
        })
    })
    .boxed()
}

pub(super) fn keyword<'src>(name: &'static str) -> PBox<'src, ()> {
    select! { Token::Ident(s) if s == name => () }.boxed()
}
pub(super) fn member<'src>() -> PBox<'src, StyleInputDecl> {
    ident()
        .separated_by(just(Token::Dot))
        .at_least(1)
        .collect::<Vec<_>>()
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .map_with(|(names, (ty, _)), e| StyleInputDecl {
            name: names.join("."),
            ty,
            span: e.span(),
        })
        .boxed()
}
pub(super) fn fields<'src>() -> PBox<'src, Vec<StyleGraphField>> {
    ident()
        .separated_by(just(Token::Dot))
        .at_least(1)
        .collect::<Vec<_>>()
        .then_ignore(just(Token::Colon))
        .then(
            expr::expr_parser()
                .separated_by(just(Token::Comma))
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map_with(|(name, values), e| StyleGraphField {
            name: name.join("."),
            values,
            span: e.span(),
        })
        .separated_by(sep())
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
        .boxed()
}
pub(super) fn capability<'src>() -> PBox<'src, StyleCapabilityDecl> {
    keyword("capability")
        .ignore_then(ident())
        .then(
            member()
                .separated_by(sep())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|(name, members), e| StyleCapabilityDecl {
            source_file: String::new(),
            name,
            members,
            span: e.span(),
        })
        .boxed()
}
pub(super) fn provider<'src>() -> PBox<'src, StyleProviderDecl> {
    let binding = ident()
        .then_ignore(just(Token::Eq))
        .then(expr::expr_parser())
        .map(|v| (Some(v), None));
    let block = ident().then(fields()).map_with(|(name, fields), e| {
        (
            None,
            Some(StyleProviderBlock {
                name,
                fields,
                span: e.span(),
            }),
        )
    });
    keyword("provide")
        .ignore_then(ident())
        .then_ignore(just(Token::For))
        .then(ident())
        .then(
            choice((binding, block))
                .separated_by(sep())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|((contract, renderer), members), e| StyleProviderDecl {
            source_file: String::new(),
            contract,
            renderer,
            inputs: members.iter().filter_map(|(v, _)| v.clone()).collect(),
            blocks: members.into_iter().filter_map(|(_, v)| v).collect(),
            span: e.span(),
        })
        .boxed()
}
