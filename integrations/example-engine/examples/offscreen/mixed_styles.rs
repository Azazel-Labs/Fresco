//! Multiple styles, shared material/mesh ranges, and independent view submissions.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{
        FrameInputs,
        camera::OrbitCamera,
        preview::{PreviewGeometry, PreviewShape},
    },
    runtime::{
        depth::DepthTarget, forward_plus::LightingEnvironment, mesh::MeshRenderer,
        mesh_geometry::MeshGeometry,
    },
};
use std::error::Error;

fn model(x: f32, y: f32, z: f32, scale: f32) -> [f32; 16] {
    [
        scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, 0.0, 0.0, 0.0, scale, 0.0, x, y, z, 1.0,
    ]
}

struct Setup<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    wgsl: &'a str,
    manifest: &'a fresco_artifact::ManifestRoot,
}
impl Setup<'_> {
    async fn draw(
        &self,
        material: &str,
        geometry: MeshGeometry,
    ) -> Result<MeshRenderer, Box<dyn Error>> {
        let mut draw = MeshRenderer::prepare(
            self.device.clone(),
            self.queue.clone(),
            self.wgsl,
            self.manifest,
            material,
            geometry,
            FORMAT,
        )
        .await
        .map_err(|error| format!("preparing mixed-scene material `{material}`: {error}"))?;
        if matches!(material, "chestnut_fur" | "pale_fur") {
            draw.update_parameters(
                serde_json::json!({"style.fur_length":0.07,"style.shell_opacity":0.7})
                    .as_object()
                    .unwrap(),
            )?;
        } else if matches!(material, "style_sample" | "blue_toon") {
            draw.update_parameters(
                serde_json::json!({"style.outline_width":0.8})
                    .as_object()
                    .unwrap(),
            )?;
        }
        let mut lighting = LightingEnvironment::Preview.lighting();
        lighting.shadows = false;
        draw.set_scene_lighting(lighting, &[])?;
        Ok(draw)
    }
}

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let source = format!(
        "{}\n{}\n{}",
        include_str!("../../../../examples/40) surface shaders/style_sample.fr"),
        include_str!("../../../../examples/40) surface shaders/style_sample_fur.fr"),
        include_str!("../../tests/fixtures/mixed-style-materials.fr")
    );
    let shape = PreviewGeometry::sphere();
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let other_depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let other_output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("independent mixed-style view"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let other_view = other_output.create_view(&Default::default());
    let left = model(-0.8, 0.45, 0.0, 0.20);
    let right = model(0.8, 0.45, 0.0, 0.20);
    let middle = model(0.0, -0.45, 0.0, 0.24);
    let mut reference: Option<(Vec<u8>, Vec<u8>)> = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert("main.fr".into(), source.clone());
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
        let setup = Setup {
            device,
            queue,
            wgsl: &compiled.wgsl,
            manifest: &manifest,
        };
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
        let count = u32::try_from(shape.indices.len())?;
        // Round down to a complete triangle at the halfway point.
        let split = count.div_euclid(6) * 3;
        let frame = FrameInputs {
            time: 0.0,
            delta_time: 0.0,
            physical_size: [SIZE, SIZE],
        };
        let mut scene = OrbitCamera::default().scene(PreviewShape::Sphere, frame);
        scene.displacement.enabled = false;
        let mut isolated = setup.draw("chestnut_fur", geometry.clone()).await?;
        let mut isolated_images = Vec::new();
        for transform in [left, right] {
            scene.model = transform;
            isolated.render(&scene, view, depth.view())?;
            isolated_images.push(readback(device, queue, output)?);
        }
        scene.model = left;
        let mut draw = setup.draw("chestnut_fur", geometry.clone()).await?;
        // Interleave opaque, fur, Toon, and glass submissions; graph phases own scheduling.
        for (material, range, transform) in [
            ("plain", 0..count, model(0.8, -0.45, 0.0, 0.24)),
            ("pale_fur", 0..count, model(0.0, 0.45, 0.0, 0.20)),
            ("blue_toon", 0..split, middle),
            ("glass", 0..count, model(0.0, 0.45, 0.15, 0.19)),
            ("chestnut_fur", 0..count, right),
            ("style_sample", 0..count, model(-0.8, -0.45, 0.0, 0.24)),
            ("plain", split..count, middle),
        ] {
            draw.add_object(
                setup
                    .draw(material, geometry.with_draw_range(range)?)
                    .await?,
                transform,
            )?;
        }
        draw.render(&scene, view, depth.view())?;
        let original = readback(device, queue, output)?;
        for (index, isolated) in isolated_images.iter().enumerate() {
            let covered: Vec<_> = isolated
                .as_chunks::<4>()
                .0
                .iter()
                .zip(original.as_chunks::<4>().0)
                .filter(|(pixel, _)| pixel[3] > 8)
                .collect();
            assert!(
                covered.len() > 30,
                "{path}: shared fur object {index} has insufficient coverage"
            );
            assert!(
                covered.iter().all(|(expected, actual)| expected
                    .iter()
                    .zip(*actual)
                    .all(|(a, b)| a.abs_diff(*b) <= 3)),
                "{path}: shared material geometry/density aliases between objects"
            );
        }
        let mut camera = OrbitCamera::default();
        camera.drag([14.0, 8.0])?;
        let mut second = camera.scene(PreviewShape::Sphere, FrameInputs { time: 1.7, ..frame });
        second.model = left;
        second.displacement.enabled = false;
        // Submit both views before readback; the first view must survive subsequent bindings.
        draw.render(&second, &other_view, other_depth.view())?;
        draw.render(&scene, view, depth.view())?;
        let changed_view = readback(device, queue, &other_output)?;
        assert_eq!(
            original,
            readback(device, queue, output)?,
            "{path}: later view contaminated original view"
        );
        assert_ne!(
            original, changed_view,
            "{path}: view/time changes must affect the mixed scene"
        );
        if let Some((first, second)) = &reference {
            for (expected, actual) in [(first, &original), (second, &changed_view)] {
                assert!(
                    expected
                        .iter()
                        .zip(actual)
                        .all(|(a, b)| a.abs_diff(*b) <= 10),
                    "{path}: mixed scene differs across renderers"
                );
            }
        } else {
            reference = Some((original, changed_view));
        }
        println!(
            "{path}: Toon, fur, opaque/glass, shared materials, mesh ranges, and independent views passed"
        );
    }
    Ok(())
}
