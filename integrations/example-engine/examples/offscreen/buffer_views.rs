//! Read back authored fullscreen inspectors against known material data.
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
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = if path == "deferred" {
            fresco_example_engine::source_files_for_deferred()
        } else {
            fresco_example_engine::source_files_for_renderer(path == "forward-plus")
        };
        files.insert(
            "main.fr".into(),
            "surface probe(sp: surf) -> material(standard) { compose { base(albedo: #f00, roughness: 0.25, metallic: 0.75, occlusion: 0.5, emissive: vec3(0.2)) } }".into(),
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
            "probe",
            geometry,
            FORMAT,
        )
        .await?;
        renderer.render(&scene, view, depth.view())?;
        let pixels = readback(device, queue, output)?;
        let ids: Vec<_> = renderer.buffer_views().iter().map(|view| view.id).collect();
        assert_eq!(
            ids.len(),
            if path == "forward" {
                0
            } else if path == "deferred" {
                12
            } else {
                4
            }
        );
        assert!(renderer.set_buffer_view("missing").is_err());
        renderer.render(&scene, view, depth.view())?;
        assert_eq!(
            pixels,
            readback(device, queue, output)?,
            "invalid mode must preserve output"
        );
        if path == "forward" {
            continue;
        }
        for id in ids {
            renderer.set_buffer_view(id)?;
            renderer.render(&scene, view, depth.view())?;
            let displayed = readback(device, queue, output)?;
            let midpoint = SIZE.div_euclid(2);
            let center = ((midpoint * SIZE + midpoint) * 4) as usize;
            let rgb = &displayed[center..center + 3];
            let expected = match id {
                "albedo" => Some([255, 0, 0]),
                "roughness" => Some([64, 64, 64]),
                "metallic" => Some([191, 191, 191]),
                "occlusion" => Some([128, 128, 128]),
                "emissive" => Some([43, 43, 43]),
                _ => None,
            };
            if let Some(expected) = expected {
                assert!(
                    rgb.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                    "{path}/{id}: {rgb:?}"
                );
            }
            if id == "all" {
                // Each cell samples the original full image, including known channels.
                for (column, row, expected) in [
                    (0, 1, [255, 0, 0]),
                    (2, 1, [64, 64, 64]),
                    (3, 1, [191, 191, 191]),
                    (1, 2, [128, 128, 128]),
                ] {
                    let x = ((column * 2 + 1) * SIZE).div_euclid(8);
                    let y = ((row * 2 + 1) * SIZE).div_euclid(6);
                    let offset = ((y * SIZE + x) * 4) as usize;
                    assert!(
                        displayed[offset..offset + 3]
                            .iter()
                            .zip(expected)
                            .all(|(a, b)| a.abs_diff(b) <= 1),
                        "overview cell {column},{row}"
                    );
                }
            }
            if id == "shaded" {
                assert_eq!(displayed, pixels);
            } else {
                assert_ne!(displayed, pixels, "{path}/{id} must display another buffer");
            }
        }
        renderer.set_point_lights(&[])?;
        renderer.set_buffer_view("tiles")?;
        renderer.render(&scene, view, depth.view())?;
        assert!(
            readback(device, queue, output)?
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0, 0, 255, 255])
        );
        renderer.set_buffer_view("shaded")?;
        renderer.render(&scene, view, depth.view())?;
        assert_eq!(
            pixels,
            readback(device, queue, output)?,
            "Preview sun lighting must survive switching views"
        );
    }
    println!(
        "Buffer inspectors: actual GBuffer channels, live tile counts, invalid selection isolation, and shaded restoration passed."
    );
    Ok(())
}
