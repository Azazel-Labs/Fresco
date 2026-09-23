use serde::Deserialize;
use std::collections::HashMap;

mod bridge;
mod draw;
mod merge;
mod reduce;
mod uniforms;

const MAX_RUNTIME_PARAMS: usize = 16;
const PREVIEW_UNIFORM_PARAM_VECS: usize = MAX_RUNTIME_PARAMS.div_ceil(4);
pub(super) const VIZ_SERIES_SAMPLES: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualizerKind {
    Sparkline,
    Swatch,
    Thumbnail,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualizerSpec<'a> {
    pub kind: VisualizerKind,
    pub domain: &'a str,
    pub sweep_max: f32,
    pub semantic_type: &'a str,
    pub range_hint: Option<(f32, f32)>,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualizerMetadata {
    pub kind: String,
    pub domain: String,
    pub sweep_max: f32,
    pub x_axis_label: Option<String>,
    pub y_axis_label: Option<String>,
    pub fit_mode: Option<String>,
    pub y_min_hint: Option<f32>,
    pub y_max_hint: Option<f32>,
    pub preview_label: Option<String>,
    pub preview_detail: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualizerShaderSet {
    pub draw_wgsl: String,
    pub reduce_wgsl: String,
}

#[derive(Debug, Clone)]
struct EntryParam {
    name: String,
    ty: String,
}

#[derive(Debug, Clone)]
struct ManifestParamDef {
    name: String,
    ty: String,
}

#[derive(Debug, Deserialize)]
struct ManifestRoot {
    canvases: Vec<ManifestCanvas>,
}

#[derive(Debug, Deserialize)]
struct ManifestCanvas {
    name: String,
    params: Vec<ManifestParam>,
}

#[derive(Debug, Deserialize)]
struct ManifestParam {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Debug, Clone)]
struct CallOptions<'a> {
    uv_expr: &'a str,
    time_expr: &'a str,
    delta_expr: &'a str,
    res_expr: &'a str,
}

pub(crate) fn build_shaders(
    variant_wgsl: &str,
    manifest_text: &str,
    spec: &VisualizerSpec<'_>,
    context: Option<&crate::context::EntryContext>,
) -> Result<VisualizerShaderSet, String> {
    let fn_name = serde_json::from_str::<ManifestRoot>(manifest_text)
        .ok()
        .and_then(|manifest| {
            manifest
                .canvases
                .first()
                .map(|canvas| format!("fresco_{}", canvas.name))
        })
        .or_else(|| detect_fresco_entry_function(variant_wgsl))
        .ok_or_else(|| "no fresco entry function found".to_string())?;
    let call_args = build_viz_call_args(
        variant_wgsl,
        &fn_name,
        manifest_text,
        context,
        CallOptions {
            uv_expr: "sample_uv",
            time_expr: "sample_t",
            delta_expr: "u._pad0.x",
            res_expr: "u.res",
        },
    )?;

    let domain = match spec.domain {
        "time" => "time",
        "x" | "thumb" => "x",
        other => return Err(format!("unsupported visualizer domain `{other}`")),
    };
    let sweep_max = if spec.sweep_max.is_finite() && spec.sweep_max > 0.0 {
        spec.sweep_max.max(0.0001)
    } else if domain == "time" {
        4.0
    } else {
        1.0
    };
    let mode = match spec.kind {
        VisualizerKind::Sparkline => draw::DrawMode::Sparkline,
        VisualizerKind::Swatch => draw::DrawMode::Swatch,
        VisualizerKind::Thumbnail => draw::DrawMode::Thumbnail,
    };
    let draw_wgsl = draw::build_draw_shader(
        variant_wgsl,
        &fn_name,
        &call_args,
        domain,
        sweep_max,
        mode,
        spec.semantic_type,
    )?;
    let reduce_wgsl = reduce::build_reduce_shader(
        variant_wgsl,
        &fn_name,
        &call_args,
        domain,
        sweep_max,
        spec.semantic_type,
        spec.range_hint,
        spec.kind == VisualizerKind::Thumbnail,
    )?;
    Ok(VisualizerShaderSet {
        draw_wgsl,
        reduce_wgsl,
    })
}

#[cfg(test)]
pub(crate) fn build_shader(
    variant_wgsl: &str,
    manifest_text: &str,
    spec: &VisualizerSpec<'_>,
) -> Result<String, String> {
    build_shaders(variant_wgsl, manifest_text, spec, None).map(|set| set.draw_wgsl)
}

