use fresco_example_engine::runtime::particle_contract::ParticleContract;
use std::{collections::HashMap, fs, path::Path};

fn files() -> HashMap<String, String> {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        CONTRACT.into(),
        include_str!("../../../tests/fixtures/particle_contract.fr").into(),
    );
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    files
}

const CONTRACT: &str = "engine/core/06_particle_contract.fr";

#[test]
fn particle_layout_and_dispatch_follow_authored_contract() {
    let mut files = files();
    let contract = files.get_mut(CONTRACT).unwrap();
    *contract = contract
        .replace("@meta(capacity, 64)", "@meta(capacity, 129)")
        .replace("@workgroup_size(64)", "@workgroup_size(32)")
        .replace("velocity: vec4\n", "velocity: vec4\n\theat: f32\n")
        .replace(
            "cos(angle) * 0.12, 0.0, 0.0))",
            "cos(angle) * 0.12, 0.0, 0.0), heat: id / count)",
        )
        .replace(
            "return particle_integrate(p, dt)",
            "var next = particle_integrate(p, dt)\n next.heat = p.heat + dt\n return next",
        );
    let output =
        fresco::driver::compile_bundle_virtual(&files, "main.fr", false).expect("extended state");
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let particles = &particle_json(&manifest);
    assert_eq!(particles["particle_count"], 129);
    assert_eq!(particles["workgroup_size"], 32);
    assert_eq!(particles["particle_stride"], 48);
    assert_eq!(particles["state_fields"][2]["name"], "heat");
    assert_eq!(particles["state_fields"][2]["offset"], 32);
    assert!(
        output
            .wgsl
            .contains(particles["spawn_entry"].as_str().unwrap())
    );
    assert!(output.wgsl.contains("@workgroup_size(32, 1, 1)"));
    assert!(output.wgsl.contains("p.heat"));
}

#[test]
fn particle_settings_and_initialization_fail_closed() {
    for (old, new, diagnostic) in [
        ("@meta(capacity, 64)", "", "explicit capacity"),
        ("@meta(capacity, 64)", "@meta(capacity, 0)", "1.."),
        ("@meta(capacity, 64)", "@meta(capacity, 1.5)", "integer"),
        ("@workgroup_size(64)", "", "workgroup size"),
        ("@workgroup_size(64)", "@workgroup_size(257)", "1..256"),
        (
            "@meta(capacity, 64)",
            "@meta(capacity, 64) @meta(capacity, 64)",
            "unique key",
        ),
        ("fn spawn(", "fn unused_spawn(", "spawn"),
        ("spawn(id: f32", "spawn(id: u32", "unsupported type"),
        (
            "delta_time: f32",
            "delta_time: vec2",
            "invalid function call",
        ),
        (
            "@dispatch(simulate, capacity)",
            "@dispatch(unknown, capacity)",
            "unknown",
        ),
    ] {
        let mut files = files();
        let contract = files.get_mut(CONTRACT).unwrap();
        assert!(contract.contains(old));
        *contract = contract.replace(old, new);
        let errors = compile_and_validate(&files).expect_err(new);
        assert!(errors.contains(diagnostic), "{old} -> {new}: {errors:?}");
    }
}

#[test]
fn incomplete_or_ambiguous_particle_contract_is_rejected() {
    for (from, to) in [
        ("@node(draw)", "@node(other)"),
        (
            "@draw(vertex, fragment, fullscreen, 6)",
            "@draw(vertex, missing, fullscreen, 6)",
        ),
    ] {
        let mut files = files();
        let contract = files.get_mut(CONTRACT).unwrap();
        assert!(contract.contains(from));
        *contract = contract.replace(from, to);
        let errors = compile_and_validate(&files).expect_err("incomplete contract");
        assert!(
            errors.contains("draw") || errors.contains("missing"),
            "{errors}"
        );
    }
}

