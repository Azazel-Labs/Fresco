//! Authored particle raster stages; particles supply instance data, not vertices.
use super::{RuntimeError, mesh::DEPTH_FORMAT};
use crate::runtime::particle_contract::ParticleContract;
use fresco_artifact::ManifestSurfaceSettings;

pub struct ParticleDraw {
    pipeline: wgpu::RenderPipeline,
    vertices: u32,
}

impl ParticleDraw {
    pub(crate) fn pipeline(&self) -> wgpu::RenderPipeline {
        self.pipeline.clone()
    }

    pub async fn prepare(
        device: &wgpu::Device,
        wgsl: &str,
        contract: &ParticleContract,
        settings: &ManifestSurfaceSettings,
        bindings: &[Option<&wgpu::BindGroupLayout>],
        format: wgpu::TextureFormat,
    ) -> Result<Self, RuntimeError> {
        if contract.vertex_count == 0
            || contract.vertex_entry.is_empty()
            || contract.fragment_entry.is_empty()
        {
            return Err(RuntimeError::ParticleResources(
                "particle raster entries and vertex count are required".into(),
            ));
        }
        let state = super::raster_state::resolve(settings, &contract.draw_pass)?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("authored particle raster shader"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle raster resources"),
            bind_group_layouts: bindings,
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("authored particle raster"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(&contract.vertex_entry),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(&contract.fragment_entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: state.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: state.cull,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(state.depth_write),
                depth_compare: Some(state.depth_compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            pipeline,
            vertices: contract.vertex_count,
        })
    }

    /// Color/depth attachment ownership and submission belong to the renderer.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        groups: &[(u32, wgpu::BindGroup)],
        capacity: u32,
    ) {
        if capacity == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        for (index, group) in groups {
            pass.set_bind_group(*index, group, &[]);
        }
        pass.draw(0..self.vertices, 0..capacity);
    }
}
