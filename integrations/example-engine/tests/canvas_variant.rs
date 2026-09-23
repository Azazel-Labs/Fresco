use fresco_artifact::{
    ManifestEnginePass, ManifestEnginePassVariant, ManifestEnginePassVariantBinding,
};
use fresco_example_engine::runtime::canvas_variant::{VariantSelection, select};

fn pass() -> ManifestEnginePass {
    ManifestEnginePass {
        pipeline: "profile".into(),
        pass: "draw".into(),
        interface: "Canvas".into(),
        vertex_entry: "base_v".into(),
        fragment_entry: "base_f".into(),
        vertex_count: 3,
        instance_uniform_group: 0,
        instance_uniform_binding: 0,
        variants: ["low", "high"]
            .into_iter()
            .map(|value| ManifestEnginePassVariant {
                key: value.into(),
                bindings: vec![ManifestEnginePassVariantBinding {
                    axis: "quality".into(),
                    value: value.into(),
                }],
                vertex_entry: format!("{value}_v"),
                fragment_entry: format!("{value}_f"),
            })
            .collect(),
    }
}

#[test]
fn explicit_selection_resolves_both_specialized_stages() {
    let pass = pass();
    for value in ["low", "high"] {
        let selection = VariantSelection::from([("quality".into(), value.into())]);
        let stages = select("art", &pass, Some(&selection)).unwrap();
        assert_eq!(stages.vertex, format!("{value}_v"));
        assert_eq!(stages.fragment, format!("{value}_f"));
    }
}

#[test]
fn incomplete_unknown_and_ambiguous_selections_fail() {
    let mut pass = pass();
    assert!(select("art", &pass, None).is_err());
    for selection in [
        VariantSelection::new(),
        VariantSelection::from([("quality".into(), "other".into())]),
        VariantSelection::from([
            ("quality".into(), "low".into()),
            ("extra".into(), "on".into()),
        ]),
    ] {
        assert!(select("art", &pass, Some(&selection)).is_err());
    }
    let low = VariantSelection::from([("quality".into(), "low".into())]);
    pass.variants.push(pass.variants[0].clone());
    assert!(
        select("art", &pass, Some(&low))
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
}

#[test]
fn malformed_bindings_and_missing_stages_fail() {
    let mut pass = pass();
    let low = VariantSelection::from([("quality".into(), "low".into())]);
    let duplicate = pass.variants[0].bindings[0].clone();
    pass.variants[0].bindings.push(duplicate);
    assert!(select("art", &pass, Some(&low)).is_err());
    pass.variants[0].bindings.pop();
    pass.variants[0].fragment_entry.clear();
    assert!(select("art", &pass, Some(&low)).is_err());
}

#[test]
fn non_permuted_pass_uses_authored_base_stages() {
    let mut pass = pass();
    pass.variants.clear();
    let stages = select("art", &pass, None).unwrap();
    assert_eq!(stages.vertex, "base_v");
    assert_eq!(stages.fragment, "base_f");
}

#[test]
fn all_axes_are_required_and_duplicate_default_variants_fail() {
    let mut pass = pass();
    for variant in &mut pass.variants {
        variant.bindings.push(ManifestEnginePassVariantBinding {
            axis: "mode".into(),
            value: "on".into(),
        });
    }
    let mut selection = VariantSelection::from([("quality".into(), "low".into())]);
    assert!(select("art", &pass, Some(&selection)).is_err());
    selection.insert("mode".into(), "on".into());
    assert_eq!(
        select("art", &pass, Some(&selection)).unwrap().fragment,
        "low_f"
    );
    for variant in &mut pass.variants {
        variant.bindings.clear();
    }
    assert!(
        select("art", &pass, None)
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
}

#[test]
fn sole_specialized_variant_is_selected_without_host_configuration() {
    let mut pass = pass();
    pass.variants.truncate(1);
    let stages = select("art", &pass, None).unwrap();
    assert_eq!(stages.vertex, "low_v");
    assert_eq!(stages.fragment, "low_f");
    let wrong = VariantSelection::from([("quality".into(), "high".into())]);
    assert!(select("art", &pass, Some(&wrong)).is_err());
    assert!(select("art", &pass, Some(&VariantSelection::new())).is_err());
    let duplicate = pass.variants[0].bindings[0].clone();
    pass.variants[0].bindings.push(duplicate);
    assert!(select("art", &pass, None).is_err());
}
