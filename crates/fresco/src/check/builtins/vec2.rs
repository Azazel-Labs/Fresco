//! Vec2 geometry builtins.

use crate::builtin;
use crate::check::Value;
use crate::hir::{Sx, SxVec};

fn vec2_dot(a: (Sx, Sx), b: (Sx, Sx)) -> Sx {
    Sx::Dot {
        a: SxVec::V2(Box::new(a)),
        b: SxVec::V2(Box::new(b)),
    }
}

fn vec3_dot(a: (Sx, Sx, Sx), b: (Sx, Sx, Sx)) -> Sx {
    Sx::Dot {
        a: SxVec::V3(Box::new(a)),
        b: SxVec::V3(Box::new(b)),
    }
}

fn vec4_dot(a: (Sx, Sx, Sx, Sx), b: (Sx, Sx, Sx, Sx)) -> Sx {
    Sx::Dot {
        a: SxVec::V4(Box::new(a)),
        b: SxVec::V4(Box::new(b)),
    }
}

fn vec2_normalize(v: (Sx, Sx)) -> (Sx, Sx) {
    let sv = SxVec::V2(Box::new(v));
    (
        Sx::NormalizeComponent {
            v: sv.clone(),
            index: 0,
        },
        Sx::NormalizeComponent { v: sv, index: 1 },
    )
}

fn vec3_normalize(v: (Sx, Sx, Sx)) -> (Sx, Sx, Sx) {
    let sv = SxVec::V3(Box::new(v));
    (
        Sx::NormalizeComponent {
            v: sv.clone(),
            index: 0,
        },
        Sx::NormalizeComponent {
            v: sv.clone(),
            index: 1,
        },
        Sx::NormalizeComponent { v: sv, index: 2 },
    )
}

fn vec4_normalize(v: (Sx, Sx, Sx, Sx)) -> (Sx, Sx, Sx, Sx) {
    let sv = SxVec::V4(Box::new(v));
    (
        Sx::NormalizeComponent {
            v: sv.clone(),
            index: 0,
        },
        Sx::NormalizeComponent {
            v: sv.clone(),
            index: 1,
        },
        Sx::NormalizeComponent {
            v: sv.clone(),
            index: 2,
        },
        Sx::NormalizeComponent { v: sv, index: 3 },
    )
}

fn vec2_reflect(i: (Sx, Sx), n: (Sx, Sx)) -> (Sx, Sx) {
    let (ix, iy) = i;
    let (nx, ny) = n;
    let dot_in = vec2_dot((ix.clone(), iy.clone()), (nx.clone(), ny.clone()));
    let scale = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(dot_in));
    (
        Sx::Sub(
            Box::new(ix),
            Box::new(Sx::Mul(Box::new(scale.clone()), Box::new(nx))),
        ),
        Sx::Sub(
            Box::new(iy),
            Box::new(Sx::Mul(Box::new(scale), Box::new(ny))),
        ),
    )
}

fn vec3_reflect(i: (Sx, Sx, Sx), n: (Sx, Sx, Sx)) -> (Sx, Sx, Sx) {
    let (ix, iy, iz) = i;
    let (nx, ny, nz) = n;
    let dot_in = vec3_dot(
        (ix.clone(), iy.clone(), iz.clone()),
        (nx.clone(), ny.clone(), nz.clone()),
    );
    let scale = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(dot_in));
    (
        Sx::Sub(
            Box::new(ix),
            Box::new(Sx::Mul(Box::new(scale.clone()), Box::new(nx))),
        ),
        Sx::Sub(
            Box::new(iy),
            Box::new(Sx::Mul(Box::new(scale.clone()), Box::new(ny))),
        ),
        Sx::Sub(
            Box::new(iz),
            Box::new(Sx::Mul(Box::new(scale), Box::new(nz))),
        ),
    )
}

