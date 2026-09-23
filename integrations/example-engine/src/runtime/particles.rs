//! Shared particle renderer: authored simulation, material resources, and drawing.
use super::technique::{Executor, Invocation, Parameters, Pipeline};
use super::{
    RuntimeError,
    material_resources::MaterialResources,
    particle_bindings::ParticleBindings,
    particle_buffers::ParticleBuffers,
    particle_compute::ParticleCompute,
    particle_draw::ParticleDraw,
    particle_layout::ParticleLimits,
    particle_playback::{ParticlePlayback, ParticleStep},
    resources::Resource,
    textures::TextureInputs,
};
use crate::profile::mesh::{MeshSceneInputs, SCENE_BYTES};
use crate::runtime::particle_contract::ParticleContract;
use fresco_artifact::{
    ManifestRoot, ManifestSurfaceSettings, ManifestTechnique, ManifestTechniqueOperation,
};
use std::collections::BTreeMap;

pub struct ParticleInputs<'a> {
    pub textures: &'a TextureInputs,
    pub parameters: Option<&'a serde_json::Map<String, serde_json::Value>>,
}

#[derive(Clone)]
struct Source {
    wgsl: String,
    technique: ManifestTechnique,
    contract: ParticleContract,
    settings: ManifestSurfaceSettings,
    format: wgpu::TextureFormat,
}

struct System {
    buffers: ParticleBuffers,
    bindings: ParticleBindings,
    executor: Executor,
}

impl System {
    async fn prepare(
        device: &wgpu::Device,
        source: &Source,
        buffers: ParticleBuffers,
        scene: &wgpu::Buffer,
        resources: &BTreeMap<u32, Vec<(u32, Resource)>>,
    ) -> Result<Self, RuntimeError> {
        let bindings = ParticleBindings::prepare_resources(
            device,
            &source.contract,
            &buffers,
            scene,
            resources,
        )
        .await?;
        let layouts = |stage: &super::particle_bindings::ParticleStageBindings| {
            let highest = stage.layouts().keys().next_back().copied().unwrap_or(0);
            (0..=highest)
                .map(|g| stage.layouts().get(&g).cloned())
                .collect::<Vec<_>>()
        };
        let compute_layouts = layouts(&bindings.compute);
        let compute_refs: Vec<_> = compute_layouts.iter().map(Option::as_ref).collect();
        let compute =
            ParticleCompute::prepare(device, &source.wgsl, &source.contract, &compute_refs).await?;
        let draw_layouts = layouts(&bindings.draw);
        let draw_refs: Vec<_> = draw_layouts.iter().map(Option::as_ref).collect();
        let draw = ParticleDraw::prepare(
            device,
            &source.wgsl,
            &source.contract,
            &source.settings,
            &draw_refs,
            source.format,
        )
        .await?;
        let (spawn, update) = compute.pipelines();
        let pipelines = source
            .technique
            .steps
            .iter()
            .map(|step| {
                let pipeline = match &step.operation {
                    ManifestTechniqueOperation::Compute { entry, .. }
                        if entry == &source.contract.spawn_entry =>
                    {
                        Pipeline::Compute(spawn.clone())
                    }
                    ManifestTechniqueOperation::Compute { entry, .. }
                        if entry == &source.contract.compute_entry =>
                    {
                        Pipeline::Compute(update.clone())
                    }
                    ManifestTechniqueOperation::Draw { .. } => Pipeline::Draw(draw.pipeline()),
                    _ => unreachable!("particle contract validates lifecycle entries"),
                };
                (step.name.clone(), pipeline)
            })
            .collect();
        let executor = Executor::new(source.technique.clone(), pipelines, &device.limits())?;
        Ok(Self {
            buffers,
            bindings,
            executor,
        })
    }
}

pub struct ParticleRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    source: Source,
    scene: wgpu::Buffer,
    material: MaterialResources,
    playback: ParticlePlayback,
    system: System,
    environment: Option<(super::mesh::MeshRenderer, [f32; 16])>,
}

