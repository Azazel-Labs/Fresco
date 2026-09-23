#[path = "support/common.rs"]
mod common;

use common::{ExampleCompilePolicy, collect_example_files_with_policies, repo_root};
use fresco::driver::{self, VisualizerKind};

#[derive(Debug, Clone)]
struct Binding {
    name: String,
    span_start: usize,
    span_end: usize,
    expr_text: String,
    is_param: bool,
}

#[derive(Debug, Clone)]
struct VizDirective {
    kind: String,
    domain: String,
}

#[derive(Debug, Clone)]
struct VizAnnotation {
    name: String,
    span_start: usize,
    span_end: usize,
    kind: String,
    domain: String,
    is_param: bool,
}

fn is_ident_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_'
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn parse_identifier(bytes: &[u8], mut i: usize) -> Option<(String, usize)> {
    if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return None;
    }
    let start = i;
    i += 1;
    while i < bytes.len() && is_ident_char(bytes[i]) {
        i += 1;
    }
    let ident = std::str::from_utf8(&bytes[start..i]).ok()?.to_string();
    Some((ident, i))
}

fn parse_binding(line: &str, line_start_byte: usize) -> Option<Binding> {
    let comment_idx = line.find("//").unwrap_or(line.len());
    let code = &line[..comment_idx];
    let bytes = code.as_bytes();
    let i0 = skip_ws(bytes, 0);

    let parse_let_or_space = |kw: &str| -> Option<Binding> {
        let kw_bytes = kw.as_bytes();
        if bytes.len() < i0 + kw_bytes.len() + 1 {
            return None;
        }
        if &bytes[i0..i0 + kw_bytes.len()] != kw_bytes {
            return None;
        }
        let mut i = i0 + kw_bytes.len();
        if i >= bytes.len() || !bytes[i].is_ascii_whitespace() {
            return None;
        }
        i = skip_ws(bytes, i);
        let (name, mut i) = parse_identifier(bytes, i)?;
        i = skip_ws(bytes, i);
        if i >= bytes.len() || bytes[i] != b'=' {
            return None;
        }
        i += 1;
        i = skip_ws(bytes, i);
        let mut end = code.len();
        while end > i && code.as_bytes()[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        Some(Binding {
            name,
            span_start: line_start_byte + i,
            span_end: line_start_byte + end,
            expr_text: code[i..end].to_string(),
            is_param: false,
        })
    };

    if let Some(binding) = parse_let_or_space("let") {
        return Some(binding);
    }
    if let Some(binding) = parse_let_or_space("space") {
        return Some(binding);
    }

    if bytes.len() < i0 + 6
        || &bytes[i0..i0 + 5] != b"param"
        || !bytes[i0 + 5].is_ascii_whitespace()
    {
        return None;
    }
    let mut i = skip_ws(bytes, i0 + 5);
    let (name, after_name) = parse_identifier(bytes, i)?;
    i = after_name;
    i = skip_ws(bytes, i);
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }

    let name_start = code.find(&name)?;
    let name_end = name_start + name.len();
    Some(Binding {
        name,
        span_start: line_start_byte + name_start,
        span_end: line_start_byte + name_end,
        expr_text: String::new(),
        is_param: true,
    })
}

fn split_directive_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut prev = '\0';

    for ch in text.chars() {
        if (ch == '"' || ch == '\'') && prev != '\\' {
            quote = match quote {
                None => Some(ch),
                Some(q) if q == ch => None,
                Some(q) => Some(q),
            };
            current.push(ch);
            prev = ch;
            continue;
        }

        if ch == ',' && quote.is_none() {
            let token = current.trim();
            if !token.is_empty() {
                tokens.push(token.to_string());
            }
            current.clear();
            prev = ch;
            continue;
        }

        current.push(ch);
        prev = ch;
    }

    let token = current.trim();
    if !token.is_empty() {
        tokens.push(token.to_string());
    }

    tokens
}

fn strip_quotes(value: &str) -> String {
    let text = value.trim();
    if text.len() >= 2 {
        let bytes = text.as_bytes();
        let first = bytes[0];
        let last = bytes[text.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return text[1..text.len() - 1].to_string();
        }
    }
    text.to_string()
}

