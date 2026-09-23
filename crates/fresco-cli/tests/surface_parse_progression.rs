#[path = "support/common.rs"]
mod common;

use common::{run_fresco, unique_temp_path};
use serde_json::Value;
use std::fs;

#[test]
fn engine_material_attributes_reject_malformed_and_ambiguous_declarations() {
    for (declaration, expected) in [
        (
            "@default_material(1 + 2) material_properties bad { channel x: f32 = 0 }",
            "invalid or duplicate material schema attribute",
        ),
        (
            "@default_material @default_material material_properties bad { channel x: f32 = 0 }",
            "invalid or duplicate material schema attribute",
        ),
        (
            "material_properties bad { channel @blend(custom) x: f32 = 0 }",
            "invalid or duplicate material channel attribute `@blend`",
        ),
        (
            "material_properties bad { channel @blend() x: f32 = 0 }",
            "invalid or duplicate material channel attribute",
        ),
        (
            "material_properties bad { channel @unknown(operation) x: f32 = 0 }",
            "invalid or duplicate material channel attribute",
        ),
        (
            "@default_material material_properties bad { channel x: f32 = 0 }",
            "multiple @default_material declarations are ambiguous",
        ),
    ] {
        let source = format!(
            "{declaration}\nsurface probe(sp: surf) -> material(bad) {{ compose {{ base(x: 1) }} }}"
        );
        let (success, output) = compile(&source);
        assert!(!success, "{source}");
        assert!(output.contains(expected), "{source}: {output}");
    }
}

#[test]
fn surface_vertex_updates_engine_context_fields_without_geometry_names() {
    let source = r#"
struct SampleDomain {
    @semantic(coord) coordinate: vec2
    displacement: f32
}
@context(SampleDomain, sample)
@composition(initialize, coat, weight)
material_properties energy {
    channel power: f32 = sample.displacement
}
surface displaced(sample: SampleDomain) -> material(energy) {
    vertex { displacement: sample.displacement + 1.0 }
    compose { initialize() }
}
"#;
    let (success, output) = compile(source);
    assert!(success, "{output}");
    assert!(
        output.lines().any(
            |line| line.starts_with("fn fresco_surface_vertex_displaced(")
                && line.ends_with(": SampleDomain) -> SampleDomain {")
        ),
        "{output}"
    );
    assert!(
        output.contains(".coordinate"),
        "vertex output must preserve untouched context fields: {output}"
    );
    let (success, output) = compile(&source.replace("vertex { displacement:", "vertex { normal:"));
    assert!(!success);
    assert!(
        output.contains("unknown vertex context field `normal`"),
        "{output}"
    );
}

fn write_temp(src: &str) -> std::path::PathBuf {
    let path = unique_temp_path("surface_parse");
    fs::write(&path, src).expect("failed to write temp file");
    path
}

fn compile_manifest(src: &str) -> (bool, String) {
    let path = write_temp(src);
    // Use run_fresco_with_args but override the default --emit wgsl by passing
    // the input path ourselves via a raw invocation.
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");
    let out = std::process::Command::new(bin)
        .arg(&path)
        .arg("--emit")
        .arg("manifest")
        .output()
        .expect("failed to run fresco binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{stdout}{stderr}");
    (out.status.success(), combined)
}

fn compile(src: &str) -> (bool, String) {
    let path = write_temp(src);
    let out = run_fresco(&path);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

fn parse_manifest(output: &str) -> Value {
    serde_json::from_str(output).expect("manifest output should be valid JSON")
}

fn surface_manifest_by_name<'a>(manifest: &'a Value, name: &str) -> &'a Value {
    manifest
        .get("surfaces")
        .and_then(Value::as_array)
        .and_then(|surfaces| {
            surfaces
                .iter()
                .find(|surface| surface.get("name").and_then(Value::as_str) == Some(name))
        })
        .expect("surface manifest entry should exist")
}

