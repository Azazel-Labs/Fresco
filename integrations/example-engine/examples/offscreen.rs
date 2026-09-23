//! Local GPU probe; deliberately an executable example, never a CI GPU test.
use std::error::Error;
use std::time::Duration;

use fresco_artifact::ManifestRoot;
use fresco_example_engine::profile::FrameInputs;
use fresco_example_engine::runtime::canvas::CanvasRenderer;

#[path = "offscreen/contributions.rs"]
mod contributions;
#[path = "offscreen/deferred.rs"]
mod deferred;
#[path = "offscreen/environment.rs"]
mod environment;
#[path = "offscreen/factory_variants.rs"]
mod factory_variants;
#[path = "offscreen/forward_plus.rs"]
mod forward_plus;
#[path = "offscreen/fur.rs"]
mod fur;
#[path = "offscreen/generated.rs"]
mod generated;
#[path = "offscreen/geometry.rs"]
mod geometry;
#[path = "offscreen/lighting_service.rs"]
mod lighting_service;
#[path = "offscreen/mesh.rs"]
mod mesh;
#[path = "offscreen/mixed_styles.rs"]
mod mixed_styles;
#[path = "offscreen/multi_pass.rs"]
mod multi_pass;
#[path = "offscreen/normal_mapping.rs"]
mod normal_mapping;
#[path = "offscreen/particle_draw.rs"]
mod particle_draw;
#[path = "offscreen/particle_managed.rs"]
mod particle_managed;
#[path = "offscreen/particle_renderer.rs"]
mod particle_renderer;
#[path = "offscreen/particles.rs"]
mod particles;
#[path = "offscreen/resource_placement.rs"]
mod resource_placement;
#[path = "offscreen/samplers.rs"]
mod samplers;
#[path = "offscreen/techniques.rs"]
mod techniques;
#[path = "offscreen/tiny_engine.rs"]
mod tiny_engine;
#[path = "offscreen/transparency.rs"]
mod transparency;
#[path = "offscreen/typed_schema.rs"]
mod typed_schema;

#[path = "offscreen/variants.rs"]
mod variants;

#[path = "offscreen/shadows.rs"]
mod shadows;

#[path = "offscreen/compute.rs"]
mod compute;
#[path = "offscreen/instances.rs"]
mod instances;

#[path = "offscreen/prepared.rs"]
mod prepared;

#[path = "offscreen/styles.rs"]
mod styles;
#[path = "offscreen/toon.rs"]
mod toon;

#[path = "offscreen/buffer_views.rs"]
mod buffer_views;

#[path = "offscreen/torch.rs"]
mod torch;

const SIZE: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn compile(changed_vertex: bool) -> Result<(String, ManifestRoot), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    if changed_vertex {
        let contract = files.get_mut("engine/core/04_canvas_contract.fr").unwrap();
        *contract = contract.replace("uv: p * 0.5 + vec2(0.5, 0.5)", "uv: vec2(0.25, 0.75)");
    }
    files.insert(
        "main.fr".into(),
        "canvas probe(ctx: CanvasContext) -> color { rgba(ctx.uv.x, ctx.uv.y, time(), 1.0) }"
            .into(),
    );
    let artifact = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("compile failed: {errors:#?}"))?;
    Ok((artifact.wgsl, serde_json::from_str(&artifact.manifest)?))
}

fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<u8>, Box<dyn Error>> {
    if texture.format().block_dimensions() != (1, 1) {
        return Err("readback requires an uncompressed texture".into());
    }
    let pixel_bytes = texture
        .format()
        .block_copy_size(None)
        .ok_or("texture has no copy size")?;
    let row_bytes = texture
        .width()
        .checked_mul(pixel_bytes)
        .ok_or("readback row overflow")?;
    let stride = row_bytes
        .checked_next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        .ok_or("readback stride overflow")?;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("offscreen readback"),
        size: u64::from(stride) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).expect("readback receiver exists");
        });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(Duration::from_secs(20)),
    })?;
    receive.recv_timeout(Duration::from_secs(20))??;
    let bytes = buffer
        .slice(..)
        .get_mapped_range()?
        .chunks_exact(usize::try_from(stride)?)
        .flat_map(|row| {
            row[..usize::try_from(row_bytes).expect("row fits stride")]
                .iter()
                .copied()
        })
        .collect();
    buffer.unmap();
    Ok(bytes)
}

