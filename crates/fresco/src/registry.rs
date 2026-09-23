#![allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]

use crate::builtin_catalog::BuiltinCaps;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltinId(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
    /// An expression checked by the builtin implementation, not a color.
    Expr,
    Path,
    Contour,
    Shape,
    Layer,
    Scalar,
    I32,
    U32,
    F64,
    Half,
    Vec2,
    Vec3,
    Vec4,
    Mat2,
    Mat3,
    Mat4,
    Color,
    ColorField,
    Coverage,
    Mask,
    Gradient,
    Bool,
    Coord,
    CoordLike,
    Signal,
    Delta,
    Resolution,
    Angle,
    Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeRef {
    Primitive(PrimitiveType),
    Named(TypeId),
    Enum(EnumId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeCtor {
    Circle,
    Capsule,
    Box,
}

#[derive(Clone, Copy)]
pub enum BuiltinLowering {
    ShapeCtor {
        kind: ShapeCtor,
        lower: BuiltinLowerFn,
    },
    /// Direct implementation function that handles checking and HIR construction.
    Impl(BuiltinImplFn),
    /// Receiver-based implementation function (Shape, Layer, etc.)
    ImplReceiver(BuiltinImplReceiverFn),
    /// Discriminated signature - variant name passed as parameter
    ImplDiscriminated {
        discriminator: &'static str,
        default: Option<&'static str>,
        impl_fn: BuiltinImplDiscriminatedFn,
    },
}

impl std::fmt::Debug for BuiltinLowering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ShapeCtor { kind, .. } => {
                f.debug_struct("ShapeCtor").field("kind", kind).finish()
            }
            Self::Impl(_) => f.debug_tuple("Impl").field(&"<fn>").finish(),
            Self::ImplReceiver(_) => f.debug_tuple("ImplReceiver").field(&"<fn>").finish(),
            Self::ImplDiscriminated { discriminator, .. } => f
                .debug_struct("ImplDiscriminated")
                .field("discriminator", discriminator)
                .finish(),
        }
    }
}

