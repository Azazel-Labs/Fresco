//! Platform hosts for the example engine. Rendering lives in `fresco-example-engine`.

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
pub mod browser;

pub const DEMO: &str = include_str!("../demo.fr");

pub struct Artifact {
    pub wgsl: String,
    pub manifest: fresco_artifact::ManifestRoot,
}

mod scene;
