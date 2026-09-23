use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Once;
use web_time::Instant;

#[cfg(feature = "trace-chrome")]
use std::sync::Mutex;

mod compute_host;
mod compute_operations;
mod emit;
mod engine_pass;
mod entry_contract;
mod explain;
mod gpu_function;
mod implementations;
mod mesh_pass;
mod operation_composition;
pub(crate) mod pass_plan;
mod pipeline;
mod prepared_geometry;
mod recipes;
mod render_state;
mod renderers;
mod resource_groups;
mod resource_ports;
mod schema_evaluation;
pub(crate) mod schema_function;
mod shader_iterators;
mod shader_services;
mod style_availability;
mod style_graph;
mod style_operations;
#[cfg(test)]
mod style_tests;
mod styles;
mod surface_properties;
mod tables;
mod techniques;
mod viz;

use crate::diag;
pub use emit::EmitTarget;
pub use pipeline::{BuildProfile, CompileContext, engine_keyword_spans, material_callables};

static TRACE_INIT: Once = Once::new();
#[cfg(feature = "trace-chrome")]
static TRACE_CHROME_GUARD: Mutex<Option<tracing_chrome::FlushGuard>> = Mutex::new(None);

fn finalize_trace_output() {
    #[cfg(feature = "trace-chrome")]
    {
        if let Ok(mut guard) = TRACE_CHROME_GUARD.lock() {
            let _ = guard.take();
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TraceVerbosity {
    #[default]
    Info,
    Debug,
    Trace,
}

impl TraceVerbosity {
    fn as_filter(self) -> &'static str {
        match self {
            TraceVerbosity::Info => "fresco=info",
            TraceVerbosity::Debug => "fresco=debug",
            TraceVerbosity::Trace => "fresco=trace",
        }
    }
}

fn init_trace_subscriber_once(verbose: bool, trace_verbosity: TraceVerbosity) {
    TRACE_INIT.call_once(|| {
        let mode = if verbose {
            "fmt".to_string()
        } else {
            std::env::var("FRESCO_TRACE")
                .map(|v| v.to_ascii_lowercase())
                .unwrap_or_else(|_| "off".to_string())
        };

        if matches!(mode.as_str(), "" | "0" | "off" | "false") {
            return;
        }

        let default_filter = if verbose {
            trace_verbosity.as_filter()
        } else {
            "fresco=trace"
        };

        // Explicit CLI verbosity takes precedence over an inherited RUST_LOG.
        let filter = if verbose {
            tracing_subscriber::EnvFilter::new(default_filter)
        } else {
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter))
        };

        match mode.as_str() {
            "1" | "on" | "fmt" => {
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_span_events(
                        tracing_subscriber::fmt::format::FmtSpan::NEW
                            | tracing_subscriber::fmt::format::FmtSpan::CLOSE,
                    )
                    .with_target(false)
                    .with_writer(std::io::stderr)
                    .try_init();
            }
            "json" => {
                let _ = tracing_subscriber::fmt()
                    .json()
                    .with_env_filter(filter)
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .with_writer(std::io::stderr)
                    .try_init();
            }
            #[cfg(feature = "trace-chrome")]
            "chrome" => {
                use tracing_subscriber::layer::SubscriberExt;
                let (chrome_layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
                    .file("fresco-trace.json")
                    .include_args(true)
                    .build();
                if let Ok(mut slot) = TRACE_CHROME_GUARD.lock() {
                    *slot = Some(guard);
                }
                let subscriber = tracing_subscriber::registry()
                    .with(filter)
                    .with(chrome_layer);
                let _ = tracing::subscriber::set_global_default(subscriber);
                eprintln!("trace enabled: chrome output -> fresco-trace.json");
            }
            #[cfg(not(feature = "trace-chrome"))]
            "chrome" => {
                eprintln!(
                    "FRESCO_TRACE=chrome requires --features trace-chrome; falling back to fmt"
                );
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .with_target(false)
                    .with_writer(std::io::stderr)
                    .try_init();
            }
            #[cfg(feature = "trace-tracy")]
            "tracy" => {
                use tracing_subscriber::layer::SubscriberExt;
                let subscriber = tracing_subscriber::registry()
                    .with(filter)
                    .with(tracing_tracy::TracyLayer::default());
                let _ = tracing::subscriber::set_global_default(subscriber);
                eprintln!("trace enabled: tracy stream");
            }
            #[cfg(not(feature = "trace-tracy"))]
            "tracy" => {
                eprintln!(
                    "FRESCO_TRACE=tracy requires --features trace-tracy; falling back to fmt"
                );
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .with_target(false)
                    .with_writer(std::io::stderr)
                    .try_init();
            }
            other => {
                eprintln!("unknown FRESCO_TRACE mode `{other}`. valid: off|fmt|json|chrome|tracy");
                let _ = tracing_subscriber::fmt()
                    .with_env_filter(filter)
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE)
                    .with_target(false)
                    .with_writer(std::io::stderr)
                    .try_init();
            }
        }
    });
}

pub struct Options {
    pub input: PathBuf,
    pub engine_dir: Option<PathBuf>,
    pub emit: EmitTarget,
    pub explain: bool,
    pub output: Option<PathBuf>,
    pub timings: bool,
    pub verbose: bool,
    pub trace_verbosity: TraceVerbosity,
    pub context: CompileContext,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ContextFile {
    pub check: ContextFileCheck,
    pub build_profile: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ContextFileCheck {
    pub wide_effect_warn_ratio: Option<f32>,
    pub wide_effect_note_ratio: Option<f32>,
    pub shape_aa_min_px: Option<f32>,
    pub shape_aa_max_px: Option<f32>,
    pub shape_aa_style: Option<String>,
    pub shape_aa_level: Option<String>,
    pub expr_timeout_ms: Option<u64>,
    pub allow_implicit_texture_uv: Option<bool>,
    pub warn_implicit_texture_uv: Option<bool>,
}

impl ContextFile {
    pub fn from_json_str(src: &str) -> Result<Self, String> {
        serde_json::from_str(src).map_err(|e| format!("invalid JSON compiler context: {e}"))
    }

    pub fn from_json_path(path: &std::path::Path) -> Result<Self, String> {
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read context file {}: {e}", path.display()))?;
        Self::from_json_str(&src)
    }

    pub fn apply_to(&self, context: &mut CompileContext) {
        fn parse_build_profile(value: &str) -> Option<BuildProfile> {
            match value.to_ascii_lowercase().as_str() {
                "all" => Some(BuildProfile::All),
                "editor" => Some(BuildProfile::Editor),
                "runtime" => Some(BuildProfile::Runtime),
                _ => None,
            }
        }

        fn parse_aa_style(value: &str) -> Option<crate::hir::ShapeAaStyle> {
            match value.to_ascii_lowercase().as_str() {
                "gradient" => Some(crate::hir::ShapeAaStyle::Gradient),
                "fwidth" => Some(crate::hir::ShapeAaStyle::Fwidth),
                "conservative" | "best" => Some(crate::hir::ShapeAaStyle::Conservative),
                _ => None,
            }
        }

        fn parse_aa_level(value: &str) -> Option<(f32, f32)> {
            match value.to_ascii_lowercase().as_str() {
                "sharp" | "low" => Some((1.0, 2.0)),
                "balanced" | "medium" => Some((1.5, 3.0)),
                "soft" | "high" => Some((2.0, 4.0)),
                "ultra" | "max" => Some((2.5, 5.5)),
                _ => None,
            }
        }

        if let Some(v) = self.check.wide_effect_warn_ratio {
            context.check.wide_effect_warn_ratio = v;
        }
        if let Some(v) = self.check.wide_effect_note_ratio {
            context.check.wide_effect_note_ratio = v;
        }
        if let Some(v) = self.check.shape_aa_min_px {
            context.check.shape_aa_min_px = Some(v);
        }
        if let Some(v) = self.check.shape_aa_max_px {
            context.check.shape_aa_max_px = Some(v);
        }
        if let Some(v) = &self.check.shape_aa_style
            && let Some(style) = parse_aa_style(v)
        {
            context.check.shape_aa_style = Some(style);
        }
        if let Some(v) = &self.check.shape_aa_level
            && let Some((min_px, max_px)) = parse_aa_level(v)
        {
            context.check.shape_aa_min_px = Some(min_px);
            context.check.shape_aa_max_px = Some(max_px);
        }
        if let Some(v) = self.check.expr_timeout_ms {
            context.check.expr_timeout_ms = Some(v);
        }
        if let Some(v) = self.check.allow_implicit_texture_uv {
            context.check.allow_implicit_texture_uv = v;
        }
        if let Some(v) = self.check.warn_implicit_texture_uv {
            context.check.warn_implicit_texture_uv = v;
        }
        if let Some(v) = &self.build_profile
            && let Some(profile) = parse_build_profile(v)
        {
            context.build_profile = profile;
        }
    }
}

impl CompileContext {
    pub fn validate(&self) -> Result<(), String> {
        let warn = self.check.wide_effect_warn_ratio;
        let note = self.check.wide_effect_note_ratio;
        if !warn.is_finite() || warn <= 0.0 {
            return Err(format!(
                "check.wide_effect_warn_ratio must be a finite value > 0, got {warn}"
            ));
        }
        if !note.is_finite() || note < 0.0 {
            return Err(format!(
                "check.wide_effect_note_ratio must be a finite value >= 0, got {note}"
            ));
        }
        if note > warn {
            return Err(format!(
                "check.wide_effect_note_ratio ({note}) must be <= check.wide_effect_warn_ratio ({warn})"
            ));
        }

        for (name, value) in [
            ("check.shape_aa_min_px", self.check.shape_aa_min_px),
            ("check.shape_aa_max_px", self.check.shape_aa_max_px),
            (
                "check.projective_footprint_max_px",
                self.check.projective_footprint_max_px,
            ),
        ] {
            if let Some(value) = value
                && (!value.is_finite() || value <= 0.0)
            {
                return Err(format!("{name} must be a finite value > 0, got {value}"));
            }
        }
        if let (Some(aa_min), Some(aa_max)) =
            (self.check.shape_aa_min_px, self.check.shape_aa_max_px)
            && aa_min > aa_max
        {
            return Err(format!(
                "check.shape_aa_min_px ({aa_min}) must be <= check.shape_aa_max_px ({aa_max})"
            ));
        }

        if self.build_profile.as_str().is_empty() {
            return Err("build_profile must not be empty".to_string());
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DiagnosticRecord {
    pub file: String,
    pub severity: String,
    pub message: String,
    pub label: Option<String>,
    pub help: Option<String>,
    pub span_start: usize,
    pub span_end: usize,
}

#[derive(Debug, Clone)]
pub struct CompileOutput {
    pub emitted: String,
    pub explain: Option<String>,
    pub diagnostics: Vec<DiagnosticRecord>,
}

#[derive(Debug, Clone)]
pub struct CompileBundleOutput {
    pub wgsl: String,
    /// Manifest serialized as JSON text.
    ///
    /// This remains a string so CLI `--emit manifest` behavior stays stable and
    /// shell-friendly even while wasm/web layers may deserialize into typed objects.
    pub manifest: String,
    pub explain: Option<String>,
    pub diagnostics: Vec<DiagnosticRecord>,
    pub timings: CompileBundleTimings,
}

#[derive(Debug, Clone, Default)]
pub struct CompileBundleTimings {
    pub pipeline: pipeline::PipelineTimings,
    pub emit_wgsl_ms: f64,
    pub emit_manifest_ms: f64,
    pub explain_ms: f64,
    pub total_ms: f64,
}

pub fn compile_source(
    src: &str,
    filename: &str,
    emit_target: &str,
    include_explain: bool,
) -> Result<CompileOutput, Vec<DiagnosticRecord>> {
    compile_source_with_context(
        src,
        filename,
        emit_target,
        include_explain,
        &CompileContext::default(),
    )
}

pub fn compile_source_with_context(
    src: &str,
    filename: &str,
    emit_target: &str,
    include_explain: bool,
    context: &CompileContext,
) -> Result<CompileOutput, Vec<DiagnosticRecord>> {
    let emit_target =
        EmitTarget::from_str(emit_target).map_err(|err| vec![emit_failure(filename, err)])?;
    let compiled =
        pipeline::compile_with_options(src, filename, true, context).map_err(flatten_file_diags)?;
    render_compiled_source(compiled, filename, emit_target, include_explain, context)
}

/// Compile an explicit virtual source bundle to one output format.
/// No engine modules are discovered from the host filesystem.
pub fn compile_virtual_with_context(
    files: &HashMap<String, String>,
    entrypoint: &str,
    emit_target: &str,
    include_explain: bool,
    context: &CompileContext,
) -> Result<CompileOutput, Vec<DiagnosticRecord>> {
    let emit_target =
        EmitTarget::from_str(emit_target).map_err(|err| vec![emit_failure(entrypoint, err)])?;
    let compiled = pipeline::compile_with_virtual_files(files, entrypoint, true, context)
        .map_err(flatten_file_diags)?;
    render_compiled_source(compiled, entrypoint, emit_target, include_explain, context)
}

fn render_compiled_source(
    compiled: pipeline::CompiledProgram,
    filename: &str,
    emit_target: EmitTarget,
    include_explain: bool,
    context: &CompileContext,
) -> Result<CompileOutput, Vec<DiagnosticRecord>> {
    let emitted = emit::render(
        emit_target,
        context.build_profile,
        &compiled.hirs,
        &compiled.material_hirs,
        &compiled.compute_library,
        &compiled.structs,
        &compiled.passes,
        &compiled.pipelines,
        &compiled.vertex_interfaces,
        &compiled.vertex_formats,
        &compiled.vertex_factories,
        &compiled.module,
        &compiled.info,
        &compiled.tex_bindings,
        &compiled.pass_target_bindings,
        &compiled.path_bindings,
        &compiled.param_bindings,
        &compiled.global_uniform_bindings,
        compiled.resource_layout,
    )
    .map_err(|err| vec![emit_failure(filename, err)])?;
    let explain = include_explain.then(|| {
        explain::render(
            &compiled.hirs,
            &compiled.stats,
            Some(explain::EmitForStats {
                text: &emitted,
                format_label: emit_target.as_str(),
            }),
        )
    });

    Ok(CompileOutput {
        emitted,
        explain,
        diagnostics: flatten_file_diags(compiled.diagnostics),
    })
}

pub fn compile_source_bundle(
    src: &str,
    filename: &str,
    include_explain: bool,
) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
    compile_source_bundle_with_context(src, filename, include_explain, &CompileContext::default())
}

pub fn compile_source_bundle_with_context(
    src: &str,
    filename: &str,
    include_explain: bool,
    context: &CompileContext,
) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
    compile_source_bundle_with_engine_dir(src, filename, include_explain, context, None)
}

/// Compile using an explicit engine directory containing `engine.fr`.
/// `None` retains ancestor-based engine discovery. A selected engine is never
/// supplemented with modules from a discovered engine.
pub fn compile_source_bundle_with_engine_dir(
    src: &str,
    filename: &str,
    include_explain: bool,
    context: &CompileContext,
    engine_dir: Option<&std::path::Path>,
) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
    let total_start = Instant::now();
    let compiled = pipeline::compile_with_engine_dir(src, filename, true, context, engine_dir)
        .map_err(flatten_file_diags)?;
    render_compiled_bundle(compiled, filename, include_explain, context, total_start)
}

/// Compile a filesystem source with an authoritative embedded engine bundle.
/// User imports resolve relative to `filename`; engine imports resolve only in
/// `engine_files`. Missing engine imports never fall back to the filesystem.
pub fn compile_source_bundle_with_engine_files(
    src: &str,
    filename: &str,
    include_explain: bool,
    context: &CompileContext,
    engine_files: &HashMap<String, String>,
    engine_entrypoint: &str,
) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
    let total_start = Instant::now();
    let compiled = pipeline::compile_with_engine_files(
        src,
        filename,
        context,
        engine_files,
        engine_entrypoint,
    )
    .map_err(flatten_file_diags)?;
    render_compiled_bundle(compiled, filename, include_explain, context, total_start)
}

fn render_compiled_bundle(
    compiled: pipeline::CompiledProgram,
    filename: &str,
    include_explain: bool,
    context: &CompileContext,
    total_start: Instant,
) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
    let emit_wgsl_start = Instant::now();
    let wgsl = emit::render(
        EmitTarget::Wgsl,
        context.build_profile,
        &compiled.hirs,
        &compiled.material_hirs,
        &compiled.compute_library,
        &compiled.structs,
        &compiled.passes,
        &compiled.pipelines,
        &compiled.vertex_interfaces,
        &compiled.vertex_formats,
        &compiled.vertex_factories,
        &compiled.module,
        &compiled.info,
        &compiled.tex_bindings,
        &compiled.pass_target_bindings,
        &compiled.path_bindings,
        &compiled.param_bindings,
        &compiled.global_uniform_bindings,
        compiled.resource_layout,
    )
    .map_err(|err| vec![emit_failure(filename, err)])?;
    let emit_wgsl_ms = elapsed_ms(emit_wgsl_start);

