use std::collections::HashMap;
use std::fs;

#[path = "support/common.rs"]
mod common;

use common::{normalize, run_fresco, unique_temp_dir};
use fresco::driver;

#[test]
fn import_function_module_compiles() {
    let dir = unique_temp_dir("import_ok");
    let lib = dir.join("math.fr");
    let main = dir.join("main.fr");

    fs::write(
        &lib,
        r#"fn sparkle(amount: f32) -> f32 {
  amount
}
"#,
    )
    .expect("failed to write imported module source");

    fs::write(
        &main,
        r#"import "math.fr"

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#,
    )
    .expect("failed to write root source");

    let output = run_fresco(&main);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "expected imported function call to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn import_surface_module_compiles() {
    let dir = unique_temp_dir("import_surface_ok");
    let lib = dir.join("materials.fr");
    let main = dir.join("main.fr");

    fs::write(
        &lib,
        r#"surface imported_mat(sp: surf) -> material(unlit) {
  compose {
    base(albedo: #33aaff)
  }
}
"#,
    )
    .expect("failed to write imported surface module");

    fs::write(
        &main,
        r#"import "materials.fr"

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#,
    )
    .expect("failed to write root source");

    let output = run_fresco(&main);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "expected imported surface module to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let wgsl = normalize(&output.stdout);
    assert!(
        wgsl.contains("fn fresco_imported_mat("),
        "expected imported surface function in WGSL output\nstdout:\n{}",
        wgsl
    );
}

#[test]
fn missing_import_reports_diagnostic() {
    let dir = unique_temp_dir("import_missing");
    let main = dir.join("main.fr");

    fs::write(
        &main,
        r#"import "does-not-exist.fr"

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#,
    )
    .expect("failed to write root source");

    let output = run_fresco(&main);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        !output.status.success(),
        "expected missing import to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("cannot read imported module"),
        "expected import resolution diagnostic\nstderr:\n{stderr}"
    );
}

#[test]
fn cyclic_import_reports_diagnostic_chain() {
    let dir = unique_temp_dir("import_cycle");
    let main = dir.join("main.fr");
    let a = dir.join("a.fr");
    let b = dir.join("b.fr");

    fs::write(
        &main,
        r#"import "a.fr"

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#,
    )
    .expect("failed to write root source");

    fs::write(&a, "import \"b.fr\"\n").expect("failed to write module a");
    fs::write(&b, "import \"a.fr\"\n").expect("failed to write module b");

    let output = run_fresco(&main);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        !output.status.success(),
        "expected cyclic imports to fail\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("cyclic import detected"),
        "expected cycle diagnostic headline\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("import cycle:"),
        "expected cycle chain details\nstderr:\n{stderr}"
    );
}

#[test]
fn imported_module_parse_error_reports_imported_filename() {
    let dir = unique_temp_dir("import_parse_error");
    let main = dir.join("main.fr");
    let bad = dir.join("broken.fr");

    fs::write(
        &main,
        r#"import "broken.fr"

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#,
    )
    .expect("failed to write root source");

    fs::write(
        &bad,
        r#"fn oops(amount: f32) -> f32 {
  return
}
"#,
    )
    .expect("failed to write broken module");

    let output = run_fresco(&main);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        !output.status.success(),
        "expected imported parse failure\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("broken.fr"),
        "expected diagnostics to reference imported filename\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("unexpected") || stderr.contains("expected"),
        "expected parser diagnostic details\nstderr:\n{stderr}"
    );
}