fn main() -> Result<(), Box<dyn Error>> {
    // Compilation uses recursive compiler passes; match the CLI's stack allowance.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| pollster::block_on(run()).map_err(|error| error.to_string()))?
        .join()
        .expect("offscreen worker panicked")
        .map_err(Into::into)
}

async fn run() -> Result<(), Box<dyn Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .await?;
    println!("Adapter: {}", adapter.get_info().name);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_limits: fresco_example_engine::runtime::device::requested_limits(
                &adapter.limits(),
            ),
            ..Default::default()
        })
        .await?;
    if std::env::args().any(|arg| arg == "--torch-only") {
        return torch::verify(&device, &queue).await;
    }
    let particles_only = std::env::args().any(|arg| arg == "--particles-only");
    let compute_only = std::env::args().any(|arg| arg == "--compute-only");
    let shading_only = std::env::args().any(|arg| arg == "--shading-only");
    let deferred_only = std::env::args().any(|arg| arg == "--deferred-only");
    let generated_only = std::env::args().any(|arg| arg == "--generated-only");
    let transparency_only = std::env::args().any(|arg| arg == "--transparency-only");
    let lighting_service_only = std::env::args().any(|arg| arg == "--lighting-service-only");
    let fur_only = std::env::args().any(|arg| arg == "--fur-only");
    let mixed_styles_only = std::env::args().any(|arg| arg == "--mixed-styles-only");
    let placement_only = std::env::args().any(|arg| arg == "--placement-only");
    if !particles_only
        && !compute_only
        && !shading_only
        && !deferred_only
        && !transparency_only
        && !generated_only
        && !lighting_service_only
        && !fur_only
        && !mixed_styles_only
        && !placement_only
    {
        techniques::verify(&device, &queue).await?;
        typed_schema::verify(&device, &queue).await?;
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen canvas"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    if !compute_only && !particles_only {
        variants::run(&device, &queue, &texture).await?;
    }
    let view = texture.create_view(&Default::default());
    if particles_only {
        return particle_renderer::verify(&device, &queue, &texture, &view).await;
    }
    if placement_only {
        return resource_placement::verify(&device, &queue, &texture, &view).await;
    }
    if generated_only {
        return generated::verify(&device, &queue, &texture, &view).await;
    }
    if fur_only {
        return fur::verify(&device, &queue, &texture, &view).await;
    }
    if mixed_styles_only {
        return mixed_styles::verify(&device, &queue, &texture, &view).await;
    }
    if lighting_service_only {
        return lighting_service::verify(&device, &queue, &texture, &view).await;
    }
    if transparency_only {
        return transparency::verify(&device, &queue, &texture, &view).await;
    }
    if deferred_only {
        return deferred::verify(&device, &queue, &texture, &view).await;
    }
    if shading_only {
        samplers::verify(&device, &queue, &texture, &view).await?;
        instances::verify(&device, &queue, &texture, &view).await?;
        return compute::verify_shading(&device, &queue, &texture, &view).await;
    }
    // Resume the baseline acceptance slice independently of the advanced style
    // probes. The default invocation still executes every check.
    if !std::env::args().any(|arg| arg == "--baseline-only") {
        compute::verify(&device, &queue, &texture, &view).await?;
        if compute_only {
            return Ok(());
        }
        styles::verify_operations(&device, &queue, &texture, &view).await?;
        transparency::verify(&device, &queue, &texture, &view).await?;
        lighting_service::verify(&device, &queue, &texture, &view).await?;
        fur::verify(&device, &queue, &texture, &view).await?;
        mixed_styles::verify(&device, &queue, &texture, &view).await?;
        resource_placement::verify(&device, &queue, &texture, &view).await?;
        generated::verify(&device, &queue, &texture, &view).await?;
        prepared::verify(&device, &queue, &texture, &view).await?;
        environment::verify(&device, &queue, &texture, &view).await?;
        contributions::verify(&device, &queue, &texture, &view).await?;
        buffer_views::verify(&device, &queue, &texture, &view).await?;
        toon::verify(&device, &queue, &texture, &view).await?;
        styles::verify(&device, &queue, &texture, &view).await?;
        shadows::verify(&device, &queue, &texture, &view).await?;
        multi_pass::verify(&device, &queue, &texture, &view).await?;
    }
    let (wgsl, manifest) = compile(false)?;
    let mut renderer = CanvasRenderer::prepare(
        device.clone(),
        queue.clone(),
        &wgsl,
        &manifest,
        "probe",
        FORMAT,
    )
    .await?;
    let frame = FrameInputs {
        time: 0.25,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    };
    assert!(renderer.render(frame, &view)?);
    let first = readback(&device, &queue, &texture)?;
    assert!(
        first
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[2].abs_diff(64) <= 1 && pixel[3] == 255)
    );
    assert!(renderer.render(
        FrameInputs {
            time: 0.75,
            ..frame
        },
        &view
    )?);
    let second = readback(&device, &queue, &texture)?;
    assert!(
        second
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[2].abs_diff(191) <= 1)
    );
    assert!(!renderer.render(
        FrameInputs {
            physical_size: [0, SIZE],
            ..frame
        },
        &view
    )?);
    assert_eq!(readback(&device, &queue, &texture)?, second);
    // Failed preparation must not invalidate the last installed renderer.
    assert!(
        CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            "invalid wgsl",
            &manifest,
            "probe",
            FORMAT
        )
        .await
        .is_err()
    );
    assert!(renderer.render(frame, &view)?);
    assert_eq!(readback(&device, &queue, &texture)?, first);
    assert!(
        CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            &wgsl,
            &manifest,
            "missing",
            FORMAT
        )
        .await
        .is_err()
    );
    // A library helper is not an executable canvas contract.
    let mut helper_only = manifest.clone();
    helper_only.canvases[0].engine_pass = None;
    assert!(
        CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            &wgsl,
            &helper_only,
            "probe",
            FORMAT,
        )
        .await
        .is_err()
    );
    assert!(renderer.render(frame, &view)?);
    assert_eq!(readback(&device, &queue, &texture)?, first);
    let mut unsupported = manifest.clone();
    let extra_pass = unsupported.canvases[0].pass_plan.passes[0].clone();
    unsupported.canvases[0].pass_plan.passes.push(extra_pass);
    assert!(
        CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            &wgsl,
            &unsupported,
            "probe",
            FORMAT
        )
        .await
        .is_err()
    );
    assert!(
        renderer
            .render(
                FrameInputs {
                    time: f32::NAN,
                    ..frame
                },
                &view
            )
            .is_err()
    );
    assert_eq!(readback(&device, &queue, &texture)?, first);
    let (wgsl, manifest) = compile(true)?;
    let mut changed = CanvasRenderer::prepare(
        device.clone(),
        queue.clone(),
        &wgsl,
        &manifest,
        "probe",
        FORMAT,
    )
    .await?;
    assert!(changed.render(frame, &view)?);
    let final_pixels = readback(&device, &queue, &texture)?;
    assert_ne!(
        first, final_pixels,
        "authored vertex changes must affect the image"
    );
    assert!(
        final_pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(64) <= 1 && pixel[1].abs_diff(191) <= 1)
    );
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        r#"canvas parameters(ctx: CanvasContext) -> color {
        param gain: f32 = 0.25 in 0 .. 1
        param enabled: bool = true
        param palette: array<color, 2> = [#ff0000, #00ff00]
        param transforms: array<mat2, 1> = [mat2(vec2(0.5, 0.75), vec2(0.0, 1.0))]
        let point = transforms[0] * vec2(1.0, 0.0)
        if enabled { rgba(gain, point.x, point.y, 1.0) } else { palette[0] }
    }"#
        .into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("parameter compile failed: {errors:#?}"))?;
    let manifest = serde_json::from_str(&output.manifest)?;
    let mut parameters = CanvasRenderer::prepare(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "parameters",
        FORMAT,
    )
    .await?;
    parameters.render(frame, &view)?;
    let defaults = readback(&device, &queue, &texture)?;
    assert!(
        defaults
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(64) <= 1
                && pixel[1].abs_diff(128) <= 1
                && pixel[2].abs_diff(191) <= 1)
    );
    parameters.update_parameters(
        serde_json::json!({"gain":0.75,"transforms":[[[0.25,0.5],[0.0,1.0]]]})
            .as_object()
            .unwrap(),
    )?;
    parameters.render(frame, &view)?;
    let edited = readback(&device, &queue, &texture)?;
    assert!(
        edited
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(191) <= 1
                && pixel[1].abs_diff(64) <= 1
                && pixel[2].abs_diff(128) <= 1)
    );
    assert!(
        parameters
            .update_parameters(
                serde_json::json!({"gain":0.1,"transforms":[[0.0]]})
                    .as_object()
                    .unwrap()
            )
            .is_err()
    );
    parameters.render(frame, &view)?;
    assert_eq!(readback(&device, &queue, &texture)?, edited);
    parameters.update_parameters(
        serde_json::json!({"enabled":false,"palette":[[0,1,0,1],[1,0,0,1]]})
            .as_object()
            .unwrap(),
    )?;
    parameters.render(frame, &view)?;
    assert!(
        readback(&device, &queue, &texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 255, 0, 255])
    );
    verify_textures(&device, &queue, &texture, &view, frame).await?;
    verify_storage(&device, &queue, &texture, &view, frame).await?;
    verify_paths(&device, &queue, &texture, &view, frame).await?;
    geometry::verify(&device, &queue, &texture, &view).await?;
    mesh::verify(&device, &queue, &texture, &view).await?;
    forward_plus::verify(&device, &queue, &texture, &view).await?;
    tiny_engine::verify(&device, &queue, &texture, &view).await?;
    deferred::verify(&device, &queue, &texture, &view).await?;
    normal_mapping::verify(&device, &queue, &texture, &view).await?;
    factory_variants::verify(&device, &queue, &texture, &view).await?;
    particles::verify(&device, &queue).await?;
    particle_renderer::verify(&device, &queue, &texture, &view).await?;
    verify_intermediate_targets(&device, &queue, &mut parameters, frame).await?;
    if let Some(path) = std::env::args_os()
        .nth(1)
        .filter(|path| !path.to_string_lossy().starts_with("--"))
    {
        let mut ppm = format!("P6\n{SIZE} {SIZE}\n255\n").into_bytes();
        for pixel in first.as_chunks::<4>().0.iter() {
            ppm.extend_from_slice(&pixel[..3]);
        }
        std::fs::write(path, ppm)?;
    }
    println!(
        "Verified authored stages, frame updates, parameters, atomic storage resizing/stale updates, texture sampling/bindings, intermediate targets, zero-size suspension, and failed-preparation recovery at {SIZE}x{SIZE}."
    );
    Ok(())
}