    let emit_manifest_start = Instant::now();
    let manifest = emit::render(
        EmitTarget::Manifest,
        context.build_profile,
        &compiled.hirs,
        &compiled.material_hirs,
        &compiled.compute_library,
        &compiled.structs,
        &compiled.passes,
        &compiled.pipelines,
        &compiled.vertex_interfaces,
        &compiled.vertex_formats,
        &compiled.vertex_factories,
        &compiled.module,
        &compiled.info,
        &compiled.tex_bindings,
        &compiled.pass_target_bindings,
        &compiled.path_bindings,
        &compiled.param_bindings,
        &compiled.global_uniform_bindings,
        compiled.resource_layout,
    )
    .map_err(|err| vec![emit_failure(filename, err)])?;
    let emit_manifest_ms = elapsed_ms(emit_manifest_start);

    let explain_start = Instant::now();
    let explain = include_explain.then(|| {
        explain::render(
            &compiled.hirs,
            &compiled.stats,
            Some(explain::EmitForStats {
                text: &wgsl,
                format_label: "wgsl",
            }),
        )
    });
    let explain_ms = if include_explain {
        elapsed_ms(explain_start)
    } else {
        0.0
    };
    let pipeline_timings = compiled.timings.clone();

    Ok(CompileBundleOutput {
        wgsl,
        manifest,
        explain,
        diagnostics: flatten_file_diags(compiled.diagnostics),
        timings: CompileBundleTimings {
            pipeline: pipeline_timings,
            emit_wgsl_ms,
            emit_manifest_ms,
            explain_ms,
            total_ms: elapsed_ms(total_start),
        },
    })
}

/// Compile a multi-file Fresco program supplied as a virtual file system.
/// `files` maps virtual path → source text; `entrypoint` must be a key in
/// `files`. Any root entries declared there are compiled; callers may choose
/// which one to preview or inspect.
pub fn compile_bundle_virtual(
    files: &HashMap<String, String>,
    entrypoint: &str,
    include_explain: bool,
) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
    compile_bundle_virtual_with_context(
        files,
        entrypoint,
        include_explain,
        &CompileContext::default(),
    )
}

pub fn compile_bundle_virtual_with_context(
    files: &HashMap<String, String>,
    entrypoint: &str,
    include_explain: bool,
    context: &CompileContext,
) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
    let total_start = Instant::now();
    let compiled = pipeline::compile_with_virtual_files(files, entrypoint, true, context)
        .map_err(flatten_file_diags)?;

    render_compiled_bundle(compiled, entrypoint, include_explain, context, total_start)
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// Result of a span type query (§21 rung 2).
#[derive(Debug, Clone)]
pub struct SpanQueryResult {
    /// The semantic type of the expression at the queried span.
    pub kind: crate::check::SpanValueKind,
    /// The tightest-fit span that contained the query range.
    pub span_start: usize,
    pub span_end: usize,
}

/// Result of a variant compilation (§21 rung 1).
#[derive(Debug, Clone)]
pub struct VariantCompileOutput {
    pub wgsl: String,
    pub entry_context: Option<crate::context::EntryContext>,
    /// Manifest serialized as JSON text for CLI/driver compatibility.
    pub manifest: String,
    /// Semantic type of the captured expression (drives visualization).
    pub semantic_type: String,
    pub diagnostics: Vec<DiagnosticRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerKind {
    Sparkline,
    Swatch,
    Thumbnail,
}

#[derive(Debug, Clone)]
pub struct VisualizerCompileOutput {
    pub wgsl: String,
    pub draw_wgsl: String,
    pub reduce_wgsl: String,
    pub semantic_type: String,
    pub metadata: VisualizerMetadata,
    pub diagnostics: Vec<DiagnosticRecord>,
}

#[derive(Debug, Clone)]
pub struct VisualizerMetadata {
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

/// Query the semantic type of the expression closest to `span_start..span_end`
/// in the given source.  Returns `None` when no expression spans the range.
pub fn query_span_info(
    src: &str,
    filename: &str,
    span_start: usize,
    span_end: usize,
) -> Option<SpanQueryResult> {
    query_span_info_with_files(src, filename, span_start, span_end, None)
}

pub fn query_span_info_with_files(
    src: &str,
    filename: &str,
    span_start: usize,
    span_end: usize,
    files: Option<&HashMap<String, String>>,
) -> Option<SpanQueryResult> {
    let context = CompileContext::default();
    let (wb, _) = pipeline::compile_with_workbook(
        src,
        filename,
        true,
        &context,
        Some((span_start, span_end)),
        files,
    )
    .ok()?;

    let cap = wb.captured?;
    let kind = match &cap {
        crate::check::SpanCapture::Layer(_) => crate::check::SpanValueKind::Layer,
        crate::check::SpanCapture::Shape(_) => crate::check::SpanValueKind::Shape,
        crate::check::SpanCapture::Color(_) => crate::check::SpanValueKind::Color,
        crate::check::SpanCapture::ColorField(_) => crate::check::SpanValueKind::ColorField,
        crate::check::SpanCapture::Scalar(_) => crate::check::SpanValueKind::Scalar,
        crate::check::SpanCapture::Vec2(_) => crate::check::SpanValueKind::Vec2,
        crate::check::SpanCapture::Vec3(_) => crate::check::SpanValueKind::Vec3,
        crate::check::SpanCapture::Vec4(_) => crate::check::SpanValueKind::Vec4,
        crate::check::SpanCapture::Space(_) => crate::check::SpanValueKind::Space,
    };
    // Find the tightest-fitting span from the log that contains the query range
    // and matches the captured kind.
    let matched_span = wb
        .span_log
        .iter()
        .filter(|(sp, k)| *k == kind && sp.start <= span_start && span_end <= sp.end)
        .min_by_key(|(sp, _)| sp.end - sp.start);
    let (result_start, result_end) = matched_span
        .map(|(sp, _)| (sp.start, sp.end))
        .unwrap_or((span_start, span_end));

    Some(SpanQueryResult {
        kind,
        span_start: result_start,
        span_end: result_end,
    })
}

/// Compile a variant shader that renders only the expression at
/// `span_start..span_end` in the source canvas.
pub fn compile_variant(
    src: &str,
    filename: &str,
    span_start: usize,
    span_end: usize,
) -> Result<VariantCompileOutput, Vec<DiagnosticRecord>> {
    compile_variant_with_files(src, filename, span_start, span_end, None)
}

pub fn compile_variant_with_files(
    src: &str,
    filename: &str,
    span_start: usize,
    span_end: usize,
    files: Option<&HashMap<String, String>>,
) -> Result<VariantCompileOutput, Vec<DiagnosticRecord>> {
    let context = CompileContext::default();
    // Resolve to the tightest matching capture span so semantic metadata and
    // variant synthesis target the same expression.
    let span_info = query_span_info_with_files(src, filename, span_start, span_end, files);
    let (kind, resolved_span) = span_info
        .map(|r| (r.kind, (r.span_start, r.span_end)))
        .unwrap_or((crate::check::SpanValueKind::Layer, (span_start, span_end)));

    let compiled =
        pipeline::compile_variant_at_span(src, filename, true, &context, resolved_span, files)
            .map_err(flatten_file_diags)?;

    let wgsl = emit::render(
        EmitTarget::Wgsl,
        context.build_profile,
        &compiled.hirs,
        &compiled.material_hirs,
        &compiled.compute_library,
        &compiled.structs,
        &compiled.passes,
        &compiled.pipelines,
        &compiled.vertex_interfaces,
        &compiled.vertex_formats,
        &compiled.vertex_factories,
        &compiled.module,
        &compiled.info,
        &compiled.tex_bindings,
        &compiled.pass_target_bindings,
        &compiled.path_bindings,
        &compiled.param_bindings,
        &compiled.global_uniform_bindings,
        compiled.resource_layout,
    )
    .map_err(|err| vec![emit_failure(filename, err)])?;

    let manifest = emit::render(
        EmitTarget::Manifest,
        context.build_profile,
        &compiled.hirs,
        &compiled.material_hirs,
        &compiled.compute_library,
        &compiled.structs,
        &compiled.passes,
        &compiled.pipelines,
        &compiled.vertex_interfaces,
        &compiled.vertex_formats,
        &compiled.vertex_factories,
        &compiled.module,
        &compiled.info,
        &compiled.tex_bindings,
        &compiled.pass_target_bindings,
        &compiled.path_bindings,
        &compiled.param_bindings,
        &compiled.global_uniform_bindings,
        compiled.resource_layout,
    )
    .map_err(|err| vec![emit_failure(filename, err)])?;

    Ok(VariantCompileOutput {
        wgsl,
        entry_context: compiled
            .hirs
            .first()
            .and_then(|hir| hir.entry_context.clone()),
        manifest,
        semantic_type: kind.as_str().to_string(),
        diagnostics: flatten_file_diags(compiled.diagnostics),
    })
}

pub fn compile_visualizer(
    src: &str,
    filename: &str,
    span_start: usize,
    span_end: usize,
    kind: VisualizerKind,
    domain: &str,
    sweep_max: f32,
) -> Result<VisualizerCompileOutput, Vec<DiagnosticRecord>> {
    compile_visualizer_with_files(
        src,
        filename,
        (span_start, span_end),
        kind,
        domain,
        sweep_max,
        None,
    )
}

pub fn compile_visualizer_with_files(
    src: &str,
    filename: &str,
    span: (usize, usize),
    kind: VisualizerKind,
    domain: &str,
    sweep_max: f32,
    files: Option<&HashMap<String, String>>,
) -> Result<VisualizerCompileOutput, Vec<DiagnosticRecord>> {
    let (span_start, span_end) = span;
    let resolved_span = query_span_info_with_files(src, filename, span_start, span_end, files)
        .map(|r| (r.span_start, r.span_end))
        .unwrap_or((span_start, span_end));
    let expr_text = src.get(resolved_span.0..resolved_span.1);
    let range_hint = expr_text.and_then(viz::extract_range_hint_for_visualizer);
    let variant = compile_variant_with_files(src, filename, span_start, span_end, files)?;
    let visualizer_spec = viz::VisualizerSpec {
        kind: match kind {
            VisualizerKind::Sparkline => viz::VisualizerKind::Sparkline,
            VisualizerKind::Swatch => viz::VisualizerKind::Swatch,
            VisualizerKind::Thumbnail => viz::VisualizerKind::Thumbnail,
        },
        domain,
        sweep_max,
        semantic_type: &variant.semantic_type,
        range_hint,
    };
    let shaders = viz::build_shaders(
        &variant.wgsl,
        &variant.manifest,
        &visualizer_spec,
        variant.entry_context.as_ref(),
    )
    .map_err(|message| vec![visualizer_failure(filename, message)])?;
    let metadata = viz::build_metadata(&visualizer_spec, expr_text);

    Ok(VisualizerCompileOutput {
        wgsl: shaders.draw_wgsl.clone(),
        draw_wgsl: shaders.draw_wgsl,
        reduce_wgsl: shaders.reduce_wgsl,
        semantic_type: variant.semantic_type,
        metadata: VisualizerMetadata {
            kind: metadata.kind,
            domain: metadata.domain,
            sweep_max: metadata.sweep_max,
            x_axis_label: metadata.x_axis_label,
            y_axis_label: metadata.y_axis_label,
            fit_mode: metadata.fit_mode,
            y_min_hint: metadata.y_min_hint,
            y_max_hint: metadata.y_max_hint,
            preview_label: metadata.preview_label,
            preview_detail: metadata.preview_detail,
        },
        diagnostics: variant.diagnostics,
    })
}

#[cfg(test)]
const TEST_SOURCE_PATH: &str = "test.fr";

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_source_bundle(
        src: &str,
        filename: &str,
        explain: bool,
    ) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
        super::compile_bundle_virtual(
            &crate::test_support::files(src, filename),
            filename,
            explain,
        )
    }

