//! A raised occluder must darken a receiver without removing ambient fill.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{
        depth::DepthTarget, mesh::MeshRenderer, mesh_geometry::MeshGeometry, vertices::VertexValues,
    },
};
use std::{collections::BTreeMap, error::Error};

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    verify_case(device, queue, output, view, false).await
}

pub(super) async fn verify_service(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    verify_case(device, queue, output, view, true).await
}

async fn verify_case(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
    contributed: bool,
) -> Result<(), Box<dyn Error>> {
    let positions = [
        -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0, -0.25, -0.1, 0.65, 0.25,
        -0.1, 0.65, 0.25, 0.4, 0.65, -0.25, 0.4, 0.65,
    ];
    let normals = [0.0, 0.0, 1.0].repeat(8);
    let tangents = [1.0, 0.0, 0.0].repeat(8);
    let uv = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0].repeat(2);
    let streams = BTreeMap::from([
        ("position".into(), VertexValues::F32(&positions)),
        ("normal".into(), VertexValues::F32(&normals)),
        ("tangent".into(), VertexValues::F32(&tangents)),
        ("uv".into(), VertexValues::F32(&uv)),
        ("uv2".into(), VertexValues::F32(&uv)),
    ]);
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let mut reference: Option<Vec<u8>> = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut images = Vec::new();
        for shadowed in [true, false] {
            let mut files = if path == "deferred" {
                fresco_example_engine::source_files_for_deferred()
            } else {
                fresco_example_engine::source_files_for_renderer(path == "forward-plus")
            };
            files.insert("main.fr".into(), "surface probe(sp: surf) -> material(standard) { compose { base(albedo: rgba(0.7, 0.6, 0.5, 1.0), roughness: 0.8) } }".into());
            if contributed {
                files.insert("main.fr".into(), format!("{}\nsurface blocker(sp: surf) -> material(unlit) {{ compose {{ base(albedo: rgba(0.0, 0.0, 0.0, 1.0)) }} }}", include_str!("../../tests/fixtures/lighting-service.fr")));
            }
            let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                .map_err(|e| format!("{e:?}"))?;
            let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
            let factory = manifest
                .vertex_factories
                .iter()
                .find(|f| f.name == "preview_static")
                .unwrap();
            if path == "forward" && shadowed {
                let mut incompatible = manifest.clone();
                let binding = incompatible
                    .vertex_factories
                    .iter_mut()
                    .find(|f| f.name == "preview_static")
                    .unwrap()
                    .bindings
                    .iter_mut()
                    .find(|b| b.name == "shadow_map")
                    .unwrap();
                binding.signature = Some("texture_2d<f32>".into());
                let geometry = MeshGeometry::prepare(
                    device,
                    queue,
                    factory,
                    8,
                    &streams,
                    Some(&[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]),
                )
                .await?;
                let failure = MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &incompatible,
                    if contributed { "shell" } else { "probe" },
                    geometry,
                    FORMAT,
                )
                .await;
                assert!(
                    matches!(failure, Err(ref error) if error.to_string().contains("sampled texture type is incompatible with image format"))
                );
            }
            let geometry = MeshGeometry::prepare(
                device,
                queue,
                factory,
                8,
                &streams,
                Some(&[0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]),
            )
            .await?;
            let mut renderer = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                if contributed { "shell" } else { "probe" },
                if contributed {
                    geometry.with_draw_range(0..6)?
                } else {
                    geometry.clone()
                },
                FORMAT,
            )
            .await?;
            if contributed {
                let blocker = MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &manifest,
                    "blocker",
                    geometry.with_draw_range(6..12)?,
                    FORMAT,
                )
                .await?;
                renderer.add_object(blocker, scene.model)?;
            }
            assert!(renderer.supports_lighting_environment());
            let mut lighting =
                fresco_example_engine::runtime::forward_plus::LightingEnvironment::Preview
                    .lighting();
            lighting.shadows = shadowed;
            renderer.set_scene_lighting(lighting, &[])?;
            renderer.render(&scene, view, depth.view())?;
            images.push(readback(device, queue, output)?);
        }
        let shaded: Vec<_> = images[0]
            .as_chunks::<4>()
            .0
            .iter()
            .zip(images[1].as_chunks::<4>().0)
            .filter(|(a, b)| b[0].saturating_sub(a[0]) > 20)
            .collect();
        assert!(
            shaded.len() > 10,
            "{path}: occluder must cast a visible shadow, got {} pixels",
            shaded.len()
        );
        assert!(
            shaded
                .iter()
                .all(|(a, _)| a[0] > 15 && a[1] > 15 && a[2] > 15),
            "{path}: shadows must retain ambient fill"
        );
        if let Some(reference) = &reference {
            assert!(
                reference
                    .iter()
                    .zip(&images[0])
                    .all(|(a, b)| a.abs_diff(*b) <= 8),
                "{path}: preview must agree across renderers"
            );
        } else {
            reference = Some(images.remove(0));
        }
    }
    println!(
        "Preview shadows (contributed={contributed}): occlusion, ambient fill, and all three renderer paths agree."
    );
    Ok(())
}
