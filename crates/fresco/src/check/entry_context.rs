use super::*;
use crate::context::{ContextStruct, ContextType, EntryContext};

impl Checker {
    pub(super) fn activate_context_coordinate(&mut self) -> Option<(Sx, Sx)> {
        let roles = self.evaluation_context.as_mut()?;
        let Value::Vec2(coord) = roles.get("coord")?.clone() else {
            unreachable!("checked role");
        };
        roles.insert("coord".to_string(), Value::Vec2((Sx::CoordX, Sx::CoordY)));
        Some(coord)
    }

    pub(super) fn context_layer(&mut self, inner: LayerId, coord: Option<(Sx, Sx)>) -> LayerId {
        let Some((x, y)) = coord else {
            return inner;
        };
        self.hir.layer(Layer::InSpace {
            inner,
            xforms: vec![Xform::Warp {
                by: (
                    Sx::Sub(Box::new(Sx::CoordX), Box::new(x)),
                    Sx::Sub(Box::new(Sx::CoordY), Box::new(y)),
                ),
            }],
        })
    }

    pub(super) fn bind_entry_context(
        &mut self,
        entry: &NormalizedRootEntry,
        contract: EntryContext,
    ) {
        if entry.params.len() != 1 || entry.params[0].ty_name != contract.ty.name {
            self.diags.push(Diag::error(
                entry.span.clone(),
                format!(
                    "{} `{}` must implement {}.{} with one parameter of type `{}`",
                    entry.entry_kind,
                    entry.name,
                    contract.interface,
                    contract.method,
                    contract.ty.name
                ),
            ));
            return;
        }
        self.bind_context_input(&entry.params[0].name, contract, &entry.span);
    }

    pub(super) fn bind_context_input(
        &mut self,
        parameter: &str,
        contract: EntryContext,
        span: &Span,
    ) {
        fn input_value(ty: &ContextStruct, next: &mut usize) -> Value {
            let mut fields = HashMap::new();
            for field in &ty.fields {
                let mut component = || {
                    let value = Sx::Param(format!("fresco_context_input_{}", *next));
                    *next += 1;
                    value
                };
                let value = match &field.ty {
                    ContextType::Float => Value::Scalar(component()),
                    ContextType::Scalar(kind) => {
                        Value::Scalar(crate::typed_scalar::Scalar::input(component(), *kind))
                    }
                    ContextType::Vector(2) => Value::Vec2((component(), component())),
                    ContextType::Vector(3) => Value::Vec3((component(), component(), component())),
                    ContextType::Vector(4) => {
                        Value::Vec4((component(), component(), component(), component()))
                    }
                    ContextType::Vector(_) => unreachable!("validated context vector width"),
                    ContextType::Struct(nested) => input_value(nested, next),
                };
                fields.insert(field.name.clone(), value);
            }
            Value::Struct {
                ty_name: ty.name.clone(),
                fields,
            }
        }

        let value = input_value(&contract.ty, &mut 0);
        let mut roles = self.context_roles(&value, span);
        // Ambient sample inputs are distinct from immutable argument projections.
        if roles.contains_key("coord") {
            roles.insert("coord".to_string(), Value::Vec2((Sx::CoordX, Sx::CoordY)));
        }
        for (role, input) in [
            ("time", hir::EntryInput::Time),
            ("delta_time", hir::EntryInput::Delta),
        ] {
            if roles.contains_key(role) {
                roles.insert(role.to_string(), Value::Scalar(Sx::EntryInput(input)));
            }
        }
        if roles.contains_key("resolution") {
            roles.insert(
                "resolution".to_string(),
                Value::Vec2((
                    Sx::EntryInput(hir::EntryInput::ResolutionX),
                    Sx::EntryInput(hir::EntryInput::ResolutionY),
                )),
            );
        }
        self.evaluation_context = Some(roles);
        self.runtime_channel_cache = globals::RuntimeChannelCache::default();
        self.bind(parameter.to_string(), value);
        self.hir.entry_context = Some(contract);
    }

    pub(super) fn append_context_components(&mut self) {
        let Some(context) = &self.hir.entry_context else {
            return;
        };
        for component in &context.components {
            if self
                .hir
                .params
                .iter()
                .any(|param| param.name == component.name)
            {
                self.diags.push(Diag::error(
                    0..0,
                    format!("reserved context input name `{}`", component.name),
                ));
                continue;
            }
            self.hir.params.push(Param {
                name: component.name.clone(),
                ty_name: "f32".to_string(),
                default: ParamDefault::Scalar(0.0),
                min: None,
                max: None,
            });
        }
    }

    pub(super) fn context_roles(&mut self, value: &Value, span: &Span) -> HashMap<String, Value> {
        fn collect(
            checker: &mut Checker,
            value: &Value,
            span: &Span,
            out: &mut HashMap<String, Value>,
        ) {
            let Value::Struct { ty_name, fields } = value else {
                checker.diags.push(Diag::error(
                    span.clone(),
                    "@context requires a struct value",
                ));
                return;
            };
            let Some(definition) = checker.struct_defs.get(ty_name).cloned() else {
                return;
            };
            for (name, field) in &definition.fields {
                let Some(value) = fields.get(name) else {
                    continue;
                };
                if let Some(role) = &field.semantic {
                    let valid = match role.as_str() {
                        "coord" | "resolution" => matches!(value, Value::Vec2(_)),
                        "time" | "delta_time" => matches!(value, Value::Scalar(_)),
                        _ => false,
                    };
                    if !valid {
                        checker.diags.push(Diag::error(
                            field.span.clone(),
                            format!("unknown or incorrectly typed context semantic `{role}`"),
                        ));
                    } else if out.insert(role.clone(), value.clone()).is_some() {
                        checker.diags.push(Diag::error(
                            field.span.clone(),
                            format!("duplicate context semantic `{role}`"),
                        ));
                    }
                }
                if matches!(value, Value::Struct { .. }) {
                    collect(checker, value, span, out);
                }
            }
        }
        let mut roles = HashMap::new();
        collect(self, value, span, &mut roles);
        roles
    }

    pub(super) fn context_role(&mut self, role: &str, span: &Span) -> Option<Value> {
        if let Some(value) = self
            .evaluation_context
            .as_ref()
            .and_then(|roles| roles.get(role))
        {
            return Some(value.clone());
        }
        self.diags.push(
            Diag::error(
                span.clone(),
                format!("context semantic `{role}` is not available in this scope"),
            )
            .with_help("pass a struct with this @semantic role through an @context parameter"),
        );
        None
    }
}
