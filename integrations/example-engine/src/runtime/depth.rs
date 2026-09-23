//! Owned depth targets, prepared before a host replaces its installed target.
use super::{RuntimeError, mesh::DEPTH_FORMAT};

pub struct DepthTarget {
    size: [u32; 2],
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl DepthTarget {
    /// Zero dimensions suspend rendering without allocating a GPU texture.
    /// Keep the old target until this future returns a successful replacement.
    pub async fn prepare(
        device: &wgpu::Device,
        size: [u32; 2],
    ) -> Result<Option<Self>, RuntimeError> {
        let limit = device.limits().max_texture_dimension_2d;
        if size.iter().any(|dimension| *dimension > limit) {
            return Err(RuntimeError::DepthTarget(format!(
                "dimensions exceed device limit {limit}"
            )));
        }
        if size.contains(&0) {
            return Ok(None);
        }
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh depth"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        // Pop both scopes before yielding: WebGPU error scopes are a device stack,
        // not task-local storage, and other preparations can overlap this future.
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Some(Self {
            size,
            texture,
            view,
        }))
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}
