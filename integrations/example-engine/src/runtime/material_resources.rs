//! Material inputs shared by mesh and particle renderers.
use super::{
    RuntimeError,
    resources::{Resource, append_textures},
    surface_parameters::SurfaceParameters,
    textures::{TextureInputs, TextureLimits, validate_bindings},
    uniforms::{UniformLimits, UniformSet},
};
use crate::profile::{FrameInputs, frame_uniform};
use fresco_artifact::ManifestSurface;
use std::collections::BTreeMap;

pub struct MaterialResources {
    pub(crate) resources: BTreeMap<u32, Vec<(u32, Resource)>>,
    uniforms: UniformSet,
    uniform_buffers: Vec<wgpu::Buffer>,
    parameters: SurfaceParameters,
    parameter_buffers: Vec<wgpu::Buffer>,
}

impl MaterialResources {
    pub async fn prepare(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &ManifestSurface,
        occupied: &[(u32, u32)],
        textures: &TextureInputs,
        overrides: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<Self, RuntimeError> {
        let limits = device.limits();
        let uniform_limits = UniformLimits {
            max_bind_groups: limits.max_bind_groups,
            max_bindings_per_bind_group: limits.max_bindings_per_bind_group,
            max_uniform_buffer_binding_size: limits.max_uniform_buffer_binding_size,
        };
        let mut uniforms = UniformSet::new(surface.global_uniforms.clone(), uniform_limits)?;
        uniforms.update(|def, field| {
            frame_uniform(
                def,
                field,
                FrameInputs {
                    time: 0.0,
                    delta_time: 0.0,
                    physical_size: [1, 1],
                },
            )
        })?;
        if uniforms
            .definitions()
            .any(|def| occupied.contains(&(def.group, def.binding)))
        {
            return Err(RuntimeError::UniformLayout {
                uniform: surface.name.clone(),
                reason: "frame and engine bindings collide".into(),
            });
        }
        let mut parameters = SurfaceParameters::new(
            &surface.params,
            uniform_limits,
            uniforms
                .definitions()
                .map(|d| (d.group, d.binding))
                .chain(occupied.iter().copied()),
        )?;
        if let Some(overrides) = overrides {
            parameters.update(overrides)?;
        }
        validate_bindings(
            &surface.textures,
            surface.sampler.as_ref(),
            textures,
            uniforms
                .definitions()
                .map(|d| (d.group, d.binding))
                .chain(occupied.iter().copied())
                .chain(surface.params.iter().map(|d| (d.group, d.binding))),
            TextureLimits {
                max_dimension: limits.max_texture_dimension_2d,
                max_bind_groups: limits.max_bind_groups,
                max_bindings_per_group: limits.max_bindings_per_bind_group,
                max_textures_per_stage: limits.max_sampled_textures_per_shader_stage,
                max_samplers_per_stage: limits.max_samplers_per_shader_stage,
            },
        )?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let buffer = |size, label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let mut resources: BTreeMap<u32, Vec<(u32, Resource)>> = BTreeMap::new();
        let mut uniform_buffers = Vec::new();
        for upload in uniforms.uploads().expect("initialized uniforms") {
            let gpu = buffer(upload.bytes.len() as u64, "frame uniform");
            queue.write_buffer(&gpu, 0, upload.bytes);
            resources
                .entry(upload.group)
                .or_default()
                .push((upload.binding, Resource::Uniform(gpu.clone())));
            uniform_buffers.push(gpu);
        }
        let mut parameter_buffers = Vec::new();
        for upload in parameters.uploads() {
            let gpu = buffer(16, "material parameter");
            queue.write_buffer(&gpu, 0, upload.bytes);
            resources
                .entry(upload.group)
                .or_default()
                .push((upload.binding, Resource::Uniform(gpu.clone())));
            parameter_buffers.push(gpu);
        }
        append_textures(
            device,
            queue,
            &surface.textures,
            surface.sampler.as_ref(),
            textures,
            &mut resources,
        );
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            resources,
            uniforms,
            uniform_buffers,
            parameters,
            parameter_buffers,
        })
    }

    pub fn values(&self) -> serde_json::Map<String, serde_json::Value> {
        self.parameters.values()
    }
    pub fn update_parameters(
        &mut self,
        queue: &wgpu::Queue,
        updates: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        self.parameters.update(updates)?;
        for (buffer, upload) in self.parameter_buffers.iter().zip(self.parameters.uploads()) {
            queue.write_buffer(buffer, 0, upload.bytes);
        }
        Ok(())
    }
    pub fn update_frame(
        &mut self,
        queue: &wgpu::Queue,
        frame: FrameInputs,
    ) -> Result<(), RuntimeError> {
        self.uniforms
            .update(|def, field| frame_uniform(def, field, frame))?;
        for (buffer, upload) in self
            .uniform_buffers
            .iter()
            .zip(self.uniforms.uploads().expect("updated uniforms"))
        {
            queue.write_buffer(buffer, 0, upload.bytes);
        }
        Ok(())
    }
}
