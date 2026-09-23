//! One-shot GPU execution of a resolved authored compute invocation.
//!
//! Preparation always creates fresh outputs. Command buffers and returned handles
//! retain the resources; dropping the invocation never recycles an in-flight
//! allocation. The frame scheduler owns dependency ordering and input resolution.
use std::collections::BTreeMap;

use fresco_artifact::{ManifestComputeBindingSource, ManifestGpuProgram};
use wgpu::util::DeviceExt;

use super::{
    RuntimeError,
    compute_plan::{ComputeInvocationPlan, OwnedAllocation},
};

/// An input already resolved from invocation provenance by the frame scheduler.
#[derive(Clone)]
pub enum ComputeInput {
    Buffer(wgpu::Buffer),
    Image(wgpu::TextureView),
}

impl ComputeInput {
    fn binding(&self) -> wgpu::BindingResource<'_> {
        match self {
            Self::Buffer(buffer) => buffer.as_entire_binding(),
            Self::Image(view) => wgpu::BindingResource::TextureView(view),
        }
    }
}

/// Resource and logical allocation metadata travel together. Physical sentinel
/// allocations must not become authored logical dimensions in later operations.
#[derive(Clone)]
pub struct ComputeOutput {
    allocation: OwnedAllocation,
    input: ComputeInput,
}

impl ComputeOutput {
    pub fn allocation(&self) -> OwnedAllocation {
        self.allocation
    }
    pub fn input(&self) -> ComputeInput {
        self.input.clone()
    }
}

pub struct ComputeKernel {
    program: ManifestGpuProgram,
    pipeline: wgpu::ComputePipeline,
}

pub struct OwnedCompute {
    pipeline: wgpu::ComputePipeline,
    groups: Vec<wgpu::BindGroup>,
    dispatch: [u32; 3],
    output: ComputeOutput,
}

fn invalid(reason: impl Into<String>) -> RuntimeError {
    RuntimeError::PassPlan(format!("owned compute: {}", reason.into()))
}

impl OwnedCompute {
    /// A query-only consumer may bind an allocation before its producer executes.
    /// Reading its data still requires the producer's GPU dependency to complete.
    pub fn output(&self) -> &ComputeOutput {
        &self.output
    }

    /// Compile and prepare an isolated invocation, capturing GPU preparation errors.
    pub async fn prepare(
        device: &wgpu::Device,
        wgsl: &str,
        program: &ManifestGpuProgram,
        plan: ComputeInvocationPlan,
        inputs: BTreeMap<String, ComputeInput>,
    ) -> Result<Self, RuntimeError> {
        let kernel = ComputeKernel::prepare(device, wgsl, program).await?;
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = kernel.instantiate(device, plan, inputs);
        let validation = validation.pop();
        let allocation = allocation.pop();
        let validation = validation.await;
        let allocation = allocation.await;
        if let Some(error) = validation.or(allocation) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        result
    }
}

