//! Custom schemas must execute their own shading response in every renderer.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{
        FrameInputs,
        preview::{PreviewGeometry, sphere_scene},
    },
    runtime::{depth::DepthTarget, mesh::MeshRenderer, mesh_geometry::MeshGeometry},
};
use std::{collections::BTreeSet, error::Error};

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let shape = PreviewGeometry::sphere();
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let mut reference: Option<Vec<u8>> = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = if path == "deferred" {
            fresco_example_engine::source_files_for_deferred()
        } else {
            fresco_example_engine::source_files_for_renderer(path == "forward-plus")
        };
        files.insert(
            "main.fr".into(),
            include_str!("../../tests/fixtures/custom-response.fr").into(),
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
            "style_sample",
            geometry,
            FORMAT,
        )
        .await?;
        renderer.render(&scene, view, depth.view())?;
        let pixels = readback(device, queue, output)?;
        let colors: BTreeSet<_> = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] != 0)
            .copied()
            .collect();
        assert_eq!(
            colors.len(),
            3,
            "{path}: toon response must produce three flat bands"
        );
        if let Some(reference) = &reference {
            assert!(
                reference
                    .iter()
                    .zip(&pixels)
                    .all(|(a, b)| a.abs_diff(*b) <= 1),
                "{path}: custom response must survive renderer switches"
            );
        } else {
            reference = Some(pixels.clone());
        }
        renderer.update_parameters(&serde_json::from_value(
            serde_json::json!({"midtone_level": 0.3}),
        )?)?;
        renderer.render(&scene, view, depth.view())?;
        assert_ne!(
            pixels,
            readback(device, queue, output)?,
            "{path}: toon parameter must update the actual shader"
        );
    }
    println!("Custom toon: three discrete bands, live controls, and all renderer paths agree.");
    Ok(())
}
