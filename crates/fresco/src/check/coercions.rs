use super::*;
use web_time::Instant;

impl Checker {
    fn anchor_variant_vec2(name: &str) -> Option<V2> {
        anchor_variant_coords(name).map(|(x, y)| (Sx::Lit(x), Sx::Lit(y)))
    }

    fn resolve_anchor_literal_vec2(&mut self, e: &SExpr) -> Result<Option<V2>, ()> {
        let Expr::Var(name) = &e.node else {
            return Ok(None);
        };

        if let Some((enum_name, variant_name)) = name.split_once('.') {
            if enum_name != "Anchor" {
                return Ok(None);
            }
            if variant_name.contains('.') {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("invalid enum variant path `{name}`"),
                    )
                    .with_help("use `Anchor.<variant>`, for example `Anchor.center`"),
                );
                return Err(());
            }

            let Some(anchor_def) = self.enum_defs.get("Anchor") else {
                self.diags.push(
                    Diag::error(e.span.clone(), "enum `Anchor` is not available")
                        .with_help("ensure stdlib enums are loaded before using anchor literals"),
                );
                return Err(());
            };

            if !anchor_def.variants.contains_key(variant_name) {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("unknown anchor variant `{variant_name}`"),
                    )
                    .with_help("supported anchor variants: center, top_center, bottom_center, left_center, right_center, top_left, top_right, bottom_left, bottom_right"),
                );
                return Err(());
            }

            let Some(v) = Self::anchor_variant_vec2(variant_name) else {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("anchor variant `{variant_name}` cannot be used as a coordinate"),
                    )
                    .with_help("use one of the built-in positional anchor variants"),
                );
                return Err(());
            };
            return Ok(Some(v));
        }

        if !Self::is_anchor_variant_name(name) {
            return Ok(None);
        }

        let owners: Vec<&str> = self
            .enum_defs
            .iter()
            .filter(|(enum_name, _)| crate::registry::enum_allows_unqualified_values(enum_name))
            .filter_map(|(enum_name, enum_def)| {
                enum_def
                    .variants
                    .contains_key(name)
                    .then_some(enum_name.as_str())
            })
            .collect();

        if owners.is_empty() {
            return Ok(None);
        }

        if owners.len() > 1 {
            self.diags.push(
                Diag::error(e.span.clone(), format!("ambiguous enum variant `{name}`")).with_help(
                    format!(
                        "qualify the variant: {}",
                        owners
                            .iter()
                            .map(|owner| format!("`{owner}.{name}`"))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    ),
                ),
            );
            return Err(());
        }

        if owners[0] != "Anchor" {
            return Ok(None);
        }

        let Some(v) = Self::anchor_variant_vec2(name) else {
            self.diags.push(
                Diag::error(
                    e.span.clone(),
                    format!("anchor variant `{name}` cannot be used as a coordinate"),
                )
                .with_help("use one of the built-in positional anchor variants"),
            );
            return Err(());
        };
        Ok(Some(v))
    }

    pub(super) fn as_color_expr(&mut self, e: &SExpr) -> Option<[Sx; 4]> {
        let t0 = Instant::now();
        let out = match self.eval(e)? {
            Value::Error => None,
            Value::Color { rgba, .. } => Some([
                Sx::Lit(rgba[0]),
                Sx::Lit(rgba[1]),
                Sx::Lit(rgba[2]),
                Sx::Lit(rgba[3]),
            ]),
            Value::ColorField { rgba, .. } => Some(rgba),
            Value::Gradient { kind, stops } => Some(hir::GradientSample::channels(kind, stops)),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!(
                        "expected a color expression (literal, rgb, rgba, or color arithmetic), found a {}",
                        v.kind()
                    ),
                ));
                None
            }
        };
        self.record_check_profile_timing("as_color_expr", t0.elapsed());
        out
    }

    pub(super) fn as_scalar(&mut self, e: &SExpr) -> Option<Sx> {
        let t0 = Instant::now();
        let out = match self.eval(e)? {
            Value::Error => None,
            Value::Scalar(s) => Some(s),
            Value::Distance(s) => Some(s),
            Value::Coverage(s) => Some(s),
            Value::Mask(s) => Some(s),
            v => {
                self.diags.push(
                    Diag::error(e.span.clone(), format!("expected a scalar, found a {}", v.kind()))
                        .with_help("scalars: numbers with optional units (0.5, 12px, 20deg, 2s), time, arithmetic thereof"),
                );
                None
            }
        };
        self.record_check_profile_timing("as_scalar", t0.elapsed());
        out
    }

    pub(super) fn as_vec2(&mut self, e: &SExpr) -> Option<V2> {
        let t0 = Instant::now();
        match self.resolve_anchor_literal_vec2(e) {
            Ok(Some(v)) => {
                self.record_check_profile_timing("as_vec2", t0.elapsed());
                return Some(v);
            }
            Ok(None) => {}
            Err(()) => {
                self.record_check_profile_timing("as_vec2", t0.elapsed());
                return None;
            }
        }

        let out = match self.eval(e)? {
            Value::Error => None,
            Value::Vec2(v) => Some(v),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!("expected a vec2 like `(x, y)`, found a {}", v.kind()),
                ));
                None
            }
        };
        self.record_check_profile_timing("as_vec2", t0.elapsed());
        out
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub(super) fn as_vec3(&mut self, e: &SExpr) -> Option<(Sx, Sx, Sx)> {
        let t0 = Instant::now();
        let out = match self.eval(e)? {
            Value::Error => None,
            Value::Vec3(v) => Some(v),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!("expected a vec3 like `(x, y, z)`, found a {}", v.kind()),
                ));
                None
            }
        };
        self.record_check_profile_timing("as_vec3", t0.elapsed());
        out
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub(super) fn as_vec4(&mut self, e: &SExpr) -> Option<(Sx, Sx, Sx, Sx)> {
        let t0 = Instant::now();
        let out = match self.eval(e)? {
            Value::Error => None,
            Value::Vec4(v) => Some(v),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!("expected a vec4 like `(x, y, z, w)`, found a {}", v.kind()),
                ));
                None
            }
        };
        self.record_check_profile_timing("as_vec4", t0.elapsed());
        out
    }

    pub(super) fn as_mat2(&mut self, e: &SExpr) -> Option<(V2, V2)> {
        match self.eval(e)? {
            Value::Error => None,
            Value::Mat2(m) => Some(m),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!("expected a mat2, found a {}", v.kind()),
                ));
                None
            }
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub(super) fn as_mat3(&mut self, e: &SExpr) -> Option<Mat3Value> {
        match self.eval(e)? {
            Value::Error => None,
            Value::Mat3(m) => Some(*m),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!("expected a mat3, found a {}", v.kind()),
                ));
                None
            }
        }
    }

    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    pub(super) fn as_mat4(&mut self, e: &SExpr) -> Option<Mat4Value> {
        match self.eval(e)? {
            Value::Error => None,
            Value::Mat4(m) => Some(*m),
            v => {
                self.diags.push(Diag::error(
                    e.span.clone(),
                    format!("expected a mat4, found a {}", v.kind()),
                ));
                None
            }
        }
    }

    pub(super) fn as_fill_style(&mut self, e: &SExpr) -> Option<FillStyle> {
        let t0 = Instant::now();
        let out = match self.eval(e)? {
            Value::Error => None,
            Value::Color { rgba, .. } => Some(FillStyle::Solid(rgba)),
            Value::ColorField { rgba, .. } => Some(FillStyle::Dynamic(rgba)),
            Value::Gradient { kind, stops } => Some(FillStyle::Gradient { kind, stops }),
            v => {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("expected a color or gradient, found a {}", v.kind()),
                    )
                    .with_help("use `#rrggbb` colors or `gradient(kind: linear|radial, stops: [stop(...) ...])`"),
                );
                None
            }
        };
        self.record_check_profile_timing("as_fill_style", t0.elapsed());
        out
    }

    pub(super) fn as_color_source(&mut self, e: &SExpr) -> Option<ColorSource> {
        match self.as_fill_style(e)? {
            FillStyle::Solid(color) => Some(ColorSource::Solid([
                Sx::Lit(color[0]),
                Sx::Lit(color[1]),
                Sx::Lit(color[2]),
                Sx::Lit(color[3]),
            ])),
            FillStyle::Dynamic(rgba) => Some(ColorSource::Solid(rgba)),
            FillStyle::Gradient { kind, stops } => {
                if matches!(
                    kind,
                    GradientKind::Linear {
                        anchor: GradientAnchor::Shape,
                        ..
                    }
                ) {
                    self.diags.push(
                        Diag::error(
                            e.span.clone(),
                            "`anchor: shape` is only supported for shape fills",
                        )
                        .with_help("apply the gradient through `some_shape |> fill(gradient(..., anchor: shape, ...))`")
                    );
                    None
                } else {
                    Some(ColorSource::Gradient { kind, stops })
                }
            }
        }
    }

    pub(super) fn as_glow_reach(&mut self, e: &SExpr) -> Option<GlowReach> {
        match self.eval(e)? {
            Value::Scalar(sx) => Some(GlowReach::Scalar(sx)),
            Value::Vec2(v) => Some(GlowReach::Vec2(v)),
            v => {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!(
                            "expected glow reach to be a scalar or vec2, found a {}",
                            v.kind()
                        ),
                    )
                    .with_help("use `reach: 12px` or `reach: (24px, 8px)`"),
                );
                None
            }
        }
    }

    pub(super) fn static_glow_reach_extent(reach: &GlowReach) -> Option<f32> {
        match reach {
            GlowReach::Scalar(sx) => Checker::try_eval_static_scalar(sx).map(f32::abs),
            GlowReach::Vec2((x, y)) => {
                let x = Checker::try_eval_static_scalar(x)?.abs();
                let y = Checker::try_eval_static_scalar(y)?.abs();
                Some(x.max(y))
            }
        }
    }
    pub(super) fn parse_glow_falloff(&mut self, e: &SExpr) -> Option<GlowFalloff> {
        let Expr::Var(name) = &e.node else {
            self.diags.push(
                Diag::error(
                    e.span.clone(),
                    "`falloff:` expects `exp`, `gaussian`, or `linear`",
                )
                .with_help("example: `glow(..., falloff: gaussian)`"),
            );
            return None;
        };

        let symbol = name.rsplit('.').next().unwrap_or(name.as_str());
        match symbol {
            "exp" => Some(GlowFalloff::Exp),
            "gaussian" => Some(GlowFalloff::Gaussian),
            "linear" => Some(GlowFalloff::Linear),
            _ => {
                self.diags.push(
                    Diag::error(e.span.clone(), format!("unknown glow falloff `{symbol}`"))
                        .with_help("use `exp`, `gaussian`, or `linear`"),
                );
                None
            }
        }
    }

    pub(super) fn parse_glow_color_space(&mut self, e: &SExpr) -> Option<GlowColorSpace> {
        let Expr::Var(name) = &e.node else {
            self.diags.push(
                Diag::error(
                    e.span.clone(),
                    "`color_space:` expects `scene`, `shape`, or `glow`",
                )
                .with_help("example: `glow(..., color_space: glow)`"),
            );
            return None;
        };

        let symbol = name.rsplit('.').next().unwrap_or(name.as_str());
        match symbol {
            "scene" => Some(GlowColorSpace::Scene),
            "shape" => Some(GlowColorSpace::Shape),
            "glow" => Some(GlowColorSpace::Glow),
            _ => {
                self.diags.push(
                    Diag::error(
                        e.span.clone(),
                        format!("unknown glow color_space `{symbol}`"),
                    )
                    .with_help("use `scene`, `shape`, or `glow`"),
                );
                None
            }
        }
    }
}
