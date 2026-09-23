use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const INDENT: &str = "    ";

#[derive(Debug, Clone, Copy, Default, clap::ValueEnum, PartialEq, Eq)]
pub enum BinaryWrapStyle {
    #[default]
    Indent,
    AlignOperator,
    AlignOperand,
}

#[derive(Debug, Clone, Copy)]
pub struct FormatOptions {
    pub check: bool,
    pub stdout: bool,
    pub binary_wrap_style: BinaryWrapStyle,
}

pub fn run(paths: &[PathBuf], options: FormatOptions) -> ExitCode {
    let mut files = Vec::new();
    for path in paths {
        if let Err(msg) = collect_fr_files(path, &mut files) {
            eprintln!("error: {msg}");
            return ExitCode::FAILURE;
        }
    }

    if files.is_empty() {
        eprintln!("error: no .fr files found");
        return ExitCode::FAILURE;
    }

    if options.stdout && files.len() != 1 {
        eprintln!("error: --stdout requires exactly one .fr file");
        return ExitCode::FAILURE;
    }

    let mut changed = 0usize;
    let mut had_error = false;

    for file in files {
        let src = match fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot read {}: {e}", file.display());
                had_error = true;
                continue;
            }
        };

        let formatted = if matches!(options.binary_wrap_style, BinaryWrapStyle::Indent) {
            format_source(&src)
        } else {
            format_source_with_options(&src, options.binary_wrap_style)
        };
        if formatted != src {
            changed += 1;

            if options.check {
                eprintln!("needs formatting: {}", file.display());
                continue;
            }

            if options.stdout {
                print!("{formatted}");
                continue;
            }

            if let Err(e) = fs::write(&file, formatted) {
                eprintln!("error: cannot write {}: {e}", file.display());
                had_error = true;
                continue;
            }
            eprintln!("formatted {}", file.display());
        } else if options.stdout {
            print!("{formatted}");
        }
    }

    if had_error {
        return ExitCode::FAILURE;
    }

    if options.check && changed > 0 {
        eprintln!("{changed} file(s) need formatting");
        return ExitCode::FAILURE;
    }

    if options.check {
        eprintln!("all checked .fr files are formatted");
    }

    ExitCode::SUCCESS
}

fn collect_fr_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        if is_fr_file(path) {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }

    if !path.is_dir() {
        return Err(format!("path not found: {}", path.display()));
    }

    let entries =
        fs::read_dir(path).map_err(|e| format!("cannot read directory {}: {e}", path.display()))?;

    for entry in entries {
        let entry =
            entry.map_err(|e| format!("cannot read directory entry in {}: {e}", path.display()))?;
        let p = entry.path();
        if p.is_dir() {
            collect_fr_files(&p, out)?;
        } else if is_fr_file(&p) {
            out.push(p);
        }
    }

    out.sort();
    out.dedup();
    Ok(())
}

fn is_fr_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("fr"))
        .unwrap_or(false)
}

#[derive(Default)]
struct ScanStats {
    leading_closing_curly: usize,
    leading_closing_paren: usize,
    leading_closing_bracket: usize,
    open_curly: usize,
    close_curly: usize,
    open_paren: usize,
    close_paren: usize,
    open_bracket: usize,
    close_bracket: usize,
}

fn scan_line(line: &str) -> ScanStats {
    let mut s = ScanStats::default();
    let bytes = line.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] as char {
            ' ' | '\t' => i += 1,
            '}' => {
                s.leading_closing_curly += 1;
                i += 1;
            }
            ')' => {
                s.leading_closing_paren += 1;
                i += 1;
            }
            ']' => {
                s.leading_closing_bracket += 1;
                i += 1;
            }
            _ => break,
        }
    }

    let mut in_string = false;
    let mut escaped = false;
    i = 0;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if !in_string && ch == '/' && i + 1 < bytes.len() && bytes[i + 1] as char == '/' {
            break;
        }

        if in_string {
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => s.open_curly += 1,
            '}' => s.close_curly += 1,
            '(' => s.open_paren += 1,
            ')' => s.close_paren += 1,
            '[' => s.open_bracket += 1,
            ']' => s.close_bracket += 1,
            _ => {}
        }
        i += 1;
    }

    s
}

fn split_top_level(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for ch in input.chars() {
        if in_string {
            current.push(ch);
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' => {
                paren += 1;
                current.push(ch);
            }
            ')' => {
                paren = paren.saturating_sub(1);
                current.push(ch);
            }
            '[' => {
                bracket += 1;
                current.push(ch);
            }
            ']' => {
                bracket = bracket.saturating_sub(1);
                current.push(ch);
            }
            '{' => {
                brace += 1;
                current.push(ch);
            }
            '}' => {
                brace = brace.saturating_sub(1);
                current.push(ch);
            }
            ',' if paren == 0 && bracket == 0 && brace == 0 => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    out
}

fn find_matching_paren(input: &str) -> Option<usize> {
    let mut paren = 1usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '(' => paren += 1,
            ')' => {
                paren = paren.saturating_sub(1);
                if paren == 0 && bracket == 0 && brace == 0 {
                    return Some(idx);
                }
            }
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            _ => {}
        }
    }

    None
}

