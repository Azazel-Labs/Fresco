//! Declarative builtin implementations using the builtin! macro.
//!
//! The builtin! macro is now a procedural macro defined in fresco-macros.
//! See crates/fresco-macros/src/lib.rs for the implementation.

pub mod color;
pub mod filtering;
pub mod gradient;
pub mod image;
pub mod layer_ops;
pub mod math;
pub mod paint;
pub mod path_ops;
pub mod patterns;
pub mod scalar_ops;
pub mod shape_ops;
pub mod shape_to_layer;
pub mod shapes;
pub mod signals;
pub mod svg;
pub mod tex_at;
pub mod vec2;
