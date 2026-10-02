use std::collections::HashMap;

#[path = "support/wgsl.rs"]
mod wgsl;
use wgsl::{Capabilities, assert_valid_wgsl};

fn files(kind: &str, method: &str, body: &str) -> HashMap<String, String> {
    // These engines register their own entry vocabulary (including `canvas`),
    // so they take only the rendering policy, not the shared `canvas` contract.
    let policy = include_str!("../../fresco/tests/fixtures/engines/policy/engine/engine.fr");
    HashMap::from([
        (
            "engine/engine.fr".into(),
            format!(
                r#"{policy}
struct SampleContext {{ @semantic(coord) uv: vec2 }}
struct Varying {{ clip_pos: vec4, uv: vec2 }}
@entry({kind}, {method})
interface Painting {{ fn {method}(@context sample: SampleContext) -> color }}
pass present for Painting {{
    stage: raster
    draw: fullscreen
    fn vertex(vertex_id: u32) -> Varying {{
        let p = fullscreen_triangle_position(vertex_id)
        return Varying(clip_pos: vec4(p, 0.0, 1.0), uv: p * 0.5 + vec2(0.5))
    }}
    fn shade(v: Varying, painting: Painting) -> color {{
        return painting.{method}(SampleContext(uv: v.uv))
    }}
}}
pipeline display for Painting {{ present }}
"#
            ),
        ),
        ("main.fr".into(), body.into()),
    ])
}

#[test]
fn engine_vocabulary_can_rename_entries_and_blocks() {
    for (kind, method) in [
        ("picture", "paint"),
        ("widget", "evaluate"),
        ("canvas", "draw"),
    ] {
        let source = format!(
            "{kind} demo {{ {method} {{ return rgba(sample.uv.x, sample.uv.y, 0.0, 1.0) }} }}"
        );
        let output =
            fresco::driver::compile_bundle_virtual(&files(kind, method, &source), "main.fr", false)
                .unwrap_or_else(|errors| panic!("{kind}: {errors:?}"));
        assert_valid_wgsl(&output.wgsl, Capabilities::all());
        let manifest: serde_json::Value = serde_json::from_str(&output.manifest).unwrap();
        assert_eq!(manifest["raster_entries"][0]["name"], "demo");
        assert!(output.wgsl.contains("fresco_demo"));
    }
}

#[test]
fn explain_receipt_names_the_registered_entry_kind() {
    for (kind, method) in [("picture", "paint"), ("canvas", "draw")] {
        let source = format!("{kind} demo {{ {method} {{ return #fff }} }}");
        let output =
            fresco::driver::compile_bundle_virtual(&files(kind, method, &source), "main.fr", true)
                .unwrap_or_else(|errors| panic!("{kind}: {errors:?}"));
        let explain = output.explain.expect("explain requested");
        assert!(
            explain.starts_with(&format!("== fresco --explain: {kind} `demo` ==")),
            "{kind}: explain header should name the entry kind
{explain}"
        );
    }
}

#[test]
fn registered_shorthand_uses_authored_parameter_and_context_types() {
    let output = fresco::driver::compile_bundle_virtual(
        &files(
            "picture",
            "paint",
            "picture demo(input: SampleContext) -> color { rgba(input.uv.x, input.uv.y, 0.0, 1.0) }",
        ),
        "main.fr",
        false,
    );
    output.expect("registered shorthand");
}

#[test]
fn entry_blocks_call_typed_sibling_modules() {
    let mut files = files(
        "picture",
        "paint",
        "picture demo { tint { return rgba(value, 0.0, 0.0, 1.0) }\npaint { return tint(sample.uv.x) } }",
    );
    let engine = files.get_mut("engine/engine.fr").unwrap();
    *engine = engine.replace("interface Painting { fn paint(@context sample: SampleContext) -> color }",
        "interface Painting {\nfn paint(@context sample: SampleContext) -> color\nfn tint(value: f32) -> color\n}");
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .expect("multiple named blocks");
}

