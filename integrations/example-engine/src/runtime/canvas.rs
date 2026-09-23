//! First executable runtime path: an authored fullscreen pass, without a window or DOM.
use std::collections::BTreeMap;
use std::sync::Arc;

use fresco_artifact::ManifestRoot;

use super::RuntimeError;
use super::parameters::{CanvasParameters, INSTANCE_BYTES, PARAMETER_OFFSET};
use super::paths::PathBuffers;
use super::resources::{Resource, append_textures};
use super::storage::StorageLimits;
use super::textures::{TextureInputs, TextureLimits, validate_bindings};
use super::uniforms::{UniformLimits, UniformSet};
use crate::profile::{FrameInputs, frame_uniform};

// Current compiler fullscreen instance ABI: time + padding, resolution + padding,
// then 16 vec4 parameter slots, packed by ParameterSet.
const INSTANCE_SIZE: u64 = INSTANCE_BYTES as u64;

pub struct CanvasInputs<'a> {
    pub textures: &'a TextureInputs,
    pub variant: Option<&'a super::canvas_variant::VariantSelection>,
}

pub struct CanvasRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    groups: Vec<(u32, wgpu::BindGroup)>,
    uniform_buffers: Vec<wgpu::Buffer>,
    uniforms: UniformSet,
    instance: wgpu::Buffer,
    vertex_count: u32,
    parameters: CanvasParameters,
    resources: BTreeMap<u32, Vec<(u32, Resource)>>,
    layouts: BTreeMap<u32, wgpu::BindGroupLayout>,
    parameter_revision: Arc<()>,
}

/// Owns everything needed to prepare an update without borrowing the renderer.
/// Frames can continue using the old resources while GPU validation is pending.
pub struct PendingParameterUpdate {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layouts: BTreeMap<u32, wgpu::BindGroupLayout>,
    prepared: PreparedParameterUpdate,
    storage_changed: bool,
}

pub struct PreparedParameterUpdate {
    parameters: CanvasParameters,
    resources: BTreeMap<u32, Vec<(u32, Resource)>>,
    groups: Vec<(u32, wgpu::BindGroup)>,
    revision: Arc<()>,
}

impl PendingParameterUpdate {
    pub async fn prepare(mut self) -> Result<PreparedParameterUpdate, RuntimeError> {
        if self.storage_changed {
            let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let allocation = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            for upload in self.prepared.parameters.storage.uploads() {
                let resource = self
                    .prepared
                    .resources
                    .get_mut(&upload.definition.group)
                    .and_then(|bindings| {
                        bindings
                            .iter_mut()
                            .find(|(binding, _)| *binding == upload.definition.binding)
                    })
                    .expect("validated storage resource");
                let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&upload.definition.name),
                    size: u64::try_from(upload.bytes.len()).expect("validated storage size"),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.queue.write_buffer(&buffer, 0, upload.bytes);
                resource.1 = Resource::Storage {
                    buffer,
                    min_size: upload.stride as u64,
                };
            }
            self.prepared.groups = self
                .prepared
                .resources
                .iter()
                .map(|(group, bindings)| {
                    let entries: Vec<_> = bindings
                        .iter()
                        .map(|(binding, resource)| wgpu::BindGroupEntry {
                            binding: *binding,
                            resource: resource.binding(),
                        })
                        .collect();
                    (
                        *group,
                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("updated canvas inputs"),
                            layout: &self.layouts[group],
                            entries: &entries,
                        }),
                    )
                })
                .collect();
            // Pop the entire stack synchronously before yielding. Another
            // preparation may push scopes on this same device during the await.
            let allocation_result = allocation.pop();
            let validation_result = validation.pop();
            let allocation_error = allocation_result.await;
            let validation_error = validation_result.await;
            if let Some(error) = validation_error.or(allocation_error) {
                return Err(RuntimeError::GpuValidation(error.to_string()));
            }
        }
        Ok(self.prepared)
    }
}

impl CanvasRenderer {
    /// Build a candidate without mutating an installed renderer. The host replaces
    /// its current renderer only after this future succeeds.
    pub async fn prepare(
        device: wgpu::Device,
        queue: wgpu::Queue,
        wgsl: &str,
        manifest: &ManifestRoot,
        entry: &str,
        output_format: wgpu::TextureFormat,
    ) -> Result<Self, RuntimeError> {
        Self::prepare_with_textures(
            device,
            queue,
            wgsl,
            manifest,
            entry,
            output_format,
            &TextureInputs::new(),
        )
        .await
    }

    pub async fn prepare_with_textures(
        device: wgpu::Device,
        queue: wgpu::Queue,
        wgsl: &str,
        manifest: &ManifestRoot,
        entry: &str,
        output_format: wgpu::TextureFormat,
        textures: &TextureInputs,
    ) -> Result<Self, RuntimeError> {
        Self::prepare_with_inputs(
            device,
            queue,
            wgsl,
            manifest,
            entry,
            output_format,
            CanvasInputs {
                textures,
                variant: None,
            },
        )
        .await
    }

