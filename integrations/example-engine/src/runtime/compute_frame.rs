//! Resolve one frame/view/range's compute arguments and retain its owned outputs.
use super::{
    RuntimeError,
    compute_graph::{ComputeGraph, ComputePrerequisite},
    compute_plan::{ComputeInvocationPlan, OwnedAllocation},
    owned_compute::{ComputeInput, ComputeKernel, ComputeOutput},
    resources::Resource,
    style_parameters::StyleParameters,
};
use fresco_artifact::{
    ComputeScalar, ManifestComputeArgument as Argument, ManifestComputeBindingSource as Source,
    ManifestComputeDimension as Dimension, ManifestComputeGeometryRole as GeometryRole,
    ManifestComputeOutputLayout, ManifestDrawComputeBinding, ManifestGpuProgram, ManifestRoot,
};
use std::collections::{BTreeMap, BTreeSet};
use wgpu::util::DeviceExt;

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::PassPlan(format!("compute invocation: {}", message.into()))
}
fn compact(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) struct GeometryInputs {
    pub resources: BTreeMap<String, Resource>,
    pub vertex_count: u32,
    pub index_count: u32,
}

struct Kernel {
    program: ManifestGpuProgram,
    kernel: ComputeKernel,
    prerequisites: BTreeSet<ComputePrerequisite>,
}

#[derive(Default)]
pub(crate) struct PreparedCompute {
    kernels: Vec<Kernel>,
    features: wgpu::Features,
}

pub(crate) struct FrameNode {
    pub name: String,
    pub entry: String,
    pub pipeline: wgpu::ComputePipeline,
    pub groups: Vec<(u32, wgpu::BindGroup)>,
    pub threads: [u32; 3],
    pub workgroup: [u32; 3],
    pub prerequisites: BTreeSet<ComputePrerequisite>,
}

#[derive(Default)]
pub(crate) struct ComputeFrame {
    pub depth: Option<f32>,
    pub depths: BTreeMap<String, f32>,
    pub culled: BTreeSet<String>,
    pub nodes: Vec<FrameNode>,
    pub outputs: BTreeMap<String, ComputeOutput>,
    pub generated: BTreeMap<String, super::mesh_geometry::MeshGeometry>,
}

fn scalar(ty: &str, value: &serde_json::Value) -> Result<ComputeScalar, RuntimeError> {
    let bad = || invalid(format!("invalid captured {ty} value"));
    Ok(match ty {
        "u32" => ComputeScalar::U32(
            value
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(bad)?,
        ),
        "i32" => ComputeScalar::I32(
            value
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(bad)?,
        ),
        "bool" => ComputeScalar::Bool(value.as_bool().ok_or_else(bad)?),
        "f32" => {
            let value = value.as_f64().ok_or_else(bad)? as f32;
            if !value.is_finite() {
                return Err(bad());
            }
            ComputeScalar::F32(value)
        }
        _ => return Err(bad()),
    })
}
fn scalar_bytes(value: ComputeScalar) -> [u8; 4] {
    match value {
        ComputeScalar::U32(v) => v.to_le_bytes(),
        ComputeScalar::I32(v) => v.to_le_bytes(),
        ComputeScalar::F32(v) => v.to_le_bytes(),
        ComputeScalar::Bool(v) => u32::from(v).to_le_bytes(),
    }
}
fn value_bytes(ty: &str, value: &serde_json::Value) -> Result<Vec<u8>, RuntimeError> {
    let width = match ty {
        "vec2" => Some(2),
        "vec3" => Some(3),
        "vec4" | "color" => Some(4),
        _ => None,
    };
    if let Some(width) = width {
        let values = value
            .as_array()
            .filter(|v| v.len() == width)
            .ok_or_else(|| invalid("captured vector shape mismatch"))?;
        let mut bytes = Vec::new();
        for value in values {
            bytes.extend_from_slice(&scalar_bytes(scalar("f32", value)?));
        }
        if width == 3 {
            bytes.extend_from_slice(&[0; 4]);
        }
        Ok(bytes)
    } else {
        Ok(scalar_bytes(scalar(ty, value)?).to_vec())
    }
}
fn uniform(device: &wgpu::Device, bytes: &[u8]) -> ComputeInput {
    ComputeInput::Buffer(
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("captured compute input"),
            contents: bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        }),
    )
}
fn input(resource: &Resource) -> Result<ComputeInput, RuntimeError> {
    match resource {
        Resource::Uniform(buffer) | Resource::Storage { buffer, .. } => {
            Ok(ComputeInput::Buffer(buffer.clone()))
        }
        Resource::Texture(view) => Ok(ComputeInput::Image(view.clone())),
        Resource::Sampler(_, _) => Err(invalid("samplers are not compute operation arguments")),
    }
}
fn dimension(output: OwnedAllocation, axis: Dimension) -> Result<u32, RuntimeError> {
    match (output, axis) {
        (OwnedAllocation::Buffer(b), Dimension::Count) => Ok(b.elements()),
        (OwnedAllocation::Image { allocation, .. }, Dimension::Width) => {
            Ok(allocation.logical()[0])
        }
        (OwnedAllocation::Image { allocation, .. }, Dimension::Height) => {
            Ok(allocation.logical()[1])
        }
        _ => Err(invalid("resource dimension does not match output kind")),
    }
}

