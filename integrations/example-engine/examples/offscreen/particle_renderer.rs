use super::{FORMAT, SIZE, readback};
use fresco_artifact::ManifestRoot;
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{
        depth::DepthTarget,
        particles::{ParticleInputs, ParticleRenderer},
        textures::TextureInputs,
    },
};
use std::error::Error;

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    verify_environment(device, queue, target, view).await?;
    let mut files = fresco_example_engine::source_files();
    let source=include_str!("../../../../examples/50) particles/drifting_sparks.fr")
        .replace("spawn_rate: 60.0","spawn_rate: 4.0\n    allocation: ParticleAllocationMode.Automatic\n    allocation_hint: 1\n    max_particles: 32")
        .replace("particle_fade_size(0.055)","particle_fade_size(0.3)")
        .replace("    let radius =", "    param tint: color = #fff\n    let radius =")
        .replace("rgba(1.0, 0.15 + 0.75 * core, 0.03 + 0.35 * core, 1.0)","tint");
    files.insert("main.fr".into(), source);
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let textures = TextureInputs::new();
    let mut renderer = ParticleRenderer::prepare(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "drifting_sparks",
        ParticleInputs {
            textures: &textures,
            parameters: None,
        },
        FORMAT,
    )
    .await?;
    assert_eq!(renderer.capacity(), 1);
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let frame = FrameInputs {
        time: 0.25,
        delta_time: 0.25,
        physical_size: [SIZE, SIZE],
    };
    let pending = renderer.begin_frame(&sphere_scene(frame))?.unwrap();
    renderer.reset()?;
    let obsolete = pending.prepare().await?;
    assert!(
        renderer
            .render_prepared(obsolete, view, depth.view())
            .is_err()
    );
    assert_eq!(
        renderer.capacity(),
        1,
        "reset rejects pending growth without installing it"
    );
    renderer
        .render(&sphere_scene(frame), view, depth.view())
        .await?;
    assert_eq!(
        renderer.capacity(),
        16,
        "birth reservations trigger buffer growth"
    );
    let first = readback(device, queue, target)?;
    assert!(
        first
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] > 200 && p[3] > 200),
        "authored particles must be visible"
    );
    let paused = FrameInputs {
        delta_time: 0.0,
        ..frame
    };
    let competing = renderer
        .begin_frame(&sphere_scene(frame))?
        .unwrap()
        .prepare()
        .await?;
    renderer
        .render(&sphere_scene(paused), view, depth.view())
        .await?;
    assert!(
        renderer
            .render_prepared(competing, view, depth.view())
            .is_err()
    );
    assert_eq!(
        readback(device, queue, target)?,
        first,
        "paused frames preserve particle state"
    );
    assert!(
        !renderer
            .render(
                &sphere_scene(FrameInputs {
                    physical_size: [0, 0],
                    delta_time: 1.0,
                    ..frame
                }),
                view,
                depth.view()
            )
            .await?
    );
    assert!(
        renderer
            .render(
                &sphere_scene(FrameInputs {
                    delta_time: f32::NAN,
                    ..frame
                }),
                view,
                depth.view()
            )
            .await
            .is_err()
    );
    renderer
        .render(&sphere_scene(paused), view, depth.view())
        .await?;
    assert_eq!(readback(device, queue, target)?, first);
    renderer
        .render(
            &sphere_scene(FrameInputs { time: 0.5, ..frame }),
            view,
            depth.view(),
        )
        .await?;
    assert_ne!(
        readback(device, queue, target)?,
        first,
        "forward playback moves particles"
    );
    renderer.reset()?;
    renderer
        .render(&sphere_scene(frame), view, depth.view())
        .await?;
    assert_eq!(
        readback(device, queue, target)?,
        first,
        "reset and replay reproduce the initial state"
    );
    renderer.update_parameters(&serde_json::Map::from_iter([(
        "tint".into(),
        serde_json::json!([0, 1, 0, 1]),
    )]))?;
    renderer
        .render(&sphere_scene(paused), view, depth.view())
        .await?;
    let green = readback(device, queue, target)?;
    assert!(
        green
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] < 10 && p[1] > 200 && p[3] > 200)
    );
    assert_ne!(green, first);
    assert!(
        ParticleRenderer::prepare(
            device.clone(),
            queue.clone(),
            "invalid WGSL",
            &manifest,
            "drifting_sparks",
            ParticleInputs {
                textures: &textures,
                parameters: None
            },
            FORMAT
        )
        .await
        .is_err()
    );
    renderer
        .render(&sphere_scene(paused), view, depth.view())
        .await?;
    assert_eq!(readback(device, queue, target)?, green);
    println!(
        "Particle renderer GPU checks: automatic growth, authored simulation/draw, pause/reset, invalid input/replacement, zero size, and material edits passed."
    );
    Ok(())
}