#[test]
fn surface_simple_compiles_to_wgsl() {
    let (success, output) = compile(
        r#"
surface simple(sp: surf) -> material {
    compose {
        base(albedo: #ff0000, roughness: 0.5)
    }
}
"#,
    );
    assert!(success, "simple surface should compile, got:\n{output}");
    assert!(
        output.contains("FrescoMaterial"),
        "missing material struct:\n{output}"
    );
    assert!(
        output.contains("fresco_simple"),
        "missing surface fn:\n{output}"
    );
}

#[test]
fn surface_with_param_compiles() {
    let (success, output) = compile(
        r#"
surface mat(sp: surf) -> material {
    param roughness: f32 = 0.5
    compose {
        base(albedo: #888888, roughness: roughness)
    }
}
"#,
    );
    assert!(success, "surface with param should compile, got:\n{output}");
    assert!(
        output.contains("fresco_mat"),
        "missing surface fn:\n{output}"
    );
}

#[test]
fn surface_unlit_variant_compiles() {
    let (success, output) = compile(
        r#"
surface holo(sp: surf) -> material(unlit) {
    compose {
        base(albedo: #ff0000)
    }
}
"#,
    );
    assert!(success, "surface(unlit) should compile, got:\n{output}");
    assert!(
        output.contains("fresco_holo"),
        "missing surface fn:\n{output}"
    );
}

#[test]
fn surface_surface_point_uv2_member_compiles() {
    let (success, output) = compile(
        r#"
surface uv2_mat(sp: surf) -> material {
    let tiled = checker(scale: 8.0, at: sp.uv2)
    compose {
        base(albedo: rgba(tiled, tiled, tiled, 1.0), roughness: 0.5)
    }
}
"#,
    );
    assert!(
        success,
        "surface using sp.uv2 should compile, got:\n{output}"
    );
}

#[test]
fn surface_wgsl_signature_includes_uv2_argument() {
    let (success, output) = compile(
        r#"
surface uv2_sig(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );
    assert!(success, "surface should compile, got:\n{output}");
    assert!(
        output.contains("fn fresco_surface_response_uv2_sig(")
            && output.contains("fn fresco_uv2_sig("),
        "surface WGSL should emit a distinct response helper and public surface wrapper:\n{output}"
    );
}

#[test]
fn canvas_plus_surface_both_compile() {
    let (success, output) = compile(
        r#"
canvas preview(uv: coord, time: signal) -> color {
    compose {
        fill(#ff8800)
    }
}

surface my_mat(sp: surf) -> material {
    compose {
        base(albedo: #ffffff, roughness: 0.5)
    }
}
"#,
    );
    assert!(
        success,
        "mixed canvas+surface should compile, got:\n{output}"
    );
    assert!(
        output.contains("fresco_preview"),
        "missing canvas fn:\n{output}"
    );
    assert!(
        output.contains("fresco_my_mat"),
        "missing surface fn:\n{output}"
    );
}

// ── Manifest tests ────────────────────────────────────────────────────────────

#[test]
fn surface_manifest_includes_surfaces_array() {
    let (success, manifest) = compile_manifest(
        r#"
surface rock(sp: surf) -> material {
    compose {
        base(albedo: #888888, roughness: 0.8)
    }
}
"#,
    );
    assert!(success, "surface manifest should compile, got:\n{manifest}");
    assert!(
        manifest.contains("\"surfaces\""),
        "manifest missing surfaces key:\n{manifest}"
    );
    assert!(
        manifest.contains("\"rock\""),
        "manifest missing surface name:\n{manifest}"
    );
    assert!(
        manifest.contains("\"fixture_surface\""),
        "manifest missing material_ty:\n{manifest}"
    );
    assert!(
        manifest.contains("\"surface_shader\": \"rock\"")
            && manifest.contains("\"render_policy\": \"default\""),
        "manifest missing explicit surface shader or render policy fields:\n{manifest}"
    );
}

#[test]
fn surface_manifest_unlit_variant() {
    let (success, manifest) = compile_manifest(
        r#"
surface glow(sp: surf) -> material(unlit) {
    compose {
        base(albedo: #ff4400)
    }
}
"#,
    );
    assert!(
        success,
        "unlit surface manifest should compile, got:\n{manifest}"
    );
    assert!(
        manifest.contains("\"unlit\""),
        "manifest should have material_ty unlit:\n{manifest}"
    );
    assert!(
        manifest.contains("\"render_policy\": \"unlit\"")
            && manifest.contains("\"contract_requirements\""),
        "manifest should expose render policy and contract requirements for unlit surfaces:\n{manifest}"
    );
}

#[test]
fn surface_manifest_exposes_params_with_bindings() {
    let (success, manifest) = compile_manifest(
        r#"
surface mat(sp: surf) -> material {
    param roughness: f32 = 0.5
    param tint: color = #ff8800
    compose {
        base(albedo: tint, roughness: roughness)
    }
}
"#,
    );
    assert!(
        success,
        "parametrized surface manifest should compile, got:\n{manifest}"
    );
    assert!(
        manifest.contains("\"roughness\""),
        "manifest missing roughness param:\n{manifest}"
    );
    assert!(
        manifest.contains("\"tint\""),
        "manifest missing tint param:\n{manifest}"
    );
    // bindings should be sequential starting from 0
    assert!(
        manifest.contains("\"binding\": 0"),
        "manifest missing binding 0:\n{manifest}"
    );
    assert!(
        manifest.contains("\"binding\": 1"),
        "manifest missing binding 1:\n{manifest}"
    );
}

#[test]
fn canvas_plus_surface_manifest_has_both() {
    let (success, manifest) = compile_manifest(
        r#"
canvas preview(uv: coord, time: signal) -> color {
    compose { fill(#aaaaaa) }
}
surface my_mat(sp: surf) -> material {
    compose { base(albedo: #ffffff, roughness: 0.5) }
}
"#,
    );
    assert!(success, "mixed manifest should compile, got:\n{manifest}");
    assert!(
        manifest.contains("\"canvases\""),
        "manifest missing canvases:\n{manifest}"
    );
    assert!(
        manifest.contains("\"preview\""),
        "manifest missing canvas name:\n{manifest}"
    );
    assert!(
        manifest.contains("\"surfaces\""),
        "manifest missing surfaces:\n{manifest}"
    );
    assert!(
        manifest.contains("\"my_mat\""),
        "manifest missing surface name:\n{manifest}"
    );
}

#[test]
fn surface_manifest_reports_engine_context_without_inventing_vertex_streams() {
    let (success, manifest) = compile_manifest(
        r#"
surface uv_requirements(sp: surf) -> material {
    let tiled = checker(scale: 8.0, at: sp.uv2)
    compose {
        base(albedo: rgba(tiled, tiled, tiled, 1.0), roughness: 0.5)
    }
}
"#,
    );
    assert!(
        success,
        "surface manifest with uv requirements should compile, got:\n{manifest}"
    );

    assert!(
        manifest.contains("\"surface_requirements\""),
        "manifest missing surface_requirements block:\n{manifest}"
    );
    assert!(
        manifest.contains("\"uv_channels\""),
        "manifest missing uv_channels list:\n{manifest}"
    );
    let parsed: Value = serde_json::from_str(&manifest).unwrap();
    let requirements = &parsed["surfaces"][0]["surface_requirements"];
    assert_eq!(requirements["context_type"], "surf");
    let fields = requirements["context_fields"].as_array().unwrap();
    assert!(fields.iter().any(|field| field["name"] == "uv"
        && field["ty"] == "vec2"
        && field["semantic"] == "coord"));
    assert!(
        fields
            .iter()
            .any(|field| field["name"] == "uv2" && field["ty"] == "vec2")
    );
    assert!(
        !fields
            .iter()
            .any(|field| field["name"] == "uv3" || field["name"] == "uv4")
    );
    assert!(
        requirements["uv_channels"].as_array().unwrap().is_empty(),
        "geometry stream bindings belong to the engine vertex contract"
    );
}

#[test]
fn surface_texture_sampling_reuses_explicit_uv_sample() {
    let (success, output) = compile(
        r#"
texture_type Packed {
    r: x
    g: y
    b: z
}

surface sample_reuse(sp: surf) -> material {
    param tex: texture<Packed> = "assets/textures/wood_floor/albedo.jpg"
    let packed = tex.at(sp.uv)
    compose {
        base(albedo: rgba(packed.x, packed.y, packed.z, 1.0), roughness: 0.5)
    }
}
"#,
    );
    assert!(
        success,
        "surface with explicit sample should compile, got:\n{output}"
    );

    let sample_count = output.matches("textureSample(").count();
    assert_eq!(
        sample_count, 1,
        "expected one textureSample call for repeated channels at the same explicit uv, got {sample_count}:\n{output}"
    );
}

#[test]
fn surface_vertex_block_lowers_to_wgsl_vertex_helper() {
    let (success, output) = compile(
        r#"
surface bend(sp: surf) -> material {
    vertex {
        position: (sp.position.x, sp.position.y + sin(sp.uv.x * 6.28318) * 0.1, sp.position.z)
        normal: sp.normal
    }

    compose {
        base(albedo: #88bbff, roughness: 0.4)
    }
}
"#,
    );

    assert!(
        success,
        "surface with vertex block should compile, got:\n{output}"
    );
    assert!(
        output.contains("fn fresco_surface_vertex_bend("),
        "surface vertex helper entry point missing in WGSL output:\n{output}"
    );
    assert!(
        output
            .lines()
            .any(|line| line.starts_with("fn fresco_surface_vertex_bend(")
                && line.ends_with(": surf) -> surf {")),
        "surface vertex helper must update the engine context:\n{output}"
    );
}

#[test]
fn surface_manifest_emits_vertex_stage_requirements() {
    let (success, manifest) = compile_manifest(
        r#"
surface bend(sp: surf) -> material {
    vertex {
        position: (sp.position.x, sp.position.y + 0.05, sp.position.z)
    }

    compose {
        base(albedo: #ffffff, roughness: 0.5)
    }
}
"#,
    );

    assert!(
        success,
        "surface vertex manifest should compile, got:\n{manifest}"
    );
    assert!(
        manifest.contains("\"vertex_stage\""),
        "manifest missing vertex_stage requirements block:\n{manifest}"
    );
    assert!(
        manifest.contains("\"entry_point\": \"fresco_surface_vertex_bend\""),
        "manifest missing surface vertex entry point:\n{manifest}"
    );
    assert!(
        serde_json::from_str::<Value>(&manifest).unwrap()["surfaces"][0]["surface_requirements"]["vertex_stage"]
            ["fields"]
            == serde_json::json!(["position"]),
        "manifest missing authored context field:\n{manifest}"
    );
}

#[test]
fn surface_custom_channels_lower_to_named_material_fields() {
    let (success, output) = compile(
        r#"
material_properties nickspbr {
    channel albedo: color = #ffffff
    channel roughness: f32 = 1.0
    channel coat_weight: f32 = 0.0
    channel sheen_tint: vec3 = (0.0, 0.0, 0.0)
}

surface nickspbr(sp: surf) -> material(nickspbr) {
    compose {
        base(
            albedo: #88bbff,
            roughness: 0.4,
            coat_weight: 0.35,
            sheen_tint: (0.1, 0.2, 0.3)
        )
    }
}
"#,
    );

    assert!(
        success,
        "surface with custom channels should compile, got:\n{output}"
    );
    assert!(
        output.contains("coat_weight: f32") && output.contains("sheen_tint: vec3<f32>"),
        "expected lowered material payload to expose named custom fields in WGSL output:\n{output}"
    );
}

#[test]
fn surface_manifest_emits_custom_channel_mapping() {
    let (success, manifest) = compile_manifest(
        r#"
material_properties openpbr_like_model {
    channel albedo: color = #ffffff
    channel roughness: f32 = 1.0
    channel coat_weight: f32 = 0.0
    channel anisotropy: f32 = 0.0
    channel transmission_color: vec4 = (1.0, 1.0, 1.0, 0.0)
}

surface openpbr_like(sp: surf) -> material(openpbr_like_model) {
    compose {
        base(
            albedo: #ffffff,
            roughness: 0.5,
            coat_weight: 0.25,
            anisotropy: 0.7,
            transmission_color: (0.8, 0.9, 1.0, 0.4)
        )
    }
}
"#,
    );

    assert!(
        success,
        "surface manifest with custom channels should compile, got:\n{manifest}"
    );
    assert!(
        manifest.contains("\"custom_channels\""),
        "manifest missing custom_channels mapping:\n{manifest}"
    );
    assert!(
        manifest.contains("\"name\": \"coat_weight\"")
            && manifest.contains("\"name\": \"anisotropy\"")
            && manifest.contains("\"name\": \"transmission_color\""),
        "manifest missing custom channel names:\n{manifest}"
    );
    assert!(
        manifest.contains("\"field\": \"coat_weight\"")
            && manifest.contains("\"field\": \"anisotropy\""),
        "manifest missing named channel field mapping:\n{manifest}"
    );
}

#[test]
fn surface_manifest_supports_named_material_model_identifier() {
    let (success, manifest) = compile_manifest(
        r#"
material_properties nickspbr {
    channel albedo: color = #ffffff
    channel roughness: f32 = 1.0
    channel base_color: color = #ffffff
    channel coat_weight: f32 = 0.0
    channel sheen_color: vec3 = (0.0, 0.0, 0.0)
}

surface nick_model(sp: surf) -> material(nickspbr) {
    compose {
        base(
            albedo: #dde8ff,
            roughness: 0.3,
            coat_weight: 0.65,
            sheen_color: (0.2, 0.35, 0.9)
        )
    }
}
"#,
    );

    assert!(
        success,
        "named material model surface should compile, got:\n{manifest}"
    );
    assert!(
        manifest.contains("\"material_ty\": \"nickspbr\""),
        "manifest should preserve named material model identifier:\n{manifest}"
    );
}

#[test]
fn material_model_duplicate_declaration_reports_error() {
    let (success, output) = compile(
        r#"
material_properties nickspbr {
    channel albedo: color = #ffffff
    channel roughness: f32 = 1.0
    channel coat_weight: f32 = 0.0
}

material_properties nickspbr {
    channel albedo: color = #ffffff
    channel roughness: f32 = 1.0
    channel coat_weight: f32 = 0.25
}

surface sample_value(sp: surf) -> material(nickspbr) {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        !success,
        "duplicate material_properties should fail:\n{output}"
    );
    assert!(
        output.contains("duplicate material_properties declaration `nickspbr`"),
        "missing duplicate material_properties diagnostic:\n{output}"
    );
}

#[test]
fn material_model_channel_type_and_default_mismatch_report_errors() {
    let (success, output) = compile(
        r#"
material_properties broken {
    channel a: shape
    channel b: vec3 = 1.0
}

surface sample_value(sp: surf) -> material(broken) {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        !success,
        "invalid material_properties should fail:\n{output}"
    );
    assert!(
        output.contains("unsupported material channel type `shape`"),
        "missing unsupported channel type diagnostic:\n{output}"
    );
    assert!(
        output.contains(
            "default for material_properties channel `broken.b` does not match declared type `vec3`"
        ),
        "missing channel default type mismatch diagnostic:\n{output}"
    );
}

#[test]
fn material_model_spatial_channel_type_compiles() {
    let (_success, output) = compile(
        r#"
material_properties standard_like {
    channel albedo: color = #ffffff
    channel opacity: f32 = 1.0
    channel normal: vec3 in world = (0.0, 0.0, 1.0)
}

schema_evaluator standard_like_lit for standard_like {
    shade: (
        albedo.r + normal.x * 0.0,
        albedo.g + normal.y * 0.0,
        albedo.b + normal.z * 0.0,
        opacity
    )
}

surface sample_value(sp: surf) -> material(standard_like) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        !output.contains("unsupported material channel type `vec3 in world`"),
        "spatially tagged vec3 channel type should be accepted by checker:\n{output}"
    );
}

#[test]
fn material_model_inherited_spatial_channel_override_requires_transform() {
    let (success, output) = compile(
        r#"
material_properties base_surface {
    channel albedo: color = #ffffff
    channel opacity: f32 = 1.0
    channel normal: vec3 in world = (0.0, 0.0, 1.0)
}

material_properties derived_surface extends base_surface {
    channel normal: vec3 in object = (0.0, 0.0, 1.0)
}

schema_evaluator derived_surface_lit for derived_surface {
    shade: (
        albedo.r + normal.x * 0.0,
        albedo.g + normal.y * 0.0,
        albedo.b + normal.z * 0.0,
        opacity
    )
}

surface sample_value(sp: surf) -> material(derived_surface) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        !success,
        "override across labeled spaces should fail without explicit transform:\n{output}"
    );
    assert!(
        output.contains("no implicit space transform is available"),
        "expected transform-required diagnostic for spatial override:\n{output}"
    );
}

#[test]
fn material_model_inherited_unlabeled_channel_can_override_to_spatial() {
    let (_success, output) = compile(
        r#"
material_properties base_surface {
    channel albedo: color = #ffffff
    channel opacity: f32 = 1.0
    channel normal: vec3 = (0.0, 0.0, 1.0)
}

material_properties derived_surface extends base_surface {
    channel normal: vec3 in object = (0.0, 0.0, 1.0)
}

schema_evaluator derived_surface_lit for derived_surface {
    shade: (
        albedo.r + normal.x * 0.0,
        albedo.g + normal.y * 0.0,
        albedo.b + normal.z * 0.0,
        opacity
    )
}

surface sample_value(sp: surf) -> material(derived_surface) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        !output.contains("no implicit space transform is available"),
        "unlabeled vector should bridge to labeled vector without transform diagnostic:\n{output}"
    );
}

#[test]
fn surface_named_material_model_must_exist() {
    let (success, output) = compile(
        r#"
surface sample_value(sp: surf) -> material(openpbr) {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        !success,
        "unknown material_properties reference should fail:\n{output}"
    );
    assert!(
        output.contains("references unknown material_properties `openpbr`"),
        "missing unknown material_properties diagnostic:\n{output}"
    );
}

#[test]
fn schema_evaluator_shade_lowers_to_wgsl_and_manifest_entry() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel rim: f32 = 0.0
}

schema_evaluator toon_lit for toon {
    shade: (
        albedo.r + emissive.x + rim,
        albedo.g + emissive.y + rim,
        albedo.b + emissive.z + rim,
        albedo.a * opacity
    )
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(
            albedo: #88bbff,
            rim: 0.25
        )
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "surface using authored model shade should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fn fresco_evaluation_shader_toon_surface("),
        "expected lowered authored material shader helper in WGSL output:\n{wgsl}"
    );

    let (manifest_success, manifest) = compile_manifest(
        r#"
 material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel rim: f32 = 0.0
}

schema_evaluator toon_lit for toon {
    shade: (
        albedo.r + emissive.x + rim,
        albedo.g + emissive.y + rim,
        albedo.b + emissive.z + rim,
        albedo.a * opacity
    )
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(
            albedo: #88bbff,
            rim: 0.25
        )
    }
}
"#,
    );

    assert!(manifest_success, "manifest compile failed:\n{manifest}");
    assert!(
        manifest.contains("\"evaluation_shader_entry\": \"fresco_evaluation_shader_toon_surface\""),
        "expected manifest evaluation_shader_entry for authored lighting shader:\n{manifest}"
    );
    assert!(
        manifest.contains("\"schema_evaluator\": \"toon_lit\""),
        "expected manifest schema_evaluator name:\n{manifest}"
    );
    assert!(
        manifest.contains("\"material_properties\": \"toon\"")
            && manifest.contains("\"surface_shader\": \"toon_surface\""),
        "expected manifest to expose explicit surface model identity fields:\n{manifest}"
    );
}

#[test]
fn schema_program_explicit_resolve_call_lowers_to_helper() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel rim: f32 = 0.0
}

schema_program toon_model for toon {
    fn direct() -> vec4 {
        return (
            self.albedo.r + self.rim,
            self.albedo.g + self.rim,
            self.albedo.b + self.rim,
            self.albedo.a * self.opacity
        )
    }

    fn indirect() -> vec4 {
        return (
            self.emissive.x,
            self.emissive.y,
            self.emissive.z,
            1.0
        )
    }

    fn resolve() -> vec4 {
        return (
            self.albedo.r + self.emissive.x,
            self.albedo.g + self.emissive.y,
            self.albedo.b + self.emissive.z,
            self.albedo.a * self.opacity
        )
    }
    output: resolve()
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(
            albedo: #88bbff,
            rim: 0.25
        )
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "surface using schema_program should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fn fresco_evaluation_shader_toon_surface("),
        "expected lowered lighting helper entry in WGSL output:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fresco_schema_value.emissive.x")
            && wgsl.contains("fresco_schema_value.albedo.r"),
        "expected resolve expression to feed emitted lighting helper:\n{wgsl}"
    );
}

#[test]
fn schema_program_authored_expression_combines_function_results() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel rim: f32 = 0.0
}

schema_program toon_model for toon {
    fn direct() -> vec4 {
        return (
            self.albedo.r + self.rim,
            self.albedo.g + self.rim,
            self.albedo.b + self.rim,
            self.albedo.a * self.opacity
        )
    }

    fn indirect() -> vec4 {
        return (
            self.emissive.x,
            self.emissive.y,
            self.emissive.z,
            1.0
        )
    }
    output: vec4(direct().rgb + indirect().rgb, direct().a)
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(
            albedo: #88bbff,
            rim: 0.25
        )
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "surface using schema_program without resolve should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fn fresco_evaluation_shader_toon_surface("),
        "expected lowered lighting helper entry in WGSL output:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fresco_schema_value.albedo.r + fresco_schema_value.rim")
            && wgsl.contains("fresco_schema_value.emissive.x"),
        "expected helper to combine direct and indirect shading contributions:\n{wgsl}"
    );
}

#[test]
fn schema_program_explicit_output_is_independent_of_additive_pipeline() {
    let (success, output) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_program toon_model for toon {
    fn direct() -> vec4 {
        return (self.albedo.r, self.albedo.g, self.albedo.b, self.albedo.a)
    }
    fn indirect() -> vec4 {
        return (self.emissive.x, self.emissive.y, self.emissive.z, 1.0)
    }
    fn resolve() -> vec4 {
        return (self.albedo.r, self.albedo.g, self.albedo.b, self.albedo.a * self.opacity)
    }
    output: resolve()
}

pass fwd_add for toon {
    stage: raster
    draw: per_binding(light)
    blend: additive
}

pipeline(lighting) forward for toon {
    fwd_add
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        success,
        "explicit output must compile independently of additive scheduling:\n{output}"
    );
    assert!(
        output.contains("fn fresco_evaluation_shader_toon_surface(")
            && output.contains("fresco_schema_value.opacity"),
        "expected explicitly selected result:\n{output}"
    );
}

#[test]
fn schema_program_explicit_output_can_select_one_function() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_program toon_model for toon {
    fn direct() -> vec4 {
        return (self.albedo.r + 111.0, self.albedo.g, self.albedo.b, self.albedo.a * self.opacity)
    }

    fn indirect() -> vec4 {
        return (self.emissive.x + 222.0, self.emissive.y, self.emissive.z, 1.0)
    }
    output: direct()
}

pass fwd_base for toon {
    stage: raster
    draw: per_object
    blend: opaque
}

pass fwd_add for toon {
    stage: raster
    draw: per_binding(light)
    blend: additive
}

pipeline(lighting) forward for toon {
    fwd_base
    fwd_add
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "forward topology with schema_program should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("111") && !wgsl.contains("222"),
        "expected explicit output to select the direct function:\n{wgsl}"
    );
}

#[test]
fn schema_program_explicit_output_can_combine_functions() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_program toon_model for toon {
    fn direct() -> vec4 {
        return (self.albedo.r + 13.0, self.albedo.g, self.albedo.b, self.albedo.a * self.opacity)
    }

    fn indirect() -> vec4 {
        return (self.emissive.x + 29.0, self.emissive.y, self.emissive.z, 1.0)
    }
    output: vec4(direct().rgb + indirect().rgb, direct().a)
}

pass cluster_cull for toon {
    stage: compute
    writes: light_bins
}

pass fp_shade for toon {
    stage: raster
    draw: per_object
    blend: opaque
    reads: light_bins
}

pipeline(lighting) forward_plus for toon {
    cluster_cull
    fp_shade
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "forward_plus topology with schema_program should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("13") && wgsl.contains("29"),
        "expected explicit output to combine function results:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_permutations_expand_specialized_wgsl_helpers() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel rim: f32 = 0.0
}

schema_evaluator toon_lit for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
    }

    permutations {
        light_count: 1..2
    }

    specialize as "toon_${light_count}"

    shade: (
        albedo.r + emissive.x + rim,
        albedo.g + emissive.y + rim,
        albedo.b + emissive.z + rim,
        albedo.a * opacity
    )
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(
            albedo: #88bbff,
            rim: 0.25
        )
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "surface using authored lighting permutations should compile, got:\n{wgsl}"
    );
    assert!(
        !wgsl.contains("fn fresco_evaluation_shader_toon_surface("),
        "specialization must not manufacture an unspecialized fallback:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fn toon_1") && wgsl.contains("fn toon_2"),
        "expected specialized lighting helpers for each permutation value:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_variants_generate_specialized_entries_for_permutation_branches() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel rim: f32 = 0.0
}

