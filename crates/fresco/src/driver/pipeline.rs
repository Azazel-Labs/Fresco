use crate::ast::{PragmaValue, Program, Span};
use crate::check;
use crate::diag::Diag;
use crate::hir::{Blend, Hir, Layer, Shape, ShapeAaStyle, Sx, Xform};
use crate::lexer;
use crate::lower;
use crate::material_hir::MaterialHir;
use crate::parser;
use crate::rewrite;
use chumsky::Parser as _;
use logos::Logos;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use tracing::{info, info_span};
use web_time::Instant;

mod engine_policy;
mod entry_highlighting;
mod material_tooling;
pub use entry_highlighting::engine_keyword_spans;
pub use material_tooling::material_callables;
mod loaders;
use loaders::{EmbeddedEngineLoader, FsModuleLoader, MemModuleLoader, ModuleLoader};

#[derive(Debug, Clone)]
pub struct FileDiagnostics {
    pub filename: String,
    pub src: String,
    pub diags: Vec<Diag>,
}

struct ResolvedProgram {
    program: Program,
    sources: HashMap<String, String>,
    engine_pragmas: Vec<(String, Vec<crate::ast::PragmaDecl>)>,
}

struct PreparedWorkbookInput {
    resolved: ResolvedProgram,
    effective_context: CompileContext,
}

struct ImportContext {
    root_pragmas: Vec<crate::ast::PragmaDecl>,
    engine_pragmas: Vec<(String, Vec<crate::ast::PragmaDecl>)>,
    visited: HashSet<String>,
    active_stack: Vec<String>,
    sources: HashMap<String, String>,
    out_groups: Vec<crate::ast::ResourceGroupDecl>,
    out_consts: Vec<crate::ast::ConstDecl>,
    out_params: Vec<crate::ast::GlobalParamDecl>,
    out_tags: Vec<crate::ast::TagsDecl>,
    out_axes: Vec<crate::ast::AxisDecl>,
    out_axis_defaults: Vec<crate::ast::AxisDefaultDecl>,
    out_passes: Vec<crate::ast::PassDecl>,
    out_pipelines: Vec<crate::ast::PipelineDecl>,
    out_surfaces: Vec<crate::ast::SurfaceDecl>,
    out_material_properties: Vec<crate::ast::MaterialPropertiesDecl>,
    out_schema_programs: Vec<crate::ast::SchemaProgramDecl>,
    out_vertex_interfaces: Vec<crate::ast::VertexInterfaceDecl>,
    out_vertex_formats: Vec<crate::ast::VertexFormatDecl>,
    out_vertex_factories: Vec<crate::ast::VertexFactoryDecl>,
    out_schema_expressions: Vec<crate::ast::SchemaExpressionDecl>,
    out_schema_evaluators: Vec<crate::ast::SchemaEvaluatorDecl>,
    out_enums: Vec<crate::ast::EnumDecl>,
    out_structs: Vec<crate::ast::StructDecl>,
    out_functions: Vec<crate::ast::FnDecl>,
    out_interfaces: Vec<crate::ast::InterfaceDecl>,
    out_conformances: Vec<crate::ast::ConformanceDecl>,
    out_style_contracts: Vec<crate::ast::StyleContractDecl>,
    out_style_capabilities: Vec<crate::ast::StyleCapabilityDecl>,
    out_style_providers: Vec<crate::ast::StyleProviderDecl>,
    out_styles: Vec<crate::ast::StyleDecl>,
    out_texture_types: Vec<crate::ast::TextureTypeDecl>,
    out_templates: Vec<crate::ast::TemplateDecl>,
    out_effects: Vec<crate::ast::EffectDecl>,
    diags: Vec<FileDiagnostics>,
}

struct AppliedProgramPragmas {
    context: CompileContext,
    explain_feature_labels: BTreeMap<String, String>,
}

impl FileDiagnostics {
    fn one(filename: impl Into<String>, src: impl Into<String>, diags: Vec<Diag>) -> Self {
        Self {
            filename: filename.into(),
            src: src.into(),
            diags,
        }
    }
}

pub struct CompiledProgram {
    pub resource_layout: crate::ast::ResourceLayout,
    pub compute_library: check::compute::ComputeLibrary,
    pub module: naga::Module,
    pub info: naga::valid::ModuleInfo,
    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub consts: Vec<crate::ast::ConstDecl>,
    pub hirs: Vec<Hir>,
    pub material_hirs: Vec<MaterialHir>,
    pub structs: Vec<crate::ast::StructDecl>,
    pub passes: Vec<crate::ast::PassDecl>,
    pub pipelines: Vec<crate::ast::PipelineDecl>,
    pub vertex_interfaces: Vec<crate::ast::VertexInterfaceDecl>,
    pub vertex_formats: Vec<crate::ast::VertexFormatDecl>,
    pub vertex_factories: Vec<crate::ast::VertexFactoryDecl>,
    pub stats: Vec<lower::Stats>,
    pub diagnostics: Vec<FileDiagnostics>,
    pub timings: PipelineTimings,
    /// Texture binding assignments: name → binding index in `@group(1)`.
    /// Sampler is always at `@group(1) @binding(0)`.
    pub tex_bindings: HashMap<String, u32>,
    /// `canvas_name -> (target_id -> binding_index)` for compiler-internal
    /// pass-target textures used by multi-pass replay.
    pub pass_target_bindings: HashMap<String, HashMap<usize, u32>>,
    /// Path buffer binding assignments: `canvas_name:path_index` -> binding
    /// index in `@group(2)` for lowered storage-backed path segment tables.
    pub path_bindings: HashMap<String, u32>,
    /// Dynamic array param binding assignments: `canvas_name:param_name` -> binding
    /// index in `@group(0)` for runtime-sized param storage buffers.
    pub param_bindings: HashMap<String, u32>,
    /// Struct-typed global `param` binding assignments: `global_param_name`
    /// -> binding index in `@group(3)`.
    pub global_uniform_bindings: HashMap<String, u32>,
}

