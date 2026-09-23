use fresco_artifact::ManifestSurfaceParam;
use fresco_example_engine::runtime::{
    surface_parameters::SurfaceParameters, uniforms::UniformLimits,
};
use serde_json::{Value, json};

fn limits() -> UniformLimits {
    UniformLimits {
        max_bind_groups: 4,
        max_bindings_per_bind_group: 1000,
        max_uniform_buffer_binding_size: 65536,
    }
}

fn definitions() -> Vec<ManifestSurfaceParam> {
    serde_json::from_value(json!([
        {"name":"gain", "type":"f32", "group":0, "binding":7, "default":0.5,"min":0,"max":1},
        {"name":"enabled", "type":"bool", "group":0, "binding":2, "default":true},
        {"name":"steps", "type":"i32", "group":0, "binding":9, "default":3},
        {"name":"tint", "type":"color", "group":0, "binding":4, "default":[1,0,0,1]}
    ]))
    .unwrap()
}

fn bytes(parameters: &SurfaceParameters) -> Vec<(u32, Vec<u8>)> {
    parameters
        .uploads()
        .map(|upload| (upload.binding, upload.bytes.to_vec()))
        .collect()
}

#[test]
fn defaults_and_atomic_edits_use_separate_reflected_bindings() {
    let mut parameters = SurfaceParameters::new(&definitions(), limits(), []).unwrap();
    let original = bytes(&parameters);
    assert_eq!(
        original
            .iter()
            .map(|(binding, _)| *binding)
            .collect::<Vec<_>>(),
        [7, 2, 9, 4]
    );
    for ((_, bytes), expected) in original.iter().zip([0.5, 1.0, 3.0, 1.0]) {
        assert_eq!(bytes.len(), 16);
        assert_eq!(f32::from_le_bytes(bytes[..4].try_into().unwrap()), expected);
    }
    assert_eq!(&original[0].1[4..], &[0; 12]);
    parameters
        .update(
            json!({"gain":0.25,"enabled":false,"steps":4,"tint":[0,1,0,1]})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    let committed = bytes(&parameters);
    let values = parameters.values();
    for invalid in [
        json!({"gain":0,"tint":[1,2,3]}),
        json!({"gain":1.1}),
        json!({"enabled":0}),
        json!({"steps":16_777_217}),
        json!({"steps":1.5}),
        json!({"missing":1}),
    ] {
        assert!(parameters.update(invalid.as_object().unwrap()).is_err());
        assert_eq!(bytes(&parameters), committed);
        assert_eq!(parameters.values(), values);
    }
    parameters
        .update(json!({"gain":1}).as_object().unwrap())
        .unwrap();
    assert_eq!(parameters.values()["enabled"], false);
    assert_eq!(parameters.values()["steps"], 4);
}

#[test]
fn malformed_declarations_limits_and_binding_collisions_fail() {
    let mutations: [fn(&mut Vec<ManifestSurfaceParam>); 6] = [
        |d| d[1].name = d[0].name.clone(),
        |d| d[1].binding = d[0].binding,
        |d| d[0].min = Some(2.0),
        |d| d[0].max = Some(f64::NAN),
        |d| d[0].default = Value::Null,
        |d| d[0].ty = "vec3".into(),
    ];
    for mutate in mutations {
        let mut defs = definitions();
        mutate(&mut defs);
        assert!(SurfaceParameters::new(&defs, limits(), []).is_err());
    }
    assert!(SurfaceParameters::new(&definitions(), limits(), [(0, 7)]).is_err());
    assert!(SurfaceParameters::new(&definitions(), limits(), [(2, 7)]).is_ok());
    for limits in [
        UniformLimits {
            max_bind_groups: 0,
            ..limits()
        },
        UniformLimits {
            max_bindings_per_bind_group: 7,
            ..limits()
        },
        UniformLimits {
            max_uniform_buffer_binding_size: 15,
            ..limits()
        },
    ] {
        assert!(SurfaceParameters::new(&definitions(), limits, []).is_err());
    }
}

#[test]
fn independent_uniforms_do_not_inherit_canvas_component_capacity() {
    let template = definitions().remove(0);
    let definitions: Vec<_> = (0..100)
        .map(|index| ManifestSurfaceParam {
            group: 0,
            name: format!("p{index}"),
            binding: index,
            ..template.clone()
        })
        .collect();
    let parameters = SurfaceParameters::new(&definitions, limits(), []).unwrap();
    assert_eq!(parameters.uploads().count(), 100);
}

#[test]
fn real_material_uniform_types_match_the_host_transport() {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        include_str!("../examples/material_parameters.fr").into(),
    );
    let bundle = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&bundle.manifest).unwrap();
    let surface = manifest
        .surfaces
        .iter()
        .find(|surface| surface.name == "adjustable")
        .unwrap();
    let packed = SurfaceParameters::new(&surface.params, limits(), []).unwrap();
    assert_eq!(packed.values().len(), 4);
    let module = naga::front::wgsl::parse_str(&bundle.wgsl).unwrap();
    for param in &surface.params {
        let global = module
            .global_variables
            .iter()
            .find_map(|(_, global)| {
                global
                    .binding
                    .as_ref()
                    .filter(|b| b.group == 0 && b.binding == param.binding)
                    .map(|_| global)
            })
            .unwrap();
        assert_eq!(global.space, naga::AddressSpace::Uniform);
        let expected = if param.ty == "color" {
            naga::TypeInner::Vector {
                size: naga::VectorSize::Quad,
                scalar: naga::Scalar::F32,
            }
        } else {
            naga::TypeInner::Scalar(naga::Scalar::F32)
        };
        assert_eq!(module.types[global.ty].inner, expected);
    }
}
