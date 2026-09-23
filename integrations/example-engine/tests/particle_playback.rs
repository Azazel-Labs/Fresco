use fresco_example_engine::runtime::particle_contract::{ParticleAllocation, ParticleContract};

use fresco_example_engine::runtime::{
    particle_layout::ParticleLimits, particle_playback::ParticlePlayback,
};

fn contract() -> ParticleContract {
    ParticleContract {
        allocation: None,
        properties: vec![],
        spawn_entry: "spawn".into(),
        compute_entry: "update".into(),
        particle_stride: 16,
        state_fields: vec![],
        simulation_pass: "sim".into(),
        draw_pass: "draw".into(),
        vertex_entry: "vertex".into(),
        fragment_entry: "fragment".into(),
        particle_count: 1,
        vertex_count: 6,
        workgroup_size: 64,
        bindings: vec![],
    }
}
fn limits() -> ParticleLimits {
    ParticleLimits {
        max_buffer_size: 4096,
        max_storage_buffer_binding_size: 4096,
        max_compute_workgroups_per_dimension: 64,
        max_compute_workgroup_size_x: 256,
        max_compute_invocations_per_workgroup: 256,
    }
}

#[test]
fn unmanaged_initialization_pause_reset_and_stale_steps() {
    let mut playback = ParticlePlayback::new(&contract(), limits()).unwrap();
    let first = playback.begin_step(0.0).unwrap();
    assert!(first.spawn());
    assert!(!first.update());
    assert!(first.commands().is_none());
    let duplicate = playback.begin_step(0.0).unwrap();
    playback.commit(first).unwrap();
    assert!(playback.commit(duplicate).is_err());
    let paused = playback.begin_step(0.0).unwrap();
    assert!(!paused.spawn());
    assert!(!paused.update());
    let advancing = playback.begin_step(0.25).unwrap();
    assert!(!advancing.spawn());
    assert!(advancing.update());
    playback.reset().unwrap();
    assert!(playback.commit(advancing).is_err());
    assert!(playback.begin_step(0.0).unwrap().spawn());
    let other = ParticlePlayback::new(&contract(), limits()).unwrap();
    assert!(playback.commit(other.begin_step(0.0).unwrap()).is_err());
}

#[test]
fn managed_growth_is_prospective_until_commit_and_reset_retains_capacity() {
    let mut contract = contract();
    contract.allocation = Some(ParticleAllocation {
        mode: "automatic".into(),
        initial_capacity: 1,
        max_capacity: 4,
        growth_factor: 2.0,
        spawn_rate: 0.0,
        spawn_burst: 3,
        max_spawn_per_step: 4,
        max_lifespan: 1.0,
        overflow: "drop_new".into(),
    });
    let mut playback = ParticlePlayback::new(&contract, limits()).unwrap();
    let abandoned = playback.begin_step(0.0).unwrap();
    assert_eq!(abandoned.layout().capacity, 4);
    let expected = abandoned.commands().unwrap().to_vec();
    drop(abandoned);
    for invalid in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(playback.begin_step(invalid).is_err());
    }
    let retry = playback.begin_step(0.0).unwrap();
    assert_eq!(retry.commands().unwrap(), expected);
    playback.commit(retry).unwrap();
    let paused = playback.begin_step(0.0).unwrap();
    assert!(paused.spawn());
    assert!(!paused.update());
    assert!(paused.commands().unwrap().iter().all(|s| s[1] == 0));
    playback.reset().unwrap();
    assert!(playback.commit(paused).is_err());
    let reset = playback.begin_step(0.0).unwrap();
    assert_eq!(reset.layout().capacity, 4);
    assert_eq!(reset.commands().unwrap(), expected);
}
