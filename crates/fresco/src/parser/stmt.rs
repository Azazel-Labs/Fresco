use super::*;
use crate::lexer::Unit;

type BlendStmtResult = Result<(SExpr, Option<AstSpanned<String>>), (Span, String)>;

fn compose_entry_span(entry: &ComposeEntry) -> Span {
    match entry {
        ComposeEntry::Expr { expr, .. } => expr.span.clone(),
        ComposeEntry::Block { span, .. }
        | ComposeEntry::If { span, .. }
        | ComposeEntry::InSpace { span, .. }
        | ComposeEntry::For { span, .. } => span.clone(),
    }
}

fn parse_blend_mode_from_args(
    args: &[Arg],
    fallback_span: &Span,
) -> Result<AstSpanned<String>, (Span, String)> {
    if args.is_empty() {
        return Err((
            fallback_span.clone(),
            "`blend(...)` expects one mode argument: over, add, screen, or multiply".to_string(),
        ));
    }
    if args.len() > 1 {
        return Err((
            args[1].value.span.clone(),
            "`blend(...)` accepts only one mode argument".to_string(),
        ));
    }

    let arg = &args[0];
    if let Some(name) = arg.name.as_deref()
        && name != "mode"
    {
        return Err((
            arg.value.span.clone(),
            format!("unknown argument `{name}` to `blend`; expected `mode`"),
        ));
    }

    match &arg.value.node {
        Expr::Var(mode) => Ok(AstSpanned {
            node: mode.clone(),
            span: arg.value.span.clone(),
        }),
        _ => Err((
            arg.value.span.clone(),
            "`blend(...)` expects a mode identifier: over, add, screen, or multiply".to_string(),
        )),
    }
}

fn extract_compose_pipe_blend(expr: SExpr) -> BlendStmtResult {
    match expr.node {
        Expr::Pipe {
            recv,
            name,
            name_span,
            args,
        } if name == "blend" => {
            let mode = parse_blend_mode_from_args(&args, &name_span)?;
            Ok((*recv, Some(mode)))
        }
        _ => Ok((expr, None)),
    }
}

pub(super) fn stmt_parser<'src>() -> PBox<'src, Stmt> {
    stmt_parser_with_casts(false)
}

