//! Compute-written vertex streams use checked, distinct shell offsets.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{
        FrameInputs,
        preview::{PreviewGeometry, sphere_scene},
    },
    runtime::{depth::DepthTarget, mesh::MeshRenderer, mesh_geometry::MeshGeometry},
};
use std::error::Error;

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let shape = PreviewGeometry::sphere();
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let source = include_str!("../../tests/fixtures/generated-shells.fr");
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut results = Vec::new();
        for offset in [
            "checked_mul(layer, geometry.vertex_count)",
            "0u",
            "checked_mul(3u, geometry.vertex_count)",
        ] {
            let mut files = fresco_example_engine::source_files_for_recipe(renderer);
            files.insert(
                "main.fr".into(),
                source.replace(
                    "base_vertex checked_mul(layer, geometry.vertex_count)",
                    &format!("base_vertex {offset}"),
                ),
            );
            let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                .map_err(|errors| format!("{errors:?}"))?;
            let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
            let factory = manifest
                .vertex_factories
                .iter()
                .find(|f| f.name == "preview_static")
                .unwrap();
            let geometry = MeshGeometry::prepare(
                device,
                queue,
                factory,
                shape.vertex_count,
                &shape.streams(),
                Some(&shape.indices),
            )
            .await?;
            let mut draw = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "item",
                geometry,
                FORMAT,
            )
            .await?;
            let mut scene = sphere_scene(FrameInputs {
                time: 0.0,
                delta_time: 0.0,
                physical_size: [SIZE, SIZE],
            });
            scene.displacement.enabled = false;
            let rendered = draw.render(&scene, view, depth.view());
            if offset == "checked_mul(3u, geometry.vertex_count)" {
                let error =
                    rendered.expect_err("out-of-capacity shell must fail before submission");
                assert!(error.to_string().contains("generated vertex"), "{error}");
                continue;
            }
            rendered?;
            let pixels = readback(device, queue, output)?;
            draw.render(&scene, view, depth.view())?;
            assert_eq!(
                pixels,
                readback(device, queue, output)?,
                "{renderer}: fresh frame must preserve generated geometry"
            );
            draw.update_parameters(serde_json::json!({"style.limit": 2}).as_object().unwrap())?;
            let error = draw
                .render(&scene, view, depth.view())
                .expect_err("captured draw precondition must be checked each frame");
            assert!(error.to_string().contains("precondition failed"), "{error}");
            draw.update_parameters(serde_json::json!({"style.limit": 3}).as_object().unwrap())?;
            draw.render(&scene, view, depth.view())?;
            assert_eq!(
                pixels,
                readback(device, queue, output)?,
                "{renderer}: rejected frame must not corrupt later output"
            );
            results.push(pixels);
        }
        let changed = results[0]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(results[1].as_chunks::<4>().0)
            .filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 2))
            .count();
        assert!(
            changed > 32,
            "{renderer}: distinct shell offsets must change rendered coverage ({changed} pixels)"
        );
        verify_bounds(device, queue, output, view, renderer).await?;
    }
    println!(
        "Generated geometry: compute outputs change shell coverage, fresh frames agree, invalid offsets reject in every renderer."
    );
    Ok(())
}

async fn verify_bounds(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
    renderer: &str,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files_for_recipe(renderer);
    files.insert(
        "main.fr".into(),
        include_str!("../../tests/fixtures/generated-shells.fr").into(),
    );
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| f.name == "preview_static")
        .unwrap();
    // Source geometry lies just beyond the right frustum plane. Compute moves
    // each generated shell left, so only expanded contribution bounds admit it.
    let mut shape = PreviewGeometry {
        positions: vec![1.26, -0.3, 0.0, 1.5, -0.3, 0.0, 1.26, 0.3, 0.0],
        normals: vec![-1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        tangents: vec![0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0],
        uv: vec![0.0; 6],
        uv2: vec![0.0; 6],
        indices: vec![0, 1, 2],
        vertex_count: 3,
    };
    // Another material range shares the upload but must not enlarge this range.
    shape
        .positions
        .extend_from_slice(&[-2.0, -0.3, 0.0, -1.7, -0.3, 0.0, -2.0, 0.3, 0.0]);
    shape
        .normals
        .extend_from_slice(&[-1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
    shape
        .tangents
        .extend_from_slice(&[0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
    shape.uv.extend_from_slice(&[0.0; 6]);
    shape.uv2.extend_from_slice(&[0.0; 6]);
    shape.indices.extend_from_slice(&[3, 4, 5]);
    shape.vertex_count = 6;
    let geometry = MeshGeometry::prepare(
        device,
        queue,
        factory,
        shape.vertex_count,
        &shape.streams(),
        Some(&shape.indices),
    )
    .await?
    .with_draw_range(0..3)?;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let mut draw = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "item",
        geometry.clone(),
        FORMAT,
    )
    .await?;
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.model = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    scene.displacement.enabled = false;
    let mut images = Vec::new();
    for expansion in [0.0, 0.2] {
        draw.update_parameters(
            serde_json::json!({"style.expansion": expansion})
                .as_object()
                .unwrap(),
        )?;
        draw.render(&scene, view, depth.view())?;
        images.push(readback(device, queue, output)?);
    }
    let changed = images[0]
        .as_chunks::<4>()
        .0
        .iter()
        .zip(images[1].as_chunks::<4>().0)
        .filter(|(a, b)| b[0] > a[0].saturating_add(5))
        .count();
    assert!(
        changed > 8,
        "{renderer}: expanded bounds must reveal shells beyond the source frustum ({changed} pixels)"
    );
    let mut other_view = scene;
    other_view.view[12] += 4.0;
    draw.render(&other_view, view, depth.view())?;
    assert_eq!(
        images[0],
        readback(device, queue, output)?,
        "{renderer}: another view must cull from its own frustum"
    );
    draw.render(&scene, view, depth.view())?;
    assert_eq!(
        images[1],
        readback(device, queue, output)?,
        "{renderer}: restoring the view restores visibility"
    );
    draw.update_parameters(
        serde_json::json!({"style.expansion": -0.1})
            .as_object()
            .unwrap(),
    )?;
    let error = draw
        .render(&scene, view, depth.view())
        .expect_err("negative bounds expansion must reject before submission");
    assert!(error.to_string().contains("nonnegative"), "{error}");

    let source = include_str!("../../tests/fixtures/generated-shells.fr").replace(
        "properties { style: Shells }", "vertex { position: sp.position + vec3(-0.01, 0.0, 0.0) }\n    properties { style: Shells }");
    files.insert("main.fr".into(), source);
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
    let mut custom = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "item",
        geometry.clone(),
        FORMAT,
    )
    .await?;
    let error = custom
        .render(&scene, view, depth.view())
        .expect_err("arbitrary vertex code cannot inherit undeformed bounds");
    assert!(
        error
            .to_string()
            .contains("prepared bounds are unavailable"),
        "{error}"
    );
    let conservative = geometry
        .source_bounds("position")?
        .unwrap()
        .expand(0.01)?
        .transform(&scene.model)?;
    custom.set_prepared_bounds(Some(conservative))?;
    custom.render(&scene, view, depth.view())?;
    let custom_pixels = readback(device, queue, output)?;
    assert!(
        images[0]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(custom_pixels.as_chunks::<4>().0)
            .filter(|(a, b)| b[0] > a[0].saturating_add(5))
            .count()
            > 8,
        "{renderer}: explicit deformed bounds must reach compute, draw, and culling"
    );
    Ok(())
}
