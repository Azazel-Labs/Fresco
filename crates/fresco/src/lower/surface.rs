use super::canvas::lower_user_helper_fn;
use super::*;
use crate::hir::Hir;
use crate::material_hir::MaterialHir;
use std::collections::{BTreeMap, BTreeSet};

struct SurfaceInputs {
    argument: Handle<Ex>,
    uv: Handle<Ex>,
    time: Handle<Ex>,
    res: Handle<Ex>,
    scalars: HashMap<String, Handle<Ex>>,
}

fn surface_inputs(
    function: &mut FunctionBuilder,
    material: &MaterialHir,
    t: &TypeHandles,
) -> SurfaceInputs {
    let argument = function.arg(
        "context",
        t.entry_context.expect("registered engine context"),
    );
    let project = |function: &mut FunctionBuilder, path: &[u32]| {
        path.iter().fold(argument, |base, &index| {
            function.expr(Ex::AccessIndex { base, index })
        })
    };
    let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
    let zero2 = function.expr(Ex::Compose {
        ty: t.v2,
        components: vec![zero, zero],
    });
    let uv = material
        .context
        .roles
        .get("coord")
        .map_or(zero2, |path| project(function, path));
    let time = material
        .context
        .roles
        .get("time")
        .map_or(zero, |path| project(function, path));
    let res = material
        .context
        .roles
        .get("resolution")
        .map_or(zero2, |path| project(function, path));
    let scalars = material
        .context
        .components
        .iter()
        .map(|component| (component.name.clone(), project(function, &component.path)))
        .collect();
    SurfaceInputs {
        argument,
        uv,
        time,
        res,
        scalars,
    }
}

pub(super) fn lower_surfaces_with_policy(
    material_hirs: &[MaterialHir],
    module: &mut naga::Module,
    t: &TypeHandles,
    existing_tex_bindings: &HashMap<String, u32>,
    existing_param_bindings: &mut HashMap<String, u32>,
    policy: LoweringPolicy,
    global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
) -> Vec<Stats> {
    if material_hirs.is_empty() {
        return Vec::new();
    }

    let mut tex_globals = HashMap::new();
    for name in existing_tex_bindings.keys() {
        if let Some((handle, _)) = module
            .global_variables
            .iter()
            .find(|(_, global)| global.name.as_deref() == Some(&format!("t_{name}")))
        {
            tex_globals.insert(name.clone(), handle);
        }
    }
    let sampler_global = module.global_variables.iter().find_map(|(handle, global)| {
        (global.name.as_deref() == Some("fresco_sampler")).then_some(handle)
    });

    let mut stats = Vec::new();
    let mut next_param_binding = existing_param_bindings
        .values()
        .max()
        .map(|v| v + 1)
        .unwrap_or(0);

    for material in material_hirs {
        let mut types = *t;
        let mut builder = ModuleBuilder {
            module: std::mem::take(module),
            function_dedup_cache: HashMap::new(),
        };
        types.entry_context = Some(
            builder
                .register_context_type(&material.context.ty, &types)
                .0,
        );
        *module = builder.module;
        let t = &types;
        let material_ty = ensure_fresco_material_type(module, t, material);
        let mut material_types = HashMap::new();
        for channel in &material.material_channels {
            register_material_type(module, t, material, &channel.ty_name, &mut material_types);
        }
        let bridge_hir = bridge_hir(material);
        let param_globals = create_surface_param_globals(
            module,
            t,
            material,
            existing_param_bindings,
            &mut next_param_binding,
        );

        let mut user_helper_fns: HashMap<String, Handle<naga::Function>> = HashMap::new();
        let mut pending: BTreeMap<String, _> = bridge_hir
            .user_helpers
            .iter()
            .map(|(id, helper)| (id.clone(), helper))
            .collect();

        while !pending.is_empty() {
            let ids: Vec<String> = pending.keys().cloned().collect();
            let mut progressed = false;
            for id in ids {
                let helper = *pending
                    .get(&id)
                    .expect("pending helper id vanished during surface lowering");
                let mut deps = BTreeSet::new();
                for stmt in &helper.body_stmts {
                    collect_helper_ids_from_stmt(stmt, &mut deps);
                }
                if deps
                    .iter()
                    .all(|dep| dep == &id || !pending.contains_key(dep))
                {
                    let f = lower_user_helper_fn(
                        &bridge_hir,
                        helper,
                        t,
                        policy,
                        user_helper_fns.clone(),
                        HashMap::new(),
                        global_uniform_globals,
                    );
                    let handle = module.functions.append(f, naga::Span::UNDEFINED);
                    user_helper_fns.insert(id.clone(), handle);
                    pending.remove(&id);
                    progressed = true;
                }
            }
            if !progressed {
                let id = pending
                    .keys()
                    .next()
                    .cloned()
                    .expect("pending helpers unexpectedly empty");
                let helper = pending.remove(&id).expect("pending helper vanished");
                let f = lower_user_helper_fn(
                    &bridge_hir,
                    helper,
                    t,
                    policy,
                    user_helper_fns.clone(),
                    HashMap::new(),
                    global_uniform_globals,
                );
                let handle = module.functions.append(f, naga::Span::UNDEFINED);
                user_helper_fns.insert(id, handle);
            }
        }

        let response_function = lower_surface_response_fn(
            &bridge_hir,
            material,
            material_ty,
            t,
            policy,
            &tex_globals,
            sampler_global,
            &param_globals,
            user_helper_fns.clone(),
            global_uniform_globals,
            material_types,
        );
        let response_fn = module
            .functions
            .append(response_function, naga::Span::UNDEFINED);

        let function = lower_surface_fn(material, material_ty, t, policy, response_fn);
        module.functions.append(function, naga::Span::UNDEFINED);

        if let Some(vertex_fn) = lower_surface_vertex_fn(SurfaceVertexInputs {
            bridge_hir: &bridge_hir,
            material,
            t,
            policy,
            param_globals: &param_globals,
            user_helper_fns,
            global_uniform_globals,
        }) {
            module.functions.append(vertex_fn, naga::Span::UNDEFINED);
        }
        stats.push(Stats::default());
    }

    stats
}

