use fresco_example_engine::runtime::style_parameters::StyleParameters;
use serde_json::json;

fn manifest() -> fresco_artifact::ManifestRoot {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        include_str!("fixtures/style-settings.fr").into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    serde_json::from_str(&output.manifest).unwrap()
}

#[test]
fn independent_records_validate_atomic_updates_and_buffer_limits() {
    let mut manifest = manifest();
    let mut first = StyleParameters::new(&manifest.surfaces[0]).unwrap();
    let second = StyleParameters::new(&manifest.surfaces[1]).unwrap();
    let initial = StyleParameters::initial_bytes(&manifest, 1024).unwrap();
    assert_eq!(initial.len(), 6 * 16);
    assert_eq!(f32::from_le_bytes(initial[..4].try_into().unwrap()), 0.25);
    assert_eq!(
        f32::from_le_bytes(initial[48..52].try_into().unwrap()),
        0.75
    );
    let before = first.values();
    let before_bytes: Vec<_> = first.uploads().map(|(o, b)| (o, b.to_vec())).collect();
    for update in [
        json!({"style.gain":0.5,"style.tint":[1,0]}),
        json!({"style.gain":2}),
        json!({"style.missing":0}),
    ] {
        assert!(first.update(update.as_object().unwrap()).is_err());
        assert_eq!(first.values(), before);
        assert_eq!(
            first
                .uploads()
                .map(|(o, b)| (o, b.to_vec()))
                .collect::<Vec<_>>(),
            before_bytes
        );
    }
    first
        .update(
            json!({"style.gain":0.5,"style.tint":[0,1,0,1]})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(first.values()["style.gain"], 0.5);
    assert_eq!(second.values()["style.gain"], 0.75);
    assert!(StyleParameters::initial_bytes(&manifest, 16).is_err());
    manifest.surfaces[1]
        .settings
        .as_mut()
        .unwrap()
        .implementations[0]
        .settings_offset = 0;
    assert!(StyleParameters::initial_bytes(&manifest, 1024).is_err());
    manifest.surfaces[1]
        .settings
        .as_mut()
        .unwrap()
        .implementations[0]
        .settings_offset = u32::MAX;
    assert!(StyleParameters::initial_bytes(&manifest, 1024).is_err());
}

#[test]
fn exact_style_scalars_preserve_integer_bits_and_reject_invalid_batches() {
    let mut manifest = manifest();
    let surface = &mut manifest.surfaces[0];
    let selection = &mut surface.settings.as_mut().unwrap().implementations[0];
    selection.parameters = [
        ("seed", "u32", json!(u32::MAX)),
        ("offset", "i32", json!(i32::MIN)),
        ("enabled", "bool", json!(true)),
    ]
    .into_iter()
    .map(|(name, ty, default)| fresco_artifact::ManifestParam {
        name: name.into(),
        ty: ty.into(),
        param_type: None,
        default,
        min: None,
        max: None,
    })
    .collect();
    let mut state = StyleParameters::new(surface).unwrap();
    let decode = |bytes: &[u8]| {
        let lo = f32::from_le_bytes(bytes[..4].try_into().unwrap()) as u32;
        let hi = f32::from_le_bytes(bytes[4..8].try_into().unwrap()) as u32;
        lo | (hi << 16)
    };
    let packed: Vec<_> = state.uploads().map(|(_, bytes)| decode(bytes)).collect();
    assert_eq!(packed, [u32::MAX, 0x80000000, 1]);
    let before = state.values();
    for update in [
        json!({"style.seed":4294967296u64}),
        json!({"style.seed":1.5}),
        json!({"style.offset":2147483648i64}),
        json!({"style.seed":123,"style.enabled":1}),
    ] {
        assert!(state.update(update.as_object().unwrap()).is_err());
        assert_eq!(state.values(), before);
    }
    state
        .update(
            json!({"style.seed":16777217,"style.offset":-16777217,"style.enabled":false})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    let packed: Vec<_> = state.uploads().map(|(_, bytes)| decode(bytes)).collect();
    assert_eq!(
        packed,
        [
            16777217,
            u32::from_le_bytes((-16777217i32).to_le_bytes()),
            0
        ]
    );
}
