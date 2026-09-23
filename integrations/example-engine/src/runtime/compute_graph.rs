//! Per-material operation dependencies, before assigning frame/view/range resources.
//! Allocation order and GPU execution readiness intentionally use different edges.
use std::collections::{BTreeMap, BTreeSet};

use super::{
    RuntimeError,
    compute_plan::{ComputeInvocationPlan, ComputeLimits, OwnedAllocation},
};
use fresco_artifact::{ManifestComputeArgument as Argument, ManifestGpuProgram, ManifestRoot};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComputePrerequisite {
    Operation(String),
    PreparedGeometry(String),
    EngineNode(String),
}

pub struct ComputeNode<'a> {
    pub program: &'a ManifestGpuProgram,
    pub prerequisites: BTreeSet<ComputePrerequisite>,
}

pub struct ComputeGraph<'a> {
    nodes: Vec<ComputeNode<'a>>,
}

/// One object's kernels. Allocation dependencies have already been resolved;
/// these prerequisites describe GPU execution readiness only.
pub struct ScheduledCompute<'a> {
    pub name: &'a str,
    pub prerequisites: &'a BTreeSet<ComputePrerequisite>,
}

/// Incrementally insert kernels at the earliest completed engine boundary.
/// Operation and geometry identities are local to an object; engine boundaries
/// are view-wide and may be released only after every contributing range finishes.
pub struct ComputeSchedule<'a> {
    objects: Vec<Vec<ScheduledCompute<'a>>>,
    completed: Vec<BTreeSet<ComputePrerequisite>>,
}

impl<'a> ComputeSchedule<'a> {
    pub fn new(objects: Vec<Vec<ScheduledCompute<'a>>>) -> Self {
        let completed = vec![BTreeSet::new(); objects.len()];
        Self { objects, completed }
    }

    pub fn geometry_completed(&mut self, object: usize, producer: &str) {
        self.completed[object].insert(ComputePrerequisite::PreparedGeometry(producer.into()));
    }

    pub fn engine_completed(&mut self, producer: &str) {
        for completed in &mut self.completed {
            completed.insert(ComputePrerequisite::EngineNode(producer.into()));
        }
    }

    pub fn is_complete(&self, object: usize, producer: &str) -> bool {
        self.completed[object].contains(&ComputePrerequisite::Operation(producer.into()))
    }

    /// Returned kernels must be encoded in this order before releasing another
    /// boundary. Resolve the ready closure so serialization order creates no edges.
    pub fn ready(&mut self) -> Vec<(usize, usize)> {
        let mut ready = Vec::new();
        loop {
            let before = ready.len();
            for (object, nodes) in self.objects.iter().enumerate() {
                for (index, node) in nodes.iter().enumerate() {
                    let identity = ComputePrerequisite::Operation(node.name.into());
                    if !self.completed[object].contains(&identity)
                        && node.prerequisites.is_subset(&self.completed[object])
                    {
                        self.completed[object].insert(identity);
                        ready.push((object, index));
                    }
                }
            }
            if ready.len() == before {
                return ready;
            }
        }
    }