#[derive(Debug, Clone, Default)]
pub struct PipelineTimings {
    pub lex_ms: f64,
    pub parse_ms: f64,
    pub resolve_imports_ms: f64,
    pub check_ms: f64,
    pub rewrite_ms: f64,
    pub check_rewrite_ms: f64,
    pub lower_validate_ms: f64,
    pub map_diagnostics_ms: f64,
    pub total_ms: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct CheckRewriteTimings {
    check_ms: f64,
    rewrite_ms: f64,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum BuildProfile {
    #[default]
    All,
    Editor,
    Runtime,
}

impl BuildProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildProfile::All => "all",
            BuildProfile::Editor => "editor",
            BuildProfile::Runtime => "runtime",
        }
    }

    pub fn includes_editor_only(self) -> bool {
        !matches!(self, BuildProfile::Runtime)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompileContext {
    pub renderer: Option<String>,
    pub property_overrides: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    pub check: check::CheckOptions,
    pub build_profile: BuildProfile,
}

pub fn compile_with_options(
    src: &str,
    filename: &str,
    include_stdlib: bool,
    context: &CompileContext,
) -> Result<CompiledProgram, Vec<FileDiagnostics>> {
    compile_with_engine_dir(src, filename, include_stdlib, context, None)
}

pub fn compile_with_engine_dir(
    src: &str,
    filename: &str,
    include_stdlib: bool,
    context: &CompileContext,
    engine_dir: Option<&std::path::Path>,
) -> Result<CompiledProgram, Vec<FileDiagnostics>> {
    let loader = FsModuleLoader { engine_dir };
    compile_with_loader(src, filename, include_stdlib, context, &loader)
}

/// Compile a program supplied as an in-memory virtual file system.
/// `files` maps virtual path → source text.  `entrypoint` must be a key
/// that exists in `files`. Any root entries declared there are compiled into
/// the resulting WGSL and manifest; higher layers may choose which one to preview.
pub fn compile_with_virtual_files(
    files: &HashMap<String, String>,
    entrypoint: &str,
    include_stdlib: bool,
    context: &CompileContext,
) -> Result<CompiledProgram, Vec<FileDiagnostics>> {
    let src = files.get(entrypoint).cloned().unwrap_or_default();
    let loader = MemModuleLoader {
        files: files.clone(),
    };
    let mut configured = context.clone();
    if let Some(configuration) = files.get("fresco.config.json") {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Configuration {
            #[serde(default)]
            renderer: Option<String>,
            #[serde(default)]
            property_overrides: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
        }
        let config: Configuration = serde_json::from_str(configuration).map_err(|error| {
            vec![FileDiagnostics::one(
                "fresco.config.json",
                configuration,
                vec![Diag::error(
                    0..configuration.len(),
                    format!("invalid compile configuration: {error}. Select styles with property_overrides.<surface>.<implementation_property> using a symbolic implementation name and settings; see LANGUAGE.md"),
                )],
            )]
        })?;
        if configured.renderer.is_none() {
            configured.renderer = config.renderer;
        }
        for (entry, values) in config.property_overrides {
            configured
                .property_overrides
                .entry(entry)
                .or_default()
                .extend(values);
        }
    }
    compile_with_loader(&src, entrypoint, include_stdlib, &configured, &loader)
}

pub fn compile_with_engine_files(
    src: &str,
    filename: &str,
    context: &CompileContext,
    engine_files: &HashMap<String, String>,
    engine_entrypoint: &str,
) -> Result<CompiledProgram, Vec<FileDiagnostics>> {
    let loader = EmbeddedEngineLoader {
        engine: MemModuleLoader {
            files: engine_files.clone(),
        },
        entrypoint: engine_entrypoint.into(),
    };
    compile_with_loader(src, filename, true, context, &loader)
}

fn compile_with_loader(
    src: &str,
    filename: &str,
    include_stdlib: bool,
    context: &CompileContext,
    loader: &dyn ModuleLoader,
) -> Result<CompiledProgram, Vec<FileDiagnostics>> {
    info!(filename = %filename, include_stdlib = include_stdlib, "compile started");
    let _compile_span = info_span!(
        "compile_with_options",
        filename = %filename,
        include_stdlib = include_stdlib
    )
    .entered();
    let total_start = Instant::now();

    let _lex_span = info_span!("lex").entered();
    info!("phase start: lex");
    let lex_start = Instant::now();
    let pre_src =
        preprocess_source(src).map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    let tokens = lex(&pre_src).map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    let lex_ms = elapsed_ms(lex_start);
    info!(ms = lex_ms, tokens = tokens.len(), "phase done: lex");
    drop(_lex_span);

    let _parse_span = info_span!("parse").entered();
    info!("phase start: parse");
    let parse_start = Instant::now();
    let program = parse(&pre_src, &tokens, filename, true)
        .map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    let parse_ms = elapsed_ms(parse_start);
    info!(
        ms = parse_ms,
        canvases = program.canvases.len(),
        functions = program.functions.len(),
        "phase done: parse"
    );
    drop(_parse_span);

    let root_program = Program {
        pragmas: program.pragmas.clone(),
        ..Program::default()
    };

    let _resolve_span = info_span!("resolve_imports").entered();
    info!("phase start: resolve_imports");
    let resolve_start = Instant::now();
    let entry_surfaces = program
        .surfaces
        .iter()
        .map(|surface| surface.name.clone())
        .collect();
    let mut resolved = resolve_imports(program, filename, &pre_src, include_stdlib, loader)?;
    super::operation_composition::reject_legacy_syntax(&resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    let engine_context = engine_policy::apply(context, &resolved, filename, src)?;
    let applied_pragmas =
        apply_program_pragmas(&engine_context, &root_program, filename, src, true)?;
    let effective_context = applied_pragmas.context;
    super::renderers::select(&mut resolved.program, effective_context.renderer.as_deref())
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    check::axes::resolve(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::prepared_geometry::collect(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::styles::lower(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::surface_properties::resolve_with_overrides(
        &mut resolved.program,
        &effective_context.property_overrides,
        Some(&entry_surfaces),
    )
    .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::entry_contract::resolve(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::implementations::prepare(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::style_graph::prepare(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::style_operations::prepare(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    super::operation_composition::prepare(&mut resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    let resolve_imports_ms = elapsed_ms(resolve_start);
    info!(
        ms = resolve_imports_ms,
        sources = resolved.sources.len(),
        "phase done: resolve_imports"
    );
    drop(_resolve_span);

    let _check_rewrite_span = info_span!("check_and_rewrite").entered();
    info!("phase start: check_and_rewrite");
    let check_rewrite_start = Instant::now();
    let (hirs, material_hirs, non_error_diags, check_rewrite_timings) =
        check_and_rewrite(&resolved.program, &effective_context)
            .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    let check_rewrite_ms = elapsed_ms(check_rewrite_start);
    info!(
        ms = check_rewrite_ms,
        hirs = hirs.len(),
        material_hirs = material_hirs.len(),
        check_ms = check_rewrite_timings.check_ms,
        rewrite_ms = check_rewrite_timings.rewrite_ms,
        non_error_diags = non_error_diags.len(),
        "phase done: check_and_rewrite"
    );
    drop(_check_rewrite_span);

    let _lower_validate_span = info_span!("lower_and_validate").entered();
    info!("phase start: lower_and_validate");
    let lower_validate_start = Instant::now();
    let mut compiled = lower_and_validate(LowerInputs {
        resource_layout: resolved.program.resource_layout,
        compute_library: check::compute::ComputeLibrary::new(
            &resolved.program,
            effective_context.check,
        ),
        hirs,
        material_hirs,
        structs: resolved.program.structs.clone(),
        consts: resolved.program.consts.clone(),
        passes: resolved.program.passes.clone(),
        pipelines: resolved.program.pipelines.clone(),
        vertex_interfaces: resolved.program.vertex_interfaces.clone(),
        vertex_formats: resolved.program.vertex_formats.clone(),
        vertex_factories: resolved.program.vertex_factories.clone(),
    })
    .map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    if !applied_pragmas.explain_feature_labels.is_empty() {
        for hir in &mut compiled.hirs {
            for (feature, label) in &applied_pragmas.explain_feature_labels {
                hir.notes
                    .push(format!("explain: feature_label {feature}={label}"));
            }
        }
        for hir in &mut compiled.material_hirs {
            for (feature, label) in &applied_pragmas.explain_feature_labels {
                hir.notes
                    .push(format!("explain: feature_label {feature}={label}"));
            }
        }
    }
    let lower_validate_ms = elapsed_ms(lower_validate_start);
    info!(
        ms = lower_validate_ms,
        stats = compiled.stats.len(),
        tex_bindings = compiled.tex_bindings.len(),
        "phase done: lower_and_validate"
    );
    drop(_lower_validate_span);

    let _map_diags_span = info_span!("map_file_diags").entered();
    info!("phase start: map_file_diags");
    let map_diags_start = Instant::now();
    compiled.diagnostics = map_file_diags(non_error_diags, &resolved.sources, filename, src);
    let map_diagnostics_ms = elapsed_ms(map_diags_start);
    info!(
        ms = map_diagnostics_ms,
        diagnostic_files = compiled.diagnostics.len(),
        "phase done: map_file_diags"
    );
    drop(_map_diags_span);
    compiled.timings = PipelineTimings {
        lex_ms,
        parse_ms,
        resolve_imports_ms,
        check_ms: check_rewrite_timings.check_ms,
        rewrite_ms: check_rewrite_timings.rewrite_ms,
        check_rewrite_ms,
        lower_validate_ms,
        map_diagnostics_ms,
        total_ms: elapsed_ms(total_start),
    };
    info!(ms = compiled.timings.total_ms, "compile finished");
    Ok(compiled)
}

pub fn expected_tokens_at_cursor(src: &str, cursor: usize) -> Vec<String> {
    let mut cursor = cursor.min(src.len());
    while cursor > 0 && !src.is_char_boundary(cursor) {
        cursor -= 1;
    }

    let probe = &src[..cursor];
    let Ok(pre_probe) = preprocess_source(probe) else {
        return Vec::new();
    };
    let Ok(tokens) = lex(&pre_probe) else {
        return Vec::new();
    };

    let eoi = pre_probe.len()..pre_probe.len();
    let (_program, parse_errs) = parser::program()
        .parse(parser::input(&tokens, eoi.clone()))
        .into_output_errors();

    if parse_errs.is_empty() {
        return Vec::new();
    }

    let mut best_distance = usize::MAX;
    let mut bucket = Vec::new();

    for err in &parse_errs {
        let span = err.span();
        let distance = if span.start > eoi.start {
            span.start - eoi.start
        } else {
            eoi.start.saturating_sub(span.end)
        };

        if distance < best_distance {
            best_distance = distance;
            bucket.clear();
            bucket.push(err);
        } else if distance == best_distance {
            bucket.push(err);
        }
    }

    let mut expected = BTreeSet::new();
    for err in bucket {
        for token in err.expected() {
            expected.insert(token.to_string());
        }
    }

    expected.into_iter().collect()
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn map_file_diags(
    diags: Vec<Diag>,
    sources: &HashMap<String, String>,
    default_filename: &str,
    default_src: &str,
) -> Vec<FileDiagnostics> {
    let mut grouped: BTreeMap<String, Vec<Diag>> = BTreeMap::new();
    for mut d in diags {
        let file = d
            .file
            .take()
            .unwrap_or_else(|| default_filename.to_string());
        grouped.entry(file).or_default().push(d);
    }

    grouped
        .into_iter()
        .map(|(file, ds)| {
            let src = sources
                .get(&file)
                .cloned()
                .unwrap_or_else(|| default_src.to_string());
            FileDiagnostics::one(file, src, ds)
        })
        .collect()
}

fn mask_line(line: &str) -> String {
    line.chars()
        .map(|ch| if ch == '\n' || ch == '\r' { ch } else { ' ' })
        .collect()
}

fn macro_expr_value(expr: &str, macros: &HashMap<String, String>) -> i64 {
    let atom = expr.trim();
    if atom.is_empty() {
        return 0;
    }
    if let Ok(v) = atom.parse::<i64>() {
        return v;
    }
    if let Some(v) = macros.get(atom) {
        return v.trim().parse::<i64>().unwrap_or(0);
    }
    0
}

fn parse_if_condition(expr: &str, macros: &HashMap<String, String>) -> bool {
    let raw = expr.trim();
    if raw.is_empty() {
        return false;
    }

    if let Some((lhs, rhs)) = raw.split_once("==") {
        return macro_expr_value(lhs.trim(), macros) == macro_expr_value(rhs.trim(), macros);
    }

    if let Some((lhs, rhs)) = raw.split_once("!=") {
        return macro_expr_value(lhs.trim(), macros) != macro_expr_value(rhs.trim(), macros);
    }

    macro_expr_value(raw, macros) != 0
}

fn expand_macros(line: &str, macros: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\n' || ch == '\r' {
            out.push(ch);
            i += 1;
            continue;
        }

        if ch == '_' || ch.is_ascii_alphabetic() {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i] == '_' || chars[i].is_ascii_alphanumeric()) {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            if let Some(value) = macros.get(&ident) {
                out.push_str(value);
            } else {
                out.push_str(&ident);
            }
            continue;
        }

        out.push(ch);
        i += 1;
    }
    out
}

fn normalize_trailing_dot_literals(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len() + 8);
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        let prev_is_ident = i > 0 && (chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '_');
        if ch.is_ascii_digit() && !prev_is_ident {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }

            if i < chars.len() && chars[i] == '.' {
                let next = chars.get(i + 1).copied();
                if !matches!(next, Some(d) if d.is_ascii_digit() || d == '.') {
                    for c in &chars[start..i] {
                        out.push(*c);
                    }
                    out.push('.');
                    out.push('0');
                    i += 1;
                    continue;
                }
            }

            for c in &chars[start..i] {
                out.push(*c);
            }
            continue;
        }

        out.push(ch);
        i += 1;
    }

    out
}

fn preprocess_source(src: &str) -> Result<String, Vec<Diag>> {
    #[derive(Clone, Copy)]
    struct IfFrame {
        parent_active: bool,
        condition_true: bool,
        in_else: bool,
    }

    let mut out = String::with_capacity(src.len());
    let mut macros: HashMap<String, String> = HashMap::new();
    let mut stack: Vec<IfFrame> = Vec::new();
    let mut active = true;
    let mut diags = Vec::new();
    let mut offset = 0usize;

    for line in src.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + line.len();
        let span = line_start..line_end;
        offset = line_end;

        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            if active {
                let expanded = expand_macros(line, &macros);
                out.push_str(&normalize_trailing_dot_literals(&expanded));
            } else {
                out.push_str(&mask_line(line));
            }
            continue;
        }

        let directive = trimmed.trim_end_matches(['\r', '\n']);
        let mut parts = directive.split_whitespace();
        let keyword = parts.next().unwrap_or_default();

        match keyword {
            "#define" => {
                if active {
                    let Some(name) = parts.next() else {
                        diags.push(Diag::error(span.clone(), "invalid #define directive"));
                        out.push_str(&mask_line(line));
                        continue;
                    };
                    let value = parts.collect::<Vec<_>>().join(" ");
                    macros.insert(name.to_string(), value);
                }
                out.push_str(&mask_line(line));
            }
            "#if" => {
                let expr = directive.strip_prefix("#if").unwrap_or("").trim();
                let condition_true = if active {
                    parse_if_condition(expr, &macros)
                } else {
                    false
                };
                stack.push(IfFrame {
                    parent_active: active,
                    condition_true,
                    in_else: false,
                });
                active = active && condition_true;
                out.push_str(&mask_line(line));
            }
            "#else" => {
                if let Some(frame) = stack.last_mut() {
                    if frame.in_else {
                        diags.push(Diag::error(span.clone(), "duplicate #else in conditional"));
                    }
                    frame.in_else = true;
                    active = frame.parent_active && !frame.condition_true;
                } else {
                    diags.push(Diag::error(span.clone(), "#else without matching #if"));
                }
                out.push_str(&mask_line(line));
            }
            "#endif" => {
                if let Some(frame) = stack.pop() {
                    active = frame.parent_active;
                } else {
                    diags.push(Diag::error(span.clone(), "#endif without matching #if"));
                }
                out.push_str(&mask_line(line));
            }
            "#pragma" => {
                if active {
                    // Preserve pragma lines for the lexer/parser stage.
                    out.push_str(line);
                } else {
                    out.push_str(&mask_line(line));
                }
            }
            _ => {
                diags.push(
                    Diag::error(
                        span.clone(),
                        format!("unsupported preprocessor directive `{keyword}`"),
                    )
                    .with_help("supported directives: #define, #if, #else, #endif, #pragma"),
                );
                out.push_str(&mask_line(line));
            }
        }
    }

    if !stack.is_empty() {
        diags.push(Diag::error(
            src.len()..src.len(),
            "unterminated preprocessor conditional; missing #endif",
        ));
    }

    if diags.is_empty() {
        Ok(out)
    } else {
        Err(diags)
    }
}

fn lex(src: &str) -> Result<Vec<(lexer::Token, Span)>, Vec<Diag>> {
    let mut tokens = Vec::new();
    let mut lex_diags = Vec::new();
    for (tok, span) in lexer::Token::lexer(src).spanned() {
        match tok {
            Ok(t) => tokens.push((t, span)),
            Err(()) => lex_diags.push(
                Diag::error(span, "unrecognized token")
                    .with_help("valid literals look like: 0.5, 12px, 20deg, 2s, #ff2d78"),
            ),
        }
    }
    if lex_diags.is_empty() {
        Ok(tokens)
    } else {
        Err(lex_diags)
    }
}

fn parse(
    src: &str,
    tokens: &[(lexer::Token, Span)],
    filename: &str,
    _require_entry_point: bool,
) -> Result<Program, Vec<Diag>> {
    let eoi = src.len()..src.len();
    let (program, parse_errs) = parser::program()
        .parse(parser::input(tokens, eoi))
        .into_output_errors();

    if !parse_errs.is_empty() {
        return Err(parse_errs.into_iter().map(perr_to_diag).collect());
    }

    let mut program = program.unwrap_or_default();
    parser::attach_leading_fn_docs(src, &mut program);
    for contract in &mut program.style_contracts {
        contract.source_file = filename.to_string();
    }
    for declaration in &mut program.style_capabilities {
        declaration.source_file = filename.to_string();
    }
    for declaration in &mut program.style_providers {
        declaration.source_file = filename.to_string();
    }
    for style in &mut program.styles {
        style.source_file = filename.to_string();
    }
    for method in program
        .styles
        .iter_mut()
        .flat_map(|s| &mut s.methods)
        .chain(
            program
                .style_contracts
                .iter_mut()
                .flat_map(|c| &mut c.hooks)
                .filter_map(|h| h.default.as_mut()),
        )
    {
        method.source_file = filename.to_string();
    }
    check::bindings::validate(&program)?;

    // Standalone `.fr` files may contain helpers or library functions without
    // declaring a canvas or surface entry point. Host preview tooling may pick
    // one root entry when one exists, but the compiler should not reject such
    // modules just because they are not directly previewable.
    Ok(program)
}

fn apply_program_pragmas(
    base: &CompileContext,
    program: &Program,
    filename: &str,
    src: &str,
    validate: bool,
) -> Result<AppliedProgramPragmas, Vec<FileDiagnostics>> {
    #[cfg_attr(
        target_pointer_width = "64",
        expect(
            clippy::result_large_err,
            reason = "Preserve the complete compiler diagnostic payload at this pipeline boundary."
        )
    )]
    fn pragma_number(pragma: &crate::ast::PragmaDecl) -> Result<f32, Diag> {
        match &pragma.value {
            PragmaValue::Number(v) => Ok(*v as f32),
            PragmaValue::Ident(v) => Err(Diag::error(
                pragma.value_span.clone(),
                format!("pragma `{}` expects a numeric value, got `{v}`", pragma.key),
            )),
        }
    }

    fn parse_aa_style(value: &str) -> Option<ShapeAaStyle> {
        match value {
            "gradient" => Some(ShapeAaStyle::Gradient),
            "fwidth" => Some(ShapeAaStyle::Fwidth),
            "conservative" | "best" => Some(ShapeAaStyle::Conservative),
            _ => None,
        }
    }

    fn parse_aa_level(value: &str) -> Option<(f32, f32)> {
        match value {
            "sharp" | "low" => Some((1.0, 2.0)),
            "balanced" | "medium" => Some((1.5, 3.0)),
            "soft" | "high" => Some((2.0, 4.0)),
            "ultra" | "max" => Some((2.5, 5.5)),
            _ => None,
        }
    }

    fn parse_bool(value: &PragmaValue) -> Option<bool> {
        match value {
            PragmaValue::Ident(v) => match v.to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" | "enable" | "enabled" => Some(true),
                "false" | "off" | "no" | "disable" | "disabled" => Some(false),
                _ => None,
            },
            PragmaValue::Number(v) => {
                if (*v - 0.0).abs() < f64::EPSILON {
                    Some(false)
                } else if (*v - 1.0).abs() < f64::EPSILON {
                    Some(true)
                } else {
                    None
                }
            }
        }
    }

    let mut context = base.clone();
    let mut explain_feature_labels = BTreeMap::new();
    let mut diags = Vec::new();

    for pragma in &program.pragmas {
        match pragma.key.as_str() {
            "check.wide_effect_warn_ratio" => match pragma_number(pragma) {
                Ok(value) => context.check.wide_effect_warn_ratio = value,
                Err(diag) => diags.push(diag),
            },
            "check.wide_effect_note_ratio" => match pragma_number(pragma) {
                Ok(value) => context.check.wide_effect_note_ratio = value,
                Err(diag) => diags.push(diag),
            },
            "check.shape_aa_min_px" => match pragma_number(pragma) {
                Ok(value) => context.check.shape_aa_min_px = Some(value),
                Err(diag) => diags.push(diag),
            },
            "check.shape_aa_max_px" => match pragma_number(pragma) {
                Ok(value) => context.check.shape_aa_max_px = Some(value),
                Err(diag) => diags.push(diag),
            },
            "check.projective_footprint_max_px" => match pragma_number(pragma) {
                Ok(value) => context.check.projective_footprint_max_px = Some(value),
                Err(diag) => diags.push(diag),
            },
            "check.shape_aa_style" => {
                let raw = match &pragma.value {
                    PragmaValue::Ident(v) => v.to_ascii_lowercase(),
                    PragmaValue::Number(v) => {
                        if (*v - 0.0).abs() < f64::EPSILON {
                            "gradient".to_string()
                        } else if (*v - 1.0).abs() < f64::EPSILON {
                            "fwidth".to_string()
                        } else if (*v - 2.0).abs() < f64::EPSILON {
                            "conservative".to_string()
                        } else {
                            String::new()
                        }
                    }
                };
                if let Some(style) = parse_aa_style(raw.as_str()) {
                    context.check.shape_aa_style = Some(style);
                } else {
                    diags.push(
                        Diag::error(
                            pragma.value_span.clone(),
                            "`check.shape_aa_style` must be one of: gradient, fwidth, conservative (or 0, 1, 2)",
                        ),
                    );
                }
            }
            "check.shape_aa_level" => {
                let raw = match &pragma.value {
                    PragmaValue::Ident(v) => v.to_ascii_lowercase(),
                    PragmaValue::Number(v) => {
                        if (*v - 0.0).abs() < f64::EPSILON {
                            "low".to_string()
                        } else if (*v - 1.0).abs() < f64::EPSILON {
                            "medium".to_string()
                        } else if (*v - 2.0).abs() < f64::EPSILON {
                            "high".to_string()
                        } else if (*v - 3.0).abs() < f64::EPSILON {
                            "max".to_string()
                        } else {
                            String::new()
                        }
                    }
                };
                if let Some((min_px, max_px)) = parse_aa_level(raw.as_str()) {
                    context.check.shape_aa_min_px = Some(min_px);
                    context.check.shape_aa_max_px = Some(max_px);
                } else {
                    diags.push(
                        Diag::error(
                            pragma.value_span.clone(),
                            "`check.shape_aa_level` must be one of: low, medium, high, max (aliases: sharp, balanced, soft, ultra; or 0..3)",
                        ),
                    );
                }
            }
            "check.allow_implicit_texture_uv" => {
                if let Some(value) = parse_bool(&pragma.value) {
                    context.check.allow_implicit_texture_uv = value;
                } else {
                    diags.push(
                        Diag::error(
                            pragma.value_span.clone(),
                            "`check.allow_implicit_texture_uv` must be true/false (or 1/0)",
                        )
                        .with_help("example: `#pragma check.allow_implicit_texture_uv = true`"),
                    );
                }
            }
            "check.warn_implicit_texture_uv" => {
                if let Some(value) = parse_bool(&pragma.value) {
                    context.check.warn_implicit_texture_uv = value;
                } else {
                    diags.push(
                        Diag::error(
                            pragma.value_span.clone(),
                            "`check.warn_implicit_texture_uv` must be true/false (or 1/0)",
                        )
                        .with_help("example: `#pragma check.warn_implicit_texture_uv = true`"),
                    );
                }
            }
            _ if pragma.key.starts_with("explain.label.")
                || pragma.key.starts_with("explain.tag.") =>
            {
                let feature = pragma
                    .key
                    .strip_prefix("explain.label.")
                    .or_else(|| pragma.key.strip_prefix("explain.tag."))
                    .unwrap_or_default();
                if feature.is_empty() {
                    diags.push(
                        Diag::error(
                            pragma.key_span.clone(),
                            "explain label pragma must target a feature key",
                        )
                        .with_help(
                            "use `#pragma explain.label.<feature> = <label>` (example: `#pragma explain.label.motion_blur = camera_smear`)",
                        ),
                    );
                    continue;
                }

                match &pragma.value {
                    PragmaValue::Ident(v) => {
                        explain_feature_labels.insert(feature.to_string(), v.clone());
                    }
                    PragmaValue::Number(_) => {
                        diags.push(
                            Diag::error(
                                pragma.value_span.clone(),
                                format!(
                                    "pragma `{}` expects an identifier label value",
                                    pragma.key
                                ),
                            )
                            .with_help("use an identifier label like `camera_smear` or `edge_aa`"),
                        );
                    }
                }
            }
            _ => {
                diags.push(
                    Diag::error(
                        pragma.key_span.clone(),
                        format!("unknown pragma `{}`", pragma.key),
                    )
                    .with_help(
                        "supported pragmas: check.wide_effect_warn_ratio, check.wide_effect_note_ratio, check.shape_aa_min_px, check.shape_aa_max_px, check.shape_aa_style, check.shape_aa_level, check.projective_footprint_max_px, check.allow_implicit_texture_uv, check.warn_implicit_texture_uv, explain.label.<feature>, explain.tag.<feature>",
                    ),
                );
            }
        }
    }

    if validate
        && diags.is_empty()
        && let Err(message) = context.validate()
    {
        let span = program
            .pragmas
            .last()
            .map(|p| p.span.clone())
            .unwrap_or(0..0);
        diags.push(Diag::error(
            span,
            format!("invalid pragma configuration: {message}"),
        ));
    }

    if diags.is_empty() {
        Ok(AppliedProgramPragmas {
            context,
            explain_feature_labels,
        })
    } else {
        Err(vec![FileDiagnostics::one(filename, src, diags)])
    }
}

