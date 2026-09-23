//! GPU mechanics for reflected pass recipes. No renderer names or channel layouts.
use super::{
    RuntimeError, mesh_geometry::MeshGeometry, raster_state::RasterState, resources::Resource,
};
use fresco_artifact::{
    ManifestGpuProgram, ManifestRasterEntry, ManifestRecipeStep, ManifestRenderer, ManifestRoot,
    ManifestSurface,
};
use std::collections::{BTreeMap, BTreeSet};
fn invalid(reason: impl Into<String>) -> RuntimeError {
    RuntimeError::PassPlan(reason.into())
}
pub(crate) fn format(name: &str) -> Result<wgpu::TextureFormat, RuntimeError> {
    use fresco_artifact::types::ImageFormat as F;
    let format = F::parse(name).ok_or_else(|| invalid(format!("unknown image format `{name}`")))?;
    if let Some(storage) = format.naga_storage() {
        return Ok(wgpu_naga_bridge::map_storage_format_from_naga(storage));
    }
    Ok(match format {
        F::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        F::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
        F::Depth16Unorm => wgpu::TextureFormat::Depth16Unorm,
        F::Depth24Plus => wgpu::TextureFormat::Depth24Plus,
        F::Depth24PlusStencil8 => wgpu::TextureFormat::Depth24PlusStencil8,
        F::Depth32Float => wgpu::TextureFormat::Depth32Float,
        F::Depth32FloatStencil8 => wgpu::TextureFormat::Depth32FloatStencil8,
        F::Stencil8 => wgpu::TextureFormat::Stencil8,
        _ => return Err(invalid("image registry is missing its Naga representation")),
    })
}

