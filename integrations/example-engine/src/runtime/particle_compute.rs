//! Authored spawn/update pipelines. The renderer owns resource bindings and timing.
use super::RuntimeError;
use crate::runtime::particle_contract::ParticleContract;

pub struct ParticleCompute {
    spawn: wgpu::ComputePipeline,
    update: wgpu::ComputePipeline,
    workgroup_size: u32,
    max_workgroups: u32,
}

impl ParticleCompute {
    pub(crate) fn pipelines(&self) -> (wgpu::ComputePipeline, wgpu::ComputePipeline) {
        (self.spawn.clone(), self.update.clone())
    }

    pub async fn prepare(
        device: &wgpu::Device,
        wgsl: &str,
        contract: &ParticleContract,
        bindings: &[Option<&wgpu::BindGroupLayout>],
    ) -> Result<Self, RuntimeError> {
        let limits = device.limits();
        if contract.workgroup_size == 0
            || contract.workgroup_size > limits.max_compute_workgroup_size_x
            || contract.workgroup_size > limits.max_compute_invocations_per_workgroup
            || contract.spawn_entry.is_empty()
            || contract.compute_entry.is_empty()
        {
            return Err(RuntimeError::ParticleResources(
                "invalid particle compute entry or workgroup size".into(),
            ));
        }
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("authored particle shader"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle compute resources"),
            bind_group_layouts: bindings,
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let spawn = pipeline(&contract.spawn_entry);
        let update = pipeline(&contract.compute_entry);
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            spawn,
            update,
            workgroup_size: contract.workgroup_size,
            max_workgroups: limits.max_compute_workgroups_per_dimension,
        })
    }

    /// Encode spawn before update; zero elapsed-time frames can skip update.
    /// Resource groups must match the layouts supplied during preparation.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        groups: &[(u32, wgpu::BindGroup)],
        capacity: u32,
        spawn: bool,
        update: bool,
    ) -> Result<(), RuntimeError> {
        let count = capacity.div_ceil(self.workgroup_size);
        if count > self.max_workgroups {
            return Err(RuntimeError::ParticleResources(
                "particle dispatch exceeds device limits".into(),
            ));
        }
        if count == 0 || (!spawn && !update) {
            return Ok(());
        }
        for (enabled, pipeline) in [(spawn, &self.spawn), (update, &self.update)] {
            if !enabled {
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("particle simulation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            for (index, group) in groups {
                pass.set_bind_group(*index, group, &[]);
            }
            pass.dispatch_workgroups(count, 1, 1);
        }
        Ok(())
    }
}
