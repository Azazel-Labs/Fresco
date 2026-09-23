use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "fresco-engine-selection-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(path.join("project/engine")).unwrap();
        std::fs::create_dir_all(path.join("selected")).unwrap();
        std::fs::write(
            path.join("project/main.fr"),
            "canvas probe(uv: coord) -> color { rgba(uv.x, uv.y, 0.0, 1.0) }",
        )
        .unwrap();
        std::fs::write(
            path.join("project/engine/engine.fr"),
            "invalid ancestor engine",
        )
        .unwrap();
        std::fs::write(
            path.join("selected/policy.fr"),
            include_str!("../../../tests/render-policy/engine/engine.fr"),
        )
        .unwrap();
        Self(path)
    }

    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_fresco"))
            .current_dir(&self.0)
            .args(["project/main.fr", "--engine-dir", "selected"])
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn embedded_engine_and_filesystem_imports_keep_separate_resolution_domains() {
    let fixture = Fixture::new();
    let source = "import \"helper.fr\"\ncanvas probe(uv: coord) -> color { rgba(engine_value(), project_value(), 0.0, 1.0) }";
    std::fs::write(
        fixture.0.join("project/helper.fr"),
        "fn project_value() -> f32 { return 0.75 }",
    )
    .unwrap();
    std::fs::write(fixture.0.join("project/main.fr"), source).unwrap();
    let mut engine = HashMap::from([
        (
            "engine/engine.fr".into(),
            format!(
                "import \"helper.fr\"\n{}",
                include_str!("../../../tests/render-policy/engine/engine.fr")
            ),
        ),
        (
            "engine/helper.fr".into(),
            "fn engine_value() -> f32 { return 0.25 }".into(),
        ),
    ]);
    let compile = |files: &HashMap<String, String>| {
        fresco::driver::compile_source_bundle_with_engine_files(
            source,
            &fixture.0.join("project/main.fr").to_string_lossy(),
            false,
            &fresco::driver::CompileContext::default(),
            files,
            "engine/engine.fr",
        )
    };
    let artifact = compile(&engine).expect("both import domains resolve their own helper.fr");
    naga::front::wgsl::parse_str(&artifact.wgsl).expect("valid emitted WGSL");
    engine.remove("engine/helper.fr");
    std::fs::write(
        fixture.0.join("project/engine/helper.fr"),
        "fn engine_value() -> f32 { return 0.25 }",
    )
    .unwrap();
    let errors = compile(&engine).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("helper.fr"))
    );
    engine.remove("engine/engine.fr");
    assert!(
        compile(&engine)
            .unwrap_err()
            .iter()
            .any(|error| error.message.contains("embedded engine entry"))
    );
}

#[test]
fn explicit_engine_replaces_discovery_and_resolves_its_own_imports() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("selected/engine.fr"), "import \"policy.fr\"").unwrap();
    let output = fixture.run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let source = std::fs::read_to_string(fixture.0.join("project/main.fr")).unwrap();
    let artifact = fresco::driver::compile_source_bundle_with_engine_dir(
        &source,
        &fixture.0.join("project/main.fr").to_string_lossy(),
        false,
        &fresco::driver::CompileContext::default(),
        Some(&fixture.0.join("selected")),
    )
    .expect("explicit API selection");
    assert!(artifact.wgsl.contains("fresco_probe"));
}

#[test]
fn explicit_engine_requires_entrypoint_even_when_other_modules_exist() {
    let fixture = Fixture::new();
    let output = fixture.run();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot resolve explicit engine entry")
    );
}

#[test]
fn explicit_engine_does_not_fill_missing_imports_from_discovered_engine() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.0.join("selected/engine.fr"),
        "import \"missing.fr\"",
    )
    .unwrap();
    std::fs::write(fixture.0.join("project/engine/missing.fr"), "").unwrap();
    let output = fixture.run();
    assert!(!output.status.success());
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        errors.contains("cannot read imported module `missing.fr`"),
        "{errors}"
    );
}
