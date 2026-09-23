//! GPU behavior checks for typed schema values and explicit specialization.
use std::{collections::HashMap, error::Error};

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<(), Box<dyn Error>> {
    let files = HashMap::from([
        ("engine/engine.fr".into(), r#"
#pragma check.shape_aa_min_px = 1.5
#pragma check.shape_aa_max_px = 3.0
#pragma check.shape_aa_style = gradient
#pragma check.projective_footprint_max_px = 64.0
struct Domain { coordinate: vec2 }
@context(Domain, point)
@composition(base, material, weight)
material_properties Data { channel power: f32 = 2.0 }
struct LayerPayload { weights: array<vec2, 2>, matrix: mat2, enabled: bool }
fn compose_weights(previous: array<vec2, 2>, next: array<vec2, 2>, weight: f32) -> array<vec2, 2> {
    return [mix(previous[0], next[0], weight), next[1]]
}
fn compose_payload(previous: LayerPayload, next: LayerPayload, weight: f32) -> LayerPayload {
    return LayerPayload(weights: compose_weights(previous.weights, next.weights, weight), matrix: previous.matrix, enabled: select(previous.enabled, next.enabled, weight >= 0.5))
}
@context(Domain, point)
@composition(base, material, weight)
material_properties Composed {
    channel @compose(compose_payload) payload: LayerPayload = LayerPayload(weights: [vec2(1.0), vec2(2.0)], matrix: mat2(vec2(1.0, 0.0), vec2(0.0, 1.0)), enabled: false)
    channel untouched: f32 = 7.0
}
schema_program read_composition for Composed {
    output: vec4(self.payload.weights[0].x, self.payload.matrix[0][0], select(0.0, 1.0, self.payload.enabled), self.untouched)
}
fn exact_pair(values: uvec2) -> uvec2 {
    var result: uvec2 = values + uvec2(1, 2)
    result = result - uvec2(1, 2)
    return result
}
fn accumulate_id(previous: u32, next: u32, weight: f32) -> u32 {
    let pair = exact_pair(uvec2(previous, next))
    var value: u32 = previous
    value = pair.x + pair.y
    if (weight > 0.5) { return value }
    return previous
}
struct ExactDomain { count: u32, enabled: bool }
@context(ExactDomain, point)
@composition(base, material, weight)
material_properties Exact {
    channel @compose(accumulate_id) identity: u32 = 16777217
}
struct Packet { exact: u32, energy: f32, matrix_value: f32 }
schema_program transfer for Data {
    fn run(count: u32, enabled: bool, matrix: mat2, values: array<f32, 2>) -> Packet {
        return Packet(exact: count, energy: select(-2.0, self.power * values[1], enabled), matrix_value: matrix[1][0])
    }
    output: run
}
@context(Domain, point)
@composition(base, material, weight)
material_properties VariantData { channel power: f32 = 2.0 }
schema_evaluator varied for VariantData {
    contract { inputs: gain: f32, enabled: bool }
    permutations { mode: low|high }
    specialize as "typed_${mode}"
    variant unrelated when mode == low: select(0.0, gain * power, enabled)
    variant misleading when mode == high: select(0.0, gain * power * 3.0, enabled)
}
"#.into()),
        ("main.fr".into(), "surface exact(point: ExactDomain) -> material(Exact) { compose { base()\nlayer material(identity: point.count, weight: select(0.0, 1.0, point.enabled)) } }\nsurface record(point: Domain) -> material(Data) { compose { base() } }\nsurface variants(point: Domain) -> material(VariantData) { compose { base() } }\nsurface composed(point: Domain) -> material(Composed) { compose { base()\nlayer material(payload: LayerPayload(weights: [vec2(3.0), vec2(4.0)], matrix: mat2(vec2(2.0), vec2(2.0)), enabled: true), weight: point.coordinate.x) } }".into()),
    ]);
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let shader = output.wgsl
        + r#"
struct Inputs { count: u32, enabled: u32, gain: f32, other: f32 }
@group(0) @binding(0) var<storage, read> inputs: Inputs;
@group(0) @binding(1) var<storage, read_write> results: array<u32, 10>;
@compute @workgroup_size(1) fn check_values() {
    let domain = Domain(vec2<f32>(0.0));
    let packet = fresco_evaluation_shader_record(domain, inputs.count, inputs.enabled != 0u,
        mat2x2<f32>(1.0, 2.0, inputs.gain, 4.0), array<f32, 2>(inputs.gain, inputs.other), FrescoMaterial_record(2.0));
    results[0] = packet.exact;
    results[1] = bitcast<u32>(packet.energy);
    results[2] = bitcast<u32>(packet.matrix_value);
    results[3] = bitcast<u32>(typed_low(domain, inputs.gain, inputs.enabled != 0u, FrescoMaterial_variants(2.0)));
    results[4] = bitcast<u32>(typed_high(domain, inputs.gain, inputs.enabled != 0u, FrescoMaterial_variants(2.0)));
    let composition_context = Domain(vec2<f32>(inputs.gain * 0.125));
    let composition = fresco_evaluation_shader_composed(composition_context, fresco_composed(composition_context));
    results[5] = bitcast<u32>(composition.x);
    results[6] = bitcast<u32>(composition.y);
    results[7] = bitcast<u32>(composition.z);
    results[8] = bitcast<u32>(composition.w);
    results[9] = fresco_exact(ExactDomain(inputs.count, inputs.enabled != 0u)).identity;
}
"#;
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("typed schema behavior"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("typed schema behavior"),
        layout: None,
        module: &module,
        entry_point: Some("check_values"),
        compilation_options: Default::default(),
        cache: None,
    });
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let result = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 40,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: result.as_entire_binding(),
            },
        ],
    });
    if let Some(error) = validation.pop().await {
        return Err(error.to_string().into());
    }
    for (count, enabled, gain, other) in [
        (16_777_217u32, true, 3.0f32, 4.0f32),
        (16_777_219, true, 7.0, 9.0),
        (16_777_221, false, 5.0, 6.0),
    ] {
        let values = [count, u32::from(enabled), gain.to_bits(), other.to_bits()];
        queue.write_buffer(
            &input,
            0,
            &values
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        queue.submit([encoder.finish()]);
        let bytes = super::particles::read(device, queue, &result)?;
        let actual: Vec<_> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| u32::from_le_bytes(*v))
            .collect();
        let energy = if enabled { 2.0 * other } else { -2.0 };
        let low = if enabled { 2.0 * gain } else { 0.0 };
        let high = if enabled { 6.0 * gain } else { 0.0 };
        assert_eq!(
            actual,
            [
                count,
                energy.to_bits(),
                gain.to_bits(),
                low.to_bits(),
                high.to_bits(),
                (1.0 + 2.0 * gain * 0.125).to_bits(),
                1.0f32.to_bits(),
                (if gain >= 4.0 { 1.0f32 } else { 0.0f32 }).to_bits(),
                7.0f32.to_bits(),
                16_777_217 + if enabled { count } else { 0 },
            ]
        );
    }
    println!(
        "Typed schema GPU checks: exact u32, bool, record, matrix, array changing specialized runtime inputs, and authored structured layer composition passed."
    );
    Ok(())
}