struct SurfaceVertexInputs<'a> {
    bridge_hir: &'a Hir,
    material: &'a MaterialHir,
    t: &'a TypeHandles,
    policy: LoweringPolicy,
    param_globals: &'a HashMap<String, SurfaceParamGlobal>,
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    global_uniform_globals: &'a HashMap<String, Handle<naga::GlobalVariable>>,
}

fn lower_surface_vertex_fn(input: SurfaceVertexInputs<'_>) -> Option<naga::Function> {
    let SurfaceVertexInputs {
        bridge_hir,
        material,
        t,
        policy,
        param_globals,
        user_helper_fns,
        global_uniform_globals,
    } = input;

    let vertex_program = material.vertex_program.as_ref()?;

    let mut function =
        FunctionBuilder::new(format!("fresco_surface_vertex_{}", material.name), policy);
    let SurfaceInputs {
        argument,
        uv,
        time,
        res,
        scalars,
    } = surface_inputs(&mut function, material, t);
    let surface_vertex_ty = t.entry_context.expect("registered engine context");
    let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
    let one = function.expr(Ex::Literal(naga::Literal::F32(1.0)));

    let mut param_scalars = scalars;
    let mut param_scalar_ptrs = HashSet::new();
    for (name, global) in param_globals {
        match global {
            SurfaceParamGlobal::Scalar(handle) => {
                let ptr = function.expr(Ex::GlobalVariable(*handle));
                param_scalars.insert(name.clone(), ptr);
                param_scalar_ptrs.insert(ptr);
            }
            SurfaceParamGlobal::Color(handle) => {
                let base = function.expr(Ex::GlobalVariable(*handle));
                for (index, suffix) in [(0_u32, "r"), (1, "g"), (2, "b"), (3, "a")] {
                    let ptr = function.expr(Ex::AccessIndex { base, index });
                    param_scalars.insert(format!("{name}.{suffix}"), ptr);
                    param_scalar_ptrs.insert(ptr);
                }
            }
        }
    }

    function.set_result(surface_vertex_ty);

    let mut cx = FnCtx {
        hir: bridge_hir,
        ir: IrBuilder {
            function,
            types: *t,
            uv,
            time,
            time_override: None,
            delta: zero,
            res,
            px: one,
            aa: one,
            jacobian_j11: None,
            jacobian_j12: None,
            jacobian_j21: None,
            jacobian_j22: None,
            param_scalars,
            param_scalar_ptrs,
            tex_globals: HashMap::new(),
            sampler_global: None,
            param_storage_globals: HashMap::new(),
            global_uniform_globals: global_uniform_globals.clone(),
        },
        params_are_immutable: true,
        current_pass_id: None,
        layer_to_pass: Vec::new(),
        pass_output_target_names: HashMap::new(),
        sdf_cache: HashMap::new(),
        sx_cache: HashMap::new(),
        contour_cache: HashMap::new(),
        invariant_sx_cache: HashMap::new(),
        invariant_user_helper_cache: HashMap::new(),
        local_pixel_span_cache: HashMap::new(),
        shape_anchor_cache: HashMap::new(),
        shape_half_extent_cache: HashMap::new(),
        shape_local_point_cache: HashMap::new(),
        shape_anchor_space_point_cache: HashMap::new(),
        shape_glow_warp_cache: HashMap::new(),
        source_color_ctx: Vec::new(),
        effect_input_layer_ctx: Vec::new(),
        scatter_instance_ctx: Vec::new(),
        repeat_cell_ctx: Vec::new(),
        cellular_sample_depth: 0,
        discontinuity_space_depth: 0,
        box_filter_domains: HashMap::new(),
        scatter_body_fns: HashMap::new(),
        user_helper_fns,
        user_call_cache: HashMap::new(),
        path_helper_fns: HashMap::new(),
        path_sample_cache: HashMap::new(),
        path_param_sample_cache: HashMap::new(),
        texture_sample_cache: HashMap::new(),
        gradient_dither3_cache: None,
        color_shape: None,
        cell_inset_pixel_span: None,
        var_overrides: Vec::new(),
        stats: Stats::default(),
    };

    let mut components = Vec::new();
    for (index, field) in material.context.ty.fields.iter().enumerate() {
        let value = if let Some((_, values)) = vertex_program
            .fields
            .iter()
            .find(|(name, _)| name == &field.name)
        {
            let values = values
                .iter()
                .map(|sx| cx.native_scalar(sx, uv))
                .collect::<Vec<_>>();
            match values.as_slice() {
                [value] => *value,
                _ => {
                    let ty = match values.len() {
                        2 => t.v2,
                        3 => t.v3,
                        4 => t.v4,
                        _ => unreachable!("checked vertex output width"),
                    };
                    cx.ir.add(Ex::Compose {
                        ty,
                        components: values,
                    })
                }
            }
        } else {
            cx.ir.add(Ex::AccessIndex {
                base: argument,
                index: u32::try_from(index).expect("context index fits backend"),
            })
        };
        components.push(value);
    }
    let out = cx.ir.add(Ex::Compose {
        ty: surface_vertex_ty,
        components,
    });
    cx.ir.return_value(out);
    Some(cx.ir.finish())
}

