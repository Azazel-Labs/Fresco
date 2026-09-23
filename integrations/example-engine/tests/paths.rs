use fresco_artifact::{
    ManifestCanvas, ManifestPathData, ManifestPathSegment, PATH_SEGMENT_LAYOUT, PATH_SEGMENT_STRIDE,
};
use fresco_example_engine::runtime::{paths::PathBuffers, storage::StorageLimits};
use serde_json::json;

fn limits() -> StorageLimits {
    StorageLimits {
        max_bind_groups: 4,
        max_bindings_per_group: 16,
        max_buffers_per_stage: 8,
        max_buffer_bytes: 4096,
    }
}

fn canvas() -> ManifestCanvas {
    let mut canvas: ManifestCanvas =
        serde_json::from_value(json!({"name":"probe", "pass_plan":{}, "path_buffers":[{
            "name":"path_0","group":2,"binding":0,"segments":2
        }]}))
        .unwrap();
    let row = ManifestPathSegment {
        p0: [1.0, 2.0],
        p1: [3.0, 4.0],
        p2: [5.0, 6.0],
        p3: [7.0, 8.0],
        s0: 9.0,
        len: 10.0,
        kind: 1,
        mid_u: 0.25,
    };
    canvas.path_buffers[0].data = Some(ManifestPathData {
        layout: PATH_SEGMENT_LAYOUT.into(),
        stride: PATH_SEGMENT_STRIDE,
        rows: vec![row.clone(), row],
    });
    canvas
}

#[test]
fn typed_path_rows_preserve_integer_bits_padding_and_record_stride() {
    let canvas = canvas();
    let paths = PathBuffers::new(&canvas, limits()).unwrap();
    let upload = paths.uploads().next().unwrap();
    assert_eq!((upload.group, upload.binding), (2, 0));
    assert_eq!(upload.bytes.len(), 112);
    let words: Vec<_> = upload.bytes[..40]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    assert_eq!(words, [1., 2., 3., 4., 5., 6., 7., 8., 9., 10.]);
    assert_eq!(&upload.bytes[40..44], &1_u32.to_le_bytes());
    assert_eq!(&upload.bytes[44..48], &0.25_f32.to_le_bytes());
    assert_eq!(&upload.bytes[48..56], &[0; 8]);
    assert_eq!(&upload.bytes[..56], &upload.bytes[56..]);
}

#[test]
fn legacy_or_malformed_geometry_fails_explicitly() {
    let mut legacy = canvas();
    legacy.path_buffers[0].data = None;
    let error = PathBuffers::new(&legacy, limits())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("recompile"), "{error}");
    for mutate in [
        |data: &mut ManifestPathData| data.layout = "unknown".into(),
        |data: &mut ManifestPathData| data.stride = 48,
        |data: &mut ManifestPathData| {
            data.rows.pop();
        },
        |data: &mut ManifestPathData| data.rows[0].p0[0] = f32::NAN,
        |data: &mut ManifestPathData| data.rows[0].len = -1.0,
        |data: &mut ManifestPathData| data.rows[0].s0 = f32::INFINITY,
        |data: &mut ManifestPathData| data.rows[0].kind = 2,
        |data: &mut ManifestPathData| data.rows[0].mid_u = 1.1,
    ] {
        let mut invalid = canvas();
        mutate(invalid.path_buffers[0].data.as_mut().unwrap());
        assert!(PathBuffers::new(&invalid, limits()).is_err());
    }
}

#[test]
fn bindings_and_combined_storage_limits_are_checked_before_gpu_allocation() {
    let base = canvas();
    for invalid_limits in [
        StorageLimits {
            max_buffer_bytes: 111,
            ..limits()
        },
        StorageLimits {
            max_bind_groups: 2,
            ..limits()
        },
        StorageLimits {
            max_buffers_per_stage: 0,
            ..limits()
        },
    ] {
        assert!(PathBuffers::new(&base, invalid_limits).is_err());
    }
    let mut collision = base.clone();
    collision.sampler = Some(fresco_artifact::ManifestSampler {
        group: 2,
        binding: 0,
    });
    assert!(PathBuffers::new(&collision, limits()).is_err());
    let mut combined = base.clone();
    combined.storage_params = vec![fresco_artifact::ManifestStorageParam {
        name: "weights".into(),
        ty: "array<f32>".into(),
        param_type: None,
        group: 2,
        binding: 1,
    }];
    assert!(
        PathBuffers::new(
            &combined,
            StorageLimits {
                max_buffers_per_stage: 1,
                ..limits()
            }
        )
        .is_err()
    );
    assert!(PathBuffers::new(&combined, limits()).is_ok());
    combined.storage_params[0].binding = 0;
    assert!(PathBuffers::new(&combined, limits()).is_err());
    let mut duplicate = base.clone();
    duplicate.path_buffers.push(base.path_buffers[0].clone());
    assert!(PathBuffers::new(&duplicate, limits()).is_err());
}

#[test]
fn authored_path_example_contains_every_byte_needed_by_an_independent_host() {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        include_str!("../examples/path_canvas.fr").into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let canvas = &manifest.canvases[0];
    assert_eq!(canvas.path_buffers.len(), 1);
    assert_eq!(canvas.storage_params.len(), 1);
    let buffer = &canvas.path_buffers[0];
    assert!(buffer.segments > 64);
    let path_limits = StorageLimits {
        max_buffer_bytes: 65536,
        ..limits()
    };
    let paths = PathBuffers::new(canvas, path_limits).unwrap();
    assert_eq!(
        paths.uploads().next().unwrap().bytes.len(),
        buffer.segments * 56
    );
    assert_eq!(buffer.data.as_ref().unwrap().rows[0].p0, [0.1, 0.3]);
    let mut collision = canvas.clone();
    collision.path_buffers[0].binding = collision.storage_params[0].binding;
    assert!(PathBuffers::new(&collision, path_limits).is_err());
    assert!(
        fresco_example_engine::runtime::parameters::CanvasParameters::new(&collision, path_limits)
            .is_err()
    );
}
