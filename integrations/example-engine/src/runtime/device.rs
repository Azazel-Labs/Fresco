//! Device limits and checked attachment accounting shared by the hosts.
use super::RuntimeError;

/// Enable the adapter's color-output capacity; each authored recipe still checks
/// its actual requirements before allocating or creating pipelines.
pub fn requested_limits(supported: &wgpu::Limits) -> wgpu::Limits {
    wgpu::Limits {
        max_color_attachments: supported.max_color_attachments,
        max_color_attachment_bytes_per_sample: supported.max_color_attachment_bytes_per_sample,
        ..Default::default()
    }
}

pub fn validate_color_attachments(
    formats: impl IntoIterator<Item = wgpu::TextureFormat>,
    limit: u32,
) -> Result<(), RuntimeError> {
    let mut total = 0u32;
    for format in formats {
        let cost = format.target_pixel_byte_cost().ok_or_else(|| {
            RuntimeError::PassPlan("format cannot be used as a color attachment".into())
        })?;
        let alignment = format.target_component_alignment().ok_or_else(|| {
            RuntimeError::PassPlan("color attachment has no component alignment".into())
        })?;
        total = total
            .checked_next_multiple_of(alignment)
            .and_then(|n| n.checked_add(cost))
            .ok_or_else(|| RuntimeError::PassPlan("color attachment byte count overflow".into()))?;
    }
    if total > limit {
        return Err(RuntimeError::PassPlan(format!(
            "color attachments require {total} bytes per sample, device permits {limit}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn color_budget_uses_format_costs_and_rejects_before_pipeline_creation() {
        use wgpu::TextureFormat as F;
        let formats = [
            F::Rgba8UnormSrgb,
            F::Rgba16Float,
            F::Rgba16Float,
            F::Rg32Uint,
            F::Rg32Float,
        ];
        assert!(super::validate_color_attachments(formats, 40).is_ok());
        assert!(super::validate_color_attachments(formats, 39).is_err());
        assert!(super::validate_color_attachments([F::Depth32Float], 40).is_err());
    }
}
