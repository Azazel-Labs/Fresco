//! End-to-end owned compute consumption and invocation isolation in each renderer.
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
    let sample = format!("{}\n{}", include_str!("../../tests/fixtures/owned-compute.fr").replace(
        "let dimensions = FieldDimensions(source: field)",
        "let dimensions = FieldDimensions(source: field)\n Show(geometry: self, scene: frame, values: copied, field: field, target: target.color)"
    ), include_str!("../../tests/fixtures/owned-compute-display.fr"));
    verify_sample(
        device,
        queue,
        output,
        view,
        &sample,
        &["forward", "forward-plus", "deferred"],
    )
    .await
}

pub(super) async fn verify_shading(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let sample = include_str!("../../tests/fixtures/owned-compute.fr")
        .replace("f32(id.x))", "f32(id.x) + f32(size))")
        .replace("style Processing for standard : StandardStyle {", r#"
style Processing for standard : StandardStyle {
    requires SurfaceUV, DrawShadingResources
    shading_input positions: buffer<vec4, read> scope draw
    shading_input density: texture2d<rg32float, read> scope draw
    shading_input filtering: sampler scope draw
    fn finish(surface: StandardSurface, context: ShadingContext, lighting: LightingResult) -> vec3 {
        return vec3(0.5) + positions[0u].xyz * 0.05 + vec3(density.sample_level(filtering, vec2(0.0), 0.0).r * 0.002)
    }
"#)
        .replace("let dimensions = FieldDimensions(source: field)",
            "let dimensions = FieldDimensions(source: field)\n bind shading.positions = copied\n bind shading.density = field\n bind shading.filtering = nearest_repeat");
    verify_sample(
        device,
        queue,
        output,
        view,
        &sample,
        &["forward", "forward-plus", "deferred"],
    )
    .await
}

async fn verify_sample(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
    sample: &str,
    paths: &[&str],
) -> Result<(), Box<dyn Error>> {
    let shape = PreviewGeometry::sphere();
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in paths {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert("main.fr".into(), sample.into());
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
            "item",
            geometry.clone(),
            FORMAT,
        )
        .await?;
        reference.render(&scene, view, depth.view())?;
        let first = readback(device, queue, output)?;
        reference.update_parameters(serde_json::json!({"style.gain":0.2}).as_object().unwrap())?;
        reference.render(&scene, view, depth.view())?;
        let changed = readback(device, queue, output)?;
        assert_ne!(
            first, changed,
            "{path}: compute results must reach the draw and settings must remain live"
        );
        reference.update_parameters(serde_json::json!({"style.gain":1.25}).as_object().unwrap())?;
        reference.render(&scene, view, depth.view())?;
        assert_eq!(
            first,
            readback(device, queue, output)?,
            "{path}: fresh frame outputs must be deterministic"
        );
        scene.view[12] += 0.3;
        reference.render(&scene, view, depth.view())?;
        assert_ne!(
            first,
            readback(device, queue, output)?,
            "{path}: compute inputs must follow the current view"
        );
        scene.view[12] -= 0.3;
        reference.update_parameters(serde_json::json!({"style.size":0}).as_object().unwrap())?;
        reference.render(&scene, view, depth.view())?;
        assert_ne!(
            first,
            readback(device, queue, output)?,
            "{path}: zero image extent must reach the draw as logical zero"
        );
        reference.update_parameters(serde_json::json!({"style.size":65}).as_object().unwrap())?;
        reference.render(&scene, view, depth.view())?;
        reference.set_compute_budget(1);
        assert!(
            reference.render(&scene, view, depth.view()).is_err(),
            "{path}: allocation budgets must reject before submission"
        );
        assert_eq!(
            first,
            readback(device, queue, output)?,
            "{path}: rejected work must not alter presentation"
        );
        reference.set_compute_budget(device.limits().max_buffer_size);
        for gain in [0.4, 0.7, 1.25] {
            reference
                .update_parameters(serde_json::json!({"style.gain":gain}).as_object().unwrap())?;
            reference.render(&scene, view, depth.view())?;
        }
        assert_eq!(
            first,
            readback(device, queue, output)?,
            "{path}: queued frames must retain their captured inputs and outputs"
        );
        let mut other_view = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "item",
            geometry.clone(),
            FORMAT,
        )
        .await?;
        let other_output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("independent compute capture view"),
            size: output.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let other_target = other_output.create_view(&Default::default());
        let other_depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
        scene.view[12] += 0.3;
        other_view.update_parameters(serde_json::json!({"style.gain":0.4}).as_object().unwrap())?;
        other_view.render(&scene, &other_target, other_depth.view())?;
        let other_expected = readback(device, queue, &other_output)?;
        assert_ne!(
            first, other_expected,
            "{path}: independent view fixture must differ"
        );
        // Queue both views before reading either, in both orders. They select the
        // same material and producer symbols but must own separate compute outputs.
        for reverse in [false, true] {
            if reverse {
                other_view.render(&scene, &other_target, other_depth.view())?;
            }
            scene.view[12] -= 0.3;
            reference.render(&scene, view, depth.view())?;
            scene.view[12] += 0.3;
            if !reverse {
                other_view.render(&scene, &other_target, other_depth.view())?;
            }
            assert_eq!(
                first,
                readback(device, queue, output)?,
                "{path}: another view must not overwrite this view's compute captures"
            );
            assert_eq!(
                other_expected,
                readback(device, queue, &other_output)?,
                "{path}: captured resources must remain independent of view submission order"
            );
        }
        scene.view[12] -= 0.3;
        let mut empty = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "item",
            geometry.with_draw_range(0..0)?,
            FORMAT,
        )
        .await?;
        empty.render(&scene, view, depth.view())?;
        let background = readback(device, queue, output)?;
        assert_ne!(
            first, background,
            "{path}: an empty range must not invoke another object's geometry"
        );
        let mut range_reference = None;
        let count = u32::try_from(shape.indices.len())?;
        let midpoint = count.div_euclid(6) * 3;
        for reverse in [false, true] {
            let mut ranges = [0..midpoint, midpoint..count];
            if reverse {
                ranges.reverse();
            }
            let mut first_range = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "item",
                geometry.with_draw_range(ranges[0].clone())?,
                FORMAT,
            )
            .await?;
            let second_range = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "item",
                geometry.with_draw_range(ranges[1].clone())?,
                FORMAT,
            )
            .await?;
            first_range.add_object(second_range, scene.model)?;
            first_range.render(&scene, view, depth.view())?;
            let pixels = readback(device, queue, output)?;
            if let Some(expected) = &range_reference {
                assert_eq!(
                    expected, &pixels,
                    "{path}: compute resources must follow their material range after reordering"
                );
            } else {
                range_reference = Some(pixels);
            }
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
                "item",
                geometry.clone(),
                FORMAT,
            )
            .await?;
            let second = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &compiled.wgsl,
                &manifest,
                "item",
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
                    "{path}: compute outputs must remain local to each object despite a shared mesh/material"
                );
            } else {
                ordered = Some(pixels);
            }
        }
    }
    println!(
        "Owned compute scene: buffer/image reads, settings, views, draw ranges, and object isolation passed for {paths:?}."
    );
    Ok(())
}