pub(crate) fn build_metadata(
    spec: &VisualizerSpec<'_>,
    expr_text: Option<&str>,
) -> VisualizerMetadata {
    let resolved_sweep_max = if spec.sweep_max.is_finite() && spec.sweep_max > 0.0 {
        spec.sweep_max.max(0.0001)
    } else if spec.domain == "time" {
        4.0
    } else if spec.domain == "x" {
        1.0
    } else {
        0.0
    };
    let range_hint = spec
        .range_hint
        .or_else(|| extract_range_hint(expr_text.unwrap_or_default()));

    match spec.kind {
        VisualizerKind::Sparkline => VisualizerMetadata {
            kind: "sparkline".to_string(),
            domain: spec.domain.to_string(),
            sweep_max: resolved_sweep_max,
            x_axis_label: Some(if spec.domain == "time" { "time" } else { "x" }.to_string()),
            y_axis_label: Some("value".to_string()),
            fit_mode: Some("auto-range".to_string()),
            y_min_hint: range_hint.map(|(lo, _)| lo),
            y_max_hint: range_hint.map(|(_, hi)| hi),
            preview_label: None,
            preview_detail: None,
        },
        VisualizerKind::Swatch => VisualizerMetadata {
            kind: "swatch".to_string(),
            domain: "time".to_string(),
            sweep_max: resolved_sweep_max,
            x_axis_label: Some("time".to_string()),
            y_axis_label: Some("color".to_string()),
            fit_mode: None,
            y_min_hint: None,
            y_max_hint: None,
            preview_label: None,
            preview_detail: None,
        },
        VisualizerKind::Thumbnail => {
            build_thumbnail_metadata(spec.semantic_type, expr_text.unwrap_or_default())
        }
    }
}

fn build_thumbnail_metadata(semantic_type: &str, expr_text: &str) -> VisualizerMetadata {
    if semantic_type == "space" {
        let transform_labels = collect_space_transform_labels(expr_text);
        let summary = format_space_transform_summary(&transform_labels);
        let preview_detail = format_space_preview_detail(&transform_labels);
        return VisualizerMetadata {
            kind: "thumbnail".to_string(),
            domain: "thumb".to_string(),
            sweep_max: 0.0,
            x_axis_label: Some("space probe".to_string()),
            y_axis_label: Some("space".to_string()),
            fit_mode: Some("space-probe".to_string()),
            y_min_hint: None,
            y_max_hint: None,
            preview_label: Some(format!("space {summary}")),
            preview_detail: Some(preview_detail),
        };
    }

    if semantic_type == "shape" {
        let shape_labels = collect_shape_preview_labels(expr_text);
        let summary = format_shape_preview_summary(&shape_labels);
        let preview_detail = format_shape_preview_detail(expr_text, &shape_labels);
        return VisualizerMetadata {
            kind: "thumbnail".to_string(),
            domain: "thumb".to_string(),
            sweep_max: 0.0,
            x_axis_label: Some("shape probe".to_string()),
            y_axis_label: Some("shape".to_string()),
            fit_mode: Some("shape-probe".to_string()),
            y_min_hint: None,
            y_max_hint: None,
            preview_label: Some(format!("shape {summary}")),
            preview_detail: Some(preview_detail),
        };
    }

    VisualizerMetadata {
        kind: "thumbnail".to_string(),
        domain: "thumb".to_string(),
        sweep_max: 0.0,
        x_axis_label: None,
        y_axis_label: None,
        fit_mode: None,
        y_min_hint: None,
        y_max_hint: None,
        preview_label: None,
        preview_detail: None,
    }
}

fn collect_shape_preview_labels(expr_text: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let mut saw_constructor = false;
    let bytes = expr_text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = index;
            index += 1;
            while index < bytes.len() {
                let next = bytes[index] as char;
                if next.is_ascii_alphanumeric() || next == '_' {
                    index += 1;
                } else {
                    break;
                }
            }
            let ident = &expr_text[start..index];
            let mut probe = index;
            while probe < bytes.len() && (bytes[probe] as char).is_ascii_whitespace() {
                probe += 1;
            }
            if probe < bytes.len()
                && bytes[probe] as char == '('
                && let Some(label) = map_shape_preview_label(ident, saw_constructor)
            {
                if is_shape_constructor_name(ident) {
                    saw_constructor = true;
                }
                labels.push(label);
            }
        } else {
            index += 1;
        }
    }
    labels.dedup();
    labels
}