/// Function pointer for builtin implementation.
pub type BuiltinImplFn =
    fn(&mut crate::check::Checker, &mut crate::check::ArgBag<'_>) -> Option<crate::check::Value>;

/// Function pointer for discriminated builtin implementation (variant name provided).
pub type BuiltinImplDiscriminatedFn = fn(
    &mut crate::check::Checker,
    &str,
    &mut crate::check::ArgBag<'_>,
) -> Option<crate::check::Value>;

/// Receiver-based check functions - type varies by receiver
pub type BuiltinImplReceiverShapeFn = fn(
    &mut crate::check::Checker,
    crate::hir::ShapeId,
    &mut crate::check::ArgBag<'_>,
) -> Option<crate::check::Value>;
pub type BuiltinImplReceiverLayerFn = fn(
    &mut crate::check::Checker,
    crate::hir::LayerId,
    &mut crate::check::ArgBag<'_>,
) -> Option<crate::check::Value>;
pub type BuiltinImplReceiverPathFn =
    fn(&mut crate::check::Checker, &mut crate::check::ArgBag<'_>) -> Option<crate::check::Value>;

/// Generic wrapper for receiver-based builtins
#[derive(Clone, Copy)]
pub struct BuiltinImplReceiverFn {
    shape: Option<BuiltinImplReceiverShapeFn>,
    layer: Option<BuiltinImplReceiverLayerFn>,
    path: Option<BuiltinImplReceiverPathFn>,
}

impl BuiltinImplReceiverFn {
    pub const fn shape(f: BuiltinImplReceiverShapeFn) -> Self {
        Self {
            shape: Some(f),
            layer: None,
            path: None,
        }
    }

    pub const fn layer(f: BuiltinImplReceiverLayerFn) -> Self {
        Self {
            shape: None,
            layer: Some(f),
            path: None,
        }
    }

    pub const fn path(f: BuiltinImplReceiverPathFn) -> Self {
        Self {
            shape: None,
            layer: None,
            path: Some(f),
        }
    }

    pub fn call_with_shape(
        &self,
        ctx: &mut crate::check::Checker,
        recv: crate::hir::ShapeId,
        bag: &mut crate::check::ArgBag<'_>,
    ) -> Option<crate::check::Value> {
        (self.shape?)(ctx, recv, bag)
    }

    pub fn call_with_layer(
        &self,
        ctx: &mut crate::check::Checker,
        recv: crate::hir::LayerId,
        bag: &mut crate::check::ArgBag<'_>,
    ) -> Option<crate::check::Value> {
        (self.layer?)(ctx, recv, bag)
    }

    pub fn supports_shape(&self) -> bool {
        self.shape.is_some()
    }

    pub fn supports_layer(&self) -> bool {
        self.layer.is_some()
    }

    pub fn call_with_path(
        &self,
        ctx: &mut crate::check::Checker,
        bag: &mut crate::check::ArgBag<'_>,
    ) -> Option<crate::check::Value> {
        (self.path?)(ctx, bag)
    }

    pub fn supports_path(&self) -> bool {
        self.path.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinSignature {
    pub receiver: Option<TypeRef>,
    pub args: &'static [BuiltinArgDecl],
    pub result: TypeRef,
    pub result_alternatives: &'static [TypeRef],
    pub caps: BuiltinCaps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeDecl {
    pub id: TypeId,
    pub kind: PrimitiveType,
    pub docs: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumVariantDecl {
    pub name: &'static str,
    pub docs: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumDecl {
    pub id: EnumId,
    pub docs: &'static str,
    pub variants: &'static [EnumVariantDecl],
    /// Short variants resolve only in an argument expecting this enum.
    pub contextual: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinArgDecl {
    pub name: &'static str,
    pub ty: TypeRef,
    pub viz_role: Option<&'static str>,
    pub required: bool,
    pub docs: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinDiscriminatorDecl {
    pub name: &'static str,
    pub default: Option<&'static str>,
    pub variants: &'static [&'static str],
}

impl BuiltinArgDecl {
    pub const fn required(name: &'static str, ty: TypeRef, docs: &'static str) -> Self {
        Self {
            name,
            ty,
            viz_role: None,
            required: true,
            docs,
        }
    }

    pub const fn required_with_role(
        name: &'static str,
        ty: TypeRef,
        viz_role: &'static str,
        docs: &'static str,
    ) -> Self {
        Self {
            name,
            ty,
            viz_role: Some(viz_role),
            required: true,
            docs,
        }
    }

    pub const fn optional(name: &'static str, ty: TypeRef, docs: &'static str) -> Self {
        Self {
            name,
            ty,
            viz_role: None,
            required: false,
            docs,
        }
    }

    pub const fn optional_with_role(
        name: &'static str,
        ty: TypeRef,
        viz_role: &'static str,
        docs: &'static str,
    ) -> Self {
        Self {
            name,
            ty,
            viz_role: Some(viz_role),
            required: false,
            docs,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BuiltinDecl {
    pub id: BuiltinId,
    pub name: &'static str,
    pub discriminator: Option<BuiltinDiscriminatorDecl>,
    pub signature: BuiltinSignature,
    pub lowering: BuiltinLowering,
    pub docs: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceTransformArgDecl {
    pub name: &'static str,
    pub ty: &'static str,
    pub value_kind: &'static str,
    pub enum_type: Option<&'static str>,
    pub required: bool,
    pub docs: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceTransformDecl {
    pub name: &'static str,
    pub summary: &'static str,
    pub args: &'static [SpaceTransformArgDecl],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallableArgDecl {
    pub name: &'static str,
    pub ty: &'static str,
    pub value_kind: &'static str,
    pub required: bool,
    pub docs: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallableDecl {
    pub name: &'static str,
    pub summary: &'static str,
    pub context: &'static str,
    pub args: &'static [CallableArgDecl],
    pub returns: &'static [&'static str],
}

/// Members of a `.cells(..., cell: name)` binding, shared with editor tooling.
pub const CONTOUR_MEMBERS: &[(&str, &str, &str)] = &[
    (
        "edge_distance",
        "scalar",
        "Signed inward half-plane field; use for bands matching straight inset edges.",
    ),
    (
        "distance",
        "scalar",
        "Unsigned Euclidean distance to the contour in normalized cell units.",
    ),
    (
        "progress",
        "scalar",
        "Nearest position around the contour, measured by arc length from 0 to 1.",
    ),
    (
        "length",
        "scalar",
        "Perimeter in normalized cell units; zero for a collapsed contour.",
    ),
    (
        "point",
        "method",
        "Sample the contour by normalized arc length: point(at: 0.25).",
    ),
];
pub const CELL_MEMBERS: &[(&str, &str, &str)] = &[
    (
        "contour",
        "method",
        "Closed ownership boundary inset by normalized units or explicit px.",
    ),
    ("id", "vec2", "Stable generating lattice ID."),
    (
        "center",
        "vec2",
        "Site in local drawing coordinates; not the polygon centroid.",
    ),
    (
        "uv",
        "vec2",
        "Normalized cell coordinate; site is at (0.5, 0.5).",
    ),
    ("rand", "scalar", "Stable seeded random value."),
    (
        "local",
        "vec2",
        "Normalized coordinate relative to the site.",
    ),
    (
        "angle",
        "angle",
        "Angle around the site in normalized coordinates, positive counterclockwise.",
    ),
    (
        "edge_distance",
        "scalar",
        "Inward distance to the nearest ownership boundary in normalized units.",
    ),
    (
        "inset_distance",
        "method",
        "Distance field to an inset contour. by: accepts normalized units or explicit px.",
    ),
    (
        "boundary_point",
        "method",
        "Boundary point along angle: from the site, in local drawing coordinates.",
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericTypeFamily {
    Scalar,
    Vector,
    Matrix,
    Atomic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericScalarKind {
    Bool,
    F16,
    F32,
    F64,
    I8,
    I32,
    U8,
    U32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericSurfaceScope {
    Public,
    InternalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericRuntimeBehavior {
    Native,
    ClampToI8ViaI32,
    ClampToU8ViaU32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericPrimitiveId {
    F16,
    F32,
    F64,
    I8,
    I32,
    U8,
    U32,
    F16x2,
    F16x3,
    F16x4,
    F32x2,
    F32x3,
    F32x4,
    I32x2,
    I32x3,
    I32x4,
    U32x2,
    U32x3,
    U32x4,
    Boolx2,
    Boolx3,
    Boolx4,
    F16x2x2,
    F16x2x3,
    F16x2x4,
    F16x3x2,
    F16x3x3,
    F16x3x4,
    F16x4x2,
    F16x4x3,
    F16x4x4,
    F32x2x2,
    F32x2x3,
    F32x2x4,
    F32x3x2,
    F32x3x3,
    F32x3x4,
    F32x4x2,
    F32x4x3,
    F32x4x4,
    AtomicI32,
    AtomicU32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericPrimitiveDoc {
    pub id: NumericPrimitiveId,
    pub name: &'static str,
    pub family: NumericTypeFamily,
    pub scalar: NumericScalarKind,
    pub columns: u8,
    pub rows: u8,
    pub implemented: bool,
    pub wgsl_supported: bool,
    pub wgsl_feature: Option<&'static str>,
    pub docs: &'static str,
}

macro_rules! numeric_primitive_doc {
    (
        $id:ident,
        $name:literal,
        $family:ident,
        $scalar:ident,
        $columns:expr,
        $rows:expr,
        $implemented:expr,
        $wgsl_supported:expr,
        $wgsl_feature:expr,
        $docs:literal
    ) => {
        NumericPrimitiveDoc {
            id: NumericPrimitiveId::$id,
            name: $name,
            family: NumericTypeFamily::$family,
            scalar: NumericScalarKind::$scalar,
            columns: $columns,
            rows: $rows,
            implemented: $implemented,
            wgsl_supported: $wgsl_supported,
            wgsl_feature: $wgsl_feature,
            docs: $docs,
        }
    };
}

const NUMERIC_PRIMITIVE_DOCS: &[NumericPrimitiveDoc] = &[
    numeric_primitive_doc!(
        F16,
        "f16",
        Scalar,
        F16,
        1,
        1,
        false,
        true,
        Some("shader-f16"),
        "16-bit floating-point scalar"
    ),
    numeric_primitive_doc!(
        F32,
        "f32",
        Scalar,
        F32,
        1,
        1,
        true,
        true,
        None,
        "32-bit floating-point scalar"
    ),
    numeric_primitive_doc!(
        F64,
        "f64",
        Scalar,
        F64,
        1,
        1,
        true,
        false,
        None,
        "64-bit floating-point scalar alias (non-WGSL compatibility)"
    ),
    numeric_primitive_doc!(
        I8,
        "i8",
        Scalar,
        I8,
        1,
        1,
        false,
        false,
        None,
        "signed 8-bit integer scalar (internal-only; lowered via i32 clamp)"
    ),
    numeric_primitive_doc!(
        I32,
        "i32",
        Scalar,
        I32,
        1,
        1,
        true,
        true,
        None,
        "signed 32-bit integer scalar"
    ),
    numeric_primitive_doc!(
        U8,
        "u8",
        Scalar,
        U8,
        1,
        1,
        false,
        false,
        None,
        "unsigned 8-bit integer scalar (internal-only; lowered via u32 clamp)"
    ),
    numeric_primitive_doc!(
        U32,
        "u32",
        Scalar,
        U32,
        1,
        1,
        true,
        true,
        None,
        "unsigned 32-bit integer scalar"
    ),
    numeric_primitive_doc!(
        F16x2,
        "f16x2",
        Vector,
        F16,
        2,
        1,
        false,
        true,
        Some("shader-f16"),
        "2-lane f16 vector"
    ),
    numeric_primitive_doc!(
        F16x3,
        "f16x3",
        Vector,
        F16,
        3,
        1,
        false,
        true,
        Some("shader-f16"),
        "3-lane f16 vector"
    ),
    numeric_primitive_doc!(
        F16x4,
        "f16x4",
        Vector,
        F16,
        4,
        1,
        false,
        true,
        Some("shader-f16"),
        "4-lane f16 vector"
    ),
    numeric_primitive_doc!(
        F32x2,
        "f32x2",
        Vector,
        F32,
        2,
        1,
        true,
        true,
        None,
        "2-lane f32 vector (maps to vec2 family)"
    ),
    numeric_primitive_doc!(
        F32x3,
        "f32x3",
        Vector,
        F32,
        3,
        1,
        true,
        true,
        None,
        "3-lane f32 vector (maps to vec3 family)"
    ),
    numeric_primitive_doc!(
        F32x4,
        "f32x4",
        Vector,
        F32,
        4,
        1,
        true,
        true,
        None,
        "4-lane f32 vector (maps to vec4 family)"
    ),
    numeric_primitive_doc!(
        I32x2,
        "i32x2",
        Vector,
        I32,
        2,
        1,
        false,
        true,
        None,
        "2-lane i32 vector"
    ),
    numeric_primitive_doc!(
        I32x3,
        "i32x3",
        Vector,
        I32,
        3,
        1,
        false,
        true,
        None,
        "3-lane i32 vector"
    ),
    numeric_primitive_doc!(
        I32x4,
        "i32x4",
        Vector,
        I32,
        4,
        1,
        false,
        true,
        None,
        "4-lane i32 vector"
    ),
    numeric_primitive_doc!(
        U32x2,
        "u32x2",
        Vector,
        U32,
        2,
        1,
        false,
        true,
        None,
        "2-lane u32 vector"
    ),
    numeric_primitive_doc!(
        U32x3,
        "u32x3",
        Vector,
        U32,
        3,
        1,
        false,
        true,
        None,
        "3-lane u32 vector"
    ),
    numeric_primitive_doc!(
        U32x4,
        "u32x4",
        Vector,
        U32,
        4,
        1,
        false,
        true,
        None,
        "4-lane u32 vector"
    ),
    numeric_primitive_doc!(
        Boolx2,
        "boolx2",
        Vector,
        Bool,
        2,
        1,
        false,
        true,
        None,
        "2-lane bool vector"
    ),
    numeric_primitive_doc!(
        Boolx3,
        "boolx3",
        Vector,
        Bool,
        3,
        1,
        false,
        true,
        None,
        "3-lane bool vector"
    ),
    numeric_primitive_doc!(
        Boolx4,
        "boolx4",
        Vector,
        Bool,
        4,
        1,
        false,
        true,
        None,
        "4-lane bool vector"
    ),
    numeric_primitive_doc!(
        F16x2x2,
        "f16x2x2",
        Matrix,
        F16,
        2,
        2,
        false,
        true,
        Some("shader-f16"),
        "2x2 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x2x3,
        "f16x2x3",
        Matrix,
        F16,
        2,
        3,
        false,
        true,
        Some("shader-f16"),
        "2x3 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x2x4,
        "f16x2x4",
        Matrix,
        F16,
        2,
        4,
        false,
        true,
        Some("shader-f16"),
        "2x4 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x3x2,
        "f16x3x2",
        Matrix,
        F16,
        3,
        2,
        false,
        true,
        Some("shader-f16"),
        "3x2 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x3x3,
        "f16x3x3",
        Matrix,
        F16,
        3,
        3,
        false,
        true,
        Some("shader-f16"),
        "3x3 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x3x4,
        "f16x3x4",
        Matrix,
        F16,
        3,
        4,
        false,
        true,
        Some("shader-f16"),
        "3x4 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x4x2,
        "f16x4x2",
        Matrix,
        F16,
        4,
        2,
        false,
        true,
        Some("shader-f16"),
        "4x2 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x4x3,
        "f16x4x3",
        Matrix,
        F16,
        4,
        3,
        false,
        true,
        Some("shader-f16"),
        "4x3 f16 matrix"
    ),
    numeric_primitive_doc!(
        F16x4x4,
        "f16x4x4",
        Matrix,
        F16,
        4,
        4,
        false,
        true,
        Some("shader-f16"),
        "4x4 f16 matrix"
    ),
    numeric_primitive_doc!(
        F32x2x2,
        "f32x2x2",
        Matrix,
        F32,
        2,
        2,
        true,
        true,
        None,
        "2x2 f32 matrix (maps to mat2 family)"
    ),
    numeric_primitive_doc!(
        F32x2x3,
        "f32x2x3",
        Matrix,
        F32,
        2,
        3,
        false,
        true,
        None,
        "2x3 f32 matrix"
    ),
    numeric_primitive_doc!(
        F32x2x4,
        "f32x2x4",
        Matrix,
        F32,
        2,
        4,
        false,
        true,
        None,
        "2x4 f32 matrix"
    ),
    numeric_primitive_doc!(
        F32x3x2,
        "f32x3x2",
        Matrix,
        F32,
        3,
        2,
        false,
        true,
        None,
        "3x2 f32 matrix"
    ),
    numeric_primitive_doc!(
        F32x3x3,
        "f32x3x3",
        Matrix,
        F32,
        3,
        3,
        true,
        true,
        None,
        "3x3 f32 matrix (maps to mat3 family)"
    ),
    numeric_primitive_doc!(
        F32x3x4,
        "f32x3x4",
        Matrix,
        F32,
        3,
        4,
        false,
        true,
        None,
        "3x4 f32 matrix"
    ),
    numeric_primitive_doc!(
        F32x4x2,
        "f32x4x2",
        Matrix,
        F32,
        4,
        2,
        false,
        true,
        None,
        "4x2 f32 matrix"
    ),
    numeric_primitive_doc!(
        F32x4x3,
        "f32x4x3",
        Matrix,
        F32,
        4,
        3,
        false,
        true,
        None,
        "4x3 f32 matrix"
    ),
    numeric_primitive_doc!(
        F32x4x4,
        "f32x4x4",
        Matrix,
        F32,
        4,
        4,
        true,
        true,
        None,
        "4x4 f32 matrix (maps to mat4 family)"
    ),
    numeric_primitive_doc!(
        AtomicI32,
        "atomic<i32>",
        Atomic,
        I32,
        1,
        1,
        false,
        true,
        None,
        "WGSL atomic signed integer type"
    ),
    numeric_primitive_doc!(
        AtomicU32,
        "atomic<u32>",
        Atomic,
        U32,
        1,
        1,
        false,
        true,
        None,
        "WGSL atomic unsigned integer type"
    ),
];

pub fn numeric_primitive_doc_by_id(id: NumericPrimitiveId) -> Option<&'static NumericPrimitiveDoc> {
    NUMERIC_PRIMITIVE_DOCS.iter().find(|doc| doc.id == id)
}

impl NumericPrimitiveId {
    pub fn metadata(self) -> &'static NumericPrimitiveDoc {
        numeric_primitive_doc_by_id(self)
            .expect("numeric primitive id must have a matching metadata entry")
    }

    pub const fn allow_as_param(self) -> bool {
        !matches!(self, Self::I8 | Self::U8)
    }

    pub const fn surface_scope(self) -> NumericSurfaceScope {
        match self {
            Self::I8 | Self::U8 => NumericSurfaceScope::InternalOnly,
            _ => NumericSurfaceScope::Public,
        }
    }

    pub const fn runtime_behavior(self) -> NumericRuntimeBehavior {
        match self {
            Self::I8 => NumericRuntimeBehavior::ClampToI8ViaI32,
            Self::U8 => NumericRuntimeBehavior::ClampToU8ViaU32,
            _ => NumericRuntimeBehavior::Native,
        }
    }
}

pub type BuiltinLowerFn = fn() -> BuiltinLoweringPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinLoweringPlan {
    ShapeCtor(ShapeCtor),
}

inventory::collect!(TypeDecl);
inventory::collect!(EnumDecl);
inventory::collect!(BuiltinDecl);

const ANGLE_CALLABLE_ARGS: &[CallableArgDecl] = &[CallableArgDecl {
    name: "theta",
    ty: "f32",
    value_kind: "expr",
    required: true,
    docs: "Angle in radians or with `deg` unit.",
}];

const PATH_SAMPLE_CALLABLE_ARGS: &[CallableArgDecl] = &[
    CallableArgDecl {
        name: "path",
        ty: "path",
        value_kind: "expr",
        required: true,
        docs: "Path value to sample.",
    },
    CallableArgDecl {
        name: "s",
        ty: "f32",
        value_kind: "expr",
        required: true,
        docs: "Arc-length parameter.",
    },
];

const CALLABLE_DECLS: &[CallableDecl] = &[
    CallableDecl {
        name: "contour",
        context: "cell-method",
        summary: "Create a closed cell contour with shared distance and arc-length queries.",
        args: &[CallableArgDecl {
            name: "inset",
            ty: "scalar",
            value_kind: "type",
            required: false,
            docs: "Nonnegative inward offset; normalized cell units or explicit px. Defaults to zero.",
        }],
        returns: &["contour"],
    },
    CallableDecl {
        name: "point",
        context: "contour-method",
        summary: "Sample a cell contour by normalized arc length in local drawing coordinates.",
        args: &[CallableArgDecl {
            name: "at",
            ty: "scalar",
            value_kind: "type",
            required: true,
            docs: "Position around the loop. Wraps every 1. A collapsed contour returns the site.",
        }],
        returns: &["vec2"],
    },
    CallableDecl {
        name: "band",
        context: "expr-call",
        summary: "Turn an unsigned distance into a line brightness mask.",
        args: &[
            CallableArgDecl {
                name: "distance",
                ty: "scalar",
                value_kind: "type",
                required: true,
                docs: "Distance field; contour distances support screen-pixel widths.",
            },
            CallableArgDecl {
                name: "width",
                ty: "scalar",
                value_kind: "type",
                required: true,
                docs: "Full width. Positive normalized units or explicit px. Soft bands are half-bright at half-width.",
            },
            CallableArgDecl {
                name: "profile",
                ty: "BandProfile",
                value_kind: "type",
                required: false,
                docs: "solid uses a smooth edge; soft uses an exponential falloff.",
            },
        ],
        returns: &["scalar"],
    },
    CallableDecl {
        name: "chase",
        context: "expr-call",
        summary: "A fading pulse moving at constant arc-length speed around a contour.",
        args: &[
            CallableArgDecl {
                name: "head",
                ty: "scalar",
                value_kind: "type",
                required: false,
                docs: "Explicit travel position in laps along the chosen direction, driven by any signal. Choose exactly one of head, lap, or speed.",
            },
            CallableArgDecl {
                name: "motion",
                ty: "ContourMotion",
                value_kind: "type",
                required: false,
                docs: "perimeter (default) measures actual arc length; angular preserves uniform rotation around the cell site and uses lap or head with a fractional tail.",
            },
            CallableArgDecl {
                name: "along",
                ty: "contour",
                value_kind: "type",
                required: true,
                docs: "Closed contour to follow.",
            },
            CallableArgDecl {
                name: "speed",
                ty: "scalar",
                value_kind: "type",
                required: false,
                docs: "Time-driven displacement such as 20px/s; do not multiply rate literals by time again. Mutually exclusive with lap.",
            },
            CallableArgDecl {
                name: "lap",
                ty: "time",
                value_kind: "type",
                required: false,
                docs: "Positive duration of one lap, such as 6s. Mutually exclusive with speed.",
            },
            CallableArgDecl {
                name: "tail",
                ty: "scalar",
                value_kind: "type",
                required: true,
                docs: "Half-bright tail length: fraction of a lap, or explicit screen pixels.",
            },
            CallableArgDecl {
                name: "direction",
                ty: "ContourDirection",
                value_kind: "type",
                required: false,
                docs: "clockwise (default) or counterclockwise in the cell plane.",
            },
            CallableArgDecl {
                name: "phase",
                ty: "scalar",
                value_kind: "type",
                required: false,
                docs: "Initial offset in laps; defaults to zero.",
            },
        ],
        returns: &["scalar"],
    },
    CallableDecl {
        name: "inset_distance",
        summary: "Cell method: normalized distance field to an inset contour; explicit px uses the boundary normal's screen footprint.",
        context: "cell-method",
        args: &[CallableArgDecl {
            name: "by",
            ty: "scalar",
            value_kind: "type",
            required: true,
            docs: "Normalized cell units by default; explicit pixel expressions such as 2px are converted per boundary normal.",
        }],
        returns: &["scalar"],
    },
    CallableDecl {
        name: "boundary_point",
        summary: "Cell method: intersect a ray from the site with the ownership boundary; returns local drawing coordinates.",
        context: "cell-method",
        args: &[CallableArgDecl {
            name: "angle",
            ty: "angle",
            value_kind: "type",
            required: true,
            docs: "Direction in the normalized cell plane; zero points right, positive angles counterclockwise. Accepts deg and turn.",
        }],
        returns: &["vec2"],
    },
    CallableDecl {
        name: "angle",
        summary: "Construct a unit direction vector from an angle.",
        context: "expr-call",
        args: ANGLE_CALLABLE_ARGS,
        returns: &["vec2"],
    },
    CallableDecl {
        name: "point_at",
        summary: "Sample a path position by arc length using free-function call syntax.",
        context: "expr-call",
        args: PATH_SAMPLE_CALLABLE_ARGS,
        returns: &["vec2"],
    },
    CallableDecl {
        name: "tangent_at",
        summary: "Sample a path tangent by arc length using free-function call syntax.",
        context: "expr-call",
        args: PATH_SAMPLE_CALLABLE_ARGS,
        returns: &["vec2"],
    },
];

const SPACE_TRANSFORM_ROTATE_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "angle",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Rotation angle in radians or with deg unit.",
    },
    SpaceTransformArgDecl {
        name: "around",
        ty: "vec2",
        value_kind: "type",
        enum_type: None,
        required: false,
        docs: "Optional rotation pivot (defaults to center).",
    },
];

const SPACE_TRANSFORM_TRANSLATE_ARGS: &[SpaceTransformArgDecl] = &[SpaceTransformArgDecl {
    name: "by",
    ty: "vec2",
    value_kind: "type",
    enum_type: None,
    required: true,
    docs: "Translation vector in current space.",
}];

const SPACE_TRANSFORM_TRANSLATE3_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "x",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Horizontal offset.",
    },
    SpaceTransformArgDecl {
        name: "y",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Vertical offset.",
    },
    SpaceTransformArgDecl {
        name: "z",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Depth offset.",
    },
];

const SPACE_TRANSFORM_SCALE_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "factor",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Uniform scale factor.",
    },
    SpaceTransformArgDecl {
        name: "around",
        ty: "vec2",
        value_kind: "type",
        enum_type: None,
        required: false,
        docs: "Optional scale pivot (defaults to center).",
    },
];

const SPACE_TRANSFORM_REPEAT_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "every",
        ty: "scalar | vec2",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Repeat period along both axes; scalar repeats uniformly, vec2 uses independent x/y periods.",
    },
    SpaceTransformArgDecl {
        name: "cell",
        ty: "identifier",
        value_kind: "binding",
        enum_type: None,
        required: false,
        docs: "Optional block-local repeat-cell binding exposing `.id`, `.center`, `.uv`, and deterministic `.rand` (seed zero). Legacy `id:` and `as:` spellings are still accepted.",
    },
];

const SPACE_TRANSFORM_REPEAT_AXIS_ARGS: &[SpaceTransformArgDecl] = &[SpaceTransformArgDecl {
    name: "every",
    ty: "scalar",
    value_kind: "type",
    enum_type: None,
    required: true,
    docs: "Repeat period along the selected axis.",
}];

const SPACE_TRANSFORM_CELLS_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "layout",
        ty: "CellLayout",
        value_kind: "enum",
        enum_type: Some("CellLayout"),
        required: true,
        docs: "Cell ownership layout. Content is clipped to its owner; this is not overlapping scatter.",
    },
    SpaceTransformArgDecl {
        name: "every",
        ty: "scalar | vec2",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Positive compile-time lattice scale. Nearest sites use the normalized lattice metric.",
    },
    SpaceTransformArgDecl {
        name: "seed",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Compile-time integer 0..65535 controlling stable site jitter and cell.rand.",
    },
    SpaceTransformArgDecl {
        name: "sampling",
        ty: "CellSampling",
        value_kind: "enum",
        enum_type: Some("CellSampling"),
        required: true,
        docs: "Pixel-center sampling or a 2x2, 3x3, or 4x4 grid. Outermost cells samples its entire subtree, including nested cells.",
    },
    SpaceTransformArgDecl {
        name: "jitter",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: false,
        docs: "Required for jittered/voronoi, rejected otherwise. Compile-time 0..1; one site remains within each generating square.",
    },
    SpaceTransformArgDecl {
        name: "cell",
        ty: "identifier",
        value_kind: "binding",
        enum_type: None,
        required: false,
        docs: "Block-local cell exposing id, center, uv, and rand. Center is in the mapped local frame; uv can extend outside 0..1.",
    },
];

const SPACE_TRANSFORM_REPEAT_RADIAL_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "around",
        ty: "vec2",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Center point for radial repetition.",
    },
    SpaceTransformArgDecl {
        name: "count",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: false,
        docs: "Number of sectors (required with `from`/`to`; not used when `angles` is provided).",
    },
    SpaceTransformArgDecl {
        name: "from",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: false,
        docs: "Start angle for repeated span (used with `count` and `to`).",
    },
    SpaceTransformArgDecl {
        name: "to",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: false,
        docs: "End angle for repeated span (used with `count` and `from`).",
    },
    SpaceTransformArgDecl {
        name: "angles",
        ty: "number_source",
        value_kind: "type",
        enum_type: None,
        required: false,
        docs: "Explicit angle list or range; cannot be combined with `from`/`to`.",
    },
];

const SPACE_TRANSFORM_ASPECT_ARGS: &[SpaceTransformArgDecl] = &[SpaceTransformArgDecl {
    name: "ratio",
    ty: "scalar",
    value_kind: "type",
    enum_type: None,
    required: true,
    docs: "Target authored aspect ratio.",
}];

const SPACE_TRANSFORM_CENTERED_ARGS: &[SpaceTransformArgDecl] = &[SpaceTransformArgDecl {
    name: "aspect",
    ty: "CenteredMode",
    value_kind: "enum",
    enum_type: Some("CenteredMode"),
    required: false,
    docs: "Centered aspect behavior: preserve, fit, or fill.",
}];

const SPACE_TRANSFORM_ORIENTATION_ARGS: &[SpaceTransformArgDecl] = &[SpaceTransformArgDecl {
    name: "y",
    ty: "YAxis",
    value_kind: "enum",
    enum_type: Some("YAxis"),
    required: true,
    docs: "Vertical axis direction: up or down.",
}];

const SPACE_TRANSFORM_POLAR_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "center",
        ty: "vec2",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Polar center point.",
    },
    SpaceTransformArgDecl {
        name: "from",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Start angle offset.",
    },
    SpaceTransformArgDecl {
        name: "direction",
        ty: "PolarDir",
        value_kind: "enum",
        enum_type: Some("PolarDir"),
        required: true,
        docs: "Angular direction: clockwise or counterclockwise.",
    },
];

const SPACE_TRANSFORM_WARP_ARGS: &[SpaceTransformArgDecl] = &[SpaceTransformArgDecl {
    name: "by",
    ty: "vec2",
    value_kind: "type",
    enum_type: None,
    required: true,
    docs: "Displacement vector in current space.",
}];

const SPACE_TRANSFORM_PERSPECTIVE_ARGS: &[SpaceTransformArgDecl] = &[
    SpaceTransformArgDecl {
        name: "fov",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Field of view.",
    },
    SpaceTransformArgDecl {
        name: "near",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Near plane distance.",
    },
    SpaceTransformArgDecl {
        name: "far",
        ty: "scalar",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Far plane distance.",
    },
    SpaceTransformArgDecl {
        name: "origin",
        ty: "vec2",
        value_kind: "type",
        enum_type: None,
        required: true,
        docs: "Perspective origin in screen space.",
    },
];

const SPACE_TRANSFORM_DECLS: &[SpaceTransformDecl] = &[
    SpaceTransformDecl {
        name: "rotate",
        summary: "Rotate in authored 2D space around an anchor point.",
        args: SPACE_TRANSFORM_ROTATE_ARGS,
    },
    SpaceTransformDecl {
        name: "rotate_x",
        summary: "Apply pseudo-3D X-axis rotation in flat perspective.",
        args: SPACE_TRANSFORM_ROTATE_ARGS,
    },
    SpaceTransformDecl {
        name: "rotate_y",
        summary: "Apply pseudo-3D Y-axis rotation in flat perspective.",
        args: SPACE_TRANSFORM_ROTATE_ARGS,
    },
    SpaceTransformDecl {
        name: "rotate_z",
        summary: "Alias of rotate for explicit axis style.",
        args: SPACE_TRANSFORM_ROTATE_ARGS,
    },
    SpaceTransformDecl {
        name: "translate",
        summary: "Translate authored coordinates.",
        args: SPACE_TRANSFORM_TRANSLATE_ARGS,
    },
    SpaceTransformDecl {
        name: "translate3",
        summary: "Pseudo-3D translation with explicit z depth.",
        args: SPACE_TRANSFORM_TRANSLATE3_ARGS,
    },
    SpaceTransformDecl {
        name: "scale",
        summary: "Uniform scale in authored space.",
        args: SPACE_TRANSFORM_SCALE_ARGS,
    },
    SpaceTransformDecl {
        name: "repeat",
        summary: "Repeat authored space on a 2-D lattice, optionally exposing a named repeat-cell binding.",
        args: SPACE_TRANSFORM_REPEAT_ARGS,
    },
    SpaceTransformDecl {
        name: "cells",
        summary: "Filtered square, staggered brick, hexagonal, jittered-square, or bounded Voronoi ownership spaces. Use as the final transform in a chain; nest subsequent transforms.",
        args: SPACE_TRANSFORM_CELLS_ARGS,
    },
    SpaceTransformDecl {
        name: "repeat_x",
        summary: "Repeat authored space along X axis.",
        args: SPACE_TRANSFORM_REPEAT_AXIS_ARGS,
    },
    SpaceTransformDecl {
        name: "repeat_y",
        summary: "Repeat authored space along Y axis.",
        args: SPACE_TRANSFORM_REPEAT_AXIS_ARGS,
    },
    SpaceTransformDecl {
        name: "repeat_radial",
        summary: "Repeat authored space around a center in angular sectors.",
        args: SPACE_TRANSFORM_REPEAT_RADIAL_ARGS,
    },
    SpaceTransformDecl {
        name: "aspect",
        summary: "Apply authored-to-runtime aspect ratio mapping.",
        args: SPACE_TRANSFORM_ASPECT_ARGS,
    },
    SpaceTransformDecl {
        name: "centered",
        summary: "Convenience centered framing transform.",
        args: SPACE_TRANSFORM_CENTERED_ARGS,
    },
    SpaceTransformDecl {
        name: "orientation",
        summary: "Set authored vertical axis orientation.",
        args: SPACE_TRANSFORM_ORIENTATION_ARGS,
    },
    SpaceTransformDecl {
        name: "polar",
        summary: "Map into polar coordinates around a center.",
        args: SPACE_TRANSFORM_POLAR_ARGS,
    },
    SpaceTransformDecl {
        name: "warp",
        summary: "Displace sample coordinates by a vector field.",
        args: SPACE_TRANSFORM_WARP_ARGS,
    },
    SpaceTransformDecl {
        name: "perspective",
        summary: "Apply flat perspective projection parameters.",
        args: SPACE_TRANSFORM_PERSPECTIVE_ARGS,
    },
];

#[derive(Default)]
struct RegistryCache {
    builtin_decls: Vec<&'static BuiltinDecl>,
    builtin_decls_by_name: HashMap<&'static str, Vec<&'static BuiltinDecl>>,
    type_decls: Vec<&'static TypeDecl>,
    type_decls_by_name: HashMap<&'static str, &'static TypeDecl>,
    enum_decls: Vec<&'static EnumDecl>,
    enum_decls_by_name: HashMap<&'static str, &'static EnumDecl>,
}

fn registry_cache() -> &'static RegistryCache {
    static CACHE: OnceLock<RegistryCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut cache = RegistryCache::default();

        for decl in inventory::iter::<BuiltinDecl> {
            cache.builtin_decls.push(decl);
            cache
                .builtin_decls_by_name
                .entry(decl.name)
                .or_default()
                .push(decl);
        }

        for decl in inventory::iter::<TypeDecl> {
            cache.type_decls.push(decl);
            cache.type_decls_by_name.insert(decl.id.0, decl);
        }

        for decl in inventory::iter::<EnumDecl> {
            cache.enum_decls.push(decl);
            cache.enum_decls_by_name.insert(decl.id.0, decl);
        }

        cache
    })
}

pub fn builtin_decls() -> &'static [&'static BuiltinDecl] {
    registry_cache().builtin_decls.as_slice()
}

