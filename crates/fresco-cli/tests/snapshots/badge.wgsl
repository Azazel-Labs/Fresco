fn fresco_badge(uv: vec2<f32>, time: f32, res: vec2<f32>) -> vec4<f32> {
    let px = (1f / res.y);
    let aa = (1.5f * px);
    let space_ang_t0_ = -((0.34906584f * time));
    let space_cos_t0_ = cos(space_ang_t0_);
    let space_sin_t0_ = sin(space_ang_t0_);
    let space_center_t0_ = vec2<f32>(0.5f, 0.5f);
    let space_q_t0_ = (uv - space_center_t0_);
    let space_rot_x_t0_ = ((space_cos_t0_ * space_q_t0_.x) + (space_sin_t0_ * space_q_t0_.y));
    let space_rot_y_t0_ = ((space_cos_t0_ * space_q_t0_.y) - (space_sin_t0_ * space_q_t0_.x));
    let space_rot_t0_ = vec2<f32>(space_rot_x_t0_, space_rot_y_t0_);
    let p_space = (space_center_t0_ + space_rot_t0_);
    let compose_over_s0_l0_ = mix(vec3(0f), vec3<f32>(0.0627451f, 0.0627451f, 0.09411765f), vec3(1f));
    let box_q_s1_ = ((abs(((p_space - vec2<f32>((6f * px), (6f * px))) - vec2<f32>(0.5f, 0.5f))) - vec2<f32>((0.3f * 0.5f), (0.2f * 0.5f))) + vec2(0.04f));
    let box_q_pos_s1_ = max(box_q_s1_, vec2(0f));
    let box_q_neg_s1_ = min(max(box_q_s1_.x, box_q_s1_.y), 0f);
    let d_s1_ = ((length(box_q_pos_s1_) + box_q_neg_s1_) - 0.04f);
    let soft_half_r = ((12f * px) * 0.5f);
    let shadow_a_l1_ = ((1f - smoothstep(-(soft_half_r), soft_half_r, d_s1_)) * 0.6666667f);
    let compose_over_s1_l1_ = mix(compose_over_s0_l0_, vec3<f32>(0f, 0f, 0f), vec3(shadow_a_l1_));
    let box_q_s1_1 = ((abs((p_space - vec2<f32>(0.5f, 0.5f))) - vec2<f32>((0.3f * 0.5f), (0.2f * 0.5f))) + vec2(0.04f));
    let box_q_pos_s1_1 = max(box_q_s1_1, vec2(0f));
    let box_q_neg_s1_1 = min(max(box_q_s1_1.x, box_q_s1_1.y), 0f);
    let d_s1_1 = ((length(box_q_pos_s1_1) + box_q_neg_s1_1) - 0.04f);
    let fill_a_l2_ = (clamp((0.5f - (d_s1_1 / aa)), 0f, 1f) * 1f);
    let compose_over_s2_l2_ = mix(compose_over_s1_l1_, vec3<f32>(1f, 0.1764706f, 0.47058824f), vec3(fill_a_l2_));
    let outline_abs_s2_ = abs(d_s1_1);
    let outline_half_w_s2_ = ((2f * px) * 0.5f);
    let d_s2_ = (outline_abs_s2_ - outline_half_w_s2_);
    let fill_a_l3_ = (clamp((0.5f - (d_s2_ / aa)), 0f, 1f) * 1f);
    let col = mix(compose_over_s2_l2_, vec3<f32>(1f, 1f, 1f), vec3(fill_a_l3_));
    return vec4<f32>(col, 1f);
}

