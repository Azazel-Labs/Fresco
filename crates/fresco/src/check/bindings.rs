//! Lexical assignment validation, shared by every evaluator and shader stage.
//! Run before specialization so dead branches cannot hide writes to immutable values.
use std::collections::HashMap;

use crate::ast::{ComposeEntry, FnDecl, FnParam, PassFnHookDecl, Program, Span, Stmt};
use crate::diag::Diag;

#[derive(Clone)]
struct Binding {
    mutable: bool,
    span: Span,
}
type Scope = HashMap<String, Binding>;

fn bind(scope: &mut Scope, name: &str, span: &Span, mutable: bool) {
    scope.insert(
        name.to_owned(),
        Binding {
            mutable,
            span: span.clone(),
        },
    );
}

fn params(scope: &mut Scope, parameters: &[FnParam]) {
    for param in parameters {
        bind(scope, &param.name, &param.name_span, false);
    }
}

fn function(function: &FnDecl, outer: &Scope, diags: &mut Vec<Diag>) {
    let first_diag = diags.len();
    let mut scope = outer.clone();
    params(&mut scope, &function.params);
    for param in &function.const_params {
        bind(&mut scope, &param.name, &param.name_span, false);
    }
    statements(&function.body, &mut scope, diags);
    if !function.source_file.is_empty() {
        for diag in &mut diags[first_diag..] {
            diag.file = Some(function.source_file.clone());
        }
    }
}

fn block(body: &[Stmt], outer: &Scope, diags: &mut Vec<Diag>) {
    statements(body, &mut outer.clone(), diags);
}

fn entries(items: &[ComposeEntry], scope: &Scope, diags: &mut Vec<Diag>) {
    for entry in items {
        match entry {
            ComposeEntry::Block { body, .. } | ComposeEntry::InSpace { body, .. } => {
                block(body, scope, diags);
            }
            ComposeEntry::If {
                then_body,
                else_body,
                ..
            } => {
                block(then_body, scope, diags);
                if let Some(body) = else_body {
                    block(body, scope, diags);
                }
            }
            ComposeEntry::For {
                name,
                index_name,
                body,
                span,
                ..
            } => {
                let mut inner = scope.clone();
                bind(&mut inner, name, span, false);
                if let Some((name, span)) = index_name {
                    bind(&mut inner, name, span, false);
                }
                statements(body, &mut inner, diags);
            }
            ComposeEntry::Expr { .. } => {} // Expressions cannot contain assignment statements.
        }
    }
}

fn statements(body: &[Stmt], scope: &mut Scope, diags: &mut Vec<Diag>) {
    for stmt in body {
        match stmt {
            Stmt::Store { target, span, .. } => {
                if let crate::ast::Expr::Index { array, .. } = &target.node
                    && let crate::ast::Expr::Var(name) = &array.node
                    && let Some(binding) = scope.get(name)
                    && !binding.mutable
                {
                    diags.push(Diag::error(
                        span.clone(),
                        format!("cannot assign to immutable binding `{name}`"),
                    ));
                }
            }
            Stmt::Let {
                name,
                name_span,
                mutable,
                ..
            } => bind(scope, name, name_span, *mutable),
            Stmt::Const {
                name, name_span, ..
            }
            | Stmt::Param {
                name, name_span, ..
            }
            | Stmt::TextureBinding {
                name, name_span, ..
            }
            | Stmt::SpaceDecl {
                name, name_span, ..
            } => bind(scope, name, name_span, false),
            // Styles have their own namespace and do not shadow value bindings.
            Stmt::StyleDecl { .. } => {}
            Stmt::Assign {
                name, name_span, ..
            } => {
                // Unknown targets remain the responsibility of name/type checking.
                if let Some(binding) = scope.get(name)
                    && !binding.mutable
                {
                    diags.push(Diag::error(name_span.clone(), format!("cannot assign to immutable binding `{name}`"))
                        .with_code("immutable_binding")
                        .with_related_label(binding.span.clone(), "immutable binding declared here")
                        .with_help("declare a mutable local with `var`, or copy this value into a new `var` before updating it"));
                }
            }
            Stmt::For {
                name,
                name_span,
                index_name,
                body,
                ..
            } => {
                let mut inner = scope.clone();
                bind(&mut inner, name, name_span, false);
                if let Some((name, span)) = index_name {
                    bind(&mut inner, name, span, false);
                }
                statements(body, &mut inner, diags);
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                block(then_body, scope, diags);
                if let Some(body) = else_body {
                    block(body, scope, diags);
                }
            }
            Stmt::Match {
                arms, default_body, ..
            } => {
                for arm in arms {
                    block(&arm.body, scope, diags);
                }
                if let Some(body) = default_body {
                    block(body, scope, diags);
                }
            }
            Stmt::LetScatter {
                name,
                name_span,
                scatter,
            } => {
                block(&scatter.body, scope, diags);
                bind(scope, name, name_span, false);
            }
            Stmt::Block { body, .. }
            | Stmt::InSpace { body, .. }
            | Stmt::InContext { body, .. } => block(body, scope, diags),
            Stmt::Seq { body, .. } => statements(body, scope, diags),
            Stmt::Compose { entries: items, .. } | Stmt::ComposePiped { entries: items, .. } => {
                entries(items, scope, diags);
            }
            Stmt::LocalFnDecl(decl) => function(decl, scope, diags),
            Stmt::CanvasSpace { .. }
            | Stmt::SurfaceVertex { .. }
            | Stmt::ReturnVoid { .. }
            | Stmt::Return { .. }
            | Stmt::Break { .. }
            | Stmt::Expr(_) => {}
        }
    }
}

