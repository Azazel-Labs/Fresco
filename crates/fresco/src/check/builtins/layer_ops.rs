//! Layer manipulation builtins (receiver-based).

use crate::builtin;
use crate::check::{ArgBag, Checker, Value};
use crate::diag::Diag;
use crate::hir::{ColorSource, GlowColorSpace, GlowFalloff, Layer, Sx};
use crate::registry::{
    BuiltinArgDecl, BuiltinDecl, BuiltinId, BuiltinImplReceiverFn, BuiltinLowering,
    BuiltinSignature, PrimitiveType, TypeRef,
};

builtin! {
    name = "blur",
    signature = single {
        receiver = Layer,
        args(radius: Scalar = "blur radius"),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, radius| {
        Value::Layer(ctx.hir.layer(Layer::Blur {
            inner: _receiver,
            radius,
            span: bag.call_span().clone(),
        }))
    }
}

builtin! {
    name = "motion_blur",
    signature = single {
        receiver = Layer,
        args(
            shutter: Scalar = "motion blur shutter amount",
            velocity: Optional<Vec2> = "deprecated: removed in favor of temporal sampling",
            offset: Optional<Vec2> = "deprecated: removed in favor of temporal sampling"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, shutter, velocity, offset| {
        if velocity.is_some() || offset.is_some() {
            ctx.diags.push(
                Diag::error(
                    bag.call_span().clone(),
                    "`motion_blur` no longer accepts `velocity:` or `offset:`",
                )
                .with_help("use shutter-only temporal blur: `motion_blur(shutter: 8ms)`"),
            );
            return None;
        }

        ctx.hir
            .notes
            .push("effect: motion_blur lowered as temporal stochastic loop x5".to_string());

        Value::Layer(ctx.hir.layer(Layer::MotionBlur {
            inner: _receiver,
            shutter,
            offset: (Sx::Lit(0.0), Sx::Lit(0.0)),
            span: bag.call_span().clone(),
        }))
    }
}

pub fn blur_shape_receiver_error_check_impl(
    ctx: &mut Checker,
    _shape_id: crate::hir::ShapeId,
    bag: &mut ArgBag<'_>,
) -> Option<Value> {
    ctx.diags.push(
        Diag::error(
            bag.call_span().clone(),
            "`blur` applies to layers, not bare shapes",
        )
        .with_help("fill it first — `shape |> fill(...) |> blur(8px)` — or use `soften`, `dilate`, or `erode` on the shape directly"),
    );
    None
}

#[doc(hidden)]
inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("blur_shape_receiver_error"),
        name: "blur",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: Some(TypeRef::Primitive(PrimitiveType::Shape)),
            args: &[
                BuiltinArgDecl::required(
                    "radius",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "blur radius",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Layer),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits() | crate::builtin_catalog::BuiltinCaps::LAYER_OUT.bits()
            ),
        },
        lowering: BuiltinLowering::ImplReceiver(
            BuiltinImplReceiverFn::shape(blur_shape_receiver_error_check_impl),
        ),
        docs: "Blur shape receiver error overload",
    }
}

