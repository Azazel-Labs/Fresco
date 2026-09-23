use fresco::driver::compile_bundle_virtual;
use std::collections::HashMap;

#[path = "support/common.rs"]
mod common;

fn compile(
    body: &str,
) -> Result<fresco::driver::CompileBundleOutput, Vec<fresco::driver::DiagnosticRecord>> {
    let files = HashMap::from([
        (
            "main.fr".into(),
            format!("canvas t(uv: coord, time: signal) -> color {{ {body} }}"),
        ),
        (
            "engine/engine.fr".into(),
            include_str!("../../../tests/render-policy/engine/engine.fr").into(),
        ),
    ]);
    compile_bundle_virtual(&files, "main.fr", true)
}

#[test]
fn every_cell_layout_lowers_owner_identity_and_filtered_coverage() {
    for layout in ["square", "brick", "hex", "jittered", "voronoi"] {
        let jitter = if matches!(layout, "jittered" | "voronoi") {
            ", jitter: 1.0"
        } else {
            ""
        };
        let body = format!(
            "compose {{ in space translate(by: (-0.4, -0.3)) . cells(layout: {layout}, every: (0.2, 0.3), seed: 17, sampling: grid2x2{jitter}, cell: tile) {{ circle(at: tile.center, radius: 0.06) |> fill(rgba(tile.rand, fract(tile.id.x * 0.3), tile.uv.y, 1.0)) }} }}"
        );
        let output = compile(&body).unwrap_or_else(|errors| panic!("{layout}: {errors:?}"));
        assert!(output.wgsl.contains("p_space"), "{layout}: {}", output.wgsl);
        assert!(output.wgsl.contains("cellular_filtered_alpha"), "{layout}");
        assert!(
            output.wgsl.contains("bitcast<u32>"),
            "integer seeded hash: {layout}"
        );
        assert!(
            !output.wgsl.contains("aa_adaptive"),
            "occupancy must be integrated once, not blurred a second time: {layout}"
        );
    }
}

#[test]
fn cellular_inputs_fail_at_the_language_boundary() {
    for (arguments, diagnostic) in [
        (
            "layout: unknown, every: 0.2, seed: 0, sampling: grid2x2",
            "cells layout",
        ),
        ("layout: square, every: 0.2, seed: 0", "sampling"),
        ("layout: square, every: 0.2, sampling: grid2x2", "seed"),
        (
            "layout: square, every: time, seed: 0, sampling: grid2x2",
            "compile-time lengths",
        ),
        (
            "layout: square, every: (0.2, 0.0), seed: 0, sampling: grid2x2",
            "positive finite",
        ),
        (
            "layout: square, every: 0.2, seed: -1, sampling: grid2x2",
            "cells seed",
        ),
        (
            "layout: square, every: 0.2, seed: 0.5, sampling: grid2x2",
            "cells seed",
        ),
        (
            "layout: square, every: 0.2, seed: 0, sampling: grid5x5",
            "cells sampling",
        ),
        (
            "layout: square, every: 0.2, seed: 0, sampling: 0",
            "cells sampling",
        ),
        (
            "layout: square, every: 0.2, seed: 0, sampling: 4",
            "cells sampling",
        ),
        (
            "layout: square, every: 0.2, seed: 0, sampling: time",
            "cells sampling",
        ),
        (
            "layout: square, every: 0.2, seed: 0, samples: 4",
            "sampling",
        ),
        (
            "layout: voronoi, every: 0.2, seed: 0, sampling: grid2x2",
            "jitter",
        ),
        (
            "layout: voronoi, every: 0.2, seed: 0, sampling: grid2x2, jitter: 1.1",
            "cells jitter",
        ),
        (
            "layout: square, every: 0.2, seed: 0, sampling: grid2x2, jitter: 0.5",
            "jitter",
        ),
        (
            "layout: square, every: 0.2, seed: 0, sampling: grid2x2, cell: 7",
            "binding name",
        ),
    ] {
        let errors = compile(&format!(
            "compose {{ in space cells({arguments}) {{ fill(#fff) }} }}"
        ))
        .expect_err(arguments);
        assert!(
            errors.iter().any(|e| e.message.contains(diagnostic)),
            "{arguments}: {errors:?}"
        );
    }
}