schema_evaluator dynamic_lit for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
    }

    permutations {
        light_count: 1|2
        light_kind: directional|point
    }

    specialize as "dynamic_${light_kind}_${light_count}"

    variant directional when light_kind == directional: (
        albedo.r + emissive.x + rim + light_count,
        albedo.g + emissive.y + rim + light_count,
        albedo.b + emissive.z + rim + light_count,
        albedo.a * opacity
    )

    variant point when light_kind == point: (
        albedo.r + emissive.x + rim + (light_count * 0.25),
        albedo.g + emissive.y + rim + (light_count * 0.25),
        albedo.b + emissive.z + rim + (light_count * 0.25),
        albedo.a * opacity
    )
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(
            albedo: #88bbff,
            rim: 0.25
        )
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "branching lighting model should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("fn dynamic_directional_1") && wgsl.contains("fn dynamic_point_2"),
        "expected specialized lighting helpers for each branch permutation:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_variants_lower_to_variant_specific_specialized_helpers() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator dynamic_lit for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
        runtime: light_positions, light_colors
    }

    permutations {
        light_count: 1|2
        kind: directional|point
    }

    specialize as "dynamic_${kind}_${light_count}"

    variant directional when kind == directional: (
        albedo.r + 100.0,
        albedo.g + 101.0,
        albedo.b + 102.0,
        albedo.a * opacity
    )

    variant point when kind == point: (
        albedo.r + 200.0,
        albedo.g + 201.0,
        albedo.b + 202.0,
        albedo.a * opacity
    )
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "variant-based lighting shader should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("+ 100.0)") && wgsl.contains("+ 200.0)"),
        "expected specialized lighting helpers to preserve variant-specific constants:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_typed_contract_entries_lower_to_declared_wgsl_types() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator typed_contract_lit for toon {
    contract {
        inputs: lighting_ctx: vec3, shading_ctx: f32, light_sample: f32
        runtime: light_positions: vec3, light_colors: vec3
    }

    permutations {
        light_count: 1
    }

    specialize as "typed_${light_count}"

    shade: (
        albedo.r + emissive.x + opacity,
        albedo.g + emissive.y + opacity,
        albedo.b + emissive.z + opacity,
        albedo.a * opacity
    )
}

