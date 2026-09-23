use std::collections::BTreeMap;

use fresco_artifact::ManifestVertexFactory;
use fresco_example_engine::runtime::vertices::{VertexLimits, VertexValues, pack_vertices};
use serde_json::json;

fn limits() -> VertexLimits {
    VertexLimits {
        max_attributes: 16,
        max_stride: 2048,
        max_buffer_bytes: 65536,
    }
}

fn factory() -> ManifestVertexFactory {
    serde_json::from_value(json!({
        "name":"custom", "vertex_format":"Packed", "array_stride":24,
        "attributes":[
            {"name":"point", "type":"vec3", "shader_location":3, "offset":4,
             "gpu_format":"float32x3", "required":true,"defaulted":false},
            {"name":"id", "type":"u32", "shader_location":0, "offset":20,
             "gpu_format":"uint32", "required":true,"defaulted":false}
        ]
    }))
    .unwrap()
}

#[test]
fn reflected_offsets_locations_and_padding_control_the_upload() {
    let mut definition = factory();
    let streams = BTreeMap::from([
        (
            "point".into(),
            VertexValues::F32(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
        ),
        ("id".into(), VertexValues::U32(&[u32::MAX, 16_777_217])),
    ]);
    let bytes = pack_vertices(&definition, 2, &streams, limits()).unwrap();
    assert_eq!(bytes.len(), 48);
    for (vertex, record) in bytes.as_chunks::<24>().0.iter().enumerate() {
        assert_eq!(&record[..4], &[0; 4]);
        assert_eq!(&record[16..20], &[0; 4]);
        for (lane, bytes) in record[4..16].as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(f32::from_le_bytes(*bytes), (vertex * 3 + lane + 1) as f32);
        }
    }
    assert_eq!(
        u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        u32::MAX
    );
    assert_eq!(
        u32::from_le_bytes(bytes[44..48].try_into().unwrap()),
        16_777_217
    );
    definition.attributes.reverse();
    assert_eq!(
        pack_vertices(&definition, 2, &streams, limits()).unwrap(),
        bytes
    );
}

#[test]
fn every_compiler_vertex_format_preserves_scalar_bits() {
    for (prefix, values, expected) in [
        (
            "float32",
            VertexValues::F32(&[0.1, -2.0, 3.0, 4.0]),
            0.1_f32.to_le_bytes(),
        ),
        (
            "sint32",
            VertexValues::I32(&[i32::MIN, -1, 0, i32::MAX]),
            i32::MIN.to_le_bytes(),
        ),
        (
            "uint32",
            VertexValues::U32(&[u32::MAX, 16_777_217, 0, 1]),
            u32::MAX.to_le_bytes(),
        ),
    ] {
        for components in 1..=4 {
            let mut definition = factory();
            definition.attributes.truncate(1);
            let field = &mut definition.attributes[0];
            field.offset = Some(0);
            field.gpu_format = Some(if components == 1 {
                prefix.into()
            } else {
                format!("{prefix}x{components}")
            });
            definition.array_stride = Some(components * 4);
            let values = match values {
                VertexValues::F32(v) => VertexValues::F32(&v[..components as usize]),
                VertexValues::I32(v) => VertexValues::I32(&v[..components as usize]),
                VertexValues::U32(v) => VertexValues::U32(&v[..components as usize]),
            };
            let bytes = pack_vertices(
                &definition,
                1,
                &BTreeMap::from([("point".into(), values)]),
                limits(),
            )
            .unwrap();
            assert_eq!(&bytes[..4], &expected);
            assert_eq!(bytes.len(), components as usize * 4);
        }
    }
}

#[test]
fn invalid_layouts_streams_and_limits_fail_before_upload() {
    let original = factory();
    let streams = BTreeMap::from([
        ("point".into(), VertexValues::F32(&[1.0, 2.0, 3.0])),
        ("id".into(), VertexValues::U32(&[9])),
    ]);
    let mutations: [fn(&mut ManifestVertexFactory); 9] = [
        |f| f.array_stride = None,
        |f| f.array_stride = Some(23),
        |f| f.attributes[0].offset = Some(2),
        |f| f.attributes[0].offset = Some(u32::MAX - 3),
        |f| f.attributes[1].offset = Some(8),
        |f| f.attributes[1].name = "point".into(),
        |f| f.attributes[1].shader_location = 3,
        |f| f.attributes[1].shader_location = 16,
        |f| f.attributes[0].gpu_format = Some("float16x4".into()),
    ];
    for mutate in mutations {
        let mut definition = original.clone();
        mutate(&mut definition);
        assert!(pack_vertices(&definition, 1, &streams, limits()).is_err());
    }
    for bad in [
        VertexValues::F32(&[1.0, 2.0]),
        VertexValues::U32(&[1, 2, 3]),
        VertexValues::F32(&[1.0, f32::NAN, 3.0]),
    ] {
        let mut invalid = streams.clone();
        invalid.insert("point".into(), bad);
        assert!(pack_vertices(&original, 1, &invalid, limits()).is_err());
    }
    let mut missing = streams.clone();
    missing.remove("id");
    assert!(pack_vertices(&original, 1, &missing, limits()).is_err());
    let mut unknown = streams.clone();
    unknown.insert("typo".into(), VertexValues::U32(&[0]));
    assert!(pack_vertices(&original, 1, &unknown, limits()).is_err());
    for restricted in [
        VertexLimits {
            max_buffer_bytes: 23,
            ..limits()
        },
        VertexLimits {
            max_stride: 20,
            ..limits()
        },
        VertexLimits {
            max_attributes: 1,
            ..limits()
        },
    ] {
        assert!(pack_vertices(&original, 1, &streams, restricted).is_err());
    }
}

#[test]
fn shipped_mesh_factory_packs_without_a_host_struct_layout() {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let bundle = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&bundle.manifest).unwrap();
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| f.name == "preview_static")
        .unwrap();
    let streams = BTreeMap::from([
        ("position".into(), VertexValues::F32(&[1.0, 2.0, 3.0])),
        ("normal".into(), VertexValues::F32(&[4.0, 5.0, 6.0])),
        ("tangent".into(), VertexValues::F32(&[7.0, 8.0, 9.0])),
        ("uv".into(), VertexValues::F32(&[10.0, 11.0])),
        ("uv2".into(), VertexValues::F32(&[12.0, 13.0])),
    ]);
    let bytes = pack_vertices(factory, 1, &streams, limits()).unwrap();
    assert_eq!(bytes.len(), 52);
    let decoded: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(decoded, (1..=13).map(|i| i as f32).collect::<Vec<_>>());
}
