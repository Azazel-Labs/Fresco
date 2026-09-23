//! Contextual editor keywords derived from reachable engine interfaces.
use super::loaders::{MemModuleLoader, ModuleLoader};
use crate::{
    lexer::{self, Token},
    parser,
};
use chumsky::Parser as _;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// Best-effort highlighting works even while the effect is syntactically incomplete.
/// Registration metadata uses the actual parser and the compiler's virtual loader.
pub fn engine_keyword_spans(
    source: &str,
    filename: &str,
    files: &HashMap<String, String>,
) -> Vec<Range<usize>> {
    let loader = MemModuleLoader {
        files: files.clone(),
    };
    let root = loader.root_canonical_key(filename);
    let mut pending = vec![(root.clone(), source.to_string())];
    if let Ok(modules) = loader.implicit_engine_modules(&root) {
        pending.extend(modules);
    }
    let mut visited = HashSet::new();
    let mut entries = HashMap::<String, HashSet<String>>::new();
    while let Some((path, text)) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let tokens = lexer::lex_spanned(&text);
        // Imports remain discoverable when an unfinished body prevents parsing.
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
            for interface in program.templates {
                if let Some(block) = interface.property_block {
                    entries.entry("surface".into()).or_default().insert(block);
                }
                if let Some((name, _)) = interface.entry {
                    entries
                        .entry(name)
                        .or_default()
                        .extend(interface.plugs.into_iter().map(|method| method.name));
                }
            }
        }
    }
    let tokens: Vec<_> = lexer::lex_spanned(source)
        .into_iter()
        .filter(|(token, _)| !matches!(token, Token::Newline))
        .collect();
    let mut depth = 0usize;
    let mut current = None;
    let mut spans = Vec::new();
    for (i, (token, span)) in tokens.iter().enumerate() {
        let name = match token {
            Token::Ident(name) => Some(name.as_str()),
            Token::Canvas => Some("canvas"),
            Token::Surface => Some("surface"),
            _ => None,
        };
        if let Some(name) = name {
            let next = tokens.get(i + 1).map(|(token, _)| token);
            if depth == 0 && entries.contains_key(name) && matches!(next, Some(Token::Ident(_))) {
                spans.push(span.clone());
                current = entries.get(name);
            } else if depth == 1
                && current.is_some_and(|methods| methods.contains(name))
                && matches!(next, Some(Token::LBrace))
            {
                spans.push(span.clone());
            }
        }
        match token {
            Token::LBrace => depth += 1,
            Token::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    current = None;
                }
            }
            _ => {}
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words(source: &str, files: &HashMap<String, String>) -> Vec<String> {
        engine_keyword_spans(source, "main.fr", files)
            .into_iter()
            .map(|span| source[span].to_string())
            .collect()
    }
    #[test]
    fn derives_contextual_keywords_from_transitive_engine_imports() {
        let files = HashMap::from([
            ("engine/engine.fr".into(), r#"import "./parts/../contract.fr""#.into()),
            ("engine/contract.fr".into(), "@entry(swarm, tick) interface System { fn birth(p: f32) -> f32\n fn tick(p: f32) -> f32 }".into()),
            ("unused.fr".into(), "@entry(ghost, tick) interface Ghost { fn tick(p: f32) -> f32 }".into()),
        ]);
        let source = "// \u{1f31f} swarm ghost\nswarm sparks { birth { let swarm = 1; } tick { birth(); } }\nghost nope {}";
        assert_eq!(words(source, &files), ["swarm", "birth", "tick"]);
        assert_eq!(
            words("swarm unfinished { tick {", &files),
            ["swarm", "tick"]
        );
        assert!(words("fn swarm() -> f32 { let tick = 1; return tick }", &files).is_empty());
        assert!(words("swarm x {}", &HashMap::new()).is_empty());
    }
}
