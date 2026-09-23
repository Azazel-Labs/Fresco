const MATERIAL: &str = "surface sample(sp: surf) -> material(unlit) { compose { base(albedo: rgba(1.0, 0.0, 0.0, 1.0)) } }";
const LIBRARY: &str = "interface ProbeService { fn evaluate(position: vec3) -> vec3 }\n@service(ProbeService) pass service_library { fn evaluate(position: vec3) -> vec3 { return position } }";

#[test]
fn forward_lighting_service_links_real_engine_resources_in_every_renderer() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert(
            "main.fr".into(),
            include_str!("fixtures/lighting-service.fr").into(),
        );
        let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&result.manifest).unwrap();
        let renderer = manifest.renderers.iter().find(|r| r.selected).unwrap();
        let draw = renderer
            .steps
            .iter()
            .find(|s| {
                s.invocation
                    .as_ref()
                    .is_some_and(|i| i.operation == "LitOverlay")
            })
            .unwrap();
        for resource in ["point_lights", "environment", "shadow_camera", "shadow_map"] {
            assert!(
                draw.bindings
                    .keys()
                    .any(|name| name.starts_with("__service_") && name.ends_with(resource)),
                "missing {resource}"
            );
        }
    }
}

#[test]
fn service_helpers_preserve_reserved_identifiers_without_escape_collisions() {
    let mut files = service_files("forward", "evaluate");
    for source in files.values_mut() {
        *source = source.replace(
            "fn service_color(position: vec3) -> vec3 { return environment.sky.rgb + position }",
            "fn service_color(active: vec3) -> vec3 { var fresco_reserved_active = environment.sky.rgb\nfresco_reserved_active = fresco_reserved_active + active\nreturn fresco_reserved_active }",
        );
    }
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&compiled.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn service_iterators_expand_into_valid_shader_loops() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = service_files(renderer, "evaluate");
        for source in files.values_mut() {
            *source = source
                .replace("fn evaluate(position: vec3) -> vec3", "fn evaluate(position: vec3) -> iterator<vec3>")
                .replace("return service_color(position)", "let accumulated = position\nfor i in 0 .. 3 { if i > u32(0) { yield(service_color(accumulated)) } } yield(accumulated)")
                .replace("return vec4(vec3(lights.evaluate(vec3(0.0))) * ink.a, ink.a)", "var accumulated = vec3(0.0)\nfor sample in lights.evaluate(vec3(0.0)) { accumulated = accumulated + sample }\nreturn vec4(accumulated * ink.a, ink.a)");
        }
        let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{renderer}: {e:?}"));
        let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert!(!result.wgsl.contains("iterator<"));
        assert!(!result.wgsl.contains("yield("));
        assert!(result.wgsl.contains("fresco_iterator_"));
    }
}

#[test]
fn shader_iterator_yields_and_value_usage_are_checked() {
    for (body, expected) in [
        ("yield(0.0)", "yields `vec3`, found `f32`"),
        ("yield()", "yield requires one positional value"),
        (
            "yield(position, position)",
            "yield requires one positional value",
        ),
        ("return position", "complete by falling through"),
        (
            "let stored = evaluate(position)",
            "must be consumed directly by for",
        ),
    ] {
        let mut files = service_files("forward", "shadow");
        for source in files.values_mut() {
            *source = source
                .replace(
                    "fn evaluate(position: vec3) -> vec3",
                    "fn evaluate(position: vec3) -> iterator<vec3>",
                )
                .replace("return service_color(position)", body);
        }
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{body}: {errors:?}"
        );
    }
}

fn service_files(renderer: &str, method: &str) -> std::collections::HashMap<String, String> {
    let mut files = fresco_example_engine::source_files_for_recipe(renderer);
    for source in files.values_mut() {
        if source.contains("contract StandardStyle for standard {") {
            *source = source.replace("contract StandardStyle for standard {", "interface ProbeService {\nfn evaluate(position: vec3) -> vec3\nfn shadow(position: vec3) -> f32\n}\ncapability ProbeLighting { probe: ProbeService }\ncontract StandardStyle for standard {\noptional capability ProbeLighting");
        }
        if source.contains("pass preview_mesh for base {") {
            *source = source.replace("pass preview_mesh for base {", "@service(ProbeService) pass preview_mesh for base {\nfn evaluate(position: vec3) -> vec3 { return service_color(position) }\nfn service_color(position: vec3) -> vec3 { return environment.sky.rgb + position }\nfn shadow(position: vec3) -> f32 { return preview_visibility(position, vec3(0.0, 1.0, 0.0)) }");
        }
        if source.contains("provide StandardStyle for renderer_forward {") {
            *source = source.replace("    frame = frame", "    ProbeLighting { probe: shader_service(transparent); factories: preview_static }\n    frame = frame");
        }
    }
    let source = include_str!("fixtures/transparent-queue.fr")
        .replace("    @fragment fn fragment()", "    fn unused_shadow() -> f32 { return lights.shadow(vec3(0.0)) }\n    @fragment fn fragment()")
        .replace(
            "scene: frame, ink: ink",
            "scene: frame, lights: probe, ink: ink",
        )
        .replace(
            "scene: PreviewScene, ink: color",
            "scene: PreviewScene, lights: ProbeService, ink: color",
        )
        .replace(
            "vec4(ink.rgb * ink.a, ink.a)",
            &format!("vec4(vec3(lights.{method}(vec3(0.0))) * ink.a, ink.a)"),
        );
    files.insert("main.fr".into(), source);
    files
}