impl ParticleRenderer {
    pub async fn prepare(
        device: wgpu::Device,
        queue: wgpu::Queue,
        wgsl: &str,
        manifest: &ManifestRoot,
        entry: &str,
        inputs: ParticleInputs<'_>,
        format: wgpu::TextureFormat,
    ) -> Result<Self, RuntimeError> {
        super::validate_artifact(manifest)?;
        let invalid = |reason: &str| RuntimeError::ParticleResources(reason.into());
        let mut entries = manifest.surfaces.iter().filter(|s| s.name == entry);
        let surface = entries
            .next()
            .ok_or_else(|| invalid("particle surface is missing"))?;
        if entries.next().is_some() {
            return Err(invalid("particle surface is ambiguous"));
        }
        let contract = ParticleContract::for_surface(manifest, entry)
            .map_err(|e| invalid(&e))?
            .ok_or_else(|| invalid("authored particle technique is missing"))?;
        let settings = surface
            .settings
            .clone()
            .ok_or_else(|| invalid("particle material settings are missing"))?;
        let playback = ParticlePlayback::new(&contract, ParticleLimits::from(&device.limits()))
            .map_err(invalid)?;
        let occupied: Vec<_> = contract
            .bindings
            .iter()
            .map(|b| (b.group, b.binding))
            .collect();
        let material = MaterialResources::prepare(
            &device,
            &queue,
            surface,
            &occupied,
            inputs.textures,
            inputs.parameters,
        )
        .await?;
        let buffers = ParticleBuffers::prepare(&device, &contract, contract.particle_count).await?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let scene = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle scene"),
            size: SCENE_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        let source = Source {
            wgsl: wgsl.into(),
            technique: manifest
                .techniques
                .iter()
                .find(|t| {
                    t.surface.as_deref() == Some(entry)
                        && t.metadata.get("engine").is_some_and(|v| v == "particle")
                })
                .expect("validated particle technique")
                .clone(),
            contract,
            settings,
            format,
        };
        let system =
            System::prepare(&device, &source, buffers, &scene, &material.resources).await?;
        Ok(Self {
            device,
            queue,
            source,
            scene,
            material,
            playback,
            system,
            environment: None,
        })
    }

    /// Install scene content rendered before the particle draw using the same camera.
    pub fn set_environment(&mut self, renderer: super::mesh::MeshRenderer, model: [f32; 16]) {
        self.environment = Some((renderer, model));
    }

    pub fn supports_lighting_environment(&self) -> bool {
        self.environment
            .as_ref()
            .is_some_and(|(renderer, _)| renderer.supports_lighting_environment())
    }

    pub fn set_lighting_environment(
        &mut self,
        environment: super::forward_plus::LightingEnvironment,
    ) -> Result<(), RuntimeError> {
        let (renderer, _) = self.environment.as_mut().ok_or_else(|| {
            RuntimeError::ParticleResources("particle preview has no scene environment".into())
        })?;
        renderer.set_lighting_environment(environment)
    }

    /// Inspect the scene buffers rendered before the particle overlay.
    pub fn buffer_views(&self) -> &[super::buffer_views::BufferView] {
        match &self.environment {
            Some((renderer, _)) => renderer.buffer_views(),
            None => &[],
        }
    }

    pub fn set_buffer_view(&self, id: &str) -> Result<(), RuntimeError> {
        let (renderer, _) = self.environment.as_ref().ok_or_else(|| {
            RuntimeError::ParticleResources("particle preview has no scene environment".into())
        })?;
        renderer.set_buffer_view(id)
    }

    pub fn parameter_values(&self) -> serde_json::Map<String, serde_json::Value> {
        self.material.values()
    }
    pub fn update_parameters(
        &mut self,
        updates: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        self.material.update_parameters(&self.queue, updates)
    }
    pub fn reset(&mut self) -> Result<(), RuntimeError> {
        self.playback
            .reset()
            .map_err(|e| RuntimeError::ParticleResources(e.into()))
    }
    /// Copy the currently submitted particle state into an independently owned
    /// readback buffer. Hosts map it asynchronously and drive device polling on
    /// native platforms. Later simulation or replacement cannot change the copy.
    pub fn copy_state_for_readback(&self) -> wgpu::Buffer {
        let source = self.system.buffers.state();
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle state readback"),
            size: source.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
        self.queue.submit([encoder.finish()]);
        staging
    }

    pub fn pool(&self) -> Option<&super::particle_pool::ParticlePool> {
        self.playback.pool()
    }

    pub fn capacity(&self) -> u32 {
        self.system.buffers.layout().capacity
    }

    /// Stage an owned frame without retaining a renderer borrow across GPU waits.
    pub fn begin_frame(
        &self,
        inputs: &MeshSceneInputs,
    ) -> Result<Option<PendingParticleFrame>, RuntimeError> {
        if inputs.frame.physical_size.contains(&0) {
            return Ok(None);
        }
        let scene = inputs.pack()?;
        let step = self
            .playback
            .begin_step(inputs.frame.delta_time)
            .map_err(|e| RuntimeError::ParticleResources(e.into()))?;
        let growth = (step.layout().capacity != self.capacity()).then(|| Growth {
            device: self.device.clone(),
            queue: self.queue.clone(),
            source: self.source.clone(),
            buffers: self.system.buffers.clone(),
            scene: self.scene.clone(),
            resources: self.material.resources.clone(),
        });
        Ok(Some(PendingParticleFrame {
            step,
            inputs: *inputs,
            scene,
            growth,
        }))
    }