fn argument_value<'a>(
    argument: &'a Argument,
    settings: &'a StyleParameters,
) -> Result<(&'a str, &'a serde_json::Value), RuntimeError> {
    match argument {
        Argument::Constant { ty, value } => Ok((ty, value)),
        Argument::Setting { ty, offset, .. } => Ok((
            ty,
            settings
                .value_at(*offset)
                .ok_or_else(|| invalid("missing captured style setting"))?,
        )),
        _ => Err(invalid("argument is not a captured value")),
    }
}

impl PreparedCompute {
    pub async fn new(
        device: &wgpu::Device,
        wgsl: &str,
        manifest: &ManifestRoot,
        material: &str,
    ) -> Result<Self, RuntimeError> {
        let graph = ComputeGraph::new(manifest, material)?;
        let mut kernels = Vec::new();
        for node in graph.allocation_order() {
            kernels.push(Kernel {
                program: node.program.clone(),
                kernel: ComputeKernel::prepare(device, wgsl, node.program).await?,
                prerequisites: node.prerequisites.clone(),
            });
        }
        Ok(Self {
            kernels,
            features: device.features(),
        })
    }

    pub fn binding_type(
        &self,
        binding: &ManifestDrawComputeBinding,
        signature: &str,
    ) -> Result<wgpu::BindingType, RuntimeError> {
        let kernel = self
            .kernels
            .iter()
            .find(|k| k.program.pass == binding.producer)
            .ok_or_else(|| invalid("draw references a missing or foreign compute output"))?;
        let output = &kernel
            .program
            .compute_invocation
            .as_ref()
            .expect("validated invocation")
            .output;
        if compact(&binding.ty) != compact(&output.ty) {
            return Err(invalid(
                "draw resource type disagrees with producer return type",
            ));
        }
        if let Some(axis) = binding.dimension {
            if compact(signature) != "uniform<u32>"
                || !matches!(
                    (&output.layout, axis),
                    (ManifestComputeOutputLayout::Buffer { .. }, Dimension::Count)
                        | (
                            ManifestComputeOutputLayout::Image { .. },
                            Dimension::Width | Dimension::Height
                        )
                )
            {
                return Err(invalid("invalid draw logical dimension binding"));
            }
            return Ok(wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(4),
            });
        }
        match &output.layout {
            ManifestComputeOutputLayout::Buffer { element, stride } => {
                if compact(signature) != format!("buffer<{}>", compact(element)) {
                    return Err(invalid(
                        "draw storage element disagrees with compute output",
                    ));
                }
                Ok(wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(u64::from(*stride)),
                })
            }
            ManifestComputeOutputLayout::Image { format } => {
                let format = fresco_artifact::types::ImageFormat::parse(format)
                    .ok_or_else(|| invalid("unknown output image format"))?;
                if compact(signature) != format!("texture_2d<{}>", format.info().scalar.name()) {
                    return Err(invalid(
                        "draw sampled image type disagrees with compute output",
                    ));
                }
                let sample_type = super::recipe::format(format.info().name)?
                    .sample_type(None, Some(self.features))
                    .ok_or_else(|| invalid("compute output cannot be sampled"))?;
                Ok(wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                })
            }
        }
    }

    pub fn instantiate(
        &self,
        device: &wgpu::Device,
        settings: &StyleParameters,
        geometry: &BTreeMap<String, GeometryInputs>,
        external: &BTreeMap<String, Resource>,
        image_sizes: &BTreeMap<String, [u32; 2]>,
        budget: &mut u64,
    ) -> Result<ComputeFrame, RuntimeError> {
        let mut plans: BTreeMap<String, ComputeInvocationPlan> = BTreeMap::new();
        let read_dimension = |program: &ManifestGpuProgram,
                              parameter: &str,
                              axis: Dimension,
                              plans: &BTreeMap<String, ComputeInvocationPlan>|
         -> Result<u32, RuntimeError> {
            let invocation = program.compute_invocation.as_ref().expect("invocation");
            match invocation
                .arguments
                .get(parameter)
                .ok_or_else(|| invalid("missing resource argument"))?
            {
                Argument::Output { producer } => dimension(
                    plans
                        .get(producer)
                        .ok_or_else(|| invalid("output allocation is unavailable"))?
                        .output,
                    axis,
                ),
                Argument::External { resource, .. } => match axis {
                    Dimension::Width | Dimension::Height => image_sizes
                        .get(resource)
                        .map(|v| v[usize::from(axis == Dimension::Height)])
                        .ok_or_else(|| invalid("external image has no logical dimensions")),
                    Dimension::Count => {
                        let stride = program.bindings.iter().find(|b| matches!(program.compute_bindings.get(&b.name), Some(Source::Resource { parameter: p }) if p == parameter)).and_then(|b| b.element_stride).filter(|s| *s > 0).ok_or_else(|| invalid("external buffer has no element stride"))?;
                        let Some(Resource::Storage { buffer, .. }) = external.get(resource) else {
                            return Err(invalid("external buffer is missing"));
                        };
                        if !buffer.size().is_multiple_of(u64::from(stride)) {
                            return Err(invalid("external buffer has a partial element"));
                        }
                        u32::try_from(buffer.size().div_euclid(u64::from(stride)))
                            .map_err(|_| invalid("external buffer count exceeds u32"))
                    }
                },
                _ => Err(invalid("argument has no resource dimensions")),
            }
        };
        let geometry_count = |program: &ManifestGpuProgram,
                              parameter: &str,
                              member: &str|
         -> Result<u32, RuntimeError> {
            let invocation = program.compute_invocation.as_ref().expect("invocation");
            let Some(Argument::Geometry { producer, .. }) = invocation.arguments.get(parameter)
            else {
                return Err(invalid("not a geometry argument"));
            };
            let g = geometry
                .get(producer)
                .ok_or_else(|| invalid("prepared geometry is unavailable"))?;
            match program.compute_bindings.values().find_map(|b| match b {
                Source::Geometry {
                    parameter: p,
                    member: m,
                    role,
                } if p == parameter && m == member => Some(role),
                _ => None,
            }) {
                Some(GeometryRole::VertexCount) => Ok(g.vertex_count),
                Some(GeometryRole::IndexCount) => Ok(g.index_count),
                _ => Err(invalid("geometry member is not a CPU-known count")),
            }
        };
        let mut remaining = *budget;
        for kernel in &self.kernels {
            let program = &kernel.program;
            let invocation = program.compute_invocation.as_ref().expect("invocation");
            let read = |name: &str, member: Option<&str>| {
                let argument = invocation
                    .arguments
                    .get(name)
                    .ok_or_else(|| invalid("unknown host input"))?;
                if let Some(member) = member {
                    let value = if matches!(argument, Argument::Geometry { .. }) {
                        geometry_count(program, name, member)?
                    } else {
                        let axis = match member {
                            "count" => Dimension::Count,
                            "width" => Dimension::Width,
                            "height" => Dimension::Height,
                            _ => return Err(invalid("unknown logical dimension")),
                        };
                        read_dimension(program, name, axis, &plans)?
                    };
                    Ok(ComputeScalar::U32(value))
                } else {
                    let (ty, value) = argument_value(argument, settings)?;
                    scalar(ty, value)
                }
            };
            let plan =
                ComputeInvocationPlan::new(program, (&device.limits()).into(), remaining, &read)?;
            remaining = remaining
                .checked_sub(plan.output.bytes())
                .ok_or_else(|| invalid("frame compute budget exceeded"))?;
            plans.insert(program.pass.clone(), plan);
        }
        // All extents and preconditions succeeded before any output allocation.
        let mut frame = ComputeFrame::default();
        for kernel in &self.kernels {
            let program = &kernel.program;
            let invocation = program.compute_invocation.as_ref().expect("invocation");
            let mut inputs = BTreeMap::new();
            for binding in &program.bindings {
                if !binding.entry_access.contains_key(&program.entries[0].entry) {
                    continue;
                }
                let resource = match &program.compute_bindings[&binding.name] {
                    Source::Value { parameter, .. } => match &invocation.arguments[parameter] {
                        Argument::External { resource, .. } => input(
                            external
                                .get(resource)
                                .ok_or_else(|| invalid("missing engine value resource"))?,
                        )?,
                        argument => {
                            let (ty, value) = argument_value(argument, settings)?;
                            uniform(device, &value_bytes(ty, value)?)
                        }
                    },
                    Source::Resource { parameter } => match &invocation.arguments[parameter] {
                        Argument::Output { producer } => frame
                            .outputs
                            .get(producer)
                            .ok_or_else(|| invalid("missing producer allocation"))?
                            .input(),
                        Argument::External { resource, .. } => input(
                            external
                                .get(resource)
                                .ok_or_else(|| invalid("missing engine resource"))?,
                        )?,
                        _ => return Err(invalid("invalid resource argument")),
                    },
                    Source::Dimension { parameter, axis } => uniform(
                        device,
                        &read_dimension(program, parameter, *axis, &plans)?.to_le_bytes(),
                    ),
                    Source::Geometry {
                        parameter,
                        member,
                        role,
                    } => {
                        let Argument::Geometry { producer, .. } = &invocation.arguments[parameter]
                        else {
                            return Err(invalid("invalid geometry argument"));
                        };
                        let geometry = geometry
                            .get(producer)
                            .ok_or_else(|| invalid("missing geometry resources"))?;
                        match role {
                            GeometryRole::Vertices | GeometryRole::Indices => input(
                                geometry
                                    .resources
                                    .get(if *role == GeometryRole::Vertices {
                                        "vertices"
                                    } else {
                                        "indices"
                                    })
                                    .ok_or_else(|| invalid("missing prepared stream"))?,
                            )?,
                            GeometryRole::Bounds => input(
                                geometry
                                    .resources
                                    .get("bounds")
                                    .ok_or_else(|| invalid("missing prepared bounds uniform"))?,
                            )?,
                            GeometryRole::VertexCount | GeometryRole::IndexCount => uniform(
                                device,
                                &geometry_count(program, parameter, member)?.to_le_bytes(),
                            ),
                        }
                    }
                    Source::Output | Source::Dispatch => continue,
                };
                inputs.insert(binding.name.clone(), resource);
            }
            let plan = plans[&program.pass];
            let (output, pipeline, groups) = kernel
                .kernel
                .instantiate(device, plan, inputs)?
                .into_parts();
            frame.outputs.insert(program.pass.clone(), output);
            frame.nodes.push(FrameNode {
                name: program.pass.clone(),
                entry: program.entries[0].entry.clone(),
                pipeline,
                groups,
                threads: plan.dispatch.threads(),
                workgroup: program.workgroup_size.expect("validated workgroup"),
                prerequisites: kernel.prerequisites.clone(),
            });
        }
        *budget = remaining;
        Ok(frame)
    }
}

