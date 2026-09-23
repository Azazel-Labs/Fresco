//! Real scene geometry must participate in depth, materials, lights, and shadows.
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

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let mut camera = OrbitCamera::default();
    camera.drag([45.0, 40.0])?;
    let scene = camera.scene(
        PreviewShape::Sphere,
        FrameInputs {
            time: 0.0,
            delta_time: 0.0,
            physical_size: [SIZE, SIZE],
        },
    );
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let mut reference: Option<Vec<u8>> = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::preview_source_files();
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":path}).to_string(),
        );
        files.insert("main.fr".into(), "surface probe(sp: surf) -> material(standard) { compose { base(albedo: rgba(0.7, 0.4, 0.2, 1.0), roughness: 0.7) } }".into());
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
        let factory = manifest
            .vertex_factories
            .iter()
            .find(|f| f.name == "preview_static")
            .unwrap();
        let mut renderers = Vec::new();
        for (name, mesh) in [
            ("probe", PreviewGeometry::sphere()),
            ("fresco_scene_ground", PreviewGeometry::plane()),
        ] {
            let geometry = MeshGeometry::prepare(
                device,
                queue,
                factory,
                mesh.vertex_count,
                &mesh.streams(),
                Some(&mesh.indices),
            )
            .await?;
            renderers.push(
                MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &manifest,
                    name,
                    geometry,
                    FORMAT,
                )
                .await?,
            );
        }
        let floor = renderers.pop().unwrap();
        let mut renderer = renderers.pop().unwrap();
        renderer.render(&scene, view, depth.view())?;
        let isolated = readback(device, queue, output)?;
        renderer.add_object(
            floor,
            [
                4.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, -0.92, 0.0, 1.0,
            ],
        )?;
        renderer.render(&scene, view, depth.view())?;
        let pixels = readback(device, queue, output)?;
        let floor_pixels: Vec<_> = isolated
            .as_chunks::<4>()
            .0
            .iter()
            .zip(pixels.as_chunks::<4>().0)
            .enumerate()
            .filter_map(|(i, (a, b))| (a[3] == 0 && b[3] == 255).then_some(i))
            .collect();
        assert!(
            floor_pixels.len() > 200,
            "floor must be actual covered geometry: {path}"
        );
        if path == "deferred" {
            let identity = renderer.material_id_texture().unwrap();
            assert_eq!(identity.format(), wgpu::TextureFormat::Rg32Uint);
            let ids = readback(device, queue, identity)?;
            assert_eq!(ids.len(), (SIZE * SIZE * 8) as usize);
            let ids: Vec<_> = ids
                .as_chunks::<8>()
                .0
                .iter()
                .map(|p| u32::from_le_bytes(p[..4].try_into().unwrap()))
                .collect();
            let center_id = ids[((SIZE * SIZE).div_euclid(2) + SIZE.div_euclid(2)) as usize];
            assert!(
                floor_pixels
                    .iter()
                    .all(|i| ids[*i] != 0 && ids[*i] != center_id),
                "floor must have its own generated material ID"
            );
        }
        if let Some(reference) = &reference {
            assert!(
                reference
                    .iter()
                    .zip(&pixels)
                    .all(|(a, b)| a.abs_diff(*b) <= 8),
                "scene renderer mismatch {path}"
            );
        } else {
            reference = Some(pixels.clone());
        }
        image::save_buffer(
            format!("target/scene-{path}.png"),
            &pixels,
            SIZE,
            SIZE,
            image::ColorType::Rgba8,
        )?;
        let mut lighting = LightingEnvironment::Preview.lighting();
        lighting.shadows = false;
        renderer.set_scene_lighting(lighting, &[])?;
        renderer.render(&scene, view, depth.view())?;
        let unshadowed = readback(device, queue, output)?;
        assert!(
            floor_pixels
                .iter()
                .any(|i| unshadowed[i * 4] > pixels[i * 4].saturating_add(8)),
            "floor must receive the sphere shadow: {path}"
        );
        renderer.set_background([0.075, 0.11, 0.17, 1.0], [0.23, 0.26, 0.30, 1.0])?;
        renderer.render(&scene, view, depth.view())?;
        let sky = readback(device, queue, output)?;
        assert!(sky.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        for (before, after) in unshadowed
            .as_chunks::<4>()
            .0
            .iter()
            .zip(sky.as_chunks::<4>().0)
        {
            if before[3] == 255 {
                assert_eq!(before, after);
            }
        }
        if path != "forward" {
            renderer.set_buffer_view("depth")?;
            renderer.render(&scene, view, depth.view())?;
            let shown = readback(device, queue, output)?;
            renderer.set_background([0.0; 4], [0.0; 4])?;
            renderer.render(&scene, view, depth.view())?;
            assert_eq!(
                shown,
                readback(device, queue, output)?,
                "sky must not change inspectors"
            );
            assert!(
                floor_pixels.iter().any(|i| shown[i * 4] < 250),
                "floor is missing from depth inspection"
            );
        }
    }
    verify_transparency(device, queue, output, view).await?;
    println!(
        "Scene: independent floor mesh/material, shared shadows, depth and material IDs, sky isolation, and renderer parity passed."
    );
    Ok(())
}