pub(crate) struct StepPlan {
    pub declaration: ManifestRecipeStep,
    pub program: Option<ManifestGpuProgram>,
    pub vertex: Option<String>,
    pub entry: Option<String>,
    pub targets: Vec<Option<wgpu::ColorTargetState>>,
    pub depth: Option<wgpu::TextureFormat>,
    pub state: RasterState,
}
pub(crate) struct Plan {
    pub recipe: ManifestRenderer,
    pub steps: Vec<StepPlan>,
    pub table_bytes: BTreeMap<String, Vec<u8>>,
}
impl Plan {
    pub fn new(
        manifest: &ManifestRoot,
        surface: &ManifestSurface,
        mesh_entries: &BTreeMap<String, &[ManifestRasterEntry]>,
        output_format: wgpu::TextureFormat,
        limits: &wgpu::Limits,
    ) -> Result<Self, RuntimeError> {
        let selected: Vec<_> = manifest.renderers.iter().filter(|r| r.selected).collect();
        let [recipe] = selected.as_slice() else {
            return Err(invalid(
                "artifact requires one explicitly selected renderer",
            ));
        };
        if recipe.steps.is_empty() {
            return Err(invalid("selected renderer has no executable recipe"));
        }
        validate_invocations(&recipe.steps)?;
        fresco_artifact::validate_transparent_queues(&recipe.steps).map_err(invalid)?;
        for invocation in recipe.steps.iter().filter_map(|s| s.invocation.as_ref()) {
            if !manifest
                .surfaces
                .iter()
                .any(|s| s.name == invocation.material)
            {
                return Err(invalid("style invocation names an unknown material"));
            }
        }
        let mut resources = BTreeMap::new();
        let mut table_bytes = BTreeMap::new();
        let mut external_sources = BTreeSet::new();
        for resource in &recipe.resources {
            if resources.insert(resource.name.as_str(), resource).is_some() {
                return Err(invalid("duplicate recipe resource"));
            }
            match resource.kind.as_str() {
                "image" => {
                    format(
                        resource
                            .format
                            .as_deref()
                            .ok_or_else(|| invalid("missing image format"))?,
                    )?;
                }
                "buffer" => {
                    let size = u64::from(
                        resource
                            .bytes
                            .ok_or_else(|| invalid("missing buffer size"))?,
                    );
                    if size == 0
                        || size % 4 != 0
                        || size > limits.max_buffer_size
                        || size > limits.max_storage_buffer_binding_size
                    {
                        return Err(invalid("recipe buffer exceeds device limits"));
                    }
                }
                "external" => {
                    let source = resource
                        .source
                        .as_ref()
                        .ok_or_else(|| invalid("external resource has no source"))?;
                    if !external_sources.insert(source) {
                        return Err(invalid("duplicate external resource source"));
                    }
                }
                "table_data" => {
                    let bytes = super::tables::column_bytes(
                        manifest,
                        resource
                            .table
                            .as_deref()
                            .ok_or_else(|| invalid("table name missing"))?,
                        resource
                            .column
                            .as_deref()
                            .ok_or_else(|| invalid("table column missing"))?,
                        limits
                            .max_buffer_size
                            .min(limits.max_storage_buffer_binding_size),
                    )?;
                    table_bytes.insert(resource.name.clone(), bytes);
                }
                _ => return Err(invalid("unsupported recipe resource")),
            }
        }
        let settings = surface
            .settings
            .as_ref()
            .ok_or_else(|| invalid("surface settings missing"))?;
        let mut steps = Vec::new();
        let mut written = BTreeSet::new();
        let mut completed = BTreeSet::new();
        for declaration in &recipe.steps {
            if declaration.dispatch_scale.contains(&0) {
                return Err(invalid("dispatch scale must be positive"));
            }
            for (name, stride) in &declaration.capacity {
                if declaration.domain != "compute"
                    || *stride == 0
                    || !resources
                        .get(name.as_str())
                        .is_some_and(|r| r.kind == "buffer")
                    || !declaration.bindings.values().any(|r| r == name)
                {
                    return Err(invalid("invalid compute capacity contract"));
                }
            }
            if declaration
                .after
                .iter()
                .any(|name| !completed.contains(name))
            {
                return Err(invalid("recipe order has an unsatisfied dependency"));
            }
            if !completed.insert(declaration.name.clone()) {
                return Err(invalid("duplicate recipe node"));
            }
            if let Some(condition) = &declaration.condition
                && !*settings
                    .recipe_conditions
                    .get(condition)
                    .ok_or_else(|| invalid("recipe condition is unresolved"))?
            {
                continue;
            }
            for (name, ops) in &declaration.attachments {
                if !declaration
                    .colors
                    .values()
                    .chain(declaration.depth.iter())
                    .any(|a| a == name)
                {
                    return Err(invalid("operations reference a non-attachment"));
                }
                if ops.load && !written.contains(name) {
                    return Err(invalid(format!(
                        "attachment `{name}` loads undefined contents"
                    )));
                }
            }
            let program = if declaration.is_draw_scoped() {
                let pass = surface
                    .mesh_passes
                    .iter()
                    .find(|p| p.pass == declaration.pass)
                    .ok_or_else(|| invalid("recipe draw pass missing"))?;
                if pass.procedural != (declaration.domain == "instance") {
                    return Err(invalid("recipe draw domain and vertex source disagree"));
                }
                None
            } else {
                Some(
                    manifest
                        .gpu_programs
                        .iter()
                        .find(|p| p.pass == declaration.pass)
                        .ok_or_else(|| invalid("recipe GPU program missing"))?
                        .clone(),
                )
            };
            let entries = if let Some(program) = &program {
                program.entries.as_slice()
            } else {
                *mesh_entries
                    .get(&declaration.pass)
                    .ok_or_else(|| invalid("recipe mesh program missing"))?
            };
            let entry = |name: &str, stage: &str| -> Result<&ManifestRasterEntry, RuntimeError> {
                let candidates: Vec<_> = entries
                    .iter()
                    .filter(|e| e.function == name && e.stage == stage)
                    .collect();
                let [entry] = candidates.as_slice() else {
                    return Err(invalid(format!(
                        "missing or ambiguous {stage} function `{name}`"
                    )));
                };
                Ok(*entry)
            };
            let (vertex, entry) = match declaration.domain.as_str() {
                "compute" => (None, Some(entry(&declaration.entry, "compute")?)),
                "mesh" | "instance" | "fullscreen" => (
                    Some(
                        entry(
                            declaration
                                .vertex
                                .as_deref()
                                .ok_or_else(|| invalid("vertex function missing"))?,
                            "vertex",
                        )?
                        .entry
                        .clone(),
                    ),
                    if declaration.entry.is_empty() {
                        None
                    } else {
                        Some(entry(&declaration.entry, "fragment")?)
                    },
                ),
                _ => return Err(invalid("unsupported recipe draw domain")),
            };
            let state = if declaration.domain == "mesh"
                || settings.pass_states.contains_key(&declaration.pass)
            {
                super::raster_state::resolve_targets(
                    settings,
                    &declaration.pass,
                    !declaration.colors.is_empty(),
                )?
            } else {
                RasterState {
                    cull: None,
                    blend: None,
                    depth_write: false,
                    depth_compare: wgpu::CompareFunction::Always,
                }
            };
            if declaration.transparent_queue.is_some() {
                settings
                    .pass_states
                    .get(&declaration.pass)
                    .ok_or_else(|| invalid("transparent queue requires explicit raster state"))?
                    .validate_transparent_queue()
                    .map_err(invalid)?;
            }
            let mut targets = Vec::new();
            if entry.map_or(0, |e| e.outputs.len()) != declaration.colors.len() {
                return Err(invalid("fragment output and attachment counts disagree"));
            }
            for output in entry.into_iter().flat_map(|e| &e.outputs) {
                if output.location >= limits.max_color_attachments {
                    return Err(invalid("attachment location exceeds device limits"));
                }
                let name = declaration
                    .colors
                    .get(&output.location)
                    .ok_or_else(|| invalid("fragment output has no attachment"))?;
                let resource = resources
                    .get(name.as_str())
                    .ok_or_else(|| invalid("attachment resource missing"))?;
                let target = if resource.source.as_deref() == Some("presentation") {
                    output_format
                } else {
                    format(
                        resource
                            .format
                            .as_deref()
                            .ok_or_else(|| invalid("color output requires an image"))?,
                    )?
                };
                let valid = match target.sample_type(None, None) {
                    Some(wgpu::TextureSampleType::Uint) => {
                        fresco_artifact::types::ImageScalar::U32.accepts_shader_output(&output.ty)
                    }
                    Some(wgpu::TextureSampleType::Sint) => {
                        fresco_artifact::types::ImageScalar::I32.accepts_shader_output(&output.ty)
                    }
                    Some(wgpu::TextureSampleType::Depth) | None => false,
                    Some(wgpu::TextureSampleType::Float { .. }) => {
                        fresco_artifact::types::ImageScalar::F32.accepts_shader_output(&output.ty)
                    }
                };
                if !valid {
                    return Err(invalid(
                        "fragment output type is incompatible with attachment format",
                    ));
                }
                if !written.insert(name.clone()) && !declaration.attachments.contains_key(name) {
                    return Err(invalid(
                        "multiple active writes require an explicit load/accumulation contract",
                    ));
                }
                let required = usize::try_from(output.location + 1)
                    .map_err(|_| invalid("attachment location overflow"))?;
                if targets.len() < required {
                    targets.resize_with(required, || None);
                }
                if targets[output.location as usize].is_some() {
                    return Err(invalid("duplicate attachment location"));
                }
                targets[output.location as usize] = Some(wgpu::ColorTargetState {
                    format: target,
                    blend: state.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                });
            }
            super::device::validate_color_attachments(
                targets.iter().flatten().map(|target| target.format),
                limits.max_color_attachment_bytes_per_sample,
            )?;
            let depth = if let Some(name) = &declaration.depth {
                let resource = resources
                    .get(name.as_str())
                    .ok_or_else(|| invalid("depth resource missing"))?;
                let format = if resource.source.as_deref() == Some("depth") {
                    wgpu::TextureFormat::Depth32Float
                } else {
                    format(
                        resource
                            .format
                            .as_deref()
                            .ok_or_else(|| invalid("depth attachment requires an image"))?,
                    )?
                };
                if !format.is_depth_stencil_format() {
                    return Err(invalid("depth attachment requires a depth format"));
                }
                written.insert(name.clone());
                Some(format)
            } else {
                None
            };
            if let Some(program) = &program {
                if program.bindings.len() != declaration.bindings.len() {
                    return Err(invalid("GPU program resource wiring is incomplete"));
                }
                for binding in &program.bindings {
                    let name = declaration
                        .bindings
                        .get(&binding.name)
                        .ok_or_else(|| invalid("GPU binding has no resource"))?;
                    let resource = resources
                        .get(name.as_str())
                        .ok_or_else(|| invalid("GPU resource missing"))?;
                    if binding.group >= limits.max_bind_groups
                        || binding.binding >= limits.max_bindings_per_bind_group
                    {
                        return Err(invalid("GPU binding exceeds device limits"));
                    }
                    if binding.kind == "texture" && resource.kind != "image" {
                        return Err(invalid("sampled binding requires an internal image"));
                    }
                    if binding.kind != "texture"
                        && (resource.kind == "image"
                            || matches!(resource.source.as_deref(), Some("presentation" | "depth")))
                    {
                        return Err(invalid("buffer binding requires a buffer resource"));
                    }
                    if declaration.colors.values().any(|output| output == name)
                        || declaration.depth.as_ref() == Some(name)
                    {
                        return Err(invalid("step samples its active attachment"));
                    }
                    if binding.kind == "texture" && !written.contains(name) {
                        return Err(invalid("recipe reads an undefined image"));
                    }
                    if matches!(binding.access.as_str(), "write" | "read_write") {
                        written.insert(name.clone());
                    }
                    if resource.kind == "buffer"
                        && binding.access == "read"
                        && !written.contains(name)
                    {
                        return Err(invalid("recipe reads an undefined buffer"));
                    }
                }
                if declaration.domain == "compute" {
                    let size = program
                        .workgroup_size
                        .ok_or_else(|| invalid("compute workgroup missing"))?;
                    if size[0] > limits.max_compute_workgroup_size_x
                        || size[1] > limits.max_compute_workgroup_size_y
                        || size[2] > limits.max_compute_workgroup_size_z
                        || size
                            .into_iter()
                            .try_fold(1u32, u32::checked_mul)
                            .is_none_or(|n| n > limits.max_compute_invocations_per_workgroup)
                    {
                        return Err(invalid("workgroup exceeds device limits"));
                    }
                }
            }
            for (name, ops) in &declaration.attachments {
                if !ops.store {
                    written.remove(name);
                }
            }
            let entry = entry.map(|e| e.entry.clone());
            steps.push(StepPlan {
                declaration: declaration.clone(),
                program,
                vertex,
                entry,
                targets,
                depth,
                state,
            });
        }
        if !steps.iter().any(|s| s.declaration.domain == "mesh") {
            return Err(invalid("mesh recipe has no active mesh draw"));
        }
        Ok(Self {
            recipe: (*recipe).clone(),
            steps,
            table_bytes,
        })
    }
}
use super::technique::{Invocation, Parameters, Pipeline};
struct Step {
    plan: StepPlan,
    pipeline: Pipeline,
}
type MeshGroups = BTreeMap<String, Vec<(u32, wgpu::BindGroup)>>;
type SceneObject<'a> = (
    &'a Executor,
    &'a MeshGeometry,
    &'a MeshGroups,
    f32,
    &'a super::compute_frame::ComputeFrame,
    &'a BTreeMap<String, BTreeSet<String>>,
);