    pub async fn prepare_with_inputs(
        device: wgpu::Device,
        queue: wgpu::Queue,
        wgsl: &str,
        manifest: &ManifestRoot,
        entry: &str,
        output_format: wgpu::TextureFormat,
        inputs: CanvasInputs<'_>,
    ) -> Result<Self, RuntimeError> {
        super::validate_artifact(manifest)?;
        let textures = inputs.textures;
        let invalid = |reason: &str| RuntimeError::CanvasContract {
            entry: entry.into(),
            reason: reason.into(),
        };
        let mut selected = manifest
            .canvases
            .iter()
            .filter(|canvas| canvas.name == entry);
        let canvas = selected
            .next()
            .ok_or_else(|| invalid("entry does not exist"))?;
        if selected.next().is_some() {
            return Err(invalid("duplicate canvas entries"));
        }
        let pass = canvas
            .engine_pass
            .as_ref()
            .ok_or_else(|| invalid("no authored executable engine pass"))?;
        if pass.vertex_count == 0 || pass.vertex_entry.is_empty() || pass.fragment_entry.is_empty()
        {
            return Err(invalid("invalid authored stage entry or vertex count"));
        }
        let plan = &canvas.pass_plan;
        super::pass_plan::ValidatedPassPlan::new(plan)?;
        if plan.passes.len() != 1
            || !plan.targets.is_empty()
            || !plan.edges.is_empty()
            || plan.passes[0].kernel_strategy != "fused"
            || !plan.passes[0].inputs.is_empty()
            || plan.passes[0].output_target.is_some()
        {
            return Err(invalid(
                "the Rust canvas path currently requires one fused pass with no intermediate targets",
            ));
        }
        let instance_slot = (pass.instance_uniform_group, pass.instance_uniform_binding);
        if instance_slot.0 >= device.limits().max_bind_groups
            || instance_slot.1 >= device.limits().max_bindings_per_bind_group
        {
            return Err(invalid("fullscreen instance binding exceeds device limits"));
        }
        let stages = super::canvas_variant::select(entry, pass, inputs.variant)?;
        let limits = device.limits();
        let parameters = CanvasParameters::new(canvas, StorageLimits::from(&limits))?;
        let paths = PathBuffers::new(canvas, StorageLimits::from(&limits))?;
        if limits.max_uniform_buffer_binding_size < INSTANCE_SIZE {
            return Err(invalid("device cannot bind the fullscreen instance buffer"));
        }
        let mut uniforms = UniformSet::new(
            canvas.global_uniforms.clone(),
            UniformLimits {
                max_bind_groups: limits.max_bind_groups,
                max_bindings_per_bind_group: limits.max_bindings_per_bind_group,
                max_uniform_buffer_binding_size: limits.max_uniform_buffer_binding_size,
            },
        )?;
        if uniforms
            .definitions()
            .any(|def| (def.group, def.binding) == instance_slot)
        {
            return Err(invalid(
                "engine uniform collides with the fullscreen instance binding",
            ));
        }
        // Validate this profile's host-value providers before installing GPU resources.
        uniforms.update(|uniform, field| {
            frame_uniform(
                uniform,
                field,
                FrameInputs {
                    time: 0.0,
                    delta_time: 0.0,
                    physical_size: [1, 1],
                },
            )
        })?;
        validate_bindings(
            &canvas.textures,
            canvas.sampler.as_ref(),
            textures,
            std::iter::once(instance_slot)
                .chain(uniforms.definitions().map(|def| (def.group, def.binding))),
            TextureLimits {
                max_dimension: limits.max_texture_dimension_2d,
                max_bind_groups: limits.max_bind_groups,
                max_bindings_per_group: limits.max_bindings_per_bind_group,
                max_textures_per_stage: limits.max_sampled_textures_per_shader_stage,
                max_samplers_per_stage: limits.max_samplers_per_shader_stage,
            },
        )?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(entry),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        if let Some(error) = scope.pop().await {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation_scope = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let instance = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("canvas instance"),
            size: INSTANCE_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut resources = BTreeMap::<u32, Vec<(u32, Resource)>>::new();
        resources.insert(
            instance_slot.0,
            vec![(instance_slot.1, Resource::Uniform(instance.clone()))],
        );
        let uniform_buffers = uniforms
            .definitions()
            .map(|def| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&def.name),
                    size: u64::from(def.byte_size),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                resources
                    .entry(def.group)
                    .or_default()
                    .push((def.binding, Resource::Uniform(buffer.clone())));
                buffer
            })
            .collect();
        for upload in parameters.storage.uploads() {
            let def = upload.definition;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&def.name),
                size: u64::try_from(upload.bytes.len()).expect("validated storage size"),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, upload.bytes);
            resources.entry(def.group).or_default().push((
                def.binding,
                Resource::Storage {
                    buffer,
                    min_size: upload.stride as u64,
                },
            ));
        }
        for upload in paths.uploads() {
            let size = u64::try_from(upload.bytes.len()).expect("validated path buffer size");
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(upload.name),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, upload.bytes);
            resources.entry(upload.group).or_default().push((
                upload.binding,
                Resource::Storage {
                    buffer,
                    min_size: size,
                },
            ));
        }
        append_textures(
            &device,
            &queue,
            &canvas.textures,
            canvas.sampler.as_ref(),
            textures,
            &mut resources,
        );
        let mut layouts = BTreeMap::new();
        let mut groups = Vec::new();
        for (&group, bindings) in &resources {
            let entries: Vec<_> = bindings
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupLayoutEntry {
                    binding: *binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: resource.binding_type(),
                    count: None,
                })
                .collect();
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &entries,
            });
            let entries: Vec<_> = bindings
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: resource.binding(),
                })
                .collect();
            groups.push((
                group,
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &layout,
                    entries: &entries,
                }),
            ));
            layouts.insert(group, layout);
        }
        let highest_group = *layouts.keys().next_back().expect("instance group exists");
        let layout_refs: Vec<_> = (0..=highest_group)
            .map(|group| layouts.get(&group))
            .collect();
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("authored canvas layout"),
            bind_group_layouts: &layout_refs,
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(stages.vertex),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(stages.fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let allocation_result = allocation_scope.pop();
        let validation_result = scope.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = validation_error.or(allocation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            device,
            queue,
            pipeline,
            groups,
            uniform_buffers,
            uniforms,
            instance,
            vertex_count: pass.vertex_count,
            parameters,
            resources,
            layouts,
            parameter_revision: Arc::new(()),
        })
    }

    pub fn parameters(&self) -> &CanvasParameters {
        &self.parameters
    }

    /// Synchronous instance-only edit. Use begin_parameter_update for mixed
    /// batches or storage arrays, whose replacement GPU buffers need validation.
    pub fn update_parameters(
        &mut self,
        values: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        self.parameters.instance.update(values)?;
        self.parameter_revision = Arc::new(());
        Ok(())
    }

    /// Validate a mixed instance/storage batch and return an owned GPU candidate.
    pub fn begin_parameter_update(
        &self,
        values: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<PendingParameterUpdate, RuntimeError> {
        let mut parameters = self.parameters.clone();
        parameters.update(values)?;
        let storage_changed = values
            .keys()
            .any(|name| self.parameters.storage.values().contains_key(name));
        Ok(PendingParameterUpdate {
            device: self.device.clone(),
            queue: self.queue.clone(),
            layouts: self.layouts.clone(),
            storage_changed,
            prepared: PreparedParameterUpdate {
                parameters,
                resources: self.resources.clone(),
                groups: self.groups.clone(),
                revision: self.parameter_revision.clone(),
            },
        })
    }

    pub fn apply_parameter_update(
        &mut self,
        prepared: PreparedParameterUpdate,
    ) -> Result<(), RuntimeError> {
        if !Arc::ptr_eq(&self.parameter_revision, &prepared.revision) {
            return Err(RuntimeError::Parameter {
                name: "parameters".into(),
                reason: "prepared update belongs to a replaced renderer or obsolete input state"
                    .into(),
            });
        }
        self.parameters = prepared.parameters;
        self.resources = prepared.resources;
        self.groups = prepared.groups;
        self.parameter_revision = Arc::new(());
        Ok(())
    }

    /// Render to a host-owned target of the prepared format and physical size.
    /// Zero-sized hosts suspend drawing. No clock, window, or surface is consulted.
    pub fn render(
        &mut self,
        frame: FrameInputs,
        target: &wgpu::TextureView,
    ) -> Result<bool, RuntimeError> {
        if frame.physical_size.contains(&0) {
            return Ok(false);
        }
        if !frame.time.is_finite() || !frame.delta_time.is_finite() {
            return Err(RuntimeError::UniformValue {
                uniform: "frame".into(),
                field: "time/delta_time".into(),
                reason: "frame times must be finite".into(),
            });
        }
        self.uniforms
            .update(|uniform, field| frame_uniform(uniform, field, frame))?;
        for (upload, buffer) in self
            .uniforms
            .uploads()
            .expect("update succeeded")
            .zip(&self.uniform_buffers)
        {
            self.queue.write_buffer(buffer, 0, upload.bytes);
        }
        let mut instance = [0_u8; INSTANCE_SIZE as usize];
        instance[PARAMETER_OFFSET..].copy_from_slice(self.parameters.instance.bytes());
        instance[0..4].copy_from_slice(&frame.time.to_le_bytes());
        instance[32..36].copy_from_slice(&(frame.physical_size[0] as f32).to_le_bytes());
        instance[36..40].copy_from_slice(&(frame.physical_size[1] as f32).to_le_bytes());
        self.queue.write_buffer(&self.instance, 0, &instance);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("example canvas"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("authored canvas"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            for (group, binding) in &self.groups {
                pass.set_bind_group(*group, binding, &[]);
            }
            pass.draw(0..self.vertex_count, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(true)
    }
}
