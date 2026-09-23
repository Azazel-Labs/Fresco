//! Path receiver builtins (phase-1 typed stubs).

use crate::check::{Checker, Value};
use crate::registry::{
    BuiltinArgDecl, BuiltinDecl, BuiltinId, BuiltinImplReceiverFn, BuiltinLowering,
    BuiltinSignature, PrimitiveType, TypeRef,
};

pub fn point_at_path_check_impl(
    ctx: &mut Checker,
    bag: &mut crate::check::ArgBag<'_>,
) -> Option<Value> {
    let Some(path) = bag.path_receiver().cloned() else {
        return Some(Value::Error);
    };
    let s_expr = bag.require("s", &mut ctx.diags)?;
    let s_val = ctx.eval(s_expr)?;
    let Some((s_sx, _kind)) = Checker::as_numeric_scalar(&s_val) else {
        if !matches!(s_val, Value::Error) {
            ctx.diags.push(
                crate::diag::Diag::error(
                    s_expr.span.clone(),
                    format!(
                        "argument `s` to `point_at` expected scalar, found {}",
                        s_val.kind()
                    ),
                )
                .with_label("type mismatch in path evaluator call"),
            );
        }
        return Some(Value::Error);
    };

    ctx.record_path_eval_demand(path.profile_id(), "point_at");
    Some(Value::Vec2((
        crate::hir::Sx::PathPointAtComponent {
            path_id: path.profile_id(),
            s: Box::new(s_sx.clone()),
            component: 0,
        },
        crate::hir::Sx::PathPointAtComponent {
            path_id: path.profile_id(),
            s: Box::new(s_sx),
            component: 1,
        },
    )))
}

pub fn tangent_at_path_check_impl(
    ctx: &mut Checker,
    bag: &mut crate::check::ArgBag<'_>,
) -> Option<Value> {
    let Some(path) = bag.path_receiver().cloned() else {
        return Some(Value::Error);
    };
    let s_expr = bag.require("s", &mut ctx.diags)?;
    let s_val = ctx.eval(s_expr)?;
    if !matches!(s_val, Value::Error) && Checker::as_numeric_scalar(&s_val).is_none() {
        ctx.diags.push(
            crate::diag::Diag::error(
                s_expr.span.clone(),
                format!(
                    "argument `s` to `tangent_at` expected scalar, found {}",
                    s_val.kind()
                ),
            )
            .with_label("type mismatch in path evaluator call"),
        );
        return Some(Value::Error);
    }

    let Some((s_sx, _kind)) = Checker::as_numeric_scalar(&s_val) else {
        return Some(Value::Error);
    };
    ctx.record_path_eval_demand(path.profile_id(), "tangent_at");
    Some(Value::Vec2((
        crate::hir::Sx::PathTangentAtComponent {
            path_id: path.profile_id(),
            s: Box::new(s_sx.clone()),
            component: 0,
        },
        crate::hir::Sx::PathTangentAtComponent {
            path_id: path.profile_id(),
            s: Box::new(s_sx),
            component: 1,
        },
    )))
}

#[doc(hidden)]
inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("point_at_path"),
        name: "point_at",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: Some(TypeRef::Primitive(PrimitiveType::Path)),
            args: &[
                BuiltinArgDecl::required_with_role(
                    "s",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "path parameter",
                    "arc-length parameter",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Vec2),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::VEC2_OUT.bits()
                    | crate::builtin_catalog::BuiltinCaps::PURE.bits(),
            ),
        },
        lowering: BuiltinLowering::ImplReceiver(BuiltinImplReceiverFn::path(point_at_path_check_impl)),
        docs: "Return the point at an arc-length parameter on a path.",
    }
}

#[doc(hidden)]
inventory::submit! {
    BuiltinDecl {
        id: BuiltinId("tangent_at_path"),
        name: "tangent_at",
        discriminator: None,
        signature: BuiltinSignature {
            receiver: Some(TypeRef::Primitive(PrimitiveType::Path)),
            args: &[
                BuiltinArgDecl::required_with_role(
                    "s",
                    TypeRef::Primitive(PrimitiveType::Scalar),
                    "path parameter",
                    "arc-length parameter",
                ),
            ],
            result: TypeRef::Primitive(PrimitiveType::Vec2),
            result_alternatives: &[],
            caps: crate::builtin_catalog::BuiltinCaps::from_bits_retain(
                crate::builtin_catalog::BuiltinCaps::VEC2_OUT.bits()
                    | crate::builtin_catalog::BuiltinCaps::PURE.bits(),
            ),
        },
        lowering: BuiltinLowering::ImplReceiver(BuiltinImplReceiverFn::path(tangent_at_path_check_impl)),
        docs: "Return the tangent at an arc-length parameter on a path.",
    }
}
