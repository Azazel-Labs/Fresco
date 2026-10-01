use super::*;

pub(super) type Signature = (Handle<naga::Function>, Vec<Type>, Option<Type>);

impl Compiler<'_> {
    pub(super) fn call(
        &mut self,
        function: &mut Function,
        name: &str,
        args: &[ast::Arg],
        span: &ast::Span,
    ) -> Result<Option<Value>> {
        if args.iter().any(|a| a.name.is_some()) {
            return Err(Error::new(
                span.clone(),
                "native helper arguments must be positional",
            ));
        }
        let (callee, parameters, result) = self.function(name, span)?;
        if parameters.len() != args.len() {
            return Err(Error::new(
                span.clone(),
                "native helper argument count mismatch",
            ));
        }
        let mut arguments = Vec::new();
        for (arg, expected) in args.iter().zip(parameters) {
            let value = self.value(function, &arg.value)?;
            if value.ty != expected {
                return Err(Error::new(
                    arg.value.span.clone(),
                    "native helper argument type mismatch; pointer access conversions are not implicit",
                ));
            }
            arguments.push(value.expression);
        }
        let value = result.map(|ty| Value {
            expression: function.expr(Ex::CallResult(callee), false),
            ty,
            place: None,
        });
        function.body.push(
            naga::Statement::Call {
                function: callee,
                arguments,
                result: value.as_ref().map(|v| v.expression),
            },
            SP,
        );
        Ok(value)
    }

    pub(super) fn function(&mut self, name: &str, span: &ast::Span) -> Result<Signature> {
        if let Some(signature) = self.functions.get(name) {
            return Ok(signature.clone());
        }
        if !self.compiling_functions.insert(name.into()) {
            return Err(Error::new(
                span.clone(),
                "recursive native function calls are not supported",
            ));
        }
        let declaration = self
            .program
            .functions
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| Error::new(span.clone(), format!("unknown native function `{name}`")))?
            .clone();
        if !declaration.type_params.is_empty()
            || !declaration.const_params.is_empty()
            || declaration.is_builtin
        {
            return Err(Error::new(
                declaration.span,
                "native generic and extern helper specialization is not supported yet",
            ));
        }
        let mut function = Function::default();
        function.ir.name = Some(name.into());
        let mut parameters = Vec::new();
        for (index, param) in declaration.params.iter().enumerate() {
            let ty =
                Type::parse(&param.ty_name).map_err(|e| Error::new(param.ty_span.clone(), e))?;
            if self.contains_pointer(&ty, &param.ty_span)? {
                return Err(Error::new(
                    param.ty_span.clone(),
                    "pointer helper parameters require additional Naga support; helpers are not implicitly inlined",
                ));
            }
            let resolved = self.resolve(&ty, &param.ty_span)?;
            function.ir.arguments.push(naga::FunctionArgument {
                name: Some(param.name.clone()),
                ty: resolved.handle,
                binding: None,
            });
            let index = u32::try_from(index)
                .map_err(|_| Error::new(param.name_span.clone(), "too many function parameters"))?;
            let expression = function.expr(Ex::FunctionArgument(index), false);
            function.variables.insert(
                param.name.clone(),
                Value {
                    expression,
                    ty: ty.clone(),
                    place: None,
                },
            );
            parameters.push(ty);
        }
        if let Some((name, span)) = &declaration.ret_ty {
            let ty = Type::parse(name).map_err(|e| Error::new(span.clone(), e))?;
            if self.contains_pointer(&ty, span)? {
                return Err(Error::new(
                    span.clone(),
                    "pointer helper return types require additional Naga support",
                ));
            }
            function.ir.result = Some(naga::FunctionResult {
                ty: self.resolve(&ty, span)?.handle,
                binding: None,
            });
            function.result = Some(ty);
        }
        self.statements(
            &mut function,
            declaration.typed_body.as_ref().unwrap_or(&declaration.body),
        )?;
        let result = function.result.clone();
        let handle = self.module.functions.append(function.finish(), SP);
        self.compiling_functions.remove(name);
        let signature = (handle, parameters, result);
        self.functions.insert(name.into(), signature.clone());
        Ok(signature)
    }
}
