//! Resolve particle resource roles from the manifest into stage-specific bindings.
use super::{
    RuntimeError, material_resources::MaterialResources, particle_buffers::ParticleBuffers,
    resources::Resource,
};
use crate::runtime::particle_contract::ParticleContract;

use std::collections::{BTreeMap, BTreeSet};

pub struct ParticleStageBindings {
    layouts: BTreeMap<u32, wgpu::BindGroupLayout>,
    groups: Vec<(u32, wgpu::BindGroup)>,
}

impl ParticleStageBindings {
    /// Merge these layouts with material/global layouts before creating pipelines.
    pub fn layouts(&self) -> &BTreeMap<u32, wgpu::BindGroupLayout> {
        &self.layouts
    }
    pub fn groups(&self) -> &[(u32, wgpu::BindGroup)] {
        &self.groups
    }
}

pub struct ParticleBindings {
    pub compute: ParticleStageBindings,
    pub draw: ParticleStageBindings,
}

struct Binding<'a> {
    group: u32,
    binding: u32,
    buffer: &'a wgpu::Buffer,
    ty: wgpu::BufferBindingType,
}

impl ParticleBindings {
    pub async fn prepare(
        device: &wgpu::Device,
        contract: &ParticleContract,
        buffers: &ParticleBuffers,
        scene: &wgpu::Buffer,
    ) -> Result<Self, RuntimeError> {
        Self::prepare_resources(device, contract, buffers, scene, &BTreeMap::new()).await
    }

    pub async fn prepare_with_material(
        device: &wgpu::Device,
        contract: &ParticleContract,
        buffers: &ParticleBuffers,
        scene: &wgpu::Buffer,
        material: &MaterialResources,
    ) -> Result<Self, RuntimeError> {
        Self::prepare_resources(device, contract, buffers, scene, &material.resources).await
    }

    pub(crate) async fn prepare_resources(
        device: &wgpu::Device,
        contract: &ParticleContract,
        buffers: &ParticleBuffers,
        scene: &wgpu::Buffer,
        extras: &BTreeMap<u32, Vec<(u32, Resource)>>,
    ) -> Result<Self, RuntimeError> {
        let invalid = |reason: &str| RuntimeError::ParticleResources(reason.into());
        let mut names = BTreeSet::new();
        let mut compute = Vec::new();
        let mut draw = Vec::new();
        for binding in &contract.bindings {
            if !names.insert(binding.name.as_str()) {
                return Err(invalid("duplicate particle resource role"));
            }
            let (buffer, resource, access, compute_stage, draw_stage) = match binding.name.as_str()
            {
                "scene" => (scene, "uniform", "read", false, true),
                "particles" => (buffers.state(), "storage", "read_write", true, false),
                "particle_render" => (buffers.state(), "storage", "read", false, true),
                "particle_config" => (buffers.config(), "uniform", "read", true, false),
                "particle_slots" => (
                    buffers
                        .slots()
                        .ok_or_else(|| invalid("slot binding requires managed storage"))?,
                    "storage",
                    "read",
                    true,
                    true,
                ),
                _ => return Err(invalid("unsupported particle resource role")),
            };
            if binding.resource != resource || binding.access != access {
                return Err(invalid(
                    "particle resource type or access does not match its role",
                ));
            }
            let ty = if resource == "uniform" {
                wgpu::BufferBindingType::Uniform
            } else {
                wgpu::BufferBindingType::Storage {
                    read_only: access == "read",
                }
            };
            let make = || Binding {
                group: binding.group,
                binding: binding.binding,
                buffer,
                ty,
            };
            if compute_stage {
                compute.push(make());
            }
            if draw_stage {
                draw.push(make());
            }
        }
        for required in ["scene", "particles", "particle_render", "particle_config"] {
            if !names.contains(required) {
                return Err(invalid("required particle resource role is missing"));
            }
        }
        if buffers.slots().is_some() != names.contains("particle_slots") {
            return Err(invalid("managed storage and slot binding disagree"));
        }
        for stage in [&compute, &draw] {
            let mut occupied = BTreeSet::new();
            for (group, resources) in extras {
                for (binding, _) in resources {
                    if !occupied.insert((*group, *binding)) {
                        return Err(invalid("duplicate material binding"));
                    }
                }
            }
            for binding in stage {
                if !occupied.insert((binding.group, binding.binding)) {
                    return Err(invalid("particle stage bindings collide"));
                }
            }
        }
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let compute = build(device, &compute, extras, wgpu::ShaderStages::COMPUTE);
        let draw = build(device, &draw, extras, wgpu::ShaderStages::VERTEX_FRAGMENT);
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self { compute, draw })
    }
}

fn build(
    device: &wgpu::Device,
    bindings: &[Binding<'_>],
    extras: &BTreeMap<u32, Vec<(u32, Resource)>>,
    visibility: wgpu::ShaderStages,
) -> ParticleStageBindings {
    let mut by_group: BTreeMap<u32, Vec<&Binding<'_>>> = BTreeMap::new();
    for binding in bindings {
        by_group.entry(binding.group).or_default().push(binding);
    }
    for group in extras.keys() {
        by_group.entry(*group).or_default();
    }
    let mut layouts = BTreeMap::new();
    let mut groups = Vec::new();
    for (index, bindings) in by_group {
        let mut entries: Vec<_> = bindings
            .iter()
            .map(|b| wgpu::BindGroupLayoutEntry {
                binding: b.binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: b.ty,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(b.buffer.size()),
                },
                count: None,
            })
            .collect();
        if let Some(resources) = extras.get(&index) {
            entries.extend(resources.iter().map(|(binding, resource)| {
                wgpu::BindGroupLayoutEntry {
                    binding: *binding,
                    visibility,
                    ty: resource.binding_type(),
                    count: None,
                }
            }));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle resources"),
            entries: &entries,
        });
        let mut entries: Vec<_> = bindings
            .iter()
            .map(|b| wgpu::BindGroupEntry {
                binding: b.binding,
                resource: b.buffer.as_entire_binding(),
            })
            .collect();
        if let Some(resources) = extras.get(&index) {
            entries.extend(
                resources
                    .iter()
                    .map(|(binding, resource)| wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: resource.binding(),
                    }),
            );
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle resources"),
            layout: &layout,
            entries: &entries,
        });
        layouts.insert(index, layout);
        groups.push((index, group));
    }
    ParticleStageBindings { layouts, groups }
}
