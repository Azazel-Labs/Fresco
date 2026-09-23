use fresco_artifact::{ManifestArrayDefault, ManifestParamDefault};
use serde_json::{Value, json};

#[test]
fn scalar_and_color_defaults_keep_f32_serialization_precision() {
    let scalar = 0.1_f32;
    let emitted = serde_json::to_string(&ManifestParamDefault::Scalar(scalar)).unwrap();
    assert_eq!(emitted, serde_json::to_string(&scalar).unwrap());
    assert_ne!(emitted, serde_json::to_string(&f64::from(scalar)).unwrap());

    let color = [0.1_f32, 0.2, 0.3, 1.0];
    assert_eq!(
        serde_json::to_string(&ManifestParamDefault::Color(color)).unwrap(),
        serde_json::to_string(&color).unwrap()
    );
}

#[test]
fn integer_boolean_and_array_defaults_preserve_the_wire_shape() {
    for (default, expected) in [
        (ManifestParamDefault::Int(i32::MIN), json!(i32::MIN)),
        (ManifestParamDefault::UInt(u32::MAX), json!(u32::MAX)),
        (ManifestParamDefault::Bool(true), json!(true)),
        (
            ManifestParamDefault::Array(ManifestArrayDefault {
                elem_type: "vec3".into(),
                values: vec![json!([1.0, 2.0, 3.0])],
            }),
            json!({"type": "vec3", "values": [[1.0, 2.0, 3.0]]}),
        ),
    ] {
        let encoded = serde_json::to_string(&default).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), expected);
    }

    let empty: ManifestArrayDefault =
        serde_json::from_value(json!({"type": "f32", "values": []})).unwrap();
    assert_eq!(empty.elem_type, "f32");
    assert!(empty.values.is_empty());
}

#[test]
fn shared_parameter_records_preserve_producer_precision_and_reader_types() {
    use fresco_artifact::{ManifestParam, ManifestParamTypeInfo, ManifestSurfaceParam};

    let parameter = ManifestParam {
        name: "gain".into(),
        ty: "f32".into(),
        param_type: Some(ManifestParamTypeInfo {
            name: "f32".into(),
            params: None,
            size: None,
        }),
        default: ManifestParamDefault::Scalar(0.1),
        min: Some(0.1_f32),
        max: Some(0.9_f32),
    };
    let encoded = serde_json::to_string(&parameter).unwrap();
    let reader: ManifestParam = serde_json::from_str(&encoded).unwrap();
    assert_eq!(reader.name, "gain");
    assert_eq!(reader.ty, "f32");
    assert_eq!(reader.default, json!(0.1));
    assert_eq!(reader.min, Some(0.1));
    assert_eq!(reader.max, Some(0.9));
    assert_eq!(reader.param_type.unwrap().name, "f32");
    assert!(
        !encoded.contains("0.100000001"),
        "must retain f32 decimal encoding"
    );

    let surface = ManifestSurfaceParam {
        group: 0,
        name: "mask".into(),
        ty: "u32".into(),
        binding: 7,
        default: ManifestParamDefault::UInt(u32::MAX),
        min: None::<f32>,
        max: None,
    };
    let reader: ManifestSurfaceParam =
        serde_json::from_str(&serde_json::to_string(&surface).unwrap()).unwrap();
    assert_eq!(reader.default.as_u64(), Some(u64::from(u32::MAX)));
    assert_eq!(reader.binding, 7);
    assert_eq!(reader.min, None);
    assert_eq!(reader.max, None);
}

#[cfg(feature = "typescript")]
#[test]
fn parameter_transport_generics_do_not_leak_into_browser_contracts() {
    use fresco_artifact::{
        ManifestCanvas, ManifestParam, ManifestRoot, ManifestSurface, ManifestSurfaceParam,
    };
    use ts_rs::{Config, TS};
    for declaration in [
        <ManifestParam as TS>::decl(&Config::default()),
        <ManifestSurfaceParam as TS>::decl(&Config::default()),
    ] {
        assert!(declaration.contains("default: unknown"));
        assert!(declaration.contains("min: number | null"));
        assert!(declaration.contains("max: number | null"));
        assert!(!declaration.contains("<D"));
        assert!(!declaration.contains("<N"));
    }
    for declaration in [
        <ManifestRoot as TS>::decl(&Config::default()),
        <ManifestCanvas as TS>::decl(&Config::default()),
        <ManifestSurface as TS>::decl(&Config::default()),
    ] {
        assert!(!declaration.contains("<D"));
        assert!(!declaration.contains("<N"));
        assert!(!declaration.contains("<unknown"));
    }
}
