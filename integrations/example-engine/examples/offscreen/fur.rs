//! Full shell-fur acceptance, including compute captures and renderer agreement.
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
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let shape = PreviewGeometry::sphere();
    let mut reference: Option<Vec<Vec<u8>>> = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert(
            "main.fr".into(),
            include_str!("../../../../examples/40) surface shaders/style_sample_fur.fr").into(),
        );
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
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
            "chestnut_fur",
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
        let mut images = Vec::new();
        for (time, opacity, seed, tint) in [
            (0.0, 0.7, 7, [0.8, 0.3, 0.1, 1.0]),
            (2.0, 0.7, 7, [0.8, 0.3, 0.1, 1.0]),
            (0.0, 0.0, 7, [0.8, 0.3, 0.1, 1.0]),
            (0.0, 0.0, 37, [0.8, 0.3, 0.1, 1.0]),
            (0.0, 0.7, 7, [0.1, 0.3, 0.8, 1.0]),
            (0.0, 0.7, 7, [0.8, 0.3, 0.1, 1.0]),
        ] {
            scene.frame.time = time;
            draw.update_parameters(serde_json::json!({"style.fur_length": 0.15, "style.shell_opacity": opacity, "style.seed": seed, "style.fur_tint": tint, "style.wind": [0.8, 0.0, 0.0]}).as_object().unwrap())?;
            draw.render(&scene, view, depth.view())?;
            images.push(readback(device, queue, output)?);
        }
        let changed = |a: usize, b: usize| {
            images[a]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(images[b].as_chunks::<4>().0)
                .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
                .count()
        };
        assert!(
            changed(0, 1) > 32,
            "{path}: frame.time must animate generated vertices"
        );
        assert!(
            changed(0, 2) > 32,
            "{path}: transparent shells must contribute visible color"
        );
        assert!(
            changed(2, 3) > 32,
            "{path}: computed density must affect opaque roots without shells"
        );
        assert!(
            changed(0, 4) > 32,
            "{path}: projected dynamic tint must change shell color"
        );
        assert_eq!(
            images[0], images[5],
            "{path}: later frames must restore independent outputs"
        );
        if let Some(reference) = &reference {
            for (index, (expected, actual)) in reference.iter().zip(&images).enumerate() {
                assert!(
                    expected
                        .iter()
                        .zip(actual)
                        .all(|(a, b)| a.abs_diff(*b) <= 8),
                    "{path}: fur mutation {index} differs across renderers"
                );
            }
        } else {
            reference = Some(images);
        }
        println!(
            "{path}: full MeadowFur time, shell, density, tint, and frame-isolation mutations passed"
        );
    }
    Ok(())
}
