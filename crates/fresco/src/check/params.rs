use super::*;

impl Checker {
    pub(super) fn lookup(&self, name: &str) -> Option<Value> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name))
            .cloned()
            .or_else(|| (name == "coord").then_some(Value::Vec2((Sx::CoordX, Sx::CoordY))))
    }

    pub(super) fn lookup_style(&self, name: &str) -> Option<StyleDef> {
        self.style_scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name))
            .cloned()
    }

    pub(super) fn bind(&mut self, name: String, v: Value) {
        self.scopes.last_mut().unwrap().insert(name, v);
    }

    pub(super) fn assign(&mut self, name: &str, v: Value) -> bool {
        for (index, scope) in self.scopes.iter_mut().enumerate().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), v);
                for writes in &mut self.assignment_scopes {
                    writes.insert((index, name.to_string()));
                }
                return true;
            }
        }
        false
    }

    pub(super) fn bind_style(&mut self, name: String, def: StyleDef, name_span: Span) {
        let scope = self
            .style_scopes
            .last_mut()
            .expect("style scope is always present");
        if scope.insert(name.clone(), def).is_some() {
            self.diags.push(
                Diag::error(name_span, format!("duplicate style declaration `{name}`"))
                    .with_help("rename the style or remove the earlier declaration in this scope"),
            );
        }
    }

    pub(super) fn unit_family(unit: Unit) -> &'static str {
        match unit {
            Unit::None => "unitless",
            Unit::Px | Unit::Uv | Unit::Vw | Unit::Vh | Unit::Vmin | Unit::Vmax => "length",
            Unit::Deg | Unit::Turn => "angle",
            Unit::Sec | Unit::MilliSec => "time",
        }
    }

    pub(super) fn literal_num_unit(expr: &SExpr) -> Option<Unit> {
        match expr.node {
            Expr::Num(_, unit) => Some(unit),
            _ => None,
        }
    }

    pub(super) fn validate_param_literal_units(
        &mut self,
        name: &str,
        name_span: &Span,
        ty_name: &str,
        default: &SExpr,
        range: Option<&(SExpr, SExpr)>,
    ) -> bool {
        let mut literals: Vec<(&str, Span, Unit)> = Vec::new();
        if let Some(u) = Self::literal_num_unit(default) {
            literals.push(("default", default.span.clone(), u));
        }
        if let Some((min, max)) = range {
            if let Some(u) = Self::literal_num_unit(min) {
                literals.push(("range minimum", min.span.clone(), u));
            }
            if let Some(u) = Self::literal_num_unit(max) {
                literals.push(("range maximum", max.span.clone(), u));
            }
        }

        match ty_name {
            "i32" | "u32" => {
                for (label, span, unit) in literals {
                    if unit != Unit::None {
                        self.diags.push(
                            Diag::error(
                                span,
                                format!("param `{name}` {label} must be unitless for `{ty_name}`"),
                            )
                            .with_label("remove the unit suffix")
                            .with_help("integer params accept plain integer literals only"),
                        );
                        return false;
                    }
                }
            }
            "f32" | "f64" | "half" if literals.len() >= 2 => {
                let base_family = Self::unit_family(literals[0].2);
                let mismatch = literals
                    .iter()
                    .find(|(_, _, u)| Self::unit_family(*u) != base_family);
                if let Some((label, _span, unit)) = mismatch {
                    self.diags.push(
                        Diag::error(
                            name_span.clone(),
                            format!(
                                "param `{name}` mixes incompatible unit families in default/range"
                            ),
                        )
                        .with_label(format!(
                            "{label} uses {} units here",
                            Self::unit_family(*unit)
                        ))
                        .with_help(format!(
                            "use one unit family consistently (current baseline: {base_family})"
                        )),
                    );
                    return false;
                }
            }
            _ => {}
        }

        true
    }

    pub(super) fn declare_param(
        &mut self,
        name: &str,
        name_span: &Span,
        ty_name: &str,
        default: &SExpr,
        range: Option<&(SExpr, SExpr)>,
    ) {
        use crate::hir::{ArrayElemType, ArrayParam, ArrayParamSize};

        // Check if this is a dynamic array (array<T> without length)
        if let Some((elem_ty_str, ArrayParamSize::Dynamic)) =
            hir::parse_array_param_type_ex(ty_name)
        {
            let Some(elem_type) = ArrayElemType::from_str(elem_ty_str) else {
                self.diags.push(
                    Diag::error(
                        name_span.clone(),
                        format!(
                            "param `{name}` uses unsupported array element type `{elem_ty_str}`"
                        ),
                    )
                    .with_help("supported types: f32, i32, u32, bool, vec2, vec3, vec4, mat2, mat3, mat4, color"),
                );
                return;
            };

            // Dynamic arrays require an array literal as default.
            // Non-empty literals are treated as preview/UI hints for the playground;
            // the actual data is always supplied at runtime via a storage buffer.
            let Expr::Array(items) = &default.node else {
                self.diags.push(
                    Diag::error(
                        default.span.clone(),
                        format!("dynamic array param `{name}` default must be an array literal"),
                    )
                    .with_help(
                        "use `[]` for no initial values, or `[0.5, 0.5]` to set preview hints",
                    ),
                );
                return;
            };

            let mut values = Vec::with_capacity(items.len());

            // Parse literal elements as preview hint values.
            if !items.is_empty() {
                match elem_type {
                    ArrayElemType::F32 => {
                        for item in items.iter() {
                            let Some(item_sx) = self.as_scalar(item) else {
                                return;
                            };
                            let Some(value) = self.require_const_param_scalar(
                                &item_sx,
                                "element",
                                item.span.clone(),
                            ) else {
                                return;
                            };
                            values.push(hir::ArrayElemValue::F32(value));
                        }
                    }
                    ArrayElemType::I32 => {
                        for item in items.iter() {
                            let Some(item_sx) =
                                self.param_integer_expression(item, crate::typed_scalar::Kind::I32)
                            else {
                                return;
                            };
                            let Some(value) = self.require_const_param_i32(
                                &item_sx,
                                "element",
                                item.span.clone(),
                            ) else {
                                return;
                            };
                            values.push(hir::ArrayElemValue::I32(value));
                        }
                    }
                    ArrayElemType::U32 => {
                        for item in items.iter() {
                            let Some(item_sx) =
                                self.param_integer_expression(item, crate::typed_scalar::Kind::U32)
                            else {
                                return;
                            };
                            let Some(value) = self.require_const_param_u32(
                                &item_sx,
                                "element",
                                item.span.clone(),
                            ) else {
                                return;
                            };
                            values.push(hir::ArrayElemValue::U32(value));
                        }
                    }
                    ArrayElemType::Bool => {
                        for item in items.iter() {
                            let Some(value) = self.require_const_param_bool(item) else {
                                return;
                            };
                            values.push(hir::ArrayElemValue::Bool(value));
                        }
                    }
                    ArrayElemType::Vec2 => {
                        for item in items.iter() {
                            let Some(v2) = self.as_vec2(item) else {
                                return;
                            };
                            let Some((x, y)) =
                                self.require_const_param_vec2(&v2, "element", item.span.clone())
                            else {
                                return;
                            };
                            values.push(hir::ArrayElemValue::Vec2((x, y)));
                        }
                    }
                    ArrayElemType::Vec3 => {
                        for item in items.iter() {
                            let Some(v3) = self.as_vec3(item) else {
                                return;
                            };
                            let Some((x, y, z)) =
                                self.require_const_param_vec3(&v3, "element", item.span.clone())
                            else {
                                return;
                            };
                            values.push(hir::ArrayElemValue::Vec3((x, y, z)));
                        }
                    }
                    ArrayElemType::Vec4 => {
                        for item in items.iter() {
                            let Some(v4) = self.as_vec4(item) else {
                                return;
                            };
                            let Some((x, y, z, w)) =
                                self.require_const_param_vec4(&v4, "element", item.span.clone())
                            else {
                                return;
                            };
                            values.push(hir::ArrayElemValue::Vec4((x, y, z, w)));
                        }
                    }
                    _ => {
                        self.diags.push(
                            Diag::error(
                                default.span.clone(),
                                format!(
                                    "preview-hint defaults for `array<{elem_ty_str}>` are not yet supported"
                                ),
                            )
                            .with_help("use `[]` as the default; specify values at runtime"),
                        );
                        return;
                    }
                }
            }

            // Store the ArrayParam (values may be empty or contain preview hints)
            let arr_param = ArrayParam { elem_type, values };

            self.hir.params.push(Param {
                name: name.to_string(),
                ty_name: ty_name.to_string(),
                default: ParamDefault::Array(arr_param),
                min: None,
                max: None,
            });

            // Bind the dynamic array in scope so it can be indexed at runtime.
            // Only types that have storage-buffer support are indexable.
            let is_indexable = matches!(
                elem_type,
                ArrayElemType::F32
                    | ArrayElemType::I32
                    | ArrayElemType::U32
                    | ArrayElemType::Bool
                    | ArrayElemType::Vec2
                    | ArrayElemType::Vec3
                    | ArrayElemType::Vec4
            );
            if is_indexable {
                self.bind(
                    name.to_string(),
                    Value::DynamicArray {
                        param_name: name.to_string(),
                        elem_type,
                    },
                );
            }
            return;
        }

        if let Some((elem_ty_str, len)) = hir::parse_array_param_type(ty_name) {
            use crate::hir::{ArrayElemType, ArrayElemValue, ArrayParam};

            let Some(elem_type) = ArrayElemType::from_str(elem_ty_str) else {
                self.diags.push(
                    Diag::error(
                        name_span.clone(),
                        format!(
                            "param `{name}` uses unsupported array element type `{elem_ty_str}`"
                        ),
                    )
                    .with_help("supported types: f32, i32, u32, bool, vec2, vec3, vec4, mat2, mat3, mat4, color"),
                );
                return;
            };

            // Validate array literal exists
            let Expr::Array(items) = &default.node else {
                self.diags.push(
                    Diag::error(
                        default.span.clone(),
                        format!("param `{name}` default must be an array literal"),
                    )
                    .with_help("use a literal array like `[0.1, 0.2, 0.3]`"),
                );
                return;
            };

            // Validate length
            if items.len() != len {
                self.diags.push(
                    Diag::error(
                        default.span.clone(),
                        format!(
                            "param `{name}` expected {len} elements, found {}",
                            items.len()
                        ),
                    )
                    .with_help("match the array length declared in the param type"),
                );
                return;
            }

            // Parse elements based on type
            let mut values = Vec::with_capacity(len);
            let mut runtime_values = Vec::with_capacity(len);

            match elem_type {
                ArrayElemType::F32 => {
                    if !self.validate_param_literal_units(name, name_span, "f32", default, range) {
                        return;
                    }

                    for (index, item) in items.iter().enumerate() {
                        let Some(item_sx) = self.as_scalar(item) else {
                            return;
                        };
                        let Some(value) =
                            self.require_const_param_scalar(&item_sx, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::F32(value));
                        runtime_values.push(Value::Scalar(Sx::Param(format!("{name}__{index}"))));
                    }
                }

                ArrayElemType::I32 => {
                    if !self.validate_param_literal_units(name, name_span, "i32", default, range) {
                        return;
                    }

                    for (index, item) in items.iter().enumerate() {
                        let Some(item_sx) =
                            self.param_integer_expression(item, crate::typed_scalar::Kind::I32)
                        else {
                            return;
                        };
                        let Some(value) =
                            self.require_const_param_i32(&item_sx, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::I32(value));
                        // Decode the existing float transport at the typed expression boundary.
                        runtime_values.push(Value::Scalar(crate::typed_scalar::Scalar::cast(
                            Sx::Param(format!("{name}__{index}")),
                            crate::typed_scalar::Kind::I32,
                        )));
                    }
                }

                ArrayElemType::U32 => {
                    if !self.validate_param_literal_units(name, name_span, "u32", default, range) {
                        return;
                    }

                    for (index, item) in items.iter().enumerate() {
                        let Some(item_sx) =
                            self.param_integer_expression(item, crate::typed_scalar::Kind::U32)
                        else {
                            return;
                        };
                        let Some(value) =
                            self.require_const_param_u32(&item_sx, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::U32(value));
                        // Decode the existing float transport at the typed expression boundary.
                        runtime_values.push(Value::Scalar(crate::typed_scalar::Scalar::cast(
                            Sx::Param(format!("{name}__{index}")),
                            crate::typed_scalar::Kind::U32,
                        )));
                    }
                }

                ArrayElemType::Bool => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(value) = self.require_const_param_bool(item) else {
                            return;
                        };

                        values.push(ArrayElemValue::Bool(value));
                        // Decode the existing float transport at the typed expression boundary.
                        runtime_values.push(Value::Scalar(crate::typed_scalar::Scalar::cast(
                            Sx::Param(format!("{name}__{index}")),
                            crate::typed_scalar::Kind::Bool,
                        )));
                    }
                }

                ArrayElemType::Vec2 => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(v2) = self.as_vec2(item) else {
                            return;
                        };
                        let Some((x, y)) =
                            self.require_const_param_vec2(&v2, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::Vec2((x, y)));
                        runtime_values.push(Value::Vec2((
                            Sx::Param(format!("{name}__{index}__x")),
                            Sx::Param(format!("{name}__{index}__y")),
                        )));
                    }
                }

                ArrayElemType::Vec3 => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(v3) = self.as_vec3(item) else {
                            return;
                        };
                        let Some((x, y, z)) =
                            self.require_const_param_vec3(&v3, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::Vec3((x, y, z)));
                        runtime_values.push(Value::Vec3((
                            Sx::Param(format!("{name}__{index}__x")),
                            Sx::Param(format!("{name}__{index}__y")),
                            Sx::Param(format!("{name}__{index}__z")),
                        )));
                    }
                }

                ArrayElemType::Vec4 => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(v4) = self.as_vec4(item) else {
                            return;
                        };
                        let Some((x, y, z, w)) =
                            self.require_const_param_vec4(&v4, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::Vec4((x, y, z, w)));
                        runtime_values.push(Value::Vec4((
                            Sx::Param(format!("{name}__{index}__x")),
                            Sx::Param(format!("{name}__{index}__y")),
                            Sx::Param(format!("{name}__{index}__z")),
                            Sx::Param(format!("{name}__{index}__w")),
                        )));
                    }
                }

                ArrayElemType::Mat2 => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(mat) = self.as_mat2(item) else {
                            return;
                        };
                        // Convert (V2, V2) to [[Sx; 2]; 2]
                        let mat_array = [
                            [mat.0.0.clone(), mat.0.1.clone()],
                            [mat.1.0.clone(), mat.1.1.clone()],
                        ];
                        let Some(values_mat) =
                            self.require_const_param_mat2(&mat_array, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::Mat2(values_mat));
                        runtime_values.push(Value::Mat2((
                            (
                                Sx::Param(format!("{name}__{index}__0_0")),
                                Sx::Param(format!("{name}__{index}__0_1")),
                            ),
                            (
                                Sx::Param(format!("{name}__{index}__1_0")),
                                Sx::Param(format!("{name}__{index}__1_1")),
                            ),
                        )));
                    }
                }

                ArrayElemType::Mat3 => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(mat) = self.as_mat3(item) else {
                            return;
                        };
                        // Convert Mat3Value to [[Sx; 3]; 3]
                        let mat_array = [
                            [mat.0.0.clone(), mat.0.1.clone(), mat.0.2.clone()],
                            [mat.1.0.clone(), mat.1.1.clone(), mat.1.2.clone()],
                            [mat.2.0.clone(), mat.2.1.clone(), mat.2.2.clone()],
                        ];
                        let Some(values_mat) =
                            self.require_const_param_mat3(&mat_array, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::Mat3(values_mat));
                        runtime_values.push(Value::Mat3(Box::new((
                            (
                                Sx::Param(format!("{name}__{index}__0_0")),
                                Sx::Param(format!("{name}__{index}__0_1")),
                                Sx::Param(format!("{name}__{index}__0_2")),
                            ),
                            (
                                Sx::Param(format!("{name}__{index}__1_0")),
                                Sx::Param(format!("{name}__{index}__1_1")),
                                Sx::Param(format!("{name}__{index}__1_2")),
                            ),
                            (
                                Sx::Param(format!("{name}__{index}__2_0")),
                                Sx::Param(format!("{name}__{index}__2_1")),
                                Sx::Param(format!("{name}__{index}__2_2")),
                            ),
                        ))));
                    }
                }

                ArrayElemType::Mat4 => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(mat) = self.as_mat4(item) else {
                            return;
                        };
                        // Convert Mat4Value to [[Sx; 4]; 4]
                        let mat_array = [
                            [
                                mat.0.0.clone(),
                                mat.0.1.clone(),
                                mat.0.2.clone(),
                                mat.0.3.clone(),
                            ],
                            [
                                mat.1.0.clone(),
                                mat.1.1.clone(),
                                mat.1.2.clone(),
                                mat.1.3.clone(),
                            ],
                            [
                                mat.2.0.clone(),
                                mat.2.1.clone(),
                                mat.2.2.clone(),
                                mat.2.3.clone(),
                            ],
                            [
                                mat.3.0.clone(),
                                mat.3.1.clone(),
                                mat.3.2.clone(),
                                mat.3.3.clone(),
                            ],
                        ];
                        let Some(values_mat) =
                            self.require_const_param_mat4(&mat_array, "element", item.span.clone())
                        else {
                            return;
                        };

                        values.push(ArrayElemValue::Mat4(values_mat));
                        runtime_values.push(Value::Mat4(Box::new((
                            (
                                Sx::Param(format!("{name}__{index}__0_0")),
                                Sx::Param(format!("{name}__{index}__0_1")),
                                Sx::Param(format!("{name}__{index}__0_2")),
                                Sx::Param(format!("{name}__{index}__0_3")),
                            ),
                            (
                                Sx::Param(format!("{name}__{index}__1_0")),
                                Sx::Param(format!("{name}__{index}__1_1")),
                                Sx::Param(format!("{name}__{index}__1_2")),
                                Sx::Param(format!("{name}__{index}__1_3")),
                            ),
                            (
                                Sx::Param(format!("{name}__{index}__2_0")),
                                Sx::Param(format!("{name}__{index}__2_1")),
                                Sx::Param(format!("{name}__{index}__2_2")),
                                Sx::Param(format!("{name}__{index}__2_3")),
                            ),
                            (
                                Sx::Param(format!("{name}__{index}__3_0")),
                                Sx::Param(format!("{name}__{index}__3_1")),
                                Sx::Param(format!("{name}__{index}__3_2")),
                                Sx::Param(format!("{name}__{index}__3_3")),
                            ),
                        ))));
                    }
                }

                ArrayElemType::Color => {
                    for (index, item) in items.iter().enumerate() {
                        let Some(rgba) = self.require_const_param_color(item) else {
                            return;
                        };

                        values.push(ArrayElemValue::Color(rgba));
                        // Store color as a runtime value similar to single color params
                        runtime_values.push(Value::ColorField {
                            rgba: [
                                Sx::Param(format!("{name}__{index}.r")),
                                Sx::Param(format!("{name}__{index}.g")),
                                Sx::Param(format!("{name}__{index}.b")),
                                Sx::Param(format!("{name}__{index}.a")),
                            ],
                            space: ColorSpace::Linear,
                        });
                    }
                }
            }

            // Range validation (optional, type-dependent)
            let (min_v, max_v) = match elem_type {
                ArrayElemType::F32 | ArrayElemType::I32 | ArrayElemType::U32 => match range {
                    Some((min, max)) => {
                        let Some(min_sx) = self.as_scalar(min) else {
                            return;
                        };
                        let Some(max_sx) = self.as_scalar(max) else {
                            return;
                        };

                        let Some(min_v) = self.require_const_param_scalar(
                            &min_sx,
                            "range minimum",
                            min.span.clone(),
                        ) else {
                            return;
                        };
                        let Some(max_v) = self.require_const_param_scalar(
                            &max_sx,
                            "range maximum",
                            max.span.clone(),
                        ) else {
                            return;
                        };

                        if min_v > max_v {
                            self.diags.push(
                                Diag::error(name_span.clone(), "param range is inverted")
                                    .with_help("write ranges as `min .. max` where `min <= max`")
                                    .with_label(format!("got {} .. {}", min_v, max_v)),
                            );
                            return;
                        }

                        (Some(min_v), Some(max_v))
                    }
                    None => (None, None),
                },
                ArrayElemType::Bool
                | ArrayElemType::Vec2
                | ArrayElemType::Vec3
                | ArrayElemType::Vec4
                | ArrayElemType::Mat2
                | ArrayElemType::Mat3
                | ArrayElemType::Mat4
                | ArrayElemType::Color => {
                    if range.is_some() {
                        self.diags.push(
                            Diag::error(
                                name_span.clone(),
                                format!(
                                    "param `{name}` of type `array<{}, {}>` does not support ranges",
                                    elem_ty_str, len
                                ),
                            )
                            .with_help(format!(
                                "remove the `in min .. max` clause for {} arrays",
                                elem_ty_str
                            )),
                        );
                        return;
                    }
                    (None, None)
                }
            };

            // Create param
            self.hir.params.push(Param {
                name: name.to_string(),
                ty_name: ty_name.to_string(),
                default: ParamDefault::Array(ArrayParam { elem_type, values }),
                min: min_v,
                max: max_v,
            });

            self.bind(name.to_string(), Value::Array(runtime_values));
            return;
        }

        match ty_name {
            "f32" | "f64" | "half" => {
                if !self.validate_param_literal_units(name, name_span, ty_name, default, range) {
                    return;
                }
                let Some(default_sx) = self.as_scalar(default) else {
                    return;
                };
                let Some(default_v) =
                    self.require_const_param_scalar(&default_sx, "default", default.span.clone())
                else {
                    return;
                };

                let (min_v, max_v) = match range {
                    Some((min, max)) => {
                        let Some(min_sx) = self.as_scalar(min) else {
                            return;
                        };
                        let Some(max_sx) = self.as_scalar(max) else {
                            return;
                        };

                        let Some(min_v) = self.require_const_param_scalar(
                            &min_sx,
                            "range minimum",
                            min.span.clone(),
                        ) else {
                            return;
                        };
                        let Some(max_v) = self.require_const_param_scalar(
                            &max_sx,
                            "range maximum",
                            max.span.clone(),
                        ) else {
                            return;
                        };

                        if min_v > max_v {
                            self.diags.push(
                                Diag::error(name_span.clone(), "param range is inverted")
                                    .with_help("write ranges as `min .. max` where `min <= max`")
                                    .with_label(format!("got {} .. {}", min_v, max_v)),
                            );
                            return;
                        }

                        (Some(min_v), Some(max_v))
                    }
                    None => (None, None),
                };

                self.hir.params.push(Param {
                    name: name.to_string(),
                    ty_name: ty_name.to_string(),
                    default: ParamDefault::Scalar(default_v),
                    min: min_v,
                    max: max_v,
                });
                self.bind(name.to_string(), Value::Scalar(Sx::Param(name.to_string())));
            }
            "i32" => {
                if !self.validate_param_literal_units(name, name_span, ty_name, default, range) {
                    return;
                }
                let Some(default_sx) =
                    self.param_integer_expression(default, crate::typed_scalar::Kind::I32)
                else {
                    return;
                };
                let Some(default_v) =
                    self.require_const_param_i32(&default_sx, "default", default.span.clone())
                else {
                    return;
                };

                let (min_v, max_v) = match range {
                    Some((min, max)) => {
                        let Some(min_sx) =
                            self.param_integer_expression(min, crate::typed_scalar::Kind::I32)
                        else {
                            return;
                        };
                        let Some(max_sx) =
                            self.param_integer_expression(max, crate::typed_scalar::Kind::I32)
                        else {
                            return;
                        };

                        let Some(min_v) = self.require_const_param_i32(
                            &min_sx,
                            "range minimum",
                            min.span.clone(),
                        ) else {
                            return;
                        };
                        let Some(max_v) = self.require_const_param_i32(
                            &max_sx,
                            "range maximum",
                            max.span.clone(),
                        ) else {
                            return;
                        };

                        if min_v > max_v {
                            self.diags.push(
                                Diag::error(name_span.clone(), "param range is inverted")
                                    .with_help("write ranges as `min .. max` where `min <= max`")
                                    .with_label(format!("got {} .. {}", min_v, max_v)),
                            );
                            return;
                        }

                        (Some(min_v as f32), Some(max_v as f32))
                    }
                    None => (None, None),
                };

                self.hir.params.push(Param {
                    name: name.to_string(),
                    ty_name: ty_name.to_string(),
                    default: ParamDefault::Int(default_v),
                    min: min_v,
                    max: max_v,
                });
                self.bind(
                    name.to_string(),
                    Value::Scalar(crate::typed_scalar::Scalar::cast(
                        Sx::Param(name.to_string()),
                        crate::typed_scalar::Kind::I32,
                    )),
                );
            }
            "u32" => {
                if !self.validate_param_literal_units(name, name_span, ty_name, default, range) {
                    return;
                }
                let Some(default_sx) =
                    self.param_integer_expression(default, crate::typed_scalar::Kind::U32)
                else {
                    return;
                };
                let Some(default_v) =
                    self.require_const_param_u32(&default_sx, "default", default.span.clone())
                else {
                    return;
                };

                let (min_v, max_v) = match range {
                    Some((min, max)) => {
                        let Some(min_sx) =
                            self.param_integer_expression(min, crate::typed_scalar::Kind::U32)
                        else {
                            return;
                        };
                        let Some(max_sx) =
                            self.param_integer_expression(max, crate::typed_scalar::Kind::U32)
                        else {
                            return;
                        };

                        let Some(min_v) = self.require_const_param_u32(
                            &min_sx,
                            "range minimum",
                            min.span.clone(),
                        ) else {
                            return;
                        };
                        let Some(max_v) = self.require_const_param_u32(
                            &max_sx,
                            "range maximum",
                            max.span.clone(),
                        ) else {
                            return;
                        };

                        if min_v > max_v {
                            self.diags.push(
                                Diag::error(name_span.clone(), "param range is inverted")
                                    .with_help("write ranges as `min .. max` where `min <= max`")
                                    .with_label(format!("got {} .. {}", min_v, max_v)),
                            );
                            return;
                        }

                        (Some(min_v as f32), Some(max_v as f32))
                    }
                    None => (None, None),
                };

                self.hir.params.push(Param {
                    name: name.to_string(),
                    ty_name: ty_name.to_string(),
                    default: ParamDefault::UInt(default_v),
                    min: min_v,
                    max: max_v,
                });
                self.bind(
                    name.to_string(),
                    Value::Scalar(crate::typed_scalar::Scalar::cast(
                        Sx::Param(name.to_string()),
                        crate::typed_scalar::Kind::U32,
                    )),
                );
            }
            "bool" => {
                if let Some((_min, _max)) = range {
                    self.diags.push(
                        Diag::error(name_span.clone(), "bool params do not support ranges")
                            .with_label("remove this `in min .. max` range")
                            .with_help("`param enabled: bool = true` exposes a toggle; ranges are numeric only"),
                    );
                    return;
                }

                let Some(default_bool) = self.require_const_param_bool(default) else {
                    return;
                };

                self.hir.params.push(Param {
                    name: name.to_string(),
                    ty_name: ty_name.to_string(),
                    default: ParamDefault::Bool(default_bool),
                    min: None,
                    max: None,
                });
                self.bind(
                    name.to_string(),
                    Value::Scalar(crate::typed_scalar::Scalar::cast(
                        Sx::Param(name.to_string()),
                        crate::typed_scalar::Kind::Bool,
                    )),
                );
            }
            "color" => {
                if let Some((min, max)) = range {
                    self.diags.push(
                        Diag::error(name_span.clone(), "color params do not support ranges")
                            .with_label("remove this `in min .. max` range")
                            .with_help(format!(
                                "`param {name}: color = ...` exposes a color directly; only scalar params support slider ranges (range written at spans {}..{} and {}..{})",
                                min.span.start, min.span.end, max.span.start, max.span.end
                            )),
                    );
                    return;
                }

                let Some(default_color) = self.require_const_param_color(default) else {
                    return;
                };

                self.hir.params.push(Param {
                    name: name.to_string(),
                    ty_name: ty_name.to_string(),
                    default: ParamDefault::Color(default_color),
                    min: None,
                    max: None,
                });
                self.bind(
                    name.to_string(),
                    Value::ColorField {
                        rgba: [
                            Sx::Param(format!("{name}.r")),
                            Sx::Param(format!("{name}.g")),
                            Sx::Param(format!("{name}.b")),
                            Sx::Param(format!("{name}.a")),
                        ],
                        space: ColorSpace::Linear,
                    },
                );
            }
            other => {
                // Check for typed texture: `texture<TypeName>`
                if let Some(type_name) = other
                    .strip_prefix("texture<")
                    .and_then(|s| s.strip_suffix('>'))
                {
                    // Validate the texture_type exists
                    if !self.hir.texture_type_defs.contains_key(type_name) {
                        self.diags.push(
                            Diag::error(
                                name_span.clone(),
                                format!("unknown texture_type `{type_name}` in param `{name}`"),
                            )
                            .with_help("declare a `texture_type` with this name before using it"),
                        );
                        return;
                    }
                    let default_asset = match &default.node {
                        Expr::Str(s) => s.clone(),
                        _ => {
                            self.diags.push(
                                Diag::error(
                                    default.span.clone(),
                                    format!("param `{name}` default must be a string (asset path)"),
                                )
                                .with_help(r#"example: param orm: texture<ORM> = "orm.png""#),
                            );
                            return;
                        }
                    };
                    self.hir.register_texture(name);
                    self.hir.set_texture_default_asset(name, default_asset);
                    if let Some(meta) = self.hir.texture_metadata.get_mut(name) {
                        meta.texture_type_name = Some(type_name.to_string());
                    }
                    let layer_id = self.hir.layer(Layer::Image {
                        tex_name: name.to_string(),
                    });
                    self.bind(
                        name.to_string(),
                        Value::TypedTextureSample {
                            layer_id,
                            tex_name: name.to_string(),
                            sample_at: None,
                        },
                    );
                    return;
                }
                // Check for plain `texture`
                if other == "texture" {
                    let default_asset = match &default.node {
                        Expr::Str(s) => s.clone(),
                        _ => {
                            self.diags.push(
                                Diag::error(
                                    default.span.clone(),
                                    format!("param `{name}` default must be a string (asset path)"),
                                )
                                .with_help(r#"example: param albedo: texture = "albedo.png""#),
                            );
                            return;
                        }
                    };
                    self.hir.register_texture(name);
                    self.hir.set_texture_default_asset(name, default_asset);
                    let layer_id = self.hir.layer(Layer::Image {
                        tex_name: name.to_string(),
                    });
                    self.bind(
                        name.to_string(),
                        Value::TypedTextureSample {
                            layer_id,
                            tex_name: name.to_string(),
                            sample_at: None,
                        },
                    );
                    return;
                }
                self.diags.push(
                    Diag::error(
                        name_span.clone(),
                        format!("unsupported param type `{other}`"),
                    )
                    .with_help("v0 supports `param` declarations with `f32`, `i32`, `u32`, `bool`, `color`, `texture`, and `texture<TypeName>`"),
                );
            }
        }
    }

    pub(super) fn require_const_param_scalar(
        &mut self,
        sx: &Sx,
        label: &str,
        span: Span,
    ) -> Option<f32> {
        match sx {
            Sx::Lit(v) | Sx::PxLit(v) => Some(*v),
            _ => {
                self.diags.push(
                    Diag::error(span, format!("param {label} must be a numeric literal"))
                        .with_help("v0 manifest output requires literal `default`, `min`, and `max` values"),
                );
                None
            }
        }
    }

    pub(super) fn require_const_param_color(&mut self, expr: &SExpr) -> Option<[f32; 4]> {
        let color = self.as_color_expr(expr)?;
        let r =
            self.require_const_param_scalar(&color[0], "default red channel", expr.span.clone())?;
        let g =
            self.require_const_param_scalar(&color[1], "default green channel", expr.span.clone())?;
        let b =
            self.require_const_param_scalar(&color[2], "default blue channel", expr.span.clone())?;
        let a =
            self.require_const_param_scalar(&color[3], "default alpha channel", expr.span.clone())?;
        Some([r, g, b, a])
    }

    fn param_integer_expression(
        &mut self,
        expression: &SExpr,
        kind: crate::typed_scalar::Kind,
    ) -> Option<Sx> {
        let value = self.eval_scalar_expected(expression, Some(kind))?;
        match value {
            Value::Scalar(value) => Some(value),
            other => {
                self.diags.push(Diag::error(
                    expression.span.clone(),
                    format!(
                        "integer parameter requires a scalar, found {}",
                        other.kind()
                    ),
                ));
                None
            }
        }
    }

    pub(super) fn require_const_param_i32(
        &mut self,
        sx: &Sx,
        label: &str,
        span: Span,
    ) -> Option<i32> {
        if let Sx::Typed(value) = sx {
            return match value.evaluate(&HashMap::new()) {
                Ok(naga::Literal::I32(value)) => Some(value),
                result => {
                    self.diags.push(Diag::error(
                        span,
                        format!("param {label} must be a compile-time i32 value: {result:?}"),
                    ));
                    None
                }
            };
        }
        let value = self.require_const_param_scalar(sx, label, span.clone())?;
        if value.fract() != 0.0 {
            self.diags.push(
                Diag::error(span, format!("param {label} must be an integer literal"))
                    .with_help("examples: -3, 0, 42"),
            );
            return None;
        }
        if value < i32::MIN as f32 || value > i32::MAX as f32 {
            self.diags.push(
                Diag::error(span, format!("param {label} is out of i32 range")).with_help(format!(
                    "expected {} .. {}",
                    i32::MIN,
                    i32::MAX
                )),
            );
            return None;
        }
        Some(value as i32)
    }

    pub(super) fn require_const_param_u32(
        &mut self,
        sx: &Sx,
        label: &str,
        span: Span,
    ) -> Option<u32> {
        if let Sx::Typed(value) = sx {
            return match value.evaluate(&HashMap::new()) {
                Ok(naga::Literal::U32(value)) => Some(value),
                result => {
                    self.diags.push(Diag::error(
                        span,
                        format!("param {label} must be a compile-time u32 value: {result:?}"),
                    ));
                    None
                }
            };
        }
        let value = self.require_const_param_scalar(sx, label, span.clone())?;
        if value.fract() != 0.0 {
            self.diags.push(
                Diag::error(span, format!("param {label} must be an integer literal"))
                    .with_help("examples: 0, 7, 128"),
            );
            return None;
        }
        if value < 0.0 || value > u32::MAX as f32 {
            self.diags.push(
                Diag::error(span, format!("param {label} is out of u32 range"))
                    .with_help(format!("expected 0 .. {}", u32::MAX)),
            );
            return None;
        }
        Some(value as u32)
    }

    pub(super) fn require_const_param_bool(&mut self, expr: &SExpr) -> Option<bool> {
        match &expr.node {
            Expr::Var(name) if name == "true" => Some(true),
            Expr::Var(name) if name == "false" => Some(false),
            _ => {
                self.diags.push(
                    Diag::error(expr.span.clone(), "param default must be a bool literal")
                        .with_help("use `true` or `false`"),
                );
                None
            }
        }
    }

    pub(super) fn require_const_param_vec2(
        &mut self,
        v2: &V2,
        label: &str,
        span: Span,
    ) -> Option<(f32, f32)> {
        let x = self.require_const_param_scalar(&v2.0, &format!("{label} x"), span.clone())?;
        let y = self.require_const_param_scalar(&v2.1, &format!("{label} y"), span)?;
        Some((x, y))
    }

    pub(super) fn require_const_param_vec3(
        &mut self,
        v3: &(Sx, Sx, Sx),
        label: &str,
        span: Span,
    ) -> Option<(f32, f32, f32)> {
        let x = self.require_const_param_scalar(&v3.0, &format!("{label} x"), span.clone())?;
        let y = self.require_const_param_scalar(&v3.1, &format!("{label} y"), span.clone())?;
        let z = self.require_const_param_scalar(&v3.2, &format!("{label} z"), span)?;
        Some((x, y, z))
    }

    pub(super) fn require_const_param_vec4(
        &mut self,
        v4: &(Sx, Sx, Sx, Sx),
        label: &str,
        span: Span,
    ) -> Option<(f32, f32, f32, f32)> {
        let x = self.require_const_param_scalar(&v4.0, &format!("{label} x"), span.clone())?;
        let y = self.require_const_param_scalar(&v4.1, &format!("{label} y"), span.clone())?;
        let z = self.require_const_param_scalar(&v4.2, &format!("{label} z"), span.clone())?;
        let w = self.require_const_param_scalar(&v4.3, &format!("{label} w"), span)?;
        Some((x, y, z, w))
    }

    pub(super) fn require_const_param_mat2(
        &mut self,
        mat: &[[Sx; 2]; 2],
        label: &str,
        span: Span,
    ) -> Option<[[f32; 2]; 2]> {
        let mut result = [[0.0; 2]; 2];
        for i in 0..2 {
            for j in 0..2 {
                result[i][j] = self.require_const_param_scalar(
                    &mat[i][j],
                    &format!("{label} [{i}][{j}]"),
                    span.clone(),
                )?;
            }
        }
        Some(result)
    }

    pub(super) fn require_const_param_mat3(
        &mut self,
        mat: &[[Sx; 3]; 3],
        label: &str,
        span: Span,
    ) -> Option<[[f32; 3]; 3]> {
        let mut result = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                result[i][j] = self.require_const_param_scalar(
                    &mat[i][j],
                    &format!("{label} [{i}][{j}]"),
                    span.clone(),
                )?;
            }
        }
        Some(result)
    }

    pub(super) fn require_const_param_mat4(
        &mut self,
        mat: &[[Sx; 4]; 4],
        label: &str,
        span: Span,
    ) -> Option<[[f32; 4]; 4]> {
        let mut result = [[0.0; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                result[i][j] = self.require_const_param_scalar(
                    &mat[i][j],
                    &format!("{label} [{i}][{j}]"),
                    span.clone(),
                )?;
            }
        }
        Some(result)
    }
}
