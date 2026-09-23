//! Parser built on `chumsky` 0.13 over the logos token stream.
//!
//! Precedence, loosest to tightest:
//!   pipe `|>`  <  shape ops `|` `&`  <  `+` `-`  <  `*` `/`  <  `at`  <  unary `-`
//!   <  primary
//!
//! `at` (field resampling operator) is at its own level between `*`/`/` and
//! unary `-`, so `luma at cell.center` and `base * 2.0 at coord` both parse
//! with the expected grouping.
//!
//! `-` serves both numeric and shape subtraction; the type checker
//! disambiguates. Newlines are statement / compose-entry separators at the
//! top level, but are ignored inside parenthesized expression/argument lists.

mod axis;
mod expr;
mod stmt;
mod style_graph;
mod top;

use crate::ast::{
    Arg, BinOp, ComposeEntry, ConformanceDecl, EffectDecl, EnumDecl, EnumVariant,
    EvaluationPermutationAtom, EvaluationPermutationSpec, Expr, FnDecl, FnParam, ImportDecl,
    InterfaceDecl, InterfaceMethodDecl, LambdaBody, LocalityClass, MaterialChannelDecl,
    MaterialPropertiesDecl, PragmaDecl, PragmaValue, Program, RewriteComposition, RewritePattern,
    RewriteRule, SExpr, ScatterDecl, ScatterLifecycle, ScatterPipeCall,
    SchemaEvaluatorContractDecl, SchemaEvaluatorDecl, SchemaEvaluatorPermutationDecl,
    SchemaExpressionDecl, SpaceCall, SpaceItem, Span, Spanned as AstSpanned, Stmt, StructDecl,
    StructFieldDecl, StyleParam, StyleStage, TextureChannelDef, TextureTypeDecl, TypeParam, UnOp,
};
use crate::lexer::{Token, Unit};
use chumsky::{
    extra,
    input::{Input as _, MappedInput},
    prelude::*,
};

pub type PInput<'src> = MappedInput<'src, Token, Span, &'src [(Token, Span)]>;
pub type PError<'src> = Rich<'src, Token, Span>;
pub(super) type PExtra<'src> = extra::Err<PError<'src>>;
pub(super) type PBox<'src, T> = Boxed<'src, 'src, PInput<'src>, T, PExtra<'src>>;

pub fn input<'src>(tokens: &'src [(Token, Span)], eoi: Span) -> PInput<'src> {
    tokens.map(eoi, |(token, span)| (token, span))
}

pub(super) fn sp(node: Expr, span: Span) -> SExpr {
    AstSpanned { node, span }
}

pub(super) fn binx(op: BinOp, l: SExpr, r: SExpr) -> SExpr {
    let span = l.span.start..r.span.end;
    sp(Expr::Binary(op, Box::new(l), Box::new(r)), span)
}

pub fn program<'src>() -> PBox<'src, Program> {
    top::program_parser()
}

/// Parse a retained hook when its pass is selected for semantic checking.
///
/// Engine libraries also contain staged pass syntax. Loading declarations keeps
/// those bodies intact; selecting one must surface errors instead of treating an
/// unsupported or malformed body as an empty function.
pub fn pass_hook_body<'src>(
    hook: &'src crate::ast::PassFnHookDecl,
) -> Result<Vec<Stmt>, Vec<PError<'src>>> {
    stmt::block_parser()
        .parse(input(&hook.body, hook.span.end..hook.span.end))
        .into_result()
}

/// Executable typed hooks retain scalar conversions for WGSL lowering. The
/// signal-expression dialect's historical cast erasure is not valid here.
pub(crate) fn executable_pass_hook_body<'src>(
    hook: &'src crate::ast::PassFnHookDecl,
) -> Result<Vec<Stmt>, Vec<PError<'src>>> {
    stmt::block_parser_with_casts(true)
        .parse(input(&hook.body, hook.span.end..hook.span.end))
        .into_result()
}

fn extract_leading_doc_comment(src: &str, anchor: usize) -> Option<String> {
    let mut line_start = src[..anchor.min(src.len())]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let mut lines = Vec::new();

    while line_start > 0 {
        let prev_end = line_start.saturating_sub(1);
        let prev_start = src[..prev_end].rfind('\n').map(|idx| idx + 1).unwrap_or(0);
        let line = src[prev_start..prev_end].trim_end_matches('\r');
        let trimmed = line.trim_start();

        if let Some(text) = trimmed.strip_prefix("///") {
            lines.push(text.trim_start().to_string());
        } else if let Some(text) = trimmed.strip_prefix("//") {
            lines.push(text.trim_start().to_string());
        } else {
            break;
        }

        line_start = prev_start;
    }

    if lines.is_empty() {
        None
    } else {
        lines.reverse();
        Some(lines.join("\n"))
    }
}

