//! Shader-only, allocation-free iterators expanded at their consuming loop.
//! The generator and consumer retain separate lexical environments.
use crate::ast::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn element(ty: &str) -> Option<&str> {
    ty.trim()
        .strip_prefix("iterator<")?
        .strip_suffix('>')
        .map(str::trim)
}

fn rename(value: &mut SExpr, names: &BTreeMap<String, String>) -> Result<(), String> {
    match &mut value.node {
        Expr::Var(name) => {
            let (root, suffix) = name
                .split_once('.')
                .map_or((name.as_str(), None), |(a, b)| (a, Some(b)));
            if let Some(replacement) = names.get(root) {
                *name = suffix.map_or_else(
                    || replacement.clone(),
                    |suffix| format!("{replacement}.{suffix}"),
                );
            }
        }
        Expr::Num(..) | Expr::Color(..) | Expr::Str(..) => {}
        Expr::Vec2(a, b) | Expr::Binary(_, a, b) | Expr::Range(a, b) => {
            rename(a, names)?;
            rename(b, names)?;
        }
        Expr::Vec3(a, b, c) => {
            rename(a, names)?;
            rename(b, names)?;
            rename(c, names)?;
        }
        Expr::Vec4(a, b, c, d) => {
            rename(a, names)?;
            rename(b, names)?;
            rename(c, names)?;
            rename(d, names)?;
        }
        Expr::Array(values) => {
            for value in values {
                rename(value, names)?;
            }
        }
        Expr::Unary(_, value) | Expr::Member(value, _) => rename(value, names)?,
        Expr::Index { array, index } => {
            rename(array, names)?;
            rename(index, names)?;
        }
        Expr::Call {
            args, const_args, ..
        } => {
            for arg in args {
                rename(&mut arg.value, names)?;
            }
            for arg in const_args {
                rename(&mut arg.value, names)?;
            }
        }
        Expr::Pipe { recv, args, .. } => {
            rename(recv, names)?;
            for arg in args {
                rename(&mut arg.value, names)?;
            }
        }
        other => return Err(format!("unsupported shader iterator expression: {other:?}")),
    }
    Ok(())
}

pub(super) struct Expansion<'a> {
    pub occupied: &'a mut BTreeSet<String>,
    pub serial: &'a mut u32,
    pub variable: &'a str,
    pub consumer: &'a [Stmt],
    pub element: &'a str,
    pub span: &'a Span,
}

impl Expansion<'_> {
    fn fresh(&mut self, label: &str) -> Result<String, String> {
        loop {
            let name = format!("fresco_iterator_{}_{label}", self.serial);
            *self.serial = self
                .serial
                .checked_add(1)
                .ok_or("shader iterator name capacity exceeded")?;
            if self.occupied.insert(name.clone()) {
                return Ok(name);
            }
        }
    }

    fn local(&self, name: String, ty: String, value: SExpr) -> Stmt {
        Stmt::Let {
            mutable: false,
            name,
            name_span: self.span.clone(),
            declared_ty_name: Some(ty),
            declared_ty_span: Some(self.span.clone()),
            value,
        }
    }

    fn body(
        &mut self,
        body: &[Stmt],
        mut names: BTreeMap<String, String>,
    ) -> Result<Vec<Stmt>, String> {
        let mut result = Vec::new();
        for source in body {
            let mut statement = source.clone();
            match &mut statement {
                Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. } => {
                    rename(value, &names)?;
                    let replacement = self.fresh(name)?;
                    names.insert(name.clone(), replacement.clone());
                    *name = replacement;
                }
                Stmt::Assign { name, value, .. } => {
                    rename(value, &names)?;
                    if let Some(replacement) = names.get(name) {
                        name.clone_from(replacement);
                    }
                }
                Stmt::For {
                    name,
                    iterable,
                    body,
                    index_name: None,
                    ..
                } => {
                    rename(iterable, &names)?;
                    let mut nested = names.clone();
                    let replacement = self.fresh(name)?;
                    nested.insert(name.clone(), replacement.clone());
                    *name = replacement;
                    *body = self.body(body, nested)?;
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    rename(cond, &names)?;
                    *then_body = self.body(then_body, names.clone())?;
                    if let Some(body) = else_body {
                        *body = self.body(body, names.clone())?;
                    }
                }
                Stmt::Block { body, .. } => {
                    *body = self.body(body, names.clone())?;
                }
                Stmt::Expr(value) => {
                    rename(value, &names)?;
                    if let Expr::Call {
                        name,
                        args,
                        const_args,
                        ..
                    } = &value.node
                        && name == "yield"
                    {
                        if args.len() != 1 || args[0].name.is_some() || !const_args.is_empty() {
                            return Err("yield requires one positional value".into());
                        }
                        let mut body = vec![self.local(
                            self.variable.into(),
                            self.element.into(),
                            args[0].value.clone(),
                        )];
                        body.extend_from_slice(self.consumer);
                        statement = Stmt::Block {
                            body,
                            span: value.span.clone(),
                        };
                    }
                }
                Stmt::Return { .. } | Stmt::ReturnVoid { .. } | Stmt::Break { .. } => {
                    return Err("shader iterators complete by falling through; early return and break are not supported".into());
                }
                other => return Err(format!("unsupported shader iterator statement: {other:?}")),
            }
            result.push(statement);
        }
        Ok(result)
    }

    pub(super) fn expand(
        &mut self,
        hook: &PassFnHookDecl,
        arguments: &[Arg],
    ) -> Result<Stmt, String> {
        if arguments.len() != hook.params.len() || arguments.iter().any(|a| a.name.is_some()) {
            return Err("shader iterator calls require explicit positional arguments".into());
        }
        let mut names = BTreeMap::new();
        let mut result = Vec::new();
        for (parameter, argument) in hook.params.iter().zip(arguments) {
            let name = self.fresh(&parameter.name)?;
            result.push(self.local(
                name.clone(),
                parameter.ty_name.clone(),
                argument.value.clone(),
            ));
            names.insert(parameter.name.clone(), name);
        }
        let body = crate::parser::executable_pass_hook_body(hook)
            .map_err(|e| format!("invalid shader iterator: {e:?}"))?;
        result.extend(self.body(&body, names)?);
        Ok(Stmt::Block {
            body: result,
            span: self.span.clone(),
        })
    }
}

