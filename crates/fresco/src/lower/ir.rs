use super::*;
use std::collections::{HashMap, HashSet};

const SP: naga::Span = naga::Span::UNDEFINED;

fn project_context(
    function: &mut FunctionBuilder,
    mut base: Handle<Ex>,
    path: &[u32],
) -> Handle<Ex> {
    for &index in path {
        base = function.expr(Ex::AccessIndex { base, index });
    }
    base
}

impl LoweringPolicy {
    pub(super) fn from_env() -> Self {
        match std::env::var("FRESCO_LOWERING_POLICY") {
            Ok(v) if v.eq_ignore_ascii_case("compact") => Self::Compact,
            _ => Self::Readable,
        }
    }

    pub(super) fn compact(self) -> bool {
        matches!(self, Self::Compact)
    }
}

impl ModuleBuilder {
    pub(super) fn register_context_type(
        &mut self,
        context: &crate::context::ContextStruct,
        t: &TypeHandles,
    ) -> (Handle<naga::Type>, u32, u32) {
        use crate::context::ContextType;
        let mut members = Vec::new();
        let mut offset = 0u32;
        let mut alignment = 1u32;
        for field in &context.fields {
            let (ty, size, align) = match &field.ty {
                ContextType::Float => (t.f32_, 4, 4),
                ContextType::Scalar(kind) => (scalar_type_handle(*kind, t), 4, 4),
                ContextType::Vector(2) => (t.v2, 8, 8),
                ContextType::Vector(3) => (t.v3, 12, 16),
                ContextType::Vector(4) => (t.v4, 16, 16),
                ContextType::Vector(_) => unreachable!("validated vector width"),
                ContextType::Struct(nested) => self.register_context_type(nested, t),
            };
            offset = offset.next_multiple_of(align);
            members.push(naga::StructMember {
                name: Some(field.name.clone()),
                ty,
                binding: None,
                offset,
            });
            offset += size;
            alignment = alignment.max(align);
        }
        let size = offset.next_multiple_of(alignment);
        let existing = self
            .module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(&context.name))
            .map(|(handle, _)| handle);
        let handle = existing.unwrap_or_else(|| {
            self.module.types.insert(
                naga::Type {
                    name: Some(context.name.clone()),
                    inner: naga::TypeInner::Struct {
                        members,
                        span: size,
                    },
                },
                SP,
            )
        });
        (handle, size, alignment)
    }
    pub(super) fn new() -> Self {
        Self {
            module: naga::Module::default(),
            function_dedup_cache: HashMap::new(),
        }
    }

    pub(super) fn register_core_types(&mut self) -> TypeHandles {
        self.register_core_types_named("FrescoCtx")
    }

    pub(super) fn register_core_types_named(&mut self, context_name: &str) -> TypeHandles {
        let scalar_ty = |inner| naga::Type { name: None, inner };
        let f32_ty = self
            .module
            .types
            .insert(scalar_ty(naga::TypeInner::Scalar(naga::Scalar::F32)), SP);
        let u32_ty = self
            .module
            .types
            .insert(scalar_ty(naga::TypeInner::Scalar(naga::Scalar::U32)), SP);
        let v2_ty = self.module.types.insert(
            scalar_ty(naga::TypeInner::Vector {
                size: VectorSize::Bi,
                scalar: naga::Scalar::F32,
            }),
            SP,
        );
        let v3_ty = self.module.types.insert(
            scalar_ty(naga::TypeInner::Vector {
                size: VectorSize::Tri,
                scalar: naga::Scalar::F32,
            }),
            SP,
        );
        let v4_ty = self.module.types.insert(
            scalar_ty(naga::TypeInner::Vector {
                size: VectorSize::Quad,
                scalar: naga::Scalar::F32,
            }),
            SP,
        );
        let ctx_ty = self.module.types.insert(
            naga::Type {
                name: Some(context_name.to_string()),
                inner: naga::TypeInner::Struct {
                    members: vec![
                        naga::StructMember {
                            name: Some("res".to_string()),
                            ty: v2_ty,
                            binding: None,
                            offset: 0,
                        },
                        naga::StructMember {
                            name: Some("delta".to_string()),
                            ty: f32_ty,
                            binding: None,
                            offset: 8,
                        },
                        naga::StructMember {
                            name: Some("px".to_string()),
                            ty: f32_ty,
                            binding: None,
                            offset: 12,
                        },
                        naga::StructMember {
                            name: Some("aa".to_string()),
                            ty: f32_ty,
                            binding: None,
                            offset: 16,
                        },
                    ],
                    span: 24,
                },
            },
            SP,
        );
        let native_vectors = [
            naga::Scalar::F32,
            naga::Scalar::I32,
            naga::Scalar::U32,
            naga::Scalar::BOOL,
        ]
        .map(|scalar| {
            [VectorSize::Bi, VectorSize::Tri, VectorSize::Quad].map(|size| {
                self.module
                    .types
                    .insert(scalar_ty(naga::TypeInner::Vector { size, scalar }), SP)
            })
        });
        TypeHandles {
            native_vectors,
            entry_context: None,
            f32_: f32_ty,
            u32_: u32_ty,
            i32_: self
                .module
                .types
                .insert(scalar_ty(naga::TypeInner::Scalar(naga::Scalar::I32)), SP),
            bool_: self
                .module
                .types
                .insert(scalar_ty(naga::TypeInner::Scalar(naga::Scalar::BOOL)), SP),
            v2: v2_ty,
            v3: v3_ty,
            v4: v4_ty,
            ctx: ctx_ty,
            m2: self.module.types.insert(
                scalar_ty(naga::TypeInner::Matrix {
                    columns: VectorSize::Bi,
                    rows: VectorSize::Bi,
                    scalar: naga::Scalar::F32,
                }),
                SP,
            ),
            m3: self.module.types.insert(
                scalar_ty(naga::TypeInner::Matrix {
                    columns: VectorSize::Tri,
                    rows: VectorSize::Tri,
                    scalar: naga::Scalar::F32,
                }),
                SP,
            ),
            m4: self.module.types.insert(
                scalar_ty(naga::TypeInner::Matrix {
                    columns: VectorSize::Quad,
                    rows: VectorSize::Quad,
                    scalar: naga::Scalar::F32,
                }),
                SP,
            ),
            contour_arrays: [4, 6, 49].map(|count| {
                self.module.types.insert(
                    scalar_ty(naga::TypeInner::Array {
                        base: v4_ty,
                        size: naga::ArraySize::Constant(
                            NonZeroU32::new(count).expect("nonzero capacity"),
                        ),
                        stride: 16,
                    }),
                    SP,
                )
            }),
            path_seg: f32_ty,
        }
    }

    pub(super) fn push_function(&mut self, function: naga::Function) -> Handle<naga::Function> {
        self.module.functions.append(function, SP)
    }

    pub(super) fn push_function_dedup(
        &mut self,
        function: naga::Function,
    ) -> Handle<naga::Function> {
        let key = scene_function_dedup_key(&function);
        if let Some(&handle) = self.function_dedup_cache.get(&key) {
            return handle;
        }
        let handle = self.push_function(function);
        self.function_dedup_cache.insert(key, handle);
        handle
    }

    pub(super) fn finish(self) -> naga::Module {
        self.module
    }
}

