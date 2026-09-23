//! Editor signatures from reachable engine material schemas.
use super::loaders::{MemModuleLoader, ModuleLoader};
use crate::{
    ast::MaterialPropertiesDecl,
    language_docs::{DocCallable, DocCallableArg},
    lexer::{self, Token},
    parser,
};
use chumsky::Parser as _;
use std::collections::{HashMap, HashSet};

/// Best-effort metadata for an incomplete surface. Invalid or ambiguous engine
/// contracts do not manufacture a conventional material signature.
pub fn material_callables(
    source: &str,
    cursor: usize,
    filename: &str,
    files: &HashMap<String, String>,
) -> Vec<DocCallable> {
    let loader = MemModuleLoader {
        files: files.clone(),
    };
    let root = loader.root_canonical_key(filename);
    let mut pending = vec![(root.clone(), source.to_owned())];
    if let Ok(modules) = loader.implicit_engine_modules(&root) {
        pending.extend(modules);
    }
    let mut root_program = None;
    let mut imported_templates = Vec::new();
    let mut imported_enums = Vec::new();
    let mut visited = HashSet::new();
    let mut schemas = HashMap::<String, MaterialPropertiesDecl>::new();
    let mut response_profiles = HashMap::new();
    while let Some((path, text)) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let tokens = lexer::lex_spanned(&text);
        for pair in tokens.windows(2) {
            if let (Token::Import, Token::Str(import)) = (&pair[0].0, &pair[1].0)
                && let Ok(module) = loader.resolve_import(import, &path)
            {
                pending.push(module);
            }
        }
        if let Some(program) = parser::program()
            .parse(parser::input(&tokens, text.len()..text.len()))
            .into_output()
        {
            for response in &program.schema_expressions {
                response_profiles.insert(response.name.clone(), response.properties_name.clone());
            }
            for schema in &program.material_properties {
                if schemas
                    .insert(schema.name.clone(), schema.clone())
                    .is_some()
                {
                    return Vec::new();
                }
            }
            if path == root {
                root_program = Some(program);
            } else {
                imported_templates.extend(program.templates);
                imported_enums.extend(program.enums);
            }
        }
    }
    let resolved_profile = if let Some(mut program) = root_program {
        program.templates.extend(imported_templates);
        program.enums.extend(imported_enums);
        program.material_properties = schemas.values().cloned().collect();
        if super::super::surface_properties::resolve(&mut program).is_err() {
            return Vec::new();
        }
        program
            .surfaces
            .iter()
            .find(|surface| surface.span.start <= cursor && cursor <= surface.span.end)
            .and_then(|surface| match &surface.material_ty {
                crate::ast::MaterialReturnTy::Named(name) => Some(name.clone()),
                crate::ast::MaterialReturnTy::Default => None,
            })
    } else {
        None
    };
    let tokens = lexer::lex_spanned(source);
    let Some(start) = tokens
        .iter()
        .rposition(|(token, span)| matches!(token, Token::Surface) && span.start <= cursor)
    else {
        return Vec::new();
    };
    let header = &tokens[start..];
    let header_end = header
        .iter()
        .position(|(token, _)| matches!(token, Token::LBrace))
        .unwrap_or(header.len());
    let profile = header[..header_end].windows(3).find_map(|triple| {
        if matches!(&triple[0].0, Token::Ident(name) if name == "material")
            && matches!(triple[1].0, Token::LParen)
            && let Token::Ident(name) = &triple[2].0
        {
            return Some(name.as_str());
        }
        None
    });
    let defaults = schemas
        .values()
        .filter(|schema| schema.is_default)
        .collect::<Vec<_>>();
    let selected = match resolved_profile.as_deref().or(profile) {
        Some(name) => schemas.get(name).or_else(|| {
            response_profiles
                .get(name)
                .and_then(|profile| schemas.get(profile))
        }),
        None if defaults.len() == 1 => Some(defaults[0]),
        None => None,
    };
    let Some(selected) = selected else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(selected);
    while let Some(schema) = current {
        if !seen.insert(&schema.name) {
            return Vec::new();
        }
        chain.push(schema);
        current = match &schema.extends_name {
            Some(name) => match schemas.get(name) {
                Some(parent) => Some(parent),
                None => return Vec::new(),
            },
            None => None,
        };
    }
    let composition = chain
        .iter()
        .find_map(|schema| schema.composition.as_ref())
        .or_else(|| {
            if defaults.len() == 1 {
                defaults[0].composition.as_ref()
            } else {
                None
            }
        });
    let Some([initialize, layer, weight]) = composition else {
        return Vec::new();
    };
    let mut channels = Vec::new();
    for schema in chain.iter().rev() {
        for channel in &schema.channels {
            if let Some(index) =
                channels
                    .iter()
                    .position(|existing: &&crate::ast::MaterialChannelDecl| {
                        existing.name == channel.name
                    })
            {
                channels[index] = channel;
            } else {
                channels.push(channel);
            }
        }
    }
    [initialize, layer]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut args = channels
                .iter()
                .map(|channel| DocCallableArg {
                    name: channel.name.clone(),
                    ty: channel.ty_name.clone(),
                    value_kind: "expr".into(),
                    required: index == 0 && channel.default.is_none(),
                    docs: format!("Channel declared by engine material `{}`.", selected.name),
                })
                .collect::<Vec<_>>();
            if index == 1 {
                args.push(DocCallableArg {
                    name: weight.clone(),
                    ty: "f32".into(),
                    value_kind: "expr".into(),
                    required: false,
                    docs: "Weight passed to engine channel blend functions.".into(),
                });
            }
            DocCallable {
                name: name.clone(),
                summary: format!("Engine composition operation for `{}`.", selected.name),
                context: "surface-compose".into(),
                args,
                returns: vec![selected.name.clone()],
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_uses_engine_names_inheritance_and_requiredness() {
        let files = HashMap::from([(
            "engine.fr".into(),
            String::from(
                "@composition(initialize, coat, coverage)\nmaterial_properties parent { channel radiance: vec3 }\n@default_material\nmaterial_properties thermal extends parent { channel temperature: f32 = 0 }",
            ),
        )]);
        let source =
            "import \"engine.fr\"\nsurface effect(sp: Sample) -> material { compose { initialize(";
        let calls = material_callables(source, source.len(), "main.fr", &files);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "initialize");
        assert_eq!(calls[0].args[0].name, "radiance");
        assert_eq!(calls[0].args[0].ty, "vec3");
        assert!(calls[0].args[0].required);
        assert!(!calls[0].args[1].required);
        assert_eq!(calls[1].name, "coat");
        assert!(calls[1].args.iter().all(|arg| !arg.required));
        assert_eq!(calls[1].args[2].name, "coverage");
    }

    #[test]
    fn absent_engine_does_not_advertise_material_channels() {
        let source = "surface effect(sp: Sample) -> material { compose { base(";
        assert!(material_callables(source, source.len(), "main.fr", &HashMap::new()).is_empty());
    }
}
