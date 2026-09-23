//! Shape manipulation builtins (receiver-based).

use crate::builtin;
use crate::check::{Checker, Value};
use crate::hir::{Shape, Sx};

builtin! {
    name = "round",
    signature = single {
        receiver = Shape,
        args(radius: Scalar = "corner radius"),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, radius| {
        match ctx.hir.shapes[_receiver].clone() {
            Shape::RBox { center, half, .. } => {
                Value::Shape(ctx.hir.shape(Shape::RBox {
                    center,
                    half,
                    round: radius,
                }))
            }
            _ => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        crate::ast::Span::default(),
                        "`round` applies only to box shapes",
                    )
                    .with_help("circles are already round; for other shapes use `dilate` (roadmap)"),
                );
                return None;
            }
        }
    }
}

builtin! {
    name = "dilate",
    signature = single {
        receiver = Shape,
        args(radius: Scalar = "dilation radius"),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, radius| {
        let out = ctx.hir.shape(Shape::Offset {
            inner: _receiver,
            delta: radius,
        });
        ctx.note_shape_exactness(out, "dilate");
        Value::Shape(out)
    }
}

builtin! {
    name = "erode",
    signature = single {
        receiver = Shape,
        args(radius: Scalar = "erosion radius"),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, radius| {
        let out = ctx.hir.shape(Shape::Offset {
            inner: _receiver,
            delta: Sx::Neg(Box::new(radius)),
        });
        ctx.note_shape_exactness(out, "erode");
        Value::Shape(out)
    }
}

builtin! {
    name = "smooth",
    signature = single {
        receiver = Shape,
        args(radius: Scalar = "smoothing radius"),
        result = Shape,
        caps = PURE,
    },
    check = |ctx, radius| {
        if let Some(v) = Checker::try_eval_static_scalar(&radius)
            && v <= 0.0
        {
            ctx.diags.push(
                crate::diag::Diag::error(
                    crate::ast::Span::default(),
                    "`smooth` radius must be > 0",
                )
                .with_help("use a positive smoothing radius, for example `smooth(radius: 0.05uv)`"),
            );
            return None;
        }
        match ctx.hir.shapes[_receiver] {
            Shape::Union(a, b) => {
                let out = ctx.hir.shape(Shape::SmoothUnion(a, b, radius));
                ctx.note_shape_exactness(out, "smooth");
                Value::Shape(out)
            }
            _ => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        crate::ast::Span::default(),
                        "`smooth` modifies a union",
                    )
                    .with_label("this shape is not a `|` union")
                    .with_help("write it as `(a | b) |> smooth(radius: 0.05uv)`"),
                );
                return None;
            }
        }
    }
}
