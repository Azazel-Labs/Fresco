//! Window, surface, file selection, and frame scheduling; no shader implementation.

use std::path::PathBuf;

mod renderer;
mod window;

#[derive(clap::Parser, Debug, Clone)]
#[command(about = "Run the embedded Fresco example engine", version)]
pub struct Options {
    /// Authored source file; omit to run the embedded demo.
    #[arg(long)]
    pub source: Option<PathBuf>,
    /// Replace the complete embedded engine with this directory's engine.fr.
    #[arg(long)]
    pub engine_dir: Option<PathBuf>,
    /// Renderer ID declared by the engine; omit to use its default.
    #[arg(long, default_value = "")]
    pub renderer: String,
    /// Canvas or surface name; required when the source declares multiple entries.
    #[arg(long)]
    pub entry: Option<String>,
    /// Preview geometry for surface entries: sphere, plane, or box.
    #[arg(long, default_value = "sphere")]
    pub mesh: fresco_example_engine::profile::preview::PreviewShape,
    /// JSON object of parameter overrides, for example '{"radius":0.25}'.
    #[arg(long, conflicts_with = "params_file")]
    pub params: Option<String>,
    /// Exact compile-known canvas bindings as a JSON object, e.g. '{"quality":"low"}'.
    #[arg(long)]
    pub variant: Option<String>,
    /// Read parameter overrides from a JSON file; --watch reloads it on edits.
    #[arg(long)]
    pub params_file: Option<PathBuf>,
    /// Override a texture by shader name: --texture checker=picture.png.
    #[arg(long)]
    pub texture: Vec<String>,
    /// Root for authored external asset identities (otherwise the source directory).
    #[arg(long)]
    pub asset_root: Option<PathBuf>,
    /// Compile and report the selected entry without initializing a window or GPU.
    #[arg(long)]
    pub check: bool,
    /// Watch .fr files under the source directory and engine override directory.
    #[arg(long)]
    pub watch: bool,
    /// Exit after this many presented frames (for local smoke testing).
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub frames: Option<u32>,
    /// Create a hidden window for automated local smoke testing.
    #[arg(long, requires = "frames")]
    pub hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Canvas,
    Mesh,
    Particles,
}

impl EntryKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Canvas => "canvas",
            Self::Mesh => "surface",
            Self::Particles => "particles",
        }
    }
}

pub struct PreparedArtifact {
    pub mesh: fresco_example_engine::profile::preview::PreviewShape,
    pub kind: EntryKind,
    pub artifact: crate::Artifact,
    pub entry: String,
    pub parameters: serde_json::Map<String, serde_json::Value>,
    pub variant: Option<fresco_example_engine::runtime::canvas_variant::VariantSelection>,
    pub textures: fresco_example_engine::runtime::textures::TextureInputs,
    pub asset_paths: Vec<PathBuf>,
}