fn resolve_imports(
    root: Program,
    filename: &str,
    root_src: &str,
    // The stdlib prelude now ships as an ordinary engine module
    // supplied by the selected engine and merged via the loader below,
    // so this flag is no longer consulted
    // here. Kept on the signature to avoid a wider public API churn across
    // `driver.rs`/`fresco-wasm` call sites; see `stdlib.rs` for context.
    _include_stdlib: bool,
    loader: &dyn ModuleLoader,
) -> Result<ResolvedProgram, Vec<FileDiagnostics>> {
    let mut merged = Program {
        groups: root.groups.clone(),
        authored_entries: root.authored_entries.clone(),
        consts: root.consts.clone(),
        enums: root.enums.clone(),
        structs: root.structs.clone(),
        params: root.params.clone(),
        tags: root.tags.clone(),
        axes: root.axes.clone(),
        axis_defaults: root.axis_defaults.clone(),
        pipelines: root.pipelines.clone(),
        canvases: root.canvases.clone(),
        surfaces: root.surfaces.clone(),
        material_properties: root.material_properties.clone(),
        schema_programs: root.schema_programs.clone(),
        vertex_interfaces: root.vertex_interfaces.clone(),
        vertex_formats: root.vertex_formats.clone(),
        vertex_factories: root.vertex_factories.clone(),
        schema_expressions: root.schema_expressions.clone(),
        schema_evaluators: root.schema_evaluators.clone(),
        texture_types: root.texture_types.clone(),
        templates: root.templates.clone(),
        interfaces: root.interfaces.clone(),
        conformances: root.conformances.clone(),
        style_contracts: root.style_contracts.clone(),
        style_capabilities: root.style_capabilities.clone(),
        style_providers: root.style_providers.clone(),
        styles: root.styles.clone(),
        effects: root.effects.clone(),
        ..Program::default()
    };

    let root_canonical = loader.root_canonical_key(filename);

    let (implicit_engine_modules, root_is_engine) =
        load_implicit_engine_modules(loader, &root_canonical)?;
    for module in &implicit_engine_modules {
        merged.groups.extend(module.program.groups.clone());
        merged.enums.extend(module.program.enums.clone());
        merged.structs.extend(module.program.structs.clone());
        merged.consts.extend(module.program.consts.clone());
        merged.params.extend(module.program.params.clone());
        merged.tags.extend(module.program.tags.clone());
        merged.axes.extend(module.program.axes.clone());
        merged
            .axis_defaults
            .extend(module.program.axis_defaults.clone());
        for mut pass in module.program.passes.clone() {
            pass.source_file.clone_from(&module.canonical);
            merged.passes.push(pass);
        }
        merged.pipelines.extend(module.program.pipelines.clone());
        merged.surfaces.extend(module.program.surfaces.clone());
        merged
            .material_properties
            .extend(module.program.material_properties.clone());
        merged
            .schema_programs
            .extend(module.program.schema_programs.clone());
        merged
            .vertex_interfaces
            .extend(module.program.vertex_interfaces.clone());
        merged
            .vertex_formats
            .extend(module.program.vertex_formats.clone());
        merged
            .vertex_factories
            .extend(module.program.vertex_factories.clone());
        merged
            .schema_expressions
            .extend(module.program.schema_expressions.clone());
        merged
            .schema_evaluators
            .extend(module.program.schema_evaluators.clone());
        merged
            .texture_types
            .extend(module.program.texture_types.clone());
        merged
            .style_contracts
            .extend(module.program.style_contracts.clone());
        merged
            .style_capabilities
            .extend(module.program.style_capabilities.clone());
        merged
            .style_providers
            .extend(module.program.style_providers.clone());
        merged.styles.extend(module.program.styles.clone());
        merged.interfaces.extend(module.program.interfaces.clone());
        merged
            .conformances
            .extend(module.program.conformances.clone());
        merged.effects.extend(module.program.effects.clone());
        merged.templates.extend(module.program.templates.clone());
        for mut f in module.program.functions.clone() {
            f.source_file.clone_from(&module.canonical);
            merged.functions.push(f);
        }
    }

    let mut ctx = ImportContext {
        root_pragmas: root.pragmas.clone(),
        engine_pragmas: if root_is_engine {
            vec![(root_canonical.clone(), root.pragmas.clone())]
        } else {
            Vec::new()
        },
        visited: HashSet::from([root_canonical.clone()]),
        active_stack: vec![root_canonical.clone()],
        sources: HashMap::from([
            (filename.to_string(), root_src.to_string()),
            (root_canonical.clone(), root_src.to_string()),
        ]),
        out_groups: Vec::new(),
        out_consts: Vec::new(),
        out_params: Vec::new(),
        out_tags: Vec::new(),
        out_axes: Vec::new(),
        out_axis_defaults: Vec::new(),
        out_passes: Vec::new(),
        out_pipelines: Vec::new(),
        out_surfaces: Vec::new(),
        out_material_properties: Vec::new(),
        out_schema_programs: Vec::new(),
        out_vertex_interfaces: Vec::new(),
        out_vertex_formats: Vec::new(),
        out_vertex_factories: Vec::new(),
        out_schema_expressions: Vec::new(),
        out_schema_evaluators: Vec::new(),
        out_enums: Vec::new(),
        out_structs: Vec::new(),
        out_functions: Vec::new(),
        out_interfaces: Vec::new(),
        out_conformances: Vec::new(),
        out_style_contracts: Vec::new(),
        out_style_capabilities: Vec::new(),
        out_style_providers: Vec::new(),
        out_styles: Vec::new(),
        out_texture_types: Vec::new(),
        out_templates: Vec::new(),
        out_effects: Vec::new(),
        diags: Vec::new(),
    };

    for module in &implicit_engine_modules {
        if !ctx
            .engine_pragmas
            .iter()
            .any(|(file, _)| file == &module.canonical)
        {
            ctx.engine_pragmas
                .push((module.canonical.clone(), module.program.pragmas.clone()));
        }
        ctx.visited.insert(module.canonical.clone());
        ctx.sources
            .insert(module.canonical.clone(), module.src.clone());
        ctx.active_stack.push(module.canonical.clone());
        load_import_functions(
            &module.program,
            &module.canonical,
            &module.src,
            &mut ctx,
            loader,
            true,
            true,
        );
        ctx.active_stack.pop();
    }

    let root_is_engine = root_is_engine
        || ctx
            .engine_pragmas
            .iter()
            .any(|(file, _)| file == &root_canonical);
    load_import_functions(
        &root,
        &root_canonical,
        root_src,
        &mut ctx,
        loader,
        false,
        root_is_engine,
    );
    if !ctx.diags.is_empty() {
        return Err(ctx.diags);
    }

    merged.groups.append(&mut ctx.out_groups);
    merged.enums.append(&mut ctx.out_enums);
    merged.structs.append(&mut ctx.out_structs);
    merged.consts.append(&mut ctx.out_consts);
    merged.params.append(&mut ctx.out_params);
    merged.tags.append(&mut ctx.out_tags);
    merged.axes.append(&mut ctx.out_axes);
    merged.axis_defaults.append(&mut ctx.out_axis_defaults);
    merged.passes.append(&mut ctx.out_passes);
    for mut pass in root.passes {
        pass.source_file = filename.to_string();
        merged.passes.push(pass);
    }
    merged.pipelines.append(&mut ctx.out_pipelines);
    merged.functions.append(&mut ctx.out_functions);
    merged.surfaces.append(&mut ctx.out_surfaces);
    merged
        .material_properties
        .append(&mut ctx.out_material_properties);
    merged.schema_programs.append(&mut ctx.out_schema_programs);
    merged
        .vertex_interfaces
        .append(&mut ctx.out_vertex_interfaces);
    merged.vertex_formats.append(&mut ctx.out_vertex_formats);
    merged
        .vertex_factories
        .append(&mut ctx.out_vertex_factories);
    merged
        .schema_expressions
        .append(&mut ctx.out_schema_expressions);
    merged
        .schema_evaluators
        .append(&mut ctx.out_schema_evaluators);
    merged.interfaces.append(&mut ctx.out_interfaces);
    merged.conformances.append(&mut ctx.out_conformances);
    merged.style_contracts.append(&mut ctx.out_style_contracts);
    merged
        .style_capabilities
        .append(&mut ctx.out_style_capabilities);
    merged.style_providers.append(&mut ctx.out_style_providers);
    merged.styles.append(&mut ctx.out_styles);
    merged.texture_types.append(&mut ctx.out_texture_types);
    merged.templates.append(&mut ctx.out_templates);
    merged.effects.append(&mut ctx.out_effects);
    for mut f in root.functions {
        f.source_file = filename.to_string();
        merged.functions.push(f);
    }
    super::resource_groups::resolve(&mut merged)
        .map_err(|diags| map_file_diags(diags, &ctx.sources, filename, root_src))?;
    Ok(ResolvedProgram {
        program: merged,
        sources: ctx.sources,
        engine_pragmas: ctx.engine_pragmas,
    })
}

