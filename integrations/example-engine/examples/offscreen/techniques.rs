//! A non-particle graph: independent producers join at a depth-only draw.
use fresco_artifact::{ManifestRoot, ManifestTechniqueOperation};
use fresco_example_engine::runtime::technique::{Executor, Invocation, Parameters, Pipeline};
use std::{collections::BTreeMap, error::Error};

const SOURCE: &str = r#"
@shader pass produce {
    stage: compute
    binding { @group(0) @binding(0) @access(write) output: buffer<u32> }
    @compute @workgroup_size(1) fn run() { output[u32(0)] = u32(1) }
}
struct Vertex { @builtin(position) position: vec4 }
@shader pass raster {
    stage: raster
    binding {
        @group(0) @binding(0) a: buffer<u32>
        @group(0) @binding(1) b: buffer<u32>
    }
    @vertex fn vertex(@builtin(vertex_index) id: u32) -> Vertex {
        var p = vec2(-1.0, -1.0)
        if id == u32(1) { p = vec2(3.0, -1.0) }
        if id == u32(2) { p = vec2(-1.0, 3.0) }
        return Vertex(vec4(p, f32(a[u32(0)] + b[u32(0)]) * 0.125, 1.0))
    }
}
@technique @buffer(left, 4) @buffer(right, 4)
@image(result, depth32float) @provider(result, borrowed_target) @output(depth, result)
pipeline(compute) probe {
    @node(raster) @draw_depth(vertex, 3) @bind(a, left) @bind(b, right) @depth(result) raster
    @node(left) @dispatch(run, extent) @bind(output, left) produce
    @node(right) @dispatch(run, extent) @bind(output, right) produce
}
"#;

pub async fn verify(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    files.insert("main.fr".into(), SOURCE.into());
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("technique compile: {e:?}"))?;
    let root: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let graph = root
        .techniques
        .iter()
        .find(|t| t.name == "probe")
        .unwrap()
        .clone();
    assert_eq!(graph.steps.last().unwrap().after, ["left", "right"]);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("generic technique probe"),
        source: wgpu::ShaderSource::Wgsl(output.wgsl.into()),
    });
    let buffers: BTreeMap<_, _> = ["left", "right"]
        .map(|name| {
            (
                name.to_string(),
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(name),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }),
            )
        })
        .into();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("generic depth target"),
        size: wgpu::Extent3d {
            width: super::SIZE,
            height: super::SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut pipelines = BTreeMap::new();
    let mut groups = BTreeMap::new();
    for step in &graph.steps {
        let pipeline = match &step.operation {
            ManifestTechniqueOperation::Compute { entry, .. } => Pipeline::Compute(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(&step.name),
                    layout: None,
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            ),
            ManifestTechniqueOperation::Draw {
                vertex, fragment, ..
            } => {
                assert!(fragment.is_none());
                Pipeline::Draw(
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(&step.name),
                        layout: None,
                        vertex: wgpu::VertexState {
                            module: &shader,
                            entry_point: Some(vertex),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        fragment: None,
                        primitive: Default::default(),
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: Some(true),
                            depth_compare: Some(wgpu::CompareFunction::Less),
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: Default::default(),
                        multiview_mask: None,
                        cache: None,
                    }),
                )
            }
        };
        let program = root
            .gpu_programs
            .iter()
            .find(|p| p.pass == step.pass)
            .unwrap();
        let entries: Vec<_> = program
            .bindings
            .iter()
            .map(|binding| wgpu::BindGroupEntry {
                binding: binding.binding,
                resource: buffers[&step.bindings[&binding.name]].as_entire_binding(),
            })
            .collect();
        let layout = match &pipeline {
            Pipeline::Compute(p) => p.get_bind_group_layout(0),
            Pipeline::Draw(p) => p.get_bind_group_layout(0),
        };
        groups.insert(
            step.name.clone(),
            vec![(
                0,
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&step.name),
                    layout: &layout,
                    entries: &entries,
                }),
            )],
        );
        pipelines.insert(step.name.clone(), pipeline);
    }
    let runner = Executor::new(graph, pipelines, &device.limits())?;
    let mut parameters = Parameters::default();
    parameters.extents.insert("extent".into(), [1, 1, 1]);
    let invoke = |parameters: &Parameters, encoder: &mut wgpu::CommandEncoder| {
        runner.encode(
            encoder,
            Invocation {
                parameters,
                groups: groups
                    .iter()
                    .map(|(name, groups)| (name.clone(), groups.as_slice()))
                    .collect(),
                attachments: BTreeMap::from([("result".into(), &view)]),
                geometry: BTreeMap::new(),
            },
        )
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(invoke(&Parameters::default(), &mut encoder).is_err());
    invoke(&parameters, &mut encoder)?;
    queue.submit([encoder.finish()]);
    let pixels = super::readback(device, queue, &texture)?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| (f32::from_le_bytes(*p) - 0.25).abs() < 0.0001)
    );
    parameters.extents.insert("extent".into(), [u32::MAX, 1, 1]);
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(invoke(&parameters, &mut encoder).is_err());
    println!(
        "Generic technique: independent producers, depth-only draw, and invocation validation passed"
    );
    Ok(())
}