fn parse_viz_directive(line: &str) -> Option<(VizDirective, bool)> {
    let comment_idx = line.find("//")?;
    let comment = line[comment_idx + 2..].trim_start();
    let lower = comment.to_ascii_lowercase();

    let marker = if lower.starts_with("@viz") {
        "@viz"
    } else if lower.starts_with("@visualizer") {
        "@visualizer"
    } else {
        return None;
    };

    let mut rest = &comment[marker.len()..];
    let mut option_text = String::new();

    if rest.trim_start().starts_with('(') {
        let trim_ws = rest.trim_start();
        let mut depth = 0usize;
        let mut end_idx = None;
        for (idx, ch) in trim_ws.char_indices() {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    end_idx = Some(idx);
                    break;
                }
            }
        }
        let close = end_idx?;
        option_text.push_str(&trim_ws[1..close]);
        rest = &trim_ws[close + 1..];
    }

    let trailing = rest.trim();
    if !trailing.is_empty() {
        if !option_text.is_empty() {
            option_text.push(' ');
        }
        option_text.push_str(trailing);
    }

    let mut kind = String::new();
    let mut domain = String::new();

    for token in split_directive_tokens(&option_text) {
        let token_trim = token.trim();
        if token_trim.is_empty() {
            continue;
        }
        if let Some(eq_idx) = token_trim.find('=') {
            let key = token_trim[..eq_idx].trim().to_ascii_lowercase();
            let value = strip_quotes(token_trim[eq_idx + 1..].trim());
            let normalized = value.to_ascii_lowercase();
            if key == "kind" || key == "mode" {
                kind = match normalized.as_str() {
                    "timeseries" | "chart" | "sparkline" => "timeseries".to_string(),
                    "thumbnail" | "preview" | "thumb" => "thumbnail".to_string(),
                    "color" | "swatch" => "swatch".to_string(),
                    _ => normalized,
                };
            } else if key == "domain"
                && (normalized == "time" || normalized == "x" || normalized == "thumb")
            {
                domain = normalized;
            }
        } else {
            let normalized = token_trim.to_ascii_lowercase();
            if normalized == "time" || normalized == "x" || normalized == "thumb" {
                domain = normalized;
            } else if normalized == "timeseries" || normalized == "chart" {
                kind = "timeseries".to_string();
            } else if normalized == "thumbnail" || normalized == "preview" {
                kind = "thumbnail".to_string();
            }
        }
    }

    let directive_only_line = line.trim_start().starts_with("//");
    Some((VizDirective { kind, domain }, directive_only_line))
}

fn detect_auto_domain(expr_text: &str) -> String {
    let lower = expr_text.to_ascii_lowercase();
    if lower.contains("time") {
        return "time".to_string();
    }

    let cues = [
        "period:",
        "cycle:",
        "frequency:",
        "freq:",
        "hz:",
        "rate:",
        "every:",
        "over:",
        "window:",
        "duration:",
        "span:",
        "horizon:",
    ];

    if cues.iter().any(|cue| lower.contains(cue)) {
        return "time".to_string();
    }

    "x".to_string()
}

fn resolve_domain(directive_domain: &str, expr_text: &str) -> String {
    if directive_domain == "time" || directive_domain == "x" || directive_domain == "thumb" {
        return directive_domain.to_string();
    }
    detect_auto_domain(expr_text)
}

fn scan_viz_annotations(source: &str) -> Vec<VizAnnotation> {
    let mut out = Vec::new();
    let mut byte_offset = 0usize;
    let mut pending_directive: Option<VizDirective> = None;
    let mut pending_binding: Option<Binding> = None;

    for line in source.split('\n') {
        let line_start = byte_offset;
        let directive = parse_viz_directive(line);
        let binding = parse_binding(line, line_start);

        match (directive, binding) {
            (Some((directive, _directive_only)), Some(binding)) => {
                let domain = resolve_domain(&directive.domain, &binding.expr_text);
                out.push(VizAnnotation {
                    name: binding.name,
                    span_start: binding.span_start,
                    span_end: binding.span_end,
                    kind: directive.kind,
                    domain,
                    is_param: binding.is_param,
                });
                pending_directive = None;
                pending_binding = None;
            }
            (None, Some(binding)) => {
                if let Some(directive) = pending_directive.take() {
                    let domain = resolve_domain(&directive.domain, &binding.expr_text);
                    out.push(VizAnnotation {
                        name: binding.name,
                        span_start: binding.span_start,
                        span_end: binding.span_end,
                        kind: directive.kind,
                        domain,
                        is_param: binding.is_param,
                    });
                    pending_binding = None;
                } else {
                    pending_binding = Some(binding);
                }
            }
            (Some((directive, directive_only_line)), None) => {
                if directive_only_line {
                    if let Some(binding) = pending_binding.take() {
                        let domain = resolve_domain(&directive.domain, &binding.expr_text);
                        out.push(VizAnnotation {
                            name: binding.name,
                            span_start: binding.span_start,
                            span_end: binding.span_end,
                            kind: directive.kind,
                            domain,
                            is_param: binding.is_param,
                        });
                        pending_directive = None;
                    } else {
                        pending_directive = Some(directive);
                    }
                } else {
                    pending_directive = None;
                    pending_binding = None;
                }
            }
            (None, None) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with("//") {
                    pending_directive = None;
                    pending_binding = None;
                }
            }
        }

        byte_offset += line.len() + 1;
    }

    out
}