surface typed_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "typed contract shader should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("lighting_ctx: vec3<f32>"),
        "expected typed contract input in WGSL signature:\n{wgsl}"
    );
    assert!(
        wgsl.contains("shading_ctx: f32"),
        "expected typed contract input in WGSL signature:\n{wgsl}"
    );
    assert!(
        wgsl.contains("light_positions: vec3<f32>"),
        "expected typed runtime binding in WGSL signature:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_input_block_accepts_array_runtime_bindings() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator input_block_lit for toon {
    input {
        light_positions: array<vec3, light_count>
    }

    permutations {
        light_count: 2|4
    }

    specialize as "input_${light_count}"

    shade: (
        albedo.r + emissive.x + light_positions[0].x + opacity,
        albedo.g + emissive.y + light_positions[0].y + opacity,
        albedo.b + emissive.z + light_positions[0].z + opacity,
        albedo.a * opacity
    )
}

surface input_block_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "input-block lighting shader should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("light_positions"),
        "expected input-block shader to preserve the runtime binding:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_contract_array_sizes_expand_with_permutation_values() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator array_contract_lit for toon {
    contract {
        inputs: lighting_ctx: vec3, shading_ctx: f32, light_sample: f32
        runtime: light_positions: array<vec3, light_count>
    }

    permutations {
        light_count: 2|4
    }

    specialize as "array_${light_count}"

    shade: (
        albedo.r + emissive.x + opacity,
        albedo.g + emissive.y + opacity,
        albedo.b + emissive.z + opacity,
        albedo.a * opacity
    )
}

surface array_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "array-shaped contract shader should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("array<vec3<f32>, 2>"),
        "expected array contract size to expand to the 2-value permutation:\n{wgsl}"
    );
    assert!(
        wgsl.contains("array<vec3<f32>, 4>"),
        "expected array contract size to expand to the 4-value permutation:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_contract_inputs_and_runtime_names_lower_to_contract_arguments() {
    let (wgsl_success, wgsl) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator contract_lit for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
        runtime: light_positions, light_colors
    }

    permutations {
        light_count: 1
    }

    specialize as "contract_${light_count}"

    shade: (
        albedo.r + emissive.x + opacity,
        albedo.g + emissive.y + opacity,
        albedo.b + emissive.z + opacity,
        albedo.a * opacity
    )
}

surface contract_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        wgsl_success,
        "contract-based lighting shader should compile, got:\n{wgsl}"
    );
    assert!(
        wgsl.contains("lighting_ctx: f32"),
        "expected contract input in WGSL signature:\n{wgsl}"
    );
    assert!(
        wgsl.contains("shading_ctx: f32"),
        "expected contract input in WGSL signature:\n{wgsl}"
    );
    assert!(
        wgsl.contains("light_sample: f32"),
        "expected contract input in WGSL signature:\n{wgsl}"
    );
    assert!(
        wgsl.contains("light_positions: f32"),
        "expected runtime binding in WGSL signature:\n{wgsl}"
    );
    assert!(
        wgsl.contains("light_colors: f32"),
        "expected runtime binding in WGSL signature:\n{wgsl}"
    );
}

#[test]
fn schema_evaluator_expression_is_exposed_via_manifest_contract_and_entrypoints() {
    let (success, manifest) = compile_manifest(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator toon_lit for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
        runtime: light_positions, light_colors
    }

    permutations {
        light_count: 1|2
    }

    specialize as "toon_${light_count}"

    shade: (
        albedo.r + emissive.x + opacity,
        albedo.g + emissive.y + opacity,
        albedo.b + emissive.z + opacity,
        albedo.a * opacity
    )
}

surface toon_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        success,
        "lighting shader expression should compile to manifest, got:\n{manifest}"
    );
    assert!(
        manifest.contains("\"surface_shader_entry\""),
        "expected manifest to expose the surface shader entrypoint:\n{manifest}"
    );
    assert!(
        manifest.contains("\"evaluation_contract\""),
        "expected manifest to expose the lighting contract:\n{manifest}"
    );
    assert!(
        manifest.contains("\"evaluation_contract\"")
            && manifest.contains("\"inputs\": [\n          \"lighting_ctx\",\n          \"shading_ctx\",\n          \"light_sample\"
        ]"),
        "expected manifest to preserve the contract inputs:\n{manifest}"
    );
}

