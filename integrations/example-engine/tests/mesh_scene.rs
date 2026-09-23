use fresco_example_engine::profile::{
    FrameInputs,
    mesh::{DisplacementInputs, MeshSceneInputs, SCENE_BYTES},
};

fn inputs() -> MeshSceneInputs {
    MeshSceneInputs {
        model: std::array::from_fn(|i| i as f32),
        view: std::array::from_fn(|i| (i + 16) as f32),
        projection: std::array::from_fn(|i| (i + 32) as f32),
        camera_position: [48.0, 49.0, 50.0],
        frame: FrameInputs {
            time: 51.0,
            delta_time: 0.1,
            physical_size: [640, 480],
        },
        displacement: DisplacementInputs {
            enabled: true,
            amplitude: 0.25,
            frequency: 3.0,
            speed: 2.0,
        },
    }
}

#[test]
fn scene_bytes_preserve_column_major_matrices_and_authored_field_offsets() {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let bundle = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&bundle.wgsl).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&bundle.manifest).unwrap();
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| f.name == "preview_static")
        .unwrap();
    let binding = factory.bindings.iter().find(|b| b.name == "scene").unwrap();
    assert_eq!(binding.signature.as_deref(), Some("uniform<PreviewScene>"));
    let scene = module
        .global_variables
        .iter()
        .find_map(|(_, global)| {
            if global.space == naga::AddressSpace::Uniform
                && global.binding.as_ref().is_some_and(|b| {
                    Some(b.group) == binding.group_index && Some(b.binding) == binding.binding
                })
            {
                Some(&module.types[global.ty].inner)
            } else {
                None
            }
        })
        .expect("reflected scene binding must reach emitted WGSL");
    let naga::TypeInner::Struct { members, span } = scene else {
        panic!("scene must be a struct");
    };
    assert_eq!(*span as usize, SCENE_BYTES);
    assert_eq!(
        members.iter().map(|m| m.offset).collect::<Vec<_>>(),
        [0, 64, 128, 192, 204, 208, 216, 224]
    );
    for member in &members[..3] {
        assert!(matches!(
            module.types[member.ty].inner,
            naga::TypeInner::Matrix {
                columns: naga::VectorSize::Quad,
                rows: naga::VectorSize::Quad,
                scalar: naga::Scalar::F32
            }
        ));
    }
    let bytes = inputs().pack().unwrap();
    let lanes: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(&lanes[..51], &(0..51).map(|i| i as f32).collect::<Vec<_>>());
    assert_eq!(
        &lanes[51..],
        &[51.0, 640.0, 480.0, 0.0, 0.0, 1.0, 0.25, 3.0, 2.0]
    );
}

#[test]
fn invalid_scene_inputs_cannot_produce_uploads() {
    let valid = inputs();
    for invalid in [
        MeshSceneInputs {
            model: [f32::NAN; 16],
            ..valid
        },
        MeshSceneInputs {
            view: [f32::INFINITY; 16],
            ..valid
        },
        MeshSceneInputs {
            projection: [f32::NEG_INFINITY; 16],
            ..valid
        },
        MeshSceneInputs {
            camera_position: [f32::NAN; 3],
            ..valid
        },
        MeshSceneInputs {
            frame: FrameInputs {
                time: f32::NAN,
                ..valid.frame
            },
            ..valid
        },
        MeshSceneInputs {
            displacement: DisplacementInputs {
                speed: f32::INFINITY,
                ..valid.displacement
            },
            ..valid
        },
    ] {
        assert!(invalid.pack().is_err());
    }
    let disabled = MeshSceneInputs {
        displacement: DisplacementInputs {
            enabled: false,
            ..valid.displacement
        },
        ..valid
    }
    .pack()
    .unwrap();
    assert_eq!(
        f32::from_le_bytes(disabled[224..228].try_into().unwrap()),
        0.0
    );
}