fn vec4_reflect(i: (Sx, Sx, Sx, Sx), n: (Sx, Sx, Sx, Sx)) -> (Sx, Sx, Sx, Sx) {
    let (ix, iy, iz, iw) = i;
    let (nx, ny, nz, nw) = n;
    let dot_in = vec4_dot(
        (ix.clone(), iy.clone(), iz.clone(), iw.clone()),
        (nx.clone(), ny.clone(), nz.clone(), nw.clone()),
    );
    let scale = Sx::Mul(Box::new(Sx::Lit(2.0)), Box::new(dot_in));
    (
        Sx::Sub(
            Box::new(ix),
            Box::new(Sx::Mul(Box::new(scale.clone()), Box::new(nx))),
        ),
        Sx::Sub(
            Box::new(iy),
            Box::new(Sx::Mul(Box::new(scale.clone()), Box::new(ny))),
        ),
        Sx::Sub(
            Box::new(iz),
            Box::new(Sx::Mul(Box::new(scale.clone()), Box::new(nz))),
        ),
        Sx::Sub(
            Box::new(iw),
            Box::new(Sx::Mul(Box::new(scale), Box::new(nw))),
        ),
    )
}

fn refract_coeff(dot_in: Sx, eta: Sx) -> (Sx, Sx) {
    let one = Sx::Lit(1.0);
    let dot_in2 = Sx::Mul(Box::new(dot_in.clone()), Box::new(dot_in.clone()));
    let eta2 = Sx::Mul(Box::new(eta.clone()), Box::new(eta.clone()));
    let term = Sx::Sub(Box::new(one.clone()), Box::new(dot_in2));
    let k = Sx::Sub(
        Box::new(one),
        Box::new(Sx::Mul(Box::new(eta2), Box::new(term))),
    );
    let sqrt_k = Sx::Sqrt(Box::new(k.clone()));
    let coeff = Sx::Add(
        Box::new(Sx::Mul(Box::new(eta), Box::new(dot_in))),
        Box::new(sqrt_k),
    );
    (coeff, k)
}

fn refract_mask(k: Sx) -> Sx {
    Sx::Step(Box::new(Sx::Lit(0.0)), Box::new(k))
}

builtin! {
    name = "length",
    signature = single {
        args(v: Expr = "vector (vec2, vec3, or vec4)"),
        result = Scalar,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, v| {
        let vv = ctx.eval(v).unwrap_or(Value::Error);
        match vv {
            Value::Vec2(v2) => Value::Scalar(Sx::Length(SxVec::V2(Box::new(v2)))),
            Value::Vec3(v3) => Value::Scalar(Sx::Length(SxVec::V3(Box::new(v3)))),
            Value::Vec4(v4) => Value::Scalar(Sx::Length(SxVec::V4(Box::new(v4)))),
            Value::Error => Value::Error,
            other => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        v.span.clone(),
                        format!("`length` expects vec2, vec3, or vec4, found {}", other.kind()),
                    )
                    .with_help("use `length((x, y))`, `length((x, y, z))`, or `length((x, y, z, w))`"),
                );
                Value::Error
            }
        }
    }
}

builtin! {
    name = "dot",
    signature = single {
        args(
            a: Expr = "first vector (vec2, vec3, or vec4)",
            b: Expr = "second vector (matching size)"
        ),
        result = Scalar,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, a, b| {
        let av = ctx.eval(a).unwrap_or(Value::Error);
        let bv = ctx.eval(b).unwrap_or(Value::Error);
        match (av, bv) {
            (Value::Vec2(a2), Value::Vec2(b2)) => Value::Scalar(vec2_dot(a2, b2)),
            (Value::Vec3(a3), Value::Vec3(b3)) => Value::Scalar(vec3_dot(a3, b3)),
            (Value::Vec4(a4), Value::Vec4(b4)) => Value::Scalar(vec4_dot(a4, b4)),
            (Value::Error, _) | (_, Value::Error) => Value::Error,
            (lhs, rhs) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        a.span.clone(),
                        format!(
                            "`dot` expects matching vec2/vec3/vec4 inputs, found {}/{}",
                            lhs.kind(),
                            rhs.kind(),
                        ),
                    )
                    .with_help("use `dot(v2, v2)`, `dot(v3, v3)`, or `dot(v4, v4)`"),
                );
                Value::Error
            }
        }
    }
}

