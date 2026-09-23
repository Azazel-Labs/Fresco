#[path = "support/common.rs"]
mod common;

/// Integration tests for the `texture_type` feature.
///
/// Covers:
/// - Parsing `texture_type` declarations
/// - `param name: texture<Type>` and plain `param name: texture`
/// - Semantic channel access: `.roughness`, `.metallic`, raw `.r`/`.g`/`.b`/`.a`
/// - Manifest output includes `texture_type` and `channels` metadata
/// - Error cases: unknown type, unknown channel name
use fresco::driver;

fn compile_manifest(src: &str) -> Result<String, Vec<driver::DiagnosticRecord>> {
    driver::compile_source(src, common::TEST_SOURCE_PATH, "manifest", false).map(|out| out.emitted)
}

fn compile_wgsl(src: &str) -> Result<String, Vec<driver::DiagnosticRecord>> {
    driver::compile_source(src, common::TEST_SOURCE_PATH, "wgsl", false).map(|out| out.emitted)
}

// ---------------------------------------------------------------------------
// Parsing: texture_type declaration at top level
// ---------------------------------------------------------------------------

#[test]
fn texture_type_parses_and_compiles() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let r = orm.at(uv).roughness
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(r, r, r, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected texture_type program to compile\nErrors: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// Manifest: channel layout emitted for typed textures
// ---------------------------------------------------------------------------

#[test]
fn manifest_includes_texture_type_and_channels() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let r = orm.at(uv).roughness
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(r, r, r, 1.0))
    }
}
"#;
    let manifest = compile_manifest(src).expect("manifest should succeed");

    // texture_type field
    assert!(
        manifest.contains(r#""texture_type": "ORM""#)
            || manifest.contains(r#""texture_type":"ORM""#),
        "manifest should include texture_type name\n{manifest}"
    );
    // channel layout
    assert!(
        manifest.contains(r#""roughness""#),
        "manifest should include channel semantic name\n{manifest}"
    );
    assert!(
        manifest.contains(r#""metallic""#),
        "manifest should include channel semantic name\n{manifest}"
    );
    // default_asset
    assert!(
        manifest.contains(r#"orm.png"#),
        "manifest should include default asset path\n{manifest}"
    );
}

// ---------------------------------------------------------------------------
// Raw channel access: .r .g .b .a
// ---------------------------------------------------------------------------

#[test]
fn raw_channel_access_compiles() {
    let src = r#"
texture_type Packed {
    r: shadow
    g: roughness
    b: metallic
    a: emissive
}

canvas test(uv: coord) -> color {
    param packed: texture<Packed> = "packed.png"
    let g = packed.at(uv).g
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(g, g, g, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected raw channel .g access to compile\nErrors: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// Plain (untyped) `texture` param
// ---------------------------------------------------------------------------

#[test]
fn plain_texture_param_requires_explicit_sampling() {
    let src = r#"
canvas test(uv: coord) -> color {
    param albedo: texture = "albedo.png"
    compose {
        albedo.at(uv)
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected plain texture param to compile\nErrors: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// Error: unknown texture_type
// ---------------------------------------------------------------------------

#[test]
fn error_unknown_texture_type() {
    let src = r#"
canvas test(uv: coord) -> color {
    param orm: texture<NoSuchType> = "orm.png"
    compose {
        circle(at: center, radius: 0.3) |> fill(#ffffff)
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_err(),
        "expected unknown texture_type to produce a compile error"
    );
    let diags = result.unwrap_err();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("NoSuchType")),
        "expected error mentioning the unknown type name\nGot: {messages:?}"
    );
}

// ---------------------------------------------------------------------------
// Error: unknown channel semantic name
// ---------------------------------------------------------------------------

#[test]
fn error_unknown_channel_name() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let v = orm.at(uv).emissive
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(v, v, v, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_err(),
        "expected unknown channel access to produce a compile error"
    );
    let diags = result.unwrap_err();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("emissive")),
        "expected error mentioning the unknown channel name\nGot: {messages:?}"
    );
}

// ---------------------------------------------------------------------------
// Multiple channel access in same canvas
// ---------------------------------------------------------------------------

#[test]
fn multiple_channel_access_compiles() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let sampled = orm.at(uv)
    let occ = sampled.occlusion
    let rough = sampled.roughness
    let metal = sampled.metallic
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(occ, rough, metal, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected multiple channel accesses to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn typed_texture_channel_access_after_explicit_uv_sample_compiles() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let rough = image(orm, at: uv * 2.0).roughness
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(rough, rough, rough, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected typed texture channel access after explicit UV sampling to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn typed_texture_result_expression_can_return_color() {
    let src = r#"
texture_type AlbedoRGB -> color {
    r: red
    g: green
    b: blue

    return rgb(r, g, b)
}

canvas test(uv: coord) -> color {
    param albedo: texture<AlbedoRGB> = "albedo.png"
    let sampled = albedo.at(uv)
    compose {
        circle(at: center, radius: 0.3) |> fill(sampled)
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected typed texture result expression to return color\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn plain_texture_explicit_uv_sample_compiles() {
    let src = r#"
canvas test(uv: coord) -> color {
    param albedo: texture = "albedo.png"
    compose {
        image(albedo, at: uv * 3.0)
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected plain texture explicit UV sampling to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn texture_type_affine_decode_compiles() {
    let src = r#"
texture_type NormalGL -> vec3 {
    r: nx * 2 - 1
    g: ny * 2 - 1
    b: nz * 2 - 1

    return normalize(vec3(nx, ny, nz))
}

canvas test(uv: coord) -> color {
    param normal_tex: texture<NormalGL> = "normal.png"
    let normal = normal_tex.at(uv)
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(normal.x, normal.y, normal.z, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected texture_type result vec3 to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn typed_texture_at_helper_syntax_compiles() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let rough = orm.at(uv * 2.0).roughness
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(rough, rough, rough, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected typed texture .at(...) helper syntax to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn typed_texture_reused_channels_share_one_sample() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let sampled = orm.at(uv)
    let rough = sampled.roughness
    let metal = sampled.metallic
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(rough, metal, rough, 1.0))
    }
}
"#;
    let result = compile_wgsl(src).expect("typed texture reuse should compile");
    let sample_count = result.matches("textureSample(").count();
    assert_eq!(
        sample_count, 1,
        "expected one textureSample call when reusing the same explicit sample for roughness and metallic, got {sample_count}\n{result}"
    );
}

#[test]
fn plain_texture_at_helper_syntax_compiles() {
    let src = r#"
canvas test(uv: coord) -> color {
    param albedo: texture = "albedo.png"
    compose {
        albedo.at(uv * 3.0)
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected plain texture .at(...) helper syntax to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn implicit_texture_sampling_errors_by_default() {
    let src = r#"
texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let rough = orm.roughness
    compose {
        orm
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_err(),
        "expected implicit texture sampling to fail by default"
    );
    let diags = result.unwrap_err();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| {
            m.contains("implicit texture sampling is disabled")
                || m.contains("implicit channel read is disabled")
                || m.contains("implicit texture layer use is disabled")
        }),
        "expected explicit-sampling diagnostic\nGot: {messages:?}"
    );
}

#[test]
fn migration_flag_allows_implicit_texture_sampling() {
    let src = r#"
#pragma check.allow_implicit_texture_uv = true

texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let rough = orm.roughness
    compose {
        orm
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected migration flag to allow implicit sampling\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn warning_flag_allows_implicit_texture_sampling_during_rollout() {
    let src = r#"
#pragma check.warn_implicit_texture_uv = true

texture_type ORM {
    r: occlusion
    g: roughness
    b: metallic
}

canvas test(uv: coord) -> color {
    param orm: texture<ORM> = "orm.png"
    let rough = orm.roughness
    compose {
        orm
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected warning mode to allow implicit sampling during rollout\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn tex_at_helper_missing_coordinate_has_texture_specific_diagnostic() {
    let src = r#"
canvas test(uv: coord) -> color {
    param albedo: texture = "albedo.png"
    compose {
        albedo.at()
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_err(),
        "expected invalid tex.at helper usage to fail"
    );
    let diags = result.unwrap_err();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("`tex.at(...)` requires an explicit sample coordinate")),
        "expected texture-specific tex.at diagnostic\nGot: {messages:?}"
    );
}

#[test]
fn texture_type_decode_expression_with_raw_and_bit_helpers_compiles() {
    let src = r#"
texture_type Packed {
    r: occlusion = unpack_unorm8(raw, byte: 0)
    g: roughness = bit_extract(raw, lsb: 0, bits: 5) / 31.0
    b: metallic = unpack_snorm8(raw, byte: 2) * 0.5 + 0.5
}

canvas test(uv: coord) -> color {
    param packed: texture<Packed> = "packed.png"
    let sample_value = packed.at(uv)
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(sample_value.occlusion, sample_value.roughness, sample_value.metallic, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected decode expression syntax to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn bit_pack_unpack_helpers_compile() {
    let src = r#"
canvas test(uv: coord) -> color {
    let packed = pack_unorm8x4(0.2, 0.4, 0.8, 1.0)
    let b = unpack_unorm8(packed, byte: 2)
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(b, b, b, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected bit helper builtins to compile\nErrors: {:?}",
        result.err()
    );
}

#[test]
fn texture_type_decode_expression_can_use_full_texel_components() {
    let src = r#"
texture_type PackedRG {
    r: low_bits = bit_extract(texel.r * 255.0 + texel.g * 65535.0, lsb: 0, bits: 8) / 255.0
    g: high_bits = bit_extract(texel.r * 255.0 + texel.g * 65535.0, lsb: 8, bits: 8) / 255.0
}

canvas test(uv: coord) -> color {
    param packed: texture<PackedRG> = "packed.png"
    let sample_value = packed.at(uv)
    compose {
        circle(at: center, radius: 0.3) |> fill(rgba(sample_value.low_bits, sample_value.high_bits, 0.0, 1.0))
    }
}
"#;
    let result = compile_wgsl(src);
    assert!(
        result.is_ok(),
        "expected decode expression to use texel.r/g/b/a\nErrors: {:?}",
        result.err()
    );
}
