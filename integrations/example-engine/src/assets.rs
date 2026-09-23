//! Explicit sample assets and shared PNG/JPEG decoding. No filesystem or network IO.
use crate::runtime::{
    RuntimeError,
    textures::{TextureImage, TextureInputs, error},
};
use fresco_artifact::ManifestTexture;
use std::collections::BTreeMap;
use std::io::Cursor;

pub const CHECKER_ID: &str = "example://checker";
pub const CHECKER_PNG: &[u8] = include_bytes!("../assets/checker.png");
pub const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 16_384;

pub fn decode(name: &str, bytes: &[u8]) -> Result<TextureImage, RuntimeError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| error(name, e.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| error(name, e.to_string()))?;
    let byte_len = u64::from(image.width()) * u64::from(image.height()) * 4;
    if byte_len > MAX_DECODED_BYTES {
        return Err(error(
            name,
            "decoded RGBA8 image exceeds the sample engine's memory limit",
        ));
    }
    let rgba = image.into_rgba8();
    let result = TextureImage {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw().into(),
    };
    result.validate(name, MAX_DIMENSION)?;
    Ok(result)
}

/// Overrides are keyed by the shader's texture name. All other assets must be
/// explicitly embedded by this profile; hosts resolve external identities first.
pub fn resolve(
    definitions: &[ManifestTexture],
    overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<TextureInputs, RuntimeError> {
    for name in overrides.keys() {
        if !definitions.iter().any(|def| def.name == *name) {
            return Err(error(name, "no such texture in the selected canvas"));
        }
    }
    let mut inputs = TextureInputs::new();
    for def in definitions {
        if inputs.contains_key(&def.name) {
            return Err(error(&def.name, "duplicate texture name"));
        }
        let bytes = if let Some(bytes) = overrides.get(&def.name) {
            bytes.as_slice()
        } else {
            match def
                .metadata
                .as_ref()
                .and_then(|m| m.default_asset.as_deref())
            {
                Some(CHECKER_ID) => CHECKER_PNG,
                Some(asset) => {
                    return Err(error(
                        &def.name,
                        format!("host must supply asset `{asset}`"),
                    ));
                }
                None => {
                    return Err(error(
                        &def.name,
                        "no default asset; host must supply image bytes",
                    ));
                }
            }
        };
        inputs.insert(def.name.clone(), decode(&def.name, bytes)?);
    }
    Ok(inputs)
}