#[test]
fn entry_binding_rejects_missing_unknown_duplicate_and_mistyped_blocks() {
    for (source, expected) in [
        ("picture demo {}", "missing required entry block"),
        (
            "unknown demo { paint { return #fff } }",
            "unregistered entry",
        ),
        (
            "picture demo { typo { return #fff } }",
            "unknown entry block",
        ),
        (
            "picture demo { paint { return #fff }\npaint { return #fff } }",
            "duplicate entry block",
        ),
        ("picture demo { paint { return vec2(1.0) } }", "return"),
        (
            "picture demo(p: SampleContext) -> vec2 { vec2(1.0) }",
            "result type",
        ),
    ] {
        let errors = fresco::driver::compile_bundle_virtual(
            &files("picture", "paint", source),
            "main.fr",
            false,
        )
        .expect_err(source);
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{source}: {errors:?}"
        );
    }
    let mut files = files("picture", "paint", "picture demo { paint { return #fff } }");
    files.get_mut("engine/engine.fr").unwrap().push_str("\n@entry(picture, paint) interface Other { fn paint(@context c: SampleContext) -> color }\n");
    let errors = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .expect_err("duplicate registration");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("duplicate @entry"))
    );
}

fn errors_for(files: &HashMap<String, String>) -> Vec<String> {
    fresco::driver::compile_bundle_virtual(files, "main.fr", false)
        .expect_err("compile should fail")
        .into_iter()
        .map(|error| error.message)
        .collect()
}

#[test]
fn canvas_is_not_built_in_without_an_engine_registration() {
    // `canvas` used to fall back to a compiler-owned standalone ABI. Entry
    // kinds now come only from engine `@entry` registrations.
    for source in [
        "canvas demo { draw { return #fff } }",
        "canvas demo(ctx: CanvasContext) -> color { return #fff }",
        "canvas demo(uv: coord, t: signal, res: resolution) -> color { return rgba(uv.x, t, res.x, 1.0) }",
    ] {
        let errors = errors_for(&files("picture", "paint", source));
        assert!(
            errors
                .iter()
                .any(|message| message.contains("unregistered entry declaration `canvas`")),
            "{source}: {errors:?}"
        );
    }
}

#[test]
fn registered_canvas_rejects_the_legacy_standalone_signature() {
    let source = "canvas demo(uv: coord, t: signal, res: resolution) -> color { return rgba(uv.x, t, res.x, 1.0) }";
    let errors = errors_for(&files("canvas", "draw", source));
    assert!(
        errors.iter().any(|message| message.contains(
            "canvas `demo` must implement Painting.draw with one parameter of type `SampleContext`"
        )),
        "{errors:?}"
    );
}

#[test]
fn entries_declared_in_imported_modules_are_rejected() {
    let mut files = files("canvas", "draw", "import \"parts.fr\"\n");
    files.insert(
        "parts.fr".into(),
        "canvas hidden { draw { return #fff } }\n".into(),
    );
    let errors = errors_for(&files);
    assert!(
        errors.iter().any(|message| message.contains(
            "imported module `parts.fr` contains `canvas` entry `hidden`, which is not supported in imported modules"
        )),
        "{errors:?}"
    );
}

#[test]
fn registered_canvas_methods_use_explicit_context_without_field_name_synthesis() {
    let source = "canvas demo { param gain: f32 = 0.5\nfn draw(@context actual: SampleContext) -> color { return rgba(actual.uv.x * gain, 0.0, 0.0, 1.0) } }";
    fresco::driver::compile_bundle_virtual(&files("canvas", "draw", source), "main.fr", false)
        .expect("registered canvas method uses actual context");
    let source = source.replace("@context actual: SampleContext", "actual: vec2");
    let errors =
        fresco::driver::compile_bundle_virtual(&files("canvas", "draw", &source), "main.fr", false)
            .expect_err("wrong explicit context");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("signature does not match"))
    );
}