#[derive(Clone)]
struct ImplicitEngineModule {
    canonical: String,
    src: String,
    program: Program,
}

fn load_implicit_engine_modules(
    loader: &dyn ModuleLoader,
    root_canonical: &str,
) -> Result<(Vec<ImplicitEngineModule>, bool), Vec<FileDiagnostics>> {
    let modules = loader
        .implicit_engine_modules(root_canonical)
        .map_err(|msg| {
            vec![FileDiagnostics::one(
                root_canonical,
                "",
                vec![Diag::error(0..0, msg)],
            )]
        })?;
    let root_is_engine = modules
        .iter()
        .any(|(canonical, _)| canonical == root_canonical);
    let modules = modules
        .into_iter()
        // The entry file itself may live inside the discovered `engine/`
        // directory tree (e.g. compiling an engine's `core/01_core.fr`
        // standalone); exclude it here so it isn't merged into itself.
        .filter(|(canonical, _)| canonical != root_canonical)
        .collect::<Vec<_>>();

    let mut out = Vec::with_capacity(modules.len());
    let mut diags = Vec::new();

    for (canonical, src) in modules {
        let pre_src = match preprocess_source(&src) {
            Ok(s) => s,
            Err(ds) => {
                diags.push(FileDiagnostics::one(canonical.clone(), src.as_str(), ds));
                continue;
            }
        };

        let tokens = match lex(&pre_src) {
            Ok(t) => t,
            Err(ds) => {
                diags.push(FileDiagnostics::one(canonical.clone(), src.as_str(), ds));
                continue;
            }
        };

        let program = match parse(&pre_src, &tokens, &canonical, false) {
            Ok(p) => p,
            Err(ds) => {
                diags.push(FileDiagnostics::one(canonical.clone(), src.as_str(), ds));
                continue;
            }
        };

        if !program.canvases.is_empty() || !program.authored_entries.is_empty() {
            diags.push(FileDiagnostics::one(
                canonical.clone(),
                src.as_str(),
                vec![Diag::error(
                    0..0,
                    "implicit engine modules cannot declare canvas entry points",
                )
                .with_help("move canvas definitions to the root source file; engine modules may provide helpers, surfaces, and material models")],
            ));
            continue;
        }

        out.push(ImplicitEngineModule {
            canonical,
            src,
            program,
        });
    }

    if diags.is_empty() {
        Ok((out, root_is_engine))
    } else {
        Err(diags)
    }
}

