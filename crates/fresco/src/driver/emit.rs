use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;

use fresco_artifact::{
    ManifestArrayDefault as ArrayParamValue, ManifestCanvas, ManifestConfigAxis, ManifestEdge,
    ManifestEnginePass, ManifestEnginePassVariant, ManifestEnginePassVariantBinding,
    ManifestEntryProperty, ManifestGlobalUniform, ManifestGlobalUniformField,
    ManifestIntermediateTarget, ManifestMeshPass, ManifestParam,
    ManifestParamDefault as ParamDefaultValue, ManifestParamTypeInfo, ManifestPass,
    ManifestPassInput, ManifestPassPlan, ManifestPathBuffer, ManifestPathData, ManifestPathSegment,
    ManifestPipeline, ManifestPipelinePassSemantics, ManifestPipelineSemanticSummary, ManifestRoot,
    ManifestSampler, ManifestSharedLayoutSignatureUse, ManifestSourceRange, ManifestStorageParam,
    ManifestSurface, ManifestSurfaceContextField, ManifestSurfaceContractRequirements,
    ManifestSurfaceCustomChannel, ManifestSurfaceEvaluationContract,
    ManifestSurfaceEvaluationVariant, ManifestSurfaceEvaluationVariantBinding,
    ManifestSurfaceParam, ManifestSurfaceRequirements, ManifestSurfaceSettings,
    ManifestSurfaceVertexStage, ManifestTexture, ManifestTextureChannel, ManifestTextureMetadata,
    ManifestVertexAttribute, ManifestVertexFactory, ManifestVertexFactoryBinding,
};
use thiserror::Error;

use crate::ast::MaterialReturnTy;
use crate::ast::{
    PassDecl, PipelineDecl, StructDecl, VertexFactoryDecl, VertexFormatDecl, VertexInterfaceDecl,
    VertexInterfaceMemberDecl,
};
use crate::driver::BuildProfile;
use crate::driver::pass_plan::{self, KernelStrategy};
use crate::hir::{Hir, ParamDefault};
use crate::material_hir::MaterialHir;
use crate::pipeline_layout::canonical_layout_identity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitTarget {
    Wgsl,
    Ir,
    Manifest,
}

impl EmitTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            EmitTarget::Wgsl => "wgsl",
            EmitTarget::Ir => "ir",
            EmitTarget::Manifest => "manifest",
        }
    }
}

impl FromStr for EmitTarget {
    type Err = EmitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "wgsl" => Ok(EmitTarget::Wgsl),
            "ir" => Ok(EmitTarget::Ir),
            "manifest" => Ok(EmitTarget::Manifest),
            other => Err(EmitError::UnknownTarget(other.to_string())),
        }
    }
}

#[derive(Debug, Error)]
pub enum EmitError {
    #[error(
        "error: unknown --emit target `{0}` (v0 supports: wgsl, ir, manifest).\n\
         hlsl/msl/spirv are one naga feature flag + ~20 lines away — see README roadmap."
    )]
    UnknownTarget(String),
    #[error("internal compiler error: WGSL backend failed: {0}")]
    WgslBackend(String),
    #[error("engine-authored pass emission failed: {0}")]
    ExecutablePass(String),
    #[error("internal compiler error: failed to serialize manifest JSON: {0}")]
    ManifestSerialization(#[from] serde_json::Error),
    #[error(
        "runtime profile strips editor-only pass `{stripped_pass}` but shipping pass `{consumer_pass}` still reads it in pipeline `{pipeline}`"
    )]
    RuntimeProfileDanglingRead {
        pipeline: String,
        consumer_pass: String,
        stripped_pass: String,
    },
}

fn resolve_vertex_format_members<'a>(
    format_name: &str,
    formats: &HashMap<&str, &'a VertexFormatDecl>,
    visiting: &mut BTreeSet<String>,
) -> Option<Vec<&'a VertexInterfaceMemberDecl>> {
    if !visiting.insert(format_name.to_string()) {
        return None;
    }
    let format = formats.get(format_name)?;
    let mut members = if let Some(parent) = &format.parent {
        resolve_vertex_format_members(parent, formats, visiting)?
    } else {
        Vec::new()
    };
    for member in &format.members {
        if let Some(index) = members
            .iter()
            .position(|existing| existing.name == member.name)
        {
            members[index] = member;
        } else {
            members.push(member);
        }
    }
    visiting.remove(format_name);
    Some(members)
}

fn vertex_gpu_format(ty_name: &str) -> Option<(&'static str, u32)> {
    let storage_ty = crate::check::strip_spatial_type_suffix(ty_name);
    match canonical_layout_identity(storage_ty).as_str() {
        "f32" => Some(("float32", 4)),
        "vec2" => Some(("float32x2", 8)),
        "vec3" => Some(("float32x3", 12)),
        "vec4" | "color" => Some(("float32x4", 16)),
        "u32" => Some(("uint32", 4)),
        "uvec2" => Some(("uint32x2", 8)),
        "uvec3" => Some(("uint32x3", 12)),
        "uvec4" => Some(("uint32x4", 16)),
        "i32" => Some(("sint32", 4)),
        "ivec2" => Some(("sint32x2", 8)),
        "ivec3" => Some(("sint32x3", 12)),
        "ivec4" => Some(("sint32x4", 16)),
        _ => None,
    }
}

fn manifest_vertex_factories(
    interfaces: &[VertexInterfaceDecl],
    formats: &[VertexFormatDecl],
    factories: &[VertexFactoryDecl],
) -> Vec<ManifestVertexFactory> {
    let formats_by_name = formats
        .iter()
        .map(|format| (format.name.as_str(), format))
        .collect::<HashMap<_, _>>();
    factories
        .iter()
        .map(|factory| {
            let members = resolve_vertex_format_members(
                &factory.target_format,
                &formats_by_name,
                &mut BTreeSet::new(),
            )
            .unwrap_or_default();
            let mut offset = Some(0_u32);
            let attributes = members
                .iter()
                .enumerate()
                .map(|(location, member)| {
                    let format = vertex_gpu_format(&member.ty_name);
                    let attribute_offset = offset;
                    offset = match (offset, format) {
                        (Some(current), Some((_, size))) => current.checked_add(size),
                        _ => None,
                    };
                    ManifestVertexAttribute {
                        name: member.name.clone(),
                        ty: member.ty_name.clone(),
                        shader_location: u32::try_from(location)
                            .expect("vertex attribute location must fit u32"),
                        offset: attribute_offset,
                        gpu_format: format.map(|(name, _)| name.into()),
                        required: !member.optional,
                        defaulted: member.optional && member.default.is_some(),
                    }
                })
                .collect::<Vec<_>>();
            let satisfies_interfaces = interfaces
                .iter()
                .filter(|interface| {
                    interface.members.iter().all(|required| {
                        members.iter().any(|member| {
                            member.name == required.name
                                && canonical_layout_identity(
                                    crate::check::strip_spatial_type_suffix(&member.ty_name),
                                ) == canonical_layout_identity(
                                    crate::check::strip_spatial_type_suffix(&required.ty_name),
                                )
                                && (!member.optional || member.default.is_some())
                        })
                    })
                })
                .map(|interface| interface.name.clone())
                .collect();
            let bindings = factory
                .bindings
                .iter()
                .map(|binding| {
                    let group = binding
                        .attrs
                        .iter()
                        .find(|attribute| attribute.name == "group")
                        .and_then(|attribute| attribute.args.first())
                        .map(String::as_str);
                    let group_index = binding.group_index;
                    let binding_index = binding.binding_index;
                    ManifestVertexFactoryBinding {
                        sampler: binding.attrs.iter().find(|a| a.name == "sampler").map(|a| {
                            fresco_artifact::types::SamplerPreset::parse(&a.args[0])
                                .expect("validated sampler preset")
                        }),
                        draw_data: binding
                            .attrs
                            .iter()
                            .any(|a| a.name == "draw_data")
                            .then_some(fresco_artifact::ManifestDrawData::InstanceId),
                        geometry: None,
                        source: binding
                            .attrs
                            .iter()
                            .find(|a| a.name == "source")
                            .and_then(|a| a.args.first())
                            .cloned(),
                        name: binding.name.clone(),
                        group: group.map(str::to_owned),
                        group_index,
                        binding: binding_index,
                        signature: binding.value_signature.clone(),
                    }
                })
                .collect();
            ManifestVertexFactory {
                name: factory.name.clone(),
                vertex_format: factory.target_format.clone(),
                bindings,
                attributes,
                satisfies_interfaces,
                array_stride: offset,
            }
        })
        .collect()
}

fn has_pass_attr(pass_decl: &PassDecl, attr_name: &str) -> bool {
    pass_decl.attrs.iter().any(|attr| attr.name == attr_name)
}

fn has_permutation_attr(permutation: &crate::ast::PassPermutationDecl, attr_name: &str) -> bool {
    permutation.attrs.iter().any(|attr| attr.name == attr_name)
}

fn has_permutation_editor_config(permutation: &crate::ast::PassPermutationDecl) -> bool {
    permutation.attrs.iter().any(|attr| {
        if attr.name == "config_editor" {
            return true;
        }
        if attr.name != "config" {
            return false;
        }
        attr.args
            .first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case("editor"))
    })
}

