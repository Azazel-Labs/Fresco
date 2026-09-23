#![deny(unused_must_use)]
#![deny(clippy::dbg_macro, clippy::todo)]

mod browser_manifest;

impl browser_manifest::ManifestResponse for CompileSuccess {}
impl browser_manifest::ManifestResponse for VariantCompileSuccess {}

use fresco_artifact::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;
use wasm_bindgen::prelude::*;

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct CompileDiagnostic {
    file: String,
    severity: String,
    message: String,
    label: Option<String>,
    help: Option<String>,
    span_start: usize,
    span_end: usize,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct CompileSuccess {
    ok: bool,
    wgsl: String,
    manifest: ManifestRoot,
    explain: String,
    diagnostics: Vec<CompileDiagnostic>,
    timings: CompileTimings,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct CompileTimings {
    total_ms: f64,
    pipeline: PipelineTimings,
    emit_wgsl_ms: f64,
    emit_manifest_ms: f64,
    explain_ms: f64,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct PipelineTimings {
    total_ms: f64,
    lex_ms: f64,
    parse_ms: f64,
    resolve_imports_ms: f64,
    check_rewrite_ms: f64,
    lower_validate_ms: f64,
    map_diagnostics_ms: f64,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct CompileFailure {
    ok: bool,
    diagnostics: Vec<CompileDiagnostic>,
}

// The browser host selects its authored library; the compiler embeds no engine.
fn example_language_docs() -> fresco::language_docs::LanguageDocsModel {
    fresco::language_docs::build_language_docs_model_with_prelude(fresco_example_engine::PRELUDE)
        .expect("bundled example prelude must parse")
}

fn manifest_parse_failure(message: String) -> CompileFailure {
    CompileFailure {
        ok: false,
        diagnostics: vec![CompileDiagnostic {
            file: String::from("<manifest>"),
            severity: String::from("error"),
            message,
            label: None,
            help: Some(String::from(
                "compiler emitted a manifest shape that does not match wasm contract bindings",
            )),
            span_start: 0,
            span_end: 0,
        }],
    }
}

fn parse_manifest_root(manifest_json: &str) -> Result<ManifestRoot, CompileFailure> {
    serde_json::from_str(manifest_json)
        .map_err(|e| manifest_parse_failure(format!("invalid manifest payload from compiler: {e}")))
}

enum BuildProfileArg {
    All,
    Editor,
    Runtime,
}

impl BuildProfileArg {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "editor" => Some(Self::Editor),
            "runtime" => Some(Self::Runtime),
            _ => None,
        }
    }

    fn into_driver(self) -> fresco::driver::BuildProfile {
        match self {
            Self::All => fresco::driver::BuildProfile::All,
            Self::Editor => fresco::driver::BuildProfile::Editor,
            Self::Runtime => fresco::driver::BuildProfile::Runtime,
        }
    }
}

fn context_for_build_profile(
    build_profile: &str,
) -> Result<fresco::driver::CompileContext, CompileFailure> {
    let mut context = fresco::driver::CompileContext::default();
    let trimmed = build_profile.trim();
    if trimmed.is_empty() {
        return Ok(context);
    }

    let Some(profile) = BuildProfileArg::parse(trimmed) else {
        return Err(CompileFailure {
            ok: false,
            diagnostics: vec![CompileDiagnostic {
                file: String::from("<compile>"),
                severity: String::from("error"),
                message: format!(
                    "invalid build profile `{trimmed}`; expected one of: all, editor, runtime"
                ),
                label: None,
                help: Some(String::from(
                    "pass an empty string for default behavior, or one of: all, editor, runtime",
                )),
                span_start: 0,
                span_end: 0,
            }],
        });
    };

    context.build_profile = profile.into_driver();
    Ok(context)
}

fn map_driver_diagnostics(
    diagnostics: impl IntoIterator<Item = fresco::driver::DiagnosticRecord>,
) -> Vec<CompileDiagnostic> {
    diagnostics
        .into_iter()
        .map(|d| CompileDiagnostic {
            file: d.file,
            severity: d.severity,
            message: d.message,
            label: d.label,
            help: d.help,
            span_start: d.span_start,
            span_end: d.span_end,
        })
        .collect()
}

#[derive(Serialize)]
struct LanguageProfile {
    keywords: Vec<String>,
    type_keywords: Vec<String>,
    units: Vec<String>,
    builtins: Vec<String>,
    callables: Vec<String>,
    builtin_reference: Vec<fresco::language_docs::DocBuiltin>,
    space_transforms: Vec<String>,
    blend_modes: Vec<String>,
    enum_members: Vec<String>,
    stdlib_exports: Vec<StdlibExport>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct StdlibExport {
    name: String,
    params: Vec<StdlibExportParam>,
    ret_ty: String,
    documentation: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct StdlibExportParam {
    name: String,
    ty_name: String,
    documentation: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct LexToken {
    kind: String,
    start: usize,
    end: usize,
}

fn lex_token(kind: fresco::lexer::TokenHighlightKind, start: usize, end: usize) -> LexToken {
    LexToken {
        kind: kind.as_str().to_string(),
        start,
        end,
    }
}

fn scan_comment_tokens(source: &str) -> Vec<LexToken> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    while i + 1 < bytes.len() {
        let ch = bytes[i] as char;
        let next = bytes[i + 1] as char;

        if escaped {
            escaped = false;
            i += 1;
            continue;
        }

        if (in_single || in_double) && ch == '\\' {
            escaped = true;
            i += 1;
            continue;
        }

        if !in_double && ch == '\'' {
            in_single = !in_single;
            i += 1;
            continue;
        }
        if !in_single && ch == '"' {
            in_double = !in_double;
            i += 1;
            continue;
        }

        if !in_single && !in_double && ch == '/' && next == '/' {
            let start = i;
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(lex_token(
                fresco::lexer::TokenHighlightKind::Comment,
                start,
                i,
            ));
            continue;
        }
        i += 1;
    }

    out
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct CompletionItem {
    label: String,
    insert_text: String,
    kind: String,
    detail: String,
    boost: i32,
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    documentation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    info: Option<String>,
}

#[wasm_bindgen(start)]
pub fn wasm_start() {
    #[cfg(target_arch = "wasm32")]
    // SAFETY: wasm-bindgen calls this start function once on module initialization.
    // The linker supplies __wasm_call_ctors; inventory constructors must run before
    // any exported compiler API accesses their registered static entries.
    unsafe {
        // Ensure linker-registered constructors (e.g. inventory submissions)
        // are executed before calling into compiler APIs.
        __wasm_call_ctors();
    }
    console_error_panic_hook::set_once();
}

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn __wasm_call_ctors();
}

#[wasm_bindgen]
pub fn fresco_wasm_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen]
pub fn fresco_wasm_build_mode() -> String {
    if cfg!(debug_assertions) {
        "dev"
    } else {
        "release"
    }
    .to_string()
}

#[wasm_bindgen]
pub fn compile_fresco(source: &str) -> JsValue {
    compile_fresco_with_options(source, false)
}

#[wasm_bindgen]
pub fn compile_fresco_with_options(source: &str, include_explain: bool) -> JsValue {
    match fresco::driver::compile_source_bundle(source, "<source>", include_explain) {
        Ok(result) => {
            let diagnostics = map_driver_diagnostics(result.diagnostics.clone());
            let manifest = match parse_manifest_root(&result.manifest) {
                Ok(manifest) => manifest,
                Err(failure) => {
                    return serde_wasm_bindgen::to_value(&failure)
                        .expect("failed to serialize compile manifest parse failure");
                }
            };
            let success = CompileSuccess {
                ok: true,
                wgsl: result.wgsl,
                manifest,
                explain: result.explain.unwrap_or_default(),
                diagnostics,
                timings: CompileTimings {
                    total_ms: result.timings.total_ms,
                    pipeline: PipelineTimings {
                        total_ms: result.timings.pipeline.total_ms,
                        lex_ms: result.timings.pipeline.lex_ms,
                        parse_ms: result.timings.pipeline.parse_ms,
                        resolve_imports_ms: result.timings.pipeline.resolve_imports_ms,
                        check_rewrite_ms: result.timings.pipeline.check_rewrite_ms,
                        lower_validate_ms: result.timings.pipeline.lower_validate_ms,
                        map_diagnostics_ms: result.timings.pipeline.map_diagnostics_ms,
                    },
                    emit_wgsl_ms: result.timings.emit_wgsl_ms,
                    emit_manifest_ms: result.timings.emit_manifest_ms,
                    explain_ms: result.timings.explain_ms,
                },
            };
            browser_manifest::serialize_success(&success)
                .expect("failed to serialize successful compile response")
        }
        Err(diags) => {
            let diagnostics = map_driver_diagnostics(diags);
            let failure = CompileFailure {
                ok: false,
                diagnostics,
            };
            serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize failed compile response")
        }
    }
}

/// The selected example profile's complete embedded source bundle, for standalone hosts.
#[wasm_bindgen]
pub fn example_engine_sources_json() -> String {
    let sources: std::collections::BTreeMap<_, _> = fresco_example_engine::SOURCES
        .iter()
        .map(|source| (source.path, source.source))
        .collect();
    serde_json::to_string(&sources).expect("string source map serializes")
}

/// Compile a multi-file Fresco program supplied as a virtual file system.
/// `files` must be a JS object mapping virtual path strings to source strings
/// (e.g. `{ "main.fr": "...", "lib.fr": "..." }`).  `entrypoint` must be one
/// of those keys and must contain at least one `canvas` definition.
#[wasm_bindgen]
pub fn compile_fresco_bundle(files: JsValue, entrypoint: &str, include_explain: bool) -> JsValue {
    let files: HashMap<String, String> = match serde_wasm_bindgen::from_value(files) {
        Ok(m) => m,
        Err(e) => {
            let failure = CompileFailure {
                ok: false,
                diagnostics: vec![CompileDiagnostic {
                    file: String::from("<bundle>"),
                    severity: String::from("error"),
                    message: format!("invalid files argument: {e}"),
                    label: None,
                    help: Some(String::from(
                        "pass a JS object mapping virtual paths to source strings",
                    )),
                    span_start: 0,
                    span_end: 0,
                }],
            };
            return serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize bundle failure response");
        }
    };

    match fresco::driver::compile_bundle_virtual(&files, entrypoint, include_explain) {
        Ok(result) => {
            let diagnostics = map_driver_diagnostics(result.diagnostics.clone());
            let manifest = match parse_manifest_root(&result.manifest) {
                Ok(manifest) => manifest,
                Err(failure) => {
                    return serde_wasm_bindgen::to_value(&failure)
                        .expect("failed to serialize bundle manifest parse failure");
                }
            };
            let success = CompileSuccess {
                ok: true,
                wgsl: result.wgsl,
                manifest,
                explain: result.explain.unwrap_or_default(),
                diagnostics,
                timings: CompileTimings {
                    total_ms: result.timings.total_ms,
                    pipeline: PipelineTimings {
                        total_ms: result.timings.pipeline.total_ms,
                        lex_ms: result.timings.pipeline.lex_ms,
                        parse_ms: result.timings.pipeline.parse_ms,
                        resolve_imports_ms: result.timings.pipeline.resolve_imports_ms,
                        check_rewrite_ms: result.timings.pipeline.check_rewrite_ms,
                        lower_validate_ms: result.timings.pipeline.lower_validate_ms,
                        map_diagnostics_ms: result.timings.pipeline.map_diagnostics_ms,
                    },
                    emit_wgsl_ms: result.timings.emit_wgsl_ms,
                    emit_manifest_ms: result.timings.emit_manifest_ms,
                    explain_ms: result.timings.explain_ms,
                },
            };
            browser_manifest::serialize_success(&success)
                .expect("failed to serialize bundle compile success response")
        }
        Err(diags) => {
            let diagnostics = map_driver_diagnostics(diags);
            let failure = CompileFailure {
                ok: false,
                diagnostics,
            };
            serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize bundle compile failure response")
        }
    }
}

#[wasm_bindgen]
pub fn compile_fresco_bundle_with_profile(
    files: JsValue,
    entrypoint: &str,
    include_explain: bool,
    build_profile: &str,
) -> JsValue {
    let files: HashMap<String, String> = match serde_wasm_bindgen::from_value(files) {
        Ok(m) => m,
        Err(e) => {
            let failure = CompileFailure {
                ok: false,
                diagnostics: vec![CompileDiagnostic {
                    file: String::from("<bundle>"),
                    severity: String::from("error"),
                    message: format!("invalid files argument: {e}"),
                    label: None,
                    help: Some(String::from(
                        "pass a JS object mapping virtual paths to source strings",
                    )),
                    span_start: 0,
                    span_end: 0,
                }],
            };
            return serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize bundle failure response");
        }
    };

    let context = match context_for_build_profile(build_profile) {
        Ok(context) => context,
        Err(failure) => {
            return serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize bundle build-profile failure response");
        }
    };

    match fresco::driver::compile_bundle_virtual_with_context(
        &files,
        entrypoint,
        include_explain,
        &context,
    ) {
        Ok(result) => {
            let diagnostics = map_driver_diagnostics(result.diagnostics.clone());
            let manifest = match parse_manifest_root(&result.manifest) {
                Ok(manifest) => manifest,
                Err(failure) => {
                    return serde_wasm_bindgen::to_value(&failure)
                        .expect("failed to serialize bundle manifest parse failure");
                }
            };
            let success = CompileSuccess {
                ok: true,
                wgsl: result.wgsl,
                manifest,
                explain: result.explain.unwrap_or_default(),
                diagnostics,
                timings: CompileTimings {
                    total_ms: result.timings.total_ms,
                    pipeline: PipelineTimings {
                        total_ms: result.timings.pipeline.total_ms,
                        lex_ms: result.timings.pipeline.lex_ms,
                        parse_ms: result.timings.pipeline.parse_ms,
                        resolve_imports_ms: result.timings.pipeline.resolve_imports_ms,
                        check_rewrite_ms: result.timings.pipeline.check_rewrite_ms,
                        lower_validate_ms: result.timings.pipeline.lower_validate_ms,
                        map_diagnostics_ms: result.timings.pipeline.map_diagnostics_ms,
                    },
                    emit_wgsl_ms: result.timings.emit_wgsl_ms,
                    emit_manifest_ms: result.timings.emit_manifest_ms,
                    explain_ms: result.timings.explain_ms,
                },
            };
            browser_manifest::serialize_success(&success)
                .expect("failed to serialize bundle compile success response")
        }
        Err(diags) => {
            let diagnostics = map_driver_diagnostics(diags);
            let failure = CompileFailure {
                ok: false,
                diagnostics,
            };
            serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize bundle compile failure response")
        }
    }
}

#[wasm_bindgen]
pub fn fresco_language_profile() -> JsValue {
    let docs = example_language_docs();
    let profile = LanguageProfile {
        keywords: docs.keywords,
        type_keywords: docs.type_keywords,
        units: docs.units,
        builtins: docs.builtins,
        callables: docs.callables,
        builtin_reference: docs.builtin_reference,
        space_transforms: docs.space_transforms,
        blend_modes: docs.blend_modes,
        enum_members: docs.enum_members,
        stdlib_exports: docs
            .stdlib_exports
            .into_iter()
            .map(|f| StdlibExport {
                name: f.name,
                params: f
                    .params
                    .into_iter()
                    .map(|p| StdlibExportParam {
                        name: p.name,
                        ty_name: p.ty_name,
                        documentation: p.docs,
                    })
                    .collect(),
                ret_ty: f.ret_ty,
                documentation: f.docs,
            })
            .collect(),
    };
    serde_wasm_bindgen::to_value(&profile).expect("failed to serialize language profile")
}

#[wasm_bindgen]
pub fn fresco_lex_tokens(source: &str) -> JsValue {
    let tokens = lex_tokens_for_highlighting(source);

    serde_wasm_bindgen::to_value(&tokens).expect("failed to serialize lexical tokens")
}

/// Bundle-aware lexical annotations retain identifiers outside DSL declaration positions.
#[wasm_bindgen]
pub fn fresco_lex_tokens_with_files(
    source: &str,
    filename: &str,
    files: JsValue,
) -> Result<JsValue, JsValue> {
    let files = workbook_files(files)?.unwrap_or_default();
    let spans = fresco::driver::engine_keyword_spans(source, filename, &files);
    let mut tokens = lex_tokens_for_highlighting(source);
    for token in &mut tokens {
        if spans
            .iter()
            .any(|span| span.start == token.start && span.end == token.end)
        {
            token.kind = "engine_keyword".into();
        }
    }
    Ok(serde_wasm_bindgen::to_value(&tokens).expect("failed to serialize bundle lexical tokens"))
}

fn lex_tokens_for_highlighting(source: &str) -> Vec<LexToken> {
    let mut tokens: Vec<LexToken> = fresco::lexer::lex_spanned(source)
        .into_iter()
        .filter_map(|(token, span)| {
            token
                .highlight_kind()
                .map(|kind| lex_token(kind, span.start, span.end))
        })
        .collect();

    tokens.extend(scan_comment_tokens(source));
    tokens.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    tokens
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct SpanQueryResponse {
    ok: bool,
    #[serde(rename = "type")]
    ty: String,
    span_start: usize,
    span_end: usize,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct SpanQueryFailure {
    ok: bool,
}

#[wasm_bindgen]
pub fn fresco_query_span(
    source: &str,
    span_start: usize,
    span_end: usize,
    files: JsValue,
) -> JsValue {
    let files = match workbook_files(files) {
        Ok(files) => files,
        Err(failure) => return failure,
    };
    match fresco::driver::query_span_info_with_files(
        source,
        "<source>",
        span_start,
        span_end,
        files.as_ref(),
    ) {
        Some(result) => {
            let response = SpanQueryResponse {
                ok: true,
                ty: result.kind.as_str().to_string(),
                span_start: result.span_start,
                span_end: result.span_end,
            };
            serde_wasm_bindgen::to_value(&response)
                .expect("failed to serialize span query response")
        }
        None => {
            let failure = SpanQueryFailure { ok: false };
            serde_wasm_bindgen::to_value(&failure).expect("failed to serialize span query failure")
        }
    }
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct VariantCompileSuccess {
    ok: bool,
    wgsl: String,
    manifest: ManifestRoot,
    #[serde(rename = "semanticType")]
    semantic_type: String,
    diagnostics: Vec<CompileDiagnostic>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct VisualizerCompileSuccess {
    ok: bool,
    wgsl: String,
    #[serde(rename = "drawWgsl")]
    draw_wgsl: String,
    #[serde(rename = "reduceWgsl")]
    reduce_wgsl: String,
    #[serde(rename = "semanticType")]
    semantic_type: String,
    metadata: VisualizerMetadata,
    diagnostics: Vec<CompileDiagnostic>,
}

#[derive(Serialize, TS)]
#[ts(export, export_to = "../web/src/generated/wasm-contracts/")]
struct VisualizerMetadata {
    kind: String,
    domain: String,
    #[serde(rename = "sweepMax")]
    sweep_max: f32,
    #[serde(rename = "xAxisLabel")]
    x_axis_label: Option<String>,
    #[serde(rename = "yAxisLabel")]
    y_axis_label: Option<String>,
    #[serde(rename = "fitMode")]
    fit_mode: Option<String>,
    #[serde(rename = "yMinHint")]
    y_min_hint: Option<f32>,
    #[serde(rename = "yMaxHint")]
    y_max_hint: Option<f32>,
    #[serde(rename = "previewLabel")]
    preview_label: Option<String>,
    #[serde(rename = "previewDetail")]
    preview_detail: Option<String>,
}

#[wasm_bindgen]
pub fn fresco_compile_variant(
    source: &str,
    span_start: usize,
    span_end: usize,
    files: JsValue,
) -> JsValue {
    let files = match workbook_files(files) {
        Ok(files) => files,
        Err(failure) => return failure,
    };
    match fresco::driver::compile_variant_with_files(
        source,
        "<source>",
        span_start,
        span_end,
        files.as_ref(),
    ) {
        Ok(result) => {
            let diagnostics = map_driver_diagnostics(result.diagnostics.clone());
            let manifest = match parse_manifest_root(&result.manifest) {
                Ok(manifest) => manifest,
                Err(failure) => {
                    return serde_wasm_bindgen::to_value(&failure)
                        .expect("failed to serialize variant manifest parse failure");
                }
            };
            let success = VariantCompileSuccess {
                ok: true,
                wgsl: result.wgsl,
                manifest,
                semantic_type: result.semantic_type,
                diagnostics,
            };
            browser_manifest::serialize_success(&success)
                .expect("failed to serialize variant compile response")
        }
        Err(diags) => {
            let diagnostics = map_driver_diagnostics(diags);
            let failure = CompileFailure {
                ok: false,
                diagnostics,
            };
            serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize variant compile failure")
        }
    }
}

#[wasm_bindgen]
pub fn fresco_compile_visualizer(
    source: &str,
    span_start: usize,
    span_end: usize,
    kind: &str,
    domain: &str,
    sweep_max: f32,
    files: JsValue,
) -> JsValue {
    let files = match workbook_files(files) {
        Ok(files) => files,
        Err(failure) => return failure,
    };
    let visualizer_kind = match kind {
        "sparkline" | "chart" | "timeseries" => fresco::driver::VisualizerKind::Sparkline,
        "swatch" | "color" => fresco::driver::VisualizerKind::Swatch,
        "thumbnail" | "thumb" => fresco::driver::VisualizerKind::Thumbnail,
        other => {
            let failure = CompileFailure {
                ok: false,
                diagnostics: vec![CompileDiagnostic {
                    file: "<source>".to_string(),
                    severity: "error".to_string(),
                    message: format!("unsupported visualizer kind: {other}"),
                    label: None,
                    help: None,
                    span_start: 0,
                    span_end: 0,
                }],
            };
            return serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize visualizer compile failure");
        }
    };

    match fresco::driver::compile_visualizer_with_files(
        source,
        "<source>",
        (span_start, span_end),
        visualizer_kind,
        domain,
        sweep_max,
        files.as_ref(),
    ) {
        Ok(result) => {
            let diagnostics = map_driver_diagnostics(result.diagnostics.clone());
            let success = VisualizerCompileSuccess {
                ok: true,
                wgsl: result.wgsl.clone(),
                draw_wgsl: result.draw_wgsl,
                reduce_wgsl: result.reduce_wgsl,
                semantic_type: result.semantic_type,
                metadata: VisualizerMetadata {
                    kind: result.metadata.kind,
                    domain: result.metadata.domain,
                    sweep_max: result.metadata.sweep_max,
                    x_axis_label: result.metadata.x_axis_label,
                    y_axis_label: result.metadata.y_axis_label,
                    fit_mode: result.metadata.fit_mode,
                    y_min_hint: result.metadata.y_min_hint,
                    y_max_hint: result.metadata.y_max_hint,
                    preview_label: result.metadata.preview_label,
                    preview_detail: result.metadata.preview_detail,
                },
                diagnostics,
            };
            serde_wasm_bindgen::to_value(&success)
                .expect("failed to serialize visualizer compile response")
        }
        Err(diags) => {
            let diagnostics = map_driver_diagnostics(diags);
            let failure = CompileFailure {
                ok: false,
                diagnostics,
            };
            serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize visualizer compile failure")
        }
    }
}

fn workbook_files(files: JsValue) -> Result<Option<HashMap<String, String>>, JsValue> {
    if files.is_null() || files.is_undefined() {
        return Ok(None);
    }
    serde_wasm_bindgen::from_value(files)
        .map(Some)
        .map_err(|error| {
            let failure = CompileFailure {
                ok: false,
                diagnostics: vec![CompileDiagnostic {
                    file: "<bundle>".into(),
                    severity: "error".into(),
                    message: format!("invalid files argument: {error}"),
                    label: None,
                    help: Some("pass a JS object mapping virtual paths to source strings".into()),
                    span_start: 0,
                    span_end: 0,
                }],
            };
            serde_wasm_bindgen::to_value(&failure)
                .expect("failed to serialize workbook input failure")
        })
}

fn current_line_prefix(source: &str, cursor: usize) -> &str {
    let cursor = cursor.min(source.len());
    let before = &source[..cursor];
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    &before[line_start..]
}

fn source_prefix(source: &str, cursor: usize) -> &str {
    let cursor = cursor.min(source.len());
    &source[..cursor]
}

fn import_binding_name_from_path(path: &str) -> Option<String> {
    let normalized_path = path.replace('\\', "/");
    let file_name = normalized_path.rsplit('/').next().unwrap_or(path).trim();
    let stem = file_name.strip_suffix(".fr").unwrap_or(file_name).trim();
    if stem.is_empty() {
        return None;
    }

    let mut out = String::new();
    for ch in stem.chars() {
        if out.is_empty() {
            if is_ident_start(ch) {
                out.push(ch);
            } else if ch.is_ascii_digit() {
                out.push('_');
                out.push(ch);
            } else {
                out.push('_');
            }
        } else if is_ident_char(ch) {
            out.push(ch);
        } else {
            out.push('_');
        }
    }

    if out.is_empty() { None } else { Some(out) }
}

fn collect_identifiers_in_scope(source: &str, cursor: usize) -> Vec<String> {
    use std::collections::BTreeSet;

    let mut scopes: Vec<BTreeSet<String>> = vec![BTreeSet::new()];
    let mut expect_decl_ident = false;
    let mut expect_fn_name = false;
    let mut expect_fn_lparen = false;
    let mut in_fn_params = false;
    let mut fn_param_depth = 0usize;
    let mut expect_param_name = false;
    let mut expect_import_path = false;
    let mut pending_import_binding: Option<String> = None;
    let mut expect_import_alias_keyword = false;
    let mut expect_import_alias_name = false;

    for (token, _span) in fresco::lexer::lex_spanned(source_prefix(source, cursor)) {
        match token {
            fresco::lexer::Token::LBrace => scopes.push(BTreeSet::new()),
            fresco::lexer::Token::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
            }
            fresco::lexer::Token::Import => {
                expect_import_path = true;
                pending_import_binding = None;
                expect_import_alias_keyword = false;
                expect_import_alias_name = false;
            }
            fresco::lexer::Token::Str(path) => {
                if expect_import_path {
                    pending_import_binding = import_binding_name_from_path(&path);
                    expect_import_path = false;
                    expect_import_alias_keyword = true;
                }
            }
            fresco::lexer::Token::Newline => {
                if let Some(name) = pending_import_binding.take()
                    && let Some(scope) = scopes.first_mut()
                {
                    scope.insert(name);
                }
                expect_import_path = false;
                expect_import_alias_keyword = false;
                expect_import_alias_name = false;
            }
            fresco::lexer::Token::Let | fresco::lexer::Token::Param | fresco::lexer::Token::For => {
                expect_decl_ident = true;
            }
            fresco::lexer::Token::Fn => {
                expect_fn_name = true;
                expect_fn_lparen = false;
                in_fn_params = false;
                fn_param_depth = 0;
                expect_param_name = false;
            }
            fresco::lexer::Token::LParen => {
                if expect_fn_lparen {
                    in_fn_params = true;
                    fn_param_depth = 1;
                    expect_param_name = true;
                    expect_fn_lparen = false;
                } else if in_fn_params {
                    fn_param_depth += 1;
                }
            }
            fresco::lexer::Token::RParen => {
                if in_fn_params {
                    fn_param_depth = fn_param_depth.saturating_sub(1);
                    if fn_param_depth == 0 {
                        in_fn_params = false;
                        expect_param_name = false;
                    }
                }
            }
            fresco::lexer::Token::Comma => {
                if in_fn_params && fn_param_depth == 1 {
                    expect_param_name = true;
                }
            }
            fresco::lexer::Token::Colon => {
                if in_fn_params && fn_param_depth == 1 {
                    expect_param_name = false;
                }
            }
            fresco::lexer::Token::Ident(name) => {
                if expect_import_alias_keyword {
                    if name.as_str() == "as" {
                        expect_import_alias_keyword = false;
                        expect_import_alias_name = true;
                        continue;
                    }

                    if let Some(import_name) = pending_import_binding.take()
                        && let Some(scope) = scopes.first_mut()
                    {
                        scope.insert(import_name);
                    }
                    expect_import_alias_keyword = false;
                }

                if expect_import_alias_name {
                    if let Some(scope) = scopes.first_mut() {
                        scope.insert(name.to_string());
                    }
                    pending_import_binding = None;
                    expect_import_alias_name = false;
                    continue;
                }

                if let Some(scope) = scopes.last_mut() {
                    if expect_fn_name {
                        scope.insert(name.to_string());
                        expect_fn_name = false;
                        expect_fn_lparen = true;
                        continue;
                    }

                    if expect_decl_ident {
                        scope.insert(name.to_string());
                        expect_decl_ident = false;
                        continue;
                    }

                    if in_fn_params && fn_param_depth == 1 && expect_param_name {
                        scope.insert(name.to_string());
                        expect_param_name = false;
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(name) = pending_import_binding
        && let Some(scope) = scopes.first_mut()
    {
        scope.insert(name);
    }

    let mut out = BTreeSet::new();
    for scope in scopes {
        out.extend(scope);
    }
    out.into_iter().collect()
}

#[derive(Default)]
struct CompletionExpectations {
    has_data: bool,
    exact_tokens: std::collections::HashSet<String>,
    exact_keywords: std::collections::HashSet<String>,
    expects_identifier: bool,
    expects_type: bool,
    expects_unit: bool,
    expects_number: bool,
}

fn normalize_expected_token(token: &str) -> String {
    token
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .to_ascii_lowercase()
}

fn is_word_like_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|ch| ch == '_' || ch == '@' || ch == '#' || ch.is_ascii_alphanumeric())
}

fn completion_expectations(
    source: &str,
    cursor: usize,
    keywords: &[String],
    type_keywords: &[String],
    units: &[String],
) -> CompletionExpectations {
    let expected = fresco::driver::expected_tokens_at_cursor(source, cursor);
    if expected.is_empty() {
        return CompletionExpectations::default();
    }

    let mut exact_tokens = std::collections::HashSet::new();
    for token in expected {
        let normalized = normalize_expected_token(&token);
        if !normalized.is_empty() {
            exact_tokens.insert(normalized);
        }
    }

    let keyword_set: std::collections::HashSet<String> =
        keywords.iter().map(|k| k.to_ascii_lowercase()).collect();
    let type_set: std::collections::HashSet<String> = type_keywords
        .iter()
        .map(|k| k.to_ascii_lowercase())
        .collect();
    let unit_set: std::collections::HashSet<String> =
        units.iter().map(|u| u.to_ascii_lowercase()).collect();

    let exact_keywords = exact_tokens
        .iter()
        .filter(|token| keyword_set.contains(*token))
        .cloned()
        .collect::<std::collections::HashSet<_>>();

    let expects_identifier = exact_tokens.contains("identifier")
        || exact_tokens.iter().any(|token| {
            is_word_like_token(token)
                && !matches!(
                    token.as_str(),
                    "number" | "string literal" | "color literal" | "newline" | "end of input"
                )
        });
    let expects_type =
        exact_tokens.contains("type") || exact_tokens.iter().any(|token| type_set.contains(token));
    let expects_unit = exact_tokens.iter().any(|token| unit_set.contains(token));
    let expects_number = exact_tokens.contains("number");

    CompletionExpectations {
        has_data: true,
        exact_tokens,
        exact_keywords,
        expects_identifier,
        expects_type,
        expects_unit,
        expects_number,
    }
}

fn is_allowed_by_expectations(
    item: &CompletionItem,
    expectations: &CompletionExpectations,
) -> bool {
    if !expectations.has_data {
        return true;
    }

    let label = item.label.to_ascii_lowercase();
    if expectations.exact_tokens.contains(&label) {
        return true;
    }

    match item.kind.as_str() {
        "keyword" => {
            if !expectations.exact_keywords.is_empty() {
                expectations.exact_keywords.contains(&label)
            } else {
                expectations.expects_identifier
            }
        }
        "type" => expectations.expects_type || expectations.expects_identifier,
        "constant" => expectations.expects_unit || expectations.expects_number,
        "function" | "variable" | "property" | "enum" => {
            if !expectations.exact_keywords.is_empty()
                && !expectations.exact_tokens.contains("identifier")
            {
                false
            } else {
                expectations.expects_identifier
            }
        }
        _ => true,
    }
}

fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic()
}

fn trailing_ident_fragment(prefix: &str) -> &str {
    let mut start = prefix.len();
    for (idx, ch) in prefix.char_indices().rev() {
        if is_ident_char(ch) {
            start = idx;
            continue;
        }
        break;
    }
    &prefix[start..]
}

fn is_blend_value_context(line_prefix: &str) -> bool {
    line_prefix
        .trim_end()
        .rsplit_once("blend:")
        .map(|(_, rhs)| rhs.trim_start().chars().all(is_ident_char))
        .unwrap_or(false)
}

fn is_pipe_context(line_prefix: &str) -> bool {
    line_prefix
        .trim_end()
        .rsplit_once("|>")
        .map(|(_, rhs)| rhs.trim_start().chars().all(is_ident_char))
        .unwrap_or(false)
}

fn is_pipe_context_at_cursor(source_before_cursor: &str, line_prefix: &str) -> bool {
    if is_pipe_context(line_prefix) {
        return true;
    }

    source_before_cursor
        .trim_end()
        .rsplit_once("|>")
        .map(|(_, rhs)| rhs.trim_start().chars().all(is_ident_char))
        .unwrap_or(false)
}

fn is_shape_pipeline_signature(sig: &fresco::language_docs::DocBuiltinSignature) -> bool {
    matches!(sig.receiver.as_deref(), Some("shape" | "layer"))
}

fn is_in_keyword_context(line_prefix: &str) -> bool {
    line_prefix
        .trim_end()
        .rsplit_once("in")
        .map(|(lhs, rhs)| {
            let lhs_ok = lhs.is_empty() || lhs.ends_with(' ') || lhs.ends_with('{');
            lhs_ok && rhs.trim_start().chars().all(is_ident_char)
        })
        .unwrap_or(false)
}

fn is_type_context(line_prefix: &str) -> bool {
    let trimmed = line_prefix.trim_end();
    let in_return = trimmed
        .rsplit_once("->")
        .map(|(_, rhs)| rhs.trim_start().chars().all(is_ident_char))
        .unwrap_or(false);
    if in_return {
        return true;
    }

    if trimmed.contains("blend:") {
        return false;
    }

    trimmed
        .rsplit_once(':')
        .map(|(lhs, rhs)| {
            lhs.contains("param ")
                && rhs.trim_start().chars().all(is_ident_char)
                && !lhs.trim_end().ends_with("blend")
        })
        .unwrap_or(false)
}

fn extract_ident_before(text: &str, at: usize) -> Option<String> {
    let mut end = at;
    while end > 0 {
        let ch = text[..end].chars().next_back()?;
        if ch.is_whitespace() {
            end -= ch.len_utf8();
            continue;
        }
        break;
    }
    if end == 0 {
        return None;
    }

    let mut start = end;
    while start > 0 {
        let ch = text[..start].chars().next_back()?;
        if is_ident_char(ch) {
            start -= ch.len_utf8();
            continue;
        }
        break;
    }

    let ident = &text[start..end];
    if ident.is_empty() || !is_ident_start(ident.chars().next()?) {
        return None;
    }
    Some(ident.to_string())
}

fn collect_named_args(arg_prefix: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut i = 0usize;
    while i < arg_prefix.len() {
        let mut chars = arg_prefix[i..].char_indices();
        let Some((_, ch)) = chars.next() else {
            break;
        };
        if !is_ident_start(ch) {
            i += ch.len_utf8();
            continue;
        }

        let start = i;
        i += ch.len_utf8();
        while i < arg_prefix.len() {
            let next = arg_prefix[i..].chars().next();
            let Some(nc) = next else {
                break;
            };
            if is_ident_char(nc) {
                i += nc.len_utf8();
            } else {
                break;
            }
        }

        let name = &arg_prefix[start..i];
        let mut j = i;
        while j < arg_prefix.len() {
            let Some(ws) = arg_prefix[j..].chars().next() else {
                break;
            };
            if ws.is_whitespace() {
                j += ws.len_utf8();
            } else {
                break;
            }
        }

        if arg_prefix[j..].starts_with(':') {
            out.insert(name.to_string());
        }
    }
    out
}

fn active_call_context(
    source_before_cursor: &str,
) -> Option<(String, std::collections::HashSet<String>)> {
    let mut open_stack = Vec::new();
    for (idx, ch) in source_before_cursor.char_indices() {
        match ch {
            '(' => open_stack.push(idx),
            ')' => {
                open_stack.pop();
            }
            _ => {}
        }
    }

    let open = *open_stack.last()?;
    let call_name = extract_ident_before(source_before_cursor, open)?;
    let args_prefix = &source_before_cursor[open + 1..];
    Some((call_name, collect_named_args(args_prefix)))
}

fn active_named_arg_value_context(source_before_cursor: &str) -> Option<(String, String)> {
    let mut open_stack = Vec::new();
    for (idx, ch) in source_before_cursor.char_indices() {
        match ch {
            '(' => open_stack.push(idx),
            ')' => {
                open_stack.pop();
            }
            _ => {}
        }
    }

    let open = *open_stack.last()?;
    let call_name = extract_ident_before(source_before_cursor, open)?;
    let args_prefix = &source_before_cursor[open + 1..];
    let tail = args_prefix
        .rsplit(',')
        .next()
        .unwrap_or(args_prefix)
        .trim_start();

    let mut chars = tail.char_indices();
    let (_, first) = chars.next()?;
    if !is_ident_start(first) {
        return None;
    }
    let mut end = first.len_utf8();
    while end < tail.len() {
        let Some(ch) = tail[end..].chars().next() else {
            break;
        };
        if is_ident_char(ch) {
            end += ch.len_utf8();
        } else {
            break;
        }
    }

    let arg_name = tail[..end].to_string();
    let rest = tail[end..].trim_start();
    if !rest.starts_with(':') {
        return None;
    }

    Some((call_name, arg_name))
}

fn active_positional_arg_value_context(source_before_cursor: &str) -> Option<(String, usize)> {
    let mut open_stack = Vec::new();
    for (idx, ch) in source_before_cursor.char_indices() {
        match ch {
            '(' => open_stack.push(idx),
            ')' => {
                open_stack.pop();
            }
            _ => {}
        }
    }

    let open = *open_stack.last()?;
    let call_name = extract_ident_before(source_before_cursor, open)?;
    let args_prefix = &source_before_cursor[open + 1..];

    let mut segment_start = 0usize;
    let mut arg_index = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (idx, ch) in args_prefix.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == q {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                segment_start = idx + ch.len_utf8();
                arg_index += 1;
            }
            _ => {}
        }
    }

    let tail = args_prefix[segment_start..].trim_start();
    if tail.is_empty() {
        return Some((call_name, arg_index));
    }

    let mut chars = tail.char_indices();
    if let Some((_, first)) = chars.next()
        && is_ident_start(first)
    {
        let mut end = first.len_utf8();
        while end < tail.len() {
            let Some(ch) = tail[end..].chars().next() else {
                break;
            };
            if is_ident_char(ch) {
                end += ch.len_utf8();
            } else {
                break;
            }
        }

        let rest = tail[end..].trim_start();
        if rest.starts_with(':') {
            return None;
        }
    }

    Some((call_name, arg_index))
}

fn builtin_signature_text(name: &str, sig: &fresco::language_docs::DocBuiltinSignature) -> String {
    let params = sig
        .args
        .iter()
        .map(|arg| {
            if arg.required {
                format!("{}: {}", arg.name, arg.ty)
            } else {
                format!("{}?: {}", arg.name, arg.ty)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let returns = if sig.returns.is_empty() {
        "?".to_string()
    } else {
        sig.returns.join(" | ")
    };
    if let Some(receiver) = sig.receiver.as_deref() {
        format!("{} |> {}({}) -> {}", receiver, name, params, returns)
    } else {
        format!("{}({}) -> {}", name, params, returns)
    }
}

fn stdlib_signature_text(stdlib: &fresco::language_docs::DocStdlibExport) -> String {
    let params = stdlib
        .params
        .iter()
        .map(|arg| format!("{}: {}", arg.name, arg.ty_name))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = if stdlib.ret_ty.trim().is_empty() {
        "?"
    } else {
        stdlib.ret_ty.as_str()
    };
    format!("{}({}) -> {}", stdlib.name, params, ret)
}

fn function_snippet(name: &str, arg_names: &[String]) -> String {
    if arg_names.is_empty() {
        return format!("{}()", name);
    }
    let body = arg_names
        .iter()
        .enumerate()
        .map(|(idx, arg)| format!("{}: ${{{}}}", arg, idx + 1))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}({})", name, body)
}

fn completion_info(signature: Option<&str>, docs: Option<&str>) -> Option<String> {
    match (signature, docs) {
        (Some(sig), Some(text)) if !text.trim().is_empty() => Some(format!("{}\n\n{}", sig, text)),
        (Some(sig), _) => Some(sig.to_string()),
        (_, Some(text)) if !text.trim().is_empty() => Some(text.to_string()),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
struct NamedArgValueConstraint {
    ty_name: Option<String>,
    enum_type: Option<String>,
}

impl NamedArgValueConstraint {
    fn is_typed(&self) -> bool {
        self.ty_name.is_some() || self.enum_type.is_some()
    }
}

fn geometry_bindings(source: &str) -> Vec<(String, u8)> {
    use fresco::lexer::Token;
    let tokens = fresco::lexer::lex_spanned(source);
    let mut calls: Vec<(bool, Option<String>)> = Vec::new();
    let mut scopes: Vec<Vec<(String, u8)>> = vec![Vec::new()];
    let mut pending = None;
    for (i, (token, _)) in tokens.iter().enumerate() {
        match token {
            Token::LParen => calls.push((
                i > 0 && matches!(&tokens[i-1].0, Token::Ident(name) if name == "cells"),
                None,
            )),
            Token::RParen => {
                if let Some((true, name)) = calls.pop() {
                    pending = name;
                }
            }
            Token::Ident(name) if name == "cell" && calls.last().is_some_and(|call| call.0) => {
                if matches!(tokens.get(i + 1).map(|t| &t.0), Some(Token::Colon))
                    && let Some((Token::Ident(binding), _)) = tokens.get(i + 2)
                {
                    calls.last_mut().expect("cell call exists").1 = Some(binding.to_string());
                }
            }
            Token::LBrace => scopes.push(
                pending
                    .take()
                    .map(|name| vec![(name, 1)])
                    .unwrap_or_default(),
            ),
            Token::RBrace => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
            }
            Token::Ident(ty)
                if matches!(tokens.get(i + 1).map(|t| &t.0), Some(Token::Ident(_)))
                    && matches!(tokens.get(i + 2).map(|t| &t.0), Some(Token::Eq)) =>
            {
                if let Some((Token::Ident(name), _)) = tokens.get(i + 1) {
                    scopes
                        .last_mut()
                        .expect("scope exists")
                        .push((name.to_string(), if ty == "contour" { 2 } else { 0 }));
                }
            }
            Token::Let => {
                if let Some((Token::Ident(name), _)) = tokens.get(i + 1) {
                    let lookup = |name: &str| {
                        scopes
                            .iter()
                            .rev()
                            .flat_map(|scope| scope.iter().rev())
                            .find(|(key, _)| key == name)
                            .map_or(0, |(_, kind)| *kind)
                    };
                    let kind = if matches!(tokens.get(i + 2).map(|t| &t.0), Some(Token::Colon))
                        && matches!(tokens.get(i + 3).map(|t| &t.0), Some(Token::Ident(ty)) if ty == "contour")
                    {
                        2
                    } else if let Some((Token::Ident(base), _)) = tokens.get(i + 3) {
                        if matches!(tokens.get(i + 4).map(|t| &t.0), Some(Token::Dot))
                            && matches!(tokens.get(i + 5).map(|t| &t.0), Some(Token::Ident(method)) if method == "contour")
                            && lookup(base) == 1
                        {
                            2
                        } else if matches!(
                            tokens.get(i + 4).map(|t| &t.0),
                            None | Some(Token::Newline | Token::Semicolon | Token::RBrace)
                        ) {
                            lookup(base)
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    scopes
                        .last_mut()
                        .expect("root scope exists")
                        .push((name.to_string(), kind));
                }
            }
            _ => {}
        }
    }
    let mut seen = std::collections::HashSet::new();
    scopes
        .into_iter()
        .rev()
        .flat_map(|scope| scope.into_iter().rev())
        .filter(|(name, _)| seen.insert(name.clone()))
        .collect()
}

fn cell_member_completion(source: &str) -> Option<Vec<CompletionItem>> {
    use fresco::lexer::Token;
    let tokens = fresco::lexer::lex_spanned(source);
    let (dot, prefix) = match tokens.last()?.0 {
        Token::Dot => (tokens.len() - 1, ""),
        Token::Ident(ref name)
            if tokens.len() >= 2 && matches!(tokens[tokens.len() - 2].0, Token::Dot) =>
        {
            (tokens.len() - 2, name.as_str())
        }
        _ => return None,
    };
    let Token::Ident(ref receiver) = tokens.get(dot.checked_sub(1)?)?.0 else {
        return None;
    };
    let kind = geometry_bindings(source)
        .iter()
        .find(|(name, _)| name == receiver.as_str())
        .map_or(0, |(_, kind)| *kind);
    let (members, context) = match kind {
        1 => (fresco::language::cell_members(), "cell-method"),
        2 => (fresco::language::contour_members(), "contour-method"),
        _ => return None,
    };
    let metadata = example_language_docs();
    Some(
        members
            .iter()
            .filter(|(name, _, _)| name.starts_with(prefix))
            .map(|&(name, ty, docs)| {
                let callable = metadata
                    .callable_reference
                    .iter()
                    .find(|decl| decl.name == name && decl.context == context);
                let signature = callable.map(|decl| {
                    format!(
                        "{name}({}) -> {}",
                        decl.args
                            .iter()
                            .map(|a| format!("{}: {}", a.name, a.ty))
                            .collect::<Vec<_>>()
                            .join(", "),
                        decl.returns.join(" | ")
                    )
                });
                CompletionItem {
                    label: name.into(),
                    insert_text: name.into(),
                    kind: if ty == "method" {
                        "function"
                    } else {
                        "property"
                    }
                    .into(),
                    detail: format!("cell member ({ty})"),
                    boost: 300,
                    allowed: true,
                    signature,
                    documentation: Some(docs.into()),
                    snippet: None,
                    info: Some(docs.into()),
                }
            })
            .collect(),
    )
}

fn completion_items_for_cursor(source: &str, cursor_utf8: usize) -> Vec<CompletionItem> {
    completion_items_with_files(source, cursor_utf8, "main.fr", &HashMap::new())
}

fn completion_items_with_files(
    source: &str,
    cursor_utf8: usize,
    filename: &str,
    files: &HashMap<String, String>,
) -> Vec<CompletionItem> {
    use std::collections::{HashMap, HashSet};

    let mut docs = example_language_docs();
    docs.callable_reference
        .extend(fresco::driver::material_callables(
            source,
            cursor_utf8,
            filename,
            files,
        ));
    let cursor = cursor_utf8.min(source.len());
    if let Some(items) = cell_member_completion(source_prefix(source, cursor)) {
        return items;
    }
    let source_before_cursor = source_prefix(source, cursor);
    let line_prefix = current_line_prefix(source, cursor);
    let in_pipe_context = is_pipe_context_at_cursor(source_before_cursor, line_prefix);
    let frag = trailing_ident_fragment(line_prefix).to_ascii_lowercase();
    let expectations = completion_expectations(
        source,
        cursor,
        &docs.keywords,
        &docs.type_keywords,
        &docs.units,
    );

    let mut items: Vec<CompletionItem> = Vec::new();
    let mut seen_labels: HashSet<String> = HashSet::new();
    let mut named_arg_value_constraint = NamedArgValueConstraint::default();
    let mut in_named_arg_value_context = false;

    let builtin_by_name: HashMap<&str, &fresco::language_docs::DocBuiltin> = docs
        .builtin_reference
        .iter()
        .map(|b| (b.name.as_str(), b))
        .collect();
    let stdlib_by_name: HashMap<&str, &fresco::language_docs::DocStdlibExport> = docs
        .stdlib_exports
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    let callable_by_name: HashMap<&str, &fresco::language_docs::DocCallable> = docs
        .callable_reference
        .iter()
        .map(|callable| (callable.name.as_str(), callable))
        .collect();
    let space_transform_by_name: HashMap<&str, &fresco::language_docs::DocSpaceTransform> = docs
        .space_transform_reference
        .iter()
        .map(|x| (x.name.as_str(), x))
        .collect();
    let enum_values_by_type: HashMap<&str, Vec<&str>> = docs
        .enum_variants_by_type
        .iter()
        .map(|(k, values)| (k.as_str(), values.iter().map(String::as_str).collect()))
        .collect();
    let type_docs: HashMap<&str, &str> = docs
        .types
        .iter()
        .map(|t| (t.name.as_str(), t.docs.as_str()))
        .collect();
    let blend_mode_docs: HashMap<&str, &str> = docs
        .enums
        .iter()
        .find(|e| e.name == "BlendMode")
        .map(|e| {
            e.variants
                .iter()
                .map(|v| (v.name.as_str(), v.docs.as_str()))
                .collect()
        })
        .unwrap_or_default();

    let mut push_item = |item: CompletionItem| {
        let key = item.label.to_ascii_lowercase();
        if seen_labels.insert(key) {
            items.push(item);
        }
    };

    if is_blend_value_context(line_prefix) {
        for mode in &docs.blend_modes {
            let mode_docs = blend_mode_docs.get(mode.as_str()).copied();
            push_item(CompletionItem {
                label: mode.clone(),
                insert_text: mode.clone(),
                kind: "enum".to_string(),
                detail: "Fresco blend mode".to_string(),
                boost: 230,
                allowed: true,
                signature: None,
                documentation: mode_docs.map(ToString::to_string),
                snippet: None,
                info: completion_info(None, mode_docs),
            });
        }
    } else if is_in_keyword_context(line_prefix) {
        push_item(CompletionItem {
            label: "space".to_string(),
            insert_text: "space ".to_string(),
            kind: "keyword".to_string(),
            detail: "in space statement".to_string(),
            boost: 220,
            allowed: true,
            signature: None,
            documentation: Some("Start an in space transform chain.".to_string()),
            snippet: None,
            info: Some("Start an in space transform chain.".to_string()),
        });
    } else if in_pipe_context {
        for builtin in &docs.builtin_reference {
            let Some(sig) = builtin
                .signatures
                .iter()
                .find(|s| is_shape_pipeline_signature(s))
            else {
                continue;
            };

            let sig_text = Some(builtin_signature_text(&builtin.name, sig));
            let snippet = Some({
                function_snippet(
                    &builtin.name,
                    &sig.args.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                )
            });
            push_item(CompletionItem {
                label: builtin.name.clone(),
                insert_text: format!("{}()", builtin.name),
                kind: "function".to_string(),
                detail: format!("Builtin ({})", builtin.category),
                boost: 180,
                allowed: true,
                signature: sig_text.clone(),
                documentation: Some(builtin.summary.clone()),
                snippet,
                info: completion_info(sig_text.as_deref(), Some(builtin.summary.as_str())),
            });
        }
    } else if let Some((call_name, used_named)) = active_call_context(source_before_cursor) {
        if let Some(callable) = callable_by_name.get(call_name.as_str()) {
            let params = callable
                .args
                .iter()
                .map(|arg| {
                    if arg.required {
                        format!("{}: {}", arg.name, arg.ty)
                    } else {
                        format!("{}?: {}", arg.name, arg.ty)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let returns = if callable.returns.is_empty() {
                "?".to_string()
            } else {
                callable.returns.join(" | ")
            };
            let signature = format!("{}({}) -> {}", callable.name, params, returns);

            for arg in &callable.args {
                if used_named.contains(arg.name.as_str()) {
                    continue;
                }
                push_item(CompletionItem {
                    label: format!("{}:", arg.name),
                    insert_text: format!("{}: ", arg.name),
                    kind: "property".to_string(),
                    detail: format!("{} arg ({})", call_name, arg.ty),
                    boost: 248,
                    allowed: true,
                    signature: Some(signature.clone()),
                    documentation: Some(arg.docs.clone()),
                    snippet: None,
                    info: completion_info(Some(signature.as_str()), Some(arg.docs.as_str())),
                });
            }
        }

        if let Some(space_transform) = space_transform_by_name.get(call_name.as_str()) {
            let signature = {
                let params = space_transform
                    .args
                    .iter()
                    .map(|arg| {
                        if arg.required {
                            format!("{}: {}", arg.name, arg.ty)
                        } else {
                            format!("{}?: {}", arg.name, arg.ty)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({})", space_transform.name, params)
            };

            for arg in &space_transform.args {
                if used_named.contains(arg.name.as_str()) {
                    continue;
                }
                push_item(CompletionItem {
                    label: format!("{}:", arg.name),
                    insert_text: format!("{}: ", arg.name),
                    kind: "property".to_string(),
                    detail: format!("{} arg ({})", space_transform.name, arg.ty),
                    boost: 245,
                    allowed: true,
                    signature: Some(signature.clone()),
                    documentation: if arg.docs.trim().is_empty() {
                        None
                    } else {
                        Some(arg.docs.clone())
                    },
                    snippet: None,
                    info: completion_info(
                        Some(signature.as_str()),
                        if arg.docs.trim().is_empty() {
                            None
                        } else {
                            Some(arg.docs.as_str())
                        },
                    ),
                });
            }
        }

        if let Some(builtin) = builtin_by_name.get(call_name.as_str()) {
            for sig in &builtin.signatures {
                let signature = builtin_signature_text(&builtin.name, sig);
                for arg in &sig.args {
                    if used_named.contains(arg.name.as_str()) {
                        continue;
                    }
                    push_item(CompletionItem {
                        label: format!("{}:", arg.name),
                        insert_text: format!("{}: ", arg.name),
                        kind: "property".to_string(),
                        detail: format!("{} arg ({})", builtin.name, arg.ty),
                        boost: 240,
                        allowed: true,
                        signature: Some(signature.clone()),
                        documentation: if arg.docs.trim().is_empty() {
                            None
                        } else {
                            Some(arg.docs.clone())
                        },
                        snippet: None,
                        info: completion_info(
                            Some(signature.as_str()),
                            if arg.docs.trim().is_empty() {
                                None
                            } else {
                                Some(arg.docs.as_str())
                            },
                        ),
                    });
                }
            }

            if let Some(discriminator) = builtin.discriminator.as_ref()
                && !used_named.contains(discriminator.name.as_str())
            {
                push_item(CompletionItem {
                    label: format!("{}:", discriminator.name),
                    insert_text: format!("{}: ", discriminator.name),
                    kind: "property".to_string(),
                    detail: if let Some(enum_type) = discriminator.enum_type.as_deref() {
                        format!("{} arg ({})", builtin.name, enum_type)
                    } else {
                        format!("{} discriminator", builtin.name)
                    },
                    boost: 246,
                    allowed: true,
                    signature: None,
                    documentation: Some("discriminator argument".to_string()),
                    snippet: None,
                    info: Some("discriminator argument".to_string()),
                });
            }
        }

        if let Some(stdlib) = stdlib_by_name.get(call_name.as_str()) {
            let signature = stdlib_signature_text(stdlib);
            for arg in &stdlib.params {
                if used_named.contains(arg.name.as_str()) {
                    continue;
                }
                let arg_docs = format!("{} parameter for {}", arg.name, stdlib.name);
                let arg_docs = arg.docs.clone().unwrap_or(arg_docs);
                push_item(CompletionItem {
                    label: format!("{}:", arg.name),
                    insert_text: format!("{}: ", arg.name),
                    kind: "property".to_string(),
                    detail: format!("{} arg ({})", stdlib.name, arg.ty_name),
                    boost: 235,
                    allowed: true,
                    signature: Some(signature.clone()),
                    documentation: Some(arg_docs.clone()),
                    snippet: None,
                    info: completion_info(Some(signature.as_str()), Some(arg_docs.as_str())),
                });
            }
        }

        if let Some((value_call, value_arg)) = active_named_arg_value_context(source_before_cursor)
            && value_call == call_name
        {
            in_named_arg_value_context = true;
            let mut push_enum_values = |enum_type: &str, detail_prefix: &str| {
                if let Some(values) = enum_values_by_type.get(enum_type) {
                    for enum_value in values {
                        push_item(CompletionItem {
                            label: (*enum_value).to_string(),
                            insert_text: (*enum_value).to_string(),
                            kind: "enum".to_string(),
                            detail: format!("{} ({})", detail_prefix, enum_type),
                            boost: 255,
                            allowed: true,
                            signature: None,
                            documentation: None,
                            snippet: None,
                            info: None,
                        });
                    }
                }
            };

            if let Some(callable) = callable_by_name.get(call_name.as_str())
                && let Some(arg) = callable.args.iter().find(|arg| arg.name == value_arg)
            {
                named_arg_value_constraint.ty_name = Some(arg.ty.clone());
                if enum_values_by_type.contains_key(arg.ty.as_str()) {
                    named_arg_value_constraint.enum_type = Some(arg.ty.clone());
                    push_enum_values(&arg.ty, "Callable enum");
                }
            }

            if let Some(space_transform) = space_transform_by_name.get(call_name.as_str())
                && let Some(arg) = space_transform
                    .args
                    .iter()
                    .find(|arg| arg.name == value_arg)
            {
                named_arg_value_constraint.ty_name = Some(arg.ty.clone());
                named_arg_value_constraint
                    .enum_type
                    .clone_from(&arg.enum_type);
                if let Some(enum_type) = arg.enum_type.as_deref() {
                    push_enum_values(enum_type, "Space transform enum");
                }
            }

            if let Some(builtin) = builtin_by_name.get(call_name.as_str()) {
                for sig in &builtin.signatures {
                    if let Some(arg) = sig.args.iter().find(|arg| arg.name == value_arg) {
                        if named_arg_value_constraint.ty_name.is_none() {
                            named_arg_value_constraint.ty_name = Some(arg.ty.clone());
                        }
                        if named_arg_value_constraint.enum_type.is_none() {
                            named_arg_value_constraint
                                .enum_type
                                .clone_from(&arg.enum_type);
                        }
                        if let Some(enum_type) = arg.enum_type.as_deref() {
                            push_enum_values(enum_type, "Builtin enum");
                        }
                    }
                }

                if let Some(discriminator) = builtin.discriminator.as_ref()
                    && value_arg == discriminator.name
                {
                    named_arg_value_constraint.ty_name = Some(discriminator.name.clone());
                    named_arg_value_constraint
                        .enum_type
                        .clone_from(&discriminator.enum_type);
                    if let Some(enum_type) = discriminator.enum_type.as_deref() {
                        push_enum_values(enum_type, "Builtin enum");
                    } else {
                        for variant in &discriminator.variants {
                            push_item(CompletionItem {
                                label: variant.clone(),
                                insert_text: variant.clone(),
                                kind: "enum".to_string(),
                                detail: "Builtin discriminator value".to_string(),
                                boost: 255,
                                allowed: true,
                                signature: None,
                                documentation: None,
                                snippet: None,
                                info: None,
                            });
                        }
                    }
                }
            }

            if let Some(stdlib) = stdlib_by_name.get(call_name.as_str())
                && let Some(arg) = stdlib.params.iter().find(|arg| arg.name == value_arg)
            {
                named_arg_value_constraint.ty_name = Some(arg.ty_name.clone());
            }

            // Keep parser-token fallback only for untyped named-value contexts.
            // Typed contexts must remain metadata-constrained (enum/type aware).
            if !named_arg_value_constraint.is_typed() {
                for token in &expectations.exact_tokens {
                    if !is_word_like_token(token)
                        || matches!(
                            token.as_str(),
                            "identifier"
                                | "number"
                                | "string literal"
                                | "color literal"
                                | "newline"
                                | "end of input"
                                | "true"
                                | "false"
                        )
                        || docs.keywords.iter().any(|k| k.eq_ignore_ascii_case(token))
                        || docs
                            .type_keywords
                            .iter()
                            .any(|k| k.eq_ignore_ascii_case(token))
                        || docs.units.iter().any(|u| u.eq_ignore_ascii_case(token))
                    {
                        continue;
                    }

                    push_item(CompletionItem {
                        label: token.clone(),
                        insert_text: token.clone(),
                        kind: "enum".to_string(),
                        detail: "Expected value".to_string(),
                        boost: 250,
                        allowed: true,
                        signature: None,
                        documentation: None,
                        snippet: None,
                        info: None,
                    });
                }
            }
        }

        if let Some((value_call, arg_index)) =
            active_positional_arg_value_context(source_before_cursor)
            && value_call == call_name
        {
            in_named_arg_value_context = true;
            let mut push_enum_values = |enum_type: &str, detail_prefix: &str| {
                if let Some(values) = enum_values_by_type.get(enum_type) {
                    for enum_value in values {
                        push_item(CompletionItem {
                            label: (*enum_value).to_string(),
                            insert_text: (*enum_value).to_string(),
                            kind: "enum".to_string(),
                            detail: format!("{} ({})", detail_prefix, enum_type),
                            boost: 255,
                            allowed: true,
                            signature: None,
                            documentation: None,
                            snippet: None,
                            info: None,
                        });
                    }
                }
            };

            if let Some(space_transform) = space_transform_by_name.get(call_name.as_str())
                && let Some(arg) = space_transform.args.get(arg_index)
            {
                named_arg_value_constraint.ty_name = Some(arg.ty.clone());
                named_arg_value_constraint
                    .enum_type
                    .clone_from(&arg.enum_type);
                if let Some(enum_type) = arg.enum_type.as_deref() {
                    push_enum_values(enum_type, "Space transform enum");
                }
            }

            if let Some(builtin) = builtin_by_name.get(call_name.as_str()) {
                for sig in &builtin.signatures {
                    if let Some(arg) = sig.args.get(arg_index) {
                        if named_arg_value_constraint.ty_name.is_none() {
                            named_arg_value_constraint.ty_name = Some(arg.ty.clone());
                        }
                        if named_arg_value_constraint.enum_type.is_none() {
                            named_arg_value_constraint
                                .enum_type
                                .clone_from(&arg.enum_type);
                        }
                        if let Some(enum_type) = arg.enum_type.as_deref() {
                            push_enum_values(enum_type, "Builtin enum");
                        }
                    }
                }

                if arg_index == 0
                    && let Some(discriminator) = builtin.discriminator.as_ref()
                {
                    if named_arg_value_constraint.ty_name.is_none() {
                        named_arg_value_constraint.ty_name = Some(discriminator.name.clone());
                    }
                    if named_arg_value_constraint.enum_type.is_none() {
                        named_arg_value_constraint
                            .enum_type
                            .clone_from(&discriminator.enum_type);
                    }

                    if let Some(enum_type) = discriminator.enum_type.as_deref() {
                        push_enum_values(enum_type, "Builtin enum");
                    } else {
                        for variant in &discriminator.variants {
                            push_item(CompletionItem {
                                label: variant.clone(),
                                insert_text: variant.clone(),
                                kind: "enum".to_string(),
                                detail: "Builtin discriminator value".to_string(),
                                boost: 255,
                                allowed: true,
                                signature: None,
                                documentation: None,
                                snippet: None,
                                info: None,
                            });
                        }
                    }
                }
            }

            if let Some(stdlib) = stdlib_by_name.get(call_name.as_str())
                && let Some(arg) = stdlib.params.get(arg_index)
            {
                named_arg_value_constraint.ty_name = Some(arg.ty_name.clone());
            }

            if call_name == "blend" && arg_index == 0 {
                for mode in &docs.blend_modes {
                    let mode_docs = blend_mode_docs.get(mode.as_str()).copied();
                    push_item(CompletionItem {
                        label: mode.clone(),
                        insert_text: mode.clone(),
                        kind: "enum".to_string(),
                        detail: "Fresco blend mode".to_string(),
                        boost: 260,
                        allowed: true,
                        signature: None,
                        documentation: mode_docs.map(ToString::to_string),
                        snippet: None,
                        info: completion_info(None, mode_docs),
                    });
                }
            }

            // Keep parser-token fallback only for untyped positional-value contexts.
            // Typed contexts must remain metadata-constrained (enum/type aware).
            if !named_arg_value_constraint.is_typed() {
                for token in &expectations.exact_tokens {
                    if !is_word_like_token(token)
                        || matches!(
                            token.as_str(),
                            "identifier"
                                | "number"
                                | "string literal"
                                | "color literal"
                                | "newline"
                                | "end of input"
                                | "true"
                                | "false"
                        )
                        || docs.keywords.iter().any(|k| k.eq_ignore_ascii_case(token))
                        || docs
                            .type_keywords
                            .iter()
                            .any(|k| k.eq_ignore_ascii_case(token))
                        || docs.units.iter().any(|u| u.eq_ignore_ascii_case(token))
                    {
                        continue;
                    }

                    push_item(CompletionItem {
                        label: token.clone(),
                        insert_text: token.clone(),
                        kind: "enum".to_string(),
                        detail: "Expected value".to_string(),
                        boost: 250,
                        allowed: true,
                        signature: None,
                        documentation: None,
                        snippet: None,
                        info: None,
                    });
                }
            }
        }
    } else if is_type_context(line_prefix) {
        for ty in &docs.type_keywords {
            let docs_text = type_docs.get(ty.as_str()).copied();
            push_item(CompletionItem {
                label: ty.clone(),
                insert_text: ty.clone(),
                kind: "type".to_string(),
                detail: "Fresco type".to_string(),
                boost: 210,
                allowed: true,
                signature: Some(format!("type {}", ty)),
                documentation: docs_text.map(ToString::to_string),
                snippet: None,
                info: completion_info(Some(format!("type {}", ty).as_str()), docs_text),
            });
        }
    } else {
        for kw in &docs.keywords {
            let mut item = CompletionItem {
                label: kw.clone(),
                insert_text: kw.clone(),
                kind: "keyword".to_string(),
                detail: "Fresco keyword".to_string(),
                boost: 100,
                allowed: true,
                signature: None,
                documentation: None,
                snippet: None,
                info: None,
            };
            item.allowed = is_allowed_by_expectations(&item, &expectations);
            if !item.allowed {
                item.boost -= 80;
            }
            push_item(item);
        }
        for ty in &docs.type_keywords {
            let docs_text = type_docs.get(ty.as_str()).copied();
            let mut item = CompletionItem {
                label: ty.clone(),
                insert_text: ty.clone(),
                kind: "type".to_string(),
                detail: "Fresco type".to_string(),
                boost: 95,
                allowed: true,
                signature: Some(format!("type {}", ty)),
                documentation: docs_text.map(ToString::to_string),
                snippet: None,
                info: completion_info(Some(format!("type {}", ty).as_str()), docs_text),
            };
            item.allowed = is_allowed_by_expectations(&item, &expectations);
            if !item.allowed {
                item.boost -= 80;
            }
            push_item(item);
        }
        for unit in &docs.units {
            let mut item = CompletionItem {
                label: unit.clone(),
                insert_text: unit.clone(),
                kind: "constant".to_string(),
                detail: "Fresco unit".to_string(),
                boost: 90,
                allowed: true,
                signature: None,
                documentation: None,
                snippet: None,
                info: None,
            };
            item.allowed = is_allowed_by_expectations(&item, &expectations);
            if !item.allowed {
                item.boost -= 80;
            }
            push_item(item);
        }
        for builtin in &docs.builtin_reference {
            let sig_text = builtin
                .signatures
                .first()
                .map(|sig| builtin_signature_text(&builtin.name, sig));
            let snippet = builtin.signatures.first().map(|sig| {
                function_snippet(
                    &builtin.name,
                    &sig.args.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                )
            });
            let mut item = CompletionItem {
                label: builtin.name.clone(),
                insert_text: format!("{}()", builtin.name),
                kind: "function".to_string(),
                detail: format!("Builtin ({})", builtin.category),
                boost: 130,
                allowed: true,
                signature: sig_text.clone(),
                documentation: Some(builtin.summary.clone()),
                snippet,
                info: completion_info(sig_text.as_deref(), Some(builtin.summary.as_str())),
            };
            item.allowed = is_allowed_by_expectations(&item, &expectations);
            if !item.allowed {
                item.boost -= 80;
            }
            push_item(item);
        }
        for callable in &docs.callable_reference {
            let params = callable
                .args
                .iter()
                .map(|arg| {
                    if arg.required {
                        format!("{}: {}", arg.name, arg.ty)
                    } else {
                        format!("{}?: {}", arg.name, arg.ty)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let returns = if callable.returns.is_empty() {
                "?".to_string()
            } else {
                callable.returns.join(" | ")
            };
            let sig_text = format!("{}({}) -> {}", callable.name, params, returns);
            let snippet = function_snippet(
                &callable.name,
                &callable
                    .args
                    .iter()
                    .map(|arg| arg.name.clone())
                    .collect::<Vec<_>>(),
            );
            let mut item = CompletionItem {
                label: callable.name.clone(),
                insert_text: format!("{}()", callable.name),
                kind: "function".to_string(),
                detail: format!("Fresco callable ({})", callable.context),
                boost: 125,
                allowed: true,
                signature: Some(sig_text.clone()),
                documentation: Some(callable.summary.clone()),
                snippet: Some(snippet),
                info: completion_info(Some(sig_text.as_str()), Some(callable.summary.as_str())),
            };
            item.allowed = is_allowed_by_expectations(&item, &expectations);
            if !item.allowed {
                item.boost -= 80;
            }
            push_item(item);
        }
        for stdlib in &docs.stdlib_exports {
            let sig_text = stdlib_signature_text(stdlib);
            let snippet = function_snippet(
                &stdlib.name,
                &stdlib
                    .params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>(),
            );
            let mut item = CompletionItem {
                label: stdlib.name.clone(),
                insert_text: format!("{}()", stdlib.name),
                kind: "function".to_string(),
                detail: "Fresco stdlib function".to_string(),
                boost: 110,
                allowed: true,
                signature: Some(sig_text.clone()),
                documentation: stdlib.docs.clone(),
                snippet: Some(snippet),
                info: completion_info(Some(sig_text.as_str()), stdlib.docs.as_deref()),
            };
            item.allowed = is_allowed_by_expectations(&item, &expectations);
            if !item.allowed {
                item.boost -= 80;
            }
            push_item(item);
        }
    }

    // When a named-argument value position carries explicit type info
    // (enum/layer/color/etc), avoid suggesting untyped scope symbols.
    // This keeps completions semantically valid instead of token-valid.
    if !named_arg_value_constraint.is_typed() && !in_named_arg_value_context {
        for ident in collect_identifiers_in_scope(source, cursor) {
            let mut item = CompletionItem {
                label: ident.clone(),
                insert_text: ident,
                kind: "variable".to_string(),
                detail: "Symbol in scope".to_string(),
                boost: 50,
                allowed: true,
                signature: None,
                documentation: None,
                snippet: None,
                info: None,
            };
            item.allowed = is_allowed_by_expectations(&item, &expectations);
            if !item.allowed {
                item.boost -= 80;
            }
            push_item(item);
        }
    }

    if named_arg_value_constraint.ty_name.as_deref() == Some("contour") {
        items.clear();
        for (name, kind) in geometry_bindings(source_before_cursor) {
            if kind != 2 {
                continue;
            }
            items.push(CompletionItem {
                label: name.clone(),
                insert_text: name,
                kind: "variable".into(),
                detail: "Local (contour)".into(),
                boost: 300,
                allowed: true,
                signature: None,
                documentation: Some(
                    "Closed cell boundary with distance, progress, length and point queries."
                        .into(),
                ),
                snippet: None,
                info: None,
            });
        }
    }

    // An enum value slot accepts its declared variants, not argument names or
    // unrelated functions that happen to be syntactically valid identifiers.
    if active_named_arg_value_context(source_before_cursor).is_some()
        && let Some(enum_type) = named_arg_value_constraint.enum_type.as_deref()
        && let Some(variants) = enum_values_by_type.get(enum_type)
    {
        items.retain(|item| variants.contains(&item.label.as_str()));
    }

    let mut filtered = if frag.is_empty() {
        items
    } else {
        items
            .into_iter()
            .filter(|item| item.label.to_ascii_lowercase().starts_with(&frag))
            .collect()
    };

    filtered.sort_by(|a, b| {
        b.allowed
            .cmp(&a.allowed)
            .then(b.boost.cmp(&a.boost))
            .then_with(|| a.label.cmp(&b.label))
    });

    filtered
}

#[wasm_bindgen]
pub fn fresco_completion_items(source: &str, cursor_utf8: usize) -> JsValue {
    let filtered = completion_items_for_cursor(source, cursor_utf8);

    serde_wasm_bindgen::to_value(&filtered).expect("failed to serialize completion items")
}

#[wasm_bindgen]
pub fn fresco_completion_items_with_files(
    source: &str,
    cursor_utf8: usize,
    filename: &str,
    files: JsValue,
) -> Result<JsValue, JsValue> {
    let files = workbook_files(files)?.unwrap_or_default();
    let items = completion_items_with_files(source, cursor_utf8, filename, &files);
    Ok(serde_wasm_bindgen::to_value(&items).expect("failed to serialize engine completion items"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_example_sources_preserve_the_embedded_profile() {
        let json = example_engine_sources_json();
        let sources: HashMap<String, String> = serde_json::from_str(&json).unwrap();
        assert_eq!(sources, fresco_example_engine::source_files());
        assert!(sources.contains_key(fresco_example_engine::ENTRYPOINT));
        assert_eq!(json, example_engine_sources_json());
    }

    #[test]
    fn export_wasm_contract_types() {
        let cfg = ts_rs::Config::default();
        export_contract_types(&cfg);
    }

    fn export_contract_types(cfg: &ts_rs::Config) {
        // CI runs only this exporter, so each public contract must bring along
        // its transitive types without relying on ts-rs auto-export tests.
        CompileDiagnostic::export_all(cfg).expect("failed to export CompileDiagnostic");
        CompileSuccess::export_all(cfg).expect("failed to export CompileSuccess");
        CompileTimings::export_all(cfg).expect("failed to export CompileTimings");
        PipelineTimings::export_all(cfg).expect("failed to export PipelineTimings");
        CompileFailure::export_all(cfg).expect("failed to export CompileFailure");
        <ManifestRoot as TS>::export_all(cfg).expect("failed to export ManifestRoot");
        ManifestVertexFactory::export_all(cfg).expect("failed to export ManifestVertexFactory");
        ManifestVertexFactoryBinding::export_all(cfg)
            .expect("failed to export ManifestVertexFactoryBinding");
        ManifestVertexAttribute::export_all(cfg).expect("failed to export ManifestVertexAttribute");
        ManifestSurfaceEvaluationContract::export_all(cfg)
            .expect("failed to export ManifestSurfaceEvaluationContract");
        ManifestSurfaceEvaluationVariant::export_all(cfg)
            .expect("failed to export ManifestSurfaceEvaluationVariant");
        ManifestSurfaceEvaluationVariantBinding::export_all(cfg)
            .expect("failed to export ManifestSurfaceEvaluationVariantBinding");
        ManifestSurfaceContractRequirements::export_all(cfg)
            .expect("failed to export ManifestSurfaceContractRequirements");
        ManifestSurfaceCustomChannel::export_all(cfg)
            .expect("failed to export ManifestSurfaceCustomChannel");
        <ManifestSurface as TS>::export_all(cfg).expect("failed to export ManifestSurface");
        ManifestMeshPass::export_all(cfg).expect("failed to export ManifestMeshPass");
        ManifestSurfaceRequirements::export_all(cfg)
            .expect("failed to export ManifestSurfaceRequirements");
        ManifestSurfaceVertexStage::export_all(cfg)
            .expect("failed to export ManifestSurfaceVertexStage");
        ManifestSurfaceUvChannelRequirement::export_all(cfg)
            .expect("failed to export ManifestSurfaceUvChannelRequirement");
        <ManifestSurfaceParam as TS>::export_all(cfg)
            .expect("failed to export ManifestSurfaceParam");
        ManifestGlobalUniform::export_all(cfg).expect("failed to export ManifestGlobalUniform");
        ManifestGlobalUniformField::export_all(cfg)
            .expect("failed to export ManifestGlobalUniformField");
        <ManifestCanvas as TS>::export_all(cfg).expect("failed to export ManifestCanvas");
        ManifestEnginePass::export_all(cfg).expect("failed to export ManifestEnginePass");
        ManifestEnginePassVariant::export_all(cfg)
            .expect("failed to export ManifestEnginePassVariant");
        ManifestEnginePassVariantBinding::export_all(cfg)
            .expect("failed to export ManifestEnginePassVariantBinding");
        <ManifestParam as TS>::export_all(cfg).expect("failed to export ManifestParam");
        ManifestParamTypeInfo::export_all(cfg).expect("failed to export ManifestParamTypeInfo");
        ManifestStorageParam::export_all(cfg).expect("failed to export ManifestStorageParam");
        ManifestTexture::export_all(cfg).expect("failed to export ManifestTexture");
        ManifestTextureMetadata::export_all(cfg).expect("failed to export ManifestTextureMetadata");
        ManifestTextureChannel::export_all(cfg).expect("failed to export ManifestTextureChannel");
        ManifestSampler::export_all(cfg).expect("failed to export ManifestSampler");
        ManifestPathBuffer::export_all(cfg).expect("failed to export ManifestPathBuffer");
        ManifestPassPlan::export_all(cfg).expect("failed to export ManifestPassPlan");
        ManifestPass::export_all(cfg).expect("failed to export ManifestPass");
        ManifestPassInput::export_all(cfg).expect("failed to export ManifestPassInput");
        ManifestIntermediateTarget::export_all(cfg)
            .expect("failed to export ManifestIntermediateTarget");
        ManifestEdge::export_all(cfg).expect("failed to export ManifestEdge");
        StdlibExport::export_all(cfg).expect("failed to export StdlibExport");
        StdlibExportParam::export_all(cfg).expect("failed to export StdlibExportParam");
        LexToken::export_all(cfg).expect("failed to export LexToken");
        CompletionItem::export_all(cfg).expect("failed to export CompletionItem");
        SpanQueryResponse::export_all(cfg).expect("failed to export SpanQueryResponse");
        SpanQueryFailure::export_all(cfg).expect("failed to export SpanQueryFailure");
        VariantCompileSuccess::export_all(cfg).expect("failed to export VariantCompileSuccess");
        VisualizerCompileSuccess::export_all(cfg)
            .expect("failed to export VisualizerCompileSuccess");
        VisualizerMetadata::export_all(cfg).expect("failed to export VisualizerMetadata");
    }

    #[test]
    fn export_wasm_contract_types_includes_dependencies_in_clean_directory() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time precedes epoch")
            .as_nanos();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wasm-contract-tests")
            .join(format!("{}-{stamp}", std::process::id()));
        let bindings = root.join("bindings");
        std::fs::create_dir_all(&bindings).expect("failed to create isolated export directory");
        let cfg = ts_rs::Config::default().with_out_dir(bindings);
        export_contract_types(&cfg);
        let output = root.join("web/src/generated/wasm-contracts");
        assert!(output.join("ManifestRoot.ts").is_file());
        for entry in std::fs::read_dir(&output).expect("missing generated contracts") {
            let path = entry
                .expect("failed to read generated contract entry")
                .path();
            let source = std::fs::read_to_string(&path).expect("failed to read generated contract");
            for line in source
                .lines()
                .filter(|line| line.starts_with("import type "))
            {
                let import = line
                    .split('"')
                    .nth(1)
                    .expect("missing generated import path");
                let dependency = path
                    .parent()
                    .expect("missing parent directory")
                    .join(import)
                    .with_extension("ts");
                assert!(
                    dependency.is_file(),
                    "{} imports missing {}",
                    path.display(),
                    dependency.display()
                );
            }
        }
    }

    fn source_and_cursor(marked: &str) -> (String, usize) {
        let cursor = marked
            .find('|')
            .expect("cursor marker `|` is required in test source");
        let mut source = marked.to_string();
        source.remove(cursor);
        (source, cursor)
    }

    fn completion_labels(items: &[CompletionItem]) -> Vec<String> {
        items.iter().map(|it| it.label.clone()).collect()
    }

    fn completion_items_for_cursor_e2e(source: &str, cursor_utf8: usize) -> Vec<CompletionItem> {
        #[cfg(target_arch = "wasm32")]
        {
            let value = fresco_completion_items(source, cursor_utf8);
            return serde_wasm_bindgen::from_value(value)
                .expect("failed to deserialize wasm completion items");
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            // Native unit tests cannot call wasm-bindgen imported functions.
            completion_items_for_cursor(source, cursor_utf8)
        }
    }

    fn highlighted_text_and_kind(source: &str) -> Vec<(String, String)> {
        lex_tokens_for_highlighting(source)
            .into_iter()
            .map(|token| (source[token.start..token.end].to_string(), token.kind))
            .collect()
    }

    #[test]
    fn blend_context_suggests_blend_modes() {
        let (source, cursor) = source_and_cursor("compose {\n  item blend: ad|\n}\n");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "add"));
    }

    #[test]
    fn blend_call_arg_context_suggests_blend_modes() {
        let marked = "canvas t(uv: coord, time: signal) -> color {\n  let b = box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> blend(ad<cursor>)\n}\n";
        let cursor = marked.find("<cursor>").expect("cursor marker required");
        let source = marked.replace("<cursor>", "");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(
            labels.iter().any(|label| label == "add"),
            "expected blend mode completion in positional call arg context, got: {labels:?}"
        );
    }

    #[test]
    fn pipe_context_does_not_fall_back_to_keywords() {
        let marked = "circle(radius: 10px) |> <cursor>\n";
        let cursor = marked.find("<cursor>").expect("cursor marker required");
        let source = marked.replace("<cursor>", "");
        let line_prefix = current_line_prefix(&source, cursor);
        assert!(is_pipe_context(line_prefix));
        let items = completion_items_for_cursor(&source, cursor);
        assert!(items.iter().all(|item| item.kind != "keyword"));
    }

    #[test]
    fn pipe_context_filters_to_shape_layer_relevant_operations() {
        let marked = "canvas t(uv: coord, time: signal) -> color {\n  let b = box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> <cursor>\n}\n";
        let cursor = marked.find("<cursor>").expect("cursor marker required");
        let source = marked.replace("<cursor>", "");
        let line_prefix = current_line_prefix(&source, cursor);
        assert!(
            is_pipe_context_at_cursor(source_prefix(&source, cursor), line_prefix),
            "expected pipe context, got line prefix: {line_prefix:?}"
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);

        assert!(
            labels.iter().any(|label| label == "round"),
            "expected round in labels, got: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == "fill"),
            "expected fill in labels, got: {labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label == "abs"),
            "unexpected abs in labels: {labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label == "box"),
            "unexpected box in labels: {labels:?}"
        );
    }

    #[test]
    fn call_arg_context_suggests_named_parameters() {
        let (source, cursor) = source_and_cursor("shadow(of|)");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "offset:"));
    }

    #[test]
    fn centered_call_arg_context_suggests_aspect() {
        let (source, cursor) = source_and_cursor("centered(a|)");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "aspect:"));
    }

    #[test]
    fn wave_call_arg_context_suggests_shape_discriminator() {
        let (source, cursor) = source_and_cursor("wave(sh|)");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "shape:"));
    }

    #[test]
    fn centered_aspect_value_context_suggests_centered_mode_variants() {
        let (source, cursor) = source_and_cursor("centered(aspect: |)");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "preserve"));
        assert!(labels.iter().any(|label| label == "fit"));
        assert!(labels.iter().any(|label| label == "fill"));
    }

    #[test]
    fn centered_aspect_value_context_does_not_suggest_untyped_scope_symbols() {
        let (source, cursor) = source_and_cursor(
            "canvas t(uv: coord, time: signal) -> color {\n  let pulse = wave(period: 1.6s, shape: sine, range: 0.35 .. 0.85)\n  space stage = centered(aspect: |)\n}\n",
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "preserve"));
        assert!(labels.iter().any(|label| label == "fit"));
        assert!(labels.iter().any(|label| label == "fill"));
        assert!(!labels.iter().any(|label| label == "pulse"));
        assert!(!labels.iter().any(|label| label == "time"));
    }

    #[test]
    fn cell_geometry_members_complete_in_binding_scope() {
        let (source, cursor) = source_and_cursor(
            "in space cells(layout: hex, every: 0.2, seed: 7, sampling: center, cell: tile) { tile.| }",
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        for name in [
            "center",
            "local",
            "angle",
            "edge_distance",
            "inset_distance",
            "boundary_point",
        ] {
            assert!(labels.iter().any(|label| label == name), "{name}");
        }
        assert!(
            items
                .iter()
                .all(|item| item.detail.starts_with("cell member ("))
        );
        let (source, cursor) = source_and_cursor("in space cells(cell: tile) { tile.inset_| }");
        assert_eq!(
            completion_labels(&completion_items_for_cursor(&source, cursor)),
            ["inset_distance"]
        );
        assert!(
            cell_member_completion("in space cells(cell: tile) { fill(#fff) } tile.").is_none()
        );
        assert!(
            cell_member_completion("in space cells(cell: tile) { let tile = 1; tile.").is_none()
        );
    }

    #[test]
    fn chase_arguments_and_typed_contours_are_suggested() {
        for input in ["chase(|)", "chase( |)", "chase(\n |)"] {
            let (source, cursor) = source_and_cursor(input);
            let labels = completion_labels(&completion_items_for_cursor(&source, cursor));
            assert!(
                labels.iter().any(|name| name == "along:"),
                "{input}: {labels:?}"
            );
        }
        let (source, cursor) = source_and_cursor(
            "in space cells(cell: tile) { let outer = tile.contour(); contour inner = tile.contour(); let amount = 1; chase(along: |) }",
        );
        let labels = completion_labels(&completion_items_for_cursor(&source, cursor));
        assert_eq!(labels, ["inner", "outer"]);
        let (source, cursor) = source_and_cursor(
            "in space cells(cell: tile) { let outer = tile.contour(); { let outer = 1; chase(along: |) } }",
        );
        assert!(
            !completion_labels(&completion_items_for_cursor(&source, cursor))
                .contains(&"outer".to_string())
        );
    }

    #[test]
    fn contour_members_and_enum_controls_complete() {
        assert!(
            cell_member_completion(
                "in space cells(cell: tile) { let center = tile.center; center."
            )
            .is_none()
        );
        assert!(cell_member_completion("in space cells(cell: tile) { let track = tile.contour(); let d = track.distance; d.").is_none());

        let (source, cursor) = source_and_cursor(
            "in space cells(cell: tile) { let track = tile.contour(inset: 2px) track.| }",
        );
        let labels = completion_labels(&completion_items_for_cursor(&source, cursor));
        for name in ["distance", "progress", "length", "point"] {
            assert!(labels.iter().any(|v| v == name), "{labels:?}");
        }
        assert!(!labels.iter().any(|v| v == "abs"));
        assert!(
            cell_member_completion(
                "in space cells(cell: tile) { let track = tile.contour() } track."
            )
            .is_none()
        );
        for (input, expected) in [
            ("band(0.1, width: 2px, profile: |)", vec!["solid", "soft"]),
            (
                "chase(along: track, direction: |)",
                vec!["clockwise", "counterclockwise"],
            ),
        ] {
            let (source, cursor) = source_and_cursor(input);
            let labels = completion_labels(&completion_items_for_cursor(&source, cursor));
            for name in expected {
                assert!(labels.iter().any(|v| v == name), "{input}: {labels:?}");
            }
            assert!(!labels.iter().any(|v| v == "abs"));
        }
    }

    #[test]
    fn cells_sampling_value_completion_uses_declared_variants() {
        for input in [
            ".cells(sampling: |)",
            "in space cells(layout: hex,\n sampling: |, every: 0.16)",
        ] {
            let (source, cursor) = source_and_cursor(input);
            let items = completion_items_for_cursor(&source, cursor);
            assert_eq!(
                completion_labels(&items),
                ["center", "grid2x2", "grid3x3", "grid4x4"]
            );
            assert!(items.iter().all(|item| item.allowed && item.kind == "enum"));
        }
        let (source, cursor) = source_and_cursor(".cells(sampling: grid3|)");
        assert_eq!(
            completion_labels(&completion_items_for_cursor(&source, cursor)),
            ["grid3x3"]
        );
    }

    #[test]
    fn cells_layout_value_completion_uses_declared_variants() {
        for input in [
            ".cells(layout: |, every: 0.16, jitter: 1.0, seed: 37, sampling: grid4x4, cell: tile)",
            "in space centered(aspect: preserve)\n    .cells(layout: |",
            "in space cells(every: 0.16,\n    layout: |, seed: 37)",
        ] {
            let (source, cursor) = source_and_cursor(input);
            let items = completion_items_for_cursor(&source, cursor);
            let labels = completion_labels(&items);
            assert_eq!(
                labels,
                ["brick", "hex", "jittered", "square", "voronoi"],
                "{input}"
            );
            assert!(items.iter().all(|item| item.allowed && item.kind == "enum"));
        }
        let (source, cursor) = source_and_cursor(".cells(layout: vo|, every: 0.16)");
        assert_eq!(
            completion_labels(&completion_items_for_cursor(&source, cursor)),
            ["voronoi"]
        );
    }

    #[test]
    fn wave_shape_value_context_suggests_wave_shape_variants() {
        let (source, cursor) = source_and_cursor(
            "canvas t(uv: coord, time: signal) -> color {\n  let p = wave(period: 0.11s, shape: |, range: 0.58 .. 1.0)\n}\n",
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "sine"));
        assert!(labels.iter().any(|label| label == "saw"));
        assert!(labels.iter().any(|label| label == "triangle"));
        assert!(labels.iter().any(|label| label == "square"));
    }

    #[test]
    fn wave_shape_value_context_respects_partial_prefix() {
        let (source, cursor) = source_and_cursor(
            "canvas t(uv: coord, time: signal) -> color {\n  let p = wave(period: 0.11s, shape: tri|, range: 0.58 .. 1.0)\n}\n",
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "triangle"));
        assert!(!labels.iter().any(|label| label == "sine"));
        assert!(!labels.iter().any(|label| label == "saw"));
        assert!(!labels.iter().any(|label| label == "square"));
    }

    #[test]
    fn gradient_call_arg_context_suggests_kind_discriminator() {
        let (source, cursor) = source_and_cursor("gradient(ki|)");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "kind:"));
    }

    #[test]
    fn gradient_kind_value_context_suggests_variants() {
        let (source, cursor) = source_and_cursor(
            "canvas t(uv: coord, time: signal) -> color {\n  let g = gradient(kind: |)\n}\n",
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "linear"));
        assert!(labels.iter().any(|label| label == "radial"));
    }

    #[test]
    fn gradient_kind_value_context_respects_partial_prefix() {
        let (source, cursor) = source_and_cursor(
            "canvas t(uv: coord, time: signal) -> color {\n  let g = gradient(kind: ra|)\n}\n",
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "radial"));
        assert!(!labels.iter().any(|label| label == "linear"));
    }

    #[test]
    fn gradient_kind_value_context_does_not_suggest_untyped_scope_symbols() {
        let (source, cursor) = source_and_cursor(
            "canvas t(uv: coord, time: signal) -> color {\n  let radial_hint = 1\n  let g = gradient(kind: |)\n}\n",
        );
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "linear"));
        assert!(labels.iter().any(|label| label == "radial"));
        assert!(!labels.iter().any(|label| label == "radial_hint"));
        assert!(!labels.iter().any(|label| label == "time"));
    }

    #[test]
    fn base_call_arg_context_suggests_material_parameters() {
        let (source, cursor) = source_and_cursor(
            "surface t(sp: surf) -> material {\n  compose {\n    base(|)\n  }\n}\n",
        );
        let engine = HashMap::from([(
            "engine/engine.fr".into(),
            include_str!("../../../tests/render-policy/engine/engine.fr").into(),
        )]);
        let items = completion_items_with_files(&source, cursor, "main.fr", &engine);
        let labels = completion_labels(&items);
        assert!(
            labels.iter().any(|label| label == "albedo:"),
            "got: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == "roughness:"),
            "got: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == "normal:"),
            "got: {labels:?}"
        );
    }

    #[test]
    fn layer_material_call_arg_context_suggests_material_parameters() {
        let (source, cursor) = source_and_cursor(
            "surface t(sp: surf) -> material {\n  compose {\n    base(albedo: #fff)\n    layer material(|)\n  }\n}\n",
        );
        let engine = HashMap::from([(
            "engine/engine.fr".into(),
            include_str!("../../../tests/render-policy/engine/engine.fr").into(),
        )]);
        let items = completion_items_with_files(&source, cursor, "main.fr", &engine);
        let labels = completion_labels(&items);
        assert!(
            labels.iter().any(|label| label == "weight:"),
            "got: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == "albedo:"),
            "got: {labels:?}"
        );
        assert!(
            labels.iter().any(|label| label == "emissive:"),
            "got: {labels:?}"
        );
    }

    #[test]
    fn angle_call_arg_context_suggests_theta_parameter() {
        let (source, cursor) = source_and_cursor("angle(|)");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(
            labels.iter().any(|label| label == "theta:"),
            "got: {labels:?}"
        );
    }

    #[test]
    fn point_at_call_arg_context_suggests_path_and_s_parameters() {
        let (source, cursor) = source_and_cursor("point_at(|)");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(
            labels.iter().any(|label| label == "path:"),
            "got: {labels:?}"
        );
        assert!(labels.iter().any(|label| label == "s:"), "got: {labels:?}");
    }

    #[test]
    fn discriminator_completion_matrix_covers_all_discriminated_builtins_via_wasm_api() {
        let docs = example_language_docs();
        let discriminated = docs
            .builtin_reference
            .iter()
            .filter_map(|builtin| builtin.discriminator.as_ref().map(|disc| (builtin, disc)))
            .collect::<Vec<_>>();

        assert!(
            !discriminated.is_empty(),
            "expected at least one discriminated builtin in docs model"
        );

        for (builtin, discriminator) in discriminated {
            assert!(
                !discriminator.variants.is_empty(),
                "builtin `{}` discriminator `{}` should define variants",
                builtin.name,
                discriminator.name
            );

            let arg_prefix = discriminator
                .name
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_default();

            let (arg_source, arg_cursor) =
                source_and_cursor(&format!("{}({}|)", builtin.name, arg_prefix));
            let arg_items = completion_items_for_cursor_e2e(&arg_source, arg_cursor);
            let arg_labels = completion_labels(&arg_items);
            let discriminator_label = format!("{}:", discriminator.name);

            assert!(
                arg_labels.iter().any(|label| label == &discriminator_label),
                "missing discriminator arg completion `{}` for builtin `{}`",
                discriminator_label,
                builtin.name
            );

            let (value_source, value_cursor) = source_and_cursor(&format!(
                "canvas t(uv: coord, time: signal) -> color {{\n  let _x = {}({}: |)\n}}\n",
                builtin.name, discriminator.name
            ));
            let value_items = completion_items_for_cursor_e2e(&value_source, value_cursor);
            let value_labels = completion_labels(&value_items);

            for variant in &discriminator.variants {
                assert!(
                    value_labels.iter().any(|label| label == variant),
                    "missing discriminator value completion `{}` for builtin `{}({}: ...)`",
                    variant,
                    builtin.name,
                    discriminator.name
                );
            }
        }
    }

    #[test]
    fn type_context_suggests_type_keywords() {
        let (source, cursor) = source_and_cursor("param speed: f|");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "f32"));
    }

    #[test]
    fn function_suggestions_include_rich_metadata() {
        let (source, cursor) = source_and_cursor("sha|");
        let items = completion_items_for_cursor(&source, cursor);
        let shadow = items
            .iter()
            .find(|item| item.label == "shadow")
            .expect("shadow completion should be present");
        assert!(shadow.signature.is_some());
        assert!(shadow.documentation.is_some());
        assert!(shadow.snippet.is_some());
        assert!(shadow.info.is_some());
    }

    #[test]
    fn symbol_completion_uses_scope_before_cursor() {
        let (source, cursor) = source_and_cursor("let before = 1\nbe|\nlet after = before\n");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "before"));
        assert!(!labels.iter().any(|label| label == "after"));
    }

    #[test]
    fn symbol_completion_prefers_declarations_over_plain_identifiers() {
        let (source, cursor) = source_and_cursor("foo(bar)\nfo|\n");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(!labels.iter().any(|label| label == "foo"));
        assert!(!labels.iter().any(|label| label == "bar"));
    }

    #[test]
    fn parser_expectations_keep_top_level_symbol_visible() {
        let (source, cursor) = source_and_cursor("let local = 1\n|");
        let items = completion_items_for_cursor(&source, cursor);
        let local = items
            .iter()
            .find(|item| item.label == "local")
            .expect("local completion should be present");
        assert_eq!(local.label, "local");
    }

    #[test]
    fn symbol_completion_includes_import_bound_name() {
        let (source, cursor) = source_and_cursor("import \"effects/soft_blur.fr\"\nsoft_|\n");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "soft_blur"));
    }

    #[test]
    fn symbol_completion_includes_import_alias_name() {
        let (source, cursor) = source_and_cursor("import \"effects/soft_blur.fr\" as fx\nfx|\n");
        let items = completion_items_for_cursor(&source, cursor);
        let labels = completion_labels(&items);
        assert!(labels.iter().any(|label| label == "fx"));
    }

    #[test]
    fn wasm_lexer_classification_covers_editor_token_kinds() {
        let source = concat!(
            "canvas import enum fn for if else let param in space canvas_space compose style blend ",
            "scatter within seed strategy lifetime respawn every field layer return break #pragma @builtin\n",
            "-> => <= >= == != && || |> ++ -- | & + - * / .. = ? < > . , : ; ( ) { } [ ]\n",
            "123 1.5px #ff00aa \"hi\" ident // comment\n",
        );

        let actual = highlighted_text_and_kind(source);
        let expected = vec![
            ("canvas", "keyword"),
            ("import", "keyword"),
            ("enum", "keyword"),
            ("fn", "keyword"),
            ("for", "keyword"),
            ("if", "keyword"),
            ("else", "keyword"),
            ("let", "keyword"),
            ("param", "keyword"),
            ("in", "keyword"),
            ("space", "keyword"),
            ("canvas_space", "keyword"),
            ("compose", "keyword"),
            ("style", "keyword"),
            ("blend", "keyword"),
            ("scatter", "keyword"),
            ("within", "keyword"),
            ("seed", "keyword"),
            ("strategy", "keyword"),
            ("lifetime", "keyword"),
            ("respawn", "keyword"),
            ("every", "keyword"),
            ("field", "keyword"),
            ("layer", "keyword"),
            ("return", "keyword"),
            ("break", "keyword"),
            ("#pragma", "keyword"),
            ("@builtin", "keyword"),
            ("->", "operator"),
            ("=>", "operator"),
            ("<=", "operator"),
            (">=", "operator"),
            ("==", "operator"),
            ("!=", "operator"),
            ("&&", "operator"),
            ("||", "operator"),
            ("|>", "operator"),
            ("++", "operator"),
            ("--", "operator"),
            ("|", "operator"),
            ("&", "operator"),
            ("+", "operator"),
            ("-", "operator"),
            ("*", "operator"),
            ("/", "operator"),
            ("..", "operator"),
            ("=", "operator"),
            ("?", "operator"),
            ("<", "operator"),
            (">", "operator"),
            ("123", "number"),
            ("1.5px", "number"),
            ("#ff00aa", "number"),
            ("\"hi\"", "string"),
            ("ident", "identifier"),
            ("// comment", "comment"),
        ]
        .into_iter()
        .map(|(text, kind)| (text.to_string(), kind.to_string()))
        .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }
}