impl ComputeKernel {
    pub async fn prepare(
        device: &wgpu::Device,
        wgsl: &str,
        program: &ManifestGpuProgram,
    ) -> Result<Self, RuntimeError> {
        let [entry] = program.entries.as_slice() else {
            return Err(invalid("expected exactly one compute entry"));
        };
        program
            .compute_shader_reads(&entry.entry)
            .map_err(invalid)?;
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("authored owned compute"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(&entry.entry),
            layout: None,
            module: &module,
            entry_point: Some(&entry.entry),
            compilation_options: Default::default(),
            cache: None,
        });
        let validation = validation.pop();
        let allocation = allocation.pop();
        let validation = validation.await;
        let allocation = allocation.await;
        if let Some(error) = validation.or(allocation) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            program: program.clone(),
            pipeline,
        })
    }

    /// Inputs and the checked plan must describe the same immutable invocation.
    /// The pipeline is reused; output resources and groups are always fresh.
    pub fn instantiate(
        &self,
        device: &wgpu::Device,
        plan: ComputeInvocationPlan,
        mut inputs: BTreeMap<String, ComputeInput>,
    ) -> Result<OwnedCompute, RuntimeError> {
        let program = &self.program;
        let [entry] = program.entries.as_slice() else {
            return Err(invalid("expected exactly one compute entry"));
        };
        program
            .compute_shader_reads(&entry.entry)
            .map_err(invalid)?;
        let mut required = BTreeMap::new();
        for binding in &program.bindings {
            if !binding.entry_access.contains_key(&entry.entry) {
                continue;
            }
            let source = &program.compute_bindings[&binding.name];
            if !matches!(
                source,
                ManifestComputeBindingSource::Output | ManifestComputeBindingSource::Dispatch
            ) {
                let input = inputs
                    .remove(&binding.name)
                    .ok_or_else(|| invalid(format!("missing input `{}`", binding.name)))?;
                let compatible = matches!(
                    (&input, binding.kind.as_str()),
                    (ComputeInput::Buffer(_), "uniform" | "storage")
                        | (ComputeInput::Image(_), "texture" | "storage_texture")
                );
                if !compatible {
                    return Err(invalid(format!(
                        "input kind disagrees with `{}`",
                        binding.name
                    )));
                }
                required.insert(binding.name.clone(), input);
            }
        }
        if let Some(name) = inputs.keys().next() {
            return Err(invalid(format!(
                "unexpected or engine-owned input `{name}`"
            )));
        }
        let max_group = program
            .bindings
            .iter()
            .filter(|b| b.entry_access.contains_key(&entry.entry))
            .map(|b| b.group)
            .max()
            .unwrap_or(0);
        if max_group >= device.limits().max_bind_groups {
            return Err(invalid("bind group index exceeds device limits"));
        }
        // Check capabilities before touching GPU allocation. Type registry
        // membership alone does not imply support on this enabled device.
        let image_format = if let OwnedAllocation::Image { format, .. } = plan.output {
            let format = super::recipe::format(format.info().name)?;
            let usage = wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC;
            let features = format.guaranteed_format_features(device.features());
            if !device.features().contains(format.required_features())
                || !features.allowed_usages.contains(usage)
            {
                return Err(invalid(format!(
                    "image format {format:?} lacks required device features or usages"
                )));
            }
            Some(format)
        } else {
            None
        };
        let output = match plan.output {
            OwnedAllocation::Buffer(allocation) => {
                ComputeInput::Buffer(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("owned compute output"),
                    size: allocation.bytes(),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }))
            }
            OwnedAllocation::Image { allocation, .. } => {
                let [width, height] = allocation.physical();
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("owned compute output"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: image_format.expect("validated image format"),
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                ComputeInput::Image(texture.create_view(&Default::default()))
            }
        };
        let mut dispatch_bytes = [0u8; 16];
        for (bytes, value) in dispatch_bytes
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(plan.dispatch.threads())
        {
            bytes.copy_from_slice(&value.to_le_bytes());
        }
        let dispatch = ComputeInput::Buffer(device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("owned compute logical dispatch"),
                contents: &dispatch_bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        ));
        for binding in &program.bindings {
            if !binding.entry_access.contains_key(&entry.entry) {
                continue;
            }
            match program.compute_bindings[&binding.name] {
                ManifestComputeBindingSource::Output => {
                    required.insert(binding.name.clone(), output.clone());
                }
                ManifestComputeBindingSource::Dispatch => {
                    required.insert(binding.name.clone(), dispatch.clone());
                }
                ManifestComputeBindingSource::Value { .. }
                | ManifestComputeBindingSource::Resource { .. }
                | ManifestComputeBindingSource::Geometry { .. }
                | ManifestComputeBindingSource::Dimension { .. } => {
                    // These authored input roles were resolved and checked above.
                }
            }
        }
        let groups = (0..=max_group)
            .map(|group| {
                let layout = self.pipeline.get_bind_group_layout(group);
                let entries: Vec<_> = program
                    .bindings
                    .iter()
                    .filter(|b| b.group == group && b.entry_access.contains_key(&entry.entry))
                    .map(|b| wgpu::BindGroupEntry {
                        binding: b.binding,
                        resource: required[&b.name].binding(),
                    })
                    .collect();
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("owned compute inputs"),
                    layout: &layout,
                    entries: &entries,
                })
            })
            .collect();
        Ok(OwnedCompute {
            pipeline: self.pipeline.clone(),
            groups,
            dispatch: plan.dispatch.groups(),
            output: ComputeOutput {
                allocation: plan.output,
                input: output,
            },
        })
    }
}

impl OwnedCompute {
    pub(crate) fn into_parts(
        self,
    ) -> (
        ComputeOutput,
        wgpu::ComputePipeline,
        Vec<(u32, wgpu::BindGroup)>,
    ) {
        let groups = self
            .groups
            .into_iter()
            .enumerate()
            .map(|(i, group)| (u32::try_from(i).expect("validated group index"), group))
            .collect();
        (self.output, self.pipeline, groups)
    }

    /// Consuming the invocation prevents accidental repeat writes to an output
    /// already published to another operation. Separate passes establish resource
    /// transitions when the caller encodes producer before consumer.
    pub fn encode(self, encoder: &mut wgpu::CommandEncoder) -> ComputeOutput {
        if !self.dispatch.contains(&0) {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("owned compute invocation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (index, group) in self.groups.iter().enumerate() {
                pass.set_bind_group(
                    u32::try_from(index).expect("validated bind group index"),
                    group,
                    &[],
                );
            }
            pass.dispatch_workgroups(self.dispatch[0], self.dispatch[1], self.dispatch[2]);
        }
        self.output
    }
}
