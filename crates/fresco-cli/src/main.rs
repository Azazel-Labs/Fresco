#![deny(unused_must_use)]
#![deny(clippy::dbg_macro, clippy::todo)]

use clap::Parser as ClapParser;
use fresco::driver;
use std::process::ExitCode;
use std::thread;

mod fmt;

#[derive(ClapParser)]
#[command(
    name = "fresco",
    version,
    about = "Fresco: a layer-oriented shader language, compiled through naga IR"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Command>,

    /// Input .fr source file
    input: Option<std::path::PathBuf>,

    /// Output target: `wgsl`, `ir` (debug dump), or `manifest` (parameter metadata JSON)
    #[arg(long, default_value = "wgsl")]
    emit: EmitArg,

    /// Print the compiler's receipt: rewrites applied, locality, pass
    /// structure, shape CSE stats (design doc: "no hidden passes — but
    /// visible ones")
    #[arg(long)]
    explain: bool,

    /// Write output here instead of stdout
    #[arg(short, long)]
    output: Option<std::path::PathBuf>,

    /// Print per-pass timing breakdown after compilation
    #[arg(long)]
    timings: bool,

    /// Enable compiler progress logging to stderr.
    #[arg(long)]
    verbose: bool,

    /// Verbosity for `--verbose` logging.
    #[arg(long, value_enum, default_value_t = TraceLevelArg::Info)]
    trace_level: TraceLevelArg,

    /// Path to a JSON compiler context file (merged with CLI overrides)
    #[arg(long)]
    context: Option<std::path::PathBuf>,

    /// Engine directory containing engine.fr (replaces ancestor discovery)
    #[arg(long)]
    engine_dir: Option<std::path::PathBuf>,

    /// Warn when shape-distance bound / effect extent exceeds this ratio
    #[arg(long)]
    wide_effect_warn_ratio: Option<f32>,

    /// Emit note (instead of warning) at or below this ratio
    #[arg(long)]
    wide_effect_note_ratio: Option<f32>,

    /// Abort checker expression evaluation after this many milliseconds and emit timeout diagnostics.
    #[arg(long)]
    expr_timeout_ms: Option<u64>,

    /// Temporary migration flag: allow implicit texture UV reads (for example `image(tex)` and `tex.channel`).
    #[arg(long)]
    allow_implicit_texture_uv: bool,

    /// Build profile for manifest/config-axis inclusion (`all`, `editor`, `runtime`).
    #[arg(long, value_enum)]
    build_profile: Option<BuildProfileArg>,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum EmitArg {
    Wgsl,
    Ir,
    Manifest,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum TraceLevelArg {
    Info,
    Debug,
    Trace,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum BuildProfileArg {
    All,
    Editor,
    Runtime,
}

impl From<EmitArg> for driver::EmitTarget {
    fn from(value: EmitArg) -> Self {
        match value {
            EmitArg::Wgsl => driver::EmitTarget::Wgsl,
            EmitArg::Ir => driver::EmitTarget::Ir,
            EmitArg::Manifest => driver::EmitTarget::Manifest,
        }
    }
}

impl From<TraceLevelArg> for driver::TraceVerbosity {
    fn from(value: TraceLevelArg) -> Self {
        match value {
            TraceLevelArg::Info => driver::TraceVerbosity::Info,
            TraceLevelArg::Debug => driver::TraceVerbosity::Debug,
            TraceLevelArg::Trace => driver::TraceVerbosity::Trace,
        }
    }
}

impl From<BuildProfileArg> for driver::BuildProfile {
    fn from(value: BuildProfileArg) -> Self {
        match value {
            BuildProfileArg::All => driver::BuildProfile::All,
            BuildProfileArg::Editor => driver::BuildProfile::Editor,
            BuildProfileArg::Runtime => driver::BuildProfile::Runtime,
        }
    }
}

#[derive(clap::Subcommand)]
enum Command {
    /// Format .fr files in-place (or run in check mode)
    Fmt(FmtArgs),
}

#[derive(clap::Args)]
struct FmtArgs {
    /// File(s) or directories to format recursively
    paths: Vec<std::path::PathBuf>,

    /// Check formatting without writing files
    #[arg(long)]
    check: bool,

    /// Print formatted output for a single file to stdout
    #[arg(long)]
    stdout: bool,

    /// Style for binary-operator line wrapping
    #[arg(long, value_enum, default_value_t = fmt::BinaryWrapStyle::Indent)]
    binary_wrap_style: fmt::BinaryWrapStyle,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Some(Command::Fmt(fmt_args)) = cli.cmd {
        if fmt_args.paths.is_empty() {
            eprintln!("error: `fresco fmt` requires at least one file or directory path");
            return ExitCode::FAILURE;
        }
        return fmt::run(
            &fmt_args.paths,
            fmt::FormatOptions {
                check: fmt_args.check,
                stdout: fmt_args.stdout,
                binary_wrap_style: fmt_args.binary_wrap_style,
            },
        );
    }

    let Some(input) = cli.input else {
        eprintln!(
            "error: missing input .fr file\nhelp: run `fresco <input.fr>` or `fresco fmt <path>`"
        );
        return ExitCode::FAILURE;
    };

    let mut context = driver::CompileContext::default();
    if let Some(path) = &cli.context {
        match driver::ContextFile::from_json_path(path) {
            Ok(file) => file.apply_to(&mut context),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(v) = cli.wide_effect_warn_ratio {
        context.check.wide_effect_warn_ratio = v;
    }
    if let Some(v) = cli.wide_effect_note_ratio {
        context.check.wide_effect_note_ratio = v;
    }
    if let Some(v) = cli.expr_timeout_ms {
        context.check.expr_timeout_ms = Some(v);
    }
    if cli.allow_implicit_texture_uv {
        context.check.allow_implicit_texture_uv = true;
    }
    if let Some(profile) = cli.build_profile {
        context.build_profile = profile.into();
    }
    if let Err(e) = context.validate() {
        eprintln!("error: invalid compiler context: {e}");
        return ExitCode::FAILURE;
    }

    match thread::Builder::new()
        .name("fresco-compile".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            driver::run(driver::Options {
                input,
                engine_dir: cli.engine_dir,
                emit: cli.emit.into(),
                explain: cli.explain,
                output: cli.output,
                timings: cli.timings,
                verbose: cli.verbose,
                trace_verbosity: cli.trace_level.into(),
                context,
            })
        }) {
        Ok(handle) => match handle.join() {
            Ok(code) => code,
            Err(_) => {
                eprintln!("error: compiler thread panicked");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("error: failed to start compiler thread: {e}");
            ExitCode::FAILURE
        }
    }
}
