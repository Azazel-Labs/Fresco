use fresco_artifact::ManifestRoot;
use fresco_example_engine::runtime::{
    particle_buffers::ParticleBuffers, particle_compute::ParticleCompute,
    particle_layout::ParticleLimits, particle_playback::ParticlePlayback,
};
use std::error::Error;

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    let source = include_str!("../../../../examples/50) particles/drifting_sparks.fr")
        .replace("spawn_rate: 60.0", "spawn_rate: 4.0\n    burst_count: 0")
        .replace("max_lifespan: 2.0", "max_lifespan: 1.0");
    files.insert("main.fr".into(), source);
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let contract =
        &fresco_example_engine::runtime::particle_contract::ParticleContract::for_surface(
            &manifest,
            &manifest.surfaces[0].name,
        )?
        .unwrap();
    let buffers = ParticleBuffers::prepare(device, contract, contract.particle_count).await?;
    let mut playback = ParticlePlayback::new(contract, ParticleLimits::from(&device.limits()))?;
    let step = playback.begin_step(0.375)?;
    let commands = step.commands().unwrap().to_vec();
    let scene = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particle scene binding probe"),
        size: 240,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    let bindings = fresco_example_engine::runtime::particle_bindings::ParticleBindings::prepare(
        device, contract, &buffers, &scene,
    )
    .await?;
    let highest = *bindings.compute.layouts().keys().next_back().unwrap();
    let layouts: Vec<_> = (0..=highest)
        .map(|g| bindings.compute.layouts().get(&g))
        .collect();
    let compute = ParticleCompute::prepare(device, &output.wgsl, contract, &layouts).await?;
    let groups = bindings.compute.groups();
    for mutation in ["duplicate", "access", "missing", "collision"] {
        let mut bad = contract.clone();
        match mutation {
            "duplicate" => bad.bindings.push(bad.bindings[0].clone()),
            "access" => {
                bad.bindings
                    .iter_mut()
                    .find(|b| b.name == "particles")
                    .unwrap()
                    .access = "read".into();
            }
            "missing" => bad.bindings.retain(|b| b.name != "particle_slots"),
            "collision" => {
                let slot = bad
                    .bindings
                    .iter()
                    .find(|b| b.name == "particles")
                    .unwrap()
                    .binding;
                bad.bindings
                    .iter_mut()
                    .find(|b| b.name == "particle_config")
                    .unwrap()
                    .binding = slot;
            }
            _ => unreachable!(),
        }
        assert!(
            fresco_example_engine::runtime::particle_bindings::ParticleBindings::prepare(
                device, &bad, &buffers, &scene,
            )
            .await
            .is_err(),
            "must reject {mutation}"
        );
    }
    playback.validate_step(&step)?;
    buffers.write_step(queue, step.delta(), step.commands())?;
    let mut encoder = device.create_command_encoder(&Default::default());
    compute.encode(
        &mut encoder,
        groups,
        step.layout().capacity,
        step.spawn(),
        step.update(),
    )?;
    queue.submit([encoder.finish()]);
    playback.commit(step)?;
    let state = super::particles::read(device, queue, buffers.state())?;
    let age = contract
        .state_fields
        .iter()
        .find(|f| f.name == "age")
        .unwrap()
        .offset as usize;
    assert_eq!(
        f32::from_le_bytes(state[age..age + 4].try_into().unwrap()),
        0.125,
        "new birth advances only its partial frame"
    );
    let config_before = super::particles::read(device, queue, buffers.config())?;
    let slots_before = super::particles::read(device, queue, buffers.slots().unwrap())?;
    let mut bad = commands;
    bad[0][3] = f32::NAN.to_bits();
    assert!(buffers.write_step(queue, 0.5, Some(&bad)).is_err());
    assert!(buffers.write_step(queue, 0.5, None).is_err());
    assert_eq!(
        super::particles::read(device, queue, buffers.config())?,
        config_before
    );
    assert_eq!(
        super::particles::read(device, queue, buffers.slots().unwrap())?,
        slots_before
    );
    let paused = playback.begin_step(0.0)?;
    playback.validate_step(&paused)?;
    buffers.write_step(queue, paused.delta(), paused.commands())?;
    let mut encoder = device.create_command_encoder(&Default::default());
    compute.encode(
        &mut encoder,
        groups,
        paused.layout().capacity,
        paused.spawn(),
        paused.update(),
    )?;
    queue.submit([encoder.finish()]);
    playback.commit(paused)?;
    assert_eq!(
        super::particles::read(device, queue, buffers.state())?,
        state,
        "zero-time step does not respawn existing particles"
    );
    println!(
        "Managed particle GPU checks: partial-frame births, atomic invalid uploads, and paused spawn isolation passed."
    );
    Ok(())
}