fn lower_surface_fn(
    material: &MaterialHir,
    material_ty: Handle<naga::Type>,
    t: &TypeHandles,
    policy: LoweringPolicy,
    response_fn: Handle<naga::Function>,
) -> naga::Function {
    let mut function = FunctionBuilder::new(format!("fresco_{}", material.name), policy);
    let argument = function.arg(
        "context",
        t.entry_context.expect("registered engine context"),
    );
    function.set_result(material_ty);
    let result = function.expr(Ex::CallResult(response_fn));
    function.push_statement(naga::Statement::Call {
        function: response_fn,
        arguments: vec![argument],
        result: Some(result),
    });
    function.return_value(result);
    function.finish()
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
fn lower_surface_response_fn(
    bridge_hir: &Hir,
    material: &MaterialHir,
    material_ty: Handle<naga::Type>,
    t: &TypeHandles,
    policy: LoweringPolicy,
    tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    sampler_global: Option<Handle<naga::GlobalVariable>>,
    param_globals: &HashMap<String, SurfaceParamGlobal>,
    user_helper_fns: HashMap<String, Handle<naga::Function>>,
    global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    material_types: HashMap<String, Handle<naga::Type>>,
) -> naga::Function {
    let mut function = FunctionBuilder::new(
        format!("fresco_surface_response_{}", material.surface_shader_name),
        policy,
    );
    let SurfaceInputs {
        uv,
        time,
        res,
        scalars,
        ..
    } = surface_inputs(&mut function, material, t);
    let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
    let one = function.expr(Ex::Literal(naga::Literal::F32(1.0)));

    let mut param_scalars = scalars;
    let mut param_scalar_ptrs = HashSet::new();
    for (name, global) in param_globals {
        match global {
            SurfaceParamGlobal::Scalar(handle) => {
                let ptr = function.expr(Ex::GlobalVariable(*handle));
                param_scalars.insert(name.clone(), ptr);
                param_scalar_ptrs.insert(ptr);
            }
            SurfaceParamGlobal::Color(handle) => {
                let base = function.expr(Ex::GlobalVariable(*handle));
                for (index, suffix) in [(0_u32, "r"), (1, "g"), (2, "b"), (3, "a")] {
                    let ptr = function.expr(Ex::AccessIndex { base, index });
                    param_scalars.insert(format!("{name}.{suffix}"), ptr);
                    param_scalar_ptrs.insert(ptr);
                }
            }
        }
    }

    function.set_result(material_ty);

    let mut cx = FnCtx {
        hir: bridge_hir,
        ir: IrBuilder {
            function,
            types: *t,
            uv,
            time,
            time_override: None,
            delta: zero,
            res,
            px: one,
            aa: one,
            jacobian_j11: None,
            jacobian_j12: None,
            jacobian_j21: None,
            jacobian_j22: None,
            param_scalars,
            param_scalar_ptrs,
            tex_globals: tex_globals.clone(),
            sampler_global,
            param_storage_globals: HashMap::new(),
            global_uniform_globals: global_uniform_globals.clone(),
        },
        params_are_immutable: true,
        current_pass_id: None,
        layer_to_pass: Vec::new(),
        pass_output_target_names: HashMap::new(),
        sdf_cache: HashMap::new(),
        sx_cache: HashMap::new(),
        contour_cache: HashMap::new(),
        invariant_sx_cache: HashMap::new(),
        invariant_user_helper_cache: HashMap::new(),
        local_pixel_span_cache: HashMap::new(),
        shape_anchor_cache: HashMap::new(),
        shape_half_extent_cache: HashMap::new(),
        shape_local_point_cache: HashMap::new(),
        shape_anchor_space_point_cache: HashMap::new(),
        shape_glow_warp_cache: HashMap::new(),
        source_color_ctx: Vec::new(),
        effect_input_layer_ctx: Vec::new(),
        scatter_instance_ctx: Vec::new(),
        repeat_cell_ctx: Vec::new(),
        cellular_sample_depth: 0,
        discontinuity_space_depth: 0,
        box_filter_domains: HashMap::new(),
        scatter_body_fns: HashMap::new(),
        user_helper_fns,
        user_call_cache: HashMap::new(),
        path_helper_fns: HashMap::new(),
        path_sample_cache: HashMap::new(),
        path_param_sample_cache: HashMap::new(),
        texture_sample_cache: HashMap::new(),
        gradient_dither3_cache: None,
        color_shape: None,
        cell_inset_pixel_span: None,
        var_overrides: Vec::new(),
        stats: Stats::default(),
    };

    let mut components = Vec::with_capacity(material.material_channels.len());
    for channel in &material.material_channels {
        let ty = material_types[crate::check::strip_spatial_type_suffix(&channel.ty_name)];
        let field = cx.ir.local(&channel.name, ty, None);
        let mut initialized = false;
        for payload in material
            .channel_defaults
            .get(&channel.name)
            .into_iter()
            .chain(
                material
                    .layers
                    .iter()
                    .filter_map(|layer| layer.channels.get(&channel.name)),
            )
        {
            let value = lower_material_value(
                &mut cx,
                payload,
                &channel.ty_name,
                material,
                &material_types,
                uv,
            );
            cx.ir.store(field, value);
            initialized = true;
        }
        assert!(
            initialized,
            "material field must have a checked initial value"
        );
        components.push(cx.ir.load(field));
    }
    let out = cx.ir.add(Ex::Compose {
        ty: material_ty,
        components,
    });
    cx.ir.return_value(out);
    cx.ir.finish()
}

fn bridge_hir(material: &MaterialHir) -> Hir {
    Hir {
        entry_context: Some(material.context.clone()),
        name: material.name.clone(),
        params: material.params.clone(),
        rendering_policy: material.rendering_policy,
        canvas_space: None,
        canvas_jacobian: None,
        shapes: Vec::new(),
        layers: Vec::new(),
        layer_locality: Vec::new(),
        root: 0,
        notes: material.notes.clone(),
        specialization_notes: Vec::new(),
        textures: material.textures.clone(),
        texture_index: material.texture_index.clone(),
        texture_metadata: HashMap::new(),
        texture_type_defs: HashMap::new(),
        user_helpers: material.user_helpers.clone(),
        path_profiles: Vec::new(),
        effects: Vec::new(),
        effect_by_name: HashMap::new(),
        global_uniforms: Vec::new(),
    }
}

fn ensure_fresco_material_type(
    module: &mut naga::Module,
    t: &TypeHandles,
    material: &MaterialHir,
) -> Handle<naga::Type> {
    let mut types = HashMap::new();
    for channel in &material.material_channels {
        register_material_type(module, t, material, &channel.ty_name, &mut types);
    }
    let mut members = Vec::new();
    let mut offset = 0u32;
    let mut alignment = 1u32;
    for channel in &material.material_channels {
        let ty = types[crate::check::strip_spatial_type_suffix(&channel.ty_name)];
        let (align, size) = material_type_layout(module, ty);
        alignment = alignment.max(align);
        offset = offset.next_multiple_of(align);
        members.push(naga::StructMember {
            name: Some(channel.name.clone()),
            ty,
            binding: None,
            offset,
        });
        offset = offset.checked_add(size).expect("checked material layout");
    }
    module.types.insert(
        naga::Type {
            name: Some(format!("FrescoMaterial_{}", material.name)),
            inner: naga::TypeInner::Struct {
                members,
                span: offset.next_multiple_of(alignment),
            },
        },
        naga::Span::UNDEFINED,
    )
}

fn material_type_layout(module: &naga::Module, ty: Handle<naga::Type>) -> (u32, u32) {
    let mut layout = naga::proc::Layouter::default();
    layout
        .update(module.to_ctx())
        .expect("checked material value types");
    let item = layout[ty];
    (item.alignment * 1, item.size)
}

fn register_material_type(
    module: &mut naga::Module,
    t: &TypeHandles,
    material: &MaterialHir,
    name: &str,
    types: &mut HashMap<String, Handle<naga::Type>>,
) -> Handle<naga::Type> {
    let name = crate::check::strip_spatial_type_suffix(name);
    if let Some(ty) = types.get(name) {
        return *ty;
    }
    let primitive = match name {
        "f32" | "f64" | "half" | "angle" | "length" => Some(t.f32_),
        "i32" => Some(t.i32_),
        "u32" => Some(t.u32_),
        "bool" => Some(t.bool_),
        "vec2" => Some(t.v2),
        "vec3" => Some(t.v3),
        "vec4" | "color" => Some(t.v4),
        _ => None,
    };
    let ty = if let Some(ty) = primitive {
        ty
    } else if matches!(name, "mat2" | "mat3" | "mat4") {
        let size = match name {
            "mat2" => VectorSize::Bi,
            "mat3" => VectorSize::Tri,
            _ => VectorSize::Quad,
        };
        module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Matrix {
                    columns: size,
                    rows: size,
                    scalar: naga::Scalar::F32,
                },
            },
            naga::Span::UNDEFINED,
        )
    } else if let Some((element, length)) = crate::hir::parse_array_param_type(name) {
        let base = register_material_type(module, t, material, element, types);
        let (alignment, size) = material_type_layout(module, base);
        module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Array {
                    base,
                    size: naga::ArraySize::Constant(
                        u32::try_from(length)
                            .expect("checked array length")
                            .try_into()
                            .expect("nonempty array"),
                    ),
                    stride: size.next_multiple_of(alignment),
                },
            },
            naga::Span::UNDEFINED,
        )
    } else {
        let (index, record) = material
            .record_types
            .iter()
            .enumerate()
            .find(|(_, r)| r.name == name)
            .expect("checked record type");
        let mut members = Vec::new();
        let mut offset = 0u32;
        let mut alignment = 1u32;
        for field in &record.fields {
            let ty = register_material_type(module, t, material, &field.ty_name, types);
            let (align, size) = material_type_layout(module, ty);
            alignment = alignment.max(align);
            offset = offset.next_multiple_of(align);
            members.push(naga::StructMember {
                name: Some(field.name.clone()),
                ty,
                binding: None,
                offset,
            });
            offset = offset.checked_add(size).expect("checked record size");
        }
        module.types.insert(
            naga::Type {
                name: Some(if material.context.contains_record(name) {
                    name.to_string()
                } else {
                    format!("FrescoMaterial_{}_type_{index}", material.name)
                }),
                inner: naga::TypeInner::Struct {
                    members,
                    span: offset.next_multiple_of(alignment),
                },
            },
            naga::Span::UNDEFINED,
        )
    };
    types.insert(name.into(), ty);
    ty
}

