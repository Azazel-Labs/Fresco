//! Capability flags describing builtin call surfaces.

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct BuiltinCaps: u32 {
        const PURE = 1 << 0;
        const PIPEABLE = 1 << 1;
        const SHAPE_RECV = 1 << 2;
        const LAYER_RECV = 1 << 3;
        const SCALAR_RECV = 1 << 4;
        const VEC2_RECV = 1 << 5;
        const COLOR_RECV = 1 << 6;
        const SHAPE_OUT = 1 << 7;
        const LAYER_OUT = 1 << 8;
        const SCALAR_OUT = 1 << 9;
        const VEC2_OUT = 1 << 10;
        const COLOR_OUT = 1 << 11;
        const COVERAGE_OUT = 1 << 12;
        const MASK_OUT = 1 << 13;
        const GRADIENT_OUT = 1 << 14;
        const INTRINSIC = 1 << 15;
        const ORDER_SENSITIVE = 1 << 16;
        const WIDE_EFFECT = 1 << 17;
        const VEC3_OUT = 1 << 18;
        const VEC4_OUT = 1 << 19;
    }
}
