//! Execute the bundled mesh profile's authored stages using prepared geometry.
use std::collections::BTreeMap;

use fresco_artifact::ManifestRoot;
use wgpu::util::DeviceExt;

use super::{
    RuntimeError, material_resources::MaterialResources, mesh_geometry::MeshGeometry,
    resources::Resource, textures::TextureInputs,
};
use crate::profile::mesh::{MeshSceneInputs, SCENE_BYTES};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Inputs owned or borrowed only while preparing a replacement renderer.
pub struct FactoryInputs<'a> {
    pub name: &'a str,
    /// Additional factory buffers keyed by their authored resource names.
    pub buffers: &'a BTreeMap<String, wgpu::Buffer>,
}

pub struct MeshResources<'a> {
    pub factory: Option<FactoryInputs<'a>>,
    pub geometry: MeshGeometry,
    pub textures: &'a TextureInputs,
    /// Install edited parameters together with replacement textures or a device.
    pub parameters: Option<&'a serde_json::Map<String, serde_json::Value>>,
}

struct MeshGroup {
    draw_data: BTreeMap<u32, fresco_artifact::ManifestDrawData>,
    compute: BTreeMap<u32, fresco_artifact::ManifestDrawComputeBinding>,
    group: u32,
    layout: wgpu::BindGroupLayout,
    resources: Vec<(u32, Resource)>,
    images: BTreeMap<u32, String>,
    cached: Option<wgpu::BindGroup>,
}

struct PreparedFrame {
    groups: BTreeMap<String, Vec<(u32, wgpu::BindGroup)>>,
    compute: super::compute_frame::ComputeFrame,
}

fn draw_compute_inputs(
    pass: &fresco_artifact::ManifestMeshPass,
    invocation: Option<&fresco_artifact::ManifestStyleInvocation>,
) -> Result<BTreeMap<String, fresco_artifact::ManifestDrawComputeBinding>, &'static str> {
    let mut inputs = pass.shading_inputs.clone();
    if let Some(invocation) = invocation {
        for (name, input) in &invocation.compute_inputs {
            if inputs.insert(name.clone(), input.clone()).is_some() {
                return Err("shading and operation inputs bind the same resource");
            }
        }
    }
    Ok(inputs)
}

pub struct MeshRenderer {
    factory: String,
    local_bounds: Option<super::bounds::Bounds>,
    world_bounds: Option<super::bounds::Bounds>,
    compute: super::compute_frame::PreparedCompute,
    compute_geometry: BTreeMap<String, super::compute_frame::GeometryInputs>,
    compute_draws: BTreeMap<String, std::collections::BTreeSet<String>>,
    compute_budget: u64,
    device: wgpu::Device,
    queue: wgpu::Queue,
    executor: super::recipe::Executor,
    geometry: MeshGeometry,
    groups: BTreeMap<String, Vec<MeshGroup>>,
    lighting_supported: bool,
    buffer_views: Vec<super::buffer_views::BufferView>,
    group_size: [u32; 2],
    material: MaterialResources,
    style_parameters: super::style_parameters::StyleParameters,
    objects: Vec<(Self, [f32; 16])>,
    lighting: super::forward_plus::SceneLighting,
    shader_source: std::sync::Arc<str>,
    preparations: Vec<(String, wgpu::ComputePipeline, u32)>,
}

impl MeshRenderer {
    /// Prepare a material without texture inputs, using authored defaults.
    pub async fn prepare(
        device: wgpu::Device,
        queue: wgpu::Queue,
        wgsl: &str,
        manifest: &ManifestRoot,
        entry: &str,
        geometry: MeshGeometry,
        output_format: wgpu::TextureFormat,
    ) -> Result<Self, RuntimeError> {
        Self::prepare_with_resources(
            device,
            queue,
            wgsl,
            manifest,
            entry,
            MeshResources {
                factory: None,
                geometry,
                textures: &TextureInputs::new(),
                parameters: None,
            },
            output_format,
        )
        .await
    }