fn load_import_functions(
    program: &Program,
    current_canonical: &str,
    current_src: &str,
    ctx: &mut ImportContext,
    loader: &dyn ModuleLoader,
    skip_imports_to_visited: bool,
    engine_scope: bool,
) {
    for import in &program.imports {
        let (canonical, src) = match loader.resolve_import(&import.path, current_canonical) {
            Ok(pair) => pair,
            Err(msg) => {
                ctx.diags.push(FileDiagnostics::one(
                    current_canonical,
                    current_src,
                    vec![
                        Diag::error(import.span.clone(), msg)
                            .with_help("use a file path relative to the importing source file"),
                    ],
                ));
                continue;
            }
        };

        if skip_imports_to_visited
            && ctx.visited.contains(&canonical)
            && !ctx
                .active_stack
                .iter()
                .skip(1)
                .any(|path| path == &canonical)
        {
            // A root library may itself be imported by the discovered engine.
            // Its declarations still belong to the engine, without merging it twice.
            if ctx.active_stack.first() == Some(&canonical)
                && !ctx
                    .engine_pragmas
                    .iter()
                    .any(|(file, _)| file == &canonical)
            {
                ctx.engine_pragmas
                    .push((canonical.clone(), ctx.root_pragmas.clone()));
            }
            continue;
        }

        if let Some(cycle_start) = ctx.active_stack.iter().position(|p| p == &canonical) {
            let mut chain: Vec<String> = ctx.active_stack[cycle_start..].to_vec();
            chain.push(canonical.clone());
            ctx.diags.push(FileDiagnostics::one(
                current_canonical,
                current_src,
                vec![
                    Diag::error(
                        import.span.clone(),
                        format!(
                            "cyclic import detected while importing `{}` from `{}`",
                            import.path, current_canonical
                        ),
                    )
                    .with_help(format!("import cycle: {}", chain.join(" -> "))),
                ],
            ));
            continue;
        }

        if ctx.visited.contains(&canonical) {
            continue;
        }
        ctx.visited.insert(canonical.clone());
        ctx.active_stack.push(canonical.clone());
        ctx.sources.insert(canonical.clone(), src.clone());

        let pre_src = match preprocess_source(&src) {
            Ok(s) => s,
            Err(ds) => {
                ctx.diags
                    .push(FileDiagnostics::one(canonical.clone(), src.as_str(), ds));
                ctx.active_stack.pop();
                continue;
            }
        };

        let tokens = match lex(&pre_src) {
            Ok(t) => t,
            Err(ds) => {
                ctx.diags
                    .push(FileDiagnostics::one(canonical.clone(), src.as_str(), ds));
                ctx.active_stack.pop();
                continue;
            }
        };

        let imported = match parse(&pre_src, &tokens, &canonical, false) {
            Ok(p) => p,
            Err(ds) => {
                ctx.diags
                    .push(FileDiagnostics::one(canonical.clone(), src.as_str(), ds));
                ctx.active_stack.pop();
                continue;
            }
        };

        if engine_scope {
            ctx.engine_pragmas
                .push((canonical.clone(), imported.pragmas.clone()));
        }

        if !imported.canvases.is_empty() {
            ctx.diags.push(FileDiagnostics::one(
                current_canonical,
                current_src,
                vec![
                    Diag::error(
                        import.span.clone(),
                        format!(
                            "imported module `{}` contains a canvas definition, \
                         which is not supported in imported modules",
                            import.path
                        ),
                    )
                    .with_help("keep canvas entry points in the root file; imported modules may provide helpers and surfaces"),
                ],
            ));
            ctx.active_stack.pop();
            continue;
        }

        load_import_functions(
            &imported,
            &canonical,
            &src,
            ctx,
            loader,
            skip_imports_to_visited,
            engine_scope,
        );
        if !imported.authored_entries.is_empty() {
            ctx.diags.push(FileDiagnostics::one(
                canonical.clone(),
                src.as_str(),
                vec![Diag::error(
                    imported.authored_entries[0].span.clone(),
                    "authored entries must be declared in the root module",
                )],
            ));
        }
        ctx.out_enums.extend(imported.enums);
        ctx.out_structs.extend(imported.structs);
        ctx.out_consts.extend(imported.consts);
        ctx.out_params.extend(imported.params);
        ctx.out_tags.extend(imported.tags);
        ctx.out_axes.extend(imported.axes);
        ctx.out_axis_defaults.extend(imported.axis_defaults);
        for mut pass in imported.passes {
            pass.source_file.clone_from(&canonical);
            ctx.out_passes.push(pass);
        }
        ctx.out_pipelines.extend(imported.pipelines);
        ctx.out_material_properties
            .extend(imported.material_properties);
        ctx.out_schema_programs.extend(imported.schema_programs);
        ctx.out_vertex_interfaces.extend(imported.vertex_interfaces);
        ctx.out_vertex_formats.extend(imported.vertex_formats);
        ctx.out_groups.extend(imported.groups);
        ctx.out_vertex_factories.extend(imported.vertex_factories);
        ctx.out_schema_expressions
            .extend(imported.schema_expressions);
        ctx.out_schema_evaluators.extend(imported.schema_evaluators);
        ctx.out_texture_types.extend(imported.texture_types);
        ctx.out_templates.extend(imported.templates);
        ctx.out_effects.extend(imported.effects);
        ctx.out_interfaces.extend(imported.interfaces);
        ctx.out_conformances.extend(imported.conformances);
        ctx.out_style_contracts.extend(imported.style_contracts);
        ctx.out_style_capabilities
            .extend(imported.style_capabilities);
        ctx.out_style_providers.extend(imported.style_providers);
        ctx.out_styles.extend(imported.styles);
        ctx.out_surfaces.extend(imported.surfaces);
        for mut f in imported.functions {
            f.source_file.clone_from(&canonical);
            ctx.out_functions.push(f);
        }
        ctx.active_stack.pop();
    }
}

#[expect(
    clippy::type_complexity,
    reason = "The tuple represents the fixed components of a compiler or geometry operation."
)]
fn check_and_rewrite(
    program: &Program,
    context: &CompileContext,
) -> Result<(Vec<Hir>, Vec<MaterialHir>, Vec<Diag>, CheckRewriteTimings), Vec<Diag>> {
    let mut normalized_program = program.clone();
    normalize_root_entry_contract_instances(&mut normalized_program)?;
    check::bindings::validate(&normalized_program)?;
    super::surface_properties::resolve(&mut normalized_program)?;

    check::validate_pipeline_skeleton_decls(&normalized_program)?;

    let mut hirs = Vec::new();
    let mut material_hirs = Vec::new();
    let mut non_error_diags = Vec::new();
    let mut timings = CheckRewriteTimings::default();
    let vertex_contract_diags = check::validate_vertex_contracts(
        &normalized_program.vertex_interfaces,
        &normalized_program.vertex_formats,
        &normalized_program.vertex_factories,
    );
    if !vertex_contract_diags.is_empty() {
        return Err(vertex_contract_diags);
    }

    super::styles::validate_defaults(&normalized_program, &context.check)?;
    check::validate_conformance_method_bodies(
        &normalized_program.functions,
        &normalized_program.consts,
        &normalized_program.enums,
        &normalized_program.structs,
        &normalized_program.params,
        &normalized_program.texture_types,
        &normalized_program.interfaces,
        &normalized_program.conformances,
        &normalized_program.effects,
        &context.check,
    )?;

    let _known_material_properties = check::validate_material_properties(
        &normalized_program.material_properties,
        &normalized_program.functions,
        &normalized_program.consts,
        &normalized_program.enums,
        &normalized_program.structs,
        &normalized_program.params,
        &normalized_program.texture_types,
        &normalized_program.interfaces,
        &normalized_program.conformances,
        &normalized_program.effects,
        &context.check,
    )?;
    let known_material_models = check::validate_material_models(
        &_known_material_properties,
        &normalized_program.material_properties,
        &normalized_program.schema_evaluators,
        &normalized_program.schema_programs,
        &normalized_program.functions,
        &normalized_program.structs,
    )?;
    let known_surface_models = check::validate_surface_model_bindings(
        &_known_material_properties,
        &normalized_program.schema_expressions,
        &normalized_program.functions,
        &normalized_program.structs,
    )?;
    for canvas in &normalized_program.canvases {
        let _canvas_span = info_span!("check_rewrite_canvas", canvas = %canvas.name).entered();
        let check_start = Instant::now();
        let (mut hir, mut diags) = check::check(
            &canvas.as_normalized_root_entry(),
            &normalized_program.functions,
            &normalized_program.consts,
            &normalized_program.enums,
            &normalized_program.structs,
            &normalized_program.params,
            &normalized_program.texture_types,
            &normalized_program.interfaces,
            &normalized_program.conformances,
            &normalized_program.effects,
            &normalized_program.vertex_interfaces,
            &normalized_program.vertex_formats,
            &normalized_program.vertex_factories,
            &context.check,
        )?;
        timings.check_ms += elapsed_ms(check_start);
        let rewrite_start = Instant::now();
        rewrite::run(&mut hir);
        timings.rewrite_ms += elapsed_ms(rewrite_start);
        non_error_diags.append(&mut diags);
        hirs.push(hir);
    }
    for surface in &normalized_program.surfaces {
        let check_start = Instant::now();
        let (hir, mut diags) = check::check_surface(
            surface,
            &normalized_program.passes,
            &normalized_program.pipelines,
            &known_material_models,
            &known_surface_models,
            &normalized_program.functions,
            &normalized_program.consts,
            &normalized_program.enums,
            &normalized_program.structs,
            &normalized_program.params,
            &normalized_program.texture_types,
            &normalized_program.interfaces,
            &normalized_program.conformances,
            &normalized_program.effects,
            &context.check,
        )?;
        timings.check_ms += elapsed_ms(check_start);
        non_error_diags.append(&mut diags);
        material_hirs.push(hir);
    }
    Ok((hirs, material_hirs, non_error_diags, timings))
}

