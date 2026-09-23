use super::*;
use crate::hir::{GradientAnchor, GradientKind};

impl<'h> FnCtx<'h> {
    #[expect(
        clippy::too_many_arguments,
        reason = "Scatter lowering helpers thread explicit IR handles to avoid heap allocations and preserve lowering clarity"
    )]
    fn lower_scatter_procedural_rgb(
        &mut self,
        scatter_layer_id: LayerId,
        body: LayerId,
        lifecycle: &Option<crate::hir::ScatterLifecycleParams>,
        p: Handle<Ex>,
        px: Handle<Ex>,
        py: Handle<Ex>,
        min_x: Handle<Ex>,
        min_y: Handle<Ex>,
        width: Handle<Ex>,
        height: Handle<Ex>,
        time: Handle<Ex>,
        selected: Handle<Ex>,
        bins_x: usize,
        bins_y: usize,
        emitted_instances: usize,
        procedural_samples: usize,
        avg_footprint: f32,
    ) -> (Handle<Ex>, Handle<Ex>) {
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let zero3 = self.ir.splat3(zero);

        // Hash each selected bin into a bounded number of synthetic candidates.
        // This preserves procedural O(1)-bounded work while avoiding sparse
        // one-candidate output in visually dense scatter programs.
        let mut acc_rgb = zero3;
        let mut acc_alpha = zero;
        for slot in 0..procedural_samples {
            let slot_f = self.ir.lit(slot as f32);

            let k17 = self.ir.lit(17.0);
            let h0 = k17;
            let k131 = self.ir.lit(131.0);
            let sel_term = self.ir.mul(selected, k131);
            let k19 = self.ir.lit(19.0);
            let slot_term = self.ir.mul(slot_f, k19);
            let h1_base = self.ir.addx(h0, sel_term);
            let h1 = self.ir.addx(h1_base, slot_term);
            let k129898 = self.ir.lit(12.9898);
            let h1_scaled = self.ir.mul(h1, k129898);
            let k78233 = self.ir.lit(78.233);
            let h2 = self.ir.addx(h1_scaled, k78233);
            let hs = self.ir.m1(Mf::Sin, h2);
            let k43758 = self.ir.lit(43758.547);
            let hs_scaled = self.ir.mul(hs, k43758);
            let h = self.ir.m1(Mf::Fract, hs_scaled);

            let k53 = self.ir.lit(53.0);
            let k037 = self.ir.lit(0.37);
            let h53 = self.ir.mul(h, k53);
            let sel037 = self.ir.mul(selected, k037);
            let k013 = self.ir.lit(0.13);
            let slot013 = self.ir.mul(slot_f, k013);
            let u_base = self.ir.addx(h53, sel037);
            let u_term = self.ir.addx(u_base, slot013);
            let local_u = self.ir.m1(Mf::Fract, u_term);

            let k97 = self.ir.lit(97.0);
            let k073 = self.ir.lit(0.73);
            let h97 = self.ir.mul(h, k97);
            let sel073 = self.ir.mul(selected, k073);
            let k029 = self.ir.lit(0.29);
            let slot029 = self.ir.mul(slot_f, k029);
            let v_base = self.ir.addx(h97, sel073);
            let v_term = self.ir.addx(v_base, slot029);
            let local_v = self.ir.m1(Mf::Fract, v_term);

            let bins_x_f = self.ir.lit(bins_x as f32);
            let bins_y_f = self.ir.lit(bins_y as f32);
            let sel_over_bins_x = self.ir.div(selected, bins_x_f);
            let bin_y = self.ir.m1(Mf::Floor, sel_over_bins_x);
            let by_mul = self.ir.mul(bin_y, bins_x_f);
            let bin_x = self.ir.sub(selected, by_mul);
            let bin_local_u = self.ir.addx(bin_x, local_u);
            let bin_local_v = self.ir.addx(bin_y, local_v);
            let inst_u = self.ir.div(bin_local_u, bins_x_f);
            let inst_v = self.ir.div(bin_local_v, bins_y_f);

            let emitted_f = self.ir.lit(emitted_instances as f32);
            let id_unfloored = self.ir.mul(h, emitted_f);
            let inst_id = self.ir.m1(Mf::Floor, id_unfloored);
            let denom = self.ir.lit(if emitted_instances > 1 {
                (emitted_instances - 1) as f32
            } else {
                1.0
            });
            let inst_index01 = self.ir.div(inst_id, denom);

            self.clear_point_sensitive_caches();

            let inst_dx = self.ir.mul(inst_u, width);
            let inst_dy = self.ir.mul(inst_v, height);
            let inst_pos_x = self.ir.addx(min_x, inst_dx);
            let inst_pos_y = self.ir.addx(min_y, inst_dy);
            let placeholder_age_norm = self.ir.lit(0.0);
            self.scatter_instance_ctx.push(ScatterInstanceCtx {
                pos_x: inst_pos_x,
                pos_y: inst_pos_y,
                id: inst_id,
                index01: inst_index01,
                age_norm: placeholder_age_norm,
            });
            let inst_age_norm = if let Some(lifecycle) = lifecycle.as_ref() {
                let respawn = self.sx_at(
                    &Sx::Max(
                        Box::new(lifecycle.respawn_every.clone()),
                        Box::new(Sx::Lit(1.0e-6)),
                    ),
                    p,
                );
                let time_phase = self.ir.div(time, respawn);
                let phase = self.ir.addx(time_phase, inst_index01);
                self.ir.m1(Mf::Fract, phase)
            } else {
                self.ir.lit(0.5)
            };
            let instance_visible = if let Some(lifecycle) = lifecycle.as_ref() {
                let respawn = self.sx_at(
                    &Sx::Max(
                        Box::new(lifecycle.respawn_every.clone()),
                        Box::new(Sx::Lit(1.0e-6)),
                    ),
                    p,
                );
                let lifetime = self.sx_at(
                    &Sx::Clamp(
                        Box::new(lifecycle.lifetime.clone()),
                        Box::new(Sx::Lit(0.0)),
                        Box::new(lifecycle.respawn_every.clone()),
                    ),
                    p,
                );
                let duty = self.ir.div(lifetime, respawn);
                let is_visible = self.ir.bin(Bo::Less, inst_age_norm, duty);
                self.ir.add(Ex::Select {
                    condition: is_visible,
                    accept: one,
                    reject: zero,
                })
            } else {
                one
            };

            let (rgb, a) = if let Some(&helper_fn) = self.scatter_body_fns.get(&scatter_layer_id) {
                let call_args: Vec<Handle<Ex>> = {
                    let mut args = vec![
                        p,
                        self.ir.px,
                        self.ir.aa,
                        self.ir.time,
                        self.ir.delta,
                        self.ir.res,
                        inst_pos_x,
                        inst_pos_y,
                        inst_id,
                        inst_index01,
                        inst_age_norm,
                    ];
                    for param in &self.hir.params {
                        if let Some((elem_ty_str, len)) =
                            crate::hir::parse_array_param_type(&param.ty_name)
                        {
                            use crate::hir::ArrayElemType;

                            let Some(elem_type) = ArrayElemType::from_str(elem_ty_str) else {
                                unreachable!("checker validated array element type");
                            };

                            match elem_type {
                                ArrayElemType::F32
                                | ArrayElemType::I32
                                | ArrayElemType::U32
                                | ArrayElemType::Bool => {
                                    for index in 0..len {
                                        let elem_name = format!("{}__{index}", param.name);
                                        args.push(
                                            *self
                                                .ir
                                                .param_scalars
                                                .get(&elem_name)
                                                .expect("array param element must exist"),
                                        );
                                    }
                                }
                                ArrayElemType::Vec2 => {
                                    for index in 0..len {
                                        for component in ["x", "y"] {
                                            let elem_name =
                                                format!("{}__{index}__{component}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "vec2 array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                                ArrayElemType::Vec3 => {
                                    for index in 0..len {
                                        for component in ["x", "y", "z"] {
                                            let elem_name =
                                                format!("{}__{index}__{component}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "vec3 array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                                ArrayElemType::Vec4 => {
                                    for index in 0..len {
                                        for component in ["x", "y", "z", "w"] {
                                            let elem_name =
                                                format!("{}__{index}__{component}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "vec4 array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                                ArrayElemType::Mat2 => {
                                    for index in 0..len {
                                        for i in 0..2 {
                                            for j in 0..2 {
                                                let elem_name =
                                                    format!("{}__{index}__{i}_{j}", param.name);
                                                args.push(
                                                    *self.ir.param_scalars.get(&elem_name).expect(
                                                        "mat2 array param component must exist",
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                ArrayElemType::Mat3 => {
                                    for index in 0..len {
                                        for i in 0..3 {
                                            for j in 0..3 {
                                                let elem_name =
                                                    format!("{}__{index}__{i}_{j}", param.name);
                                                args.push(
                                                    *self.ir.param_scalars.get(&elem_name).expect(
                                                        "mat3 array param component must exist",
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                ArrayElemType::Mat4 => {
                                    for index in 0..len {
                                        for i in 0..4 {
                                            for j in 0..4 {
                                                let elem_name =
                                                    format!("{}__{index}__{i}_{j}", param.name);
                                                args.push(
                                                    *self.ir.param_scalars.get(&elem_name).expect(
                                                        "mat4 array param component must exist",
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                ArrayElemType::Color => {
                                    for index in 0..len {
                                        for suffix in ["r", "g", "b", "a"] {
                                            let elem_name =
                                                format!("{}__{index}.{suffix}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "color array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        match param.ty_name.as_str() {
                            "f32" | "i32" | "u32" | "bool" => {
                                args.push(
                                    *self
                                        .ir
                                        .param_scalars
                                        .get(&param.name)
                                        .expect("param must exist"),
                                );
                            }
                            "color" => {
                                for suffix in ["r", "g", "b", "a"] {
                                    args.push(
                                        *self
                                            .ir
                                            .param_scalars
                                            .get(&format!("{}.{suffix}", param.name))
                                            .expect("color param component must exist"),
                                    );
                                }
                            }
                            _ => unreachable!("checker emitted unsupported param type"),
                        }
                    }
                    args
                };

                let call_result = self.ir.add(Ex::CallResult(helper_fn));
                self.ir.push_statement(naga::Statement::Call {
                    function: helper_fn,
                    arguments: call_args,
                    result: Some(call_result),
                });

                let body_r = self.ir.add(Ex::AccessIndex {
                    base: call_result,
                    index: 0,
                });
                let body_g = self.ir.add(Ex::AccessIndex {
                    base: call_result,
                    index: 1,
                });
                let body_b = self.ir.add(Ex::AccessIndex {
                    base: call_result,
                    index: 2,
                });
                let body_a = self.ir.add(Ex::AccessIndex {
                    base: call_result,
                    index: 3,
                });
                let body_rgb = self.ir.add(Ex::Compose {
                    ty: self.ir.types.v3,
                    components: vec![body_r, body_g, body_b],
                });
                (body_rgb, body_a)
            } else {
                self.clear_point_sensitive_caches();

                self.scatter_instance_ctx.push(ScatterInstanceCtx {
                    pos_x: inst_pos_x,
                    pos_y: inst_pos_y,
                    id: inst_id,
                    index01: inst_index01,
                    age_norm: inst_age_norm,
                });
                let result = self.layer_color(body, p);
                self.scatter_instance_ctx.pop();
                result
            };

            let dx = self.ir.sub(px, inst_pos_x);
            let dy = self.ir.sub(py, inst_pos_y);
            let dx2 = self.ir.mul(dx, dx);
            let dy2 = self.ir.mul(dy, dy);
            let dist2 = self.ir.addx(dx2, dy2);
            let footprint = self.ir.lit(avg_footprint.max(0.01));
            let two = self.ir.lit(2.0);
            let pad = self.ir.mul(self.ir.px, two);
            let bound = self.ir.addx(footprint, pad);
            let bound2 = self.ir.mul(bound, bound);
            let in_bounds = self.ir.bin(Bo::LessEqual, dist2, bound2);

            let a = self.ir.mul(a, instance_visible);
            let af = self.ir.splat3(a);
            let blended = self.ir.m3(Mf::Mix, acc_rgb, rgb, af);
            let inv_acc_alpha = self.ir.sub(one, acc_alpha);
            let src_alpha_contrib = self.ir.mul(a, inv_acc_alpha);
            let alpha_next = self.ir.addx(acc_alpha, src_alpha_contrib);
            acc_rgb = self.ir.add(Ex::Select {
                condition: in_bounds,
                accept: blended,
                reject: acc_rgb,
            });
            acc_alpha = self.ir.add(Ex::Select {
                condition: in_bounds,
                accept: alpha_next,
                reject: acc_alpha,
            });
        }

        (acc_rgb, acc_alpha)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Bin lowering helper mirrors procedural helper parameter threading for consistent call sites"
    )]
    fn lower_scatter_bin_rgb(
        &mut self,
        scatter_layer_id: LayerId,
        body: LayerId,
        lifecycle: &Option<crate::hir::ScatterLifecycleParams>,
        p: Handle<Ex>,
        px: Handle<Ex>,
        py: Handle<Ex>,
        min_x: Handle<Ex>,
        min_y: Handle<Ex>,
        width: Handle<Ex>,
        height: Handle<Ex>,
        time: Handle<Ex>,
        bin_entries: &[crate::hir::ScatterInstance],
    ) -> (Handle<Ex>, Handle<Ex>) {
        let zero = self.ir.lit(0.0);
        let zero3 = self.ir.splat3(zero);
        let slot_limit = bin_entries.len().min(Self::SCATTER_BIN_RUNTIME_CAP);
        if slot_limit == 0 {
            return (zero3, zero);
        }

        let slot_ptr = self
            .ir
            .local("scatter_slot", self.ir.types.f32_, Some(zero));
        let bin_rgb_ptr = self
            .ir
            .local("scatter_bin_rgb_acc", self.ir.types.v3, Some(zero3));
        let bin_alpha_ptr = self
            .ir
            .local("scatter_bin_alpha_acc", self.ir.types.f32_, Some(zero));

        self.ir.begin_block();
        let slot = self.ir.load(slot_ptr);

        let mut inst_u = self.ir.lit(bin_entries[0].pos.0);
        let mut inst_v = self.ir.lit(bin_entries[0].pos.1);
        let mut inst_id = self.ir.lit(bin_entries[0].id as f32);
        let mut inst_index01 = self.ir.lit(bin_entries[0].index01);
        let mut inst_footprint = self.ir.lit(bin_entries[0].footprint.max(0.01));
        for (idx, instance) in bin_entries.iter().take(slot_limit).enumerate().skip(1) {
            let idx_f = self.ir.lit(idx as f32);
            let is_idx = self.ir.bin(Bo::Equal, slot, idx_f);
            let inst_u_lit = self.ir.lit(instance.pos.0);
            let inst_v_lit = self.ir.lit(instance.pos.1);
            let inst_id_lit = self.ir.lit(instance.id as f32);
            let inst_index01_lit = self.ir.lit(instance.index01);
            let inst_footprint_lit = self.ir.lit(instance.footprint.max(0.01));
            inst_u = self.ir.add(Ex::Select {
                condition: is_idx,
                accept: inst_u_lit,
                reject: inst_u,
            });
            inst_v = self.ir.add(Ex::Select {
                condition: is_idx,
                accept: inst_v_lit,
                reject: inst_v,
            });
            inst_id = self.ir.add(Ex::Select {
                condition: is_idx,
                accept: inst_id_lit,
                reject: inst_id,
            });
            inst_index01 = self.ir.add(Ex::Select {
                condition: is_idx,
                accept: inst_index01_lit,
                reject: inst_index01,
            });
            inst_footprint = self.ir.add(Ex::Select {
                condition: is_idx,
                accept: inst_footprint_lit,
                reject: inst_footprint,
            });
        }

        self.clear_point_sensitive_caches();

        let inst_dx = self.ir.mul(inst_u, width);
        let inst_dy = self.ir.mul(inst_v, height);
        let inst_pos_x = self.ir.addx(min_x, inst_dx);
        let inst_pos_y = self.ir.addx(min_y, inst_dy);
        let placeholder_age_norm = self.ir.lit(0.0);
        self.scatter_instance_ctx.push(ScatterInstanceCtx {
            pos_x: inst_pos_x,
            pos_y: inst_pos_y,
            id: inst_id,
            index01: inst_index01,
            age_norm: placeholder_age_norm,
        });
        let inst_age_norm = if let Some(lifecycle) = lifecycle.as_ref() {
            let respawn = self.sx_at(
                &Sx::Max(
                    Box::new(lifecycle.respawn_every.clone()),
                    Box::new(Sx::Lit(1.0e-6)),
                ),
                p,
            );
            let time_phase = self.ir.div(time, respawn);
            let phase = self.ir.addx(time_phase, inst_index01);
            self.ir.m1(Mf::Fract, phase)
        } else {
            self.ir.lit(0.5)
        };
        let instance_visible = if let Some(lifecycle) = lifecycle.as_ref() {
            let respawn = self.sx_at(
                &Sx::Max(
                    Box::new(lifecycle.respawn_every.clone()),
                    Box::new(Sx::Lit(1.0e-6)),
                ),
                p,
            );
            let lifetime = self.sx_at(
                &Sx::Clamp(
                    Box::new(lifecycle.lifetime.clone()),
                    Box::new(Sx::Lit(0.0)),
                    Box::new(lifecycle.respawn_every.clone()),
                ),
                p,
            );
            let duty = self.ir.div(lifetime, respawn);
            let one = self.ir.lit(1.0);
            let is_visible = self.ir.bin(Bo::Less, inst_age_norm, duty);
            self.ir.add(Ex::Select {
                condition: is_visible,
                accept: one,
                reject: zero,
            })
        } else {
            self.ir.lit(1.0)
        };
        self.scatter_instance_ctx.pop();

        let (rgb, a) = if let Some(&helper_fn) = self.scatter_body_fns.get(&scatter_layer_id) {
            let call_args: Vec<Handle<Ex>> =
                {
                    let mut args = vec![
                        p,
                        self.ir.px,
                        self.ir.aa,
                        self.ir.time,
                        self.ir.delta,
                        self.ir.res,
                        inst_pos_x,
                        inst_pos_y,
                        inst_id,
                        inst_index01,
                        inst_age_norm,
                    ];
                    for param in &self.hir.params {
                        if let Some((elem_ty_str, len)) =
                            crate::hir::parse_array_param_type(&param.ty_name)
                        {
                            use crate::hir::ArrayElemType;

                            let Some(elem_type) = ArrayElemType::from_str(elem_ty_str) else {
                                unreachable!("checker validated array element type");
                            };

                            match elem_type {
                                ArrayElemType::F32
                                | ArrayElemType::I32
                                | ArrayElemType::U32
                                | ArrayElemType::Bool => {
                                    for index in 0..len {
                                        let elem_name = format!("{}__{index}", param.name);
                                        args.push(
                                            *self
                                                .ir
                                                .param_scalars
                                                .get(&elem_name)
                                                .expect("array param element must exist"),
                                        );
                                    }
                                }
                                ArrayElemType::Vec2 => {
                                    for index in 0..len {
                                        for component in ["x", "y"] {
                                            let elem_name =
                                                format!("{}__{index}__{component}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "vec2 array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                                ArrayElemType::Vec3 => {
                                    for index in 0..len {
                                        for component in ["x", "y", "z"] {
                                            let elem_name =
                                                format!("{}__{index}__{component}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "vec3 array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                                ArrayElemType::Vec4 => {
                                    for index in 0..len {
                                        for component in ["x", "y", "z", "w"] {
                                            let elem_name =
                                                format!("{}__{index}__{component}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "vec4 array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                                ArrayElemType::Mat2 => {
                                    for index in 0..len {
                                        for i in 0..2 {
                                            for j in 0..2 {
                                                let elem_name =
                                                    format!("{}__{index}__{i}_{j}", param.name);
                                                args.push(
                                                    *self.ir.param_scalars.get(&elem_name).expect(
                                                        "mat2 array param component must exist",
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                ArrayElemType::Mat3 => {
                                    for index in 0..len {
                                        for i in 0..3 {
                                            for j in 0..3 {
                                                let elem_name =
                                                    format!("{}__{index}__{i}_{j}", param.name);
                                                args.push(
                                                    *self.ir.param_scalars.get(&elem_name).expect(
                                                        "mat3 array param component must exist",
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                ArrayElemType::Mat4 => {
                                    for index in 0..len {
                                        for i in 0..4 {
                                            for j in 0..4 {
                                                let elem_name =
                                                    format!("{}__{index}__{i}_{j}", param.name);
                                                args.push(
                                                    *self.ir.param_scalars.get(&elem_name).expect(
                                                        "mat4 array param component must exist",
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                                ArrayElemType::Color => {
                                    for index in 0..len {
                                        for suffix in ["r", "g", "b", "a"] {
                                            let elem_name =
                                                format!("{}__{index}.{suffix}", param.name);
                                            args.push(
                                                *self.ir.param_scalars.get(&elem_name).expect(
                                                    "color array param component must exist",
                                                ),
                                            );
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        match param.ty_name.as_str() {
                            "f32" | "i32" | "u32" | "bool" => {
                                args.push(
                                    *self
                                        .ir
                                        .param_scalars
                                        .get(&param.name)
                                        .expect("param must exist"),
                                );
                            }
                            "color" => {
                                for suffix in ["r", "g", "b", "a"] {
                                    args.push(
                                        *self
                                            .ir
                                            .param_scalars
                                            .get(&format!("{}.{suffix}", param.name))
                                            .expect("color param component must exist"),
                                    );
                                }
                            }
                            _ => unreachable!("checker emitted unsupported param type"),
                        }
                    }
                    args
                };

            let call_result = self.ir.add(Ex::CallResult(helper_fn));
            self.ir.push_statement(naga::Statement::Call {
                function: helper_fn,
                arguments: call_args,
                result: Some(call_result),
            });

            let body_r = self.ir.add(Ex::AccessIndex {
                base: call_result,
                index: 0,
            });
            let body_g = self.ir.add(Ex::AccessIndex {
                base: call_result,
                index: 1,
            });
            let body_b = self.ir.add(Ex::AccessIndex {
                base: call_result,
                index: 2,
            });
            let body_a = self.ir.add(Ex::AccessIndex {
                base: call_result,
                index: 3,
            });
            let body_rgb = self.ir.add(Ex::Compose {
                ty: self.ir.types.v3,
                components: vec![body_r, body_g, body_b],
            });
            (body_rgb, body_a)
        } else {
            self.sx_cache.clear();
            self.sdf_cache.clear();

            self.scatter_instance_ctx.push(ScatterInstanceCtx {
                pos_x: inst_pos_x,
                pos_y: inst_pos_y,
                id: inst_id,
                index01: inst_index01,
                age_norm: inst_age_norm,
            });
            let result = self.layer_color(body, p);
            self.scatter_instance_ctx.pop();
            result
        };

        let dx = self.ir.sub(px, inst_pos_x);
        let dy = self.ir.sub(py, inst_pos_y);
        let dx2 = self.ir.mul(dx, dx);
        let dy2 = self.ir.mul(dy, dy);
        let dist2 = self.ir.addx(dx2, dy2);
        let two = self.ir.lit(2.0);
        let pad = self.ir.mul(self.ir.px, two);
        let bound = self.ir.addx(inst_footprint, pad);
        let bound2 = self.ir.mul(bound, bound);
        let in_bounds = self.ir.bin(Bo::LessEqual, dist2, bound2);

        let a = self.ir.mul(a, instance_visible);
        let af = self.ir.splat3(a);
        let bin_rgb_prev = self.ir.load(bin_rgb_ptr);
        let blended = self.ir.m3(Mf::Mix, bin_rgb_prev, rgb, af);
        let bin_rgb_next = self.ir.add(Ex::Select {
            condition: in_bounds,
            accept: blended,
            reject: bin_rgb_prev,
        });
        let bin_alpha_prev = self.ir.load(bin_alpha_ptr);
        let one = self.ir.lit(1.0);
        let inv_bin_alpha = self.ir.sub(one, bin_alpha_prev);
        let src_alpha_contrib = self.ir.mul(a, inv_bin_alpha);
        let bin_alpha_over = self.ir.addx(bin_alpha_prev, src_alpha_contrib);
        let bin_alpha_next = self.ir.add(Ex::Select {
            condition: in_bounds,
            accept: bin_alpha_over,
            reject: bin_alpha_prev,
        });
        self.ir.store(bin_rgb_ptr, bin_rgb_next);
        self.ir.store(bin_alpha_ptr, bin_alpha_next);
        let body_block = self.ir.end_block();

        self.ir.begin_block();
        let slot_prev = self.ir.load(slot_ptr);
        let one = self.ir.lit(1.0);
        let slot_next = self.ir.addx(slot_prev, one);
        self.ir.store(slot_ptr, slot_next);
        let continuing_block = self.ir.end_block();

        let limit = self.ir.lit(slot_limit as f32);
        let break_if = self.ir.bin(Bo::GreaterEqual, slot_next, limit);
        self.ir.push_statement(naga::Statement::Loop {
            body: body_block,
            continuing: continuing_block,
            break_if: Some(break_if),
        });

        (self.ir.load(bin_rgb_ptr), self.ir.load(bin_alpha_ptr))
    }

    fn sample_bound_texture(&mut self, tex_name: &str, p: Handle<Ex>) -> (Handle<Ex>, Handle<Ex>) {
        let &tex_global = self
            .ir
            .tex_globals
            .get(tex_name)
            .expect("texture global must be registered before lowering");
        let sampler_global = self
            .ir
            .sampler_global
            .expect("sampler global must exist when textures are used");
        let img = self.ir.add(Ex::GlobalVariable(tex_global));
        let sampler = self.ir.add(Ex::GlobalVariable(sampler_global));
        let sample = self.ir.add(Ex::ImageSample {
            image: img,
            sampler,
            gather: None,
            coordinate: p,
            array_index: None,
            offset: None,
            level: naga::SampleLevel::Auto,
            depth_ref: None,
            clamp_to_edge: false,
        });
        let r = self.ir.add(Ex::AccessIndex {
            base: sample,
            index: 0,
        });
        let g = self.ir.add(Ex::AccessIndex {
            base: sample,
            index: 1,
        });
        let b = self.ir.add(Ex::AccessIndex {
            base: sample,
            index: 2,
        });
        let a = self.ir.add(Ex::AccessIndex {
            base: sample,
            index: 3,
        });
        let rgb = self.ir.add(Ex::Compose {
            ty: self.ir.types.v3,
            components: vec![r, g, b],
        });
        (rgb, a)
    }

    /// Lower a layer to (rgb: vec3, alpha: f32) at coordinate `p`.
    pub(super) fn layer_color(&mut self, id: LayerId, p: Handle<Ex>) -> (Handle<Ex>, Handle<Ex>) {
        let previous = self.color_shape;
        self.color_shape = self.hir.color_shape(id);
        if previous != self.color_shape {
            self.clear_shape_color_caches();
        }
        let result = self.layer_color_inner(id, p);
        if previous != self.color_shape {
            self.clear_shape_color_caches();
        }
        self.color_shape = previous;
        result
    }

    fn clear_shape_color_caches(&mut self) {
        // Changing the color receiver does not invalidate geometry or scene
        // expressions. Preserve their sharing across adjacent effects.
        self.sx_cache
            .retain(|(expr, _), _| !expr.requires_shape_color_frame());
        self.user_call_cache
            .retain(|(call, _), _| !call.args.iter().any(Sx::requires_shape_color_frame));
    }

    fn layer_color_inner(&mut self, id: LayerId, p: Handle<Ex>) -> (Handle<Ex>, Handle<Ex>) {
        let prev_locality = self
            .ir
            .set_locality(self.hir.layer_locality.get(id).copied());
        if let Some(current_pass_id) = self.current_pass_id
            && let Some(layer_pass_id) = self.layer_to_pass.get(id).copied()
            && layer_pass_id != current_pass_id
            && let Some(tex_name) = self.pass_output_target_names.get(&layer_pass_id).cloned()
        {
            let sampled = self.sample_bound_texture(&tex_name, p);
            self.ir.restore_locality(prev_locality);
            return sampled;
        }
        let out = match self.hir.layers[id].clone() {
            Layer::Solid(c) => {
                let rgb = self.color3(c);
                let a = self.ir.lit(c[3]);
                (rgb, a)
            }
            Layer::Fill { shape, color } => {
                let cov = self.shape_coverage(shape, p);
                let ca = self.ir.lit(color[3]);
                let a = self.ir.mul(cov, ca);
                self.ir.name(a, &format!("fill_a_l{id}"));
                (self.color3(color), a)
            }
            Layer::FillExpr { shape, r, g, b, a } => {
                let cov = self.shape_coverage(shape, p);
                let r = self.sx_at(&r, p);
                let g = self.sx_at(&g, p);
                let b = self.sx_at(&b, p);
                let ca = self.sx_at(&a, p);
                let a = self.ir.mul(cov, ca);
                self.ir.name(a, &format!("fill_expr_a_l{id}"));
                let rgb = self.ir.add(Ex::Compose {
                    ty: self.ir.types.v3,
                    components: vec![r, g, b],
                });
                (rgb, a)
            }
            Layer::FillGradient { shape, kind, stops } => {
                let cov = self.shape_coverage(shape, p);
                let grad_p = match &kind {
                    GradientKind::Linear {
                        anchor: GradientAnchor::Shape,
                        ..
                    } => self.shape_local_point(shape, p),
                    _ => p,
                };
                let (rgb_dithered, grad_a) = self.gradient_color_at(&kind, &stops, grad_p);
                let a = self.ir.mul(cov, grad_a);
                self.ir.name(a, &format!("fill_grad_a_l{id}"));
                (rgb_dithered, a)
            }
            Layer::Soften {
                shape,
                radius,
                color,
            } => {
                let d = self.sdf(shape, p);
                let cov = self.soft_coverage(shape, d, &radius, p);
                let ca = self.sx_at(&color[3], p);
                let a = self.ir.mul(cov, ca);
                self.ir.name(a, &format!("soft_a_l{id}"));
                (self.color_expr3_at(&color, p), a)
            }
            Layer::Shadow {
                shape,
                off,
                soften,
                color,
            } => {
                // sdf(translate(shape, off)) is sdf evaluated at p - off; the
                // shifted coordinate is its own CSE key, so a fill of the
                // same shape at `p` still shares nothing incorrectly.
                let off = self.v2x_at(&off, p);
                let ps = self.ir.sub(p, off);
                let d = self.sdf(shape, ps);
                let cov = self.soft_coverage(shape, d, &soften, p);
                let ca = self.sx_at(&color[3], p);
                let a = self.ir.mul(cov, ca);
                self.ir.name(a, &format!("shadow_a_l{id}"));
                (self.color_expr3_at(&color, p), a)
            }
            Layer::Glow {
                shape,
                reach,
                strength,
                color,
                falloff,
                color_space,
            } => {
                let (glow_p, glow_reach) = match reach {
                    GlowReach::Scalar(reach) => (p, self.sx_at(&reach, p)),
                    GlowReach::Vec2((rx, ry)) => {
                        let rx = self.sx_at(&rx, p);
                        let ry = self.sx_at(&ry, p);
                        self.shape_glow_warp(shape, p, rx, ry)
                    }
                };
                let shape_color_p = self.shape_anchor_space_point(shape, p, p);
                let glow_color_p = self.shape_anchor_space_point(shape, p, glow_p);
                let color_p = match color_space {
                    GlowColorSpace::Scene => p,
                    GlowColorSpace::Shape => shape_color_p,
                    GlowColorSpace::Glow => glow_color_p,
                };
                let d = self.sdf(shape, glow_p);
                let zero = self.ir.lit(0.0);
                let dpos = self.ir.m2(Mf::Max, d, zero);
                self.ir.name(dpos, &format!("glow_dpos_l{id}"));
                let g = self.glow_falloff_at(falloff, dpos, glow_reach);
                self.ir.name(g, &format!("glow_falloff_l{id}"));
                let strength = self.sx_at(&strength, p);
                let gs = self.ir.mul(g, strength);
                let (glow_rgb, ca) = self.color_source_at(&color, color_p);
                let a = self.ir.mul(gs, ca);
                self.ir.name(a, &format!("glow_a_l{id}"));
                (glow_rgb, a)
            }
            Layer::InnerGlow {
                shape,
                reach,
                strength,
                color,
                falloff,
                color_space,
            } => {
                let (glow_p, glow_reach) = match reach {
                    GlowReach::Scalar(reach) => (p, self.sx_at(&reach, p)),
                    GlowReach::Vec2((rx, ry)) => {
                        let rx = self.sx_at(&rx, p);
                        let ry = self.sx_at(&ry, p);
                        self.shape_glow_warp(shape, p, rx, ry)
                    }
                };
                let shape_color_p = self.shape_anchor_space_point(shape, p, p);
                let glow_color_p = self.shape_anchor_space_point(shape, p, glow_p);
                let color_p = match color_space {
                    GlowColorSpace::Scene => p,
                    GlowColorSpace::Shape => shape_color_p,
                    GlowColorSpace::Glow => glow_color_p,
                };
                let d = self.sdf(shape, glow_p);
                let inside = self.coverage(shape, d, glow_p);
                let zero = self.ir.lit(0.0);
                let neg_d = self.ir.neg(d);
                let din = self.ir.m2(Mf::Max, neg_d, zero);
                let fall = self.glow_falloff_at(falloff, din, glow_reach);
                let strength = self.sx_at(&strength, p);
                let fall_strength = self.ir.mul(fall, strength);
                let s = self.ir.mul(inside, fall_strength);
                let (glow_rgb, ca) = self.color_source_at(&color, color_p);
                let a = self.ir.mul(s, ca);
                self.ir.name(a, &format!("inner_glow_a_l{id}"));
                (glow_rgb, a)
            }
            Layer::Bevel {
                shape,
                width,
                light,
                strength,
                highlight,
                shadow,
            } => {
                let d = self.sdf(shape, p);

                let px = self.ir.px;
                let zero = self.ir.lit(0.0);
                let dx = self.ir.vec2(px, zero);
                let zero2 = self.ir.lit(0.0);
                let dy = self.ir.vec2(zero2, px);
                let ppx = self.ir.addx(p, dx);
                let pmx = self.ir.sub(p, dx);
                let ppy = self.ir.addx(p, dy);
                let pmy = self.ir.sub(p, dy);
                let sdf_ppx = self.sdf(shape, ppx);
                let sdf_pmx = self.sdf(shape, pmx);
                let ddx = self.ir.sub(sdf_ppx, sdf_pmx);
                let sdf_ppy = self.sdf(shape, ppy);
                let sdf_pmy = self.sdf(shape, pmy);
                let ddy = self.ir.sub(sdf_ppy, sdf_pmy);
                let grad = self.ir.vec2(ddx, ddy);
                let grad_len = self.ir.m1(Mf::Length, grad);
                let eps = self.ir.lit(1.0e-6);
                let safe_grad_len = self.ir.m2(Mf::Max, grad_len, eps);
                let safe_grad_len2 = self.ir.splat2(safe_grad_len);
                let n = self.ir.div(grad, safe_grad_len2);

                let lv = self.v2x_at(&light, p);
                let lv_len = self.ir.m1(Mf::Length, lv);
                let safe_lv_len = self.ir.m2(Mf::Max, lv_len, eps);
                let safe_lv_len2 = self.ir.splat2(safe_lv_len);
                let ldir = self.ir.div(lv, safe_lv_len2);

                let nx = self.ir.x_of(n);
                let ny = self.ir.y_of(n);
                let lx = self.ir.x_of(ldir);
                let ly = self.ir.y_of(ldir);
                let ndotlx = self.ir.mul(nx, lx);
                let ndotly = self.ir.mul(ny, ly);
                let ndotl = self.ir.addx(ndotlx, ndotly);
                let half = self.ir.lit(0.5);
                let one = self.ir.lit(1.0);
                let half_ndotl = self.ir.mul(half, ndotl);
                let shade_raw = self.ir.addx(half, half_ndotl);
                let zero3 = self.ir.lit(0.0);
                let shade = self.ir.m3(Mf::Clamp, shade_raw, zero3, one);

                let w = self.sx_at(&width, p);
                let hw = self.ir.mul(w, half);
                let ad = self.ir.m1(Mf::Abs, d);
                let zero4 = self.ir.lit(0.0);
                let edge_step = self.ir.m3(Mf::SmoothStep, zero4, hw, ad);
                let edge01 = self.ir.sub(one, edge_step);

                let hi_rgb = self.color_expr3_at(&highlight, p);
                let lo_rgb = self.color_expr3_at(&shadow, p);
                let shade3 = self.ir.splat3(shade);
                let rgb = self.ir.m3(Mf::Mix, lo_rgb, hi_rgb, shade3);

                let hi_a = self.sx_at(&highlight[3], p);
                let lo_a = self.sx_at(&shadow[3], p);
                let a_mix = self.ir.m3(Mf::Mix, lo_a, hi_a, shade);
                let stren = self.sx_at(&strength, p);
                let bevel_strength = self.ir.mul(a_mix, stren);
                let a = self.ir.mul(edge01, bevel_strength);
                self.ir.name(a, &format!("bevel_a_l{id}"));
                (rgb, a)
            }
            Layer::GlowFx {
                inner,
                reach,
                strength,
                color,
                falloff,
            } => {
                let (inner_rgb, inner_a) = self.layer_color(inner, p);
                let reach = match reach {
                    GlowReach::Scalar(reach) => self.sx_at(&reach, p),
                    GlowReach::Vec2((rx, ry)) => {
                        let rx = self.sx_at(&rx, p);
                        let ry = self.sx_at(&ry, p);
                        let abs_rx = self.ir.m1(Mf::Abs, rx);
                        let abs_ry = self.ir.m1(Mf::Abs, ry);
                        self.ir.m2(Mf::Max, abs_rx, abs_ry)
                    }
                };
                let strength = self.sx_at(&strength, p);
                let one = self.ir.lit(1.0);
                let d_nonnegative = self.ir.sub(one, inner_a);
                let fall = self.glow_falloff_at(falloff, d_nonnegative, reach);
                let base = self.ir.mul(inner_a, strength);
                let glow_scalar = self.ir.mul(base, fall);
                self.ir
                    .name(glow_scalar, &format!("layer_glow_scalar_l{id}"));

                let (glow_rgb, ca) = self.color_source_at(&color, p);
                let glow_scalar3 = self.ir.splat3(glow_scalar);
                let glow = self.ir.mul(glow_rgb, glow_scalar3);
                self.ir.name(glow, &format!("layer_glow_rgb_l{id}"));

                let rgb = self.ir.addx(inner_rgb, glow);
                self.ir.name(rgb, &format!("layer_glow_out_rgb_l{id}"));

                let a_add = self.ir.mul(glow_scalar, ca);
                let a_raw = self.ir.addx(inner_a, a_add);
                let zero = self.ir.lit(0.0);
                let out_a = self.ir.m3(Mf::Clamp, a_raw, zero, one);
                self.ir.name(out_a, &format!("layer_glow_out_a_l{id}"));
                (rgb, out_a)
            }
            Layer::ScatterBins {
                min,
                max,
                bins_x,
                bins_y,
                strategy,
                lifecycle,
                bins,
                body,
            } => {
                let prev_feature_tag = self.ir.set_feature_tag(Some("scatter"));
                let px = self.ir.x_of(p);
                let py = self.ir.y_of(p);

                let min_x = self.sx_at(&min.0, p);
                let min_y = self.sx_at(&min.1, p);
                let max_x = self.sx_at(&max.0, p);
                let max_y = self.sx_at(&max.1, p);

                let width = self.ir.sub(max_x, min_x);
                let height = self.ir.sub(max_y, min_y);
                let eps = self.ir.lit(1.0e-6);
                let safe_w = self.ir.m2(Mf::Max, width, eps);
                let safe_h = self.ir.m2(Mf::Max, height, eps);

                let dx = self.ir.sub(px, min_x);
                let dy = self.ir.sub(py, min_y);
                let nx_raw = self.ir.div(dx, safe_w);
                let ny_raw = self.ir.div(dy, safe_h);
                let zero = self.ir.lit(0.0);
                let one_bin = self.ir.lit(0.999_999);
                let nx = self.ir.m3(Mf::Clamp, nx_raw, zero, one_bin);
                let ny = self.ir.m3(Mf::Clamp, ny_raw, zero, one_bin);

                let bins_x_f = self.ir.lit(bins_x as f32);
                let bins_y_f = self.ir.lit(bins_y as f32);
                let bx_scaled = self.ir.mul(nx, bins_x_f);
                let by_scaled = self.ir.mul(ny, bins_y_f);
                let bx = self.ir.m1(Mf::Floor, bx_scaled);
                let by = self.ir.m1(Mf::Floor, by_scaled);

                let by_offset = self.ir.mul(by, bins_x_f);
                let selected = self.ir.addx(by_offset, bx);
                self.ir.name(selected, &format!("scatter_bin_index_l{id}"));
                let time = self.ir.time;

                let occupied_bins = bins.iter().filter(|entries| !entries.is_empty()).count();
                let raw_instances = bins.iter().map(Vec::len).sum::<usize>();
                let emitted_instances = bins
                    .iter()
                    .map(|entries| entries.len().min(Self::SCATTER_BIN_RUNTIME_CAP))
                    .sum::<usize>();
                let total_footprint: f32 = bins
                    .iter()
                    .flat_map(|entries| entries.iter().take(Self::SCATTER_BIN_RUNTIME_CAP))
                    .map(|instance| instance.footprint.max(0.01))
                    .sum();
                let avg_footprint = if emitted_instances > 0 {
                    total_footprint / emitted_instances as f32
                } else {
                    0.0
                };

                let strategy_label = match strategy {
                    crate::hir::ScatterLoweringStrategy::Procedural => "procedural",
                    crate::hir::ScatterLoweringStrategy::Compact => "compact",
                    crate::hir::ScatterLoweringStrategy::BranchTree => "branch",
                };

                self.stats.scatter_occupied_bins += occupied_bins;
                if matches!(strategy, crate::hir::ScatterLoweringStrategy::Procedural) {
                    self.stats.scatter_procedural_paths += 1;
                } else if matches!(strategy, crate::hir::ScatterLoweringStrategy::Compact) {
                    self.stats.scatter_compact_paths += 1;
                } else {
                    self.stats.scatter_branch_tree_paths += 1;
                }

                if matches!(strategy, crate::hir::ScatterLoweringStrategy::Procedural) {
                    let samples_from_density = if occupied_bins > 0 {
                        raw_instances.div_ceil(occupied_bins)
                    } else {
                        1
                    };
                    // Keep a quality floor (base cap) while allowing dense scenes
                    // to scale above it in a bounded way up to the hard max.
                    let adaptive_cap =
                        if samples_from_density > Self::SCATTER_PROCEDURAL_SAMPLE_CAP_BASE {
                            let over_base =
                                samples_from_density - Self::SCATTER_PROCEDURAL_SAMPLE_CAP_BASE;
                            Self::SCATTER_PROCEDURAL_SAMPLE_CAP_BASE + over_base.div_euclid(2)
                        } else {
                            Self::SCATTER_PROCEDURAL_SAMPLE_CAP_BASE
                        };
                    let procedural_cap = adaptive_cap.clamp(
                        Self::SCATTER_PROCEDURAL_SAMPLE_CAP_BASE,
                        Self::SCATTER_PROCEDURAL_SAMPLE_CAP_HARD_MAX,
                    );
                    let procedural_samples = samples_from_density.clamp(1, procedural_cap);
                    let procedural_emitted_instances =
                        occupied_bins.saturating_mul(procedural_samples);
                    self.stats.scatter_emitted_instances += procedural_emitted_instances;
                    self.stats.scatter_procedural_samples += procedural_samples;
                    self.stats.scatter_procedural_cap += procedural_cap;
                    self.stats.scatter_layers.push(ScatterLayerStats {
                        layer_id: id,
                        strategy: strategy_label.to_string(),
                        occupied_bins,
                        emitted_instances: procedural_emitted_instances,
                        procedural_samples,
                        procedural_cap,
                    });
                    let (out, out_alpha) = self.lower_scatter_procedural_rgb(
                        id,
                        body,
                        &lifecycle,
                        p,
                        px,
                        py,
                        min_x,
                        min_y,
                        width,
                        height,
                        time,
                        selected,
                        bins_x,
                        bins_y,
                        emitted_instances,
                        procedural_samples,
                        avg_footprint,
                    );
                    self.ir.name(out, &format!("scatter_procedural_rgb_l{id}"));
                    self.ir
                        .name(out_alpha, &format!("scatter_procedural_a_l{id}"));
                    self.ir.restore_feature_tag(prev_feature_tag);
                    return (out, out_alpha);
                }

                self.stats.scatter_emitted_instances += emitted_instances;
                self.stats.scatter_layers.push(ScatterLayerStats {
                    layer_id: id,
                    strategy: strategy_label.to_string(),
                    occupied_bins,
                    emitted_instances,
                    procedural_samples: 0,
                    procedural_cap: 0,
                });

                let use_compact = matches!(strategy, crate::hir::ScatterLoweringStrategy::Compact);

                let out_init = self.ir.splat3(zero);
                let out_ptr = self.ir.local(
                    &format!("scatter_bin_out_l{id}"),
                    self.ir.types.v3,
                    Some(out_init),
                );
                let out_alpha_ptr = self.ir.local(
                    &format!("scatter_bin_out_a_l{id}"),
                    self.ir.types.f32_,
                    Some(zero),
                );

                // Keep only invariant entries created outside scatter-bin branch
                // bodies. Entries created while lowering one bin can reference
                // block-scoped temporaries and must not be reused in sibling bins.
                let invariant_sx_cache_base = self.invariant_sx_cache.clone();

                for (bin_idx, bin_entries) in bins.iter().enumerate().rev() {
                    if bin_entries.is_empty() {
                        continue;
                    }

                    // Each bin lowers inside its own conditional block. Do not
                    // reuse cached handles across bins, or WGSL emission can
                    // reference identifiers introduced in a different branch.
                    self.clear_point_sensitive_caches();
                    // Restore only the safe baseline from outside branch scopes.
                    self.invariant_sx_cache = invariant_sx_cache_base.clone();

                    let idx_f = self.ir.lit(bin_idx as f32);
                    let cond = self.ir.bin(Bo::Equal, selected, idx_f);
                    if use_compact {
                        let (bin_rgb, bin_alpha) = self.lower_scatter_bin_rgb(
                            id,
                            body,
                            &lifecycle,
                            p,
                            px,
                            py,
                            min_x,
                            min_y,
                            width,
                            height,
                            time,
                            bin_entries,
                        );
                        self.ir
                            .name(bin_rgb, &format!("scatter_bin_rgb_l{id}_b{bin_idx}"));
                        self.ir
                            .name(bin_alpha, &format!("scatter_bin_a_l{id}_b{bin_idx}"));
                        let prior = self.ir.load(out_ptr);
                        let prior_alpha = self.ir.load(out_alpha_ptr);
                        let merged = self.ir.add(Ex::Select {
                            condition: cond,
                            accept: bin_rgb,
                            reject: prior,
                        });
                        let merged_alpha = self.ir.add(Ex::Select {
                            condition: cond,
                            accept: bin_alpha,
                            reject: prior_alpha,
                        });
                        self.ir.store(out_ptr, merged);
                        self.ir.store(out_alpha_ptr, merged_alpha);
                    } else {
                        self.ir.begin_block();
                        let (bin_rgb, bin_alpha) = self.lower_scatter_bin_rgb(
                            id,
                            body,
                            &lifecycle,
                            p,
                            px,
                            py,
                            min_x,
                            min_y,
                            width,
                            height,
                            time,
                            bin_entries,
                        );
                        self.ir
                            .name(bin_rgb, &format!("scatter_bin_rgb_l{id}_b{bin_idx}"));
                        self.ir
                            .name(bin_alpha, &format!("scatter_bin_a_l{id}_b{bin_idx}"));
                        self.ir.store(out_ptr, bin_rgb);
                        self.ir.store(out_alpha_ptr, bin_alpha);
                        let accept = self.ir.end_block();

                        let reject = naga::Block::default();
                        self.ir.push_statement(naga::Statement::If {
                            condition: cond,
                            accept,
                            reject,
                        });
                    }
                }

                // Do not leak bin-local invariant cache entries to later layers.
                self.invariant_sx_cache = invariant_sx_cache_base;

                let out = self.ir.load(out_ptr);
                let out_alpha = self.ir.load(out_alpha_ptr);
                self.ir.restore_feature_tag(prev_feature_tag);
                (out, out_alpha)
            }
            Layer::InSpace { xforms, inner } => {
                if self.cellular_sample_depth == 0
                    && let Some(axis) = xforms.iter().find_map(|xf| match xf {
                        Xform::Cellular(cells) => Some(cells.samples_axis),
                        _ => None,
                    })
                {
                    let out = self.cellular_layer(id, p, axis);
                    self.ir.restore_locality(prev_locality);
                    return out;
                }
                let repeat_depth = self.repeat_cell_ctx.len();
                let prev_discontinuity_space_depth = self.discontinuity_space_depth;

                let has_discontinuity = xforms.iter().any(|xf| {
                    matches!(
                        xf,
                        Xform::RepeatX(_)
                            | Xform::Cellular(_)
                            | Xform::RepeatY(_)
                            | Xform::Repeat2D { .. }
                            | Xform::RepeatRadial { .. }
                            | Xform::Polar { .. }
                    )
                });
                if has_discontinuity {
                    self.discontinuity_space_depth += 1;
                }

                let prev_j = (
                    self.ir.jacobian_j11,
                    self.ir.jacobian_j12,
                    self.ir.jacobian_j21,
                    self.ir.jacobian_j22,
                );

                // Compose local-space Jacobian into the active footprint so AA/filtering
                // inside this block tracks explicit `in space` transforms.
                let (l11_sx, l12_sx, l21_sx, l22_sx) = crate::deriv::jacobian(&xforms);
                let l11 = self.sx_at(&l11_sx, p);
                let l12 = self.sx_at(&l12_sx, p);
                let l21 = self.sx_at(&l21_sx, p);
                let l22 = self.sx_at(&l22_sx, p);

                let (n11, n12, n21, n22) =
                    if let (Some(p11), Some(p12), Some(p21), Some(p22)) = prev_j {
                        let n11_l = self.ir.mul(l11, p11);
                        let n11_r = self.ir.mul(l12, p21);
                        let n11 = self.ir.addx(n11_l, n11_r);

                        let n12_l = self.ir.mul(l11, p12);
                        let n12_r = self.ir.mul(l12, p22);
                        let n12 = self.ir.addx(n12_l, n12_r);

                        let n21_l = self.ir.mul(l21, p11);
                        let n21_r = self.ir.mul(l22, p21);
                        let n21 = self.ir.addx(n21_l, n21_r);

                        let n22_l = self.ir.mul(l21, p12);
                        let n22_r = self.ir.mul(l22, p22);
                        let n22 = self.ir.addx(n22_l, n22_r);

                        (n11, n12, n21, n22)
                    } else {
                        (l11, l12, l21, l22)
                    };

                self.ir.jacobian_j11 = Some(n11);
                self.ir.jacobian_j12 = Some(n12);
                self.ir.jacobian_j21 = Some(n21);
                self.ir.jacobian_j22 = Some(n22);

                let p2 = self.enter_space(p, &xforms);
                let out = self.layer_color(inner, p2);

                self.ir.jacobian_j11 = prev_j.0;
                self.ir.jacobian_j12 = prev_j.1;
                self.ir.jacobian_j21 = prev_j.2;
                self.ir.jacobian_j22 = prev_j.3;

                self.repeat_cell_ctx.truncate(repeat_depth);
                self.discontinuity_space_depth = prev_discontinuity_space_depth;
                out
            }
            Layer::If {
                cond,
                then_layer,
                else_layer,
            } => {
                let cond_sx = self.sx_at(&cond, p);
                let zero = self.ir.lit(0.0);
                let cond_bool = self.ir.bin(Bo::NotEqual, cond_sx, zero);

                let zero3 = self.ir.splat3(zero);
                let out_rgb_ptr = self.ir.local("if_rgb", self.ir.types.v3, Some(zero3));
                let out_a_ptr = self.ir.local("if_a", self.ir.types.f32_, Some(zero));
                let branch_expr_start = self.ir.function.function.expressions.len();

                // Branch-local lowering is context-sensitive. Reusing cached
                // handles across branches can emit identifiers from the wrong
                // block scope in WGSL, producing unresolved-value validation errors.
                // Branches emit nested WGSL blocks. Any cache that stores
                // expression handles must be cleared so branch-local values do
                // not leak into sibling/outer scopes.
                self.clear_point_sensitive_caches();
                self.invariant_sx_cache.clear();
                self.user_call_cache.clear();
                self.path_sample_cache.clear();
                self.path_param_sample_cache.clear();
                self.gradient_dither3_cache = None;
                self.texture_sample_cache.clear();
                self.ir.begin_block();
                {
                    let (then_rgb, then_a) = self.layer_color(then_layer, p);
                    self.ir.store(out_rgb_ptr, then_rgb);
                    self.ir.store(out_a_ptr, then_a);
                }
                let accept = self.ir.end_block();

                self.clear_point_sensitive_caches();
                self.invariant_sx_cache.clear();
                self.user_call_cache.clear();
                self.path_sample_cache.clear();
                self.path_param_sample_cache.clear();
                self.gradient_dither3_cache = None;
                self.texture_sample_cache.clear();
                self.ir.begin_block();
                {
                    let (else_rgb, else_a) = self.layer_color(else_layer, p);
                    self.ir.store(out_rgb_ptr, else_rgb);
                    self.ir.store(out_a_ptr, else_a);
                }
                let reject = self.ir.end_block();

                // A context argument can mix varying coordinates and uniform frame
                // fields. WGSL validators need not track uniformity per field, so
                // even a time-only condition can be divergent at this boundary.
                // Evaluate derivative-sensitive pure layer arms before selecting
                // their result. Calls are conservative: a helper may use derivatives.
                let requires_uniform_evaluation = self
                    .ir
                    .function
                    .function
                    .expressions
                    .iter()
                    .skip(branch_expr_start)
                    .any(|(_, expression)| {
                        matches!(
                            expression,
                            Ex::Derivative { .. }
                                | Ex::CallResult(_)
                                | Ex::ImageSample {
                                    level: naga::SampleLevel::Auto | naga::SampleLevel::Bias(_),
                                    ..
                                }
                        )
                    });

                self.clear_point_sensitive_caches();
                self.invariant_sx_cache.clear();
                self.user_call_cache.clear();
                self.path_sample_cache.clear();
                self.path_param_sample_cache.clear();
                self.gradient_dither3_cache = None;
                self.texture_sample_cache.clear();

                if requires_uniform_evaluation {
                    self.ir.push_statement(naga::Statement::Block(accept));
                    let then_rgb = self.ir.load(out_rgb_ptr);
                    let then_a = self.ir.load(out_a_ptr);
                    self.ir.push_statement(naga::Statement::Block(reject));
                    let else_rgb = self.ir.load(out_rgb_ptr);
                    let else_a = self.ir.load(out_a_ptr);
                    let rgb = self.ir.add(Ex::Select {
                        condition: cond_bool,
                        accept: then_rgb,
                        reject: else_rgb,
                    });
                    let alpha = self.ir.add(Ex::Select {
                        condition: cond_bool,
                        accept: then_a,
                        reject: else_a,
                    });
                    (rgb, alpha)
                } else {
                    self.ir.push_statement(naga::Statement::If {
                        condition: cond_bool,
                        accept,
                        reject,
                    });

                    let out_rgb = self.ir.load(out_rgb_ptr);
                    let out_a = self.ir.load(out_a_ptr);
                    (out_rgb, out_a)
                }
            }
            Layer::Opacity { inner, alpha } => {
                let (rgb, a) = self.layer_color(inner, p);
                let alpha = self.sx_at(&alpha, p);
                let out_a = self.ir.mul(a, alpha);
                self.ir.name(out_a, &format!("opacity_a_l{id}"));
                (rgb, out_a)
            }
            Layer::Tint {
                inner,
                color,
                amount,
            } => {
                let (inner_rgb, inner_a) = self.layer_color(inner, p);
                let tint_rgb = self.color_expr3_at(&color, p);
                let amount = self.sx_at(&amount, p);
                let amount3 = self.ir.splat3(amount);
                let rgb = self.ir.m3(Mf::Mix, inner_rgb, tint_rgb, amount3);
                self.ir.name(rgb, &format!("tint_rgb_l{id}"));
                (rgb, inner_a)
            }
            Layer::PostProcess { inner, rgba } => {
                let (src_rgb, src_a) = self.layer_color(inner, p);
                let source_ctx = SourceColorCtx {
                    r: self.ir.x_of(src_rgb),
                    g: self.ir.y_of(src_rgb),
                    b: self.ir.add(Ex::AccessIndex {
                        base: src_rgb,
                        index: 2,
                    }),
                    a: src_a,
                };

                self.source_color_ctx.push(source_ctx);
                self.clear_point_sensitive_caches();
                let r = self.sx_at(&rgba[0], p);
                let g = self.sx_at(&rgba[1], p);
                let b = self.sx_at(&rgba[2], p);
                let a = self.sx_at(&rgba[3], p);
                self.source_color_ctx.pop();
                self.clear_point_sensitive_caches();

                let rgb = self.ir.add(Ex::Compose {
                    ty: self.ir.types.v3,
                    components: vec![r, g, b],
                });
                self.ir.name(rgb, &format!("post_rgb_l{id}"));
                self.ir.name(a, &format!("post_a_l{id}"));
                (rgb, a)
            }
            Layer::Image { tex_name } => {
                let (rgb, a) = self.sample_bound_texture(&tex_name, p);
                self.ir.name(rgb, &format!("img_rgb_l{id}"));
                self.ir.name(a, &format!("img_a_l{id}"));
                (rgb, a)
            }
            Layer::ImageAt {
                tex_name,
                sample_x,
                sample_y,
            } => {
                let x = self.sx_at(&sample_x, p);
                let y = self.sx_at(&sample_y, p);
                let sample_p = self.ir.vec2(x, y);
                let (rgb, a) = self.sample_bound_texture(&tex_name, sample_p);
                self.ir.name(rgb, &format!("img_rgb_l{id}"));
                self.ir.name(a, &format!("img_a_l{id}"));
                (rgb, a)
            }
            Layer::ColorExpr { r, g, b, a } => {
                let r = self.sx_at(&r, p);
                let g = self.sx_at(&g, p);
                let b = self.sx_at(&b, p);
                let a = self.sx_at(&a, p);
                let rgb = self.ir.add(Ex::Compose {
                    ty: self.ir.types.v3,
                    components: vec![r, g, b],
                });
                (rgb, a)
            }
            Layer::Grey { value } => {
                let v = self.sx_at(&value, p);
                let rgb = self.ir.splat3(v);
                let one = self.ir.lit(1.0);
                (rgb, one)
            }
            Layer::Compose(entries) => self.fold_compose(&entries, p),
            Layer::Blur { inner, radius, .. } => {
                let prev_feature_tag = self.ir.set_feature_tag(Some("blur"));
                // Inline N-tap box filter (§11 step 3, rung 1).
                //
                // Samples the inner layer on a `(2*HALF+1)²` grid centred on
                // `p` and averages the results.  For radii that the pass plan
                // classifies as `separable-hv` or `downsample-chain` the
                // manifest describes the correct multi-pass schedule; the inline
                // path here provides a single-pass approximation so that these
                // programs still compile and produce visible output (with
                // reduced quality for large radii).
                const HALF: i32 = crate::driver::pass_plan::INLINE_HALF_TAPS as i32;
                let radius_h = self.sx_at(&radius, p);
                let half_f = self.ir.lit(HALF as f32);
                let tap_step = self.ir.div(radius_h, half_f);

                let zero = self.ir.lit(0.0);
                let zero3 = self.ir.splat3(zero);
                let mut acc_rgb = zero3;
                let mut acc_alpha = zero;
                let total = ((2 * HALF + 1) * (2 * HALF + 1)) as f32;

                for dy in -HALF..=HALF {
                    for dx in -HALF..=HALF {
                        let dx_f = self.ir.lit(dx as f32);
                        let ox = self.ir.mul(dx_f, tap_step);
                        let dy_f = self.ir.lit(dy as f32);
                        let oy = self.ir.mul(dy_f, tap_step);
                        let off = self.ir.vec2(ox, oy);
                        let tp = self.ir.addx(p, off);
                        // Clear caches so each tap evaluates independently at
                        // its own coordinate.
                        self.clear_point_sensitive_caches();
                        let (rgb, a) = self.layer_color(inner, tp);
                        let a3 = self.ir.splat3(a);
                        let premul = self.ir.mul(rgb, a3);
                        acc_rgb = self.ir.addx(acc_rgb, premul);
                        acc_alpha = self.ir.addx(acc_alpha, a);
                    }
                }
                // Caches cleared inside the tap loop; restore them to a clean
                // state for any lowering that follows.
                self.clear_point_sensitive_caches();

                let n = self.ir.lit(total);
                let n3 = self.ir.splat3(n);
                let avg_rgb = self.ir.div(acc_rgb, n3);
                let avg_alpha = self.ir.div(acc_alpha, n);
                self.ir.name(avg_rgb, &format!("blur_rgb_l{id}"));
                self.ir.name(avg_alpha, &format!("blur_a_l{id}"));
                self.ir.restore_feature_tag(prev_feature_tag);
                (avg_rgb, avg_alpha)
            }
            Layer::MotionBlur { inner, shutter, .. } => {
                let prev_feature_tag = self.ir.set_feature_tag(Some("motion_blur"));
                // Temporal shutter integration approximation.
                // Samples `inner` at staggered times across the shutter window,
                // at the same coordinate.
                const HALF: i32 = crate::driver::pass_plan::INLINE_HALF_TAPS as i32;
                let sample_count = (2 * HALF + 1) as f32;
                let n = self.ir.lit(sample_count);
                let shutter_h = self.sx_at(&shutter, p);
                let eps = self.ir.lit(1.0e-6);
                let safe_delta = self.ir.m2(Mf::Max, self.ir.delta, eps);
                let frame_div = self.ir.div(self.ir.time, safe_delta);
                let frame_index = self.ir.m1(Mf::Floor, frame_div);
                let px = self.ir.x_of(p);
                let py = self.ir.y_of(p);
                let ign_kx = self.ir.lit(0.06711056);
                let ign_ky = self.ir.lit(0.00583715);
                let ign_mul = self.ir.lit(52.982_918);
                let jitter_frame_weight = self.ir.lit(0.754_877_7);
                let jitter_index_weight = self.ir.lit(0.5698403);

                let zero = self.ir.lit(0.0);
                let zero3 = self.ir.splat3(zero);
                let mut acc_rgb = zero3;
                let mut acc_alpha = zero;

                for i in -HALF..=HALF {
                    let k = self.ir.lit((i + HALF) as f32);
                    let ign_dot_x = self.ir.mul(px, ign_kx);
                    let ign_dot_y = self.ir.mul(py, ign_ky);
                    let ign_dot = self.ir.addx(ign_dot_x, ign_dot_y);
                    let ign_seed = self.ir.m1(Mf::Fract, ign_dot);
                    let jitter_frame_term = self.ir.mul(frame_index, jitter_frame_weight);
                    let jitter_index_term = self.ir.mul(k, jitter_index_weight);
                    let jitter_mix = self.ir.addx(jitter_frame_term, jitter_index_term);
                    let jitter_phase = self.ir.addx(ign_seed, jitter_mix);
                    let jitter_phase_frac = self.ir.m1(Mf::Fract, jitter_phase);
                    let jitter_scaled = self.ir.mul(ign_mul, jitter_phase_frac);
                    let jitter = self.ir.m1(Mf::Fract, jitter_scaled);
                    let k_jittered = self.ir.addx(k, jitter);
                    let frac = self.ir.div(k_jittered, n);
                    let dt = self.ir.mul(shutter_h, frac);
                    let tau = self.ir.sub(self.ir.time, dt);

                    let saved_time_override = self.ir.time_override;
                    self.ir.time_override = Some(tau);

                    self.clear_point_sensitive_caches();
                    let (rgb, a) = self.layer_color(inner, p);
                    self.ir.time_override = saved_time_override;

                    let a3 = self.ir.splat3(a);
                    let premul = self.ir.mul(rgb, a3);
                    acc_rgb = self.ir.addx(acc_rgb, premul);
                    acc_alpha = self.ir.addx(acc_alpha, a);
                }

                self.clear_point_sensitive_caches();

                let n3 = self.ir.splat3(n);
                let avg_rgb = self.ir.div(acc_rgb, n3);
                let avg_alpha = self.ir.div(acc_alpha, n);
                self.ir.name(avg_rgb, &format!("motion_blur_rgb_l{id}"));
                self.ir.name(avg_alpha, &format!("motion_blur_a_l{id}"));
                self.ir.restore_feature_tag(prev_feature_tag);
                (avg_rgb, avg_alpha)
            }
            Layer::UserEffect {
                def_idx,
                inner,
                args,
                ..
            } => {
                // Lower a user-defined effect by inlining its body layer with
                // actual argument values substituted for symbolic `Sx::Var(name)`
                // parameters.
                //
                // Point effects: inline the body directly at `p`.
                // Local/Global effects: currently lowered the same as point
                // (pass-based lowering for Local/Global is a future concern).
                let body_layer = self.hir.effects[def_idx].body_layer;
                let param_names: Vec<String> = self.hir.effects[def_idx].param_names.clone();

                // Symbolic `Sx::Var` references inside the body are resolved
                // when `sx_at` encounters them via the `Sx::Var` arm in
                // `lower/scalar.rs`. Actual arg values are pushed as overrides
                // before lowering and popped after.

                // Evaluate args first (before pushing overrides to avoid self-referential issues).
                let evaluated_args: Vec<Handle<Ex>> =
                    args.iter().map(|sx| self.sx_at(sx, p)).collect();

                // Push var overrides.
                for (name, handle) in param_names.iter().zip(evaluated_args.iter()) {
                    self.push_var_override(name.clone(), *handle);
                }

                if let Some(inner_id) = inner {
                    self.effect_input_layer_ctx.push(inner_id);
                    self.clear_point_sensitive_caches();
                }

                let (rgb, a) = self.layer_color(body_layer, p);

                if inner.is_some() {
                    self.effect_input_layer_ctx.pop();
                    self.clear_point_sensitive_caches();
                }

                // Pop var overrides in reverse.
                for name in param_names.iter().rev() {
                    self.pop_var_override(name);
                }

                self.ir
                    .name(rgb, &format!("user_effect_{def_idx}_rgb_l{id}"));
                self.ir.name(a, &format!("user_effect_{def_idx}_a_l{id}"));
                (rgb, a)
            }
        };
        self.ir.restore_locality(prev_locality);
        out
    }

    /// The painter's stack becomes a fold over blend ops (§10.3).
    fn fold_compose_impl(
        &mut self,
        entries: &[(LayerId, Blend)],
        p: Handle<Ex>,
        track_alpha: bool,
    ) -> (Handle<Ex>, Handle<Ex>) {
        let zero = self.ir.lit(0.0);
        let mut col = self.ir.splat3(zero);
        let mut alpha = zero;
        let one = self.ir.lit(1.0);
        for (index, (layer, blend)) in entries.iter().enumerate() {
            let (rgb, a) = self.layer_color(*layer, p);
            let af = self.ir.splat3(a);
            col = match blend {
                Blend::Over => {
                    let mixed = self.ir.m3(Mf::Mix, col, rgb, af);
                    self.ir
                        .name(mixed, &format!("compose_over_s{index}_l{layer}"));
                    if track_alpha {
                        let inv_alpha = self.ir.sub(one, alpha);
                        let src_contrib = self.ir.mul(a, inv_alpha);
                        alpha = self.ir.addx(alpha, src_contrib);
                        self.ir
                            .name(alpha, &format!("compose_over_a_s{index}_l{layer}"));
                    }
                    mixed
                }
                Blend::Add => {
                    let src = self.ir.mul(rgb, af);
                    self.ir.name(src, &format!("add_src_l{layer}"));
                    let added = self.ir.addx(col, src);
                    self.ir
                        .name(added, &format!("compose_add_s{index}_l{layer}"));
                    if track_alpha {
                        let alpha_add = self.ir.addx(alpha, a);
                        alpha = self.ir.m3(Mf::Clamp, alpha_add, zero, one);
                        self.ir
                            .name(alpha, &format!("compose_add_a_s{index}_l{layer}"));
                    }
                    added
                }
                Blend::Screen => {
                    let one3 = self.ir.splat3(one);
                    self.ir.name(one3, "screen_one");
                    let src = self.ir.mul(rgb, af);
                    self.ir.name(src, &format!("screen_src_l{layer}"));
                    let ic = self.ir.sub(one3, col);
                    self.ir.name(ic, &format!("screen_inv_dst_l{layer}"));
                    let is = self.ir.sub(one3, src);
                    self.ir.name(is, &format!("screen_inv_src_l{layer}"));
                    let m = self.ir.mul(ic, is);
                    self.ir.name(m, &format!("screen_mul_l{layer}"));
                    let screened = self.ir.sub(one3, m);
                    self.ir
                        .name(screened, &format!("compose_screen_s{index}_l{layer}"));
                    if track_alpha {
                        let inv_alpha = self.ir.sub(one, alpha);
                        let src_contrib = self.ir.mul(a, inv_alpha);
                        alpha = self.ir.addx(alpha, src_contrib);
                        self.ir
                            .name(alpha, &format!("compose_screen_a_s{index}_l{layer}"));
                    }
                    screened
                }
                Blend::Multiply => {
                    let m = self.ir.mul(col, rgb);
                    self.ir.name(m, &format!("mul_rgb_l{layer}"));
                    let mixed = self.ir.m3(Mf::Mix, col, m, af);
                    self.ir
                        .name(mixed, &format!("compose_mul_s{index}_l{layer}"));
                    if track_alpha {
                        let inv_alpha = self.ir.sub(one, alpha);
                        let src_contrib = self.ir.mul(a, inv_alpha);
                        alpha = self.ir.addx(alpha, src_contrib);
                        self.ir
                            .name(alpha, &format!("compose_mul_a_s{index}_l{layer}"));
                    }
                    mixed
                }
            };
        }
        (col, alpha)
    }

    /// The painter's stack becomes a fold over blend ops (§10.3).
    pub(super) fn fold_compose(
        &mut self,
        entries: &[(LayerId, Blend)],
        p: Handle<Ex>,
    ) -> (Handle<Ex>, Handle<Ex>) {
        self.fold_compose_impl(entries, p, true)
    }

    pub(super) fn fold_compose_rgb_only(
        &mut self,
        entries: &[(LayerId, Blend)],
        p: Handle<Ex>,
    ) -> Handle<Ex> {
        self.fold_compose_impl(entries, p, false).0
    }
}