#[cfg(test)]
#[expect(
    clippy::items_after_test_module,
    reason = "These tests are colocated with the internal IR structures they exercise."
)]
mod tests {
    use super::*;

    #[test]
    fn push_function_dedup_reuses_identical_functions() {
        let mut builder = ModuleBuilder::new();
        let first = naga::Function {
            name: Some("helper_a".to_string()),
            ..Default::default()
        };
        let second = naga::Function {
            name: Some("helper_b".to_string()),
            ..Default::default()
        };

        let first_handle = builder.push_function_dedup(first);
        let second_handle = builder.push_function_dedup(second);

        assert_eq!(first_handle, second_handle);
        assert_eq!(builder.module.functions.len(), 1);
    }
}

impl FunctionBuilder {
    pub(super) fn new(name: String, policy: LoweringPolicy) -> Self {
        Self {
            function: naga::Function {
                name: Some(name),
                ..Default::default()
            },
            block_stack: Vec::new(),
            policy,
            emitted_instructions: 0,
            feature_instruction_counts: BTreeMap::new(),
            feature_instruction_counts_by_locality: BTreeMap::new(),
            active_feature_tag: None,
            active_locality: None,
        }
    }

    pub(super) fn should_emit(e: &Ex) -> bool {
        !matches!(
            e,
            Ex::Literal(_)
                | Ex::FunctionArgument(_)
                | Ex::Constant(_)
                | Ex::CallResult(_)
                | Ex::LocalVariable(_)
                | Ex::GlobalVariable(_)
        )
    }

