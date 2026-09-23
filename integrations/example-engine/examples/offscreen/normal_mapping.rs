//! Tangent-space normals must follow the UV frame, including mirrored/degenerate UVs.
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
    let scene = sphere_scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [SIZE, SIZE],
    });
    let depth = DepthTarget::prepare(device, [SIZE, SIZE]).await?.unwrap();
    for deferred in [false, true] {
        for (uv, expected) in [
            ("uv", "vec3(0.6, 0.0, 0.8)"),
            ("vec2(-uv.x, uv.y)", "vec3(-0.6, 0.0, 0.8)"),
            ("vec2(0.0)", "vec3(0.0, 0.0, 1.0)"),
        ] {
            let mut images = Vec::new();
            for normal in [
                "normal_map: vec3(0.6, 0.0, 0.8)".to_string(),
                format!("normal: {expected}"),
            ] {
                let mut files = if deferred {
                    fresco_example_engine::source_files_for_deferred()
                } else {
                    fresco_example_engine::source_files_for_renderer(true)
                };
                let contract = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
                assert!(contract.contains("        uv,\n"));
                *contract = contract.replace("        uv,\n", &format!("        {uv},\n"));
                files.insert("main.fr".into(), format!("surface probe(sp: surf) -> material(standard) {{ compose {{ base(albedo: rgba(0.7, 0.4, 0.2, 1.0), roughness: 0.35, {normal}) }} }}"));
                let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
                    .map_err(|e| format!("{e:?}"))?;
                let manifest = serde_json::from_str(&compiled.manifest)?;
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
                renderer.render(&scene, view, depth.view())?;
                images.push(readback(device, queue, output)?);
            }
            assert!(
                images[0]
                    .iter()
                    .zip(&images[1])
                    .all(|(a, b)| a.abs_diff(*b) <= 1),
                "normal map must match analytic world normal: deferred={deferred}, uv={uv}"
            );
        }
    }
    println!(
        "Normal maps: analytic world-normal agreement, mirrored UVs, and degenerate-UV fallback passed in Forward+ and deferred."
    );
    Ok(())
}