fn pass_included_for_profile(pass_decl: &PassDecl, build_profile: BuildProfile) -> bool {
    if !has_pass_attr(pass_decl, "editor_only") {
        return true;
    }
    build_profile.includes_editor_only()
}

fn known_mode_from_permutation_attrs(
    permutation: &crate::ast::PassPermutationDecl,
) -> &'static str {
    if let Some(known_attr) = permutation.attrs.iter().find(|attr| attr.name == "known")
        && let Some(mode) = known_attr.args.first()
    {
        return match mode.trim().to_ascii_lowercase().as_str() {
            "compile" => "compile",
            "pipeline" => "pipeline",
            "draw" => "draw",
            _ => "unknown",
        };
    }
    if has_permutation_attr(permutation, "known_compile") {
        "compile"
    } else if has_permutation_attr(permutation, "known_pipeline") {
        "pipeline"
    } else if has_permutation_attr(permutation, "known_draw") {
        "draw"
    } else {
        "unknown"
    }
}

pub(super) fn manifest_entry_property(
    property: crate::ast::EntryProperty,
) -> ManifestEntryProperty {
    ManifestEntryProperty {
        editable: property.editable,
        block: property.block,
        block_present: property.block_present,
        entry: property.entry,
        name: property.name,
        ty: property.ty,
        value: property.value,
        choices: property.choices,
        permutation: property.permutation,
        value_span: property.value_span.map(|span| ManifestSourceRange {
            start: span.start,
            end: span.end,
        }),
        insert_at: property.insert_at,
    }
}

fn manifest_surface_settings(settings: crate::ast::SurfaceSettings) -> ManifestSurfaceSettings {
    ManifestSurfaceSettings {
        implementations: settings.implementations,
        recipe_conditions: settings.recipe_conditions,
        pass_states: settings.pass_states,
        evaluation_axes: settings.evaluation_axes,
        properties: settings
            .properties
            .into_iter()
            .map(manifest_entry_property)
            .collect(),
        usages: settings.usages,
    }
}

fn manifest_surface_requirements(mat: &MaterialHir) -> ManifestSurfaceRequirements {
    let vertex_stage = mat
        .vertex_program
        .as_ref()
        .map(|vertex| ManifestSurfaceVertexStage {
            entry_point: format!("fresco_surface_vertex_{}", mat.name),
            fields: vertex.fields.iter().map(|(name, _)| name.clone()).collect(),
        });

    ManifestSurfaceRequirements {
        context_type: Some(mat.context.ty.name.clone()),
        context_fields: mat
            .context
            .ty
            .fields
            .iter()
            .map(|field| ManifestSurfaceContextField {
                name: field.name.clone(),
                ty: match &field.ty {
                    crate::context::ContextType::Float => "f32".into(),
                    crate::context::ContextType::Scalar(kind) => kind.name().into(),
                    crate::context::ContextType::Vector(width) => format!("vec{width}"),
                    crate::context::ContextType::Struct(record) => record.name.clone(),
                },
                semantic: field.semantic.clone(),
            })
            .collect(),
        uv_channels: Vec::new(),
        vertex_stage,
    }
}

fn build_param_type_info(ty_name: &str) -> ManifestParamTypeInfo {
    // Array types: array<T> (dynamic) or array<T, N> (fixed)
    if let Some((elem_ty, array_size)) = crate::hir::parse_array_param_type_ex(ty_name) {
        return ManifestParamTypeInfo {
            name: "array".to_string(),
            params: Some(vec![elem_ty.trim().to_string()]),
            size: match array_size {
                crate::hir::ArrayParamSize::Fixed(n) => Some(n),
                crate::hir::ArrayParamSize::Dynamic => None,
            },
        };
    }
    // Generic types like vec4<f32> or mat4x4<f32>
    if let Some(bracket) = ty_name.find('<')
        && ty_name.ends_with('>')
    {
        let name = ty_name[..bracket].to_string();
        let inner = &ty_name[bracket + 1..ty_name.len() - 1];
        let params: Vec<String> = inner.split(',').map(|s| s.trim().to_string()).collect();
        return ManifestParamTypeInfo {
            name,
            params: Some(params),
            size: None,
        };
    }
    // Plain scalar/named type: f32, i32, u32, bool, color, etc.
    ManifestParamTypeInfo {
        name: ty_name.to_string(),
        params: None,
        size: None,
    }
}