#[test]
fn particle_modules_use_normal_generic_functions_and_control_flow() {
    let mut files = files();
    let contract = files.get_mut(CONTRACT).unwrap();
    *contract = contract.replace(
        "return particle_integrate(p, dt)",
        r#"
        let falling = particle_gravity(p, vec3(0.0, -2.0, 0.0), dt)
        let slowed = particle_drag(falling, 0.5, dt)
        var next = particle_integrate(slowed, dt)
        if next.position.y < -1.0 {
            next.position = vec4(next.position.x, -1.0, next.position.z, 1.0)
            next.velocity = vec4(next.velocity.x, abs(next.velocity.y) * 0.5, next.velocity.z, 0.0)
        }
        return next
    "#,
    );
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .expect("generic particle stack");
}

#[test]
fn particle_module_errors_are_checked_before_gpu_emission() {
    for (body, expected) in [
        (
            "var next = p\n next.heat = 1.0\n return next",
            "invalid assignment target",
        ),
        (
            "var next = p\n next.velocity = 1.0\n return next",
            "cannot assign",
        ),
        (
            "var next = p\n next.position = vec4(ddx(dt))\n return next",
            "screen-space",
        ),
        ("return particle_drag(p, vec3(1.0), dt)", "rate"),
        ("if dt > 0.0 { return p }", "not all control-flow"),
    ] {
        let mut files = files();
        let contract = files.get_mut(CONTRACT).unwrap();
        *contract = contract.replace("return particle_integrate(p, dt)", body);
        let errors =
            fresco::driver::compile_bundle_virtual(&files, "main.fr", false).expect_err(body);
        assert!(
            errors.iter().any(|e| e.message.contains(expected)),
            "{body}: {errors:?}"
        );
    }
}

#[test]
fn particle_local_hooks_can_respawn_without_colliding_with_imported_functions() {
    let mut files = files();
    let contract = files.get_mut(CONTRACT).unwrap();
    *contract = contract.replace(
        "return particle_integrate(p, dt)",
        "if dt > 1.0 { return spawn(0.0, 64.0) }\n return particle_integrate(p, dt)",
    );
    contract.push_str("\nfn spawn(value: vec3) -> vec3 { return value }\n");
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .expect("lexical hook scope and authored respawn");
}

#[test]
fn sample_authors_emitter_using_engine_modules() {
    let mut files = files();
    files.insert(
        CONTRACT.into(),
        include_str!("../engine/core/06_particle_contract.fr").into(),
    );
    files.insert(
        "main.fr".into(),
        include_str!("../../../examples/50) particles/drifting_sparks.fr").into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .expect("authored fountain");
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let particles = &particle_json(&manifest);
    assert_eq!(particles["particle_count"], 128);
    assert_eq!(particles["state_fields"][2]["name"], "age");
    assert_eq!(particles["state_fields"][3]["name"], "lifespan");
    assert!(output.wgsl.contains("_face_camera("));
    assert!(output.wgsl.contains("view[u32(0.0)].x"));
    let module = naga::front::wgsl::parse_str(&output.wgsl).expect("valid WGSL");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("valid particle module");
}

#[test]
fn emitter_sample_compiles_from_its_filesystem_location() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/50) particles/drifting_sparks.fr");
    let source = fs::read_to_string(&path).unwrap();
    fresco::driver::compile_source_bundle_with_engine_dir(
        &source,
        &path.to_string_lossy(),
        false,
        &fresco::driver::CompileContext::default(),
        Some(&Path::new(env!("CARGO_MANIFEST_DIR")).join("engine")),
    )
    .expect("engine modules resolve on disk as well as in the playground");
}

#[test]
fn engine_particle_modules_are_available_without_instantiating_a_system() {
    let mut files = files();
    files.insert(
        CONTRACT.into(),
        include_str!("../engine/core/06_particle_contract.fr").into(),
    );
    let output =
        fresco::driver::compile_bundle_virtual(&files, "main.fr", false).expect("ordinary surface");
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert!(particle_json(&manifest).is_null());
    assert!(!output.wgsl.contains("fn fresco_particle_spawn("));
}