    pub fn finish(&self) -> Result<(), RuntimeError> {
        if self.objects.iter().enumerate().any(|(object, nodes)| {
            nodes
                .iter()
                .any(|node| !self.is_complete(object, node.name))
        }) {
            return Err(invalid(
                "compute invocation has unsatisfied renderer dependencies",
            ));
        }
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::PassPlan(format!("compute graph: {}", message.into()))
}

impl<'a> ComputeGraph<'a> {
    pub fn new(manifest: &'a ManifestRoot, material: &str) -> Result<Self, RuntimeError> {
        super::validate_artifact(manifest)?;
        let surfaces: Vec<_> = manifest
            .surfaces
            .iter()
            .filter(|s| s.name == material)
            .collect();
        let [surface] = surfaces.as_slice() else {
            return Err(invalid("material is absent or ambiguous"));
        };
        let mut programs = BTreeMap::new();
        let mut ordinals = BTreeSet::new();
        for program in &manifest.gpu_programs {
            let Some(invocation) = &program.compute_invocation else {
                continue;
            };
            if invocation.material != material {
                continue;
            }
            if programs.insert(program.pass.as_str(), program).is_some()
                || !ordinals.insert(invocation.ordinal)
            {
                return Err(invalid("duplicate invocation identity"));
            }
        }
        let mut metadata = BTreeMap::new();
        let mut nodes = BTreeMap::new();
        for (&name, &program) in &programs {
            let invocation = program
                .compute_invocation
                .as_ref()
                .expect("selected invocation");
            if invocation.bindings != program.compute_bindings {
                return Err(invalid(
                    "invocation bindings disagree with shader provenance",
                ));
            }
            let [entry] = program.entries.as_slice() else {
                return Err(invalid("operation requires one compute entry"));
            };
            let reads = program
                .compute_shader_reads(&entry.entry)
                .map_err(invalid)?;
            if reads
                .data
                .iter()
                .chain(&reads.values)
                .chain(&reads.dimensions)
                .any(|parameter| !invocation.arguments.contains_key(parameter))
            {
                return Err(invalid(format!(
                    "`{name}` is missing an argument read by its shader"
                )));
            }
            let mut allocations = BTreeSet::new();
            let mut data = BTreeSet::new();
            let mut prerequisites = BTreeSet::new();
            for (parameter, argument) in &invocation.arguments {
                match argument {
                    Argument::Output { producer } => {
                        if !programs.contains_key(producer.as_str()) {
                            return Err(invalid(format!(
                                "`{name}` references missing or foreign-material output `{producer}`"
                            )));
                        }
                        allocations.insert(producer.as_str());
                        if reads.data.contains(parameter) {
                            data.insert(producer.as_str());
                        }
                    }
                    Argument::Geometry {
                        producer,
                        node,
                        resource_type,
                        ..
                    } => {
                        let valid = surface.mesh_passes.iter().any(|p| {
                            p.pass == *producer
                                && p.preparation.as_ref().is_some_and(|preparation| {
                                    preparation.node == *node
                                        && preparation.resource_type == *resource_type
                                })
                        });
                        if !valid {
                            return Err(invalid(format!(
                                "`{name}` references unavailable prepared geometry `{producer}`"
                            )));
                        }
                        if reads.data.contains(parameter) {
                            prerequisites
                                .insert(ComputePrerequisite::PreparedGeometry(producer.clone()));
                        }
                    }
                    Argument::Setting { ty, name, offset } => {
                        let matches = surface
                            .settings
                            .iter()
                            .flat_map(|settings| &settings.implementations)
                            .flat_map(|selection| {
                                selection.parameters.iter().enumerate().filter_map(
                                    move |(index, parameter)| {
                                        let candidate =
                                            u32::try_from(index).ok().and_then(|index| {
                                                selection.settings_offset.checked_add(index)
                                            })?;
                                        (candidate == *offset).then_some(parameter)
                                    },
                                )
                            })
                            .collect::<Vec<_>>();
                        if !matches!(matches.as_slice(), [parameter] if parameter.name == *name && parameter.ty == *ty)
                        {
                            return Err(invalid(
                                "captured setting disagrees with the owning material's declaration",
                            ));
                        }
                    }
                    Argument::Constant { .. } | Argument::External { .. } => {}
                }
            }
            let declared_data: BTreeSet<_> =
                invocation.dependencies.iter().map(String::as_str).collect();
            let declared_metadata: BTreeSet<_> = invocation
                .metadata_dependencies
                .iter()
                .map(String::as_str)
                .collect();
            if declared_data != data
                || declared_data.len() != invocation.dependencies.len()
                || declared_metadata != allocations
                || declared_metadata.len() != invocation.metadata_dependencies.len()
            {
                return Err(invalid(format!(
                    "`{name}` dependency declarations disagree with resource uses"
                )));
            }
            prerequisites.extend(
                data.into_iter()
                    .map(|p| ComputePrerequisite::Operation(p.into())),
            );
            for engine_node in &invocation.engine_dependencies {
                if !manifest
                    .renderers
                    .iter()
                    .filter(|r| r.selected)
                    .flat_map(|r| &r.steps)
                    .any(|s| s.name == *engine_node)
                {
                    return Err(invalid(format!(
                        "`{name}` references missing engine node `{engine_node}`"
                    )));
                }
                prerequisites.insert(ComputePrerequisite::EngineNode(engine_node.clone()));
            }
            metadata.insert(name, allocations);
            nodes.insert(
                name,
                ComputeNode {
                    program,
                    prerequisites,
                },
            );
        }
        // Stable topological allocation order. Authored ordinals break ties, but
        // neither artifact serialization order nor ordinals establish dependencies.
        let mut allocated = BTreeSet::new();
        let mut ordered = Vec::new();
        while !nodes.is_empty() {
            let next = nodes
                .iter()
                .filter(|(name, _)| metadata[*name].is_subset(&allocated))
                .min_by_key(|(_, node)| {
                    node.program
                        .compute_invocation
                        .as_ref()
                        .expect("invocation")
                        .ordinal
                })
                .map(|(&name, _)| name)
                .ok_or_else(|| invalid("owned output allocation dependency cycle"))?;
            allocated.insert(next);
            ordered.push(nodes.remove(next).expect("selected node"));
        }
        Ok(Self { nodes: ordered })
    }