builtin! {
    name = "normalize",
    signature = single {
        args(v: Expr = "vector (vec2, vec3, or vec4)"),
        result = Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, v| {
        let vv = ctx.eval(v).unwrap_or(Value::Error);
        match vv {
            Value::Vec2(v2) => Value::Vec2(vec2_normalize(v2)),
            Value::Vec3(v3) => Value::Vec3(vec3_normalize(v3)),
            Value::Vec4(v4) => Value::Vec4(vec4_normalize(v4)),
            Value::Error => Value::Error,
            other => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        v.span.clone(),
                        format!("`normalize` expects vec2, vec3, or vec4, found {}", other.kind()),
                    )
                    .with_help("use `normalize(v2)`, `normalize(v3)`, or `normalize(v4)`"),
                );
                Value::Error
            }
        }
    }
}

builtin! {
    name = "distance",
    signature = single {
        args(
            a: Expr = "first point (vec2, vec3, or vec4)",
            b: Expr = "second point (matching size)"
        ),
        result = Scalar,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, a, b| {
        let av = ctx.eval(a).unwrap_or(Value::Error);
        let bv = ctx.eval(b).unwrap_or(Value::Error);
        match (av, bv) {
            (Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => {
                let dx = Sx::Sub(Box::new(ax), Box::new(bx));
                let dy = Sx::Sub(Box::new(ay), Box::new(by));
                Value::Scalar(Sx::Sqrt(Box::new(vec2_dot(
                    (dx.clone(), dy.clone()),
                    (dx, dy),
                ))))
            }
            (Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => {
                let dx = Sx::Sub(Box::new(ax), Box::new(bx));
                let dy = Sx::Sub(Box::new(ay), Box::new(by));
                let dz = Sx::Sub(Box::new(az), Box::new(bz));
                Value::Scalar(Sx::Sqrt(Box::new(vec3_dot(
                    (dx.clone(), dy.clone(), dz.clone()),
                    (dx, dy, dz),
                ))))
            }
            (Value::Vec4((ax, ay, az, aw)), Value::Vec4((bx, by, bz, bw))) => {
                let dx = Sx::Sub(Box::new(ax), Box::new(bx));
                let dy = Sx::Sub(Box::new(ay), Box::new(by));
                let dz = Sx::Sub(Box::new(az), Box::new(bz));
                let dw = Sx::Sub(Box::new(aw), Box::new(bw));
                Value::Scalar(Sx::Sqrt(Box::new(vec4_dot(
                    (dx.clone(), dy.clone(), dz.clone(), dw.clone()),
                    (dx, dy, dz, dw),
                ))))
            }
            (Value::Error, _) | (_, Value::Error) => Value::Error,
            (lhs, rhs) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        a.span.clone(),
                        format!(
                            "`distance` expects matching vec2/vec3/vec4 inputs, found {}/{}",
                            lhs.kind(),
                            rhs.kind(),
                        ),
                    )
                    .with_help("use `distance(v2, v2)`, `distance(v3, v3)`, or `distance(v4, v4)`"),
                );
                Value::Error
            }
        }
    }
}