impl ComputeFrame {
    fn draw_expression(
        &self,
        invocation: &fresco_artifact::ManifestStyleInvocation,
        expression: &fresco_artifact::ManifestComputeExpression,
        geometry: &BTreeMap<String, GeometryInputs>,
        settings: &StyleParameters,
    ) -> Result<ComputeScalar, RuntimeError> {
        let host = invocation
            .host
            .as_ref()
            .ok_or_else(|| invalid("missing captured draw inputs"))?;
        let read = |parameter: &str, member: Option<&str>| {
            if let Some(member) = member {
                if let Some(declared) = host.geometry.get(parameter) {
                    let input = geometry
                        .get(&declared.producer)
                        .ok_or_else(|| invalid("missing draw geometry preparation"))?;
                    let count = if member == declared.vertex_count_member {
                        input.vertex_count
                    } else if member == declared.index_count_member {
                        input.index_count
                    } else {
                        return Err(invalid("unknown prepared geometry count"));
                    };
                    return Ok(ComputeScalar::U32(count));
                }
                let axis = match member {
                    "count" => Dimension::Count,
                    "width" => Dimension::Width,
                    "height" => Dimension::Height,
                    _ => return Err(invalid("unknown draw resource dimension")),
                };
                let output = invocation
                    .compute_inputs
                    .get(parameter)
                    .and_then(|binding| self.outputs.get(&binding.producer))
                    .ok_or_else(|| invalid("missing captured draw resource"))?;
                return Ok(ComputeScalar::U32(dimension(output.allocation(), axis)?));
            }
            let argument = host
                .arguments
                .get(parameter)
                .ok_or_else(|| invalid("missing captured draw argument"))?;
            let (ty, value) = argument_value(argument, settings)?;
            scalar(ty, value)
        };
        super::compute_expression::evaluate(expression, &read)
    }