async fn verify_paths(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    view: &wgpu::TextureView,
    frame: FrameInputs,
) -> Result<(), Box<dyn Error>> {
    for command in [
        "L 0.9 0.5 L 0.1 0.5 ",
        "C 0.2 0.2 0.8 0.8 0.9 0.5 C 0.8 0.8 0.2 0.2 0.1 0.5 ",
    ] {
        let mut files = fresco_example_engine::source_files();
        let source = |copies| {
            format!(
                r#"canvas path_probe(ctx: CanvasContext) -> color {{
            param widths: array<f32> = [0.03]
            let curve = path_svg("M 0.1 0.5 {}", preserve_cubics: 1)
            compose {{ curve |> stroke(width: widths[0]) }}
        }}"#,
                command.repeat(copies)
            )
        };
        files.insert("main.fr".into(), source(1));
        let reference = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|errors| format!("constant path: {errors:#?}"))?;
        let reference_manifest: ManifestRoot = serde_json::from_str(&reference.manifest)?;
        assert!(reference_manifest.canvases[0].path_buffers.is_empty());
        let mut reference_renderer = CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            &reference.wgsl,
            &reference_manifest,
            "path_probe",
            FORMAT,
        )
        .await?;
        reference_renderer.render(frame, view)?;
        let expected = readback(device, queue, texture)?;
        assert!(
            expected.as_chunks::<4>().0.iter().any(|p| p[0] > 128),
            "reference path must be visible"
        );
        assert!(
            expected.as_chunks::<4>().0.iter().any(|p| p[0] == 0),
            "reference path must leave background pixels"
        );
        files.insert("main.fr".into(), source(33));
        let buffered = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|errors| format!("buffered path: {errors:#?}"))?;
        let mut manifest: ManifestRoot = serde_json::from_str(&buffered.manifest)?;
        assert_eq!(manifest.canvases[0].path_buffers[0].segments, 66);
        let mut renderer = CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            &buffered.wgsl,
            &manifest,
            "path_probe",
            FORMAT,
        )
        .await?;
        renderer.render(frame, view)?;
        assert_eq!(
            readback(device, queue, texture)?,
            expected,
            "buffered and constant representations of identical geometry must match"
        );
        let mut incomplete = manifest.clone();
        incomplete.canvases[0].path_buffers[0].data = None;
        assert!(
            CanvasRenderer::prepare(
                device.clone(),
                queue.clone(),
                &buffered.wgsl,
                &incomplete,
                "path_probe",
                FORMAT
            )
            .await
            .is_err()
        );
        renderer.render(frame, view)?;
        assert_eq!(readback(device, queue, texture)?, expected);

        // Reflect a non-default group/binding and then resize an array while the
        // static path buffer stays installed in the candidate's resource set.
        let path = &mut manifest.canvases[0].path_buffers[0];
        let old = (path.group, path.binding);
        path.group = 1;
        path.binding = 7;
        let mut module = naga::front::wgsl::parse_str(&buffered.wgsl)?;
        for (_, global) in module.global_variables.iter_mut() {
            if let Some(binding) = &mut global.binding
                && (binding.group, binding.binding) == old
            {
                binding.group = 1;
                binding.binding = 7;
            }
        }
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)?;
        let moved =
            naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())?;
        let mut moved_renderer = CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            &moved,
            &manifest,
            "path_probe",
            FORMAT,
        )
        .await?;
        for current in [&mut renderer, &mut moved_renderer, &mut reference_renderer] {
            let update = current
                .begin_parameter_update(
                    serde_json::json!({"widths":[0.015,0.03]})
                        .as_object()
                        .unwrap(),
                )?
                .prepare()
                .await?;
            current.apply_parameter_update(update)?;
        }
        reference_renderer.render(frame, view)?;
        let narrow = readback(device, queue, texture)?;
        assert_ne!(narrow, expected, "array edit must change the path width");
        for current in [&mut renderer, &mut moved_renderer] {
            current.render(frame, view)?;
            assert_eq!(
                readback(device, queue, texture)?,
                narrow,
                "path geometry survives array replacement and binding relocation"
            );
        }
    }
    Ok(())
}

