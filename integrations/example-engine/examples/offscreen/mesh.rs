use std::{collections::BTreeMap, error::Error};

use fresco_artifact::ManifestRoot;
use fresco_example_engine::{
    profile::{
        FrameInputs,
        mesh::{DisplacementInputs, MeshSceneInputs},
    },
    runtime::{
        mesh::{DEPTH_FORMAT, MeshRenderer},
        mesh_geometry::MeshGeometry,
        vertices::VertexValues,
    },
};

use super::{FORMAT, SIZE, readback};

fn compile(changed_vertex: bool) -> Result<(String, ManifestRoot), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    if changed_vertex {
        let contract = files.get_mut("engine/core/05_mesh_contract.fr").unwrap();
        assert!(contract.contains("        uv,\n"));
        *contract = contract.replace("        uv,\n", "        vec2(0.25, 0.75),\n");
    }
    files.insert("main.fr".into(), "surface probe(sp: surf) -> material(unlit) { compose { base(albedo: rgba(sp.uv.x, sp.uv.y, time(), 1.0)) } }".into());
    let bundle = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("mesh compile failed: {errors:#?}"))?;
    Ok((bundle.wgsl, serde_json::from_str(&bundle.manifest)?))
}

pub(super) async fn geometry(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    manifest: &ManifestRoot,
) -> Result<MeshGeometry, Box<dyn Error>> {
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| f.name == "preview_static")
        .unwrap();
    let streams = BTreeMap::from([
        (
            "position".into(),
            VertexValues::F32(&[-1.0, -1.0, 0.0, 3.0, -1.0, 0.0, -1.0, 3.0, 0.0]),
        ),
        (
            "normal".into(),
            VertexValues::F32(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]),
        ),
        (
            "tangent".into(),
            VertexValues::F32(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        ),
        (
            "uv".into(),
            VertexValues::F32(&[0.0, 0.0, 2.0, 0.0, 0.0, 2.0]),
        ),
        (
            "uv2".into(),
            VertexValues::F32(&[0.0, 0.0, 2.0, 0.0, 0.0, 2.0]),
        ),
    ]);
    Ok(MeshGeometry::prepare(device, queue, factory, 3, &streams, Some(&[0, 1, 2])).await?)
}

pub(super) async fn verify(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    target: &wgpu::TextureView,
) -> Result<(), Box<dyn Error>> {
    let depth_target =
        fresco_example_engine::runtime::depth::DepthTarget::prepare(device, [SIZE, SIZE])
            .await?
            .expect("nonzero size");
    assert_eq!(depth_target.texture().format(), DEPTH_FORMAT);
    let depth = depth_target.view().clone();
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut inputs = MeshSceneInputs {
        model: identity,
        view: identity,
        projection: identity,
        camera_position: [0.0, 0.0, 2.0],
        frame: FrameInputs {
            time: 0.25,
            delta_time: 0.0,
            physical_size: [SIZE, SIZE],
        },
        displacement: DisplacementInputs {
            enabled: false,
            amplitude: 0.0,
            frequency: 0.0,
            speed: 0.0,
        },
    };
    let (wgsl, manifest) = compile(false)?;
    let mut renderer = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &wgsl,
        &manifest,
        "probe",
        geometry(device, queue, &manifest).await?,
        FORMAT,
    )
    .await?;
    assert!(renderer.render(&inputs, target, &depth)?);
    let first = readback(device, queue, texture)?;
    let oversized = device
        .limits()
        .max_texture_dimension_2d
        .checked_add(1)
        .expect("representable oversized target");
    assert!(
        fresco_example_engine::runtime::depth::DepthTarget::prepare(device, [oversized, SIZE])
            .await
            .is_err()
    );
    let resized = fresco_example_engine::runtime::depth::DepthTarget::prepare(device, [32, SIZE])
        .await?
        .unwrap();
    assert_eq!(resized.size(), [32, SIZE]);
    assert_eq!(
        [resized.texture().width(), resized.texture().height()],
        [32, SIZE]
    );
    assert!(
        fresco_example_engine::runtime::depth::DepthTarget::prepare(device, [0, SIZE])
            .await?
            .is_none()
    );
    renderer.render(&inputs, target, &depth)?;
    assert_eq!(
        readback(device, queue, texture)?,
        first,
        "depth candidates and failed allocation preserve the installed target"
    );

    let mut malformed = manifest.clone();
    let factory = malformed
        .vertex_factories
        .iter_mut()
        .find(|f| f.name == "preview_static")
        .unwrap();
    factory.attributes[1] = factory.attributes[0].clone();
    assert!(
        MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            &wgsl,
            &malformed,
            "probe",
            geometry(device, queue, &manifest).await?,
            FORMAT,
        )
        .await
        .is_err(),
        "duplicated reflected attributes cannot masquerade as a compatible geometry layout"
    );
    assert!(
        first
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[2].abs_diff(64) <= 1 && pixel[3] == 255),
        "authored material and frame uniform reach the mesh fragment stage"
    );
    assert!(first.as_chunks::<4>().0.iter().any(|pixel| pixel[0] < 32));
    assert!(first.as_chunks::<4>().0.iter().any(|pixel| pixel[0] > 224));
    inputs.frame.time = 0.75;
    renderer.render(&inputs, target, &depth)?;
    let later = readback(device, queue, texture)?;
    assert!(
        later
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[2].abs_diff(191) <= 1 && pixel[3] == 255)
    );
    assert!(
        MeshRenderer::prepare(
            device.clone(),
            queue.clone(),
            "invalid shader",
            &manifest,
            "probe",
            geometry(device, queue, &manifest).await?,
            FORMAT
        )
        .await
        .is_err()
    );
    renderer.render(&inputs, target, &depth)?;
    assert_eq!(
        readback(device, queue, texture)?,
        later,
        "failed preparation preserves the installed material"
    );
    inputs.frame.physical_size = [0, SIZE];
    assert!(!renderer.render(&inputs, target, &depth)?);
    assert_eq!(readback(device, queue, texture)?, later);
    inputs.frame.physical_size = [SIZE, SIZE];
    inputs.model[0] = f32::NAN;
    assert!(renderer.render(&inputs, target, &depth).is_err());
    assert_eq!(readback(device, queue, texture)?, later);
    inputs.model = identity;
    let (changed, manifest) = compile(true)?;
    let mut changed = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &changed,
        &manifest,
        "probe",
        geometry(device, queue, &manifest).await?,
        FORMAT,
    )
    .await?;
    changed.render(&inputs, target, &depth)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[0].abs_diff(64) <= 1
                && pixel[1].abs_diff(191) <= 1
                && pixel[2].abs_diff(191) <= 1
                && pixel[3] == 255),
        "authored vertex hook controls the mesh's material coordinates"
    );
    verify_parameters(device, queue, texture, target, &depth, &inputs).await?;
    verify_textures(device, queue, texture, target, &depth, &inputs).await?;
    println!(
        "Verified authored mesh stages, scene/frame inputs, zero-size suspension, invalid-input rejection, and failed-preparation isolation."
    );
    Ok(())
}