#[test]
fn engine_blocks_require_all_methods_and_accept_engine_configuration() {
    let mut files = files("picture", "paint", "picture demo { paint { return #fff } }");
    let engine = files.get_mut("engine/engine.fr").unwrap();
    *engine = engine.replace("interface Painting { fn paint(@context sample: SampleContext) -> color }",
        "interface Painting {\nfn paint(@context sample: SampleContext) -> color\nfn tint(value: f32) -> color\n}");
    let errors =
        fresco::driver::compile_bundle_virtual(&files, "main.fr", false).expect_err("missing tint");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("missing required entry block `tint`")
    }));
    let engine = files.get_mut("engine/engine.fr").unwrap();
    *engine = engine.replace("fn tint(value: f32) -> color", "param intensity: f32 = 1.0");
    fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .expect("engine supplies configuration default");
}

fn configured_files(kind: &str, settings: &str, schema: &str) -> HashMap<String, String> {
    let mut result = files(
        kind,
        "paint",
        &format!("{kind} demo {{ {settings}\npaint {{ return rgba(intensity, 0.0, 0.0, 1.0) }} }}"),
    );
    let engine = result.get_mut("engine/engine.fr").unwrap();
    *engine = engine.replace(
        "interface Painting {",
        &format!("interface Painting {{\n{schema}\n"),
    );
    result
}

#[test]
fn configuration_uses_engine_defaults_and_checks_authored_overrides() {
    for kind in ["picture", "canvas"] {
        let compile = |settings, schema| {
            fresco::driver::compile_bundle_virtual(
                &configured_files(kind, settings, schema),
                "main.fr",
                false,
            )
            .expect("typed settings")
        };
        let default = compile("", "param intensity: f32 = 0.25");
        let overridden = compile("intensity: 0.75", "param intensity: f32 = 0.25");
        let explicit = compile("intensity: 0.75", "param intensity: f32");
        assert_ne!(
            default.wgsl, overridden.wgsl,
            "setting must affect generated shader"
        );
        assert_eq!(
            overridden.wgsl, explicit.wgsl,
            "explicit setting replaces engine default"
        );
        for (settings, schema, expected) in [
            (
                "",
                "param intensity: f32",
                "missing required entry setting `intensity`",
            ),
            (
                "typo: 1.0",
                "param intensity: f32 = 0.25",
                "unknown entry setting `typo`",
            ),
            (
                "intensity: 1.0\nintensity: 2.0",
                "param intensity: f32",
                "duplicate entry setting",
            ),
            (
                "intensity: vec2(1.0)",
                "param intensity: f32",
                "const `intensity` expected",
            ),
            (
                "",
                "param intensity: f32 = vec2(1.0)",
                "const `intensity` expected",
            ),
            (
                "intensity: sample.uv.x",
                "param intensity: f32",
                "compile-time",
            ),
            (
                "",
                "param intensity: f32 = 0.25\nparam intensity: f32 = 0.5",
                "duplicate engine configuration field",
            ),
        ] {
            let errors = fresco::driver::compile_bundle_virtual(
                &configured_files(kind, settings, schema),
                "main.fr",
                false,
            )
            .expect_err(settings);
            assert!(
                errors.iter().any(|error| error.message.contains(expected)),
                "{kind} {settings}: {errors:?}"
            );
        }
    }
}

fn composition_files(calls: &str) -> HashMap<String, String> {
    let mut result = files(
        "picture",
        "paint",
        &format!(
            "picture demo {{\nintensity: 0.25\nsteps {{ {calls} }}\npaint {{ return rgba(steps(sample.uv.x), 0.0, 0.0, 1.0) }}\n}}"
        ),
    );
    let engine = result.get_mut("engine/engine.fr").unwrap();
    *engine = engine.replace(
        "interface Painting {",
        "interface Painting {\nparam intensity: f32\n@compose(value) fn steps(value: f32) -> f32\n",
    );
    engine.push_str("\nfn add(value: f32, amount: f32) -> f32 { return value + amount }\nfn scale(value: f32, amount: f32) -> f32 { return value * amount }\nfn bad(value: f32) -> vec2 { return vec2(value) }\n");
    result
}

