//! An authored response changes lighting for the same standard material.
use super::{FORMAT, SIZE, mesh::geometry, readback};
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
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for closed in [false, true] {
        let mut pbr = None;
        let mut toon = None;
        for renderer_path in ["forward", "forward-plus", "deferred"] {
            for symbol in ["StandardGGX", "Toon"] {
                let mut files = fresco_example_engine::source_files();
                files.insert(
                    "main.fr".into(),
                    include_str!("../../../../examples/40) surface shaders/style_sample.fr").into(),
                );
                files.insert("fresco.config.json".into(), serde_json::json!({"renderer": renderer_path, "property_overrides": {"style_sample": {"style": symbol}}}).to_string());
                let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                    .map_err(|e| format!("{e:?}"))?;
                let manifest: fresco_artifact::ManifestRoot =
                    serde_json::from_str(&compiled.manifest)?;
                let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
                assert_eq!(
                    recipe
                        .steps
                        .iter()
                        .filter(|s| s
                            .invocation
                            .as_ref()
                            .is_some_and(|i| i.operation == "InvertedHull"))
                        .count(),
                    usize::from(symbol == "Toon")
                );
                let mesh = if closed {
                    let shape = PreviewGeometry::sphere();
                    MeshGeometry::prepare(
                        device,
                        queue,
                        manifest
                            .vertex_factories
                            .iter()
                            .find(|f| f.name == "preview_static")
                            .unwrap(),
                        shape.vertex_count,
                        &shape.streams(),
                        Some(&shape.indices),
                    )
                    .await?
                } else {
                    geometry(device, queue, &manifest).await?
                };
                let mut renderer = MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &manifest,
                    "style_sample",
                    mesh,
                    FORMAT,
                )
                .await?;
                renderer.render(&scene, view, depth.view())?;
                let pixels = readback(device, queue, output)?;
                if closed {
                    image::save_buffer(
                        format!("target/outline-{renderer_path}-{symbol}.png"),
                        &pixels,
                        SIZE,
                        SIZE,
                        image::ColorType::Rgba8,
                    )?;
                }
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[3] != 0 && p[0] > 0)
                );
                let reference: &mut Option<Vec<u8>> = if symbol == "StandardGGX" {
                    &mut pbr
                } else {
                    &mut toon
                };
                if let Some(reference) = reference {
                    let max_difference = reference
                        .iter()
                        .zip(&pixels)
                        .map(|(a, b)| a.abs_diff(*b))
                        .max()
                        .unwrap();
                    // Deferred stores material data in half precision; compare at 8-bit readback.
                    assert!(
                        max_difference <= 8,
                        "{renderer_path}/{symbol}: max difference {max_difference}"
                    );
                } else {
                    *reference = Some(pixels);
                }
            }
        }
        let coverage = |pixels: &[u8]| {
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] != 0)
                .count()
        };
        if closed {
            assert!(
                coverage(toon.as_ref().unwrap()) > coverage(pbr.as_ref().unwrap()),
                "the outline must add silhouette coverage, not just darken the surface"
            );
        }
        assert_ne!(
            pbr, toon,
            "authored Toon must change the actual standard-material image"
        );
    }
    println!(
        "Authored styles: PBR and external Toon on unchanged standard data agree across all renderer paths."
    );
    verify_accounting(device, queue, output, view).await?;
    verify_settings(device, queue, output, view).await?;
    verify_exact_settings(device, queue, output, view).await?;
    verify_view_boundary(device, queue, output, view).await?;
    Ok(())
}

