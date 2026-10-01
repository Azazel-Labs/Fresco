#[path = "support/common.rs"]
mod common;

use common::{normalize, repo_root};
use libtest_mimic::Failed;
use std::{ffi::OsString, fs, path::Path, process::Command};

fn validator() -> OsString {
    std::env::var_os("SPIRV_VAL").unwrap_or_else(|| "spirv-val".into())
}

fn validate_binary(path: &Path) -> Result<(), Failed> {
    let output = Command::new(validator())
        .args(["--target-env", "vulkan1.1"])
        .arg(path)
        .output()
        .map_err(|error| {
            Failed::from(format!(
                "cannot run spirv-val: {error}; install SPIRV-Tools or set SPIRV_VAL to its executable"
            ))
        })?;
    if !output.status.success() {
        return Err(Failed::from(format!(
            "spirv-val rejected {}\n{}\n{}",
            path.display(),
            normalize(&output.stdout),
            normalize(&output.stderr)
        )));
    }
    Ok(())
}

fn validate_example(file: &Path) -> Result<(), Failed> {
    let output = common::run_example(file);
    if !output.status.success() {
        return Err(Failed::from(format!(
            "Fresco compilation failed\n{}\n{}",
            normalize(&output.stdout),
            normalize(&output.stderr)
        )));
    }
    let source = String::from_utf8(output.stdout)
        .map_err(|error| Failed::from(format!("invalid UTF-8 output: {error}")))?;
    // Transitional boundary: engine stages are still appended as WGSL by the
    // driver. Use the COMPLETE output, not its earlier, incomplete Naga module.
    // Replace this frontend step with the complete IR API when lowering migrates.
    let module = naga::front::wgsl::parse_str(&source)
        .map_err(|error| Failed::from(error.emit_to_string(&source)))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|error| Failed::from(format!("invalid Naga IR: {error:#?}")))?;
    if module.entry_points.is_empty() {
        return Err(Failed::from("example generated no shader entry points"));
    }
    let options = naga::back::spv::Options {
        lang_version: (1, 3),
        fake_missing_bindings: false,
        binding_map: module
            .global_variables
            .iter()
            .filter_map(|(_, global)| {
                global.binding.map(|binding| {
                    (
                        binding,
                        naga::back::spv::BindingInfo {
                            descriptor_set: binding.group,
                            binding: binding.binding,
                            binding_array_size: None,
                        },
                    )
                })
            })
            .collect(),
        ..Default::default()
    };
    let directory = common::unique_temp_dir("spirv");
    for (index, entry) in module.entry_points.iter().enumerate() {
        let pipeline = naga::back::spv::PipelineOptions {
            shader_stage: entry.stage,
            entry_point: entry.name.clone(),
        };
        let words = naga::back::spv::write_vec(&module, &info, &options, Some(&pipeline)).map_err(
            |error| {
                Failed::from(format!(
                    "SPIR-V emission for {:?} {} failed: {error}",
                    entry.stage, entry.name
                ))
            },
        )?;
        let binary = directory.join(format!("{index}.spv"));
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        fs::write(&binary, bytes).map_err(|error| Failed::from(error.to_string()))?;
        validate_binary(&binary).map_err(|error| {
            Failed::from(format!("entry {:?} {}: {error:?}", entry.stage, entry.name))
        })?;
    }
    fs::remove_dir_all(directory).map_err(|error| Failed::from(error.to_string()))?;
    Ok(())
}

fn main() {
    let args = common::harness_args();
    let trials = common::must_compile_example_trials(&repo_root(), validate_example);
    assert!(
        !trials.is_empty(),
        "expected supported examples to validate"
    );
    libtest_mimic::run(&args, trials).exit();
}