fn expand_inline_gradient_stops(normalized: &str) -> String {
    let mut out = Vec::new();

    for line in normalized.split('\n') {
        if !(line.contains("gradient(") && line.contains("stops: [")) {
            out.push(line.to_string());
            continue;
        }

        let Some(gradient_idx) = line.find("gradient(") else {
            out.push(line.to_string());
            continue;
        };

        let prefix = &line[..gradient_idx];
        let after_open = &line[gradient_idx + "gradient(".len()..];
        let Some(close_idx) = find_matching_paren(after_open) else {
            out.push(line.to_string());
            continue;
        };

        let body = &after_open[..close_idx];
        let suffix = &after_open[close_idx + 1..];
        let args = split_top_level(body);
        let stop_idx = args
            .iter()
            .position(|arg| arg.trim_start().starts_with("stops:"));
        let Some(stop_idx) = stop_idx else {
            out.push(line.to_string());
            continue;
        };

        let stops_arg = args[stop_idx].trim();
        let Some(colon_idx) = stops_arg.find(':') else {
            out.push(line.to_string());
            continue;
        };
        let stops_value = stops_arg[colon_idx + 1..].trim();
        if !(stops_value.starts_with('[') && stops_value.ends_with(']')) {
            out.push(line.to_string());
            continue;
        }

        let items_body = &stops_value[1..stops_value.len() - 1];
        let stop_items = split_top_level(items_body);
        if stop_items.is_empty() {
            out.push(line.to_string());
            continue;
        }

        out.push(format!("{prefix}gradient("));
        for (idx, arg) in args.iter().enumerate() {
            if idx == stop_idx {
                let has_following_args = idx + 1 < args.len();
                out.push("stops: [".to_string());
                for item in &stop_items {
                    out.push(format!("{item},"));
                }
                out.push(if has_following_args {
                    "],".to_string()
                } else {
                    "]".to_string()
                });
                continue;
            }

            let suffix_comma = if idx + 1 < args.len() { "," } else { "" };
            out.push(format!("{}{suffix_comma}", arg.trim()));
        }
        out.push(format!("){suffix}",));
    }

    out.join("\n")
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
fn find_top_level_equal(input: &str) -> Option<usize> {
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            '=' if paren == 0 && bracket == 0 && brace == 0 => return Some(idx),
            _ => {}
        }
    }

    None
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
fn split_top_level_shape_chain(input: &str) -> Option<(Vec<String>, Vec<char>)> {
    let mut parts = Vec::new();
    let mut ops = Vec::new();
    let mut current = String::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    let chars = input.chars().collect::<Vec<_>>();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            current.push(ch);
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            i += 1;
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' => {
                paren += 1;
                current.push(ch);
            }
            ')' => {
                paren = paren.saturating_sub(1);
                current.push(ch);
            }
            '[' => {
                bracket += 1;
                current.push(ch);
            }
            ']' => {
                bracket = bracket.saturating_sub(1);
                current.push(ch);
            }
            '{' => {
                brace += 1;
                current.push(ch);
            }
            '}' => {
                brace = brace.saturating_sub(1);
                current.push(ch);
            }
            '|' if paren == 0 && bracket == 0 && brace == 0 => {
                // Keep pipelines (`|>`) untouched.
                if i + 1 < chars.len() && chars[i + 1] == '>' {
                    current.push(ch);
                } else {
                    let trimmed = current.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    parts.push(trimmed.to_string());
                    ops.push('|');
                    current.clear();
                }
            }
            '&' if paren == 0 && bracket == 0 && brace == 0 => {
                let trimmed = current.trim();
                if trimmed.is_empty() {
                    return None;
                }
                parts.push(trimmed.to_string());
                ops.push('&');
                current.clear();
            }
            '-' if paren == 0 && bracket == 0 && brace == 0 => {
                // Skip arrows (`->`) and likely unary minus sites.
                if i + 1 < chars.len() && chars[i + 1] == '>' {
                    current.push(ch);
                } else {
                    let prev_non_ws = chars[..i]
                        .iter()
                        .rev()
                        .find(|c| !c.is_whitespace())
                        .copied();
                    let next_non_ws = chars[i + 1..].iter().find(|c| !c.is_whitespace()).copied();
                    let unary_site = prev_non_ws.is_none_or(|c| {
                        matches!(
                            c,
                            '(' | '[' | '{' | '=' | ':' | ',' | '+' | '-' | '*' | '/' | '|' | '&'
                        )
                    });
                    if unary_site || next_non_ws.is_none() {
                        current.push(ch);
                    } else {
                        let trimmed = current.trim();
                        if trimmed.is_empty() {
                            return None;
                        }
                        parts.push(trimmed.to_string());
                        ops.push('-');
                        current.clear();
                    }
                }
            }
            _ => current.push(ch),
        }

        i += 1;
    }

    let tail = current.trim();
    if tail.is_empty() {
        return None;
    }
    parts.push(tail.to_string());

    if parts.len() >= 2 && ops.len() + 1 == parts.len() {
        Some((parts, ops))
    } else {
        None
    }
}

fn is_fully_wrapped_parens(input: &str) -> bool {
    let trimmed = input.trim();
    if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
        return false;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    let mut paren = 1usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for ch in inner.chars() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '(' => paren += 1,
            ')' => {
                paren = paren.saturating_sub(1);
                if paren == 0 && (bracket > 0 || brace > 0) {
                    return false;
                }
            }
            '[' => bracket += 1,
            ']' => bracket = bracket.saturating_sub(1),
            '{' => brace += 1,
            '}' => brace = brace.saturating_sub(1),
            _ => {}
        }
    }

    paren == 1 && bracket == 0 && brace == 0 && !in_string
}