pub(crate) struct Executor {
    runner: super::technique::Executor,
    prepared: BTreeMap<String, MeshGeometry>,
    plan: ManifestRenderer,
    steps: Vec<Step>,
    pub resources: BTreeMap<String, Resource>,
    textures: BTreeMap<String, wgpu::Texture>,
    size: [u32; 2],
}
impl Executor {
    pub fn prepare_generated(
        &self,
        frame: &mut super::compute_frame::ComputeFrame,
        geometry: &BTreeMap<String, super::compute_frame::GeometryInputs>,
        settings: &super::style_parameters::StyleParameters,
    ) -> Result<(), RuntimeError> {
        for step in &self.steps {
            let declaration = &step.plan.declaration;
            if let Some(invocation) = &declaration.invocation {
                frame.validate_draw(invocation, geometry, settings)?;
                if invocation.generated_vertices.is_none() {
                    continue;
                }
                let source = self
                    .prepared
                    .get(&declaration.name)
                    .ok_or_else(|| invalid("generated draw is missing prepared geometry"))?;
                let generated = frame.generated_geometry(invocation, source, geometry, settings)?;
                frame.generated.insert(declaration.name.clone(), generated);
            }
        }
        Ok(())
    }

    pub fn prepare_spatial(
        &self,
        frame: &mut super::compute_frame::ComputeFrame,
        geometry: &BTreeMap<String, super::compute_frame::GeometryInputs>,
        settings: &super::style_parameters::StyleParameters,
        bounds: Option<super::bounds::Bounds>,
        scene: &crate::profile::mesh::MeshSceneInputs,
    ) -> Result<(), RuntimeError> {
        for step in &self.steps {
            let declaration = &step.plan.declaration;
            if let Some(invocation) = &declaration.invocation {
                let (visible, depth) =
                    frame.draw_spatial(invocation, geometry, settings, bounds, scene)?;
                if !visible {
                    frame.culled.insert(declaration.name.clone());
                }
                if let Some(depth) = depth {
                    frame.depths.insert(declaration.name.clone(), depth);
                }
            }
        }
        Ok(())
    }

