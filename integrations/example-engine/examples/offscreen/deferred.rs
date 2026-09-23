//! Actual integer-attachment readback: IDs follow the engine table, never float-packed.
use super::{FORMAT, SIZE, mesh::geometry, readback};
use fresco_example_engine::{
    profile::{FrameInputs, preview::sphere_scene},
    runtime::{depth::DepthTarget, mesh::MeshRenderer},
};
use std::error::Error;
pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files_for_deferred();
    files.insert("main.fr".into(), "surface earlier(sp: surf) -> material(unlit) { compose { base(albedo: rgba(0.1, 0.2, 0.3, 1.0)) } }\nsurface probe(sp: surf) -> material(standard) { compose { base(albedo: rgba(0.8, 0.4, 0.2, 1.0), roughness: 0.3, metallic: 0.4) } }".into());
    let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
    let expected_id = manifest
        .tables
        .iter()
        .find(|t| t.name == "DrawRecord")
        .unwrap()
        .records
        .iter()
        .find(|r| r.key == "probe")
        .unwrap()
        .index;
    assert_eq!(expected_id, 2);
    let mut renderer = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &compiled.wgsl,
        &manifest,
        "probe",
        geometry(device, queue, &manifest).await?,
        FORMAT,
    )
    .await?;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    renderer.render(&scene, view, depth.view())?;
    let lit = readback(device, queue, output)?;
    // Identical scene and material must retain their lighting when changing paths.
    // Allow only the small loss introduced by the deferred G-buffer encoding.
    let mut forward_files = fresco_example_engine::source_files_for_renderer(true);
    forward_files.insert("main.fr".into(), files["main.fr"].clone());
    let forward = fresco::driver::compile_bundle_virtual(&forward_files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let forward_manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&forward.manifest)?;
    let mut forward_renderer = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &forward.wgsl,
        &forward_manifest,
        "probe",
        geometry(device, queue, &forward_manifest).await?,
        FORMAT,
    )
    .await?;
    forward_renderer.render(&scene, view, depth.view())?;
    let forward_pixels = readback(device, queue, output)?;
    let differences: Vec<_> = lit
        .iter()
        .zip(&forward_pixels)
        .map(|(a, b)| a.abs_diff(*b))
        .collect();
    let maximum = *differences.iter().max().unwrap();
    let mean = differences.iter().map(|d| f64::from(*d)).sum::<f64>() / differences.len() as f64;
    assert!(
        maximum <= 8 && mean <= 0.5,
        "Forward+/deferred lighting mismatch: max={maximum}, mean={mean}"
    );
    use fresco_example_engine::runtime::forward_plus::LightingEnvironment;
    let mut environment_pixels = vec![lit.clone()];
    for environment in [LightingEnvironment::Unlit, LightingEnvironment::Directional] {
        renderer.set_lighting_environment(environment)?;
        forward_renderer.set_lighting_environment(environment)?;
        renderer.render(&scene, view, depth.view())?;
        let deferred_pixels = readback(device, queue, output)?;
        forward_renderer.render(&scene, view, depth.view())?;
        let forward_pixels = readback(device, queue, output)?;
        assert!(
            deferred_pixels
                .iter()
                .zip(&forward_pixels)
                .all(|(a, b)| a.abs_diff(*b) <= 8),
            "lighting environment must match across paths"
        );
        assert!(
            environment_pixels
                .iter()
                .all(|pixels| pixels != &deferred_pixels),
            "each environment must visibly change the lighting"
        );
        environment_pixels.push(deferred_pixels);
    }
    renderer.set_lighting_environment(LightingEnvironment::ThreeLights)?;
    renderer.render(&scene, view, depth.view())?;
    let bytes = readback(device, queue, renderer.material_id_texture().unwrap())?;
    let ids: Vec<_> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|v| u32::from_le_bytes(v[..4].try_into().unwrap()))
        .collect();
    assert!(ids.contains(&0));
    assert!(ids.contains(&expected_id));
    assert!(ids.iter().all(|id| *id == 0 || *id == expected_id));
    for (material, pixel) in ids.iter().zip(bytes.as_chunks::<8>().0) {
        let instance = u32::from_le_bytes(pixel[4..].try_into().unwrap());
        assert_eq!(
            instance,
            u32::from(*material != 0),
            "draw identity is independent of material table index"
        );
    }
    // Reorder table identities and move the integer attachment without touching Rust.
    let engine = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
    *engine = engine
        .replace(
            "@table(surfaces, ascending, 1)",
            "@table(surfaces, descending, 1)",
        )
        .replace("@location(3) identity", "@location(6) identity");
    let recipe = files.get_mut("engine/config/renderer.fr").unwrap();
    *recipe = recipe.replace("@color(3, identity)", "@color(6, identity)");
    let changed = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|e| format!("{e:?}"))?;
    let changed_manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&changed.manifest)?;
    let mut candidate = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &changed.wgsl,
        &changed_manifest,
        "probe",
        geometry(device, queue, &changed_manifest).await?,
        FORMAT,
    )
    .await?;
    candidate.render(&scene, view, depth.view())?;
    assert_eq!(
        lit,
        readback(device, queue, output)?,
        "table remapping must preserve shading"
    );
    let remapped = readback(device, queue, candidate.material_id_texture().unwrap())?;
    let remapped: Vec<_> = remapped
        .as_chunks::<8>()
        .0
        .iter()
        .map(|v| u32::from_le_bytes(v[..4].try_into().unwrap()))
        .collect();
    assert!(remapped.contains(&1));
    assert!(remapped.iter().all(|id| *id == 0 || *id == 1));
    renderer.set_point_lights(&[])?;
    renderer.render(&scene, view, depth.view())?;
    let ambient = readback(device, queue, output)?;
    assert_ne!(ambient, lit);
    scene.projection = [0.0; 16];
    assert!(renderer.render(&scene, view, depth.view()).is_err());
    assert_eq!(ambient, readback(device, queue, output)?);
    println!(
        "Deferred GPU checks: exact integer material IDs, background coverage, tiled lighting, and invalid camera isolation passed."
    );
    verify_uv(device, queue, output, view).await?;
    Ok(())
}