    pub(super) fn arg(&mut self, name: &str, ty: Handle<naga::Type>) -> Handle<Ex> {
        let index = u32::try_from(self.function.arguments.len())
            .expect("naga function argument index overflow");
        self.function.arguments.push(naga::FunctionArgument {
            name: Some(name.into()),
            ty,
            binding: None,
        });
        self.function
            .expressions
            .append(Ex::FunctionArgument(index), SP)
    }

    pub(super) fn set_result(&mut self, ty: Handle<naga::Type>) {
        self.function.result = Some(naga::FunctionResult { ty, binding: None });
    }

    pub(super) fn expr(&mut self, e: Ex) -> Handle<Ex> {
        let needs_emit = Self::should_emit(&e);
        let start = self.function.expressions.len();
        let h = self.function.expressions.append(e, SP);
        if needs_emit {
            self.emitted_instructions += 1;
            if let Some(tag) = &self.active_feature_tag {
                *self
                    .feature_instruction_counts
                    .entry(tag.clone())
                    .or_insert(0) += 1;
                if let Some(locality) = self.active_locality {
                    let locality_label = match locality {
                        Locality::Point => "point",
                        Locality::Local => "local",
                        Locality::Global => "global",
                    };
                    let by_feature = self
                        .feature_instruction_counts_by_locality
                        .entry(locality_label.to_string())
                        .or_default();
                    *by_feature.entry(tag.clone()).or_insert(0) += 1;
                }
            }
            self.push_statement(naga::Statement::Emit(
                self.function.expressions.range_from(start),
            ));
        }
        h
    }

    pub(super) fn set_feature_tag(&mut self, next: Option<&str>) -> Option<String> {
        let prev = self.active_feature_tag.clone();
        self.active_feature_tag = next.map(ToOwned::to_owned);
        prev
    }

    pub(super) fn restore_feature_tag(&mut self, prev: Option<String>) {
        self.active_feature_tag = prev;
    }

    pub(super) fn set_locality(&mut self, next: Option<Locality>) -> Option<Locality> {
        let prev = self.active_locality;
        self.active_locality = next;
        prev
    }

    pub(super) fn restore_locality(&mut self, prev: Option<Locality>) {
        self.active_locality = prev;
    }

    pub(super) fn push_statement(&mut self, stmt: naga::Statement) {
        if let Some(block) = self.block_stack.last_mut() {
            block.push(stmt, SP);
        } else {
            self.function.body.push(stmt, SP);
        }
    }

    pub(super) fn begin_block(&mut self) {
        self.block_stack.push(naga::Block::default());
    }

    pub(super) fn end_block(&mut self) -> naga::Block {
        self.block_stack
            .pop()
            .expect("begin_block/end_block mismatch")
    }

    pub(super) fn local(
        &mut self,
        name: &str,
        ty: Handle<naga::Type>,
        init: Option<Handle<Ex>>,
    ) -> Handle<Ex> {
        let h = self.function.local_variables.append(
            naga::LocalVariable {
                name: Some(name.to_string()),
                ty,
                init,
            },
            SP,
        );
        self.expr(Ex::LocalVariable(h))
    }

    pub(super) fn store(&mut self, pointer: Handle<Ex>, value: Handle<Ex>) {
        self.push_statement(naga::Statement::Store { pointer, value });
    }

    pub(super) fn name_expr(&mut self, h: Handle<Ex>, name: &str) {
        if self.policy.compact() {
            return;
        }
        self.function.named_expressions.insert(h, name.to_string());
    }

    pub(super) fn return_value(&mut self, value: Handle<Ex>) {
        self.function
            .body
            .push(naga::Statement::Return { value: Some(value) }, SP);
    }

    pub(super) fn finish(self) -> naga::Function {
        self.function
    }
}