    /// Prepare an independent candidate including textures and edited inputs.
    /// Failure leaves every resource in the installed renderer unchanged.
    pub async fn prepare_with_resources(
        device: wgpu::Device,
        queue: wgpu::Queue,
        wgsl: &str,
        manifest: &ManifestRoot,
        entry: &str,
        resources: MeshResources<'_>,
        output_format: wgpu::TextureFormat,
    ) -> Result<Self, RuntimeError> {
        super::validate_artifact(manifest)?;
        let MeshResources {
            factory: requested_factory,
            geometry,
            textures,
            parameters: overrides,
        } = resources;
        let invalid = |reason: &str| RuntimeError::MeshContract {
            entry: entry.into(),
            reason: reason.into(),
        };
        let mut entries = manifest
            .surfaces
            .iter()
            .filter(|surface| surface.name == entry);
        let surface = entries
            .next()
            .ok_or_else(|| invalid("surface entry is missing"))?;
        if entries.next().is_some() {
            return Err(invalid("surface entry is ambiguous"));
        }
        let pass = surface
            .mesh_passes
            .first()
            .ok_or_else(|| invalid("authored mesh stages are missing"))?;
        let variant = if let Some(selection) = &requested_factory {
            Some(
                pass.variants
                    .iter()
                    .find(|v| v.factory == selection.name)
                    .ok_or_else(|| {
                        invalid("requested factory variant is absent from the artifact")
                    })?,
            )
        } else {
            None
        };
        let factory_name = variant.map_or(pass.factory.as_str(), |v| v.factory.as_str());
        let mut entries = BTreeMap::new();
        for program in &surface.mesh_passes {
            let variant = program
                .variants
                .iter()
                .find(|v| v.factory == factory_name)
                .ok_or_else(|| {
                    invalid("mesh pass does not support the selected geometry factory")
                })?;
            entries.insert(program.pass.clone(), variant.entries.as_slice());
        }
        let mut factories = manifest
            .vertex_factories
            .iter()
            .filter(|factory| factory.name == factory_name);
        let factory = factories
            .next()
            .ok_or_else(|| invalid("selected vertex factory is missing"))?;
        if factories.next().is_some() {
            return Err(invalid("vertex factory is ambiguous"));
        }
        if !geometry.supports(factory) {
            return Err(invalid(
                "geometry does not match the selected vertex layout",
            ));
        }
        let limits = device.limits();
        let plan = super::recipe::Plan::new(manifest, surface, &entries, output_format, &limits)?;
        let buffer_views = super::buffer_views::available(
            plan.steps.iter().map(|step| step.declaration.pass.as_str()),
        );
        let mesh_steps: Vec<_> = plan
            .steps
            .iter()
            .filter(|s| s.declaration.is_draw_scoped())
            .collect();
        // Reflect reachable global uses per draw, so an earlier pass can write an
        // image which a later entry of the same factory samples.
        let module =
            naga::front::wgsl::parse_str(wgsl).map_err(|e| invalid(&e.emit_to_string(wgsl)))?;
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|e| invalid(&e.to_string()))?;
        let preparation_metadata: BTreeMap<_, _> = surface
            .mesh_passes
            .iter()
            .filter_map(|p| p.preparation.as_ref().map(|m| (p.pass.clone(), m)))
            .collect();
        for pass in surface
            .mesh_passes
            .iter()
            .filter(|p| p.preparation.is_some())
        {
            let metadata = pass.preparation.as_ref().expect("preparation");
            let (entry_index, _) = module
                .entry_points
                .iter()
                .enumerate()
                .find(|(_, e)| e.name == metadata.entry && e.stage == naga::ShaderStage::Compute)
                .ok_or_else(|| invalid("missing preparation compute entry"))?;
            let binding = pass
                .bindings
                .iter()
                .find(|b| {
                    b.geometry
                        .as_ref()
                        .is_some_and(|g| g.role == "output" && g.producer == pass.pass)
                })
                .ok_or_else(|| invalid("missing preparation output binding"))?;
            let output = module
                .global_variables
                .iter()
                .find(|(handle, global)| {
                    !info.get_entry_point(entry_index)[*handle].is_empty()
                        && global.binding.as_ref().is_some_and(|slot| {
                            Some(slot.group) == binding.group_index
                                && Some(slot.binding) == binding.binding
                        })
                })
                .ok_or_else(|| invalid("preparation output is not used by its compute entry"))?
                .1;
            if !matches!(module.types[output.ty].inner, naga::TypeInner::Array { stride, .. } if stride == metadata.vertex_stride)
            {
                return Err(invalid(
                    "prepared vertex stride disagrees with shader storage layout",
                ));
            }
        }
        let mut uses = BTreeMap::new();
        let mut data_uses = BTreeMap::new();
        let mut stage_uses = BTreeMap::new();
        for step in &mesh_steps {
            let mut combined = std::collections::BTreeSet::new();
            for compute in [false, true] {
                let mut used = std::collections::BTreeSet::new();
                for (index, entry) in module.entry_points.iter().enumerate() {
                    let selected = if compute {
                        preparation_metadata
                            .get(&step.declaration.pass)
                            .is_some_and(|p| {
                                p.node == step.declaration.name && p.entry == entry.name
                            })
                    } else {
                        Some(&entry.name) == step.vertex.as_ref()
                            || Some(&entry.name) == step.entry.as_ref()
                    };
                    if selected {
                        for (handle, global) in module.global_variables.iter() {
                            if !info.get_entry_point(index)[handle].is_empty()
                                && let Some(binding) = &global.binding
                            {
                                used.insert((binding.group, binding.binding));
                                if !compute
                                    && info.get_entry_point(index)[handle]
                                        .contains(naga::valid::GlobalUse::READ)
                                {
                                    data_uses
                                        .entry(step.declaration.name.clone())
                                        .or_insert_with(std::collections::BTreeSet::new)
                                        .insert((binding.group, binding.binding));
                                }
                            }
                        }
                    }
                }
                combined.extend(used.iter().copied());
                stage_uses.insert((step.declaration.name.clone(), compute), used);
            }
            uses.insert(step.declaration.name.clone(), combined);
        }
        let lighting_supported = plan
            .recipe
            .resources
            .iter()
            .any(|r| r.source.as_deref() == Some("lighting_environment"))
            && mesh_steps.iter().any(|step| {
                factory.bindings.iter().any(|b| {
                    b.group_index
                        .zip(b.binding)
                        .is_some_and(|slot| uses[&step.declaration.name].contains(&slot))
                        && step.declaration.bindings.get(&b.name).is_some_and(|name| {
                            plan.recipe.resources.iter().any(|r| {
                                &r.name == name
                                    && r.source.as_deref() == Some("lighting_environment")
                            })
                        })
                })
            })
            || plan.steps.iter().any(|step| {
                step.program.as_ref().is_some_and(|program| {
                    program.bindings.iter().any(|binding| {
                        if !step
                            .entry
                            .iter()
                            .chain(step.vertex.iter())
                            .any(|entry| binding.entry_access.contains_key(entry))
                        {
                            return false;
                        }
                        step.declaration
                            .bindings
                            .get(&binding.name)
                            .is_some_and(|name| {
                                plan.recipe.resources.iter().any(|r| {
                                    &r.name == name
                                        && r.source.as_deref() == Some("lighting_environment")
                                })
                            })
                    })
                })
            });
        let bindings: BTreeMap<_, Vec<_>> = mesh_steps
            .iter()
            .map(|step| {
                let pass = surface
                    .mesh_passes
                    .iter()
                    .find(|p| p.pass == step.declaration.pass)
                    .expect("validated mesh pass");
                (
                    step.declaration.name.clone(),
                    factory.bindings.iter().chain(&pass.bindings).collect(),
                )
            })
            .collect();
        let owned_compute =
            super::compute_frame::PreparedCompute::new(&device, wgsl, manifest, entry).await?;
        let mut compute_draws: BTreeMap<String, std::collections::BTreeSet<String>> =
            BTreeMap::new();
        let mut draw_inputs = BTreeMap::new();
        for step in &mesh_steps {
            let pass = surface
                .mesh_passes
                .iter()
                .find(|pass| pass.pass == step.declaration.pass)
                .ok_or_else(|| invalid("missing mesh pass resource declaration"))?;
            let inputs =
                draw_compute_inputs(pass, step.declaration.invocation.as_ref()).map_err(invalid)?;
            for (name, input) in &inputs {
                let binding = bindings[&step.declaration.name]
                    .iter()
                    .find(|b| b.name == *name)
                    .ok_or_else(|| invalid("unknown compute draw binding"))?;
                if step.declaration.bindings.contains_key(name) {
                    return Err(invalid(
                        "compute draw binding also names an engine resource",
                    ));
                }
                owned_compute.binding_type(
                    input,
                    binding
                        .signature
                        .as_deref()
                        .ok_or_else(|| invalid("missing compute draw signature"))?,
                )?;
                let slot = (
                    binding
                        .group_index
                        .ok_or_else(|| invalid("missing draw group"))?,
                    binding
                        .binding
                        .ok_or_else(|| invalid("missing draw binding"))?,
                );
                if input.dimension.is_none()
                    && data_uses
                        .get(&step.declaration.name)
                        .is_some_and(|s| s.contains(&slot))
                {
                    compute_draws
                        .entry(step.declaration.name.clone())
                        .or_default()
                        .insert(input.producer.clone());
                }
            }
            draw_inputs.insert(step.declaration.name.clone(), inputs);
        }
        let mut table_records = BTreeMap::new();
        let mut supplied = std::collections::BTreeSet::new();
        for step in &mesh_steps {
            let mut step_slots = std::collections::BTreeSet::new();
            for binding in &bindings[&step.declaration.name] {
                let group = binding
                    .group_index
                    .filter(|g| *g < limits.max_bind_groups)
                    .ok_or_else(|| invalid("unsupported factory resource group"))?;
                let index = binding
                    .binding
                    .filter(|b| *b < limits.max_bindings_per_bind_group)
                    .ok_or_else(|| invalid("unsupported factory resource binding"))?;
                if !step_slots.insert((group, index)) {
                    return Err(invalid("factory resources collide"));
                }
                if !uses[&step.declaration.name].contains(&(group, index)) {
                    continue;
                }
                let wiring = &step.declaration.bindings;
                let signature = binding
                    .signature
                    .as_deref()
                    .ok_or_else(|| invalid("factory resource has no type"))?;
                if binding.sampler.is_some() {
                    if signature != "sampler"
                        || binding.draw_data.is_some()
                        || binding.source.is_some()
                        || binding.geometry.is_some()
                        || wiring.contains_key(&binding.name)
                        || draw_inputs[&step.declaration.name].contains_key(&binding.name)
                    {
                        return Err(invalid(
                            "sampler preset requires an exclusive sampler binding",
                        ));
                    }
                    continue;
                }
                if binding.draw_data.is_some() {
                    if signature != "uniform<u32>"
                        || binding.source.is_some()
                        || binding.geometry.is_some()
                        || wiring.contains_key(&binding.name)
                        || draw_inputs[&step.declaration.name].contains_key(&binding.name)
                    {
                        return Err(invalid(
                            "draw instance data requires an exclusive uniform<u32> binding",
                        ));
                    }
                } else if draw_inputs[&step.declaration.name].contains_key(&binding.name) {
                    // Producer identity, access, and layout were checked above.
                    continue;
                } else if let Some(geometry) = &binding.geometry {
                    if !preparation_metadata.contains_key(&geometry.producer)
                        || !matches!(
                            geometry.role.as_str(),
                            "raw_vertices"
                                | "raw_indices"
                                | "counts"
                                | "bounds"
                                | "vertices"
                                | "indices"
                                | "output"
                        )
                    {
                        return Err(invalid("invalid geometry producer or binding role"));
                    }
                } else if let Some(name) = &binding.source {
                    if signature != format!("uniform<{name}>") {
                        return Err(invalid(
                            "table source requires the matching uniform record type",
                        ));
                    }
                    let table = manifest
                        .tables
                        .iter()
                        .find(|t| &t.name == name)
                        .ok_or_else(|| invalid("unknown factory table source"))?;
                    let record = table
                        .records
                        .iter()
                        .find(|r| r.key == surface.name)
                        .ok_or_else(|| invalid("surface table record missing"))?;
                    if record.values.is_empty() || record.values.len() != table.fields.len() {
                        return Err(invalid("invalid table record width"));
                    }
                    let bytes: Vec<u8> =
                        record.values.iter().flat_map(|v| v.to_le_bytes()).collect();
                    let size = u64::try_from(bytes.len())
                        .map_err(|_| invalid("table record too large"))?
                        .checked_next_multiple_of(16)
                        .ok_or_else(|| invalid("table size overflow"))?;
                    if size > limits.max_uniform_buffer_binding_size {
                        return Err(invalid("table record exceeds uniform buffer limit"));
                    }
                    table_records.insert(binding.name.clone(), (size, bytes));
                } else if let Some(resource) = wiring.get(&binding.name) {
                    let preparation_read = stage_uses[&(step.declaration.name.clone(), true)]
                        .contains(&(group, index));
                    let resource = plan
                        .recipe
                        .resources
                        .iter()
                        .find(|r| &r.name == resource)
                        .ok_or_else(|| invalid("factory wiring references a missing resource"))?;
                    if preparation_read
                        && !matches!(resource.kind.as_str(), "external" | "table_data")
                    {
                        return Err(invalid(
                            "geometry preparation requires frame inputs available before renderer work",
                        ));
                    }
                    if resource.kind == "image" {
                        if !signature.starts_with("texture_") {
                            return Err(invalid("image requires a sampled texture binding"));
                        }
                        let format = super::recipe::format(
                            resource.format.as_deref().expect("validated image format"),
                        )?;
                        let compatible = match signature {
                            "texture_depth_2d" => format.is_depth_stencil_format(),
                            "texture_2d<u32>" => {
                                format.sample_type(None, None)
                                    == Some(wgpu::TextureSampleType::Uint)
                            }
                            "texture_2d<i32>" => {
                                format.sample_type(None, None)
                                    == Some(wgpu::TextureSampleType::Sint)
                            }
                            "texture_2d<f32>" => matches!(
                                format.sample_type(None, None),
                                Some(wgpu::TextureSampleType::Float { .. })
                            ),
                            _ => false,
                        };
                        if !compatible {
                            return Err(invalid(
                                "sampled texture type is incompatible with image format",
                            ));
                        }
                        if !plan
                            .steps
                            .iter()
                            .take_while(|earlier| earlier.declaration.name != step.declaration.name)
                            .any(|earlier| {
                                earlier
                                    .declaration
                                    .colors
                                    .values()
                                    .any(|name| name == &resource.name)
                                    || earlier.declaration.depth.as_ref() == Some(&resource.name)
                            })
                        {
                            return Err(invalid("mesh step reads an undefined image"));
                        }
                        if step
                            .declaration
                            .colors
                            .values()
                            .any(|name| name == &resource.name)
                            || step.declaration.depth.as_ref() == Some(&resource.name)
                        {
                            return Err(invalid("mesh step samples its active attachment"));
                        }
                    } else if matches!(resource.source.as_deref(), Some("presentation" | "depth"))
                        || !matches!(resource.kind.as_str(), "external" | "buffer" | "table_data")
                    {
                        return Err(invalid("factory resource requires a buffer"));
                    }
                } else if requested_factory
                    .as_ref()
                    .is_some_and(|f| f.buffers.contains_key(&binding.name))
                {
                    supplied.insert(binding.name.clone());
                } else {
                    return Err(invalid(&format!(
                        "missing factory resource `{}`",
                        binding.name
                    )));
                }
                if !signature.starts_with("uniform<")
                    && !signature.starts_with("buffer<")
                    && !signature.starts_with("texture_")
                {
                    return Err(invalid("unsupported factory buffer type"));
                }
            }
        }
        for step in &mesh_steps {
            for name in step.declaration.bindings.keys() {
                if !bindings[&step.declaration.name]
                    .iter()
                    .any(|b| &b.name == name)
                {
                    return Err(invalid("recipe binds an unknown factory resource"));
                }
            }
        }
        for resource in &plan.recipe.resources {
            if resource.kind == "external" {
                let source = resource
                    .source
                    .as_deref()
                    .ok_or_else(|| invalid("external resource source missing"))?;
                if !matches!(
                    source,
                    "scene"
                        | "point_lights"
                        | "style_parameters"
                        | "lighting_environment"
                        | "scene_background"
                        | "buffer_view"
                        | "resolve_camera"
                        | "shadow_camera"
                        | "presentation"
                        | "depth"
                ) {
                    if !requested_factory
                        .as_ref()
                        .is_some_and(|f| f.buffers.contains_key(source))
                    {
                        return Err(invalid(&format!("missing engine input `{source}`")));
                    }
                    supplied.insert(source.into());
                }
            }
        }
        if requested_factory
            .as_ref()
            .is_some_and(|f| f.buffers.keys().any(|name| !supplied.contains(name)))
        {
            return Err(invalid(
                "a supplied buffer is not an additional resource of this factory",
            ));
        }
        let mut style_parameters = super::style_parameters::StyleParameters::new(surface)?;
        let style_names = style_parameters.values();
        let (style_overrides, material_overrides): (serde_json::Map<_, _>, serde_json::Map<_, _>) =
            overrides
                .into_iter()
                .flat_map(|o| o.iter())
                .map(|(k, v)| (k.clone(), v.clone()))
                .partition(|(k, _)| style_names.contains_key(k));
        style_parameters.update(&style_overrides)?;
        let style_bytes = super::style_parameters::StyleParameters::initial_bytes(
            manifest,
            device
                .limits()
                .max_storage_buffer_binding_size
                .min(device.limits().max_buffer_size),
        )?;
        let material = MaterialResources::prepare(
            &device,
            &queue,
            surface,
            // Pass bindings occupy their own entry-point layouts, not the
            // material's layout. Naga validation above rejects simultaneous
            // shader uses of distinct globals at the same binding.
            &[],
            textures,
            Some(&material_overrides),
        )
        .await?;
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let make_buffer = |size, uniform, label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: (if uniform {
                    wgpu::BufferUsages::UNIFORM
                } else {
                    wgpu::BufferUsages::STORAGE
                }) | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let mut graph_resources = BTreeMap::new();
        for resource in &plan.recipe.resources {
            let (size, uniform, initial) = match resource.kind.as_str() {
                "buffer" => (
                    u64::from(resource.bytes.expect("validated size")),
                    false,
                    None,
                ),
                "table_data" => {
                    let bytes = plan.table_bytes[&resource.name].clone();
                    (bytes.len() as u64, false, Some(bytes))
                }
                "external" => match resource.source.as_deref().expect("validated source") {
                    "style_parameters" => {
                        (style_bytes.len() as u64, false, Some(style_bytes.clone()))
                    }
                    "scene" => (SCENE_BYTES as u64, true, None),
                    "point_lights" => {
                        let bytes =
                            super::forward_plus::pack_lights(&[]).expect("valid demo lights");
                        (bytes.len() as u64, false, Some(bytes))
                    }
                    "resolve_camera" => (96, true, None),
                    "shadow_camera" => (64, true, None),
                    "buffer_view" => (16, true, Some(vec![0; 16])),
                    "scene_background" => (32, true, Some(vec![0; 32])),
                    "lighting_environment" => (
                        80,
                        true,
                        Some(
                            super::forward_plus::LightingEnvironment::Preview
                                .bytes()
                                .to_vec(),
                        ),
                    ),
                    "presentation" | "depth" => continue,
                    source => {
                        let buffer = requested_factory
                            .as_ref()
                            .expect("validated supplied input")
                            .buffers[source]
                            .clone();
                        graph_resources.insert(
                            resource.name.clone(),
                            Resource::Storage {
                                min_size: buffer.size(),
                                buffer,
                            },
                        );
                        continue;
                    }
                },
                "image" => {
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some(&resource.name),
                        size: wgpu::Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: super::recipe::format(
                            resource.format.as_deref().expect("validated format"),
                        )?,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    });
                    graph_resources.insert(
                        resource.name.clone(),
                        Resource::Texture(texture.create_view(&Default::default())),
                    );
                    continue;
                }
                _ => unreachable!("validated recipe resource"),
            };
            let buffer = make_buffer(size, uniform, &resource.name);
            if let Some(bytes) = initial {
                queue.write_buffer(&buffer, 0, &bytes);
            }
            graph_resources.insert(
                resource.name.clone(),
                if uniform {
                    Resource::Uniform(buffer)
                } else {
                    Resource::Storage {
                        buffer,
                        min_size: size,
                    }
                },
            );
        }
        let mut prepared_streams = BTreeMap::new();
        let mut prepared_resources = BTreeMap::new();
        for (name, metadata) in &preparation_metadata {
            let (stream, resources) = geometry.prepare_stream(&device, &queue, metadata)?;
            prepared_streams.insert(name.clone(), stream);
            prepared_resources.insert(name.clone(), resources);
        }
        let mut prepared_steps = BTreeMap::new();
        for step in &mesh_steps {
            let pass = surface
                .mesh_passes
                .iter()
                .find(|p| p.pass == step.declaration.pass)
                .expect("validated pass");
            let source = pass
                .prepared_source
                .as_ref()
                .or_else(|| pass.preparation.as_ref().map(|_| &pass.pass));
            if let Some(source) = source {
                let stream = prepared_streams
                    .get(source)
                    .ok_or_else(|| invalid("prepared draw has no active producer"))?;
                prepared_steps.insert(step.declaration.name.clone(), stream.clone());
            }
        }
        let mut step_groups = BTreeMap::new();
        let mut step_layouts = BTreeMap::new();
        for step in &mesh_steps {
            for compute in [false, true] {
                if compute
                    && !preparation_metadata
                        .get(&step.declaration.pass)
                        .is_some_and(|p| p.node == step.declaration.name)
                {
                    continue;
                }
                let key = if compute {
                    format!("prepare::{}", step.declaration.name)
                } else {
                    step.declaration.name.clone()
                };
                let wiring = &step.declaration.bindings;
                let mut resources = material.resources.clone();
                // A pass-local binding may reuse a material slot when that
                // material global is not used by the pass (e.g. a procedural
                // resolve following a parameterized mesh draw). Keep only the
                // material resources owned by this entry's layout before
                // inserting its explicit bindings below.
                let pass_slots: std::collections::BTreeSet<_> = bindings[&step.declaration.name]
                    .iter()
                    .map(|binding| {
                        (
                            binding.group_index.expect("validated group"),
                            binding.binding.expect("validated binding"),
                        )
                    })
                    .collect();
                for (group, values) in &mut resources {
                    values.retain(|(binding, _)| !pass_slots.contains(&(*group, *binding)));
                }
                let mut images = BTreeMap::new();
                let mut texture_types = BTreeMap::new();
                let mut dynamic_inputs = BTreeMap::new();
                let mut draw_data = BTreeMap::new();
                for binding in &bindings[&step.declaration.name] {
                    let slot = (
                        binding.group_index.expect("validated group"),
                        binding.binding.expect("validated binding"),
                    );
                    if !stage_uses[&(step.declaration.name.clone(), compute)].contains(&slot) {
                        continue;
                    }
                    let signature = binding.signature.as_deref().expect("validated signature");
                    if let Some(data) = binding.draw_data {
                        draw_data.insert(slot, data);
                        resources.entry(slot.0).or_default();
                        continue;
                    }
                    if let Some(input) = draw_inputs[&step.declaration.name].get(&binding.name) {
                        let ty = owned_compute.binding_type(input, signature)?;
                        dynamic_inputs.insert(slot, (input.clone(), ty));
                        resources.entry(slot.0).or_default();
                        continue;
                    }
                    let value = if let Some(preset) = binding.sampler {
                        super::resources::sampler_preset(&device, preset)
                    } else if let Some(geometry) = &binding.geometry {
                        prepared_resources[&geometry.producer][&geometry.role].clone()
                    } else if let Some((size, bytes)) = table_records.get(&binding.name) {
                        let buffer = make_buffer(*size, true, &binding.name);
                        queue.write_buffer(&buffer, 0, bytes);
                        Resource::Uniform(buffer)
                    } else if let Some(name) = wiring.get(&binding.name) {
                        images.insert(slot, name.clone());
                        if signature.starts_with("texture_") {
                            images.insert(slot, name.clone());
                            texture_types.insert(
                                slot,
                                match signature {
                                    "texture_depth_2d" => wgpu::TextureSampleType::Depth,
                                    "texture_2d<u32>" => wgpu::TextureSampleType::Uint,
                                    "texture_2d<i32>" => wgpu::TextureSampleType::Sint,
                                    "texture_2d<f32>" => {
                                        wgpu::TextureSampleType::Float { filterable: false }
                                    }
                                    _ => return Err(invalid("unsupported sampled texture type")),
                                },
                            );
                        }
                        match &graph_resources[name] {
                            Resource::Uniform(buffer) | Resource::Storage { buffer, .. }
                                if signature.starts_with("uniform<") =>
                            {
                                Resource::Uniform(buffer.clone())
                            }
                            resource => resource.clone(),
                        }
                    } else {
                        let buffer = requested_factory
                            .as_ref()
                            .expect("validated factory input")
                            .buffers[&binding.name]
                            .clone();
                        if signature.starts_with("uniform<") {
                            Resource::Uniform(buffer)
                        } else {
                            Resource::Storage {
                                min_size: buffer.size(),
                                buffer,
                            }
                        }
                    };
                    resources.entry(slot.0).or_default().push((slot.1, value));
                }
                let mut layouts = BTreeMap::new();
                let mut groups = Vec::new();
                for (&group, buffers) in &resources {
                    let mut entries: Vec<_> = buffers
                        .iter()
                        .map(|(binding, buffer)| wgpu::BindGroupLayoutEntry {
                            binding: *binding,
                            visibility: if compute {
                                wgpu::ShaderStages::COMPUTE
                            } else {
                                wgpu::ShaderStages::VERTEX_FRAGMENT
                            },
                            ty: texture_types.get(&(group, *binding)).map_or_else(
                                || {
                                    let mut ty = buffer.binding_type();
                                    if bindings[&step.declaration.name].iter().any(|b| {
                                        b.group_index == Some(group)
                                            && b.binding == Some(*binding)
                                            && b.geometry
                                                .as_ref()
                                                .is_some_and(|g| g.role == "output")
                                    }) && let wgpu::BindingType::Buffer {
                                        ty: ref mut kind, ..
                                    } = ty
                                    {
                                        *kind =
                                            wgpu::BufferBindingType::Storage { read_only: false };
                                    }
                                    ty
                                },
                                |sample_type| wgpu::BindingType::Texture {
                                    sample_type: *sample_type,
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                            ),
                            count: None,
                        })
                        .collect();
                    entries.extend(dynamic_inputs.iter().filter(|((g, _), _)| *g == group).map(
                        |((_, binding), (_, ty))| wgpu::BindGroupLayoutEntry {
                            binding: *binding,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: *ty,
                            count: None,
                        },
                    ));
                    entries.extend(draw_data.keys().filter(|(g, _)| *g == group).map(
                        |(_, binding)| wgpu::BindGroupLayoutEntry {
                            binding: *binding,
                            visibility: if compute {
                                wgpu::ShaderStages::COMPUTE
                            } else {
                                wgpu::ShaderStages::VERTEX_FRAGMENT
                            },
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: std::num::NonZeroU64::new(4),
                            },
                            count: None,
                        },
                    ));
                    let layout =
                        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: None,
                            entries: &entries,
                        });
                    groups.push(MeshGroup {
                        draw_data: draw_data
                            .iter()
                            .filter(|((g, _), _)| *g == group)
                            .map(|((_, binding), data)| (*binding, *data))
                            .collect(),
                        compute: dynamic_inputs
                            .iter()
                            .filter(|((g, _), _)| *g == group)
                            .map(|((_, b), (input, _))| (*b, input.clone()))
                            .collect(),
                        group,
                        layout: layout.clone(),
                        resources: buffers.clone(),
                        cached: None,
                        images: images
                            .iter()
                            .filter(|((g, _), _)| *g == group)
                            .map(|((_, b), name)| (*b, name.clone()))
                            .collect(),
                    });
                    layouts.insert(group, layout);
                }
                let layout_refs: Vec<_> =
                    layouts.keys().next_back().map_or_else(Vec::new, |highest| {
                        (0..=*highest).map(|group| layouts.get(&group)).collect()
                    });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("authored mesh layout"),
                    bind_group_layouts: &layout_refs,
                    immediate_size: 0,
                });
                step_groups.insert(key.clone(), groups);
                step_layouts.insert(key, layout);
            }
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(entry),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let mut preparations = Vec::new();
        for (name, metadata) in &preparation_metadata {
            let key = format!("prepare::{}", metadata.node);
            let layout = step_layouts
                .get(&key)
                .ok_or_else(|| invalid("preparation node is not active"))?;
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(layout),
                module: &shader,
                entry_point: Some(&metadata.entry),
                compilation_options: Default::default(),
                cache: None,
            });
            preparations.push((key, pipeline, prepared_streams[name].preparation_groups()));
        }
        let executor = super::recipe::Executor::new(
            &device,
            &shader,
            plan,
            graph_resources,
            &step_layouts,
            &geometry,
            prepared_steps,
        )?;
        let allocation_result = allocation.pop();
        let validation_result = validation.pop();
        let allocation_error = allocation_result.await;
        let validation_error = validation_result.await;
        if let Some(error) = allocation_error.or(validation_error) {
            return Err(RuntimeError::GpuValidation(error.to_string()));
        }
        Ok(Self {
            factory: factory_name.into(),
            local_bounds: if factory_name == "preview_static"
                && surface.surface_requirements.vertex_stage.is_none()
            {
                geometry.source_bounds("position")?
            } else {
                None
            },
            world_bounds: None,
            compute: owned_compute,
            compute_geometry: prepared_resources
                .into_iter()
                .map(|(name, resources)| {
                    let (vertex_count, index_count) = prepared_streams[&name].logical_counts();
                    (
                        name,
                        super::compute_frame::GeometryInputs {
                            resources,
                            vertex_count,
                            index_count,
                        },
                    )
                })
                .collect(),
            compute_draws,
            compute_budget: limits.max_buffer_size,
            device,
            queue,
            executor,
            geometry,
            groups: step_groups,
            lighting_supported,
            buffer_views,
            group_size: [0; 2],
            material,
            style_parameters,
            objects: Vec::new(),
            lighting: super::forward_plus::LightingEnvironment::Preview.lighting(),
            shader_source: wgsl.into(),
            preparations,
        })
    }
    /// Supply conservative world bounds for custom factory/deformation output.
    /// Update these before rendering whenever the object's prepared geometry moves.
    pub fn set_prepared_bounds(
        &mut self,
        bounds: Option<super::bounds::Bounds>,
    ) -> Result<(), RuntimeError> {
        self.world_bounds = bounds
            .map(|b| super::bounds::Bounds::new(b.minimum, b.maximum))
            .transpose()?;
        Ok(())
    }

    pub fn set_point_lights(
        &mut self,
        lights: &[super::forward_plus::PointLight],
    ) -> Result<(), RuntimeError> {
        let bytes = super::forward_plus::pack_lights(lights)?;
        let buffer = self
            .executor
            .buffer_for_source("point_lights")
            .ok_or_else(|| RuntimeError::ForwardPlus("recipe has no point-light input".into()))?;
        self.queue.write_buffer(buffer, 0, &bytes);
        Ok(())
    }
    pub fn light_tile_masks(&self) -> Option<&wgpu::Buffer> {
        self.executor.written_buffer()
    }
    pub fn set_lighting_environment(
        &mut self,
        environment: super::forward_plus::LightingEnvironment,
    ) -> Result<(), RuntimeError> {
        self.set_scene_lighting(environment.lighting(), &environment.point_lights())
    }
    pub fn set_scene_lighting(
        &mut self,
        lighting: super::forward_plus::SceneLighting,
        lights: &[super::forward_plus::PointLight],
    ) -> Result<(), RuntimeError> {
        let bytes = lighting.bytes()?;
        let points = super::forward_plus::pack_lights(lights)?;
        let environment = self
            .executor
            .buffer_for_source("lighting_environment")
            .ok_or_else(|| RuntimeError::ForwardPlus("recipe has no lighting input".into()))?;
        let point_buffer = self
            .executor
            .buffer_for_source("point_lights")
            .ok_or_else(|| RuntimeError::ForwardPlus("recipe has no point-light input".into()))?;
        self.queue.write_buffer(environment, 0, &bytes);
        self.queue.write_buffer(point_buffer, 0, &points);
        self.lighting = lighting;
        Ok(())
    }
    /// Set a procedural sky for the scene. Alpha controls background visibility.
    pub fn set_background(&self, zenith: [f32; 4], horizon: [f32; 4]) -> Result<(), RuntimeError> {
        let values: Vec<_> = zenith.into_iter().chain(horizon).collect();
        if values.iter().any(|v| !v.is_finite()) {
            return Err(RuntimeError::PassPlan("nonfinite sky color".into()));
        }
        if let Some(buffer) = self.executor.buffer_for_source("scene_background") {
            let bytes: Vec<_> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.queue.write_buffer(buffer, 0, &bytes);
        }
        Ok(())
    }
    pub fn supports_lighting_environment(&self) -> bool {
        self.lighting_supported
            || self
                .objects
                .iter()
                .any(|(object, _)| object.lighting_supported)
    }
    pub fn buffer_views(&self) -> &[super::buffer_views::BufferView] {
        &self.buffer_views
    }
    pub fn set_buffer_view(&self, id: &str) -> Result<(), RuntimeError> {
        let view = self
            .buffer_views
            .iter()
            .find(|view| view.id == id)
            .ok_or_else(|| RuntimeError::PassPlan(format!("buffer view `{id}` is unavailable")))?;
        let buffer = self
            .executor
            .buffer_for_source("buffer_view")
            .ok_or_else(|| RuntimeError::PassPlan("buffer view input is missing".into()))?;
        self.queue.write_buffer(buffer, 0, &view.mode.to_le_bytes());
        Ok(())
    }
    /// Inspect an allocated recipe image by its authored resource name.
    pub fn resource_texture(&self, name: &str) -> Option<&wgpu::Texture> {
        self.executor.texture(name)
    }
    /// Depth written by the opaque scene, or the caller-supplied depth target.
    pub fn scene_depth(&self) -> Result<Option<wgpu::TextureView>, RuntimeError> {
        self.executor.scene_depth()
    }
    pub fn material_id_texture(&self) -> Option<&wgpu::Texture> {
        self.executor.integer_texture()
    }
    pub fn parameter_values(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut values = self.material.values();
        values.extend(self.style_parameters.values());
        values
    }

    /// Fixed-size uniform edits need no pipeline or bind-group replacement.
    /// Invalid batches change neither the CPU values nor any GPU buffer.
    pub fn update_parameters(
        &mut self,
        updates: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        let names = self.style_parameters.values();
        let (style_updates, material_updates) = updates
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .partition(|(k, _)| names.contains_key(k));
        let mut candidate = self.style_parameters.clone();
        candidate.update(&style_updates)?;
        self.material
            .update_parameters(&self.queue, &material_updates)?;
        self.style_parameters = candidate;
        Ok(())
    }

    fn prepare_frame(
        &mut self,
        inputs: &MeshSceneInputs,
        shadow_inputs: &MeshSceneInputs,
        budget: &mut u64,
        instance_id: std::num::NonZeroU32,
    ) -> Result<PreparedFrame, RuntimeError> {
        let scene = inputs.pack()?;
        let inverse = if self.executor.buffer_for_source("resolve_camera").is_some() {
            let mut bytes = Vec::with_capacity(96);
            for value in super::deferred::inverse_view_projection(inputs)? {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in inputs.camera_position.into_iter().chain([0.0]) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in inputs.frame.physical_size.into_iter().chain([0, 0]) {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            Some(bytes)
        } else {
            None
        };
        self.executor
            .resize(&self.device, inputs.frame.physical_size)?;
        self.material.update_frame(&self.queue, inputs.frame)?;
        if let Some(buffer) = self.executor.buffer_for_source("scene") {
            self.queue.write_buffer(buffer, 0, &scene);
        }
        if let Some(bytes) = inverse {
            self.queue.write_buffer(
                self.executor
                    .buffer_for_source("resolve_camera")
                    .expect("selected camera input"),
                0,
                &bytes,
            );
        }
        if let Some(buffer) = self.executor.buffer_for_source("shadow_camera") {
            self.queue.write_buffer(
                buffer,
                0,
                &super::forward_plus::shadow_camera_bytes(shadow_inputs, self.lighting.direction),
            );
        }
        let resized = self.group_size != inputs.frame.physical_size;
        let image_sizes = self.executor.image_sizes();
        let bounds = if let Some(bounds) = self.world_bounds {
            Some(bounds)
        } else {
            crate::profile::mesh::prepared_bounds(&self.factory, self.local_bounds, inputs)?
        };
        let bounds_bytes = super::bounds::uniform_bytes(bounds);
        for geometry in self.compute_geometry.values() {
            let Some(Resource::Uniform(buffer)) = geometry.resources.get("bounds") else {
                return Err(RuntimeError::PassPlan(
                    "missing prepared geometry bounds uniform".into(),
                ));
            };
            self.queue.write_buffer(buffer, 0, &bounds_bytes);
        }
        let mut compute = self.compute.instantiate(
            &self.device,
            &self.style_parameters,
            &self.compute_geometry,
            &self.executor.resources,
            &image_sizes,
            budget,
        )?;
        compute.depth = bounds.map(|b| b.view_depth(&inputs.view)).transpose()?;
        self.executor.prepare_generated(
            &mut compute,
            &self.compute_geometry,
            &self.style_parameters,
        )?;
        self.executor.prepare_spatial(
            &mut compute,
            &self.compute_geometry,
            &self.style_parameters,
            bounds,
            inputs,
        )?;
        // Fresh per-view storage: queued frames never share a writable ID uniform.
        let instance = self
            .groups
            .values()
            .flatten()
            .any(|g| !g.draw_data.is_empty())
            .then(|| {
                let mut bytes = [0u8; 16];
                bytes[..4].copy_from_slice(&instance_id.get().to_le_bytes());
                Resource::Uniform(self.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("draw shading instance"),
                        contents: &bytes,
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                ))
            });
        let groups = self
            .groups
            .iter_mut()
            .map(|(name, groups)| {
                let groups = groups
                    .iter_mut()
                    .map(|group| {
                        if group.cached.is_none()
                            || !group.compute.is_empty()
                            || !group.draw_data.is_empty()
                            || (resized && !group.images.is_empty())
                        {
                            let dynamic: Vec<_> = group
                                .compute
                                .iter()
                                .map(|(binding, input)| {
                                    compute
                                        .draw_resource(&self.device, input)
                                        .map(|r| (*binding, r))
                                })
                                .collect::<Result<_, _>>()?;
                            let mut entries: Vec<_> = group
                                .resources
                                .iter()
                                .map(|(binding, resource)| {
                                    let resource = group
                                        .images
                                        .get(binding)
                                        .map_or(resource, |name| &self.executor.resources[name]);
                                    wgpu::BindGroupEntry {
                                        binding: *binding,
                                        resource: resource.binding(),
                                    }
                                })
                                .collect();
                            entries.extend(dynamic.iter().map(|(binding, resource)| {
                                wgpu::BindGroupEntry {
                                    binding: *binding,
                                    resource: resource.binding(),
                                }
                            }));
                            entries.extend(group.draw_data.iter().map(|(binding, data)| {
                                let resource = match data {
                                    fresco_artifact::ManifestDrawData::InstanceId => {
                                        instance.as_ref().expect("instance binding")
                                    }
                                };
                                wgpu::BindGroupEntry {
                                    binding: *binding,
                                    resource: resource.binding(),
                                }
                            }));
                            group.cached =
                                Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some(name),
                                    layout: &group.layout,
                                    entries: &entries,
                                }));
                        }
                        Ok((
                            group.group,
                            group.cached.as_ref().expect("prepared group").clone(),
                        ))
                    })
                    .collect::<Result<_, RuntimeError>>()?;
                Ok((name.clone(), groups))
            })
            .collect::<Result<_, RuntimeError>>()?;
        self.group_size = inputs.frame.physical_size;
        Ok(PreparedFrame { groups, compute })
    }

    /// Add an ordinary mesh/material instance to this frame's shared render graph.
    /// Object transforms are world-space; view and lighting are common to the scene.
    pub fn add_object(&mut self, mut object: Self, model: [f32; 16]) -> Result<(), RuntimeError> {
        if model.iter().any(|n| !n.is_finite())
            || !object.objects.is_empty()
            || object.shader_source != self.shader_source
        {
            return Err(RuntimeError::PassPlan(
                "invalid, nested, or independently compiled scene object".into(),
            ));
        }
        self.executor.validate_scene_object(&object.executor)?;
        // A previously rendered object may still cache bindings to its private
        // frame buffers. Its next draw must bind this scene's shared resources.
        for group in object.groups.values_mut().flatten() {
            group.cached = None;
        }
        self.objects.push((object, model));
        Ok(())
    }

    /// Maximum owned compute output bytes across all ranges in one rendered view.
    pub fn set_compute_budget(&mut self, bytes: u64) {
        self.compute_budget = bytes;
    }

    pub fn render(
        &mut self,
        inputs: &MeshSceneInputs,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> Result<bool, RuntimeError> {
        if inputs.frame.physical_size.contains(&0) {
            return Ok(false);
        }
        let mut shadow_inputs = *inputs;
        // Conservative world-space bounds for the bundled unit geometry and all
        // submitted objects. Neither shadow fitting nor execution knows about floors.
        if !self.objects.is_empty() {
            shadow_inputs.model = scene_shadow_bounds(
                std::iter::once(&inputs.model).chain(self.objects.iter().map(|(_, model)| model)),
            );
        }
        // A material row is shared by every occurrence of that material. Reject
        // contradictory per-object edits before uploading any style values.
        let mut style_uploads = BTreeMap::new();
        for state in std::iter::once(&self.style_parameters)
            .chain(self.objects.iter().map(|(o, _)| &o.style_parameters))
        {
            for (offset, bytes) in state.uploads() {
                if style_uploads
                    .insert(offset, bytes)
                    .is_some_and(|old| old != bytes)
                {
                    return Err(RuntimeError::Parameter {
                        name: "style".into(),
                        reason:
                            "scene objects using the same material have conflicting style settings"
                                .into(),
                    });
                }
            }
        }
        // Compute captures use native scalar uniforms. A shared table is only
        // needed when the compiled renderer actually binds style parameters.
        if let Some(buffer) = self.executor.buffer_for_source("style_parameters") {
            for (offset, bytes) in style_uploads {
                self.queue
                    .write_buffer(buffer, u64::from(offset) * 16, bytes);
            }
        }
        let mut budget = self.compute_budget;
        let primary_groups = self.prepare_frame(
            inputs,
            &shadow_inputs,
            &mut budget,
            std::num::NonZeroU32::MIN,
        )?;
        let mut object_groups = Vec::new();
        for (index, (object, model)) in self.objects.iter_mut().enumerate() {
            let instance_id = u32::try_from(index)
                .ok()
                .and_then(|i| i.checked_add(2))
                .and_then(std::num::NonZeroU32::new)
                .ok_or_else(|| {
                    RuntimeError::PassPlan("draw instance ID capacity exceeded".into())
                })?;
            object.lighting = self.lighting;
            object.executor.share_frame_resources(&self.executor)?;
            let mut object_inputs = *inputs;
            object_inputs.model = *model;
            object_inputs.displacement.enabled = false;
            object_groups.push(object.prepare_frame(
                &object_inputs,
                &shadow_inputs,
                &mut budget,
                instance_id,
            )?);
        }
        let view_depth = |model: &[f32; 16]| {
            -(inputs.view[2] * model[12]
                + inputs.view[6] * model[13]
                + inputs.view[10] * model[14]
                + inputs.view[14])
        };
        let mut objects = vec![(
            &self.executor,
            &self.geometry,
            &primary_groups.groups,
            primary_groups
                .compute
                .depth
                .unwrap_or_else(|| view_depth(&inputs.model)),
            &primary_groups.compute,
            &self.compute_draws,
        )];
        objects.extend(
            self.objects
                .iter()
                .zip(&object_groups)
                .map(|((object, model), groups)| {
                    (
                        &object.executor,
                        &object.geometry,
                        &groups.groups,
                        groups.compute.depth.unwrap_or_else(|| view_depth(model)),
                        &groups.compute,
                        &object.compute_draws,
                    )
                }),
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let encode_preparation =
            |encoder: &mut wgpu::CommandEncoder,
             renderer: &Self,
             groups: &BTreeMap<String, Vec<(u32, wgpu::BindGroup)>>| {
                for (node, pipeline, count) in &renderer.preparations {
                    if *count == 0 {
                        continue;
                    }
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("geometry preparation"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(pipeline);
                    for (group, binding) in &groups[node] {
                        pass.set_bind_group(*group, binding, &[]);
                    }
                    pass.dispatch_workgroups(*count, 1, 1);
                }
            };
        encode_preparation(&mut encoder, self, &primary_groups.groups);
        for ((object, _), groups) in self.objects.iter().zip(&object_groups) {
            encode_preparation(&mut encoder, object, &groups.groups);
        }
        super::recipe::Executor::encode_scene(&self.device, &mut encoder, color, depth, &objects)?;
        self.queue.submit([encoder.finish()]);
        Ok(true)
    }
}

// Center the shadow fit on the scene, independently of which object owns it.
fn scene_shadow_bounds<'a>(models: impl Iterator<Item = &'a [f32; 16]> + Clone) -> [f32; 16] {
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for model in models.clone() {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(model[12 + axis]);
            maximum[axis] = maximum[axis].max(model[12 + axis]);
        }
    }
    let center = std::array::from_fn::<_, 3, _>(|axis| minimum[axis] * 0.5 + maximum[axis] * 0.5);
    let mut radius = 0.0_f32;
    for model in models {
        let scale = (0..3)
            .map(|col| {
                model[col * 4..col * 4 + 3]
                    .iter()
                    .map(|v| v * v)
                    .sum::<f32>()
                    .sqrt()
            })
            .fold(0.0_f32, f32::max);
        let distance = model[12..15]
            .iter()
            .zip(center)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            .sqrt();
        radius = radius.max(scale + distance);
    }
    [
        radius, 0.0, 0.0, 0.0, 0.0, radius, 0.0, 0.0, 0.0, 0.0, radius, 0.0, center[0], center[1],
        center[2], 1.0,
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn shading_outputs_bind_without_a_contributed_draw_invocation() {
        use fresco_artifact::{ManifestMeshPass, ManifestStyleInvocation};
        let pass: ManifestMeshPass = serde_json::from_value(serde_json::json!({
            "pass": "base", "factory": "mesh",
            "shading_inputs": {
                "density": { "producer": "density_compute", "ty": "texture2d<r32float, read>", "dimension": null }
            }
        })).unwrap();
        let inputs = super::draw_compute_inputs(&pass, None).unwrap();
        assert_eq!(inputs["density"].producer, "density_compute");
        let mut invocation: ManifestStyleInvocation = serde_json::from_value(serde_json::json!({
            "point": "finish", "material": "shared", "operation": "shell", "ordinal": 1,
            "compute_inputs": {
                "offsets": { "producer": "offset_compute", "ty": "buffer<vec4, read>", "dimension": null }
            }
        })).unwrap();
        let combined = super::draw_compute_inputs(&pass, Some(&invocation)).unwrap();
        assert_eq!(combined.len(), 2);
        assert_eq!(combined["offsets"].producer, "offset_compute");
        invocation
            .compute_inputs
            .insert("density".into(), combined["offsets"].clone());
        assert!(super::draw_compute_inputs(&pass, Some(&invocation)).is_err());
        assert_eq!(
            super::draw_compute_inputs(&pass, None).unwrap()["density"].producer,
            "density_compute"
        );
    }

    #[test]
    fn scene_shadow_fit_is_order_independent_and_contains_each_object() {
        let a = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -2.0, 0.0, 0.0, 1.0,
        ];
        let mut b = a;
        b[12] = 3.0;
        b[0] = 2.0;
        let fit = super::scene_shadow_bounds([&a, &b].into_iter());
        assert_eq!(fit, super::scene_shadow_bounds([&b, &a].into_iter()));
        assert_eq!(fit[12], 0.5);
        assert!(fit[0] >= (a[12] - fit[12]).abs() + 1.0);
        assert!(fit[0] >= (b[12] - fit[12]).abs() + 2.0);
    }
}
