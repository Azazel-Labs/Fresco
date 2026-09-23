use fresco_artifact::{ManifestGlobalUniform, ManifestGlobalUniformField, ManifestRoot};
use fresco_example_engine::profile::{FrameInputs, frame_uniform};
use fresco_example_engine::runtime::RuntimeError;
use fresco_example_engine::runtime::uniforms::{UniformLimits, UniformSet, UniformValue};

const LIMITS: UniformLimits = UniformLimits {
    max_bind_groups: 4,
    max_bindings_per_bind_group: 16,
    max_uniform_buffer_binding_size: 256,
};

fn field(name: &str, offset: u32, components: u32, scalar: &str) -> ManifestGlobalUniformField {
    ManifestGlobalUniformField {
        name: name.into(),
        offset,
        components,
        scalar_type: scalar.into(),
        ty: if components == 1 {
            scalar.into()
        } else {
            format!("vec{components}<{scalar}>")
        },
    }
}

fn weather() -> ManifestGlobalUniform {
    ManifestGlobalUniform {
        name: "weather".into(),
        ty: "Weather".into(),
        group: 2,
        binding: 5,
        byte_size: 48,
        fields: vec![
            field("amount", 0, 1, "f32"),
            field("wind", 16, 3, "f32"),
            field("origin", 32, 2, "f32"),
        ],
    }
}

fn bytes(set: &UniformSet) -> Vec<Vec<u8>> {
    set.uploads()
        .unwrap()
        .map(|upload| upload.bytes.to_vec())
        .collect()
}

#[test]
fn reflected_offsets_and_binding_numbers_drive_packing_with_zero_padding() {
    let mut set = UniformSet::new(vec![weather()], LIMITS).unwrap();
    assert!(set.uploads().is_none());
    set.update(|_, field| {
        Some(UniformValue::F32(match field.name.as_str() {
            "amount" => vec![7.0],
            "wind" => vec![1.0, 2.0, 3.0],
            "origin" => vec![4.0, 5.0],
            _ => panic!("unexpected field"),
        }))
    })
    .unwrap();
    let upload = set.uploads().unwrap().next().unwrap();
    assert_eq!((upload.group, upload.binding), (2, 5));
    let expected: Vec<u8> = [
        7.0_f32, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 0.0, 0.0,
    ]
    .into_iter()
    .flat_map(f32::to_le_bytes)
    .collect();
    assert_eq!(upload.bytes, expected);
}