fn normalize_root_entry_contract_instances(program: &mut Program) -> Result<(), Vec<Diag>> {
    fn num_expr(value: f64, span: Span) -> crate::ast::SExpr {
        crate::ast::Spanned {
            node: crate::ast::Expr::Num(value, crate::ast::Unit::None),
            span,
        }
    }

    fn var_expr(name: &str, span: Span) -> crate::ast::SExpr {
        crate::ast::Spanned {
            node: crate::ast::Expr::Var(name.to_string()),
            span,
        }
    }

    fn vec2_zero_expr(span: Span) -> crate::ast::SExpr {
        crate::ast::Spanned {
            node: crate::ast::Expr::Vec2(
                Box::new(num_expr(0.0, span.clone())),
                Box::new(num_expr(0.0, span.clone())),
            ),
            span,
        }
    }

    fn vec3_zero_expr(span: Span) -> crate::ast::SExpr {
        crate::ast::Spanned {
            node: crate::ast::Expr::Vec3(
                Box::new(num_expr(0.0, span.clone())),
                Box::new(num_expr(0.0, span.clone())),
                Box::new(num_expr(0.0, span.clone())),
            ),
            span,
        }
    }

    fn vec4_zero_expr(span: Span) -> crate::ast::SExpr {
        crate::ast::Spanned {
            node: crate::ast::Expr::Vec4(
                Box::new(num_expr(0.0, span.clone())),
                Box::new(num_expr(0.0, span.clone())),
                Box::new(num_expr(0.0, span.clone())),
                Box::new(num_expr(0.0, span.clone())),
            ),
            span,
        }
    }

    #[derive(Clone)]
    struct WrapperInputs {
        primary: String,
        signal: Option<String>,
        resolution: Option<String>,
    }

    #[derive(Default)]
    struct WrapperNeeds {
        signal: bool,
        resolution: bool,
    }

    fn strip_spatial_suffix(ty_name: &str) -> &str {
        let trimmed = ty_name.trim();
        if let Some((base, _)) = trimmed.rsplit_once(" in ")
            && !base.trim().is_empty()
        {
            return base.trim();
        }
        if let Some((base, _)) = trimmed.rsplit_once(" from ")
            && !base.trim().is_empty()
        {
            return base.trim();
        }
        trimmed
    }

    fn synthesize_arg_for_type(
        ty_name: &str,
        field_name_hint: Option<&str>,
        inputs: &WrapperInputs,
        span: Span,
        struct_defs: &HashMap<String, crate::ast::StructDecl>,
        visiting_structs: &mut HashSet<String>,
    ) -> Option<crate::ast::SExpr> {
        let normalized = strip_spatial_suffix(ty_name);
        match normalized {
            "coord" | "coord_like" => Some(var_expr(&inputs.primary, span)),
            "resolution" => Some(match &inputs.resolution {
                Some(name) => var_expr(name, span),
                None => vec2_zero_expr(span),
            }),
            "signal" | "delta" => Some(match &inputs.signal {
                Some(name) => var_expr(name, span),
                None => num_expr(0.0, span),
            }),
            "f32" | "f64" | "half" | "i32" | "u32" | "angle" | "length" | "mask" | "coverage" => {
                Some(num_expr(0.0, span))
            }
            "bool" => Some(var_expr("false", span)),
            "vec2" => {
                let hint = field_name_hint.unwrap_or_default().to_ascii_lowercase();
                if hint.contains("uv") || hint.contains("coord") {
                    Some(var_expr(&inputs.primary, span))
                } else if hint.contains("res") {
                    Some(match &inputs.resolution {
                        Some(name) => var_expr(name, span),
                        None => vec2_zero_expr(span),
                    })
                } else {
                    Some(vec2_zero_expr(span))
                }
            }
            "vec3" => Some(vec3_zero_expr(span)),
            "vec4" => Some(vec4_zero_expr(span)),
            "color" => Some(crate::ast::Spanned {
                node: crate::ast::Expr::Color([0.0, 0.0, 0.0, 1.0]),
                span,
            }),
            "array" => Some(crate::ast::Spanned {
                node: crate::ast::Expr::Array(Vec::new()),
                span,
            }),
            _ => {
                if normalized.starts_with("array<") {
                    return Some(crate::ast::Spanned {
                        node: crate::ast::Expr::Array(Vec::new()),
                        span,
                    });
                }

                let struct_decl = struct_defs.get(normalized)?;
                if !visiting_structs.insert(normalized.to_string()) {
                    return None;
                }

                let mut args = Vec::new();
                for field in &struct_decl.fields {
                    let value = synthesize_arg_for_type(
                        &field.ty_name,
                        Some(&field.name),
                        inputs,
                        span.clone(),
                        struct_defs,
                        visiting_structs,
                    )?;
                    args.push(crate::ast::Arg {
                        name: Some(field.name.clone()),
                        value,
                    });
                }
                visiting_structs.remove(normalized);

                Some(crate::ast::Spanned {
                    node: crate::ast::Expr::Call {
                        name: normalized.to_string(),
                        name_span: span.clone(),
                        const_args: Vec::new(),
                        args,
                    },
                    span,
                })
            }
        }
    }

    fn collect_wrapper_needs(
        ty_name: &str,
        struct_defs: &HashMap<String, crate::ast::StructDecl>,
        visiting_structs: &mut HashSet<String>,
        needs: &mut WrapperNeeds,
    ) {
        let normalized = strip_spatial_suffix(ty_name);
        match normalized {
            "signal" | "delta" => needs.signal = true,
            "resolution" => needs.resolution = true,
            other => {
                let Some(struct_decl) = struct_defs.get(other) else {
                    return;
                };
                if !visiting_structs.insert(other.to_string()) {
                    return;
                }
                for field in &struct_decl.fields {
                    collect_wrapper_needs(&field.ty_name, struct_defs, visiting_structs, needs);
                }
                visiting_structs.remove(other);
            }
        }
    }

    let struct_defs: HashMap<String, crate::ast::StructDecl> = program
        .structs
        .iter()
        .cloned()
        .map(|decl| (decl.name.clone(), decl))
        .collect();
    let interfaces = program.interfaces.clone();

    let mut retained_canvases = Vec::with_capacity(program.canvases.len());
    let mut normalized_conformances = Vec::new();
    let mut diags = Vec::new();

    for canvas in program.canvases.drain(..) {
        let mut methods = Vec::new();
        let mut instance_params = Vec::new();
        let mut saw_unsupported_stmt = false;
        for stmt in &canvas.body {
            match stmt {
                crate::ast::Stmt::LocalFnDecl(method) => methods.push(method.clone()),
                crate::ast::Stmt::Param { .. } => instance_params.push(stmt.clone()),
                _ => saw_unsupported_stmt = true,
            }
        }

        let is_contract_instance = canvas.params.is_empty() && !methods.is_empty();
        if !is_contract_instance {
            retained_canvases.push(canvas);
            continue;
        }

        if saw_unsupported_stmt {
            diags.push(
                Diag::error(
                    canvas.name_span.clone(),
                    format!("contract-style canvas `{}` contains a statement that is neither an instance parameter nor a plug implementation", canvas.name),
                )
                .with_help(
                    "keep only `param ...` and `fn ...` declarations in this block, or use legacy `canvas name(uv: coord) -> color { ... }` entry syntax",
                ),
            );
            continue;
        }

        let matching_interfaces = interfaces
            .iter()
            .filter(|interface| {
                !interface.methods.is_empty()
                    && interface
                        .methods
                        .iter()
                        .all(|required| methods.iter().any(|method| method.name == required.name))
            })
            .collect::<Vec<_>>();
        let selected_interface = interfaces
            .iter()
            .find(|interface| interface.name == "Canvas")
            .or_else(|| matching_interfaces.first().copied());
        let Some(selected_interface) = selected_interface else {
            diags.push(
                Diag::error(
                    canvas.name_span.clone(),
                    format!(
                        "contract-style canvas `{}` does not implement any declared renderer interface",
                        canvas.name
                    ),
                )
                .with_help("implement every required plug from an engine-authored interface"),
            );
            continue;
        };
        let canvas_iface_methods = &selected_interface.methods;

        let contract_entry_method_name = if canvas_iface_methods.len() == 1 {
            canvas_iface_methods[0].name.clone()
        } else if let Some(draw_method) = canvas_iface_methods
            .iter()
            .find(|method| method.name == "draw")
        {
            draw_method.name.clone()
        } else {
            "draw".to_string()
        };

        let Some(draw_method) = methods
            .iter()
            .find(|method| method.name == contract_entry_method_name)
            .cloned()
        else {
            diags.push(
                Diag::error(
                    canvas.name_span.clone(),
                    format!(
                        "contract-style canvas `{}` must define contract entry `fn {}(...)`",
                        canvas.name, contract_entry_method_name
                    ),
                )
                .with_help(
                    "declare the contract entry method in this block (matching `interface canvas` when present) so the entry can lower explicitly",
                ),
            );
            continue;
        };

        let uv_param_name = draw_method
            .params
            .first()
            .map(|param| param.name.clone())
            .unwrap_or_else(|| "input".to_string());
        let uv_param_span = draw_method
            .params
            .first()
            .map(|param| param.name_span.clone())
            .unwrap_or_else(|| draw_method.span.clone());
        let call_span = draw_method.span.clone();

        let primary_ty_name = draw_method
            .params
            .first()
            .map(|param| strip_spatial_suffix(&param.ty_name))
            .unwrap_or("coord");
        let primary_entry_ty = match primary_ty_name {
            "surf" => "surf".to_string(),
            "coord" | "coord_like" | "vec2" => "coord".to_string(),
            _ => {
                diags.push(
                    Diag::error(
                        call_span.clone(),
                        format!(
                            "contract-style canvas `{}` entry method `{}` must start with `coord` or `surf`, found `{}`",
                            canvas.name, draw_method.name, primary_ty_name
                        ),
                    )
                    .with_help(
                        "make the first parameter `uv: coord` (or equivalent) for 2D sampling, or `sp: surf` for surface-style roots",
                    ),
                );
                continue;
            }
        };

        let mut needs = WrapperNeeds::default();
        for param in draw_method.params.iter().skip(1) {
            let mut visiting_structs = HashSet::new();
            collect_wrapper_needs(
                &param.ty_name,
                &struct_defs,
                &mut visiting_structs,
                &mut needs,
            );
        }

        let mut used_param_names = HashSet::new();
        used_param_names.insert(uv_param_name.clone());
        let mut unique_name = |candidate: String| {
            let name = candidate;
            if used_param_names.insert(name.clone()) {
                return name;
            }
            let mut idx = 2usize;
            loop {
                let fallback = format!("{name}_{idx}");
                if used_param_names.insert(fallback.clone()) {
                    return fallback;
                }
                idx += 1;
            }
        };

        let signal_name = if needs.signal {
            let direct = draw_method
                .params
                .iter()
                .skip(1)
                .find(|param| matches!(strip_spatial_suffix(&param.ty_name), "signal" | "delta"))
                .map(|param| param.name.clone())
                .unwrap_or_else(|| format!("{}_signal", uv_param_name));
            Some(unique_name(direct))
        } else {
            None
        };

        let resolution_name = if needs.resolution {
            let direct = draw_method
                .params
                .iter()
                .skip(1)
                .find(|param| strip_spatial_suffix(&param.ty_name) == "resolution")
                .map(|param| param.name.clone())
                .unwrap_or_else(|| format!("{}_resolution", uv_param_name));
            Some(unique_name(direct))
        } else {
            None
        };

        let wrapper_inputs = WrapperInputs {
            primary: uv_param_name.clone(),
            signal: signal_name.clone(),
            resolution: resolution_name.clone(),
        };

        let mut args = vec![crate::ast::Arg {
            name: None,
            value: var_expr(&uv_param_name, call_span.clone()),
        }];
        let mut synth_failed = false;
        for param in draw_method.params.iter().skip(1) {
            let mut visiting_structs = HashSet::new();
            let Some(value) = synthesize_arg_for_type(
                &param.ty_name,
                Some(&param.name),
                &wrapper_inputs,
                call_span.clone(),
                &struct_defs,
                &mut visiting_structs,
            ) else {
                diags.push(
                    Diag::error(
                        param.ty_span.clone(),
                        format!(
                            "contract-style canvas `{}` draw parameter `{}: {}` cannot be synthesized for lowering",
                            canvas.name, param.name, param.ty_name
                        ),
                    )
                    .with_help(
                        "use draw parameter types that can be materialized from entry inputs (coord/signal/resolution/scalars/vectors/colors/structs of those)",
                    ),
                );
                synth_failed = true;
                break;
            };
            args.push(crate::ast::Arg { name: None, value });
        }
        if synth_failed {
            continue;
        }

        normalized_conformances.push(crate::ast::ConformanceDecl {
            type_name: canvas.name.clone(),
            type_name_span: canvas.name_span.clone(),
            interface_name: selected_interface.name.clone(),
            interface_name_span: canvas.name_span.clone(),
            methods,
            body_checked_with_instance: true,
            span: canvas.span,
        });

        let draw_call = crate::ast::Spanned {
            node: crate::ast::Expr::Call {
                name: draw_method.name.clone(),
                name_span: call_span.clone(),
                const_args: Vec::new(),
                args,
            },
            span: call_span.clone(),
        };

        let normalized_root_entry = crate::ast::NormalizedRootEntry {
            entry_kind: canvas.entry_kind.clone(),
            kind: crate::ast::RootEntryKind::Canvas,
            name: canvas.name,
            name_span: canvas.name_span,
            params: {
                let mut params = vec![crate::ast::NormalizedRootEntryParam {
                    name: uv_param_name,
                    name_span: uv_param_span,
                    ty_name: primary_entry_ty,
                    ty_span: call_span.clone(),
                    role: crate::ast::RootEntryParamRole::Primary,
                }];
                if let Some(name) = signal_name {
                    params.push(crate::ast::NormalizedRootEntryParam {
                        name,
                        name_span: call_span.clone(),
                        ty_name: "signal".to_string(),
                        ty_span: call_span.clone(),
                        role: crate::ast::RootEntryParamRole::Signal,
                    });
                }
                if let Some(name) = resolution_name {
                    params.push(crate::ast::NormalizedRootEntryParam {
                        name,
                        name_span: call_span.clone(),
                        ty_name: "resolution".to_string(),
                        ty_span: call_span.clone(),
                        role: crate::ast::RootEntryParamRole::Resolution,
                    });
                }
                params
            },
            material_ty: None,
            body: {
                instance_params.extend([
                    crate::ast::Stmt::LocalFnDecl(draw_method),
                    crate::ast::Stmt::Expr(crate::ast::Spanned {
                        node: crate::ast::Expr::Layer(Box::new(draw_call)),
                        span: call_span.clone(),
                    }),
                ]);
                instance_params
            },
            span: call_span,
        };

        retained_canvases.push(normalized_root_entry.into_canvas());
    }

    program.canvases = retained_canvases;
    program.conformances.extend(normalized_conformances);

    if diags.is_empty() { Ok(()) } else { Err(diags) }
}