    /// Convenience for serialized native/offscreen hosts.
    pub async fn render(
        &mut self,
        inputs: &MeshSceneInputs,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> Result<bool, RuntimeError> {
        let Some(pending) = self.begin_frame(inputs)? else {
            return Ok(false);
        };
        self.render_prepared(pending.prepare().await?, color, depth)
    }

    /// Submit a prepared frame synchronously. Stale/foreign frames enqueue no writes.
    pub fn render_prepared(
        &mut self,
        prepared: PreparedParticleFrame,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> Result<bool, RuntimeError> {
        let PreparedParticleFrame {
            step,
            inputs,
            scene,
            candidate,
        } = prepared;
        let invalid = |e: &str| RuntimeError::ParticleResources(e.into());
        self.playback.validate_step(&step).map_err(invalid)?;
        self.material.update_frame(&self.queue, inputs.frame)?;
        let system = candidate.as_ref().unwrap_or(&self.system);
        system
            .buffers
            .write_step(&self.queue, step.delta(), step.commands())?;
        self.queue.write_buffer(&self.scene, 0, &scene);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let parameters = Parameters {
            extents: BTreeMap::from([("capacity".into(), [step.layout().capacity, 1, 1])]),
            counts: BTreeMap::from([("capacity".into(), step.layout().capacity)]),
            enabled: BTreeMap::from([
                ("spawn".into(), step.spawn()),
                ("update".into(), step.update()),
            ]),
        };
        let scene_depth = if let Some((environment, model)) = &mut self.environment {
            let mut scene_inputs = inputs;
            scene_inputs.model = *model;
            scene_inputs.displacement.enabled = false;
            environment.render(&scene_inputs, color, depth)?;
            environment.scene_depth()?
        } else {
            None
        };
        let depth = scene_depth.as_ref().unwrap_or(depth);
        let mut attachments = BTreeMap::new();
        let groups = self
            .source
            .technique
            .steps
            .iter()
            .map(|node| {
                let groups = match &node.operation {
                    ManifestTechniqueOperation::Compute { .. } => system.bindings.compute.groups(),
                    ManifestTechniqueOperation::Draw {
                        colors,
                        depth: target_depth,
                        ..
                    } => {
                        for name in colors.values() {
                            attachments.insert(name.clone(), color);
                        }
                        if let Some(name) = target_depth {
                            attachments.insert(name.clone(), depth);
                        }
                        system.bindings.draw.groups()
                    }
                };
                (node.name.clone(), groups)
            })
            .collect();
        let initialized = if self.environment.is_some() {
            attachments.keys().cloned().collect()
        } else {
            Default::default()
        };
        system.executor.encode_over(
            &mut encoder,
            Invocation {
                parameters: &parameters,
                groups,
                attachments,
                geometry: BTreeMap::new(),
            },
            initialized,
        )?;
        self.queue.submit([encoder.finish()]);
        self.playback.commit(step).map_err(invalid)?;
        if let Some(candidate) = candidate {
            self.system = candidate;
        }
        Ok(true)
    }
}

struct Growth {
    device: wgpu::Device,
    queue: wgpu::Queue,
    source: Source,
    buffers: ParticleBuffers,
    scene: wgpu::Buffer,
    resources: BTreeMap<u32, Vec<(u32, Resource)>>,
}

pub struct PendingParticleFrame {
    step: ParticleStep,
    inputs: MeshSceneInputs,
    scene: [u8; SCENE_BYTES],
    growth: Option<Growth>,
}

pub struct PreparedParticleFrame {
    step: ParticleStep,
    inputs: MeshSceneInputs,
    scene: [u8; SCENE_BYTES],
    candidate: Option<System>,
}

impl PendingParticleFrame {
    pub async fn prepare(self) -> Result<PreparedParticleFrame, RuntimeError> {
        let candidate = if let Some(growth) = self.growth {
            let buffers = growth
                .buffers
                .resized(&growth.device, &growth.queue, self.step.layout().capacity)
                .await?;
            Some(
                System::prepare(
                    &growth.device,
                    &growth.source,
                    buffers,
                    &growth.scene,
                    &growth.resources,
                )
                .await?,
            )
        } else {
            None
        };
        Ok(PreparedParticleFrame {
            step: self.step,
            inputs: self.inputs,
            scene: self.scene,
            candidate,
        })
    }
}
