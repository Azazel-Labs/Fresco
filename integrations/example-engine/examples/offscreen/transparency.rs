//! Ordinary and contributed translucent draws share one depth-sorted GPU queue.
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
    let mut reference: Option<Vec<Vec<u8>>> = None;
    for renderer in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(renderer);
        files.insert(
            "main.fr".into(),
            include_str!("../../tests/fixtures/transparent-queue.fr").into(),
        );
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
        let scene = sphere_scene(FrameInputs {
            time: 0.0,
            delta_time: 0.0,
            physical_size: [SIZE, SIZE],
        });
        let mut results = Vec::new();
        for shell_near in [false, true] {
            let mut expected = None;
            for shell_primary in [false, true] {
                let mut inputs = scene;
                inputs.displacement.enabled = false;
                let model = |near: bool| {
                    let mut model = scene.model;
                    let offset = if near { 0.3 } else { -0.3 };
                    for (axis, component) in [2, 6, 10].into_iter().enumerate() {
                        model[12 + axis] += scene.view[component] * offset;
                    }
                    model
                };
                let primary_name = if shell_primary { "shell" } else { "glass" };
                let other_name = if shell_primary { "glass" } else { "shell" };
                let mut primary = MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &manifest,
                    primary_name,
                    geometry.clone(),
                    FORMAT,
                )
                .await?;
                let other = MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &manifest,
                    other_name,
                    geometry.clone(),
                    FORMAT,
                )
                .await?;
                inputs.model = model(shell_primary == shell_near);
                primary.add_object(other, model(shell_primary != shell_near))?;
                primary.render(&inputs, view, depth.view())?;
                let pixels = readback(device, queue, output)?;
                let offset = usize::try_from((SIZE.div_euclid(2) * SIZE + SIZE.div_euclid(2)) * 4)?;
                let pixel = &pixels[offset..offset + 4];
                let (near, far) = if shell_near {
                    (pixel[0], pixel[1])
                } else {
                    (pixel[1], pixel[0])
                };
                assert!(
                    near > far.saturating_add(20),
                    "{renderer}: near draw must blend last: {pixel:?}"
                );
                if let Some(expected) = &expected {
                    assert_eq!(
                        &pixels, expected,
                        "{renderer}: primary ownership cannot change transparency"
                    );
                } else {
                    expected = Some(pixels);
                }
            }
            results.push(expected.unwrap());
        }
        if let Some(reference) = &reference {
            for (actual, expected) in results.iter().zip(reference) {
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                    "{renderer}: transparency differs across renderers"
                );
            }
        } else {
            reference = Some(results);
        }
    }
    println!(
        "Global transparency: ordinary and contributed draws interleave by depth, independent of primary ownership, in every renderer."
    );
    Ok(())
}
