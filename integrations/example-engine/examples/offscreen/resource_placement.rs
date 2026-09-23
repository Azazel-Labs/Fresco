//! Local pixel equivalence for explicit and resource-driven placement.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{
        FrameInputs,
        preview::{PreviewGeometry, sphere_scene},
    },
    runtime::{depth::DepthTarget, mesh::MeshRenderer, mesh_geometry::MeshGeometry},
};
use std::error::Error;

#[path = "../../tests/fixtures/resource_placement.rs"]
mod fixture;

fn model(x: f32) -> [f32; 16] {
    [
        0.42, 0.0, 0.0, 0.0, 0.0, 0.42, 0.0, 0.0, 0.0, 0.0, 0.42, 0.0, x, 0.0, 0.0, 1.0,
    ]
}

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let shape = PreviewGeometry::sphere();
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut reference = None;
        for inferred in [false, true] {
            let mut files = fresco_example_engine::source_files_for_recipe(renderer);
            files.insert("main.fr".into(), fixture::source(inferred));
            let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                .map_err(|errors| format!("{errors:?}"))?;
            let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
            let factory = manifest
                .vertex_factories
                .iter()
                .find(|factory| factory.name == "preview_static")
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
                "first",
                geometry.clone(),
                FORMAT,
            )
            .await?;
            let other = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "second",
                geometry.clone(),
                FORMAT,
            )
            .await?;
            draw.add_object(other, model(0.6))?;
            let count = u32::try_from(shape.indices.len())?;
            let split = count.div_euclid(6) * 3;
            // Repeated material/mesh identity with a distinct selected range.
            let repeated = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "first",
                geometry.with_draw_range(0..split)?,
                FORMAT,
            )
            .await?;
            let mut third = model(0.0);
            third[13] = 0.7;
            draw.add_object(repeated, third)?;
            let mut scene = sphere_scene(FrameInputs {
                time: 0.0,
                delta_time: 0.0,
                physical_size: [SIZE, SIZE],
            });
            scene.displacement.enabled = false;
            scene.model = model(-0.6);
            let mut images = Vec::new();
            for time in [0.0, 1.0] {
                scene.frame.time = time;
                draw.render(&scene, view, depth.view())?;
                images.push(readback(device, queue, output)?);
            }
            assert_ne!(
                images[0], images[1],
                "{renderer}: returned compute data must affect pixels"
            );
            assert!(
                images[0]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|pixel| pixel[3] > 0)
                    .count()
                    > 30,
                "{renderer}: fixture must render visible geometry"
            );
            if let Some(explicit) = &reference {
                assert_eq!(
                    explicit, &images,
                    "{renderer}: inferred placement changed pixels"
                );
            } else {
                reference = Some(images);
            }
        }
        println!(
            "{renderer}: explicit/inferred pixels match with multiple contributors, repeated ranges, and consumed early compute"
        );
    }
    Ok(())
}
