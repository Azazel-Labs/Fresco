//! Shared GPU execution. Hosts prepare resources/pipelines; the graph owns order
//! and invocation counts. Provider, material, and lifecycle policy stay outside.
use super::{RuntimeError, mesh_geometry::MeshGeometry};
use fresco_artifact::{
    ManifestDispatchExtent, ManifestDrawCount, ManifestTechnique, ManifestTechniqueOperation,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub enum Pipeline {
    Compute(wgpu::ComputePipeline),
    Draw(wgpu::RenderPipeline),
}

#[derive(Default)]
pub struct Parameters {
    pub extents: BTreeMap<String, [u32; 3]>,
    pub counts: BTreeMap<String, u32>,
    pub enabled: BTreeMap<String, bool>,
}

/// Prepared pipelines keyed by graph node, never by lifecycle role.
pub struct Executor {
    pub(crate) graph: ManifestTechnique,
    pipelines: BTreeMap<String, Pipeline>,
    max_workgroups: u32,
}

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::PassPlan(message.into())
}

impl Executor {
    pub fn new(
        mut graph: ManifestTechnique,
        pipelines: BTreeMap<String, Pipeline>,
        limits: &wgpu::Limits,
    ) -> Result<Self, RuntimeError> {
        let mut names = BTreeSet::new();
        for step in &graph.steps {
            if !names.insert(step.name.clone()) {
                return Err(invalid("duplicate technique node"));
            }
            match (&step.operation, pipelines.get(&step.name)) {
                (
                    ManifestTechniqueOperation::Compute { workgroup_size, .. },
                    Some(Pipeline::Compute(_)),
                ) => {
                    if workgroup_size.contains(&0)
                        || workgroup_size[0] > limits.max_compute_workgroup_size_x
                        || workgroup_size[1] > limits.max_compute_workgroup_size_y
                        || workgroup_size[2] > limits.max_compute_workgroup_size_z
                        || workgroup_size
                            .iter()
                            .try_fold(1u32, |n, v| n.checked_mul(*v))
                            .is_none_or(|n| n > limits.max_compute_invocations_per_workgroup)
                    {
                        return Err(invalid("technique workgroup exceeds device limits"));
                    }
                }
                (
                    ManifestTechniqueOperation::Draw {
                        vertices,
                        fragment,
                        colors,
                        depth,
                        ..
                    },
                    Some(Pipeline::Draw(_)),
                ) => {
                    if *vertices == 0
                        || (fragment.is_none() && (!colors.is_empty() || depth.is_none()))
                    {
                        return Err(invalid("invalid technique draw"));
                    }
                    if colors
                        .keys()
                        .any(|slot| *slot >= limits.max_color_attachments)
                    {
                        return Err(invalid("technique color attachment exceeds device limits"));
                    }
                }
                _ => {
                    return Err(invalid(format!(
                        "missing or incompatible pipeline for `{}`",
                        step.name
                    )));
                }
            }
        }
        if names.len() != pipelines.len() {
            return Err(invalid("unexpected technique pipeline"));
        }
        let mut ordered = Vec::new();
        let mut done = BTreeSet::new();
        while ordered.len() < graph.steps.len() {
            let step = graph
                .steps
                .iter()
                .find(|s| !done.contains(&s.name) && s.after.iter().all(|p| done.contains(p)))
                .ok_or_else(|| invalid("technique dependency cycle or missing predecessor"))?;
            done.insert(step.name.clone());
            ordered.push(step.clone());
        }
        graph.steps = ordered;
        Ok(Self {
            graph,
            pipelines,
            max_workgroups: limits.max_compute_workgroups_per_dimension,
        })
    }