fn hook(hook: &PassFnHookDecl, globals: &Scope, diags: &mut Vec<Diag>) {
    // Unselected staged hooks may use syntax not yet supported by the ordinary
    // parser. Their existing selected-hook validation owns those parse errors.
    if let Ok(body) = crate::parser::pass_hook_body(hook) {
        let mut scope = globals.clone();
        for param in &hook.params {
            bind(&mut scope, &param.name, &param.name_span, false);
        }
        statements(&body, &mut scope, diags);
    }
}

pub(crate) fn validate(program: &Program) -> Result<(), Vec<Diag>> {
    let mut diags = Vec::new();
    let mut globals = Scope::new();
    for item in &program.consts {
        bind(&mut globals, &item.name, &item.name_span, false);
    }
    for item in &program.params {
        bind(&mut globals, &item.name, &item.name_span, false);
    }
    for item in &program.functions {
        function(item, &globals, &mut diags);
    }
    for conformance in &program.conformances {
        for method in &conformance.methods {
            function(method, &globals, &mut diags);
        }
    }
    for entry in program.root_entries() {
        let entry = entry.as_normalized_root_entry();
        let mut scope = globals.clone();
        for param in &entry.params {
            bind(&mut scope, &param.name, &param.name_span, false);
        }
        statements(&entry.body, &mut scope, &mut diags);
    }
    for entry in &program.authored_entries {
        let mut scope = globals.clone();
        for param in &entry.params {
            bind(&mut scope, &param.name, &param.name_span, false);
        }
        block(&entry.body, &scope, &mut diags);
        for item in &entry.blocks {
            let mut scope = scope.clone();
            if let Some(parameters) = &item.params {
                params(&mut scope, parameters);
            }
            statements(&item.body, &mut scope, &mut diags);
        }
    }
    for effect in &program.effects {
        let mut scope = globals.clone();
        params(&mut scope, &effect.params);
        bind(&mut scope, "self", &effect.name_span, false);
        block(&effect.body, &scope, &mut diags);
    }
    for model in &program.schema_programs {
        for function in &model.functions {
            let mut scope = globals.clone();
            params(&mut scope, &function.params);
            bind(&mut scope, "self", &model.name_span, false);
            block(&function.body, &scope, &mut diags);
        }
    }
    for pass in &program.passes {
        for item in &pass.hooks {
            hook(item, &globals, &mut diags);
        }
    }
    for factory in &program.vertex_factories {
        for item in &factory.hooks {
            hook(item, &globals, &mut diags);
        }
    }
    if diags.is_empty() { Ok(()) } else { Err(diags) }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;
    use logos::Logos;

    fn parse(source: &str) -> Program {
        let tokens = crate::lexer::Token::lexer(source)
            .spanned()
            .map(|(token, span)| (token.expect("valid token"), span))
            .collect::<Vec<_>>();
        let (program, errors) = crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_output_errors();
        assert!(errors.is_empty(), "{errors:?}");
        program.expect("valid program")
    }

    #[test]
    fn immutable_writes_are_rejected_in_every_branch_and_field() {
        for statement in [
            "x = 2.0",
            "x += 2.0",
            "x.x = 2.0",
            "x.xy = vec2(2.0)",
            "if false { x = 2.0 }",
            "{ x = 2.0 }",
        ] {
            let source = format!("fn test() {{ let x = vec2(1.0); {statement}; }}");
            let errors = validate(&parse(&source)).expect_err("immutable write");
            assert_eq!(errors.len(), 1, "{source}");
            assert_eq!(errors[0].code.as_deref(), Some("immutable_binding"));
            assert_eq!(&source[errors[0].span.clone()], "x");
        }
    }

    #[test]
    fn mutable_locals_and_shadowing_have_lexical_scope() {
        validate(&parse("fn test() { var x: f32 = 1.0; { let x = 2.0; } x += 1.0; let y: f32 = x; { var y = 3.0; y += 1.0; } }")).unwrap();
        let errors = validate(&parse(
            "fn test() { let x = 1.0; { var x = 2.0; x += 1.0; } x = 3.0; }",
        ))
        .unwrap_err();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn parameters_constants_loops_and_captures_are_immutable() {
        for source in [
            "fn test(x: f32) { x = 2.0; }",
            "const x: f32 = 1.0; fn test() { x = 2.0; }",
            "fn test() { const f32 x = 1.0; x = 2.0; }",
            "fn test() { for x in 0..3 { x = 2.0; } }",
            "fn test() { let x = 1.0; fn nested() { x = 2.0; } }",
        ] {
            assert!(validate(&parse(source)).is_err(), "{source}");
        }
    }

    #[test]
    fn styles_do_not_shadow_value_mutability() {
        for keyword in ["let", "var"] {
            let source = format!(
                "canvas test(uv: coord) -> color {{ {keyword} x = 1.0; style x = fill(#fff); x += 1.0; compose {{ fill(#fff) }} }}"
            );
            assert_eq!(validate(&parse(&source)).is_ok(), keyword == "var");
        }
    }

    #[test]
    fn pass_hooks_share_binding_rules() {
        for keyword in ["let", "var"] {
            let source = format!(
                "pass p {{ fn shade() -> color {{ {keyword} x = 1.0; x += 1.0; return rgba(x, 0.0, 0.0, 1.0); }} }}"
            );
            assert_eq!(validate(&parse(&source)).is_ok(), keyword == "var");
        }
    }
}