#[test]
fn service_calls_link_only_reachable_helpers_and_explicit_resources() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        for method in ["evaluate", "shadow"] {
            let result = fresco::driver::compile_bundle_virtual(
                &service_files(renderer, method),
                "main.fr",
                false,
            )
            .unwrap_or_else(|e| panic!("{renderer}/{method}: {e:?}"));
            let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            let manifest: fresco_artifact::ManifestRoot =
                serde_json::from_str(&result.manifest).unwrap();
            let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
            let draw = recipe
                .steps
                .iter()
                .find(|s| {
                    s.invocation
                        .as_ref()
                        .is_some_and(|i| i.operation == "PaintShell")
                })
                .unwrap();
            let resources: Vec<_> = draw
                .bindings
                .keys()
                .filter(|n| n.starts_with("__service_"))
                .collect();
            assert!(
                resources.iter().any(|n| n.ends_with("environment")),
                "{renderer}/{method}"
            );
            assert_eq!(
                resources.iter().any(|n| n.ends_with("shadow_map")),
                method == "shadow"
            );
            assert!(
                !draw.after.iter().any(|n| n == "transparent"),
                "a service reference must not execute its exporting draw"
            );
        }
    }
}

#[test]
fn service_providers_and_call_arguments_are_checked_nominally() {
    for (old, new, expected) in [
        (
            "shader_service(transparent)",
            "shader_service(missing)",
            "unknown shader service node",
        ),
        (
            "@service(ProbeService)",
            "",
            "does not export shader service",
        ),
        (
            "@bind(environment, environment_input)",
            "",
            "unknown provider endpoint",
        ),
        (
            "lights.evaluate(vec3(0.0))",
            "lights.evaluate(0.0)",
            "requires `vec3`, found `f32`",
        ),
        (
            "lights.evaluate(vec3(0.0))",
            "lights.missing(vec3(0.0))",
            "unknown shader service method",
        ),
        (
            "@depth(sun_depth)",
            "@depth(sun_depth) @attachment(sun_depth, clear, discard)",
            "no unconditional producer",
        ),
    ] {
        let mut files = service_files("forward", "evaluate");
        for source in files.values_mut() {
            *source = source.replace(old, new);
        }
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{new}: {errors:?}"
        );
    }
}

#[test]
fn repeated_service_uses_have_independent_capture_bindings() {
    let mut files = service_files("forward", "evaluate");
    let source = files.get_mut("main.fr").unwrap();
    let call = source
        .lines()
        .find(|line| line.trim_start().starts_with("PaintShell(geometry:"))
        .unwrap()
        .to_string();
    *source = source.replace(&call, &format!("{call}\n{call}"));
    let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&result.manifest).unwrap();
    let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
    let captures: Vec<_> = recipe
        .steps
        .iter()
        .filter(|s| s.invocation.is_some())
        .flat_map(|s| s.bindings.keys().filter(|n| n.starts_with("__service_")))
        .collect();
    assert_eq!(captures.len(), 2);
    assert_ne!(captures[0], captures[1]);
}