builtin! {
    name = "opacity",
    signature = single {
        receiver = Layer,
        args(alpha: Scalar = "opacity alpha multiplier"),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, alpha| {
        Value::Layer(ctx.hir.layer(Layer::Opacity {
            inner: _receiver,
            alpha,
        }))
    }
}

builtin! {
    name = "mask",
    signature = single {
        receiver = Layer,
        args(mask: Scalar = "alpha mask multiplier"),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, mask| {
        let alpha = Sx::Clamp(
            Box::new(mask),
            Box::new(Sx::Lit(0.0)),
            Box::new(Sx::Lit(1.0)),
        );
        Value::Layer(ctx.hir.layer(Layer::Opacity {
            inner: _receiver,
            alpha,
        }))
    }
}

builtin! {
    name = "tint",
    signature = single {
        receiver = Layer,
        args(
            color: Color = "tint color",
            amount: Optional<Scalar> = "tint blend amount"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, color, amount| {
        Value::Layer(ctx.hir.layer(Layer::Tint {
            inner: _receiver,
            color,
            amount: amount.unwrap_or(Sx::Lit(1.0)),
        }))
    }
}

pub fn postprocess_layer_check_impl(
    ctx: &mut Checker,
    recv: crate::hir::LayerId,
    bag: &mut ArgBag<'_>,
) -> Option<Value> {
    let fn_expr = bag
        .take_named("fn")
        .or_else(|| bag.take("fn"))
        .or_else(|| bag.take("effect"));
    let Some(fn_expr) = fn_expr else {
        ctx.diags.push(
            Diag::error(
                bag.call_span().clone(),
                "`postprocess(...)` expects one function identifier",
            )
            .with_help(
                "use `compose { ... } |> postprocess(musicviz_finish)` or `|> postprocess(fn: musicviz_finish)`",
            ),
        );
        return None;
    };

    let crate::ast::Expr::Var(fn_name) = &fn_expr.node else {
        ctx.diags.push(
            Diag::error(
                fn_expr.span.clone(),
                "`postprocess(...)` expects a function identifier",
            )
            .with_help("pass a declared function name such as `postprocess(musicviz_finish)`"),
        );
        return None;
    };

    let rgba = ctx.eval_postprocess_fn_ref(fn_name, &fn_expr.span, bag.call_span())?;
    Some(Value::Layer(
        ctx.hir.layer(Layer::PostProcess { inner: recv, rgba }),
    ))
}

inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("postprocess_layer_receiver"),
        name: "postprocess",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: Some(TypeRef::Primitive(PrimitiveType::Layer)),
            args: &[
                BuiltinArgDecl::required(
                    "fn",
                    TypeRef::Primitive(PrimitiveType::Layer),
                    "postprocess function identifier",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Layer),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::PURE.bits() | crate::builtin_catalog::BuiltinCaps::LAYER_OUT.bits()
            ),
        },
        lowering: BuiltinLowering::ImplReceiver(BuiltinImplReceiverFn::layer(postprocess_layer_check_impl)),
        docs: "Apply a point-local postprocess function to the composed output of a layer stack",
    }
}

builtin! {
    name = "glow",
    signature = single {
        receiver = Layer,
        args(
            reach: Expr = "glow reach distance (scalar or vec2)",
            strength: Scalar = "glow strength multiplier",
            color: Optional<ColorExpr> = "glow color or gradient (default: white)",
            falloff: Optional<Expr> = "glow falloff (`exp`, `gaussian`, or `linear`)"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, reach, strength, color, falloff| {
        let reach = ctx.as_glow_reach(reach)?;
        let color = match color {
            Some(expr) => ctx.as_color_source(expr)?,
            None => ColorSource::Solid([
                Sx::Lit(1.0),
                Sx::Lit(1.0),
                Sx::Lit(1.0),
                Sx::Lit(1.0),
            ]),
        };
        let falloff = match falloff {
            Some(expr) => ctx.parse_glow_falloff(expr)?,
            None => GlowFalloff::Exp,
        };
        ctx.hir.notes.push(
            "effect: glow(layer) lowered as single-pass layer glow approximation".to_string(),
        );
        Value::Layer(ctx.hir.layer(Layer::GlowFx {
            inner: _receiver,
            reach,
            strength,
            color,
            falloff,
        }))
    }
}

builtin! {
    name = "soften",
    signature = single {
        receiver = Layer,
        args(
            radius: Scalar = "soften radius",
            color: Optional<Color> = "soften color override"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, radius, color| {
        let override_color = color;
        match ctx.hir.layers[_receiver].clone() {
            Layer::Fill { shape, color } => {
                let extent = Checker::try_eval_static_scalar(&radius).map(f32::abs);
                ctx.note_wide_effect_if_needed_with_extent(shape, "soften", bag.call_span(), extent);
                Value::Layer(ctx.hir.layer(Layer::Soften {
                    shape,
                    radius,
                    color: override_color.unwrap_or_else(|| {
                        [
                            Sx::Lit(color[0]),
                            Sx::Lit(color[1]),
                            Sx::Lit(color[2]),
                            Sx::Lit(color[3]),
                        ]
                    }),
                }))
            }
            Layer::FillExpr { shape, r, g, b, a } => {
                let extent = Checker::try_eval_static_scalar(&radius).map(f32::abs);
                ctx.note_wide_effect_if_needed_with_extent(shape, "soften", bag.call_span(), extent);
                Value::Layer(ctx.hir.layer(Layer::Soften {
                    shape,
                    radius,
                    color: override_color.unwrap_or([r, g, b, a]),
                }))
            }
            Layer::FillGradient { shape, kind, stops } => {
                let extent = Checker::try_eval_static_scalar(&radius).map(f32::abs);
                ctx.note_wide_effect_if_needed_with_extent(shape, "soften", bag.call_span(), extent);
                Value::Layer(ctx.hir.layer(Layer::Soften {
                    shape, radius,
                    color: override_color.unwrap_or_else(|| crate::hir::GradientSample::channels(kind, stops)),
                }))
            }
            _ => {
                ctx.diags.push(
                    Diag::error(
                        bag.call_span().clone(),
                        "`soften` currently applies to shapes or direct shape fills",
                    )
                    .with_help("use `shape |> soften(radius: ...)` or `shape |> fill(...) |> soften(radius: ...)`"),
                );
                return None;
            }
        }
    }
}

builtin! {
    name = "inner_glow",
    signature = single {
        receiver = Layer,
        args(
            reach: Expr = "inner glow reach distance (scalar or vec2)",
            strength: Scalar = "inner glow strength multiplier",
            color: Optional<Color> = "inner glow color override"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, reach, strength, color| {
        let reach = ctx.as_glow_reach(reach)?;
        let override_color = color;
        match ctx.hir.layers[_receiver].clone() {
            Layer::Fill { shape, color } => {
                let extent = Checker::static_glow_reach_extent(&reach);
                ctx.note_wide_effect_if_needed_with_extent(shape, "inner_glow", bag.call_span(), extent);
                Value::Layer(ctx.hir.layer(Layer::InnerGlow {
                    shape,
                    reach,
                    strength,
                    color: ColorSource::Solid(override_color.unwrap_or_else(|| {
                        [
                            Sx::Lit(color[0]),
                            Sx::Lit(color[1]),
                            Sx::Lit(color[2]),
                            Sx::Lit(color[3]),
                        ]
                    })),
                    falloff: GlowFalloff::Exp,
                    color_space: GlowColorSpace::Scene,
                }))
            }
            Layer::FillExpr { shape, r, g, b, a } => {
                let extent = Checker::static_glow_reach_extent(&reach);
                ctx.note_wide_effect_if_needed_with_extent(shape, "inner_glow", bag.call_span(), extent);
                Value::Layer(ctx.hir.layer(Layer::InnerGlow {
                    shape,
                    reach,
                    strength,
                    color: ColorSource::Solid(override_color.unwrap_or([r, g, b, a])),
                    falloff: GlowFalloff::Exp,
                    color_space: GlowColorSpace::Scene,
                }))
            }
            Layer::FillGradient { shape, kind, stops } => {
                let extent = Checker::static_glow_reach_extent(&reach);
                ctx.note_wide_effect_if_needed_with_extent(shape, "inner_glow", bag.call_span(), extent);
                Value::Layer(ctx.hir.layer(Layer::InnerGlow {
                    shape, reach, strength,
                    color: override_color.map(ColorSource::Solid).unwrap_or(ColorSource::Gradient { kind, stops }),
                    falloff: GlowFalloff::Exp,
                    color_space: GlowColorSpace::Scene,
                }))
            }
            _ => {
                ctx.diags.push(
                    Diag::error(
                        bag.call_span().clone(),
                        "`inner_glow` currently applies to shapes or direct shape fills",
                    )
                    .with_help("use `shape |> inner_glow(...)` or `shape |> fill(...) |> inner_glow(...)`"),
                );
                return None;
            }
        }
    }
}

builtin! {
    name = "bevel",
    signature = single {
        receiver = Layer,
        args(
            width: Optional<Scalar> = "bevel width (default: 2px)",
            light: Optional<Vec2> = "light direction (default: (1, -1))",
            strength: Optional<Scalar> = "bevel strength (default: 1.0)",
            highlight: Optional<Color> = "highlight color",
            shadow: Optional<Color> = "shadow color"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, width, light, strength, highlight, shadow| {
        let width = width.unwrap_or(Sx::PxLit(2.0));
        let light = light.unwrap_or((Sx::Lit(1.0), Sx::Lit(-1.0)));
        let strength = strength.unwrap_or(Sx::Lit(1.0));
        let highlight = highlight.unwrap_or([
            Sx::Lit(1.0),
            Sx::Lit(1.0),
            Sx::Lit(1.0),
            Sx::Lit(0.7),
        ]);
        let shadow = shadow.unwrap_or([
            Sx::Lit(0.0),
            Sx::Lit(0.0),
            Sx::Lit(0.0),
            Sx::Lit(0.6),
        ]);

        match ctx.hir.layers[_receiver].clone() {
            Layer::Fill { shape, .. } | Layer::FillExpr { shape, .. } | Layer::FillGradient { shape, .. } => {
                let extent = Checker::try_eval_static_scalar(&width).map(f32::abs);
                ctx.note_wide_effect_if_needed_with_extent(shape, "bevel", bag.call_span(), extent);
                Value::Layer(ctx.hir.layer(Layer::Bevel {
                    shape,
                    width,
                    light,
                    strength,
                    highlight,
                    shadow: Box::new(shadow),
                }))
            }
            _ => {
                ctx.diags.push(
                    Diag::error(
                        bag.call_span().clone(),
                        "`bevel` currently applies to shapes or direct shape fills",
                    )
                    .with_help("use `shape |> bevel(...)` or `shape |> fill(...) |> bevel(...)`"),
                );
                return None;
            }
        }
    }
}