async fn verify_parameters(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    target: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    inputs: &MeshSceneInputs,
) -> Result<(), Box<dyn Error>> {
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        include_str!("../material_parameters.fr").into(),
    );
    let bundle = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("material parameter probe: {errors:#?}"))?;
    let manifest: ManifestRoot = serde_json::from_str(&bundle.manifest)?;
    let mut renderer = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &bundle.wgsl,
        &manifest,
        "adjustable",
        geometry(device, queue, &manifest).await?,
        FORMAT,
    )
    .await?;
    renderer.render(inputs, target, depth)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 0, 0, 255]),
        "material defaults reach their reflected uniform bindings"
    );
    renderer.update_parameters(
        serde_json::json!({"tint":[1,1,1,1],"gain":0.5})
            .as_object()
            .unwrap(),
    )?;
    renderer.render(inputs, target, depth)?;
    let edited = readback(device, queue, texture)?;
    assert!(
        edited
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0].abs_diff(128) <= 1 && p[1..] == [255, 255, 255])
    );
    let values = renderer.parameter_values();
    assert!(
        renderer
            .update_parameters(
                serde_json::json!({"gain":0,"tint":[1,2,3]})
                    .as_object()
                    .unwrap()
            )
            .is_err()
    );
    assert_eq!(renderer.parameter_values(), values);
    renderer.render(inputs, target, depth)?;
    assert_eq!(
        readback(device, queue, texture)?,
        edited,
        "invalid batches preserve every material uniform"
    );
    renderer.update_parameters(
        serde_json::json!({"enabled":false,"steps":4})
            .as_object()
            .unwrap(),
    )?;
    renderer.render(inputs, target, depth)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0].abs_diff(128) <= 1 && p[1..] == [0, 0, 255]),
        "integer and boolean parameter edits affect authored branches"
    );
    println!(
        "Verified material scalar/color defaults, integer/boolean branches, live uniform edits, and rejected-batch isolation."
    );
    use fresco_example_engine::profile::preview::{PreviewGeometry, sphere_scene};
    let preview = PreviewGeometry::sphere();
    let factory = manifest
        .vertex_factories
        .iter()
        .find(|f| f.name == "preview_static")
        .unwrap();
    let geometry = MeshGeometry::prepare(
        device,
        queue,
        factory,
        preview.vertex_count,
        &preview.streams(),
        Some(&preview.indices),
    )
    .await?;
    let mut preview_renderer = MeshRenderer::prepare(
        device.clone(),
        queue.clone(),
        &bundle.wgsl,
        &manifest,
        "adjustable",
        geometry,
        FORMAT,
    )
    .await?;
    preview_renderer.render(&sphere_scene(inputs.frame), target, depth)?;
    let sphere = readback(device, queue, texture)?;
    assert!(
        sphere.as_chunks::<4>().0.contains(&[255, 0, 0, 255]),
        "native preview sphere is visible with back-face culling and perspective"
    );
    assert!(
        sphere.as_chunks::<4>().0.contains(&[0, 0, 0, 0]),
        "preview camera leaves background around the sphere"
    );
    Ok(())
}