pub fn compile(options: &Options) -> Result<PreparedArtifact, String> {
    let source = match &options.source {
        Some(path) => {
            std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?
        }
        None => crate::DEMO.into(),
    };
    let filename = options.source.as_ref().map_or_else(
        || "embedded-demo.fr".into(),
        |path| path.to_string_lossy().into_owned(),
    );
    let context = fresco::driver::CompileContext {
        renderer: (!options.renderer.is_empty()).then(|| options.renderer.clone()),
        ..Default::default()
    };
    let engine_files = fresco_example_engine::preview_source_files();
    let output = if let Some(engine) = &options.engine_dir {
        fresco::driver::compile_source_bundle_with_engine_dir(
            &source,
            &filename,
            false,
            &context,
            Some(engine),
        )
    } else if options.source.is_some() {
        fresco::driver::compile_source_bundle_with_engine_files(
            &source,
            &filename,
            false,
            &context,
            &engine_files,
            fresco_example_engine::ENTRYPOINT,
        )
    } else {
        let mut files = engine_files;
        files.insert(filename.clone(), source);
        fresco::driver::compile_bundle_virtual(&files, &filename, false)
    }
    .map_err(|errors| format!("compilation failed: {errors:#?}"))?;
    for diagnostic in &output.diagnostics {
        eprintln!("compiler: {}", diagnostic.message);
    }
    let manifest: fresco_artifact::ManifestRoot =
        serde_json::from_str(&output.manifest).map_err(|error| error.to_string())?;
    let entries: Vec<_> = manifest
        .canvases
        .iter()
        .map(|entry| (entry.name.as_str(), EntryKind::Canvas))
        .chain(
            manifest
                .surfaces
                .iter()
                .filter(|entry| entry.name != "fresco_scene_ground")
                .map(|entry| {
                    (
                        entry.name.as_str(),
                        if fresco_example_engine::runtime::particle_contract::uses_particles(
                            &manifest,
                            &entry.name,
                        ) {
                            EntryKind::Particles
                        } else {
                            EntryKind::Mesh
                        },
                    )
                }),
        )
        .filter(|(name, _)| {
            options
                .entry
                .as_ref()
                .is_none_or(|selected| selected == name)
        })
        .collect();
    if entries.is_empty()
        && let Some(name) = &options.entry
    {
        return Err(format!("entry `{name}` does not exist"));
    }
    let [(name, kind)] = entries.as_slice() else {
        return Err("select an unambiguous canvas or surface with --entry".into());
    };
    let entry = (*name).to_owned();
    let kind = *kind;
    let variant = options
        .variant
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| format!("variant bindings must be a JSON object of strings: {error}"))?;
    if variant.is_some() && kind != EntryKind::Canvas {
        return Err("explicit variant bindings currently require a canvas entry".into());
    }
    let parameter_json = if let Some(path) = &options.params_file {
        Some(
            std::fs::read_to_string(path)
                .map_err(|error| format!("{}: {error}", path.display()))?,
        )
    } else {
        options.params.clone()
    };
    let updates = parameter_json
        .as_deref()
        .map(serde_json::from_str::<serde_json::Map<String, serde_json::Value>>)
        .transpose()
        .map_err(|error| format!("parameter overrides must be a JSON object: {error}"))?
        .unwrap_or_default();
    let limits = wgpu::Limits::default();
    let (parameters, texture_definitions) = match kind {
        EntryKind::Canvas => {
            let canvas = manifest
                .canvases
                .iter()
                .find(|canvas| canvas.name == entry)
                .expect("selected canvas");
            let pass = canvas
                .engine_pass
                .as_ref()
                .ok_or("canvas has no authored engine pass")?;
            fresco_example_engine::runtime::canvas_variant::select(&entry, pass, variant.as_ref())
                .map_err(|error| error.to_string())?;
            let mut parameters = fresco_example_engine::runtime::parameters::CanvasParameters::new(
                canvas,
                fresco_example_engine::runtime::storage::StorageLimits::from(&limits),
            )
            .map_err(|e| e.to_string())?;
            parameters.update(&updates).map_err(|e| e.to_string())?;
            fresco_example_engine::runtime::paths::PathBuffers::new(
                canvas,
                fresco_example_engine::runtime::storage::StorageLimits::from(&limits),
            )
            .map_err(|e| e.to_string())?;
            (parameters.values(), canvas.textures.as_slice())
        }
        EntryKind::Mesh | EntryKind::Particles => {
            let surface = manifest
                .surfaces
                .iter()
                .find(|surface| surface.name == entry)
                .expect("selected surface");
            let occupied: Vec<_> = if let Some(contract) =
                fresco_example_engine::runtime::particle_contract::ParticleContract::for_surface(
                    &manifest,
                    &surface.name,
                )? {
                fresco_example_engine::runtime::particle_playback::ParticlePlayback::new(
                    &contract,
                    fresco_example_engine::runtime::particle_layout::ParticleLimits::from(&limits),
                )
                .map_err(str::to_owned)?;
                contract
                    .bindings
                    .iter()
                    .map(|b| (b.group, b.binding))
                    .collect()
            } else {
                let pass = surface
                    .mesh_passes
                    .first()
                    .ok_or("surface does not provide an executable mesh pass")?;
                let factory = manifest
                    .vertex_factories
                    .iter()
                    .find(|factory| factory.name == pass.factory)
                    .ok_or("mesh factory is missing")?;
                let mesh = options
                    .mesh
                    .geometry_for_surface(surface)
                    .map_err(|e| e.to_string())?;
                fresco_example_engine::runtime::vertices::pack_vertices(
                    factory,
                    mesh.vertex_count,
                    &mesh.streams(),
                    fresco_example_engine::runtime::vertices::VertexLimits {
                        max_attributes: limits.max_vertex_attributes,
                        max_stride: limits.max_vertex_buffer_array_stride,
                        max_buffer_bytes: limits.max_buffer_size,
                    },
                )
                .map_err(|e| e.to_string())?;
                factory
                    .bindings
                    .iter()
                    .filter_map(|binding| Some((binding.group_index?, binding.binding?)))
                    .collect()
            };
            let mut parameters =
                fresco_example_engine::runtime::surface_parameters::SurfaceParameters::new(
                    &surface.params,
                    fresco_example_engine::runtime::uniforms::UniformLimits {
                        max_bind_groups: limits.max_bind_groups,
                        max_bindings_per_bind_group: limits.max_bindings_per_bind_group,
                        max_uniform_buffer_binding_size: limits.max_uniform_buffer_binding_size,
                    },
                    surface
                        .global_uniforms
                        .iter()
                        .map(|def| (def.group, def.binding))
                        .chain(occupied),
                )
                .map_err(|e| e.to_string())?;
            let mut style_parameters =
                fresco_example_engine::runtime::style_parameters::StyleParameters::new(surface)
                    .map_err(|e| e.to_string())?;
            let style_names = style_parameters.values();
            let (style_updates, material_updates) = updates
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .partition(|(k, _)| style_names.contains_key(k));
            style_parameters
                .update(&style_updates)
                .map_err(|e| e.to_string())?;
            parameters
                .update(&material_updates)
                .map_err(|e| e.to_string())?;
            let mut values = parameters.values();
            values.extend(style_parameters.values());
            (values, surface.textures.as_slice())
        }
    };
    let (textures, asset_paths) = load_textures(options, texture_definitions)?;
    Ok(PreparedArtifact {
        mesh: options.mesh,
        variant,
        kind,
        textures,
        asset_paths,
        parameters,
        artifact: crate::Artifact {
            wgsl: output.wgsl,
            manifest,
        },
        entry,
    })
}

