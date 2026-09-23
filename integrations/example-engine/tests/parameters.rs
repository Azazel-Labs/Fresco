use fresco_artifact::ManifestParam;
use fresco_example_engine::runtime::parameters::{
    INSTANCE_BYTES, PARAMETER_BYTES, PARAMETER_OFFSET, ParameterSet,
};
use serde_json::{Map, Value, json};

fn def(name: &str, ty: &str, default: Value) -> ManifestParam {
    ManifestParam {
        name: name.into(),
        ty: ty.into(),
        default,
        param_type: None,
        min: None,
        max: None,
    }
}

fn values(json: Value) -> Map<String, Value> {
    json.as_object().unwrap().clone()
}

fn floats(set: &ParameterSet) -> Vec<f32> {
    set.bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect()
}

#[test]
fn mixed_defaults_pack_without_vector_padding_and_keep_unused_bytes_zero() {
    let set = ParameterSet::new(vec![
        def("gain", "f32", json!(0.25)),
        def("enabled", "bool", json!(true)),
        def("tint", "color", json!([0.1, 0.2, 0.3, 1.0])),
        def(
            "points",
            "array<vec3, 2>",
            json!({"elem_type":"vec3","values":[[1,2,3],[4,5,6]]}),
        ),
        def(
            "matrix",
            "array<mat2, 1>",
            json!({"elem_type":"mat2","values":[[[1,0],[0,1]]]}),
        ),
        def(
            "ids",
            "array<i32, 2>",
            json!({"elem_type":"i32","values":[-7,8]}),
        ),
    ])
    .unwrap();
    assert_eq!(
        &floats(&set)[..18],
        &[
            0.25, 1.0, 0.1, 0.2, 0.3, 1.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 1.0, 0.0, 0.0, 1.0, -7.0,
            8.0
        ]
    );
    assert!(set.bytes()[18 * 4..].iter().all(|byte| *byte == 0));
    assert_eq!(set.values()["points"], json!([[1, 2, 3], [4, 5, 6]]));
}

#[test]
fn failed_batch_preserves_bytes_and_values_then_recovers() {
    let mut gain = def("gain", "f32", json!(0.5));
    gain.min = Some(0.0);
    gain.max = Some(1.0);
    let mut set = ParameterSet::new(vec![gain, def("tint", "color", json!([1, 0, 0, 1]))]).unwrap();
    let before = *set.bytes();
    let previous = set.values().clone();
    for invalid in [
        json!({"gain":0.75,"tint":[1,2]}),
        json!({"gain":2}),
        json!({"missing":1}),
        json!({"gain":"bad"}),
    ] {
        assert!(set.update(&values(invalid)).is_err());
        assert_eq!(*set.bytes(), before);
        assert_eq!(set.values(), &previous);
    }
    set.update(&values(json!({"gain":0.75,"tint":[0,1,0,1]})))
        .unwrap();
    assert_eq!(&floats(&set)[..5], &[0.75, 0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn integer_transport_and_boolean_types_fail_without_silent_conversion() {
    let mut set = ParameterSet::new(vec![
        def("id", "u32", json!(0)),
        def("enabled", "bool", json!(false)),
    ])
    .unwrap();
    for invalid in [
        json!({"id":16777217}),
        json!({"id":4294967295u64}),
        json!({"id":-1}),
        json!({"id":0.5}),
        json!({"enabled":1}),
    ] {
        assert!(set.update(&values(invalid)).is_err());
        assert_eq!(set.bytes(), &[0; PARAMETER_BYTES]);
    }
    set.update(&values(json!({"id":16777218,"enabled":true})))
        .unwrap();
    assert_eq!(&floats(&set)[..2], &[16777218.0, 1.0]);
}

#[test]
fn malformed_and_oversized_declarations_fail_before_allocation() {
    for ty in [
        "array<f32>",
        "array<f32, 0>",
        "array<mat4, 5>",
        "array<f32, 18446744073709551615>",
        "array<unknown, 1>",
        "texture",
    ] {
        assert!(
            ParameterSet::new(vec![def("p", ty, json!([]))]).is_err(),
            "{ty}"
        );
    }
    assert!(ParameterSet::new(vec![def("p", "f32", json!(0)), def("p", "f32", json!(0))]).is_err());
    let mut defs = vec![def("p", "array<f32, 64>", json!({"values":vec![0.0;64]}))];
    assert!(ParameterSet::new(defs.clone()).is_ok());
    defs.push(def("overflow", "f32", json!(0)));
    assert!(ParameterSet::new(defs).is_err());
}

#[test]
fn real_compiler_defaults_use_the_same_host_parameter_contract() {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        r#"canvas probe(ctx: CanvasContext) -> color {
        param gain: f32 = 0.5 in 0 .. 1
        param flags: array<bool, 2> = [true, false]
        param palette: array<color, 2> = [#ff0000, #00ff00]
        param transforms: array<mat2, 1> = [mat2(1, 0, 0, 1)]
        rgba(gain, 0.0, 0.0, 1.0)
    }"#
        .into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let module = naga::front::wgsl::parse_str(&output.wgsl).unwrap();
    let (_, instance) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("FrescoFullscreenUniforms"))
        .unwrap();
    let naga::TypeInner::Struct { members, span } = &instance.inner else {
        panic!("fullscreen instance must be a struct")
    };
    assert_eq!(*span as usize, INSTANCE_BYTES);
    let params = members
        .iter()
        .find(|member| member.name.as_deref() == Some("params"))
        .unwrap();
    assert_eq!(params.offset as usize, PARAMETER_OFFSET);
    let naga::TypeInner::Array {
        base,
        size: naga::ArraySize::Constant(count),
        stride,
    } = module.types[params.ty].inner
    else {
        panic!("parameter storage must be a fixed array")
    };
    assert_eq!(count.get() as usize * stride as usize, PARAMETER_BYTES);
    assert!(matches!(
        module.types[base].inner,
        naga::TypeInner::Vector {
            size: naga::VectorSize::Quad,
            scalar: naga::Scalar {
                kind: naga::ScalarKind::Float,
                width: 4
            }
        }
    ));
    let root: fresco_artifact::ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    let set = ParameterSet::new(root.canvases[0].params.clone()).unwrap();
    assert_eq!(
        &floats(&set)[..15],
        &[
            0.5, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0
        ]
    );
}

#[test]
fn floating_ranges_compare_at_shader_precision() {
    let mut definition = def("gain", "f32", json!(0.05));
    definition.min = Some(f64::from(0.05_f32));
    definition.max = Some(f64::from(0.55_f32));
    let mut set = ParameterSet::new(vec![definition]).unwrap();
    set.update(&values(json!({"gain":0.55}))).unwrap();
    assert!(set.update(&values(json!({"gain":0.56}))).is_err());
    assert_eq!(set.values()["gain"], 0.55);
}
