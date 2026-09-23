//! Preset behavior and explicit level/gradient sampling through every renderer.
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
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.displacement.enabled = false;
    let mut reference = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut samples = Vec::new();
        for preset in [
            "nearest_repeat",
            "nearest_clamp",
            "linear_repeat",
            "linear_clamp",
        ] {
            let mut files = fresco_example_engine::source_files_for_recipe(path);
            files.insert(
                "main.fr".into(),
                include_str!("../../tests/fixtures/shading-sampler.fr")
                    .replace("nearest_repeat", preset),
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
            let mut renderer = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "item",
                geometry,
                FORMAT,
            )
            .await?;
            renderer.render(&scene, view, depth.view())?;
            let pixels = readback(device, queue, output)?;
            let offset = usize::try_from((SIZE.div_euclid(2) * SIZE + SIZE.div_euclid(2)) * 4)?;
            let pixel: [u8; 4] = pixels[offset..offset + 4].try_into()?;
            samples.push(pixel);
        }
        let [nearest_repeat, nearest_clamp, linear_repeat, linear_clamp] = samples.as_slice()
        else {
            unreachable!()
        };
        assert!(
            nearest_repeat[0] < 10 && nearest_repeat[1] > 100 && nearest_repeat[2] > 100,
            "{path}: nearest/repeat must choose the expected texels: {samples:?}"
        );
        assert!(
            nearest_clamp[0] > 100 && linear_clamp[0] > 100,
            "{path}: clamp must select the final texel: {samples:?}"
        );
        assert!(
            linear_repeat[0] < 10,
            "{path}: linear/repeat must wrap coordinates"
        );
        assert!(
            nearest_repeat[1].saturating_sub(linear_repeat[1]) > 15,
            "{path}: sample_grad must honor linear filtering: {samples:?}"
        );
        assert!(
            nearest_clamp[2].saturating_sub(linear_clamp[2]) > 15,
            "{path}: sample_level must honor linear filtering: {samples:?}"
        );
        if let Some(reference) = &reference {
            assert_eq!(
                &samples, reference,
                "{path}: renderer paths must preserve sampler behavior"
            );
        } else {
            reference = Some(samples);
        }
    }
    println!(
        "Shading samplers: repeat/clamp, nearest/linear, explicit LOD/gradients agree across all renderers."
    );
    let uv_source = include_str!("../../tests/fixtures/shading-sampler.fr")
        .replace("nearest_repeat", "linear_repeat")
        .replace("return vec3(a, b, c)", "return palette.sample_level(filtering, vec2(surface.uv.x * 23.0 + surface.uv.y * 19.0, 0.5), 0.0).rgb");
    let mut uv_reference: Option<Vec<u8>> = None;
    for (path, half_uv) in [
        ("forward", false),
        ("forward-plus", false),
        ("deferred", false),
        ("deferred", true),
    ] {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert("main.fr".into(), uv_source.clone());
        if half_uv {
            let renderer = files.get_mut("engine/config/renderer.fr").unwrap();
            assert!(renderer.contains("@image(surface_uv, rg32float)"));
            *renderer = renderer.replace(
                "@image(surface_uv, rg32float)",
                "@image(surface_uv, rg16float)",
            );
        }
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
        let mut renderer = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "item",
            geometry,
            FORMAT,
        )
        .await?;
        renderer.render(&scene, view, depth.view())?;
        let pixels = readback(device, queue, output)?;
        if let Some(reference) = &uv_reference {
            let difference = reference
                .iter()
                .zip(&pixels)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            if half_uv {
                assert!(
                    difference > 1,
                    "reducing lookup coordinate precision must fail the captured-image comparison"
                );
            } else {
                assert!(
                    difference <= 1,
                    "{path}: UV lookup into the captured compute image differs by {difference} output steps"
                );
            }
        } else {
            uv_reference = Some(pixels);
        }
    }
    println!(
        "Captured-image UV lookup agrees across renderers; half-precision storage mutation is detected."
    );
    Ok(())
}