/// Prepared/procedural raster programs use the same hygienic expansion as
/// factory raster programs, before library-call resolution and GPU emission.
pub(super) fn expand_body(pass: &PassDecl, body: &mut [Stmt]) -> Result<(), String> {
    struct Walker<'a> {
        pass: &'a PassDecl,
        occupied: BTreeSet<String>,
        serial: u32,
        count: u32,
    }
    impl Walker<'_> {
        fn walk(&mut self, body: &mut [Stmt]) -> Result<(), String> {
            for statement in body {
                match statement {
                    Stmt::For {
                        name,
                        iterable,
                        body,
                        index_name: None,
                        ..
                    } => {
                        let generator = if let Expr::Call { name, args, .. } = &iterable.node {
                            self.pass
                                .hooks
                                .iter()
                                .find(|hook| hook.name == *name)
                                .and_then(|hook| {
                                    hook.return_ty
                                        .as_ref()
                                        .and_then(|ty| element(&ty.node))
                                        .map(|element| (hook, element, args))
                                })
                        } else {
                            None
                        };
                        if let Some((hook, element, args)) = generator {
                            self.count = self.count.checked_add(1).filter(|count| *count <= 256)
                                .ok_or("shader iterator expansion exceeds 256 calls; check for recursion")?;
                            let mut expanded = Expansion {
                                occupied: &mut self.occupied,
                                serial: &mut self.serial,
                                variable: name,
                                consumer: body,
                                element,
                                span: &iterable.span,
                            }
                            .expand(hook, args)?;
                            self.walk(std::slice::from_mut(&mut expanded))?;
                            *statement = expanded;
                        } else {
                            self.walk(body)?;
                        }
                    }
                    Stmt::If {
                        then_body,
                        else_body,
                        ..
                    } => {
                        self.walk(then_body)?;
                        if let Some(body) = else_body {
                            self.walk(body)?;
                        }
                    }
                    Stmt::Block { body, .. } => self.walk(body)?,
                    // These statements contain no nested shader statement lists.
                    Stmt::Let { .. }
                    | Stmt::Const { .. }
                    | Stmt::Assign { .. }
                    | Stmt::Store { .. }
                    | Stmt::Return { .. }
                    | Stmt::ReturnVoid { .. }
                    | Stmt::Break { .. }
                    | Stmt::Expr(_) => {}
                    other => {
                        return Err(format!(
                            "unsupported statement in shader iterator consumer: {other:?}"
                        ));
                    }
                }
            }
            Ok(())
        }
    }
    let mut occupied = BTreeSet::new();
    for hook in &pass.hooks {
        occupied.extend(hook.params.iter().map(|p| p.name.clone()));
        occupied.extend(
            hook.body
                .iter()
                .filter_map(|(token, _)| crate::parser::identifier_token(token).map(str::to_owned)),
        );
    }
    occupied.extend(pass.bindings.iter().map(|binding| binding.name.clone()));
    Walker {
        pass,
        occupied,
        serial: 0,
        count: 0,
    }
    .walk(body)
}