// Swapping the primary object must not move the outline ahead of another object's
// opaque draw, or move a transparent object's draw ahead of the outline.
async fn verify_view_boundary(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert("main.fr".into(),format!("{}\nsurface ordinary(sp: surf) -> material(standard) {{ compose {{ base(albedo: #38a9d4) }} }}\nsurface glass(sp: surf) -> material(standard) {{ properties {{ blend: SurfaceBlend.Translucent }}; compose {{ base(albedo: #ef7e4c80) }} }}",include_str!("../../../../examples/40) surface shaders/style_sample.fr")));
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
        let shape = PreviewGeometry::sphere();
        let factory = manifest
            .vertex_factories
            .iter()
            .find(|f| f.name == "preview_static")
            .unwrap();
        let mut reference: Option<Vec<u8>> = None;
        for order in [&[0, 1, 2][..], &[2, 1, 0][..], &[2][..]] {
            let mut objects = Vec::new();
            for &index in order {
                let geometry = MeshGeometry::prepare(
                    device,
                    queue,
                    factory,
                    shape.vertex_count,
                    &shape.streams(),
                    Some(&shape.indices),
                )
                .await?;
                let renderer = MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &manifest,
                    ["style_sample", "ordinary", "glass"][index],
                    geometry,
                    FORMAT,
                )
                .await?;
                let mut model = scene.model;
                model[12] += [-0.25, 0.25, 0.0][index];
                model[14] += [0.0, 0.25, 0.5][index];
                objects.push((renderer, model));
            }
            let (mut renderer, model) = objects.remove(0);
            for (object, model) in objects {
                renderer.add_object(object, model)?;
            }
            let mut inputs = scene;
            inputs.model = model;
            renderer.render(&inputs, view, depth.view())?;
            let pixels = readback(device, queue, output)?;
            assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] != 0));
            if order.len() == 1 {
                renderer.render(&inputs, view, depth.view())?;
                assert_eq!(
                    pixels,
                    readback(device, queue, output)?,
                    "{path}: transparent-only views must clear previous opaque contents"
                );
            } else if let Some(reference) = &reference {
                let differing = reference
                    .iter()
                    .zip(&pixels)
                    .filter(|(a, b)| a != b)
                    .count();
                assert!(
                    differing == 0,
                    "{path}: view completion must not depend on which material owns the scene ({differing} differing channels)"
                );
            } else {
                reference = Some(pixels);
            }
        }
    }
    println!(
        "Typed style boundaries: overlapping opaque, outlined, and transparent objects are independent of primary-object order in all renderers."
    );
    Ok(())
}

async fn verify_accounting(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    use fresco_example_engine::runtime::forward_plus::{PointLight, SceneLighting};
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.model = identity;
    scene.view = identity;
    scene.projection = identity;
    scene.camera_position = [0.0, 0.0, 2.0];
    scene.displacement.enabled = false;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        for (symbol, divisor) in [("Accounting", 1.0_f32), ("FinishAccounting", 2.0)] {
            let mut files = fresco_example_engine::source_files();
            files.insert(
                "main.fr".into(),
                include_str!("../../tests/fixtures/style-accounting.fr")
                    .replace("style: Accounting", &format!("style: {symbol}")),
            );
            files.insert(
                "fresco.config.json".into(),
                serde_json::json!({"renderer": path}).to_string(),
            );
            let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                .map_err(|e| format!("{e:?}"))?;
            let manifest = serde_json::from_str(&compiled.manifest)?;
            let mut renderer = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "accounting_probe",
                geometry(device, queue, &manifest).await?,
                FORMAT,
            )
            .await?;
            for (sun, points, direct) in [
                (true, false, 0.125_f32),
                (true, true, 0.25),
                (false, false, 0.0),
            ] {
                let lighting = SceneLighting {
                    unlit: false,
                    sky: [0.25; 3],
                    ground: [0.25; 3],
                    specular_fill: 0.0,
                    direction: [0.0, 0.0, 1.0],
                    directional_radiance: if sun { [3.0, 2.0, 1.0] } else { [0.0; 3] },
                    shadows: false,
                };
                let lights = if points {
                    vec![
                        PointLight {
                            position: [0.0, 0.0, 1.0],
                            radius: 10.0,
                            color: [0.25, 0.5, 0.75],
                            intensity: 4.0,
                        },
                        PointLight {
                            position: [0.0, 0.0, 1.0],
                            radius: 10.0,
                            color: [1.0; 3],
                            intensity: 0.0,
                        },
                        PointLight {
                            position: [100.0, 0.0, 1.0],
                            radius: 1.0,
                            color: [1.0; 3],
                            intensity: 4.0,
                        },
                    ]
                } else {
                    vec![]
                };
                renderer.set_scene_lighting(lighting, &lights)?;
                renderer.render(&scene, view, depth.view())?;
                let pixels = readback(device, queue, output)?;
                let expected = [direct / divisor, 0.125 / divisor, 0.125 / divisor, 1.0];
                let middle = SIZE.div_euclid(2);
                let center = usize::try_from((middle * SIZE + middle) * 4)?;
                for (actual, expected) in pixels[center..center + 4].iter().zip(expected) {
                    assert!(
                        (f32::from(*actual) - expected * 255.0).abs() <= 1.0,
                        "{path}/{symbol}/sun={sun}/points={points}: pixel {:?}, expected {:?}",
                        &pixels[center..center + 4],
                        expected
                    );
                }
            }
        }
    }
    println!(
        "Style accounting: complete direct contributions, indirect, emission and one finish call agree across renderers."
    );
    Ok(())
}

