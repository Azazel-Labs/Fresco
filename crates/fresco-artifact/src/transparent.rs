//! Shared structural rules for globally sorted transparent draw queues.
use crate::ManifestRecipeStep;
use std::collections::BTreeSet;

impl crate::ManifestRasterState {
    pub fn validate_transparent_queue(&self) -> Result<(), String> {
        if self.blend.as_deref() != Some("premultiplied")
            || self.depth_write != Some(false)
            || !matches!(self.depth_compare.as_deref(), Some("less" | "less_equal"))
        {
            return Err("transparent queues require premultiplied blending and read-only less/less_equal depth testing".into());
        }
        Ok(())
    }
}

pub fn validate_transparent_queues(steps: &[ManifestRecipeStep]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let mut start = 0;
    while start < steps.len() {
        let Some(queue) = &steps[start].transparent_queue else {
            start += 1;
            continue;
        };
        if queue.is_empty() || !seen.insert(queue) {
            return Err("transparent queue must form one named contiguous boundary".into());
        }
        let end = steps[start..]
            .iter()
            .position(|step| step.transparent_queue.as_ref() != Some(queue))
            .map_or(steps.len(), |offset| start + offset);
        let first = &steps[start];
        let members: BTreeSet<_> = steps[start..end].iter().map(|step| &step.name).collect();
        for step in &steps[start..end] {
            if !step.is_draw_scoped()
                || step.colors.is_empty()
                || step.depth.is_none()
                || step.colors != first.colors
                || step.depth != first.depth
            {
                return Err("transparent queue requires draw-scoped raster nodes with identical color and depth attachments".into());
            }
            if step.colors.values().chain(step.depth.iter()).any(|name| {
                !step
                    .attachments
                    .get(name)
                    .is_some_and(|ops| ops.load && ops.store)
            }) {
                return Err("transparent queue attachments must preserve color and depth".into());
            }
            if step.after.iter().any(|name| members.contains(name)) {
                return Err(
                    "transparent queue members cannot impose internal node ordering".into(),
                );
            }
        }
        start = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn transparent_state_requires_premultiplied_color_and_read_only_depth() {
        let valid = crate::ManifestRasterState {
            blend: Some("premultiplied".into()),
            depth_write: Some(false),
            depth_compare: Some("less_equal".into()),
            ..Default::default()
        };
        valid.validate_transparent_queue().unwrap();
        for state in [
            crate::ManifestRasterState {
                blend: Some("alpha".into()),
                ..valid.clone()
            },
            crate::ManifestRasterState {
                blend: None,
                ..valid.clone()
            },
            crate::ManifestRasterState {
                depth_write: Some(true),
                ..valid.clone()
            },
            crate::ManifestRasterState {
                depth_write: None,
                ..valid.clone()
            },
            crate::ManifestRasterState {
                depth_compare: Some("always".into()),
                ..valid
            },
        ] {
            assert!(state.validate_transparent_queue().is_err());
        }
    }
}