fn attach_docs_to_fn_decl(src: &str, func: &mut FnDecl) {
    func.docs = extract_leading_doc_comment(src, func.span.start);
    attach_docs_to_stmt_block(src, &mut func.body);
}

fn attach_docs_to_compose_entries(src: &str, entries: &mut [ComposeEntry]) {
    for entry in entries {
        match entry {
            ComposeEntry::Expr { .. } => {}
            ComposeEntry::Block { body, .. } => attach_docs_to_stmt_block(src, body),
            ComposeEntry::If {
                then_body,
                else_body,
                ..
            } => {
                attach_docs_to_stmt_block(src, then_body);
                if let Some(body) = else_body {
                    attach_docs_to_stmt_block(src, body);
                }
            }
            ComposeEntry::InSpace { body, .. } | ComposeEntry::For { body, .. } => {
                attach_docs_to_stmt_block(src, body);
            }
        }
    }
}

fn attach_docs_to_stmt_block(src: &str, body: &mut [Stmt]) {
    for stmt in body {
        match stmt {
            Stmt::For { body, .. }
            | Stmt::InSpace { body, .. }
            | Stmt::InContext { body, .. }
            | Stmt::Block { body, .. }
            | Stmt::Seq { body, .. } => attach_docs_to_stmt_block(src, body),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                attach_docs_to_stmt_block(src, then_body);
                if let Some(body) = else_body {
                    attach_docs_to_stmt_block(src, body);
                }
            }
            Stmt::Match {
                arms, default_body, ..
            } => {
                for arm in arms {
                    attach_docs_to_stmt_block(src, &mut arm.body);
                }
                if let Some(body) = default_body {
                    attach_docs_to_stmt_block(src, body);
                }
            }
            Stmt::LetScatter { scatter, .. } => attach_docs_to_stmt_block(src, &mut scatter.body),
            Stmt::Compose { entries, .. } | Stmt::ComposePiped { entries, .. } => {
                attach_docs_to_compose_entries(src, entries);
            }
            Stmt::LocalFnDecl(func) => attach_docs_to_fn_decl(src, func),
            _ => {}
        }
    }
}

pub fn attach_leading_fn_docs(src: &str, program: &mut Program) {
    for func in &mut program.functions {
        attach_docs_to_fn_decl(src, func);
    }

    for method in program
        .styles
        .iter_mut()
        .flat_map(|s| &mut s.methods)
        .chain(
            program
                .style_contracts
                .iter_mut()
                .flat_map(|c| &mut c.hooks)
                .filter_map(|h| h.default.as_mut()),
        )
    {
        attach_docs_to_fn_decl(src, method);
    }

    for conformance in &mut program.conformances {
        for func in &mut conformance.methods {
            attach_docs_to_fn_decl(src, func);
        }
    }

    for canvas in &mut program.canvases {
        attach_docs_to_stmt_block(src, &mut canvas.body);
    }

    for surface in &mut program.surfaces {
        attach_docs_to_stmt_block(src, &mut surface.body);
    }

    for effect in &mut program.effects {
        attach_docs_to_stmt_block(src, &mut effect.body);
    }
}

pub(super) fn ident<'src>() -> PBox<'src, String> {
    select! {
        token if identifier_token(&token).is_some() => identifier_token(&token).expect("identifier token").to_string(),
    }
    .labelled("identifier")
    .boxed()
}

/// Shared contextual identifier spelling for parsing and resolved AST linking.
pub(crate) fn identifier_token(token: &Token) -> Option<&str> {
    Some(match token {
        Token::Ident(s) => s.as_str(),
        Token::Every => "every",
        Token::Blend => "blend",
        // `style` starts a style declaration, but also names ordinary properties.
        Token::Style => "style",
        // `fold ... seed` remains contextual; shader inputs can also be named seed.
        Token::Seed => "seed",
        // `field expr` is a prefix expression; an image resource can also be
        // named field in declarations, member calls, and bare value positions.
        Token::Field => "field",
        // `layer expr` is contextual, like `field expr`; shell indices can
        // also use `layer` as a parameter, local, or named argument.
        Token::Layer => "layer",
        // Shading hooks use `surface` for their prepared material argument.
        Token::Surface => "surface",
        // The interface-first canvas contract reuses `canvas` as a named contract type,
        // so it must also parse as an identifier in those contexts.
        Token::Canvas => "canvas",
        // `at` is also used as a named argument label (e.g. `stop(at: 0.0)`)
        // so it must be accepted as an identifier wherever labels appear.
        Token::At => "at",
        _ => return None,
    })
}

