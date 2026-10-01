#![deny(unused_must_use)]
#![deny(clippy::dbg_macro, clippy::todo)]

mod browser_manifest;
mod completion;

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
    let files = source_files_from_js(files)?.unwrap_or_default();
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
    let files = match source_files_from_js(files) {
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
    let files = match source_files_from_js(files) {
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
    let files = match source_files_from_js(files) {
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

fn source_files_from_js(files: JsValue) -> Result<Option<HashMap<String, String>>, JsValue> {
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
        completion::CompletionItem::export_all(cfg).expect("failed to export CompletionItem");
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
}
