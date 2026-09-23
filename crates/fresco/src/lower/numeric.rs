use super::*;

const PROJECTIVE_GUARD_FOOTPRINT_SPAN_SCALE: f32 = 1.0;

impl<'h> FnCtx<'h> {
    pub(super) fn eps(&mut self) -> Handle<Ex> {
        self.ir.lit(1.0e-6)
    }

    pub(super) fn clamp_positive(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        let eps = self.eps();
        self.ir.m2(Mf::Max, v, eps)
    }

    pub(super) fn clamp_abs_nonzero(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        let eps = self.eps();
        let abs_v = self.ir.m1(Mf::Abs, v);
        self.ir.m2(Mf::Max, abs_v, eps)
    }

    pub(super) fn clamp_signed_nonzero(&mut self, v: Handle<Ex>) -> Handle<Ex> {
        let safe_abs_v = self.clamp_abs_nonzero(v);
        let sign_v = self.ir.m1(Mf::Sign, v);
        let zero = self.ir.lit(0.0);
        let one = self.ir.lit(1.0);
        let use_one = self.ir.bin(Bo::Equal, sign_v, zero);
        let safe_sign = self.ir.add(Ex::Select {
            condition: use_one,
            accept: one,
            reject: sign_v,
        });
        self.ir.mul(safe_abs_v, safe_sign)
    }

    pub(super) fn safe_div_positive(
        &mut self,
        numerator: Handle<Ex>,
        denominator: Handle<Ex>,
    ) -> Handle<Ex> {
        let safe_denominator = self.clamp_positive(denominator);
        self.ir.div(numerator, safe_denominator)
    }

    pub(super) fn safe_div_abs_nonzero(
        &mut self,
        numerator: Handle<Ex>,
        denominator: Handle<Ex>,
    ) -> Handle<Ex> {
        let safe_denominator = self.clamp_abs_nonzero(denominator);
        self.ir.div(numerator, safe_denominator)
    }

    pub(super) fn safe_div_signed_nonzero(
        &mut self,
        numerator: Handle<Ex>,
        denominator: Handle<Ex>,
    ) -> Handle<Ex> {
        let safe_denominator = self.clamp_signed_nonzero(denominator);
        self.ir.div(numerator, safe_denominator)
    }

    pub(super) fn guard_projective_visible(
        &mut self,
        value: Handle<Ex>,
        denominator: Handle<Ex>,
        footprint_span: Handle<Ex>,
    ) -> Handle<Ex> {
        // Perspective inverse is only valid when the projective denominator
        // stays comfortably away from the horizon. Express the guard in local
        // footprint units rather than focal-length proxy units so the same
        // rule scales with the actual sample footprint.
        //
        // This multiplier is the current WGSL-target tuning constant. Keep it
        // explicit so other targets can choose a different footprint-space
        // guard if their numeric precision or shader model requires it.
        let eps = self.eps();
        let span_scale = self.ir.lit(PROJECTIVE_GUARD_FOOTPRINT_SPAN_SCALE);
        let scaled_span = self.ir.mul(footprint_span, span_scale);
        let threshold = self.ir.m2(Mf::Max, eps, scaled_span);
        let visible = self.ir.bin(Bo::Greater, denominator, threshold);
        let far = self.ir.lit(1.0e6);
        self.ir.add(Ex::Select {
            condition: visible,
            accept: value,
            reject: far,
        })
    }

    pub(super) fn projective_div_signed_or_far(
        &mut self,
        numerator: Handle<Ex>,
        denominator: Handle<Ex>,
        footprint_span: Handle<Ex>,
    ) -> Handle<Ex> {
        let div = self.safe_div_signed_nonzero(numerator, denominator);
        self.guard_projective_visible(div, denominator, footprint_span)
    }
}