fn lower_material_value(
    cx: &mut FnCtx<'_>,
    value: &crate::material_hir::MaterialValue,
    name: &str,
    material: &MaterialHir,
    types: &HashMap<String, Handle<naga::Type>>,
    uv: Handle<Ex>,
) -> Handle<Ex> {
    use crate::material_hir::MaterialValue;
    let name = crate::check::strip_spatial_type_suffix(name);
    let ty = types[name];
    let mut scalar = |value: &Sx| cx.sx_at(value, uv);
    let values = match value {
        MaterialValue::Scalar(value) => {
            if matches!(value, Sx::Typed(_)) {
                let expected =
                    crate::typed_scalar::Kind::parse(name).expect("scalar material type");
                let value = crate::typed_scalar::Scalar::cast(value.clone(), expected);
                return cx.native_scalar(&value, uv);
            }
            let value = scalar(value);
            return match name {
                "i32" | "u32" => cx.ir.add(Ex::As {
                    expr: value,
                    kind: if name == "i32" {
                        naga::ScalarKind::Sint
                    } else {
                        naga::ScalarKind::Uint
                    },
                    convert: Some(4),
                }),
                "bool" => {
                    let zero = cx.ir.add(Ex::Literal(naga::Literal::F32(0.0)));
                    cx.ir.add(Ex::Binary {
                        op: naga::BinaryOperator::NotEqual,
                        left: value,
                        right: zero,
                    })
                }
                _ => value,
            };
        }
        MaterialValue::Vector(values) => values
            .iter()
            .map(|value| cx.native_scalar(value, uv))
            .collect(),
        MaterialValue::Matrix(columns) => columns
            .iter()
            .map(|column| {
                let ty = match column.len() {
                    2 => cx.ir.types.v2,
                    3 => cx.ir.types.v3,
                    4 => cx.ir.types.v4,
                    _ => unreachable!("checked matrix dimension"),
                };
                let components = column.iter().map(|v| cx.sx_at(v, uv)).collect();
                cx.ir.add(Ex::Compose { ty, components })
            })
            .collect(),
        MaterialValue::Array(values) => {
            let (element, _) = crate::hir::parse_array_param_type(name).expect("checked array");
            values
                .iter()
                .map(|v| lower_material_value(cx, v, element, material, types, uv))
                .collect()
        }
        MaterialValue::Record(fields) => material
            .record_types
            .iter()
            .find(|r| r.name == name)
            .expect("checked record")
            .fields
            .iter()
            .map(|f| lower_material_value(cx, &fields[&f.name], &f.ty_name, material, types, uv))
            .collect(),
    };
    cx.ir.add(Ex::Compose {
        ty,
        components: values,
    })
}

