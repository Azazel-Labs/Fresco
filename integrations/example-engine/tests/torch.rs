use fresco_example_engine::source_files;

#[test]
fn torch_compiles_with_simulation_and_three_sprite_inputs() {
    let mut files = source_files();
    files.insert(
        "main.fr".into(),
        include_str!("../../../examples/50) particles/torch.fr").into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let varying = module
        .types
        .iter()
        .find_map(|(_, ty)| {
            if ty.name.as_deref()?.ends_with("CameraParticleVarying")
                && let naga::TypeInner::Struct { members, .. } = &ty.inner
            {
                return Some(members);
            }
            None
        })
        .expect("billboard varyings");
    for name in ["particle_life", "particle_velocity"] {
        let member = varying
            .iter()
            .find(|m| m.name.as_deref() == Some(name))
            .unwrap();
        assert!(
            matches!(
                member.binding,
                Some(naga::Binding::Location {
                    interpolation: Some(naga::Interpolation::Flat),
                    ..
                })
            ),
            "per-particle values must not interpolate across a sprite"
        );
    }
    assert!(
        module
            .entry_points
            .iter()
            .any(|e| e.stage == naga::ShaderStage::Compute)
    );
    assert!(
        module
            .entry_points
            .iter()
            .any(|e| e.stage == naga::ShaderStage::Fragment)
    );
    for name in ["torch_flame.png", "torch_smoke.png", "torch_ember.png"] {
        assert!(
            output.manifest.contains(name),
            "missing sprite binding: {name}"
        );
    }
    assert!(
        !output.wgsl.contains("discard;"),
        "torch uses soft transparency"
    );
}

#[test]
fn nested_record_updates_preserve_types_and_reject_invalid_swizzles() {
    let source = r#"
struct Inner { value: vec3 }
struct Outer { inner: Inner; sibling: f32 }
fn modify(p: Outer) -> Outer {
    var next = p
    next.inner.value.y += 0.2
    return next
}
canvas probe(ctx: CanvasContext) -> color {
    var p = modify(Outer(inner: Inner(value: vec3(ctx.uv, 0.3)), sibling: 0.7))
    p.inner.value.x = 0.5
    rgba(p.inner.value.x, p.inner.value.y, p.inner.value.z, p.sibling)
}
"#;
    let mut files = source_files();
    files.insert("main.fr".into(), source.into());
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let equivalent = source.replace(
        "rgba(p.inner.value.x, p.inner.value.y, p.inner.value.z, p.sibling)",
        "rgba(0.5, ctx.uv.y + 0.2, 0.3, 0.7)",
    );
    files.insert("main.fr".into(), equivalent);
    let expected = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    assert_eq!(
        output.wgsl, expected.wgsl,
        "nested writes must preserve the other vector components and sibling record field"
    );
    for (from, to) in [
        ("value.y += 0.2", "value.xx = vec2(0.2)"),
        ("value.y += 0.2", "missing.y = 0.2"),
        ("value.x = 0.5", "value = 0.5"),
    ] {
        files.insert("main.fr".into(), source.replace(from, to));
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(format!("{errors:?}").contains("invalid assignment target"));
    }
}