async fn verify_transparency(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let mut camera = OrbitCamera::default();
    camera.drag([45.0, 40.0])?;
    let scene = camera.scene(
        PreviewShape::Sphere,
        FrameInputs {
            time: 0.0,
            delta_time: 0.0,
            physical_size: [SIZE, SIZE],
        },
    );
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let mut reference: Option<Vec<u8>> = None;
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::preview_source_files();
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer":path}).to_string(),
        );
        files.insert("main.fr".into(), "surface glass(sp: surf) -> material(standard) { properties { blend: SurfaceBlend.Translucent }\n param visibility: f32 = 0.5\n compose { base(albedo: rgba(0.8, 0.1, 0.05, 1.0), roughness: 0.6, opacity: visibility) } }".into());
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
        let factory = manifest
            .vertex_factories
            .iter()
            .find(|f| f.name == "preview_static")
            .unwrap();
        let mut objects = Vec::new();
        for (name, mesh) in [
            ("glass", PreviewGeometry::sphere()),
            ("fresco_scene_ground", PreviewGeometry::plane()),
        ] {
            let geometry = MeshGeometry::prepare(
                device,
                queue,
                factory,
                mesh.vertex_count,
                &mesh.streams(),
                Some(&mesh.indices),
            )
            .await?;
            objects.push(
                MeshRenderer::prepare(
                    device.clone(),
                    queue.clone(),
                    &compiled.wgsl,
                    &manifest,
                    name,
                    geometry,
                    FORMAT,
                )
                .await?,
            );
        }
        let floor = objects.pop().unwrap();
        let mut renderer = objects.pop().unwrap();
        renderer.add_object(
            floor,
            [
                4.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, -0.92, 0.0, 1.0,
            ],
        )?;
        renderer.render(&scene, view, depth.view())?;
        let glass = readback(device, queue, output)?;
        if let Some(reference) = &reference {
            assert!(
                reference
                    .iter()
                    .zip(&glass)
                    .all(|(a, b)| a.abs_diff(*b) <= 8),
                "translucent scene mismatch: {path}"
            );
        } else {
            reference = Some(glass.clone());
        }
        let updates = serde_json::from_value(serde_json::json!({"visibility":0.0}))?;
        renderer.update_parameters(&updates)?;
        renderer.render(&scene, view, depth.view())?;
        let floor_only = readback(device, queue, output)?;
        let changed = glass
            .as_chunks::<4>()
            .0
            .iter()
            .zip(floor_only.as_chunks::<4>().0)
            .filter(|(a, b)| b[3] == 255 && a[3] == 255 && a[0].abs_diff(b[0]) > 8)
            .count();
        assert!(
            changed > 100,
            "translucent object must blend over the opaque floor, not be overwritten: {path}"
        );
        if path == "deferred" {
            let identity = renderer.material_id_texture().unwrap();
            assert_eq!(identity.format(), wgpu::TextureFormat::Rg32Uint);
            let ids = readback(device, queue, identity)?;
            assert_eq!(ids.len(), (SIZE * SIZE * 8) as usize);
            let unique: std::collections::BTreeSet<_> = ids
                .as_chunks::<8>()
                .0
                .iter()
                .map(|p| u32::from_le_bytes(p[..4].try_into().unwrap()))
                .collect();
            assert_eq!(
                unique.len(),
                2,
                "only background and opaque floor belong in the GBuffer"
            );
            assert!(unique.contains(&0));
        }
    }
    Ok(())
}
