//! Example-engine particle policy decoded from ordinary technique contracts.
use fresco_artifact::{ManifestEntryProperty, ManifestRoot, ManifestTechniqueOperation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParticleContract {
    #[serde(default)]
    pub allocation: Option<ParticleAllocation>,
    #[serde(default)]
    pub properties: Vec<ManifestEntryProperty>,
    pub spawn_entry: String,
    pub particle_stride: u32,
    pub state_fields: Vec<ParticleField>,
    pub simulation_pass: String,
    pub draw_pass: String,
    pub compute_entry: String,
    pub vertex_entry: String,
    pub fragment_entry: String,
    pub particle_count: u32,
    pub vertex_count: u32,
    pub workgroup_size: u32,
    pub bindings: Vec<ParticleBinding>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParticleAllocation {
    pub mode: String,
    pub initial_capacity: u32,
    pub max_capacity: u32,
    pub growth_factor: f32,
    pub spawn_rate: f32,
    pub spawn_burst: u32,
    pub max_spawn_per_step: u32,
    pub max_lifespan: f32,
    pub overflow: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleField {
    pub name: String,
    pub offset: u32,
    pub ty: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParticleBinding {
    pub name: String,
    pub group: u32,
    pub binding: u32,
    pub resource: String,
    pub access: String,
}

pub fn uses_particles(root: &ManifestRoot, entry: &str) -> bool {
    root.techniques.iter().any(|t| {
        t.surface.as_deref() == Some(entry)
            && t.metadata.get("engine").is_some_and(|v| v == "particle")
    })
}

impl ParticleContract {
    pub fn for_surface(root: &ManifestRoot, entry: &str) -> Result<Option<Self>, String> {
        let mut candidates = root.techniques.iter().filter(|t| {
            t.surface.as_deref() == Some(entry)
                && t.metadata.get("engine").is_some_and(|v| v == "particle")
        });
        let Some(technique) = candidates.next() else {
            return Ok(None);
        };
        if candidates.next().is_some() {
            return Err("ambiguous particle techniques; select a single system".into());
        }
        let step = |name: &str| {
            technique
                .steps
                .iter()
                .find(|s| s.name == name)
                .ok_or_else(|| format!("particle technique requires {name} step"))
        };
        let spawn = step("spawn")?;
        let update = step("update")?;
        let draw = step("draw")?;
        if !update.after.contains(&spawn.name) || !draw.after.contains(&update.name) {
            return Err("particle technique must order spawn, update, and draw".into());
        }
        if technique.steps.len() != 3
            || spawn.enabled.as_deref() != Some("spawn")
            || update.enabled.as_deref() != Some("update")
            || draw.enabled.is_some()
        {
            return Err("particle engine requires spawn/update activation and exactly three lifecycle steps".into());
        }
        for step in [spawn, update] {
            if !matches!(&step.operation, ManifestTechniqueOperation::Compute {
                extent: fresco_artifact::ManifestDispatchExtent::Parameter { parameter }, ..
            } if parameter == "capacity")
            {
                return Err("particle compute extent must use the capacity parameter".into());
            }
        }
        if !matches!(&draw.operation, ManifestTechniqueOperation::Draw {
            instances: fresco_artifact::ManifestDrawCount::Parameter { parameter }, ..
        } if parameter == "capacity")
        {
            return Err("particle draw instances must use the capacity parameter".into());
        }
        if spawn.bindings != update.bindings {
            return Err("particle initialization and update must share resource bindings".into());
        }
        let require_provider = |resource: Option<&String>, provider: &str| -> Result<(), String> {
            let supplied =
                resource.and_then(|name| technique.resources.iter().find(|r| &r.name == name));
            if !supplied.is_some_and(|r| matches!(&r.source,
                fresco_artifact::ManifestResourceSource::External { provider: source } if source == provider)) {
                return Err(format!("particle engine requires the {provider} resource provider"));
            }
            Ok(())
        };
        require_provider(update.bindings.get("particles"), "particle_state")?;
        require_provider(update.bindings.get("particle_config"), "particle_config")?;
        require_provider(draw.bindings.get("scene"), "scene")?;
        if let ManifestTechniqueOperation::Draw { colors, depth, .. } = &draw.operation {
            if colors.len() != 1 {
                return Err("particle draw requires one presentation attachment".into());
            }
            require_provider(colors.get(&0), "presentation")?;
            require_provider(depth.as_ref(), "depth")?;
        }
        let program = |step: &fresco_artifact::ManifestTechniqueStep| {
            root.gpu_programs
                .iter()
                .find(|p| p.pass == step.pass && p.surface.as_deref().is_none_or(|s| s == entry))
                .ok_or_else(|| format!("particle program missing for {}", step.name))
        };
        let simulation = program(update)?;
        if spawn.pass != update.pass {
            return Err("particle engine requires shared initialization/update bindings".into());
        }
        let raster = program(draw)?;
        let (compute_entry, workgroup_size) = match &update.operation {
            ManifestTechniqueOperation::Compute {
                entry,
                workgroup_size,
                ..
            } if workgroup_size[1..] == [1, 1] => (entry.clone(), workgroup_size[0]),
            _ => return Err("particle update requires a one-dimensional compute program".into()),
        };
        let spawn_entry = match &spawn.operation {
            ManifestTechniqueOperation::Compute {
                entry,
                workgroup_size: size,
                ..
            } if size == &[workgroup_size, 1, 1] => entry.clone(),
            _ => return Err("particle initialization requires matching compute dimensions".into()),
        };
        let (vertex_entry, fragment_entry, vertex_count) = match &draw.operation {
            ManifestTechniqueOperation::Draw {
                vertex,
                fragment,
                vertices,
                ..
            } => (
                vertex.clone(),
                fragment
                    .clone()
                    .ok_or("particle presentation requires a fragment entry")?,
                *vertices,
            ),
            _ => return Err("particle presentation requires a draw".into()),
        };
        if workgroup_size == 0 || workgroup_size > 256 {
            return Err("particle workgroup size must be in 1..256".into());
        }
        let value = |name: &str| {
            simulation
                .metadata
                .get(name)
                .ok_or_else(|| format!("particle engine requires explicit {name} metadata"))
        };
        let integer = |name: &str| -> Result<u32, String> {
            let n = value(name)?
                .parse::<u32>()
                .map_err(|_| format!("{name} must be an integer"))?;
            if n == 0 || n > 1_048_576 {
                return Err(format!("{name} must be in 1..1048576"));
            }
            Ok(n)
        };
        let scalar = |name: &str| -> Result<f32, String> {
            let n = value(name)?
                .parse::<f32>()
                .map_err(|_| format!("invalid {name}"))?;
            if !n.is_finite() || n < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
            Ok(n)
        };
        let particle_count = integer("capacity")?;
        let allocation = if simulation.metadata.contains_key("allocation_mode") {
            let mode = match value("allocation_mode")?.as_str() {
                "0" => "fixed",
                "1" => "estimated",
                "2" => "automatic",
                _ => return Err("unknown particle allocation mode".into()),
            };
            let max_capacity = integer("max_capacity")?;
            let growth_factor = scalar("growth_factor")?;
            let max_lifespan = scalar("max_lifespan")?;
            if max_capacity < particle_count || growth_factor <= 1.0 || max_lifespan <= 0.0 {
                return Err("allocation requires maximum >= initial capacity, growth factor > 1, and maximum lifespan > 0".into());
            }
            let spawn_burst = value("spawn_burst")?
                .parse::<u32>()
                .map_err(|_| "spawn burst must be an integer")?;
            if spawn_burst > 1_048_576 {
                return Err("spawn burst exceeds engine capacity".into());
            }
            if value("overflow")? != "drop_new" {
                return Err("unsupported particle overflow policy".into());
            }
            Some(ParticleAllocation {
                mode: mode.into(),
                initial_capacity: particle_count,
                max_capacity,
                growth_factor,
                max_lifespan,
                spawn_rate: scalar("spawn_rate")?,
                spawn_burst,
                max_spawn_per_step: integer("max_spawn_per_step")?,
                overflow: "drop_new".into(),
            })
        } else {
            if [
                "max_capacity",
                "growth_factor",
                "spawn_rate",
                "spawn_burst",
                "max_lifespan",
                "max_spawn_per_step",
                "overflow",
            ]
            .iter()
            .any(|k| simulation.metadata.contains_key(*k))
            {
                return Err("managed allocation metadata requires allocation_mode".into());
            }
            None
        };
        let state = simulation
            .bindings
            .iter()
            .find(|b| b.name == "particles")
            .ok_or("particle state binding missing")?;
        let config = simulation
            .bindings
            .iter()
            .find(|b| b.name == "particle_config")
            .ok_or("particle config binding missing")?;
        let config_fields: Vec<_> = config
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.ty.as_str(), f.offset))
            .collect();
        if config.kind != "uniform"
            || config_fields
                != [
                    ("delta_time", "f32", 0),
                    ("particle_count", "u32", 4),
                    ("padding", "vec2", 8),
                ]
        {
            return Err(
                "particle config ABI requires delta_time: f32, particle_count: u32, padding: vec2"
                    .into(),
            );
        }
        let render = raster
            .bindings
            .iter()
            .find(|b| b.name == "particle_render")
            .ok_or("particle draw state binding missing")?;
        if state.ty != render.ty || state.element_stride != render.element_stride {
            return Err("particle render storage must contain the simulation state type".into());
        }
        if update.bindings.get("particles") != draw.bindings.get("particle_render") {
            return Err("particle update and draw must share a resource".into());
        }
        let particle_stride = state
            .element_stride
            .ok_or("particle state layout missing")?;
        particle_count
            .checked_mul(particle_stride)
            .ok_or("particle storage byte size overflows u32")?;
        if allocation.is_some() {
            require_provider(update.bindings.get("particle_slots"), "particle_slots")?;
            if update.bindings.get("particle_slots") != draw.bindings.get("particle_slots") {
                return Err("particle update and draw must share slot storage".into());
            }
            let slots = simulation
                .bindings
                .iter()
                .find(|b| b.name == "particle_slots")
                .ok_or("particle slot binding missing")?;
            let fields: Vec<_> = slots
                .fields
                .iter()
                .map(|f| (f.name.as_str(), f.ty.as_str()))
                .collect();
            if fields
                != [
                    ("birth_id", "u32"),
                    ("spawn", "u32"),
                    ("enabled", "u32"),
                    ("dt", "f32"),
                ]
            {
                return Err("particle slot ABI requires birth_id, spawn, enabled, dt".into());
            }
        }
        let mut bindings: std::collections::BTreeMap<String, ParticleBinding> =
            std::collections::BTreeMap::new();
        for binding in simulation.bindings.iter().chain(&raster.bindings) {
            let reflected = ParticleBinding {
                name: binding.name.clone(),
                group: binding.group,
                binding: binding.binding,
                resource: binding.kind.clone(),
                access: binding.access.clone(),
            };
            if let Some(previous) = bindings.get(&binding.name) {
                if previous.group != reflected.group
                    || previous.binding != reflected.binding
                    || previous.resource != reflected.resource
                    || previous.access != reflected.access
                {
                    return Err("particle resource binding differs between stages".into());
                }
            } else {
                bindings.insert(binding.name.clone(), reflected);
            }
        }
        let mut slots = std::collections::BTreeSet::new();
        for binding in bindings.values() {
            if !slots.insert((binding.group, binding.binding)) {
                return Err("particle binding collision".into());
            }
        }
        Ok(Some(Self {
            allocation,
            properties: simulation.properties.clone(),
            spawn_entry,
            particle_stride,
            state_fields: state
                .fields
                .iter()
                .map(|f| ParticleField {
                    name: f.name.clone(),
                    offset: f.offset,
                    ty: f.ty.clone(),
                })
                .collect(),
            simulation_pass: update.pass.clone(),
            draw_pass: draw.pass.clone(),
            compute_entry,
            vertex_entry,
            fragment_entry,
            particle_count,
            vertex_count,
            workgroup_size,
            bindings: bindings.into_values().collect(),
        }))
    }
}
