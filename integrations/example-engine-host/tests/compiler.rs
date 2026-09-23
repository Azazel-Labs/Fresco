#![cfg(all(feature = "native", not(target_arch = "wasm32")))]

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn emitter_sources_select_particle_playback_outside_the_repository() {
    let project = Project::new();
    let source = include_str!("../../../examples/50) particles/drifting_sparks.fr");
    for (name, source) in [
        ("fixed.fr", source.to_owned()),
        ("growing.fr", source.replace("spawn_rate: 60.0", "spawn_rate: 4.0\n    allocation: ParticleAllocationMode.Automatic\n    allocation_hint: 1\n    max_particles: 32")),
    ] {
        std::fs::write(project.0.join(name), source).unwrap();
        let output = project.run(&["--source", name]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("Compiled particles `drifting_sparks`"));
    }
}

#[test]
fn material_parameters_and_textures_work_outside_the_repository() {
    let project = Project::new();
    std::fs::write(
        project.0.join("material.fr"),
        include_str!("../../example-engine/examples/material_parameters.fr"),
    )
    .unwrap();
    let output = project.run(&[
        "--source",
        "material.fr",
        "--params",
        "{\"gain\":0.5,\"enabled\":false}",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Compiled surface `adjustable`"));
    for shape in ["sphere", "plane", "box"] {
        let output = project.run(&["--source", "material.fr", "--mesh", shape]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let unknown = project.run(&["--source", "material.fr", "--mesh", "unknown"]);
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("mesh must be sphere, plane, or box")
    );
    assert!(
        !project
            .run(&["--source", "material.fr", "--params", "{\"gain\":2}"])
            .status
            .success()
    );
    std::fs::write(
        project.0.join("textured.fr"),
        include_str!("../../example-engine/examples/material_texture.fr"),
    )
    .unwrap();
    std::fs::write(
        project.0.join("image.png"),
        include_bytes!("../../example-engine/assets/checker.png"),
    )
    .unwrap();
    let textured = project.run(&["--source", "textured.fr", "--texture", "paint=image.png"]);
    assert!(
        textured.status.success(),
        "{}",
        String::from_utf8_lossy(&textured.stderr)
    );
}

#[test]
fn mixed_canvas_and_surface_entries_require_explicit_selection() {
    let project = Project::new();
    let source = format!(
        "{}\ncanvas canvas_probe(ctx: CanvasContext) -> color {{ rgba(1,0,0,1) }}",
        include_str!("../../example-engine/examples/material_parameters.fr")
    );
    std::fs::write(project.0.join("mixed.fr"), source).unwrap();
    assert!(!project.run(&["--source", "mixed.fr"]).status.success());
    for entry in ["canvas_probe", "adjustable"] {
        let output = project.run(&["--source", "mixed.fr", "--entry", entry]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fresco-host-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_fresco-example-engine"))
            .current_dir(&self.0)
            .arg("--check")
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn path_example_compiles_outside_the_repository_with_embedded_engine_contracts() {
    let project = Project::new();
    std::fs::write(
        project.0.join("paths.fr"),
        include_str!("../../example-engine/examples/path_canvas.fr"),
    )
    .unwrap();
    let output = project.run(&[
        "--source",
        "paths.fr",
        "--params",
        "{\"widths\":[0.03,0.05]}",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn canvas_storage_and_uniform_overrides_validate_together() {
    let project = Project::new();
    std::fs::write(
        project.0.join("arrays.fr"),
        r#"canvas arrays(ctx: CanvasContext) -> color {
        param gain: f32 = 1.0
        param points: array<vec3> = [vec3(0,0,0), vec3(0,1,0)]
        rgba(0.0, points[1].y * gain, 0.0, 1.0)
    }"#,
    )
    .unwrap();
    let output = project.run(&["--source", "arrays.fr", "--params", "{\"gain\":0.5}"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for json in [
        r#"{"gain":0.75,"points":[[1,0,0],[0,0,1],[0,1,0]]}"#,
        r#"{"points":[]}"#,
    ] {
        let output = project.run(&["--source", "arrays.fr", "--params", json]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for json in [
        r#"{"gain":0.75,"points":[[1,0]]}"#,
        r#"{"points":[[1e100,0,0]]}"#,
    ] {
        let output = project.run(&["--source", "arrays.fr", "--params", json]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("points"));
    }
}

#[test]
fn embedded_demo_compiles_outside_the_repository_without_engine_files() {
    let project = Project::new();
    let result = project.run(&[]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("`demo`"));
}

#[test]
fn parameter_overrides_validate_names_types_and_ranges_without_a_gpu() {
    let project = Project::new();
    std::fs::write(project.0.join("main.fr"), "canvas probe(ctx: CanvasContext) -> color { param gain: f32 = 0.5 in 0 .. 1; rgba(gain, 0.0, 0.0, 1.0) }").unwrap();
    let good = project.run(&["--source", "main.fr", "--params", r#"{"gain":0.75}"#]);
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stderr)
    );
    for value in [
        r#"{"gain":2}"#,
        r#"{"unknown":1}"#,
        r#"{"gain":true}"#,
        "[]",
    ] {
        let bad = project.run(&["--source", "main.fr", "--params", value]);
        assert!(!bad.status.success(), "accepted {value}");
    }
    std::fs::write(project.0.join("parameters.json"), r#"{"gain":0.3}"#).unwrap();
    let file = project.run(&["--source", "main.fr", "--params-file", "parameters.json"]);
    assert!(
        file.status.success(),
        "{}",
        String::from_utf8_lossy(&file.stderr)
    );
    std::fs::write(project.0.join("parameters.json"), r#"{"gain":2}"#).unwrap();
    assert!(
        !project
            .run(&["--source", "main.fr", "--params-file", "parameters.json"])
            .status
            .success()
    );
}

#[test]
fn source_imports_use_the_project_while_the_engine_remains_embedded() {
    let project = Project::new();
    std::fs::create_dir(project.0.join("engine")).unwrap();
    std::fs::write(
        project.0.join("engine/engine.fr"),
        "invalid discovered engine",
    )
    .unwrap();
    std::fs::write(
        project.0.join("helper.fr"),
        "fn tint(uv: vec2) -> color { return rgba(uv.x, uv.y, 0.0, 1.0) }",
    )
    .unwrap();
    std::fs::write(
        project.0.join("main.fr"),
        "import \"helper.fr\"\ncanvas authored(ctx: CanvasContext) -> color { tint(ctx.uv) }",
    )
    .unwrap();
    let result = project.run(&["--source", "main.fr"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("`authored`"));
}

#[test]
fn external_texture_assets_require_explicit_readable_images() {
    let project = Project::new();
    let source = "canvas paint(ctx: CanvasContext) -> color { [binding(default = \"paint.png\")] uniform paint: texture; paint.at(ctx.uv) }";
    std::fs::write(project.0.join("main.fr"), source).unwrap();
    assert!(!project.run(&["--source", "main.fr"]).status.success());
    std::fs::write(
        project.0.join("paint.png"),
        fresco_example_engine::assets::CHECKER_PNG,
    )
    .unwrap();
    let loaded = project.run(&["--source", "main.fr"]);
    assert!(
        loaded.status.success(),
        "{}",
        String::from_utf8_lossy(&loaded.stderr)
    );
    std::fs::write(project.0.join("paint.png"), b"broken").unwrap();
    assert!(!project.run(&["--source", "main.fr"]).status.success());
    std::fs::create_dir(project.0.join("assets")).unwrap();
    std::fs::write(
        project.0.join("assets/paint.png"),
        fresco_example_engine::assets::CHECKER_PNG,
    )
    .unwrap();
    assert!(
        project
            .run(&["--source", "main.fr", "--asset-root", "assets"])
            .status
            .success()
    );
    std::fs::write(
        project.0.join("replacement.png"),
        fresco_example_engine::assets::CHECKER_PNG,
    )
    .unwrap();
    assert!(
        project
            .run(&["--source", "main.fr", "--texture", "paint=replacement.png"])
            .status
            .success()
    );
    assert!(
        !project
            .run(&[
                "--source",
                "main.fr",
                "--texture",
                "unknown=replacement.png"
            ])
            .status
            .success()
    );
    assert!(
        !project
            .run(&[
                "--source",
                "main.fr",
                "--texture",
                "paint=replacement.png",
                "--texture",
                "paint=replacement.png"
            ])
            .status
            .success()
    );
}

#[test]
fn ambiguous_or_unknown_entries_require_explicit_valid_selection() {
    let project = Project::new();
    std::fs::write(project.0.join("main.fr"), "canvas first(ctx: CanvasContext) -> color { rgba(1, 0, 0, 1) }\ncanvas second(ctx: CanvasContext) -> color { rgba(0, 1, 0, 1) }").unwrap();
    let ambiguous = project.run(&["--source", "main.fr"]);
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("--entry"));
    let selected = project.run(&["--source", "main.fr", "--entry", "second"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    assert!(String::from_utf8_lossy(&selected.stdout).contains("`second`"));
    let missing = project.run(&["--source", "main.fr", "--entry", "missing"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("does not exist"));
}

#[test]
fn incomplete_engine_override_is_not_completed_from_the_embedded_profile() {
    let project = Project::new();
    std::fs::create_dir(project.0.join("override")).unwrap();
    std::fs::write(
        project.0.join("override/engine.fr"),
        "import \"core/00_prelude.fr\"",
    )
    .unwrap();
    let result = project.run(&["--engine-dir", "override"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("00_prelude.fr"));
}

#[test]
fn native_canvas_variants_require_complete_explicit_bindings() {
    let project = Project::new();
    for file in fresco_example_engine::SOURCES {
        let path = project.0.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = if file.path == "engine/core/04_canvas_contract.fr" {
            file.source.replace("    binding {", "    permutations { @known(compile) quality: \"low\" | \"high\" }\n    binding {")
                .replace("        return t.draw(ctx)", "        if quality == \"low\" { return rgba(1.0, 0.0, 0.0, 1.0) }\n        return t.draw(ctx)")
        } else {
            file.source.into()
        };
        std::fs::write(path, source).unwrap();
    }
    let missing = project.run(&["--engine-dir", "engine"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("explicit pass variant"));
    for value in ["low", "high"] {
        let selection = format!(r#"{{"quality":"{value}"}}"#);
        let result = project.run(&["--engine-dir", "engine", "--variant", &selection]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for selection in [
        "{}",
        r#"{"quality":"invalid"}"#,
        r#"{"quality":"low","extra":"on"}"#,
        r#"{"quality":1}"#,
    ] {
        assert!(
            !project
                .run(&["--engine-dir", "engine", "--variant", selection])
                .status
                .success()
        );
    }
}