async fn verify_storage(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    view: &wgpu::TextureView,
    frame: FrameInputs,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        r#"canvas storage_probe(ctx: CanvasContext) -> color {
        param gain: f32 = 0.5
        param index: f32 = 1.0
        param weights: array<f32> = [0.0, 0.5]
        param points: array<vec3> = [vec3(0.0,0.0,0.0), vec3(0.25,0.5,0.75)]
        rgba(weights[index] * gain, points[index].y, points[index].z, 1.0)
    }"#
        .into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("storage compile failed: {errors:#?}"))?;
    let manifest = serde_json::from_str(&output.manifest)?;
    let mut renderer = CanvasRenderer::prepare(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "storage_probe",
        FORMAT,
    )
    .await?;
    renderer.render(frame, view)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(64) <= 1
                && pixel[1].abs_diff(128) <= 1
                && pixel[2].abs_diff(191) <= 1)
    );
    renderer.update_parameters(serde_json::json!({"gain":1.0}).as_object().unwrap())?;
    renderer.render(frame, view)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(128) <= 1
                && pixel[1].abs_diff(128) <= 1
                && pixel[2].abs_diff(191) <= 1)
    );
    let before = readback(device, queue, texture)?;
    let pending = renderer.begin_parameter_update(
        serde_json::json!({
            "gain":0.5,"index":2,"weights":[0,0,1],"points":[[0,0,0],[0,0,0],[0,0.25,0.5]]
        })
        .as_object()
        .unwrap(),
    )?;
    renderer.render(frame, view)?;
    assert_eq!(
        readback(device, queue, texture)?,
        before,
        "CPU preparation must not change installed data"
    );
    let prepared = pending.prepare().await?;
    renderer.render(frame, view)?;
    assert_eq!(
        readback(device, queue, texture)?,
        before,
        "GPU preparation must not change installed data"
    );
    renderer.apply_parameter_update(prepared)?;
    renderer.render(frame, view)?;
    let grown = readback(device, queue, texture)?;
    assert!(
        grown
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(128) <= 1
                && pixel[1].abs_diff(64) <= 1
                && pixel[2].abs_diff(128) <= 1)
    );
    let values = renderer.parameters().values();
    assert!(
        renderer
            .begin_parameter_update(
                serde_json::json!({"gain":0,"points":[[1,2]]})
                    .as_object()
                    .unwrap()
            )
            .is_err()
    );
    assert_eq!(renderer.parameters().values(), values);
    let stale = renderer
        .begin_parameter_update(serde_json::json!({"gain":0.1}).as_object().unwrap())?
        .prepare()
        .await?;
    let shrink = renderer
        .begin_parameter_update(
            serde_json::json!({
                "gain":1,"index":0,"weights":[0.25],"points":[[0,0.75,0.5]]
            })
            .as_object()
            .unwrap(),
        )?
        .prepare()
        .await?;
    renderer.apply_parameter_update(shrink)?;
    assert!(
        renderer.apply_parameter_update(stale).is_err(),
        "older preparation cannot undo a committed edit"
    );
    renderer.render(frame, view)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(64) <= 1
                && pixel[1].abs_diff(191) <= 1
                && pixel[2].abs_diff(128) <= 1)
    );
    let other_update = renderer
        .begin_parameter_update(serde_json::json!({"gain":0.25}).as_object().unwrap())?
        .prepare()
        .await?;
    let mut replacement = CanvasRenderer::prepare(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "storage_probe",
        FORMAT,
    )
    .await?;
    assert!(
        replacement.apply_parameter_update(other_update).is_err(),
        "updates are tied to their renderer"
    );
    let empty = renderer
        .begin_parameter_update(
            serde_json::json!({"weights":[],"points":[]})
                .as_object()
                .unwrap(),
        )?
        .prepare()
        .await?;
    renderer.apply_parameter_update(empty)?;
    renderer.render(frame, view)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 0, 0, 255])
    );
    assert_eq!(
        renderer.parameters().values()["points"],
        serde_json::json!([])
    );
    Ok(())
}