    #[test]
    fn virtual_visualizer_resolves_engine_policy_and_rejects_missing_declarations() {
        let src = "canvas t(uv: coord) -> color { compose { circle(at: center, radius: 0.2) |> fill(#fff) } }";
        let expression = "circle(at: center, radius: 0.2)";
        let start = src.find(expression).expect("shape expression");
        let span = (start, start + expression.len());
        let mut files = HashMap::from([
            ("main.fr".into(), src.into()),
            ("engine/engine.fr".into(), "import \"policy.fr\"".into()),
            (
                "engine/policy.fr".into(),
                include_str!("../tests/fixtures/engines/policy/engine/engine.fr").into(),
            ),
        ]);
        let result = compile_visualizer_with_files(
            src,
            "main.fr",
            span,
            VisualizerKind::Thumbnail,
            "thumb",
            0.0,
            Some(&files),
        )
        .expect("virtual workbook must use its authored engine policy");
        assert_eq!(result.semantic_type, "shape");
        assert!(result.draw_wgsl.contains("aa_directional_px"));
        files.insert("engine/policy.fr".into(), String::new());
        let errors = compile_visualizer_with_files(
            src,
            "main.fr",
            span,
            VisualizerKind::Thumbnail,
            "thumb",
            0.0,
            Some(&files),
        )
        .expect_err("virtual workbook cannot inherit filesystem policy or hidden defaults");
        assert_eq!(
            errors
                .iter()
                .filter(|diag| diag.message.contains("missing required engine setting"))
                .count(),
            4
        );
    }

