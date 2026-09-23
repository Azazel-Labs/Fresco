use super::{FORMAT, SIZE, readback};
use fresco_artifact::ManifestRoot;
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{depth::DepthTarget, particle_buffers::ParticleBuffers, particle_draw::ParticleDraw},
};
use std::error::Error;
use wgpu::util::DeviceExt;

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    wgsl: &str,
    manifest: &ManifestRoot,
    buffers: &ParticleBuffers,
) -> Result<(), Box<dyn Error>> {
    let surface = &manifest.surfaces[0];
    let contract =
        &fresco_example_engine::runtime::particle_contract::ParticleContract::for_surface(
            manifest,
            &surface.name,
        )?
        .unwrap();
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    })
    .pack()?;
    let scene = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("particle draw scene"),
        contents: &scene,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let occupied: Vec<_> = contract
        .bindings
        .iter()
        .map(|b| (b.group, b.binding))
        .collect();
    let mut material =
        fresco_example_engine::runtime::material_resources::MaterialResources::prepare(
            device,
            queue,
            surface,
            &occupied,
            &fresco_example_engine::runtime::textures::TextureInputs::new(),
            None,
        )
        .await?;
    let bindings =
        fresco_example_engine::runtime::particle_bindings::ParticleBindings::prepare_with_material(
            device, contract, buffers, &scene, &material,
        )
        .await?;
    let highest = *bindings.draw.layouts().keys().next_back().unwrap();
    let layouts: Vec<_> = (0..=highest)
        .map(|g| bindings.draw.layouts().get(&g))
        .collect();
    let draw = ParticleDraw::prepare(
        device,
        wgsl,
        contract,
        surface.settings.as_ref().unwrap(),
        &layouts,
        FORMAT,
    )
    .await?;
    let groups = bindings.draw.groups();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("particle raster output"),
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
    let view = texture.create_view(&Default::default());
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for (count, green) in [
        (contract.particle_count, false),
        (contract.particle_count, true),
        (0, true),
    ] {
        let tint = if green {
            serde_json::json!([0, 1, 0, 1])
        } else {
            serde_json::json!([1, 1, 1, 1])
        };
        material.update_parameters(queue, &serde_json::Map::from_iter([("tint".into(), tint)]))?;
        let before = material.values();
        assert!(
            material
                .update_parameters(
                    queue,
                    &serde_json::Map::from_iter([("tint".into(), serde_json::json!([1, 2, 3]))])
                )
                .is_err()
        );
        assert_eq!(material.values(), before);

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("particle raster probe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth.view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            draw.draw(&mut pass, groups, count);
        }
        queue.submit([encoder.finish()]);
        let pixels = readback(device, queue, &texture)?;
        let white = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| {
                p[1] > 240
                    && p[3] == 255
                    && if green {
                        p[0] < 10 && p[2] < 10
                    } else {
                        p[0] > 240 && p[2] > 240
                    }
            })
            .count();
        if count == 0 {
            assert!(pixels.iter().all(|b| *b == 0));
        } else {
            assert!(white > 30, "simulated particles must cover visible pixels");
            assert!(white < 2000, "particle draw must preserve the background");
        }
    }
    assert!(
        ParticleDraw::prepare(
            device,
            "invalid WGSL",
            contract,
            surface.settings.as_ref().unwrap(),
            &layouts,
            FORMAT
        )
        .await
        .is_err()
    );
    println!(
        "Particle raster GPU checks: authored stages draw simulated state and zero instances draw nothing."
    );
    Ok(())
}