fn is_shape_constructor_name(name: &str) -> bool {
    matches!(
        name,
        "box"
            | "circle"
            | "ring"
            | "line"
            | "polyline"
            | "sector"
            | "arc"
            | "superellipse"
            | "capsule"
            | "ellipse"
            | "star"
            | "triangle"
            | "polygon"
            | "cross"
            | "diamond"
            | "svg"
            | "svg_path"
    )
}

fn map_shape_preview_label(name: &str, saw_constructor: bool) -> Option<String> {
    match name {
        "box" | "circle" | "ring" | "line" | "polyline" | "sector" | "arc" | "superellipse"
        | "capsule" | "ellipse" | "star" | "triangle" | "polygon" | "cross" | "diamond" => {
            Some(name.replace('_', "-"))
        }
        "svg" => Some("svg-shape".to_string()),
        "svg_path" => Some("svg-path".to_string()),
        "round" | "dilate" | "erode" | "smooth" if saw_constructor => Some(name.replace('_', "-")),
        _ => None,
    }
}

fn format_shape_preview_summary(labels: &[String]) -> String {
    if labels.is_empty() {
        return "probe".to_string();
    }
    const MAX_LABELS: usize = 3;
    let mut parts = labels.iter().take(MAX_LABELS).cloned().collect::<Vec<_>>();
    if labels.len() > MAX_LABELS {
        parts.push(format!("+{}", labels.len() - MAX_LABELS));
    }
    parts.join(" -> ")
}

fn format_shape_preview_detail(expr_text: &str, labels: &[String]) -> String {
    let mut markers = vec!["sdf".to_string(), "silhouette".to_string()];
    if labels.iter().any(|label| label == "round") {
        markers.push("rounded".to_string());
    }
    if labels.iter().any(|label| label == "smooth") {
        markers.push("smooth-union".to_string());
    }
    if labels
        .iter()
        .any(|label| label == "dilate" || label == "erode")
    {
        markers.push("offset".to_string());
    }
    if expr_text.contains(" | ") {
        markers.push("union".to_string());
    }
    if expr_text.contains(" & ") {
        markers.push("intersect".to_string());
    }
    if expr_text.contains(" - ") {
        markers.push("subtract".to_string());
    }
    format!(
        "legend: cyan=inside, white=edge, hue=edge-direction; probe={}",
        markers.join(", ")
    )
}

fn collect_space_transform_labels(expr_text: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let bytes = expr_text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if ch.is_ascii_alphabetic() || ch == '_' {
            let start = index;
            index += 1;
            while index < bytes.len() {
                let next = bytes[index] as char;
                if next.is_ascii_alphanumeric() || next == '_' {
                    index += 1;
                } else {
                    break;
                }
            }
            let ident = &expr_text[start..index];
            let mut probe = index;
            while probe < bytes.len() && (bytes[probe] as char).is_ascii_whitespace() {
                probe += 1;
            }
            if probe < bytes.len()
                && bytes[probe] as char == '('
                && let Some(label) = map_space_transform_label(ident, expr_text)
            {
                labels.push(label);
            }
        } else {
            index += 1;
        }
    }
    labels.dedup();
    labels
}

fn map_space_transform_label(name: &str, expr_text: &str) -> Option<String> {
    match name {
        "centered" => Some(format!(
            "centered({})",
            if expr_text.contains("aspect: fill") {
                "fill"
            } else if expr_text.contains("aspect: fit") {
                "fit"
            } else {
                "preserve"
            }
        )),
        "repeat_x" => Some("repeat-x".to_string()),
        "repeat_y" => Some("repeat-y".to_string()),
        "repeat_radial" => Some("repeat-radial".to_string()),
        "rotate_x" => Some("rotate-x".to_string()),
        "rotate_y" => Some("rotate-y".to_string()),
        "translate3" => Some("translate-3d".to_string()),
        "orientation" => Some(if expr_text.contains("y: down") {
            "orientation(y-down)".to_string()
        } else {
            "orientation(y-up)".to_string()
        }),
        "rotate" | "translate" | "scale" | "perspective" | "aspect" | "polar" | "warp" => {
            Some(name.replace('_', "-"))
        }
        _ => None,
    }
}

