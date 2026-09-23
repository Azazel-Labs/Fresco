use super::{VIZ_SERIES_SAMPLES, normalize_type_name, preview_uniform_params_field_wgsl};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DrawMode {
    Sparkline,
    Swatch,
    Thumbnail,
}

pub(super) fn build_draw_shader(
    variant_wgsl: &str,
    fn_name: &str,
    call_args: &[String],
    domain: &str,
    sweep_max: f32,
    mode: DrawMode,
    semantic_type: &str,
) -> Result<String, String> {
    let sample_uv = if domain == "time" {
        "vec2<f32>(0.5, 0.5)"
    } else {
        "vec2<f32>(s, 0.5)"
    };
    let time_expr = if domain == "time" { "s" } else { "u.time" };
    let (expr_eval, shade) = match mode {
        DrawMode::Swatch | DrawMode::Thumbnail => (
            format!(
                "let expr_out = fresco_viz_bridge({sample_uv}, {time_expr});",
                sample_uv = sample_uv,
                time_expr = time_expr,
            ),
            "return expr_out;".to_string(),
        ),
        DrawMode::Sparkline => (
            String::new(),
            format!(
                r#"
  let fit_min = viz_range.x;
  let fit_max = viz_range.y;
  let span = max(fit_max - fit_min, 1.0e-4);
  let sample_pos = clamp(in.uv.x, 0.0, 1.0) * f32({sample_count}u - 1u);
  let sample_index = u32(floor(sample_pos));
  let next_index = min(sample_index + 1u, {sample_count}u - 1u);
  let mix_t = sample_pos - f32(sample_index);
  let sample_value = mix(viz_series[sample_index], viz_series[next_index], mix_t);
  let norm = clamp((sample_value - fit_min) / span, 0.0, 1.0);
    let line_y = 1.0 - norm;
    let dist = abs(in.uv.y - line_y);
    let line = 1.0 - smoothstep(0.0, 0.018, dist);
    let glow = 1.0 - smoothstep(0.018, 0.05, dist);
    let axis = 1.0 - smoothstep(0.0, 0.008, abs(in.uv.y - 0.999));
    let guide = 1.0 - smoothstep(0.0, 0.006, abs(in.uv.y - 0.5));
    let bg = vec3<f32>(0.05, 0.09, 0.14);
    let rgb = bg
        + vec3<f32>(0.10, 0.13, 0.18) * guide * 0.45
        + vec3<f32>(0.18, 0.22, 0.28) * axis
        + vec3<f32>(0.20, 0.72, 0.82) * glow * 0.28
        + vec3<f32>(0.42, 0.94, 0.98) * line;
    return vec4<f32>(rgb, 1.0);"#,
                sample_count = VIZ_SERIES_SAMPLES
            ),
        ),
    };

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
@group(0) @binding(1) var<storage, read> viz_range: vec2<f32>;
@group(0) @binding(2) var<storage, read> viz_series: array<f32, {sample_count}>;

struct VsOut {{
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
}};

@vertex
fn vs(@builtin(vertex_index) vid: u32) -> VsOut {{
  var pos = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0)
  );
  let p = pos[vid];
  var out: VsOut;
  out.pos = vec4<f32>(p, 0.0, 1.0);
  out.uv = p * 0.5 + vec2<f32>(0.5, 0.5);
  return out;
}}

fn fresco_viz_bridge(sample_uv: vec2<f32>, sample_t: f32) -> vec4<f32> {{
  {bridge_return}
}}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {{
    let s = in.uv.x * {sweep_max};
    {expr_eval}
  {shade}
}}"#,
        variant_wgsl = variant_wgsl,
        params_field = preview_uniform_params_field_wgsl(),
        bridge_return = bridge_return,
        sample_count = VIZ_SERIES_SAMPLES,
        sweep_max = sweep_max,
        expr_eval = expr_eval,
        shade = shade,
    );

    emit_normalized_wgsl(&source)
}

fn emit_normalized_wgsl(source: &str) -> Result<String, String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|err| format!("visualizer draw WGSL parse failed: {err}"))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    );
    let info = validator
        .validate(&module)
        .map_err(|err| format!("visualizer draw naga validation failed: {err:?}"))?;
    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|err| format!("visualizer draw wgsl emit failed: {err}"))
}
