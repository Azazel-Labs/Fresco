use super::{VIZ_SERIES_SAMPLES, normalize_type_name, preview_uniform_params_field_wgsl};

const REDUCE_PAD_FRACTION: f32 = 0.16;

fn build_noop_reduce_shader() -> Result<String, String> {
    let source = format!(
        r#"struct Uniforms {{
  time: f32,
  _pad0: vec3<f32>,
  res: vec2<f32>,
  _pad1: vec2<f32>,
  params: {params_field},
}};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read_write> viz_range: vec2<f32>;
@group(0) @binding(2) var<storage, read_write> viz_series: array<f32, {sample_count}>;

@compute @workgroup_size(1, 1, 1)
fn cs_reduce() {{
    viz_range = vec2<f32>(0.0, 1.0);
    for (var i: u32 = 0u; i < {sample_count}u; i = i + 1u) {{
        viz_series[i] = 0.0;
    }}
}}"#,
        params_field = preview_uniform_params_field_wgsl(),
        sample_count = VIZ_SERIES_SAMPLES,
    );
    emit_normalized_wgsl(&source)
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(super) fn build_reduce_shader(
    variant_wgsl: &str,
    fn_name: &str,
    call_args: &[String],
    domain: &str,
    sweep_max: f32,
    semantic_type: &str,
    range_hint: Option<(f32, f32)>,
    is_thumbnail: bool,
) -> Result<String, String> {
    // Thumbnails don't use series data, and shape variants may contain
    // fragment-only functions (dpdx/dpdy) that are invalid in compute shaders.
    // Generate a minimal no-op reduce shader for thumbnails.
    if is_thumbnail {
        return build_noop_reduce_shader();
    }

    let sample_uv = if domain == "time" {
        "vec2<f32>(0.5, 0.5)"
    } else {
        "vec2<f32>(s, 0.5)"
    };
    let time_expr = if domain == "time" { "s" } else { "u.time" };

    let bridge_return = match normalize_type_name(semantic_type).as_str() {
        "space" | "vec2" | "vec2<f32>" => {
            format!(
                "let raw = {fn_name}({call}); return vec4<f32>(raw.x, raw.y, 0.0, 1.0);",
                fn_name = fn_name,
                call = call_args.join(", ")
            )
        }
        _ => format!(
            "return {fn_name}({call});",
            fn_name = fn_name,
            call = call_args.join(", ")
        ),
    };

    let fit_logic = if let Some((lo, hi)) =
        range_hint.filter(|(lo, hi)| lo.is_finite() && hi.is_finite() && hi > lo)
    {
        let span = (hi - lo).max(1.0e-4);
        let pad = (span * REDUCE_PAD_FRACTION).max(1.0e-3);
        format!("viz_range = vec2<f32>({}f, {}f);", lo - pad, hi + pad)
    } else {
        format!(
            r#"let raw_span = max(fit_max - fit_min, 1.0e-4);
  let pad = max(raw_span * {pad}, 1.0e-3);
  viz_range = vec2<f32>(fit_min - pad, fit_max + pad);"#,
            pad = REDUCE_PAD_FRACTION,
        )
    };

    let source = format!(
        r#"{variant_wgsl}

struct Uniforms {{
  time: f32,
  _pad0: vec3<f32>,
  res: vec2<f32>,
  _pad1: vec2<f32>,
  params: {params_field},
}};
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read_write> viz_range: vec2<f32>;
@group(0) @binding(2) var<storage, read_write> viz_series: array<f32, {sample_count}>;

fn fresco_viz_bridge(sample_uv: vec2<f32>, sample_t: f32) -> vec4<f32> {{
  {bridge_return}
}}

@compute @workgroup_size(1, 1, 1)
fn cs_reduce() {{
    var fit_min = 1.0e9;
    var fit_max = -1.0e9;
    for (var i: u32 = 0u; i < {sample_count}u; i = i + 1u) {{
        let s = (f32(i) / f32(max({sample_count}u - 1u, 1u))) * {sweep_max};
        let sample = fresco_viz_bridge({sample_uv}, {time_expr}).r;
        viz_series[i] = sample;
        fit_min = min(fit_min, sample);
        fit_max = max(fit_max, sample);
    }}
    {fit_logic}
}}"#,
        variant_wgsl = variant_wgsl,
        params_field = preview_uniform_params_field_wgsl(),
        bridge_return = bridge_return,
        sample_count = VIZ_SERIES_SAMPLES,
        sweep_max = sweep_max,
        sample_uv = sample_uv,
        time_expr = time_expr,
        fit_logic = fit_logic,
    );

    emit_normalized_wgsl(&source)
}

fn emit_normalized_wgsl(source: &str) -> Result<String, String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|err| format!("visualizer reduce WGSL parse failed: {err}"))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    let info = validator
        .validate(&module)
        .map_err(|err| format!("visualizer reduce naga validation failed: {err}"))?;
    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|err| format!("visualizer reduce wgsl emit failed: {err}"))
}