fn format_space_transform_summary(labels: &[String]) -> String {
    if labels.is_empty() {
        return "probe".to_string();
    }
    const MAX_LABELS: usize = 3;
    let mut parts = labels.iter().take(MAX_LABELS).cloned().collect::<Vec<_>>();
    if labels.len() > MAX_LABELS {
        parts.push(format!("+{}", labels.len() - MAX_LABELS));
    }
    parts.join(" -> ")
}

fn format_space_preview_detail(labels: &[String]) -> String {
    let mut markers = vec![
        "before-left".to_string(),
        "after-right".to_string(),
        "distortion-field".to_string(),
        "warp-grid".to_string(),
        "frame-axes".to_string(),
    ];
    if labels.iter().any(|label| label.starts_with("repeat-")) {
        markers.push("repeat-markers".to_string());
    }
    if labels.iter().any(|label| label == "polar") {
        markers.push("polar-spokes".to_string());
    }
    if labels.iter().any(|label| {
        label == "perspective"
            || label == "rotate-x"
            || label == "rotate-y"
            || label == "translate-3d"
    }) {
        markers.push("depth-stack".to_string());
    }
    if labels.iter().any(|label| label == "warp") {
        markers.push("warp-grid".to_string());
    }
    format!(
        "legend: left=before, right=after, warm=distortion, seam=split; probe={}",
        markers.join(", ")
    )
}

fn extract_range_hint(expr_text: &str) -> Option<(f32, f32)> {
    let idx = expr_text.find("range")?;
    let tail = &expr_text[idx..];
    let colon_idx = tail.find(':')?;
    let after_colon = trim_top_level_expr_end(tail[colon_idx + 1..].trim_start());

    if let Some((lo_expr, hi_expr)) = split_top_level_range(after_colon) {
        let lo = eval_scalar_hint_expr(lo_expr.trim())?;
        let hi = eval_scalar_hint_expr(trim_top_level_expr_end(hi_expr).trim())?;
        return Some((lo, hi));
    }

    None
}

pub(crate) fn extract_range_hint_for_visualizer(expr_text: &str) -> Option<(f32, f32)> {
    extract_range_hint(expr_text)
}

fn trim_top_level_expr_end(text: &str) -> &str {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut end = text.len();
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    end = idx;
                    break;
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                end = idx;
                break;
            }
            _ => {}
        }
    }
    let _ = bytes;
    &text[..end]
}

fn split_top_level_range(text: &str) -> Option<(&str, &str)> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(pair) = split_top_level_range_inner(trimmed) {
        return Some(pair);
    }

    if (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('(') && trimmed.ends_with(')'))
    {
        let inner = &trimmed[1..trimmed.len().saturating_sub(1)];
        return split_top_level_range_inner(inner);
    }

    None
}

fn split_top_level_range_inner(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut idx = 0usize;
    while idx + 1 < bytes.len() {
        match bytes[idx] as char {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            '.' if depth == 0 && bytes[idx + 1] as char == '.' => {
                return Some((&text[..idx], &text[idx + 2..]));
            }
            ',' if depth == 0 => {
                return Some((&text[..idx], &text[idx + 1..]));
            }
            _ => {}
        }
        idx += 1;
    }

    if idx < bytes.len() && depth == 0 && bytes[idx] as char == ',' {
        return Some((&text[..idx], &text[idx + 1..]));
    }

    None
}

fn eval_scalar_hint_expr(text: &str) -> Option<f32> {
    let mut parser = ScalarHintParser::new(text);
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if parser.is_eof() { Some(value) } else { None }
}

struct ScalarHintParser<'a> {
    text: &'a str,
    index: usize,
}