pub fn type_decls() -> &'static [&'static TypeDecl] {
    registry_cache().type_decls.as_slice()
}

pub fn enum_decls() -> &'static [&'static EnumDecl] {
    registry_cache().enum_decls.as_slice()
}

pub fn builtin_decl_by_name(name: &str) -> &'static [&'static BuiltinDecl] {
    registry_cache()
        .builtin_decls_by_name
        .get(name)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub fn type_decl_by_name(name: &str) -> Option<&'static TypeDecl> {
    registry_cache().type_decls_by_name.get(name).copied()
}

pub fn enum_decl_by_name(name: &str) -> Option<&'static EnumDecl> {
    registry_cache().enum_decls_by_name.get(name).copied()
}

pub fn enum_allows_unqualified_values(name: &str) -> bool {
    enum_decl_by_name(name).is_none_or(|decl| !decl.contextual)
}

pub fn space_transform_decls() -> &'static [SpaceTransformDecl] {
    SPACE_TRANSFORM_DECLS
}

pub fn callable_decls() -> &'static [CallableDecl] {
    CALLABLE_DECLS
}

pub fn callable_decl_by_name(name: &str) -> Option<&'static CallableDecl> {
    callable_decls().iter().find(|decl| decl.name == name)
}

pub fn numeric_primitive_docs() -> &'static [NumericPrimitiveDoc] {
    NUMERIC_PRIMITIVE_DOCS
}