#[test]
fn cellular_binding_scope_and_transform_order_are_checked() {
    let errors = compile("compose { in space cells(layout: hex, every: 0.2, seed: 0, sampling: grid2x2, cell: tile) . rotate(20deg) { fill(#fff) } }")
        .expect_err("post-cell transforms must use a nested evaluation frame");
    assert!(errors.iter().any(|e| e.message.contains("final transform")));
    let errors = compile("compose { in space cells(layout: square, every: 0.2, seed: 0, sampling: grid2x2, cell: tile) { fill(#fff) }; fill(rgba(tile.rand, 0.0, 0.0, 1.0)) }")
        .expect_err("cell cannot escape its block");
    assert!(
        errors.iter().any(|e| e.message.contains("tile")),
        "{errors:?}"
    );
}

#[test]
fn nested_cells_and_named_spaces_preserve_deterministic_lowering() {
    let body = "space tiles = cells(layout: square, every: 0.4, seed: 1, sampling: grid2x2)
        compose { in space tiles { in space rotate(15deg) . cells(layout: brick, every: 0.1, seed: 2, sampling: grid2x2, cell: brick) { circle(at: brick.center, radius: 0.02) |> fill(rgba(brick.rand, 0.3, 0.5, 1.0)) } } }";
    let first = compile(body).expect("nested cell spaces");
    let second = compile(body).expect("repeat compile");
    assert_eq!(first.wgsl, second.wgsl);
    assert_eq!(first.manifest, second.manifest);
    // Nested cells share the outer sampling domain rather than compounding budgets.
    assert_eq!(first.wgsl.matches("let cellular_filtered_alpha").count(), 1);
}

#[test]
fn cellular_sample_budget_does_not_duplicate_shader_body() {
    for layout in ["square", "brick", "hex", "jittered", "voronoi"] {
        let jitter = if matches!(layout, "jittered" | "voronoi") {
            ", jitter: 1.0"
        } else {
            ""
        };
        let shader = |sampling| {
            compile(&format!(
                "compose {{ in space cells(layout: {layout}, every: 0.11, seed: 37, sampling: {sampling}{jitter}, cell: tile) {{ circle(at: tile.center, radius: 0.018 + 0.02 * hash(tile)) |> fill(rgba(tile.rand, tile.uv.x, tile.uv.y, 0.7)) }} }}"
            ))
            .expect("bounded cellular loops must validate")
            .wgsl
        };
        let four = shader("grid2x2");
        let sixteen = shader("grid4x4");
        for (sampling, count) in [
            ("center", 1),
            ("grid2x2", 4),
            ("grid3x3", 9),
            ("grid4x4", 16),
        ] {
            let wgsl = shader(sampling);
            assert!(
                wgsl.contains(&format!(">= {count}f")),
                "{layout}: {sampling} loop bound"
            );
            assert!(
                wgsl.len().abs_diff(four.len()) < 128,
                "{layout}: {sampling} expanded the shader"
            );
        }
        assert!(
            sixteen.len().abs_diff(four.len()) < 128,
            "{layout}: sample count expanded the body: {} -> {} bytes",
            four.len(),
            sixteen.len()
        );
        let loops = if matches!(layout, "hex" | "voronoi") {
            2
        } else {
            1
        };
        assert_eq!(sixteen.matches("loop {").count(), loops, "{layout}");
        assert!(sixteen.contains(">= 16f"), "sample loop bound: {layout}");
        if loops == 2 {
            assert!(sixteen.contains(">= 25f"), "search loop bound: {layout}");
        }
    }
}