    /// Validate the entire invocation before recording commands. Skipped nodes
    /// retain their ordering position; compiler validation requires initialized
    /// external resources for conditional producers.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        invocation: Invocation<'_>,
    ) -> Result<(), RuntimeError> {
        self.encode_over(encoder, invocation, BTreeSet::new())
    }

    /// Compose over initialized attachments. Unspecified first-use operations load
    /// their existing contents; explicit authored operations still take precedence.
    pub fn encode_over(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        invocation: Invocation<'_>,
        initialized: BTreeSet<String>,
    ) -> Result<(), RuntimeError> {
        if initialized
            .iter()
            .any(|name| !invocation.attachments.contains_key(name))
        {
            return Err(invalid("initialized attachment is not bound"));
        }
        let mut inherited = initialized.clone();
        let mut resolved = Vec::new();
        for step in &self.graph.steps {
            let enabled = match &step.enabled {
                Some(name) => *invocation
                    .parameters
                    .enabled
                    .get(name)
                    .ok_or_else(|| invalid(format!("missing activation `{name}`")))?,
                None => true,
            };
            if !enabled {
                continue;
            }
            let groups = invocation
                .groups
                .get(&step.name)
                .ok_or_else(|| invalid(format!("missing resource groups for `{}`", step.name)))?;
            let count = match &step.operation {
                ManifestTechniqueOperation::Compute {
                    extent,
                    workgroup_size,
                    ..
                } => {
                    let extent = match extent {
                        ManifestDispatchExtent::Fixed(value) => value,
                        ManifestDispatchExtent::Parameter { parameter } => invocation
                            .parameters
                            .extents
                            .get(parameter)
                            .ok_or_else(|| invalid(format!("missing extent `{parameter}`")))?,
                    };
                    let count =
                        std::array::from_fn(|axis| extent[axis].div_ceil(workgroup_size[axis]));
                    if count.iter().any(|n| *n > self.max_workgroups) {
                        return Err(invalid("technique dispatch exceeds device limits"));
                    }
                    count
                }
                ManifestTechniqueOperation::Draw {
                    instances,
                    colors,
                    depth,
                    ..
                } => {
                    for name in colors.values().chain(depth.iter()) {
                        if !invocation.attachments.contains_key(name) {
                            return Err(invalid(format!("missing attachment `{name}`")));
                        }
                    }
                    let count = match instances {
                        ManifestDrawCount::Fixed(value) => *value,
                        ManifestDrawCount::Parameter { parameter } => *invocation
                            .parameters
                            .counts
                            .get(parameter)
                            .ok_or_else(|| invalid(format!("missing count `{parameter}`")))?,
                    };
                    if invocation.geometry.contains_key(&step.name) && count != 1 {
                        return Err(invalid("geometry adapter requires one instance"));
                    }
                    [count, 1, 1]
                }
            };
            resolved.push((step, groups, count));
        }
        // Preflight load/store across the active invocation before recording anything.
        let mut initialized = initialized;
        for (step, _, count) in &resolved {
            if count.contains(&0) {
                continue;
            }
            if let ManifestTechniqueOperation::Draw { colors, depth, .. } = &step.operation {
                for name in step.attachments.keys() {
                    if !colors.values().chain(depth.iter()).any(|a| a == name) {
                        return Err(invalid("operations reference a non-attachment"));
                    }
                }
                for name in colors.values().chain(depth.iter()) {
                    let ops = step.attachments.get(name);
                    if ops.is_some_and(|ops| ops.load) && !initialized.contains(name) {
                        return Err(invalid(format!(
                            "attachment `{name}` loads undefined contents"
                        )));
                    }
                    if ops.is_none_or(|ops| ops.store) {
                        initialized.insert(name.clone());
                    } else {
                        initialized.remove(name);
                    }
                }
            } else if !step.attachments.is_empty() {
                return Err(invalid(
                    "compute steps cannot declare attachment operations",
                ));
            }
        }
        for (step, groups, count) in resolved {
            if count.contains(&0) {
                continue;
            }
            match (&step.operation, &self.pipelines[&step.name]) {
                (ManifestTechniqueOperation::Compute { .. }, Pipeline::Compute(pipeline)) => {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some(&step.name),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(pipeline);
                    for (index, group) in *groups {
                        pass.set_bind_group(*index, group, &[]);
                    }
                    pass.dispatch_workgroups(count[0], count[1], count[2]);
                }
                (
                    ManifestTechniqueOperation::Draw {
                        vertices,
                        colors,
                        depth,
                        ..
                    },
                    Pipeline::Draw(pipeline),
                ) => {
                    let mut attachments = Vec::new();
                    for (location, name) in colors {
                        attachments.resize_with(*location as usize + 1, || None);
                        attachments[*location as usize] = Some(wgpu::RenderPassColorAttachment {
                            view: invocation.attachments[name],
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: if step
                                    .attachments
                                    .get(name)
                                    .map_or_else(|| inherited.contains(name), |ops| ops.load)
                                {
                                    wgpu::LoadOp::Load
                                } else {
                                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                                },
                                store: if step.attachments.get(name).is_none_or(|ops| ops.store) {
                                    wgpu::StoreOp::Store
                                } else {
                                    wgpu::StoreOp::Discard
                                },
                            },
                        });
                    }
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some(&step.name),
                        color_attachments: &attachments,
                        depth_stencil_attachment: depth.as_ref().map(|name| {
                            wgpu::RenderPassDepthStencilAttachment {
                                view: invocation.attachments[name],
                                depth_ops: Some(wgpu::Operations {
                                    load: if step
                                        .attachments
                                        .get(name)
                                        .map_or_else(|| inherited.contains(name), |ops| ops.load)
                                    {
                                        wgpu::LoadOp::Load
                                    } else {
                                        wgpu::LoadOp::Clear(1.0)
                                    },
                                    store: if step.attachments.get(name).is_none_or(|ops| ops.store)
                                    {
                                        wgpu::StoreOp::Store
                                    } else {
                                        wgpu::StoreOp::Discard
                                    },
                                }),
                                stencil_ops: None,
                            }
                        }),
                        ..Default::default()
                    });
                    for name in colors.values().chain(depth.iter()) {
                        inherited.remove(name);
                    }
                    pass.set_pipeline(pipeline);
                    for (index, group) in *groups {
                        pass.set_bind_group(*index, group, &[]);
                    }
                    if let Some(geometry) = invocation.geometry.get(&step.name) {
                        geometry.draw(&mut pass);
                    } else {
                        pass.draw(0..*vertices, 0..count[0]);
                    }
                }
                _ => unreachable!("pipeline kinds validated during preparation"),
            }
        }
        Ok(())
    }
}

/// Resource handles are resolved by the host before execution. Bind groups may
/// include engine-specific material resources beyond the explicit shader inputs.
pub struct Invocation<'a> {
    pub parameters: &'a Parameters,
    pub groups: BTreeMap<String, &'a [(u32, wgpu::BindGroup)]>,
    pub attachments: BTreeMap<String, &'a wgpu::TextureView>,
    pub geometry: BTreeMap<String, &'a MeshGeometry>,
}
