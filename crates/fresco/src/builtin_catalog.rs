#![allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]

use std::sync::LazyLock;

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct BuiltinCaps: u32 {
        const PURE = 1 << 0;
        const PIPEABLE = 1 << 1;
        const SHAPE_RECV = 1 << 2;
        const LAYER_RECV = 1 << 3;
        const SCALAR_RECV = 1 << 4;
        const VEC2_RECV = 1 << 5;
        const COLOR_RECV = 1 << 6;
        const SHAPE_OUT = 1 << 7;
        const LAYER_OUT = 1 << 8;
        const SCALAR_OUT = 1 << 9;
        const VEC2_OUT = 1 << 10;
        const COLOR_OUT = 1 << 11;
        const COVERAGE_OUT = 1 << 12;
        const MASK_OUT = 1 << 13;
        const GRADIENT_OUT = 1 << 14;
        const INTRINSIC = 1 << 15;
        const ORDER_SENSITIVE = 1 << 16;
        const WIDE_EFFECT = 1 << 17;
        const VEC3_OUT = 1 << 18;
        const VEC4_OUT = 1 << 19;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Shape,
    Layer,
    Scalar,
    Vec2,
    Vec3,
    Vec4,
    Color,
    ColorField,
    Coverage,
    Mask,
    Gradient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinKind {
    Circle,
    Capsule,
    Box,
    Lines,
    Gridline,
    FillColor,
    FillShape,
    StrokeShape,
    RoundShape,
    RoundScalar,
    SmoothShape,
    DilateShape,
    ErodeShape,
    ShadowShape,
    GlowShape,
    InnerGlowShape,
    BevelShape,
    SoftenShape,
    SoftenLayer,
    InnerGlowLayer,
    BevelLayer,
    GlowLayer,
    BlurLayer,
    OpacityLayer,
    TintLayer,
    Image,
    Grey,
    Rgb,
    Rgba,
    Rand,
    Wrap,
    Noise1,
    Noise2,
    Noise3,
    Fbm,
    AgeNorm,
    Wave,
    Pulse,
    Ramp,
    Gradient,
    Abs,
    Sign,
    Sqrt,
    InverseSqrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Floor,
    Ceil,
    Trunc,
    Pow,
    Exp,
    Exp2,
    Log,
    Log2,
    Min,
    Max,
    Clamp,
    Mix,
    Step,
    SmoothStep,
    Length,
    Dot,
    Normalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinSpec {
    pub kind: BuiltinKind,
    pub name: &'static str,
    pub receiver: Option<ValueKind>,
    pub result: ValueKind,
    pub caps: BuiltinCaps,
}

impl BuiltinSpec {
    pub fn new(
        kind: BuiltinKind,
        name: &'static str,
        receiver: Option<ValueKind>,
        result: ValueKind,
        caps: BuiltinCaps,
    ) -> Self {
        Self {
            kind,
            name,
            receiver,
            result,
            caps,
        }
    }

    pub fn is_pipeable(self) -> bool {
        self.caps.contains(BuiltinCaps::PIPEABLE)
    }
}

static BUILTIN_SPECS: LazyLock<Vec<BuiltinSpec>> = LazyLock::new(|| {
    vec![
        BuiltinSpec::new(
            BuiltinKind::Circle,
            "circle",
            None,
            ValueKind::Shape,
            BuiltinCaps::PURE | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Capsule,
            "capsule",
            None,
            ValueKind::Shape,
            BuiltinCaps::PURE | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Box,
            "box",
            None,
            ValueKind::Shape,
            BuiltinCaps::PURE | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Lines,
            "lines",
            None,
            ValueKind::Shape,
            BuiltinCaps::PURE | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Gridline,
            "gridline",
            None,
            ValueKind::Shape,
            BuiltinCaps::PURE | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::FillColor,
            "fill",
            None,
            ValueKind::Layer,
            BuiltinCaps::PURE | BuiltinCaps::LAYER_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::FillShape,
            "fill",
            Some(ValueKind::Shape),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::LAYER_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::StrokeShape,
            "stroke",
            Some(ValueKind::Shape),
            ValueKind::Shape,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::RoundShape,
            "round",
            Some(ValueKind::Shape),
            ValueKind::Shape,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::RoundScalar,
            "round",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::SmoothShape,
            "smooth",
            Some(ValueKind::Shape),
            ValueKind::Shape,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::SHAPE_OUT
                | BuiltinCaps::ORDER_SENSITIVE,
        ),
        BuiltinSpec::new(
            BuiltinKind::DilateShape,
            "dilate",
            Some(ValueKind::Shape),
            ValueKind::Shape,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::ErodeShape,
            "erode",
            Some(ValueKind::Shape),
            ValueKind::Shape,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::SHAPE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::ShadowShape,
            "shadow",
            Some(ValueKind::Shape),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::GlowShape,
            "glow",
            Some(ValueKind::Shape),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::InnerGlowShape,
            "inner_glow",
            Some(ValueKind::Shape),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::BevelShape,
            "bevel",
            Some(ValueKind::Shape),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::SoftenShape,
            "soften",
            Some(ValueKind::Shape),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::SHAPE_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::SoftenLayer,
            "soften",
            Some(ValueKind::Layer),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::LAYER_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::InnerGlowLayer,
            "inner_glow",
            Some(ValueKind::Layer),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::LAYER_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::BevelLayer,
            "bevel",
            Some(ValueKind::Layer),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::LAYER_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::GlowLayer,
            "glow",
            Some(ValueKind::Layer),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::LAYER_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::BlurLayer,
            "blur",
            Some(ValueKind::Layer),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::LAYER_RECV
                | BuiltinCaps::LAYER_OUT
                | BuiltinCaps::WIDE_EFFECT,
        ),
        BuiltinSpec::new(
            BuiltinKind::OpacityLayer,
            "opacity",
            Some(ValueKind::Layer),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::LAYER_RECV
                | BuiltinCaps::LAYER_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::TintLayer,
            "tint",
            Some(ValueKind::Layer),
            ValueKind::Layer,
            BuiltinCaps::PURE
                | BuiltinCaps::PIPEABLE
                | BuiltinCaps::LAYER_RECV
                | BuiltinCaps::LAYER_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Image,
            "image",
            None,
            ValueKind::Layer,
            BuiltinCaps::PURE | BuiltinCaps::LAYER_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Grey,
            "grey",
            None,
            ValueKind::Layer,
            BuiltinCaps::PURE | BuiltinCaps::LAYER_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Rgb,
            "rgb",
            None,
            ValueKind::ColorField,
            BuiltinCaps::PURE | BuiltinCaps::COLOR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Rgba,
            "rgba",
            None,
            ValueKind::ColorField,
            BuiltinCaps::PURE | BuiltinCaps::COLOR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Rand,
            "rand",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Wrap,
            "wrap",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Noise1,
            "noise1",
            None,
            ValueKind::Mask,
            BuiltinCaps::PURE | BuiltinCaps::MASK_OUT | BuiltinCaps::INTRINSIC,
        ),
        BuiltinSpec::new(
            BuiltinKind::Noise2,
            "noise2",
            None,
            ValueKind::Mask,
            BuiltinCaps::PURE | BuiltinCaps::MASK_OUT | BuiltinCaps::INTRINSIC,
        ),
        BuiltinSpec::new(
            BuiltinKind::Noise3,
            "noise3",
            None,
            ValueKind::Mask,
            BuiltinCaps::PURE | BuiltinCaps::MASK_OUT | BuiltinCaps::INTRINSIC,
        ),
        BuiltinSpec::new(
            BuiltinKind::Fbm,
            "fbm",
            None,
            ValueKind::Mask,
            BuiltinCaps::PURE | BuiltinCaps::MASK_OUT | BuiltinCaps::INTRINSIC,
        ),
        BuiltinSpec::new(
            BuiltinKind::AgeNorm,
            "age_norm",
            None,
            ValueKind::Coverage,
            BuiltinCaps::PURE | BuiltinCaps::COVERAGE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Wave,
            "wave",
            None,
            ValueKind::Coverage,
            BuiltinCaps::PURE | BuiltinCaps::COVERAGE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Pulse,
            "pulse",
            None,
            ValueKind::Coverage,
            BuiltinCaps::PURE | BuiltinCaps::COVERAGE_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Ramp,
            "ramp",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Gradient,
            "gradient",
            None,
            ValueKind::Gradient,
            BuiltinCaps::PURE | BuiltinCaps::GRADIENT_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Abs,
            "abs",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Sign,
            "sign",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Sqrt,
            "sqrt",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::InverseSqrt,
            "inversesqrt",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::InverseSqrt,
            "inverse_sqrt",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::InverseSqrt,
            "inverseSqrt",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Sin,
            "sin",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Cos,
            "cos",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Tan,
            "tan",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Asin,
            "asin",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Acos,
            "acos",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Atan,
            "atan",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Atan2,
            "atan2",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Floor,
            "floor",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Ceil,
            "ceil",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Trunc,
            "trunc",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Pow,
            "pow",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Exp,
            "exp",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Exp2,
            "exp2",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Log,
            "log",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Log2,
            "log2",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Min,
            "min",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Max,
            "max",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Clamp,
            "clamp",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Mix,
            "mix",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Step,
            "step",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::SmoothStep,
            "smoothstep",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Length,
            "length",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Dot,
            "dot",
            None,
            ValueKind::Scalar,
            BuiltinCaps::PURE | BuiltinCaps::SCALAR_OUT,
        ),
        BuiltinSpec::new(
            BuiltinKind::Normalize,
            "normalize",
            None,
            ValueKind::Vec2,
            BuiltinCaps::PURE | BuiltinCaps::VEC2_OUT,
        ),
    ]
});

pub fn builtin_specs() -> &'static [BuiltinSpec] {
    BUILTIN_SPECS.as_slice()
}

pub fn builtin_specs_by_name(name: &str) -> impl Iterator<Item = &'static BuiltinSpec> {
    BUILTIN_SPECS.iter().filter(move |spec| spec.name == name)
}

pub fn builtin_spec_names() -> impl Iterator<Item = &'static str> {
    BUILTIN_SPECS.iter().map(|spec| spec.name)
}
