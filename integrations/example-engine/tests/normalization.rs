//! The authored normalization helpers must link through engine GPU passes too.
use fresco_example_engine::source_files;

#[test]
fn normalization_overloads_link_in_registered_surface_responses() {
    let source = include_str!(
        "../../../examples/40) surface shaders/style_sample.fr"
    )
    .replace(
        "let nl = dot(n, l)",
        "let pair = normalize_or_zero(vec2(n.x, n.y))\n\
             let quad = normalize_or(vec4(n, 0.0), vec4(0.0, 1.0, 0.0, 0.0))\n\
             let normal = normalize_or(n, vec3(0.0, 1.0, 0.0))\n\
             let nl = dot(normal, normalize_or_zero(l + n * 0.0)) + pair.x * quad.x * 0.01",
    )
    .replace(
        "let view_normal = (scene.view * vec4(projected.world_normal, 0.0)).xyz",
        "let view_normal = normalize_or_zero(inverse_sqrt(vec3(1.0)) * (scene.view * vec4(projected.world_normal, 0.0)).xyz)",
    );
    let mut files = source_files();
    files.insert("main.fr".into(), source);
    let compute = files
        .get_mut("engine/pipelines/40_gpu_programs.fr")
        .unwrap();
    *compute = compute
        .replace("length(plane.xyz)", "length(normalize_or_zero(plane.xyz))")
        .replace(
            "let sphere = lights[i * u32(2)]",
            "let sphere = normalize_or(inverse_sqrt(vec4(1.0)) * lights[i * u32(2)], vec4(0.0))",
        );
    for renderer in ["forward", "forward-plus", "deferred"] {
        let output = fresco::driver::compile_bundle_virtual_with_context(
            &files,
            "main.fr",
            false,
            &fresco::driver::CompileContext {
                renderer: Some(renderer.into()),
                ..Default::default()
            },
        )
        .expect("normalization helpers must resolve and lower in surface response functions");
        let module = naga::front::wgsl::parse_str(&output.wgsl).expect("valid emitted WGSL");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("valid normalization calls in GPU program");
    }
}