fn load_textures(
    options: &Options,
    definitions: &[fresco_artifact::ManifestTexture],
) -> Result<
    (
        fresco_example_engine::runtime::textures::TextureInputs,
        Vec<PathBuf>,
    ),
    String,
> {
    let mut paths = std::collections::BTreeMap::new();
    for value in &options.texture {
        let (name, path) = value
            .split_once('=')
            .filter(|(name, path)| !name.is_empty() && !path.is_empty())
            .ok_or("--texture requires name=path")?;
        if !definitions.iter().any(|def| def.name == name) {
            return Err(format!("unknown texture `{name}`"));
        }
        if paths
            .insert(name.to_string(), PathBuf::from(path))
            .is_some()
        {
            return Err(format!("duplicate texture override `{name}`"));
        }
    }
    let root = options.asset_root.clone().or_else(|| {
        options
            .source
            .as_ref()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
    });
    for def in definitions {
        if paths.contains_key(&def.name) {
            continue;
        }
        if let Some(asset) = def
            .metadata
            .as_ref()
            .and_then(|m| m.default_asset.as_deref())
        {
            if asset == fresco_example_engine::assets::CHECKER_ID {
                continue;
            }
            let root = root.as_ref().ok_or_else(|| {
                format!(
                    "texture `{}` needs --asset-root or --texture for `{asset}`",
                    def.name
                )
            })?;
            paths.insert(def.name.clone(), root.join(asset));
        }
    }
    let mut encoded = std::collections::BTreeMap::new();
    for (name, path) in &paths {
        encoded.insert(
            name.clone(),
            std::fs::read(path)
                .map_err(|error| format!("texture `{name}` ({}): {error}", path.display()))?,
        );
    }
    let textures = fresco_example_engine::assets::resolve(definitions, &encoded)
        .map_err(|error| error.to_string())?;
    Ok((textures, paths.into_values().collect()))
}

pub fn run(options: Options) -> Result<(), String> {
    if options.watch
        && options.source.is_none()
        && options.engine_dir.is_none()
        && options.params_file.is_none()
        && options.texture.is_empty()
    {
        return Err("--watch requires --source, --engine-dir, --params-file, or --texture".into());
    }
    let initial = options.clone();
    let program = std::thread::Builder::new()
        .name("fresco-compile".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || compile(&initial))
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "compiler worker panicked")??;
    if options.check {
        println!(
            "Compiled {} `{}` successfully.",
            program.kind.label(),
            program.entry
        );
        return Ok(());
    }
    window::run(options, program)
}
