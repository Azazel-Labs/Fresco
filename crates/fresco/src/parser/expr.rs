use super::*;
use crate::lexer::Unit;

pub(super) fn expr_parser<'src>() -> PBox<'src, SExpr> {
    expr_parser_with_casts(false)
}

pub(super) fn expr_parser_with_casts<'src>(preserve_casts: bool) -> PBox<'src, SExpr> {
    let nl = nl();
    let comma_sep = comma_sep();
    let ident_sp = ident_sp();

    recursive(move |expr| {
        let ternary_desugar = |cond: SExpr, then_expr: SExpr, else_expr: SExpr| {
            let span = cond.span.start..else_expr.span.end;
            sp(Expr::Call { name: "select".into(), name_span: span.clone(), const_args: Vec::new(), args: vec![
                Arg { name: None, value: else_expr }, Arg { name: None, value: then_expr }, Arg { name: None, value: cond },
            ] }, span)
        };
        let logical_and_desugar = |l, r| binx(BinOp::LogicalAnd, l, r);
        let logical_or_desugar = |l, r| binx(BinOp::LogicalOr, l, r);

        let arg = argument_name()
            .then_ignore(just(Token::Colon))
            .or_not()
            .then(expr.clone())
            .map(|(name, value)| Arg { name, value })
            .boxed();

        let args = arg
            .separated_by(comma_sep.clone())
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(
                just(Token::LParen).then_ignore(nl.clone()),
                nl.clone().ignore_then(just(Token::RParen)),
            )
            .labelled("argument list")
            .boxed();

        let const_template_arg = choice((
            just(Token::Minus)
                .ignore_then(select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) })
                .map_with(|v, e| crate::ast::ConstTemplateArg {
                    value: sp(Expr::Num(-v, Unit::None), e.span()),
                    span: e.span(),
                })
                .boxed(),
            select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) }
                .map_with(|v, e| crate::ast::ConstTemplateArg {
                    value: sp(Expr::Num(v, Unit::None), e.span()),
                    span: e.span(),
                })
                .boxed(),
            ident_sp
                .clone()
                .map_with(|(name, span), e| crate::ast::ConstTemplateArg {
                    value: sp(Expr::Var(name), span),
                    span: e.span(),
                })
                .boxed(),
            select! { Token::Ident(s) if s == "true" || s == "false" => s.to_string() }
                .map_with(|v, e| crate::ast::ConstTemplateArg {
                    value: sp(Expr::Var(v), e.span()),
                    span: e.span(),
                })
                .boxed(),
        ))
        .boxed();

        let const_template_args = const_template_arg
            .separated_by(comma_sep.clone())
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::Lt), just(Token::Gt))
            .boxed();

        let call_suffix = const_template_args
            .clone()
            .then(args.clone())
            .or(args.clone().map(|args| (Vec::new(), args)))
            .boxed();

        // Parse ident-leading expressions while preserving dotted names
        // (`Anchor.center`) and supporting method-style sugar
        // (`curve.point_at(s)`) lowered as a pipe call.
        let call_or_var = ident_sp
            .clone()
            .then(
                just(Token::Dot)
                    .ignore_then(ident_sp.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then(call_suffix.or_not())
            .map_with(move |(((name, name_span), tail), call_suffix), e| {
                if let Some((const_args, args)) = call_suffix {
                    if tail.is_empty() {
                        let all_positional = args.iter().all(|a| a.name.is_none());
                        if all_positional && !preserve_casts {
                            let mut vals = args.iter().map(|a| a.value.clone()).collect::<Vec<_>>();
                            match (name.as_str(), vals.len()) {
                                ("vec2", 2) => {
                                    let b = vals.pop().expect("vec2 arity checked");
                                    let a = vals.pop().expect("vec2 arity checked");
                                    return sp(Expr::Vec2(Box::new(a), Box::new(b)), e.span());
                                }
                                ("vec2", 1) => {
                                    let a = vals.pop().expect("vec2 arity checked");
                                    return sp(
                                        Expr::Vec2(Box::new(a.clone()), Box::new(a)),
                                        e.span(),
                                    );
                                }
                                ("vec3", 3) => {
                                    let c = vals.pop().expect("vec3 arity checked");
                                    let b = vals.pop().expect("vec3 arity checked");
                                    let a = vals.pop().expect("vec3 arity checked");
                                    return sp(
                                        Expr::Vec3(Box::new(a), Box::new(b), Box::new(c)),
                                        e.span(),
                                    );
                                }
                                ("vec3", 2) => {
                                    let c = vals.pop().expect("vec3 arity checked");
                                    let ab = vals.pop().expect("vec3 arity checked");
                                    let ax = sp(
                                        Expr::Member(Box::new(ab.clone()), "x".to_string()),
                                        e.span(),
                                    );
                                    let ay =
                                        sp(Expr::Member(Box::new(ab), "y".to_string()), e.span());
                                    return sp(
                                        Expr::Vec3(Box::new(ax), Box::new(ay), Box::new(c)),
                                        e.span(),
                                    );
                                }
                                ("vec3", 1) => {
                                    let a = vals.pop().expect("vec3 arity checked");
                                    return sp(
                                        Expr::Vec3(
                                            Box::new(a.clone()),
                                            Box::new(a.clone()),
                                            Box::new(a),
                                        ),
                                        e.span(),
                                    );
                                }
                                ("vec4", 4) => {
                                    let d = vals.pop().expect("vec4 arity checked");
                                    let c = vals.pop().expect("vec4 arity checked");
                                    let b = vals.pop().expect("vec4 arity checked");
                                    let a = vals.pop().expect("vec4 arity checked");
                                    return sp(
                                        Expr::Vec4(
                                            Box::new(a),
                                            Box::new(b),
                                            Box::new(c),
                                            Box::new(d),
                                        ),
                                        e.span(),
                                    );
                                }
                                ("vec4", 1) => {
                                    let a = vals.pop().expect("vec4 arity checked");
                                    return sp(
                                        Expr::Vec4(
                                            Box::new(a.clone()),
                                            Box::new(a.clone()),
                                            Box::new(a.clone()),
                                            Box::new(a),
                                        ),
                                        e.span(),
                                    );
                                }
                                _ => {}
                            }
                        }

                        return sp(
                            Expr::Call {
                                name,
                                name_span,
                                const_args,
                                args,
                            },
                            e.span(),
                        );
                    }

                    let mut recv_name = name;
                    let mut recv_end = name_span.end;
                    for (part, span) in &tail[..tail.len() - 1] {
                        recv_name.push('.');
                        recv_name.push_str(part);
                        recv_end = span.end;
                    }
                    let recv = sp(Expr::Var(recv_name), name_span.start..recv_end);
                    let (method_name, method_span) = tail
                        .last()
                        .cloned()
                        .expect("tail is non-empty for method-style calls");
                    if method_name == "at" {
                        // Texture helper sugar: `tex.at(uv)` -> `tex_at(tex, uv)`.
                        // This keeps typed channel chaining ergonomic:
                        // `orm.at(uv2).roughness`.
                        let mut helper_args = Vec::with_capacity(args.len() + 1);
                        helper_args.push(Arg {
                            name: None,
                            value: recv,
                        });
                        helper_args.extend(args);
                        return sp(
                            Expr::Call {
                                name: "tex_at".to_string(),
                                name_span: method_span,
                                const_args: Vec::new(),
                                args: helper_args,
                            },
                            e.span(),
                        );
                    }
                    return sp(
                        Expr::Pipe {
                            recv: Box::new(recv),
                            name: method_name,
                            name_span: method_span,
                            args,
                        },
                        e.span(),
                    );
                }

                {
                    let mut full_name = name;
                    let mut end = name_span.end;
                    for (part, span) in tail {
                        full_name.push('.');
                        full_name.push_str(&part);
                        end = span.end;
                    }
                    sp(Expr::Var(full_name), name_span.start..end)
                }
            })
            .boxed();

        let num = select! { Token::Num((bits, unit)) => (f64::from_bits(bits), unit) }
            .map_with(|(v, u), e| sp(Expr::Num(v, u), e.span()))
            .or(select! { Token::TypedNum((bits, unsigned)) => (f64::from_bits(bits), unsigned) }
                .map_with(move |(value, unsigned), e| {
                    let span: Span = e.span();
                    let number = sp(Expr::Num(value, Unit::None), span.clone());
                    if preserve_casts {
                        sp(Expr::Call {
                            name: if unsigned { "u32" } else { "i32" }.into(), name_span: span.clone(), const_args: vec![],
                            args: vec![Arg { name: None, value: number }],
                        }, span)
                    } else { number }
                }))
            .or(select! { Token::Rate((bits, unit, milliseconds)) => (bits, unit, milliseconds) }
                .map_with(|(bits, unit, milliseconds), e| {
                    let span: Span = e.span();
                    let value = f64::from_bits(bits) * if milliseconds { 1000.0 } else { 1.0 };
                    let clock = sp(Expr::Call {
                        name: "context".to_string(),
                        name_span: span.clone(),
                        const_args: Vec::new(),
                        args: vec![Arg {
                            name: None,
                            value: sp(Expr::Var("time".to_string()), span.clone()),
                        }],
                    }, span.clone());
                    // Rate literals use the same scoped clock and unit conversion as
                    // explicitly multiplying a quantity by context(time).
                    binx(BinOp::Mul, sp(Expr::Num(value, unit), span), clock)
                }))
            .labelled("number")
            .boxed();

        let color = select! { Token::Color(c) => c }
            .map_with(|c, e| {
                let f = |b: u32| (b & 0xff) as f32 / 255.0;
                sp(
                    Expr::Color([f(c >> 24), f(c >> 16), f(c >> 8), f(c)]),
                    e.span(),
                )
            })
            .labelled("color literal")
            .boxed();

        let string_lit = select! { Token::Str(s) => s }
            .map_with(|s, e| sp(Expr::Str(s), e.span()))
            .labelled("string literal")
            .boxed();

        let paren = expr
            .clone()
            .separated_by(comma_sep.clone())
            .at_least(1)
            .at_most(4)
            .collect::<Vec<_>>()
            .delimited_by(
                just(Token::LParen).then_ignore(nl.clone()),
                nl.clone().ignore_then(just(Token::RParen)),
            )
            .try_map(|items, span| match items.as_slice() {
                [a] => Ok(a.clone()),
                [a, b] => Ok(sp(
                    Expr::Vec2(Box::new(a.clone()), Box::new(b.clone())),
                    span,
                )),
                [a, b, c] => Ok(sp(
                    Expr::Vec3(
                        Box::new(a.clone()),
                        Box::new(b.clone()),
                        Box::new(c.clone()),
                    ),
                    span,
                )),
                [a, b, c, d] => Ok(sp(
                    Expr::Vec4(
                        Box::new(a.clone()),
                        Box::new(b.clone()),
                        Box::new(c.clone()),
                        Box::new(d.clone()),
                    ),
                    span,
                )),
                _ => Err(Rich::custom(
                    span,
                    "tuples support 1, 2, 3, or 4 values in expression position",
                )),
            })
            .boxed();

        let array_comp = just(Token::For)
            .ignore_then(ident_sp.clone())
            .then_ignore(just(Token::In))
            .then(expr.clone())
            .then_ignore(just(Token::FatArrow))
            .then(expr.clone())
            .delimited_by(
                just(Token::LBracket).then_ignore(nl.clone()),
                nl.clone().ignore_then(just(Token::RBracket)),
            )
            .map_with(|(((name, _name_span), iterable), body), e| {
                sp(
                    Expr::ArrayComp {
                        name,
                        iterable: Box::new(iterable),
                        body: Box::new(body),
                    },
                    e.span(),
                )
            })
            .boxed();

        let array = array_comp
            .or(expr
                .clone()
                .separated_by(comma_sep.clone())
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LBracket).then_ignore(nl.clone()),
                    nl.clone().ignore_then(just(Token::RBracket)),
                )
                .map_with(|items, e| sp(Expr::Array(items), e.span())))
            .boxed();

        let kw = |s: &'static str| {
            select! { Token::Ident(name) if name.as_str() == s => () }
                .labelled(s)
                .boxed()
        };

        let path_move = kw("move")
            .ignore_then(expr.clone())
            .map_with(|to, e| crate::ast::PathCommand {
                kind: crate::ast::PathCommandKind::Move { to },
                span: e.span(),
            })
            .boxed();

        let path_line = kw("line")
            .ignore_then(expr.clone())
            .map_with(|to, e| crate::ast::PathCommand {
                kind: crate::ast::PathCommandKind::Line { to },
                span: e.span(),
            })
            .boxed();

        let path_cubic = kw("cubic")
            .ignore_then(expr.clone())
            .then(expr.clone())
            .then(expr.clone())
            .map_with(|((c1, c2), to), e| crate::ast::PathCommand {
                kind: crate::ast::PathCommandKind::Cubic { c1, c2, to },
                span: e.span(),
            })
            .boxed();

        let path_arc = kw("arc")
            .ignore_then(nl.clone())
            .ignore_then(kw("center"))
            .ignore_then(just(Token::Colon))
            .ignore_then(nl.clone())
            .ignore_then(expr.clone())
            .then_ignore(nl.clone())
            .then_ignore(kw("radius"))
            .then_ignore(just(Token::Colon))
            .then_ignore(nl.clone())
            .then(expr.clone())
            .then_ignore(nl.clone())
            .then_ignore(kw("sweep"))
            .then_ignore(just(Token::Colon))
            .then_ignore(nl.clone())
            .then(expr.clone())
            .map_with(|((center, radius), sweep), e| crate::ast::PathCommand {
                kind: crate::ast::PathCommandKind::Arc {
                    center,
                    radius,
                    sweep,
                },
                span: e.span(),
            })
            .boxed();

        let path_command = choice((path_move, path_line, path_cubic, path_arc)).boxed();

        let path_future = select! { Token::Ident(name) if name.as_str() == "path" => () }
            .ignore_then(
                nl.clone()
                    .ignore_then(path_command)
                    .then_ignore(nl.clone())
                    .repeated()
                    .collect::<Vec<_>>()
                    .delimited_by(
                        just(Token::LBrace).then_ignore(nl.clone()),
                        nl.clone().ignore_then(just(Token::RBrace)),
                    ),
            )
            .map_with(|commands, e| sp(Expr::PathFuture { commands }, e.span()))
            .boxed();

        let lambda_body = choice((
            just(Token::LBrace)
                .ignore_then(nl.clone())
                .ignore_then(just(Token::Return))
                .ignore_then(expr.clone())
                .then_ignore(nl.clone())
                .then_ignore(just(Token::RBrace))
                .map(|ret| LambdaBody::BlockReturn(Box::new(ret))),
            expr.clone().map(|e| LambdaBody::Expr(Box::new(e))),
        ))
        .boxed();

        let lambda = just(Token::Bar)
            .ignore_then(
                ident_sp
                    .clone()
                    .map(|(name, span)| AstSpanned { node: name, span })
                    .separated_by(comma_sep.clone())
                    .at_least(1)
                    .allow_trailing()
                    .collect::<Vec<_>>(),
            )
            .then_ignore(just(Token::Bar))
            .then(lambda_body)
            .map_with(|(params, body), e| sp(Expr::Lambda { params, body }, e.span()))
            .boxed();

        let primary_atom = choice((
            lambda,
            path_future,
            call_or_var,
            num,
            color,
            string_lit,
            paren,
            array,
        ))
        .boxed();

        let primary = recursive(|primary| {
            let field_expr = just(Token::Field)
                .ignore_then(primary.clone())
                .then(just(Token::At).ignore_then(expr.clone()).or_not())
                .map_with(|(inner, coord_opt), e| match coord_opt {
                    None => sp(Expr::Field(Box::new(inner)), e.span()),
                    Some(coord) => sp(
                        Expr::FieldAt {
                            inner: Box::new(inner),
                            coord: Box::new(coord),
                        },
                        e.span(),
                    ),
                })
                .boxed();

            let layer_expr = just(Token::Layer)
                .ignore_then(primary.clone())
                .map_with(|inner, e| sp(Expr::Layer(Box::new(inner)), e.span()))
                .boxed();

            choice((field_expr, layer_expr, primary_atom.clone())).boxed()
        })
        .boxed();

        let unary = just(Token::Minus)
            .repeated()
            .collect::<Vec<_>>()
            .then(primary)
            .map(|(ops, e)| {
                ops.into_iter().rev().fold(e, |e, _m| {
                    let span = e.span.clone();
                    sp(Expr::Unary(UnOp::Neg, Box::new(e)), span)
                })
            })
            .boxed();

        // A postfix step is either `.field` member access or `[index]` subscript access.
        enum AccessStep {
            Member(String, Span),
            Index(SExpr, Span),
            Method(String, Span, Vec<Arg>, Span),
        }

        let postfix_method = nl.clone().ignore_then(just(Token::Dot)).then_ignore(nl.clone())
            .ignore_then(ident_sp.clone()).then(args.clone())
            .map_with(|((name, name_span), args), e| AccessStep::Method(name, name_span, args, e.span()))
            .boxed();

        let postfix_member = just(Token::Dot)
            .ignore_then(ident_sp.clone().or(just(Token::Vertex)
                .map_with(|_, e| ("vertex".to_string(), e.span()))))
            .map(|(field, field_span): (String, Span)| AccessStep::Member(field, field_span))
            .boxed();

        let postfix_index = expr
            .clone()
            .delimited_by(
                just(Token::LBracket).then_ignore(nl.clone()),
                nl.clone().ignore_then(just(Token::RBracket)),
            )
            .map_with(|index_expr, e| AccessStep::Index(index_expr, e.span()))
            .boxed();

        let postfix = unary
            .clone()
            .then(
                postfix_method
                    .or(postfix_member)
                    .or(postfix_index)
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(base, steps)| {
                steps.into_iter().fold(base, |recv, step| match step {
                    AccessStep::Method(name, name_span, mut args, step_span) => {
                        let span = recv.span.start..step_span.end;
                        if name == "at" {
                            args.insert(0, Arg { name: None, value: recv });
                            sp(Expr::Call { name: "tex_at".into(), name_span, const_args: Vec::new(), args }, span)
                        } else {
                            sp(Expr::Pipe { recv: Box::new(recv), name, name_span, args }, span)
                        }
                    }
                    AccessStep::Member(field, field_span) => {
                        let span = recv.span.start..field_span.end;
                        sp(Expr::Member(Box::new(recv), field), span)
                    }
                    AccessStep::Index(index_expr, step_span) => {
                        let span = recv.span.start..step_span.end;
                        sp(
                            Expr::Index {
                                array: Box::new(recv),
                                index: Box::new(index_expr),
                            },
                            span,
                        )
                    }
                })
            })
            .boxed();

        // `expr at coord` — binary-operator form of field resampling.
        //
        // `luma at cell.center` is equivalent to `field luma at cell.center` but
        // allows the left-hand side to be any bound field value, not only an inline
        // `field …` expression.  The checker handles both forms via `Expr::FieldAt`.
        //
        // Precedence: tighter than `+`/`-` (its left operand is a postfix chain)
        // and equal to `field … at …` which is already resolved at the primary level.
        // The right-hand coord is a full expression so that `at (coord + offset)` works
        // without extra parentheses when the coord involves arithmetic.
        // Logical negation has boolean equality's type contract and evaluates
        // its operand once. Parse it outside the complete member/index chain.
        let logical_unary = just(Token::Bang)
            .repeated().collect::<Vec<_>>().then(postfix)
            .map(|(ops, value)| ops.into_iter().rev().fold(value, |value, _| {
                let span = value.span.clone();
                binx(BinOp::Eq, value, sp(Expr::Var("false".into()), span))
            })).boxed();
        let at_expr = logical_unary
            .clone()
            .then(just(Token::At).ignore_then(expr.clone()).or_not())
            .map_with(|(base, coord_opt), e| match coord_opt {
                None => base,
                Some(coord) => sp(
                    Expr::FieldAt {
                        inner: Box::new(base),
                        coord: Box::new(coord),
                    },
                    e.span(),
                ),
            })
            .boxed();

        // Staged parse support for tagged-expression suffixes (`expr in world`).
        // The tag is currently syntax-only; semantic enforcement is deferred.
        let tagged_expr = at_expr
            .clone()
            .then(just(Token::In).ignore_then(ident_sp.clone()).or_not())
            .map_with(|(base, _tag), _e| base)
            .boxed();

        let mul_div_op = nl
            .clone()
            .ignore_then(
                just(Token::Star)
                    .to(BinOp::Mul)
                    .or(just(Token::Slash).to(BinOp::Div))
                    .or(just(Token::Percent).to(BinOp::Mod)),
            )
            .or(just(Token::Star)
                .to(BinOp::Mul)
                .or(just(Token::Slash).to(BinOp::Div))
                .or(just(Token::Percent).to(BinOp::Mod)))
            .then_ignore(nl.clone())
            .boxed();

        let product = tagged_expr
            .clone()
            .then(mul_div_op.then(tagged_expr).repeated().collect::<Vec<_>>())
            .try_map(|(l, ops), _span| {
                ops.into_iter().try_fold(l, |l, (op, r)| {
                    if matches!(op, BinOp::Div)
                        && matches!(r.node, Expr::Num(_, Unit::Sec | Unit::MilliSec)) {
                        return Err(Rich::custom(r.span, "literal time denominators are not division operands; use an attached `/s` or `/ms` suffix"));
                    }
                    Ok(binx(op, l, r))
                })
            })
            .boxed();

        let add_sub_op = nl
            .clone()
            .ignore_then(
                just(Token::Plus)
                    .to(BinOp::Add)
                    .or(just(Token::Minus).to(BinOp::Sub)),
            )
            .or(just(Token::Plus)
                .to(BinOp::Add)
                .or(just(Token::Minus).to(BinOp::Sub)))
            .then_ignore(nl.clone())
            .boxed();

        let sum = product
            .clone()
            .then(
                add_sub_op
                    .then(product.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(l, ops)| ops.into_iter().fold(l, |l, (op, r)| binx(op, l, r)))
            .boxed();

        let shift_op = nl
            .clone()
            .ignore_then(
                just(Token::Shl)
                    .to(BinOp::Shl)
                    .or(just(Token::Shr).to(BinOp::Shr)),
            )
            .or(just(Token::Shl)
                .to(BinOp::Shl)
                .or(just(Token::Shr).to(BinOp::Shr)))
            .then_ignore(nl.clone())
            .boxed();

        let shift = sum
            .clone()
            .then(shift_op.then(sum.clone()).repeated().collect::<Vec<_>>())
            .map(|(l, ops)| ops.into_iter().fold(l, |l, (op, r)| binx(op, l, r)))
            .boxed();

        let range = shift
            .clone()
            .then(just(Token::RangeOp).ignore_then(shift.clone()).or_not())
            .map(|(a, b)| match b {
                Some(b) => {
                    let span = a.span.start..b.span.end;
                    sp(Expr::Range(Box::new(a), Box::new(b)), span)
                }
                None => a,
            })
            .boxed();

        let cmp_op = just(Token::Le)
            .to(BinOp::Le)
            .or(just(Token::Ge).to(BinOp::Ge))
            .or(just(Token::EqEq).to(BinOp::Eq))
            .or(just(Token::NotEq).to(BinOp::Ne))
            .or(just(Token::Lt).to(BinOp::Lt))
            .or(just(Token::Gt).to(BinOp::Gt))
            .boxed();

        let cmp = range
            .clone()
            .then(cmp_op.then(range.clone()).or_not())
            .map(|(l, rest)| match rest {
                Some((op, r)) => binx(op, l, r),
                None => l,
            })
            .boxed();

        let shape_op = nl
            .clone()
            .ignore_then(
                just(Token::Bar)
                    .to(BinOp::Union)
                    .or(just(Token::Amp).to(BinOp::Intersect)),
            )
            .or(just(Token::Bar)
                .to(BinOp::Union)
                .or(just(Token::Amp).to(BinOp::Intersect)))
            .then_ignore(nl.clone())
            .boxed();

        let logical_and = cmp
            .clone()
            .then(
                just(Token::AndAnd)
                    .ignore_then(cmp.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(move |(l, rest)| rest.into_iter().fold(l, logical_and_desugar))
            .boxed();

        let logical_or = logical_and
            .clone()
            .then(
                just(Token::OrOr)
                    .ignore_then(logical_and.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(move |(l, rest)| rest.into_iter().fold(l, logical_or_desugar))
            .boxed();

        let ternary = logical_or
            .clone()
            .then(
                just(Token::Question)
                    .ignore_then(expr.clone())
                    .then_ignore(just(Token::Colon))
                    .then(expr.clone())
                    .or_not(),
            )
            .map(move |(cond, branch)| match branch {
                Some((then_expr, else_expr)) => ternary_desugar(cond, then_expr, else_expr),
                None => cond,
            })
            .boxed();

        let bit_xor_op = nl
            .clone()
            .ignore_then(just(Token::Caret).to(BinOp::BitXor))
            .or(just(Token::Caret).to(BinOp::BitXor))
            .then_ignore(nl.clone())
            .boxed();

        let bit_xor = ternary
            .clone()
            .then(
                bit_xor_op
                    .then(ternary.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map(|(l, ops)| ops.into_iter().fold(l, |l, (op, r)| binx(op, l, r)))
            .boxed();

        let shape = bit_xor
            .clone()
            .then(shape_op.then(bit_xor).repeated().collect::<Vec<_>>())
            .map(|(l, ops)| ops.into_iter().fold(l, |l, (op, r)| binx(op, l, r)))
            .boxed();

        let pipe_rhs = ident_sp
            .clone()
            .then(args.clone().or_not())
            .map_with(|(n, a), e| (n, a.unwrap_or_default(), e.span()))
            .boxed();

        let pipe_sep = nl
            .clone()
            .ignore_then(just(Token::PipeOp))
            .or(just(Token::PipeOp))
            .boxed();

        // Space chain parser for `through space <chain>`.
        // Uses the same `args` and `ident_sp` already in scope so that space
        // call arguments are parsed with the full expression grammar.
        let space_call_expr = ident_sp
            .clone()
            .then(args.clone())
            .map(|((name, name_span), args)| SpaceCall {
                name,
                name_span,
                args,
            })
            .boxed();
        let space_item_expr = space_call_expr
            .map(SpaceItem::Call)
            .or(ident_sp
                .clone()
                .map(|(name, name_span)| SpaceItem::Ref { name, name_span }))
            .boxed();
        let space_dot_sep_expr = nl
            .clone()
            .ignore_then(just(Token::Dot))
            .then_ignore(nl.clone())
            .boxed();
        let through_space_chain = just(Token::Through)
            .ignore_then(just(Token::Space))
            .ignore_then(
                space_item_expr
                    .separated_by(space_dot_sep_expr)
                    .at_least(1)
                    .collect::<Vec<_>>(),
            )
            .boxed();

        // A postfix step is either `|> name(args)` or `through space <chain>`.
        // Both are left-associative at the same precedence level.
        enum PostfixStep {
            Pipe {
                name: String,
                name_span: Span,
                args: Vec<Arg>,
                end_span: Span,
            },
            Through {
                chain: Vec<SpaceItem>,
                end_span: Span,
            },
        }

        let pipe_step = pipe_sep
            .ignore_then(pipe_rhs)
            .map(|((name, name_span), args, end_span)| PostfixStep::Pipe {
                name,
                name_span,
                args,
                end_span,
            })
            .boxed();
        let through_step = nl
            .clone()
            .ignore_then(through_space_chain.clone())
            .or(through_space_chain)
            .map_with(|chain, e| PostfixStep::Through {
                chain,
                end_span: e.span(),
            })
            .boxed();

        shape
            .then(pipe_step.or(through_step).repeated().collect::<Vec<_>>())
            .map(|(recv, steps)| {
                steps.into_iter().fold(recv, |recv, step| match step {
                    PostfixStep::Pipe {
                        name,
                        name_span,
                        args,
                        end_span,
                    } => {
                        let span = recv.span.start..end_span.end;
                        sp(
                            Expr::Pipe {
                                recv: Box::new(recv),
                                name,
                                name_span,
                                args,
                            },
                            span,
                        )
                    }
                    PostfixStep::Through { chain, end_span } => {
                        let span = recv.span.start..end_span.end;
                        sp(
                            Expr::Through {
                                layer: Box::new(recv),
                                chain,
                            },
                            span,
                        )
                    }
                })
            })
    })
    .boxed()
}