enum SurfaceParamGlobal {
    Scalar(Handle<naga::GlobalVariable>),
    Color(Handle<naga::GlobalVariable>),
}

fn create_surface_param_globals(
    module: &mut naga::Module,
    t: &TypeHandles,
    material: &MaterialHir,
    existing_param_bindings: &mut HashMap<String, u32>,
    next_param_binding: &mut u32,
) -> HashMap<String, SurfaceParamGlobal> {
    let mut out = HashMap::new();
    for param in &material.params {
        let key = format!("{}:{}", material.name, param.name);
        let binding = *existing_param_bindings.entry(key).or_insert_with(|| {
            let current = *next_param_binding;
            *next_param_binding += 1;
            current
        });
        match param.ty_name.as_str() {
            "color" => {
                let global = module.global_variables.append(
                    naga::GlobalVariable {
                        name: Some(format!("fresco_param_{}_{}", material.name, param.name)),
                        space: naga::AddressSpace::Uniform,
                        binding: Some(naga::ResourceBinding { group: 0, binding }),
                        ty: t.v4,
                        init: None,
                        memory_decorations: naga::MemoryDecorations::empty(),
                    },
                    naga::Span::UNDEFINED,
                );
                out.insert(param.name.clone(), SurfaceParamGlobal::Color(global));
            }
            _ => {
                let global = module.global_variables.append(
                    naga::GlobalVariable {
                        name: Some(format!("fresco_param_{}_{}", material.name, param.name)),
                        space: naga::AddressSpace::Uniform,
                        binding: Some(naga::ResourceBinding { group: 0, binding }),
                        ty: t.f32_,
                        init: None,
                        memory_decorations: naga::MemoryDecorations::empty(),
                    },
                    naga::Span::UNDEFINED,
                );
                out.insert(param.name.clone(), SurfaceParamGlobal::Scalar(global));
            }
        }
    }
    out
}
