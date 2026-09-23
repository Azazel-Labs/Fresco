//! The same compiled contribution instantiated for two independent mesh draws.
use super::{FORMAT, SIZE, readback};
use fresco_artifact::{
    ManifestAttachmentOps, ManifestDrawCount, ManifestRoot, ManifestTechnique,
    ManifestTechniqueOperation, ManifestTechniqueStep,
};
use fresco_example_engine::runtime::{
    mesh_geometry::MeshGeometry,
    technique::{Executor, Invocation, Parameters, Pipeline},
    vertices::VertexValues,
};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
};

pub async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let files = HashMap::from([
        ("engine/engine.fr".into(), include_str!("../../../../crates/fresco/tests/fixtures/engines/style-operations.fr").into()),
        ("main.fr".into(), "surface item(sp: SamplePoint) -> material(Value) { properties { action: Extra }; compose { Value(tint: #fff) } }".into()),
    ]);
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&compiled.manifest)?;
    let recipe = manifest.renderers.iter().find(|r| r.selected).unwrap();
    let operation = recipe
        .steps
        .iter()
        .find(|s| {
            s.invocation
                .as_ref()
                .is_some_and(|i| i.operation == "Paint")
        })
        .unwrap();
    let display = recipe.steps.iter().find(|s| s.name == "display").unwrap();
    assert!(display.after.contains(&operation.name));
    let factory = &manifest.vertex_factories[0];
    let mut geometry = Vec::new();
    for offset in [0.0, 1.0] {
        let points = [
            -0.9 + offset,
            -0.7,
            0.0,
            -0.1 + offset,
            -0.7,
            0.0,
            -0.5 + offset,
            0.7,
            0.0,
        ];
        geometry.push(
            MeshGeometry::prepare(
                device,
                queue,
                factory,
                3,
                &BTreeMap::from([("point".into(), VertexValues::F32(&points))]),
                None,
            )
            .await?,
        );
    }
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("independent contribution probe"),
        source: wgpu::ShaderSource::Wgsl(compiled.wgsl.into()),
    });
    let mut prepared = BTreeMap::new();
    let mut steps = Vec::new();
    let mut geometries = BTreeMap::new();
    for (instance, geometry) in geometry.iter().enumerate() {
        // Inspect the intermediate target before presentation. This probe instantiates
        // the base draw and reusable operation directly with two geometry bindings;
        // the compiled presentation edge is asserted above.
        for declaration in recipe.steps.iter().filter(|s| s.name != "display") {
            let pass = manifest.surfaces[0]
                .mesh_passes
                .iter()
                .find(|p| p.pass == declaration.pass)
                .unwrap();
            let vertex = &pass
                .entries
                .iter()
                .find(|e| e.stage == "vertex")
                .unwrap()
                .entry;
            let fragment = &pass
                .entries
                .iter()
                .find(|e| e.stage == "fragment")
                .unwrap()
                .entry;
            prepared.entry(declaration.pass.clone()).or_insert_with(|| {
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&declaration.pass),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(vertex),
                        compilation_options: Default::default(),
                        buffers: &[Some(geometry.vertex_layout())],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fragment),
                        compilation_options: Default::default(),
                        targets: &[Some(FORMAT.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            });
            let name = format!("object_{instance}_{}", declaration.name);
            let after = steps
                .last()
                .map(|s: &ManifestTechniqueStep| vec![s.name.clone()])
                .unwrap_or_default();
            let load = !steps.is_empty();
            steps.push(ManifestTechniqueStep {
                name: name.clone(),
                pass: declaration.pass.clone(),
                enabled: None,
                bindings: BTreeMap::new(),
                reads: vec![],
                writes: vec!["result".into()],
                after,
                attachments: BTreeMap::from([(
                    "result".into(),
                    ManifestAttachmentOps { load, store: true },
                )]),
                operation: ManifestTechniqueOperation::Draw {
                    vertex: vertex.clone(),
                    fragment: Some(fragment.clone()),
                    vertices: 3,
                    instances: ManifestDrawCount::Fixed(1),
                    colors: declaration.colors.clone(),
                    depth: None,
                },
            });
            geometries.insert(name, geometry);
        }
    }
    let pipelines: BTreeMap<_, _> = steps
        .iter()
        .map(|s| (s.name.clone(), Pipeline::Draw(prepared[&s.pass].clone())))
        .collect();
    let graph = ManifestTechnique {
        surface: None,
        metadata: BTreeMap::new(),
        name: "two_instances".into(),
        resources: vec![],
        steps,
        outputs: BTreeMap::new(),
    };
    let groups: BTreeMap<String, &[(u32, wgpu::BindGroup)]> = graph
        .steps
        .iter()
        .map(|s| (s.name.clone(), &[][..]))
        .collect();
    let parameters = Parameters {
        enabled: BTreeMap::from([("first".into(), false)]),
        ..Default::default()
    };
    let invoke = |runner: &Executor, encoder: &mut wgpu::CommandEncoder| {
        runner.encode(
            encoder,
            Invocation {
                parameters: &parameters,
                groups: groups.clone(),
                attachments: BTreeMap::from([("result".into(), view)]),
                geometry: geometries.clone(),
            },
        )
    };
    let runner = Executor::new(graph.clone(), pipelines.clone(), &device.limits())?;
    let mut encoder = device.create_command_encoder(&Default::default());
    invoke(&runner, &mut encoder)?;
    queue.submit([encoder.finish()]);
    let pixels = readback(device, queue, output)?;
    for half in [0, 1] {
        for expected in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .enumerate()
                    .any(
                        |(i, p)| (i % SIZE as usize).div_euclid((SIZE as usize).div_euclid(2))
                            == half
                            && *p == expected
                    ),
                "each mesh retains its first draw and its own contribution"
            );
        }
    }
    for failure in 0..3 {
        let mut invalid = graph.clone();
        if failure == 2 {
            invalid.steps[0].enabled = Some("first".into());
        } else if failure == 1 {
            invalid.steps[0]
                .attachments
                .get_mut("result")
                .unwrap()
                .store = false;
        } else {
            invalid.steps[0].attachments.get_mut("result").unwrap().load = true;
        }
        let runner = Executor::new(invalid, pipelines.clone(), &device.limits())?;
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(invoke(&runner, &mut encoder).is_err());
        queue.submit([encoder.finish()]);
        assert_eq!(pixels, readback(device, queue, output)?);
    }
    println!(
        "Contributions: two independent geometry invocations preserve prior pixels; invalid loads and discarded contents fail atomically."
    );
    Ok(())
}