async fn verify_settings(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.view = identity;
    scene.projection = identity;
    scene.camera_position = [0.0, 0.0, 2.0];
    scene.displacement.enabled = false;
    scene.model = identity;
    scene.model[0] = 0.25;
    scene.model[5] = 0.25;
    scene.model[12] = -0.5;
    let mut second_model = scene.model;
    second_model[12] = 0.5;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files();
        files.insert(
            "main.fr".into(),
            include_str!("../../tests/fixtures/style-settings.fr").into(),
        );
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":path}).to_string(),
        );
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest = serde_json::from_str(&compiled.manifest)?;
        let mut first = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "first",
            geometry(device, queue, &manifest).await?,
            FORMAT,
        )
        .await?;
        let mut second = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "second",
            geometry(device, queue, &manifest).await?,
            FORMAT,
        )
        .await?;
        second.update_parameters(
            serde_json::json!({"style.tint":[0.0,1.0,0.0,1.0]})
                .as_object()
                .unwrap(),
        )?;
        second.render(&scene, view, depth.view())?;
        first.add_object(second, second_model)?;
        for gain in [0.25, 0.5] {
            first.update_parameters(serde_json::json!({"style.gain":gain,"style.tint":[1.0,0.0,0.0,1.0], "style.direction":[0.0,1.0,0.0]}).as_object().unwrap())?;
            let before = first.parameter_values();
            assert!(
                first
                    .update_parameters(
                        serde_json::json!({"style.gain":2.0,"style.tint":[0.0,0.0,1.0,1.0]})
                            .as_object()
                            .unwrap()
                    )
                    .is_err()
            );
            assert_eq!(first.parameter_values(), before);
            first.render(&scene, view, depth.view())?;
            let pixels = readback(device, queue, output)?;
            for (x, expected) in [
                (SIZE.div_euclid(4), [gain, 0.0, 0.0, 1.0]),
                (SIZE.div_euclid(4) * 3, [0.0, 0.75, 0.0, 1.0]),
            ] {
                let offset = usize::try_from((SIZE.div_euclid(2) * SIZE + x) * 4)?;
                for (actual, expected) in pixels[offset..offset + 4].iter().zip(expected) {
                    assert!(
                        (f64::from(*actual) - expected * 255.0).abs() <= 1.0,
                        "{path}: runtime material settings pixel {:?}",
                        &pixels[offset..offset + 4]
                    );
                }
            }
        }
    }
    println!(
        "Runtime style settings: independent material records and atomic edits agree across all renderers."
    );
    Ok(())
}

