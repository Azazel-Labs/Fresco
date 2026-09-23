use fresco_artifact::ManifestRoot;
use fresco_example_engine::runtime::particle_contract::{ParticleAllocation, ParticleContract};
use fresco_example_engine::runtime::particle_layout::{ParticleLayout, ParticleLimits};

fn contract() -> ParticleContract {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "engine/core/06_particle_contract.fr".into(),
        include_str!("../../../tests/fixtures/particle_contract.fr").into(),
    );
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    ParticleContract::for_surface(&manifest, &manifest.surfaces[0].name)
        .unwrap()
        .unwrap()
}

fn limits() -> ParticleLimits {
    ParticleLimits {
        max_buffer_size: 1 << 24,
        max_storage_buffer_binding_size: 1 << 24,
        max_compute_workgroups_per_dimension: 65535,
        max_compute_workgroup_size_x: 256,
        max_compute_invocations_per_workgroup: 256,
    }
}

#[test]
fn compiled_contract_controls_storage_dispatch_and_config_bytes() {
    let mut contract = contract();
    contract.particle_count = 129;
    contract.workgroup_size = 32;
    let layout = ParticleLayout::new(&contract, 129, limits()).unwrap();
    assert_eq!(
        layout.state_bytes,
        129 * u64::from(contract.particle_stride)
    );
    assert_eq!(layout.workgroups, 5);
    assert_eq!(layout.slot_bytes, None);
    assert_eq!(
        layout.config_bytes(0.5).unwrap(),
        [0, 0, 0, 63, 129, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    for delta in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(layout.config_bytes(delta).is_err());
    }
    assert!(ParticleLayout::new(&contract, 130, limits()).is_err());
}

#[test]
fn device_limits_cover_dispatch_state_and_managed_slot_storage() {
    let mut contract = contract();
    let count = contract.particle_count;
    let exact = u64::from(count) * u64::from(contract.particle_stride);
    assert!(
        ParticleLayout::new(
            &contract,
            count,
            ParticleLimits {
                max_buffer_size: exact - 1,
                ..limits()
            }
        )
        .is_err()
    );
    assert!(
        ParticleLayout::new(
            &contract,
            count,
            ParticleLimits {
                max_storage_buffer_binding_size: exact - 1,
                ..limits()
            }
        )
        .is_err()
    );
    assert!(
        ParticleLayout::new(
            &contract,
            count,
            ParticleLimits {
                max_compute_workgroups_per_dimension: 0,
                ..limits()
            }
        )
        .is_err()
    );
    assert!(
        ParticleLayout::new(
            &contract,
            count,
            ParticleLimits {
                max_compute_invocations_per_workgroup: 1,
                ..limits()
            }
        )
        .is_err()
    );
    contract.allocation = Some(ParticleAllocation {
        mode: "automatic".into(),
        initial_capacity: count,
        max_capacity: count * 2,
        growth_factor: 2.0,
        spawn_rate: 1.0,
        spawn_burst: 0,
        max_spawn_per_step: 10,
        max_lifespan: 1.0,
        overflow: "drop_new".into(),
    });
    let grown = ParticleLayout::new(&contract, count * 2, limits()).unwrap();
    assert_eq!(grown.slot_bytes, Some(u64::from(count) * 32));
    assert!(ParticleLayout::new(&contract, count * 2 + 1, limits()).is_err());
    // A valid compact state may fit while the separate 16-byte slot ABI does not.
    contract.particle_stride = 4;
    assert!(
        ParticleLayout::new(
            &contract,
            count,
            ParticleLimits {
                max_buffer_size: u64::from(count) * 8,
                ..limits()
            }
        )
        .is_err()
    );
    contract.workgroup_size = 0;
    assert!(ParticleLayout::new(&contract, count, limits()).is_err());
    contract.workgroup_size = u32::MAX;
    assert!(ParticleLayout::new(&contract, count, limits()).is_err());
}
