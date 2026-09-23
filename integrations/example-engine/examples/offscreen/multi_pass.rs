//! One geometry/material, two invocations of a depth program, and a color pass.
use super::{FORMAT, SIZE, readback};
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{
        depth::DepthTarget,
        mesh::{FactoryInputs, MeshRenderer, MeshResources},
        mesh_geometry::MeshGeometry,
        textures::TextureInputs,
        vertices::VertexValues,
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
};

pub async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let files = HashMap::from([
        ("engine/engine.fr".into(), include_str!("../../tests/fixtures/multi-pass-engine.fr").into()),
        ("main.fr".into(), "surface sample(sp: SamplePoint) -> material(Pigment) { compose { Pigment(tint: rgba(0.2, 0.4, 0.6, 1.0)) } }".into()),
    ]);
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
    let points = [-1.0, -1.0, 0.0, 3.0, -1.0, 0.0, -1.0, 3.0, 0.0];
    let geometry = MeshGeometry::prepare(
        device,
        queue,
        &manifest.vertex_factories[0],
        3,
        &BTreeMap::from([("point".into(), VertexValues::F32(&points))]),
        None,
    )
    .await?;
    let make_view = |depth: f32, gain: f32| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("independent draw input"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let data: Vec<_> = [depth, gain, 0.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect();
        queue.write_buffer(&buffer, 0, &data);
        buffer
    };
    let buffers = BTreeMap::from([
        ("camera_a".into(), make_view(0.25, 1.0)),
        ("camera_b".into(), make_view(0.75, 0.5)),
    ]);
    let mut renderer = MeshRenderer::prepare_with_resources(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "sample",
        MeshResources {
            factory: Some(FactoryInputs {
                name: "plain",
                buffers: &buffers,
            }),
            geometry,
            textures: &TextureInputs::new(),
            parameters: None,
        },
        FORMAT,
    )
    .await?;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    renderer.render(&scene, view, depth.view())?;
    let check_depth = |name: &str, expected: f32| -> Result<(), Box<dyn Error>> {
        let pixels = readback(device, queue, renderer.resource_texture(name).unwrap())?;
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| (f32::from_le_bytes(*p) - expected).abs() < 0.0001),
            "{name}"
        );
        Ok(())
    };
    check_depth("measured", 0.25)?;
    check_depth("measured_again", 0.75)?;
    let pixels = readback(device, queue, output)?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0].abs_diff(26) <= 1 && p[1] == 51 && p[2].abs_diff(77) <= 1)
    );
    // A live edit changes only the draw invocations bound to that input.
    queue.write_buffer(&buffers["camera_a"], 0, &0.4f32.to_le_bytes());
    renderer.render(&scene, view, depth.view())?;
    let pixels = readback(
        device,
        queue,
        renderer.resource_texture("measured").unwrap(),
    )?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| (f32::from_le_bytes(*p) - 0.4).abs() < 0.0001)
    );
    let pixels = readback(
        device,
        queue,
        renderer.resource_texture("measured_again").unwrap(),
    )?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| (f32::from_le_bytes(*p) - 0.75).abs() < 0.0001)
    );
    println!(
        "Multi-pass mesh: reused depth entries, independent view bindings, material color, and isolated live updates passed"
    );
    Ok(())
}
