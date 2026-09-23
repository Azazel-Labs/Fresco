//! Compare culled rendering against an independently selected all-light draw.
use super::{FORMAT, SIZE, mesh::geometry, readback};
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{depth::DepthTarget, forward_plus::PointLight, mesh::MeshRenderer},
};
use std::error::Error;

async fn renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    brute: bool,
) -> Result<MeshRenderer, Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files_for_renderer(true);
    if brute {
        let contract = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
        let condition = "(mask & (u32(1) << (i % u32(32)))) != u32(0)";
        assert!(contract.contains(condition));
        *contract = contract.replace(condition, "true");
    }
    files.insert("main.fr".into(), "surface probe(sp: surf) -> material(standard) { compose { base(albedo: rgba(0.8, 0.8, 0.8, 1.0)) } }".into());
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest = serde_json::from_str(&output.manifest)?;
    Ok(MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "probe",
        geometry(device, queue, &manifest).await?,
        FORMAT,
    )
    .await?)
}

fn masks(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &MeshRenderer,
) -> Result<Vec<u32>, Box<dyn Error>> {
    let size = u64::from(SIZE.div_ceil(16) * SIZE.div_ceil(16) * 8);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tile mask readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(renderer.light_tile_masks().unwrap(), 0, &staging, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(std::time::Duration::from_secs(20)),
    })?;
    receive.recv_timeout(std::time::Duration::from_secs(20))??;
    let result = staging
        .slice(..)
        .get_mapped_range()?
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| u32::from_le_bytes(*v))
        .collect();
    staging.unmap();
    Ok(result)
}

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let mut tiled = renderer(device, queue, false).await?;
    let mut brute = renderer(device, queue, true).await?;
    tiled.set_lighting_environment(
        fresco_example_engine::runtime::forward_plus::LightingEnvironment::ThreeLights,
    )?;
    brute.set_lighting_environment(
        fresco_example_engine::runtime::forward_plus::LightingEnvironment::ThreeLights,
    )?;
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let lights: Vec<_> = (0..64)
        .map(|i| PointLight {
            position: [
                (i % 8) as f32 * 0.35 - 1.2,
                (i >> 3) as f32 * 0.35 - 1.2,
                0.2,
            ],
            radius: 0.55,
            color: [0.8, 0.35, 0.1],
            intensity: 0.7,
        })
        .collect();
    tiled.set_point_lights(&lights)?;
    brute.set_point_lights(&lights)?;
    for camera in [0.0, 0.4, -0.5] {
        scene.view[12] = camera;
        tiled.render(&scene, view, depth.view())?;
        let actual = readback(device, queue, texture)?;
        let tile_masks = masks(device, queue, &tiled)?;
        assert!(
            tile_masks.iter().any(|v| *v != u32::MAX),
            "GPU must reject non-overlapping lights"
        );
        assert!(
            tile_masks.iter().any(|v| *v != 0),
            "GPU must retain overlapping lights"
        );
        brute.render(&scene, view, depth.view())?;
        assert_eq!(
            actual,
            readback(device, queue, texture)?,
            "tile culling must match all 64 lights across camera changes"
        );
        assert!(
            actual.as_chunks::<4>().0.iter().any(|p| p[0] > p[2] + 10),
            "lights must contribute visible color"
        );
    }
    // All 64 lights overlap every visible tile: no list overflow or last-bit loss.
    let overlapping = vec![
        PointLight {
            position: [0.0, 0.0, 0.4],
            radius: 10.0,
            color: [1.0, 0.5, 0.2],
            intensity: 0.005
        };
        64
    ];
    tiled.set_point_lights(&overlapping)?;
    brute.set_point_lights(&overlapping)?;
    tiled.render(&scene, view, depth.view())?;
    let full = readback(device, queue, texture)?;
    assert!(masks(device, queue, &tiled)?.iter().all(|v| *v == u32::MAX));
    brute.render(&scene, view, depth.view())?;
    assert_eq!(full, readback(device, queue, texture)?);
    assert!(tiled.set_point_lights(&vec![overlapping[0]; 65]).is_err());
    tiled.render(&scene, view, depth.view())?;
    assert_eq!(
        full,
        readback(device, queue, texture)?,
        "invalid light edits preserve installed buffers"
    );
    tiled.set_point_lights(&[])?;
    tiled.render(&scene, view, depth.view())?;
    let ambient = readback(device, queue, texture)?;
    assert!(masks(device, queue, &tiled)?.iter().all(|v| *v == 0));
    assert_ne!(full, ambient, "empty light set must clear stale masks");
    assert!(
        ambient
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .all(|p| p[0] == p[1] && p[1] == p[2] && p[0] <= 7)
    );
    scene.frame.physical_size = [8193, SIZE];
    assert!(tiled.render(&scene, view, depth.view()).is_err());
    assert_eq!(ambient, readback(device, queue, texture)?);
    scene.frame.physical_size = [0, SIZE];
    assert!(!tiled.render(&scene, view, depth.view())?);
    println!(
        "Forward+ GPU checks: 64-light culling matches brute force, camera motion, overlap, empty lists and invalid-edit isolation passed."
    );
    Ok(())
}