#[test]
fn service_linking_preserves_contextual_identifiers() {
    let mut files = service_files("forward", "evaluate");
    for source in files.values_mut() {
        *source = source
            .replace("environment: uniform<", "field: uniform<")
            .replace("environment.", "field.")
            .replace("@bind(environment,", "@bind(field,");
    }
    let source = files.get_mut("main.fr").unwrap();
    *source = source.replace("lights", "field");
    let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn service_linking_uses_resolved_calls_across_grouping_and_line_breaks() {
    for call in [
        "(lights).evaluate(vec3(0.0))",
        "lights\n.evaluate(vec3(0.0))",
        "((lights))\n.evaluate(lights.evaluate(vec3(0.0)))",
    ] {
        let mut files = service_files("forward", "evaluate");
        let source = files.get_mut("main.fr").unwrap();
        *source = source.replace("lights.evaluate(vec3(0.0))", call);
        let result = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .unwrap_or_else(|e| panic!("{call}: {e:?}"));
        let module = naga::front::wgsl::parse_str(&result.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
fn service_resources_must_be_ready_at_the_consuming_integration_boundary() {
    let mut files = service_files("forward", "shadow");
    let main = files.get_mut("main.fr").unwrap();
    *main = main.replace("at transparent", "at after_opaque");
    let recipe = files
        .values_mut()
        .find(|s| s.contains("pipeline(postprocess) renderer_forward {"))
        .unwrap();
    *recipe = recipe.replacen(
        "@image(sun_depth, depth32float)",
        "@image(sun_depth, depth32float) @image(late_depth, depth32float)",
        1,
    );
    let start = recipe.find("    @node(transparent)").unwrap();
    let end = start + recipe[start..].find("    @after(transparent)").unwrap();
    let transparent = recipe[start..end]
        .replace(
            "@bind(shadow_map, sun_depth)",
            "@bind(shadow_map, late_depth)",
        )
        .replace(
            "@after(preview_mesh)",
            "@after(preview_mesh) @after(late_shadow)",
        );
    let producer = "    @node(late_shadow) @after(preview_mesh) @draw(shadow_vertex, shadow_fragment, mesh) @depth(late_depth)\n    @bind(scene, frame) @bind(shadow_camera, sun_camera)\n    preview_mesh\n";
    recipe.replace_range(start..end, &format!("{producer}{transparent}"));
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(
        errors.iter().any(|e| e.message.contains(
            "resource `late_depth` is not initialized before `StandardStyle::after_opaque`"
        )),
        "{errors:?}"
    );
    let main = files.get_mut("main.fr").unwrap();
    *main = main.replace("at after_opaque", "at transparent");
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
}

#[test]
fn declaring_a_service_does_not_schedule_or_emit_its_pass() {
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert("main.fr".into(), MATERIAL.into());
        let baseline = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        files.insert("main.fr".into(), format!("{MATERIAL}\n{LIBRARY}"));
        let exported = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let baseline: serde_json::Value = serde_json::from_str(&baseline.manifest).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&exported.manifest).unwrap();
        assert_eq!(baseline["renderers"], manifest["renderers"], "{renderer}");
        assert!(!exported.wgsl.contains("service_library"), "{renderer}");
    }
}

#[test]
fn unselected_service_signature_is_checked() {
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    files.insert(
        "main.fr".into(),
        format!(
            "{MATERIAL}\n{}",
            LIBRARY.replace(
                "fn evaluate(position: vec3) -> vec3 {",
                "fn evaluate(position: vec3) -> f32 {"
            )
        ),
    );
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("incompatible return type")),
        "{errors:?}"
    );
}

#[test]
fn unused_service_checks_reachable_local_and_imported_helpers() {
    let library = "interface ProbeService { fn evaluate(position: vec3) -> vec3 }\n@service(ProbeService) pass service_library {\nfn evaluate(position: vec3) -> vec3 { return bounce(position) }\nfn bounce(position: vec3) -> vec3 { return imported(position) }\n}\nfn imported(position: vec3) -> vec3 { return position }";
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    files.insert("main.fr".into(), format!("{MATERIAL}\n{library}"));
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let lexical = format!(
        "{}\nfn bounce(position: vec3) -> vec3 {{ return position }}",
        library.replace("return position", "return bounce(position)")
    );
    files.insert("main.fr".into(), format!("{MATERIAL}\n{lexical}"));
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    for (old, new, expected) in [
        (
            "return position",
            "return missing_resource",
            "unknown raster value",
        ),
        ("return imported(position)", "return 0.0", "return requires"),
        (
            "return imported(position)",
            "if position.x > 0.0 { return position }",
            "return on every path",
        ),
        (
            "return imported(position)",
            "return evaluate(position)",
            "recursive helper calls",
        ),
        (
            "return position",
            "return imported(position)",
            "recursive helper calls",
        ),
        (
            "fn bounce(position",
            "@fragment fn bounce(position",
            "cannot call stage",
        ),
    ] {
        files.insert(
            "main.fr".into(),
            format!("{MATERIAL}\n{}", library.replace(old, new)),
        );
        let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{new}: {errors:?}"
        );
    }
}

#[test]
fn service_helpers_capture_declared_bindings_but_imports_require_arguments() {
    let source = r#"
struct ServiceInputs { gain: f32 }
interface ProbeService { fn evaluate(position: vec3) -> vec3 }
@service(ProbeService) pass service_library {
    binding { @group(0) @binding(0) scene: uniform<ServiceInputs> }
    fn evaluate(position: vec3) -> vec3 { return scale(position, scene.gain) }
}
fn scale(position: vec3, gain: f32) -> vec3 { return position * gain }
"#;
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    files.insert("main.fr".into(), format!("{MATERIAL}\n{source}"));
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    files.insert(
        "main.fr".into(),
        format!(
            "{MATERIAL}\n{}",
            source.replace("return position * gain", "return position * scene.gain")
        ),
    );
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("unknown raster value `scene`")),
        "{errors:?}"
    );
}