impl IrBuilder {
    #[expect(
        clippy::too_many_arguments,
        reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
    )]
    pub(super) fn new(
        function_name: &str,
        t: &TypeHandles,
        params: &[crate::hir::Param],
        policy: LoweringPolicy,
        tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
        sampler_global: Option<Handle<naga::GlobalVariable>>,
        param_storage_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
        global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
        scene_ctx_sig: bool,
        entry_context: Option<&crate::context::EntryContext>,
    ) -> Self {
        let mut function = FunctionBuilder::new(function_name.to_string(), policy);
        let context_argument = entry_context
            .map(|_| function.arg("ctx", t.entry_context.expect("registered context type")));
        let (uv, time, delta, res, px, aa) = if let Some(context) = entry_context {
            let base = context_argument.expect("context argument");
            let mut role = |name: &str, vector: bool| {
                if let Some(path) = context.roles.get(name) {
                    project_context(&mut function, base, path)
                } else {
                    // Unused internal channels have no provider. The checker diagnoses reads.
                    let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
                    if vector {
                        function.expr(Ex::Compose {
                            ty: t.v2,
                            components: vec![zero, zero],
                        })
                    } else {
                        zero
                    }
                }
            };
            let uv = role("coord", true);
            let time = role("time", false);
            let delta = role("delta_time", false);
            let res = role("resolution", true);
            (uv, time, delta, res, uv, uv)
        } else if scene_ctx_sig {
            let p = function.arg("p", t.v2);
            let tau = function.arg("t", t.f32_);
            let ctx = function.arg("ctx", t.ctx);
            let res = function.expr(Ex::AccessIndex {
                base: ctx,
                index: 0,
            });
            let delta = function.expr(Ex::AccessIndex {
                base: ctx,
                index: 1,
            });
            let px = function.expr(Ex::AccessIndex {
                base: ctx,
                index: 2,
            });
            let aa = function.expr(Ex::AccessIndex {
                base: ctx,
                index: 3,
            });
            (p, tau, delta, res, px, aa)
        } else {
            let uv = function.arg("uv", t.v2);
            let time = function.arg("time", t.f32_);
            let delta = function.arg("delta", t.f32_);
            let res = function.arg("res", t.v2);
            (uv, time, delta, res, uv, uv)
        };
        let mut param_scalars = HashMap::new();
        for p in params {
            if let Some(component) = entry_context.and_then(|context| {
                context
                    .components
                    .iter()
                    .find(|component| component.name == p.name)
            }) {
                let value = project_context(
                    &mut function,
                    context_argument.expect("context argument"),
                    &component.path,
                );
                param_scalars.insert(p.name.clone(), value);
                continue;
            }
            if let Some((elem_ty_str, len)) = crate::hir::parse_array_param_type(&p.ty_name) {
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
                            let elem_name = format!("{}__{index}", p.name);
                            let h = function.arg(&elem_name, t.f32_);
                            param_scalars.insert(elem_name, h);
                        }
                    }
                    ArrayElemType::Vec2 => {
                        for index in 0..len {
                            let x_name = format!("{}__{index}__x", p.name);
                            let y_name = format!("{}__{index}__y", p.name);
                            let hx = function.arg(&x_name, t.f32_);
                            let hy = function.arg(&y_name, t.f32_);
                            param_scalars.insert(x_name, hx);
                            param_scalars.insert(y_name, hy);
                        }
                    }
                    ArrayElemType::Vec3 => {
                        for index in 0..len {
                            let x_name = format!("{}__{index}__x", p.name);
                            let y_name = format!("{}__{index}__y", p.name);
                            let z_name = format!("{}__{index}__z", p.name);
                            let hx = function.arg(&x_name, t.f32_);
                            let hy = function.arg(&y_name, t.f32_);
                            let hz = function.arg(&z_name, t.f32_);
                            param_scalars.insert(x_name, hx);
                            param_scalars.insert(y_name, hy);
                            param_scalars.insert(z_name, hz);
                        }
                    }
                    ArrayElemType::Vec4 => {
                        for index in 0..len {
                            let x_name = format!("{}__{index}__x", p.name);
                            let y_name = format!("{}__{index}__y", p.name);
                            let z_name = format!("{}__{index}__z", p.name);
                            let w_name = format!("{}__{index}__w", p.name);
                            let hx = function.arg(&x_name, t.f32_);
                            let hy = function.arg(&y_name, t.f32_);
                            let hz = function.arg(&z_name, t.f32_);
                            let hw = function.arg(&w_name, t.f32_);
                            param_scalars.insert(x_name, hx);
                            param_scalars.insert(y_name, hy);
                            param_scalars.insert(z_name, hz);
                            param_scalars.insert(w_name, hw);
                        }
                    }
                    ArrayElemType::Mat2 => {
                        for index in 0..len {
                            for i in 0..2 {
                                for j in 0..2 {
                                    let elem_name = format!("{}__{index}__{i}_{j}", p.name);
                                    let h = function.arg(&elem_name, t.f32_);
                                    param_scalars.insert(elem_name, h);
                                }
                            }
                        }
                    }
                    ArrayElemType::Mat3 => {
                        for index in 0..len {
                            for i in 0..3 {
                                for j in 0..3 {
                                    let elem_name = format!("{}__{index}__{i}_{j}", p.name);
                                    let h = function.arg(&elem_name, t.f32_);
                                    param_scalars.insert(elem_name, h);
                                }
                            }
                        }
                    }
                    ArrayElemType::Mat4 => {
                        for index in 0..len {
                            for i in 0..4 {
                                for j in 0..4 {
                                    let elem_name = format!("{}__{index}__{i}_{j}", p.name);
                                    let h = function.arg(&elem_name, t.f32_);
                                    param_scalars.insert(elem_name, h);
                                }
                            }
                        }
                    }
                    ArrayElemType::Color => {
                        for index in 0..len {
                            let elem_name = format!("{}__{index}", p.name);
                            let v = function.arg(&elem_name, t.v4);
                            // Decompose into r,g,b,a components like single colors
                            for (comp_idx, comp_name) in
                                [(0_u32, "r"), (1, "g"), (2, "b"), (3, "a")]
                            {
                                let component = function.expr(Ex::AccessIndex {
                                    base: v,
                                    index: comp_idx,
                                });
                                param_scalars.insert(format!("{elem_name}.{comp_name}"), component);
                            }
                        }
                    }
                }
                continue;
            }
            // Check for dynamic arrays - these are handled via storage buffers, not function args
            if let Some((_, crate::hir::ArrayParamSize::Dynamic)) =
                crate::hir::parse_array_param_type_ex(&p.ty_name)
            {
                // Dynamic arrays are accessed via storage buffers created in the lowering phase,
                // not passed as function arguments
                continue;
            }
            match p.ty_name.as_str() {
                "f32" | "i32" | "u32" | "bool" => {
                    let h = function.arg(&p.name, t.f32_);
                    param_scalars.insert(p.name.clone(), h);
                }
                "color" => {
                    let v = function.arg(&p.name, t.v4);
                    for (index, suffix) in [(0_u32, "r"), (1, "g"), (2, "b"), (3, "a")] {
                        let component = function.expr(Ex::AccessIndex { base: v, index });
                        function.name_expr(component, &format!("{}_{}", p.name, suffix));
                        param_scalars.insert(format!("{}.{}", p.name, suffix), component);
                    }
                }
                _ => unreachable!("checker emitted unsupported param type"),
            }
        }
        function.set_result(t.v4);

        Self {
            function,
            types: *t,
            uv,
            time,
            time_override: None,
            delta,
            res,
            px,
            aa,
            jacobian_j11: None,
            jacobian_j12: None,
            jacobian_j21: None,
            jacobian_j22: None,
            param_scalars,
            param_scalar_ptrs: HashSet::new(),
            tex_globals: tex_globals.clone(),
            sampler_global,
            param_storage_globals: param_storage_globals.clone(),
            global_uniform_globals: global_uniform_globals.clone(),
        }
    }

    pub(super) fn new_user_helper(
        helper: &UserFnHelper,
        t: &TypeHandles,
        policy: LoweringPolicy,
        global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    ) -> Self {
        let lowered_name = helper
            .id
            .rsplit_once('_')
            .and_then(|(base, suffix)| {
                matches!(
                    suffix,
                    "f" | "v2"
                        | "v3"
                        | "v4"
                        | "m2"
                        | "m3"
                        | "m4"
                        | "s"
                        | "i"
                        | "u"
                        | "coord"
                        | "color"
                        | "shape"
                        | "layer"
                        | "struct"
                        | "enum"
                        | "fn"
                        | "t"
                )
                .then_some(format!("{base}_"))
            })
            .unwrap_or_else(|| helper.id.clone());
        let mut function = FunctionBuilder::new(lowered_name, policy);

        let mut param_scalars = HashMap::new();
        if helper.params.is_empty() {
            for name in &helper.param_scalars {
                let h = function.arg(name, t.f32_);
                param_scalars.insert(name.clone(), h);
            }
        } else {
            for param in &helper.params {
                let arg_name = format!("arg_{}", param.name);
                match param.ty {
                    crate::hir::UserFnParamTy::Scalar => {
                        let h = function.arg(&arg_name, scalar_type_handle(param.scalar_kind, t));
                        if let Some(slot) = param.scalar_slots.first() {
                            param_scalars.insert(slot.clone(), h);
                        }
                    }
                    crate::hir::UserFnParamTy::Vec2
                    | crate::hir::UserFnParamTy::Vec3
                    | crate::hir::UserFnParamTy::Vec4 => {
                        let ty = match param.ty {
                            crate::hir::UserFnParamTy::Vec2 => {
                                vector_type_handle(param.scalar_kind, 2, t)
                            }
                            crate::hir::UserFnParamTy::Vec3 => {
                                vector_type_handle(param.scalar_kind, 3, t)
                            }
                            crate::hir::UserFnParamTy::Vec4 => {
                                vector_type_handle(param.scalar_kind, 4, t)
                            }
                            _ => unreachable!(),
                        };
                        let vec_arg = function.arg(&arg_name, ty);
                        for (index, slot) in param.scalar_slots.iter().enumerate() {
                            let comp = function.expr(Ex::AccessIndex {
                                base: vec_arg,
                                index: index as u32,
                            });
                            param_scalars.insert(slot.clone(), comp);
                        }
                    }
                    crate::hir::UserFnParamTy::Mat2
                    | crate::hir::UserFnParamTy::Mat3
                    | crate::hir::UserFnParamTy::Mat4 => {
                        let (matrix_ty, rows) = match param.ty {
                            crate::hir::UserFnParamTy::Mat2 => (t.m2, 2usize),
                            crate::hir::UserFnParamTy::Mat3 => (t.m3, 3usize),
                            crate::hir::UserFnParamTy::Mat4 => (t.m4, 4usize),
                            _ => unreachable!(),
                        };
                        let mat_arg = function.arg(&arg_name, matrix_ty);
                        for (index, slot) in param.scalar_slots.iter().enumerate() {
                            let col = index.div_euclid(rows);
                            let row = index % rows;
                            let col_expr = function.expr(Ex::AccessIndex {
                                base: mat_arg,
                                index: col as u32,
                            });
                            let comp = function.expr(Ex::AccessIndex {
                                base: col_expr,
                                index: row as u32,
                            });
                            param_scalars.insert(slot.clone(), comp);
                        }
                    }
                }
            }
        }

        let result_ty = match helper.ret_components {
            1 => scalar_type_handle(helper.ret_kind, t),
            2..=4 => vector_type_handle(helper.ret_kind, usize::from(helper.ret_components), t),
            _ => unreachable!("unsupported helper return arity"),
        };
        function.set_result(result_ty);

        let zero = function.expr(Ex::Literal(naga::Literal::F32(0.0)));
        let zero_vec2 = function.expr(Ex::Compose {
            ty: t.v2,
            components: vec![zero, zero],
        });

        let (uv, time, delta, res) = if helper.needs_entry_inputs {
            (
                function.arg("entry_coord", t.v2),
                function.arg("entry_time", t.f32_),
                function.arg("entry_delta", t.f32_),
                function.arg("entry_res", t.v2),
            )
        } else {
            (zero_vec2, zero, zero, zero_vec2)
        };

        Self {
            function,
            types: *t,
            uv,
            time,
            time_override: None,
            delta,
            res,
            px: zero,
            aa: zero,
            jacobian_j11: None,
            jacobian_j12: None,
            jacobian_j21: None,
            jacobian_j22: None,
            param_scalars,
            param_scalar_ptrs: HashSet::new(),
            tex_globals: HashMap::new(),
            sampler_global: None,
            param_storage_globals: HashMap::new(),
            global_uniform_globals: global_uniform_globals.clone(),
        }
    }

    /// Build an `IrBuilder` for a scatter body helper function.
    ///
    /// The helper has the signature:
    /// `fn fresco_{canvas}_scatter_l{id}(p, px, aa, time, delta, res, inst_pos_x, inst_pos_y, inst_id,
    ///  inst_index01, inst_age_norm, …canvas_params…) -> vec4<f32>`
    ///
    /// Returns the builder and a `ScatterInstanceCtx` wired to the instance args.
    pub(super) fn new_scatter_body(
        entry_name: String,
        t: &TypeHandles,
        params: &[crate::hir::Param],
        policy: LoweringPolicy,
        tex_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
        sampler_global: Option<Handle<naga::GlobalVariable>>,
        global_uniform_globals: &HashMap<String, Handle<naga::GlobalVariable>>,
    ) -> (Self, ScatterInstanceCtx) {
        let mut function = FunctionBuilder::new(entry_name, policy);
        let p = function.arg("p", t.v2);
        let px_h = function.arg("px", t.f32_);
        let aa_h = function.arg("aa", t.f32_);
        let time_h = function.arg("time", t.f32_);
        let delta_h = function.arg("delta", t.f32_);
        let res_h = function.arg("res", t.v2);
        let inst_pos_x = function.arg("inst_pos_x", t.f32_);
        let inst_pos_y = function.arg("inst_pos_y", t.f32_);
        let inst_id = function.arg("inst_id", t.f32_);
        let inst_index01 = function.arg("inst_index01", t.f32_);
        let inst_age_norm = function.arg("inst_age_norm", t.f32_);

        // Canvas params: scalars pass through directly; color params are
        // decomposed into per-component f32 args to avoid aggregates.
        let mut param_scalars = HashMap::new();
        for param in params {
            match param.ty_name.as_str() {
                "f32" | "i32" | "u32" | "bool" => {
                    let h = function.arg(&param.name, t.f32_);
                    param_scalars.insert(param.name.clone(), h);
                }
                "color" => {
                    for suffix in ["r", "g", "b", "a"] {
                        let h = function.arg(&format!("{}_{suffix}", param.name), t.f32_);
                        param_scalars.insert(format!("{}.{suffix}", param.name), h);
                    }
                }
                _ => unreachable!("checker emitted unsupported param type"),
            }
        }

        function.set_result(t.v4);

        let ctx = ScatterInstanceCtx {
            pos_x: inst_pos_x,
            pos_y: inst_pos_y,
            id: inst_id,
            index01: inst_index01,
            age_norm: inst_age_norm,
        };

        let ir = Self {
            function,
            types: *t,
            uv: p,
            time: time_h,
            time_override: None,
            delta: delta_h,
            res: res_h,
            px: px_h,
            aa: aa_h,
            jacobian_j11: None,
            jacobian_j12: None,
            jacobian_j21: None,
            jacobian_j22: None,
            param_scalars,
            param_scalar_ptrs: HashSet::new(),
            tex_globals: tex_globals.clone(),
            sampler_global,
            // Scatter helper functions do not access dynamic array params directly;
            // they receive expanded scalar arguments. Leave globals empty.
            param_storage_globals: HashMap::new(),
            global_uniform_globals: global_uniform_globals.clone(),
        };

        (ir, ctx)
    }

    pub(super) fn add(&mut self, e: Ex) -> Handle<Ex> {
        self.function.expr(e)
    }

    /// Return handles for all function arguments in declaration order.
    /// This is useful when forwarding entrypoint parameters to helper functions.
    pub(super) fn function_arg_count(&self) -> usize {
        self.function.function.arguments.len()
    }

    pub(super) fn function_args_in_order_from(
        &mut self,
        start: usize,
        end: usize,
    ) -> Vec<Handle<Ex>> {
        (start..end)
            .map(|i| self.add(Ex::FunctionArgument(i as u32)))
            .collect()
    }

    pub(super) fn set_feature_tag(&mut self, tag: Option<&str>) -> Option<String> {
        self.function.set_feature_tag(tag)
    }

    pub(super) fn restore_feature_tag(&mut self, prev: Option<String>) {
        self.function.restore_feature_tag(prev);
    }

    pub(super) fn emitted_instructions(&self) -> usize {
        self.function.emitted_instructions
    }

    pub(super) fn feature_instruction_counts(&self) -> BTreeMap<String, usize> {
        self.function.feature_instruction_counts.clone()
    }

    pub(super) fn feature_instruction_counts_by_locality(
        &self,
    ) -> BTreeMap<String, BTreeMap<String, usize>> {
        self.function.feature_instruction_counts_by_locality.clone()
    }

    pub(super) fn set_locality(&mut self, locality: Option<Locality>) -> Option<Locality> {
        self.function.set_locality(locality)
    }

    pub(super) fn restore_locality(&mut self, prev: Option<Locality>) {
        self.function.restore_locality(prev);
    }

    pub(super) fn name(&mut self, h: Handle<Ex>, name: &str) {
        self.function.name_expr(h, name);
    }

    pub(super) fn access_index_of(&self, h: Handle<Ex>) -> Option<(Handle<Ex>, u32)> {
        match self.function.function.expressions[h] {
            Ex::AccessIndex { base, index } => Some((base, index)),
            _ => None,
        }
    }

    pub(super) fn return_value(&mut self, value: Handle<Ex>) {
        self.function.return_value(value);
    }

    pub(super) fn local(
        &mut self,
        name: &str,
        ty: Handle<naga::Type>,
        init: Option<Handle<Ex>>,
    ) -> Handle<Ex> {
        self.function.local(name, ty, init)
    }

    pub(super) fn store(&mut self, pointer: Handle<Ex>, value: Handle<Ex>) {
        self.function.store(pointer, value);
    }

    pub(super) fn bind_scalar_slot_ptr(&mut self, name: String, ptr: Handle<Ex>) {
        self.param_scalars.insert(name, ptr);
        self.param_scalar_ptrs.insert(ptr);
    }

    pub(super) fn bind_scalar_slot_value(&mut self, name: String, value: Handle<Ex>) {
        self.param_scalars.insert(name, value);
        self.param_scalar_ptrs.remove(&value);
    }

    pub(super) fn is_scalar_slot_ptr(&self, handle: Handle<Ex>) -> bool {
        self.param_scalar_ptrs.contains(&handle)
    }

    pub(super) fn finish(self) -> naga::Function {
        self.function.finish()
    }

    pub(super) fn lit(&mut self, v: f32) -> Handle<Ex> {
        self.add(Ex::Literal(naga::Literal::F32(v)))
    }

    pub(super) fn lit_u32(&mut self, v: u32) -> Handle<Ex> {
        self.add(Ex::Literal(naga::Literal::U32(v)))
    }

    pub(super) fn bin(&mut self, op: Bo, l: Handle<Ex>, r: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Binary {
            op,
            left: l,
            right: r,
        })
    }

    pub(super) fn addx(&mut self, l: Handle<Ex>, r: Handle<Ex>) -> Handle<Ex> {
        self.bin(Bo::Add, l, r)
    }
    pub(super) fn sub(&mut self, l: Handle<Ex>, r: Handle<Ex>) -> Handle<Ex> {
        self.bin(Bo::Subtract, l, r)
    }
    pub(super) fn mul(&mut self, l: Handle<Ex>, r: Handle<Ex>) -> Handle<Ex> {
        self.bin(Bo::Multiply, l, r)
    }
    pub(super) fn div(&mut self, l: Handle<Ex>, r: Handle<Ex>) -> Handle<Ex> {
        self.bin(Bo::Divide, l, r)
    }

    pub(super) fn neg(&mut self, e: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Unary {
            op: Uo::Negate,
            expr: e,
        })
    }

    pub(super) fn m1(&mut self, fun: Mf, a: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Math {
            fun,
            arg: a,
            arg1: None,
            arg2: None,
            arg3: None,
        })
    }

    pub(super) fn m2(&mut self, fun: Mf, a: Handle<Ex>, b: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Math {
            fun,
            arg: a,
            arg1: Some(b),
            arg2: None,
            arg3: None,
        })
    }

    pub(super) fn m3(
        &mut self,
        fun: Mf,
        a: Handle<Ex>,
        b: Handle<Ex>,
        c: Handle<Ex>,
    ) -> Handle<Ex> {
        self.add(Ex::Math {
            fun,
            arg: a,
            arg1: Some(b),
            arg2: Some(c),
            arg3: None,
        })
    }

    pub(super) fn vec2(&mut self, x: Handle<Ex>, y: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Compose {
            ty: self.types.v2,
            components: vec![x, y],
        })
    }

    pub(super) fn splat2(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Splat {
            size: VectorSize::Bi,
            value: v,
        })
    }

    pub(super) fn splat3(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Splat {
            size: VectorSize::Tri,
            value: v,
        })
    }

    pub(super) fn vec4(
        &mut self,
        x: Handle<Ex>,
        y: Handle<Ex>,
        z: Handle<Ex>,
        w: Handle<Ex>,
    ) -> Handle<Ex> {
        self.add(Ex::Compose {
            ty: self.types.v4,
            components: vec![x, y, z, w],
        })
    }
    pub(super) fn z_of(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::AccessIndex { base: v, index: 2 })
    }
    pub(super) fn w_of(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::AccessIndex { base: v, index: 3 })
    }
    pub(super) fn xy_of(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        let x = self.x_of(v);
        let y = self.y_of(v);
        self.vec2(x, y)
    }
    pub(super) fn x_of(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::AccessIndex { base: v, index: 0 })
    }

    pub(super) fn y_of(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::AccessIndex { base: v, index: 1 })
    }

    pub(super) fn load(&mut self, pointer: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Load { pointer })
    }

    pub(super) fn begin_block(&mut self) {
        self.function.begin_block();
    }

    pub(super) fn end_block(&mut self) -> naga::Block {
        self.function.end_block()
    }

    pub(super) fn push_statement(&mut self, stmt: naga::Statement) {
        self.function.push_statement(stmt);
    }

    /// Compute the screen-space derivative of an expression along the x-axis (dpdx).
    pub(super) fn dpdx(&mut self, expr: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Derivative {
            axis: Da::X,
            ctrl: Dc::None,
            expr,
        })
    }

    /// Compute the screen-space derivative of an expression along the y-axis (dpdy).
    pub(super) fn dpdy(&mut self, expr: Handle<Ex>) -> Handle<Ex> {
        self.add(Ex::Derivative {
            axis: Da::Y,
            ctrl: Dc::None,
            expr,
        })
    }
}