#[test]
fn point_and_cone_light_sources_are_supported_in_contracts() {
    let (success, manifest) = compile_manifest(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator point_light for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
        runtime: light_positions, light_colors, light_ranges
    }

    permutations {
        light_count: 2
    }

    specialize as "point_${light_count}"

    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

surface point_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        success,
        "point/cone light contract should compile to manifest, got:\n{manifest}"
    );
    assert!(
        manifest.contains("\"evaluation_contract\"")
            && manifest.contains("\"runtime\": [\n          \"light_positions\",\n          \"light_colors\",\n          \"light_ranges\"\n        ]"),
        "expected manifest to preserve point-light contract metadata:\n{manifest}"
    );
}

#[test]
fn clustered_evaluation_contract_emits_runtime_manifest_metadata() {
    let (success, manifest) = compile_manifest(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator clustered_toon for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
        runtime: camera, scene, cluster_table
    }

    permutations {
        light_count: 8|16
        tile_size: 8|16
        cluster_depth_slices: 16|32
    }

    specialize as "clustered_${tile_size}_${cluster_depth_slices}_${light_count}"

    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

surface clustered_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        success,
        "clustered contract surface should compile to manifest, got:\n{manifest}"
    );
    let manifest_json = parse_manifest(&manifest);
    let clustered_surface = surface_manifest_by_name(&manifest_json, "clustered_surface");
    assert!(
        manifest.contains("\"evaluation_contract\"")
            && manifest.contains("\"runtime\": [\n          \"camera\",\n          \"scene\",\n          \"cluster_table\"
        ]"),
        "expected clustered contract metadata in manifest:\n{manifest}"
    );
    assert!(
        manifest.contains("\"evaluation_variants\"")
            && manifest.contains("\"axis\": \"tile_size\"")
            && manifest.contains("\"axis\": \"cluster_depth_slices\""),
        "expected generated lighting variants with permutation bindings in manifest:\n{manifest}"
    );

    let variant_entries = clustered_surface
        .get("evaluation_variants")
        .and_then(Value::as_array)
        .expect("clustered surface should include evaluation_variants")
        .iter()
        .map(|variant| {
            variant
                .get("entry")
                .and_then(Value::as_str)
                .expect("variant entry should be a string")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        variant_entries,
        vec![
            "clustered_16_16_16_",
            "clustered_16_16_8_",
            "clustered_16_32_16_",
            "clustered_16_32_8_",
            "clustered_8_16_16_",
            "clustered_8_16_8_",
            "clustered_8_32_16_",
            "clustered_8_32_8_",
        ],
        "expected deterministic lexical ordering for emitted clustered lighting variants"
    );

    let first_variant_bindings = clustered_surface
        .get("evaluation_variants")
        .and_then(Value::as_array)
        .and_then(|variants| variants.first())
        .and_then(|variant| variant.get("bindings"))
        .and_then(Value::as_array)
        .expect("first clustered variant should include bindings");

    let binding_axes = first_variant_bindings
        .iter()
        .map(|binding| {
            binding
                .get("axis")
                .and_then(Value::as_str)
                .expect("binding axis should be a string")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        binding_axes,
        vec!["light_count", "tile_size", "cluster_depth_slices"],
        "expected stable axis ordering in variant bindings"
    );
}

#[test]
fn forward_plus_manifest_variant_order_is_deterministic() {
    let (success, manifest) = compile_manifest(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator fp_toon for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
    }

    permutations {
        light_count: 4|8
        tile_size: 8|16
        cluster_depth_slices: 16|32
    }

    specialize as "forward_plus_${tile_size}_${cluster_depth_slices}_${light_count}"

    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

surface fp_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        success,
        "forward_plus contract surface should compile to manifest, got:\n{manifest}"
    );

    let manifest_json = parse_manifest(&manifest);
    let surface = surface_manifest_by_name(&manifest_json, "fp_surface");
    let entries = surface
        .get("evaluation_variants")
        .and_then(Value::as_array)
        .expect("forward_plus surface should include evaluation_variants")
        .iter()
        .map(|variant| {
            variant
                .get("entry")
                .and_then(Value::as_str)
                .expect("variant entry should be a string")
        })
        .collect::<Vec<_>>();

    assert_eq!(
        entries,
        vec![
            "forward_plus_16_16_4_",
            "forward_plus_16_16_8_",
            "forward_plus_16_32_4_",
            "forward_plus_16_32_8_",
            "forward_plus_8_16_4_",
            "forward_plus_8_16_8_",
            "forward_plus_8_32_4_",
            "forward_plus_8_32_8_",
        ],
        "expected deterministic lexical ordering for emitted forward_plus lighting variants"
    );
}