#[test]
fn imported_module_semantic_error_reports_imported_filename() {
    let dir = unique_temp_dir("import_semantic_error");
    let main = dir.join("main.fr");
    let bad = dir.join("broken.fr");

    fs::write(
        &main,
        r#"import "broken.fr"

canvas t(uv: coord, time: signal) -> color {
  let _k = bad_fn(1.0)
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#,
    )
    .expect("failed to write root source");

    fs::write(
        &bad,
        r#"fn bad_fn(amount: f32) -> f32 {
  compose {
    circle(at: center, radius: amount * 10px) |> fill(#ffffff)
  }
  amount
}
"#,
    )
    .expect("failed to write broken module");

    let output = run_fresco(&main);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        !output.status.success(),
        "expected imported semantic failure\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );

    let stderr = normalize(&output.stderr);
    assert!(
        stderr.contains("broken.fr"),
        "expected semantic diagnostics to reference imported filename\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("statement `compose` is not supported yet"),
        "expected imported semantic diagnostic details\nstderr:\n{stderr}"
    );
}

#[test]
fn virtual_bundle_compiles_library_function() {
    let mut files = HashMap::from([(
        "engine/engine.fr".into(),
        include_str!("../../../tests/render-policy/engine/engine.fr").into(),
    )]);
    files.insert(
        "lib.fr".to_string(),
        r#"fn sparkle(amount: f32) -> f32 {
  amount
}
"#
        .to_string(),
    );
    files.insert(
        "main.fr".to_string(),
        r#"import "lib.fr"

canvas t(uv: coord, time: signal) -> color {
  let k = sparkle(1.0)
  compose {
    circle(at: center, radius: 20px * k) |> fill(#ffffff)
  }
}
"#
        .to_string(),
    );

    let result = driver::compile_bundle_virtual(&files, "main.fr", false);
    assert!(
        result.is_ok(),
        "expected virtual bundle with library function to compile;\ndiagnostics:\n{:?}",
        result.err()
    );
}

#[test]
fn virtual_bundle_compiles_imported_surface_module() {
    let mut files = HashMap::from([(
        "engine/engine.fr".into(),
        include_str!("../../../tests/render-policy/engine/engine.fr").into(),
    )]);
    files.insert(
        "materials.fr".to_string(),
        r#"surface imported_surface(sp: surf) -> material {
  compose {
    base(albedo: #dddddd, roughness: 0.6)
  }
}
"#
        .to_string(),
    );
    files.insert(
        "main.fr".to_string(),
        r#"import "materials.fr"

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#
        .to_string(),
    );

    let result = driver::compile_bundle_virtual(&files, "main.fr", false);
    assert!(
        result.is_ok(),
        "expected virtual bundle with imported surface module to compile;\ndiagnostics:\n{:?}",
        result.err()
    );

    let compiled = result.expect("expected successful virtual bundle compile");
    assert!(
        compiled.manifest.contains("\"surfaces\"")
            && compiled.manifest.contains("\"imported_surface\""),
        "expected imported surface to be present in manifest\nmanifest:\n{}",
        compiled.manifest
    );
}

#[test]
fn import_module_with_vnext_surface_data_declarations_compiles() {
    let dir = unique_temp_dir("import_vnext_decls_ok");
    let lib = dir.join("models.fr");
    let main = dir.join("main.fr");

    fs::write(
        &lib,
        r#"material_properties base_surface {
  channel albedo: color = #ffffff
  channel emissive: vec3 = (0.0, 0.0, 0.0)
  channel opacity: f32 = 1.0
}

material_properties pbr_surface extends base_surface {
  channel roughness: f32 = 0.5
  channel metallic: f32 = 0.0
}

schema_expression pbr_response for pbr_surface {
  shade: (
    albedo.r + emissive.x,
    albedo.g + emissive.y,
    albedo.b + emissive.z,
    albedo.a * opacity
  )
}

schema_evaluator forward_plus_clustered for pbr_surface {
  shade: (
    albedo.r + emissive.x,
    albedo.g + emissive.y,
    albedo.b + emissive.z,
    albedo.a * opacity
  )
}
"#,
    )
    .expect("failed to write imported declaration module");

    fs::write(
        &main,
        r#"import "models.fr"

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#,
    )
    .expect("failed to write root source");

    let output = run_fresco(&main);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "expected imported vnext declaration module to compile\nstdout:\n{}\nstderr:\n{}",
        normalize(&output.stdout),
        normalize(&output.stderr)
    );
}

#[test]
fn virtual_bundle_missing_import_reports_diagnostic() {
    let mut files = HashMap::from([(
        "engine/engine.fr".into(),
        include_str!("../../../tests/render-policy/engine/engine.fr").into(),
    )]);
    files.insert(
        "main.fr".to_string(),
        r#"import "nonexistent.fr"

canvas t(uv: coord, time: signal) -> color {
  compose {
    circle(at: center, radius: 20px) |> fill(#ffffff)
  }
}
"#
        .to_string(),
    );

    let result = driver::compile_bundle_virtual(&files, "main.fr", false);
    assert!(result.is_err(), "expected missing virtual import to fail");
    let diags = result.err().unwrap();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("nonexistent.fr")),
        "expected import resolution diagnostic referencing missing file\ndiagnostics:\n{messages:?}"
    );
}