builtin! {
    name = "cross",
    signature = single {
        args(
            a: Expr = "first vector (vec2 or vec3)",
            b: Expr = "second vector (matching size)"
        ),
        result = Scalar | Vec3,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, a, b| {
        let av = ctx.eval(a).unwrap_or(Value::Error);
        let bv = ctx.eval(b).unwrap_or(Value::Error);
        match (av, bv) {
            (Value::Vec2((ax, ay)), Value::Vec2((bx, by))) => {
                let lhs = Sx::Mul(Box::new(ax), Box::new(by));
                let rhs = Sx::Mul(Box::new(ay), Box::new(bx));
                Value::Scalar(Sx::Sub(Box::new(lhs), Box::new(rhs)))
            }
            (Value::Vec3((ax, ay, az)), Value::Vec3((bx, by, bz))) => {
                let cx = Sx::Sub(
                    Box::new(Sx::Mul(Box::new(ay.clone()), Box::new(bz.clone()))),
                    Box::new(Sx::Mul(Box::new(az.clone()), Box::new(by.clone()))),
                );
                let cy = Sx::Sub(
                    Box::new(Sx::Mul(Box::new(az), Box::new(bx.clone()))),
                    Box::new(Sx::Mul(Box::new(ax.clone()), Box::new(bz))),
                );
                let cz = Sx::Sub(
                    Box::new(Sx::Mul(Box::new(ax), Box::new(by))),
                    Box::new(Sx::Mul(Box::new(ay), Box::new(bx))),
                );
                Value::Vec3((cx, cy, cz))
            }
            (Value::Error, _) | (_, Value::Error) => Value::Error,
            (lhs, rhs) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        a.span.clone(),
                        format!(
                            "`cross` expects matching vec2/vec3 inputs, found {}/{}",
                            lhs.kind(),
                            rhs.kind(),
                        ),
                    )
                    .with_help("use `cross(v2, v2)` or `cross(v3, v3)`"),
                );
                Value::Error
            }
        }
    }
}

builtin! {
    name = "reflect",
    signature = single {
        args(
            i: Expr = "incident vector (vec2, vec3, or vec4)",
            n: Expr = "normal vector (matching size)"
        ),
        result = Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, i, n| {
        let iv = ctx.eval(i).unwrap_or(Value::Error);
        let nv = ctx.eval(n).unwrap_or(Value::Error);
        match (iv, nv) {
            (Value::Vec2(i2), Value::Vec2(n2)) => Value::Vec2(vec2_reflect(i2, n2)),
            (Value::Vec3(i3), Value::Vec3(n3)) => Value::Vec3(vec3_reflect(i3, n3)),
            (Value::Vec4(i4), Value::Vec4(n4)) => Value::Vec4(vec4_reflect(i4, n4)),
            (Value::Error, _) | (_, Value::Error) => Value::Error,
            (lhs, rhs) => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        i.span.clone(),
                        format!(
                            "`reflect` expects matching vec2/vec3/vec4 inputs, found {}/{}",
                            lhs.kind(),
                            rhs.kind(),
                        ),
                    )
                    .with_help("use `reflect(v2, v2)`, `reflect(v3, v3)`, or `reflect(v4, v4)`"),
                );
                Value::Error
            }
        }
    }
}