async fn verify_uv(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    output: &wgpu::Texture,
    view: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let source = r#"
style Coordinates for standard : StandardStyle {
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 { return vec3(0.0) }
    fn finish(surface: StandardSurface, context: ShadingContext, lighting: LightingResult) -> vec3 {
        return vec3(fract(surface.uv * vec2(997.0, 991.0)), 0.3)
    }
}
surface item(sp: surf) -> material(standard) { properties { style: Coordinates }; compose { base(albedo: #fff) } }
"#;
    let shape = fresco_example_engine::profile::preview::PreviewGeometry::sphere();
    let mut reference: Option<Vec<u8>> = None;
    let mut scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    scene.displacement.enabled = false;
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for path in ["forward", "forward-plus", "deferred"] {
        let mut files = fresco_example_engine::source_files_for_recipe(path);
        files.insert("main.fr".into(), source.into());
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
            .map_err(|e| format!("{e:?}"))?;
        let manifest: fresco_artifact::ManifestRoot = serde_json::from_str(&compiled.manifest)?;
        let factory = manifest
            .vertex_factories
            .iter()
            .find(|f| f.name == "preview_static")
            .unwrap();
        let geometry = fresco_example_engine::runtime::mesh_geometry::MeshGeometry::prepare(
            device,
            queue,
            factory,
            shape.vertex_count,
            &shape.streams(),
            Some(&shape.indices),
        )
        .await?;
        let mut renderer = MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &compiled.wgsl,
            &manifest,
            "item",
            geometry.clone(),
            FORMAT,
        )
        .await?;
        renderer.render(&scene, view, depth.view())?;
        let pixels = readback(device, queue, output)?;
        if let Some(expected) = &reference {
            assert!(
                pixels
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(*b) <= 1),
                "{path}: surface UV lookup must preserve f32 precision across renderers"
            );
        } else {
            reference = Some(pixels.clone());
        }
        if path == "deferred" {
            let recipe = files.get_mut("engine/config/renderer.fr").unwrap();
            *recipe = recipe.replace(
                "@image(surface_uv, rg32float)",
                "@image(surface_uv, rg16float)",
            );
            let lossy = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                .map_err(|e| format!("{e:?}"))?;
            let manifest = serde_json::from_str(&lossy.manifest)?;
            let mut renderer = MeshRenderer::prepare(
                device.clone(),
                queue.clone(),
                &lossy.wgsl,
                &manifest,
                "item",
                geometry,
                FORMAT,
            )
            .await?;
            renderer.render(&scene, view, depth.view())?;
            let lossy_pixels = readback(device, queue, output)?;
            assert!(
                pixels
                    .iter()
                    .zip(lossy_pixels)
                    .any(|(a, b)| a.abs_diff(b) > 1),
                "precision probe must detect an accidental half-float UV attachment"
            );
        }
    }
    println!("Surface UVs: high-frequency lookup agrees across Forward, Forward+, and Deferred.");
    Ok(())
}