fn stmt_parser_with_casts<'src>(preserve_casts: bool) -> PBox<'src, Stmt> {
    let nl = nl();
    let comma_sep = comma_sep();
    let sep = sep();
    let expr = expr::expr_parser_with_casts(preserve_casts);
    let ident = ident();
    let ident_sp = ident_sp();
    let array_param_ty = select! { Token::Ident(name) if name.as_str() == "array" => () }
        .ignore_then(just(Token::Lt))
        .ignore_then(ident.clone())
        .then(
            just(Token::Comma)
                .ignore_then(select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) })
                .or_not(),
        )
        .then_ignore(just(Token::Gt))
        .try_map(|(elem_ty, len_opt), span| {
            if let Some(len) = len_opt {
                // Fixed-size array: array<T, N>
                if len.fract() != 0.0 || len < 0.0 {
                    Err(Rich::custom(
                        span,
                        "array param length must be a non-negative integer literal",
                    ))
                } else {
                    Ok(format!("array<{elem_ty}, {}>", len as usize))
                }
            } else {
                // Dynamic array: array<T>
                Ok(format!("array<{elem_ty}>"))
            }
        })
        .boxed();
    // `texture<TypeName>` — typed texture param
    let texture_typed_param_ty = select! { Token::Ident(name) if name.as_str() == "texture" => () }
        .ignore_then(just(Token::Lt))
        .ignore_then(ident.clone())
        .then_ignore(just(Token::Gt))
        .map(|type_name| format!("texture<{type_name}>"))
        .boxed();
    // plain `texture` — untyped texture param (also matches via ident fallback, but explicit for clarity)
    let texture_plain_param_ty =
        select! { Token::Ident(name) if name.as_str() == "texture" => "texture".to_string() }
            .boxed();
    let param_ty = array_param_ty
        .or(texture_typed_param_ty)
        .or(texture_plain_param_ty)
        .or(ident.clone())
        .boxed();

    let arg_outer = argument_name()
        .then_ignore(just(Token::Colon))
        .or_not()
        .then(expr.clone())
        .map(|(name, value)| Arg { name, value })
        .boxed();
    let args_outer = arg_outer
        .separated_by(comma_sep.clone())
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LParen).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RParen)),
        )
        .boxed();

    let space_call = ident_sp
        .clone()
        .then(args_outer.clone())
        .map(|((name, name_span), args)| SpaceCall {
            name,
            name_span,
            args,
        })
        .boxed();

    let space_item = space_call
        .clone()
        .map(SpaceItem::Call)
        .or(ident_sp
            .clone()
            .map(|(name, name_span)| SpaceItem::Ref { name, name_span }))
        .boxed();

    let space_dot_sep = nl
        .clone()
        .ignore_then(just(Token::Dot))
        .then_ignore(nl.clone())
        .boxed();

    let space_chain = space_item
        .clone()
        .separated_by(space_dot_sep)
        .at_least(1)
        .collect::<Vec<_>>()
        .boxed();

    let style_stage = ident_sp
        .clone()
        .then(args_outer.clone().or_not())
        .map(|((name, name_span), args)| StyleStage {
            name,
            name_span,
            args: args.unwrap_or_default(),
        })
        .boxed();

    let style_stage_pipe = (nl
        .clone()
        .ignore_then(just(Token::PipeOp))
        .or(just(Token::PipeOp)))
    .ignore_then(style_stage.clone())
    .boxed();

    let style_chain = style_stage
        .clone()
        .then(style_stage_pipe.repeated().collect::<Vec<_>>())
        .map(|(head, tail)| {
            let mut stages = Vec::with_capacity(tail.len() + 1);
            stages.push(head);
            stages.extend(tail);
            stages
        })
        .boxed();

    let style_param = ident_sp
        .clone()
        .then_ignore(just(Token::Colon))
        .then(ident_sp.clone())
        .then_ignore(just(Token::Eq))
        .then(expr.clone())
        .map(
            |(((name, name_span), (ty_name, ty_span)), default)| StyleParam {
                name,
                name_span,
                ty_name,
                ty_span,
                default,
            },
        )
        .boxed();

    let style_params = style_param
        .separated_by(comma_sep.clone())
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(
            just(Token::LParen).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RParen)),
        )
        .or_not()
        .map(Option::unwrap_or_default)
        .boxed();

    recursive(|stmt| {
        // Staged fallback for constructs that are accepted syntactically before
        // full semantic/lowering support is implemented.
        let _opaque_braced_block = recursive(|nested| {
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
            nl.clone().ignore_then(just(Token::LBrace)).then_ignore(nl.clone()),
            nl.clone().ignore_then(just(Token::RBrace)),
        )
        .boxed();

        let block = stmt
            .clone()
            .then_ignore(sep.clone().or_not())
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(
                nl.clone().ignore_then(just(Token::LBrace)).then_ignore(nl.clone()),
                nl.clone().ignore_then(just(Token::RBrace)),
            )
            .labelled("block")
            .boxed();

        let local_decl = choice((
            just(Token::Let).to(false),
            select! { Token::Ident(s) if s == "var" => true },
        ))
        .then(ident_sp.clone())
        .then(just(Token::Colon).ignore_then(fn_param_type_sp()).or_not())
        .then_ignore(just(Token::Eq))
        .then(expr.clone())
        .map(|(((mutable, (name, name_span)), ty), value)| Stmt::Let {
            mutable,
            name,
            name_span,
            declared_ty_name: ty.as_ref().map(|(name, _)| name.clone()),
            declared_ty_span: ty.map(|(_, span)| span),
            value,
        })
        .boxed();
        let let_stmt = local_decl;

        let const_stmt = select! { Token::Ident(s) if s == "const" => () }
           .ignore_then(choice((
                ident_sp.clone().then_ignore(just(Token::Colon)).then(fn_param_type_sp())
                    .map(|(name, ty)| (ty, name)),
                fn_param_type_sp().then(ident_sp.clone()),
            )))
            .then_ignore(just(Token::Eq))
            .then(expr.clone())
            .map(|(((ty_name, ty_span), (name, name_span)), value)| Stmt::Const {
                name,
                name_span,
                ty_name,
                ty_span,
                value,
            })
            .boxed();

        // Accept any identifier or callable type (`fn(T)->R`) as a typed declaration
        // type. The only excluded identifier is `const`, which is handled by the
        // `const`-prefix branch below. All other identifiers — built-in types,
        // user-defined enum names, generic type parameters — are accepted here and
        // validated semantically by the checker.
        let typed_decl_type = fn_param_type_sp()
            .try_map(|(ty_name, ty_span), _| {
                if ty_name == "const" || ty_name == "var" {
                    Err(Rich::custom(
                        ty_span,
                        "expected a local declaration type before variable name",
                    ))
                } else {
                    Ok((ty_name, ty_span))
                }
            })
            .boxed();

        let typed_decl_item = ident_sp
            .clone()
            .then(just(Token::Eq).ignore_then(expr.clone()).or_not())
            .boxed();

        let typed_let_stmt = typed_decl_type
            .clone()
            .then(
                typed_decl_item
                    .clone()
                    .separated_by(comma_sep.clone())
                    .at_least(1)
                    .collect::<Vec<_>>(),
            )
            .try_map(|(declared_ty, items), e| {
                let (declared_ty_name, declared_ty_span) = declared_ty;
                let default_for_ty = |ty_name: &str, span: &Span| -> Option<SExpr> {
                    let num0 = || sp(Expr::Num(0.0, Unit::None), span.clone());
                    Some(match ty_name {
                        "f32" | "float" | "signal" | "delta" => num0(),
                        "vec2" | "coord" | "coord_like" | "resolution" => {
                            sp(Expr::Vec2(Box::new(num0()), Box::new(num0())), span.clone())
                        }
                        "vec3" => sp(
                            Expr::Vec3(Box::new(num0()), Box::new(num0()), Box::new(num0())),
                            span.clone(),
                        ),
                        "vec4" | "color" => sp(
                            Expr::Vec4(
                                Box::new(num0()),
                                Box::new(num0()),
                                Box::new(num0()),
                                Box::new(num0()),
                            ),
                            span.clone(),
                        ),
                        _ => return None,
                    })
                };

                let lets = items
                    .into_iter()
                    .map(|((name, name_span), init)| {
                        let value = if let Some(init) = init {
                            init
                        } else {
                            default_for_ty(&declared_ty_name, &name_span).ok_or_else(|| {
                                Rich::custom(
                                    name_span.clone(),
                                    format!(
                                        "typed declaration `{name}` of `{declared_ty_name}` requires an initializer",
                                    ),
                                )
                            })?
                        };

                        Ok(Stmt::Let {
                            mutable: true,
                            name,
                            name_span,
                            declared_ty_name: Some(declared_ty_name.clone()),
                            declared_ty_span: Some(declared_ty_span.clone()),
                            value,
                        })
                    })
                    .collect::<Result<Vec<_>, Rich<'_, Token, Span>>>()?;

                Ok(if lets.len() == 1 {
                    lets.into_iter()
                        .next()
                        .expect("typed declaration list always has at least one item")
                } else {
                    Stmt::Seq {
                        body: lets,
                        span: e,
                    }
                })
            })
            .boxed();

        let store_stmt = ident_sp.clone()
            .then(expr.clone().delimited_by(just(Token::LBracket),just(Token::RBracket)))
            .then_ignore(just(Token::Eq)).then(expr.clone())
            .map_with(|(((name,name_span),index),value),e| Stmt::Store {
                target: sp(Expr::Index {array: Box::new(sp(Expr::Var(name),name_span)),index: Box::new(index)}, e.span()),
                value, span:e.span(),
            }).boxed();
        let assign_stmt = ident_sp
            .clone()
            .then(
                just(Token::Dot)
                    .ignore_then(ident_sp.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then(
                just(Token::Eq)
                    .map_with(|_, e| (None, e.span()))
                    .or(just(Token::Plus)
                        .then_ignore(just(Token::Eq))
                        .map_with(|_, e| (Some(BinOp::Add), e.span())))
                    .or(just(Token::Minus)
                        .then_ignore(just(Token::Eq))
                        .map_with(|_, e| (Some(BinOp::Sub), e.span())))
                    .or(just(Token::Star)
                        .then_ignore(just(Token::Eq))
                        .map_with(|_, e| (Some(BinOp::Mul), e.span())))
                    .or(just(Token::Slash)
                        .then_ignore(just(Token::Eq))
                        .map_with(|_, e| (Some(BinOp::Div), e.span()))),
            )
            .then(expr.clone())
            .map_with(|((((name, name_span), tail), (op, eq_span)), rhs), _e| {
                let field_path = if tail.is_empty() {
                    None
                } else {
                    Some(
                        tail.into_iter()
                            .map(|(segment, _)| segment)
                            .collect::<Vec<_>>()
                            .join("."),
                    )
                };

                let lhs_name = if let Some(path) = &field_path {
                    format!("{name}.{path}")
                } else {
                    name.clone()
                };
                let lhs = sp(Expr::Var(lhs_name), name_span.clone());
                let rhs_span = rhs.span.clone();

                let (value, span) = match op {
                    None => (rhs, eq_span.start..rhs_span.end),
                    Some(bin_op) => {
                        let value = binx(bin_op, lhs, rhs);
                        (value.clone(), eq_span.start..value.span.end)
                    }
                };

                Stmt::Assign {
                    name,
                    name_span,
                    field_path,
                    value,
                    span,
                }
            })
            .boxed();

        let for_body = block.clone().or(stmt.clone().map(|single| vec![single])).boxed();
        let for_body_with_nl = nl
            .clone()
            .ignore_then(for_body.clone())
            .or(for_body.clone())
            .boxed();

        let for_stmt = select! { Token::Ident(s) if s == "unroll" => () }
            .or_not()
            .ignore_then(just(Token::For))
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(for_body_with_nl.clone())
            .map_with(|(((name, name_span), iterable), body), e| Stmt::For {
                name,
                name_span,
                iterable,
                body,
                index_name: None,
                span: e.span(),
            })
            .boxed();

        let c_for_stmt = select! { Token::Ident(s) if s == "unroll" => () }
            .or_not()
            .ignore_then(just(Token::For))
            .ignore_then(just(Token::LParen))
            .ignore_then(
                select! { Token::Ident(s) if s == "int" || s == "i32" || s == "u32" => () }
                    .or_not(),
            )
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::Eq))
            .then(expr.clone())
            .then_ignore(just(Token::Semicolon))
            .then(expr.clone())
            .then_ignore(just(Token::Semicolon))
            .then(ident_sp.clone())
            .then(just(Token::PlusPlus).to(true).or(just(Token::MinusMinus).to(false)))
            .then_ignore(just(Token::RParen))
            .then(for_body_with_nl.clone())
            .map_with(|payload, e| (payload, e.span()))
            .try_map(
                |(payload, span), _| {
                    let ((((((name, name_span), start), cond_expr), (inc_name, inc_name_span)), incrementing), body) = payload;
                    if inc_name != name {
                        return Err(Rich::custom(
                            inc_name_span,
                            "C-style for increment must use `i++` or `i--` on the same loop variable",
                        ));
                    }

                    let cmp_from_cond = |node: &SExpr| -> Option<(BinOp, SExpr)> {
                        let Expr::Binary(op, lhs, rhs) = &node.node else {
                            return None;
                        };
                        if !matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
                            return None;
                        }
                        let Expr::Var(var_name) = &lhs.node else {
                            return None;
                        };
                        if var_name != &name {
                            return None;
                        }
                        Some((*op, (*rhs.clone())))
                    };

                    let (cmp, end, extra_cond) = if let Some((cmp, end)) = cmp_from_cond(&cond_expr)
                    {
                        (cmp, end, None)
                    } else if let Expr::Binary(BinOp::LogicalAnd, left, right) = &cond_expr.node {
                        if let Some((cmp, end)) = cmp_from_cond(left) {
                            (cmp, end, Some(*right.clone()))
                        } else if let Some((cmp, end)) = cmp_from_cond(right) {
                            (cmp, end, Some(*left.clone()))
                        } else {
                            return Err(Rich::custom(
                                cond_expr.span.clone(),
                                "C-style for condition must be `i < end` (or `i < end && guard`) using the loop variable",
                            ));
                        }
                    } else {
                        return Err(Rich::custom(
                            cond_expr.span,
                            "C-style for condition must be `i < end` (or `i < end && guard`) using the loop variable",
                        ));
                    };

                    let num = |v: f64| sp(Expr::Num(v, Unit::None), span.clone());
                    let plus_one = |e: SExpr| binx(BinOp::Add, e, num(1.0));
                    let minus_expr = |a: SExpr, b: SExpr| binx(BinOp::Sub, a, b);
                    let range_expr = |a: SExpr, b: SExpr| {
                        let range_span = a.span.start..b.span.end;
                        sp(Expr::Range(Box::new(a), Box::new(b)), range_span)
                    };
                    let guard_stmt = |cond: SExpr| Stmt::If {
                        cond,
                        then_body: Vec::new(),
                        else_body: Some(vec![Stmt::Break { span: span.clone() }]),
                        span: span.clone(),
                    };

                    if incrementing {
                        let end_expr = match cmp {
                            BinOp::Lt => end,
                            BinOp::Le => plus_one(end),
                            BinOp::Gt | BinOp::Ge => {
                                return Err(Rich::custom(
                                    span,
                                    "`i++` loop requires `<` or `<=` condition",
                                ));
                            }
                            _ => unreachable!(),
                        };

                        let iterable_span = start.span.start..end_expr.span.end;
                        let iterable =
                            sp(Expr::Range(Box::new(start), Box::new(end_expr)), iterable_span);

                        let mut lowered_body =
                            Vec::with_capacity(body.len() + usize::from(extra_cond.is_some()));
                        if let Some(cond) = extra_cond {
                            lowered_body.push(guard_stmt(cond));
                        }
                        lowered_body.extend(body);

                        return Ok(Stmt::For {
                            name,
                            name_span,
                            iterable,
                            body: lowered_body,
                            index_name: None,
                            span,
                        });
                    }

                    let lower = match cmp {
                        BinOp::Gt => plus_one(end),
                        BinOp::Ge => end,
                        BinOp::Lt | BinOp::Le => {
                            return Err(Rich::custom(
                                span,
                                "`i--` loop requires `>` or `>=` condition",
                            ));
                        }
                        _ => unreachable!(),
                    };
                    let upper = plus_one(start.clone());
                    let idx_name = format!("__cfor_{}", span.start);
                    let idx_var = sp(Expr::Var(idx_name.clone()), span.clone());
                    let lowered_idx = minus_expr(idx_var, lower.clone());
                    let user_value = minus_expr(start, lowered_idx);

                    let mut lowered_body =
                        Vec::with_capacity(body.len() + 1 + usize::from(extra_cond.is_some()));
                    lowered_body.push(Stmt::Let {
                        mutable: false,
                        name: name.clone(),
                        name_span: name_span.clone(),
                        declared_ty_name: None,
                        declared_ty_span: None,
                        value: user_value,
                    });
                    if let Some(cond) = extra_cond {
                        lowered_body.push(guard_stmt(cond));
                    }
                    lowered_body.extend(body);

                    let iterable = range_expr(lower, upper);

                    Ok(Stmt::For {
                        name: idx_name,
                        name_span,
                        iterable,
                        body: lowered_body,
                        index_name: None,
                        span,
                    })
                },
            )
            .boxed();

        // `each (val, idx) in expr { body }` — compile-time loop with index binding.
        // `each (val) in expr { body }` — single-binding form (equivalent to `for val in expr`).
        let each_binding = just(Token::LParen)
            .ignore_then(ident_sp.clone())
            .then(
                just(Token::Comma)
                    .ignore_then(ident_sp.clone())
                    .or_not(),
            )
            .then_ignore(just(Token::RParen));

        let each_stmt = just(Token::Each)
            .ignore_then(each_binding.clone())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(for_body_with_nl.clone())
            .map_with(
                |((((val_name, val_name_span), idx_binding), iterable), body), e| Stmt::For {
                    name: val_name,
                    name_span: val_name_span,
                    iterable,
                    body,
                    index_name: idx_binding,
                    span: e.span(),
                },
            )
            .boxed();

        let if_stmt = recursive(|if_stmt| {
            let if_body = block.clone().or(stmt.clone().map(|single| vec![single])).boxed();

            let else_clause = just(Token::Else)
                .or(sep.clone().ignore_then(just(Token::Else)))
                .ignore_then(if_stmt.clone().map(|nested| vec![nested]).or(if_body.clone()))
                .or_not()
                .boxed();

            just(Token::If)
                .ignore_then(expr.clone())
                .then(if_body)
                .then(else_clause)
                .map_with(|((cond, then_body), else_body), e| Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    span: e.span(),
                })
                .boxed()
        })
        .boxed();

        let scatter_lifecycle = nl
            .clone()
            .ignore_then(just(Token::Lifetime))
            .ignore_then(
                ident_sp
                    .clone()
                    .then_ignore(just(Token::Colon))
                    .then(expr.clone()),
            )
            .then_ignore(just(Token::Respawn))
            .then_ignore(just(Token::Every))
            .then(expr.clone())
            .map(
                |(((name, span), lifetime), respawn_every)| ScatterLifecycle {
                    instance_name: crate::ast::Spanned { node: name, span },
                    lifetime,
                    respawn_every,
                },
            )
            .or_not()
            .boxed();

        let pipe_call = (nl
            .clone()
            .ignore_then(just(Token::PipeOp))
            .or(just(Token::PipeOp)))
        .ignore_then(ident_sp.clone().then(args_outer.clone()))
        .map(|((name, name_span), args)| ScatterPipeCall {
            name,
            name_span,
            args,
        })
        .boxed();

        let scatter_decl = just(Token::Scatter)
            .ignore_then(expr.clone())
            .then_ignore(nl.clone())
            .then_ignore(just(Token::Within))
            .then_ignore(nl.clone())
            .then(expr.clone())
            .then_ignore(nl.clone())
            .then_ignore(just(Token::Seed))
            .then_ignore(nl.clone())
            .then(expr.clone())
            .then_ignore(nl.clone())
            .then_ignore(just(Token::Strategy))
            .then_ignore(nl.clone())
            .then(ident_sp.clone())
            .then(scatter_lifecycle)
            .then_ignore(nl.clone().or_not())
            .then(block.clone())
            .then(pipe_call.clone().repeated().collect::<Vec<_>>())
            .map_with(|value, e| {
                let (
                    (((((count, region), seed), (strategy, strategy_span)), lifecycle), body),
                    post_pipes,
                ) = value;
                ScatterDecl {
                    count,
                    region,
                    seed,
                    strategy: crate::ast::Spanned {
                        node: strategy,
                        span: strategy_span,
                    },
                    lifecycle,
                    body,
                    post_pipes,
                    span: e.span(),
                }
            })
            .boxed();

        let let_scatter_stmt = just(Token::Let)
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::Eq))
            .then(scatter_decl)
            .map(|((name, name_span), scatter)| Stmt::LetScatter {
                name,
                name_span,
                scatter,
            })
            .boxed();

        let space_decl = just(Token::Space)
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::Eq))
            .then(space_chain.clone())
            .map_with(|((name, name_span), chain), e| Stmt::SpaceDecl {
                name,
                name_span,
                chain,
                span: e.span(),
            })
            .boxed();

        let canvas_space_decl = just(Token::CanvasSpace)
            .ignore_then(just(Token::Eq))
            .ignore_then(space_chain.clone())
            .map_with(|chain, e| Stmt::CanvasSpace {
                chain,
                span: e.span(),
            })
            .boxed();

        let style_decl = just(Token::Style)
            .ignore_then(ident_sp.clone())
            .then(style_params.clone())
            .then_ignore(just(Token::Eq))
            .then(style_chain)
            .map_with(|(((name, name_span), params), stages), e| Stmt::StyleDecl {
                name,
                name_span,
                params,
                stages,
                span: e.span(),
            })
            .boxed();

        let kw_binding = select! {
            Token::Ident(s) if s.as_str() == "binding" => ()
        }
        .boxed();
        let kw_default = select! {
            Token::Ident(s) if s.as_str() == "default" => ()
        }
        .boxed();
        let kw_uniform = select! {
            Token::Ident(s) if s.as_str() == "uniform" => ()
        }
        .boxed();

        let binding_attr = just(Token::LBracket)
            .ignore_then(kw_binding)
            .ignore_then(just(Token::LParen))
            .ignore_then(kw_default)
            .then_ignore(just(Token::Colon).or(just(Token::Eq)))
            .ignore_then(select! { Token::Str(s) => s })
            .then_ignore(just(Token::RParen))
            .then_ignore(just(Token::RBracket))
            .or_not()
            .boxed();

        let texture_binding_stmt = binding_attr
            .then_ignore(nl.clone())
            .then_ignore(kw_uniform)
            .then(ident_sp.clone())
            .then_ignore(just(Token::Colon))
            .then(ident.clone())
            .map_with(
                |((default_asset, (name, name_span)), ty_name), e| Stmt::TextureBinding {
                    name,
                    name_span,
                    ty_name,
                    default_asset,
                    span: e.span(),
                },
            )
            .boxed();

        let param_stmt = just(Token::Param)
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::Colon))
            .then(param_ty)
            .then_ignore(just(Token::Eq))
            .then(expr.clone())
            .then(
                just(Token::In)
                    .ignore_then(
                        expr.clone()
                            .try_map(|range_expr, span| match range_expr.node {
                                Expr::Range(min, max) => Ok((*min, *max)),
                                _ => Err(Rich::custom(span, "expected range `min .. max`")),
                            }),
                    )
                    .or_not(),
            )
            .map_with(
                |((((name, name_span), ty_name), default), range), e| Stmt::Param {
                    name,
                    name_span,
                    ty_name,
                    default,
                    range,
                    span: e.span(),
                },
            )
            .boxed();

        let blend_pipe_suffix = (nl
            .clone()
            .ignore_then(just(Token::PipeOp))
            .or(just(Token::PipeOp)))
        .ignore_then(ident_sp.clone().then(args_outer.clone()))
        .try_map(|((name, name_span), args), _| {
            if name != "blend" {
                return Err(Rich::custom(
                    name_span,
                    "compose entry pipe suffix only supports `blend(...)`",
                ));
            }
            parse_blend_mode_from_args(&args, &name_span)
                .map_err(|(span, msg)| Rich::custom(span, msg))
        })
        .or_not()
        .boxed();

        let entry_expr = expr
            .clone()
            .try_map(|e, _| {
                let (expr, blend_pipe_mode) =
                    extract_compose_pipe_blend(e).map_err(|(span, msg)| Rich::custom(span, msg))?;
                Ok(ComposeEntry::Expr {
                    expr,
                    blend: blend_pipe_mode,
                })
            })
            .boxed();

        let entry_block = block
            .clone()
            .then(blend_pipe_suffix.clone())
            .map_with(|(body, blend_pipe_mode), e| ComposeEntry::Block {
                body,
                blend: blend_pipe_mode,
                span: e.span(),
            })
            .boxed();

        let entry_in_space = just(Token::In)
            .ignore_then(just(Token::Space))
            .ignore_then(space_chain.clone())
            .then(block.clone())
            .then(blend_pipe_suffix.clone())
            .map_with(
                |((chain, body), blend_pipe_mode), e| ComposeEntry::InSpace {
                    chain,
                    body,
                    blend: blend_pipe_mode,
                    span: e.span(),
                },
            )
            .boxed();

        let entry_for = just(Token::For)
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(block.clone())
            .then(blend_pipe_suffix.clone())
            .map_with(
                |((((name, _name_span), iterable), body), blend_pipe_mode), e| ComposeEntry::For {
                    name,
                    iterable,
                    body,
                    blend: blend_pipe_mode,
                    index_name: None,
                    span: e.span(),
                },
            )
            .boxed();

        let entry_c_for = just(Token::For)
            .ignore_then(just(Token::LParen))
            .ignore_then(
                select! { Token::Ident(s) if s == "int" || s == "i32" || s == "u32" => () }
                    .or_not(),
            )
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::Eq))
            .then(expr.clone())
            .then_ignore(just(Token::Semicolon))
            .then(ident_sp.clone())
            .then(
                just(Token::Lt)
                    .to(BinOp::Lt)
                    .or(just(Token::Le).to(BinOp::Le))
                    .or(just(Token::Gt).to(BinOp::Gt))
                    .or(just(Token::Ge).to(BinOp::Ge)),
            )
            .then(expr.clone())
            .then(just(Token::AndAnd).ignore_then(expr.clone()).or_not())
            .then_ignore(just(Token::Semicolon))
            .then(ident_sp.clone())
            .then(just(Token::PlusPlus).to(true).or(just(Token::MinusMinus).to(false)))
            .then_ignore(just(Token::RParen))
            .then(block.clone())
            .then(blend_pipe_suffix.clone())
            .map_with(|payload, e| (payload, e.span()))
            .try_map(
                |(payload, span), _| {
                    let ((((((((((name, name_span), start), (cond_name, cond_name_span)), cmp), end), extra_cond), (inc_name, inc_name_span)), incrementing), body), blend_pipe_mode) = payload;

                    if cond_name != name {
                        return Err(Rich::custom(
                            cond_name_span,
                            "C-style for condition must compare the loop variable",
                        ));
                    }
                    if inc_name != name {
                        return Err(Rich::custom(
                            inc_name_span,
                            "C-style for increment must use `i++` or `i--` on the same loop variable",
                        ));
                    }
                    if extra_cond.is_some() {
                        return Err(Rich::custom(
                            span,
                            "C-style `for` with `&&` condition is not yet supported",
                        ));
                    }

                    let num = |v: f64| sp(Expr::Num(v, Unit::None), span.clone());
                    let plus_one = |e: SExpr| binx(BinOp::Add, e, num(1.0));
                    let minus_expr = |a: SExpr, b: SExpr| binx(BinOp::Sub, a, b);

                    if incrementing {
                        let end_expr = match cmp {
                            BinOp::Lt => end,
                            BinOp::Le => plus_one(end),
                            BinOp::Gt | BinOp::Ge => {
                                return Err(Rich::custom(
                                    span,
                                    "`i++` loop requires `<` or `<=` condition",
                                ));
                            }
                            _ => unreachable!(),
                        };

                        let iterable_span = start.span.start..end_expr.span.end;
                        let iterable =
                            sp(Expr::Range(Box::new(start), Box::new(end_expr)), iterable_span);

                        return Ok(ComposeEntry::For {
                            name,
                            iterable,
                            body,
                            blend: blend_pipe_mode,
                            index_name: None,
                            span,
                        });
                    }

                    let lower = match cmp {
                        BinOp::Gt => plus_one(end),
                        BinOp::Ge => end,
                        BinOp::Lt | BinOp::Le => {
                            return Err(Rich::custom(
                                span,
                                "`i--` loop requires `>` or `>=` condition",
                            ));
                        }
                        _ => unreachable!(),
                    };
                    let upper = plus_one(start.clone());
                    let idx_name = format!("__cfor_{}", span.start);
                    let idx_var = sp(Expr::Var(idx_name.clone()), span.clone());
                    let lowered_idx = minus_expr(idx_var, lower.clone());
                    let user_value = minus_expr(start, lowered_idx);

                    let mut lowered_body = Vec::with_capacity(body.len() + 1);
                    lowered_body.push(Stmt::Let {
                        mutable: false,
                        name,
                        name_span,
                        declared_ty_name: None,
                        declared_ty_span: None,
                        value: user_value,
                    });
                    lowered_body.extend(body);

                    let iterable_span = lower.span.start..upper.span.end;
                    let iterable = sp(Expr::Range(Box::new(lower), Box::new(upper)), iterable_span);

                    Ok(ComposeEntry::For {
                        name: idx_name,
                        iterable,
                        body: lowered_body,
                        blend: blend_pipe_mode,
                        index_name: None,
                        span,
                    })
                },
            )
            .boxed();

        let entry_if = recursive(|entry_if| {
            let else_clause = nl
                .clone()
                .ignore_then(just(Token::Else))
                .ignore_then(
                    entry_if
                        .clone()
                        .map(|nested| {
                            let span = compose_entry_span(&nested);
                            vec![Stmt::Compose {
                                entries: vec![nested],
                                span,
                            }]
                        })
                        .or(block.clone()),
                )
                .or_not()
                .boxed();

            just(Token::If)
                .ignore_then(expr.clone())
                .then(block.clone())
                .then(else_clause)
                .then(blend_pipe_suffix.clone())
                .map_with(
                    |(((cond, then_body), else_body), blend_pipe_mode), e| ComposeEntry::If {
                        cond,
                        then_body,
                        else_body,
                        blend: blend_pipe_mode,
                        span: e.span(),
                    },
                )
                .boxed()
        })
        .boxed();

        let entry_each = just(Token::Each)
            .ignore_then(each_binding.clone())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then(block.clone())
            .then(blend_pipe_suffix.clone())
            .map_with(
                |(((((val_name, _val_name_span), idx_binding), iterable), body), blend_pipe_mode), e| {
                    ComposeEntry::For {
                        name: val_name,
                        iterable,
                        body,
                        blend: blend_pipe_mode,
                        index_name: idx_binding,
                        span: e.span(),
                    }
                },
            )
            .boxed();

        let entry = choice((entry_in_space, entry_c_for, entry_for, entry_each, entry_if, entry_block, entry_expr)).boxed();

        let compose_entries = just(Token::Compose)
            .ignore_then(
                entry
                    .separated_by(sep.clone())
                    .allow_leading()
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(
                        nl.clone().ignore_then(just(Token::LBrace)).then_ignore(nl.clone()),
                        nl.clone().ignore_then(just(Token::RBrace)),
                    ),
            )
            .boxed();

        let compose = compose_entries
            .then(pipe_call.repeated().collect::<Vec<_>>())
            .map_with(|(entries, pipes), e| {
                if pipes.is_empty() {
                    Stmt::Compose {
                        entries,
                        span: e.span(),
                    }
                } else {
                    Stmt::ComposePiped {
                        entries,
                        pipes,
                        span: e.span(),
                    }
                }
            })
            .labelled("compose block")
            .boxed();

        let vertex_field = ident_sp.clone().then_ignore(just(Token::Colon)).then(expr.clone());
        let vertex_stmt = just(Token::Vertex)
            .ignore_then(vertex_field.separated_by(sep.clone()).allow_leading().allow_trailing().collect::<Vec<_>>()
                .delimited_by(nl.clone().ignore_then(just(Token::LBrace)).then_ignore(nl.clone()), nl.clone().ignore_then(just(Token::RBrace))))
            .try_map(|fields, span| {
                let mut seen = std::collections::HashSet::new();
                if fields.is_empty() { return Err(Rich::custom(span, "vertex block must define at least one context field")); }
                let mut outputs = Vec::new();
                for ((name, name_span), value) in fields {
                    if !seen.insert(name.clone()) { return Err(Rich::custom(name_span, format!("duplicate `{name}:` in vertex block"))); }
                    outputs.push((crate::ast::Spanned { node: name, span: name_span }, value));
                }
                Ok(Stmt::SurfaceVertex { fields: outputs, span })
            }).boxed();

        let return_value = if preserve_casts {
            expr.clone().or_not().boxed()
        } else {
            expr.clone().map(Some).boxed()
        };
        let return_stmt = just(Token::Return)
            .ignore_then(return_value)
            .map_with(|value, e| match value {
                Some(value) => Stmt::Return { value, span: e.span() },
                None => Stmt::ReturnVoid { span: e.span() },
            })
            .boxed();

        let break_stmt = just(Token::Break)
            .map_with(|_, e| Stmt::Break { span: e.span() })
            .boxed();

        let in_context = just(Token::In)
            .ignore_then(just(Token::Ident("context".into())))
            .ignore_then(expr.clone())
            .then(block.clone())
            .map_with(|(value, body), e| Stmt::InContext { value, body, span: e.span() })
            .boxed();

        let in_space = just(Token::In)
            .ignore_then(just(Token::Space))
            .ignore_then(space_chain)
            .then(block.clone())
            .map_with(|(chain, body), e| Stmt::InSpace {
                chain,
                body,
                span: e.span(),
            })
            .labelled("in-space block")
            .boxed();

        let grouped_block = block
            .clone()
            .map_with(|body, e| Stmt::Block {
                body,
                span: e.span(),
            })
            .boxed();

        let match_arm_label = ident_sp
            .clone()
            .then(
                just(Token::Dot)
                    .ignore_then(ident_sp.clone())
                    .or_not(),
            )
            .map_with(|((head, head_span), tail), e| match tail {
                Some((variant_name, variant_span)) => crate::ast::MatchArm {
                    enum_name: Some(head),
                    enum_name_span: Some(head_span),
                    variant_name,
                    variant_span,
                    body: Vec::new(),
                    span: e.span(),
                },
                None => crate::ast::MatchArm {
                    enum_name: None,
                    enum_name_span: None,
                    variant_name: head,
                    variant_span: head_span,
                    body: Vec::new(),
                    span: e.span(),
                },
            })
            .boxed();

        enum MatchItem {
            Arm(crate::ast::MatchArm),
            Default(Vec<Stmt>, Span),
        }

        let match_arm_body = block
            .clone()
            .or(stmt.clone().map(|s| vec![s]))
            .boxed();

        let match_arm_item = match_arm_label
            .clone()
            .then_ignore(just(Token::Colon))
            .then(match_arm_body.clone())
            .map_with(|(mut arm, body), e| {
                arm.body = body;
                arm.span = e.span();
                MatchItem::Arm(arm)
            })
            .boxed();

        let match_default_item = select! { Token::Ident(s) if s == "default" => () }
            .then_ignore(just(Token::Colon))
            .then(match_arm_body.clone())
            .map_with(|(_, body), e| MatchItem::Default(body, e.span()))
            .boxed();

        let match_stmt = select! { Token::Ident(s) if s == "unroll" => () }
            .or_not()
            .ignore_then(select! { Token::Ident(s) if s == "match" => () })
            .ignore_then(expr.clone())
            .then(
                choice((match_default_item, match_arm_item))
                    .separated_by(sep.clone())
                    .allow_leading()
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(
                        just(Token::LBrace).then_ignore(nl.clone()),
                        nl.clone().ignore_then(just(Token::RBrace)),
                    ),
            )
            .try_map(|(value, items), span| {
                let mut arms = Vec::new();
                let mut default_body = None;
                let mut default_span = None;

                for item in items {
                    match item {
                        MatchItem::Arm(arm) => arms.push(arm),
                        MatchItem::Default(body, dspan) => {
                            if default_body.is_some() {
                                return Err(Rich::custom(dspan, "duplicate `default` match arm"));
                            }
                            default_body = Some(body);
                            default_span = Some(dspan);
                        }
                    }
                }

                if arms.is_empty() && default_body.is_none() {
                    return Err(Rich::custom(span, "`match` requires at least one arm"));
                }

                Ok(Stmt::Match {
                    value,
                    arms,
                    default_body,
                    default_span,
                    span,
                })
            })
            .boxed();

        let expr_stmt = expr.clone().map(Stmt::Expr).boxed();

        // Local fn declaration: `fn name(params) -> ty { body }`
        // Parsed as a statement so helpers can be defined inside canvas/fn bodies.
        let local_fn_param = select! { Token::Attribute(name) if name == "context" => () }
            .or_not()
            .then(ident_sp
            .clone()
            .then_ignore(just(Token::Colon))
            .then(fn_param_type_sp())
            .then(just(Token::Eq).ignore_then(expr.clone()).or_not()))
            .map(
                |(context, (((name, name_span), (ty_name, ty_span)), default))| FnParam {
                    is_context: context.is_some(),
                    name,
                    name_span,
                    ty_name,
                    ty_span,
                    keyword_only: false,
                    default,
                },
            )
            .boxed();

        enum LocalFnParamItem {
            Marker(Span),
            Param(FnParam),
        }

        let local_fn_param_item = choice((
            just(Token::Star).map_with(|_, e| LocalFnParamItem::Marker(e.span())),
            local_fn_param.clone().map(LocalFnParamItem::Param),
        ))
        .boxed();

        let local_fn_params = local_fn_param_item
            .clone()
            .then(
                comma_sep
                    .clone()
                    .ignore_then(local_fn_param_item.clone())
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
                        LocalFnParamItem::Marker(star_span) => {
                            if saw_keyword_separator {
                                return Err(Rich::custom(
                                    star_span,
                                    "duplicate `*` keyword-only separator in parameter list",
                                ));
                            }
                            saw_keyword_separator = true;
                            separator_span = Some(star_span);
                        }
                        LocalFnParamItem::Param(mut param) => {
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
        let local_fn_ret_ty = just(Token::Arrow)
            .ignore_then(fn_param_type_sp())
            .or_not()
            .boxed();
        let local_fn_decl = just(Token::Fn)
            .ignore_then(ident_sp.clone())
            .then(local_fn_params)
            .then(local_fn_ret_ty)
            .then(nl.clone().ignore_then(block.clone()))
            .map_with(|((((name, name_span), params), ret_ty), body), e| {
                Stmt::LocalFnDecl(FnDecl {
                    typed_body: None,
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
                })
            })
            .boxed();

        choice((
            texture_binding_stmt,
            param_stmt,
            canvas_space_decl,
            space_decl,
            style_decl,
            let_scatter_stmt,
            const_stmt,
            typed_let_stmt,
            store_stmt,
            assign_stmt,
            if_stmt,
            c_for_stmt,
            for_stmt,
            each_stmt,
            let_stmt,
            return_stmt,
            break_stmt,
            in_space,
            in_context,
            local_fn_decl,
            grouped_block,
            match_stmt,
            compose,
            vertex_stmt,
            expr_stmt,
        ))
    })
    .boxed()
}

pub(super) fn block_parser<'src>() -> PBox<'src, Vec<Stmt>> {
    block_parser_with_casts(false)
}

pub(super) fn block_parser_with_casts<'src>(preserve_casts: bool) -> PBox<'src, Vec<Stmt>> {
    let stmt = stmt_parser_with_casts(preserve_casts);
    let block_stmt = stmt.clone().filter(|stmt| {
        matches!(
            stmt,
            Stmt::If { .. }
                | Stmt::For { .. }
                | Stmt::Match { .. }
                | Stmt::Block { .. }
                | Stmt::InSpace { .. }
                | Stmt::InContext { .. }
                | Stmt::Compose { .. }
                | Stmt::LocalFnDecl(_)
        )
    });
    sep()
        .repeated()
        .ignore_then(
            block_stmt
                .then_ignore(sep().or_not())
                .or(stmt.then_ignore(sep().or(just(Token::RBrace).rewind().ignored())))
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(sep().repeated())
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
        .boxed()
}