impl<'a> ScalarHintParser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text: text.trim(),
            index: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.index >= self.text.len()
    }

    fn rest(&self) -> &'a str {
        &self.text[self.index..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += ch.len_utf8();
        Some(ch)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            let _ = self.bump();
        }
    }

    fn parse_expr(&mut self) -> Option<f32> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.bump();
                    value += self.parse_term()?;
                }
                Some('-') => {
                    self.bump();
                    value -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Some(value)
    }

    fn parse_term(&mut self) -> Option<f32> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.bump();
                    value *= self.parse_factor()?;
                }
                Some('/') => {
                    self.bump();
                    let rhs = self.parse_factor()?;
                    if rhs.abs() <= f32::EPSILON {
                        return None;
                    }
                    value /= rhs;
                }
                _ => break,
            }
        }
        Some(value)
    }

    fn parse_factor(&mut self) -> Option<f32> {
        self.skip_ws();
        match self.peek()? {
            '+' => {
                self.bump();
                self.parse_factor()
            }
            '-' => {
                self.bump();
                Some(-self.parse_factor()?)
            }
            '(' => {
                self.bump();
                let value = self.parse_expr()?;
                self.skip_ws();
                if self.bump()? != ')' {
                    return None;
                }
                Some(value)
            }
            _ => self.parse_number(),
        }
    }

    fn parse_number(&mut self) -> Option<f32> {
        self.skip_ws();
        let start = self.index;
        let mut saw_digit = false;
        let mut saw_dot = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                saw_digit = true;
                self.bump();
            } else if ch == '.' && !saw_dot {
                saw_dot = true;
                self.bump();
            } else {
                break;
            }
        }
        if !saw_digit {
            return None;
        }
        let number = self.text[start..self.index].parse::<f32>().ok()?;

        let unit_start = self.index;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphabetic()) {
            self.bump();
        }
        let unit = &self.text[unit_start..self.index];
        let scaled = match unit {
            "ms" => number / 1000.0,
            "s" | "hz" | "" => number,
            _ => number,
        };
        Some(scaled)
    }
}

fn build_viz_call_args(
    wgsl: &str,
    fn_name: &str,
    manifest_text: &str,
    context: Option<&crate::context::EntryContext>,
    options: CallOptions<'_>,
) -> Result<Vec<String>, String> {
    let reflected_params = parse_entry_params(wgsl, fn_name);
    if reflected_params.is_empty() {
        return Ok(Vec::new());
    }

    let (has_delta, mut fixed_count) = entry_prefix_info_from_types(
        &reflected_params
            .iter()
            .map(|p| p.ty.clone())
            .collect::<Vec<_>>(),
    );
    let mut args = Vec::new();
    if let Some(context) = context {
        args.push(viz_context_expression(&context.ty, &options)?);
        fixed_count = 1;
    } else {
        if !reflected_params.is_empty() {
            args.push(options.uv_expr.to_string());
        }
        if reflected_params.len() > 1 {
            args.push(options.time_expr.to_string());
        }
        if has_delta && reflected_params.len() > 2 {
            args.push(options.delta_expr.to_string());
        }
        if reflected_params.len() >= fixed_count {
            args.push(options.res_expr.to_string());
        }
    }

    let reflected_by_name = reflected_params
        .iter()
        .skip(fixed_count)
        .filter(|p| !p.name.is_empty())
        .map(|p| (p.name.clone(), p.ty.clone()))
        .collect::<HashMap<_, _>>();
    let manifest_defs = parse_manifest_params(manifest_text, fn_name);
    let param_defs: Vec<ManifestParamDef> = if manifest_defs.is_empty() {
        reflected_params
            .iter()
            .skip(fixed_count)
            .enumerate()
            .map(|(index, param)| ManifestParamDef {
                name: if param.name.is_empty() {
                    format!("param{}", index + 1)
                } else {
                    param.name.clone()
                },
                ty: param.ty.clone(),
            })
            .collect()
    } else {
        manifest_defs
            .into_iter()
            .map(|def| ManifestParamDef {
                name: def.name.clone(),
                ty: reflected_by_name
                    .get(&def.name)
                    .cloned()
                    .unwrap_or(def.ty.clone()),
            })
            .collect()
    };

    let mut slot_offset = 0usize;
    for def in param_defs {
        args.push(viz_uniform_arg_expression(&def.ty, slot_offset));
        slot_offset += slot_width_for_type(&def.ty);
    }
    Ok(args)
}

