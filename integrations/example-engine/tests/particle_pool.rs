use fresco_example_engine::runtime::particle_contract::ParticleAllocation;

use fresco_example_engine::runtime::particle_pool::ParticlePool;

fn policy() -> ParticleAllocation {
    ParticleAllocation {
        mode: "fixed".into(),
        initial_capacity: 2,
        max_capacity: 8,
        growth_factor: 2.0,
        spawn_rate: 2.0,
        spawn_burst: 2,
        max_lifespan: 1.0,
        max_spawn_per_step: 100,
        overflow: "drop_new".into(),
    }
}

fn active(pool: &ParticlePool) -> u32 {
    pool.commands().iter().map(|s| s[2]).sum()
}

#[test]
fn fixed_storage_expires_leases_and_reuses_slots_with_new_identities() {
    let mut pool = ParticlePool::new(policy(), 8).unwrap();
    pool.advance(0.0).unwrap();
    assert_eq!(active(&pool), 2);
    pool.advance(0.5).unwrap();
    assert_eq!(pool.dropped(), 1);
    pool.advance(0.5).unwrap();
    assert_eq!(active(&pool), 1);
    assert_eq!(pool.commands()[1][0], 3);
    assert_eq!(pool.capacity(), 2);
    assert_eq!(pool.peak(), 2);
}

#[test]
fn growth_is_bounded_and_reset_obeys_allocation_mode() {
    for mode in ["estimated", "automatic"] {
        let mut pool = ParticlePool::new(
            ParticleAllocation {
                mode: mode.into(),
                spawn_burst: 10,
                ..policy()
            },
            8,
        )
        .unwrap();
        pool.advance(0.0).unwrap();
        assert_eq!(pool.capacity(), 8);
        assert_eq!(active(&pool), 8);
        assert_eq!(pool.dropped(), 2);
        assert_eq!(pool.commands()[0][0], 0);
        pool.reset();
        assert_eq!(pool.capacity(), if mode == "automatic" { 8 } else { 2 });
        assert_eq!(active(&pool), 0);
        assert_eq!(pool.peak(), 0);
        assert_eq!(pool.dropped(), 0);
    }
}

#[test]
fn births_receive_partial_frame_time_in_little_endian_commands() {
    let mut pool = ParticlePool::new(
        ParticleAllocation {
            spawn_burst: 0,
            spawn_rate: 4.0,
            ..policy()
        },
        8,
    )
    .unwrap();
    pool.advance(0.375).unwrap();
    assert_eq!(active(&pool), 1);
    assert_eq!(f32::from_bits(pool.commands()[0][3]), 0.125);
    let bytes = pool.bytes();
    assert_eq!(bytes.len(), pool.capacity() * 16);
    assert_eq!(
        &bytes[..16],
        &[0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 62]
    );
    pool.advance(0.0).unwrap();
    assert_eq!(pool.commands()[0][1], 0, "birth command fires only once");
}

#[test]
fn bounded_work_and_invalid_steps_preserve_installed_commands() {
    let mut pool = ParticlePool::new(
        ParticleAllocation {
            spawn_burst: 100,
            max_spawn_per_step: 1,
            ..policy()
        },
        8,
    )
    .unwrap();
    pool.advance(0.0).unwrap();
    assert_eq!(active(&pool), 1);
    assert_eq!(pool.dropped(), 99);
    let bytes = pool.bytes();
    for invalid in [-1.0, f64::INFINITY, f64::NAN, 20_000_000.0] {
        assert!(pool.advance(invalid).is_err());
        assert_eq!(pool.bytes(), bytes);
        assert_eq!(pool.dropped(), 99);
    }
    pool.reset();
    pool.advance(0.0).unwrap();
    assert_eq!(pool.commands()[0][0], 0);
}

#[test]
fn policies_are_validated_before_allocating() {
    assert!(ParticlePool::new(policy(), 1).is_err());
    for invalid in [
        ParticleAllocation {
            mode: "unknown".into(),
            ..policy()
        },
        ParticleAllocation {
            overflow: "overwrite".into(),
            ..policy()
        },
        ParticleAllocation {
            initial_capacity: 0,
            ..policy()
        },
        ParticleAllocation {
            max_lifespan: f32::NAN,
            ..policy()
        },
        ParticleAllocation {
            spawn_rate: -1.0,
            ..policy()
        },
        ParticleAllocation {
            growth_factor: 1.0,
            ..policy()
        },
    ] {
        assert!(ParticlePool::new(invalid, 8).is_err());
    }
}
