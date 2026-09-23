use super::*;
use crate::ast::{
    Canvas, CanvasParam, ConstDecl, EvaluationContractBindingDecl, MaterialReturnTy,
    PassFnHookDecl, PassFnHookParamDecl, SchemaEvaluatorVariantDecl, SchemaProgramDecl,
    StyleContractDecl, StyleDecl, StyleHookDecl, SurfaceDecl, TemplateConfigParamDecl,
    TemplateDecl, TemplatePlugDecl, VertexFactoryDecl, VertexFormatDecl, VertexInterfaceDecl,
    VertexInterfaceMemberDecl,
};
use crate::pipeline_layout::canonical_layout_signature_from_tokens;

pub(super) fn program_parser<'src>() -> PBox<'src, Program> {
    let nl = nl();
    let comma_sep = comma_sep();
    let sep = sep();
    let ident = ident();
    let ident_sp = ident_sp();
    let type_ident_sp = type_ident_sp();
    let block = stmt::block_parser();

    // Opaque balanced-brace parser used by staged declarations where syntax is
    // accepted before semantic/lowering support lands.
    let opaque_braced_block = recursive(|nested| {
        choice((
            select! { tok if !matches!(tok, Token::LBrace | Token::RBrace) => () }
                .ignored()
                .boxed(),
            just(Token::LBrace)
                .ignore_then(nested.repeated())
                .then_ignore(just(Token::RBrace))
                .ignored()
                .boxed(),
        ))
    })
    .repeated()
    .ignored()
    .delimited_by(
        just(Token::LBrace).then_ignore(nl.clone()),
        nl.clone().ignore_then(just(Token::RBrace)),
    )
    .boxed();

    let opaque_paren_block = recursive(|nested| {
        choice((
            select! { tok if !matches!(tok, Token::LParen | Token::RParen) => () }
                .ignored()
                .boxed(),
            just(Token::LParen)
                .ignore_then(nested.repeated())
                .then_ignore(just(Token::RParen))
                .ignored()
                .boxed(),
        ))
    })
    .repeated()
    .ignored()
    .delimited_by(just(Token::LParen), just(Token::RParen))
    .map_with(|_, e| e.span())
    .boxed();

    let pipeline_attr_arg = choice((
        select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits).to_string() }.boxed(),
        select! { Token::Str(s) => s }.boxed(),
        ident_sp.clone().map(|(name, _)| name).boxed(),
        just(Token::Pass).to("pass".to_string()).boxed(),
        just(Token::Vertex).to("vertex".to_string()).boxed(),
        just(Token::Surface).to("surface".to_string()).boxed(),
        just(Token::Pipeline).to("pipeline".to_string()).boxed(),
    ))
    .boxed();

    let pipeline_attr_args = pipeline_attr_arg
        .separated_by(just(Token::Comma).padded_by(nl.clone()))
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .map_with(|args, e| (Some(e.span()), args))
        .boxed();

    let pipeline_attr = select! { Token::Attribute(name) => name.to_string(), Token::Builtin => "builtin".to_string() }
        .map_with(|name, e| (name, e.span()))
        .then(
            choice((
                pipeline_attr_args,
                opaque_paren_block
                    .clone()
                    .map(|span| (Some(span), Vec::<String>::new()))
                    .boxed(),
            ))
            .or_not(),
        )
        .map_with(
            |((name, name_span), args_data), e| crate::ast::PipelineAttribute {
                expressions: Vec::new(),
                name,
                name_span,
                args: args_data
                    .as_ref()
                    .map(|(_, args)| args.clone())
                    .unwrap_or_default(),
                args_span: args_data.and_then(|(span, _)| span),
                span: e.span(),
            },
        )
        .boxed();

    let resource_attribute = select! { Token::Attribute(name) if matches!(name.as_str(),
    "meta" | "when" | "configure" | "require") => name.to_string() }
    .map_with(|name, e| (name, e.span()))
    .then(
        expr::expr_parser()
            .separated_by(comma_sep.clone())
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen)),
    )
    .map_with(
        |((name, name_span), expressions), e| crate::ast::PipelineAttribute {
            name,
            name_span,
            expressions,
            args: Vec::new(),
            args_span: None,
            span: e.span(),
        },
    )
    .boxed();
    let typed_input = select! { Token::Attribute(name) if name == "input" => name }
        .then(
            ident
                .clone()
                .then_ignore(just(Token::Comma))
                .then(fn_param_type_sp())
                .delimited_by(just(Token::LParen), just(Token::RParen)),
        )
        .map_with(
            |(name, (input, (ty, _))), e| crate::ast::PipelineAttribute {
                name: name.to_string(),
                args: vec![input, ty],
                expressions: vec![],
                name_span: e.span(),
                args_span: None,
                span: e.span(),
            },
        );
    let pipeline_attr = typed_input.or(resource_attribute).or(pipeline_attr).boxed();
    let pipeline_attrs = pipeline_attr
        .then_ignore(nl.clone())
        .repeated()
        .collect::<Vec<_>>()
        .boxed();

    let import = just(Token::Import)
        .ignore_then(select! { Token::Str(s) => s })
        .map_with(|path, e| ImportDecl {
            path,
            span: e.span(),
        })
        .boxed();

    let pragma_key = ident_sp
        .clone()
        .then(
            just(Token::Dot)
                .ignore_then(ident_sp.clone())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|((head, head_span), tail)| {
            let mut key = head;
            let mut key_span = head_span;
            for (seg, seg_span) in tail {
                key.push('.');
                key.push_str(&seg);
                key_span.end = seg_span.end;
            }
            (key, key_span)
        })
        .boxed();

    let pragma_value = select! {
        Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => PragmaValue::Number(f64::from_bits(bits)),
        Token::Ident(v) => PragmaValue::Ident(v.to_string()),
    }
    .map_with(|v, e| (v, e.span()))
    .boxed();

    let pragma = just(Token::Pragma)
        .ignore_then(pragma_key)
        .then_ignore(just(Token::Eq))
        .then(pragma_value)
        .map_with(|((key, key_span), (value, value_span)), e| PragmaDecl {
            key,
            key_span,
            value,
            value_span,
            span: e.span(),
        })
        .boxed();

    let canvas_param = ident_sp
        .clone()
        .then_ignore(just(Token::Colon))
        .then(type_ident_sp.clone())
        .map(|((name, name_span), (ty_name, ty_span))| CanvasParam {
            name,
            name_span,
            ty_name,
            ty_span,
        })
        .boxed();
    let canvas_params = canvas_param
        .separated_by(comma_sep.clone())
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LParen).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RParen)),
        )
        .boxed();

    let entry_template_parts = ident_sp
        .clone()
        .then(
            canvas_params
                .clone()
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .boxed();

    let template_attr = select! { Token::Attribute(name) => name.to_string() }
        .then(
            just(Token::LParen)
                .ignore_then(
                    choice((
                        ident.clone(),
                        just(Token::Minus)
                            .or_not()
                            .then(
                                select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) },
                            )
                            .map(|(negative, value)| {
                                if negative.is_some() {
                                    format!("-{value}")
                                } else {
                                    value.to_string()
                                }
                            }),
                    ))
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>(),
                )
                .then_ignore(just(Token::RParen))
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .boxed();

    let template_attrs = template_attr.repeated().collect::<Vec<_>>().boxed();

    let interface_config_param = just(Token::Param)
        .ignore_then(template_attrs.clone())
        .then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .then(
            just(Token::Eq)
                .ignore_then(expr::expr_parser_with_casts(true))
                .or_not(),
        )
        .map_with(
            |(((attrs, (name, name_span)), (ty_name, ty_span)), default), e| {
                TemplateConfigParamDecl {
                    attrs,
                    name,
                    name_span,
                    ty_name,
                    ty_span,
                    default,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let entry_kind = choice((just(Token::Canvas).to("canvas".to_string()), ident.clone()));
    let entry_block = ident_sp
        .clone()
        .then(nl.clone().ignore_then(block.clone()))
        .map_with(
            |((name, name_span), body), e| crate::ast::AuthoredEntryBlock {
                params: None,
                return_ty: None,
                name,
                name_span,
                body,
                span: e.span(),
            },
        );
    let entry_setting = ident_sp
        .clone()
        .then_ignore(just(Token::Colon))
        .then(expr::expr_parser_with_casts(true))
        .map(|((node, span), value)| (crate::ast::Spanned { node, span }, value));
    let named_blocks = choice((
        entry_block.map(|block| (Some(block), None, None)),
        entry_setting
            .clone()
            .map(|setting| (None, Some(setting), None)),
        stmt::stmt_parser().map(|statement| (None, None, Some(statement))),
    ))
    .separated_by(sep.clone())
    .allow_leading()
    .allow_trailing()
    .at_least(1)
    .collect::<Vec<_>>()
    .delimited_by(just(Token::LBrace), just(Token::RBrace));
    let authored_entry = entry_kind
        .then(entry_template_parts.clone())
        .then(
            just(Token::Arrow)
                .ignore_then(type_ident_sp.clone())
                .map(|(node, span)| crate::ast::Spanned { node, span })
                .or_not(),
        )
        .then(nl.clone().ignore_then(choice((
            named_blocks.map(|items| {
                let mut blocks = Vec::new();
                let mut config = Vec::new();
                let mut body = Vec::new();
                for (block, setting, statement) in items {
                    if let Some(statement) = statement {
                        body.push(statement);
                    }
                    if let Some(block) = block {
                        blocks.push(block);
                    }
                    if let Some(setting) = setting {
                        config.push(setting);
                    }
                }
                (body, blocks, config)
            }),
            block.clone().map(|body| (body, Vec::new(), Vec::new())),
        ))))
        .map_with(
            |(((kind, ((name, name_span), params)), return_ty), (body, blocks, config)), e| {
                crate::ast::AuthoredEntry {
                    config,
                    kind,
                    name,
                    name_span,
                    params,
                    return_ty,
                    body,
                    blocks,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let context_param = select! { Token::Attribute(name) if name == "context" => () }
        .or_not()
        .map(|value| value.is_some());
    let fn_param = context_param
        .then(
            select! { Token::Ident(s) if s == "static" => () }
                .or_not()
                .ignore_then(ident_sp.clone())
                .then_ignore(just(Token::Colon))
                .then(fn_param_type_sp())
                .then(just(Token::Eq).ignore_then(expr::expr_parser()).or_not()),
        )
        .map(
            |(is_context, (((name, name_span), (ty_name, ty_span)), default))| FnParam {
                is_context,
                name,
                name_span,
                ty_name,
                ty_span,
                keyword_only: false,
                default,
            },
        )
        .boxed();

    let glsl_param = fn_param_type_sp()
        .clone()
        .then(ident_sp.clone())
        .map(|((ty_name, ty_span), (name, name_span))| FnParam {
            is_context: false,
            name,
            name_span,
            ty_name,
            ty_span,
            keyword_only: false,
            default: None,
        })
        .boxed();

    let glsl_params = glsl_param
        .separated_by(comma_sep.clone())
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LParen).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RParen)),
        )
        .boxed();

    enum FnParamItem {
        Marker(Span),
        Param(FnParam),
    }

    let fn_param_item = choice((
        just(Token::Star).map_with(|_, e| FnParamItem::Marker(e.span())),
        fn_param.clone().map(FnParamItem::Param),
    ))
    .boxed();

    let fn_params = fn_param_item
        .clone()
        .then(
            comma_sep
                .clone()
                .ignore_then(fn_param_item.clone())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(head, tail)| {
            let mut items = Vec::with_capacity(1 + tail.len());
            items.push(head);
            items.extend(tail);
            items
        })
        .or_not()
        .map(Option::unwrap_or_default)
        .then_ignore(comma_sep.clone().or_not())
        .delimited_by(
            just(Token::LParen).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RParen)),
        )
        .try_map(|items, _span| {
            let mut params = Vec::new();
            let mut saw_keyword_separator = false;
            let mut saw_keyword_only_param = false;
            let mut separator_span: Option<Span> = None;

            for item in items {
                match item {
                    FnParamItem::Marker(star_span) => {
                        if saw_keyword_separator {
                            return Err(Rich::custom(
                                star_span,
                                "duplicate `*` keyword-only separator in parameter list",
                            ));
                        }
                        saw_keyword_separator = true;
                        separator_span = Some(star_span);
                    }
                    FnParamItem::Param(mut param) => {
                        if saw_keyword_separator {
                            param.keyword_only = true;
                            saw_keyword_only_param = true;
                        }
                        params.push(param);
                    }
                }
            }

            if saw_keyword_separator && !saw_keyword_only_param {
                return Err(Rich::custom(
                    separator_span.unwrap_or(0..0),
                    "`*` must be followed by at least one keyword-only parameter",
                ));
            }

            Ok(params)
        })
        .boxed();

    let type_param = ident_sp
        .clone()
        .then(
            just(Token::Colon)
                .ignore_then(
                    type_ident_sp
                        .clone()
                        .map(|(name, _)| name)
                        .separated_by(just(Token::Plus))
                        .at_least(1)
                        .collect::<Vec<_>>(),
                )
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .map_with(|((name, name_span), bounds), e| TypeParam {
            name,
            name_span,
            bounds,
            span: e.span(),
        })
        .boxed();

    enum TemplateParamItem {
        Type(TypeParam),
        Const(crate::ast::ConstTemplateParam),
    }

    let const_param_kind = type_ident_sp
        .clone()
        .try_map(|(name, span), _| match name.as_str() {
            "u32" => Ok((crate::ast::ConstTemplateKind::U32, span)),
            "i32" => Ok((crate::ast::ConstTemplateKind::I32, span)),
            _ => Err(Rich::custom(
                span,
                "const template parameter type must be `u32` or `i32`",
            )),
        })
        .boxed();

    let const_param = select! { Token::Ident(s) if s == "const" => () }
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(const_param_kind)
        .map_with(
            |((name, name_span), (kind, kind_span)), e| crate::ast::ConstTemplateParam {
                name,
                name_span,
                kind,
                kind_span,
                span: e.span(),
            },
        )
        .boxed();

    let template_params = choice((
        const_param.map(TemplateParamItem::Const),
        type_param.clone().map(TemplateParamItem::Type),
    ))
    .separated_by(comma_sep.clone())
    .allow_trailing()
    .collect::<Vec<_>>()
    .delimited_by(just(Token::Lt), just(Token::Gt))
    .or_not()
    .try_map(|opt, _| {
        let mut type_params = Vec::new();
        let mut const_params = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for item in opt.unwrap_or_default() {
            match item {
                TemplateParamItem::Type(param) => {
                    if !seen.insert(param.name.clone()) {
                        return Err(Rich::custom(
                            param.name_span,
                            "duplicate template parameter name",
                        ));
                    }
                    type_params.push(param);
                }
                TemplateParamItem::Const(param) => {
                    if !seen.insert(param.name.clone()) {
                        return Err(Rich::custom(
                            param.name_span,
                            "duplicate template parameter name",
                        ));
                    }
                    const_params.push(param);
                }
            }
        }

        Ok((type_params, const_params))
    })
    .boxed();

    let type_params = type_param
        .separated_by(comma_sep.clone())
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Lt), just(Token::Gt))
        .or_not()
        .map(Option::unwrap_or_default)
        .boxed();

    let ret_ty = just(Token::Arrow)
        .ignore_then(fn_param_type_sp())
        .or_not()
        .boxed();

    #[derive(Debug, Clone)]
    enum InterfaceBodyItem {
        Config(TemplateConfigParamDecl),
        Plug(TemplatePlugDecl),
    }

    let composition = select! { Token::Attribute(name) if name == "compose" => () }
        .ignore_then(just(Token::LParen))
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::RParen))
        .map(|(node, span)| crate::ast::Spanned { node, span })
        .or_not();
    let binding = select! { Token::Attribute(name) if name == "bind" => () }
        .ignore_then(just(Token::LParen))
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::RParen))
        .map(|(node, span)| crate::ast::Spanned { node, span })
        .or_not();
    let interface_plug = binding
        .then_ignore(nl.clone())
        .then(composition)
        .then_ignore(nl.clone())
        .then_ignore(just(Token::Fn))
        .then(ident_sp.clone())
        .then(fn_params.clone())
        .then(ret_ty.clone())
        .map_with(
            |((((binding_name, compose_param), (name, name_span)), params), return_ty), e| {
                TemplatePlugDecl {
                    binding_name,
                    compose_param,
                    name,
                    name_span,
                    params,
                    return_ty: return_ty.map(|(ty_name, ty_span)| crate::ast::Spanned {
                        node: ty_name,
                        span: ty_span,
                    }),
                    span: e.span(),
                }
            },
        )
        .boxed();

    let interface_body_item = choice((
        interface_config_param
            .clone()
            .map(InterfaceBodyItem::Config)
            .boxed(),
        interface_plug.clone().map(InterfaceBodyItem::Plug).boxed(),
    ))
    .boxed();

    let interface_body = interface_body_item
        .separated_by(nl.clone())
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LBrace).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

    let entry_registration = select! { Token::Attribute(name) if name == "entry" => () }
        .ignore_then(just(Token::LParen))
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::Comma))
        .then(ident_sp.clone())
        .then_ignore(just(Token::RParen))
        .map(|((kind, _), (method, _))| (kind, method))
        .or_not();
    let property_registration =
        select! { Token::Attribute(name) if name == "surface_properties" => () }
            .ignore_then(just(Token::LParen))
            .ignore_then(
                ident
                    .clone()
                    .separated_by(just(Token::Comma))
                    .at_least(1)
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::RParen))
            .or_not();
    let surface_defaults = select! { Token::Attribute(name) if name == "surface_defaults" => () }
        .ignore_then(just(Token::LParen))
        .ignore_then(ident.clone().then_ignore(just(Token::Comma)).or_not())
        .then(
            ident_sp
                .clone()
                .then_ignore(just(Token::Colon))
                .then(expr::expr_parser())
                .map(|((node, span), value)| (crate::ast::Spanned { node, span }, value))
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::RParen))
        .or_not()
        .map(Option::unwrap_or_default);
    let interface_contract = property_registration
        .then_ignore(nl.clone())
        .then(entry_registration)
        .then_ignore(nl.clone())
        .then(surface_defaults)
        .then_ignore(nl.clone())
        .then(
            just(Token::Interface)
                .ignore_then(ident_sp.clone())
                .then(interface_body.clone()),
        )
        .map_with(
            |(((property_block, entry), surface_defaults), ((name, name_span), body_items)), e| {
                let mut plugs = Vec::new();
                let mut config_params = Vec::new();
                for item in body_items {
                    match item {
                        InterfaceBodyItem::Config(param) => config_params.push(param),
                        InterfaceBodyItem::Plug(plug) => plugs.push(plug),
                    }
                }
                TemplateDecl {
                    surface_defaults_block: surface_defaults.0,
                    surface_defaults: surface_defaults.1,
                    property_targets: property_block
                        .as_ref()
                        .map(|args| args[1..].to_vec())
                        .unwrap_or_default(),
                    property_block: property_block.map(|args| args[0].clone()),
                    entry,
                    name,
                    name_span,
                    params: Vec::new(),
                    plugs,
                    config_params,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let func = just(Token::Fn)
        .ignore_then(ident_sp.clone())
        .then(template_params.clone())
        .then(fn_params.clone())
        .then(ret_ty.clone())
        .then(
            nl.clone().ignore_then(
                block
                    .clone()
                    .rewind()
                    .then(stmt::block_parser_with_casts(true)),
            ),
        )
        .map_with(
            |(
                ((((name, name_span), (type_params, const_params)), params), ret_ty),
                (body, typed_body),
            ),
             e| {
                FnDecl {
                    typed_body: Some(typed_body),
                    name,
                    name_span,
                    docs: None,
                    type_params,
                    const_params,
                    params,
                    ret_ty,
                    is_internal: false,
                    derivative_free: false,
                    is_builtin: false,
                    source_file: String::new(),
                    body,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let internal_func = just(Token::Internal)
        .ignore_then(just(Token::Fn))
        .ignore_then(ident_sp.clone())
        .then(template_params.clone())
        .then(fn_params.clone())
        .then(ret_ty.clone())
        .then(
            nl.clone().ignore_then(
                block
                    .clone()
                    .rewind()
                    .then(stmt::block_parser_with_casts(true)),
            ),
        )
        .map_with(
            |(
                ((((name, name_span), (type_params, const_params)), params), ret_ty),
                (body, typed_body),
            ),
             e| {
                FnDecl {
                    typed_body: Some(typed_body),
                    name,
                    name_span,
                    docs: None,
                    type_params,
                    const_params,
                    params,
                    ret_ty,
                    is_internal: true,
                    derivative_free: false,
                    is_builtin: false,
                    source_file: String::new(),
                    body,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let extern_func = just(Token::Extern)
        .ignore_then(just(Token::Fn))
        .ignore_then(ident_sp.clone())
        .then(template_params.clone())
        .then(fn_params.clone())
        .then(ret_ty.clone())
        .map_with(
            |((((name, name_span), (type_params, const_params)), params), ret_ty), e| FnDecl {
                typed_body: None,
                name,
                name_span,
                docs: None,
                type_params,
                const_params,
                params,
                ret_ty,
                is_internal: false,
                derivative_free: false,
                is_builtin: false,
                source_file: String::new(),
                body: Vec::new(),
                span: e.span(),
            },
        )
        .boxed();

    let glsl_func = type_ident_sp
        .clone()
        .then(ident_sp.clone())
        .then(glsl_params)
        .then(
            nl.clone().ignore_then(
                block
                    .clone()
                    .rewind()
                    .then(stmt::block_parser_with_casts(true)),
            ),
        )
        .map_with(
            |((((ret_name, ret_span), (name, name_span)), params), (body, typed_body)), e| {
                let ret_ty = if ret_name == "void" {
                    None
                } else {
                    Some((ret_name, ret_span))
                };

                FnDecl {
                    typed_body: Some(typed_body),
                    name,
                    name_span,
                    docs: None,
                    type_params: Vec::new(),
                    const_params: Vec::new(),
                    params,
                    ret_ty,
                    is_internal: false,
                    derivative_free: false,
                    is_builtin: false,
                    source_file: String::new(),
                    body,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let builtin_func = just(Token::Builtin)
        .ignore_then(just(Token::Fn))
        .ignore_then(ident_sp.clone())
        .then(template_params.clone())
        .then(fn_params.clone())
        .then(ret_ty.clone())
        .then(nl.clone().ignore_then(block.clone()).or_not())
        .try_map(
            |(((((name, name_span), (type_params, const_params)), params), ret_ty), maybe_body),
             span| {
                if maybe_body.is_some() {
                    return Err(Rich::custom(
                        span,
                        "@builtin functions are declaration-only; remove the body block",
                    ));
                }

                Ok(FnDecl {
                    typed_body: None,
                    name,
                    name_span,
                    docs: None,
                    type_params,
                    const_params,
                    params,
                    ret_ty,
                    is_internal: false,
                    derivative_free: false,
                    is_builtin: true,
                    source_file: String::new(),
                    body: Vec::new(),
                    span,
                })
            },
        )
        .boxed();

    let enum_variant = ident_sp
        .clone()
        .then(
            just(Token::Eq)
                .ignore_then(select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) })
                .or_not(),
        )
        .map(|((name, span), _value)| EnumVariant { name, span })
        .boxed();

    let enum_sep = comma_sep.clone().or(sep.clone()).boxed();

    let enum_decl = just(Token::Enum)
        .ignore_then(ident_sp.clone())
        .then(
            just(Token::Colon)
                .ignore_then(type_ident_sp.clone())
                .or_not(),
        )
        .then(
            enum_variant
                .separated_by(enum_sep.clone())
                .allow_leading()
                .allow_trailing()
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(|(((name, name_span), _backing_ty), variants), e| EnumDecl {
            name,
            name_span,
            variants,
            span: e.span(),
        })
        .boxed();

    let struct_field = pipeline_attrs
        .clone()
        .map(|attrs| {
            attrs
                .into_iter()
                .map(|a| (a.name, a.args))
                .collect::<Vec<_>>()
        })
        .then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .then(just(Token::Eq).ignore_then(expr::expr_parser()).or_not())
        .map_with(
            |(((attrs, (name, name_span)), (ty_name, ty_span)), default), e| StructFieldDecl {
                attrs: attrs.clone(),
                semantic: attrs
                    .into_iter()
                    .find(|(name, _)| name == "semantic")
                    .map(|(_, args)| args.join(",")),
                name,
                name_span,
                ty_name,
                ty_span,
                default,
                span: e.span(),
            },
        )
        .boxed();

    let struct_decl = pipeline_attrs
        .clone()
        .then_ignore(just(Token::Struct))
        .then(ident_sp.clone())
        .then(
            struct_field
                .clone()
                .separated_by(enum_sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(|((attrs, (name, name_span)), fields), e| StructDecl {
            attrs,
            name,
            name_span,
            fields,
            span: e.span(),
        })
        .boxed();

    let resource_decl = pipeline_attrs
        .clone()
        .then_ignore(style_graph::keyword("resource"))
        .then(ident_sp.clone())
        .then(
            struct_field
                .separated_by(enum_sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(|((mut attrs, (name, name_span)), fields), e| {
            attrs.push(crate::ast::PipelineAttribute {
                name: "resource".into(),
                args: vec![],
                expressions: vec![],
                name_span: e.span(),
                args_span: None,
                span: e.span(),
            });
            StructDecl {
                attrs,
                name,
                name_span,
                fields,
                span: e.span(),
            }
        })
        .boxed();

    let interface_method = just(Token::Fn)
        .ignore_then(ident_sp.clone())
        .then(fn_params.clone())
        .then(ret_ty.clone())
        .map_with(
            |(((name, name_span), params), ret_ty), e| InterfaceMethodDecl {
                name,
                name_span,
                params,
                ret_ty,
                span: e.span(),
            },
        )
        .boxed();
    let interface_sep = comma_sep.clone().or(sep.clone()).boxed();
    let interface_decl = just(Token::Interface)
        .ignore_then(ident_sp.clone())
        .then(
            interface_method
                .clone()
                .separated_by(interface_sep)
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(|((name, name_span), methods), e| InterfaceDecl {
            entry: None,
            name,
            name_span,
            methods,
            span: e.span(),
        })
        .boxed();
    let conform_decl = just(Token::Conform)
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(ident_sp.clone())
        .then(
            func.clone()
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(
            |(((type_name, type_name_span), (interface_name, interface_name_span)), methods), e| {
                ConformanceDecl {
                    type_name,
                    type_name_span,
                    interface_name,
                    interface_name_span,
                    methods,
                    body_checked_with_instance: false,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let global_param_decl = just(Token::Param)
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(type_ident_sp.clone())
        .then(just(Token::Eq).ignore_then(expr::expr_parser()).or_not())
        .then(
            just(Token::In)
                .ignore_then(expr::expr_parser().try_map(
                    |range_expr, span| match range_expr.node {
                        Expr::Range(min, max) => Ok((*min, *max)),
                        _ => Err(Rich::custom(span, "expected range `min .. max`")),
                    },
                ))
                .or_not(),
        )
        .map_with(
            |((((name, name_span), (ty_name, ty_span)), default), range), e| {
                crate::ast::GlobalParamDecl {
                    name,
                    name_span,
                    ty_name,
                    ty_span,
                    default,
                    range,
                    span: e.span(),
                }
            },
        )
        .boxed();

    // The style spelling keeps constraints beside the type; the ordinary param
    // spelling remains accepted for consistency with existing surface programs.
    let constrained_style_param = just(Token::Param)
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(type_ident_sp.clone())
        .then_ignore(just(Token::In))
        .then(
            expr::expr_parser()
                .then_ignore(just(Token::Comma))
                .then(expr::expr_parser())
                .delimited_by(just(Token::LBracket), just(Token::RBracket)),
        )
        .then_ignore(just(Token::Eq))
        .then(expr::expr_parser())
        .map_with(
            |((((name, name_span), (ty_name, ty_span)), range), default), e| {
                crate::ast::GlobalParamDecl {
                    name,
                    name_span,
                    ty_name,
                    ty_span,
                    default: Some(default),
                    range: Some(range),
                    span: e.span(),
                }
            },
        )
        .boxed();
    let unsupported_style_member = select! {
        Token::Param => "style settings",
        Token::For => "style techniques",
        Token::At => "style integration points",
        Token::Optional => "optional contract capabilities and integration points",
        Token::Ident(s) if s == "static" => "static style graph branches",
        Token::Ident(s) if matches!(s.as_str(), "input" | "optional" | "capability" | "point" | "shading_input") => "contract capabilities and integration points",
    }.try_map(|feature, span| Err::<FnDecl, _>(Rich::custom(span,
        format!("{feature} are not supported yet; graph operations require the upcoming style graph implementation")))).boxed();
    let style_hook = choice((
        func.clone().map(|f| StyleHookDecl {
            signature: InterfaceMethodDecl {
                name: f.name.clone(),
                name_span: f.name_span.clone(),
                params: f.params.clone(),
                ret_ty: f.ret_ty.clone(),
                span: f.span.clone(),
            },
            default: Some(f),
        }),
        interface_method.map(|signature| StyleHookDecl {
            signature,
            default: None,
        }),
        unsupported_style_member.clone().map(|_| unreachable!()),
    ))
    .boxed();
    #[derive(Clone)]
    enum ContractMember {
        Hook(Box<StyleHookDecl>),
        Input(crate::ast::StyleInputDecl),
        Capability(crate::ast::StyleCapabilityUse),
        Point(crate::ast::StylePointDecl),
    }
    let optional = just(Token::Optional).or_not().map(|v| v.is_some());
    let contract_member = choice((
        style_hook.map(|hook| ContractMember::Hook(Box::new(hook))),
        style_graph::keyword("input")
            .ignore_then(style_graph::member())
            .map(ContractMember::Input),
        optional
            .clone()
            .then_ignore(style_graph::keyword("capability"))
            .then(ident.clone())
            .map_with(|(optional, name), e| {
                ContractMember::Capability(crate::ast::StyleCapabilityUse {
                    name,
                    optional,
                    span: e.span(),
                })
            }),
        optional
            .then_ignore(style_graph::keyword("point"))
            .then(style_graph::member())
            .then(style_graph::fields())
            .map_with(|((optional, member), fields), e| {
                ContractMember::Point(crate::ast::StylePointDecl {
                    name: member.name,
                    ty: member.ty,
                    optional,
                    fields,
                    span: e.span(),
                })
            }),
    ))
    .boxed();
    let style_contract_decl = select! { Token::Ident(s) if s == "contract" => () }
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::For))
        .then(ident_sp.clone())
        .then(
            contract_member
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(
            |(((name, name_span), (schema, schema_span)), hooks), e| StyleContractDecl {
                source_file: String::new(),
                name,
                name_span,
                schema,
                schema_span,
                hooks: hooks
                    .iter()
                    .filter_map(|m| {
                        if let ContractMember::Hook(v) = m {
                            Some((**v).clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                inputs: hooks
                    .iter()
                    .filter_map(|m| {
                        if let ContractMember::Input(v) = m {
                            Some(v.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                capabilities: hooks
                    .iter()
                    .filter_map(|m| {
                        if let ContractMember::Capability(v) = m {
                            Some(v.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                points: hooks
                    .into_iter()
                    .filter_map(|m| {
                        if let ContractMember::Point(v) = m {
                            Some(v)
                        } else {
                            None
                        }
                    })
                    .collect(),
                span: e.span(),
            },
        )
        .boxed();
    #[derive(Clone)]
    enum StyleMember {
        Method(FnDecl),
        Parameter(crate::ast::GlobalParamDecl),
        StaticParameter(crate::ast::GlobalParamDecl),
        Requirement(Vec<SExpr>),
        Graph(crate::ast::Spanned<crate::ast::StyleGraphNode>),
        ShadingInput(crate::ast::StyleShadingInputDecl),
    }
    let shading_input = style_graph::keyword("shading_input")
        .ignore_then(ident.clone())
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .then_ignore(style_graph::keyword("scope"))
        .then(ident.clone())
        .map_with(
            |((name, (ty, _)), scope), e| crate::ast::StyleShadingInputDecl {
                name,
                ty,
                scope,
                span: e.span(),
            },
        );
    let style_decl = just(Token::Style)
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::For))
        .then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(ident_sp.clone())
        .then(
            choice((
                func.clone().map(StyleMember::Method),
                shading_input.map(StyleMember::ShadingInput),
                select! { Token::Ident(s) if s == "static" => () }
                    .ignore_then(choice((
                        constrained_style_param.clone(),
                        global_param_decl.clone(),
                    )))
                    .map(StyleMember::StaticParameter),
                constrained_style_param.map(StyleMember::Parameter),
                global_param_decl.clone().map(StyleMember::Parameter),
                style_graph::keyword("requires")
                    .ignore_then(
                        expr::expr_parser()
                            .separated_by(just(Token::Comma))
                            .at_least(1)
                            .collect(),
                    )
                    .map(StyleMember::Requirement),
                style_graph::graph().map(StyleMember::Graph),
                unsupported_style_member.map(|_| unreachable!()),
            ))
            .separated_by(sep.clone())
            .allow_leading()
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(
                just(Token::LBrace).then_ignore(nl.clone()),
                nl.clone().ignore_then(just(Token::RBrace)),
            ),
        )
        .map_with(
            |((((name, name_span), (schema, schema_span)), (contract, contract_span)), methods),
             e| StyleDecl {
                source_file: String::new(),
                name,
                name_span,
                schema,
                schema_span,
                contract,
                contract_span,
                params: methods
                    .iter()
                    .filter_map(|member| {
                        if let StyleMember::Parameter(value) = member {
                            Some(value.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                static_params: methods
                    .iter()
                    .filter_map(|member| {
                        if let StyleMember::StaticParameter(value) = member {
                            Some(value.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                requirements: methods
                    .iter()
                    .filter_map(|member| {
                        if let StyleMember::Requirement(value) = member {
                            Some(value.clone())
                        } else {
                            None
                        }
                    })
                    .flatten()
                    .collect(),
                graph: methods
                    .iter()
                    .filter_map(|member| {
                        if let StyleMember::Graph(value) = member {
                            Some(value.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                shading_inputs: methods
                    .iter()
                    .filter_map(|member| {
                        if let StyleMember::ShadingInput(value) = member {
                            Some(value.clone())
                        } else {
                            None
                        }
                    })
                    .collect(),
                methods: methods
                    .into_iter()
                    .filter_map(|member| {
                        if let StyleMember::Method(value) = member {
                            Some(value)
                        } else {
                            None
                        }
                    })
                    .collect(),
                span: e.span(),
            },
        )
        .boxed();

    // --- texture_type parser ---
    // texture_type Name [-> vec3] {
    //     r: semantic_name
    //     g: semantic_name
    //     ...
    //     return expr
    // }
    let texture_channel_name = select! {
        Token::Ident(s) if matches!(s.as_str(), "r" | "g" | "b" | "a") => s.to_string()
    }
    .map_with(|ch, e| (ch, e.span()))
    .boxed();

    let texture_type_result_ty = just(Token::Arrow)
        .ignore_then(fn_param_type_sp())
        .or_not()
        .boxed();

    let texture_decode_num = select! {
        Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) as f32
    }
    .boxed();

    // Optional per-channel decode syntax:
    //   r: nx * 2 - 1
    //   g: roughness * 1
    // Applied as: decoded = raw * mul + add.
    let texture_channel_decode_affine = just(Token::Star)
        .ignore_then(texture_decode_num.clone())
        .then(
            just(Token::Plus)
                .ignore_then(texture_decode_num.clone())
                .or(just(Token::Minus)
                    .ignore_then(texture_decode_num)
                    .map(|v| -v))
                .or_not(),
        )
        .map(|(mul, add)| crate::ast::TextureChannelDecode::Affine {
            mul,
            add: add.unwrap_or(0.0),
        })
        .boxed();

    // Rich decode syntax:
    //   r: nx = raw * 2.0 - 1.0
    //   a: id = unpack_unorm8(raw, byte: 0)
    // Expression is evaluated during checking with `raw` as the sampled channel value.
    let texture_channel_decode_expr = just(Token::Eq)
        .ignore_then(expr::expr_parser())
        .map(crate::ast::TextureChannelDecode::Expr)
        .boxed();

    let texture_channel_decode =
        choice((texture_channel_decode_affine, texture_channel_decode_expr))
            .or_not()
            .boxed();

    let texture_channel_def = texture_channel_name
        .clone()
        .then_ignore(just(Token::Colon))
        .then(ident_sp.clone())
        .then(texture_channel_decode)
        .map_with(
            |(((channel, channel_span), (semantic_name, _sem_span)), decode), e| {
                TextureChannelDef {
                    channel,
                    channel_span,
                    semantic_name,
                    decode,
                    span: e.span(),
                }
            },
        )
        .boxed();

    enum TextureTypeBodyItem {
        Channel(TextureChannelDef),
        Return(SExpr),
        SampleMode,
        Layered,
        AtSignature,
    }

    let texture_type_at_signature = just(Token::At)
        .ignore_then(opaque_paren_block.clone())
        .then(just(Token::Arrow).ignore_then(fn_param_type_sp()).or_not())
        .ignored()
        .map(|_| TextureTypeBodyItem::AtSignature)
        .boxed();

    // `fn at(...) -> T` makes callable declarations uniform across the language while
    // remaining parse-only metadata for texture_type in this phase.
    let texture_type_fn_at_signature = just(Token::Fn)
        .ignore_then(just(Token::At))
        .ignore_then(opaque_paren_block.clone())
        .then(just(Token::Arrow).ignore_then(fn_param_type_sp()).or_not())
        .ignored()
        .map(|_| TextureTypeBodyItem::AtSignature)
        .boxed();

    let texture_type_body_item = choice((
        select! { Token::Ident(s) if s == "sample" => () }
            .ignore_then(just(Token::Colon))
            .ignore_then(choice((
                ident_sp.clone().map(|(mode, _)| mode).boxed(),
                just(Token::Pass).to("pass".to_string()).boxed(),
                just(Token::Pipeline).to("pipeline".to_string()).boxed(),
            )))
            .ignored()
            .map(|_| TextureTypeBodyItem::SampleMode)
            .boxed(),
        select! { Token::Ident(s) if s == "layered" => () }
            .map(|_| TextureTypeBodyItem::Layered)
            .boxed(),
        texture_type_at_signature,
        texture_type_fn_at_signature,
        texture_channel_def
            .clone()
            .map(TextureTypeBodyItem::Channel)
            .boxed(),
        just(Token::Return)
            .ignore_then(expr::expr_parser())
            .map(TextureTypeBodyItem::Return)
            .boxed(),
    ))
    .boxed();

    let texture_channel_sep = comma_sep.clone().or(sep.clone()).boxed();

    let texture_type_decl = just(Token::TextureType)
        .ignore_then(ident_sp.clone())
        .then(texture_type_result_ty)
        .then(
            texture_type_body_item
                .separated_by(texture_channel_sep)
                .allow_leading()
                .allow_trailing()
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(|(((name, name_span), result_ty), items), e| {
            let mut channels = Vec::new();
            let mut result_expr = None;

            for item in items {
                match item {
                    TextureTypeBodyItem::Channel(channel) => channels.push(channel),
                    TextureTypeBodyItem::Return(expr) => result_expr = Some(expr),
                    TextureTypeBodyItem::SampleMode => {}
                    TextureTypeBodyItem::Layered => {}
                    TextureTypeBodyItem::AtSignature => {}
                }
            }

            TextureTypeDecl {
                name,
                name_span,
                result_ty,
                channels,
                result_expr,
                span: e.span(),
            }
        })
        .boxed();

    let material_model_channel = select! { Token::Ident(s) if s == "channel" => () }
        .ignore_then(template_attrs.clone())
        .then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .then(just(Token::Eq).ignore_then(expr::expr_parser()).or_not())
        .validate(
            |(((attrs, (name, name_span)), (ty_name, ty_span)), default), extra, emitter| {
                let span = extra.span();
                let mut compose = None;
                for (attribute, args) in attrs {
                    if attribute == "compose" && args.len() == 1 && compose.is_none() {
                        compose = Some(args[0].clone());
                    } else {
                        emitter.emit(Rich::custom(span.clone(),
                            format!("invalid or duplicate material channel attribute `@{attribute}`; expected @compose(function)")));
                    }
                }
                MaterialChannelDecl {
                    compose,
                    name,
                    name_span,
                    ty_name,
                    ty_span,
                    default,
                    span,
                }
            },
        )
        .boxed();

    let material_properties_decl = pipeline_attrs
        .clone()
        .then(
            select! { Token::Ident(s) if s == "material_properties" => () }
                .ignore_then(ident_sp.clone())
                .then(
                    select! { Token::Ident(s) if s == "extends" => () }
                        .ignore_then(ident_sp.clone())
                        .or_not(),
                )
                .then(
                    material_model_channel
                        .clone()
                        .separated_by(sep.clone())
                        .allow_leading()
                        .allow_trailing()
                        .collect::<Vec<_>>()
                        .delimited_by(
                            just(Token::LBrace).then_ignore(nl.clone()),
                            nl.clone().ignore_then(just(Token::RBrace)),
                        ),
                )
                .map_with(
                    |(((name, name_span), extends), channels), e| MaterialPropertiesDecl {
                        context_parameter: None,
                        context: None,
                        is_default: false,
                        evaluator: None,
                        composition: None,
                        name,
                        name_span,
                        extends_name: extends.as_ref().map(|(n, _)| n.clone()),
                        extends_span: extends.map(|(_, sp)| sp),
                        channels,
                        span: e.span(),
                    },
                )
                .boxed(),
        )
        .validate(|(attrs, mut decl), _extra, emitter| {
            for attr in attrs {
                match attr.name.as_str() {
                    "default_material" if attr.args_span.is_none() && !decl.is_default => {
                        decl.is_default = true;
                    }
                    "composition" if attr.args.len() == 3 && decl.composition.is_none() => {
                        decl.composition = Some(
                            attr.args
                                .clone()
                                .try_into()
                                .expect("three composition names"),
                        );
                    }
                    "context" if attr.args.len() == 2 && decl.context.is_none() => {
                        decl.context = Some(attr.args[0].clone());
                        decl.context_parameter = Some(attr.args[1].clone());
                    }
                    "evaluator" if attr.args.len() == 1 && decl.evaluator.is_none() => {
                        decl.evaluator = Some(attr.args[0].clone());
                    }
                    _ => {
                        emitter.emit(Rich::custom(
                            attr.span,
                            "invalid or duplicate material schema attribute",
                        ));
                    }
                }
            }
            decl
        })
        .boxed();

    let schema_expression_decl = select! { Token::Ident(s) if s == "schema_expression" => () }
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::For))
        .then(ident_sp.clone())
        .then(
            select! { Token::Ident(s) if s == "shade" => () }
                .ignore_then(just(Token::Colon))
                .ignore_then(expr::expr_parser_with_casts(true))
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(
            |(((name, name_span), (properties_name, properties_span)), shade), e| {
                SchemaExpressionDecl {
                    name,
                    name_span,
                    properties_name,
                    properties_span,
                    shade,
                    span: e.span(),
                }
            },
        )
        .boxed();

    #[derive(Debug, Clone)]
    enum SchemaProgramItem {
        Function(FnDecl),
        Output(SExpr),
    }
    let schema_program_item = choice((
        func.clone().map(SchemaProgramItem::Function),
        select! { Token::Ident(s) if s == "output" => () }
            .ignore_then(just(Token::Colon))
            .ignore_then(expr::expr_parser_with_casts(true))
            .map(SchemaProgramItem::Output),
    ))
    .boxed();
    let schema_program_decl = select! { Token::Ident(s) if s == "schema_program" => () }
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::For))
        .then(ident_sp.clone())
        .then(
            schema_program_item
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .try_map(
            |(((name, name_span), (material_model_name, material_model_span)), items), _span| {
                let mut functions = Vec::new();
                let mut output = Vec::new();
                for item in items {
                    match item {
                        SchemaProgramItem::Function(function) => functions.push(function),
                        SchemaProgramItem::Output(value) => output.push(value),
                    }
                }
                Ok(SchemaProgramDecl {
                    name,
                    name_span,
                    material_model_name,
                    material_model_span,
                    functions,
                    output,
                })
            },
        )
        .boxed();

    #[derive(Debug, Clone)]
    enum EvaluationContractItem {
        Inputs(Vec<EvaluationContractBindingDecl>),
        Runtime(Vec<EvaluationContractBindingDecl>),
        Input(Vec<EvaluationContractBindingDecl>),
    }

    let evaluation_contract_name = choice((
        ident_sp.clone().boxed(),
        select! { Token::Str(s) => s }
            .map_with(|value, e| (value, e.span()))
            .boxed(),
    ))
    .boxed();

    let evaluation_contract_binding = evaluation_contract_name
        .clone()
        .then(just(Token::Colon).ignore_then(fn_param_type_sp()).or_not())
        .map(|((name, name_span), ty)| {
            let (ty_name, ty_span) = ty.map_or((None, None), |(ty_name, ty_span)| {
                (Some(ty_name), Some(ty_span))
            });
            let end_span = ty_span.clone().unwrap_or(name_span.clone());
            EvaluationContractBindingDecl {
                name,
                name_span: name_span.clone(),
                ty_name,
                ty_span,
                span: name_span.start..end_span.end,
            }
        })
        .boxed();

    let const_decl = select! { Token::Ident(s) if s == "const" => () }
        .ignore_then(choice((
            ident_sp
                .clone()
                .then_ignore(just(Token::Colon))
                .then(fn_param_type_sp())
                .map(|(name, ty)| (ty, name)),
            fn_param_type_sp().then(ident_sp.clone()),
        )))
        .then_ignore(just(Token::Eq))
        .then(expr::expr_parser_with_casts(true))
        .map_with(
            |(((ty_name, ty_span), (name, name_span)), value), e| ConstDecl {
                name,
                name_span,
                ty_name,
                ty_span,
                value,
                span: e.span(),
            },
        )
        .boxed();

    let evaluation_contract_binding_sep = comma_sep.clone().or(sep.clone()).boxed();

    let evaluation_contract_item = choice((
        select! { Token::Ident(s) if s == "inputs" => () }
            .ignore_then(just(Token::Colon))
            .ignore_then(
                evaluation_contract_binding
                    .clone()
                    .separated_by(comma_sep.clone())
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .map(EvaluationContractItem::Inputs),
        select! { Token::Ident(s) if s == "runtime" => () }
            .ignore_then(just(Token::Colon))
            .ignore_then(
                evaluation_contract_binding
                    .clone()
                    .separated_by(comma_sep.clone())
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .map(EvaluationContractItem::Runtime),
        select! { Token::Ident(s) if s == "input" => () }
            .ignore_then(
                evaluation_contract_binding
                    .clone()
                    .separated_by(comma_sep.clone())
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .map(EvaluationContractItem::Input),
    ))
    .boxed();

    let evaluation_contract_decl = select! { Token::Ident(s) if s == "contract" => () }
        .ignore_then(
            evaluation_contract_item
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .try_map(|items, span| {
            let mut inputs = Vec::new();
            let mut runtime = Vec::new();

            for item in items {
                match item {
                    EvaluationContractItem::Inputs(names) => inputs.extend(names),
                    EvaluationContractItem::Runtime(names) => runtime.extend(names),
                    EvaluationContractItem::Input(names) => runtime.extend(names),
                }
            }

            Ok(SchemaEvaluatorContractDecl {
                inputs,
                runtime,
                span,
            })
        })
        .boxed();

    let evaluation_input_decl = select! { Token::Ident(s) if s == "input" => () }
        .ignore_then(
            evaluation_contract_binding
                .clone()
                .separated_by(evaluation_contract_binding_sep)
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(|bindings, e| SchemaEvaluatorContractDecl {
            inputs: Vec::new(),
            runtime: bindings,
            span: e.span(),
        })
        .boxed();

    #[derive(Debug, Clone)]
    enum EvaluationPermutationAtomItem {
        Ident(String),
        Number(u32),
    }

    let evaluation_permutation_atom = choice((
        ident_sp
            .clone()
            .map(|(name, _span)| EvaluationPermutationAtomItem::Ident(name))
            .boxed(),
        select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) }
            .map(|count| EvaluationPermutationAtomItem::Number(count as u32))
            .boxed(),
    ))
    .boxed();

    let evaluation_permutation_item = ident_sp
        .clone()
        .then_ignore(just(Token::Colon))
        .then(choice((
            select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) }
                .then_ignore(just(Token::RangeOp))
                .then(select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) })
                .map(|(start, end)| EvaluationPermutationSpec::Range {
                    start: start as u32,
                    end: end as u32,
                })
                .boxed(),
            evaluation_permutation_atom
                .clone()
                .separated_by(just(Token::Bar))
                .at_least(1)
                .allow_trailing()
                .collect::<Vec<_>>()
                .map(|items| {
                    let atoms = items
                        .into_iter()
                        .map(|item| match item {
                            EvaluationPermutationAtomItem::Ident(name) => {
                                EvaluationPermutationAtom::Ident(name)
                            }
                            EvaluationPermutationAtomItem::Number(count) => {
                                EvaluationPermutationAtom::Number(count)
                            }
                        })
                        .collect::<Vec<_>>();
                    if atoms.len() == 1 {
                        EvaluationPermutationSpec::Single(atoms[0].clone())
                    } else {
                        EvaluationPermutationSpec::Set(atoms)
                    }
                })
                .boxed(),
        )))
        .map_with(
            |((name, name_span), spec), e| SchemaEvaluatorPermutationDecl {
                name,
                name_span,
                spec,
                span: e.span(),
            },
        )
        .boxed();

    let evaluation_permutations_decl = select! { Token::Ident(s) if s == "permutations" => () }
        .ignore_then(
            evaluation_permutation_item
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .boxed();

    let evaluation_specialize_decl = select! { Token::Ident(s) if s == "specialize" => () }
        .ignore_then(select! { Token::Ident(s) if s == "as" => () })
        .ignore_then(select! { Token::Str(s) => s })
        .map_with(|pattern, e| (pattern, e.span()))
        .boxed();

    let evaluation_variant_decl = select! { Token::Ident(s) if s == "variant" => () }
        .ignore_then(ident_sp.clone())
        .then(
            just(Token::When)
                .ignore_then(expr::expr_parser_with_casts(true))
                .or_not(),
        )
        .then_ignore(just(Token::Colon))
        .then(expr::expr_parser_with_casts(true))
        .map_with(
            |(((name, name_span), predicate), shade), e| SchemaEvaluatorVariantDecl {
                name,
                name_span,
                predicate,
                shade,
                span: e.span(),
            },
        )
        .boxed();

    #[derive(Debug, Clone)]
    enum SchemaEvaluatorItem {
        Contract(SchemaEvaluatorContractDecl),
        Input(SchemaEvaluatorContractDecl),
        Permutations(Vec<SchemaEvaluatorPermutationDecl>),
        Specialize(String, Span),
        Shade(SExpr),
        ShaderSource(String),
        Variant(SchemaEvaluatorVariantDecl),
    }

    let schema_evaluator_item = choice((
        evaluation_contract_decl
            .map(SchemaEvaluatorItem::Contract)
            .boxed(),
        evaluation_input_decl
            .map(SchemaEvaluatorItem::Input)
            .boxed(),
        evaluation_permutations_decl
            .map(SchemaEvaluatorItem::Permutations)
            .boxed(),
        evaluation_specialize_decl
            .map(|(pattern, span)| SchemaEvaluatorItem::Specialize(pattern, span))
            .boxed(),
        select! { Token::Ident(s) if s == "shade" => () }
            .ignore_then(just(Token::Colon))
            .ignore_then(expr::expr_parser_with_casts(true))
            .map(SchemaEvaluatorItem::Shade),
        select! { Token::Ident(s) if s == "shader" => () }
            .ignore_then(just(Token::Colon))
            .ignore_then(select! { Token::Str(s) => s })
            .map(SchemaEvaluatorItem::ShaderSource),
        evaluation_variant_decl.map(SchemaEvaluatorItem::Variant),
    ))
    .boxed();

    let schema_evaluator_decl = select! { Token::Ident(s) if s == "schema_evaluator" => () }
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::For))
        .then(ident_sp.clone())
        .then(
            schema_evaluator_item
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .try_map(
            |(((name, name_span), (material_model_name, material_model_span)), items), span| {
                let mut contract = None;
                let mut permutations = Vec::new();
                let mut specialize_pattern = None;
                let mut specialize_pattern_span = None;
                let mut shade = None;
                let mut shader_source = None;
                let mut variants = Vec::new();

                for item in items {
                    match item {
                        SchemaEvaluatorItem::Contract(decl) | SchemaEvaluatorItem::Input(decl) => {
                            contract = Some(decl);
                        }
                        SchemaEvaluatorItem::Permutations(mut decls) => {
                            permutations.append(&mut decls);
                        }
                        SchemaEvaluatorItem::Specialize(pattern, pattern_span) => {
                            specialize_pattern = Some(pattern);
                            specialize_pattern_span = Some(pattern_span);
                        }
                        SchemaEvaluatorItem::Shade(expr) => shade = Some(expr),
                        SchemaEvaluatorItem::ShaderSource(source) => shader_source = Some(source),
                        SchemaEvaluatorItem::Variant(variant) => variants.push(variant),
                    }
                }

                let shade = shade.unwrap_or_else(|| crate::ast::Spanned {
                    node: Expr::Var("true".to_string()),
                    span,
                });

                Ok(SchemaEvaluatorDecl {
                    name,
                    name_span,
                    material_model_name,
                    material_model_span,
                    contract,
                    permutations,
                    specialize_pattern,
                    specialize_pattern_span,
                    shade,
                    shader_source,
                    variants,
                })
            },
        )
        .boxed();

    // --- effect parser ---
    // effect Name<T>(param: type, ...) point|local|local(r)|global {
    //     <body stmts>
    //     rewrite outer(args) compose inner(args) => result(args) [when <expr>]
    //     ...
    // }

    // Locality clause: `point`, `local`, `local(<expr>)`, `global`
    let locality_point = select! { Token::Ident(s) if s == "point" => () }
        .map(|_| LocalityClass::Point)
        .boxed();

    let locality_global = select! { Token::Ident(s) if s == "global" => () }
        .map(|_| LocalityClass::Global)
        .boxed();

    let locality_local_with_radius = select! { Token::Ident(s) if s == "local" => () }
        .ignore_then(
            just(Token::LParen)
                .ignore_then(expr::expr_parser())
                .then_ignore(just(Token::RParen))
                .map(|e| LocalityClass::Local {
                    constraint: Some(Box::new(e)),
                })
                .boxed(),
        )
        .boxed();

    let locality_local_bare = select! { Token::Ident(s) if s == "local" => () }
        .map(|_| LocalityClass::Local { constraint: None })
        .boxed();

    // Must try local-with-radius before local-bare
    let locality_clause = choice((
        locality_point,
        locality_global,
        locality_local_with_radius,
        locality_local_bare,
    ))
    .boxed();

    // Rewrite pattern: `name(expr, expr, ...)`
    let rewrite_pattern = ident_sp
        .clone()
        .then(
            expr::expr_parser()
                .separated_by(comma_sep.clone())
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LParen).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RParen)),
                ),
        )
        .map_with(|((name, name_span), args), e| RewritePattern {
            name,
            name_span,
            args,
            span: e.span(),
        })
        .boxed();

    // Rewrite rule:
    // `rewrite outer(args) compose inner(args) => result(args) [within <expr>] [when <expr>]`
    let rewrite_rule = just(Token::Rewrite)
        .ignore_then(rewrite_pattern.clone())
        .then_ignore(just(Token::Compose))
        .then(rewrite_pattern.clone())
        .then_ignore(just(Token::FatArrow))
        .then(rewrite_pattern.clone())
        .then(
            just(Token::Within)
                .ignore_then(expr::expr_parser())
                .map(|e| Some(Box::new(e)))
                .or_not()
                .map(Option::flatten)
                .boxed(),
        )
        .then(
            just(Token::When)
                .ignore_then(expr::expr_parser())
                .map(|e| Some(Box::new(e)))
                .or_not()
                .map(Option::flatten)
                .boxed(),
        )
        .map_with(|((((outer, inner), result), tolerance), guard), e| {
            let lhs_span = outer.span.start..inner.span.end;
            RewriteRule {
                lhs: RewriteComposition {
                    outer,
                    inner,
                    span: lhs_span,
                },
                result,
                tolerance,
                guard,
                span: e.span(),
            }
        })
        .boxed();

    let rewrite_sep = comma_sep.clone().or(sep.clone()).boxed();

    // Effect body: statements (no surrounding braces — they come from `effect_inner`).
    let effect_body = stmt::stmt_parser()
        .separated_by(sep.clone())
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .boxed();

    let effect_rewrites = rewrite_rule
        .separated_by(rewrite_sep)
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .boxed();

    // Effect block: `{ <stmts> [rewrite ...] }`.
    // The outer braces come from `delimited_by`; the body is a flat statement list.
    let effect_inner = effect_body
        .then(
            nl.clone()
                .ignore_then(effect_rewrites)
                .or_not()
                .map(Option::unwrap_or_default),
        )
        .delimited_by(
            just(Token::LBrace).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

    let effect_decl = just(Token::Effect)
        .ignore_then(ident_sp.clone())
        .then(type_params.clone())
        .then(fn_params.clone())
        .then(locality_clause)
        .then(nl.clone().ignore_then(effect_inner))
        .map_with(
            |(((((name, name_span), type_params), params), locality), (body, rewrites)), e| {
                EffectDecl {
                    name,
                    name_span,
                    type_params,
                    params,
                    locality,
                    rewrites,
                    body,
                    span: e.span(),
                }
            },
        )
        .boxed();

    // --- surface parser ---
    // surface Name(sp: surf, time: signal, ...) -> material { <body> }
    // surface Name(sp: surf, ...) -> material(unlit|<model_name>) { <body> }
    //
    // The `surface` keyword is a first-class entry point parallel to `canvas`.
    // The return type is always `material` (optionally with a variant in
    // parentheses).  The body is a plain statement block.
    let surface_property_block = ident
        .clone()
        .then(
            entry_setting
                .clone()
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::LBrace), just(Token::RBrace)),
        )
        .map_with(|(name, values), e| crate::ast::SurfacePropertyBlock {
            name,
            values,
            span: e.span(),
        });
    let surface_body = choice((
        surface_property_block.map(|block| (Some(block), None)),
        stmt::stmt_parser().map(|statement| (None, Some(statement))),
    ))
    .separated_by(sep.clone())
    .allow_leading()
    .allow_trailing()
    .collect::<Vec<_>>()
    .delimited_by(just(Token::LBrace), just(Token::RBrace))
    .map_with(|items, e| (items, e.span().start + 1));
    let surface = just(Token::Surface)
        .ignore_then(entry_template_parts.clone())
        .then(
            just(Token::Arrow)
                .ignore_then(select! { Token::Ident(s) if s == "material" => () })
                .ignore_then(
                    just(Token::LParen)
                        .ignore_then(select! { Token::Ident(s) => s })
                        .map(|name| MaterialReturnTy::Named(name.to_string()))
                        .then_ignore(just(Token::RParen))
                        .or_not()
                        .map(|opt| opt.unwrap_or(MaterialReturnTy::Default)),
                )
                .or_not()
                .map(|opt| opt.unwrap_or(MaterialReturnTy::Default)),
        )
        .then(nl.clone().ignore_then(surface_body))
        .map_with(
            |((((name, name_span), params), material_ty), (items, body_start)), e| {
                let mut property_blocks = Vec::new();
                let mut body = Vec::new();
                for (block, statement) in items {
                    if let Some(block) = block {
                        property_blocks.push(block);
                    }
                    if let Some(statement) = statement {
                        body.push(statement);
                    }
                }
                SurfaceDecl {
                    body_start,
                    property_blocks,
                    settings: None,
                    name,
                    name_span,
                    params,
                    material_ty,
                    body,
                    span: e.span(),
                }
            },
        )
        .boxed();

    let tag_atom_sp = choice((
        ident_sp.clone().boxed(),
        just(Token::Pipeline)
            .map_with(|_, e| ("pipeline".to_string(), e.span()))
            .boxed(),
        just(Token::Space)
            .map_with(|_, e| ("space".to_string(), e.span()))
            .boxed(),
    ))
    .boxed();

    let tags_decl = just(Token::Tags)
        .ignore_then(tag_atom_sp.clone())
        .then(
            tag_atom_sp
                .clone()
                .map(|(value, span)| crate::ast::Spanned { node: value, span })
                .separated_by(comma_sep.clone())
                .allow_leading()
                .allow_trailing()
                .at_least(1)
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map_with(|((domain, domain_span), values), e| crate::ast::TagsDecl {
            domain,
            domain_span,
            values,
            span: e.span(),
        })
        .boxed();

    let budget_block = select! { Token::Ident(s) if s == "budget" => () }
        .ignore_then(opaque_braced_block.clone())
        .boxed();

    let budget_decl = budget_block.clone().ignored().boxed();

    // Staged parse-only support for top-level bind-group naming declarations:
    // `group frame, pass, material, draw`
    let group_name = choice((
        ident_sp.clone().map(|(name, _)| name).boxed(),
        just(Token::Pass).to("pass".to_string()).boxed(),
        just(Token::Pipeline).to("pipeline".to_string()).boxed(),
    ))
    .boxed();

    let group_decl = pipeline_attrs
        .clone()
        .then_ignore(select! { Token::Ident(s) if s == "group" => () })
        .then(
            group_name
                .separated_by(comma_sep.clone())
                .allow_trailing()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map_with(|(attrs, names), e| {
            names
                .into_iter()
                .map(|name| crate::ast::ResourceGroupDecl {
                    name,
                    attrs: attrs.clone(),
                    span: e.span(),
                })
                .collect::<Vec<_>>()
        })
        .boxed();

    let pass_binding_decl = pipeline_attrs
        .clone()
        .then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(
            select! { tok if !matches!(tok, Token::Newline | Token::RBrace) => tok }
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map_with(
            |((attrs, (name, name_span)), rhs_tokens), e| crate::ast::PassBindingDecl {
                operation_alias: None,
                group_index: None,
                binding_index: None,
                name,
                name_span,
                attrs,
                value_signature: canonical_layout_signature_from_tokens(&rhs_tokens),
                span: e.span(),
            },
        )
        .boxed();

    let pass_permutation_value = choice((
        ident_sp
            .clone()
            .filter(|(value, _)| value != "when" && value != "else")
            .map(|(value, span)| crate::ast::Spanned { node: value, span })
            .boxed(),
        select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) }
            .map_with(|value, e| crate::ast::Spanned {
                node: value.to_string(),
                span: e.span(),
            })
            .boxed(),
        select! { Token::Str(s) => s }
            .map_with(|value, e| crate::ast::Spanned {
                node: value,
                span: e.span(),
            })
            .boxed(),
    ))
    .boxed();

    let pass_permutation_decl = pipeline_attrs
        .clone()
        .then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(
            pass_permutation_value
                .clone()
                .separated_by(just(Token::Bar))
                .allow_trailing()
                .at_least(1)
                .collect::<Vec<_>>()
                .boxed(),
        )
        .then(
            just(Token::When)
                .ignore_then(expr::expr_parser())
                .or_not()
                .boxed(),
        )
        .then(
            just(Token::Else)
                .ignore_then(pass_permutation_value.clone())
                .or_not()
                .boxed(),
        )
        .map_with(
            |((((attrs, (name, name_span)), values), when_guard), else_value), e| {
                let value_signature = values
                    .iter()
                    .map(|value| value.node.clone())
                    .collect::<Vec<_>>()
                    .join("|");
                crate::ast::PassPermutationDecl {
                    name,
                    name_span,
                    attrs,
                    values,
                    when_guard: when_guard.map(axis::constraint),
                    else_value,
                    value_signature: Some(value_signature),
                    span: e.span(),
                }
            },
        )
        .boxed();

    let pass_requirement_decl = expr::expr_parser()
        .then_ignore(just(Token::FatArrow))
        .then(expr::expr_parser())
        .map_with(|(guard, constraint), e| crate::ast::PassRequirementDecl {
            clause: crate::ast::PassRequirementClause::Implication {
                guard: axis::constraint(guard),
                constraint: axis::constraint(constraint),
            },
            span: e.span(),
        })
        .boxed();

    let pass_requirement_clause = choice((
        pass_requirement_decl.clone(),
        expr::expr_parser()
            .map_with(|constraint, e| crate::ast::PassRequirementDecl {
                clause: crate::ast::PassRequirementClause::Plain {
                    constraint: axis::constraint(constraint),
                },
                span: e.span(),
            })
            .boxed(),
    ))
    .boxed();

    let pass_binding_block = select! { Token::Ident(s) if s == "binding" => () }
        .ignore_then(
            pass_binding_decl
                .clone()
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace)
                        .or(nl.clone().ignore_then(just(Token::LBrace)))
                        .then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .boxed();

    let pass_permutations_block = select! { Token::Ident(s) if s == "permutations" => () }
        .ignore_then(
            pass_permutation_decl
                .clone()
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace)
                        .or(nl.clone().ignore_then(just(Token::LBrace)))
                        .then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .boxed();

    let pass_require_block = select! { Token::Ident(s) if s == "require" => () }
        .ignore_then(
            pass_requirement_clause
                .clone()
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace)
                        .or(nl.clone().ignore_then(just(Token::LBrace)))
                        .then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .boxed();

    let pass_group_size_item = select! { Token::Ident(s) if s == "group_size" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(
            select! { tok if !matches!(tok, Token::Newline | Token::RBrace) => tok }
                .repeated()
                .at_least(1)
                .ignored(),
        )
        .boxed();

    let pass_dispatch_block_item = select! { Token::Ident(s) if s == "dispatch" => () }
        .ignore_then(
            select! { tok if !matches!(tok, Token::LBrace | Token::Newline | Token::RBrace) => tok }
                .repeated()
                .ignored(),
        )
        .then(opaque_braced_block.clone())
        .ignored()
        .boxed();

    let pass_fn_hook_name = choice((
        ident_sp.clone(),
        just(Token::Vertex)
            .map_with(|_, e| ("vertex".to_string(), e.span()))
            .boxed(),
    ))
    .boxed();

    let pass_fn_hook_param = pipeline_attrs
        .clone()
        .then(ident_sp.clone())
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .map_with(
            |((attrs, (name, name_span)), (ty_name, ty_span)), e| PassFnHookParamDecl {
                attrs,
                name,
                name_span,
                ty_name,
                ty_span,
                span: e.span(),
            },
        )
        .boxed();

    let pass_fn_hook_item = pipeline_attrs.clone().then_ignore(just(Token::Fn))
        .then(pass_fn_hook_name)
        .then(
            pass_fn_hook_param
                .clone()
                .separated_by(comma_sep.clone())
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LParen).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RParen)),
                )
                .or(just(Token::LParen)
                    .then_ignore(just(Token::RParen))
                    .map(|_| Vec::new()))
                .boxed(),
        )
        .then(
            just(Token::Arrow)
                .ignore_then(fn_param_type_sp())
                .map_with(|(ty_name, ty_span), _e| crate::ast::Spanned {
                    node: ty_name,
                    span: ty_span,
                })
                .or_not(),
        )
        .then(nl.clone().ignore_then(
            recursive(|nested| {
                just(Token::LBrace)
                    .map_with(|token, e| vec![(token, e.span())])
                    .then(choice((
                        nested,
                        select! { token if !matches!(token, Token::LBrace | Token::RBrace) => token }
                            .map_with(|token, e| vec![(token, e.span())]),
                    )).repeated().collect::<Vec<_>>())
                    .then(just(Token::RBrace).map_with(|token, e| (token, e.span())))
                    .map(|((mut open, contents), close)| {
                        open.extend(contents.into_iter().flatten());
                        open.push(close);
                        open
                    })
            }),
        ))
        .map_with(
            |((((attrs, (name, name_span)), params), return_ty), body), e| PassFnHookDecl {
                attrs,
                dispatch: None,
                name,
                name_span,
                params,
                return_ty,
                body,
                span: e.span(),
            },
        )
        .boxed();

    let pass_scalar_value = choice((
        ident_sp
            .clone()
            .map(|(value, span)| crate::ast::Spanned { node: value, span })
            .boxed(),
        select! { Token::Str(s) => s }
            .map_with(|value, e| crate::ast::Spanned {
                node: value,
                span: e.span(),
            })
            .boxed(),
    ))
    .boxed();

    let pass_resource_list = ident_sp
        .clone()
        .map(|(name, span)| crate::ast::Spanned { node: name, span })
        .separated_by(comma_sep.clone())
        .allow_leading()
        .allow_trailing()
        .at_least(1)
        .collect::<Vec<_>>()
        .boxed();

    let pass_scalar_tail = select! { tok if !matches!(tok, Token::Newline | Token::RBrace) => () }
        .repeated()
        .ignored()
        .boxed();

    let pass_stage_item = select! { Token::Ident(s) if s == "stage" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(pass_scalar_value.clone())
        .then_ignore(pass_scalar_tail.clone())
        .boxed();

    let pass_draw_item = select! { Token::Ident(s) if s == "draw" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(pass_scalar_value.clone())
        .then_ignore(pass_scalar_tail.clone())
        .boxed();

    let pass_blend_item = just(Token::Blend)
        .ignore_then(just(Token::Colon))
        .ignore_then(pass_scalar_value.clone())
        .then_ignore(pass_scalar_tail)
        .boxed();

    let pass_cull_item = select! { Token::Ident(s) if s == "cull" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(expr::expr_parser())
        .boxed();
    let state_fields = ident
        .clone()
        .then_ignore(just(Token::Colon))
        .then(expr::expr_parser())
        .separated_by(enum_sep.clone())
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LBrace).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

    let pass_sort_item = select! { Token::Ident(s) if s == "sort" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(
            select! { tok if !matches!(tok, Token::Newline | Token::RBrace) => tok }
                .repeated()
                .at_least(1)
                .ignored(),
        )
        .boxed();

    let pass_depth_block_item = select! { Token::Ident(s) if s == "depth" => () }
        .ignore_then(state_fields.clone())
        .boxed();

    let pass_stencil_block_item = select! { Token::Ident(s) if s == "stencil" => () }
        .ignore_then(opaque_braced_block.clone())
        .ignored()
        .boxed();

    let pass_blend_block_item = just(Token::Blend).ignore_then(state_fields).boxed();

    let pass_reads_item = select! { Token::Ident(s) if s == "reads" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(pass_resource_list.clone())
        .boxed();

    let pass_writes_item = select! { Token::Ident(s) if s == "writes" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(pass_resource_list.clone())
        .boxed();

    enum PassItem {
        Stage(crate::ast::Spanned<String>),
        Draw(crate::ast::Spanned<String>),
        Blend(crate::ast::Spanned<String>),
        Reads(Vec<crate::ast::Spanned<String>>),
        Writes(Vec<crate::ast::Spanned<String>>),
        Permutation(crate::ast::PassPermutationDecl),
        Requirement(crate::ast::PassRequirementDecl),
        Binding(crate::ast::PassBindingDecl),
        BindingBlock(Vec<crate::ast::PassBindingDecl>),
        PermutationsBlock(Vec<crate::ast::PassPermutationDecl>),
        RequirementsBlock(Vec<crate::ast::PassRequirementDecl>),
        GroupSize,
        Dispatch,
        Hook(PassFnHookDecl),
        Cull(SExpr),
        Sort,
        Depth(Vec<(String, SExpr)>),
        Stencil,
        BlendBlock(Vec<(String, SExpr)>),
        OperationRequires(SExpr),
        Raster(SExpr, Option<(SExpr, SExpr)>),
        Attachments(Vec<crate::ast::StyleGraphField>),
        Visibility(SExpr),
        SortPosition(SExpr),
        Output(crate::ast::ComputeOutput),
        Workgroup(Vec<SExpr>),
        Threads(Vec<SExpr>),
        OutputReturn(crate::ast::Spanned<String>),
    }

    let operation_dimensions = expr::expr_parser()
        .separated_by(comma_sep.clone())
        .allow_trailing()
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LParen).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RParen)),
        )
        .boxed();
    let compute_item = choice((
        style_graph::keyword("output")
            .ignore_then(style_graph::member())
            .then(operation_dimensions.clone())
            .map_with(|(member, extents), e| {
                PassItem::Output(crate::ast::ComputeOutput {
                    name: member.name,
                    ty: member.ty,
                    extents,
                    span: e.span(),
                })
            }),
        style_graph::keyword("workgroup_size")
            .then_ignore(just(Token::Colon))
            .ignore_then(operation_dimensions.clone())
            .map(PassItem::Workgroup),
        style_graph::keyword("dispatch")
            .ignore_then(style_graph::keyword("threads"))
            .ignore_then(operation_dimensions)
            .map(PassItem::Threads),
        just(Token::Return)
            .ignore_then(ident_sp.clone())
            .map(|(node, span)| PassItem::OutputReturn(crate::ast::Spanned { node, span })),
    ))
    .boxed();
    let pass_item = choice((
        style_graph::keyword("requires")
            .ignore_then(expr::expr_parser())
            .map(PassItem::OperationRequires),
        style_graph::keyword("raster")
            .ignore_then(expr::expr_parser())
            .then(
                style_graph::keyword("using")
                    .ignore_then(expr::expr_parser())
                    .then_ignore(style_graph::keyword("base_vertex"))
                    .then(expr::expr_parser())
                    .or_not(),
            )
            .map(|(geometry, vertices)| PassItem::Raster(geometry, vertices)),
        style_graph::keyword("attachments")
            .ignore_then(style_graph::fields())
            .map(PassItem::Attachments),
        style_graph::keyword("visibility")
            .ignore_then(just(Token::Colon))
            .ignore_then(expr::expr_parser())
            .map(PassItem::Visibility),
        style_graph::keyword("sort_position")
            .ignore_then(just(Token::Colon))
            .ignore_then(expr::expr_parser())
            .map(PassItem::SortPosition),
        pass_stage_item.map(PassItem::Stage).boxed(),
        pass_draw_item.map(PassItem::Draw).boxed(),
        pass_cull_item.map(PassItem::Cull).boxed(),
        pass_sort_item.map(|_| PassItem::Sort).boxed(),
        pass_depth_block_item.map(PassItem::Depth).boxed(),
        pass_stencil_block_item.map(|_| PassItem::Stencil).boxed(),
        pass_blend_block_item.map(PassItem::BlendBlock).boxed(),
        pass_blend_item.map(PassItem::Blend).boxed(),
        pass_reads_item.map(PassItem::Reads).boxed(),
        pass_writes_item.map(PassItem::Writes).boxed(),
        pass_permutation_decl.map(PassItem::Permutation).boxed(),
        pass_requirement_decl.map(PassItem::Requirement).boxed(),
        pass_permutations_block
            .map(PassItem::PermutationsBlock)
            .boxed(),
        pass_require_block.map(PassItem::RequirementsBlock).boxed(),
        pass_binding_block
            .clone()
            .map(PassItem::BindingBlock)
            .boxed(),
        pass_group_size_item.map(|_| PassItem::GroupSize).boxed(),
        pass_dispatch_block_item.map(|_| PassItem::Dispatch).boxed(),
        pass_fn_hook_item.clone().map(PassItem::Hook).boxed(),
        pass_binding_decl.map(PassItem::Binding).boxed(),
    ))
    .boxed();

    let pass_body = compute_item
        .or(pass_item)
        .separated_by(sep.clone())
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LBrace).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

    let operation_inputs = style_graph::member()
        .separated_by(comma_sep.clone())
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LParen).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RParen)),
        );
    let pass_header = choice((
        just(Token::Pass)
            .ignore_then(ident_sp.clone())
            .then(just(Token::For).ignore_then(ident_sp.clone()).or_not())
            .map(|(name, material)| (name, material, None)),
        style_graph::keyword("draw")
            .ignore_then(ident_sp.clone())
            .then(operation_inputs.clone())
            .then(just(Token::For).ignore_then(ident_sp.clone()).or_not())
            .map(|((name, inputs), material)| {
                (
                    name,
                    material,
                    Some(Box::new(crate::ast::StyleOperation {
                        inputs,
                        raster: None,
                        generated_vertices: None,
                        requirements: vec![],
                        attachments: vec![],
                        visibility: None,
                        sort_position: None,
                        compute: None,
                    })),
                )
            }),
        style_graph::keyword("compute")
            .ignore_then(ident_sp.clone())
            .then(operation_inputs)
            .then_ignore(just(Token::Arrow))
            .then(fn_param_type_sp())
            .map(|((name, inputs), (node, span))| {
                (
                    name,
                    None,
                    Some(Box::new(crate::ast::StyleOperation {
                        inputs,
                        raster: None,
                        generated_vertices: None,
                        requirements: vec![],
                        attachments: vec![],
                        visibility: None,
                        sort_position: None,
                        compute: Some(crate::ast::ComputeOperation {
                            return_ty: crate::ast::Spanned { node, span },
                            output: None,
                            workgroup_size: None,
                            threads: None,
                            returned: None,
                        }),
                    })),
                )
            }),
    ));
    let pass_decl = pipeline_attrs.clone().then(pass_header).then(pass_body)
        .try_map(
            |((attrs, ((name, name_span), material, mut operation)), items), span| {
                let mut state = Vec::new();
                let mut stage: Option<crate::ast::Spanned<String>> = None;
                let mut draw: Option<crate::ast::Spanned<String>> = None;
                let mut blend: Option<crate::ast::Spanned<String>> = None;
                let mut reads: Vec<crate::ast::Spanned<String>> = Vec::new();
                let mut writes: Vec<crate::ast::Spanned<String>> = Vec::new();
                let mut permutations = Vec::new();
                let mut requirements = Vec::new();
                let mut bindings = Vec::new();
                let mut hooks = Vec::new();
                let mut vertex_interface: Option<crate::ast::Spanned<String>> = None;
                for item in items {
                    match item {
                        PassItem::Output(value) => {
                            let compute = operation.as_mut().and_then(|o| o.compute.as_mut()).ok_or_else(|| Rich::custom(span.clone(), "output belongs to a compute operation"))?;
                            if compute.output.replace(value).is_some() { return Err(Rich::custom(span.clone(), "compute operation returns exactly one output")); }
                        }
                        PassItem::Workgroup(value) => {
                            let compute = operation.as_mut().and_then(|o| o.compute.as_mut()).ok_or_else(|| Rich::custom(span.clone(), "workgroup_size belongs to a compute operation"))?;
                            if compute.workgroup_size.replace(value).is_some() { return Err(Rich::custom(span.clone(), "duplicate compute workgroup_size")); }
                        }
                        PassItem::Threads(value) => {
                            let compute = operation.as_mut().and_then(|o| o.compute.as_mut()).ok_or_else(|| Rich::custom(span.clone(), "dispatch threads belongs to a compute operation"))?;
                            if compute.threads.replace(value).is_some() { return Err(Rich::custom(span.clone(), "duplicate compute dispatch")); }
                        }
                        PassItem::OutputReturn(value) => {
                            let compute = operation.as_mut().and_then(|o| o.compute.as_mut()).ok_or_else(|| Rich::custom(span.clone(), "resource return belongs to a compute operation"))?;
                            if compute.returned.replace(value).is_some() { return Err(Rich::custom(span.clone(), "duplicate compute resource return")); }
                        }
                        PassItem::OperationRequires(value) => operation.as_mut().ok_or_else(|| Rich::custom(span.clone(),"requires is an operation precondition; passes use require {}"))?.requirements.push(value),
                        PassItem::Raster(value, vertices) => {
                            let operation = operation.as_mut().ok_or_else(|| Rich::custom(span.clone(),"raster belongs to a draw operation"))?;
                            if operation.raster.replace(value).is_some() { return Err(Rich::custom(span.clone(),"duplicate raster source")); }
                            operation.generated_vertices = vertices;
                        }
                        PassItem::Attachments(value) => {
                            let operation = operation.as_mut().ok_or_else(|| Rich::custom(span.clone(),"attachments belong to a draw operation"))?;
                            if !operation.attachments.is_empty() { return Err(Rich::custom(span.clone(),"duplicate operation attachments")); }
                            operation.attachments = value;
                        }
                        PassItem::SortPosition(value) => {
                            let operation = operation.as_mut().ok_or_else(|| Rich::custom(span.clone(),"sort_position belongs to a draw operation"))?;
                            if operation.sort_position.replace(value).is_some() { return Err(Rich::custom(span.clone(),"duplicate operation sort_position")); }
                        }
                        PassItem::Visibility(value) => {
                            let operation = operation.as_mut().ok_or_else(|| Rich::custom(span.clone(),"visibility belongs to a draw operation"))?;
                            if operation.visibility.replace(value).is_some() { return Err(Rich::custom(span.clone(),"duplicate operation visibility")); }
                        }
                        PassItem::Stage(value) => {
                            if let Some(existing) = &stage {
                                if existing.node != value.node {
                                    return Err(Rich::custom(
                                        value.span.clone(),
                                        format!(
                                            "conflicting pass semantic `stage` values `{}` and `{}` in pass `{}`",
                                            existing.node, value.node, name
                                        ),
                                    ));
                                }
                                return Err(Rich::custom(
                                    value.span,
                                    format!("duplicate pass semantic `stage` in pass `{}`", name),
                                ));
                            }
                            stage = Some(value);
                        }
                        PassItem::Draw(value) => {
                            if let Some(existing) = &draw {
                                if existing.node != value.node {
                                    return Err(Rich::custom(
                                        value.span.clone(),
                                        format!(
                                            "conflicting pass semantic `draw` values `{}` and `{}` in pass `{}`",
                                            existing.node, value.node, name
                                        ),
                                    ));
                                }
                                return Err(Rich::custom(
                                    value.span,
                                    format!("duplicate pass semantic `draw` in pass `{}`", name),
                                ));
                            }
                            draw = Some(value);
                        }
                        PassItem::Blend(value) => {
                            if let Some(existing) = &blend {
                                if existing.node != value.node {
                                    return Err(Rich::custom(
                                        value.span.clone(),
                                        format!(
                                            "conflicting pass semantic `blend` values `{}` and `{}` in pass `{}`",
                                            existing.node, value.node, name
                                        ),
                                    ));
                                }
                                return Err(Rich::custom(
                                    value.span,
                                    format!("duplicate pass semantic `blend` in pass `{}`", name),
                                ));
                            }
                            blend = Some(value);
                        }
                        PassItem::Reads(values) => {
                            if !reads.is_empty() {
                                return Err(Rich::custom(
                                    values[0].span.clone(),
                                    format!("duplicate pass semantic `reads` in pass `{}`", name),
                                ));
                            }
                            reads = values;
                        }
                        PassItem::Writes(values) => {
                            if !writes.is_empty() {
                                return Err(Rich::custom(
                                    values[0].span.clone(),
                                    format!("duplicate pass semantic `writes` in pass `{}`", name),
                                ));
                            }
                            writes = values;
                        }
                        PassItem::Permutation(axis) => permutations.push(axis),
                        PassItem::Requirement(requirement) => requirements.push(requirement),
                        PassItem::Binding(binding) => bindings.push(binding),
                        PassItem::BindingBlock(block_bindings) => {
                            bindings.extend(block_bindings);
                        }
                        PassItem::PermutationsBlock(block_permutations) => {
                            permutations.extend(block_permutations);
                        }
                        PassItem::RequirementsBlock(block_requirements) => {
                            requirements.extend(block_requirements);
                        }
                        PassItem::Hook(hook) => {
                            if hooks.iter().any(|existing: &PassFnHookDecl| existing.name == hook.name && existing.params.iter().map(|p| &p.ty_name).eq(hook.params.iter().map(|p| &p.ty_name))) {
                                return Err(Rich::custom(
                                    hook.name_span.clone(),
                                    format!("duplicate pass hook `{}` in pass `{name}`", hook.name),
                                ));
                            }
                            if (hook.attrs.iter().any(|a| a.name == "vertex") || hook.name == "vertex")
                                && let Some(first_param) = hook.params.first()
                                && first_param.ty_name != "u32"
                            {
                                    vertex_interface = Some(crate::ast::Spanned {
                                        node: first_param.ty_name.clone(),
                                        span: first_param.ty_span.clone(),
                                    });
                                }

                            hooks.push(hook);
                        }
                        PassItem::Cull(value) => state.push(("cull".into(), value)),
                        PassItem::Depth(fields) => state.extend(fields.into_iter().map(|(name,value)| (format!("depth_{name}"),value))),
                        PassItem::BlendBlock(fields) => state.extend(fields.into_iter().map(|(name,value)| (format!("blend_{name}"),value))),
                        PassItem::Sort | PassItem::Stencil => {
                            let key = if matches!(item, PassItem::Sort) { "sort" } else { "stencil" };
                            state.push((key.into(), crate::ast::Spanned { node: Expr::Num(0.0, Unit::None), span: span.clone() }));
                        }
                        PassItem::GroupSize | PassItem::Dispatch => {}
                    }
                }
                Ok(crate::ast::PassDecl {
                    service_captures: Vec::new(),
                    compute_invocation: None,
                    prepared_draw: None,
            preparation: None,
                    operation,
                    state,
                    entry_bindings: Vec::new(),
                    entry_properties: Vec::new(),
                    name,
                    name_span,
                    source_file: String::new(),
                    material_name: material.as_ref().map(|(name, _)| name.clone()),
                    material_span: material.map(|(_, span)| span),
                    attrs,
                    stage,
                    draw,
                    blend,
                    reads,
                    writes,
                    permutations,
                    requirements,
                    bindings,
                    hooks,
                    vertex_interface,
                    span,
                })
            },
        )
        .boxed();

    enum PipelineItem {
        Type((String, Span)),
        PassRef(crate::ast::PipelinePassRef),
    }

    let pipeline_type_item = select! { Token::Ident(s) if s == "type" => () }
        .ignore_then(just(Token::Colon))
        .ignore_then(select! { Token::Str(s) => s }.map_with(|value, e| (value, e.span())))
        .map(PipelineItem::Type)
        .boxed();

    let pipeline_pass_ref = pipeline_attrs
        .clone()
        .then(ident_sp.clone())
        .map_with(
            |(attrs, (name, name_span)), e| crate::ast::PipelinePassRef {
                invocation: None,
                name,
                name_span,
                attrs,
                span: e.span(),
            },
        )
        .boxed();

    let pipeline_item = choice((
        pipeline_type_item,
        pipeline_pass_ref.map(PipelineItem::PassRef),
    ))
    .boxed();

    let pipeline_body = pipeline_item
        .separated_by(sep.clone())
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LBrace).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

    let pipeline_kind_header = just(Token::LParen)
        .ignore_then(choice((
            ident_sp.clone().map(|(kind, span)| (kind, span)).boxed(),
            select! { Token::Str(s) => s }
                .map_with(|kind, e| (kind, e.span()))
                .boxed(),
        )))
        .then_ignore(just(Token::RParen))
        .or_not()
        .boxed();

    let pipeline_decl = pipeline_attrs
        .then_ignore(just(Token::Pipeline))
        .then(pipeline_kind_header)
        .then(ident_sp.clone())
        .then(just(Token::For).ignore_then(ident_sp.clone()).or_not())
        .then(pipeline_body)
        .try_map(
            |((((attrs, header_pipeline_type), (name, name_span)), material), items), span| {
                let mut pass_refs = Vec::new();
                let mut passes = Vec::new();
                let mut body_pipeline_type: Option<(String, Span)> = None;
                for item in items {
                    match item {
                        PipelineItem::Type(item) => {
                            if body_pipeline_type.is_some() {
                                return Err(Rich::custom(
                                    item.1,
                                    "pipeline declarations must define `type: \"...\"` exactly once",
                                ));
                            }
                            body_pipeline_type = Some(item);
                        }
                        PipelineItem::PassRef(pass_ref) => {
                            passes.push(crate::ast::Spanned {
                                node: pass_ref.name.clone(),
                                span: pass_ref.name_span.clone(),
                            });
                            pass_refs.push(pass_ref);
                        }
                    }
                }

                if let (Some((header_kind, _header_span)), Some((body_kind, body_span))) =
                    (&header_pipeline_type, &body_pipeline_type)
                    && header_kind != body_kind
                {
                    return Err(Rich::custom(
                        body_span.clone(),
                        format!(
                            "pipeline kind mismatch: header declares `{header_kind}` but body declares `{body_kind}`"
                        ),
                    ));
                }

                let resolved_pipeline_type = header_pipeline_type
                    .or(body_pipeline_type)
                    .or(Some(("postprocess".to_string(), span.clone())));

                let (pipeline_type, pipeline_type_span) = resolved_pipeline_type.unwrap();

                Ok(crate::ast::PipelineDecl {
                    resource_ports: Vec::new(),
                    name,
                    name_span,
                    material_name: material.as_ref().map(|(name, _)| name.clone()),
                    material_span: material.map(|(_, span)| span),
                    pipeline_type,
                    pipeline_type_span,
                    passes,
                    attrs,
                    pass_refs,
                    span,
                })
            },
        )
        .boxed();

    let payload_encoding_decl = select! { Token::Ident(s) if s == "payload_encoding" => () }
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::For))
        .then(ident_sp.clone())
        .then(opaque_braced_block.clone())
        .ignored()
        .boxed();

    let vertex_interface_member_decl = ident_sp
        .clone()
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .map_with(
            |((name, name_span), (ty_name, ty_span)), e| VertexInterfaceMemberDecl {
                name,
                name_span,
                ty_name,
                ty_span,
                optional: false,
                default: None,
                span: e.span(),
            },
        )
        .boxed();

    let vertex_format_member_decl = ident_sp
        .clone()
        .then_ignore(just(Token::Colon))
        .then(fn_param_type_sp())
        .then(just(Token::Eq).ignore_then(expr::expr_parser()).or_not())
        .map_with(|(((name, name_span), (ty_name, ty_span)), default), e| {
            VertexInterfaceMemberDecl {
                name,
                name_span,
                ty_name,
                ty_span,
                optional: false,
                default,
                span: e.span(),
            }
        })
        .boxed();

    let vertex_interface_decl_body = vertex_interface_member_decl
        .clone()
        .separated_by(sep.clone())
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LBrace).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

    let vertex_format_section_keyword = nl
        .clone()
        .then(choice((
            just(Token::Required).to("required".to_string()).boxed(),
            just(Token::Optional).to("optional".to_string()).boxed(),
            select! { Token::Ident(s) if s == "required" => "required".to_string() }.boxed(),
            select! { Token::Ident(s) if s == "optional" => "optional".to_string() }.boxed(),
        )))
        .map(|(_, name)| name)
        .boxed();

    let vertex_format_member_section = vertex_format_section_keyword
        .clone()
        .then(
            vertex_format_member_decl
                .clone()
                .separated_by(sep.clone())
                .allow_leading()
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBrace).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBrace)),
                ),
        )
        .map(|(section_name, members)| {
            let optional = section_name == "optional";
            members
                .into_iter()
                .map(|mut member| {
                    member.optional = optional;
                    member
                })
                .collect::<Vec<_>>()
        })
        .boxed();

    let vertex_format_decl_body = vertex_format_member_section
        .clone()
        .repeated()
        .collect::<Vec<Vec<VertexInterfaceMemberDecl>>>()
        .map(|sections| sections.into_iter().flatten().collect::<Vec<_>>())
        .delimited_by(
            just(Token::LBrace).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

    let vertex_interface_decl = select! { Token::Ident(s) if s == "vertex_interface" => () }
        .ignore_then(ident_sp.clone())
        .then(vertex_interface_decl_body)
        .map_with(|((name, name_span), members), e| VertexInterfaceDecl {
            name,
            name_span,
            members,
            span: e.span(),
        })
        .boxed();

    let vertex_format_decl = select! { Token::Ident(s) if s == "vertex_format" => () }
        .ignore_then(ident_sp.clone())
        .then(
            select! { Token::Ident(s) if s == "extends" => () }
                .ignore_then(ident_sp.clone())
                .or_not(),
        )
        .then(vertex_format_decl_body)
        .map_with(
            |(((name, name_span), parent), members), e| VertexFormatDecl {
                name,
                name_span,
                parent: parent.as_ref().map(|(name, _)| name.clone()),
                parent_span: parent.map(|(_, span)| span),
                members,
                span: e.span(),
            },
        )
        .boxed();

    enum VertexFactoryItem {
        BindingBlock(Vec<crate::ast::PassBindingDecl>),
        Hook(Box<PassFnHookDecl>),
    }

    let vertex_factory_body = choice((
        pass_binding_block
            .clone()
            .map(VertexFactoryItem::BindingBlock)
            .boxed(),
        pass_fn_hook_item
            .clone()
            .map(Box::new)
            .map(VertexFactoryItem::Hook)
            .boxed(),
    ))
    .separated_by(sep.clone())
    .allow_leading()
    .allow_trailing()
    .collect::<Vec<_>>()
    .delimited_by(
        just(Token::LBrace).then_ignore(nl.clone()),
        nl.clone().ignore_then(just(Token::RBrace)),
    )
    .boxed();

    let vertex_factory_decl = select! { Token::Ident(s) if s == "vertex_factory" => () }
        .ignore_then(ident_sp.clone())
        .then_ignore(just(Token::For))
        .then(ident_sp.clone())
        .then(vertex_factory_body)
        .try_map(
            |(((name, name_span), (target_format, target_format_span)), items), span| {
                let mut bindings = Vec::new();
                let mut hooks = Vec::new();
                for item in items {
                    match item {
                        VertexFactoryItem::BindingBlock(block) => bindings.extend(block),
                        VertexFactoryItem::Hook(hook) => {
                            if hooks.iter().any(|existing: &PassFnHookDecl| {
                                existing.name == hook.name
                                    && existing
                                        .params
                                        .iter()
                                        .map(|p| &p.ty_name)
                                        .eq(hook.params.iter().map(|p| &p.ty_name))
                            }) {
                                return Err(Rich::custom(
                                    hook.name_span.clone(),
                                    format!(
                                        "duplicate vertex factory hook `{}` in factory `{name}`",
                                        hook.name
                                    ),
                                ));
                            }
                            hooks.push(*hook);
                        }
                    }
                }
                Ok(VertexFactoryDecl {
                    name,
                    name_span,
                    target_format,
                    target_format_span,
                    bindings,
                    hooks,
                    span,
                })
            },
        )
        .boxed();

    #[derive(Debug, Clone)]
    enum Item {
        Pragma(PragmaDecl),
        Import(ImportDecl),
        Budget,
        Group(Vec<crate::ast::ResourceGroupDecl>),
        Const(ConstDecl),
        Enum(EnumDecl),
        Struct(StructDecl),
        Axis(crate::ast::AxisDecl),
        Param(crate::ast::GlobalParamDecl),
        Tags(crate::ast::TagsDecl),
        Pass(crate::ast::PassDecl),
        Pipeline(crate::ast::PipelineDecl),
        PayloadEncoding,
        VertexInterface(VertexInterfaceDecl),
        VertexFormat(VertexFormatDecl),
        VertexFactory(VertexFactoryDecl),
        MaterialProperties(MaterialPropertiesDecl),
        SchemaProgram(SchemaProgramDecl),
        SchemaExpression(SchemaExpressionDecl),
        SchemaEvaluator(SchemaEvaluatorDecl),
        TextureType(TextureTypeDecl),
        Interface(InterfaceDecl),
        Conformance(ConformanceDecl),
        StyleContract(StyleContractDecl),
        StyleCapability(crate::ast::StyleCapabilityDecl),
        StyleProvider(crate::ast::StyleProviderDecl),
        Style(StyleDecl),
        AuthoredEntry(crate::ast::AuthoredEntry),
        Surface(SurfaceDecl),
        InterfaceContract(TemplateDecl),
        Fn(FnDecl),
        Effect(EffectDecl),
    }

    let item_decl = choice((
        pragma.map(Item::Pragma),
        import.map(Item::Import),
        budget_decl.map(|_| Item::Budget),
        group_decl.map(Item::Group),
        const_decl.map(Item::Const),
        global_param_decl.map(Item::Param),
        enum_decl.map(Item::Enum),
        struct_decl.map(Item::Struct),
        resource_decl.map(Item::Struct),
        tags_decl.map(Item::Tags),
        pass_decl.map(Item::Pass),
        pipeline_decl.map(Item::Pipeline),
        payload_encoding_decl.map(|_| Item::PayloadEncoding),
        vertex_interface_decl.map(Item::VertexInterface),
        vertex_format_decl.map(Item::VertexFormat),
        vertex_factory_decl.map(Item::VertexFactory),
        material_properties_decl.map(Item::MaterialProperties),
        schema_program_decl.map(Item::SchemaProgram),
        schema_expression_decl.map(Item::SchemaExpression),
        schema_evaluator_decl.map(Item::SchemaEvaluator),
        texture_type_decl.map(Item::TextureType),
        interface_contract.clone().map(Item::InterfaceContract),
        interface_decl.map(Item::Interface),
    ))
    .boxed();

    let item_body = choice((
        style_contract_decl.map(Item::StyleContract),
        style_graph::capability().map(Item::StyleCapability),
        style_graph::provider().map(Item::StyleProvider),
        style_decl.map(Item::Style),
        conform_decl.map(Item::Conformance),
        surface.map(Item::Surface),
        interface_contract.clone().map(Item::InterfaceContract),
        effect_decl.map(Item::Effect),
        builtin_func.map(Item::Fn),
        extern_func.map(Item::Fn),
        internal_func.map(Item::Fn),
        func.map(Item::Fn),
        glsl_func.map(Item::Fn),
        authored_entry.map(Item::AuthoredEntry),
    ))
    .boxed();

    let item = choice((axis::declaration().map(Item::Axis), item_decl, item_body)).boxed();

    let items = item
        .separated_by(sep)
        .allow_leading()
        .allow_trailing()
        .collect::<Vec<_>>()
        .boxed();

    nl.clone()
        .ignore_then(items)
        .map(|items| {
            let mut program = Program::default();
            for item in items {
                match item {
                    Item::Pragma(pragma) => program.pragmas.push(pragma),
                    Item::Import(import) => program.imports.push(import),
                    Item::Budget => {}
                    Item::Group(groups) => program.groups.extend(groups),
                    Item::Const(const_decl) => program.consts.push(const_decl),
                    Item::Param(decl) => program.params.push(decl),
                    Item::Enum(enum_decl) => program.enums.push(enum_decl),
                    Item::Struct(struct_decl) => program.structs.push(struct_decl),
                    Item::Axis(axis) => program.axes.push(axis),
                    Item::Tags(tags) => program.tags.push(tags),
                    Item::Pass(pass_decl) => program.passes.push(pass_decl),
                    Item::Pipeline(pipeline_decl) => program.pipelines.push(pipeline_decl),
                    Item::PayloadEncoding => {}
                    Item::VertexInterface(vertex_interface) => {
                        program.vertex_interfaces.push(vertex_interface);
                    }
                    Item::VertexFormat(vertex_format) => program.vertex_formats.push(vertex_format),
                    Item::VertexFactory(vertex_factory) => {
                        program.vertex_factories.push(vertex_factory);
                    }
                    Item::MaterialProperties(material_properties) => {
                        program.material_properties.push(material_properties);
                    }
                    Item::SchemaProgram(schema_program) => {
                        program.schema_programs.push(schema_program);
                    }
                    Item::SchemaExpression(schema_expression) => {
                        program.schema_expressions.push(schema_expression);
                    }
                    Item::SchemaEvaluator(schema_evaluator) => {
                        program.schema_evaluators.push(schema_evaluator);
                    }
                    Item::TextureType(tt) => program.texture_types.push(tt),
                    Item::Interface(iface) => program.interfaces.push(iface),
                    Item::Conformance(conform) => program.conformances.push(conform),
                    Item::StyleContract(contract) => program.style_contracts.push(contract),
                    Item::StyleCapability(v) => program.style_capabilities.push(v),
                    Item::StyleProvider(v) => program.style_providers.push(v),
                    Item::Style(style) => program.styles.push(style),
                    Item::AuthoredEntry(entry) => {
                        if entry.kind == "canvas"
                            && entry.blocks.is_empty()
                            && entry.config.is_empty()
                        {
                            program.canvases.push(Canvas {
                                entry_kind: entry.kind,
                                declared_return_ty: entry.return_ty,
                                name: entry.name,
                                name_span: entry.name_span,
                                params: entry.params,
                                body: entry.body,
                                span: entry.span,
                            });
                        } else {
                            program.authored_entries.push(entry);
                        }
                    }
                    Item::Surface(surface) => program.surfaces.push(surface),
                    Item::InterfaceContract(interface_contract) => {
                        // One authored interface serves both pass contracts and generic bounds.
                        // Preserve complete function parameters, including keyword-only markers.
                        program.interfaces.push(InterfaceDecl {
                            entry: interface_contract.entry.clone(),
                            name: interface_contract.name.clone(),
                            name_span: interface_contract.name_span.clone(),
                            methods: interface_contract
                                .plugs
                                .iter()
                                .map(|plug| InterfaceMethodDecl {
                                    name: plug.name.clone(),
                                    name_span: plug.name_span.clone(),
                                    params: plug.params.clone(),
                                    ret_ty: plug
                                        .return_ty
                                        .as_ref()
                                        .map(|ty| (ty.node.clone(), ty.span.clone())),
                                    span: plug.span.clone(),
                                })
                                .collect(),
                            span: interface_contract.span.clone(),
                        });
                        program.templates.push(interface_contract);
                    }
                    Item::Fn(func) => program.functions.push(func),
                    Item::Effect(eff) => program.effects.push(eff),
                }
            }
            program
        })
        .then_ignore(nl)
        .then_ignore(end())
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    use logos::Logos;

    fn parse_program(src: &str) -> Program {
        let tokens = Token::lexer(src)
            .spanned()
            .map(|(tok, span)| (tok.expect("lexing should succeed in parser tests"), span))
            .collect::<Vec<_>>();
        let eoi = src.len()..src.len();
        let (program, parse_errs) = program().parse(input(&tokens, eoi)).into_output_errors();
        assert!(
            parse_errs.is_empty(),
            "unexpected parse errors: {parse_errs:?}"
        );
        program.unwrap_or_default()
    }

    #[test]
    fn canvas_and_surface_share_the_same_entry_template_shape() {
        let program = parse_program(
            r#"
            canvas preview(uv: coord) -> color {
                let x = 1.0;
            }

            surface base(sp: surf) -> material {
                let y = 2.0;
            }
            "#,
        );

        assert_eq!(program.canvases.len(), 1);
        assert_eq!(program.surfaces.len(), 1);
        assert_eq!(program.canvases[0].name, "preview");
        assert_eq!(program.surfaces[0].name, "base");
        assert_eq!(program.canvases[0].params.len(), 1);
        assert_eq!(program.surfaces[0].params.len(), 1);
        assert_eq!(program.canvases[0].body.len(), 1);
        assert_eq!(program.surfaces[0].body.len(), 1);
    }

    #[test]
    fn interface_declarations_are_parsed_as_direct_contracts() {
        let program = parse_program(
            r#"
            interface Canvas {
                fn draw(uv: vec2, ctx: CanvasContext) -> color
            }
            "#,
        );

        assert_eq!(program.templates.len(), 1);
        let template = &program.templates[0];
        assert_eq!(template.name, "Canvas");
        assert_eq!(template.plugs.len(), 1);
        assert_eq!(template.config_params.len(), 0);
    }

    #[test]
    fn global_param_decl_parses_struct_typed_without_default() {
        let program = parse_program(
            r#"
            struct FrameGlobals {
                time: f32
                delta_time: f32
                resolution: vec2
            }

            param frame: FrameGlobals
            "#,
        );

        assert_eq!(program.structs.len(), 1);
        assert_eq!(program.params.len(), 1);
        let decl = &program.params[0];
        assert_eq!(decl.name, "frame");
        assert_eq!(decl.ty_name, "FrameGlobals");
        assert!(decl.default.is_none());
        assert!(decl.range.is_none());
    }

    #[test]
    fn global_param_decl_parses_scalar_with_default_and_range() {
        let program = parse_program(
            r#"
            param intensity: f32 = 1.0 in 0.0 .. 2.0
            "#,
        );

        assert_eq!(program.params.len(), 1);
        let decl = &program.params[0];
        assert_eq!(decl.name, "intensity");
        assert_eq!(decl.ty_name, "f32");
        assert!(decl.default.is_some());
        assert!(decl.range.is_some());
    }

    #[test]
    fn vertex_factory_retains_bindings_and_transform_body() {
        let program = parse_program(
            r#"
            vertex_format StaticMesh {
                required {
                    position: vec3
                }
            }

            vertex_factory static for StaticMesh {
                binding {
                    @group(draw) model: uniform<mat4>
                }

                fn transform(v: StaticMesh) -> mat4 {
                    return model
                }
            }
            "#,
        );

        assert_eq!(program.vertex_factories.len(), 1);
        let factory = &program.vertex_factories[0];
        assert_eq!(factory.bindings.len(), 1);
        assert_eq!(factory.bindings[0].name, "model");
        assert_eq!(factory.hooks.len(), 1);
        assert_eq!(factory.hooks[0].name, "transform");
        assert!(!factory.hooks[0].body.is_empty());
    }

    #[test]
    fn real_canvas_interface_and_pass_parse() {
        let program = parse_program(
            r#"
            struct CanvasContext {
                time: f32
                resolution: vec2
                mouse: vec2
                frame: u32
            }

            struct ScreenVarying {
                clip_pos: vec4
                uv: vec2
            }

            struct CanvasViewConfig {
                @config(editor) zoom: f32 = 1.0
                @config(editor) pan: vec2 = vec2(0.0, 0.0)
            }

            interface canvas {
                fn draw(uv: vec2, ctx: CanvasContext) -> color
            }

            canvas glow {
                param intensity: f32 = 1.0

                fn draw(uv: vec2, ctx: CanvasContext) -> color {
                    let d = length(uv - vec2(0.5, 0.5))
                    return rgba(vec3(intensity / (d + 0.05)), 1.0)
                }
            }

            pass present2d for canvas {
                stage: raster
                draw: fullscreen

                binding {
                    @group(frame) ctx:  uniform<CanvasContext>
                    @group(frame) view: uniform<vec2>
                }

                fn vertex() -> ScreenVarying {
                    return ScreenVarying(clip_pos: vec4(0.0, 0.0, 0.0, 1.0), uv: vec2(0.0, 0.0))
                }

                fn shade(v: ScreenVarying, t: canvas) -> color {
                    return t.draw(v.uv, ctx)
                }
            }

            pipeline present for canvas {
                present2d
            }
            "#,
        );

        assert_eq!(program.templates.len(), 1);
        assert_eq!(program.canvases.len(), 1);
        assert_eq!(program.passes.len(), 1);
        assert_eq!(program.pipelines.len(), 1);
    }
}
