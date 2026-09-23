// Re-export procedural macros
pub use fresco_macros::{builtin, enum_decl, type_decl};

pub(crate) mod ast;
pub(crate) mod builtin_catalog;
pub(crate) mod check;
pub(crate) mod context;
pub(crate) mod deriv;
pub(crate) mod diag;
pub mod driver;
pub(crate) mod hir;
pub mod language;
pub mod language_docs;
pub mod lexer;
pub(crate) mod lower;
pub mod material_hir;
pub(crate) mod parser;
pub(crate) mod pipeline_layout;
pub(crate) mod registry;
mod registry_decls;
pub(crate) mod resource_type;
pub(crate) mod rewrite;
pub(crate) mod signal_eval;
pub(crate) mod typed_scalar;

#[cfg(test)]
pub(crate) mod test_support;