#[test]
fn emitter_vocabulary_and_execution_policy_belong_to_the_engine() {
    let mut inputs = files();
    inputs.insert(
        CONTRACT.into(),
        include_str!("../engine/core/06_particle_contract.fr").into(),
    );
    inputs.insert(
        "main.fr".into(),
        include_str!("../../../examples/50) particles/drifting_sparks.fr").into(),
    );
    let original =
        fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).expect("emitter");
    let contract = inputs.get_mut(CONTRACT).unwrap();
    *contract = contract
        .replace("@entry(emitter, update)", "@entry(fountain, animate)")
        .replace("fn update(", "fn animate(")
        .replace("fn spawn(p: Particle", "fn birth(p: Particle")
        .replace("@meta(capacity, allocation == ParticleAllocationMode.Fixed ? max(1, u32(ceil(spawn_rate * max_lifespan)) + burst_count) : allocation_hint)", "@meta(capacity, 129)")
        .replace("@workgroup_size(64)", "@workgroup_size(32)");
    let source = inputs.get_mut("main.fr").unwrap();
    *source = source
        .replace("emitter drifting", "fountain drifting")
        .replace("spawn {", "birth {")
        .replace("update {", "animate {");
    let output = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect("renamed engine contract");
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    assert_eq!(particle_json(&manifest)["particle_count"], 129);
    assert_eq!(particle_json(&manifest)["workgroup_size"], 32);
    assert_ne!(output.wgsl, original.wgsl);
    for (from, to, expected) in [
        ("@meta(capacity, 129)", "", "explicit capacity"),
        ("@workgroup_size(32)", "", "workgroup size"),
        ("@bind(advance_particle)", "", "explicit @bind"),
        ("@bind(advance_particle)", "@bind(spawn)", "conflicts"),
        (
            "@bind(advance_particle)",
            "@bind(initialize_particle)",
            "duplicate compute entry binding",
        ),
    ] {
        let mut invalid = inputs.clone();
        let contract = invalid.get_mut(CONTRACT).unwrap();
        assert!(contract.contains(from));
        *contract = contract.replace(from, to);
        let errors = compile_and_validate(&invalid).expect_err(from);
        assert!(errors.contains(expected), "{from}: {errors:?}");
    }
}

#[test]
fn authored_compute_recursion_is_still_rejected() {
    let mut inputs = files();
    inputs.insert(
        CONTRACT.into(),
        include_str!("../engine/core/06_particle_contract.fr").into(),
    );
    inputs.insert(
        "main.fr".into(),
        include_str!("../../../examples/50) particles/drifting_sparks.fr")
            .replace("particle_integrate(dt)", "update(dt)"),
    );
    let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect_err("true recursion");
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("recursive function call to `update`")),
        "{errors:?}"
    );
}

#[test]
fn particle_checker_handles_valid_and_recursive_programs_on_a_small_stack() {
    // An explicit ordinary thread budget keeps RUST_MIN_STACK in a developer's
    // environment from hiding the CI/embedding regression.
    std::thread::Builder::new()
        .name("particle-checker-small-stack".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            sample_authors_emitter_using_engine_modules();
            authored_compute_recursion_is_still_rejected();
        })
        .expect("spawn compiler regression thread")
        .join()
        .expect("compiler should succeed or diagnose recursion without exhausting the stack");
}

#[test]
fn emitter_modules_preserve_engine_extended_state() {
    let mut inputs = files();
    inputs.insert(
        CONTRACT.into(),
        include_str!("../engine/core/06_particle_contract.fr")
            .replace("    id: f32\n", "    id: f32\n    heat: f32\n")
            .replace(
                "lifespan: max_lifespan, id: id",
                "lifespan: max_lifespan, id: id, heat: 0.0",
            ),
    );
    inputs.insert("main.fr".into(), include_str!("../../../examples/50) particles/drifting_sparks.fr")
        .replace("particle_integrate(dt)", "add_heat(dt)\nparticle_integrate(dt)")
        + "\nfn add_heat<T>(p: T, dt: f32) -> T { var next = p; next.heat = p.heat + dt; return next }\n");
    let output = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect("extended engine record");
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let particle = particle_json(&manifest);
    let fields = particle["state_fields"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|field| field["name"] == "heat" && field["offset"] == 44)
    );
    let contract = inputs.get_mut(CONTRACT).unwrap();
    *contract = contract.replace(", heat: 0.0", "");
    let errors = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect_err("uninitialized state extension");
    assert!(
        errors.iter().any(|error| error.message.contains("heat")),
        "{errors:?}"
    );
}