#[test]
fn pipeline_without_evaluator_emits_data_without_lighting_diagnostic() {
    let path = write_temp(
        r#"
material_properties standard {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel roughness: f32 = 0.5
    channel metallic: f32 = 0.0
}

pass fwd_base for standard {
    stage: raster
}

pass fwd_add for standard {
    stage: raster
}

pipeline(lighting) forward for standard {
    fwd_base
    fwd_add
}

surface piped_surface(sp: surf) -> material(standard) {
    compose {
        base(albedo: #88bbff, roughness: 0.4, metallic: 0.1)
    }
}
"#,
    );
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");
    let out = std::process::Command::new(bin)
        .arg(&path)
        .arg("--emit")
        .arg("manifest")
        .output()
        .expect("failed to run fresco binary");

    let success = out.status.success();
    let manifest_stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let manifest_stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{manifest_stdout}{manifest_stderr}");

    assert!(
        success,
        "pipeline-driven lighting bootstrap should compile to manifest, got:\n{combined}"
    );

    let manifest_json = parse_manifest(&manifest_stdout);
    let surface = surface_manifest_by_name(&manifest_json, "piped_surface");
    assert!(
        surface
            .get("evaluation_shader_entry")
            .is_none_or(Value::is_null)
    );
    assert!(
        surface
            .get("evaluation_shader_source")
            .is_none_or(Value::is_null)
    );
    assert!(
        !manifest_stderr.contains("W_SURFACE_LIGHTING_UNAUTHORED"),
        "{manifest_stderr}"
    );
    assert!(
        !manifest_stderr.contains("only material outputs are emitted"),
        "{manifest_stderr}"
    );
}

#[test]
fn authored_program_uses_explicit_output_independent_of_passes() {
    let path = write_temp(
        r#"
material_properties standard {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel roughness: f32 = 0.5
    channel metallic: f32 = 0.0
}

schema_program authored_standard for standard {
    fn direct() -> vec4 {
        return (self.albedo.r + 111.0, self.albedo.g, self.albedo.b, 1.0)
    }
    fn indirect() -> vec4 {
        return (222.0, 0.0, 0.0, 1.0)
    }
    output: direct()
}

pass base_geometry for standard {
    stage: raster
    draw: per_object
    blend: opaque
}

pass light_accumulation for standard {
    stage: raster
    draw: per_binding(light)
    blend: additive
}

pipeline(lighting) semantic_forward for standard {
    base_geometry
    light_accumulation
}

surface semantic_surface(sp: surf) -> material(standard) {
    compose {
        base(albedo: #88bbff, roughness: 0.4, metallic: 0.1)
    }
}
"#,
    );
    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");
    let out = std::process::Command::new(&bin)
        .arg(&path)
        .arg("--emit")
        .arg("manifest")
        .output()
        .expect("failed to run fresco binary");

    let success = out.status.success();
    let manifest_stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let manifest_stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{manifest_stdout}{manifest_stderr}");

    assert!(
        success,
        "semantic forward pipeline should compile to manifest, got:\n{combined}"
    );

    let manifest_json = parse_manifest(&manifest_stdout);
    let surface = surface_manifest_by_name(&manifest_json, "semantic_surface");
    assert_eq!(
        surface
            .get("evaluation_shader_entry")
            .and_then(Value::as_str),
        Some("fresco_evaluation_shader_semantic_surface"),
        "expected semantic topology bootstrap lighting entry in manifest:\n{manifest_stdout}\n---\nstderr:\n{manifest_stderr}"
    );
    assert!(
        surface
            .get("evaluation_shader_source")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("schema_program:authored_standard"),
        "expected authored shader provenance:\n{manifest_stdout}\n---\nstderr:\n{manifest_stderr}"
    );
    let wgsl_output = std::process::Command::new(&bin)
        .arg(&path)
        .args(["--emit", "wgsl"])
        .output()
        .expect("compile authored lighting");
    let wgsl = String::from_utf8(wgsl_output.stdout).expect("WGSL text");
    assert!(
        wgsl_output.status.success(),
        "{}",
        String::from_utf8_lossy(&wgsl_output.stderr)
    );
    assert!(
        wgsl.contains("111") && !wgsl.contains("222"),
        "forward semantics must select authored direct response: {wgsl}"
    );
}

#[test]
fn pipeline_manifest_exposes_semantic_summary_and_pass_semantics() {
    let path = write_temp(
        r#"
material_properties standard {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel roughness: f32 = 0.5
    channel metallic: f32 = 0.0
}

pass base_geometry for standard {
    stage: raster
    draw: per_object
    blend: opaque
    writes: gbuffer_albedo
}

pass light_accumulation for standard {
    stage: raster
    draw: per_binding(light)
    blend: additive
    reads: gbuffer_albedo
}

pipeline(lighting) semantic_forward for standard {
    base_geometry
    light_accumulation
}

surface semantic_surface(sp: surf) -> material(standard) {
    compose {
        base(albedo: #88bbff, roughness: 0.4, metallic: 0.1)
    }
}
"#,
    );

    let bin = std::env::var("CARGO_BIN_EXE_fresco")
        .expect("CARGO_BIN_EXE_fresco is not set; run as an integration test");
    let out = std::process::Command::new(bin)
        .arg(&path)
        .arg("--emit")
        .arg("manifest")
        .output()
        .expect("failed to run fresco binary");

    let success = out.status.success();
    let manifest_stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let manifest_stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = format!("{manifest_stdout}{manifest_stderr}");

    assert!(
        success,
        "pipeline semantic manifest sample should compile, got:\n{combined}"
    );

    let manifest_json = parse_manifest(&manifest_stdout);
    let pipelines = manifest_json
        .get("pipelines")
        .and_then(Value::as_array)
        .expect("manifest should expose pipelines array");
    let pipeline = pipelines
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some("semantic_forward"))
        .expect("semantic_forward pipeline should be present in manifest");

    assert!(
        pipeline
            .get("semantic_summary")
            .and_then(|summary| summary.get("topology"))
            .is_none()
    );
    assert_eq!(pipeline["semantic_summary"]["resource_flow_edges"], 1);

    let pass_semantics = pipeline
        .get("pass_semantics")
        .and_then(Value::as_array)
        .expect("pipeline should include pass semantic entries");
    assert!(
        pass_semantics
            .iter()
            .any(
                |entry| entry.get("name").and_then(Value::as_str) == Some("base_geometry")
                    && entry.get("stage").and_then(Value::as_str) == Some("raster")
                    && entry.get("draw").and_then(Value::as_str) == Some("per_object")
            ),
        "expected base_geometry pass semantics in manifest:\n{manifest_stdout}\n---\nstderr:\n{manifest_stderr}"
    );
    assert!(
        pass_semantics
            .iter()
            .any(
                |entry| entry.get("name").and_then(Value::as_str) == Some("light_accumulation")
                    && entry.get("blend").and_then(Value::as_str) == Some("additive")
            ),
        "expected light_accumulation pass semantics in manifest:\n{manifest_stdout}\n---\nstderr:\n{manifest_stderr}"
    );
}

#[test]
fn surface_requires_its_explicitly_selected_evaluator() {
    let (success, output) = compile(
        r#"
@evaluator(required_pipeline)
material_properties standard {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
    channel roughness: f32 = 0.5
    channel metallic: f32 = 0.0
}

pass helper_compute for standard {
    stage: compute
}

pipeline(compute) helper for standard {
    helper_compute
}

surface missing_pipeline(sp: surf) -> material(standard) {
    compose {
        base(albedo: #88bbff, roughness: 0.4, metallic: 0.1)
    }
}
"#,
    );

    assert!(
        !success,
        "surface with lit material and no pipeline should fail, got:\n{output}"
    );
    assert!(
        output.contains("material selects missing evaluator"),
        "expected missing evaluator diagnostic:\n{output}"
    );
}

#[test]
fn forward_plus_contract_requires_tile_and_cluster_axes() {
    let (success, output) = compile(
        r#"
material_properties toon {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator broken_forward_plus for toon {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
    }

    permutations {
        light_count: 8
    }

    specialize as "broken_${light_count}"

    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

surface broken_surface(sp: surf) -> material(toon) {
    compose {
        base(albedo: #88bbff)
    }
}
"#,
    );

    assert!(
        success,
        "forward_plus contract should still compile when permutations are treated generically:\n{output}"
    );
    assert!(
        !output.contains("must declare a `tile_size` permutation axis")
            && !output.contains("must declare a `cluster_depth_slices` permutation axis"),
        "expected no special-case permutation-axis diagnostics:\n{output}"
    );
}

#[test]
fn surface_channel_contract_rejects_undeclared_channels() {
    let (success, output) = compile(
        r#"
material_properties simple_model {
    channel coat_weight: f32 = 0.0
}

surface bad_surface(sp: surf) -> material(simple_model) {
    compose {
        base(
            albedo: #ffffff,
            not_declared: 1.0
        )
    }
}
"#,
    );

    assert!(!success, "undeclared channel should fail:\n{output}");
    assert!(
        output.contains("unknown material argument `not_declared`"),
        "missing unknown material argument diagnostic:\n{output}"
    );
}

#[test]
fn multiple_schema_evaluators_can_target_the_same_material_properties() {
    let (success, output) = compile(
        r#"
@evaluator(forward)
material_properties standard {
    channel albedo: color
    channel emissive: vec3 = vec3(0.0)
    channel opacity: f32 = 1.0
}

schema_evaluator forward for standard {
    contract {
        inputs: lighting_ctx: vec3, shading_ctx: f32, light_sample: f32
        runtime: light_positions: vec3
    }

    permutations {
        light_count: 2
    }

    specialize as "forward"

    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

schema_evaluator forward_plus for standard {
    contract {
        inputs: lighting_ctx: vec3, shading_ctx: f32, light_sample: f32
        runtime: light_positions: vec3, tile_lights: f32
    }

    permutations {
        light_count: 2
    }

    specialize as "forward_plus"

    shade: (
        albedo.r + emissive.x + light_sample,
        albedo.g + emissive.y + light_sample,
        albedo.b + emissive.z + light_sample,
        albedo.a * opacity
    )
}

surface demo(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );
    assert!(
        success,
        "multi-model lighting should compile, got:\n{output}"
    );
    assert!(
        !output.contains("multiple schema_evaluator declarations target material_properties"),
        "unexpected lighting-model diagnostic:\n{output}"
    );
}

#[test]
fn top_level_light_count_is_not_special_cased() {
    let (success, output) = compile(
        r#"
material_properties standard {
    channel albedo: color
    channel emissive: vec3
    channel opacity: f32
}

schema_evaluator forward for standard {
    contract {
        inputs: lighting_ctx: vec3, shading_ctx: f32, light_sample: f32
        runtime: light_positions: vec3
    }

    light_count: 2

    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}
"#,
    );

    assert!(
        !success,
        "top-level light_count should not be treated as a special schema_evaluator item:\n{output}"
    );
    assert!(
        output.contains("light_count") || output.contains("unexpected"),
        "expected a parse error for the top-level light_count item:\n{output}"
    );
}

#[test]
fn schema_evaluator_requires_existing_material_model() {
    let (success, output) = compile(
        r#"
schema_evaluator orphan for no_such_model {
    shade: (1.0, 1.0, 1.0, 1.0)
}

surface sample_value(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        !success,
        "schema_evaluator without material_properties should fail:\n{output}"
    );
    assert!(
        output.contains("references unknown material_properties `no_such_model`"),
        "missing schema_evaluator material binding diagnostic:\n{output}"
    );
}

#[test]
fn vnext_surface_data_model_declarations_parse() {
    let (success, output) = compile(
        r#"
material_properties base_surface {
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

surface demo(sp: surf) -> material(pbr_surface) {
    compose {
        base(albedo: #8899aa, roughness: 0.6)
    }
}
"#,
    );

    assert!(
        success,
        "vnext material/surface/lighting declarations should parse cleanly, got:\n{output}"
    );
}

#[test]
fn schema_evaluator_can_define_contract_and_permutations_without_shade() {
    let (success, output) = compile(
        r#"
material_properties base_surface {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_expression pbr_response for base_surface {
    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

schema_evaluator forward_plus_clustered for base_surface {
    contract {
        inputs: lighting_ctx, shading_ctx, light_sample
    }

    permutations {
        light_count: 2
        tile_size: 8|16|32
        cluster_depth_slices: 16|32|64
    }

    specialize as "forward_plus_clustered_${tile_size}_${cluster_depth_slices}"
}

surface demo(sp: surf) -> material(base_surface) {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        success,
        "schema_evaluator should parse without an embedded shade expression, got:\n{output}"
    );
}

#[test]
fn surface_material_model_is_rejected_as_unsupported() {
    let (success, output) = compile(
        r#"
surface_material_model broken {
    properties: pbr_surface
    render_mode: opaque
}

surface demo(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        !success,
        "unsupported surface_material_model should fail parse:\n{output}"
    );
    assert!(
        output.contains("surface_material_model") || output.contains("expected"),
        "expected parser rejection for unsupported surface_material_model declaration:\n{output}"
    );
}

#[test]
fn surface_material_model_is_rejected_when_declared_with_shader_binding() {
    let (success, output) = compile(
        r#"
material_properties base_surface {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

material_properties other_surface {
    channel albedo: color = #ffffff
    channel emissive: vec3 = (0.0, 0.0, 0.0)
    channel opacity: f32 = 1.0
}

schema_expression pbr_response for base_surface {
    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

schema_evaluator forward_plus_clustered for other_surface {
    shade: (
        albedo.r + emissive.x,
        albedo.g + emissive.y,
        albedo.b + emissive.z,
        albedo.a * opacity
    )
}

surface_material_model broken {
    properties: other_surface
    surface_shader: pbr_response
    lighting: forward_plus_clustered
    render_mode: opaque
}

surface demo(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        !success,
        "unsupported surface_material_model should fail even when bindings look inconsistent:\n{output}"
    );
    assert!(
        output.contains("surface_material_model") || output.contains("expected"),
        "expected parser rejection for unsupported surface_material_model declaration:\n{output}"
    );
}

#[test]
fn material_properties_inheritance_cycle_is_rejected() {
    let (success, output) = compile(
        r#"
material_properties a extends b {
    channel albedo: color = #ffffff
}

material_properties b extends a {
    channel roughness: f32 = 0.5
}

surface demo(sp: surf) -> material {
    compose {
        base(albedo: #ffffff)
    }
}
"#,
    );

    assert!(
        !success,
        "cyclic material_properties inheritance should fail:\n{output}"
    );
    assert!(
        output.contains("cyclic material_properties inheritance")
            || output.contains("inheritance cycle"),
        "expected cycle diagnostic for material_properties inheritance:\n{output}"
    );
}

#[test]
fn emissive_only_base_uses_black_opaque_albedo() {
    for profile in ["", "(unlit)"] {
        let source = format!(
            "surface glow(sp: surf) -> material{profile} {{ compose {{ base(emissive: vec3(2.0, 0.5, 0.25)) }} }}"
        );
        let (success, output) = compile(&source);
        assert!(success, "emissive-only surface should compile:\n{output}");
        assert!(
            output.contains("vec4<f32>(0f, 0f, 0f, 1f)"),
            "omitted albedo must be opaque black, not add white to emission:\n{output}"
        );
        assert!(
            output.contains("vec3<f32>(2f, 0.5f, 0.25f)"),
            "emission must survive lowering:\n{output}"
        );
    }
}

#[test]
fn engine_defaults_initialize_builtin_material_channels_before_authored_layers() {
    let (success, output) = compile(
        r#"
material_properties glow_defaults {
    channel albedo: color = rgba(0.125, 0.25, 0.5, 0.75)
    channel emissive: vec3 = vec3(2.0, 3.0, 4.0)
    channel opacity: f32 = 0.625
    channel roughness: f32 = 0.375
    channel metallic: f32 = 0.875
    channel normal: vec3 = vec3(0.0, 1.0, 0.0)
}
surface glow(sp: surf) -> material(glow_defaults) {
    compose { base(emissive: vec3(5.0, 6.0, 7.0)) }
}
"#,
    );
    assert!(success, "engine channel defaults should compile:\n{output}");
    for assignment in [
        "albedo = vec4<f32>(0.125f, 0.25f, 0.5f, 0.75f)",
        "emissive = vec3<f32>(2f, 3f, 4f)",
        "opacity = 0.625",
        "roughness = 0.375",
        "metallic = 0.875",
        "normal = vec3<f32>(0f, 1f, 0f)",
    ] {
        assert!(
            output.contains(assignment),
            "missing engine default {assignment}:\n{output}"
        );
    }
    let default_pos = output.find("emissive = vec3<f32>(2f, 3f, 4f)").unwrap();
    let authored_pos = output
        .find("emissive = vec3<f32>(5f, 6f, 7f)")
        .expect("authored emission must override the default");
    assert!(default_pos < authored_pos);
}

#[test]
fn material_fields_are_engine_typed_without_a_special_name_or_eight_field_limit() {
    let fields = (0..12)
        .map(|index| format!("channel heat{index}: f32 = {index}.0\n"))
        .collect::<String>();
    let (success, output) = compile(&format!(
        r#"
material_properties thermal {{
    {fields}
    channel enabled: bool = true
    channel index: u32 = 7
    channel offset: i32 = -2
}}
surface thermal_value(sp: surf) -> material(thermal) {{ compose {{ base(heat0: 0.25) }} }}
"#
    ));
    assert!(success, "typed engine fields should compile: {output}");
    assert!(output.contains("enabled: bool"), "{output}");
    assert!(output.contains("index: u32"), "{output}");
    assert!(output.contains("offset: i32"), "{output}");
    assert!(output.contains("heat11_: f32"), "{output}");
}

#[test]
fn conventional_material_names_are_not_implicitly_declared() {
    let (success, output) = compile(
        r#"
material_properties thermal { channel heat: f32 = 0.0 }
surface thermal_value(sp: surf) -> material(thermal) { compose { base(albedo: #fff) } }
"#,
    );
    assert!(!success);
    assert!(
        output.contains("unknown material argument `albedo`"),
        "{output}"
    );
}

#[test]
fn engine_defines_composition_vocabulary_for_weighted_fields() {
    let (success, output) = compile(
        r#"
@composition(initialize_heat, heat_layer, weight)
material_properties thermal { channel @compose(mix) heat: f32 = 1.0 }
surface thermal_value(sp: surf) -> material(thermal) {
    compose { initialize_heat(heat: 2.0)
        layer heat_layer(heat: 6.0, weight: 0.5) }
}
"#,
    );
    assert!(success, "engine composition should compile: {output}");
    assert!(
        output.contains("mix(2f, 6f, 0.5f)"),
        "weighted heat must interpolate: {output}"
    );
}

#[test]
fn weighted_material_fields_use_declared_mix_function() {
    for (ty, previous, next) in [
        ("f32", "2.0", "6.0"),
        ("vec2", "vec2(2.0, 4.0)", "vec2(6.0, 8.0)"),
        ("vec3", "vec3(2.0, 4.0, 6.0)", "vec3(6.0, 8.0, 10.0)"),
        (
            "vec4",
            "vec4(2.0, 4.0, 6.0, 8.0)",
            "vec4(6.0, 8.0, 10.0, 12.0)",
        ),
        (
            "color",
            "rgba(0.1, 0.2, 0.3, 0.4)",
            "rgba(0.5, 0.6, 0.7, 0.8)",
        ),
    ] {
        let source = format!(
            r#"
@composition(initialize, coat, weight)
material_properties thermal {{ channel @compose(mix) heat: {ty} = {previous} }}
surface thermal_value(sp: surf) -> material(thermal) {{
    compose {{ initialize()
        layer coat(heat: {next}, weight: 0.25) }}
}}
"#
        );
        let (success, implicit) = compile(&source);
        assert!(success, "declared mix for {ty}: {implicit}");
        let explicit_source = source.replace(
            &format!("heat: {next}, weight: 0.25"),
            &format!("heat: mix({previous}, {next}, 0.25)"),
        );
        let (success, explicit) = compile(&explicit_source);
        assert!(success, "explicit mix for {ty}: {explicit}");
        assert_eq!(
            implicit, explicit,
            "weighted {ty} must use ordinary mix semantics"
        );
    }
}

#[test]
fn weighted_boolean_fields_are_rejected() {
    let (success, output) = compile(
        r#"
@composition(initialize, coat, weight)
material_properties flags { channel enabled: bool = false }
surface flag_value(sp: surf) -> material(flags) {
    compose { initialize()
        layer coat(enabled: true, weight: 0.5) }
}
"#,
    );
    assert!(!success, "booleans must not silently interpolate");
    assert!(
        output.contains("requires an explicit composition function"),
        "{output}"
    );
}

#[test]
fn bundled_material_layers_use_weight_instead_of_mask() {
    let source = r#"
surface weighted(sp: surf) -> material {
    compose { base(roughness: 0.8)
        layer material(roughness: 0.2, weight: 0.5) }
}
"#;
    let (success, output) = compile(source);
    assert!(success, "{output}");
    let (success, output) = compile(&source.replace("weight:", "mask:"));
    assert!(!success);
    assert!(
        output.contains("unknown material argument `mask`"),
        "{output}"
    );
}

#[test]
fn schema_program_uses_ordinary_records_and_arbitrary_function_names() {
    let source = r#"
struct LightSample { energy: f32 }
material_properties signal_data { channel power: f32 = 1.0 }
schema_program transfer for signal_data {
    fn deposit(sample: LightSample, scale: f32) -> vec4 {
        return vec4(sample.energy * scale)
    }
    output: deposit(LightSample(energy: self.power), 3.0)
}
surface probe(sp: surf) -> material(signal_data) { compose { base(power: 2.0) } }
"#;
    let (success, wgsl) = compile(source);
    assert!(success, "{wgsl}");
    assert!(
        wgsl.contains("fresco_schema_value.power") && wgsl.contains(", 3.0, fresco_schema_context"),
        "{wgsl}"
    );
    let renamed = source
        .replace("LightSample", "Packet")
        .replace("deposit", "transform_signal");
    let (success, renamed_wgsl) = compile(&renamed);
    assert!(success, "{renamed_wgsl}");
    assert_eq!(
        wgsl, renamed_wgsl,
        "record and function spelling must not carry lighting semantics"
    );
    let (success, diagnostics) =
        compile(&source.replace("LightSample(energy: self.power)", "vec2(1.0)"));
    assert!(
        !success,
        "incorrect argument type was accepted: {diagnostics}"
    );
}

#[test]
fn schema_program_requires_explicit_output_and_rejects_duplicate_output() {
    let source = "material_properties signal_data { channel power: f32 = 1.0 }\nschema_program transfer for signal_data { fn direct() -> vec4 { return vec4(1.0) } OUTPUT }";
    for (output, diagnostic) in [
        ("", "requires an explicit output expression"),
        (
            "\noutput: direct()\noutput: direct()\n",
            "duplicate schema program output",
        ),
    ] {
        let (success, result) = compile(&source.replace("OUTPUT", output));
        assert!(!success, "{result}");
        assert!(result.contains(diagnostic), "{result}");
    }
}

#[test]
fn schema_program_output_does_not_depend_on_pipeline_tags_or_blending() {
    let source = r#"
tags pipeline { transport }
material_properties signal_data { channel power: f32 = 1.0 }
schema_program transfer for signal_data {
    fn direct() -> vec4 { return vec4(self.power) }
    fn indirect() -> vec4 { return vec4(7.0) }
    output: direct() + indirect()
}
pass process for signal_data {
    stage: raster
    draw: per_object
    blend: opaque
}
pipeline(transport) route for signal_data { process }
surface probe(sp: surf) -> material(signal_data) { compose { base() } }
"#;
    let (success, wgsl) = compile(source);
    assert!(success, "{wgsl}");
    let (success, alternate) = compile(
        &source
            .replace("transport", "illumination")
            .replace("blend: opaque", "blend: additive"),
    );
    assert!(success, "{alternate}");
    assert_eq!(
        wgsl, alternate,
        "scheduling must not rewrite schema evaluation"
    );
}

#[test]
fn schema_program_validates_unused_local_functions() {
    let (success, result) = compile(
        r#"
material_properties signal_data { channel power: f32 = 1.0 }
schema_program transfer for signal_data {
    fn unused() -> vec4 { return vec2(self.power) }
    output: vec4(self.power)
}
surface probe(sp: surf) -> material(signal_data) { compose { base() } }
"#,
    );
    assert!(!success, "{result}");
    assert!(
        result.contains("does not match the declared return type"),
        "{result}"
    );
}

#[test]
fn material_composition_preserves_structured_payloads_and_authored_rules() {
    let source = r#"
struct Payload { weights: array<vec2, 2>, basis: mat2, enabled: bool }
fn choose_payload(previous: Payload, next: Payload, weight: f32) -> Payload {
    return Payload(weights: [mix(previous.weights[0], next.weights[0], weight), next.weights[1]], basis: previous.basis, enabled: next.enabled)
}
@composition(initialize, coat, weight)
material_properties data {
    channel @compose(choose_payload) payload: Payload = Payload(weights: [vec2(1.0), vec2(2.0)], basis: mat2(vec2(1.0, 0.0), vec2(0.0, 1.0)), enabled: false)
    channel untouched: f32 = 7.0
}
schema_program readback for data {
    output: vec4(self.payload.weights[0], self.payload.basis[0][0], self.untouched)
}
surface probe(sp: surf) -> material(data) {
    compose { initialize()
        layer coat(payload: Payload(weights: [vec2(3.0), vec2(4.0)], basis: mat2(vec2(2.0), vec2(2.0)), enabled: true), weight: 0.5) }
}
"#;
    let (success, manifest) = compile_manifest(source);
    assert!(success, "{manifest}");
    let manifest: Value = serde_json::from_str(&manifest).unwrap();
    let payload = manifest["surfaces"][0]["custom_channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["name"] == "payload")
        .unwrap();
    assert_eq!(payload["type"], "Payload");
    assert!(
        payload.get("components").is_none(),
        "records must not be described as vectors"
    );
    let (success, wgsl) = compile(source);
    assert!(success, "{wgsl}");
    assert!(wgsl.contains("array<vec2<f32>, 2>"), "{wgsl}");
    assert!(wgsl.contains("mat2x2<f32>"), "{wgsl}");
    assert!(wgsl.contains("enabled: bool"), "{wgsl}");
    let (success, diagnostic) = compile(&source.replace("@compose(choose_payload)", ""));
    assert!(!success);
    assert!(
        diagnostic.contains("requires an explicit composition function"),
        "{diagnostic}"
    );
    let (success, diagnostic) =
        compile(&source.replace("@compose(choose_payload)", "@compose(mix)"));
    assert!(
        !success,
        "records must not get an invented component-wise mix"
    );
    assert!(!diagnostic.is_empty());
}

#[test]
fn composition_rules_are_inherited_and_discrete_rules_are_authored() {
    let source = r#"
fn choose_flag(previous: bool, next: bool, weight: f32) -> bool { return select(previous, next, weight >= 0.5) }
@composition(initialize, coat, weight)
material_properties flags { channel @compose(choose_flag) enabled: bool = false }
material_properties child_flags extends flags { channel counter: f32 = 9.0 }
surface probe(sp: surf) -> material(child_flags) {
    compose { initialize()
layer coat(enabled: true, weight: 0.75) }
}
"#;
    let (success, output) = compile(source);
    assert!(success, "{output}");
    assert!(output.contains("enabled: bool"), "{output}");
    let (success, diagnostic) = compile(&source.replace("@compose(choose_flag)", "@compose(mix)"));
    assert!(
        !success,
        "numeric mix cannot implement a boolean composition rule"
    );
    assert!(
        diagnostic.contains("invalid composition function"),
        "{diagnostic}"
    );
    let (success, diagnostic) =
        compile(&source.replace("@compose(choose_flag)", "@compose(missing_rule)"));
    assert!(!success);
    assert!(diagnostic.contains("missing_rule"), "{diagnostic}");
    let (success, diagnostic) = compile(&source.replace(
        "@compose(choose_flag)",
        "@compose(choose_flag) @compose(mix)",
    ));
    assert!(!success);
    assert!(
        diagnostic.contains("duplicate material channel attribute"),
        "{diagnostic}"
    );
}

#[test]
fn composition_rejects_invalid_array_shapes_before_lowering() {
    for (ty, value) in [
        ("array<f32, 2>", "[1.0]"),
        ("array<f32, 0>", "[]"),
        ("array<f32>", "[1.0]"),
        ("array<Missing, 2>", "[1.0, 2.0]"),
    ] {
        let source = format!(
            "material_properties data {{ channel values: {ty} = {value} }}\nsurface probe(sp: surf) -> material(data) {{ compose {{ base() }} }}"
        );
        let (success, diagnostic) = compile(&source);
        assert!(!success, "invalid shape {ty} must fail checking");
        assert!(diagnostic.contains("Error"), "{diagnostic}");
        assert!(!diagnostic.contains("panicked"), "{diagnostic}");
    }
}

#[test]
fn ordinary_composition_functions_accept_and_return_arrays() {
    let source = r#"
fn compose_samples(previous: array<vec2, 2>, next: array<vec2, 2>, weight: f32) -> array<vec2, 2> {
    return [mix(previous[0], next[0], weight), previous[1]]
}
fn compose_nested(previous: array<array<f32, 2>, 2>, next: array<array<f32, 2>, 2>, weight: f32) -> array<array<f32, 2>, 2> { return next }
material_properties data {
    channel @compose(compose_samples) samples: array<vec2, 2> = [vec2(1.0), vec2(2.0)]
    channel @compose(compose_nested) nested: array<array<f32, 2>, 2> = [[1.0, 2.0], [3.0, 4.0]]
}
schema_program readback for data { output: vec4(self.samples[0], self.nested[1][0], self.nested[1][1]) }
surface probe(sp: surf) -> material(data) {
    compose { base()
layer material(samples: [vec2(3.0), vec2(4.0)], nested: [[5.0, 6.0], [7.0, 8.0]], weight: 0.5) }
}
"#;
    let (success, output) = compile(source);
    assert!(success, "{output}");
    let (success, diagnostic) = compile(&source.replace(
        "return [mix(previous[0], next[0], weight), previous[1]]",
        "return [previous[0]]",
    ));
    assert!(
        !success,
        "composition must preserve its declared array length"
    );
    assert!(diagnostic.contains("Error"), "{diagnostic}");
    assert!(!diagnostic.contains("panicked"), "{diagnostic}");
}