/// Keywords used as argument labels remain reserved in expression positions.
/// Box this shared parser to keep recursive call parsing's stack footprint small.
pub(super) fn argument_name<'src>() -> PBox<'src, String> {
    ident()
        .or(select! {
            Token::ReservedTimeUnit(unit) => unit.to_string(),
        })
        .boxed()
}

pub(super) fn ident_sp<'src>() -> PBox<'src, (String, Span)> {
    choice((
        ident().map_with(|s, e| (s, e.span())).boxed(),
        just(Token::Canvas)
            .map_with(|_, e| ("canvas".to_string(), e.span()))
            .boxed(),
    ))
    .labelled("identifier")
    .boxed()
}
pub(super) fn type_ident_sp<'src>() -> PBox<'src, (String, Span)> {
    choice((
        ident_sp(),
        just(Token::Canvas)
            .map_with(|_, e| ("canvas".to_string(), e.span()))
            .boxed(),
        just(Token::Layer)
            .map_with(|_, e| ("layer".to_string(), e.span()))
            .boxed(),
        just(Token::Field)
            .map_with(|_, e| ("field".to_string(), e.span()))
            .boxed(),
    ))
    .labelled("type")
    .boxed()
}

/// Parse a type that may appear in a `fn` parameter position.
/// Handles both simple identifiers (`f32`, `layer`, ...) and callable types
/// (`fn(T1, T2) -> T`).
///
/// Callable types are serialized to a canonical string of the form
/// `"fn(T1,T2)->T"` stored in `ty_name` so the rest of the AST stays uniform.
pub(super) fn fn_param_type_sp<'src>() -> PBox<'src, (String, Span)> {
    let comma_sep = comma_sep();
    let union_sep = nl()
        .ignore_then(just(Token::Bar))
        .then_ignore(nl())
        .ignored()
        .boxed();
    let simple = recursive(|ty| {
        let tuple_ty = ty
            .clone()
            .separated_by(comma_sep.clone())
            .allow_trailing()
            .at_least(1)
            .collect::<Vec<_>>()
            .delimited_by(
                just(Token::LParen).then_ignore(nl()),
                nl().ignore_then(just(Token::RParen)),
            )
            .map_with(|items, e| {
                let members = items
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>()
                    .join(", ");
                (format!("({members})"), e.span())
            })
            .boxed();

        let regular_ty = type_ident_sp()
            .then(
                ty.clone()
                    .or(
                        select! {Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits)}.try_map(
                            |value, span| {
                                if value.is_finite()
                                    && value >= 0.0
                                    && value.fract() == 0.0
                                    && value <= f64::from(u32::MAX)
                                {
                                    Ok((format!("{value:.0}"), span))
                                } else {
                                    Err(Rich::custom(
                                        span,
                                        "type size must be a nonnegative u32 integer",
                                    ))
                                }
                            },
                        ),
                    )
                    .separated_by(comma_sep.clone())
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(
                        just(Token::Lt).then_ignore(nl()),
                        nl().ignore_then(just(Token::Gt)),
                    )
                    .or_not(),
            )
            .then(
                choice((
                    just(Token::In)
                        .ignore_then(type_ident_sp())
                        .map(|(tag, _)| format!(" in {tag}"))
                        .boxed(),
                    select! { Token::Ident(s) if s == "from" => () }
                        .ignore_then(type_ident_sp())
                        .then_ignore(select! { Token::Ident(s) if s == "to" => () })
                        .then(type_ident_sp())
                        .map(|((from, _), (to, _))| format!(" from {from} to {to}"))
                        .boxed(),
                ))
                .or_not(),
            )
            .then(
                just(Token::LBracket)
                    .ignore_then(
                        select! { Token::Num((bits, Unit::None)) | Token::TypedNum((bits, _)) => f64::from_bits(bits) as usize },
                    )
                    .then_ignore(just(Token::RBracket))
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .map_with(|((((base, _base_span), args), suffix), array_dims), e| {
                let mut ty_name = match args {
                    Some(args) => {
                        let args = args
                            .into_iter()
                            .map(|(name, _)| name)
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("{base}<{args}>")
                    }
                    None => base,
                };
                if let Some(suffix) = suffix {
                    ty_name.push_str(&suffix);
                }
                for dim in array_dims {
                    ty_name.push_str(&format!("[{dim}]"));
                }
                (ty_name, e.span())
            })
            .boxed();

        choice((tuple_ty, regular_ty)).boxed()
    });

    // Declaration-only sugar: `i32|u32`, `i32|vec2`, etc.
    let union_simple = simple
        .clone()
        .separated_by(union_sep)
        .at_least(1)
        .collect::<Vec<_>>()
        .map_with(|branches, e| {
            let ty_name = branches
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
                .join("|");
            (ty_name, e.span())
        })
        .boxed();

    // fn(T1, T2, ...) -> RetTy
    let callable = just(Token::Fn)
        .ignore_then(
            union_simple
                .clone()
                .separated_by(comma_sep)
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(
                    just(Token::LParen).then_ignore(nl()),
                    nl().ignore_then(just(Token::RParen)),
                ),
        )
        .then(
            just(Token::Arrow)
                .ignore_then(union_simple.clone())
                .or_not(),
        )
        .map_with(|(param_tys, ret_ty), e| {
            let params_str = param_tys
                .iter()
                .map(|(ty, _)| ty.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let ret_str = match &ret_ty {
                Some((ty, _)) => format!("->{}", ty),
                None => String::new(),
            };
            (format!("fn({}){}", params_str, ret_str), e.span())
        })
        .boxed();

    choice((callable, union_simple)).labelled("type").boxed()
}

pub(super) fn nl<'src>() -> PBox<'src, ()> {
    just(Token::Newline).repeated().ignored().boxed()
}

pub(super) fn sep<'src>() -> PBox<'src, ()> {
    choice((just(Token::Newline), just(Token::Semicolon)))
        .repeated()
        .at_least(1)
        .ignored()
        .boxed()
}

pub(super) fn comma_sep<'src>() -> PBox<'src, ()> {
    nl().ignore_then(just(Token::Comma))
        .then_ignore(nl())
        .ignored()
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Stmt;

    fn parse_test_program(source: &str) -> Program {
        let tokens = crate::lexer::lex_spanned(source);
        program()
            .parse(input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap_or_else(|errors| panic!("failed to parse pass source: {errors:?}"))
    }

    #[test]
    fn style_is_contextual_in_declarations_and_value_positions() {
        let source = "pass present { draw: fullscreen\nfn shade() -> f32 {\nstyle ink() = fill(#fff)\nlet style = 1.0\nreturn style\n} }";
        let parsed = parse_test_program(source);
        let body = pass_hook_body(&parsed.passes[0].hooks[0]).unwrap();
        assert!(matches!(&body[0], Stmt::StyleDecl { name, .. } if name == "ink"));
        assert!(matches!(&body[1], Stmt::Let { name, .. } if name == "style"));
        let Stmt::Return { value, .. } = &body[2] else {
            panic!("expected a value reference after the style declaration");
        };
        assert_eq!(&source[value.span.clone()], "style");
    }

    #[test]
    fn fullscreen_hook_body_retains_authored_expression_and_span() {
        for value in ["0.25", "0.75"] {
            // Put draw after the function: declaration order must not affect parsing.
            let source = format!(
                "pass present {{ fn shade() -> f32\n{{ return {value} }}\ndraw: fullscreen\n}}"
            );
            let parsed = parse_test_program(&source);
            let body = pass_hook_body(&parsed.passes[0].hooks[0])
                .expect("fullscreen hook must parse into statements");
            let [
                Stmt::Return {
                    value: expression, ..
                },
            ] = body.as_slice()
            else {
                panic!("authored return must survive parsing");
            };
            assert_eq!(&source[expression.span.clone()], value);
            assert!(matches!(expression.node, Expr::Num(number, Unit::None)
                if number == value.parse::<f64>().expect("test number")));
        }
    }

    #[test]
    fn fullscreen_contract_has_structured_hook_bodies() {
        let source = include_str!("../tests/fixtures/engines/fullscreen.fr");
        let parsed = parse_test_program(source);
        let hooks = &parsed.passes[0].hooks;
        assert_eq!(hooks.len(), 2);
        for hook in hooks {
            let body = pass_hook_body(hook).expect("fullscreen fixture hook body must parse");
            assert!(matches!(body.last(), Some(Stmt::Return { .. })));
        }
    }

    #[test]
    fn staged_mesh_hook_retains_all_tokens_and_nested_blocks() {
        let source = include_str!("../tests/fixtures/engines/staged.fr");
        let parsed = parse_test_program(source);
        assert_eq!(parsed.passes.len(), 2);
        for pass in &parsed.passes {
            for hook in &pass.hooks {
                let tokens = &hook.body;
                let first = tokens.first().expect("opening brace");
                let last = tokens.last().expect("closing brace");
                assert_eq!(first.0, Token::LBrace);
                assert_eq!(last.0, Token::RBrace);
                let expected: Vec<_> = crate::lexer::lex_spanned(source)
                    .into_iter()
                    .filter(|(_, span)| span.start >= first.1.start && span.end <= last.1.end)
                    .collect();
                assert_eq!(*tokens, expected);
            }
        }
    }

    #[test]
    fn malformed_fullscreen_hook_reports_original_source_span() {
        let source = "pass present {\ndraw: fullscreen\nfn shade() -> color {\nlet broken =\n}\n}";
        let parsed = parse_test_program(source);
        let errors = pass_hook_body(&parsed.passes[0].hooks[0])
            .expect_err("malformed body must not become an empty statement list");
        assert!(
            errors.iter().any(|error| {
                error.span().start >= source.find("let broken").expect("test source")
            }),
            "{errors:?}"
        );
    }

    #[test]
    fn entry_template_docs_traversal_handles_shared_body() {
        let mut body = vec![Stmt::Block {
            body: vec![Stmt::Let {
                mutable: false,
                name: "x".to_string(),
                name_span: 0..0,
                declared_ty_name: None,
                declared_ty_span: None,
                value: sp(Expr::Num(1.0, Unit::None), 0..0),
            }],
            span: 0..0,
        }];

        attach_docs_to_stmt_block("/// doc comment\nlet x = 1.0", &mut body);

        assert_eq!(body.len(), 1);
    }

    #[test]
    fn compute_operation_definitions_retain_output_and_dispatch_expressions() {
        let prefix = parse_test_program("const value: f32 = field 1.0");
        assert!(matches!(prefix.consts[0].value.node, Expr::Field(_)));
        let source = r#"
compute Offsets(count: u32, width: f32) -> buffer<vec4, read> {
    requires count <= 4096u
    output offsets: buffer<vec4, write>(checked_mul(count, 2u))
    workgroup_size: (64, 1, 1)
    dispatch threads(count, 1, 1)
    @compute fn main(id: uvec3) {
        if id.x >= count { return }
        offsets[id.x] = vec4(width)
    }
    return offsets
}
compute Field(size: u32) -> texture2d<r32float, read> {
    output field: texture2d<r32float, write>(size, size)
    workgroup_size: (8, 8, 1)
    dispatch threads(size, size, 1)
    @compute fn main(id: uvec3) {
        if id.x >= size || id.y >= size { return }
        field.store(id.xy, 1.0)
    }
    return field
}
"#;
        let program = parse_test_program(source);
        assert_eq!(program.passes.len(), 2);
        assert!(
            program.pipelines.is_empty(),
            "definitions must not create work"
        );
        let operation = program.passes[0].operation.as_ref().unwrap();
        assert_eq!(operation.inputs.len(), 2);
        assert_eq!(operation.requirements.len(), 1);
        let compute = operation.compute.as_ref().unwrap();
        assert_eq!(compute.return_ty.node.replace(' ', ""), "buffer<vec4,read>");
        let output = compute.output.as_ref().unwrap();
        assert_eq!(output.name, "offsets");
        assert!(
            matches!(&output.extents[0].node, Expr::Call { name, .. } if name == "checked_mul")
        );
        assert_eq!(
            &source[output.extents[0].span.clone()],
            "checked_mul(count, 2u)"
        );
        assert_eq!(compute.workgroup_size.as_ref().unwrap().len(), 3);
        assert_eq!(compute.threads.as_ref().unwrap().len(), 3);
        assert_eq!(compute.returned.as_ref().unwrap().node, "offsets");
        assert!(
            !executable_pass_hook_body(&program.passes[0].hooks[0])
                .unwrap()
                .is_empty()
        );
        let image = program.passes[1]
            .operation
            .as_ref()
            .unwrap()
            .compute
            .as_ref()
            .unwrap();
        assert_eq!(image.output.as_ref().unwrap().extents.len(), 2);
        assert!(
            !executable_pass_hook_body(&program.passes[1].hooks[0])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn compute_graph_results_are_distinct_from_shader_lets_and_draw_calls() {
        let source = r#"
style Fuzzy for standard : StandardStyle {
    for self {
        let offsets = Offsets(count: 16u, width: 2.0)
        at after_opaque as target {
            Shell(offsets: offsets, color: target.color)
        }
    }


}
"#;
        let program = parse_test_program(source);
        let crate::ast::StyleGraphNode::ForSelf(body) = &program.styles[0].graph[0].node else {
            panic!("expected per-invocation graph");
        };
        assert!(
            matches!(&body[0].node, crate::ast::StyleGraphNode::Let { name, value } if name == "offsets" && matches!(&value.node, Expr::Call { name, .. } if name == "Offsets"))
        );
        let crate::ast::StyleGraphNode::At { body, .. } = &body[1].node else {
            panic!("expected integration point");
        };
        assert!(matches!(body[0].node, crate::ast::StyleGraphNode::Call(_)));
    }

    #[test]
    fn graph_value_bindings_preserve_expressions_for_typed_constant_checking() {
        let program = parse_test_program(
            "style S for standard : StandardStyle { for self { let layer = 3u - 1u; static if !false { Draw(layer: layer) } } }",
        );
        let crate::ast::StyleGraphNode::ForSelf(body) = &program.styles[0].graph[0].node else {
            panic!("per-invocation graph")
        };
        assert!(
            matches!(&body[0].node, crate::ast::StyleGraphNode::Let { name, value } if name == "layer" && matches!(value.node, Expr::Binary(BinOp::Sub, ..)))
        );
        assert!(
            matches!(&body[1].node, crate::ast::StyleGraphNode::StaticIf { condition, .. } if matches!(condition.node, Expr::Binary(BinOp::Eq, ..)))
        );
    }

    #[test]
    fn shading_inputs_keep_types_scope_and_explicit_graph_bindings() {
        let source = r#"
style Fuzzy for standard : StandardStyle {
    shading_input density: texture2d<r32float, read> scope draw
    shading_input values: buffer<vec4, read> scope draw
    for self {
        let field = Field(size: 16u)
        bind shading.density = field
    }
}
"#;
        let program = parse_test_program(source);
        let style = &program.styles[0];
        assert_eq!(style.shading_inputs.len(), 2);
        let slot = &style.shading_inputs[0];
        assert_eq!(slot.name, "density");
        assert_eq!(slot.scope, "draw");
        assert_eq!(slot.ty.replace(' ', ""), "texture2d<r32float,read>");
        assert!(source[slot.span.clone()].starts_with("shading_input density"));
        let crate::ast::StyleGraphNode::ForSelf(body) = &style.graph[0].node else {
            panic!("per-draw graph");
        };
        assert!(
            matches!(&body[1].node, crate::ast::StyleGraphNode::BindShading { name, value } if name == "density" && matches!(&value.node, Expr::Var(name) if name == "field"))
        );
    }

    #[test]
    fn compute_operation_parser_rejects_duplicate_and_misplaced_clauses() {
        for (source, expected) in [
            (
                "draw Bad() { output data: buffer<u32, write>(1u) }",
                "output belongs to a compute",
            ),
            (
                "pass bad { dispatch threads(1, 1, 1) }",
                "dispatch threads belongs to a compute",
            ),
            (
                "compute Bad() -> buffer<u32, read> { output a: buffer<u32, write>(1u); output b: buffer<u32, write>(1u) }",
                "exactly one output",
            ),
            (
                "compute Bad() -> buffer<u32, read> { workgroup_size: (1, 1, 1); workgroup_size: (2, 1, 1) }",
                "duplicate compute workgroup_size",
            ),
            (
                "compute Bad() -> buffer<u32, read> { dispatch threads(1, 1, 1); dispatch threads(2, 1, 1) }",
                "duplicate compute dispatch",
            ),
            (
                "compute Bad() -> buffer<u32, read> { return a; return b }",
                "duplicate compute resource return",
            ),
            (
                "style Bad for standard : StandardStyle { for self { let result = } }",
                "graph let requires an initializer",
            ),
        ] {
            let tokens = crate::lexer::lex_spanned(source);
            let errors = program()
                .parse(input(&tokens, source.len()..source.len()))
                .into_result()
                .unwrap_err();
            assert!(!errors.is_empty(), "{expected}: {source}");
        }
    }
}