    /// Allocate and bind in this order, without waiting for GPU producers.
    pub fn allocation_order(&self) -> &[ComputeNode<'a>] {
        &self.nodes
    }

    /// Validate every allocation against one shared remaining frame/view budget
    /// before the caller allocates any GPU resources. Owned-handle dimensions come
    /// from the producer plan, never a caller-supplied size or GPU data read.
    pub fn plan_allocations(
        &self,
        limits: ComputeLimits,
        byte_budget: u64,
        read: &impl Fn(
            &ManifestGpuProgram,
            &str,
            Option<&str>,
        ) -> Result<fresco_artifact::ComputeScalar, RuntimeError>,
    ) -> Result<BTreeMap<&'a str, ComputeInvocationPlan>, RuntimeError> {
        let mut plans: BTreeMap<&str, ComputeInvocationPlan> = BTreeMap::new();
        let mut remaining = byte_budget;
        for node in &self.nodes {
            let invocation = node
                .program
                .compute_invocation
                .as_ref()
                .expect("validated invocation");
            let resolve = |parameter: &str, member: Option<&str>| {
                let argument = invocation
                    .arguments
                    .get(parameter)
                    .ok_or_else(|| invalid(format!("unknown host input `{parameter}`")))?;
                if let Argument::Output { producer } = argument {
                    let producer = plans
                        .get(producer.as_str())
                        .expect("topological allocation order");
                    let value = match (producer.output, member) {
                        (OwnedAllocation::Buffer(buffer), Some("count")) => buffer.elements(),
                        (OwnedAllocation::Image { allocation, .. }, Some("width")) => {
                            allocation.logical()[0]
                        }
                        (OwnedAllocation::Image { allocation, .. }, Some("height")) => {
                            allocation.logical()[1]
                        }
                        _ => {
                            return Err(invalid(
                                "owned output host input must be a logical dimension",
                            ));
                        }
                    };
                    Ok(fresco_artifact::ComputeScalar::U32(value))
                } else {
                    read(node.program, parameter, member)
                }
            };
            let plan = ComputeInvocationPlan::new(node.program, limits, remaining, &resolve)?;
            remaining = remaining
                .checked_sub(plan.output.bytes())
                .ok_or_else(|| invalid("transient allocation budget exceeded"))?;
            plans.insert(node.program.pass.as_str(), plan);
        }
        Ok(plans)
    }

    /// All allocations are assumed prepared. Completion markers are local to one
    /// frame/view/object/material range and must never be shared between ranges.
    /// Geometry/view/settings-only work has no implicit opaque-completion edge.
    pub fn ready<'b>(
        &'b self,
        completed: &'b BTreeSet<ComputePrerequisite>,
    ) -> impl Iterator<Item = &'b ComputeNode<'a>> + 'b {
        self.nodes.iter().filter(|node| {
            !completed.contains(&ComputePrerequisite::Operation(node.program.pass.clone()))
                && node.prerequisites.is_subset(completed)
        })
    }
}