fn accumulate_delimiter_balance(input: &str, paren: &mut i32, bracket: &mut i32, brace: &mut i32) {
    let mut in_string = false;
    let mut escaped = false;
    let bytes = input.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if !in_string && ch == '/' && i + 1 < bytes.len() && bytes[i + 1] as char == '/' {
            break;
        }

        if in_string {
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            i += 1;
            continue;
        }

        match ch {
            '"' => in_string = true,
            '(' => *paren += 1,
            ')' => *paren -= 1,
            '[' => *bracket += 1,
            ']' => *bracket -= 1,
            '{' => *brace += 1,
            '}' => *brace -= 1,
            _ => {}
        }
        i += 1;
    }
}

fn normalize_parenthesized_shape_chain_blocks(normalized: &str) -> String {
    let lines = normalized.split('\n').collect::<Vec<_>>();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];
        let Some(eq_idx) = find_top_level_equal(line) else {
            out.push(line.to_string());
            i += 1;
            continue;
        };

        let lhs = line[..eq_idx].trim_end();
        let rhs = line[eq_idx + 1..].trim();
        if !rhs.starts_with('(') {
            out.push(line.to_string());
            i += 1;
            continue;
        }

        let mut paren = 0i32;
        let mut bracket = 0i32;
        let mut brace = 0i32;
        let mut block_parts = vec![rhs.to_string()];
        let mut block_lines = vec![line.to_string()];
        accumulate_delimiter_balance(rhs, &mut paren, &mut bracket, &mut brace);

        let mut j = i;
        while (paren > 0 || bracket > 0 || brace > 0) && j + 1 < lines.len() {
            j += 1;
            let next = lines[j];
            block_lines.push(next.to_string());
            let trimmed = next.trim();
            block_parts.push(trimmed.to_string());
            accumulate_delimiter_balance(trimmed, &mut paren, &mut bracket, &mut brace);
        }

        if paren != 0 || bracket != 0 || brace != 0 {
            out.extend(block_lines);
            i = j + 1;
            continue;
        }

        if block_parts.iter().any(|part| part.contains("//")) {
            out.extend(block_lines);
            i = j + 1;
            continue;
        }

        let rhs_block = block_parts.join(" ");
        if !is_fully_wrapped_parens(&rhs_block) {
            out.extend(block_lines);
            i = j + 1;
            continue;
        }

        let trimmed = rhs_block.trim();
        let rhs_inner = &trimmed[1..trimmed.len() - 1];
        let Some((parts, ops)) = split_top_level_shape_chain(rhs_inner) else {
            out.extend(block_lines);
            i = j + 1;
            continue;
        };

        if !ops.iter().any(|op| matches!(op, '|' | '&' | '-')) {
            out.extend(block_lines);
            i = j + 1;
            continue;
        }

        out.push(format!("{lhs} = ("));
        for idx in 0..parts.len() {
            if idx < ops.len() {
                out.push(format!("{} {}", parts[idx], ops[idx]));
            } else {
                out.push(parts[idx].to_string());
            }
        }
        out.push(")".to_string());
        i = j + 1;
    }

    out.join("\n")
}

fn split_top_level_dot_chain(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    let chars = input.chars().collect::<Vec<_>>();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            current.push(ch);
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            i += 1;
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' => {
                paren += 1;
                current.push(ch);
            }
            ')' => {
                paren = paren.saturating_sub(1);
                current.push(ch);
            }
            '[' => {
                bracket += 1;
                current.push(ch);
            }
            ']' => {
                bracket = bracket.saturating_sub(1);
                current.push(ch);
            }
            '{' => {
                brace += 1;
                current.push(ch);
            }
            '}' => {
                brace = brace.saturating_sub(1);
                current.push(ch);
            }
            '.' if paren == 0 && bracket == 0 && brace == 0 => {
                let next = chars.get(i + 1).copied();
                if next.is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                    current.clear();
                    current.push('.');
                } else {
                    current.push(ch);
                }
            }
            _ => current.push(ch),
        }

        i += 1;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    out
}

fn split_top_level_additive_chain(input: &str) -> Option<(Vec<String>, Vec<char>)> {
    let mut parts = Vec::new();
    let mut ops = Vec::new();
    let mut current = String::new();
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    let chars = input.chars().collect::<Vec<_>>();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            current.push(ch);
            if escaped {
                escaped = false;
                i += 1;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            i += 1;
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' => {
                paren += 1;
                current.push(ch);
            }
            ')' => {
                paren = paren.saturating_sub(1);
                current.push(ch);
            }
            '[' => {
                bracket += 1;
                current.push(ch);
            }
            ']' => {
                bracket = bracket.saturating_sub(1);
                current.push(ch);
            }
            '{' => {
                brace += 1;
                current.push(ch);
            }
            '}' => {
                brace = brace.saturating_sub(1);
                current.push(ch);
            }
            '+' | '-' if paren == 0 && bracket == 0 && brace == 0 => {
                if ch == '-' && i + 1 < chars.len() && chars[i + 1] == '>' {
                    current.push(ch);
                    i += 1;
                    continue;
                }

                let prev_non_ws = chars[..i]
                    .iter()
                    .rev()
                    .find(|c| !c.is_whitespace())
                    .copied();
                let next_non_ws = chars[i + 1..].iter().find(|c| !c.is_whitespace()).copied();
                let unary_site = prev_non_ws.is_none_or(|c| {
                    matches!(
                        c,
                        '(' | '[' | '{' | '=' | ':' | ',' | '+' | '-' | '*' | '/' | '%' | '|' | '&'
                    )
                });

                if unary_site || next_non_ws.is_none() {
                    current.push(ch);
                } else {
                    let trimmed = current.trim();
                    if trimmed.is_empty() {
                        return None;
                    }
                    parts.push(trimmed.to_string());
                    ops.push(ch);
                    current.clear();
                }
            }
            _ => current.push(ch),
        }

        i += 1;
    }

    let tail = current.trim();
    if tail.is_empty() {
        return None;
    }
    parts.push(tail.to_string());

    if parts.len() >= 2 && ops.len() + 1 == parts.len() {
        Some((parts, ops))
    } else {
        None
    }
}

