use fresco::driver::native::{self, Options, ResourceBinding, Target};
use std::{collections::BTreeMap, path::PathBuf, process::ExitCode};

#[derive(clap::Args)]
pub(crate) struct Args {
    /// Fresco source containing native records and functions.
    input: PathBuf,
    #[arg(long)]
    entry: String,
    /// Explicit root descriptor mapping, e.g. --bind roots=0:0.
    #[arg(long = "bind", value_parser = binding)]
    bindings: Vec<(String, ResourceBinding)>,
    /// Compute workgroup dimensions (x,y,z).
    #[arg(long, value_delimiter = ',', num_args = 3, default_value = "64,1,1")]
    workgroup_size: Vec<u32>,
    /// Binary little-endian SPIR-V output.
    #[arg(short, long)]
    output: PathBuf,
    /// Optional ABI and required-device-features JSON.
    #[arg(long)]
    manifest: Option<PathBuf>,
}

fn binding(text: &str) -> Result<(String, ResourceBinding), String> {
    let invalid = || "expected root=group:binding, e.g. roots=0:0".to_string();
    let (name, numbers) = text.split_once('=').ok_or_else(invalid)?;
    let (group, index) = numbers.split_once(':').ok_or_else(invalid)?;
    if name.is_empty() {
        return Err(invalid());
    }
    Ok((
        name.into(),
        ResourceBinding {
            group: group.parse().map_err(|_| invalid())?,
            binding: index.parse().map_err(|_| invalid())?,
        },
    ))
}

pub(crate) fn run(args: &Args) -> ExitCode {
    match compile(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn compile(args: &Args) -> Result<(), String> {
    let source = std::fs::read_to_string(&args.input).map_err(|e| e.to_string())?;
    let mut bindings = BTreeMap::new();
    for (name, binding) in &args.bindings {
        if bindings.insert(name.clone(), *binding).is_some() {
            return Err(format!("duplicate root binding `{name}`"));
        }
    }
    let artifact = native::compile(
        &source,
        &Options {
            target: Target::NativeSpirV,
            entry_point: args.entry.clone(),
            workgroup_size: args
                .workgroup_size
                .as_slice()
                .try_into()
                .map_err(|_| "expected three workgroup dimensions")?,
            bindings,
        },
    )
    .map_err(|e| {
        format!(
            "{}:{}..{}: {}",
            args.input.display(),
            e.span.start,
            e.span.end,
            e.message
        )
    })?;
    let words = artifact.spirv().map_err(|e| e.message)?;
    let manifest = artifact.manifest_json().map_err(|e| e.message)?;
    let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    std::fs::write(&args.output, bytes).map_err(|e| e.to_string())?;
    if let Some(path) = &args.manifest {
        std::fs::write(path, manifest).map_err(|e| e.to_string())?;
    }
    Ok(())
}