#[test]
fn frame_and_arbitrary_fields_share_a_sparse_group_without_defaulting_missing_values() {
    // Preserve the legacy host-resolver scenario: two independent blocks in
    // group 3, with binding 5 and padded fields supplied by a custom host.
    let frame_definition = ManifestGlobalUniform {
        name: "frame".into(),
        ty: "FrameGlobals".into(),
        group: 3,
        binding: 0,
        byte_size: 16,
        fields: vec![
            field("time", 0, 1, "f32"),
            field("delta_time", 4, 1, "f32"),
            field("resolution", 8, 2, "f32"),
        ],
    };
    let mut custom = weather();
    custom.group = 3;
    let mut set = UniformSet::new(vec![frame_definition, custom], LIMITS).unwrap();
    let frame = FrameInputs {
        time: 2.0,
        delta_time: 0.25,
        physical_size: [800, 600],
    };
    let error = set.update(|_, _| None).unwrap_err();
    assert!(error.to_string().contains("frame.time"));
    assert!(set.uploads().is_none());
    set.update(|uniform, field| {
        if uniform.name == "frame" {
            frame_uniform(uniform, field, frame)
        } else {
            Some(UniformValue::F32(match field.name.as_str() {
                "amount" => vec![7.0],
                "wind" => vec![1.0, 2.0, 3.0],
                "origin" => vec![4.0, 5.0],
                _ => panic!("unexpected custom field"),
            }))
        }
    })
    .unwrap();
    let uploads: Vec<_> = set.uploads().unwrap().collect();
    assert_eq!(uploads.len(), 2);
    for (upload, binding, values) in [
        (&uploads[0], 0, vec![2.0_f32, 0.25, 800.0, 600.0]),
        (
            &uploads[1],
            5,
            vec![7.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 0.0, 0.0],
        ),
    ] {
        assert_eq!((upload.group, upload.binding), (3, binding));
        assert_eq!(
            upload.bytes,
            values
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn integer_components_preserve_bits_including_values_above_float_precision() {
    let mut def = weather();
    def.fields = vec![
        field("signed", 0, 2, "i32"),
        field("unsigned", 16, 4, "u32"),
    ];
    let mut set = UniformSet::new(vec![def], LIMITS).unwrap();
    set.update(|_, field| {
        Some(if field.name == "signed" {
            UniformValue::I32(vec![i32::MIN, -1])
        } else {
            UniformValue::U32(vec![u32::MAX, 16_777_217, 0, 42])
        })
    })
    .unwrap();
    let data = bytes(&set).remove(0);
    assert_eq!(&data[..8], &[0, 0, 0, 128, 255, 255, 255, 255]);
    assert_eq!(&data[16..24], &[255, 255, 255, 255, 1, 0, 0, 1]);
}

#[test]
fn malformed_layouts_and_device_limit_violations_are_rejected() {
    let mut cases = Vec::new();
    let mut def = weather();
    def.fields[1].offset = 4;
    cases.push(def);
    let mut def = weather();
    def.fields[1].offset = 0;
    cases.push(def);
    let mut def = weather();
    def.fields[1].offset = u32::MAX - 3;
    cases.push(def);
    let mut def = weather();
    def.fields[1].offset = 48;
    cases.push(def);
    let mut def = weather();
    def.fields[1].components = 5;
    cases.push(def);
    let mut def = weather();
    def.fields[1].components = 0;
    cases.push(def);
    let mut def = weather();
    def.fields[1].scalar_type = "bool".into();
    cases.push(def);
    let mut def = weather();
    def.fields[1].name = "amount".into();
    cases.push(def);
    let mut def = weather();
    def.byte_size = 47;
    cases.push(def);
    let mut def = weather();
    def.byte_size = 0;
    cases.push(def);
    let mut def = weather();
    def.byte_size = 272;
    cases.push(def);
    let mut def = weather();
    def.group = 4;
    cases.push(def);
    let mut def = weather();
    def.binding = 16;
    cases.push(def);
    for def in cases {
        assert!(matches!(
            UniformSet::new(vec![def], LIMITS),
            Err(RuntimeError::UniformLayout { .. })
        ));
    }
    let mut other = weather();
    other.name = "other".into();
    assert!(
        UniformSet::new(vec![weather(), other], LIMITS)
            .unwrap_err()
            .to_string()
            .contains("duplicate group/binding")
    );
    let mut other = weather();
    other.binding = 6;
    assert!(
        UniformSet::new(vec![weather(), other], LIMITS)
            .unwrap_err()
            .to_string()
            .contains("duplicate uniform name")
    );
}

#[test]
fn failed_updates_preserve_every_buffer_and_recovery_replaces_every_value() {
    let first = ManifestGlobalUniform {
        fields: vec![field("value", 0, 1, "f32")],
        ..weather()
    };
    let second = ManifestGlobalUniform {
        name: "other".into(),
        binding: 6,
        ..first.clone()
    };
    let mut set = UniformSet::new(vec![first, second], LIMITS).unwrap();
    assert!(set.update(|_, _| None).is_err());
    assert!(set.uploads().is_none());
    set.update(|_, _| Some(UniformValue::F32(vec![1.0])))
        .unwrap();
    let before = bytes(&set);
    for bad in [
        None,
        Some(UniformValue::F32(vec![f32::NAN])),
        Some(UniformValue::F32(vec![f32::INFINITY])),
        Some(UniformValue::F32(vec![2.0, 3.0])),
        Some(UniformValue::U32(vec![2])),
    ] {
        let error = set
            .update(|def, _| {
                if def.name == "weather" {
                    Some(UniformValue::F32(vec![9.0]))
                } else {
                    bad.clone()
                }
            })
            .unwrap_err();
        assert!(error.to_string().contains("other.value"));
        assert_eq!(bytes(&set), before);
    }
    set.update(|_, _| Some(UniformValue::F32(vec![2.0])))
        .unwrap();
    for data in bytes(&set) {
        assert_eq!(&data[..4], &2.0_f32.to_le_bytes());
        assert!(data[4..].iter().all(|byte| *byte == 0));
    }
}

#[test]
fn bundled_engine_frame_contract_packs_time_and_resized_resolution() {
    let mut files = fresco_example_engine::source_files();
    files.insert("main.fr".into(), "canvas probe(ctx: CanvasContext) -> color { rgba(ctx.uv.x + time(), ctx.uv.y, delta_time(), 1.0) }".into());
    let artifact = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let root: ManifestRoot = serde_json::from_str(&artifact.manifest).unwrap();
    let definitions = root.canvases.into_iter().next().unwrap().global_uniforms;
    assert_eq!(definitions.len(), 1);
    let mut set = UniformSet::new(definitions, LIMITS).unwrap();
    for frame in [
        FrameInputs {
            time: 1.0,
            delta_time: 0.25,
            physical_size: [800, 600],
        },
        FrameInputs {
            time: 2.0,
            delta_time: 0.5,
            physical_size: [1024, 768],
        },
    ] {
        set.update(|uniform, field| frame_uniform(uniform, field, frame))
            .unwrap();
        let upload = set.uploads().unwrap().next().unwrap();
        assert_eq!((upload.group, upload.binding), (3, 0));
        let expected: Vec<_> = [
            frame.time,
            frame.delta_time,
            frame.physical_size[0] as f32,
            frame.physical_size[1] as f32,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
        assert_eq!(upload.bytes, expected);
    }
}