    // These tests exercise compiler behavior using an explicitly authored test engine.
    fn compile_bundle_virtual(
        files: &HashMap<String, String>,
        entrypoint: &str,
        explain: bool,
    ) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
        let mut files = files.clone();
        files.insert(
            "engine/render_policy.fr".into(),
            include_str!("../tests/fixtures/engines/policy/engine/engine.fr").into(),
        );
        if let Some(engine) = files.get_mut("engine/engine.fr")
            && !engine.contains("#pragma check.shape_aa_min_px")
        {
            engine.push_str("\nimport \"render_policy.fr\"\n");
        }
        super::compile_bundle_virtual(&files, entrypoint, explain)
    }

    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::Path;

    fn contract_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/contracts/manifest-pass-plan.contract.v1.json")
    }

    fn schema_resolve<'a>(schema: &'a Value, ref_path: &str) -> &'a Value {
        let mut cursor = schema;
        for segment in ref_path.trim_start_matches("#/").split('/') {
            cursor = cursor
                .get(segment)
                .unwrap_or_else(|| panic!("missing schema path segment `{segment}`"));
        }
        cursor
    }

    fn validate_required(value: &Value, schema_node: &Value, root_schema: &Value) {
        let resolved = if let Some(ref_path) = schema_node.get("$ref").and_then(Value::as_str) {
            schema_resolve(root_schema, ref_path)
        } else {
            schema_node
        };

        if value.is_null()
            && resolved.get("type").is_some_and(|ty| {
                ty.as_str() == Some("null")
                    || ty
                        .as_array()
                        .is_some_and(|types| types.iter().any(|ty| ty.as_str() == Some("null")))
            })
        {
            return;
        }

        if let Some(required) = resolved.get("required").and_then(Value::as_array) {
            let obj = value
                .as_object()
                .expect("schema node requires an object value");
            for key in required.iter().filter_map(Value::as_str) {
                assert!(obj.contains_key(key), "missing required key `{key}`");
            }
        }

        if let Some(props) = resolved.get("properties").and_then(Value::as_object)
            && let Some(obj) = value.as_object()
        {
            for (key, prop_schema) in props {
                if let Some(child) = obj.get(key) {
                    validate_required(child, prop_schema, root_schema);
                }
            }
        }

        if let Some(item_schema) = resolved.get("items")
            && let Some(items) = value.as_array()
        {
            for item in items {
                validate_required(item, item_schema, root_schema);
            }
        }
    }

    #[test]
    fn emitted_manifest_obeys_pass_plan_contract_required_fields() {
        let src = r#"
canvas c(uv: coord, time: signal) -> color {
  compose {
    fill(#ffffff)
  }
}
"#;
        let bundle =
            compile_source_bundle(src, TEST_SOURCE_PATH, false).expect("bundle should compile");
        let manifest: Value =
            serde_json::from_str(&bundle.manifest).expect("manifest should parse as JSON");
        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(contract_path()).expect("contract schema should load"),
        )
        .expect("contract schema should parse");
        validate_required(&manifest, &schema, &schema);
    }

    #[test]
    fn plain_library_source_without_entry_points_compiles() {
        let src = r#"
fn make_color() -> color {
  color(1.0, 0.0, 0.6)
}
"#;
        let bundle =
            compile_source_bundle(src, TEST_SOURCE_PATH, false).expect("bundle should compile");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn workbook_preview_reports_missing_root_entry_point_generically() {
        let src = r#"
fn make_color() -> color {
  color(1.0, 0.0, 0.6)
}
"#;

        let context = CompileContext::default();
        let Err(err) = pipeline::compile_with_workbook(
            src,
            TEST_SOURCE_PATH,
            true,
            &context,
            None,
            Some(&crate::test_support::files(src, TEST_SOURCE_PATH)),
        ) else {
            panic!("library source without entries should not support workbook preview")
        };

        assert!(err.iter().any(|file_diag| {
            file_diag.diags.iter().any(|diag| {
                diag.message
                    .contains("workbook query requires at least one root entry point")
            })
        }));
    }

    #[test]
    fn surface_root_source_compiles_without_canvas_entry_point() {
        let src = r#"
surface demo(sp: surf) -> material {
  compose {
    base(albedo: #fff)
  }
}
"#;
        let bundle = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("surface-only root source should compile");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn workbook_preview_reports_surface_entries_as_unsupported_not_missing() {
        let src = r#"
surface demo(sp: surf) -> material {
  compose {
    base(albedo: #fff)
  }
}
"#;

        let context = CompileContext::default();
        let Err(err) = pipeline::compile_with_workbook(
            src,
            TEST_SOURCE_PATH,
            true,
            &context,
            None,
            Some(&crate::test_support::files(src, TEST_SOURCE_PATH)),
        ) else {
            panic!("surface workbook preview should not be supported yet")
        };

        assert!(err.iter().any(|file_diag| {
            file_diag.diags.iter().any(|diag| {
                diag.message
                    .contains("workbook preview requires a previewable root entry")
            })
        }));
        assert!(err.iter().any(|file_diag| {
            file_diag
                .diags
                .iter()
                .any(|diag| diag.message.contains("surface `demo`"))
        }));
        assert!(err.iter().all(|file_diag| {
            file_diag
                .diags
                .iter()
                .all(|diag| !diag.message.contains("requires at least one canvas"))
        }));
    }

    #[test]
    fn mixed_vector_constructors_compile() {
        let src = r#"
fn project(m: mat4 from world to clip, p: vec3 in world) -> vec3 {
    let c = m * vec4(p, 1.0)
    return vec3(c.xy / c.w * 0.5 + 0.5, c.z / c.w)
}

fn constructor_mix() -> vec4 {
    let p = vec3(1.0, 2.0, 3.0)
    let q = vec2(4.0, 5.0)
    return vec4(p, 6.0)
             + vec4(7.0, p)
             + vec4(q, q)
             + vec4(q, 8.0, 9.0)
             + vec4(10.0, q, 11.0)
             + vec4(12.0, 13.0, q)
}

canvas c(uv: coord) -> color {
    let v = constructor_mix()
    compose {
        fill(rgba(v.x * 0.01, v.y * 0.01, v.z * 0.01, 1.0))
    }
}
"#;

        let bundle = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("mixed vec4 constructor forms should compile");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn emitted_manifest_pass_entry_points_exist_in_emitted_wgsl() {
        let src = r#"
canvas badge(uv: coord, time: signal) -> color {
  compose {
    fill(#ffffff)
  }
}

canvas blur_badge(uv: coord, time: signal) -> color {
  compose {
        circle(at: (0.5, 0.5), radius: 0.2) |> fill(#ffffff) |> blur(24px)
  }
}
"#;
        let bundle =
            compile_source_bundle(src, TEST_SOURCE_PATH, false).expect("bundle should compile");
        let manifest: Value =
            serde_json::from_str(&bundle.manifest).expect("manifest should parse as JSON");

        let canvases = manifest
            .get("canvases")
            .and_then(Value::as_array)
            .expect("manifest canvases should be an array");

        for canvas in canvases {
            let plan: fresco_artifact::ManifestPassPlan =
                serde_json::from_value(canvas["pass_plan"].clone())
                    .expect("runtime pass contract must decode compiler output");
            assert_eq!(
                serde_json::to_value(&plan).unwrap(),
                canvas["pass_plan"],
                "pass contract must preserve bindings, targets, kernel values, and omitted fields"
            );

            let passes = canvas
                .get("pass_plan")
                .and_then(|p| p.get("passes"))
                .and_then(Value::as_array)
                .expect("manifest pass_plan.passes should be an array");

            for pass in passes {
                let entry = pass
                    .get("entry_point")
                    .and_then(Value::as_str)
                    .expect("manifest pass entry should include entry_point");
                assert!(
                    bundle.wgsl.contains(&format!("fn {entry}(")),
                    "manifest pass entry '{entry}' missing in emitted WGSL\nWGSL:\n{}\nManifest:\n{}",
                    bundle.wgsl,
                    bundle.manifest
                );
            }
        }
    }

    fn executable_fullscreen_files(shade_uv: &str) -> HashMap<String, String> {
        let mut files = HashMap::new();
        files.insert(
            "engine/fullscreen.fr".to_string(),
            format!(
                r#"
@group(3) group frame
struct FrameGlobals {{
  @semantic(time) time: f32
  @semantic(delta_time) delta_time: f32
  @semantic(resolution) resolution: vec2
}}
param frame: FrameGlobals
struct ScreenVarying {{
  clip_pos: vec4
  uv: vec2
}}
struct CanvasContext {{
  @semantic(coord) uv: vec2
  frame: FrameGlobals
}}
@entry(canvas, draw)
interface Canvas {{ fn draw(@context ctx: CanvasContext) -> color }}
pass present for Canvas {{
  stage: raster
  draw: fullscreen
  binding {{ @group(frame) frame: uniform<FrameGlobals> }}
  fn vertex(vertex_id: u32) -> ScreenVarying {{
    let p = fullscreen_triangle_position(vertex_id)
    return ScreenVarying(clip_pos: vec4(p, 0.0, 1.0), uv: p * 0.5 + vec2(0.5, 0.5))
  }}
  fn shade(v: ScreenVarying, instance: Canvas) -> color {{
    return instance.draw(CanvasContext(uv: {shade_uv}, frame: frame))
  }}
}}
pipeline present_pipeline for Canvas {{ present }}
"#,
            ),
        );
        files.insert(
            "main.fr".to_string(),
            r#"
canvas first(ctx: CanvasContext) -> color {
  param gain: f32 = 0.25
  rgba(ctx.uv.x * gain, ctx.uv.y, 0.0, 1.0)
}
canvas second(ctx: CanvasContext) -> color {
  param gain: f32 = 0.75
  rgba(0.0, ctx.uv.x, ctx.uv.y * gain, 1.0)
}
"#
            .to_string(),
        );
        files
    }

    fn executable_fullscreen_bundle(shade_uv: &str) -> CompileBundleOutput {
        compile_bundle_virtual(&executable_fullscreen_files(shade_uv), "main.fr", false)
            .unwrap_or_else(|diagnostics| panic!("fullscreen bundle failed: {diagnostics:#?}"))
    }

    #[test]
    fn canvas_arrays_and_path_buffers_share_a_collision_free_binding_allocator() {
        let source = r#"canvas first(uv: coord) -> color {
            param weights: array<f32> = [0.5]
            let curve = path {
                move (0.10, 0.30)
                cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
                arc center: (0.72, 0.55) radius: 0.20 sweep: 360deg
            }
            compose { curve |> stroke(width: weights[0]) }
        }"#;
        let source = format!(
            "{source}\n{}",
            source.replace("canvas first", "canvas second")
        );
        let bundle = compile_bundle_virtual(
            &crate::test_support::files(&source, "main.fr"),
            "main.fr",
            false,
        )
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&bundle.manifest).unwrap();
        let module = naga::front::wgsl::parse_str(&bundle.wgsl).unwrap();
        let mut bindings = std::collections::BTreeSet::new();
        for canvas in &manifest.canvases {
            assert_eq!(canvas.storage_params.len(), 1);
            assert_eq!(canvas.path_buffers.len(), 1);
            let path = &canvas.path_buffers[0];
            let data = path
                .data
                .as_ref()
                .expect("standalone hosts receive geometry");
            assert_eq!(data.layout, fresco_artifact::PATH_SEGMENT_LAYOUT);
            assert_eq!(data.stride, fresco_artifact::PATH_SEGMENT_STRIDE);
            assert_eq!(data.rows.len(), path.segments);
            assert_eq!(data.rows[0].p0, [0.1, 0.3]);
            assert_eq!(data.rows[0].s0, 0.0);
            let global = module
                .global_variables
                .iter()
                .find_map(|(_, global)| {
                    global
                        .binding
                        .as_ref()
                        .filter(|binding| {
                            (binding.group, binding.binding) == (path.group, path.binding)
                        })
                        .map(|_| global)
                })
                .unwrap();
            let naga::TypeInner::Array {
                base,
                stride,
                size: naga::ArraySize::Constant(count),
            } = module.types[global.ty].inner
            else {
                panic!("fixed path array")
            };
            assert_eq!(stride, data.stride);
            assert_eq!(count.get() as usize, path.segments);
            let naga::TypeInner::Struct { ref members, span } = module.types[base].inner else {
                panic!("path record")
            };
            assert_eq!(span, data.stride);
            assert_eq!(
                members
                    .iter()
                    .map(|member| member.offset)
                    .collect::<Vec<_>>(),
                [0, 8, 16, 24, 32, 36, 40, 44, 48]
            );
            // WGSL identifiers may be renamed; offsets and scalar encodings are
            // the host ABI. In particular, kind must retain integer bits.
            for (index, member) in members.iter().enumerate() {
                let expected = if index < 4 {
                    naga::TypeInner::Vector {
                        size: naga::VectorSize::Bi,
                        scalar: naga::Scalar {
                            kind: naga::ScalarKind::Float,
                            width: 4,
                        },
                    }
                } else {
                    naga::TypeInner::Scalar(naga::Scalar {
                        kind: if index == 6 {
                            naga::ScalarKind::Uint
                        } else {
                            naga::ScalarKind::Float
                        },
                        width: 4,
                    })
                };
                assert_eq!(module.types[member.ty].inner, expected);
            }
            let roundtrip: fresco_artifact::ManifestPathBuffer =
                serde_json::from_str(&serde_json::to_string(path).unwrap()).unwrap();
            assert_eq!(roundtrip.data, path.data);
            for binding in canvas
                .storage_params
                .iter()
                .map(|p| (p.group, p.binding))
                .chain(canvas.path_buffers.iter().map(|p| (p.group, p.binding)))
            {
                assert!(
                    bindings.insert(binding),
                    "binding {binding:?} is shared by unrelated storage buffers"
                );
            }
        }
        let emitted: std::collections::BTreeSet<_> = module
            .global_variables
            .iter()
            .filter(|(_, global)| matches!(global.space, naga::AddressSpace::Storage { .. }))
            .map(|(_, global)| {
                let binding = global.binding.as_ref().unwrap();
                (binding.group, binding.binding)
            })
            .collect();
        assert_eq!(bindings.len(), 4);
        assert_eq!(bindings, emitted);
    }

    #[test]
    fn length_only_paths_do_not_publish_unallocated_geometry_buffers() {
        let source = r#"canvas length_only(uv: coord) -> color {
            let curve = path {
                move (0.10, 0.30)
                cubic (0.30, 0.85) (0.45, 0.05) (0.62, 0.55)
                arc center: (0.72, 0.55) radius: 0.20 sweep: 360deg
            }
            rgba(curve.length * 0.1, 0.0, 0.0, 1.0)
        }"#;
        let bundle = compile_bundle_virtual(
            &crate::test_support::files(source, "main.fr"),
            "main.fr",
            false,
        )
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&bundle.manifest).unwrap();
        assert!(manifest.canvases[0].path_buffers.is_empty());
        assert!(!bundle.wgsl.contains("fresco_path_buf_"));
    }

    #[test]
    fn authored_fullscreen_storage_bindings_do_not_collide_with_instance_uniforms() {
        let mut files = executable_fullscreen_files("v.uv");
        files.insert(
            "main.fr".into(),
            r#"
canvas arrays(ctx: CanvasContext) -> color {
    param gain: f32 = 0.5
    param weights: array<f32> = [0.25, 0.75]
    param points: array<vec3> = [vec3(0.1, 0.2, 0.3), vec3(0.4, 0.5, 0.6)]
    rgba(weights[1] * gain, points[1].y, ctx.uv.x, 1.0)
}
"#
            .into(),
        );
        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("authored stages must bind storage and instance parameters together");
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&bundle.manifest).unwrap();
        let canvas = &manifest.canvases[0];
        let pass = canvas.engine_pass.as_ref().unwrap();
        let module = naga::front::wgsl::parse_str(&bundle.wgsl).unwrap();
        for def in &canvas.storage_params {
            assert_ne!(
                (def.group, def.binding),
                (pass.instance_uniform_group, pass.instance_uniform_binding)
            );
            let global = module
                .global_variables
                .iter()
                .find_map(|(_, global)| {
                    (global.name.as_deref() == Some(&format!("fresco_param_arrays_{}", def.name)))
                        .then_some(global)
                })
                .unwrap();
            let binding = global.binding.as_ref().unwrap();
            assert_eq!((binding.group, binding.binding), (def.group, def.binding));
            if def.name == "points" {
                assert!(matches!(
                    module.types[global.ty].inner,
                    naga::TypeInner::Array { stride: 16, .. }
                ));
            }
        }
        assert_eq!(canvas.storage_params.len(), 2);
    }

    #[test]
    fn engine_authored_fullscreen_pass_emits_executable_stages_per_instance() {
        let bundle = executable_fullscreen_bundle("v.uv");
        let manifest: Value = serde_json::from_str(&bundle.manifest).expect("manifest JSON");
        let canvases = manifest["canvases"].as_array().expect("canvas array");
        assert_eq!(canvases.len(), 2);
        for canvas in canvases {
            let name = canvas["name"].as_str().expect("canvas name");
            let pass = &canvas["engine_pass"];
            for key in ["vertex_entry", "fragment_entry"] {
                let entry = pass[key].as_str().expect("engine pass stage entry");
                assert!(bundle.wgsl.contains(&format!("fn {entry}(")));
            }
            assert!(bundle.wgsl.contains(&format!("fresco_{name}(")));
            assert_eq!(canvas["params"][0]["name"], "gain");
            assert_eq!(pass["vertex_count"], 3);
            assert_eq!(pass["instance_uniform_group"], 0);
            assert_eq!(pass["instance_uniform_binding"], 0);
        }
        assert!(bundle.wgsl.contains("fresco_first(CanvasContext(v.uv"));
        assert!(bundle.wgsl.contains("fresco_second(CanvasContext(v.uv"));
        assert!(
            bundle
                .wgsl
                .contains("@group(0) @binding(0)\nvar<uniform> fresco_fullscreen_uniforms")
        );
        assert!(
            bundle
                .wgsl
                .contains("@group(3) @binding(0)\nvar<uniform> frame")
        );
        assert_eq!(canvases[0]["global_uniforms"][0]["name"], "frame");
        assert_eq!(canvases[0]["global_uniforms"][0]["group"], 3);
        assert_eq!(canvases[0]["global_uniforms"][0]["binding"], 0);
    }

    #[test]
    fn editing_authored_fullscreen_shade_changes_executed_fragment_code() {
        let original = executable_fullscreen_bundle("v.uv");
        let edited = executable_fullscreen_bundle("vec2(0.25, 0.75)");
        assert_ne!(original.wgsl, edited.wgsl);
        assert!(
            edited
                .wgsl
                .contains("fresco_first(CanvasContext(vec2<f32>(0.25, 0.75)")
        );
        assert!(
            !original
                .wgsl
                .contains("fresco_first(CanvasContext(vec2<f32>(0.25, 0.75)")
        );

        let mut vertex_files = executable_fullscreen_files("v.uv");
        let engine = vertex_files
            .get_mut("engine/fullscreen.fr")
            .expect("engine source");
        *engine = engine.replace("uv: p * 0.5 + vec2(0.5, 0.5)", "uv: vec2(0.125, 0.875)");
        let vertex_edited = compile_bundle_virtual(&vertex_files, "main.fr", false)
            .expect("vertex-edited pass compiles");
        assert_ne!(original.wgsl, vertex_edited.wgsl);
        assert!(vertex_edited.wgsl.contains(
            "FrescoFullscreenVarying_first(vec4<f32>(p, 0.0, 1.0), vec2<f32>(0.125, 0.875))"
        ));

        let repeated = executable_fullscreen_bundle("v.uv");
        assert_eq!(original.wgsl, repeated.wgsl);
        assert_eq!(original.manifest, repeated.manifest);
    }

    #[test]
    fn compile_known_fullscreen_permutations_emit_selectable_specialized_stages() {
        let mut files = executable_fullscreen_files("v.uv");
        let engine = files
            .get_mut("engine/fullscreen.fr")
            .expect("engine source");
        *engine = engine
            .replace(
                "  binding { @group(frame) frame: uniform<FrameGlobals> }",
                "  permutations {\n    @known(compile) quality: \"low\" | \"high\"\n  }\n  binding { @group(frame) frame: uniform<FrameGlobals> }",
            )
            .replace(
                "    return instance.draw(CanvasContext(uv: v.uv, frame: frame))",
                "    if quality == \"low\" {\n      return instance.draw(CanvasContext(uv: vec2(0.25, 0.75), frame: frame))\n    } else {\n      return instance.draw(CanvasContext(uv: v.uv, frame: frame))\n    }",
            );
        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("compile-known fullscreen variants compile");
        let manifest: Value = serde_json::from_str(&bundle.manifest).expect("manifest JSON");
        let variants = manifest["canvases"][0]["engine_pass"]["variants"]
            .as_array()
            .expect("engine-pass variants");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["key"], "quality=low");
        assert_eq!(variants[1]["key"], "quality=high");
        assert_eq!(variants[0]["bindings"][0]["axis"], "quality");
        assert_eq!(variants[0]["bindings"][0]["value"], "low");
        for variant in variants {
            let vertex = variant["vertex_entry"].as_str().expect("vertex entry");
            let fragment = variant["fragment_entry"].as_str().expect("fragment entry");
            assert!(bundle.wgsl.contains(&format!("fn {vertex}(")));
            assert!(bundle.wgsl.contains(&format!("fn {fragment}(")));
        }
        assert!(
            bundle
                .wgsl
                .contains("fresco_first(CanvasContext(vec2<f32>(0.25, 0.75)")
        );
        assert!(bundle.wgsl.contains("fresco_first(CanvasContext(v.uv"));
        assert!(!bundle.wgsl.contains("quality =="));
    }

    #[test]
    fn vertex_factory_contract_emits_layout_without_a_phantom_shader_entry() {
        let files = HashMap::from([(
            "main.fr".to_string(),
            r#"
@group(2) group draw
vertex_interface Shaded {
  position: vec3 in object
  normal: vec3 in object
  uv: vec2
}

vertex_format StaticMesh {
  required {
    position: vec3 in object
    normal: vec3 in object
    uv: vec2
  }
}

vertex_factory static for StaticMesh {
  binding {
    @group(draw) model: uniform<mat4>
  }

  fn transform(v: StaticMesh) -> mat4 {
    return model
  }
}
vertex_factory raw for StaticMesh {}
"#
            .to_string(),
        )]);
        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("vertex factory contract compiles");
        let manifest: Value = serde_json::from_str(&bundle.manifest).expect("manifest JSON");
        let factory = &manifest["vertex_factories"][0];
        assert_eq!(factory["name"], "static");
        assert_eq!(factory["vertex_format"], "StaticMesh");
        assert!(factory.get("transform_entry").is_none());
        assert!(
            !bundle
                .wgsl
                .contains("fresco_vertex_factory_static_transform")
        );
        assert_eq!(factory["array_stride"], 32);
        assert_eq!(factory["satisfies_interfaces"][0], "Shaded");
        assert_eq!(factory["bindings"][0]["group"], "draw");
        assert_eq!(factory["bindings"][0]["signature"], "uniform<mat4>");
        assert_eq!(factory["attributes"][0]["shader_location"], 0);
        assert_eq!(factory["attributes"][0]["offset"], 0);
        assert_eq!(factory["attributes"][0]["gpu_format"], "float32x3");
        assert_eq!(factory["attributes"][2]["offset"], 24);
        assert_eq!(factory["attributes"][2]["gpu_format"], "float32x2");
        for emitted in manifest["vertex_factories"].as_array().unwrap() {
            let shared: fresco_artifact::ManifestVertexFactory =
                serde_json::from_value(emitted.clone()).expect("shared vertex contract");
            assert_eq!(serde_json::to_value(shared).unwrap(), *emitted);
        }
        let raw = &manifest["vertex_factories"][1];
        assert_eq!(raw["name"], "raw");
        assert!(raw.get("transform_entry").is_none());
        assert_eq!(raw["bindings"], serde_json::json!([]));
    }

    #[test]
    fn vertex_factory_without_interfaces_preserves_omitted_fields() {
        let files = HashMap::from([(
            "main.fr".into(),
            "vertex_format PositionOnly { required { position: vec3 } }\nvertex_factory raw for PositionOnly {}".into(),
        )]);
        let bundle = compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: Value = serde_json::from_str(&bundle.manifest).unwrap();
        let factory = &manifest["vertex_factories"][0];
        assert!(factory.get("transform_entry").is_none());
        assert!(factory.get("satisfies_interfaces").is_none());
        assert_eq!(factory["array_stride"], 12);
        let shared: fresco_artifact::ManifestVertexFactory =
            serde_json::from_value(factory.clone()).unwrap();
        assert_eq!(serde_json::to_value(shared).unwrap(), *factory);
    }

    pub(super) fn executable_mesh_files(
        vertex_scale: &str,
        shade_expr: &str,
    ) -> HashMap<String, String> {
        HashMap::from([
            (
                "engine/mesh.fr".to_string(),
                format!(
                    r#"
@group(2) group draw
material_properties base {{
  channel albedo: color
  channel emissive: vec3
  channel opacity: f32
}}
vertex_interface PreviewShaded {{
  position: vec3 in object
  normal: vec3 in object
  tangent: vec3 in object
  uv: vec2
  uv2: vec2
}}
vertex_format PreviewMesh {{
  required {{
    position: vec3 in object
    normal: vec3 in object
    tangent: vec3 in object
    uv: vec2
    uv2: vec2
  }}
}}
struct PreviewScene {{
  model: mat4
  view: mat4
  proj: mat4
  camera_pos: vec3
  time: f32
  res: vec2
  padding: vec2
  displacement: vec4
}}
struct PreviewVarying {{
  @semantic(position) clip_pos: vec4
  world_pos: vec3
  world_normal: vec3
  world_tangent: vec3
  uv: vec2
  uv2: vec2
}}
vertex_factory preview_static for PreviewMesh {{
  binding {{ @group(draw) scene: uniform<PreviewScene> }}
  fn transform(v: PreviewMesh) -> mat4 {{ return scene.model }}
}}
@factory(preview_static) pass preview_mesh for base {{
  stage: raster
  draw: per_object
  @vertex fn vertex(v: PreviewShaded) -> PreviewVarying {{
    let model = factory.transform(v)
    let world_pos = model * vec4(v.position * {vertex_scale}, 1.0)
    return PreviewVarying(scene.proj * scene.view * world_pos, world_pos.xyz,
      normalize((model * vec4(v.normal, 0.0)).xyz), v.tangent, v.uv, v.uv2)
  }}
  fn material_context(sp: PreviewVarying) -> surf {{ return surf(sp.uv, sp.uv2, scene.time, scene.res, sp.world_pos, sp.world_normal) }}
  @evaluate(surface) fn evaluate(sp: surf) -> base {{}}
  @fragment fn raster(sp: PreviewVarying) -> color {{ return shade(sp, evaluate(material_context(sp))) }}
  fn shade(sp: PreviewVarying, m: base) -> color {{ return {shade_expr} }}
}}
pipeline(postprocess) preview for base {{ preview_mesh }}
"#,
                ),
            ),
            (
                "main.fr".to_string(),
                r#"
surface sample(sp: surf) -> material(base) {
  compose { base(albedo: rgba(sp.uv.x, sp.uv.y, 0.0, 1.0), emissive: vec3(0.2), opacity: 0.75) }
}
"#
                .to_string(),
            ),
        ])
    }

    #[test]
    fn named_implementations_select_and_dispatch_without_numeric_source_ids() {
        let mut files = executable_mesh_files("1.0", "apply_response(response, m.albedo)");
        let engine = files.get_mut("engine/mesh.fr").unwrap();
        *engine = engine.replace("fn shade(sp:", "@dispatch(Response, apply) fn apply_response(id: u32, value: vec4) -> vec4 { return vec4(0.0) }\n fn shade(sp:");
        engine.push_str(
            r#"
            interface Response { fn apply(value: vec4) -> vec4 }
            @implementation(Response) struct DefaultResponse {}
            conform DefaultResponse : Response { fn apply(value: vec4) -> vec4 { return value } }
            @surface_properties(options) interface Options {
                param @config(editor) response: implementation<Response> = DefaultResponse
            }
            @table(surfaces, ascending, 1) struct Responses {
                @table_index material_id: u32
                @implementation_value(response) response_id: u32
            }
        "#,
        );
        files.insert(
            "external.fr".into(),
            r#"
            @implementation(Response) struct HalfResponse {}
            conform HalfResponse : Response { fn apply(value: vec4) -> vec4 { return value * 0.5 } }
        "#
            .into(),
        );
        files
            .get_mut("main.fr")
            .unwrap()
            .insert_str(0, "import \"external.fr\"\n");
        let output = compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        let selection = &manifest.surfaces[0]
            .settings
            .as_ref()
            .unwrap()
            .implementations[0];
        assert_eq!(selection.symbol, "DefaultResponse");
        assert_eq!(selection.id, 1);
        assert_eq!(selection.available, ["DefaultResponse", "HalfResponse"]);
        assert!(
            !output
                .wgsl
                .contains("fresco_implementation_HalfResponse_apply")
        );
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"property_overrides": {"sample": {"response": "HalfResponse"}}})
                .to_string(),
        );
        let output = compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        assert_eq!(
            manifest.surfaces[0]
                .settings
                .as_ref()
                .unwrap()
                .implementations[0]
                .symbol,
            "HalfResponse"
        );
        assert_eq!(manifest.tables[0].records[0].values, [1, 1]);
        assert!(
            output
                .wgsl
                .contains("fresco_implementation_HalfResponse_apply")
        );
        assert!(
            !output
                .wgsl
                .contains("fresco_implementation_DefaultResponse_apply")
        );
        for value in [serde_json::json!(1), serde_json::json!("Missing")] {
            files.insert(
                "fresco.config.json".into(),
                serde_json::json!({"property_overrides": {"sample": {"response": value}}})
                    .to_string(),
            );
            assert!(compile_bundle_virtual(&files, "main.fr", false).is_err());
        }
    }

    #[test]
    fn raster_imports_link_reachable_typed_functions_in_library_scope() {
        let mut files = executable_mesh_files("1.0", "library_response(m.albedo)");
        files.get_mut("engine/mesh.fr").unwrap().push_str(
            r#"
            struct Response { weight: f32, value: vec4 }
            fn library_leaf(value: vec4) -> Response { return Response(value: value * 0.5, weight: 1.0) }
            fn library_response(value: vec4) -> vec4 { return library_leaf(value).value }
            fn unused_response(value: vec4) -> vec4 { return value * 0.25 }
            fn unused_bridge(value: vec4) -> vec4 { return unused_response(value) }
        "#,
        );
        let output = compile_bundle_virtual(&files, "main.fr", false).unwrap();
        assert!(output.wgsl.contains("import_library_leaf"));
        assert!(output.wgsl.contains("import_library_response"));
        assert!(!output.wgsl.contains("unused_response"));
        let original = files["engine/mesh.fr"].clone();
        for invalid in ["scene.time", "missing_value"] {
            files.insert(
                "engine/mesh.fr".into(),
                original.replace("value * 0.5", &format!("value * {invalid}")),
            );
            assert!(
                compile_bundle_virtual(&files, "main.fr", false).is_err(),
                "{invalid} must not capture caller bindings"
            );
        }
    }

    #[test]
    fn checked_property_overrides_feed_exact_generic_table_columns() {
        let mut files = executable_mesh_files("1.0", "m.albedo");
        files.get_mut("engine/mesh.fr").unwrap().push_str(
            r#"
            enum Choice: u32 { First, Second }
            @surface_properties(options) interface Options {
                param @config(editor) choice: Choice = Choice.First
                param @config(editor) count: u32 = u32(16777217)
                param @config(engine) locked: u32 = u32(3)
            }
            @table(surfaces, ascending, 1) struct SettingsRow {
                @table_index identity: u32
                @property_value(choice) selected: u32
                @property_value(count) count: u32
            }
        "#,
        );
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"property_overrides": {"sample": {"choice": "Choice.Second", "count": 16777219}}})
                .to_string(),
        );
        let output = compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        assert_eq!(manifest.tables[0].records[0].values, [1, 1, 16777219]);
        for overrides in [
            serde_json::json!({"missing": {"choice": "Choice.First"}}),
            serde_json::json!({"sample": {"missing": 1}}),
            serde_json::json!({"sample": {"locked": 5}}),
            serde_json::json!({"sample": {"choice": "Choice.Missing"}}),
            serde_json::json!({"sample": {"count": -1}}),
        ] {
            files.insert(
                "fresco.config.json".into(),
                serde_json::json!({"property_overrides": overrides}).to_string(),
            );
            assert!(
                compile_bundle_virtual(&files, "main.fr", false).is_err(),
                "{overrides}"
            );
        }
        files.remove("fresco.config.json");
        let engine = files.get_mut("engine/mesh.fr").unwrap();
        *engine = engine.replace("@property_value(choice)", "@property_value(missing)");
        assert!(compile_bundle_virtual(&files, "main.fr", false).is_err());
    }

    #[test]
    fn executable_mrt_encoder_preserves_unsigned_material_identity() {
        let mut files = executable_mesh_files("1.0", "m.albedo");
        let engine = files.get_mut("engine/mesh.fr").unwrap();
        *engine = engine.replace("@factory(preview_static)", "@table(surfaces, ascending, 5) struct Identity { @table_index id: u32 }\nstruct Encoded { @location(0) color: vec4\n @location(6) id: u32 }\n@factory(preview_static)");
        *engine = engine.replace("scene: uniform<PreviewScene>", "scene: uniform<PreviewScene>\n @group(draw) @source(Identity) record: uniform<Identity>");
        *engine = engine.replace("  fn shade(", "  @fragment fn pack_pixels(sp: PreviewVarying) -> Encoded { return Encoded(evaluate(material_context(sp)).albedo, record.id) }\n  fn shade(");
        let output = compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        assert_eq!(manifest.tables[0].records[0].index, 5);
        let entry = manifest.surfaces[0]
            .mesh_passes
            .first()
            .unwrap()
            .entries
            .iter()
            .find(|e| e.function == "pack_pixels")
            .unwrap();
        assert_eq!(
            entry
                .outputs
                .iter()
                .map(|o| (o.location, o.ty.as_str()))
                .collect::<Vec<_>>(),
            [(0, "vec4"), (6, "u32")]
        );
        assert!(output.wgsl.contains("@location(6) id: u32"));
        assert!(output.wgsl.contains(&entry.entry));
        let engine = files.get_mut("engine/mesh.fr").unwrap();
        *engine = engine.replace("record.id)", "f32(record.id))");
        assert!(compile_bundle_virtual(&files, "main.fr", false).is_err());
    }

    #[test]
    fn executable_mesh_storage_loops_preserve_integer_casts() {
        let mut files = executable_mesh_files("1.0", "m.albedo");
        let engine = files.get_mut("engine/mesh.fr").unwrap();
        *engine = engine.replace("scene: uniform<PreviewScene>", "scene: uniform<PreviewScene>\n @group(draw) weights: buffer<vec4>\n @group(draw) indices: buffer<u32>");
        *engine = engine.replace("return m.albedo", "var total = vec4(0.0)\n for i in 0 .. 4 { let slot = u32(i) % u32(4)\n total = total + weights[indices[slot]] }\n return total * m.albedo");
        let output = compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
        let (_, weights) = module
            .global_variables
            .iter()
            .find(|(_, global)| {
                global
                    .binding
                    .as_ref()
                    .is_some_and(|binding| binding.group == 2 && binding.binding == 1)
            })
            .expect("weights storage binding");
        assert_eq!(
            weights.space,
            naga::AddressSpace::Storage {
                access: naga::StorageAccess::LOAD
            }
        );
        let naga::TypeInner::Array { base, stride, .. } = module.types[weights.ty].inner else {
            panic!("weights must remain a storage array");
        };
        assert_eq!(stride, 16);
        assert_eq!(
            module.types[base].inner,
            naga::TypeInner::Vector {
                size: naga::VectorSize::Quad,
                scalar: naga::Scalar {
                    kind: naga::ScalarKind::Float,
                    width: 4
                },
            }
        );
        assert!(output.wgsl.contains("u32(4.0)"));
        assert!(output.wgsl.contains("for (var i"));
        let manifest: Value = serde_json::from_str(&output.manifest).unwrap();
        let bindings = manifest["vertex_factories"][0]["bindings"]
            .as_array()
            .unwrap();
        assert_eq!(bindings.len(), 3);
        assert_eq!(bindings[2]["binding"], 2);
        // Unsupported resource declarations must produce a diagnostic, not vanish.
        let engine = files.get_mut("engine/mesh.fr").unwrap();
        *engine = engine.replace("buffer<vec4>", "buffer<UnknownRecord>");
        assert!(compile_bundle_virtual(&files, "main.fr", false).is_err());
    }

    #[test]
    fn engine_authored_mesh_factory_and_material_pass_emit_executable_stages() {
        let files = executable_mesh_files("1.0", "m.albedo");
        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("engine-authored mesh contract compiles");
        let manifest: Value = serde_json::from_str(&bundle.manifest).expect("manifest JSON");
        let mesh = &manifest["surfaces"][0]["mesh_passes"][0];
        let entries = mesh["entries"].as_array().unwrap();
        let vertex = entries.iter().find(|e| e["stage"] == "vertex").unwrap()["entry"]
            .as_str()
            .unwrap();
        let fragment = entries.iter().find(|e| e["stage"] == "fragment").unwrap()["entry"]
            .as_str()
            .unwrap();
        assert!(bundle.wgsl.contains(&format!("fn {vertex}(")));
        assert!(bundle.wgsl.contains(&format!("fn {fragment}(")));
        assert!(bundle.wgsl.contains("return m.albedo;"));
        assert_eq!(mesh["factory"], "preview_static");
        assert_eq!(manifest["vertex_factories"][0]["array_stride"], 52);
        assert_eq!(
            manifest["vertex_factories"][0]["bindings"][0]["group_index"],
            2
        );
        assert_eq!(manifest["vertex_factories"][0]["bindings"][0]["binding"], 0);

        let vertex_edited =
            compile_bundle_virtual(&executable_mesh_files("0.5", "m.albedo"), "main.fr", false)
                .expect("edited mesh vertex hook compiles");
        assert_ne!(bundle.wgsl, vertex_edited.wgsl);
        assert!(vertex_edited.wgsl.contains("(v.position * 0.5)"));

        let shade_edited = compile_bundle_virtual(
            &executable_mesh_files("1.0", "rgba(m.opacity, m.opacity, m.opacity, 1.0)"),
            "main.fr",
            false,
        )
        .expect("edited mesh shade hook compiles");
        assert_ne!(bundle.wgsl, shade_edited.wgsl);
        assert!(
            shade_edited
                .wgsl
                .contains("return vec4<f32>(m.opacity, m.opacity, m.opacity, 1.0);")
        );
    }

    #[test]
    fn engine_authored_particle_compute_and_raster_entries_are_executable() {
        let mut files = executable_mesh_files("1.0", "m.albedo");
        files.insert(
            "engine/particle.fr".into(),
            include_str!("../tests/fixtures/engines/particle.fr").into(),
        );
        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("particle pipeline compiles through the test engine contract");
        let manifest: Value = serde_json::from_str(&bundle.manifest).expect("manifest JSON");
        let schema: Value = serde_json::from_str(
            &std::fs::read_to_string(contract_path()).expect("contract schema should load"),
        )
        .expect("contract schema should parse");
        validate_required(&manifest, &schema, &schema);
        let technique = &manifest["techniques"][0];
        let shared: fresco_artifact::ManifestTechnique =
            serde_json::from_value(technique.clone()).unwrap();
        assert_eq!(serde_json::to_value(&shared).unwrap(), *technique);
        assert_eq!(shared.steps.len(), 3);
        for program in manifest["gpu_programs"].as_array().unwrap() {
            for entry in program["entries"].as_array().unwrap() {
                assert!(
                    bundle
                        .wgsl
                        .contains(&format!("fn {}(", entry["entry"].as_str().unwrap()))
                );
            }
        }
        let simulation = manifest["gpu_programs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["pass"] == "preview_particle_sim")
            .unwrap();
        assert_eq!(simulation["metadata"]["capacity"], "64");
        let state = simulation["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "particles")
            .unwrap();
        assert_eq!(state["group"], 2);
        assert_eq!(state["binding"], 1);
        assert_eq!(state["access"], "read_write");
        assert!(bundle.wgsl.contains("@workgroup_size(64"));
    }

    #[test]
    fn executable_fullscreen_permutations_reject_unsupported_binding_times() {
        for mode in ["pipeline", "draw"] {
            let mut files = executable_fullscreen_files("v.uv");
            let engine = files
                .get_mut("engine/fullscreen.fr")
                .expect("engine source");
            *engine = engine.replace(
                "  binding { @group(frame) frame: uniform<FrameGlobals> }",
                &format!(
                    "  permutations {{\n    @known({mode}) quality: \"low\" | \"high\"\n  }}\n  binding {{ @group(frame) frame: uniform<FrameGlobals> }}"
                ),
            );
            let diagnostics = compile_bundle_virtual(&files, "main.fr", false)
                .expect_err("unsupported fullscreen binding time must fail");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("must use `@known(compile)`")),
                "{mode}: {diagnostics:#?}"
            );
        }
    }

    #[test]
    fn executable_fullscreen_contract_reports_missing_plug_bad_signature_and_resource() {
        let cases = [
            ("missing plug", "", "missing required entry block `draw`"),
            (
                "bad signature",
                "canvas broken { fn draw(uv: f32) -> color { return rgba(uv, uv, 0.0, 1.0) } }",
                "signature does not match engine interface `Canvas`",
            ),
            (
                "missing resource",
                "canvas broken { fn draw(@context ctx: CanvasContext) -> color { return rgba(ctx.uv.x, ctx.uv.y, 0.0, 1.0) } }",
                "requires uniform resource `frame`",
            ),
        ];

        for (label, replacement_main, expected) in cases {
            let mut files = executable_fullscreen_files("v.uv");
            if label == "missing plug" {
                files.insert("main.fr".to_string(), "canvas broken { fn other(uv: vec2) -> color { return rgba(uv.x, uv.y, 0.0, 1.0) } }".to_string());
            } else {
                files.insert("main.fr".to_string(), replacement_main.to_string());
            }
            if label == "missing resource" {
                let engine = files
                    .get_mut("engine/fullscreen.fr")
                    .expect("engine source");
                *engine = engine.replace("param frame: FrameGlobals\n", "");
            }
            let diagnostics = compile_bundle_virtual(&files, "main.fr", false).expect_err(label);
            assert!(
                diagnostics
                    .iter()
                    .any(|diag| diag.message.contains(expected)),
                "{label}: {diagnostics:#?}"
            );
        }
    }

    #[test]
    fn pass_diagnostics_retain_source_files_for_root_import_and_implicit_engine_modules() {
        let mut files = HashMap::new();
        files.insert(
            "main.fr".to_string(),
            r#"
import "lib.fr"

pass root_bad {
  stage: not_a_stage
}

canvas c(uv: coord) -> color {
  compose {
    fill(#ffffff)
  }
}
"#
            .to_string(),
        );
        files.insert(
            "lib.fr".to_string(),
            r#"
pass import_bad {
  stage: also_not_a_stage
}
"#
            .to_string(),
        );
        files.insert(
            "engine/implicit_bad.fr".to_string(),
            r#"
pass implicit_bad {
  stage: still_not_a_stage
}
"#
            .to_string(),
        );

        let err = compile_bundle_virtual(&files, "main.fr", false)
            .expect_err("invalid pass stages should fail validation");

        assert!(
            err.iter().any(|diag| {
                diag.file == "main.fr" && diag.message.contains("unknown pass stage")
            }),
            "expected root-file pass diagnostic, got: {err:#?}"
        );
        assert!(
            err.iter().any(|diag| {
                diag.file == "lib.fr" && diag.message.contains("unknown pass stage")
            }),
            "expected imported-file pass diagnostic, got: {err:#?}"
        );
        assert!(
            err.iter().any(|diag| {
                diag.file == "engine/implicit_bad.fr" && diag.message.contains("unknown pass stage")
            }),
            "expected implicit engine-file pass diagnostic, got: {err:#?}"
        );
    }

    #[test]
    fn canvas_contract_form_compiles_via_canvas_interface_conformance() {
        let src = r#"
@group(3) group frame
struct FrameGlobals {
    time: f32
    delta_time: f32
    resolution: vec2
}
param frame: FrameGlobals

struct ScreenVarying {
    clip_pos: vec4
    uv: vec2
}

struct CanvasViewConfig {
    @config(editor) zoom: f32 = 1.0
    @config(editor) pan: vec2 = vec2(0.0, 0.0)
}

interface canvas {
    fn draw(uv: vec2) -> color
}

canvas glow {
    fn draw(uv: vec2) -> color {
        return rgba(r: uv.x, g: uv.x, b: uv.x, a: 1.0)
    }
}

pass present_canvas for canvas {
    stage: raster
    draw: fullscreen

    binding {
        @group(frame) frame: uniform<FrameGlobals>
    }

    fn vertex(vertex_id: u32) -> ScreenVarying {
        let p = fullscreen_triangle_position(vertex_id)
        return ScreenVarying(
            clip_pos: vec4(p, 0.0, 1.0),
            uv: p * 0.5 + vec2(0.5, 0.5))
    }

    fn shade(v: ScreenVarying, t: canvas) -> color {
        return t.draw(v.uv)
    }
}

pipeline present for canvas {
    present_canvas
}
"#;

        let bundle = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("canvas contract form should normalize to interface conformance");
        assert!(
            bundle.diagnostics.is_empty(),
            "unexpected diagnostics: {bundle:#?}"
        );
        assert!(
            bundle.wgsl.contains("fn fresco_glow("),
            "expected lowered WGSL function for contract canvas conformance, got: {}",
            bundle.wgsl
        );
        assert!(
            bundle.manifest.contains("\"present\""),
            "expected pipeline entry in manifest, got: {}",
            bundle.manifest
        );
        assert!(
            bundle.manifest.contains("\"present_canvas\""),
            "expected pass entry in manifest, got: {}",
            bundle.manifest
        );
    }

    #[test]
    fn canvas_contract_method_body_type_mismatch_is_rejected() {
        let src = r#"
struct CanvasContext {
    time: f32
    resolution: vec2
    mouse: vec2
    frame: u32
}

interface canvas {
    fn draw(uv: vec2, ctx: CanvasContext) -> color
}

canvas glow {
    fn draw(uv: vec2, ctx: CanvasContext) -> color {
        return uv
    }
}

pass present_canvas for canvas {
    stage: raster
    draw: fullscreen
}

pipeline present for canvas {
    present_canvas
}
"#;

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("conformance method body type mismatch should fail");
        assert!(
            err.iter().any(|diag| {
                diag.message
                    .contains("function `draw` returns vec2, but declared return type is color")
            }),
            "expected conformance body type mismatch diagnostic, got: {err:#?}"
        );
    }

    #[test]
    fn conformance_enum_return_invalid_static_ordinal_is_rejected() {
        let src = r#"
enum BlendMode {
    add
    screen
}

interface blend_logic {
    fn mode() -> BlendMode
}

conform glow: blend_logic {
    fn mode() -> BlendMode {
        return 7
    }
}
"#;

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("invalid enum ordinal in conformance method should fail");
        assert!(
            err.iter().any(|diag| {
                diag.message
                    .contains("returns an invalid `BlendMode` variant value")
            }),
            "expected enum variant diagnostic for conformance method return, got: {err:#?}"
        );
    }

    #[test]
    fn conformance_callable_return_fnref_compiles() {
        let src = r#"
fn ease(x: f32) -> f32 {
    return x
}

interface curve_provider {
    fn curve() -> fn(f32) -> f32
}

conform glow: curve_provider {
    fn curve() -> fn(f32) -> f32 {
        return ease
    }
}
"#;

        compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("callable conformance return should accept named function references");
    }

    #[test]
    fn conformance_texture_return_param_compiles() {
        let src = r#"
interface texture_provider {
    fn pick(t: texture<ORM>) -> texture<ORM>
}

conform glow: texture_provider {
    fn pick(t: texture<ORM>) -> texture<ORM> {
        return t
    }
}
"#;

        compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("texture-typed conformance return should accept typed texture parameters");
    }

    #[test]
    fn conformance_typevar_return_param_compiles() {
        let src = r#"
interface identity_provider {
    fn id(x: T) -> T
}

conform glow: identity_provider {
    fn id<T>(x: T) -> T {
        return x
    }
}
"#;

        compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("typevar conformance return should allow method-local generic type parameters");
    }

    #[test]
    fn conformance_array_shape_placeholder_is_rejected() {
        let src = r#"
interface shape_array_provider {
    fn choose(xs: array<shape>) -> shape
}

conform glow: shape_array_provider {
    fn choose(xs: array<shape>) -> shape {
        return circle(at: center, radius: 0.25)
    }
}
"#;

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("array<shape> placeholder binding should be rejected");
        assert!(
            err.iter().any(|diag| {
                diag.message.contains(
                    "conformance method parameter `choose.xs` uses unsupported type `array<shape>`",
                )
            }),
            "expected unsupported placeholder diagnostic for array<shape>, got: {err:#?}"
        );
    }

    #[test]
    fn conformance_malformed_callable_return_type_is_rejected() {
        let src = r#"
fn ease(x: f32) -> f32 {
    return x
}

interface curve_provider {
    fn curve() -> fn(f32) -> unknown_type
}

conform glow: curve_provider {
    fn curve() -> fn(f32) -> unknown_type {
        return ease
    }
}
"#;

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("callable return with unknown nested type should fail");
        assert!(
            err.iter().any(|diag| diag
                .message
                .contains("unsupported function callable return type `unknown_type`")),
            "expected callable nested type diagnostic, got: {err:#?}"
        );
    }

    #[test]
    fn conformance_array_layer_placeholder_reports_precise_span() {
        let src = "interface layer_array_provider {\n    fn choose(xs: array<layer>) -> layer\n}\n\nconform glow: layer_array_provider {\n    fn choose(xs: array<layer>) -> layer {\n        return xs\n    }\n}\n";

        let matches: Vec<usize> = src
            .match_indices("array<layer>")
            .map(|(idx, _)| idx)
            .collect();
        assert_eq!(matches.len(), 2, "expected two array<layer> occurrences");
        let expected_start = matches[1];
        let expected_end = expected_start + "array<layer>".len();

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("array<layer> placeholder binding should be rejected");
        let diag = err
            .iter()
            .find(|diag| {
                diag.message.contains(
                    "conformance method parameter `choose.xs` uses unsupported type `array<layer>`",
                )
            })
            .expect("expected unsupported placeholder diagnostic for array<layer>");

        assert_eq!(
            diag.span_start, expected_start,
            "unexpected diagnostic start span: {diag:#?}"
        );
        assert_eq!(
            diag.span_end, expected_end,
            "unexpected diagnostic end span: {diag:#?}"
        );
    }

    #[test]
    fn conformance_array_shape_placeholder_reports_precise_span() {
        let src = "interface shape_array_provider {\n    fn choose(xs: array<shape>) -> shape\n}\n\nconform glow: shape_array_provider {\n    fn choose(xs: array<shape>) -> shape {\n        return xs\n    }\n}\n";

        let matches: Vec<usize> = src
            .match_indices("array<shape>")
            .map(|(idx, _)| idx)
            .collect();
        assert_eq!(matches.len(), 2, "expected two array<shape> occurrences");
        let expected_start = matches[1];
        let expected_end = expected_start + "array<shape>".len();

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("array<shape> placeholder binding should be rejected");
        let diag = err
            .iter()
            .find(|diag| {
                diag.message.contains(
                    "conformance method parameter `choose.xs` uses unsupported type `array<shape>`",
                )
            })
            .expect("expected unsupported placeholder diagnostic for array<shape>");

        assert_eq!(
            diag.span_start, expected_start,
            "unexpected diagnostic start span: {diag:#?}"
        );
        assert_eq!(
            diag.span_end, expected_end,
            "unexpected diagnostic end span: {diag:#?}"
        );
    }

    #[test]
    fn imported_modules_may_declare_surface_entries() {
        let mut files = HashMap::new();
        files.insert(
            "main.fr".to_string(),
            r#"
import "lib.fr"

canvas c(uv: coord) -> color {
    compose {
        fill(#ffffff)
    }
}
"#
            .to_string(),
        );
        files.insert(
            "lib.fr".to_string(),
            r#"
surface imported_surface(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#
            .to_string(),
        );

        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("imported surface entries should remain supported");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn implicit_engine_modules_may_declare_surface_entries() {
        let mut files = HashMap::new();
        files.insert(
            "main.fr".to_string(),
            r#"
canvas c(uv: coord) -> color {
    compose {
        fill(#ffffff)
    }
}
"#
            .to_string(),
        );
        files.insert(
            "engine/implicit_surface.fr".to_string(),
            r#"
surface implicit_surface(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#
            .to_string(),
        );

        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("implicit engine surface entries should remain supported");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn engine_entry_resolves_transitive_imports_without_loading_unrelated_files() {
        let files = HashMap::from([
            (
                "main.fr".to_string(),
                "canvas c(uv: coord) -> color { compose { fill(engine_color()) } }".to_string(),
            ),
            (
                "engine/engine.fr".to_string(),
                "import \"core/api.fr\"".to_string(),
            ),
            (
                "engine/core/api.fr".to_string(),
                "import \"../values.fr\"\nfn engine_color() -> color { return engine_value() }"
                    .to_string(),
            ),
            (
                "engine/values.fr".to_string(),
                "fn engine_value() -> color { return #123456 }".to_string(),
            ),
            (
                "engine/unrelated.fr".to_string(),
                "this is intentionally invalid".to_string(),
            ),
        ]);
        compile_bundle_virtual(&files, "main.fr", false)
            .expect("only the engine entry import graph should be loaded");
    }

    #[test]
    fn engine_entry_missing_import_reports_the_importing_file() {
        let files = HashMap::from([
            (
                "main.fr".to_string(),
                "canvas c(uv: coord) -> color { compose { fill(#fff) } }".to_string(),
            ),
            (
                "engine/engine.fr".to_string(),
                "import \"missing.fr\"".to_string(),
            ),
        ]);
        let diagnostics = compile_bundle_virtual(&files, "main.fr", false)
            .expect_err("missing engine imports must fail compilation");
        assert!(
            diagnostics.iter().any(|diag| {
                diag.file == "engine/engine.fr" && diag.message.contains("missing.fr")
            }),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn engine_entry_import_cycle_is_diagnosed() {
        let files = HashMap::from([
            (
                "main.fr".to_string(),
                "canvas c(uv: coord) -> color { compose { fill(#fff) } }".to_string(),
            ),
            (
                "engine/engine.fr".to_string(),
                "import \"loop.fr\"".to_string(),
            ),
            (
                "engine/loop.fr".to_string(),
                "import \"engine.fr\"".to_string(),
            ),
        ]);
        let diagnostics = compile_bundle_virtual(&files, "main.fr", false)
            .expect_err("engine import cycles must fail compilation");
        assert!(
            diagnostics
                .iter()
                .any(|diag| diag.message.contains("cyclic import")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn engine_entry_imports_preserve_effect_declarations() {
        let files = HashMap::from([
            (
                "main.fr".to_string(),
                "canvas c(uv: coord) -> color { compose { engine_tint(0.5) } }".to_string(),
            ),
            (
                "engine/engine.fr".to_string(),
                "import \"effects.fr\"".to_string(),
            ),
            (
                "engine/effects.fr".to_string(),
                "effect engine_tint(amount: f32) point { fill(rgba(amount, 0.0, 0.0, 1.0)) }"
                    .to_string(),
            ),
        ]);
        compile_bundle_virtual(&files, "main.fr", false)
            .expect("effects imported by the engine entry must be available to canvases");
    }

    #[test]
    fn root_file_inside_engine_dir_does_not_self_merge() {
        let mut files = HashMap::new();
        files.insert(
            "engine/core/a.fr".to_string(),
            r#"
struct Surf {
    albedo: color
}

canvas c(uv: coord) -> color {
    compose {
        fill(#ffffff)
    }
}
"#
            .to_string(),
        );

        let bundle = compile_bundle_virtual(&files, "engine/core/a.fr", false)
            .expect("root file inside engine/ should not double-declare itself");
        assert!(
            bundle.diagnostics.is_empty(),
            "expected no diagnostics, got: {:#?}",
            bundle.diagnostics
        );
    }

    #[test]
    fn struct_typed_global_param_binds_known_runtime_channels() {
        let src = r#"
@group(3) group frame
struct FrameGlobals {
    time: f32
    delta_time: f32
    resolution: vec2
}

param frame: FrameGlobals

canvas demo(uv: coord) -> color {
    let t = frame.time
    let d = frame.delta_time
    let r = frame.resolution
    compose {
        fill(rgba(t, d, r.x, 1.0))
    }
}
"#;
        let bundle = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("struct-typed global param should compile");
        assert!(
            bundle.diagnostics.is_empty(),
            "expected no diagnostics, got: {:#?}",
            bundle.diagnostics
        );
    }

    #[test]
    fn scalar_global_param_is_declared_with_real_uniform_lowering() {
        let src = r#"
param intensity: f32 = 1.0 in 0.0 .. 2.0

canvas demo(uv: coord) -> color {
    compose {
        fill(rgba(intensity, intensity, intensity, 1.0))
    }
}
"#;
        let bundle = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("scalar global param should compile");
        assert!(
            bundle.diagnostics.is_empty(),
            "expected no diagnostics, got: {:#?}",
            bundle.diagnostics
        );
        // Global scalar params are declared through the same
        // `declare_param` path as canvas/surface-scoped params, so
        // they surface as an ordinary host-tweakable param in the
        // manifest (default/range included) rather than a hardcoded
        // constant.
        assert!(
            bundle.manifest.contains("\"name\": \"intensity\""),
            "expected `intensity` registered as a real param in the manifest, got: {}",
            bundle.manifest
        );
        assert!(
            bundle.manifest.contains("\"default\": 1.0"),
            "expected intensity's default in the manifest, got: {}",
            bundle.manifest
        );
    }

    /// Compiles `src` as the entrypoint of a virtual project whose `engine/`
    /// folder supplies the crate-owned authored-helper fixture explicitly.
    fn compile_with_prelude(src: &str) -> Result<CompileBundleOutput, Vec<DiagnosticRecord>> {
        let files = HashMap::from([
            ("main.fr".to_string(), src.to_string()),
            (
                "engine/00_prelude.fr".to_string(),
                crate::test_support::PRELUDE.to_string(),
            ),
        ]);
        compile_bundle_virtual(&files, "main.fr", false)
    }

    #[test]
    fn authored_prelude_helper_compiles_in_user_code() {
        let src = r#"
canvas demo(uv: coord) -> color {
    let m = mask(0.2, 0.8, uv.x)
    compose {
        circle(at: center, radius: 0.25) |> fill(rgba(m, m, m, 1.0))
    }
}
"#;

        let result = compile_with_prelude(src);
        assert!(
            result.is_ok(),
            "expected stdlib helper to compile: {result:?}"
        );
    }

    #[test]
    fn color_and_scalar_prelude_helpers_compile_in_user_code() {
        let src = r#"
canvas demo(uv: coord) -> color {
    let tint = color_mix(rgb(1.0, 0.25, 0.1), rgb(0.1, 0.35, 1.0), clamp01(uv.x))
    let _layer = grey(0.5)
    compose {
        circle(at: center, radius: 0.35) |> fill(tint)
    }
}
"#;

        let result = compile_with_prelude(src);
        assert!(
            result.is_ok(),
            "expected color/scalar prelude helpers to compile: {result:?}"
        );
    }

    #[test]
    fn rgb24_helper_compiles_in_user_code() {
        let src = r#"
canvas demo(uv: coord) -> color {
    let c = rgb24(6737151.25)
    compose {
        circle(at: center, radius: 0.3) |> fill(c)
    }
}
"#;

        let result = compile_with_prelude(src);
        assert!(
            result.is_ok(),
            "expected rgb24 helper to compile: {result:?}"
        );
    }

    #[test]
    fn rgb24_integer_overload_compiles_in_user_code() {
        let src = r#"
canvas demo(uv: coord) -> color {
    let c = rgb24(-1)
    compose {
        circle(at: center, radius: 0.3) |> fill(c)
    }
}
"#;

        let result = compile_with_prelude(src);
        assert!(
            result.is_ok(),
            "expected rgb24 integer overload call to compile: {result:?}"
        );
    }

    #[test]
    fn rgba32_u32_helper_compiles_in_user_code() {
        let src = r#"
canvas demo(uv: coord) -> color {
    let a = rgba32(4289379276)
    compose {
        circle(at: center, radius: 0.22) |> fill(a)
    }
}
"#;

        let result = compile_with_prelude(src);
        assert!(
            result.is_ok(),
            "expected rgba32 u32 helper to compile: {result:?}"
        );
    }

    #[test]
    fn pass_permutations_accept_quoted_string_literals() {
        let mut files = HashMap::new();
        files.insert(
            "main.fr".to_string(),
            r#"
canvas c(uv: coord) -> color {
  compose {
    fill(#ffffff)
  }
}
"#
            .to_string(),
        );
        files.insert(
            "engine/pipelines/10_forward.fr".to_string(),
            r#"
pass fwd_base {
  stage: raster
  permutations {
        @known(compile) ambient: "flat"|"sh2"
  }
}
"#
            .to_string(),
        );

        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("quoted axis/permutation literal values should compile");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn imported_const_and_f32_cast_compile() {
        let mut files = HashMap::new();
        files.insert(
            "main.fr".to_string(),
            r#"
import "lib.fr"

fn mixed_math() -> vec4 {
    let taps = 4u
  let denom = f32(taps - 1u)
  let p = vec3(1.0, 2.0, 3.0)
  return vec4(p, PI / denom)
       + vec4(denom, p)
       + vec4(vec2(5.0, 6.0), vec2(7.0, 8.0))
}
"#
            .to_string(),
        );
        files.insert(
            "lib.fr".to_string(),
            r#"
const f32 PI = 3.14159265
"#
            .to_string(),
        );

        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("imported top-level consts and f32 casts should compile");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn local_const_in_function_compiles_when_foldable() {
        let src = r#"
fn folded() -> vec4 {
    const f32 PI2 = 3.14159265 * 2.0
    const vec2 OFF = vec2(1.0 + 2.0, 3.0 * 4.0)
  return vec4(OFF, PI2, 1.0)
}
"#;

        let bundle = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("local const declarations should compile when compile-time foldable");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn local_const_in_function_rejects_runtime_initializer() {
        let src = r#"
fn bad(x: f32) -> f32 {
    const f32 K = x + 1.0
  return K
}

canvas c(uv: coord, time: signal) -> color {
    let k = bad(time)
    let _keep = k
    compose {
        fill(#ffffff)
    }
}
"#;

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("runtime-dependent local const initializer should fail");
        assert!(
            err.iter().any(|diag| {
                diag.message
                    .contains("const `K` initializer must be compile-time constant")
            }),
            "expected compile-time const diagnostic, got: {err:#?}"
        );
    }

    #[test]
    fn local_const_scope_in_nested_block_and_for_loop() {
        let src = r#"
fn scoped(x: f32) -> f32 {
    {
        const f32 OFFSET = 2.0
        let scoped = x + OFFSET
        let keep_scoped = scoped
    }

    for i in 0 .. 4 {
        const f32 STEP = 0.25
        let keep_loop = STEP + f32(i) * 0.0
    }

    return x
}

canvas c(uv: coord, time: signal) -> color {
    let keep = scoped(time)
    compose {
        fill(#ffffff)
    }
}
"#;

        let bundle = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect("nested-block and for-loop local const scope should compile");
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn local_const_declared_in_for_loop_does_not_leak_scope() {
        let src = r#"
fn bad_scope() -> f32 {
    for i in 0 .. 2 {
        const f32 STEP = f32(i) * 0.5
    }
    return STEP
}

canvas c(uv: coord) -> color {
    let v = bad_scope()
    let keep = v
    compose {
        fill(#ffffff)
    }
}
"#;

        let err = compile_source_bundle(src, TEST_SOURCE_PATH, false)
            .expect_err("for-loop local const must remain loop-scoped");
        assert!(
            err.iter()
                .any(|diag| diag.message.contains("unknown name `STEP`")),
            "expected unknown-name diagnostic, got: {err:#?}"
        );
    }

    #[test]
    fn engine_include_style_const_chain_compiles_with_glsl_const_syntax() {
        let mut files = HashMap::new();
        files.insert(
            "main.fr".to_string(),
            r#"
import "engine/core/02_functions.fr"

canvas c(uv: coord, time: signal) -> color {
    let w = engine_weight(4u)
    let keep = w + time * 0.0
    compose {
        fill(#ffffff)
    }
}
"#
            .to_string(),
        );
        files.insert(
            "engine/core/02_functions.fr".to_string(),
            r#"
const f32 PI = 3.14159265
const f32 INV_PI = 0.31830989

fn engine_weight(taps: u32) -> f32 {
    let denom = f32(taps - 1u)
    return PI * INV_PI / max(denom, 1.0)
}
"#
            .to_string(),
        );

        let bundle = compile_bundle_virtual(&files, "main.fr", false)
            .expect("engine-style include chain consts should compile");
        assert!(bundle.diagnostics.is_empty());
    }
}

pub fn expected_tokens_at_cursor(src: &str, cursor: usize) -> Vec<String> {
    pipeline::expected_tokens_at_cursor(src, cursor)
}

fn flatten_file_diags(file_diags: Vec<pipeline::FileDiagnostics>) -> Vec<DiagnosticRecord> {
    let mut out = Vec::new();
    for fd in file_diags {
        for d in fd.diags {
            out.push(DiagnosticRecord {
                file: fd.filename.clone(),
                severity: d.severity.as_str().to_string(),
                message: d.message,
                label: d.label,
                help: d.help,
                span_start: d.span.start,
                span_end: d.span.end,
            });
        }
    }
    out
}

fn emit_failure(filename: &str, err: emit::EmitError) -> DiagnosticRecord {
    DiagnosticRecord {
        file: filename.to_string(),
        severity: diag::Severity::Error.as_str().to_string(),
        message: err.to_string(),
        label: None,
        help: None,
        span_start: 0,
        span_end: 0,
    }
}

fn visualizer_failure(filename: &str, message: String) -> DiagnosticRecord {
    DiagnosticRecord {
        file: filename.to_string(),
        severity: diag::Severity::Error.as_str().to_string(),
        message,
        label: None,
        help: None,
        span_start: 0,
        span_end: 0,
    }
}

pub fn run(options: Options) -> ExitCode {
    init_trace_subscriber_once(options.verbose, options.trace_verbosity);

    let src = match std::fs::read_to_string(&options.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", options.input.display());
            finalize_trace_output();
            return ExitCode::FAILURE;
        }
    };
    let filename = options.input.display().to_string();

    let compiled = match pipeline::compile_with_engine_dir(
        &src,
        &filename,
        true,
        &options.context,
        options.engine_dir.as_deref(),
    ) {
        Ok(compiled) => compiled,
        Err(file_diags) => {
            for fd in &file_diags {
                diag::emit_all(&fd.filename, &fd.src, &fd.diags);
            }
            finalize_trace_output();
            return ExitCode::FAILURE;
        }
    };

    for fd in &compiled.diagnostics {
        if !fd.diags.is_empty() {
            diag::emit_all(&fd.filename, &fd.src, &fd.diags);
        }
    }

    let out = match emit::render(
        options.emit,
        options.context.build_profile,
        &compiled.hirs,
        &compiled.material_hirs,
        &compiled.compute_library,
        &compiled.structs,
        &compiled.passes,
        &compiled.pipelines,
        &compiled.vertex_interfaces,
        &compiled.vertex_formats,
        &compiled.vertex_factories,
        &compiled.module,
        &compiled.info,
        &compiled.tex_bindings,
        &compiled.pass_target_bindings,
        &compiled.path_bindings,
        &compiled.param_bindings,
        &compiled.global_uniform_bindings,
        compiled.resource_layout,
    ) {
        Ok(out) => out,
        Err(message) => {
            eprintln!("{message}");
            finalize_trace_output();
            return ExitCode::FAILURE;
        }
    };

    if options.explain {
        explain::emit(
            &compiled.hirs,
            &compiled.stats,
            Some(explain::EmitForStats {
                text: &out,
                format_label: options.emit.as_str(),
            }),
        );
    }

    match &options.output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, out) {
                eprintln!("error: cannot write {}: {e}", path.display());
                finalize_trace_output();
                return ExitCode::FAILURE;
            }
            eprintln!("wrote {}", path.display());
        }
        None => print!("{out}"),
    }

    if options.timings {
        let t = &compiled.timings;
        eprintln!("\ncompilation timings:");
        eprintln!("  lex              {:>8.3} ms", t.lex_ms);
        eprintln!("  parse            {:>8.3} ms", t.parse_ms);
        eprintln!("  resolve imports  {:>8.3} ms", t.resolve_imports_ms);
        eprintln!("  check            {:>8.3} ms", t.check_ms);
        eprintln!("  rewrite          {:>8.3} ms", t.rewrite_ms);
        eprintln!("  check + rewrite  {:>8.3} ms", t.check_rewrite_ms);
        eprintln!("  lower + validate {:>8.3} ms", t.lower_validate_ms);
        eprintln!("  map diagnostics  {:>8.3} ms", t.map_diagnostics_ms);
        eprintln!("  ─────────────────────────────");
        eprintln!("  total pipeline   {:>8.3} ms", t.total_ms);
    }

    finalize_trace_output();
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests_canvas_variant {
    use super::*;

    fn query_span_info(
        src: &str,
        filename: &str,
        start: usize,
        end: usize,
    ) -> Option<SpanQueryResult> {
        query_span_info_with_files(
            src,
            filename,
            start,
            end,
            Some(&crate::test_support::files(src, filename)),
        )
    }

    fn compile_variant(
        src: &str,
        filename: &str,
        start: usize,
        end: usize,
    ) -> Result<VariantCompileOutput, Vec<DiagnosticRecord>> {
        compile_variant_with_files(
            src,
            filename,
            start,
            end,
            Some(&crate::test_support::files(src, filename)),
        )
    }

    fn render_compiled_wgsl(compiled: &pipeline::CompiledProgram) -> String {
        emit::render(
            EmitTarget::Wgsl,
            BuildProfile::All,
            &compiled.hirs,
            &compiled.material_hirs,
            &compiled.compute_library,
            &compiled.structs,
            &compiled.passes,
            &compiled.pipelines,
            &compiled.vertex_interfaces,
            &compiled.vertex_formats,
            &compiled.vertex_factories,
            &compiled.module,
            &compiled.info,
            &compiled.tex_bindings,
            &compiled.pass_target_bindings,
            &compiled.path_bindings,
            &compiled.param_bindings,
            &compiled.global_uniform_bindings,
            compiled.resource_layout,
        )
        .expect("variant WGSL should emit")
    }

    #[test]
    fn compile_variant_matches_resolved_span_capture() {
        let src = r#"
canvas shape_variant_alignment(uv: coord, time: signal) -> color {
    space stage = centered(aspect: preserve)

    let frame_outer = box(at: (0.5, 0.5), size: (0.64, 0.42)) |> round(0.08)
    let frame_inner = box(at: (0.5, 0.5), size: (0.52, 0.30)) |> round(0.06)
    let frame = frame_outer - frame_inner

    let orbit_x = wave(period: 4s, shape: sine, range: 0.28 .. 0.72)
    let orb = circle(at: (orbit_x, 0.5), radius: 0.06)

    compose {
        fill(#0d1424)

        in space stage {
            frame |> fill(#60a5fa)
            (orb & frame_outer) |> fill(#f472b6)
        }
    }
}
"#;

        let marker = "orb & frame_outer";
        let marker_start = src
            .find(marker)
            .expect("marker should exist in test source");
        let query_start = marker_start + 2;
        let query_end = query_start + 4;

        let span_info = query_span_info(src, TEST_SOURCE_PATH, query_start, query_end)
            .expect("query_span_info should resolve a capture");
        assert_eq!(span_info.kind.as_str(), "shape");
        assert!(
            span_info.span_start <= query_start && query_end <= span_info.span_end,
            "resolved span must contain the original query"
        );

        let variant = compile_variant(src, TEST_SOURCE_PATH, query_start, query_end)
            .expect("compile_variant succeeds");
        assert_eq!(variant.semantic_type, span_info.kind.as_str());

        let context = CompileContext::default();
        let compiled_resolved = pipeline::compile_variant_at_span(
            src,
            TEST_SOURCE_PATH,
            true,
            &context,
            (span_info.span_start, span_info.span_end),
            Some(&crate::test_support::files(src, TEST_SOURCE_PATH)),
        )
        .expect("resolved span compile should succeed");
        let resolved_wgsl = render_compiled_wgsl(&compiled_resolved);

        assert_eq!(
            variant.wgsl, resolved_wgsl,
            "compile_variant should compile the resolved capture span"
        );
    }

    #[test]
    fn compile_variant_supports_space_capture() {
        let src = r#"
canvas space_variant_preview(uv: coord, time: signal) -> color {
    space stage = centered(aspect: preserve)

    let card = box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> round(0.04uv)

    compose {
        fill(#08111f)

        in space stage {
            card |> fill(#9dd6ff)
        }
    }
}
"#;

        let marker = "centered(aspect: preserve)";
        let marker_start = src.find(marker).expect("space marker should exist");
        let query_start = marker_start + 2;
        let query_end = query_start + 8;

        let span_info = query_span_info(src, TEST_SOURCE_PATH, query_start, query_end)
            .expect("query_span_info should resolve a space capture");
        assert_eq!(span_info.kind.as_str(), "space");

        let variant = compile_variant(src, TEST_SOURCE_PATH, query_start, query_end)
            .expect("space compile_variant succeeds");
        assert_eq!(variant.semantic_type, "space");
        assert!(variant.wgsl.contains("fn fresco_space_variant_preview"));
    }

    #[test]
    fn compile_variant_supports_piped_shape_capture() {
        let src = r#"
canvas piped_shape_preview(uv: coord, time: signal) -> color {
    let card = box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> round(0.04uv)

    compose {
        card |> fill(#ff66aa)
    }
}
"#;

        let marker = "box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> round(0.04uv)";
        let marker_start = src.find(marker).expect("piped shape marker should exist");
        let query_start = marker_start + 8;
        let query_end = marker_start + marker.len() - 2;

        let span_info = query_span_info(src, TEST_SOURCE_PATH, query_start, query_end)
            .expect("query_span_info should resolve a piped shape capture");
        assert_eq!(span_info.kind.as_str(), "shape");
        assert!(span_info.span_start <= query_start && query_end <= span_info.span_end);

        let variant = compile_variant(src, TEST_SOURCE_PATH, query_start, query_end)
            .expect("piped shape compile_variant succeeds");
        assert_eq!(variant.semantic_type, "shape");
        assert!(variant.wgsl.contains("fn fresco_piped_shape_preview"));
    }

    #[test]
    fn compile_variant_reports_missing_root_entry_point_generically() {
        let src = r#"
fn make_color() -> color {
  color(1.0, 0.0, 0.6)
}
"#;

        let err = compile_variant(src, TEST_SOURCE_PATH, 0, 4)
            .expect_err("library source without root entries should not compile a variant");
        assert!(err.iter().any(|diag| {
            diag.message
                .contains("workbook query requires at least one root entry point")
        }));
    }

    #[test]
    fn compile_variant_reports_surface_root_entries_explicitly() {
        let src = r#"
surface demo(sp: surf) -> material {
  compose {
    base(albedo: #fff)
  }
}
"#;

        let err = compile_variant(src, TEST_SOURCE_PATH, 0, 4)
            .expect_err("surface root entries should remain unsupported for variants");
        assert!(err.iter().any(|diag| {
            diag.message
                .contains("workbook preview requires a previewable root entry")
        }));
        assert!(
            err.iter()
                .any(|diag| diag.message.contains("surface `demo`"))
        );
    }
}