#[test]
fn cell_geometry_queries_compile_for_every_layout_and_units() {
    for layout in ["square", "brick", "hex", "jittered", "voronoi"] {
        let jitter = if matches!(layout, "jittered" | "voronoi") {
            ", jitter: 1.0"
        } else {
            ""
        };
        let output = compile(&format!("in space cells(layout: {layout}, every: (0.2, 0.3), seed: 7, sampling: grid3x3{jitter}, cell: tile) {{ let edge = tile.edge_distance; let a = tile.inset_distance(by: 0.037); let b = tile.inset_distance(by: 2px); let c = tile.inset_distance(by: 1px * 2.0); let point = tile.boundary_point(angle: 30deg); compose {{ fill(rgba(edge + a, b + c, tile.local.x + tile.angle, 1)); circle(at: point, radius: 0.01) |> fill(#fff) }} }}"))
            .expect("cell geometry and pixel insets must validate");
        assert!(
            output.wgsl.len() < 35_000,
            "{layout}: geometry should remain bounded"
        );
    }
    for body in [
        "in space cells(layout: hex, every: 0.2, seed: 1, sampling: center, cell: t) { fill(grey(t.inset_distance())) }",
        "in space cells(layout: hex, every: 0.2, seed: 1, sampling: center, cell: t) { circle(at: t.boundary_point(angle: 0deg, nonsense: 1), radius: 0.01) |> fill(#fff) }",
    ] {
        assert!(
            compile(body).is_err(),
            "invalid geometry call must be diagnosed"
        );
    }
}

#[test]
fn sampling_variants_do_not_shadow_coordinate_anchors() {
    for sampling in [
        "center",
        "CellSampling.center",
        "grid3x3",
        "CellSampling.grid3x3",
    ] {
        compile(&format!("let origin = center; compose {{ circle(at: origin, radius: 0.1) |> fill(#fff); in space cells(layout: hex, every: 0.2, seed: 0, sampling: {sampling}, cell: tile) {{ circle(at: tile.center, radius: 0.03) |> fill(#fff) }} }}"))
            .expect("contextual sampling variants preserve bare coordinate anchors");
    }
}

#[test]
fn hex_circuit_geometry_queries_keep_compact_codegen() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/90) gallery/hex_circuit.fr");
    let source = include_str!("../../../tests/fixtures/hex_circuit_geometry.fr");
    let output = common::compile_example(source, &path.to_string_lossy(), true)
        .expect("hex circuit cell geometry queries");
    assert!(
        output.wgsl.len() < 12_000,
        "hex geometry expanded to {} bytes",
        output.wgsl.len()
    );
    assert_eq!(
        output.wgsl.matches("loop {").count(),
        2,
        "hex contours must not add neighbor searches"
    );
}

#[test]
fn cellular_polka_gallery_has_bounded_codegen_size() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/90) gallery/cellular_polka.fr");
    let source = std::fs::read_to_string(&path).expect("gallery source");
    let output = common::compile_example(&source, &path.to_string_lossy(), true)
        .expect("cellular gallery with its authored engine context");
    assert!(
        output.wgsl.len() < 30_000,
        "cellular gallery expanded to {} bytes",
        output.wgsl.len()
    );
    assert_eq!(output.wgsl.matches("loop {").count(), 5);
}

#[test]
fn cellular_loop_cached_expressions_do_not_escape_to_later_layers() {
    compile(
        "let ramp = gradient(along: y, stops: [stop(at: 0.0, color: #123), stop(at: 1.0, color: #abc)])
         compose {
             in space cells(layout: voronoi, every: 0.2, jitter: 1.0, seed: 7, sampling: grid4x4, cell: tile) {
                 circle(at: tile.center, radius: 0.05) |> fill(ramp)
             }
             circle(at: (0.5, 0.5), radius: 0.1) |> fill(ramp)
         }",
    )
    .expect("gradient dither emitted inside the sample loop must not escape its scope");
}

fn compile_contour(
    body: &str,
) -> Result<fresco::driver::CompileBundleOutput, Vec<fresco::driver::DiagnosticRecord>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/90) gallery/hex_circuit.fr");
    common::compile_example(
        &format!("canvas t(ctx: CanvasContext) -> color {{ {body} }}"),
        &path.to_string_lossy(),
        true,
    )
}

