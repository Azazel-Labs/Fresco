use fresco_artifact::ManifestRoot;
use fresco_example_engine::runtime::particle_buffers::ParticleBuffers;
use fresco_example_engine::runtime::particle_compute::ParticleCompute;
use fresco_example_engine::runtime::particle_contract::ParticleAllocation;
use std::{error::Error, time::Duration};

pub(super) fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).expect("readback receiver");
        });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(Duration::from_secs(10)),
    })?;
    receive.recv_timeout(Duration::from_secs(10))??;
    let bytes = staging.slice(..).get_mapped_range()?.to_vec();
    staging.unmap();
    Ok(bytes)
}

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "engine/core/06_particle_contract.fr".into(),
        include_str!("../../../../tests/fixtures/particle_contract.fr").into(),
    );
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { param tint: color = #fff\n compose { base(albedo: tint) } }".into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let mut contract =
        fresco_example_engine::runtime::particle_contract::ParticleContract::for_surface(
            &manifest,
            &manifest.surfaces[0].name,
        )
        .unwrap()
        .unwrap();
    let count = contract.particle_count;
    let fixed = ParticleBuffers::prepare(device, &contract, count).await?;
    assert!(fixed.slots().is_none());
    assert!(read(device, queue, fixed.state())?.iter().all(|b| *b == 0));
    assert!(fixed.resized(device, queue, count + 1).await.is_err());
    let state_binding = contract
        .bindings
        .iter()
        .find(|b| b.name == "particles")
        .unwrap();
    let config_binding = contract
        .bindings
        .iter()
        .find(|b| b.name == "particle_config")
        .unwrap();
    assert_eq!(state_binding.group, config_binding.group);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("particle compute probe"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: state_binding.binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: config_binding.binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let mut layouts = vec![None; state_binding.group as usize + 1];
    layouts[state_binding.group as usize] = Some(&layout);
    let compute = ParticleCompute::prepare(device, &output.wgsl, &contract, &layouts).await?;
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("particle compute probe"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: state_binding.binding,
                resource: fixed.state().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: config_binding.binding,
                resource: fixed.config().as_entire_binding(),
            },
        ],
    });
    let groups = [(state_binding.group, group)];
    queue.write_buffer(fixed.config(), 0, &fixed.layout().config_bytes(0.5)?);
    let mut encoder = device.create_command_encoder(&Default::default());
    compute.encode(&mut encoder, &groups, count, true, true)?;
    queue.submit([encoder.finish()]);
    let simulated = read(device, queue, fixed.state())?;
    let scalar = |offset| f32::from_le_bytes(simulated[offset..offset + 4].try_into().unwrap());
    assert!(
        (scalar(0) - 0.25).abs() < 1.0e-6,
        "authored spawn sets position"
    );
    assert!(
        (scalar(4) - 0.06).abs() < 1.0e-6,
        "authored update follows spawn"
    );
    assert!(
        ParticleCompute::prepare(device, "invalid WGSL", &contract, &layouts)
            .await
            .is_err()
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    compute.encode(&mut encoder, &groups, count, false, true)?;
    queue.submit([encoder.finish()]);
    let continued = read(device, queue, fixed.state())?;
    let y = f32::from_le_bytes(continued[4..8].try_into().unwrap());
    assert!(
        (y - 0.12).abs() < 1.0e-6,
        "failed replacement retains executable update pipeline"
    );
    super::particle_draw::verify(device, queue, &output.wgsl, &manifest, &fixed).await?;
    contract.allocation = Some(ParticleAllocation {
        mode: "automatic".into(),
        initial_capacity: count,
        max_capacity: count * 2,
        growth_factor: 2.0,
        spawn_rate: 1.0,
        spawn_burst: 1,
        max_spawn_per_step: 10,
        max_lifespan: 1.0,
        overflow: "drop_new".into(),
    });
    let original = ParticleBuffers::prepare(device, &contract, count).await?;
    queue.write_buffer(original.state(), 0, &[1, 2, 3, 4, 5, 6, 7, 8]);
    queue.write_buffer(original.slots().unwrap(), 0, &[9, 10, 11, 12]);
    let before = read(device, queue, original.state())?;
    let slots = read(device, queue, original.slots().unwrap())?;
    let grown = original.resized(device, queue, count * 2).await?;
    let after = read(device, queue, grown.state())?;
    assert_eq!(&after[..before.len()], before);
    assert!(after[before.len()..].iter().all(|b| *b == 0));
    let after_slots = read(device, queue, grown.slots().unwrap())?;
    assert_eq!(&after_slots[..slots.len()], slots);
    assert!(after_slots[slots.len()..].iter().all(|b| *b == 0));
    assert_eq!(
        read(device, queue, grown.config())?,
        grown.layout().config_bytes(0.0)?
    );
    assert!(
        original
            .resized(device, queue, count * 2 + 1)
            .await
            .is_err()
    );
    assert_eq!(read(device, queue, original.state())?, before);
    queue.write_buffer(grown.state(), 0, &[0; 8]);
    assert_eq!(
        read(device, queue, original.state())?,
        before,
        "candidate owns separate storage"
    );
    println!(
        "Particle GPU checks: authored spawn/update, failed pipeline replacement, zero initialization, growth, state/slot preservation, and config ABI passed."
    );
    super::particle_managed::verify(device, queue).await?;
    Ok(())
}