    pub fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        plan: Plan,
        resources: BTreeMap<String, Resource>,
        mesh_layouts: &BTreeMap<String, wgpu::PipelineLayout>,
        geometry: &MeshGeometry,
        prepared: BTreeMap<String, MeshGeometry>,
    ) -> Result<Self, RuntimeError> {
        let steps: Vec<Step> = plan
            .steps
            .into_iter()
            .map(|step| {
                let pipeline = if step.declaration.domain == "compute" {
                    Pipeline::Compute(device.create_compute_pipeline(
                        &wgpu::ComputePipelineDescriptor {
                            label: Some(&step.declaration.name),
                            layout: None,
                            module: shader,
                            entry_point: step.entry.as_deref(),
                            compilation_options: Default::default(),
                            cache: None,
                        },
                    ))
                } else {
                    let buffers = if step.declaration.domain == "mesh"
                        && !prepared.contains_key(&step.declaration.name)
                    {
                        vec![Some(geometry.vertex_layout())]
                    } else {
                        Vec::new()
                    };
                    Pipeline::Draw(
                        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some(&step.declaration.name),
                            layout: if step.declaration.is_draw_scoped() {
                                Some(&mesh_layouts[&step.declaration.name])
                            } else {
                                None
                            },
                            vertex: wgpu::VertexState {
                                module: shader,
                                entry_point: step.vertex.as_deref(),
                                compilation_options: Default::default(),
                                buffers: &buffers,
                            },
                            fragment: step.entry.as_ref().map(|_| wgpu::FragmentState {
                                module: shader,
                                entry_point: step.entry.as_deref(),
                                compilation_options: Default::default(),
                                targets: &step.targets,
                            }),
                            primitive: wgpu::PrimitiveState {
                                cull_mode: step.state.cull,
                                ..Default::default()
                            },
                            depth_stencil: step.depth.map(|format| wgpu::DepthStencilState {
                                format,
                                depth_write_enabled: Some(step.state.depth_write),
                                depth_compare: Some(step.state.depth_compare),
                                stencil: Default::default(),
                                bias: Default::default(),
                            }),
                            multisample: Default::default(),
                            multiview_mask: None,
                            cache: None,
                        }),
                    )
                };
                Step {
                    plan: step,
                    pipeline,
                }
            })
            .collect();
        let graph = fresco_artifact::ManifestTechnique {
            name: plan.recipe.id.clone(),
            surface: None,
            metadata: BTreeMap::new(),
            resources: Vec::new(),
            outputs: BTreeMap::new(),
            steps: steps
                .iter()
                .map(|s| fresco_artifact::ManifestTechniqueStep {
                    attachments: s.plan.declaration.attachments.clone(),
                    name: s.plan.declaration.name.clone(),
                    pass: s.plan.declaration.pass.clone(),
                    enabled: None,
                    bindings: s.plan.declaration.bindings.clone(),
                    reads: Vec::new(),
                    writes: Vec::new(),
                    after: s
                        .plan
                        .declaration
                        .after
                        .iter()
                        .filter(|name| {
                            steps
                                .iter()
                                .any(|candidate| &candidate.plan.declaration.name == *name)
                        })
                        .cloned()
                        .collect(),
                    operation: if matches!(s.pipeline, Pipeline::Compute(_)) {
                        fresco_artifact::ManifestTechniqueOperation::Compute {
                            entry: s.plan.entry.clone().expect("validated compute entry"),
                            extent: fresco_artifact::ManifestDispatchExtent::Parameter {
                                parameter: s.plan.declaration.name.clone(),
                            },
                            workgroup_size: s
                                .plan
                                .program
                                .as_ref()
                                .expect("compute program")
                                .workgroup_size
                                .expect("validated workgroup"),
                        }
                    } else {
                        fresco_artifact::ManifestTechniqueOperation::Draw {
                            vertex: s.plan.vertex.clone().expect("validated vertex"),
                            fragment: s.plan.entry.clone(),
                            vertices: s.plan.declaration.vertex_count,
                            instances: fresco_artifact::ManifestDrawCount::Fixed(1),
                            colors: s.plan.declaration.colors.clone(),
                            depth: s.plan.declaration.depth.clone(),
                        }
                    },
                })
                .collect(),
        };
        let pipelines = steps
            .iter()
            .map(|s| (s.plan.declaration.name.clone(), s.pipeline.clone()))
            .collect();
        let runner = super::technique::Executor::new(graph, pipelines, &device.limits())?;
        Ok(Self {
            runner,
            prepared,
            plan: plan.recipe,
            steps,
            resources,
            textures: BTreeMap::new(),
            size: [0; 2],
        })
    }
    pub fn buffer_for_source(&self, source: &str) -> Option<&wgpu::Buffer> {
        self.plan
            .resources
            .iter()
            .find(|r| r.source.as_deref() == Some(source))
            .and_then(|r| self.resources.get(&r.name))
            .and_then(|r| match r {
                Resource::Uniform(b) | Resource::Storage { buffer: b, .. } => Some(b),
                _ => None,
            })
    }
    pub fn written_buffer(&self) -> Option<&wgpu::Buffer> {
        self.steps
            .iter()
            .filter_map(|s| s.plan.program.as_ref().map(|p| (s, p)))
            .flat_map(|(s, p)| {
                p.bindings
                    .iter()
                    .filter(|b| matches!(b.access.as_str(), "write" | "read_write"))
                    .filter_map(move |b| s.plan.declaration.bindings.get(&b.name))
            })
            .find_map(|name| match self.resources.get(name) {
                Some(Resource::Storage { buffer, .. }) => Some(buffer),
                _ => None,
            })
    }
    pub fn texture(&self, name: &str) -> Option<&wgpu::Texture> {
        self.textures.get(name)
    }

    pub fn scene_depth(&self) -> Result<Option<wgpu::TextureView>, RuntimeError> {
        let step = self
            .steps
            .iter()
            .find(|s| {
                s.plan.declaration.is_draw_scoped()
                    && !s.plan.declaration.colors.is_empty()
                    && s.plan.state.depth_write
            })
            .ok_or_else(|| invalid("scene has no opaque depth-writing draw"))?;
        let name = step
            .plan
            .declaration
            .depth
            .as_ref()
            .ok_or_else(|| invalid("scene draw has no depth attachment"))?;
        if self
            .plan
            .resources
            .iter()
            .any(|r| r.name == *name && r.source.as_deref() == Some("depth"))
        {
            return Ok(None);
        }
        self.textures
            .get(name)
            .map(|t| Some(t.create_view(&Default::default())))
            .ok_or_else(|| invalid("scene depth image is unavailable"))
    }

    pub fn image_sizes(&self) -> BTreeMap<String, [u32; 2]> {
        self.textures
            .iter()
            .map(|(name, texture)| (name.clone(), [texture.width(), texture.height()]))
            .collect()
    }
    pub fn integer_texture(&self) -> Option<&wgpu::Texture> {
        self.textures
            .values()
            .find(|t| t.format().sample_type(None, None) == Some(wgpu::TextureSampleType::Uint))
    }
    pub fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) -> Result<(), RuntimeError> {
        if size
            .iter()
            .any(|n| *n > device.limits().max_texture_dimension_2d)
        {
            return Err(invalid("recipe viewport exceeds device limits"));
        }
        for step in &self.steps {
            for (name, stride) in &step.plan.declaration.capacity {
                let buffer = self
                    .resources
                    .get(name)
                    .ok_or_else(|| invalid("capacity resource missing"))?;
                let Resource::Storage { buffer, .. } = buffer else {
                    return Err(invalid("capacity requires a storage buffer"));
                };
                let count = u64::from(size[0].div_ceil(step.plan.declaration.dispatch_scale[0]))
                    * u64::from(size[1].div_ceil(step.plan.declaration.dispatch_scale[1]));
                if count
                    .checked_mul(u64::from(*stride))
                    .is_none_or(|bytes| bytes > buffer.size())
                {
                    return Err(invalid("dispatch exceeds declared buffer capacity"));
                }
            }
            if let Some(program) = &step.plan.program
                && let Some(workgroup) = program.workgroup_size
            {
                for axis in 0..2 {
                    if size[axis]
                        .div_ceil(step.plan.declaration.dispatch_scale[axis])
                        .div_ceil(workgroup[axis])
                        > device.limits().max_compute_workgroups_per_dimension
                    {
                        return Err(invalid("dispatch exceeds device limits"));
                    }
                }
            }
        }
        if size == self.size {
            return Ok(());
        }
        let image_usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        let images = self.plan.resources.iter().filter(|r| r.kind == "image")
            .map(|resource| {
            let format = format(resource.format.as_deref().expect("validated image format"))?;
            if !device.features().contains(format.required_features()) {
                return Err(invalid(format!("image format {format:?} requires features not enabled on this device")));
            }
            if !format.guaranteed_format_features(device.features()).allowed_usages.contains(image_usage) {
                return Err(invalid(format!("image format {format:?} does not support recipe attachment/sampling/copy usage on this device")));
            }
            Ok((resource, format))
        }).collect::<Result<Vec<_>, RuntimeError>>()?;
        for (resource, format) in images {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&resource.name),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
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
            self.resources.insert(
                resource.name.clone(),
                Resource::Texture(texture.create_view(&Default::default())),
            );
            self.textures.insert(resource.name.clone(), texture);
        }
        self.size = size;
        Ok(())
    }
    fn invocation_groups(
        &self,
        device: &wgpu::Device,
        mesh_groups: &BTreeMap<String, Vec<(u32, wgpu::BindGroup)>>,
    ) -> (BTreeMap<String, Vec<(u32, wgpu::BindGroup)>>, Parameters) {
        let mut owned_groups = BTreeMap::new();
        let mut parameters = Parameters::default();
        for step in &self.steps {
            if matches!(step.pipeline, Pipeline::Compute(_)) {
                parameters.extents.insert(
                    step.plan.declaration.name.clone(),
                    [
                        self.size[0].div_ceil(step.plan.declaration.dispatch_scale[0]),
                        self.size[1].div_ceil(step.plan.declaration.dispatch_scale[1]),
                        1,
                    ],
                );
            }
            let mut resources: BTreeMap<u32, Vec<wgpu::BindGroupEntry<'_>>> = BTreeMap::new();
            if let Some(program) = &step.plan.program {
                for binding in &program.bindings {
                    if !step
                        .plan
                        .entry
                        .iter()
                        .chain(step.plan.vertex.iter())
                        .any(|entry| binding.entry_access.contains_key(entry))
                    {
                        continue;
                    }
                    let name = &step.plan.declaration.bindings[&binding.name];
                    resources
                        .entry(binding.group)
                        .or_default()
                        .push(wgpu::BindGroupEntry {
                            binding: binding.binding,
                            resource: self.resources[name].binding(),
                        });
                }
            }
            let groups: Vec<_> = resources
                .into_iter()
                .map(|(group, entries)| {
                    let layout = match &step.pipeline {
                        Pipeline::Compute(p) => p.get_bind_group_layout(group),
                        Pipeline::Draw(p) => p.get_bind_group_layout(group),
                    };
                    (
                        group,
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some(&step.plan.declaration.name),
                            layout: &layout,
                            entries: &entries,
                        }),
                    )
                })
                .collect();
            owned_groups.insert(step.plan.declaration.name.clone(), groups);
        }
        for (name, groups) in mesh_groups {
            owned_groups.insert(name.clone(), groups.clone());
        }
        (owned_groups, parameters)
    }

    pub fn validate_scene_object(&self, other: &Self) -> Result<(), RuntimeError> {
        let left = serde_json::to_vec(&self.plan).map_err(|e| invalid(e.to_string()))?;
        let right = serde_json::to_vec(&other.plan).map_err(|e| invalid(e.to_string()))?;
        if left != right {
            return Err(invalid(
                "scene objects require identical frame resource and recipe contracts",
            ));
        }
        Ok(())
    }

    pub fn share_frame_resources(&mut self, primary: &Self) -> Result<(), RuntimeError> {
        if self.plan.id != primary.plan.id {
            return Err(invalid("scene objects require the same compiled recipe"));
        }
        for resource in &self.plan.resources {
            if resource.source.as_deref() != Some("scene")
                && let Some(value) = primary.resources.get(&resource.name)
            {
                self.resources.insert(resource.name.clone(), value.clone());
            }
        }
        self.textures.clone_from(&primary.textures);
        self.size = primary.size;
        Ok(())
    }

    pub fn encode_scene(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        objects: &[SceneObject<'_>],
    ) -> Result<(), RuntimeError> {
        let primary = objects
            .first()
            .ok_or_else(|| invalid("empty render scene"))?
            .0;
        let mut attachments = BTreeMap::new();
        for resource in &primary.plan.resources {
            let view = match resource.source.as_deref() {
                Some("presentation") => Some(color),
                Some("depth") => Some(depth),
                _ => match primary.resources.get(&resource.name) {
                    Some(Resource::Texture(v)) => Some(v),
                    _ => None,
                },
            };
            if let Some(view) = view {
                attachments.insert(resource.name.clone(), view);
            }
        }
        let prepared: Vec<_> = objects
            .iter()
            .map(|(e, _, g, _, _, _)| e.invocation_groups(device, g))
            .collect();
        let mut graph = primary.runner.graph.clone();
        graph.steps.clear();
        let mut pipelines = BTreeMap::new();
        let mut groups = BTreeMap::new();
        let mut geometry = BTreeMap::new();
        let parameters = Parameters::default();
        let mut previous: Option<String> = None;
        let mut counts = vec![0usize; primary.plan.steps.len()];
        let mut order = scene_order(
            &primary.plan.steps,
            objects.len(),
            |step, i| {
                objects[i]
                    .0
                    .steps
                    .iter()
                    .find(|s| s.plan.declaration.name == primary.plan.steps[step].name)
                    .is_some_and(|s| s.plan.state.blend.is_some())
            },
            |step, i| {
                objects[i]
                    .4
                    .depths
                    .get(&primary.plan.steps[step].name)
                    .copied()
                    .unwrap_or(objects[i].3)
            },
        );
        order.retain(|(step, i)| {
            !objects[*i]
                .4
                .culled
                .contains(&primary.plan.steps[*step].name)
        });
        use super::compute_graph::{
            ComputePrerequisite as Dependency, ComputeSchedule, ScheduledCompute,
        };
        let mut schedule = ComputeSchedule::new(
            objects
                .iter()
                .map(|object| {
                    object
                        .4
                        .nodes
                        .iter()
                        .map(|node| ScheduledCompute {
                            name: &node.name,
                            prerequisites: &node.prerequisites,
                        })
                        .collect()
                })
                .collect(),
        );
        for (index, object) in objects.iter().enumerate() {
            for dependency in object.4.nodes.iter().flat_map(|node| &node.prerequisites) {
                if let Dependency::PreparedGeometry(producer) = dependency {
                    schedule.geometry_completed(index, producer);
                }
            }
        }
        // All prepared geometry was encoded before this scene. Insert compute
        // at its earliest resource-ready boundary, independently of draw placement.
        let mut compute_groups = BTreeMap::new();
        for (index, object) in objects.iter().enumerate() {
            for node in &object.4.nodes {
                compute_groups.insert(
                    format!("object_{index}__compute__{}", node.name),
                    node.groups.clone(),
                );
            }
        }
        let mut remaining = vec![0usize; primary.plan.steps.len()];
        for &(declaration_index, index) in &order {
            let declaration = &primary.plan.steps[declaration_index];
            if objects[index]
                .0
                .runner
                .graph
                .steps
                .iter()
                .any(|s| s.name == declaration.name)
            {
                if declaration.is_draw_scoped() {
                    remaining[declaration_index] += 1;
                } else {
                    remaining[declaration_index] = 1;
                }
            }
        }
        let mut completed = BTreeSet::new();
        for name in completed_boundaries(&primary.plan.steps, &remaining, &mut completed) {
            schedule.engine_completed(&name);
        }
        append_ready_compute(
            objects,
            &mut schedule,
            &mut graph,
            &mut pipelines,
            &mut previous,
        );
        for (declaration_index, index) in order {
            let declaration = &primary.plan.steps[declaration_index];
            let instances = &mut counts[declaration_index];
            let (executor, mesh, _, _, _, draw_dependencies) = objects[index];
            let Some(step) = executor
                .runner
                .graph
                .steps
                .iter()
                .find(|s| s.name == declaration.name)
            else {
                continue;
            };
            if !declaration.is_draw_scoped() && *instances != 0 {
                continue;
            }
            if draw_dependencies
                .get(&declaration.name)
                .is_some_and(|dependencies| {
                    dependencies.iter().any(|p| !schedule.is_complete(index, p))
                })
            {
                return Err(invalid(
                    "draw reaches a compute output before its producer can execute",
                ));
            }
            let mut node = step.clone();
            node.name = format!("object_{index}__{}", step.name);
            node.after = previous.iter().cloned().collect();
            if declaration.domain == "mesh" {
                geometry.insert(
                    node.name.clone(),
                    objects[index]
                        .4
                        .generated
                        .get(&declaration.name)
                        .or_else(|| executor.prepared.get(&declaration.name))
                        .unwrap_or(mesh),
                );
            }
            if declaration.is_draw_scoped() && *instances > 0 {
                for name in declaration.colors.values().chain(declaration.depth.iter()) {
                    node.attachments.insert(
                        name.clone(),
                        fresco_artifact::ManifestAttachmentOps {
                            load: true,
                            store: true,
                        },
                    );
                }
            }
            if let fresco_artifact::ManifestTechniqueOperation::Compute { extent, .. } =
                &mut node.operation
            {
                let count = prepared[index].1.extents[&step.name];
                *extent = fresco_artifact::ManifestDispatchExtent::Fixed(count);
            }
            let pipeline = executor
                .steps
                .iter()
                .find(|s| s.plan.declaration.name == step.name)
                .expect("prepared node");
            pipelines.insert(node.name.clone(), pipeline.pipeline.clone());
            groups.insert(node.name.clone(), prepared[index].0[&step.name].as_slice());
            previous = Some(node.name.clone());
            graph.steps.push(node);
            *instances += 1;
            remaining[declaration_index] -= 1;
            if remaining[declaration_index] == 0 {
                for name in completed_boundaries(&primary.plan.steps, &remaining, &mut completed) {
                    schedule.engine_completed(&name);
                }
                append_ready_compute(
                    objects,
                    &mut schedule,
                    &mut graph,
                    &mut pipelines,
                    &mut previous,
                );
            }
        }
        schedule.finish()?;
        groups.extend(
            compute_groups
                .iter()
                .map(|(name, groups)| (name.clone(), groups.as_slice())),
        );
        let runner = super::technique::Executor::new(graph, pipelines, &device.limits())?;
        runner.encode(
            encoder,
            Invocation {
                parameters: &parameters,
                groups,
                attachments,
                geometry,
            },
        )
    }
}

