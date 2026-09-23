fn fresco_sunset(uv: vec2<f32>, time: f32, res: vec2<f32>) -> vec4<f32> {
    let px = (1f / res.y);
    let aa = (1.5f * px);
    let compose_over_s0_l2_ = mix(vec3(0f), vec3<f32>(0.101960786f, 0.14117648f, 0.2509804f), vec3(1f));
    let d_s0_ = (length((uv - vec2<f32>(0.5f, 0.38f))) - 0.16f);
    let glow_dpos_l3_ = max(d_s0_, 0f);
    let glow_scale_l3_ = ((60f * px) * 0.33333334f);
    let glow_falloff_l3_ = exp(-((glow_dpos_l3_ / glow_scale_l3_)));
    let glow_a_l3_ = ((glow_falloff_l3_ * 0.6f) * 1f);
    let add_src_l3_ = (vec3<f32>(1f, 0.8117647f, 0.36078432f) * vec3(glow_a_l3_));
    let compose_add_s1_l3_ = (compose_over_s0_l2_ + add_src_l3_);
    let soft_half_r = ((24f * px) * 0.5f);
    let soft_a_l1_ = ((1f - smoothstep(-(soft_half_r), soft_half_r, d_s0_)) * 1f);
    let screen_one = vec3(1f);
    let screen_src_l1_ = (vec3<f32>(1f, 0.6901961f, 0.4f) * vec3(soft_a_l1_));
    let screen_inv_dst_l1_ = (screen_one - compose_add_s1_l3_);
    let screen_inv_src_l1_ = (screen_one - screen_src_l1_);
    let screen_mul_l1_ = (screen_inv_dst_l1_ * screen_inv_src_l1_);
    let compose_screen_s2_l1_ = (screen_one - screen_mul_l1_);
    let fill_a_l4_ = (clamp((0.5f - (d_s0_ / aa)), 0f, 1f) * 1f);
    let compose_over_s3_l4_ = mix(compose_screen_s2_l1_, vec3<f32>(1f, 0.8117647f, 0.36078432f), vec3(fill_a_l4_));
    let d_s1_ = (length((uv - vec2<f32>(0.25f, 0.95f))) - 0.35f);
    let d_s2_ = (length((uv - vec2<f32>(0.8f, 1f))) - 0.45f);
    let smin_h_s4_ = clamp((0.5f + ((0.5f * (d_s2_ - d_s1_)) / 0.08f)), 0f, 1f);
    let smin_mix_s4_ = mix(d_s2_, d_s1_, smin_h_s4_);
    let smin_corr_s4_ = ((0.08f * smin_h_s4_) * (1f - smin_h_s4_));
    let d_s4_ = (smin_mix_s4_ - smin_corr_s4_);
    let fill_a_l5_ = (clamp((0.5f - (d_s4_ / aa)), 0f, 1f) * 1f);
    let col = mix(compose_over_s3_l4_, vec3<f32>(0.07058824f, 0.2f, 0.12156863f), vec3(fill_a_l5_));
    return vec4<f32>(col, 1f);
}