async fn verify_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    target: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    scene: &MeshSceneInputs,
) -> Result<(), Box<dyn Error>> {
    use fresco_example_engine::runtime::{
        mesh::MeshResources,
        textures::{TextureImage, TextureInputs},
    };
    let mut files = fresco_example_engine::source_files();
    files.insert(
        "main.fr".into(),
        include_str!("../material_texture.fr").into(),
    );
    let output = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)
        .map_err(|errors| format!("material texture probe: {errors:#?}"))?;
    let mut manifest: ManifestRoot = serde_json::from_str(&output.manifest)?;
    let images = TextureInputs::from([(
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
    let mut renderer = MeshRenderer::prepare_with_resources(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "textured",
        MeshResources {
            factory: None,
            geometry: geometry(device, queue, &manifest).await?,
            textures: &images,
            parameters: None,
        },
        FORMAT,
    )
    .await?;
    renderer.render(scene, target, depth)?;
    let red = readback(device, queue, texture)?;
    assert!(
        red.as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [255, 0, 0, 255]),
        "material texture row and channel orientation"
    );
    for (offset, expected) in [(0.5, [0, 255, 0, 255]), (1.0, [255, 0, 0, 255])] {
        renderer.update_parameters(serde_json::json!({"offset":offset}).as_object().unwrap())?;
        renderer.render(scene, target, depth)?;
        assert!(
            readback(device, queue, texture)?
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == expected)
        );
    }
    renderer.update_parameters(serde_json::json!({"offset":0.25}).as_object().unwrap())?;
    renderer.render(scene, target, depth)?;
    assert!(
        readback(device, queue, texture)?
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0].abs_diff(128) <= 1 && p[1].abs_diff(128) <= 1 && p[2..] == [0, 255]),
        "material textures use linear filtering"
    );
    renderer.update_parameters(
        serde_json::json!({"offset":0,"strength":0.5})
            .as_object()
            .unwrap(),
    )?;
    renderer.render(scene, target, depth)?;
    let before = readback(device, queue, texture)?;
    let values = renderer.parameter_values();
    assert!(
        MeshRenderer::prepare_with_resources(
            device.clone(),
            queue.clone(),
            &output.wgsl,
            &manifest,
            "textured",
            MeshResources {
                factory: None,
                geometry: geometry(device, queue, &manifest).await?,
                textures: &TextureInputs::new(),
                parameters: Some(&values)
            },
            FORMAT
        )
        .await
        .is_err()
    );
    renderer.render(scene, target, depth)?;
    assert_eq!(
        readback(device, queue, texture)?,
        before,
        "missing image input preserves the installed material"
    );
    let replacement_images = TextureInputs::from([(
        "paint".into(),
        TextureImage {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 255, 255].into(),
        },
    )]);
    let mut replacement = MeshRenderer::prepare_with_resources(
        device.clone(),
        queue.clone(),
        &output.wgsl,
        &manifest,
        "textured",
        MeshResources {
            factory: None,
            geometry: geometry(device, queue, &manifest).await?,
            textures: &replacement_images,
            parameters: Some(&values),
        },
        FORMAT,
    )
    .await?;
    replacement.render(scene, target, depth)?;
    let blue = readback(device, queue, texture)?;
    assert!(
        blue.as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0..2] == [0, 0] && p[2].abs_diff(128) <= 1 && p[3] == 255),
        "texture replacement includes edited material parameters"
    );
    let surface = manifest
        .surfaces
        .iter_mut()
        .find(|s| s.name == "textured")
        .unwrap();
    let image = &mut surface.textures[0];
    let old_image = (image.group, image.binding);
    image.group = 0;
    image.binding = 8;
    let sampler = surface.sampler.as_mut().unwrap();
    let old_sampler = (sampler.group, sampler.binding);
    sampler.group = 0;
    sampler.binding = 9;
    let mut module = naga::front::wgsl::parse_str(&output.wgsl)?;
    for (_, global) in module.global_variables.iter_mut() {
        if let Some(binding) = &mut global.binding {
            let old = (binding.group, binding.binding);
            if old == old_image {
                binding.group = 0;
                binding.binding = 8;
            } else if old == old_sampler {
                binding.group = 0;
                binding.binding = 9;
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
    let mut mixed = MeshRenderer::prepare_with_resources(
        device.clone(),
        queue.clone(),
        &moved,
        &manifest,
        "textured",
        MeshResources {
            factory: None,
            geometry: geometry(device, queue, &manifest).await?,
            textures: &replacement_images,
            parameters: Some(&values),
        },
        FORMAT,
    )
    .await?;
    mixed.render(scene, target, depth)?;
    assert_eq!(
        readback(device, queue, texture)?,
        blue,
        "textures and material uniforms can share a reflected bind group"
    );
    println!(
        "Verified material textures, repeat/linear sampling, failed replacement, parameter restoration, and mixed resource groups."
    );
    Ok(())
}
