use fresco_artifact::{ManifestRoot, ManifestSurface, ManifestSurfaceUvChannelRequirement};
use fresco_example_engine::profile::preview::PreviewShape;

fn surface() -> ManifestSurface {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: #fff) } }".into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest).unwrap();
    manifest.surfaces.into_iter().next().unwrap()
}

#[test]
fn shipped_surface_requirements_fit_every_preview_shape() {
    let surface = surface();
    for shape in [PreviewShape::Sphere, PreviewShape::Plane, PreviewShape::Box] {
        assert!(shape.geometry_for_surface(&surface).is_ok());
    }
}

#[test]
fn unavailable_required_uv_stream_reports_selector_and_contract() {
    let mut surface = surface();
    surface
        .surface_requirements
        .uv_channels
        .push(ManifestSurfaceUvChannelRequirement {
            selector: "uv3".into(),
            stream_index: 2,
            semantic: "TEXCOORD2".into(),
            components: 2,
            required: true,
            status: "required".into(),
        });
    for shape in [PreviewShape::Sphere, PreviewShape::Plane, PreviewShape::Box] {
        let error = shape
            .geometry_for_surface(&surface)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("missing required UV streams"));
        assert!(error.contains("uv3"));
        assert!(error.contains("surface_requirements.uv_channels"));
    }
    surface
        .surface_requirements
        .uv_channels
        .last_mut()
        .unwrap()
        .required = false;
    assert!(PreviewShape::Sphere.geometry_for_surface(&surface).is_ok());
}

#[test]
fn required_uv_width_is_checked_without_assuming_selector_names() {
    let mut surface = surface();
    surface.surface_requirements.uv_channels = vec![ManifestSurfaceUvChannelRequirement {
        selector: "authored_coordinates".into(),
        stream_index: 1,
        semantic: "TEXCOORD1".into(),
        components: 2,
        required: true,
        status: "required".into(),
    }];
    assert!(PreviewShape::Plane.geometry_for_surface(&surface).is_ok());
    surface.surface_requirements.uv_channels[0].components = 3;
    assert!(PreviewShape::Plane.geometry_for_surface(&surface).is_err());
}