fn viz_context_expression(
    context: &crate::context::ContextStruct,
    options: &CallOptions<'_>,
) -> Result<String, String> {
    let fields = context
        .fields
        .iter()
        .map(|field| {
            let expression = match field.semantic.as_deref() {
                Some("coord") => options.uv_expr,
                Some("time") => options.time_expr,
                Some("delta_time") => options.delta_expr,
                Some("resolution") => options.res_expr,
                Some(role) => {
                    return Err(format!(
                        "visualizer cannot supply context semantic `{role}`"
                    ));
                }
                None => {
                    if let crate::context::ContextType::Struct(nested) = &field.ty {
                        return viz_context_expression(nested, options);
                    }
                    return Err(format!(
                        "visualizer cannot supply context field `{}.{}` without a semantic",
                        context.name, field.name
                    ));
                }
            };
            Ok(expression.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(format!("{}({})", context.name, fields.join(", ")))
}

fn parse_manifest_params(manifest_text: &str, fn_name: &str) -> Vec<ManifestParamDef> {
    if manifest_text.trim().is_empty() {
        return Vec::new();
    }

    let Ok(manifest) = serde_json::from_str::<ManifestRoot>(manifest_text) else {
        return Vec::new();
    };
    let canvas_name = fn_name.strip_prefix("fresco_").unwrap_or(fn_name);
    let canvas = manifest
        .canvases
        .iter()
        .find(|entry| entry.name == canvas_name)
        .or_else(|| manifest.canvases.first());

    canvas
        .map(|entry| {
            entry
                .params
                .iter()
                .map(|param| ManifestParamDef {
                    name: param.name.clone(),
                    ty: param.ty.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_entry_params(wgsl: &str, fn_name: &str) -> Vec<EntryParam> {
    let Some(start) = wgsl.find(&format!("fn {fn_name}")) else {
        return Vec::new();
    };
    let Some(open_idx_rel) = wgsl[start..].find('(') else {
        return Vec::new();
    };
    let open_idx = start + open_idx_rel + 1;
    let Some(close_idx_rel) = wgsl[open_idx..].find(')') else {
        return Vec::new();
    };
    let params_text = &wgsl[open_idx..open_idx + close_idx_rel];
    params_text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let colon_idx = part.find(':');
            let (name, ty) = match colon_idx {
                Some(idx) => (part[..idx].trim(), part[idx + 1..].trim()),
                None => ("", part),
            };
            EntryParam {
                name: name.to_string(),
                ty: normalize_type_name(ty),
            }
        })
        .collect()
}

fn detect_fresco_entry_function(wgsl: &str) -> Option<String> {
    let mut candidates = Vec::new();
    let mut search_from = 0usize;
    while let Some(found) = wgsl[search_from..].find("fn fresco_") {
        let start = search_from + found + 3;
        let rest = &wgsl[start..];
        let name_len = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .count();
        if name_len == 0 {
            search_from = start;
            continue;
        }
        let name = rest[..name_len].to_string();
        let after_name = &rest[name_len..];
        let Some(open_idx_rel) = after_name.find('(') else {
            search_from = start + name_len;
            continue;
        };
        let params_start = start + name_len + open_idx_rel + 1;
        let Some(close_idx_rel) = wgsl[params_start..].find(')') else {
            search_from = params_start;
            continue;
        };
        let params_text = &wgsl[params_start..params_start + close_idx_rel];
        let types = params_text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| {
                part.find(':')
                    .map(|idx| normalize_type_name(&part[idx + 1..]))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        candidates.push((name, types));
        search_from = params_start + close_idx_rel;
    }

    candidates
        .iter()
        .find(|(name, types)| {
            !is_generated_helper_function_name(name) && is_canvas_entry_signature(types)
        })
        .map(|(name, _)| name.clone())
        .or_else(|| {
            candidates
                .iter()
                .find(|(name, _)| !is_generated_helper_function_name(name))
                .map(|(name, _)| name.clone())
        })
        .or_else(|| candidates.first().map(|(name, _)| name.clone()))
}

fn is_generated_helper_function_name(name: &str) -> bool {
    if let Some((prefix, suffix)) = name.rsplit_once("_scatter_l") {
        !prefix.is_empty() && !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit())
    } else {
        false
    }
}

fn is_canvas_entry_signature(types: &[String]) -> bool {
    let t = types
        .iter()
        .map(|item| normalize_type_name(item))
        .collect::<Vec<_>>();
    (t.len() >= 3 && t[0] == "vec2<f32>" && t[1] == "f32" && t[2] == "vec2<f32>")
        || (t.len() >= 4
            && t[0] == "vec2<f32>"
            && t[1] == "f32"
            && t[2] == "f32"
            && t[3] == "vec2<f32>")
}

fn entry_prefix_info_from_types(types: &[String]) -> (bool, usize) {
    let t = types
        .iter()
        .map(|item| normalize_type_name(item))
        .collect::<Vec<_>>();
    let has_base = t.len() >= 3 && t[0] == "vec2<f32>" && t[1] == "f32";
    if !has_base {
        return (false, t.len().min(3));
    }
    if t[2] == "vec2<f32>" {
        return (false, 3);
    }
    if t.len() >= 4 && t[2] == "f32" && t[3] == "vec2<f32>" {
        return (true, 4);
    }
    (false, 3)
}

fn normalize_type_name(ty: &str) -> String {
    ty.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn is_color_like_param_type(ty: &str) -> bool {
    matches!(normalize_type_name(ty).as_str(), "color" | "vec4<f32>")
}

fn slot_width_for_type(ty: &str) -> usize {
    if is_color_like_param_type(ty) { 4 } else { 1 }
}

fn uniform_param_expression(index: usize) -> String {
    let vec_index = index.div_euclid(4);
    let component_index = index % 4;
    format!("u.params[{vec_index}][{component_index}]")
}

fn uniform_vec4_expression(index: usize) -> String {
    format!(
        "vec4<f32>({}, {}, {}, {})",
        uniform_param_expression(index),
        uniform_param_expression(index + 1),
        uniform_param_expression(index + 2),
        uniform_param_expression(index + 3)
    )
}

fn viz_uniform_arg_expression(ty: &str, slot_offset: usize) -> String {
    let slot = uniform_param_expression(slot_offset);
    match normalize_type_name(ty).as_str() {
        "bool" => format!("({slot} >= 0.5)"),
        "u32" => format!("u32(max({slot}, 0.0))"),
        "i32" => format!("i32(round({slot}))"),
        _ if is_color_like_param_type(ty) => uniform_vec4_expression(slot_offset),
        _ => slot,
    }
}

fn preview_uniform_params_field_wgsl() -> String {
    format!("array<vec4<f32>, {PREVIEW_UNIFORM_PARAM_VECS}>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_canvas_entry_function() {
        let wgsl = r#"
fn fresco_helper_scatter_l0(p: vec2<f32>) -> vec4<f32> { return vec4<f32>(0.0); }
fn fresco_demo(uv: vec2<f32>, time: f32, res: vec2<f32>, speed: f32) -> vec4<f32> {
  return vec4<f32>(uv, time, 1.0);
}
"#;
        assert_eq!(
            detect_fresco_entry_function(wgsl).as_deref(),
            Some("fresco_demo")
        );
    }

    #[test]
    fn builds_sparkline_shader_with_manifest_backed_params() {
        let wgsl = r#"
fn fresco_demo(uv: vec2<f32>, time: f32, res: vec2<f32>, gain: f32, tint: vec4<f32>) -> vec4<f32> {
  return vec4<f32>(uv.x * gain + tint.x, uv.y, 0.0, 1.0);
}
"#;
        let manifest = r#"{
  "canvases": [
    {
      "name": "demo",
      "params": [
        { "name": "gain", "type": "f32", "default": 1.0, "min": 0.0, "max": 2.0 },
        { "name": "tint", "type": "vec4<f32>", "default": [1,1,1,1], "min": null, "max": null }
      ],
      "pass_plan": { "passes": [], "edges": [] }
    }
  ]
}"#;
        let shader = build_shader(
            wgsl,
            manifest,
            &VisualizerSpec {
                kind: VisualizerKind::Sparkline,
                domain: "x",
                sweep_max: 1.0,
                semantic_type: "scalar",
                range_hint: None,
            },
        )
        .expect("sparkline shader should build");
        assert!(shader.contains("@fragment"));
        assert!(shader.contains("fn fs"));
        assert!(shader.contains("@group(0) @binding(1)"));
        assert!(shader.contains("fresco_viz_bridge"));
        assert!(shader.contains("smoothstep"));
        assert!(shader.contains("line_y"));
    }

    #[test]
    fn builds_swatch_shader_with_time_sweep() {
        let wgsl = r#"
fn fresco_demo(uv: vec2<f32>, time: f32, res: vec2<f32>) -> vec4<f32> {
  return vec4<f32>(uv.x, time, 0.0, 1.0);
}
"#;
        let shader = build_shader(
            wgsl,
            "",
            &VisualizerSpec {
                kind: VisualizerKind::Swatch,
                domain: "time",
                sweep_max: 2.0,
                semantic_type: "color",
                range_hint: None,
            },
        )
        .expect("swatch shader should build");
        assert!(shader.contains("@fragment"));
        assert!(shader.contains("fn fs"));
        assert!(shader.contains("fresco_viz_bridge"));
    }

    #[test]
    fn builds_reduce_shader_with_explicit_range_hint() {
        let wgsl = r#"
fn fresco_demo(uv: vec2<f32>, time: f32, res: vec2<f32>) -> vec4<f32> {
  return vec4<f32>(uv.x, time, 0.0, 1.0);
}
"#;
        let shaders = build_shaders(
            wgsl,
            "",
            &VisualizerSpec {
                kind: VisualizerKind::Sparkline,
                domain: "time",
                sweep_max: 2.0,
                semantic_type: "scalar",
                range_hint: Some((0.25, 0.75)),
            },
            None,
        )
        .expect("visualizer shader set should build");
        assert!(shaders.reduce_wgsl.contains("viz_range = vec2<f32>("));
        assert!(!shaders.reduce_wgsl.contains("fit_min - pad"));
        assert!(!shaders.reduce_wgsl.contains("fit_max + pad"));
    }

    #[test]
    fn builds_thumbnail_shader_for_shape_semantics() {
        let wgsl = r#"
fn fresco_demo(uv: vec2<f32>, time: f32, res: vec2<f32>) -> vec4<f32> {
  return vec4<f32>(uv.x - 0.5, 0.0, 0.0, 1.0);
}
"#;
        let shader = build_shader(
            wgsl,
            "",
            &VisualizerSpec {
                kind: VisualizerKind::Thumbnail,
                domain: "thumb",
                sweep_max: 0.0,
                semantic_type: "shape",
                range_hint: None,
            },
        )
        .expect("thumbnail shader should build");
        assert!(shader.contains("@fragment"));
        assert!(shader.contains("fn fs"));
        assert!(shader.contains("fresco_viz_bridge"));
    }

    #[test]
    fn builds_thumbnail_shader_for_space_semantics() {
        let wgsl = r#"
fn fresco_demo(uv: vec2<f32>, time: f32, res: vec2<f32>) -> vec2<f32> {
  return vec2<f32>(uv.x * 0.8 + 0.1, uv.y);
}
"#;
        let shader = build_shader(
            wgsl,
            "",
            &VisualizerSpec {
                kind: VisualizerKind::Thumbnail,
                domain: "thumb",
                sweep_max: 0.0,
                semantic_type: "space",
                range_hint: None,
            },
        )
        .expect("space thumbnail shader should build");
        assert!(shader.contains("@fragment"));
        assert!(shader.contains("fn fs"));
        assert!(shader.contains("fresco_viz_bridge"));
    }

    #[test]
    fn builds_space_thumbnail_metadata_with_explicit_labels() {
        let metadata = build_metadata(
            &VisualizerSpec {
                kind: VisualizerKind::Thumbnail,
                domain: "thumb",
                sweep_max: 0.0,
                semantic_type: "space",
                range_hint: None,
            },
            Some("space stage = centered(aspect: preserve).polar(center: center)"),
        );
        assert_eq!(
            metadata.preview_label.as_deref(),
            Some("space centered(preserve) -> polar")
        );
        assert_eq!(metadata.fit_mode.as_deref(), Some("space-probe"));
        assert_eq!(metadata.y_axis_label.as_deref(), Some("space"));
        assert!(
            metadata
                .preview_detail
                .as_deref()
                .unwrap_or_default()
                .contains("distortion-field")
        );
        assert!(
            metadata
                .preview_detail
                .as_deref()
                .unwrap_or_default()
                .contains("polar-spokes")
        );
    }

    #[test]
    fn builds_shape_thumbnail_metadata_with_explicit_labels() {
        let metadata = build_metadata(
            &VisualizerSpec {
                kind: VisualizerKind::Thumbnail,
                domain: "thumb",
                sweep_max: 0.0,
                semantic_type: "shape",
                range_hint: None,
            },
            Some("box(at: center, size: (0.3, 0.2)) |> round(0.04)"),
        );
        assert_eq!(
            metadata.preview_label.as_deref(),
            Some("shape box -> round")
        );
        assert_eq!(metadata.fit_mode.as_deref(), Some("shape-probe"));
        assert_eq!(metadata.y_axis_label.as_deref(), Some("shape"));
        assert!(
            metadata
                .preview_detail
                .as_deref()
                .unwrap_or_default()
                .contains("rounded")
        );
    }

    #[test]
    fn extracts_range_hints_from_arithmetic_and_bracket_forms() {
        assert_eq!(
            extract_range_hint("wave(period: 2s, range: (1.0 / 4.0) .. (3.0 / 4.0))"),
            Some((0.25, 0.75))
        );
        assert_eq!(
            extract_range_hint("wave(period: 2s, range: [ -2 + 1, 2 * 3 ])"),
            Some((-1.0, 6.0))
        );
    }
}