async fn verify_intermediate_targets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut CanvasRenderer,
    frame: FrameInputs,
) -> Result<(), Box<dyn Error>> {
    use fresco_example_engine::runtime::{
        pass_plan::ValidatedPassPlan, targets::IntermediateTargets,
    };
    let mut files = fresco_example_engine::source_files();
    files.insert("main.fr".into(), "canvas probe(ctx: CanvasContext) -> color { compose { grey(ctx.uv.x) |> blur(radius: 2px) } }".into());
    let artifact = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("compile failed: {errors:#?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&artifact.manifest)?;
    let mut plan = manifest.canvases[0].pass_plan.clone();
    assert!(!plan.targets.is_empty());
    let id = plan.targets[0].id;
    for target in &mut plan.targets {
        target.format = "rgba8unorm".into();
    }
    let validated = ValidatedPassPlan::new(&plan)?;
    let installed = IntermediateTargets::prepare(device, &validated, [SIZE, SIZE]).await?;
    assert_eq!(installed.len(), plan.targets.len());
    assert_eq!(installed.size(), [SIZE, SIZE]);
    assert!(installed.get(usize::MAX).is_none());
    let target = installed.get(id).expect("compiled target");
    renderer.render(frame, target.view())?;
    let pixels = readback(device, queue, target.texture())?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 255, 0, 255])
    );
    let oversized = device
        .limits()
        .max_texture_dimension_2d
        .checked_add(1)
        .unwrap();
    assert!(
        IntermediateTargets::prepare(device, &validated, [oversized, SIZE])
            .await
            .is_err()
    );
    // Failed preparation leaves the installed views and their pixels usable.
    assert_eq!(readback(device, queue, target.texture())?, pixels);
    renderer.render(frame, target.view())?;
    assert_eq!(readback(device, queue, target.texture())?, pixels);
    for descriptor in &mut plan.targets {
        descriptor.scale = 0.5;
        descriptor.format = "rgba16float".into();
    }
    let resized_plan = ValidatedPassPlan::new(&plan)?;
    let replacement = IntermediateTargets::prepare(device, &resized_plan, [128, 96]).await?;
    let resized = replacement.get(id).unwrap().texture();
    assert_eq!([resized.width(), resized.height()], [64, 48]);
    assert_eq!(resized.format(), wgpu::TextureFormat::Rgba16Float);
    assert_eq!(target.texture().format(), FORMAT);
    assert_eq!(readback(device, queue, target.texture())?, pixels);
    let suspended = IntermediateTargets::prepare(device, &validated, [0, SIZE]).await?;
    assert!(suspended.is_empty());
    assert_eq!(suspended.size(), [0, SIZE]);
    Ok(())
}