fn managed_files() -> HashMap<String, String> {
    let mut inputs = files();
    inputs.insert(
        CONTRACT.into(),
        include_str!("../engine/core/06_particle_contract.fr").into(),
    );
    inputs.insert(
        "main.fr".into(),
        include_str!("../../../examples/50) particles/drifting_sparks.fr").into(),
    );
    inputs
}

#[test]
fn managed_allocation_is_explicit_and_properties_have_source_ranges() {
    let inputs = managed_files();
    let output = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let pipeline = &particle_json(&manifest);
    assert_eq!(pipeline["allocation"]["spawn_rate"], 60.0);
    assert_eq!(pipeline["allocation"]["mode"], "fixed");
    assert!(
        pipeline["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["binding"] == 4)
    );
    let props = pipeline["properties"].as_array().unwrap();
    let shared: ParticleContract =
        serde_json::from_value(pipeline.clone()).expect("shared particle contract");
    let round_trip: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&shared).unwrap()).unwrap();
    assert_eq!(round_trip, *pipeline);
    let rate = props.iter().find(|p| p["name"] == "spawn_rate").unwrap();
    let start = rate["value_span"]["start"].as_u64().unwrap() as usize;
    let end = rate["value_span"]["end"].as_u64().unwrap() as usize;
    assert_eq!(&inputs["main.fr"][start..end], "60.0");
    let flag = props
        .iter()
        .find(|p| p["name"] == "simulate_motion")
        .unwrap();
    assert_eq!(flag["permutation"], true);
    assert_eq!(flag["choices"].as_array().unwrap().len(), 2);
    let mut edited = inputs.clone();
    let at = flag["insert_at"].as_u64().unwrap() as usize;
    edited
        .get_mut("main.fr")
        .unwrap()
        .insert_str(at, "simulate_motion: false\n");
    let changed = fresco::driver::compile_bundle_virtual(&edited, "main.fr", false).unwrap();
    assert_ne!(output.wgsl, changed.wgsl);
    for (from, to, message) in [
        ("@meta(allocation_mode, allocation)", "", "allocation_mode"),
        ("@meta(growth_factor, 2.0)", "", "explicit growth_factor"),
        (
            "@meta(growth_factor, 2.0)",
            "@meta(growth_factor, 1.0)",
            "growth factor",
        ),
        (
            "@meta(overflow, \"drop_new\")",
            "@meta(overflow, \"overwrite\")",
            "overflow",
        ),
        (
            "@meta(max_lifespan, max_lifespan)",
            "@meta(max_lifespan, 0.0)",
            "lifespan",
        ),
        (
            "@meta(spawn_rate, spawn_rate)",
            "@meta(spawn_rate, -1.0)",
            "non-negative",
        ),
    ] {
        let mut invalid = inputs.clone();
        let contract = invalid.get_mut(CONTRACT).unwrap();
        assert!(contract.contains(from));
        *contract = contract.replace(from, to);
        let errors = compile_and_validate(&invalid).unwrap_err();
        assert!(errors.contains(message), "{errors:?}");
    }
}

#[test]
fn storage_capacity_changes_without_changing_shader_code() {
    let mut inputs = managed_files();
    let capacity = "@meta(capacity, allocation == ParticleAllocationMode.Fixed ? max(1, u32(ceil(spawn_rate * max_lifespan)) + burst_count) : allocation_hint)";
    let contract = inputs.get_mut(CONTRACT).unwrap();
    assert!(contract.contains(capacity));
    *contract = contract.replace(capacity, "@meta(capacity, 128)");
    let first = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap();
    let contract = inputs.get_mut(CONTRACT).unwrap();
    *contract = contract.replace("@meta(capacity, 128)", "@meta(capacity, 256)");
    let second = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap();
    assert_eq!(first.wgsl, second.wgsl);
    assert_ne!(first.manifest, second.manifest);
}

