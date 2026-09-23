use fresco_artifact::{ManifestSampler, ManifestTexture};
use fresco_example_engine::runtime::textures::{
    TextureImage, TextureInputs, TextureLimits, validate_bindings,
};

const LIMITS: TextureLimits = TextureLimits {
    max_dimension: 1024,
    max_bind_groups: 4,
    max_bindings_per_group: 16,
    max_textures_per_stage: 8,
    max_samplers_per_stage: 2,
};

fn def(name: &str, group: u32, binding: u32) -> ManifestTexture {
    ManifestTexture {
        name: name.into(),
        group,
        binding,
        metadata: None,
    }
}

fn image() -> TextureImage {
    TextureImage {
        width: 2,
        height: 1,
        pixels: vec![255, 0, 0, 255, 0, 255, 0, 255].into(),
    }
}

#[test]
fn reflected_sparse_groups_and_binding_numbers_drive_validation() {
    let textures = [def("paint", 2, 7)];
    let inputs = TextureInputs::from([("paint".into(), image())]);
    let sampler = ManifestSampler {
        group: 1,
        binding: 5,
    };
    validate_bindings(&textures, Some(&sampler), &inputs, [(0, 0), (3, 0)], LIMITS).unwrap();
    assert!(validate_bindings(&textures, Some(&sampler), &inputs, [(2, 7)], LIMITS).is_err());
    assert!(validate_bindings(&textures, Some(&sampler), &inputs, [(1, 5)], LIMITS).is_err());
    assert!(validate_bindings(&textures, None, &inputs, [], LIMITS).is_err());
    assert!(
        validate_bindings(&textures, Some(&sampler), &TextureInputs::new(), [], LIMITS).is_err()
    );
    let extra = TextureInputs::from([("paint".into(), image()), ("unknown".into(), image())]);
    assert!(validate_bindings(&textures, Some(&sampler), &extra, [], LIMITS).is_err());
}

#[test]
fn invalid_dimensions_byte_counts_bindings_and_device_limits_fail() {
    for invalid in [
        TextureImage {
            width: 0,
            ..image()
        },
        TextureImage {
            height: 2,
            ..image()
        },
        TextureImage {
            width: 1025,
            ..image()
        },
    ] {
        assert!(invalid.validate("paint", LIMITS.max_dimension).is_err());
    }
    let inputs = TextureInputs::from([("paint".into(), image())]);
    let sampler = ManifestSampler {
        group: 1,
        binding: 0,
    };
    for definitions in [
        vec![def("paint", 4, 1)],
        vec![def("paint", 1, 16)],
        vec![def("paint", 1, 0)],
        vec![def("paint", 1, 1), def("paint", 1, 2)],
    ] {
        assert!(validate_bindings(&definitions, Some(&sampler), &inputs, [], LIMITS).is_err());
    }
    for limits in [
        TextureLimits {
            max_textures_per_stage: 0,
            ..LIMITS
        },
        TextureLimits {
            max_samplers_per_stage: 0,
            ..LIMITS
        },
    ] {
        assert!(
            validate_bindings(&[def("paint", 1, 1)], Some(&sampler), &inputs, [], limits).is_err()
        );
    }
}

#[cfg(feature = "images")]
#[test]
fn embedded_png_and_external_jpeg_decode_in_shared_rust_code() {
    use fresco_artifact::ManifestTextureMetadata;
    use fresco_example_engine::assets::{CHECKER_ID, CHECKER_PNG, decode, resolve};
    let checker = decode("checker", CHECKER_PNG).unwrap();
    assert_eq!((checker.width, checker.height), (8, 8));
    assert_eq!(&checker.pixels[..4], &[235, 120, 40, 255]);
    assert_eq!(&checker.pixels[8..12], &[30, 80, 210, 255]);
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100)
        .encode(
            &[200, 40, 20].repeat(64),
            8,
            8,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    let jpeg = decode("photo", &bytes).unwrap();
    assert_eq!((jpeg.width, jpeg.height), (8, 8));
    assert!(
        jpeg.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0].abs_diff(200) < 4
                && p[1].abs_diff(40) < 4
                && p[2].abs_diff(20) < 4
                && p[3] == 255)
    );
    let mut texture = def("paint", 1, 1);
    texture.metadata = Some(ManifestTextureMetadata {
        default_asset: Some(CHECKER_ID.into()),
        texture_type: None,
        channels: None,
    });
    let defaults = resolve(&[texture.clone()], &Default::default()).unwrap();
    assert_eq!(defaults["paint"].pixels, checker.pixels);
    let overrides = std::collections::BTreeMap::from([("paint".into(), bytes)]);
    assert_eq!(
        resolve(&[texture.clone()], &overrides).unwrap()["paint"].pixels,
        jpeg.pixels
    );
    texture.metadata.as_mut().unwrap().default_asset = Some("missing.png".into());
    assert!(resolve(&[texture], &Default::default()).is_err());
    assert!(decode("bad", b"not an image").is_err());
    assert!(decode("truncated", &CHECKER_PNG[..20]).is_err());
}

#[cfg(feature = "images")]
#[test]
fn png_decode_preserves_channel_order_and_straight_alpha() {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[200, 100, 50, 128], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    let decoded = fresco_example_engine::assets::decode("transparent", &bytes).unwrap();
    assert_eq!(&*decoded.pixels, &[200, 100, 50, 128]);
}
