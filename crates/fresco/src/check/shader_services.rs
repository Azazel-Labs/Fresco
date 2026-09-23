//! Nominal, explicitly exported pass-local shader interfaces.
//!
//! An export describes callable helpers, not a stage entry or a scheduled draw.
//! Check exports even on passes absent from the selected renderer recipe.
use std::collections::BTreeSet;

use crate::ast::{PassDecl, Program};
use crate::diag::Diag;

fn compact(ty: &str) -> String {
    ty.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) fn validate(pass: &PassDecl, program: &Program, diags: &mut Vec<Diag>) {
    let mut exports = BTreeSet::new();
    for export in pass.attrs.iter().filter(|attr| attr.name == "service") {
        let check = || -> Result<(), String> {
            let [name] = export.args.as_slice() else {
                return Err("@service requires exactly one interface name".into());
            };
            let interfaces: Vec<_> = program
                .interfaces
                .iter()
                .filter(|i| i.name == *name)
                .collect();
            let [interface] = interfaces.as_slice() else {
                return Err(format!(
                    "shader service `{name}` must name one declared interface"
                ));
            };
            if interface.entry.is_some() {
                return Err(format!(
                    "entry interface `{name}` cannot be exported as a shader service"
                ));
            }
            if interface.methods.is_empty() {
                return Err(format!(
                    "shader service `{name}` must declare at least one method"
                ));
            }
            let mut methods = BTreeSet::new();
            for method in &interface.methods {
                if !methods.insert(&method.name) {
                    return Err(format!(
                        "shader service `{name}` has duplicate method `{}`",
                        method.name
                    ));
                }
                if method
                    .params
                    .iter()
                    .any(|p| p.is_context || p.keyword_only || p.default.is_some())
                {
                    return Err(format!(
                        "shader service method `{name}.{}` requires explicit positional parameters",
                        method.name
                    ));
                }
                let matching: Vec<_> =
                    pass.hooks
                        .iter()
                        .filter(|hook| {
                            hook.name == method.name
                                && hook.params.len() == method.params.len()
                                && hook.params.iter().zip(&method.params).all(
                                    |(actual, expected)| {
                                        compact(&actual.ty_name) == compact(&expected.ty_name)
                                    },
                                )
                        })
                        .collect();
                let [hook] = matching.as_slice() else {
                    return Err(format!(
                        "pass `{}` must implement shader service method `{name}.{}` with exactly one matching signature",
                        pass.name, method.name
                    ));
                };
                if hook.return_ty.as_ref().map(|ty| compact(&ty.node))
                    != method.ret_ty.as_ref().map(|(ty, _)| compact(ty))
                {
                    return Err(format!(
                        "shader service method `{name}.{}` has an incompatible return type",
                        method.name
                    ));
                }
                if !hook.attrs.is_empty()
                    || hook.params.iter().any(|p| !p.attrs.is_empty())
                    || hook.dispatch.is_some()
                {
                    return Err(format!(
                        "shader service method `{name}.{}` must be an ordinary helper with explicit arguments, not a stage entry, evaluator, or dispatch hook",
                        method.name
                    ));
                }
                crate::parser::pass_hook_body(hook).map_err(|errors| {
                    format!(
                        "invalid shader service method `{name}.{}` body: {errors:?}",
                        method.name
                    )
                })?;
            }
            Ok(())
        };
        let result = if !exports.insert(export.args.clone()) {
            Err("duplicate shader service export".into())
        } else {
            check()
        };
        if let Err(message) = result {
            diags.push(super::pass_error(pass, export.span.clone(), message));
        }
    }
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
        crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .expect("valid syntax")
    }

    fn check(source: &str) -> Result<(), Vec<Diag>> {
        super::super::validate_pipeline_skeleton_decls(&parse(source))
    }

    const SOURCE: &str = "interface Illumination { fn sample(position: vec3) -> vec3 }\n@service(Illumination) pass library {\nfn sample(p: vec3) -> vec3 { return p }\n}";

    #[test]
    fn exported_helpers_need_no_pipeline_and_parameter_names_are_local() {
        check(SOURCE).unwrap();
        check(&SOURCE.replace("\n}", "\nfn sample(p: f32) -> f32 { return p }\n}")).unwrap();
    }

    #[test]
    fn unused_exports_still_require_exact_method_signatures() {
        for (old, new) in [
            ("fn sample(p: vec3)", "fn missing(p: vec3)"),
            ("fn sample(p: vec3)", "fn sample(p: vec2)"),
            ("-> vec3 { return p }", "-> f32 { return 0.0 }"),
            ("@service(Illumination)", "@service(Missing)"),
            ("@service(Illumination)", "@service(Illumination, Extra)"),
        ] {
            assert!(check(&SOURCE.replace(old, new)).is_err(), "{new}");
        }
    }

    #[test]
    fn entry_points_and_implicit_arguments_are_not_service_methods() {
        for (old, new) in [
            (
                "interface Illumination",
                "@entry(example, sample) interface Illumination",
            ),
            ("fn sample(p:", "@fragment fn sample(p:"),
            ("fn sample(p:", "fn sample(@builtin(position) p:"),
            ("fn sample(position:", "fn sample(@context position:"),
        ] {
            assert!(check(&SOURCE.replace(old, new)).is_err(), "{new}");
        }
    }

    #[test]
    fn duplicate_exports_and_ambiguous_implementations_are_rejected() {
        assert!(
            check(&SOURCE.replace(
                "@service(Illumination)",
                "@service(Illumination)\n@service(Illumination)"
            ))
            .is_err()
        );
        // The parser already rejects identical argument signatures. Exercise
        // merged/generated ASTs too: return types cannot disambiguate a call.
        for return_type in ["vec3", "f32"] {
            let mut program = parse(SOURCE);
            let pass = &mut program.passes[0];
            let mut duplicate = pass.hooks[0].clone();
            duplicate.return_ty.as_mut().unwrap().node = return_type.into();
            pass.hooks.push(duplicate);
            let errors = super::super::validate_pipeline_skeleton_decls(&program).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|error| error.message.contains("exactly one matching signature"))
            );
        }
    }
}
