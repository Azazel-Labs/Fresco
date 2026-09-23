//! Public language metadata facade for editor and wasm integrations.

pub fn cell_members() -> &'static [(&'static str, &'static str, &'static str)] {
    crate::registry::CELL_MEMBERS
}

pub fn docs_model() -> crate::language_docs::LanguageDocsModel {
    crate::language_docs::build_language_docs_model()
}

pub fn docs_model_json_pretty() -> String {
    serde_json::to_string_pretty(&docs_model()).expect("language docs model must serialize")
}

pub fn docs_model_markdown() -> String {
    crate::language_docs::render_language_docs_markdown(&docs_model())
}

pub fn keywords() -> Vec<String> {
    docs_model().keywords
}

pub fn type_keywords() -> Vec<String> {
    docs_model().type_keywords
}

pub fn units() -> Vec<String> {
    docs_model().units
}

pub fn builtins() -> Vec<String> {
    docs_model().builtins
}

pub fn space_transforms() -> Vec<String> {
    docs_model().space_transforms
}

pub fn blend_modes() -> Vec<String> {
    docs_model().blend_modes
}

pub fn enum_members() -> Vec<String> {
    docs_model().enum_members
}

pub fn stdlib_exports() -> Vec<crate::language_docs::DocStdlibExport> {
    docs_model().stdlib_exports
}

/// Geometry available on cell contour values.
pub fn contour_members() -> &'static [(&'static str, &'static str, &'static str)] {
    crate::registry::CONTOUR_MEMBERS
}
