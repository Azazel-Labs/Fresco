//! Owned particle storage; replacement candidates never mutate installed buffers.
use crate::runtime::particle_contract::ParticleContract;

use wgpu::util::DeviceExt;

use super::{
    RuntimeError,
    particle_layout::{ParticleLayout, ParticleLimits},
};

#[derive(Clone)]
pub struct ParticleBuffers {
    contract: ParticleContract,
    layout: ParticleLayout,
    state: wgpu::Buffer,
    slots: Option<wgpu::Buffer>,
    config: wgpu::Buffer,
}

impl ParticleBuffers {
    pub async fn prepare(
        device: &wgpu::Device,
        contract: &ParticleContract,
        capacity: u32,
    ) -> Result<Self, RuntimeError> {
        let limits = device.limits();
        let layout = ParticleLayout::new(contract, capacity, ParticleLimits::from(&limits))
            .map_err(|e| RuntimeError::ParticleResources(e.into()))?;
        if limits.max_uniform_buffer_binding_size < 16 || limits.max_buffer_size < 16 {
            return Err(RuntimeError::ParticleResources(
                "simulation uniform exceeds device limits".into(),
            ));
        }
        let config_bytes = layout
            .config_bytes(0.0)
            .map_err(|e| RuntimeError::ParticleResources(e.into()))?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let storage = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let state = storage("particle state", layout.state_bytes);
        let slots = layout
            .slot_bytes
            .map(|bytes| storage("particle slot commands", bytes));
        let config = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle simulation config"),
            contents: &config_bytes,
            usage: wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            contract: contract.clone(),
            layout,
            state,
            slots,
            config,
        })
    }

    /// Preserve the retained prefix and leave new storage zero-initialized.
    /// Hosts must serialize simulation writes while copying a replacement.
    pub async fn resized(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        capacity: u32,
    ) -> Result<Self, RuntimeError> {
        let candidate = Self::prepare(device, &self.contract, capacity).await?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("preserve particle storage"),
        });
        encoder.copy_buffer_to_buffer(
            &self.state,
            0,
            &candidate.state,
            0,
            self.layout.state_bytes.min(candidate.layout.state_bytes),
        );
        if let (Some(old), Some(new)) = (&self.slots, &candidate.slots) {
            encoder.copy_buffer_to_buffer(old, 0, new, 0, old.size().min(new.size()));
        }
        queue.submit([encoder.finish()]);
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(candidate)
    }

    pub fn layout(&self) -> ParticleLayout {
        self.layout
    }

    /// Validate the entire step before either queue write. Managed commands come
    /// from ParticlePool after any required storage growth has been installed.
    pub fn write_step(
        &self,
        queue: &wgpu::Queue,
        delta: f32,
        commands: Option<&[[u32; 4]]>,
    ) -> Result<(), RuntimeError> {
        let invalid = |reason: &str| RuntimeError::ParticleResources(reason.into());
        let config = self.layout.config_bytes(delta).map_err(invalid)?;
        let bytes = match (&self.slots, commands) {
            (None, None) => None,
            (Some(_), Some(commands)) => {
                if commands.len() != self.layout.capacity as usize {
                    return Err(invalid("slot commands do not match particle capacity"));
                }
                for words in commands {
                    let age = f32::from_bits(words[3]);
                    if words[0] > 16_777_215
                        || words[1] > 1
                        || words[2] > 1
                        || !age.is_finite()
                        || age < 0.0
                        || age > delta
                    {
                        return Err(invalid("invalid particle slot command"));
                    }
                }
                Some(
                    commands
                        .iter()
                        .flatten()
                        .flat_map(|word| word.to_le_bytes())
                        .collect::<Vec<_>>(),
                )
            }
            _ => return Err(invalid("slot commands must match managed allocation mode")),
        };
        queue.write_buffer(&self.config, 0, &config);
        if let (Some(buffer), Some(bytes)) = (&self.slots, bytes) {
            queue.write_buffer(buffer, 0, &bytes);
        }
        Ok(())
    }
    pub fn state(&self) -> &wgpu::Buffer {
        &self.state
    }
    pub fn slots(&self) -> Option<&wgpu::Buffer> {
        self.slots.as_ref()
    }
    pub fn config(&self) -> &wgpu::Buffer {
        &self.config
    }
}
