//! The example engine's authored contracts, available without a GPU or compiler dependency.
use std::collections::HashMap;

#[cfg(feature = "images")]
pub mod assets;
pub mod profile;
pub mod runtime;

pub const PROFILE_ID: &str = "fresco-example";
pub const ENTRYPOINT: &str = "engine/engine.fr";
pub const PRELUDE: &str = include_str!("../engine/core/00_prelude.fr");

#[derive(Debug, Clone, Copy)]
pub struct EngineSource {
    pub path: &'static str,
    pub source: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/engine_sources.rs"));

/// A complete embedded profile using stable virtual paths.
/// Callers add user sources separately; an override should replace the whole profile.
pub fn source_files() -> HashMap<String, String> {
    SOURCES
        .iter()
        .map(|file| (file.path.into(), file.source.into()))
        .collect()
}

/// Select a declared renderer through compile configuration, preserving engine source.
pub fn source_files_for_renderer(forward_plus: bool) -> HashMap<String, String> {
    source_files_for_recipe(if forward_plus {
        "forward-plus"
    } else {
        "forward"
    })
}

pub fn source_files_for_recipe(renderer: &str) -> HashMap<String, String> {
    let mut files = source_files();
    files.insert(
        "fresco.config.json".into(),
        serde_json::json!({"renderer":renderer}).to_string(),
    );
    files
}

pub fn source_files_for_deferred() -> HashMap<String, String> {
    source_files_for_recipe("deferred")
}

/// Scene content is opt-in; compiler clients receive only the engine contracts.
pub fn preview_source_files() -> HashMap<String, String> {
    let mut files = source_files();
    files
        .get_mut(ENTRYPOINT)
        .expect("engine root")
        .push_str("\nimport \"scenes/preview.fr\"\n");
    files
}
