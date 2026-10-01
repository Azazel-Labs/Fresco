//! Public language metadata facade for editor and wasm integrations.

pub fn cell_members() -> &'static [(&'static str, &'static str, &'static str)] {
    crate::registry::CELL_MEMBERS
}

pub fn docs_model() -> crate::language_docs::LanguageDocsModel {
    crate::language_docs::build_language_docs_model()
}

/// Geometry available on cell contour values.
pub fn contour_members() -> &'static [(&'static str, &'static str, &'static str)] {
    crate::registry::CONTOUR_MEMBERS
}
