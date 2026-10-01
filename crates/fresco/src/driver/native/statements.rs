use super::*;
use ast::{Expr, Stmt};

impl Compiler<'_> {
    fn store(&mut self, function: &mut Function, target: &SExpr, source: &SExpr) -> Result<()> {
        let destination = self.expression(function, target)?;
        let Some(place) = destination.place else {
            return Err(Error::new(
                target.span.clone(),
                "assignment requires an assignable memory location",
            ));
        };
        if !place.writable {
            return Err(Error::new(
                target.span.clone(),
                "cannot write through a read-only pointer or immutable binding",
            ));
        }
        let value = self.value(function, source)?;
        if value.ty != destination.ty {
            return Err(Error::new(
                source.span.clone(),
                format!(
                    "assignment type mismatch: expected {:?}, found {:?}; pointer access conversions are not implicit",
                    destination.ty, value.ty
                ),
            ));
        }
        if self.contains_pointer(&value.ty, &source.span)? {
            return Err(Error::new(
                source.span.clone(),
                "storing pointers or pointer-containing aggregates is not implemented in native lowering",
            ));
        }
        self.requirements.writes_device_memory |= place.physical;
        function.body.push(
            naga::Statement::Store {
                pointer: destination.expression,
                value: value.expression,
            },
            SP,
        );
        Ok(())
    }

    pub(super) fn statements(
        &mut self,
        function: &mut Function,
        statements: &[Stmt],
    ) -> Result<()> {
        for statement in statements {
            match statement {
                Stmt::Let {
                    mutable,
                    name,
                    name_span,
                    declared_ty_name,
                    value,
                    ..
                } => {
                    let mut result = self.value(function, value)?;
                    if let Some(declared) = declared_ty_name {
                        let declared =
                            Type::parse(declared).map_err(|e| Error::new(name_span.clone(), e))?;
                        if declared != result.ty {
                            return Err(Error::new(
                                value.span.clone(),
                                "local type does not match initializer; pointer access conversions are not implicit",
                            ));
                        }
                    }
                    if *mutable {
                        if self.contains_pointer(&result.ty, &value.span)? {
                            return Err(Error::new(
                                name_span.clone(),
                                "mutable pointer locals require additional Naga support; bind pointers with let",
                            ));
                        }
                        let ty = self.resolve(&result.ty, name_span)?.handle;
                        let local = function.ir.local_variables.append(
                            naga::LocalVariable {
                                name: Some(name.clone()),
                                ty,
                                init: None,
                            },
                            SP,
                        );
                        let pointer = function.expr(Ex::LocalVariable(local), false);
                        function.body.push(
                            naga::Statement::Store {
                                pointer,
                                value: result.expression,
                            },
                            SP,
                        );
                        result.expression = pointer;
                        result.place = Some(Place {
                            writable: true,
                            physical: false,
                        });
                    }
                    function.variables.insert(name.clone(), result);
                }
                Stmt::Store { target, value, .. } => self.store(function, target, value)?,
                Stmt::Assign {
                    name,
                    name_span,
                    field_path,
                    value,
                    ..
                } => {
                    let mut target = SExpr {
                        node: Expr::Var(name.clone()),
                        span: name_span.clone(),
                    };
                    if let Some(path) = field_path {
                        for field in path.split('.') {
                            target = SExpr {
                                node: Expr::Member(Box::new(target), field.into()),
                                span: name_span.clone(),
                            };
                        }
                    }
                    self.store(function, &target, value)?;
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                    ..
                } => {
                    let condition = self.value(function, cond)?;
                    if condition.ty != Type::Bool {
                        return Err(Error::new(cond.span.clone(), "if condition must be bool"));
                    }
                    let parent = std::mem::take(&mut function.body);
                    let variables = function.variables.clone();
                    self.statements(function, then_body)?;
                    let accept = std::mem::take(&mut function.body);
                    function.variables.clone_from(&variables);
                    if let Some(body) = else_body {
                        self.statements(function, body)?;
                    }
                    let reject = std::mem::replace(&mut function.body, parent);
                    function.variables = variables;
                    function.body.push(
                        naga::Statement::If {
                            condition: condition.expression,
                            accept,
                            reject,
                        },
                        SP,
                    );
                }
                Stmt::Block { body, .. } => {
                    let variables = function.variables.clone();
                    self.statements(function, body)?;
                    function.variables = variables;
                }
                Stmt::Seq { body, .. } => self.statements(function, body)?,
                Stmt::ReturnVoid { span } => {
                    if function.result.is_some() {
                        return Err(Error::new(
                            span.clone(),
                            "non-void helper must return a value",
                        ));
                    }
                    function
                        .body
                        .push(naga::Statement::Return { value: None }, SP);
                }
                Stmt::Return { value, .. } => {
                    let result = self.value(function, value)?;
                    if function.result.as_ref() != Some(&result.ty) {
                        return Err(Error::new(value.span.clone(), "return type mismatch"));
                    }
                    function.body.push(
                        naga::Statement::Return {
                            value: Some(result.expression),
                        },
                        SP,
                    );
                }
                Stmt::Expr(expr) => {
                    if let Expr::Call {
                        name,
                        args,
                        const_args,
                        ..
                    } = &expr.node
                        && !matches!(name.as_str(), "u32" | "i32" | "f32")
                    {
                        if !const_args.is_empty() {
                            return Err(Error::new(
                                expr.span.clone(),
                                "native generic helper specialization is not supported yet",
                            ));
                        }
                        self.call(function, name, args, &expr.span)?;
                    } else {
                        self.value(function, expr)?;
                    }
                }
                _ => {
                    return Err(Error::new(
                        0..0,
                        "statement is not supported in native functions yet",
                    ));
                }
            }
        }
        Ok(())
    }
}