builtin! {
    name = "refract",
    signature = single {
        args(
            i: Expr = "incident vector (vec2, vec3, or vec4)",
            n: Expr = "normal vector (matching size)",
            eta: Expr = "ratio of indices"
        ),
        result = Vec2 | Vec3 | Vec4,
        caps = PURE | PIPEABLE,
    },
    check = |ctx, i, n, eta| {
        let iv = ctx.eval(i).unwrap_or(Value::Error);
        let nv = ctx.eval(n).unwrap_or(Value::Error);
        let etav = ctx.eval(eta).unwrap_or(Value::Error);

        if let Some((eta_s, _)) = crate::check::Checker::as_numeric_scalar(&etav) {
            match (iv, nv) {
                (Value::Vec2((ix, iy)), Value::Vec2((nx, ny))) => {
                    let dot_in = vec2_dot((ix.clone(), iy.clone()), (nx.clone(), ny.clone()));
                    let (coeff, k) = refract_coeff(dot_in, eta_s.clone());
                    let out_x = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s.clone()), Box::new(ix))),
                            Box::new(Sx::Mul(Box::new(coeff.clone()), Box::new(nx))),
                        )),
                        Box::new(refract_mask(k.clone())),
                    );
                    let out_y = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s), Box::new(iy))),
                            Box::new(Sx::Mul(Box::new(coeff), Box::new(ny))),
                        )),
                        Box::new(refract_mask(k)),
                    );
                    Value::Vec2((out_x, out_y))
                }
                (Value::Vec3((ix, iy, iz)), Value::Vec3((nx, ny, nz))) => {
                    let dot_in = vec3_dot(
                        (ix.clone(), iy.clone(), iz.clone()),
                        (nx.clone(), ny.clone(), nz.clone()),
                    );
                    let (coeff, k) = refract_coeff(dot_in, eta_s.clone());
                    let m = refract_mask(k);
                    let out_x = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s.clone()), Box::new(ix))),
                            Box::new(Sx::Mul(Box::new(coeff.clone()), Box::new(nx))),
                        )),
                        Box::new(m.clone()),
                    );
                    let out_y = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s.clone()), Box::new(iy))),
                            Box::new(Sx::Mul(Box::new(coeff.clone()), Box::new(ny))),
                        )),
                        Box::new(m.clone()),
                    );
                    let out_z = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s), Box::new(iz))),
                            Box::new(Sx::Mul(Box::new(coeff), Box::new(nz))),
                        )),
                        Box::new(m),
                    );
                    Value::Vec3((out_x, out_y, out_z))
                }
                (Value::Vec4((ix, iy, iz, iw)), Value::Vec4((nx, ny, nz, nw))) => {
                    let dot_in = vec4_dot(
                        (ix.clone(), iy.clone(), iz.clone(), iw.clone()),
                        (nx.clone(), ny.clone(), nz.clone(), nw.clone()),
                    );
                    let (coeff, k) = refract_coeff(dot_in, eta_s.clone());
                    let m = refract_mask(k);
                    let out_x = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s.clone()), Box::new(ix))),
                            Box::new(Sx::Mul(Box::new(coeff.clone()), Box::new(nx))),
                        )),
                        Box::new(m.clone()),
                    );
                    let out_y = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s.clone()), Box::new(iy))),
                            Box::new(Sx::Mul(Box::new(coeff.clone()), Box::new(ny))),
                        )),
                        Box::new(m.clone()),
                    );
                    let out_z = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s.clone()), Box::new(iz))),
                            Box::new(Sx::Mul(Box::new(coeff.clone()), Box::new(nz))),
                        )),
                        Box::new(m.clone()),
                    );
                    let out_w = Sx::Mul(
                        Box::new(Sx::Sub(
                            Box::new(Sx::Mul(Box::new(eta_s), Box::new(iw))),
                            Box::new(Sx::Mul(Box::new(coeff), Box::new(nw))),
                        )),
                        Box::new(m),
                    );
                    Value::Vec4((out_x, out_y, out_z, out_w))
                }
                (Value::Error, _) | (_, Value::Error) => Value::Error,
                (lhs, rhs) => {
                    ctx.diags.push(
                        crate::diag::Diag::error(
                            i.span.clone(),
                            format!(
                                "`refract` expects matching vec2/vec3/vec4 inputs, found {}/{}",
                                lhs.kind(),
                                rhs.kind(),
                            ),
                        )
                        .with_help("use `refract(v2, v2, eta)`, `refract(v3, v3, eta)`, or `refract(v4, v4, eta)`"),
                    );
                    Value::Error
                }
            }
        } else {
            if !matches!(etav, Value::Error) {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        eta.span.clone(),
                        format!("`refract` eta must be scalar, found {}", etav.kind()),
                    )
                    .with_help("use `refract(i, n, 1.0 / 1.33)`"),
                );
            }
            Value::Error
        }
    }
}