async fn verify_environment(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    use fresco_example_engine::{
        profile::{
            camera::OrbitCamera,
            preview::{PreviewGeometry, PreviewShape},
        },
        runtime::{mesh::MeshRenderer, mesh_geometry::MeshGeometry},
    };
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::preview_source_files();
        files.insert(
            "fresco.config.json".into(),
            serde_json::json!({"renderer": path}).to_string(),
        );
        files.insert(
            "main.fr".into(),
            include_str!("../../../../examples/50) particles/drifting_sparks.fr")
                .replace("particle_fade_size(0.055)", "particle_fade_size(0.3)"),
        );
        let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
        for ground_height in [-0.92, 0.25] {
            let factory = manifest
                .vertex_factories
                .iter()
                .find(|f| f.name == "preview_static")
                .unwrap();
            let mesh = PreviewGeometry::plane();
            let geometry = MeshGeometry::prepare(
                device,
                queue,
                factory,
                mesh.vertex_count,
                &mesh.streams(),
                Some(&mesh.indices),
            )
            .await?;
            let mut floor = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &output.wgsl,
                &manifest,
                "fresco_scene_ground",
                geometry,
                FORMAT,
            )
            .await?;
            floor.set_background([0.075, 0.11, 0.17, 1.0], [0.23, 0.26, 0.30, 1.0])?;
            let model = [
                4.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                4.0,
                0.0,
                0.0,
                ground_height,
                0.0,
                1.0,
            ];
            let mut camera = OrbitCamera::default();
            camera.drag([45.0, 40.0])?;
            let scene = camera.scene(
                PreviewShape::Sphere,
                FrameInputs {
                    time: 0.25,
                    delta_time: 0.25,
                    physical_size: [SIZE, SIZE],
                },
            );
            let mut floor_scene = scene;
            floor_scene.model = model;
            floor.render(&floor_scene, view, depth.view())?;
            let background = readback(device, queue, target)?;
            let mut particles = ParticleRenderer::prepare(
                device.clone(),
                queue.clone(),
                &output.wgsl,
                &manifest,
                "drifting_sparks",
                ParticleInputs {
                    textures: &TextureInputs::new(),
                    parameters: None,
                },
                FORMAT,
            )
            .await?;
            particles.set_environment(floor, model);
            particles.render(&scene, view, depth.view()).await?;
            let composite = readback(device, queue, target)?;
            assert!(
                composite.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
                "{path}: particles must preserve the opaque environment"
            );
            let changed = composite
                .as_chunks::<4>()
                .0
                .iter()
                .zip(background.as_chunks::<4>().0)
                .filter(|(a, b)| a != b)
                .count();
            if ground_height < 0.0 {
                assert!(
                    changed > 0 && changed < (SIZE * SIZE).div_euclid(2) as usize,
                    "{path}: visible particles blend over an otherwise preserved scene"
                );
            } else {
                assert_eq!(
                    changed, 0,
                    "{path}: the raised ground must occlude particles below it"
                );
            }
            particles.reset()?;
            let mut paused = scene;
            paused.frame.delta_time = 0.0;
            particles.render(&paused, view, depth.view()).await?;
            assert_eq!(
                readback(device, queue, target)?,
                background,
                "{path}: empty particle previews still render the environment"
            );
        }
    }
    Ok(())
}