fn append_ready_compute(
    objects: &[SceneObject<'_>],
    schedule: &mut super::compute_graph::ComputeSchedule<'_>,
    graph: &mut fresco_artifact::ManifestTechnique,
    pipelines: &mut BTreeMap<String, Pipeline>,
    previous: &mut Option<String>,
) {
    for (index, node_index) in schedule.ready() {
        let node = &objects[index].4.nodes[node_index];
        let name = format!("object_{index}__compute__{}", node.name);
        graph.steps.push(fresco_artifact::ManifestTechniqueStep {
            name: name.clone(),
            pass: node.name.clone(),
            enabled: None,
            bindings: BTreeMap::new(),
            reads: Vec::new(),
            writes: Vec::new(),
            after: previous.iter().cloned().collect(),
            attachments: BTreeMap::new(),
            operation: fresco_artifact::ManifestTechniqueOperation::Compute {
                entry: node.entry.clone(),
                extent: fresco_artifact::ManifestDispatchExtent::Fixed(node.threads),
                workgroup_size: node.workgroup,
            },
        });
        pipelines.insert(name.clone(), Pipeline::Compute(node.pipeline.clone()));
        *previous = Some(name);
    }
}

// A culled/absent preserving writer still carries its incoming graph edges.
// Completing it at frame start would release consumers before those inputs exist.
fn completed_boundaries(
    steps: &[ManifestRecipeStep],
    remaining: &[usize],
    completed: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut result = Vec::new();
    loop {
        let before = result.len();
        for (step, count) in steps.iter().zip(remaining) {
            if *count == 0
                && !completed.contains(&step.name)
                && step.after.iter().all(|name| completed.contains(name))
            {
                completed.insert(step.name.clone());
                result.push(step.name.clone());
            }
        }
        if result.len() == before {
            return result;
        }
    }
}

/// Expand a view's recipe while preserving object/range identity. Style operations
/// at one boundary compose per object; ordinary view phases keep their own order.
fn scene_order(
    steps: &[ManifestRecipeStep],
    count: usize,
    blended: impl Fn(usize, usize) -> bool,
    depth: impl Fn(usize, usize) -> f32,
) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut start = 0;
    while start < steps.len() {
        if let Some(queue) = &steps[start].transparent_queue {
            let end = steps[start..]
                .iter()
                .position(|step| step.transparent_queue.as_ref() != Some(queue))
                .map_or(steps.len(), |offset| start + offset);
            let mut draws: Vec<_> = (start..end)
                .flat_map(|step| (0..count).map(move |object| (step, object)))
                .collect();
            draws.sort_by(|&(a_step, a_object), &(b_step, b_object)| {
                depth(b_step, b_object)
                    .total_cmp(&depth(a_step, a_object))
                    .then_with(|| a_object.cmp(&b_object))
                    .then_with(|| {
                        let ordinal =
                            |step: usize| steps[step].invocation.as_ref().map_or(0, |i| i.ordinal);
                        ordinal(a_step).cmp(&ordinal(b_step))
                    })
                    .then_with(|| a_step.cmp(&b_step))
            });
            result.extend(draws);
            start = end;
        } else if let Some(invocation) = &steps[start].invocation {
            let mut end = start + 1;
            while end < steps.len()
                && steps[end]
                    .invocation
                    .as_ref()
                    .is_some_and(|i| i.point == invocation.point)
            {
                end += 1;
            }
            for object in 0..count {
                for step in start..end {
                    result.push((step, object));
                }
            }
            start = end;
        } else {
            let mut objects: Vec<_> = (0..count).collect();
            objects.sort_by(|a, b| {
                blended(start, *a).cmp(&blended(start, *b)).then_with(|| {
                    if blended(start, *a) {
                        depth(start, *b).total_cmp(&depth(start, *a))
                    } else {
                        a.cmp(b)
                    }
                })
            });
            result.extend(objects.into_iter().map(|object| (start, object)));
            start += 1;
        }
    }
    result
}