fn leading_binary_operator_len(trimmed: &str) -> Option<usize> {
    if trimmed.starts_with("//") {
        return None;
    }

    if trimmed.starts_with("&&") || trimmed.starts_with("||") {
        return Some(2);
    }

    if trimmed.starts_with('+')
        || trimmed.starts_with('-')
        || trimmed.starts_with('*')
        || trimmed.starts_with('/')
        || trimmed.starts_with('%')
        || trimmed.starts_with('&')
        || (trimmed.starts_with('|') && !trimmed.starts_with("|>"))
    {
        return Some(1);
    }

    None
}

fn starts_with_binary_continuation(trimmed: &str) -> bool {
    leading_binary_operator_len(trimmed).is_some()
}

fn ends_with_binary_operator(trimmed: &str) -> bool {
    let line = trimmed.trim_end();
    line.ends_with("&&")
        || line.ends_with("||")
        || line.ends_with('+')
        || line.ends_with('-')
        || line.ends_with('*')
        || line.ends_with('/')
        || line.ends_with('%')
        || line.ends_with('&')
        || (line.ends_with('|') && !line.ends_with("|>"))
}

fn rhs_anchor_column(trimmed: &str, indent_cols: usize) -> Option<usize> {
    let eq_idx = find_top_level_equal(trimmed)?;
    let rhs = &trimmed[eq_idx + 1..];
    let rhs_ws = rhs.len() - rhs.trim_start().len();
    let rhs_non_ws = rhs.trim_start();
    if rhs_non_ws.is_empty() {
        return None;
    }

    Some(indent_cols + eq_idx + 1 + rhs_ws)
}

#[derive(Clone, Copy, Debug, Default)]
struct BinaryContinuationState {
    indent_curly: usize,
    rhs_anchor_col: Option<usize>,
}

fn wrap_long_space_transform_chains(normalized: &str) -> String {
    const WRAP_THRESHOLD: usize = 100;

    let mut out = Vec::new();
    for line in normalized.split('\n') {
        if line.len() <= WRAP_THRESHOLD {
            out.push(line.to_string());
            continue;
        }

        let trimmed = line.trim_start();
        if !trimmed.starts_with("in space ") {
            out.push(line.to_string());
            continue;
        }

        let Some(stripped) = trimmed.strip_suffix('{') else {
            out.push(line.to_string());
            continue;
        };

        let expr = stripped["in space ".len()..].trim();
        let parts = split_top_level_dot_chain(expr);
        if parts.len() < 2 {
            out.push(line.to_string());
            continue;
        }

        out.push(format!("in space {}", parts[0]));
        for (idx, part) in parts.iter().enumerate().skip(1) {
            if idx + 1 == parts.len() {
                out.push(format!("{} {{", part));
            } else {
                out.push(part.clone());
            }
        }
    }

    out.join("\n")
}

fn wrap_long_shape_combinations(normalized: &str) -> String {
    const WRAP_THRESHOLD: usize = 100;

    let mut out = Vec::new();
    for line in normalized.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.contains("//") || line.len() <= WRAP_THRESHOLD {
            out.push(line.to_string());
            continue;
        }

        let Some(eq_idx) = find_top_level_equal(line) else {
            out.push(line.to_string());
            continue;
        };

        let lhs = line[..eq_idx].trim_end();
        let rhs = line[eq_idx + 1..].trim();
        let rhs_inner = if is_fully_wrapped_parens(rhs) {
            let trimmed = rhs.trim();
            &trimmed[1..trimmed.len() - 1]
        } else {
            rhs
        };

        let Some((parts, ops)) = split_top_level_shape_chain(rhs_inner) else {
            out.push(line.to_string());
            continue;
        };

        // Only target lines that actually combine shape-like expressions.
        if !ops.iter().any(|op| matches!(op, '|' | '&' | '-')) {
            out.push(line.to_string());
            continue;
        }

        out.push(format!("{lhs} = ("));
        for idx in 0..parts.len() {
            if idx < ops.len() {
                out.push(format!("{} {}", parts[idx], ops[idx]));
            } else {
                out.push(parts[idx].to_string());
            }
        }
        out.push(")".to_string());
    }

    out.join("\n")
}

fn wrap_long_additive_assignments(normalized: &str) -> String {
    const WRAP_THRESHOLD: usize = 100;

    let mut out = Vec::new();
    for line in normalized.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.contains("//") || line.len() <= WRAP_THRESHOLD {
            out.push(line.to_string());
            continue;
        }

        let Some(eq_idx) = find_top_level_equal(line) else {
            out.push(line.to_string());
            continue;
        };

        let lhs = line[..eq_idx].trim_end();
        let rhs = line[eq_idx + 1..].trim();
        if rhs.contains('|') || rhs.contains('&') {
            out.push(line.to_string());
            continue;
        }

        let Some((parts, ops)) = split_top_level_additive_chain(rhs) else {
            out.push(line.to_string());
            continue;
        };

        if !ops.iter().any(|op| matches!(op, '+' | '-')) {
            out.push(line.to_string());
            continue;
        }

        out.push(format!("{lhs} = {}", parts[0]));
        for idx in 1..parts.len() {
            out.push(format!("{} {}", ops[idx - 1], parts[idx]));
        }
    }

    out.join("\n")
}

