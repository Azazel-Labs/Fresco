//! Intermediate render targets owned by the engine, independent of host surfaces.
use std::collections::BTreeMap;

use super::{RuntimeError, pass_plan::ValidatedPassPlan};

pub struct IntermediateTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl IntermediateTarget {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

/// A complete target set for one plan and viewport. Prepare a replacement before
/// swapping it into a running renderer, including when the viewport resizes.
pub struct IntermediateTargets {
    size: [u32; 2],
    targets: BTreeMap<usize, IntermediateTarget>,
}

impl IntermediateTargets {
    pub async fn prepare(
        device: &wgpu::Device,
        plan: &ValidatedPassPlan,
        size: [u32; 2],
    ) -> Result<Self, RuntimeError> {
        // Validate every descriptor before creating any GPU resources.
        let sizes = plan.target_sizes(size, device.limits().max_texture_dimension_2d)?;
        let descriptors = sizes
            .into_iter()
            .map(|(target, dimensions)| {
                let format = super::recipe::format(&target.format)?;
                let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC;
                let features = format.guaranteed_format_features(device.features());
                if !device.features().contains(format.required_features())
                    || !features.allowed_usages.contains(usage)
                    || !features.flags.contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
                {
                    return Err(RuntimeError::PassPlan(format!(
                        "intermediate format {format:?} requires renderable, filterable color on this device"
                    )));
                }
                Ok((target.id, dimensions, format))
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let allocation_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut targets = BTreeMap::new();
        for (id, dimensions, format) in descriptors {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("example engine intermediate"),
                size: wgpu::Extent3d {
                    width: dimensions[0],
                    height: dimensions[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            targets.insert(id, IntermediateTarget { texture, view });
        }
        // Pop both scopes even when validation fails.
        // Clear the scope stack before awaiting either result, allowing another
        // preparation on the same device to interleave safely.
        let validation_result = validation_scope.pop();
        let allocation_result = allocation_scope.pop();
        let validation = validation_result.await;
        let allocation = allocation_result.await;
        if let Some(error) = validation.or(allocation) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self { size, targets })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    pub fn get(&self, id: usize) -> Option<&IntermediateTarget> {
        self.targets.get(&id)
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}