fn validate_invocations(steps: &[ManifestRecipeStep]) -> Result<(), RuntimeError> {
    let mut points = BTreeSet::new();
    let mut previous = None;
    let mut ordinals = BTreeMap::new();
    for step in steps {
        let Some(invocation) = &step.invocation else {
            previous = step.transparent_queue.as_ref();
            continue;
        };
        if step.domain != "mesh" || invocation.operation.is_empty() || invocation.point.is_empty() {
            return Err(invalid("invalid raster style invocation metadata"));
        }
        let boundary = step.transparent_queue.as_ref().unwrap_or(&invocation.point);
        if previous != Some(boundary) && !points.insert(boundary) {
            return Err(invalid(
                "style integration point must form one contiguous view boundary",
            ));
        }
        let key = (&invocation.point, &invocation.material);
        if ordinals
            .insert(key, invocation.ordinal)
            .is_some_and(|old| old >= invocation.ordinal)
        {
            return Err(invalid(
                "style operations must retain increasing per-material invocation order",
            ));
        }
        previous = Some(boundary);
    }
    Ok(())
}

#[cfg(test)]
mod invocation_tests {
    use super::*;
    #[test]
    fn transparent_queue_sorts_ordinary_and_contributed_draws_together() {
        let mut steps = vec![
            step("opaque", None),
            step("ordinary", None),
            step("shell0", Some(0)),
            step("shell1", Some(1)),
            step("present", None),
        ];
        for step in &mut steps[1..4] {
            step.transparent_queue = Some("transparency".into());
            step.colors.insert(0, "color".into());
            step.depth = Some("depth".into());
            for attachment in ["color", "depth"] {
                step.attachments.insert(
                    attachment.into(),
                    fresco_artifact::ManifestAttachmentOps {
                        load: true,
                        store: true,
                    },
                );
            }
        }
        fresco_artifact::validate_transparent_queues(&steps).unwrap();
        validate_invocations(&steps).unwrap();
        let order = scene_order(
            &steps,
            3,
            |step, _| (1..4).contains(&step),
            |_, object| [1.0, 3.0, 3.0][object],
        );
        assert_eq!(
            &order[3..12],
            &[
                (1, 1),
                (2, 1),
                (3, 1),
                (1, 2),
                (2, 2),
                (3, 2),
                (1, 0),
                (2, 0),
                (3, 0)
            ]
        );
        assert!(order[..3].iter().all(|(step, _)| *step == 0));
        assert!(order[12..].iter().all(|(step, _)| *step == 4));
        for mutation in 0..4 {
            let mut invalid = steps.clone();
            match mutation {
                0 => invalid[2].transparent_queue = None,
                1 => invalid[2].depth = Some("other_depth".into()),
                2 => invalid[2].attachments.get_mut("color").unwrap().load = false,
                3 => invalid[2].after.push("ordinary".into()),
                _ => unreachable!(),
            }
            assert!(fresco_artifact::validate_transparent_queues(&invalid).is_err());
        }
    }

