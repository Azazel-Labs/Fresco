use fresco::driver::compile_bundle_virtual;
use std::collections::HashMap;

fn compile(body: &str) -> fresco::driver::CompileBundleOutput {
    let source = format!(
        "canvas t(uv: coord, time: signal) -> color {{
            let g = gradient(along: x, stops: [stop(at: 0.0, color: #f008), stop(at: 1.0, color: #08fc)])
            let disk = circle(at: (0.5, 0.5), radius: 0.3)
            {body}
        }}"
    );
    compile_source(&source).unwrap_or_else(|errors| panic!("{body}: {errors:?}"))
}

fn compile_source(
    source: &str,
) -> Result<fresco::driver::CompileBundleOutput, Vec<fresco::driver::DiagnosticRecord>> {
    let files = HashMap::from([
        ("main.fr".into(), source.to_string()),
        (
            "engine/engine.fr".into(),
            include_str!("../../../tests/render-policy/engine/engine.fr").into(),
        ),
    ]);
    compile_bundle_virtual(&files, "main.fr", true)
}

#[test]
fn sampled_gradients_preserve_nested_scopes_and_shape_anchors() {
    for body in [
        "let anchored = gradient(along: x, anchor: shape, stops: [stop(at: 0.0, color: #f00), stop(at: 1.0, color: #00f)]); disk |> fill(#fff) |> tint(anchored)",
        "let anchored = gradient(along: y, anchor: shape, stops: [stop(at: 0.0, color: #f00), stop(at: 1.0, color: #00f)]); disk |> bevel(highlight: lighten(anchored, by: 0.2), shadow: anchored)",
        "let radial = gradient(kind: radial, center: (0.5, 0.5), radius: 0.3, stops: [stop(at: 0.0, color: #f00), stop(at: 1.0, color: #00f)]); disk |> shadow(offset: (0.02, 0.02), soften: 0.02, color: radial)",
        "compose { if time > 1.0 { disk |> fill(#fff) |> tint(g) } else { disk |> soften(radius: 0.03, color: g) }; circle(at: (0.2, 0.2), radius: 0.1) |> stroke(width: 0.02, color: g) }",
        "compose { in space cells(layout: hex, every: 0.2, seed: 1, sampling: grid2x2, cell: tile) { circle(at: tile.center, radius: 0.06) |> fill(#fff) |> tint(g) }; disk |> fill(darken(g, by: 0.2)) }",
    ] {
        compile(body);
    }
    let errors = compile_source("canvas t(uv: coord) -> color { let g = gradient(along: x, anchor: shape, stops: [stop(at: 0.0, color: #000), stop(at: 1.0, color: #fff)]); fill(lighten(g, by: 0.2)) }")
        .expect_err("a fullscreen color has no shape anchor");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("requires a shape receiver"))
    );
}

#[test]
fn material_color_inputs_accept_surface_uv_gradients() {
    compile_source("material_properties gradient_channels { channel albedo: color; channel emissive: color }\nsurface t(sp: surf) -> material(gradient_channels) { let g = gradient(along: x, stops: [stop(at: 0.0, color: #f00), stop(at: 1.0, color: #00f)]); compose { base(albedo: g, emissive: g) } }")
        .expect("material gradient colors");
    for color in [
        "g",
        "lighten(g, by: 0.2)",
        "gradient(along: y, stops: [stop(at: 0.0, color: g), stop(at: 1.0, color: #fff)])",
    ] {
        let errors = compile_source(&format!("surface t(sp: surf) -> material {{ let g = gradient(along: x, anchor: shape, stops: [stop(at: 0.0, color: #f00), stop(at: 1.0, color: #00f)]); compose {{ base(albedo: {color}) }} }}"))
            .expect_err("material colors have no shape receiver, including nested gradients");
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("requires a shape receiver"))
        );
    }
}

#[test]
fn color_inputs_accept_gradients_across_paint_effects_and_transforms() {
    for body in [
        "fill(g)",
        "disk |> fill(g)",
        "disk |> stroke(width: 0.02, color: g)",
        "disk |> shadow(offset: (0.02, 0.03), soften: 0.02, color: g)",
        "disk |> soften(radius: 0.02, color: g)",
        "disk |> glow(reach: 0.05, strength: 1.0, color: g)",
        "disk |> inner_glow(reach: 0.05, strength: 1.0, color: g)",
        "disk |> bevel(highlight: g, shadow: g)",
        "disk |> fill(#fff) |> tint(g)",
        "disk |> fill(#fff) |> soften(radius: 0.02, color: g)",
        "disk |> fill(#fff) |> inner_glow(reach: 0.05, strength: 1.0, color: g)",
        "disk |> fill(#fff) |> bevel(highlight: g, shadow: g)",
        "disk |> fill(#fff) |> glow(reach: 0.05, strength: 1.0, color: g)",
        "disk |> fill(g) |> soften(radius: 0.02)",
        "disk |> fill(g) |> inner_glow(reach: 0.05, strength: 1.0)",
        "disk |> fill(g) |> bevel(highlight: g, shadow: g)",
        "fill(lighten(g, by: 0.2))",
        "fill(darken(g, by: 0.2))",
        "fill(saturate(g, by: 0.2))",
        "fill(desaturate(g, by: 0.2))",
        "fill(mix(g, #fff, 0.3))",
        "fill(gradient(along: y, stops: [stop(at: 0.0, color: g), stop(at: 1.0, color: #fff)]))",
        "let p = path_svg(\"M 0.1 0.1 L 0.9 0.9\"); p |> stroke(width: 0.02, color: g)",
        "let p = path_svg(\"M 0.1 0.1 L 0.9 0.1 L 0.5 0.9 Z\"); p |> fill(g)",
    ] {
        let output = compile(body);
        assert!(
            output.wgsl.contains("mix("),
            "gradient interpolation: {body}"
        );
    }
}

#[test]
fn genuine_color_metadata_advertises_gradient_support() {
    let docs = fresco::language::docs_model();
    let mut colors = 0;
    for builtin in &docs.builtin_reference {
        for signature in &builtin.signatures {
            for arg in &signature.args {
                if arg.ty.split(" | ").any(|ty| ty == "color") {
                    colors += 1;
                    assert!(
                        arg.ty.split(" | ").any(|ty| ty == "gradient"),
                        "{}.{}",
                        builtin.name,
                        arg.name
                    );
                }
            }
        }
    }
    assert!(
        colors >= 20,
        "the color-input audit must cover actual declarations"
    );
    for name in ["image", "bit_and", "path_svg"] {
        let builtin = docs
            .builtin_reference
            .iter()
            .find(|b| b.name == name)
            .expect("builtin");
        assert!(
            builtin
                .signatures
                .iter()
                .flat_map(|s| &s.args)
                .all(|a| !a.ty.contains("color")),
            "raw expressions must not masquerade as colors: {name}"
        );
    }
}