fn wrap_long_scatter_declarations(normalized: &str) -> String {
    const WRAP_THRESHOLD: usize = 100;

    let mut out = Vec::new();
    for line in normalized.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || line.len() <= WRAP_THRESHOLD {
            out.push(line.to_string());
            continue;
        }

        let Some(eq_idx) = find_top_level_equal(line) else {
            out.push(line.to_string());
            continue;
        };

        let lhs = line[..eq_idx].trim_end();
        let rhs = line[eq_idx + 1..].trim();
        if !rhs.starts_with("scatter ") {
            out.push(line.to_string());
            continue;
        }

        let Some(within_idx) = rhs.find(" within ") else {
            out.push(line.to_string());
            continue;
        };
        let Some(seed_idx) = rhs.find(" seed ") else {
            out.push(line.to_string());
            continue;
        };
        let Some(strategy_idx) = rhs.find(" strategy ") else {
            out.push(line.to_string());
            continue;
        };

        if !(within_idx < seed_idx && seed_idx < strategy_idx) {
            out.push(line.to_string());
            continue;
        }

        let count_segment = rhs[..within_idx].trim_end();
        let within_segment = rhs[within_idx + 1..seed_idx].trim();
        let seed_segment = rhs[seed_idx + 1..strategy_idx].trim();
        let strategy_segment = rhs[strategy_idx + 1..].trim();

        out.push(format!("{lhs} = {count_segment}"));
        out.push(within_segment.to_string());
        out.push(seed_segment.to_string());
        out.push(strategy_segment.to_string());
    }

    out.join("\n")
}

// Reflow standalone calls without mistaking declaration signatures or grouped
// expressions for calls. Nested argument expressions stay intact.
fn wrap_standalone_calls(source: &str) -> String {
    let lines: Vec<_> = source.split('\n').collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        let Some(open) = line.find('(') else {
            out.push(lines[index].to_string());
            index += 1;
            continue;
        };
        let name = line[..open].trim();
        let is_call = !name.is_empty()
            && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
            && name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || ch == '_');
        if !is_call || line.contains("//") {
            out.push(lines[index].to_string());
            index += 1;
            continue;
        }

        let mut call = line.to_string();
        let mut end = index;
        while find_matching_paren(&call[open + 1..]).is_none() && end + 1 < lines.len() {
            let next = lines[end + 1].trim();
            // Do not move line comments into an argument or consume another block.
            if next.contains("//") || next.contains('{') || next.contains('}') {
                break;
            }
            call.push('\n');
            call.push_str(next);
            end += 1;
        }
        let Some(close) = find_matching_paren(&call[open + 1..]) else {
            out.push(lines[index].to_string());
            index += 1;
            continue;
        };
        let close = open + 1 + close;
        let suffix = &call[close + 1..];
        let args = split_top_level(&call[open + 1..close]);
        if args.len() < 2 || !suffix.trim().is_empty() || (end == index && call.len() <= 100) {
            out.push(lines[index].to_string());
            index += 1;
            continue;
        }
        out.push(format!("{name}("));
        let trailing_comma = call[..close].trim_end().ends_with(',');
        for (arg_index, arg) in args.iter().enumerate() {
            let comma = if arg_index + 1 < args.len() || trailing_comma {
                ","
            } else {
                ""
            };
            out.push(format!("{arg}{comma}"));
        }
        out.push(")".to_string());
        index = end + 1;
    }
    out.join("\n")
}

fn separate_member_sections(source: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut sections = vec![None];
    let mut paren = 0usize;
    let mut bracket = 0usize;
    for line in source.split('\n') {
        let trimmed = line.trim();
        let stats = scan_line(trimmed);
        let section = if trimmed.starts_with("properties {") {
            Some("properties")
        } else if trimmed.starts_with("param ") {
            Some("param")
        } else if trimmed.starts_with("compose {") {
            Some("compose")
        } else {
            None
        };
        if paren == 0 && bracket == 0 {
            let previous = sections.last_mut().expect("root section always exists");
            if let Some(section) = section {
                if previous.is_some_and(|previous| previous != section) {
                    // Keep leading comments attached to the following section.
                    let mut insertion = out.len();
                    while insertion > 0 && out[insertion - 1].trim_start().starts_with("//") {
                        insertion -= 1;
                    }
                    if insertion > 0 && !out[insertion - 1].is_empty() {
                        out.insert(insertion, String::new());
                    }
                }
                *previous = Some(section);
            }
        }
        out.push(line.to_string());
        for _ in 0..stats.open_curly {
            sections.push(None);
        }
        for _ in 0..stats.close_curly {
            if sections.len() > 1 {
                sections.pop();
            }
        }
        paren = (paren + stats.open_paren).saturating_sub(stats.close_paren);
        bracket = (bracket + stats.open_bracket).saturating_sub(stats.close_bracket);
    }
    out.join("\n")
}

pub fn format_source(src: &str) -> String {
    format_source_with_options(src, BinaryWrapStyle::Indent)
}