#[test]
fn contours_share_geometry_and_support_motion_in_every_layout() {
    for layout in ["square", "brick", "hex", "jittered", "voronoi"] {
        let jitter = if matches!(layout, "jittered" | "voronoi") {
            ", jitter: 1.0"
        } else {
            ""
        };
        let output = compile_contour(&format!(r#"
            in space cells(layout: {layout}, every: (0.2, 0.3), seed: 7, sampling: center{jitter}, cell: tile) {{
                let track = tile.contour(inset: 2px)
                let pulse = chase(along: track, speed: 20px/s, tail: 8px, phase: tile.rand)
                let reverse = chase(along: track, lap: 6s, tail: 0.1, direction: counterclockwise)
                let point = track.point(at: 0.25)
                let wire = band(track.distance, width: 1px)
                let halo = band(track.distance, width: 6px, profile: soft)
                fill(rgb(wire * pulse + point.x, halo * reverse + track.progress, track.length))
            }}
        "#)).unwrap_or_else(|errors| panic!("{layout}: {errors:?}"));
        assert!(
            output.wgsl.len() < 45_000,
            "{layout}: {}",
            output.wgsl.len()
        );
        assert_eq!(
            output.wgsl.matches("var contour_planes:").count(),
            1,
            "geometry should be shared: {}",
            output.wgsl
        );
    }
}

#[test]
fn contour_helpers_reject_invalid_arguments() {
    for (expression, diagnostic) in [
        ("tile.contour(inset: -0.1)", "nonnegative"),
        (
            "chase(along: track, head: 0.2, lap: 2s, tail: 0.1)",
            "exactly one",
        ),
        (
            "chase(along: track, speed: 20px/s, motion: angular, tail: 0.1)",
            "angular chase",
        ),
        ("band(0.1, width: 0)", "positive"),
        ("band(0.1, width: 0.1, profile: fuzzy)", "profile must"),
        (
            "chase(along: 1, speed: 20px/s, tail: 2px)",
            "must be a contour",
        ),
        ("chase(along: track, tail: 0.1)", "exactly one"),
        (
            "chase(along: track, lap: 2s, speed: 20px/s, tail: 0.1)",
            "exactly one",
        ),
        ("chase(along: track, lap: 0s, tail: 0.1)", "positive"),
        (
            "chase(along: track, lap: 2s, tail: 0.1, direction: sideways)",
            "direction must",
        ),
    ] {
        let errors = compile_contour(&format!("in space cells(layout: hex, every: 0.2, seed: 1, sampling: center, cell: tile) {{ let track = tile.contour() let result = {expression} fill(#fff) }}")).expect_err(expression);
        assert!(
            errors.iter().any(|d| d.message.contains(diagnostic)),
            "{expression}: {errors:?}"
        );
    }
}

#[test]
fn contour_band_preserves_authored_overloads_and_scoped_rates() {
    let output = compile_contour("let old = band(0.1, 0.9, 0.1, 0.5); let new = band(0.02, width: 0.1, profile: soft); fill(rgb(old, new, 0))").expect("both band overloads");
    assert!(!output.wgsl.is_empty());
}

#[test]
fn hex_contour_gallery_has_shared_bounded_geometry() {
    let source = include_str!("../../../examples/90) gallery/hex_circuit.fr");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/90) gallery/hex_circuit.fr");
    let output =
        common::compile_example(source, &path.to_string_lossy(), true).expect("contour gallery");
    assert!(
        output.wgsl.len() < 10_000,
        "contour gallery: {} bytes",
        output.wgsl.len()
    );
    assert_eq!(
        output.wgsl.matches("var contour_planes").count(),
        0,
        "angular motion and edge fields must not build contour arrays"
    );
    eprintln!("contour gallery: {} bytes", output.wgsl.len());
}

#[test]
fn contour_direction_variants_do_not_shadow_polar_direction() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/10) fundamentals/polar_dial.fr");
    let source = std::fs::read_to_string(&path).expect("polar example");
    common::compile_example(&source, &path.to_string_lossy(), true)
        .expect("contextual contour direction must not shadow polar directions");
}

#[test]
fn regular_contour_motion_uses_one_edge_walk_without_clipping() {
    let output = compile_contour("in space cells(layout: hex, every: 0.2, seed: 1, sampling: center, cell: tile) { contour track = tile.contour(inset: 0.04); let pulse = chase(along: track, speed: 20px/s, tail: 3px); fill(rgb(band(track.distance, width: 1px), pulse, track.length)) }").expect("typed contour and regular lowering");
    assert!(!output.wgsl.contains("contour_planes"));
    assert!(!output.wgsl.contains("contour_clip"));
    assert_eq!(output.wgsl.matches("var contour_edge:").count(), 1);
    assert!(output.wgsl.len() < 13_000, "{} bytes", output.wgsl.len());
}