fn manifest_global_uniforms(
    defs: &[crate::check::GlobalUniformDef],
    global_uniform_bindings: &HashMap<String, u32>,
    resource_layout: crate::ast::ResourceLayout,
) -> Option<Vec<ManifestGlobalUniform>> {
    if defs.is_empty() {
        return None;
    }
    Some(
        defs.iter()
            .map(|def| ManifestGlobalUniform {
                name: def.name.clone(),
                ty: def.ty_name.clone(),
                group: resource_layout.0[3],
                binding: *global_uniform_bindings
                    .get(&def.name)
                    .expect("lowered global uniform binding"),
                byte_size: def.byte_size(),
                fields: def
                    .fields
                    .iter()
                    .zip(def.field_layouts())
                    .map(
                        |((name, ty), (offset, components))| ManifestGlobalUniformField {
                            name: name.clone(),
                            ty: ty.clone(),
                            offset,
                            components,
                            scalar_type: "f32".into(),
                        },
                    )
                    .collect(),
            })
            .collect(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub fn render(
    target: EmitTarget,
    build_profile: BuildProfile,
    hirs: &[Hir],
    material_hirs: &[MaterialHir],
    compute_library: &crate::check::compute::ComputeLibrary,
    structs: &[StructDecl],
    pass_decls: &[PassDecl],
    pipelines: &[PipelineDecl],
    vertex_interfaces: &[VertexInterfaceDecl],
    vertex_formats: &[VertexFormatDecl],
    vertex_factories: &[VertexFactoryDecl],
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    tex_bindings: &HashMap<String, u32>,
    pass_target_bindings: &HashMap<String, HashMap<usize, u32>>,
    path_bindings: &HashMap<String, u32>,
    param_bindings: &HashMap<String, u32>,
    global_uniform_bindings: &HashMap<String, u32>,
    resource_layout: crate::ast::ResourceLayout,
) -> Result<String, EmitError> {
    let mut shader_prelude =
        naga::back::wgsl::write_string(module, info, naga::back::wgsl::WriterFlags::empty())
            .map_err(|e| EmitError::WgslBackend(e.to_string()))?;
    for material in material_hirs {
        if let Some(source) = &material.evaluation_typed_source {
            shader_prelude.push_str(source);
        }
    }
    match target {
        EmitTarget::Wgsl => {
            let mut wgsl = naga::back::wgsl::write_string(
                module,
                info,
                naga::back::wgsl::WriterFlags::empty(),
            )
            .map_err(|e| EmitError::WgslBackend(e.to_string()))?;

            for hir in hirs {
                for param in &hir.params {
                    if matches!(
                        crate::hir::parse_array_param_type_ex(&param.ty_name),
                        Some((_, crate::hir::ArrayParamSize::Dynamic))
                    ) {
                        let default_decl =
                            format!("var<storage> fresco_param_{}_{}", hir.name, param.name);
                        let readonly_decl = format!(
                            "var<storage, read> fresco_param_{}_{}",
                            hir.name, param.name
                        );
                        wgsl = wgsl.replace(&default_decl, &readonly_decl);
                    }
                }
            }

            let engine_stages = hirs
                .iter()
                .map(|hir| super::engine_pass::for_canvas(hir, pass_decls, pipelines))
                .collect::<Result<Vec<_>, _>>()
                .map_err(EmitError::ExecutablePass)?;
            if engine_stages.iter().any(Option::is_some) {
                wgsl.push_str(
                    &r#"
struct FrescoFullscreenUniforms {
  time: f32,
  _pad0: vec3<f32>,
  res: vec2<f32>,
  _pad1: vec2<f32>,
  params: array<vec4<f32>, 16>,
}

@group(0) @binding(0)
var<uniform> fresco_fullscreen_uniforms: FrescoFullscreenUniforms;
"#
                    .replace("@group(0)", &format!("@group({})", resource_layout.0[0])),
                );
                for stage in engine_stages.iter().flatten() {
                    wgsl.push_str(&stage.wgsl);
                }
            }

            for (code, _) in super::mesh_pass::shader::build(
                pass_decls,
                structs,
                !material_hirs.is_empty() || hirs.is_empty(),
                pipelines,
                compute_library,
                material_hirs,
                &shader_prelude,
            )
            .map_err(EmitError::ExecutablePass)?
            {
                wgsl.push_str(&code);
            }
            for material in material_hirs {
                if let Some(source) = &material.evaluation_typed_source {
                    wgsl.push_str(source);
                }
            }
            let mesh_stages = material_hirs
                .iter()
                .map(|material| {
                    super::mesh_pass::for_surface(
                        material,
                        compute_library,
                        structs,
                        pass_decls,
                        pipelines,
                        vertex_formats,
                        vertex_factories,
                    )
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(EmitError::ExecutablePass)?;
            let mut mesh_ranges = Vec::new();
            for stage in mesh_stages.iter().flatten() {
                let start = wgsl.len();
                wgsl.push_str(&stage.wgsl);
                mesh_ranges.push((stage, start..wgsl.len()));
            }

            match naga::front::wgsl::parse_str(&wgsl) {
                Ok(combined) => {
                    let info = naga::valid::Validator::new(
                        naga::valid::ValidationFlags::all(),
                        naga::valid::Capabilities::default(),
                    )
                    .validate(&combined)
                    .map_err(|error| EmitError::ExecutablePass(error.emit_to_string(&wgsl)))?;
                    for (stage, range) in mesh_ranges {
                        super::mesh_pass::validate_binding_scope(&combined, &info, stage, range)
                            .map_err(EmitError::ExecutablePass)?;
                    }
                }
                Err(error) if error.emit_to_string(&wgsl).contains("recursion limit") => {
                    if !mesh_ranges.is_empty() {
                        return Err(EmitError::ExecutablePass(error.emit_to_string(&wgsl)));
                    }
                    // Some already-valid, expression-heavy Naga modules exceed the
                    // WGSL frontend's recursion limit when serialized and parsed a
                    // second time. Validate each newly appended stage against an
                    // exact helper/resource stub instead of dropping validation.
                    for stage in engine_stages.iter().flatten() {
                        let module = naga::front::wgsl::parse_str(&stage.validation_wgsl).map_err(
                            |error| {
                                EmitError::ExecutablePass(
                                    error.emit_to_string(&stage.validation_wgsl),
                                )
                            },
                        )?;
                        naga::valid::Validator::new(
                            naga::valid::ValidationFlags::all(),
                            naga::valid::Capabilities::default(),
                        )
                        .validate(&module)
                        .map_err(|error| EmitError::ExecutablePass(format!("{error:#?}")))?;
                    }
                }
                Err(error) => {
                    return Err(EmitError::ExecutablePass(error.emit_to_string(&wgsl)));
                }
            }

            // Naga can leave a space after binding attributes before a newline.
            // Keep generated artifacts free of trailing whitespace.
            Ok(wgsl
                .lines()
                .map(|line| format!("{}\n", line.trim_end()))
                .collect())
        }
        EmitTarget::Ir => Ok(format!("{module:#?}\n")),
        EmitTarget::Manifest => render_manifest(ManifestInputs {
            shader_prelude: &shader_prelude,
            resource_layout,
            build_profile,
            hirs,
            material_hirs,
            compute_library,
            structs,
            pass_decls,
            pipelines,
            vertex_interfaces,
            vertex_formats,
            vertex_factories,
            tex_bindings,
            pass_target_bindings,
            path_bindings,
            param_bindings,
            global_uniform_bindings,
        }),
    }
}

struct ManifestInputs<'a> {
    shader_prelude: &'a str,
    resource_layout: crate::ast::ResourceLayout,
    build_profile: BuildProfile,
    hirs: &'a [Hir],
    material_hirs: &'a [MaterialHir],
    compute_library: &'a crate::check::compute::ComputeLibrary,
    structs: &'a [StructDecl],
    pass_decls: &'a [PassDecl],
    pipelines: &'a [PipelineDecl],
    vertex_interfaces: &'a [VertexInterfaceDecl],
    vertex_formats: &'a [VertexFormatDecl],
    vertex_factories: &'a [VertexFactoryDecl],
    tex_bindings: &'a HashMap<String, u32>,
    pass_target_bindings: &'a HashMap<String, HashMap<usize, u32>>,
    path_bindings: &'a HashMap<String, u32>,
    param_bindings: &'a HashMap<String, u32>,
    global_uniform_bindings: &'a HashMap<String, u32>,
}

fn render_manifest(input: ManifestInputs<'_>) -> Result<String, EmitError> {
    let ManifestInputs {
        shader_prelude,
        resource_layout,
        build_profile,
        hirs,
        material_hirs,
        compute_library,
        structs,
        pass_decls,
        pipelines,
        vertex_interfaces,
        vertex_formats,
        vertex_factories,
        tex_bindings,
        pass_target_bindings,
        path_bindings,
        param_bindings,
        global_uniform_bindings,
    } = input;

    let pass_by_name: HashMap<&str, &PassDecl> = pass_decls
        .iter()
        .map(|pass_decl| (pass_decl.name.as_str(), pass_decl))
        .collect();

    let mut manifest_pipelines = Vec::new();
    for pipeline_decl in pipelines {
        let all_passes = pipeline_decl
            .passes
            .iter()
            .filter_map(|pass_ref| pass_by_name.get(pass_ref.node.as_str()).copied())
            .collect::<Vec<_>>();
        let included_passes = all_passes
            .iter()
            .copied()
            .filter(|pass_decl| pass_included_for_profile(pass_decl, build_profile))
            .collect::<Vec<_>>();
        let included_names = included_passes
            .iter()
            .map(|pass| pass.name.as_str())
            .collect::<BTreeSet<_>>();
        let stripped_names = all_passes
            .iter()
            .filter(|pass| !included_names.contains(pass.name.as_str()))
            .map(|pass| pass.name.as_str())
            .collect::<BTreeSet<_>>();

        if matches!(build_profile, BuildProfile::Runtime) {
            for consumer in &included_passes {
                for read in &consumer.reads {
                    if stripped_names.contains(read.node.as_str()) {
                        return Err(EmitError::RuntimeProfileDanglingRead {
                            pipeline: pipeline_decl.name.clone(),
                            consumer_pass: consumer.name.clone(),
                            stripped_pass: read.node.clone(),
                        });
                    }
                }
            }
        }

        let pass_semantics = included_passes
            .iter()
            .map(|pass_decl| ManifestPipelinePassSemantics {
                name: pass_decl.name.clone(),
                stage: pass_decl.stage.as_ref().map(|value| value.node.clone()),
                draw: pass_decl.draw.as_ref().map(|value| value.node.clone()),
                blend: pass_decl.blend.as_ref().map(|value| value.node.clone()),
                reads: pass_decl
                    .reads
                    .iter()
                    .map(|read| read.node.clone())
                    .collect(),
                writes: pass_decl
                    .writes
                    .iter()
                    .map(|write| write.node.clone())
                    .collect(),
            })
            .collect::<Vec<_>>();

        let semantic_summary =
            summarize_pipeline_semantics_for_passes(included_passes.as_slice(), &pass_by_name);

        manifest_pipelines.push(ManifestPipeline {
            name: pipeline_decl.name.clone(),
            material: pipeline_decl.material_name.clone(),
            pipeline_type: pipeline_decl.pipeline_type.clone(),
            passes: included_passes
                .iter()
                .map(|pass_decl| pass_decl.name.clone())
                .collect(),
            semantic_summary,
            pass_semantics,
        });
    }

    let mut root = ManifestRoot {
        techniques: Vec::new(),
        schema_version: fresco_artifact::SCHEMA_VERSION,
        gpu_programs: super::mesh_pass::shader::build(
            pass_decls,
            structs,
            !material_hirs.is_empty() || hirs.is_empty(),
            pipelines,
            compute_library,
            material_hirs,
            shader_prelude,
        )
        .map_err(EmitError::ExecutablePass)?
        .into_iter()
        .map(|(_, program)| program)
        .collect(),
        renderers: super::renderers::catalog(pipelines).map_err(EmitError::ExecutablePass)?,
        tables: super::tables::build(structs, material_hirs).map_err(EmitError::ExecutablePass)?,
        build_profile: Some(build_profile.as_str().into()),
        canvases: hirs
            .iter()
            .map(
                |hir| -> Result<ManifestCanvas<ParamDefaultValue, f32>, EmitError> {
                    let plan = pass_plan::build_for_hir(hir);
                    let textures = (!hir.textures.is_empty()).then(|| {
                        hir.textures
                            .iter()
                            .enumerate()
                            .map(|(tex_i, name)| {
                                let meta = hir.texture_metadata.get(name.as_str());
                                let default_asset = hir.texture_default_asset(name);
                                let texture_type =
                                    meta.and_then(|m| m.texture_type_name.as_deref());
                                let channels: Option<Vec<ManifestTextureChannel>> = texture_type
                                    .and_then(|type_name| {
                                        let def = hir.texture_type_defs.get(type_name)?;
                                        const CHANNEL_NAMES: [&str; 4] = ["r", "g", "b", "a"];
                                        let mut chans: Vec<ManifestTextureChannel> = def
                                            .channels
                                            .iter()
                                            .map(|c| ManifestTextureChannel {
                                                channel: CHANNEL_NAMES
                                                    .get(usize::from(c.channel_idx))
                                                    .copied()
                                                    .unwrap_or("?")
                                                    .to_string(),
                                                name: c.semantic_name.clone(),
                                            })
                                            .collect();
                                        chans.sort_by(|left, right| {
                                            left.channel.cmp(&right.channel)
                                        });
                                        Some(chans)
                                    });
                                let metadata = ManifestTextureMetadata {
                                    default_asset: default_asset.map(str::to_owned),
                                    texture_type: texture_type.map(str::to_owned),
                                    channels,
                                };
                                let has_metadata = metadata.default_asset.is_some()
                                    || metadata.texture_type.is_some()
                                    || metadata.channels.is_some();
                                ManifestTexture {
                                    name: name.clone(),
                                    group: resource_layout.0[1],
                                    binding: tex_bindings
                                        .get(name)
                                        .copied()
                                        .unwrap_or((tex_i + 1) as u32),
                                    metadata: has_metadata.then_some(metadata),
                                }
                            })
                            .collect()
                    });
                    let sampler = textures.as_ref().map(|_| ManifestSampler {
                        group: resource_layout.0[1],
                        binding: 0,
                    });
                    let path_buffers = {
                        let entries = hir
                            .path_profiles
                            .iter()
                            .enumerate()
                            .filter_map(|(path_idx, profile)| {
                                if profile.storage
                                    != crate::hir::PathStorageDecision::BufferExceedsConstThreshold
                                    || !profile.demand.needs_path_geometry()
                                {
                                    return None;
                                }
                                let key = format!("{}:{}", hir.name, path_idx);
                                Some(ManifestPathBuffer {
                                    name: format!("path_{path_idx}"),
                                    group: resource_layout.0[2],
                                    binding: path_bindings.get(&key).copied().expect(
                                        "geometry-backed path has a lowered storage binding",
                                    ),
                                    segments: profile.flattened_segment_count,
                                    data: Some(ManifestPathData {
                                        layout: fresco_artifact::PATH_SEGMENT_LAYOUT.into(),
                                        stride: fresco_artifact::PATH_SEGMENT_STRIDE,
                                        rows: profile
                                            .flattened_rows()
                                            .into_iter()
                                            .map(|row| ManifestPathSegment {
                                                p0: [row.p0.0, row.p0.1],
                                                p1: [row.p1.0, row.p1.1],
                                                p2: [row.p2.0, row.p2.1],
                                                p3: [row.p3.0, row.p3.1],
                                                s0: row.s0,
                                                len: row.len,
                                                kind: row.kind,
                                                mid_u: row.mid_u,
                                            })
                                            .collect(),
                                    }),
                                })
                            })
                            .collect::<Vec<_>>();
                        (!entries.is_empty()).then_some(entries)
                    };
                    let single_pass_fused_entry = plan.passes.len() == 1
                        && matches!(plan.passes[0].kernel_strategy, KernelStrategy::Fused);
                    let passes = plan
                        .passes
                        .iter()
                        .map(|p| {
                            let (label, radius_px, levels) = match p.kernel_strategy {
                                KernelStrategy::Fused => (p.kernel_strategy.label(), None, None),
                                KernelStrategy::InlineTaps { radius_px, .. } => {
                                    (p.kernel_strategy.label(), Some(radius_px), None)
                                }
                                KernelStrategy::SeparableHV { radius_px } => {
                                    (p.kernel_strategy.label(), Some(radius_px), None)
                                }
                                KernelStrategy::DownsampleChain { radius_px, levels } => {
                                    (p.kernel_strategy.label(), Some(radius_px), Some(levels))
                                }
                                KernelStrategy::GlobalReduction => {
                                    (p.kernel_strategy.label(), None, None)
                                }
                            };
                            ManifestPass {
                                id: p.id,
                                stage: p.stage,
                                locality: pass_plan::locality_label(p.locality).into(),
                                start_layer: p.start_layer,
                                end_layer: p.end_layer,
                                count: p.count,
                                kernel_strategy: label.into(),
                                kernel_radius_px: radius_px,
                                kernel_levels: levels,
                                entry_point: if single_pass_fused_entry {
                                    format!("fresco_{}", hir.name)
                                } else {
                                    format!("fresco_{}_pass{}_", hir.name, p.id)
                                },
                                inputs: {
                                    plan.edges
                                        .iter()
                                        .filter_map(|(from, to, _)| {
                                            if *to != p.id {
                                                return None;
                                            }
                                            let source_pass =
                                                plan.passes.iter().find(|pass| pass.id == *from)?;
                                            let target_id = source_pass.output_target?;
                                            let binding = pass_target_bindings
                                                .get(&hir.name)?
                                                .get(&target_id)
                                                .copied()?;
                                            Some(ManifestPassInput {
                                                from_pass: *from,
                                                target_id,
                                                binding,
                                            })
                                        })
                                        .collect::<Vec<_>>()
                                },
                                output_target: p.output_target,
                            }
                        })
                        .collect();
                    let edges = plan
                        .edges
                        .iter()
                        .map(|(from, to, reason)| ManifestEdge {
                            from: *from,
                            to: *to,
                            reason: (*reason).to_string(),
                        })
                        .collect();
                    let targets = plan
                        .targets
                        .iter()
                        .map(|t| ManifestIntermediateTarget {
                            id: t.id,
                            format: t.format.into(),
                            scale: t.scale,
                            lifetime: t.lifetime.label().into(),
                        })
                        .collect();

                    let storage_params = {
                        let entries = hir
                            .params
                            .iter()
                            .filter_map(|p| {
                                // Check if this param is a dynamic array that uses storage buffers
                                if let Some((_, crate::hir::ArrayParamSize::Dynamic)) =
                                    crate::hir::parse_array_param_type_ex(&p.ty_name)
                                {
                                    let key = format!("{}:{}", hir.name, p.name);
                                    Some(ManifestStorageParam {
                                        name: p.name.clone(),
                                        ty: p.ty_name.clone(),
                                        param_type: Some(build_param_type_info(&p.ty_name)),
                                        group: resource_layout.0[2],
                                        binding: param_bindings.get(&key).copied().unwrap_or(0),
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>();
                        (!entries.is_empty()).then_some(entries)
                    };
                    let engine_pass = super::engine_pass::for_canvas(hir, pass_decls, pipelines)
                        .map_err(EmitError::ExecutablePass)?
                        .map(|stage| ManifestEnginePass {
                            pipeline: stage.pipeline,
                            pass: stage.pass,
                            interface: stage.interface,
                            vertex_entry: stage.vertex_entry,
                            fragment_entry: stage.fragment_entry,
                            vertex_count: 3,
                            instance_uniform_group: resource_layout.0[0],
                            instance_uniform_binding: 0,
                            variants: stage
                                .variants
                                .into_iter()
                                .map(|variant| ManifestEnginePassVariant {
                                    key: variant.key,
                                    bindings: variant
                                        .bindings
                                        .into_iter()
                                        .map(|(axis, value)| ManifestEnginePassVariantBinding {
                                            axis,
                                            value,
                                        })
                                        .collect(),
                                    vertex_entry: variant.vertex_entry,
                                    fragment_entry: variant.fragment_entry,
                                })
                                .collect(),
                        });

                    Ok(ManifestCanvas {
                        name: hir.name.clone(),
                        params: hir
                            .params
                            .iter()
                            .filter(|p| {
                                !hir.entry_context
                                    .as_ref()
                                    .is_some_and(|context| context.is_component(&p.name))
                            })
                            .map(|p| ManifestParam {
                                name: p.name.clone(),
                                ty: p.ty_name.clone(),
                                param_type: Some(build_param_type_info(&p.ty_name)),
                                default: match &p.default {
                                    ParamDefault::Scalar(value) => {
                                        ParamDefaultValue::Scalar(*value)
                                    }
                                    ParamDefault::Array(array_param) => {
                                        use crate::hir::ArrayElemValue;
                                        let values: Vec<serde_json::Value> = array_param
                                            .values
                                            .iter()
                                            .map(|v| match v {
                                                ArrayElemValue::F32(f) => serde_json::json!(f),
                                                ArrayElemValue::I32(i) => serde_json::json!(i),
                                                ArrayElemValue::U32(u) => serde_json::json!(u),
                                                ArrayElemValue::Bool(b) => serde_json::json!(b),
                                                ArrayElemValue::Vec2((x, y)) => {
                                                    serde_json::json!([x, y])
                                                }
                                                ArrayElemValue::Vec3((x, y, z)) => {
                                                    serde_json::json!([x, y, z])
                                                }
                                                ArrayElemValue::Vec4((x, y, z, w)) => {
                                                    serde_json::json!([x, y, z, w])
                                                }
                                                ArrayElemValue::Mat2(m) => {
                                                    serde_json::json!(m)
                                                }
                                                ArrayElemValue::Mat3(m) => {
                                                    serde_json::json!(m)
                                                }
                                                ArrayElemValue::Mat4(m) => {
                                                    serde_json::json!(m)
                                                }
                                                ArrayElemValue::Color(c) => serde_json::json!(c),
                                            })
                                            .collect();

                                        ParamDefaultValue::Array(ArrayParamValue {
                                            elem_type: array_param.elem_type.as_str().to_string(),
                                            values,
                                        })
                                    }
                                    ParamDefault::Int(value) => ParamDefaultValue::Int(*value),
                                    ParamDefault::UInt(value) => ParamDefaultValue::UInt(*value),
                                    ParamDefault::Bool(value) => ParamDefaultValue::Bool(*value),
                                    ParamDefault::Color(value) => ParamDefaultValue::Color(*value),
                                },
                                min: p.min,
                                max: p.max,
                            })
                            .collect(),
                        storage_params: storage_params.unwrap_or_default(),
                        global_uniforms: manifest_global_uniforms(
                            &hir.global_uniforms,
                            global_uniform_bindings,
                            resource_layout,
                        )
                        .unwrap_or_default(),
                        textures: textures.unwrap_or_default(),
                        sampler,
                        path_buffers: path_buffers.unwrap_or_default(),
                        pass_plan: ManifestPassPlan {
                            passes,
                            edges,
                            targets,
                        },
                        engine_pass,
                    })
                },
            )
            .collect::<Result<Vec<_>, _>>()?,
        surfaces: material_hirs
            .iter()
            .map(
                |mat| -> Result<ManifestSurface<ParamDefaultValue, f32>, EmitError> {
                    let mesh_passes = super::mesh_pass::for_surface(
                        mat,
                        compute_library,
                        structs,
                        pass_decls,
                        pipelines,
                        vertex_formats,
                        vertex_factories,
                    )
                    .map_err(EmitError::ExecutablePass)?
                    .into_iter()
                    .map(|stage| ManifestMeshPass {
                        procedural: stage.procedural,
                        shading_inputs: stage.shading_inputs,
                        preparation: stage.preparation,
                        prepared_source: pass_decls
                            .iter()
                            .find(|p| p.name == stage.pass)
                            .and_then(|p| p.prepared_draw.as_ref())
                            .map(|p| p.producer_pass.clone()),
                        bindings: stage
                            .bindings
                            .iter()
                            .map(|b| ManifestVertexFactoryBinding {
                                sampler: b.attrs.iter().find(|a| a.name == "sampler").map(|a| {
                                    fresco_artifact::types::SamplerPreset::parse(&a.args[0])
                                        .expect("validated sampler preset")
                                }),
                                draw_data: b
                                    .attrs
                                    .iter()
                                    .any(|a| a.name == "draw_data")
                                    .then_some(fresco_artifact::ManifestDrawData::InstanceId),
                                geometry: b
                                    .attrs
                                    .iter()
                                    .find(|a| a.name == "geometry_resource")
                                    .map(|a| fresco_artifact::ManifestGeometryBinding {
                                        producer: a.args[0].clone(),
                                        role: a.args[1].clone(),
                                    }),
                                name: b.name.clone(),
                                group: b.group_index.map(|g| g.to_string()),
                                group_index: b.group_index,
                                binding: b.binding_index,
                                signature: b.value_signature.clone(),
                                source: b
                                    .attrs
                                    .iter()
                                    .find(|a| a.name == "source")
                                    .and_then(|a| a.args.first())
                                    .cloned(),
                            })
                            .collect(),
                        entries: stage.entries,
                        variants: stage.variants,
                        pass: stage.pass,
                        factory: stage.factory,
                    })
                    .collect();
                    let material_ty = match &mat.material_ty {
                        MaterialReturnTy::Default => {
                            unreachable!("material profile resolved before emission")
                        }
                        MaterialReturnTy::Named(name) => name.clone(),
                    };
                    let render_policy = mat.render_policy_name.clone();
                    let contract_requirements = ManifestSurfaceContractRequirements {
                        required_textures: mat.textures.clone(),
                    };
                    let textures = (!mat.textures.is_empty()).then(|| {
                        mat.textures
                            .iter()
                            .enumerate()
                            .map(|(tex_i, name)| {
                                let meta = mat.texture_metadata.get(name.as_str());
                                let default_asset = mat.texture_default_asset(name);
                                let texture_type =
                                    meta.and_then(|m| m.texture_type_name.as_deref());
                                let channels: Option<Vec<ManifestTextureChannel>> = texture_type
                                    .and_then(|type_name| {
                                        let def = mat.texture_type_defs.get(type_name)?;
                                        const CHANNEL_NAMES: [&str; 4] = ["r", "g", "b", "a"];
                                        let mut chans: Vec<ManifestTextureChannel> = def
                                            .channels
                                            .iter()
                                            .map(|c| ManifestTextureChannel {
                                                channel: CHANNEL_NAMES
                                                    .get(usize::from(c.channel_idx))
                                                    .copied()
                                                    .unwrap_or("?")
                                                    .to_string(),
                                                name: c.semantic_name.clone(),
                                            })
                                            .collect();
                                        chans.sort_by(|left, right| {
                                            left.channel.cmp(&right.channel)
                                        });
                                        Some(chans)
                                    });
                                let metadata = ManifestTextureMetadata {
                                    default_asset: default_asset.map(str::to_owned),
                                    texture_type: texture_type.map(str::to_owned),
                                    channels,
                                };
                                let has_metadata = metadata.default_asset.is_some()
                                    || metadata.texture_type.is_some()
                                    || metadata.channels.is_some();
                                ManifestTexture {
                                    name: name.clone(),
                                    group: resource_layout.0[1],
                                    binding: tex_bindings
                                        .get(name)
                                        .copied()
                                        .unwrap_or((tex_i + 1) as u32),
                                    metadata: has_metadata.then_some(metadata),
                                }
                            })
                            .collect()
                    });
                    let sampler = textures.as_ref().map(|_| ManifestSampler {
                        group: resource_layout.0[1],
                        binding: 0,
                    });
                    let params = mat
                        .params
                        .iter()
                        .map(|p| {
                            let binding_key = format!("{}:{}", mat.name, p.name);
                            let binding = param_bindings.get(&binding_key).copied().unwrap_or(0);
                            ManifestSurfaceParam {
                                group: resource_layout.0[0],
                                name: p.name.clone(),
                                ty: p.ty_name.clone(),
                                binding,
                                default: match &p.default {
                                    ParamDefault::Scalar(v) => ParamDefaultValue::Scalar(*v),
                                    ParamDefault::Int(v) => ParamDefaultValue::Int(*v),
                                    ParamDefault::UInt(v) => ParamDefaultValue::UInt(*v),
                                    ParamDefault::Bool(v) => ParamDefaultValue::Bool(*v),
                                    ParamDefault::Color(v) => ParamDefaultValue::Color(*v),
                                    ParamDefault::Array(array_param) => {
                                        use crate::hir::ArrayElemValue;
                                        let values: Vec<serde_json::Value> = array_param
                                            .values
                                            .iter()
                                            .map(|v| match v {
                                                ArrayElemValue::F32(f) => serde_json::json!(f),
                                                ArrayElemValue::I32(i) => serde_json::json!(i),
                                                ArrayElemValue::U32(u) => serde_json::json!(u),
                                                ArrayElemValue::Bool(b) => serde_json::json!(b),
                                                ArrayElemValue::Vec2((x, y)) => {
                                                    serde_json::json!([x, y])
                                                }
                                                ArrayElemValue::Vec3((x, y, z)) => {
                                                    serde_json::json!([x, y, z])
                                                }
                                                ArrayElemValue::Vec4((x, y, z, w)) => {
                                                    serde_json::json!([x, y, z, w])
                                                }
                                                ArrayElemValue::Mat2(m) => serde_json::json!(m),
                                                ArrayElemValue::Mat3(m) => serde_json::json!(m),
                                                ArrayElemValue::Mat4(m) => serde_json::json!(m),
                                                ArrayElemValue::Color(c) => serde_json::json!(c),
                                            })
                                            .collect();
                                        ParamDefaultValue::Array(ArrayParamValue {
                                            elem_type: array_param.elem_type.as_str().to_string(),
                                            values,
                                        })
                                    }
                                },
                                min: p.min,
                                max: p.max,
                            }
                        })
                        .collect();
                    let custom_channels = mat
                        .material_channels
                        .iter()
                        .enumerate()
                        .map(|(slot, declared_channel)| {
                            let name = &declared_channel.name;
                            let components = match crate::check::strip_spatial_type_suffix(
                                &declared_channel.ty_name,
                            ) {
                                "f32" | "f64" | "half" | "i32" | "u32" | "bool" | "angle"
                                | "length" => Some(1),
                                "vec2" => Some(2),
                                "vec3" => Some(3),
                                "vec4" | "color" => Some(4),
                                "mat2" | "mat3" | "mat4" => None,
                                ty if crate::hir::parse_array_param_type(ty).is_some()
                                    || mat.record_types.iter().any(|record| record.name == ty) =>
                                {
                                    None
                                }
                                _ => unreachable!("unchecked material channel type"),
                            };
                            ManifestSurfaceCustomChannel {
                                name: name.clone(),
                                slot: u32::try_from(slot)
                                    .expect("material channel index exceeds backend address space"),
                                field: name.clone(),
                                components,
                                ty: Some(declared_channel.ty_name.clone()),
                            }
                        })
                        .collect();
                    let evaluation_contract = mat.evaluation_contract.as_ref().map(|contract| {
                        ManifestSurfaceEvaluationContract {
                            inputs: contract
                                .inputs
                                .iter()
                                .map(|entry| entry.name.clone())
                                .collect(),
                            runtime: contract
                                .runtime
                                .iter()
                                .map(|entry| entry.name.clone())
                                .collect(),
                        }
                    });
                    let evaluation_variants = mat
                        .evaluation_variants()
                        .into_iter()
                        .map(|variant| ManifestSurfaceEvaluationVariant {
                            result_type: variant.result_type,
                            entry: variant.entry,
                            bindings: variant
                                .bindings
                                .into_iter()
                                .map(|binding| ManifestSurfaceEvaluationVariantBinding {
                                    axis: binding.axis,
                                    value: binding.value,
                                })
                                .collect(),
                        })
                        .collect::<Vec<_>>();
                    Ok(ManifestSurface {
                        global_uniforms: manifest_global_uniforms(
                            &mat.global_uniforms,
                            global_uniform_bindings,
                            resource_layout,
                        )
                        .unwrap_or_default(),
                        name: mat.name.clone(),
                        settings: mat.settings.clone().map(manifest_surface_settings),
                        material_ty,
                        material_properties: mat.material_properties_name.clone(),
                        surface_shader: Some(mat.surface_shader_name.clone()),
                        surface_shader_entry: Some(format!("fresco_{}", mat.name)),
                        render_policy: Some(render_policy),
                        schema_evaluator: mat.schema_evaluator_name.clone(),
                        evaluation_shader_entry: mat.evaluation_shader_entry.clone(),
                        evaluation_shader_source: mat.evaluation_shader_source.clone(),
                        evaluation_contract,
                        evaluation_variants,
                        contract_requirements: Some(contract_requirements),
                        surface_requirements: manifest_surface_requirements(mat),
                        custom_channels,
                        params,
                        textures: textures.unwrap_or_default(),
                        sampler,
                        mesh_passes,
                    })
                },
            )
            .collect::<Result<Vec<_>, _>>()?,
        pipelines: manifest_pipelines,
        vertex_factories: manifest_vertex_factories(
            vertex_interfaces,
            vertex_formats,
            vertex_factories,
        ),
        config_axes: pipelines
            .iter()
            .flat_map(|pipeline_decl| {
                pipeline_decl
                    .passes
                    .iter()
                    .filter_map(|pass_ref| pass_by_name.get(pass_ref.node.as_str()).copied())
                    .flat_map(move |pass_decl| {
                        pass_decl
                            .permutations
                            .iter()
                            .filter(|perm| has_permutation_editor_config(perm))
                            .map(move |perm| ManifestConfigAxis {
                                pipeline: pipeline_decl.name.clone(),
                                pass: pass_decl.name.clone(),
                                axis: perm.name.clone(),
                                known_mode: known_mode_from_permutation_attrs(perm).into(),
                                axis_class: "editor".into(),
                                inclusion: "editor_only".into(),
                                included_in_build: build_profile.includes_editor_only(),
                            })
                    })
            })
            .collect(),
    };

    root.techniques = super::techniques::reflect(pipelines, &root.gpu_programs)
        .map_err(EmitError::ExecutablePass)?;
    super::recipes::validate(&root).map_err(EmitError::ExecutablePass)?;
    super::resource_ports::resolve(&mut root).map_err(EmitError::ExecutablePass)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&root)?))
}

fn normalize_pipeline_semantic_token(value: &str) -> String {
    canonical_layout_identity(value)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

#[allow(
    dead_code,
    reason = "kept as thin wrapper for readability and test callers"
)]
fn summarize_pipeline_semantics(
    pipeline_decl: &PipelineDecl,
    pass_by_name: &HashMap<&str, &PassDecl>,
) -> Option<ManifestPipelineSemanticSummary> {
    let included_passes = pipeline_decl
        .passes
        .iter()
        .filter_map(|pass_ref| pass_by_name.get(pass_ref.node.as_str()).copied())
        .collect::<Vec<_>>();
    summarize_pipeline_semantics_for_passes(&included_passes, pass_by_name)
}

fn summarize_pipeline_semantics_for_passes(
    included_passes: &[&PassDecl],
    pass_by_name: &HashMap<&str, &PassDecl>,
) -> Option<ManifestPipelineSemanticSummary> {
    let mut raster_passes = 0usize;
    let mut compute_passes = 0usize;
    let mut additive_passes = 0usize;
    let mut resource_flow_edges = 0usize;
    let mut shared_layout_signature_uses: BTreeSet<(String, String, String)> = BTreeSet::new();

    let mut writes_seen: HashMap<String, usize> = HashMap::new();
    let mut resource_reads: HashMap<String, usize> = HashMap::new();

    for pass_decl in included_passes {
        if let Some(stage) = &pass_decl.stage {
            match normalize_pipeline_semantic_token(&stage.node).as_str() {
                "raster" => raster_passes += 1,
                "compute" => compute_passes += 1,
                _ => {}
            }
        }

        if let Some(blend) = &pass_decl.blend
            && normalize_pipeline_semantic_token(&blend.node).contains("add")
        {
            additive_passes += 1;
        }

        for write in &pass_decl.writes {
            *writes_seen
                .entry(normalize_pipeline_semantic_token(&write.node))
                .or_insert(0) += 1;
        }

        for read in &pass_decl.reads {
            *resource_reads
                .entry(normalize_pipeline_semantic_token(&read.node))
                .or_insert(0) += 1;
        }
    }

    let included_names = included_passes
        .iter()
        .map(|pass| pass.name.as_str())
        .collect::<BTreeSet<_>>();

    for consumer_pass in included_passes {
        let consumer_signatures: BTreeSet<String> = consumer_pass
            .permutations
            .iter()
            .filter_map(|permutation| permutation.value_signature.as_ref())
            .map(|signature| canonical_layout_identity(signature))
            .collect();

        if consumer_signatures.is_empty() {
            continue;
        }

        for read_ref in &consumer_pass.reads {
            let Some(producer_pass) = pass_by_name.get(read_ref.node.as_str()).copied() else {
                continue;
            };
            if !included_names.contains(producer_pass.name.as_str()) {
                continue;
            }
            let producer_signatures: BTreeSet<String> = producer_pass
                .permutations
                .iter()
                .filter_map(|permutation| permutation.value_signature.as_ref())
                .map(|signature| canonical_layout_identity(signature))
                .collect();

            for signature_id in consumer_signatures.intersection(&producer_signatures) {
                shared_layout_signature_uses.insert((
                    signature_id.clone(),
                    producer_pass.name.clone(),
                    consumer_pass.name.clone(),
                ));
            }
        }
    }

    for (resource, read_count) in resource_reads {
        if writes_seen.contains_key(&resource) {
            resource_flow_edges += read_count;
        }
    }

    if raster_passes == 0 && compute_passes == 0 {
        None
    } else {
        Some(ManifestPipelineSemanticSummary {
            raster_passes,
            compute_passes,
            additive_passes,
            resource_flow_edges,
            shared_layout_signature_uses: shared_layout_signature_uses
                .into_iter()
                .map(|(signature_id, producer_pass, consumer_pass)| {
                    ManifestSharedLayoutSignatureUse {
                        signature_id,
                        producer_pass,
                        consumer_pass,
                    }
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shared_surface_requirements_preserve_context_and_custom_channel_contracts() {
        let source = r#"
material_properties finish {
    channel albedo: color = #fff
    channel coating: f32 = 0.0
}
surface probe(sp: surf) -> material(finish) {
    compose { base(albedo: #fff, coating: 0.25) }
}
"#;
        let output = crate::test_support::compile_source(source, "main.fr", "manifest", false)
            .expect("crate-owned surface fixture");
        let root: serde_json::Value = serde_json::from_str(&output.emitted).unwrap();
        let surface = &root["surfaces"][0];
        let requirements: fresco_artifact::ManifestSurfaceRequirements =
            serde_json::from_value(surface["surface_requirements"].clone()).unwrap();
        assert_eq!(requirements.context_type.as_deref(), Some("surf"));
        assert!(
            requirements
                .context_fields
                .iter()
                .any(|field| field.name == "uv" && field.semantic.as_deref() == Some("coord"))
        );
        assert!(requirements.vertex_stage.is_none());
        assert!(
            surface["surface_requirements"]
                .get("vertex_stage")
                .is_none()
        );
        assert_eq!(
            serde_json::to_value(requirements).unwrap(),
            surface["surface_requirements"]
        );
        let contract: fresco_artifact::ManifestSurfaceContractRequirements =
            serde_json::from_value(surface["contract_requirements"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(contract).unwrap(),
            surface["contract_requirements"]
        );
        let channels: Vec<fresco_artifact::ManifestSurfaceCustomChannel> =
            serde_json::from_value(surface["custom_channels"].clone()).unwrap();
        let coating = channels
            .iter()
            .find(|channel| channel.name == "coating")
            .unwrap();
        assert_eq!(coating.field, "coating");
        assert_eq!(coating.components, Some(1));
        assert_eq!(coating.ty.as_deref(), Some("f32"));
        assert_eq!(
            serde_json::to_value(channels).unwrap(),
            surface["custom_channels"]
        );
    }

    #[test]
    fn shared_texture_records_preserve_optional_metadata_and_named_channels() {
        for (declaration, expression, asset, typed) in [
            ("uniform image: texture", "image.at(uv)", None, false),
            (
                "param image: texture = \"paint.png\"",
                "image.at(uv)",
                Some("paint.png"),
                false,
            ),
            (
                "param image: texture<Packed> = \"packed.png\"",
                "grey(image.at(uv).power)",
                Some("packed.png"),
                true,
            ),
        ] {
            let source = format!(
                "texture_type Packed {{ r: coverage\n g: power }}\ncanvas probe(uv: coord) -> color {{ {declaration}\n {expression} }}"
            );
            let output = crate::test_support::compile_source(&source, "main.fr", "manifest", false)
                .expect("texture fixture compiles without the example engine");
            let root: serde_json::Value = serde_json::from_str(&output.emitted).unwrap();
            let texture_json = &root["canvases"][0]["textures"][0];
            let texture: fresco_artifact::ManifestTexture =
                serde_json::from_value(texture_json.clone()).unwrap();
            assert_eq!(texture.name, "image");
            assert_eq!(texture.group, 1);
            if let Some(asset) = asset {
                let metadata = texture.metadata.as_ref().expect("authored metadata");
                assert_eq!(metadata.default_asset.as_deref(), Some(asset));
                assert_eq!(metadata.texture_type.as_deref(), typed.then_some("Packed"));
                if typed {
                    let channels = metadata.channels.as_ref().expect("named channels");
                    assert_eq!(
                        channels
                            .iter()
                            .map(|channel| (channel.channel.as_str(), channel.name.as_str()))
                            .collect::<Vec<_>>(),
                        [("g", "power"), ("r", "coverage")]
                    );
                } else {
                    assert!(metadata.channels.is_none());
                    assert!(texture_json["metadata"].get("channels").is_none());
                }
            } else {
                assert!(texture.metadata.is_none());
                assert!(texture_json.get("metadata").is_none());
            }
            assert_eq!(serde_json::to_value(texture).unwrap(), *texture_json);
        }
    }

    #[test]
    fn shared_buffer_records_preserve_compiler_layout_and_type_metadata() {
        let source = r#"
struct Inputs { gain: f32; direction: vec2 }
param input: Inputs
canvas probe(uv: coord) -> color {
    param samples: array<f32> = []
    rgba(input.gain, input.direction.x, uv.y, 1.0)
}
"#;
        let output = crate::test_support::compile_source(source, "main.fr", "manifest", false)
            .expect("buffer fixture compiles without the example engine");
        let root: serde_json::Value = serde_json::from_str(&output.emitted).unwrap();
        let canvas = &root["canvases"][0];
        let uniform_json = &canvas["global_uniforms"][0];
        let uniform: fresco_artifact::ManifestGlobalUniform =
            serde_json::from_value(uniform_json.clone()).unwrap();
        assert_eq!(uniform.name, "input");
        assert_eq!(uniform.ty, "Inputs");
        assert_eq!(uniform.byte_size, 16);
        assert_eq!(uniform.fields[0].offset, 0);
        assert_eq!(uniform.fields[1].offset, 8);
        assert_eq!(uniform.fields[1].components, 2);
        assert_eq!(serde_json::to_value(&uniform).unwrap(), *uniform_json);

        let storage_json = &canvas["storage_params"][0];
        let storage: fresco_artifact::ManifestStorageParam =
            serde_json::from_value(storage_json.clone()).unwrap();
        assert_eq!(storage.name, "samples");
        let ty = storage
            .param_type
            .as_ref()
            .expect("compiler supplies type metadata");
        assert_eq!(ty.name, "array");
        assert_eq!(ty.params.as_ref().unwrap(), &["f32"]);
        assert_eq!(ty.size, None);
        assert_eq!(serde_json::to_value(storage).unwrap(), *storage_json);
    }

    use super::ManifestInputs;
    use super::render_manifest;
    use super::summarize_pipeline_semantics;
    use crate::ast::{PassBindingDecl, PassDecl, PipelineDecl, PipelinePassRef, Spanned};
    use crate::driver::BuildProfile;
    use std::collections::HashMap;

    fn pass_binding(name: &str, signature: &str) -> PassBindingDecl {
        PassBindingDecl {
            operation_alias: None,
            group_index: None,
            binding_index: None,
            name: name.to_string(),
            name_span: 0..0,
            attrs: Vec::new(),
            value_signature: Some(signature.to_string()),
            span: 0..0,
        }
    }

    fn pass_decl(
        name: &str,
        reads: &[&str],
        bindings: Vec<PassBindingDecl>,
        stage: Option<&str>,
        permutations: Vec<crate::ast::PassPermutationDecl>,
        attrs: Vec<crate::ast::PipelineAttribute>,
    ) -> PassDecl {
        PassDecl {
            service_captures: Vec::new(),
            compute_invocation: None,
            prepared_draw: None,
            preparation: None,
            operation: None,
            state: Vec::new(),
            entry_bindings: Vec::new(),
            entry_properties: Vec::new(),
            name: name.to_string(),
            name_span: 0..0,
            source_file: "main.fr".to_string(),
            material_name: None,
            material_span: None,
            attrs,
            stage: stage.map(|value| Spanned {
                node: value.to_string(),
                span: 0..0,
            }),
            draw: None,
            blend: None,
            reads: reads
                .iter()
                .map(|read| Spanned {
                    node: (*read).to_string(),
                    span: 0..0,
                })
                .collect(),
            writes: Vec::new(),
            permutations,
            requirements: Vec::new(),
            bindings,
            hooks: Vec::new(),
            vertex_interface: None,
            span: 0..0,
        }
    }

    fn pipeline_decl(name: &str, pass_names: &[&str]) -> PipelineDecl {
        PipelineDecl {
            resource_ports: Vec::new(),
            name: name.to_string(),
            name_span: 0..0,
            material_name: None,
            material_span: None,
            pipeline_type: "lighting".to_string(),
            pipeline_type_span: 0..0,
            passes: pass_names
                .iter()
                .map(|pass_name| Spanned {
                    node: (*pass_name).to_string(),
                    span: 0..0,
                })
                .collect(),
            attrs: Vec::new(),
            pass_refs: pass_names
                .iter()
                .map(|pass_name| PipelinePassRef {
                    invocation: None,
                    name: (*pass_name).to_string(),
                    name_span: 0..0,
                    attrs: Vec::new(),
                    span: 0..0,
                })
                .collect(),
            span: 0..0,
        }
    }

    #[test]
    fn summarize_pipeline_semantics_exports_shared_layout_signature_ids() {
        let producer = pass_decl(
            "cluster_cull",
            &[],
            vec![
                pass_binding("tile_size", "buffer < U32 > cluster_lights"),
                pass_binding("max_per_cluster", "buffer < U32 > cluster_lights"),
            ],
            Some("compute"),
            vec![crate::ast::PassPermutationDecl {
                name: "layout".to_string(),
                name_span: 0..0,
                attrs: Vec::new(),
                values: vec![Spanned {
                    node: "buffer<u32>cluster_lights".to_string(),
                    span: 0..0,
                }],
                when_guard: None,
                else_value: None,
                value_signature: Some("buffer<u32>cluster_lights".to_string()),
                span: 0..0,
            }],
            Vec::new(),
        );
        let consumer = pass_decl(
            "fp_shade",
            &["cluster_cull"],
            vec![pass_binding("tile_size", "buffer<u32>cluster_lights")],
            Some("raster"),
            vec![crate::ast::PassPermutationDecl {
                name: "layout".to_string(),
                name_span: 0..0,
                attrs: Vec::new(),
                values: vec![Spanned {
                    node: "buffer<u32>cluster_lights".to_string(),
                    span: 0..0,
                }],
                when_guard: None,
                else_value: None,
                value_signature: Some("buffer<u32>cluster_lights".to_string()),
                span: 0..0,
            }],
            Vec::new(),
        );
        let pipeline = pipeline_decl("forward_plus", &["cluster_cull", "fp_shade"]);
        let pass_by_name: HashMap<&str, &PassDecl> = HashMap::from([
            (producer.name.as_str(), &producer),
            (consumer.name.as_str(), &consumer),
        ]);

        let summary = summarize_pipeline_semantics(&pipeline, &pass_by_name)
            .expect("expected semantic summary for mixed-stage pipeline");

        let summary: fresco_artifact::ManifestPipelineSemanticSummary =
            serde_json::from_value(serde_json::to_value(&summary).unwrap())
                .expect("runtime contract retains the emitted shared layout metadata");

        assert_eq!(summary.shared_layout_signature_uses.len(), 1);
        let shared = &summary.shared_layout_signature_uses[0];
        assert_eq!(shared.signature_id, "buffer<u32>cluster_lights");
        assert_eq!(shared.producer_pass, "cluster_cull");
        assert_eq!(shared.consumer_pass, "fp_shade");
    }

    #[test]
    fn runtime_manifest_strips_editor_only_passes_and_marks_config_axes_excluded() {
        let known_attr = crate::ast::PipelineAttribute {
            expressions: Vec::new(),
            name: "known".to_string(),
            name_span: 0..0,
            args: vec!["draw".to_string()],
            args_span: Some(0..0),
            span: 0..0,
        };
        let config_editor_attr = crate::ast::PipelineAttribute {
            expressions: Vec::new(),
            name: "config".to_string(),
            name_span: 0..0,
            args: vec!["editor".to_string()],
            args_span: Some(0..0),
            span: 0..0,
        };
        let editor_only_attr = crate::ast::PipelineAttribute {
            expressions: Vec::new(),
            name: "editor_only".to_string(),
            name_span: 0..0,
            args: Vec::new(),
            args_span: None,
            span: 0..0,
        };

        let preview_pass = pass_decl(
            "preview_overlay",
            &[],
            Vec::new(),
            Some("raster"),
            vec![crate::ast::PassPermutationDecl {
                name: "zoom_mode".to_string(),
                name_span: 0..0,
                attrs: vec![known_attr, config_editor_attr],
                values: vec![Spanned {
                    node: "identity".to_string(),
                    span: 0..0,
                }],
                when_guard: None,
                else_value: None,
                value_signature: Some("identity".to_string()),
                span: 0..0,
            }],
            vec![editor_only_attr],
        );
        let pipeline = pipeline_decl("forward", &["preview_overlay"]);

        let manifest = render_manifest(ManifestInputs {
            shader_prelude: "",
            resource_layout: Default::default(),
            build_profile: BuildProfile::Runtime,
            hirs: &[],
            material_hirs: &[],
            compute_library: &crate::check::compute::ComputeLibrary::new(
                &crate::ast::Program::default(),
                crate::check::CheckOptions::default(),
            ),
            structs: &[],
            pass_decls: &[preview_pass],
            pipelines: &[pipeline],
            vertex_interfaces: &[],
            vertex_formats: &[],
            vertex_factories: &[],
            tex_bindings: &HashMap::new(),
            pass_target_bindings: &HashMap::new(),
            path_bindings: &HashMap::new(),
            param_bindings: &HashMap::new(),
            global_uniform_bindings: &HashMap::new(),
        })
        .expect("manifest should render");

        let root: serde_json::Value =
            serde_json::from_str(&manifest).expect("manifest JSON should parse");
        assert_eq!(
            root.get("build_profile").and_then(|v| v.as_str()),
            Some("runtime")
        );

        let first_pipeline_passes = root
            .get("pipelines")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|pipeline| pipeline.get("passes"))
            .and_then(|passes| passes.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(first_pipeline_passes.is_empty());

        let included = root
            .get("config_axes")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|axis| axis.get("included_in_build"))
            .and_then(serde_json::Value::as_bool);
        assert_eq!(included, Some(false));
    }

    #[test]
    fn profile_matrix_toggles_editor_pass_inclusion() {
        let known_attr = crate::ast::PipelineAttribute {
            expressions: Vec::new(),
            name: "known".to_string(),
            name_span: 0..0,
            args: vec!["draw".to_string()],
            args_span: Some(0..0),
            span: 0..0,
        };
        let config_editor_attr = crate::ast::PipelineAttribute {
            expressions: Vec::new(),
            name: "config".to_string(),
            name_span: 0..0,
            args: vec!["editor".to_string()],
            args_span: Some(0..0),
            span: 0..0,
        };
        let editor_only_attr = crate::ast::PipelineAttribute {
            expressions: Vec::new(),
            name: "editor_only".to_string(),
            name_span: 0..0,
            args: Vec::new(),
            args_span: None,
            span: 0..0,
        };

        let preview_pass = pass_decl(
            "preview_overlay",
            &[],
            Vec::new(),
            Some("raster"),
            vec![crate::ast::PassPermutationDecl {
                name: "zoom_mode".to_string(),
                name_span: 0..0,
                attrs: vec![known_attr, config_editor_attr],
                values: vec![Spanned {
                    node: "identity".to_string(),
                    span: 0..0,
                }],
                when_guard: None,
                else_value: None,
                value_signature: Some("identity".to_string()),
                span: 0..0,
            }],
            vec![editor_only_attr],
        );
        let pipeline = pipeline_decl("forward", &["preview_overlay"]);

        let manifest_all = render_manifest(ManifestInputs {
            shader_prelude: "",
            resource_layout: Default::default(),
            build_profile: BuildProfile::All,
            hirs: &[],
            material_hirs: &[],
            compute_library: &crate::check::compute::ComputeLibrary::new(
                &crate::ast::Program::default(),
                crate::check::CheckOptions::default(),
            ),
            structs: &[],
            pass_decls: std::slice::from_ref(&preview_pass),
            pipelines: std::slice::from_ref(&pipeline),
            vertex_interfaces: &[],
            vertex_formats: &[],
            vertex_factories: &[],
            tex_bindings: &HashMap::new(),
            pass_target_bindings: &HashMap::new(),
            path_bindings: &HashMap::new(),
            param_bindings: &HashMap::new(),
            global_uniform_bindings: &HashMap::new(),
        })
        .expect("all profile manifest should render");
        let root_all: serde_json::Value =
            serde_json::from_str(&manifest_all).expect("all manifest parse");
        let pass_count_all = root_all
            .get("pipelines")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|pipeline| pipeline.get("passes"))
            .and_then(|passes| passes.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        assert_eq!(pass_count_all, 1);

        let manifest_editor = render_manifest(ManifestInputs {
            shader_prelude: "",
            resource_layout: Default::default(),
            build_profile: BuildProfile::Editor,
            hirs: &[],
            material_hirs: &[],
            compute_library: &crate::check::compute::ComputeLibrary::new(
                &crate::ast::Program::default(),
                crate::check::CheckOptions::default(),
            ),
            structs: &[],
            pass_decls: std::slice::from_ref(&preview_pass),
            pipelines: std::slice::from_ref(&pipeline),
            vertex_interfaces: &[],
            vertex_formats: &[],
            vertex_factories: &[],
            tex_bindings: &HashMap::new(),
            pass_target_bindings: &HashMap::new(),
            path_bindings: &HashMap::new(),
            param_bindings: &HashMap::new(),
            global_uniform_bindings: &HashMap::new(),
        })
        .expect("editor profile manifest should render");
        let root_editor: serde_json::Value =
            serde_json::from_str(&manifest_editor).expect("editor manifest parse");
        let pass_count_editor = root_editor
            .get("pipelines")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|pipeline| pipeline.get("passes"))
            .and_then(|passes| passes.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        assert_eq!(pass_count_editor, 1);

        let manifest_runtime = render_manifest(ManifestInputs {
            shader_prelude: "",
            resource_layout: Default::default(),
            build_profile: BuildProfile::Runtime,
            hirs: &[],
            material_hirs: &[],
            compute_library: &crate::check::compute::ComputeLibrary::new(
                &crate::ast::Program::default(),
                crate::check::CheckOptions::default(),
            ),
            structs: &[],
            pass_decls: std::slice::from_ref(&preview_pass),
            pipelines: std::slice::from_ref(&pipeline),
            vertex_interfaces: &[],
            vertex_formats: &[],
            vertex_factories: &[],
            tex_bindings: &HashMap::new(),
            pass_target_bindings: &HashMap::new(),
            path_bindings: &HashMap::new(),
            param_bindings: &HashMap::new(),
            global_uniform_bindings: &HashMap::new(),
        })
        .expect("runtime profile manifest should render");
        let root_runtime: serde_json::Value =
            serde_json::from_str(&manifest_runtime).expect("runtime manifest parse");
        let pass_count_runtime = root_runtime
            .get("pipelines")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|pipeline| pipeline.get("passes"))
            .and_then(|passes| passes.as_array())
            .map(Vec::len)
            .unwrap_or(0);
        assert_eq!(pass_count_runtime, 0);
        for root in [&root_all, &root_editor, &root_runtime] {
            for pipeline in root["pipelines"].as_array().unwrap() {
                let shared: fresco_artifact::ManifestPipeline =
                    serde_json::from_value(pipeline.clone()).unwrap();
                assert_eq!(serde_json::to_value(shared).unwrap(), *pipeline);
            }
            for axis in root["config_axes"].as_array().unwrap() {
                let shared: fresco_artifact::ManifestConfigAxis =
                    serde_json::from_value(axis.clone()).unwrap();
                assert_eq!(serde_json::to_value(shared).unwrap(), *axis);
            }
        }
        assert!(root_runtime["pipelines"][0].get("passes").is_none());
        assert!(root_runtime["pipelines"][0].get("pass_semantics").is_none());
    }

    #[test]
    fn runtime_profile_errors_on_dangling_read_after_editor_strip() {
        let editor_only_attr = crate::ast::PipelineAttribute {
            expressions: Vec::new(),
            name: "editor_only".to_string(),
            name_span: 0..0,
            args: Vec::new(),
            args_span: None,
            span: 0..0,
        };

        let producer = pass_decl(
            "preview_producer",
            &[],
            Vec::new(),
            Some("raster"),
            Vec::new(),
            vec![editor_only_attr],
        );
        let consumer = pass_decl(
            "shipping_consumer",
            &["preview_producer"],
            Vec::new(),
            Some("raster"),
            Vec::new(),
            Vec::new(),
        );
        let pipeline = pipeline_decl("forward", &["preview_producer", "shipping_consumer"]);

        let err = render_manifest(ManifestInputs {
            shader_prelude: "",
            resource_layout: Default::default(),
            build_profile: BuildProfile::Runtime,
            hirs: &[],
            material_hirs: &[],
            compute_library: &crate::check::compute::ComputeLibrary::new(
                &crate::ast::Program::default(),
                crate::check::CheckOptions::default(),
            ),
            structs: &[],
            pass_decls: &[producer, consumer],
            pipelines: &[pipeline],
            vertex_interfaces: &[],
            vertex_formats: &[],
            vertex_factories: &[],
            tex_bindings: &HashMap::new(),
            pass_target_bindings: &HashMap::new(),
            path_bindings: &HashMap::new(),
            param_bindings: &HashMap::new(),
            global_uniform_bindings: &HashMap::new(),
        })
        .expect_err("runtime manifest should fail on dangling read");

        let msg = err.to_string();
        assert!(msg.contains("runtime profile strips editor-only pass"));
        assert!(msg.contains("shipping_consumer"));
        assert!(msg.contains("preview_producer"));
    }
}
