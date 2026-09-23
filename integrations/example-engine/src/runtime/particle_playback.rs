//! Transactional particle time and spawn state, independent of either host.
use super::{
    particle_layout::{ParticleLayout, ParticleLimits},
    particle_pool::ParticlePool,
};
use crate::runtime::particle_contract::ParticleContract;

use std::sync::Arc;

pub struct ParticlePlayback {
    contract: ParticleContract,
    limits: ParticleLimits,
    pool: Option<ParticlePool>,
    needs_spawn: bool,
    revision: u64,
    identity: Arc<()>,
}

/// Owns prospective scheduler changes while GPU preparation is pending.
pub struct ParticleStep {
    pool: Option<ParticlePool>,
    layout: ParticleLayout,
    delta: f32,
    spawn: bool,
    revision: u64,
    identity: Arc<()>,
}

impl ParticleStep {
    pub fn layout(&self) -> ParticleLayout {
        self.layout
    }
    pub fn delta(&self) -> f32 {
        self.delta
    }
    pub fn spawn(&self) -> bool {
        self.spawn
    }
    pub fn update(&self) -> bool {
        self.delta > 0.0
    }
    pub fn commands(&self) -> Option<&[[u32; 4]]> {
        self.pool.as_ref().map(ParticlePool::commands)
    }
}

impl ParticlePlayback {
    /// Committed scheduler state, for host diagnostics.
    pub fn pool(&self) -> Option<&ParticlePool> {
        self.pool.as_ref()
    }

    pub fn new(contract: &ParticleContract, limits: ParticleLimits) -> Result<Self, &'static str> {
        ParticleLayout::new(contract, contract.particle_count, limits)?;
        let pool = contract
            .allocation
            .as_ref()
            .map(|policy| {
                if policy.mode != "fixed" {
                    ParticleLayout::new(contract, policy.max_capacity, limits)?;
                }
                ParticlePool::new(policy.clone(), policy.max_capacity)
            })
            .transpose()?;
        Ok(Self {
            contract: contract.clone(),
            limits,
            pool,
            needs_spawn: true,
            revision: 0,
            identity: Arc::new(()),
        })
    }

    pub fn begin_step(&self, delta: f32) -> Result<ParticleStep, &'static str> {
        if !delta.is_finite() || delta < 0.0 {
            return Err("particle playback requires finite non-negative time");
        }
        self.revision
            .checked_add(1)
            .ok_or("particle playback revision exhausted")?;
        let mut pool = self.pool.clone();
        if let Some(pool) = &mut pool {
            pool.advance(f64::from(delta))?;
        }
        let capacity = match &pool {
            Some(pool) => {
                u32::try_from(pool.capacity()).map_err(|_| "particle capacity exceeds ABI")?
            }
            None => self.contract.particle_count,
        };
        Ok(ParticleStep {
            layout: ParticleLayout::new(&self.contract, capacity, self.limits)?,
            pool,
            delta,
            spawn: self.needs_spawn || self.pool.is_some(),
            revision: self.revision,
            identity: self.identity.clone(),
        })
    }

    /// Check immediately before queue writes; do not await between validation and commit.
    pub fn validate_step(&self, step: &ParticleStep) -> Result<(), &'static str> {
        if !Arc::ptr_eq(&self.identity, &step.identity) || self.revision != step.revision {
            return Err("particle step is stale or belongs to another playback instance");
        }
        Ok(())
    }

    pub fn commit(&mut self, step: ParticleStep) -> Result<(), &'static str> {
        self.validate_step(&step)?;
        let next = self
            .revision
            .checked_add(1)
            .ok_or("particle playback revision exhausted")?;
        self.pool = step.pool;
        self.needs_spawn = false;
        self.revision = next;
        Ok(())
    }

    /// Reset invalidates outstanding steps. Automatic pools retain grown capacity.
    pub fn reset(&mut self) -> Result<(), &'static str> {
        let next = self
            .revision
            .checked_add(1)
            .ok_or("particle playback revision exhausted")?;
        if let Some(pool) = &mut self.pool {
            pool.reset();
        }
        self.needs_spawn = true;
        self.revision = next;
        Ok(())
    }
}