/// Workbook result from a single-canvas workbook compile pass.
pub struct WorkbookCompileResult {
    #[allow(
        dead_code,
        reason = "workbook HIR output is retained for future direct workbook consumers"
    )]
    pub hir: Hir,
    pub span_log: Vec<(Span, check::SpanValueKind)>,
    pub captured: Option<check::SpanCapture>,
}

fn prepare_workbook_input(
    src: &str,
    filename: &str,
    include_stdlib: bool,
    context: &CompileContext,
    files: Option<&HashMap<String, String>>,
) -> Result<PreparedWorkbookInput, Vec<FileDiagnostics>> {
    let pre_src =
        preprocess_source(src).map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    let tokens = lex(&pre_src).map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    let program = parse(&pre_src, &tokens, filename, true)
        .map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    let root_program = Program {
        pragmas: program.pragmas.clone(),
        ..Program::default()
    };
    let memory_loader;
    let loader: &dyn ModuleLoader = if let Some(files) = files {
        memory_loader = MemModuleLoader {
            files: files.clone(),
        };
        &memory_loader
    } else {
        &FsModuleLoader::default()
    };
    let resolved = resolve_imports(program, filename, &pre_src, include_stdlib, loader)?;
    super::operation_composition::reject_legacy_syntax(&resolved.program)
        .map_err(|diags| map_file_diags(diags, &resolved.sources, filename, src))?;
    let engine_context = engine_policy::apply(context, &resolved, filename, src)?;
    let applied_pragmas =
        apply_program_pragmas(&engine_context, &root_program, filename, src, true)?;

    Ok(PreparedWorkbookInput {
        resolved,
        effective_context: applied_pragmas.context,
    })
}

fn is_previewable_root_entry(root_entry: &crate::ast::NormalizedRootEntry) -> bool {
    matches!(root_entry.kind, crate::ast::RootEntryKind::Canvas)
}

fn select_workbook_preview_entry(
    program: &Program,
    filename: &str,
    src: &str,
) -> Result<crate::ast::NormalizedRootEntry, Vec<FileDiagnostics>> {
    let mut root_entries = program.root_entries();
    let first_root_entry = root_entries.next().ok_or_else(|| {
        vec![FileDiagnostics::one(
            filename,
            src,
            vec![Diag::error(
                0..0,
                "workbook query requires at least one root entry point",
            )],
        )]
    })?;

    let first_root_entry = first_root_entry.as_normalized_root_entry();

    if is_previewable_root_entry(&first_root_entry) {
        return Ok(first_root_entry);
    }

    if let Some(previewable_entry) = root_entries
        .map(crate::ast::RootEntryRef::as_normalized_root_entry)
        .find(is_previewable_root_entry)
    {
        return Ok(previewable_entry);
    }

    let kind_label = match first_root_entry.kind {
        crate::ast::RootEntryKind::Canvas => "canvas",
        crate::ast::RootEntryKind::Surface => "surface",
    };

    Err(vec![FileDiagnostics::one(
        filename,
        src,
        vec![Diag::error(
            0..0,
            format!(
                "workbook preview requires a previewable root entry; {} `{}` is not yet supported by the current preview host",
                kind_label,
                first_root_entry.name
            ),
        )
        .with_help("surface workbook preview remains a rung-8 feature")],
    )])
}

/// Like `compile_with_options`, but runs the workbook check pass on the first
/// (only) canvas with optional span capture.  Returns an un-lowered HIR so the
/// caller can mutate the root before lowering.
pub fn compile_with_workbook(
    src: &str,
    filename: &str,
    include_stdlib: bool,
    context: &CompileContext,
    capture_range: Option<(usize, usize)>,
    files: Option<&HashMap<String, String>>,
) -> Result<(WorkbookCompileResult, HashMap<String, String>), Vec<FileDiagnostics>> {
    let prepared = prepare_workbook_input(src, filename, include_stdlib, context, files)?;
    let entry = select_workbook_preview_entry(&prepared.resolved.program, filename, src)?;

    let workbook = check::check_with_workbook(
        &entry,
        &prepared.resolved.program.consts,
        &prepared.resolved.program.functions,
        &prepared.resolved.program.enums,
        &prepared.resolved.program.structs,
        &prepared.resolved.program.params,
        &prepared.resolved.program.texture_types,
        &prepared.resolved.program.interfaces,
        &prepared.resolved.program.conformances,
        &prepared.resolved.program.effects,
        &prepared.effective_context.check,
        capture_range,
    )
    .map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;

    Ok((
        WorkbookCompileResult {
            hir: workbook.hir,
            span_log: workbook.span_log,
            captured: workbook.captured,
        },
        prepared.resolved.sources,
    ))
}

/// Compile a variant HIR where the root is replaced by the captured value at
/// `capture_range`.  Returns the lowered, validated program ready for WGSL
/// emission, or an error if no capture matched the range.
pub fn compile_variant_at_span(
    src: &str,
    filename: &str,
    include_stdlib: bool,
    context: &CompileContext,
    capture_range: (usize, usize),
    files: Option<&HashMap<String, String>>,
) -> Result<CompiledProgram, Vec<FileDiagnostics>> {
    let prepared = prepare_workbook_input(src, filename, include_stdlib, context, files)?;
    let entry = select_workbook_preview_entry(&prepared.resolved.program, filename, src)?;
    let mut wb = check::check_with_workbook(
        &entry,
        &prepared.resolved.program.consts,
        &prepared.resolved.program.functions,
        &prepared.resolved.program.enums,
        &prepared.resolved.program.structs,
        &prepared.resolved.program.params,
        &prepared.resolved.program.texture_types,
        &prepared.resolved.program.interfaces,
        &prepared.resolved.program.conformances,
        &prepared.resolved.program.effects,
        &prepared.effective_context.check,
        Some(capture_range),
    )
    .map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;

    let cap = wb.captured.ok_or_else(|| {
        vec![FileDiagnostics::one(
            filename,
            src,
            vec![Diag::error(
                capture_range.0..capture_range.1,
                "no expression found at the given span",
            )],
        )]
    })?;

    // Synthesize a new root layer from the captured value and lower.
    synthesize_variant_root(&mut wb.hir, cap);

    let mut compiled = lower_and_validate(LowerInputs {
        resource_layout: prepared.resolved.program.resource_layout,
        compute_library: check::compute::ComputeLibrary::new(
            &prepared.resolved.program,
            prepared.effective_context.check,
        ),
        hirs: vec![wb.hir],
        material_hirs: Vec::new(),
        structs: prepared.resolved.program.structs.clone(),
        consts: prepared.resolved.program.consts.clone(),
        passes: Vec::new(),
        pipelines: Vec::new(),
        vertex_interfaces: Vec::new(),
        vertex_formats: Vec::new(),
        vertex_factories: Vec::new(),
    })
    .map_err(|diags| vec![FileDiagnostics::one(filename, src, diags)])?;
    compiled.diagnostics = map_file_diags(Vec::new(), &prepared.resolved.sources, filename, src);
    Ok(compiled)
}

/// Mutates `hir.root` so that the canvas renders just the captured value.
fn synthesize_variant_root(hir: &mut Hir, cap: check::SpanCapture) {
    use check::SpanCapture;

    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    let new_root = match cap {
        SpanCapture::Layer(id) => id,
        SpanCapture::Shape(shape_id) => hir.layer(Layer::Fill {
            shape: shape_id,
            color: WHITE,
        }),
        SpanCapture::Color(rgba) => hir.layer(Layer::Solid(rgba)),
        SpanCapture::ColorField(rgba) => {
            let [r, g, b, a] = rgba;
            hir.layer(Layer::ColorExpr { r, g, b, a })
        }
        SpanCapture::Scalar(sx)
        | SpanCapture::Vec2((sx, _))
        | SpanCapture::Vec3((sx, _, _))
        | SpanCapture::Vec4((sx, _, _, _)) => {
            // For scalars/vectors: show x channel as grey.
            hir.layer(Layer::Grey { value: sx })
        }
        SpanCapture::Space(xforms) => build_space_preview_layer(hir, xforms),
    };
    hir.root = new_root;
}

