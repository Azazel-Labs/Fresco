//! Prepared geometry remains local to an object, material range, and view.
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
    let sample = include_str!("../../../../examples/40) surface shaders/style_sample.fr").replace("    param tint:", "    vertex { position: sp.position + sp.normal * sin(sp.time + sp.position.y * 3.0) * 0.08 }\n    param tint:");
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert("main.fr".into(), sample.clone());
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
        let mut scene = sphere_scene(FrameInputs {
            time: 0.7,
            delta_time: 0.0,
            physical_size: [SIZE, SIZE],
        });
        scene.displacement.enabled = false;
        for value in &mut scene.model[0..4] {
            *value *= 0.8;
        }
        for value in &mut scene.model[4..8] {
            *value *= 1.2;
        }
        let mut reference = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "style_sample",
            geometry.clone(),
            FORMAT,
        )
        .await?;
        let count = u32::try_from(shape.indices.len())?;
        let midpoint = count.div_euclid(6) * 3;
        let mut split = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "style_sample",
            geometry.with_draw_range(0..midpoint)?,
            FORMAT,
        )
        .await?;
        let second = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "style_sample",
            geometry.with_draw_range(midpoint..count)?,
            FORMAT,
        )
        .await?;
        split.add_object(second, scene.model)?;
        let empty = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "style_sample",
            geometry.with_draw_range(0..0)?,
            FORMAT,
        )
        .await?;
        split.add_object(empty, scene.model)?;
        for offset in [0.0, 0.2] {
            scene.view[12] = offset;
            reference.render(&scene, view, depth.view())?;
            let full = readback(device, queue, output)?;
            split.render(&scene, view, depth.view())?;
            assert_eq!(
                full,
                readback(device, queue, output)?,
                "{path}: range-local preparation must preserve deformation, transforms and view identity"
            );
        }
        let mut ordered: Option<Vec<u8>> = None;
        for reverse in [false, true] {
            let mut models = [scene.model; 2];
            for (index, model) in models.iter_mut().enumerate() {
                for value in &mut model[..12] {
                    *value *= 0.45;
                }
                model[12] += if index == 0 { -0.45 } else { 0.45 };
            }
            if reverse {
                models.reverse();
            }
            let mut first = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "style_sample",
                geometry.clone(),
                FORMAT,
            )
            .await?;
            let second = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "style_sample",
                geometry.clone(),
                FORMAT,
            )
            .await?;
            first.add_object(second, models[1])?;
            let mut inputs = scene;
            inputs.model = models[0];
            first.render(&inputs, view, depth.view())?;
            let pixels = readback(device, queue, output)?;
            if let Some(ordered) = &ordered {
                assert_eq!(
                    ordered, &pixels,
                    "{path}: shared mesh/material objects must prepare with independent transforms"
                );
            } else {
                ordered = Some(pixels);
            }
        }
        let mut invalid = manifest.clone();
        invalid.surfaces[0]
            .mesh_passes
            .iter_mut()
            .find_map(|p| p.preparation.as_mut())
            .unwrap()
            .vertex_stride += 4;
        let rejected = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &invalid,
            "style_sample",
            geometry.clone(),
            FORMAT,
        )
        .await;
        assert!(
            rejected.is_err(),
            "{path}: reflected preparation stride must match shader storage"
        );
        reference.update_parameters(
            serde_json::json!({"style.outline_width":0.0})
                .as_object()
                .unwrap(),
        )?;
        reference.render(&scene, view, depth.view())?;
        let narrow = readback(device, queue, output)?;
        reference.update_parameters(
            serde_json::json!({"style.outline_width":5.0,"style.outline_color":[0.9,0.1,0.2,1.0]})
                .as_object()
                .unwrap(),
        )?;
        reference.render(&scene, view, depth.view())?;
        assert_ne!(
            narrow,
            readback(device, queue, output)?,
            "{path}: outline settings must remain live"
        );
    }
    println!(
        "Prepared geometry: shared mesh ranges, empty ranges, deformation, nonuniform transforms, two views, and live outline settings agree in all renderers."
    );
    Ok(())
}