    pub fn validate_draw(
        &self,
        invocation: &fresco_artifact::ManifestStyleInvocation,
        geometry: &BTreeMap<String, GeometryInputs>,
        settings: &StyleParameters,
    ) -> Result<(), RuntimeError> {
        if let Some(host) = &invocation.host {
            for requirement in &host.requirements {
                if self.draw_expression(invocation, requirement, geometry, settings)?
                    != ComputeScalar::Bool(true)
                {
                    return Err(invalid(format!(
                        "draw operation `{}` precondition failed",
                        invocation.operation
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn draw_spatial(
        &self,
        invocation: &fresco_artifact::ManifestStyleInvocation,
        geometry: &BTreeMap<String, GeometryInputs>,
        settings: &StyleParameters,
        bounds: Option<super::bounds::Bounds>,
        scene: &crate::profile::mesh::MeshSceneInputs,
    ) -> Result<(bool, Option<f32>), RuntimeError> {
        let source = |parameter: &str| {
            let declared = invocation
                .host
                .as_ref()
                .and_then(|h| h.geometry.get(parameter))
                .ok_or_else(|| invalid("missing captured bounds source"))?;
            let input = geometry
                .get(&declared.producer)
                .ok_or_else(|| invalid("missing prepared bounds producer"))?;
            if input.vertex_count == 0 {
                return Ok(None);
            }
            bounds.map(Some).ok_or_else(|| invalid("prepared bounds are unavailable; supply factory bounds or declare uncullable visibility without an explicit sort position"))
        };
        let visible = if let Some(declaration) = &invocation.bounds {
            let ComputeScalar::F32(amount) =
                self.draw_expression(invocation, &declaration.expansion, geometry, settings)?
            else {
                return Err(invalid("bounds expansion must be f32"));
            };
            if !amount.is_finite() || amount < 0.0 {
                return Err(invalid("bounds expansion must be finite and nonnegative"));
            }
            source(&declaration.geometry)?
                .map(|b| b.expand(amount)?.visible(&scene.view, &scene.projection))
                .transpose()?
                .unwrap_or(false)
        } else {
            true
        };
        let depth = invocation
            .sort_geometry
            .as_ref()
            .map(|parameter| {
                source(parameter)?
                    .map(|b| b.view_depth(&scene.view))
                    .transpose()
            })
            .transpose()?
            .flatten();
        Ok((visible, depth))
    }

    pub fn generated_geometry(
        &self,
        invocation: &fresco_artifact::ManifestStyleInvocation,
        source: &super::mesh_geometry::MeshGeometry,
        geometry: &BTreeMap<String, GeometryInputs>,
        settings: &StyleParameters,
    ) -> Result<super::mesh_geometry::MeshGeometry, RuntimeError> {
        let generated = invocation
            .generated_vertices
            .as_ref()
            .ok_or_else(|| invalid("missing generated vertex declaration"))?;
        let ComputeScalar::U32(offset) =
            self.draw_expression(invocation, &generated.base_vertex, geometry, settings)?
        else {
            return Err(invalid("generated vertex offset must be u32"));
        };
        let binding = invocation
            .compute_inputs
            .get(&generated.binding)
            .filter(|b| b.dimension.is_none())
            .ok_or_else(|| invalid("missing generated vertex buffer binding"))?;
        let output = self
            .outputs
            .get(&binding.producer)
            .ok_or_else(|| invalid("missing generated vertex allocation"))?;
        let OwnedAllocation::Buffer(allocation) = output.allocation() else {
            return Err(invalid("generated vertices require a buffer allocation"));
        };
        let ComputeInput::Buffer(buffer) = output.input() else {
            return Err(invalid("generated vertices require a buffer resource"));
        };
        source.with_generated_vertices(buffer, allocation.stride(), allocation.elements(), offset)
    }

    pub fn draw_resource(
        &self,
        device: &wgpu::Device,
        binding: &ManifestDrawComputeBinding,
    ) -> Result<Resource, RuntimeError> {
        let output = self
            .outputs
            .get(&binding.producer)
            .ok_or_else(|| invalid("draw output allocation is missing"))?;
        if let Some(axis) = binding.dimension {
            let ComputeInput::Buffer(buffer) =
                uniform(device, &dimension(output.allocation(), axis)?.to_le_bytes())
            else {
                unreachable!()
            };
            return Ok(Resource::Uniform(buffer));
        }
        match output.input() {
            ComputeInput::Buffer(buffer) => Ok(Resource::Storage {
                min_size: buffer.size(),
                buffer,
            }),
            ComputeInput::Image(view) => Ok(Resource::Texture(view)),
        }
    }
}
