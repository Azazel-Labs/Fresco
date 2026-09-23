//! Engine-authored rendering policy. Suggestions in diagnostics never supply values.

use super::*;

const REQUIRED: [(&str, &str); 4] = [
    ("check.shape_aa_min_px", "1.5"),
    ("check.shape_aa_max_px", "3.0"),
    ("check.shape_aa_style", "gradient"),
    ("check.projective_footprint_max_px", "64.0"),
];

pub(super) fn apply(
    base: &CompileContext,
    resolved: &ResolvedProgram,
    filename: &str,
    src: &str,
) -> Result<CompileContext, Vec<FileDiagnostics>> {
    let mut context = base.clone();
    let mut declarations = BTreeMap::new();
    let mut errors = Vec::new();
    for (file, pragmas) in &resolved.engine_pragmas {
        let source = &resolved.sources[file];
        for pragma in pragmas {
            if REQUIRED.iter().any(|(key, _)| *key == pragma.key)
                && let Some((previous, _)) =
                    declarations.insert(pragma.key.as_str(), (file, pragma))
            {
                errors.push(FileDiagnostics::one(
                    file,
                    source,
                    vec![
                        Diag::error(
                            pragma.span.clone(),
                            format!("duplicate engine setting `{}`", pragma.key),
                        )
                        .with_help(format!(
                            "declare this setting once; already declared in `{previous}`"
                        )),
                    ],
                ));
            }
        }
        let program = Program {
            pragmas: pragmas.clone(),
            ..Program::default()
        };
        match apply_program_pragmas(&context, &program, file, source, false) {
            Ok(applied) => context = applied.context,
            Err(mut diags) => errors.append(&mut diags),
        }
    }

    if resolved.program.root_entries().next().is_some() {
        for (key, suggestion) in REQUIRED {
            if !declarations.contains_key(key) {
                errors.push(FileDiagnostics::one(filename, src, vec![
                    Diag::error(0..0, format!("missing required engine setting `{key}`"))
                        .with_help(format!("declare `#pragma {key} = {suggestion}` in engine/engine.fr or a module it imports (suggested starting value; no implicit default). API/config values and entry-file pragmas do not supply the engine declaration")),
                ]));
            }
        }
    }

    if errors.is_empty()
        && let Err(message) = context.validate()
    {
        let setting = message.split_whitespace().next().unwrap_or("");
        let (file, source, span) =
            declarations
                .get(setting)
                .map_or((filename, src, 0..0), |(file, pragma)| {
                    (
                        file.as_str(),
                        resolved.sources[*file].as_str(),
                        pragma.value_span.clone(),
                    )
                });
        errors.push(FileDiagnostics::one(file, source, vec![
            Diag::error(span, format!("invalid engine configuration: {message}"))
                .with_help("use finite positive AA bounds with min <= max, and a finite positive projective footprint cap; suggested starting values: min = 1.5, max = 3.0, cap = 64.0"),
        ]));
    }
    if errors.is_empty() {
        Ok(context)
    } else {
        Err(errors)
    }
}
