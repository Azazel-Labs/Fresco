#![allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]

use std::num::NonZeroU32;

use naga::{
    AddressSpace, Expression as Ex, GlobalVariable, Handle, Scalar, ScalarKind, Span,
    StorageAccess, Type, TypeInner, VectorSize,
};

const SP: Span = Span::UNDEFINED;

pub(super) struct CommonHandles {
    pub f32_ty: Handle<Type>,
    pub u32_ty: Handle<Type>,
    pub bool_ty: Handle<Type>,
    pub v2_ty: Handle<Type>,
    pub v3_ty: Handle<Type>,
    pub v4_ty: Handle<Type>,
    pub uniform_global: Handle<GlobalVariable>,
    pub range_global: Handle<GlobalVariable>,
}

pub(super) fn ensure_common(
    module: &mut naga::Module,
    range_access: StorageAccess,
) -> CommonHandles {
    let f32_ty = module.types.insert(
        Type {
            name: Some("viz_f32".to_string()),
            inner: TypeInner::Scalar(Scalar::F32),
        },
        SP,
    );
    let u32_ty = module.types.insert(
        Type {
            name: Some("viz_u32".to_string()),
            inner: TypeInner::Scalar(Scalar::U32),
        },
        SP,
    );
    let bool_ty = module.types.insert(
        Type {
            name: Some("viz_bool".to_string()),
            inner: TypeInner::Scalar(Scalar::BOOL),
        },
        SP,
    );
    let v2_ty = module.types.insert(
        Type {
            name: Some("viz_v2".to_string()),
            inner: TypeInner::Vector {
                size: VectorSize::Bi,
                scalar: Scalar::F32,
            },
        },
        SP,
    );
    let v3_ty = module.types.insert(
        Type {
            name: Some("viz_v3".to_string()),
            inner: TypeInner::Vector {
                size: VectorSize::Tri,
                scalar: Scalar::F32,
            },
        },
        SP,
    );
    let v4_ty = module.types.insert(
        Type {
            name: Some("viz_v4".to_string()),
            inner: TypeInner::Vector {
                size: VectorSize::Quad,
                scalar: Scalar::F32,
            },
        },
        SP,
    );

    let param_array_ty = module.types.insert(
        Type {
            name: Some("viz_params_array".to_string()),
            inner: TypeInner::Array {
                base: v4_ty,
                size: naga::ArraySize::Constant(NonZeroU32::new(4).expect("non-zero")),
                stride: 16,
            },
        },
        SP,
    );

    let uniform_struct_ty = module.types.insert(
        Type {
            name: Some("VisualizerUniforms".to_string()),
            inner: TypeInner::Struct {
                members: vec![
                    naga::StructMember {
                        name: Some("time".to_string()),
                        ty: f32_ty,
                        binding: None,
                        offset: 0,
                    },
                    naga::StructMember {
                        name: Some("_pad0".to_string()),
                        ty: v3_ty,
                        binding: None,
                        offset: 16,
                    },
                    naga::StructMember {
                        name: Some("res".to_string()),
                        ty: v2_ty,
                        binding: None,
                        offset: 32,
                    },
                    naga::StructMember {
                        name: Some("_pad1".to_string()),
                        ty: v2_ty,
                        binding: None,
                        offset: 40,
                    },
                    naga::StructMember {
                        name: Some("params".to_string()),
                        ty: param_array_ty,
                        binding: None,
                        offset: 48,
                    },
                ],
                span: 112,
            },
        },
        SP,
    );

    let uniform_global = module.global_variables.append(
        GlobalVariable {
            name: Some("u".to_string()),
            space: AddressSpace::Uniform,
            binding: Some(naga::ResourceBinding {
                group: 0,
                binding: 0,
            }),
            ty: uniform_struct_ty,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        },
        SP,
    );

    let range_global = module.global_variables.append(
        GlobalVariable {
            name: Some("viz_range".to_string()),
            space: AddressSpace::Storage {
                access: range_access,
            },
            binding: Some(naga::ResourceBinding {
                group: 0,
                binding: 1,
            }),
            ty: v2_ty,
            init: None,
            memory_decorations: naga::MemoryDecorations::empty(),
        },
        SP,
    );

    CommonHandles {
        f32_ty,
        u32_ty,
        bool_ty,
        v2_ty,
        v3_ty,
        v4_ty,
        uniform_global,
        range_global,
    }
}

pub(super) fn lit_f32(function: &mut naga::Function, value: f32) -> Handle<Ex> {
    function
        .expressions
        .append(Ex::Literal(naga::Literal::F32(value)), SP)
}

pub(super) fn lit_u32(function: &mut naga::Function, value: u32) -> Handle<Ex> {
    function
        .expressions
        .append(Ex::Literal(naga::Literal::U32(value)), SP)
}

pub(super) fn load_uniform_field(
    function: &mut naga::Function,
    uniform_global: Handle<GlobalVariable>,
    field_index: u32,
) -> Handle<Ex> {
    let u_ptr = function
        .expressions
        .append(Ex::GlobalVariable(uniform_global), SP);
    let field_ptr = function.expressions.append(
        Ex::AccessIndex {
            base: u_ptr,
            index: field_index,
        },
        SP,
    );
    function
        .expressions
        .append(Ex::Load { pointer: field_ptr }, SP)
}

pub(super) fn read_param_scalar(
    function: &mut naga::Function,
    uniform_global: Handle<GlobalVariable>,
    slot: usize,
) -> Handle<Ex> {
    let u_ptr = function
        .expressions
        .append(Ex::GlobalVariable(uniform_global), SP);
    let params_ptr = function.expressions.append(
        Ex::AccessIndex {
            base: u_ptr,
            index: 4,
        },
        SP,
    );
    let vec_index = lit_u32(function, slot.div_euclid(4) as u32);
    let vec_ptr = function.expressions.append(
        Ex::Access {
            base: params_ptr,
            index: vec_index,
        },
        SP,
    );
    let comp_ptr = function.expressions.append(
        Ex::AccessIndex {
            base: vec_ptr,
            index: (slot % 4) as u32,
        },
        SP,
    );
    function
        .expressions
        .append(Ex::Load { pointer: comp_ptr }, SP)
}

pub(super) fn read_range(
    function: &mut naga::Function,
    range_global: Handle<GlobalVariable>,
) -> Handle<Ex> {
    let range_ptr = function
        .expressions
        .append(Ex::GlobalVariable(range_global), SP);
    function
        .expressions
        .append(Ex::Load { pointer: range_ptr }, SP)
}

pub(super) fn vec2(
    function: &mut naga::Function,
    v2_ty: Handle<Type>,
    x: Handle<Ex>,
    y: Handle<Ex>,
) -> Handle<Ex> {
    function.expressions.append(
        Ex::Compose {
            ty: v2_ty,
            components: vec![x, y],
        },
        SP,
    )
}

pub(super) fn vec4(
    function: &mut naga::Function,
    v4_ty: Handle<Type>,
    x: Handle<Ex>,
    y: Handle<Ex>,
    z: Handle<Ex>,
    w: Handle<Ex>,
) -> Handle<Ex> {
    function.expressions.append(
        Ex::Compose {
            ty: v4_ty,
            components: vec![x, y, z, w],
        },
        SP,
    )
}

pub(super) fn as_f32(function: &mut naga::Function, expr: Handle<Ex>) -> Handle<Ex> {
    function.expressions.append(
        Ex::As {
            expr,
            kind: ScalarKind::Float,
            convert: Some(4),
        },
        SP,
    )
}