pub fn format_source_with_options(src: &str, binary_wrap_style: BinaryWrapStyle) -> String {
    let normalized = src.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = expand_inline_gradient_stops(&normalized);
    let normalized = normalize_parenthesized_shape_chain_blocks(&normalized);
    let normalized = wrap_long_scatter_declarations(&normalized);
    let normalized = wrap_long_shape_combinations(&normalized);
    let normalized = wrap_long_additive_assignments(&normalized);
    let normalized = wrap_long_space_transform_chains(&normalized);
    let normalized = wrap_standalone_calls(&normalized);
    let normalized = separate_member_sections(&normalized);

    let mut out_lines = Vec::new();
    let mut depth_curly = 0usize;
    let mut depth_paren = 0usize;
    let mut depth_bracket = 0usize;
    let mut binary_state: Option<BinaryContinuationState> = None;

    for raw_line in normalized.split('\n') {
        let trimmed_end = raw_line.trim_end();
        if trimmed_end.trim().is_empty() {
            out_lines.push(String::new());
            binary_state = None;
            continue;
        }

        let trimmed = trimmed_end.trim_start();
        let stats = scan_line(trimmed);

        let indent_curly = depth_curly.saturating_sub(stats.leading_closing_curly);
        let indent_paren = depth_paren.saturating_sub(stats.leading_closing_paren);
        let indent_bracket = depth_bracket.saturating_sub(stats.leading_closing_bracket);
        let dot_continuation = usize::from(trimmed.starts_with('.'));
        let base_continuation = usize::from(indent_paren > 0) + indent_bracket + dot_continuation;
        let base_indent_depth = indent_curly + base_continuation;
        let current_anchor = binary_state
            .and_then(|state| (state.indent_curly == indent_curly).then_some(state.rhs_anchor_col));
        let binary_contextual = starts_with_binary_continuation(trimmed)
            && (indent_paren > 0
                || indent_bracket > 0
                || binary_state.is_some_and(|state| state.indent_curly == indent_curly));

        let default_indent_depth = base_indent_depth
            + usize::from(
                binary_contextual && matches!(binary_wrap_style, BinaryWrapStyle::Indent),
            );
        let default_indent_cols = default_indent_depth * INDENT.len();
        let indent_cols = if binary_contextual {
            match binary_wrap_style {
                BinaryWrapStyle::Indent => default_indent_cols,
                BinaryWrapStyle::AlignOperator => {
                    current_anchor.flatten().unwrap_or(default_indent_cols)
                }
                BinaryWrapStyle::AlignOperand => {
                    let op_len = leading_binary_operator_len(trimmed).unwrap_or(1);
                    current_anchor
                        .flatten()
                        .map(|col| col.saturating_sub(op_len + 1))
                        .unwrap_or(default_indent_cols)
                }
            }
        } else {
            default_indent_cols
        };

        let mut line = String::new();
        for _ in 0..indent_cols {
            line.push(' ');
        }
        line.push_str(trimmed);
        out_lines.push(line);

        let assignment_anchor = rhs_anchor_column(trimmed, indent_cols);
        let carries_binary_chain =
            binary_contextual || assignment_anchor.is_some() || ends_with_binary_operator(trimmed);
        binary_state = if carries_binary_chain {
            Some(BinaryContinuationState {
                indent_curly,
                rhs_anchor_col: assignment_anchor.or(current_anchor.flatten()),
            })
        } else {
            None
        };

        depth_curly += stats.open_curly;
        depth_curly = depth_curly.saturating_sub(stats.close_curly);

        depth_paren += stats.open_paren;
        depth_paren = depth_paren.saturating_sub(stats.close_paren);

        depth_bracket += stats.open_bracket;
        depth_bracket = depth_bracket.saturating_sub(stats.close_bracket);
    }

    while out_lines.last().is_some_and(String::is_empty) {
        out_lines.pop();
    }

    let mut out = out_lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::{BinaryWrapStyle, format_source, format_source_with_options};

    #[test]
    fn separates_surface_sections_and_expands_material_arguments() {
        let src = "surface style_sample(sp: surf) -> material(standard) {\n    properties { style: Toon }\n    param tint: color = #f5ad69\n    param roughness_level: f32 = 0.45\n    param metallic_level: f32 = 0.0\n    param glow: f32 = 0.0\n    compose {\n        base(albedo: tint, roughness: clamp(roughness_level, 0.045, 1.0),\n             metallic: clamp(metallic_level, 0.0, 1.0), emissive: tint.rgb * max(glow, 0.0))\n    }\n}\n";
        let got = format_source(src);
        assert!(got.contains("properties { style: Toon }\n\n    param tint:"));
        assert!(got.contains("param glow: f32 = 0.0\n\n    compose {"));
        assert!(got.contains("        base(\n            albedo: tint,\n            roughness: clamp(roughness_level, 0.045, 1.0),\n            metallic: clamp(metallic_level, 0.0, 1.0),\n            emissive: tint.rgb * max(glow, 0.0)\n        )"));
        assert!(!got.contains('\t'));
        assert_eq!(format_source(&got), got);
        let compact = |text: &str| {
            text.chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>()
        };
        assert_eq!(compact(src), compact(&got));
        let single_line = src.replace(",\n             metallic:", ", metallic:");
        assert_eq!(format_source(&single_line), got);
        let opening_line = single_line.replace("base(albedo:", "base(\nalbedo:");
        assert_eq!(format_source(&opening_line), got);
    }

    #[test]
    fn call_wrapping_preserves_nested_arguments_strings_and_trailing_comma() {
        let src = "compose {\nshade(label: \"a, b (c)\",\npoints: [(1.0, 2.0), (3.0, 4.0)], value: clamp(x, 0.0, 1.0),)\n}\n";
        let got = format_source(src);
        assert!(got.contains("        label: \"a, b (c)\",\n"));
        assert!(got.contains("        points: [(1.0, 2.0), (3.0, 4.0)],\n"));
        assert!(got.contains("        value: clamp(x, 0.0, 1.0),\n    )"));
        assert_eq!(format_source(&got), got);
    }

    #[test]
    fn preserves_short_calls_and_comments() {
        let src = "compose {\n    base(albedo: tint, roughness: 0.5)\n    base(albedo: tint, // keep this with albedo\n        roughness: 0.5)\n}\n";
        assert_eq!(format_source(src), src);
    }

    #[test]
    fn call_wrapping_preserves_nested_multiline_layout() {
        let src = "compose {\nshade(points: [\n(1.0, 2.0),\n(3.0, 4.0)\n],\ntint: #fff)\n}\n";
        let got = format_source(src);
        assert!(got.contains("        points: [\n            (1.0, 2.0),\n            (3.0, 4.0)\n        ],\n        tint: #fff\n    )"));
        assert_eq!(format_source(&got), got);
    }

    #[test]
    fn separates_multiline_properties_and_keeps_section_comments_attached() {
        let src = "surface sample(sp: surf) -> material(standard) {\nproperties {\nstyle: Toon\n}\n// Tint control\nparam tint: color = #fff\n// Material\ncompose {\nbase(albedo: tint)\n}\n}\n";
        let got = format_source(src);
        assert!(got.contains("    }\n\n    // Tint control\n    param tint:"));
        assert!(got.contains("#fff\n\n    // Material\n    compose {"));
        assert_eq!(format_source(&got), got);
    }

    #[test]
    fn formats_nested_blocks_and_lists() {
        let src = "canvas t(uv: coord, time: signal) -> color {\ncompose {\nfill(gradient(\nalong: y,\nstops: [\nstop(at: 0.0, color: #000000),\nstop(at: 1.0, color: #ffffff)\n]\n))\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas t(uv: coord, time: signal) -> color {\n    compose {\n        fill(gradient(\n            along: y,\n            stops: [\n                stop(at: 0.0, color: #000000),\n                stop(at: 1.0, color: #ffffff)\n            ]\n        ))\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn expands_inline_gradient_stops_arrays() {
        let src = "canvas t(uv: coord, time: signal) -> color {\ncompose {\nhills |> fill(gradient(along: y, stops: [stop(at: 0.0, color: #0b1f14), stop(at: 1.0, color: #2f5a3b)]))\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas t(uv: coord, time: signal) -> color {\n    compose {\n        hills |> fill(gradient(\n            along: y,\n            stops: [\n                stop(at: 0.0, color: #0b1f14),\n                stop(at: 1.0, color: #2f5a3b),\n            ]\n        ))\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn keeps_comments_and_trims_trailing_whitespace() {
        let src = "canvas a(uv: coord, time: signal) -> color {    \n// comment\ncompose {\nfill(#ffffff)    \n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    // comment\n    compose {\n        fill(#ffffff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn normalizes_newline_style() {
        let src = "canvas a(uv: coord, time: signal) -> color {\r\ncompose {\r\nfill(#ffffff)\r\n}\r\n}\r\n";
        let got = format_source(src);
        assert!(got.contains('\n'));
        assert!(!got.contains('\r'));
    }

    #[test]
    fn preserves_comment_slashes_inside_string_literals() {
        let src = "canvas a(uv: coord, time: signal) -> color {\nlet x = note(\"http://x\")\ncompose {\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    let x = note(\"http://x\")\n    compose {\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn keeps_single_trailing_newline() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose {\nfill(#fff)\n}\n}\n\n\n";
        let got = format_source(src);
        assert!(got.ends_with('\n'));
        assert!(!got.ends_with("\n\n"));
    }

    #[test]
    fn handles_leading_closers() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose {\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    compose {\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn can_format_no_final_newline() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose {\nfill(#fff)\n}\n}";
        let got = format_source(src);
        assert!(got.ends_with('\n'));
    }

    #[test]
    fn check_mode_detects_changes() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose {\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        assert_ne!(src, got);
    }

    #[test]
    fn keeps_already_formatted_source() {
        let src = "canvas a(uv: coord, time: signal) -> color {\n    compose {\n        fill(#fff)\n    }\n}\n";
        let got = format_source(src);
        assert_eq!(src, got);
    }

    #[test]
    fn keeps_parentheses_continuation_indented() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose {\nfill(gradient(\nalong: (1.0,\n1.0),\nstops: [\nstop(at: 0.0, color: #000),\nstop(at: 1.0, color: #fff)\n]\n))\n}\n}\n";
        let got = format_source(src);
        assert!(got.contains("            1.0),"));
    }

    #[test]
    fn formats_comment_only_lines() {
        let src =
            "canvas a(uv: coord, time: signal) -> color {\n//x\ncompose {\n//y\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    //x\n    compose {\n        //y\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn retains_blank_lines_inside_blocks() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose {\n\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    compose {\n\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn handles_trailing_comment_with_brace() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose { // { in comment\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    compose { // { in comment\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn wraps_long_shape_combination_lines() {
        let src = "canvas a(uv: coord, time: signal) -> color {\nlet snow = mountain & box(at: (0.72, 0.73), size: (0.48, 0.085)) - circle(at: (0.61, 0.705), radius: 0.034) - circle(at: (0.70, 0.698), radius: 0.030) - circle(at: (0.79, 0.707), radius: 0.033)\ncompose {\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    let snow = (\n        mountain &\n        box(at: (0.72, 0.73), size: (0.48, 0.085)) -\n        circle(at: (0.61, 0.705), radius: 0.034) -\n        circle(at: (0.70, 0.698), radius: 0.030) -\n        circle(at: (0.79, 0.707), radius: 0.033)\n    )\n    compose {\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn keeps_short_shape_combination_lines() {
        let src = "canvas a(uv: coord, time: signal) -> color {\nlet mountain = (mountain_l & mountain_r) & mountain_cap\ncompose {\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    let mountain = (mountain_l & mountain_r) & mountain_cap\n    compose {\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn normalizes_multiline_parenthesized_shape_chain() {
        let src = "canvas a(uv: coord, time: signal) -> color {\nlet cloud = (capsule(from: (0.36, 0.67), to: (0.50, 0.67), radius: 0.027) |\n    capsule(from: (0.40, 0.62), to: (0.74, 0.62), radius: 0.020) |\n    (box(at: (0.58, 0.60), size: (0.34, 0.04)) |> round(0.02)))\ncompose {\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    let cloud = (\n        capsule(from: (0.36, 0.67), to: (0.50, 0.67), radius: 0.027) |\n        capsule(from: (0.40, 0.62), to: (0.74, 0.62), radius: 0.020) |\n        (box(at: (0.58, 0.60), size: (0.34, 0.04)) |> round(0.02))\n    )\n    compose {\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn wraps_long_in_space_transform_chain() {
        let src = "canvas a(uv: coord, time: signal) -> color {\ncompose {\nin space perspective(fov: 58deg, near: 0.02, far: 12.0, origin: center).rotate_x(angle: -8deg, around: center).rotate_y(angle: spin, around: center).translate3(x: 0.0, y: 0.0, z: 0.03) {\nfill(#fff)\n}\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    compose {\n        in space perspective(fov: 58deg, near: 0.02, far: 12.0, origin: center)\n            .rotate_x(angle: -8deg, around: center)\n            .rotate_y(angle: spin, around: center)\n            .translate3(x: 0.0, y: 0.0, z: 0.03) {\n            fill(#fff)\n        }\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn indents_operator_led_assignment_continuations() {
        let src = "let chrome = env\n+ (1.0, 1.0, 1.0) * spec * 1.2\n+ mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n+ (1.0, 0.4, 0.7) * horizon_ref\n";
        let got = format_source(src);
        let expected = "let chrome = env\n    + (1.0, 1.0, 1.0) * spec * 1.2\n    + mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n    + (1.0, 0.4, 0.7) * horizon_ref\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn does_not_indent_operator_line_without_context() {
        let src =
            "canvas a(uv: coord, time: signal) -> color {\ncompose {\n+ stray\nfill(#fff)\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas a(uv: coord, time: signal) -> color {\n    compose {\n        + stray\n        fill(#fff)\n    }\n}\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn aligns_operator_continuations_when_enabled() {
        let src = "let chrome = env\n+ (1.0, 1.0, 1.0) * spec * 1.2\n+ mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n";
        let got = format_source_with_options(src, BinaryWrapStyle::AlignOperator);
        let expected = "let chrome = env\n             + (1.0, 1.0, 1.0) * spec * 1.2\n             + mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn aligns_operand_continuations_when_enabled() {
        let src = "let chrome = env\n+ (1.0, 1.0, 1.0) * spec * 1.2\n+ mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n";
        let got = format_source_with_options(src, BinaryWrapStyle::AlignOperand);
        let expected = "let chrome = env\n           + (1.0, 1.0, 1.0) * spec * 1.2\n           + mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn wraps_long_additive_assignment_chains() {
        let src = "let chrome = env + (1.0, 1.0, 1.0) * spec * 1.2 + mix(c_magenta, c_cyan, 0.3) * fres * 0.7 + (1.0, 0.4, 0.7) * horizon_ref\n";
        let got = format_source(src);
        let expected = "let chrome = env\n    + (1.0, 1.0, 1.0) * spec * 1.2\n    + mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n    + (1.0, 0.4, 0.7) * horizon_ref\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn wraps_long_additive_chains_with_align_operator_style() {
        let src = "let chrome = env + (1.0, 1.0, 1.0) * spec * 1.2 + mix(c_magenta, c_cyan, 0.3) * fres * 0.7 + (1.0, 0.4, 0.7) * horizon_ref\n";
        let got = format_source_with_options(src, BinaryWrapStyle::AlignOperator);
        let expected = "let chrome = env\n             + (1.0, 1.0, 1.0) * spec * 1.2\n             + mix(c_magenta, c_cyan, 0.3) * fres * 0.7\n             + (1.0, 0.4, 0.7) * horizon_ref\n";
        assert_eq!(got, expected);
    }

    #[test]
    fn wraps_long_scatter_declarations() {
        let src = "canvas t(uv: coord, time: signal, res: resolution) -> color {\nlet stars = scatter 180 within region((0.5 - 0.5 * aspect, 0.0) .. (0.5 + 0.5 * aspect, 1.0)) seed 19 strategy procedural {\ncompose {\nstars\n}\n}\n";
        let got = format_source(src);
        let expected = "canvas t(uv: coord, time: signal, res: resolution) -> color {\n    let stars = scatter 180\n    within region((0.5 - 0.5 * aspect, 0.0) .. (0.5 + 0.5 * aspect, 1.0))\n    seed 19\n    strategy procedural {\n        compose {\n            stars\n        }\n    }\n";
        assert_eq!(got, expected);
    }
}