#[test]
fn composition_threads_state_in_source_order_and_checks_each_module() {
    let compile = |calls| {
        fresco::driver::compile_bundle_virtual(&composition_files(calls), "main.fr", false)
            .expect("ordered composition")
    };
    let forward = compile("add(intensity)\nscale(2.0)");
    let reverse = compile("scale(2.0)\nadd(intensity)");
    assert_ne!(
        forward.wgsl, reverse.wgsl,
        "module order changes the result"
    );
    compile(""); // An empty stack is identity, not uninitialized state.
    for (calls, expected) in [
        ("bad()", "typed local `value` expected"),
        (
            "let ignored = 1.0",
            "composition blocks require module calls",
        ),
        ("1.0", "composition blocks require module calls"),
        (
            "add(vec2(1.0))",
            "argument `amount` to `add` expected scalar",
        ),
    ] {
        let errors =
            fresco::driver::compile_bundle_virtual(&composition_files(calls), "main.fr", false)
                .expect_err(calls);
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "{calls}: {errors:?}"
        );
    }
    let mut invalid = composition_files("");
    let engine = invalid.get_mut("engine/engine.fr").unwrap();
    *engine = engine.replace("@compose(value)", "@compose(missing)");
    let errors = fresco::driver::compile_bundle_virtual(&invalid, "main.fr", false)
        .expect_err("invalid composition schema");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("@compose requires"))
    );
}

#[test]
fn composition_preserves_extra_record_fields_and_matches_explicit_calls() {
    let mut inputs = files(
        "picture",
        "paint",
        r#"
picture demo {
    steps { shift(0.25)
shift(0.5) }
    paint {
        let result = steps(State(value: sample.uv.x, extra: sample.uv.y))
        return rgba(result.value, result.extra, 0.0, 1.0)
    }
}
"#,
    );
    let engine = inputs.get_mut("engine/engine.fr").unwrap();
    *engine = engine.replace(
        "interface Painting {",
        "interface Painting {\n@compose(p) fn steps(p: State) -> State\n",
    );
    engine.push_str(
        r#"
struct State { value: f32, extra: f32 }
fn shift<T>(p: T, amount: f32) -> T {
    var next = p
    next.value = p.value + amount
    return next
}
"#,
    );
    let composed = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect("record composition");
    inputs.insert(
        "main.fr".into(),
        r#"
picture demo {
    fn steps(p: State) -> State { return shift(shift(p, 0.25), 0.5) }
    fn paint(@context sample: SampleContext) -> color {
        let result = steps(State(value: sample.uv.x, extra: sample.uv.y))
        return rgba(result.value, result.extra, 0.0, 1.0)
    }
}
"#
        .into(),
    );
    let explicit = fresco::driver::compile_bundle_virtual(&inputs, "main.fr", false)
        .expect("explicit methods");
    assert_eq!(
        composed.wgsl, explicit.wgsl,
        "stack threads the complete record exactly like typed function calls"
    );
    assert_valid_wgsl(&composed.wgsl, Capabilities::all());
}

#[test]
fn fullscreen_pass_local_mutation_emits_valid_wgsl() {
    for keyword in ["var", "let"] {
        let mut sources = files("picture", "paint", "picture demo { paint { return #fff } }");
        let engine = sources.get_mut("engine/engine.fr").unwrap();
        *engine = engine.replace(
            "let p = fullscreen_triangle_position(vertex_id)",
            &format!("{keyword} p: vec2 = fullscreen_triangle_position(vertex_id)\np.x += 0.1"),
        );
        let result = fresco::driver::compile_bundle_virtual(&sources, "main.fr", false);
        if keyword == "let" {
            let errors = result.expect_err("immutable pass local");
            assert!(
                errors
                    .iter()
                    .any(|error| error.message.contains("immutable binding `p`"))
            );
        } else {
            let output = result.expect("mutable pass local");
            assert_valid_wgsl(&output.wgsl, Capabilities::all());
        }
    }
}
