//! Checked particle storage and dispatch dimensions, independent of a GPU device.
use crate::runtime::particle_contract::ParticleContract;

#[derive(Clone, Copy, Debug)]
pub struct ParticleLimits {
    pub max_buffer_size: u64,
    pub max_storage_buffer_binding_size: u64,
    pub max_compute_workgroups_per_dimension: u32,
    pub max_compute_workgroup_size_x: u32,
    pub max_compute_invocations_per_workgroup: u32,
}

#[cfg(feature = "runtime")]
impl From<&wgpu::Limits> for ParticleLimits {
    fn from(value: &wgpu::Limits) -> Self {
        Self {
            max_buffer_size: value.max_buffer_size,
            max_storage_buffer_binding_size: value.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: value.max_compute_workgroups_per_dimension,
            max_compute_workgroup_size_x: value.max_compute_workgroup_size_x,
            max_compute_invocations_per_workgroup: value.max_compute_invocations_per_workgroup,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleLayout {
    pub capacity: u32,
    pub state_bytes: u64,
    pub slot_bytes: Option<u64>,
    pub workgroups: u32,
}

impl ParticleLayout {
    pub fn new(
        contract: &ParticleContract,
        capacity: u32,
        limits: ParticleLimits,
    ) -> Result<Self, &'static str> {
        if capacity == 0
            || contract.particle_count == 0
            || contract.vertex_count == 0
            || contract.particle_stride == 0
            || !contract.particle_stride.is_multiple_of(4)
        {
            return Err("particle counts and aligned stride must be nonzero");
        }
        let group = contract.workgroup_size;
        if group == 0
            || group > limits.max_compute_workgroup_size_x
            || group > limits.max_compute_invocations_per_workgroup
        {
            return Err("particle workgroup size exceeds device limits");
        }
        let workgroups = capacity.div_ceil(group);
        if workgroups > limits.max_compute_workgroups_per_dimension {
            return Err("particle dispatch exceeds device limits");
        }
        let state_bytes = u64::from(capacity) * u64::from(contract.particle_stride);
        let slot_bytes = if let Some(allocation) = &contract.allocation {
            if allocation.initial_capacity != contract.particle_count
                || capacity > allocation.max_capacity
                || (allocation.mode == "fixed" && capacity != allocation.initial_capacity)
            {
                return Err("particle capacity conflicts with allocation policy");
            }
            Some(u64::from(capacity) * 16)
        } else {
            if capacity != contract.particle_count {
                return Err("unmanaged particle capacity cannot change");
            }
            None
        };
        for bytes in std::iter::once(state_bytes).chain(slot_bytes) {
            if bytes > limits.max_buffer_size || bytes > limits.max_storage_buffer_binding_size {
                return Err("particle storage exceeds device buffer limits");
            }
        }
        Ok(Self {
            capacity,
            state_bytes,
            slot_bytes,
            workgroups,
        })
    }

    /// Configuration ABI: delta-time f32, capacity u32, and eight padding bytes.
    pub fn config_bytes(self, delta: f32) -> Result<[u8; 16], &'static str> {
        if !delta.is_finite() || delta < 0.0 {
            return Err("particle simulation requires finite non-negative time");
        }
        let mut bytes = [0; 16];
        bytes[..4].copy_from_slice(&delta.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.capacity.to_le_bytes());
        Ok(bytes)
    }
}