    #[test]
    fn transparent_sort_uses_each_contribution_position() {
        let mut steps = [
            step("ordinary", None),
            step("first", Some(0)),
            step("second", Some(1)),
        ];
        for step in &mut steps {
            step.transparent_queue = Some("queue".into());
        }
        let order = scene_order(
            &steps,
            2,
            |_, _| true,
            |step, object| [[1.0, 2.0], [4.0, 1.0], [3.0, 5.0]][step][object],
        );
        assert_eq!(order, [(2, 1), (1, 0), (2, 0), (0, 1), (0, 0), (1, 1)]);
    }

    #[test]
    fn culled_writers_do_not_release_consumers_before_incoming_work() {
        let opaque = step("opaque", None);
        let mut shell = step("culled_shell", Some(0));
        shell.after.push("opaque".into());
        let mut empty_after = step("empty_after", None);
        empty_after.after.push("culled_shell".into());
        let steps = [opaque, shell, empty_after];
        let mut completed = BTreeSet::new();
        assert!(completed_boundaries(&steps, &[1, 0, 0], &mut completed).is_empty());
        assert_eq!(
            completed_boundaries(&steps, &[0, 0, 0], &mut completed),
            ["opaque", "culled_shell", "empty_after"]
        );
        assert!(completed_boundaries(&steps, &[0, 0, 0], &mut completed).is_empty());
    }

