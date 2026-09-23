//! Conservative host-side particle lifetime reservations, without GPU readback.
use crate::runtime::particle_contract::ParticleAllocation;
use std::collections::VecDeque;

const MAX_BIRTH_ID: u32 = 16_777_215;

#[derive(Clone)]
pub struct ParticlePool {
    policy: ParticleAllocation,
    // Four ABI words per slot: birth ID, spawn flag, active flag, f32 delta bits.
    commands: Vec<[u32; 4]>,
    free: Vec<usize>,
    leases: VecDeque<(usize, f64)>,
    time: f64,
    fraction: f64,
    next_id: u32,
    first: bool,
    dropped: u32,
    peak: usize,
}

impl ParticlePool {
    pub fn new(policy: ParticleAllocation, max_slots: u32) -> Result<Self, &'static str> {
        if !matches!(policy.mode.as_str(), "fixed" | "estimated" | "automatic")
            || policy.overflow != "drop_new"
        {
            return Err("unsupported particle allocation policy");
        }
        if policy.initial_capacity == 0
            || policy.initial_capacity > policy.max_capacity
            || policy.max_capacity > max_slots
        {
            return Err("particle capacity exceeds host limits or is invalid");
        }
        if !policy.growth_factor.is_finite()
            || policy.growth_factor <= 1.0
            || !policy.spawn_rate.is_finite()
            || policy.spawn_rate < 0.0
            || !policy.max_lifespan.is_finite()
            || policy.max_lifespan <= 0.0
            || policy.max_spawn_per_step == 0
        {
            return Err("invalid particle growth, spawn, or lifespan policy");
        }
        let capacity = usize::try_from(policy.initial_capacity)
            .map_err(|_| "capacity exceeds address space")?;
        usize::try_from(policy.max_capacity)
            .ok()
            .and_then(|n| n.checked_mul(16))
            .ok_or("particle commands exceed address space")?;
        Ok(Self {
            policy,
            commands: vec![[0; 4]; capacity],
            free: (0..capacity).rev().collect(),
            leases: VecDeque::new(),
            time: 0.0,
            fraction: 0.0,
            next_id: 0,
            first: true,
            dropped: 0,
            peak: 0,
        })
    }

    pub fn capacity(&self) -> usize {
        self.commands.len()
    }
    pub fn commands(&self) -> &[[u32; 4]] {
        &self.commands
    }
    pub fn dropped(&self) -> u32 {
        self.dropped
    }
    pub fn peak(&self) -> usize {
        self.peak
    }

    /// Explicit little-endian encoding of the 16-byte slot command ABI.
    pub fn bytes(&self) -> Vec<u8> {
        self.commands
            .iter()
            .flatten()
            .flat_map(|word| word.to_le_bytes())
            .collect()
    }

    pub fn reset(&mut self) {
        if self.policy.mode != "automatic" {
            self.commands
                .truncate(self.policy.initial_capacity as usize);
        }
        self.commands.fill([0; 4]);
        self.free = (0..self.capacity()).rev().collect();
        self.leases.clear();
        self.time = 0.0;
        self.fraction = 0.0;
        self.next_id = 0;
        self.first = true;
        self.dropped = 0;
        self.peak = 0;
    }

    fn expire(&mut self, time: f64) {
        while self.leases.front().is_some_and(|(_, end)| *end <= time) {
            let (slot, _) = self.leases.pop_front().expect("checked lease");
            self.commands[slot][2] = 0;
            self.free.push(slot);
        }
    }

    /// Rejected time/identity inputs leave the pool unchanged.
    pub fn advance(&mut self, dt: f64) -> Result<(), &'static str> {
        if !dt.is_finite() || dt < 0.0 || dt > f64::from(f32::MAX) {
            return Err("particle scheduling requires finite non-negative f32 time");
        }
        let end = self.time + dt;
        let accumulated = self.fraction + dt * f64::from(self.policy.spawn_rate);
        let continuous = accumulated.floor();
        let burst = if self.first {
            self.policy.spawn_burst
        } else {
            0
        };
        let requested = continuous + f64::from(burst);
        if !end.is_finite()
            || !requested.is_finite()
            || requested > f64::from(MAX_BIRTH_ID - self.next_id)
        {
            return Err("particle birth IDs exceed the f32 ABI; reset the simulation");
        }
        let requested = requested as u32;
        let accepted = requested.min(self.policy.max_spawn_per_step);
        for words in &mut self.commands {
            words[1] = 0;
            words[3] = (dt as f32).to_bits();
        }
        for i in 0..accepted {
            let birth = if i < burst {
                self.time
            } else {
                self.time
                    + (f64::from(i - burst) + 1.0 - self.fraction)
                        / f64::from(self.policy.spawn_rate)
            };
            self.expire(birth);
            if self.free.is_empty()
                && self.policy.mode != "fixed"
                && self.capacity() < self.policy.max_capacity as usize
            {
                let old = self.capacity();
                let next = ((old as f64 * f64::from(self.policy.growth_factor))
                    .ceil()
                    .max((old + 1) as f64)
                    .min(f64::from(self.policy.max_capacity))) as usize;
                self.commands.resize(next, [0; 4]);
                self.free.extend((old..next).rev());
            }
            let Some(slot) = self.free.pop() else {
                self.dropped += 1;
                continue;
            };
            self.commands[slot] = [self.next_id + i, 1, 1, ((end - birth) as f32).to_bits()];
            self.leases
                .push_back((slot, birth + f64::from(self.policy.max_lifespan)));
            self.peak = self.peak.max(self.leases.len());
        }
        self.dropped += requested - accepted;
        self.next_id += requested;
        self.fraction = accumulated - continuous;
        self.first = false;
        self.time = end;
        self.expire(end);
        Ok(())
    }
}