async fn verify_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    view: &wgpu::TextureView,
    frame: FrameInputs,
) -> Result<(), Box<dyn Error>> {
    use fresco_example_engine::runtime::textures::{TextureImage, TextureInputs};
    let mut files = fresco_example_engine::source_files();
    files.insert("main.fr".into(), "canvas texture_probe(ctx: CanvasContext) -> color { param offset: f32 = 0.0; param strength: f32 = 1.0; param weights: array<f32> = [1.0]; uniform paint: texture; paint.at(vec2(0.25 + offset, 0.25)) |> opacity(strength * weights[0]) }".into());
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("texture compile failed: {errors:#?}"))?;
    let mut manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let inputs = TextureInputs::from([(
        "paint".into(),
        TextureImage {
            width: 2,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ]
            .into(),
        },
    )]);
    let mut renderer = CanvasRenderer::prepare_with_textures(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "texture_probe",
        FORMAT,
        &inputs,
    )
    .await?;
    renderer.render(frame, view)?;
    let red = readback(device, queue, texture)?;
    assert!(
        red.as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 255]),
        "texture row/channel orientation"
    );
    for (offset, expected) in [
        (0.5, [0, 255, 0, 255]),
        (1.0, [255, 0, 0, 255]),
        (0.25, [128, 128, 0, 255]),
    ] {
        renderer.update_parameters(serde_json::json!({"offset":offset}).as_object().unwrap())?;
        renderer.render(frame, view)?;
        assert!(
            readback(device, queue, texture)?
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 1)),
            "repeat/linear sampling at offset {offset}"
        );
    }
    renderer.update_parameters(
        serde_json::json!({"offset": 0.0, "strength": 0.5})
            .as_object()
            .unwrap(),
    )?;
    renderer.render(frame, view)?;
    let previous = readback(device, queue, texture)?;
    assert!(
        previous.as_chunks::<4>().0.iter().all(|pixel| pixel
            .iter()
            .zip([128, 0, 0, 255])
            .all(|(actual, expected)| actual.abs_diff(expected) <= 1)),
        "standalone texture opacity must apply coverage against the canvas background"
    );
    let edited = renderer
        .begin_parameter_update(
            serde_json::json!({"strength":1,"weights":[0.5,0.25]})
                .as_object()
                .unwrap(),
        )?
        .prepare()
        .await?;
    renderer.apply_parameter_update(edited)?;
    renderer.render(frame, view)?;
    assert_eq!(
        readback(device, queue, texture)?,
        previous,
        "storage replacement retains texture resources"
    );
    assert!(
        CanvasRenderer::prepare(
            device.clone(),
            queue.clone(),
            &output.wgsl,
            &manifest,
            "texture_probe",
            FORMAT
        )
        .await
        .is_err()
    );
    let invalid = TextureInputs::from([(
        "paint".into(),
        TextureImage {
            width: 2,
            height: 2,
            pixels: vec![0; 4].into(),
        },
    )]);
    assert!(
        CanvasRenderer::prepare_with_textures(
            device.clone(),
            queue.clone(),
            &output.wgsl,
            &manifest,
            "texture_probe",
            FORMAT,
            &invalid
        )
        .await
        .is_err()
    );
    renderer.render(frame, view)?;
    assert_eq!(readback(device, queue, texture)?, previous);

    // Move reflected texture/sampler bindings and the corresponding shader
    // declarations together. Runtime binding locations must follow the artifact.
    let mut module = naga::front::wgsl::parse_str(&output.wgsl)?;
    let def = &mut manifest.canvases[0].textures[0];
    let old_texture = (def.group, def.binding);
    def.group = 2;
    def.binding = 7;
    let sampler = manifest.canvases[0].sampler.as_mut().unwrap();
    let old_sampler = (sampler.group, sampler.binding);
    sampler.group = 1;
    sampler.binding = 5;
    for (_, global) in module.global_variables.iter_mut() {
        if let Some(binding) = &mut global.binding {
            if (binding.group, binding.binding) == old_texture {
                binding.group = 2;
                binding.binding = 7;
            } else if (binding.group, binding.binding) == old_sampler {
                binding.group = 1;
                binding.binding = 5;
            }
        }
    }
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)?;
    let moved =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())?;
    let mut rebound = CanvasRenderer::prepare_with_textures(
        device.clone(),
        queue.clone(),
        &moved,
        &manifest,
        "texture_probe",
        FORMAT,
        &inputs,
    )
    .await?;
    rebound.render(frame, view)?;
    assert_eq!(readback(device, queue, texture)?, red);
    let update = rebound
        .begin_parameter_update(
            serde_json::json!({"weights":[0.5,0.25]})
                .as_object()
                .unwrap(),
        )?
        .prepare()
        .await?;
    rebound.apply_parameter_update(update)?;
    rebound.render(frame, view)?;
    assert_eq!(
        readback(device, queue, texture)?,
        previous,
        "mixed texture/storage bind groups survive buffer replacement"
    );
    Ok(())
}
