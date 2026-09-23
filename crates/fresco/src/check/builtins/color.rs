//! Color constructor builtins.

use crate::builtin;
use crate::check::{ColorSpace, Value};
use crate::hir::{Layer, Sx};

builtin! {
    name = "grey",
    signature = single {
        args(value: Scalar = "grey value"),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, value| {
        Value::Layer(ctx.hir.layer(Layer::Grey { value }))
    }
}

builtin! {
    name = "rgb",
    signature = single {
        args(
            r: Scalar = "red channel",
            g: Scalar = "green channel",
            b: Scalar = "blue channel"
        ),
        result = ColorField,
        caps = PURE,
    },
    check = |_ctx, r, g, b| {
        Value::ColorField {
            rgba: [r, g, b, Sx::Lit(1.0)],
            space: ColorSpace::Srgb,
        }
    }
}

builtin! {
    name = "rgba",
    signature = single {
        args(
            r: Scalar = "red channel",
            g: Scalar = "green channel",
            b: Scalar = "blue channel",
            a: Scalar = "alpha channel"
        ),
        result = ColorField,
        caps = PURE,
    },
    check = |_ctx, r, g, b, a| {
        Value::ColorField {
            rgba: [r, g, b, a],
            space: ColorSpace::Srgb,
        }
    }
}