    fn step(name: &str, ordinal: Option<u32>) -> ManifestRecipeStep {
        ManifestRecipeStep {
            transparent_queue: None,
            invocation: ordinal.map(|ordinal| fresco_artifact::ManifestStyleInvocation {
                bounds: None,
                sort_geometry: None,
                host: None,
                generated_vertices: None,
                compute_inputs: BTreeMap::new(),
                preparation: None,
                point: "Response::finish".into(),
                material: "item".into(),
                operation: "Paint".into(),
                ordinal,
            }),
            name: name.into(),
            pass: name.into(),
            domain: "mesh".into(),
            vertex: Some("project".into()),
            entry: "paint".into(),
            bindings: BTreeMap::new(),
            colors: BTreeMap::new(),
            depth: None,
            dispatch_scale: [1, 1],
            capacity: BTreeMap::new(),
            vertex_count: 3,
            after: vec![],
            condition: None,
            attachments: BTreeMap::new(),
        }
    }
    #[test]
    fn operations_compose_per_occurrence_between_complete_view_phases() {
        let steps = [
            step("opaque", None),
            step("paint", Some(0)),
            step("outline", Some(1)),
            step("transparent", None),
        ];
        let order = scene_order(&steps, 3, |step, _| step == 3, |_, i| i as f32);
        assert_eq!(
            order,
            [
                (0, 0),
                (0, 1),
                (0, 2),
                (1, 0),
                (2, 0),
                (1, 1),
                (2, 1),
                (1, 2),
                (2, 2),
                (3, 2),
                (3, 1),
                (3, 0)
            ]
        );
        assert_eq!(
            order,
            scene_order(&steps, 3, |step, _| step == 3, |_, i| i as f32)
        );
    }
}

#[cfg(test)]
mod format_tests {
    #[test]
    fn every_registered_format_has_an_engine_representation() {
        use fresco_artifact::types::ImageFormat;
        for registered in ImageFormat::ALL {
            let info = registered.info();
            let format = super::format(info.name).unwrap();
            if let Some(bytes) = info.texel_bytes {
                assert_eq!(
                    format.block_copy_size(None),
                    Some(u32::from(bytes)),
                    "{}",
                    info.name
                );
            }
        }
        assert!(super::format("made_up_format").is_err());
    }
}
