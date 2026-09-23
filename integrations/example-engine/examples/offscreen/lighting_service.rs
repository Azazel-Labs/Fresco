//! Contributed draws consume the same mutable lighting inputs in every renderer.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{
        FrameInputs,
        preview::{PreviewGeometry, sphere_scene},
    },
    runtime::{
        depth::DepthTarget,
        forward_plus::{LightingEnvironment, PointLight},
        mesh::MeshRenderer,
        mesh_geometry::MeshGeometry,
    },
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
    let mut reference: Option<Vec<Vec<u8>>> = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert(
            "main.fr".into(),
            include_str!("../../tests/fixtures/lighting-service.fr").into(),
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
        let mut renderer = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "shell",
            geometry,
            FORMAT,
        )
        .await?;
        let mut images = Vec::new();
        for mutation in 0..5 {
            let mut lighting = LightingEnvironment::Preview.lighting();
            lighting.sky = [0.0; 3];
            lighting.ground = [0.0; 3];
            lighting.specular_fill = 0.0;
            lighting.directional_radiance = [0.0; 3];
            lighting.shadows = false;
            let mut points = Vec::new();
            match mutation {
                0 => {}
                1 => {
                    lighting.sky = [0.5, 0.1, 0.0];
                    lighting.ground = lighting.sky;
                }
                2 => {
                    lighting.directional_radiance = [0.0, 2.0, 0.0];
                }
                3 | 4 => {
                    points.push(PointLight {
                        position: [0.0, 1.0, 2.0],
                        radius: if mutation == 3 { 5.0 } else { 0.01 },
                        color: [0.0, 0.0, 1.0],
                        intensity: 5.0,
                    });
                }
                _ => unreachable!(),
            }
            renderer.set_scene_lighting(lighting, &points)?;
            renderer.render(&scene, view, depth.view())?;
            images.push(readback(device, queue, output)?);
        }
        for (index, channel) in [(1, 0), (2, 1), (3, 2)] {
            let changed = images[0]
                .as_chunks::<4>()
                .0
                .iter()
                .zip(images[index].as_chunks::<4>().0)
                .filter(|(before, after)| after[channel] > before[channel].saturating_add(8))
                .count();
            assert!(
                changed > 100,
                "{path}: lighting mutation {index} affected only {changed} pixels"
            );
        }
        assert_eq!(
            images[0], images[4],
            "{path}: out-of-volume point light must yield no sample"
        );
        if let Some(reference) = &reference {
            for (index, (expected, actual)) in reference.iter().zip(&images).enumerate() {
                assert!(
                    expected
                        .iter()
                        .zip(actual)
                        .all(|(a, b)| a.abs_diff(*b) <= 3),
                    "{path}: lighting mutation {index} differs across renderers"
                );
            }
        } else {
            reference = Some(images);
        }
        println!(
            "{path}: contributed lighting responds to indirect, sun, point-light, and radius mutations"
        );
    }
    super::shadows::verify_service(device, queue, output, view).await
}