fn build_space_preview_layer(hir: &mut Hir, xforms: Vec<Xform>) -> usize {
    const BG: [f32; 4] = [0.035, 0.07, 0.11, 1.0];
    const GRID_MINOR: [f32; 4] = [0.70, 0.86, 0.98, 0.22];
    const GRID_MAJOR: [f32; 4] = [0.86, 0.95, 1.0, 0.34];
    const FRAME: [f32; 4] = [0.21, 0.35, 0.46, 1.0];
    const AXIS_X: [f32; 4] = [0.98, 0.78, 0.33, 1.0];
    const AXIS_Y: [f32; 4] = [0.22, 0.86, 0.74, 1.0];
    const PROBE_A: [f32; 4] = [0.62, 0.84, 1.0, 1.0];
    const PROBE_B: [f32; 4] = [0.97, 0.47, 0.71, 1.0];
    const PROBE_C: [f32; 4] = [0.93, 0.94, 0.98, 1.0];
    const PROBE_D: [f32; 4] = [0.55, 0.98, 0.65, 1.0];

    let profile = analyze_space_probe_profile(&xforms);

    let background = hir.layer(Layer::Solid(BG));

    let frame = hir.shape(Shape::RBox {
        center: lit2(0.5, 0.5),
        half: lit2(0.35, 0.25),
        round: Sx::Lit(0.06),
    });
    let axis_x = hir.shape(Shape::RBox {
        center: lit2(0.5, 0.5),
        half: lit2(0.31, 0.008),
        round: Sx::Lit(0.008),
    });
    let axis_y = hir.shape(Shape::RBox {
        center: lit2(0.5, 0.5),
        half: lit2(0.008, 0.22),
        round: Sx::Lit(0.008),
    });
    let probe_box = hir.shape(Shape::RBox {
        center: lit2(0.67, 0.61),
        half: lit2(0.10, 0.07),
        round: Sx::Lit(0.03),
    });
    let probe_circle = hir.shape(Shape::Circle {
        center: lit2(0.33, 0.34),
        radius: Sx::Lit(0.08),
    });
    let probe_capsule = hir.shape(Shape::Capsule {
        from: lit2(0.24, 0.76),
        to: lit2(0.76, 0.28),
        radius: Sx::Lit(0.028),
    });
    let corner_marker = hir.shape(Shape::Circle {
        center: lit2(0.18, 0.18),
        radius: Sx::Lit(0.035),
    });
    let top_marker = hir.shape(Shape::Circle {
        center: lit2(0.82, 0.20),
        radius: Sx::Lit(0.028),
    });
    let side_marker = hir.shape(Shape::Circle {
        center: lit2(0.20, 0.80),
        radius: Sx::Lit(0.024),
    });

    let frame_layer = hir.layer(Layer::Fill {
        shape: frame,
        color: FRAME,
    });
    let axis_x_layer = hir.layer(Layer::Fill {
        shape: axis_x,
        color: AXIS_X,
    });
    let axis_y_layer = hir.layer(Layer::Fill {
        shape: axis_y,
        color: AXIS_Y,
    });
    let probe_box_layer = hir.layer(Layer::Fill {
        shape: probe_box,
        color: PROBE_A,
    });
    let probe_circle_layer = hir.layer(Layer::Fill {
        shape: probe_circle,
        color: PROBE_B,
    });
    let probe_capsule_layer = hir.layer(Layer::Fill {
        shape: probe_capsule,
        color: PROBE_C,
    });
    let corner_marker_layer = hir.layer(Layer::Fill {
        shape: corner_marker,
        color: PROBE_D,
    });
    let top_marker_layer = hir.layer(Layer::Fill {
        shape: top_marker,
        color: PROBE_C,
    });
    let side_marker_layer = hir.layer(Layer::Fill {
        shape: side_marker,
        color: PROBE_B,
    });

    let mut preview_layers = Vec::new();

    for (x, major) in [
        (0.14, false),
        (0.22, false),
        (0.30, false),
        (0.38, false),
        (0.46, true),
        (0.54, true),
        (0.62, false),
        (0.70, false),
        (0.78, false),
        (0.86, false),
    ] {
        let shape = hir.shape(Shape::RBox {
            center: lit2(x, 0.5),
            half: lit2(if major { 0.0032 } else { 0.0022 }, 0.24),
            round: Sx::Lit(0.002),
        });
        let layer = hir.layer(Layer::Fill {
            shape,
            color: if major { GRID_MAJOR } else { GRID_MINOR },
        });
        preview_layers.push((layer, Blend::Over));
    }

    for (y, major) in [
        (0.14, false),
        (0.22, false),
        (0.30, false),
        (0.38, false),
        (0.46, true),
        (0.54, true),
        (0.62, false),
        (0.70, false),
        (0.78, false),
        (0.86, false),
    ] {
        let shape = hir.shape(Shape::RBox {
            center: lit2(0.5, y),
            half: lit2(0.35, if major { 0.0032 } else { 0.0022 }),
            round: Sx::Lit(0.002),
        });
        let layer = hir.layer(Layer::Fill {
            shape,
            color: if major { GRID_MAJOR } else { GRID_MINOR },
        });
        preview_layers.push((layer, Blend::Over));
    }

    preview_layers.extend([
        (frame_layer, Blend::Over),
        (axis_x_layer, Blend::Over),
        (axis_y_layer, Blend::Over),
        (probe_box_layer, Blend::Over),
        (probe_circle_layer, Blend::Over),
        (probe_capsule_layer, Blend::Over),
        (corner_marker_layer, Blend::Over),
        (top_marker_layer, Blend::Over),
        (side_marker_layer, Blend::Over),
    ]);

    if profile.repeat_x {
        for x in [0.16, 0.50, 0.84] {
            let marker = hir.shape(Shape::Circle {
                center: lit2(x, 0.14),
                radius: Sx::Lit(0.018),
            });
            let layer = hir.layer(Layer::Fill {
                shape: marker,
                color: AXIS_X,
            });
            preview_layers.push((layer, Blend::Over));
        }
    }
    if profile.repeat_y {
        for y in [0.18, 0.50, 0.82] {
            let marker = hir.shape(Shape::Circle {
                center: lit2(0.88, y),
                radius: Sx::Lit(0.018),
            });
            let layer = hir.layer(Layer::Fill {
                shape: marker,
                color: AXIS_Y,
            });
            preview_layers.push((layer, Blend::Over));
        }
    }
    if profile.repeat_radial || profile.polar {
        let spoke = hir.shape(Shape::RBox {
            center: lit2(0.24, 0.50),
            half: lit2(0.012, 0.24),
            round: Sx::Lit(0.01),
        });
        let ring_a = hir.shape(Shape::RBox {
            center: lit2(0.52, 0.30),
            half: lit2(0.20, 0.012),
            round: Sx::Lit(0.01),
        });
        let ring_b = hir.shape(Shape::RBox {
            center: lit2(0.62, 0.70),
            half: lit2(0.16, 0.012),
            round: Sx::Lit(0.01),
        });
        for shape in [spoke, ring_a, ring_b] {
            let layer = hir.layer(Layer::Fill {
                shape,
                color: PROBE_D,
            });
            preview_layers.push((layer, Blend::Over));
        }
    }
    if profile.depth_like {
        for (cx, cy, hx, hy) in [
            (0.72, 0.22, 0.09, 0.06),
            (0.64, 0.40, 0.07, 0.05),
            (0.56, 0.58, 0.05, 0.04),
        ] {
            let shape = hir.shape(Shape::RBox {
                center: lit2(cx, cy),
                half: lit2(hx, hy),
                round: Sx::Lit(0.018),
            });
            let layer = hir.layer(Layer::Fill {
                shape,
                color: PROBE_C,
            });
            preview_layers.push((layer, Blend::Over));
        }
    }
    if profile.warp {
        for (cx, cy, hx, hy) in [
            (0.28, 0.28, 0.12, 0.008),
            (0.28, 0.72, 0.12, 0.008),
            (0.28, 0.50, 0.008, 0.12),
        ] {
            let shape = hir.shape(Shape::RBox {
                center: lit2(cx, cy),
                half: lit2(hx, hy),
                round: Sx::Lit(0.008),
            });
            let layer = hir.layer(Layer::Fill {
                shape,
                color: PROBE_D,
            });
            preview_layers.push((layer, Blend::Over));
        }
    }

    let preview_inner = hir.layer(Layer::Compose(preview_layers));
    let preview_space = hir.layer(Layer::InSpace {
        xforms,
        inner: preview_inner,
    });

    hir.layer(Layer::Compose(vec![
        (background, Blend::Over),
        (preview_space, Blend::Over),
    ]))
}

fn lit2(x: f32, y: f32) -> (Sx, Sx) {
    (Sx::Lit(x), Sx::Lit(y))
}

struct SpaceProbeProfile {
    repeat_x: bool,
    repeat_y: bool,
    repeat_radial: bool,
    polar: bool,
    depth_like: bool,
    warp: bool,
}

fn analyze_space_probe_profile(xforms: &[Xform]) -> SpaceProbeProfile {
    let mut profile = SpaceProbeProfile {
        repeat_x: false,
        repeat_y: false,
        repeat_radial: false,
        polar: false,
        depth_like: false,
        warp: false,
    };
    for xform in xforms {
        match xform {
            Xform::RepeatX(_) => profile.repeat_x = true,
            Xform::RepeatY(_) => profile.repeat_y = true,
            Xform::Repeat2D { .. } => {
                profile.repeat_x = true;
                profile.repeat_y = true;
            }
            Xform::RepeatRadial { .. } => profile.repeat_radial = true,
            Xform::Polar { .. } => profile.polar = true,
            Xform::Perspective { .. }
            | Xform::RotateX { .. }
            | Xform::RotateY { .. }
            | Xform::Translate3 { .. } => {
                profile.depth_like = true;
            }
            Xform::Warp { .. } => profile.warp = true,
            _ => {}
        }
    }
    profile
}

struct LowerInputs {
    resource_layout: crate::ast::ResourceLayout,
    compute_library: check::compute::ComputeLibrary,
    hirs: Vec<Hir>,
    material_hirs: Vec<MaterialHir>,
    structs: Vec<crate::ast::StructDecl>,
    consts: Vec<crate::ast::ConstDecl>,
    passes: Vec<crate::ast::PassDecl>,
    pipelines: Vec<crate::ast::PipelineDecl>,
    vertex_interfaces: Vec<crate::ast::VertexInterfaceDecl>,
    vertex_formats: Vec<crate::ast::VertexFormatDecl>,
    vertex_factories: Vec<crate::ast::VertexFactoryDecl>,
}

fn lower_and_validate(input: LowerInputs) -> Result<CompiledProgram, Vec<Diag>> {
    let LowerInputs {
        resource_layout,
        compute_library,
        hirs,
        material_hirs,
        structs,
        consts,
        passes,
        pipelines,
        vertex_interfaces,
        vertex_formats,
        vertex_factories,
    } = input;
    let mut result = lower::lower_all(&hirs, &material_hirs);
    for (_, variable) in result.module.global_variables.iter_mut() {
        if let Some(binding) = &mut variable.binding {
            binding.group =
                resource_layout.0[usize::try_from(binding.group).expect("internal resource class")];
        }
    }
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    let info = validator.validate(&result.module).map_err(|e| {
        vec![
            Diag::error(0..0, "generated naga IR failed validation"),
            Diag::info(
                0..0,
                "This is a Fresco compiler bug (the user program was accepted).",
            )
            .with_help(format!(
                "validator details: {e:#?}\nRe-run with `--emit ir` to inspect the module, then report the issue."
            )),
        ]
    })?;

    Ok(CompiledProgram {
        resource_layout,
        compute_library,
        module: result.module,
        info,
        consts,
        hirs,
        material_hirs,
        structs,
        passes,
        pipelines,
        vertex_interfaces,
        vertex_formats,
        vertex_factories,
        stats: result.stats,
        diagnostics: Vec::new(),
        timings: PipelineTimings::default(),
        tex_bindings: result.tex_bindings,
        pass_target_bindings: result.pass_target_bindings,
        path_bindings: result.path_bindings,
        param_bindings: result.param_bindings,
        global_uniform_bindings: result.global_uniform_bindings,
    })
}

fn perr_to_diag(e: parser::PError<'_>) -> Diag {
    if let chumsky::error::RichReason::Custom(msg) = e.reason() {
        let mut d = Diag::error(e.span().clone(), msg.to_string());
        if let Some((label, _)) = e.contexts().last() {
            d = d.with_label(format!("while parsing {label}"));
        }
        return d;
    }

    let found = e
        .found()
        .map(ToString::to_string)
        .unwrap_or_else(|| "end of input".to_string());
    let mut expected: Vec<String> = e.expected().map(ToString::to_string).collect();
    expected.sort();
    expected.dedup();

    let mut d = Diag::error(e.span().clone(), format!("unexpected {found}"));
    if let Some((label, _)) = e.contexts().last() {
        d = d.with_label(format!("while parsing {label}"));
    } else {
        d = d.with_label("unexpected token");
    }
    if !expected.is_empty() {
        d = d.with_help(format!("expected {}", expected.join(", ")));
    }
    d
}
