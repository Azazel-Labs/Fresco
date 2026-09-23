//! Local readback of compiler-authored fullscreen variants.
use super::{Error, FORMAT, FrameInputs, ManifestRoot, SIZE, readback};
use fresco_example_engine::runtime::{
    canvas::{CanvasInputs, CanvasRenderer},
    canvas_variant::VariantSelection,
    textures::TextureInputs,
};

pub async fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    let groups = files.get_mut("engine/core/01_core.fr").unwrap();
    *groups = groups
        .replace("@group(0) group material", "@group(1) group material")
        .replace("@group(1) group textures", "@group(0) group textures");
    let contract = files.get_mut("engine/core/04_canvas_contract.fr").unwrap();
    *contract = contract.replace("    binding {", "    permutations { @known(compile) quality: \"low\" | \"high\" }\n    binding {")
        .replace("        return t.draw(ctx)", "        if quality == \"low\" { return rgba(1.0, 0.0, 0.0, 1.0) }\n        return t.draw(ctx)");
    files.insert(
        "main.fr".into(),
        "canvas probe(ctx: CanvasContext) -> color { rgba(0.0, 1.0, 0.0, 1.0) }".into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("variant compilation: {errors:#?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let textures = TextureInputs::new();
    let view = texture.create_view(&Default::default());
    for (value, expected) in [("low", [255, 0, 0, 255]), ("high", [0, 255, 0, 255])] {
        let selection = VariantSelection::from([("quality".into(), value.into())]);
        let mut renderer = CanvasRenderer::prepare_with_inputs(
            device.clone(),
            queue.clone(),
            &output.wgsl,
            &manifest,
            "probe",
            FORMAT,
            CanvasInputs {
                textures: &textures,
                variant: Some(&selection),
            },
        )
        .await?;
        renderer.render(
            FrameInputs {
                time: 0.0,
                delta_time: 0.0,
                physical_size: [SIZE, SIZE],
            },
            &view,
        )?;
        let pixels = readback(device, queue, texture)?;
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == expected),
            "native {value} variant must execute its authored shade branch"
        );
    }
    println!("Native fullscreen variant readback passed");
    Ok(())
}
