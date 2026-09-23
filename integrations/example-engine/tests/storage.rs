use fresco_artifact::{ManifestParam, ManifestStorageParam};
use fresco_example_engine::runtime::storage::{StorageLimits, StorageSet};
use serde_json::{Value, json};

fn limits() -> StorageLimits {
    StorageLimits {
        max_bind_groups: 4,
        max_bindings_per_group: 16,
        max_buffers_per_stage: 8,
        max_buffer_bytes: 128,
    }
}

fn declarations(
    name: &str,
    ty: &str,
    value: Value,
    binding: u32,
) -> (ManifestParam, ManifestStorageParam) {
    let ty = format!("array<{ty}>");
    (
        ManifestParam {
            name: name.into(),
            ty: ty.clone(),
            default: json!({"values":value}),
            param_type: None,
            min: None,
            max: None,
        },
        ManifestStorageParam {
            name: name.into(),
            ty,
            param_type: None,
            group: 2,
            binding,
        },
    )
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect()
}

#[test]
fn vec3_arrays_have_padding_between_elements_and_zeroed_empty_backing() {
    let (param, binding) = declarations("points", "vec3", json!([[1, 2, 3], [4, 5, 6]]), 0);
    let mut set = StorageSet::new(&[param], vec![binding], limits(), [(0, 0)]).unwrap();
    let upload = set.uploads().next().unwrap();
    assert_eq!(upload.stride, 16);
    assert_eq!(upload.element_count, 2);
    assert_eq!(floats(upload.bytes), [1., 2., 3., 0., 4., 5., 6., 0.]);
    set.update(json!({"points":[]}).as_object().unwrap())
        .unwrap();
    let upload = set.uploads().next().unwrap();
    assert_eq!(upload.element_count, 0);
    assert_eq!(upload.bytes, [0; 16]);
    assert_eq!(set.values()["points"], json!([]));
}

#[test]
fn scalar_encoding_matches_compiler_transport_without_rounding_integers() {
    let cases = [
        ("i32", json!([-7, 16777216]), vec![-7., 16777216.]),
        ("u32", json!([0, 42]), vec![0., 42.]),
        ("bool", json!([true, false]), vec![1., 0.]),
        (
            "vec2",
            json!([[0.5, 0.25], [0.75, 1]]),
            vec![0.5, 0.25, 0.75, 1.],
        ),
        ("vec4", json!([[1, 2, 3, 4]]), vec![1., 2., 3., 4.]),
    ];
    for (ty, value, expected) in cases {
        let (param, binding) = declarations("data", ty, value, 0);
        let mut set = StorageSet::new(&[param], vec![binding], limits(), []).unwrap();
        let before = set.values().clone();
        assert_eq!(floats(set.uploads().next().unwrap().bytes), expected);
        for invalid in [json!([16777217]), json!([1e100]), json!([null])] {
            assert!(
                set.update(json!({"data":invalid}).as_object().unwrap())
                    .is_err()
            );
            assert_eq!(set.values(), &before);
        }
    }
}

#[test]
fn failed_batches_keep_all_arrays_and_enforce_device_limits() {
    let (a, ab) = declarations("a", "f32", json!([1]), 0);
    let (b, bb) = declarations("b", "vec3", json!([[2, 3, 4]]), 1);
    let mut set = StorageSet::new(&[a, b], vec![ab, bb], limits(), []).unwrap();
    let before: Vec<_> = set.uploads().map(|u| u.bytes.to_vec()).collect();
    for invalid in [
        json!({"a":[9],"b":[[2,3]]}),
        json!({"a":[9],"unknown":[]}),
        json!({"b":vec![vec![0;3];9]}),
    ] {
        assert!(set.update(invalid.as_object().unwrap()).is_err());
        assert_eq!(
            set.uploads().map(|u| u.bytes.to_vec()).collect::<Vec<_>>(),
            before
        );
    }
    set.update(
        json!({"a":[5,6,7],"b":[[8,9,10],[11,12,13]]})
            .as_object()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        set.uploads().map(|u| u.element_count).collect::<Vec<_>>(),
        [3, 2]
    );
}

#[test]
fn invalid_bindings_and_declarations_fail_before_allocation() {
    let (param, binding) = declarations("data", "f32", json!([1]), 0);
    assert!(
        StorageSet::new(
            std::slice::from_ref(&param),
            vec![binding.clone()],
            limits(),
            [(2, 0)]
        )
        .is_err()
    );
    assert!(StorageSet::new(&[], vec![binding.clone()], limits(), []).is_err());
    assert!(
        StorageSet::new(
            std::slice::from_ref(&param),
            vec![binding.clone(), binding.clone()],
            limits(),
            []
        )
        .is_err()
    );
    let mut mismatched = binding.clone();
    mismatched.ty = "array<vec2>".into();
    assert!(StorageSet::new(std::slice::from_ref(&param), vec![mismatched], limits(), []).is_err());
    for invalid_limits in [
        StorageLimits {
            max_buffers_per_stage: 0,
            ..limits()
        },
        StorageLimits {
            max_buffer_bytes: 3,
            ..limits()
        },
        StorageLimits {
            max_bind_groups: 2,
            ..limits()
        },
        StorageLimits {
            max_bindings_per_group: 0,
            ..limits()
        },
    ] {
        assert!(
            StorageSet::new(
                std::slice::from_ref(&param),
                vec![binding.clone()],
                invalid_limits,
                []
            )
            .is_err()
        );
    }
    for ty in ["mat3", "color", "vec3,2", "Other"] {
        let (param, binding) = declarations("data", ty, json!([]), 0);
        assert!(StorageSet::new(&[param], vec![binding], limits(), []).is_err());
    }
}

#[test]
fn actual_engine_artifact_supplies_storage_defaults_and_separate_uniform_parameters() {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        r#"canvas probe(ctx: CanvasContext) -> color {
        param gain: f32 = 0.5
        param points: array<vec3> = [vec3(0.1,0.2,0.3), vec3(0.4,0.5,0.6)]
        rgba(points[1].x * gain, points[1].y, points[1].z, 1.0)
    }"#
        .into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let canvas = &manifest.canvases[0];
    let set = StorageSet::new(
        &canvas.params,
        canvas.storage_params.clone(),
        limits(),
        [(0, 0)],
    )
    .unwrap();
    let upload = set.uploads().next().unwrap();
    assert_eq!(upload.definition.group, 2);
    assert_eq!(floats(upload.bytes), [0.1, 0.2, 0.3, 0., 0.4, 0.5, 0.6, 0.]);
    assert!(!set.values().contains_key("gain"));
    let mut inputs =
        fresco_example_engine::runtime::parameters::CanvasParameters::new(canvas, limits())
            .unwrap();
    let before = inputs.values();
    assert!(
        inputs
            .update(json!({"gain":0.9,"points":[[1,2]]}).as_object().unwrap())
            .is_err()
    );
    assert_eq!(
        inputs.values(),
        before,
        "an invalid array must not partially update instance parameters"
    );
    assert!(
        inputs
            .update(json!({"gain":null,"points":[[1,2,3]]}).as_object().unwrap())
            .is_err()
    );
    assert_eq!(inputs.values(), before);
    inputs
        .update(json!({"gain":0.75,"points":[[1,2,3]]}).as_object().unwrap())
        .unwrap();
    assert_eq!(inputs.values()["gain"], json!(0.75));
    assert_eq!(inputs.values()["points"], json!([[1, 2, 3]]));
}
