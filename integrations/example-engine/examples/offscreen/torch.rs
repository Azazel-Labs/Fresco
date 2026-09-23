//! Local GPU acceptance probe and preview for the textured torch.
use std::error::Error;

use fresco_artifact::ManifestRoot;
use fresco_example_engine::{
    assets,
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{
        depth::DepthTarget,
        particles::{ParticleInputs, ParticleRenderer},
        textures::TextureInputs,
    },
};

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        include_str!("../../../../examples/50) particles/torch.fr").into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let mut textures = TextureInputs::new();
    for (name, bytes) in [
        (
            "flame_tex",
            include_bytes!("../../../../examples/assets/particles/torch_flame.png").as_slice(),
        ),
        (
            "smoke_tex",
            include_bytes!("../../../../examples/assets/particles/torch_smoke.png").as_slice(),
        ),
        (
            "ember_tex",
            include_bytes!("../../../../examples/assets/particles/torch_ember.png").as_slice(),
        ),
    ] {
        textures.insert(name.into(), assets::decode(name, bytes)?);
    }
    let mut renderer = ParticleRenderer::prepare(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "torch",
        ParticleInputs {
            textures: &textures,
            parameters: None,
        },
        super::FORMAT,
    )
    .await?;
    let size = 512;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("torch preview"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: super::FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let depth = DepthTarget::prepare(device, [size, size]).await?.unwrap();
    let mut first = Vec::new();
    for frame in 0..180_u16 {
        renderer
            .render(
                &sphere_scene(FrameInputs {
                    time: f32::from(frame) / 60.0,
                    delta_time: 1.0 / 60.0,
                    physical_size: [size, size],
                }),
                &view,
                depth.view(),
            )
            .await?;
        if frame == 120 {
            first = super::readback(device, queue, &target)?;
        }
    }
    let pixels = super::readback(device, queue, &target)?;
    assert_ne!(first, pixels, "the simulated torch must evolve over time");
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 150 && u16::from(p[0]) > u16::from(p[2]) + 60)
            .count()
            > 50,
        "textured flame must be visible"
    );
    std::fs::create_dir_all("target")?;
    image::save_buffer(
        "target/torch-preview.png",
        &pixels,
        size,
        size,
        image::ColorType::Rgba8,
    )?;
    println!("Torch GPU checks passed; preview: target/torch-preview.png");
    Ok(())
}