#[test]
fn explicit_method_entries_accept_source_backed_property_overrides() {
    let mut inputs = managed_files();
    inputs.insert(
        "main.fr".into(),
        r#"
import "engine/engine.fr"
emitter example {
    fn spawn(p: Particle, id: f32, count: f32) -> Particle { return p }
    fn update(p: Particle, dt: f32) -> Particle { return p }
}
surface sprite(sp: surf) -> material(unlit) { compose { base(albedo: rgba(1.0, 1.0, 1.0, 1.0)) } }
"#
        .into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    let particle = particle_json(&manifest);
    let properties = particle["properties"].as_array().unwrap();
    let flag = properties
        .iter()
        .find(|p| p["name"] == "simulate_motion")
        .unwrap();
    let at = flag["insert_at"].as_u64().unwrap() as usize;
    inputs
        .get_mut("main.fr")
        .unwrap()
        .insert_str(at, "simulate_motion: false\n");
    fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect("editable explicit method entry");
}

#[test]
fn particle_binding_slots_follow_the_engine_declarations() {
    let mut files = files();
    let contract = files.get_mut(CONTRACT).unwrap();
    *contract = contract
        .replace("@binding(0)", "@binding(8)")
        .replace("@binding(1)", "@binding(9)")
        .replace("@binding(2)", "@binding(10)")
        .replace("@binding(3)", "@binding(11)");
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let pipeline = ParticleContract::for_surface(&manifest, &manifest.surfaces[0].name)
        .unwrap()
        .unwrap();
    assert_eq!(
        pipeline
            .bindings
            .iter()
            .map(|b| b.binding)
            .collect::<std::collections::BTreeSet<_>>(),
        [8, 9, 10, 11].into_iter().collect()
    );
    for binding in &pipeline.bindings {
        assert!(output.wgsl.contains(&format!(
            "@group({}) @binding({})",
            binding.group, binding.binding
        )));
    }
    let source = files.get_mut(CONTRACT).unwrap();
    *source = source.replace(
        "particle_render: buffer<PreviewParticle>",
        "particle_render: buffer<vec4>",
    );
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap_err();
    assert!(
        errors.iter().any(|d| d.message.contains("position")),
        "{errors:?}"
    );
}

fn particle_json(manifest: &serde_json::Value) -> serde_json::Value {
    let root: fresco_artifact::ManifestRoot = serde_json::from_value(manifest.clone()).unwrap();
    serde_json::to_value(ParticleContract::for_surface(&root, &root.surfaces[0].name).unwrap())
        .unwrap()
}

fn compile_and_validate(files: &HashMap<String, String>) -> Result<(), String> {
    let output =
        fresco::driver::compile_bundle_virtual(files, "main.fr", false).map_err(|errors| {
            errors
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })?;
    let root: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    for surface in &root.surfaces {
        ParticleContract::for_surface(&root, &surface.name)?;
    }
    Ok(())
}

#[test]
fn engine_rejects_incompatible_technique_invocations() {
    let output =
        fresco::driver::compile_bundle_virtual(&managed_files(), "main.fr", false).unwrap();
    let value: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
    for (path, replacement) in [
        (
            "/techniques/0/resources/0/source/provider",
            serde_json::json!("unsupported_provider"),
        ),
        (
            "/techniques/0/steps/2/operation/colors/0",
            serde_json::json!("state"),
        ),
        (
            "/techniques/0/steps/0/enabled",
            serde_json::json!("unrecognized"),
        ),
        (
            "/techniques/0/steps/1/operation/extent/parameter",
            serde_json::json!("other_count"),
        ),
        (
            "/techniques/0/steps/2/operation/instances/parameter",
            serde_json::json!("other_count"),
        ),
        (
            "/techniques/0/steps/0/bindings/particles",
            serde_json::json!("other_state"),
        ),
    ] {
        let mut changed = value.clone();
        *changed.pointer_mut(path).expect("technique contract field") = replacement;
        let root: fresco_artifact::ManifestRoot = serde_json::from_value(changed).unwrap();
        assert!(
            ParticleContract::for_surface(&root, &root.surfaces[0].name).is_err(),
            "{path}"
        );
    }
}