async fn verify_exact_settings(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.view = identity;
    scene.projection = identity;
    scene.model = identity;
    scene.camera_position = [0.0, 0.0, 2.0];
    scene.displacement.enabled = false;
    let source = r#"
style Exact for standard : StandardStyle {
    static param layers: u32 = u32(24)
    param seed: u32 = u32(4294967295)
    param offset: i32 = i32(-2147483648)
    param enabled: bool = true
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 { return vec3(0.0) }
    fn finish(surface: StandardSurface, context: ShadingContext, lighting: LightingResult) -> vec3 {
        if layers != u32(12) { return vec3(0.0, 0.0, 1.0) }
        if enabled && seed == u32(4294967295) && offset == i32(-2147483648) { return vec3(1.0, 0.0, 0.0) }
        if enabled == false && seed == u32(16777217) && offset == i32(-16777217) { return vec3(0.0, 1.0, 0.0) }
        return vec3(1.0, 1.0, 1.0)
    }
}
surface first(sp: surf) -> material(standard) {
    properties { style: Exact(layers: u32(12)) }
    compose { base(albedo: #fff) }
}
"#;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files();
        files.insert("main.fr".into(), source.into());
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":path}).to_string(),
        );
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest = serde_json::from_str(&compiled.manifest)?;
        let mut renderer = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "first",
            geometry(device, queue, &manifest).await?,
            FORMAT,
        )
        .await?;
        for (updates, expected) in [
            (serde_json::json!({}), [255u8, 0, 0, 255]),
            (
                serde_json::json!({"style.seed":16777217,"style.offset":-16777217,"style.enabled":false}),
                [0, 255, 0, 255],
            ),
        ] {
            renderer.update_parameters(updates.as_object().unwrap())?;
            let before = renderer.parameter_values();
            assert!(
                renderer
                    .update_parameters(serde_json::json!({"style.layers":16}).as_object().unwrap())
                    .is_err()
            );
            assert_eq!(renderer.parameter_values(), before);
            renderer.render(&scene, view, depth.view())?;
            let pixels = readback(device, queue, output)?;
            let offset = usize::try_from((SIZE.div_euclid(2) * SIZE + SIZE.div_euclid(2)) * 4)?;
            assert_eq!(
                &pixels[offset..offset + 4],
                &expected,
                "{path}: exact style settings transport and static specialization"
            );
        }
    }
    println!(
        "Exact style settings: full-width integer/boolean edits and static specialization agree across all renderers."
    );
    Ok(())
}

pub(super) async fn verify_operations(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.model = identity;
    scene.view = identity;
    scene.projection = identity;
    scene.displacement.enabled = false;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files();
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":path}).to_string(),
        );
        files.insert(
            "main.fr".into(),
            include_str!("../../tests/fixtures/style-operations.fr").into(),
        );
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest = serde_json::from_str(&compiled.manifest)?;
        let mesh = geometry(device, queue, &manifest).await?;
        assert!(mesh.with_draw_range(0..4).is_err());
        for empty in [false, true] {
            let mut first = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "item",
                mesh.with_draw_range(0..3)?,
                FORMAT,
            )
            .await?;
            let second = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "item",
                mesh.with_draw_range(if empty { 0..0 } else { 0..3 })?,
                FORMAT,
            )
            .await?;
            let mut translated = identity;
            translated[12] = 0.5;
            first.add_object(second, translated)?;
            first.render(&scene, view, depth.view())?;
            let pixels = readback(device, queue, output)?;
            let at = usize::try_from((SIZE.div_euclid(2) * SIZE + SIZE.div_euclid(2) + 4) * 4)?;
            let pixel = &pixels[at..at + 4];
            assert!(
                if empty {
                    pixel[1] > pixel[0]
                } else {
                    pixel[0] > pixel[1]
                },
                "{path}, empty={empty}: per-range operation ordering {pixel:?}"
            );
        }
    }
    println!(
        "Style operations: repeated material/mesh ranges, explicit resource captures, and per-object writer order agree across all renderers."
    );
    Ok(())
}