fn resolve_visualizer_kind_and_domain(
    annotation: &VizAnnotation,
    semantic_type: &str,
) -> (VisualizerKind, &'static str) {
    let explicit_kind = annotation.kind.to_ascii_lowercase();

    if annotation.domain == "thumb" || explicit_kind == "thumbnail" {
        return (VisualizerKind::Thumbnail, "thumb");
    }

    if semantic_type == "color" || explicit_kind == "swatch" {
        return (VisualizerKind::Swatch, "time");
    }

    if semantic_type == "scalar"
        || semantic_type == "f32"
        || semantic_type == "vec2"
        || semantic_type == "vec2<f32>"
        || explicit_kind == "timeseries"
        || explicit_kind == "chart"
    {
        let domain = if annotation.domain == "x" {
            "x"
        } else {
            "time"
        };
        return (VisualizerKind::Sparkline, domain);
    }

    (VisualizerKind::Thumbnail, "thumb")
}

fn validate_wgsl_module(label: &str, wgsl: &str) -> Result<(), String> {
    let module =
        naga::front::wgsl::parse_str(wgsl).map_err(|err| format!("{label} parse failed: {err}"))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    validator
        .validate(&module)
        .map_err(|err| format!("{label} validation failed: {err:#?}"))?;
    Ok(())
}

#[test]
fn referenced_example_visualizers_compile_to_valid_wgsl() {
    let root = repo_root();
    let examples_dir = root.join("examples");
    // Use a complete explicit virtual project, including library siblings that
    // are not independently runnable example entrypoints.
    let mut sources = fresco_example_engine::source_files();
    let mut directories = vec![examples_dir.clone()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).expect("example directory") {
            let path = entry.expect("example entry").path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|ext| ext == "fr") {
                sources.insert(
                    path.strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    std::fs::read_to_string(path).expect("example source"),
                );
            }
        }
    }

    let mut all_files = Vec::new();
    let mut files_with_policy = Vec::new();
    collect_example_files_with_policies(&examples_dir, &mut all_files, &mut files_with_policy);

    let example_files: Vec<_> = files_with_policy
        .into_iter()
        .filter(|(_, policy)| *policy == ExampleCompilePolicy::MustCompile)
        .map(|(path, _)| path)
        .collect();

    let mut failures = Vec::new();
    let mut visualizer_count = 0usize;

    for file in example_files {
        let rel = file
            .strip_prefix(&root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| file.display().to_string());

        let source = match std::fs::read_to_string(&file) {
            Ok(src) => src,
            Err(err) => {
                failures.push(format!("{rel}\nfailed to read source: {err}"));
                continue;
            }
        };

        let filename = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let annotations = scan_viz_annotations(&source);
        if annotations.is_empty() {
            continue;
        }

        for annotation in annotations {
            if annotation.is_param {
                continue;
            }
            visualizer_count += 1;
            let span_info = driver::query_span_info_with_files(
                &source,
                &filename,
                annotation.span_start,
                annotation.span_end,
                Some(&sources),
            );
            let semantic_type = span_info
                .as_ref()
                .map(|r| r.kind.as_str().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let (viz_kind, viz_domain) =
                resolve_visualizer_kind_and_domain(&annotation, &semantic_type);
            match driver::compile_visualizer_with_files(
                &source,
                &filename,
                (annotation.span_start, annotation.span_end),
                viz_kind,
                viz_domain,
                4.0,
                Some(&sources),
            ) {
                Ok(compiled) => {
                    if let Err(err) = validate_wgsl_module("draw_wgsl", &compiled.draw_wgsl) {
                        failures.push(format!(
                            "{rel}\nvisualizer `{}` at {}..{} emitted invalid draw WGSL: {}",
                            annotation.name, annotation.span_start, annotation.span_end, err
                        ));
                        continue;
                    }
                    if let Err(err) = validate_wgsl_module("reduce_wgsl", &compiled.reduce_wgsl) {
                        failures.push(format!(
                            "{rel}\nvisualizer `{}` at {}..{} emitted invalid reduce WGSL: {}",
                            annotation.name, annotation.span_start, annotation.span_end, err
                        ));
                    }
                }
                Err(diags) => {
                    let diagnostic_text = diags
                        .iter()
                        .map(|d| d.message.clone())
                        .collect::<Vec<_>>()
                        .join("\n");
                    failures.push(format!(
                        "{rel}\nvisualizer `{}` at {}..{} (kind={}, domain={}, semantic={}) failed:\n{}",
                        annotation.name,
                        annotation.span_start,
                        annotation.span_end,
                        annotation.kind,
                        annotation.domain,
                        semantic_type,
                        diagnostic_text
                    ));
                }
            }
        }
    }

    assert!(
        visualizer_count > 0,
        "expected at least one example visualizer annotation under {}",
        examples_dir.display()
    );

    assert!(
        failures.is_empty(),
        "{} visualizer compile failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n====\n\n")
    );
}
