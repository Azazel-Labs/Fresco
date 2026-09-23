use std::{collections::BTreeMap, error::Error};

use fresco_artifact::ManifestVertexFactory;
use fresco_example_engine::runtime::{mesh_geometry::MeshGeometry, vertices::VertexValues};

use super::{FORMAT, readback};

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let factory: ManifestVertexFactory = serde_json::from_value(serde_json::json!({
        "name":"probe", "vertex_format":"Probe", "array_stride":16,
        "attributes":[
            {"name":"position", "type":"vec2", "shader_location":3, "offset":4,
             "gpu_format":"float32x2", "required":true, "defaulted":false},
            {"name":"tag", "type":"u32", "shader_location":0, "offset":0,
             "gpu_format":"uint32", "required":true, "defaulted":false}
        ]
    }))?;
    let positions = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
    let streams = BTreeMap::from([
        ("position".into(), VertexValues::F32(positions.as_slice())),
        ("tag".into(), VertexValues::U32(&[16_777_217; 3])),
    ]);
    let geometry =
        MeshGeometry::prepare(device, queue, &factory, 3, &streams, Some(&[2, 0, 1])).await?;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("geometry transport probe"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) tag: u32,
}
@vertex fn vertex(@location(3) point: vec2<f32>, @location(0) tag: u32) -> Output {
    return Output(vec4<f32>(point, 0.0, 1.0), tag);
}
@fragment fn fragment(input: Output) -> @location(0) vec4<f32> {
    return select(vec4<f32>(0.0,1.0,0.0,1.0), vec4<f32>(1.0,0.0,0.0,1.0), input.tag == 16777217u);
}
"#
            .into(),
        ),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("geometry transport probe"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[Some(geometry.vertex_layout())],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
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
    let render = |geometry: &MeshGeometry| -> Result<Vec<u8>, Box<dyn Error>> {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            geometry.draw(&mut pass);
        }
        queue.submit([encoder.finish()]);
        readback(device, queue, target)
    };
    let red = render(&geometry)?;
    assert!(
        red.as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 255]),
        "integer vertex attributes must not pass through f32"
    );
    let unindexed = MeshGeometry::prepare(device, queue, &factory, 3, &streams, None).await?;
    assert_eq!(
        render(&unindexed)?,
        red,
        "indexed and non-indexed geometry agree"
    );
    assert!(
        MeshGeometry::prepare(device, queue, &factory, 3, &streams, Some(&[0, 1, 3]))
            .await
            .is_err()
    );
    assert_eq!(
        render(&geometry)?,
        red,
        "invalid replacement preserves installed geometry"
    );
    let mut edited = streams.clone();
    edited.insert("tag".into(), VertexValues::U32(&[16_777_216; 3]));
    let replacement =
        MeshGeometry::prepare(device, queue, &factory, 3, &edited, Some(&[0, 1, 2])).await?;
    assert!(
        render(&replacement)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 255, 0, 255])
    );
    assert_eq!(render(&geometry)?, red, "replacement owns distinct buffers");
    let empty_streams = BTreeMap::from([
        ("position".into(), VertexValues::F32(&[])),
        ("tag".into(), VertexValues::U32(&[])),
    ]);
    for indices in [None, Some([].as_slice())] {
        let empty =
            MeshGeometry::prepare(device, queue, &factory, 0, &empty_streams, indices).await?;
        assert!(
            render(&empty)?
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [0, 0, 0, 255])
        );
    }
    println!(
        "Verified GPU vertex layouts, integer transport, indexed/non-indexed draws, empty geometry, and replacement isolation."
    );
    Ok(())
}
