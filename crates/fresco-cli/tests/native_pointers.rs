use fresco::driver::native::{self, Options, ResourceBinding, Target};
use std::collections::BTreeMap;

const SOURCE: &str = include_str!("fixtures/native-pointers.fr");

fn options() -> Options {
    Options {
        target: Target::NativeSpirV,
        entry_point: "update_one".into(),
        workgroup_size: [64, 1, 1],
        bindings: BTreeMap::from([(
            "roots".into(),
            ResourceBinding {
                group: 0,
                binding: 0,
            },
        )]),
    }
}

#[test]
fn authored_pointers_validate_as_spirv_before_and_after_compaction() {
    validate_program("arrays", SOURCE);
    validate_program(
        "members",
        include_str!("fixtures/native-pointer-members.fr"),
    );
}

fn validate_program(case: &str, source: &str) {
    let mut artifact = native::compile(source, &options()).unwrap();
    assert_eq!(
        artifact.module.functions.len(),
        1,
        "ordinary helpers must remain real IR functions"
    );
    for compact in [false, true] {
        if compact {
            artifact.compact().unwrap();
        }
        let words = artifact.spirv().unwrap();
        let path = std::env::temp_dir().join(format!(
            "fresco-native-{}-{case}-{compact}.spv",
            std::process::id()
        ));
        std::fs::write(
            &path,
            words
                .iter()
                .flat_map(|w| w.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let output = std::process::Command::new(
            std::env::var_os("SPIRV_VAL").unwrap_or_else(|| "spirv-val".into()),
        )
        .args(["--target-env", "vulkan1.1"])
        .arg(&path)
        .output()
        .expect("install spirv-val or set SPIRV_VAL");
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn native_cli_writes_binary_and_host_contract() {
    let directory = std::env::temp_dir().join(format!("fresco-native-cli-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("shader.fr");
    let binary = directory.join("shader.spv");
    let manifest = directory.join("shader.json");
    std::fs::write(&input, SOURCE).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fresco"))
        .arg("native")
        .arg(&input)
        .args(["--entry", "update_one", "--bind", "roots=0:0"])
        .arg("-o")
        .arg(&binary)
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        &std::fs::read(&binary).unwrap()[..4],
        &0x0723_0203_u32.to_le_bytes()
    );
    let contract: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    assert_eq!(
        contract["requirements"]["physical_storage_buffer_addresses"],
        true
    );
    assert_eq!(contract["roots"][0]["binding"], 0);
    for path in [input, binary, manifest] {
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(directory).unwrap();
}
