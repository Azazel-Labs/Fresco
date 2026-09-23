//! Local-only GPU acceptance probe for owned buffers/images and chained reads.
use fresco_artifact::{
    ComputeScalar, ManifestComputeArgument, ManifestComputeBindingSource as Source,
    ManifestComputeDimension as Dimension, ManifestGpuProgram, ManifestRoot,
};
use fresco_example_engine::runtime::{
    RuntimeError,
    compute_graph::{ComputeGraph, ComputePrerequisite},
    compute_plan::{ComputeInvocationPlan, OwnedAllocation},
    owned_compute::{ComputeInput, ComputeOutput, OwnedCompute},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    time::Duration,
};
use wgpu::util::DeviceExt;

const SOURCE: &str = r#"
compute Build(size: u32, seed: u32) -> buffer<u32, read> {
    output values: buffer<u32, write>(size)
    workgroup_size: (64, 1, 1)
    dispatch threads(size, 1u, 1u)
    @compute fn main(id: uvec3) { values[id.x] = seed + id.x }
    return values
}
compute Copy(source: buffer<u32, read>) -> buffer<u32, read> {
    output values: buffer<u32, write>(source.count)
    workgroup_size: (64, 1, 1)
    dispatch threads(source.count, 1u, 1u)
    @compute fn main(id: uvec3) { values[id.x] = source[id.x] + 10u }
    return values
}
compute Field(size: u32, seed: u32) -> texture2d<r32uint, read> {
    output values: texture2d<r32uint, write>(size, 1u)
    workgroup_size: (64, 1, 1)
    dispatch threads(size, 1u, 1u)
    @compute fn main(id: uvec3) { values.store(id.xy, seed + id.x) }
    return values
}
compute ReadField(source: texture2d<r32uint, read>) -> buffer<u32, read> {
    output values: buffer<u32, write>(source.width)
    workgroup_size: (64, 1, 1)
    dispatch threads(source.width, 1u, 1u)
    @compute fn main(id: uvec3) { values[id.x] = source.load(id.xy).r + 20u }
    return values
}
compute QueryField(source: texture2d<r32uint, read>) -> buffer<u32, read> {
    output values: buffer<u32, write>(source.width)
    workgroup_size: (64, 1, 1)
    dispatch threads(source.width, 1u, 1u)
    @compute fn main(id: uvec3) { values[id.x] = textureDimensions(source).x }
    return values
}
style Probe for standard : StandardStyle {
    param size: u32 = 65u
    param seed: u32 = 7u
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 { return vec3(0.0) }
    for self {
        let values = Build(size: size, seed: seed)
        let copied = Copy(source: values)
        let field = Field(size: size, seed: seed)
        let read = ReadField(source: field)
        let dimensions = QueryField(source: field)
    }
}
surface item(sp: surf) -> material(standard) { properties { style: Probe }; compose { base(albedo: #fff) } }
"#;

async fn prepare(
    device: &wgpu::Device,
    wgsl: &str,
    program: &ManifestGpuProgram,
    plan: ComputeInvocationPlan,
    size: u32,
    seed: u32,
    source: Option<&ComputeOutput>,
) -> Result<OwnedCompute, RuntimeError> {
    let read = |name: &str, member: Option<&str>| {
        let value = match (name, member) {
            ("size", None) => size,
            ("seed", None) => seed,
            ("source", Some("count")) => match source.expect("source argument").allocation() {
                OwnedAllocation::Buffer(buffer) => buffer.elements(),
                _ => panic!("expected buffer"),
            },
            ("source", Some("width")) => match source.expect("source argument").allocation() {
                OwnedAllocation::Image { allocation, .. } => allocation.logical()[0],
                _ => panic!("expected image"),
            },
            _ => panic!("unexpected host input {name}.{member:?}"),
        };
        Ok(ComputeScalar::U32(value))
    };
    let uniform = |value: u32| {
        ComputeInput::Buffer(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("probe scalar"),
                contents: &value.to_le_bytes(),
                usage: wgpu::BufferUsages::UNIFORM,
            }),
        )
    };
    let mut inputs = BTreeMap::new();
    for binding in &program.bindings {
        if !binding.entry_access.contains_key(&program.entries[0].entry) {
            continue;
        }
        let input = match &program.compute_bindings[&binding.name] {
            Source::Value { parameter, .. } => {
                let ComputeScalar::U32(value) = read(parameter, None)? else {
                    unreachable!()
                };
                uniform(value)
            }
            Source::Resource { .. } => source.expect("resource argument").input(),
            Source::Dimension { parameter, axis } => {
                let member = match axis {
                    Dimension::Count => "count",
                    Dimension::Width => "width",
                    Dimension::Height => "height",
                };
                let ComputeScalar::U32(value) = read(parameter, Some(member))? else {
                    unreachable!()
                };
                uniform(value)
            }
            Source::Output | Source::Dispatch => continue,
            Source::Geometry { .. } => panic!("probe does not use geometry"),
        };
        inputs.insert(binding.name.clone(), input);
    }
    OwnedCompute::prepare(device, wgsl, program, plan, inputs).await
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files_for_recipe("forward");
    files.insert("main.fr".into(), SOURCE.into());
    let artifact = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&artifact.manifest)?;
    let graph = ComputeGraph::new(&manifest, "item")?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await?;
    println!("Adapter: {}", adapter.get_info().name);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await?;
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut checks = Vec::new();
    // All invocations coexist before submission: sharing a program cannot alias
    // their outputs. Counts cross a workgroup boundary and exercise empty work.
    for (count, seed) in [(65u32, 7u32), (7, 100), (0, 999)] {
        let allocations = graph.plan_allocations(
            (&device.limits()).into(),
            1024 * 1024,
            &|_, parameter, member| {
                assert!(member.is_none());
                Ok(ComputeScalar::U32(match parameter {
                    "size" => count,
                    "seed" => seed,
                    _ => panic!("unexpected captured setting"),
                }))
            },
        )?;
        let mut pending: BTreeMap<String, OwnedCompute> = BTreeMap::new();
        for node in graph.allocation_order() {
            let invocation = node.program.compute_invocation.as_ref().unwrap();
            let source = invocation.arguments.values().find_map(|a| match a {
                ManifestComputeArgument::Output { producer } => Some(pending[producer].output()),
                ManifestComputeArgument::Setting { .. } => None,
                _ => panic!("unexpected GPU probe argument"),
            });
            let operation = prepare(
                &device,
                &artifact.wgsl,
                node.program,
                allocations[node.program.pass.as_str()],
                count,
                seed,
                source,
            )
            .await?;
            pending.insert(node.program.pass.clone(), operation);
        }
        let mut completed = BTreeSet::new();
        let mut outputs = BTreeMap::new();
        let mut trace = Vec::new();
        while !pending.is_empty() {
            // Prefer the last ready node to stress dependencies rather than
            // accidentally getting correct ordering from declaration order.
            let node = graph
                .ready(&completed)
                .last()
                .expect("acyclic ready operation");
            let name = &node.program.pass;
            let operation = node
                .program
                .compute_invocation
                .as_ref()
                .unwrap()
                .operation
                .clone();
            trace.push(operation.clone());
            outputs.insert(
                operation,
                pending.remove(name).unwrap().encode(&mut encoder),
            );
            completed.insert(ComputePrerequisite::Operation(name.clone()));
        }
        assert!(
            trace.iter().position(|n| n == "QueryField") < trace.iter().position(|n| n == "Field")
        );
        for (operation, increment) in [("Copy", 10u32), ("ReadField", 20), ("QueryField", 0)] {
            let output = outputs.remove(operation).unwrap();
            let ComputeInput::Buffer(buffer) = output.input() else {
                panic!("buffer result")
            };
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("probe readback"),
                size: buffer.size(),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&buffer, 0, &staging, 0, buffer.size());
            let expected: Vec<_> = if count == 0 {
                vec![0]
            } else if operation == "QueryField" {
                vec![count; usize::try_from(count)?]
            } else {
                (0..count).map(|i| seed + increment + i).collect()
            };
            checks.push((staging, expected));
        }
    }
    let submission = queue.submit([encoder.finish()]);
    for (buffer, expected) in checks {
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).expect("receiver");
            });
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission.clone()),
            timeout: Some(Duration::from_secs(20)),
        })?;
        receive.recv_timeout(Duration::from_secs(20))??;
        {
            let bytes = buffer.slice(..).get_mapped_range()?;
            let actual: Vec<_> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b))
                .collect();
            assert_eq!(
                actual, expected,
                "GPU result disagrees with authored operation"
            );
        }
        buffer.unmap();
    }
    println!(
        "Owned compute: buffer/image consumers, rounded dispatch, empty outputs, and independent invocations passed."
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| pollster::block_on(run()).map_err(|e| e.to_string()))?
        .join()
        .expect("GPU probe panicked")
        .map_err(Into::into)
}
