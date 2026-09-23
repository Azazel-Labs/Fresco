//! Canonical language documentation model generated from registry + parser metadata.

use crate::builtin_catalog::BuiltinCaps;
use crate::registry::{
    NumericRuntimeBehavior, NumericScalarKind, NumericSurfaceScope, NumericTypeFamily,
    PrimitiveType, TypeRef,
};
use chumsky::Parser as _;
use logos::Logos as _;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const LANGUAGE_DOCS_SCHEMA_VERSION: &str = "fresco.language-docs.v1";

#[derive(Debug, Clone, Serialize)]
pub struct LanguageDocsModel {
    pub schema_version: &'static str,
    pub keywords: Vec<String>,
    pub type_keywords: Vec<String>,
    pub units: Vec<String>,
    pub builtins: Vec<String>,
    pub callables: Vec<String>,
    pub space_transforms: Vec<String>,
    pub blend_modes: Vec<String>,
    pub enum_members: Vec<String>,
    pub enum_variants_by_type: BTreeMap<String, Vec<String>>,
    pub parser_notes: Vec<String>,
    pub call_binding_rules: Vec<String>,
    pub types: Vec<DocType>,
    pub numeric_primitive_catalog: Vec<DocNumericPrimitive>,
    pub enums: Vec<DocEnum>,
    pub space_transform_reference: Vec<DocSpaceTransform>,
    pub callable_reference: Vec<DocCallable>,
    pub builtin_reference: Vec<DocBuiltin>,
    pub stdlib_exports: Vec<DocStdlibExport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocType {
    pub name: String,
    pub kind: String,
    pub docs: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocNumericPrimitive {
    pub name: String,
    pub family: String,
    pub scalar_kind: String,
    pub columns: u8,
    pub rows: u8,
    pub implemented: bool,
    pub allow_as_param: bool,
    pub surface_scope: String,
    pub runtime_behavior: String,
    pub wgsl_supported: bool,
    pub wgsl_feature: Option<String>,
    pub docs: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocEnum {
    pub name: String,
    pub docs: String,
    pub variants: Vec<DocEnumVariant>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocEnumVariant {
    pub name: String,
    pub docs: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocBuiltin {
    pub name: String,
    pub category: String,
    pub summary: String,
    pub discriminator: Option<DocBuiltinDiscriminator>,
    pub signatures: Vec<DocBuiltinSignature>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocBuiltinDiscriminator {
    pub name: String,
    pub default: Option<String>,
    pub variants: Vec<String>,
    pub enum_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocBuiltinSignature {
    pub receiver: Option<String>,
    pub args: Vec<DocBuiltinArg>,
    pub returns: Vec<String>,
    pub pipeable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocBuiltinArg {
    pub name: String,
    pub ty: String,
    pub value_kind: String,
    pub viz_role: Option<String>,
    pub enum_type: Option<String>,
    pub required: bool,
    pub docs: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocSpaceTransform {
    pub name: String,
    pub summary: String,
    pub args: Vec<DocSpaceTransformArg>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocSpaceTransformArg {
    pub name: String,
    pub ty: String,
    pub value_kind: String,
    pub enum_type: Option<String>,
    pub required: bool,
    pub docs: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocCallable {
    pub name: String,
    pub summary: String,
    pub context: String,
    pub args: Vec<DocCallableArg>,
    pub returns: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocCallableArg {
    pub name: String,
    pub ty: String,
    pub value_kind: String,
    pub required: bool,
    pub docs: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocStdlibExport {
    pub name: String,
    pub params: Vec<DocStdlibExportParam>,
    pub ret_ty: String,
    pub docs: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocStdlibExportParam {
    pub name: String,
    pub ty_name: String,
    pub docs: Option<String>,
}

pub fn build_language_docs_model() -> LanguageDocsModel {
    build_language_docs_model_for_prelude(None)
}

/// Describe an explicitly supplied authored library alongside intrinsic language metadata.
/// Invalid authored source is reported rather than silently omitted.
pub fn build_language_docs_model_with_prelude(
    source: &str,
) -> Result<LanguageDocsModel, Vec<String>> {
    let prelude = prelude_program(source).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
    })?;
    Ok(build_language_docs_model_for_prelude(Some(prelude)))
}

fn build_language_docs_model_for_prelude(
    prelude: Option<crate::ast::Program>,
) -> LanguageDocsModel {
    // Include parser contextual keywords that are intentionally tokenized as
    // identifiers (for grammar flexibility) so editor tooling still highlights
    // them as keywords via the wasm language profile.
    const CONTEXTUAL_KEYWORDS: &[&str] = &["material", "unlit", "var", "const"];

    let mut keywords: BTreeSet<String> = crate::lexer::KEYWORD_LEXEMES
        .iter()
        .map(|v| (*v).to_string())
        .collect();
    keywords.extend(CONTEXTUAL_KEYWORDS.iter().map(|v| (*v).to_string()));
    let keywords: Vec<String> = keywords.into_iter().collect();

    let mut units = crate::lexer::UNIT_SUFFIXES
        .iter()
        .map(|v| (*v).to_string())
        .collect::<Vec<_>>();
    units.sort();

    let type_keywords: BTreeSet<String> = crate::registry::type_decls()
        .iter()
        .map(|decl| decl.id.0.to_string())
        .collect();

    let builtins: Vec<String> = {
        let names: BTreeSet<String> = crate::registry::builtin_decls()
            .iter()
            .map(|decl| decl.name.to_string())
            .collect();
        names.into_iter().collect()
    };

    let mut callables: Vec<String> = crate::registry::callable_decls()
        .iter()
        .map(|decl| decl.name.to_string())
        .collect();
    callables.sort();

    let mut space_transforms: Vec<String> = crate::registry::space_transform_decls()
        .iter()
        .map(|decl| decl.name.to_string())
        .collect();
    space_transforms.sort();

    let parser_space_transforms: BTreeSet<String> = crate::check::space_transform_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let registry_space_transforms: BTreeSet<String> =
        space_transforms.iter().cloned().collect::<BTreeSet<_>>();
    debug_assert_eq!(
        parser_space_transforms, registry_space_transforms,
        "space transform names drifted between checker and registry declarations"
    );

    let mut blend_modes: Vec<String> = crate::registry::enum_decls()
        .iter()
        .find(|decl| decl.id.0 == "BlendMode")
        .map(|decl| decl.variants.iter().map(|v| v.name.to_string()).collect())
        .unwrap_or_default();
    blend_modes.sort();

    let enum_members: Vec<String> = {
        let names: BTreeSet<String> = crate::registry::enum_decls()
            .iter()
            .flat_map(|decl| decl.variants.iter().map(|variant| variant.name.to_string()))
            .collect();
        names.into_iter().collect()
    };

    let enum_variants_by_type: BTreeMap<String, Vec<String>> = crate::registry::enum_decls()
        .iter()
        .map(|decl| {
            let mut variants = decl
                .variants
                .iter()
                .map(|variant| variant.name.to_string())
                .collect::<Vec<_>>();
            variants.sort();
            (decl.id.0.to_string(), variants)
        })
        .collect();

    let types = {
        let mut out: Vec<DocType> = crate::registry::type_decls()
            .iter()
            .map(|decl| DocType {
                name: decl.id.0.to_string(),
                kind: format!("{:?}", decl.kind).to_lowercase(),
                docs: decl.docs.to_string(),
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    };

    let numeric_primitive_catalog = {
        let mut out: Vec<DocNumericPrimitive> = crate::registry::numeric_primitive_docs()
            .iter()
            .map(|entry| DocNumericPrimitive {
                name: entry.name.to_string(),
                family: match entry.family {
                    NumericTypeFamily::Scalar => "scalar",
                    NumericTypeFamily::Vector => "vector",
                    NumericTypeFamily::Matrix => "matrix",
                    NumericTypeFamily::Atomic => "atomic",
                }
                .to_string(),
                scalar_kind: match entry.scalar {
                    NumericScalarKind::Bool => "bool",
                    NumericScalarKind::F16 => "f16",
                    NumericScalarKind::F32 => "f32",
                    NumericScalarKind::F64 => "f64",
                    NumericScalarKind::I8 => "i8",
                    NumericScalarKind::I32 => "i32",
                    NumericScalarKind::U8 => "u8",
                    NumericScalarKind::U32 => "u32",
                }
                .to_string(),
                columns: entry.columns,
                rows: entry.rows,
                implemented: entry.implemented,
                allow_as_param: entry.id.allow_as_param(),
                surface_scope: match entry.id.surface_scope() {
                    NumericSurfaceScope::Public => "public",
                    NumericSurfaceScope::InternalOnly => "internal-only",
                }
                .to_string(),
                runtime_behavior: match entry.id.runtime_behavior() {
                    NumericRuntimeBehavior::Native => "native",
                    NumericRuntimeBehavior::ClampToI8ViaI32 => "clamp-to-i8-via-i32",
                    NumericRuntimeBehavior::ClampToU8ViaU32 => "clamp-to-u8-via-u32",
                }
                .to_string(),
                wgsl_supported: entry.wgsl_supported,
                wgsl_feature: entry.wgsl_feature.map(str::to_string),
                docs: entry.docs.to_string(),
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    };

    let enums = {
        let mut out: Vec<DocEnum> = crate::registry::enum_decls()
            .iter()
            .map(|decl| DocEnum {
                name: decl.id.0.to_string(),
                docs: decl.docs.to_string(),
                variants: {
                    let mut variants = decl
                        .variants
                        .iter()
                        .map(|variant| DocEnumVariant {
                            name: variant.name.to_string(),
                            docs: variant.docs.to_string(),
                        })
                        .collect::<Vec<_>>();
                    variants.sort_by(|a, b| a.name.cmp(&b.name));
                    variants
                },
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    };

    let builtin_reference = {
        let mut discriminator_by_name: BTreeMap<String, DocBuiltinDiscriminator> = BTreeMap::new();
        let mut docs_by_name: BTreeMap<String, String> = BTreeMap::new();
        for decl in crate::registry::builtin_decls() {
            let Some(discriminator) = decl.discriminator else {
                docs_by_name
                    .entry(decl.name.to_string())
                    .or_insert_with(|| decl.docs.to_string());
                continue;
            };

            let mut variants = discriminator
                .variants
                .iter()
                .map(|v| (*v).to_string())
                .collect::<Vec<_>>();
            variants.sort();

            let variant_set = variants.iter().cloned().collect::<BTreeSet<_>>();
            let enum_type = enums.iter().find_map(|enm| {
                let enum_set = enm
                    .variants
                    .iter()
                    .map(|v| v.name.clone())
                    .collect::<BTreeSet<_>>();
                if enum_set == variant_set {
                    Some(enm.name.clone())
                } else {
                    None
                }
            });

            discriminator_by_name
                .entry(decl.name.to_string())
                .or_insert_with(|| DocBuiltinDiscriminator {
                    name: discriminator.name.to_string(),
                    default: discriminator.default.map(str::to_string),
                    variants,
                    enum_type,
                });
            docs_by_name
                .entry(decl.name.to_string())
                .or_insert_with(|| decl.docs.to_string());
        }

        let mut grouped: BTreeMap<String, Vec<DocBuiltinSignature>> = BTreeMap::new();
        for decl in crate::registry::builtin_decls() {
            let receiver = decl.signature.receiver.as_ref().map(type_ref_name);
            let args = decl
                .signature
                .args
                .iter()
                .map(|arg| DocBuiltinArg {
                    name: arg.name.to_string(),
                    ty: builtin_arg_display_type(arg),
                    value_kind: type_ref_kind(&arg.ty).to_string(),
                    viz_role: arg.viz_role.map(str::to_string),
                    enum_type: type_ref_enum_type(&arg.ty),
                    required: arg.required,
                    docs: arg.docs.to_string(),
                })
                .collect::<Vec<_>>();

            let mut returns = BTreeSet::new();
            returns.insert(type_ref_name(&decl.signature.result));
            for alt in decl.signature.result_alternatives {
                returns.insert(type_ref_name(alt));
            }
            grouped
                .entry(decl.name.to_string())
                .or_default()
                .push(DocBuiltinSignature {
                    receiver,
                    args,
                    returns: returns.into_iter().collect(),
                    pipeable: decl.signature.caps.contains(BuiltinCaps::PIPEABLE),
                });
        }

        grouped
            .into_iter()
            .map(|(name, signatures)| {
                let discriminator = discriminator_by_name.get(&name).cloned();
                let summary = docs_by_name
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| "Builtin function.".to_string());
                DocBuiltin {
                    category: builtin_category_from_signatures(&signatures).to_string(),
                    summary,
                    name,
                    discriminator,
                    signatures,
                }
            })
            .collect::<Vec<_>>()
    };
    let mut builtin_reference = builtin_reference;
    builtin_reference.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| {
                let a_sig_key = a
                    .signatures
                    .iter()
                    .flat_map(|sig| sig.args.iter().map(|arg| arg.name.as_str()))
                    .collect::<Vec<_>>();
                let b_sig_key = b
                    .signatures
                    .iter()
                    .flat_map(|sig| sig.args.iter().map(|arg| arg.name.as_str()))
                    .collect::<Vec<_>>();
                a_sig_key.cmp(&b_sig_key)
            })
            .then_with(|| a.category.cmp(&b.category))
            .then_with(|| a.summary.cmp(&b.summary))
    });

    let parser_notes = vec![
        "`contract C for Schema` declarations collect required function signatures and default hook bodies. `style S for Schema : C` supplies exact concrete hook signatures; omitted hooks require explicit contract defaults.".to_string(),
        "Style params support f32, vec2, vec3, vec4, color, u32, i32, and bool, checked constant defaults/ranges, and named material overrides such as `Toon(gain: 0.5)`. Runtime hooks capture per-material settings; engine dispatch uses `@dispatch(C, hook, buffer)` with a u32 settings offset, reflected through `@implementation_settings_value(property)`. Exact integer transport preserves all 32 bits. `static param` produces specialized dispatch identities and compile-time hook bindings, reflects separately in static_parameters, and requires recompilation to edit. Static graph construction supports checked static if and bounded static for ranges. Draw invocations explicitly capture settings as typed named arguments.".to_string(),
        "Styles are selected symbolically through engine-declared `implementation<C>` surface properties and lower to existing explicit @dispatch hooks. Contract schema compatibility is checked at selection; renderer wiring remains engine-authored.".to_string(),
        "`shading_input name: ResourceType scope draw` declares a read-only shading capture; `bind shading.name = handle` inside `for self` binds an owned compute output. `@dispatch(C, hook, settings_buffer, draw)` specializes draw dispatch to the selected material implementation, preserving the authored fallback. Forward, Forward+, and Deferred bind captures per object and frame. An engine recipe can use `@draw(vertex, fragment, instance, count)` for a per-instance procedural pass with factory bindings and no mesh vertex stream; Deferred uses this to resolve pixels belonging to each stored draw-instance ID. Unrestricted dynamic capture dispatch remains unsupported. Shading hooks and their helpers reject implicit derivatives; typed image sample_level/sample_grad require explicit sampling inputs.".to_string(),
        "`@draw_data(instance_id)` on a mesh binding supplies an exclusive uniform<u32> containing a nonzero draw-instance index scoped to the rendered view. It is independent of material-table identity and uses fresh per-frame uniform storage; combining it with table, geometry, or recipe resource sources is invalid.".to_string(),
        "Capability providers can bind `shader_output(node.entry.member)` to a typed raster output and `draw_data(node.binding)` to an exclusive instance-ID binding. The provider-only `draw_instance_id` type is distinct from ordinary u32 values and material-table records. Required capabilities may be comma-separated, for example `requires SurfaceUV, DrawShadingResources`.".to_string(),
        "Shading sampler slots use `shading_input filtering: sampler scope draw` and `bind shading.filtering = nearest_repeat` in `for self`. Immutable standard sampler values are nearest_repeat, nearest_clamp, linear_repeat, and linear_clamp. Factory bindings may use `@sampler(preset) name: sampler`. Sampler state is explicit in artifacts; hosts validate sampling against image formats and enabled device capabilities.".to_string(),
        "Prepared draw operations can select an owned buffer with `raster geometry using vertices base_vertex expression`. The u32 offset uses captured scalar inputs and CPU-known logical resource dimensions, with checked arithmetic, buffer-capacity validation, and a signed base-vertex limit. Original local indices are retained; each frame/view/range owns its generated draw state. Visibility accepts `uncullable` or `geometry.bounds.expand_world(distance)` with a finite nonnegative host f32 distance; `sort_position: geometry.bounds.center` requests range-center sorting. Expansion is evaluated from current captured settings before frustum culling. Missing bounds reject explicit spatial declarations.".to_string(),
        "Draw requires expressions over operation inputs become host preconditions checked per active invocation before submission. They must be boolean and may use captured scalar values, prepared vertex/index counts, and logical owned-resource dimensions, never GPU data. Capability and material requirements remain compile-time checks.".to_string(),
        "Reusable draw operations accept explicit `sampler` parameters supplied by unshadowed standard sampler values. Each invocation retains its own sampler preset. Floating-point image inputs support `sample_level(sampler, uv, lod)` and `sample_grad(sampler, uv, dx, dy)` in draw shader bodies, with typed positional arguments.".to_string(),
        "Contracts accept typed inputs, optional capabilities, and typed attachment points with explicit scope, accepted operations, composition, and before/after phase roles. `provide C for renderer_pipeline` binds typed recipe resources and all(node, ...) boundaries; unused providers are checked. Styles may require capabilities/points or boolean material preconditions. View-scoped raster_draws support ordered_draws(engine.stable_draw_order) and global_transparent_queue with preserving single-sample color attachments and test-only depth. A transparent provider names an existing @transparent_queue(name); ordinary and contributed draws merge by view depth and require premultiplied blending, preserved attachments, and disabled depth writes.".to_string(),
        "`@factory(F) draw D(geometry: DrawRange, ...)` defines reusable typed raster work. `raster geometry` preserves the current object/material range; attachment arguments require explicit load_store policies. Definitions are checked without executing. Scalar/vector arguments accept constants, direct declared runtime settings, and typed field/swizzle projections of settings or contract uniforms. Projections capture the complete source and select fields in shader code; they cannot drive host allocation, dispatch, bounds, or preconditions. Shader bodies cannot capture undeclared engine resources or shadow operation inputs. Graph let bindings may also hold compile-time expressions over static settings and loop variables with lexical scope. Prepared geometry resources also support indexed draws. `compute C(...) -> buffer<T, read>` or `texture2d<Format, read>` defines owned compute work with `output`, `workgroup_size`, `dispatch threads(...)`, and an output return. Selected `let result = C(...)` calls emit typed allocation, binding, and dependency metadata. Returned handles can be passed to draw resource parameters; logical count/width/height members retain producer allocation metadata. The example-engine renderer allocates fresh invocation outputs per frame, schedules kernels from resource dependencies, and resolves returned handles into draw bindings.".to_string(),
        "Engine mesh entries use @vertex/@fragment; @evaluate(surface|vertex) binds a typed surface evaluator. Prepared/procedural vertex output records reserve explicit @location(n) slots and assign unused locations to remaining non-builtin fields.".to_string(),
        "Postfix method calls also accept grouped receivers and line breaks before the dot, preserving ordinary pipe-call semantics. Service linking uses resolved call-site spans, including nested calls, rather than matching adjacent source tokens.".to_string(),
        "`@service(Interface)` declares a pass-local shader service export. Even unused exports must supply one unambiguous ordinary helper per interface method with matching parameter and return types. Reachable local/imported helper bodies are checked for types, return paths, undeclared captures, and recursion. Imported functions retain module scope rather than capturing caller pass bindings or helpers. Entry interfaces, stage/evaluator/dispatch hooks, and implicit method arguments are rejected. Typed providers use `shader_service(node)`; draw operations receive an interface-typed input and call `input.method(arguments)`. Each invocation links only reachable helpers and explicit read-only resource captures. Internal resources must be initialized before its integration boundary; naming the exporting node does not schedule that node.".to_string(),
        "Pass-local raster helpers may return iterator<T> and emit typed values with yield(value). Direct for uses expand the generator with lexical bindings; conditional yields and bounded loops are supported. Arguments evaluate once. Generators finish by falling through; early return/break, recursion, stored iterator values, and imported/global iterator functions are rejected. Expansion is bounded to 256 uses. Logical negation uses ! on boolean expressions.".to_string(),
        "Resource groups use @group(n) group name; @allocate(parameters|textures|storage|globals) selects compiler-managed allocation classes.".to_string(),
        "`@geometry(vertices, indices, vertex_count, index_count, bounds) resource R { ... }` declares engine-owned prepared geometry. `@prepare(R) fn prepare(raw: VertexInterface) -> VertexRecord` on a mesh pass supplies it; a capability provider binds `node.prepare`. Active prepared draw or compute calls demand one range-local stream shared with the base draw. Without demand, vertex entries call preparation inline. Prepared draws accept a u32 vertex index and may return clip_position. Compute and draw shaders receive the same bounds metadata as host visibility/sorting. The bundled static factory supplies range-local bounds expanded for its own deformation; custom factories or authored vertex programs require explicit engine bounds or uncullable visibility. Unavailable bounds have valid=false.".to_string(),
        "Renderer integration uses typed contract points and provide declarations. Calls inside `for self { at point as target { Operation(...) } }` instantiate reusable operations. Authored @contribute, @stage, and @stage_input adapters are removed and produce migration diagnostics; ordinary passes, interfaces, and renderer pipelines remain supported.".to_string(),
        "@attachment(resource,load|clear,store|discard) declares attachment lifetime for recipe and technique draws; omitted operations retain clear/store. Contributions require explicit operations, and loads require initialized contents in the same invocation.".to_string(),
        "@renderer(id,label) pipelines describe executable images, buffers, draws, dispatches and dependencies; host selection is compile configuration.".to_string(),
        "@draw_depth(vertex, count), or @draw_depth(vertex, mesh, count) in a mesh recipe, declares a vertex-only technique draw with an explicit depth attachment; @image(name, float_color) requires a host-supplied floating-point color format, independent of provider names.".to_string(),
        "Surface artifacts expose mesh_passes; a recipe can invoke multiple named mesh programs or reuse one in multiple nodes with independent resource bindings.".to_string(),
        "@technique pipelines reflect standalone compute/draw graphs with typed resources, inferred data dependencies, pool/provider/asset requirements, and named outputs; @from binds another technique output through an engine instance slot.".to_string(),
        "@table(surfaces,ascending|descending|source,first_index) declares artifact-local u32 identities and engine-authored record fields.".to_string(),
        "Surface property schemas are optional and may target material contracts with @surface_properties(block, Material); pass state consumes their constants.".to_string(),
        "Newlines are significant and separate statements and compose entries.".to_string(),
        "Block headers (compose, if, for, in space) may place `{` on the next line.".to_string(),
        "Multiline expression continuation is currently supported before `|>`.".to_string(),
        "Numeric literals support optional unit suffixes: px, uv, deg, s, ms.".to_string(),
        "Texture sampling is explicit: use `image(tex, at: uv)` or `tex.at(uv)`.".to_string(),
        "`texture_type` channel decode supports affine (`* mul +/- add`) and expression (`= expr`) forms; use `raw` in decode expressions.".to_string(),
        "Function declarations support Python-style keyword-only separators: `fn f(a: f32, *, b: f32) -> f32`.".to_string(),
        "Parameters declared after `*` are keyword-only at call sites and cannot be bound positionally.".to_string(),
        "Interface methods and `conform` implementations must match parameter keyword-only boundaries and signature arity exactly.".to_string(),
    ];

    let call_binding_rules = vec![
        "Arguments bind by explicit name first, then remaining positional arguments bind left-to-right.".to_string(),
        "Parameters declared after `*` are keyword-only and must be provided by name.".to_string(),
        "Supplying a positional argument for a keyword-only parameter is rejected as a missing named argument.".to_string(),
        "Unknown named arguments and extra positional arguments are rejected.".to_string(),
        "For user-defined functions, argument type checking occurs after binding and reports parameter-scoped diagnostics.".to_string(),
    ];

    let mut space_transform_reference = crate::registry::space_transform_decls()
        .iter()
        .map(|decl| DocSpaceTransform {
            name: decl.name.to_string(),
            summary: decl.summary.to_string(),
            args: decl
                .args
                .iter()
                .map(|arg| DocSpaceTransformArg {
                    name: arg.name.to_string(),
                    ty: arg.ty.to_string(),
                    value_kind: arg.value_kind.to_string(),
                    enum_type: arg.enum_type.map(str::to_string),
                    required: arg.required,
                    docs: arg.docs.to_string(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    space_transform_reference.sort_by(|a, b| a.name.cmp(&b.name));

    let mut callable_reference = crate::registry::callable_decls()
        .iter()
        .map(|decl| DocCallable {
            name: decl.name.to_string(),
            summary: decl.summary.to_string(),
            context: decl.context.to_string(),
            args: decl
                .args
                .iter()
                .map(|arg| DocCallableArg {
                    name: arg.name.to_string(),
                    ty: arg.ty.to_string(),
                    value_kind: arg.value_kind.to_string(),
                    required: arg.required,
                    docs: arg.docs.to_string(),
                })
                .collect(),
            returns: decl
                .returns
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        })
        .collect::<Vec<_>>();
    callable_reference.sort_by(|a, b| a.name.cmp(&b.name));

    let stdlib_exports = {
        let mut out: Vec<DocStdlibExport> = prelude_exports(prelude);
        for export in &mut out {
            export
                .params
                .sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.ty_name.cmp(&b.ty_name)));
        }
        out.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.params.len().cmp(&b.params.len()))
                .then_with(|| {
                    let a_params = a
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), p.ty_name.clone()))
                        .collect::<Vec<_>>();
                    let b_params = b
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), p.ty_name.clone()))
                        .collect::<Vec<_>>();
                    a_params.cmp(&b_params)
                })
                .then_with(|| a.ret_ty.cmp(&b.ret_ty))
        });
        out
    };

    LanguageDocsModel {
        schema_version: LANGUAGE_DOCS_SCHEMA_VERSION,
        keywords,
        type_keywords: type_keywords.into_iter().collect(),
        units,
        builtins,
        callables,
        space_transforms,
        blend_modes,
        enum_members,
        enum_variants_by_type,
        parser_notes,
        call_binding_rules,
        types,
        numeric_primitive_catalog,
        enums,
        space_transform_reference,
        callable_reference,
        builtin_reference,
        stdlib_exports,
    }
}

pub fn render_language_docs_markdown(model: &LanguageDocsModel) -> String {
    fn esc(text: &str) -> String {
        text.replace('|', "\\|").replace('\n', " ")
    }

    fn type_name_list(items: &[String]) -> String {
        if items.is_empty() {
            "-".to_string()
        } else {
            items.join(" | ")
        }
    }

    fn format_builtin_signatures(name: &str, sig: &DocBuiltinSignature) -> Vec<String> {
        let receiver = sig
            .receiver
            .as_deref()
            .map_or(String::new(), |r| format!("{r} |> "));
        let args = sig
            .args
            .iter()
            .map(|arg| {
                let req = if arg.required { "" } else { "?" };
                format!("{}{}: {}", arg.name, req, arg.ty)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let returns = type_name_list(&sig.returns);
        let direct = format!("{}fn({}) -> {}", receiver, args, returns);
        if sig.receiver.is_none()
            && sig.pipeable
            && sig.args.first().is_some_and(|arg| arg.required)
        {
            let recv_ty = sig.args[0].ty.clone();
            let piped_args = sig
                .args
                .iter()
                .skip(1)
                .map(|arg| {
                    let req = if arg.required { "" } else { "?" };
                    format!("{}{}: {}", arg.name, req, arg.ty)
                })
                .collect::<Vec<_>>()
                .join(", ");
            let pipe_form = format!("{} |> {}({}) -> {}", recv_ty, name, piped_args, returns);
            vec![direct, pipe_form]
        } else {
            vec![direct]
        }
    }

    fn format_space_signature(xf: &DocSpaceTransform) -> String {
        let args = xf
            .args
            .iter()
            .map(|arg| {
                let req = if arg.required { "" } else { "?" };
                format!("{}{}: {}", arg.name, req, arg.ty)
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}({})", xf.name, args)
    }

    let mut out = String::new();
    out.push_str("# Fresco Language API Reference\n\n");
    out.push_str(&format!("_Schema: `{}`_\n\n", model.schema_version));

    out.push_str("## Overview\n\n");
    out.push_str("- Keywords: ");
    out.push_str(&model.keywords.join(", "));
    out.push('\n');
    out.push_str("- Type Keywords: ");
    out.push_str(&model.type_keywords.join(", "));
    out.push('\n');
    out.push_str("- Units: ");
    out.push_str(&model.units.join(", "));
    out.push('\n');
    out.push_str("- Blend Modes: ");
    out.push_str(&model.blend_modes.join(", "));
    out.push_str("\n\n");

    out.push_str("## Parser Notes\n\n");
    for note in &model.parser_notes {
        out.push_str("- ");
        out.push_str(note);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("## Call Binding Rules\n\n");
    for rule in &model.call_binding_rules {
        out.push_str("- ");
        out.push_str(rule);
        out.push('\n');
    }
    out.push('\n');

    out.push_str("## Texture Sampling And Decode Examples\n\n");
    out.push_str("```fresco\n");
    out.push_str("texture_type ORM {\n");
    out.push_str("  r: occlusion\n");
    out.push_str("  g: roughness\n");
    out.push_str("  b: metallic\n");
    out.push_str("}\n\n");
    out.push_str("canvas example(uv: coord) -> color {\n");
    out.push_str("  param orm: texture<ORM> = \"orm.png\"\n");
    out.push_str("  let sampled = orm.at(uv * 2.0)\n");
    out.push_str("  let rough = sampled.roughness\n");
    out.push_str("  compose {\n");
    out.push_str("    sampled\n");
    out.push_str("    circle(at: center, radius: 0.2) |> fill(rgba(rough, rough, rough, 0.35))\n");
    out.push_str("  }\n");
    out.push_str("}\n");
    out.push_str("```\n\n");
    out.push_str("```fresco\n");
    out.push_str("texture_type Packed {\n");
    out.push_str("  r: id = bit_extract(raw, lsb: 0, bits: 5) / 31.0\n");
    out.push_str("  g: normal_x = unpack_snorm8(raw, byte: 1)\n");
    out.push_str("  b: normal_y = unpack_snorm8(raw, byte: 2)\n");
    out.push_str("}\n");
    out.push_str("```\n\n");
    out.push_str("Use `raw` inside `texture_type` decode expressions to access the sampled channel value.\n\n");
    out.push_str("Bit utility helpers: `bit_extract`, `unpack_unorm8`, `unpack_snorm8`, `pack_unorm8x4`.\n\n");

    out.push_str("## Types\n\n");
    out.push_str("| Type | Kind | Docs |\n");
    out.push_str("| --- | --- | --- |\n");
    for ty in &model.types {
        out.push_str("| ");
        out.push_str(&esc(&ty.name));
        out.push_str(" | ");
        out.push_str(&esc(&ty.kind));
        out.push_str(" | ");
        out.push_str(&esc(&ty.docs));
        out.push_str(" |\n");
    }
    out.push('\n');

    out.push_str("## Numeric Primitive Catalog\n\n");
    out.push_str("| Name | Family | Scalar | Shape | Implemented | Param | Scope | Runtime | WGSL | Feature | Docs |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for ty in &model.numeric_primitive_catalog {
        out.push_str("| ");
        out.push_str(&esc(&ty.name));
        out.push_str(" | ");
        out.push_str(&esc(&ty.family));
        out.push_str(" | ");
        out.push_str(&esc(&ty.scalar_kind));
        out.push_str(" | ");
        out.push_str(&format!("{}x{}", ty.columns, ty.rows));
        out.push_str(" | ");
        out.push_str(if ty.implemented { "yes" } else { "no" });
        out.push_str(" | ");
        out.push_str(if ty.allow_as_param { "yes" } else { "no" });
        out.push_str(" | ");
        out.push_str(&esc(&ty.surface_scope));
        out.push_str(" | ");
        out.push_str(&esc(&ty.runtime_behavior));
        out.push_str(" | ");
        out.push_str(if ty.wgsl_supported { "yes" } else { "no" });
        out.push_str(" | ");
        out.push_str(&esc(ty.wgsl_feature.as_deref().unwrap_or("-")));
        out.push_str(" | ");
        out.push_str(&esc(&ty.docs));
        out.push_str(" |\n");
    }
    out.push('\n');

    out.push_str("## Enums\n\n");
    for en in &model.enums {
        out.push_str("### ");
        out.push_str(&en.name);
        out.push_str("\n\n");
        out.push_str(&en.docs);
        out.push_str("\n\n");
        out.push_str("| Variant | Docs |\n");
        out.push_str("| --- | --- |\n");
        for v in &en.variants {
            out.push_str("| ");
            out.push_str(&esc(&v.name));
            out.push_str(" | ");
            out.push_str(&esc(&v.docs));
            out.push_str(" |\n");
        }
        out.push('\n');
    }

    out.push_str("## Space Transforms\n\n");
    for xf in &model.space_transform_reference {
        out.push_str("### ");
        out.push_str(&xf.name);
        out.push_str("\n\n");
        out.push_str("- Summary: ");
        out.push_str(&xf.summary);
        out.push('\n');
        out.push_str("- Signature: `");
        out.push_str(&format_space_signature(xf));
        out.push_str("`\n\n");
        if !xf.args.is_empty() {
            out.push_str("| Arg | Type | Required | Docs |\n");
            out.push_str("| --- | --- | --- | --- |\n");
            for arg in &xf.args {
                out.push_str("| ");
                out.push_str(&esc(&arg.name));
                out.push_str(" | ");
                out.push_str(&esc(&arg.ty));
                out.push_str(" | ");
                out.push_str(if arg.required { "yes" } else { "no" });
                out.push_str(" | ");
                out.push_str(&esc(&arg.docs));
                out.push_str(" |\n");
            }
            out.push('\n');
        }
    }

    out.push_str("## Callables\n\n");
    for callable in &model.callable_reference {
        out.push_str("### ");
        out.push_str(&callable.name);
        out.push_str("\n\n");
        out.push_str("- Context: ");
        out.push_str(&callable.context);
        out.push('\n');
        out.push_str("- Summary: ");
        out.push_str(&callable.summary);
        out.push_str("\n\n");
        if !callable.args.is_empty() {
            out.push_str("| Arg | Type | Required | Docs |\n");
            out.push_str("| --- | --- | --- | --- |\n");
            for arg in &callable.args {
                out.push_str("| ");
                out.push_str(&esc(&arg.name));
                out.push_str(" | ");
                out.push_str(&esc(&arg.ty));
                out.push_str(" | ");
                out.push_str(if arg.required { "yes" } else { "no" });
                out.push_str(" | ");
                out.push_str(&esc(&arg.docs));
                out.push_str(" |\n");
            }
            out.push('\n');
        }
    }

    out.push_str("## Builtins\n\n");
    let mut by_category: BTreeMap<&str, Vec<&DocBuiltin>> = BTreeMap::new();
    for builtin in &model.builtin_reference {
        by_category
            .entry(builtin.category.as_str())
            .or_default()
            .push(builtin);
    }

    for (category, mut builtins) in by_category {
        builtins.sort_by(|a, b| a.name.cmp(&b.name));
        out.push_str("### ");
        out.push_str(category);
        out.push_str("\n\n");
        for builtin in builtins {
            out.push_str("#### ");
            out.push_str(&builtin.name);
            out.push_str("\n\n");
            if !builtin.summary.is_empty() && builtin.summary != builtin.name {
                out.push_str(&builtin.summary);
                out.push_str("\n\n");
            }

            if let Some(discriminator) = &builtin.discriminator {
                out.push_str("- Discriminator: `");
                out.push_str(&discriminator.name);
                out.push_str("`\n");
                if let Some(default) = &discriminator.default {
                    out.push_str("- Default Variant: `");
                    out.push_str(default);
                    out.push_str("`\n");
                }
                if !discriminator.variants.is_empty() {
                    out.push_str("- Variants: ");
                    out.push_str(&discriminator.variants.join(", "));
                    out.push('\n');
                }
                out.push('\n');
            }

            out.push_str("| Signature | Returns | Pipeable |\n");
            out.push_str("| --- | --- | --- |\n");
            for sig in &builtin.signatures {
                for rendered_sig in format_builtin_signatures(&builtin.name, sig) {
                    out.push_str("| `");
                    out.push_str(&esc(&rendered_sig));
                    out.push_str("` | ");
                    out.push_str(&esc(&type_name_list(&sig.returns)));
                    out.push_str(" | ");
                    out.push_str(if sig.pipeable { "yes" } else { "no" });
                    out.push_str(" |\n");
                }
            }
            out.push('\n');

            let mut seen_args = BTreeSet::new();
            let mut all_args = Vec::new();
            for sig in &builtin.signatures {
                for arg in &sig.args {
                    let key = (arg.name.clone(), arg.ty.clone(), arg.required);
                    if seen_args.insert(key) {
                        all_args.push(arg);
                    }
                }
            }

            if !all_args.is_empty() {
                out.push_str("| Arg | Type | Required | Viz Role | Docs |\n");
                out.push_str("| --- | --- | --- | --- | --- |\n");
                for arg in all_args {
                    out.push_str("| ");
                    out.push_str(&esc(&arg.name));
                    out.push_str(" | ");
                    out.push_str(&esc(&arg.ty));
                    out.push_str(" | ");
                    out.push_str(if arg.required { "yes" } else { "no" });
                    out.push_str(" | ");
                    out.push_str(&esc(arg.viz_role.as_deref().unwrap_or("-")));
                    out.push_str(" | ");
                    out.push_str(&esc(&arg.docs));
                    out.push_str(" |\n");
                }
                out.push('\n');
            }
        }
    }

    out.push_str("## Stdlib Exports\n\n");
    out.push_str("| Name | Params | Returns | Docs |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for export in &model.stdlib_exports {
        let docs = export
            .docs
            .as_deref()
            .filter(|docs| !docs.is_empty() && *docs != export.name);
        let params = export
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, p.ty_name))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str("| ");
        out.push_str(&esc(&export.name));
        out.push_str(" | ");
        out.push_str(&esc(&params));
        out.push_str(" | ");
        out.push_str(&esc(&export.ret_ty));
        out.push_str(" | ");
        out.push_str(&esc(docs.unwrap_or("-")));
        out.push_str(" |\n");
    }

    out.push('\n');
    for export in &model.stdlib_exports {
        if let Some(docs) = &export.docs {
            out.push_str("### ");
            out.push_str(&export.name);
            out.push_str("\n\n");
            out.push_str(docs);
            out.push_str("\n\n");
            if !export.params.is_empty() {
                out.push_str("| Param | Type | Docs |\n");
                out.push_str("| --- | --- | --- |\n");
                for param in &export.params {
                    out.push_str("| ");
                    out.push_str(&esc(&param.name));
                    out.push_str(" | ");
                    out.push_str(&esc(&param.ty_name));
                    out.push_str(" | ");
                    out.push_str(&esc(param.docs.as_deref().unwrap_or("-")));
                    out.push_str(" |\n");
                }
                out.push('\n');
            }
        }
    }

    out
}

fn type_ref_kind(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::Enum(_) => "enum",
        _ => "expr",
    }
}

fn type_ref_enum_type(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Enum(id) => Some(id.0.to_string()),
        _ => None,
    }
}

fn builtin_arg_display_type(arg: &crate::registry::BuiltinArgDecl) -> String {
    match arg.ty {
        TypeRef::Primitive(PrimitiveType::Expr) => {
            let docs = arg.docs;
            if docs.contains("scalar, vec2, vec3, or vec4")
                || docs.contains("value (scalar, vec2, vec3, or vec4)")
                || docs.contains("angle in radians (scalar, vec2, vec3, or vec4)")
            {
                "scalar | vec2 | vec3 | vec4".to_string()
            } else if docs.contains("vec2, vec3, or vec4") {
                "vec2 | vec3 | vec4".to_string()
            } else if docs.contains("scalar") && docs.contains("vec2") && docs.contains("vec3") {
                "scalar | vec2 | vec3 | vec4".to_string()
            } else {
                "expression".to_string()
            }
        }
        TypeRef::Primitive(PrimitiveType::Color) => "color | gradient".to_string(),
        _ => type_ref_name(&arg.ty),
    }
}

fn type_ref_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Expr) => "expression".to_string(),
        TypeRef::Primitive(PrimitiveType::Path) => "path".to_string(),
        TypeRef::Primitive(PrimitiveType::Contour) => "contour".to_string(),
        TypeRef::Primitive(PrimitiveType::Shape) => "shape".to_string(),
        TypeRef::Primitive(PrimitiveType::Layer) => "layer".to_string(),
        TypeRef::Primitive(PrimitiveType::Scalar) => "scalar".to_string(),
        TypeRef::Primitive(PrimitiveType::I32) => "i32".to_string(),
        TypeRef::Primitive(PrimitiveType::U32) => "u32".to_string(),
        TypeRef::Primitive(PrimitiveType::F64) => "f64".to_string(),
        TypeRef::Primitive(PrimitiveType::Half) => "half".to_string(),
        TypeRef::Primitive(PrimitiveType::Vec2) => "vec2".to_string(),
        TypeRef::Primitive(PrimitiveType::Vec3) => "vec3".to_string(),
        TypeRef::Primitive(PrimitiveType::Vec4) => "vec4".to_string(),
        TypeRef::Primitive(PrimitiveType::Mat2) => "mat2".to_string(),
        TypeRef::Primitive(PrimitiveType::Mat3) => "mat3".to_string(),
        TypeRef::Primitive(PrimitiveType::Mat4) => "mat4".to_string(),
        TypeRef::Primitive(PrimitiveType::Color) => "color".to_string(),
        TypeRef::Primitive(PrimitiveType::ColorField) => "color_field".to_string(),
        TypeRef::Primitive(PrimitiveType::Coverage) => "coverage".to_string(),
        TypeRef::Primitive(PrimitiveType::Mask) => "mask".to_string(),
        TypeRef::Primitive(PrimitiveType::Gradient) => "gradient".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
        TypeRef::Primitive(PrimitiveType::Coord) => "coord".to_string(),
        TypeRef::Primitive(PrimitiveType::CoordLike) => "coord_like".to_string(),
        TypeRef::Primitive(PrimitiveType::Signal) => "signal".to_string(),
        TypeRef::Primitive(PrimitiveType::Delta) => "delta".to_string(),
        TypeRef::Primitive(PrimitiveType::Resolution) => "resolution".to_string(),
        TypeRef::Primitive(PrimitiveType::Angle) => "angle".to_string(),
        TypeRef::Primitive(PrimitiveType::Length) => "length".to_string(),
        TypeRef::Named(id) => id.0.to_string(),
        TypeRef::Enum(id) => id.0.to_string(),
    }
}

fn builtin_category_from_signatures(signatures: &[DocBuiltinSignature]) -> &'static str {
    let has_shape_receiver = signatures
        .iter()
        .any(|sig| sig.receiver.as_deref() == Some("shape"));
    let has_shape_return = signatures
        .iter()
        .any(|sig| sig.returns.iter().any(|ret| ret == "shape"));
    let has_layer_return = signatures
        .iter()
        .any(|sig| sig.returns.iter().any(|ret| ret == "layer"));
    let has_math_return = signatures.iter().any(|sig| {
        sig.returns
            .iter()
            .any(|ret| matches!(ret.as_str(), "scalar" | "vec2" | "vec3" | "vec4"))
    });

    if has_shape_receiver && has_shape_return {
        "shape-ops"
    } else if has_shape_return {
        "shapes"
    } else if has_layer_return {
        "layer"
    } else if has_math_return {
        "math"
    } else {
        "other"
    }
}

fn parse_prelude_program(src: &str) -> Result<crate::ast::Program, Vec<crate::diag::Diag>> {
    fn perr_to_diag(e: crate::parser::PError<'_>) -> crate::diag::Diag {
        if let chumsky::error::RichReason::Custom(msg) = e.reason() {
            let mut d = crate::diag::Diag::error(e.span().clone(), msg.to_string());
            if let Some((label, _)) = e.contexts().last() {
                d = d.with_label(format!("while parsing {label}"));
            }
            return d;
        }

        let found = e
            .found()
            .map(ToString::to_string)
            .unwrap_or_else(|| "end of input".to_string());
        let mut expected: Vec<String> = e.expected().map(ToString::to_string).collect();
        expected.sort();
        expected.dedup();

        let mut d = crate::diag::Diag::error(e.span().clone(), format!("unexpected {found}"));
        if let Some((label, _)) = e.contexts().last() {
            d = d.with_label(format!("while parsing {label}"));
        } else {
            d = d.with_label("unexpected token");
        }
        if !expected.is_empty() {
            d = d.with_help(format!("expected {}", expected.join(", ")));
        }
        d
    }

    let mut tokens = Vec::new();
    let mut lex_diags = Vec::new();
    for (tok, span) in crate::lexer::Token::lexer(src).spanned() {
        match tok {
            Ok(token) => tokens.push((token, span)),
            Err(()) => lex_diags.push(
                crate::diag::Diag::error(span, "unrecognized token")
                    .with_help("valid literals look like: 0.5, 12px, 20deg, 2s, #ff2d78"),
            ),
        }
    }
    if !lex_diags.is_empty() {
        return Err(lex_diags);
    }

    let eoi = src.len()..src.len();
    let (program, parse_errs) = crate::parser::program()
        .parse(crate::parser::input(&tokens, eoi))
        .into_output_errors();

    if !parse_errs.is_empty() {
        return Err(parse_errs.into_iter().map(perr_to_diag).collect());
    }

    Ok(program.unwrap_or_default())
}

fn prelude_program(source: &str) -> Result<crate::ast::Program, Vec<crate::diag::Diag>> {
    let mut program = parse_prelude_program(source)?;
    crate::parser::attach_leading_fn_docs(source, &mut program);
    Ok(program)
}

fn param_docs_from_fn_docs(docs: &str, name: &str) -> Option<String> {
    let prefix = format!("- {name}:");
    docs.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix(prefix.as_str())
            .map(|text| text.trim().to_string())
    })
}

/// Type-name mapping for the stdlib exports table specifically: several
/// scalar-kind primitives (coverage, mask, signal, delta, angle, length, ...)
/// collapse to the single `f32` bucket here, distinct from the more granular
/// per-kind names `type_ref_name` uses for the main builtin reference table.
fn stdlib_export_type_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Expr) => "expression".to_string(),
        TypeRef::Primitive(PrimitiveType::Path) => "path".to_string(),
        TypeRef::Primitive(PrimitiveType::Contour) => "contour".to_string(),
        TypeRef::Primitive(PrimitiveType::Shape) => "shape".to_string(),
        TypeRef::Primitive(PrimitiveType::Layer) => "layer".to_string(),
        TypeRef::Primitive(PrimitiveType::Scalar)
        | TypeRef::Primitive(PrimitiveType::I32)
        | TypeRef::Primitive(PrimitiveType::U32)
        | TypeRef::Primitive(PrimitiveType::F64)
        | TypeRef::Primitive(PrimitiveType::Half)
        | TypeRef::Primitive(PrimitiveType::Coverage)
        | TypeRef::Primitive(PrimitiveType::Mask)
        | TypeRef::Primitive(PrimitiveType::Signal)
        | TypeRef::Primitive(PrimitiveType::Delta)
        | TypeRef::Primitive(PrimitiveType::Angle)
        | TypeRef::Primitive(PrimitiveType::Length) => "f32".to_string(),
        TypeRef::Primitive(PrimitiveType::Vec2)
        | TypeRef::Primitive(PrimitiveType::Coord)
        | TypeRef::Primitive(PrimitiveType::CoordLike)
        | TypeRef::Primitive(PrimitiveType::Resolution) => "vec2".to_string(),
        TypeRef::Primitive(PrimitiveType::Vec3) => "vec3".to_string(),
        TypeRef::Primitive(PrimitiveType::Vec4) => "vec4".to_string(),
        TypeRef::Primitive(PrimitiveType::Mat2) => "mat2".to_string(),
        TypeRef::Primitive(PrimitiveType::Mat3) => "mat3".to_string(),
        TypeRef::Primitive(PrimitiveType::Mat4) => "mat4".to_string(),
        TypeRef::Primitive(PrimitiveType::Color)
        | TypeRef::Primitive(PrimitiveType::ColorField)
        | TypeRef::Primitive(PrimitiveType::Gradient) => "color".to_string(),
        TypeRef::Primitive(PrimitiveType::Bool) => "bool".to_string(),
        TypeRef::Named(id) => id.0.to_string(),
        TypeRef::Enum(id) => id.0.to_string(),
    }
}

/// Arg-display mapping for the stdlib exports table; see
/// `stdlib_export_type_name`.
fn stdlib_export_arg_display_type(arg: &crate::registry::BuiltinArgDecl) -> String {
    match arg.ty {
        TypeRef::Primitive(PrimitiveType::Expr) => {
            let docs = arg.docs;
            if docs.contains("scalar, vec2, vec3, or vec4")
                || docs.contains("value (scalar, vec2, vec3, or vec4)")
                || docs.contains("angle in radians (scalar, vec2, vec3, or vec4)")
            {
                "scalar | vec2 | vec3 | vec4".to_string()
            } else if docs.contains("vec2, vec3, or vec4") {
                "vec2 | vec3 | vec4".to_string()
            } else {
                "expression".to_string()
            }
        }
        TypeRef::Primitive(PrimitiveType::Color) => "color | gradient".to_string(),
        _ => stdlib_export_type_name(&arg.ty),
    }
}

fn prelude_exports(prelude: Option<crate::ast::Program>) -> Vec<DocStdlibExport> {
    let mut out: Vec<DocStdlibExport> = crate::registry::builtin_decls()
        .iter()
        .copied()
        .filter(|decl| decl.signature.receiver.is_none())
        .map(|decl| DocStdlibExport {
            name: decl.name.to_string(),
            params: decl
                .signature
                .args
                .iter()
                .map(|arg| DocStdlibExportParam {
                    name: arg.name.to_string(),
                    ty_name: stdlib_export_arg_display_type(arg),
                    docs: Some(arg.docs.to_string()),
                })
                .collect(),
            ret_ty: {
                let mut result_types = BTreeSet::new();
                result_types.insert(stdlib_export_type_name(&decl.signature.result));
                for alt in decl.signature.result_alternatives {
                    result_types.insert(stdlib_export_type_name(alt));
                }
                result_types.into_iter().collect::<Vec<_>>().join(" | ")
            },
            docs: Some(decl.docs.to_string()),
        })
        .collect();

    if let Some(prelude) = prelude {
        out.extend(
            prelude
                .functions
                .into_iter()
                .filter(|func| !func.is_internal)
                .map(|func| {
                    let docs = func.docs;
                    DocStdlibExport {
                        name: func.name,
                        params: func
                            .params
                            .into_iter()
                            .map(|param| {
                                let param_name = param.name;
                                let export_name = if param.keyword_only {
                                    format!("{}*", param_name)
                                } else {
                                    param_name.clone()
                                };

                                DocStdlibExportParam {
                                    name: export_name,
                                    ty_name: param.ty_name,
                                    docs: docs.as_deref().and_then(|doc_text| {
                                        param_docs_from_fn_docs(doc_text, &param_name)
                                    }),
                                }
                            })
                            .collect(),
                        ret_ty: func.ret_ty.map(|(ty, _)| ty).unwrap_or_default(),
                        docs,
                    }
                }),
        );
    }

    out.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.params.len().cmp(&b.params.len()))
    });
    out.dedup_by(|a, b| {
        a.name == b.name
            && a.params.len() == b.params.len()
            && a.ret_ty == b.ret_ty
            && a.params
                .iter()
                .map(|p| p.ty_name.as_str())
                .eq(b.params.iter().map(|p| p.ty_name.as_str()))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::{build_language_docs_model, render_language_docs_markdown};
    use std::collections::BTreeSet;

    #[test]
    fn discriminated_builtin_arguments_have_stable_name_order() {
        let docs = build_language_docs_model();
        let mut checked = 0;
        for builtin in docs
            .builtin_reference
            .iter()
            .filter(|item| item.discriminator.is_some())
        {
            for signature in &builtin.signatures {
                assert!(
                    signature
                        .args
                        .windows(2)
                        .all(|pair| pair[0].name <= pair[1].name),
                    "{} has unstable discriminated argument order",
                    builtin.name
                );
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn authored_library_metadata_is_explicit_and_invalid_source_is_reported() {
        let intrinsic = build_language_docs_model();
        assert!(
            !intrinsic
                .stdlib_exports
                .iter()
                .any(|item| item.name == "probe_gain")
        );
        let extended = super::build_language_docs_model_with_prelude(
            "// Gain supplied by the host library.\nfn probe_gain(value: f32) -> f32 { return value * 2.0 }",
        ).expect("valid authored library");
        assert!(
            extended
                .stdlib_exports
                .iter()
                .any(|item| item.name == "probe_gain")
        );
        assert_eq!(intrinsic.builtins, extended.builtins);
        assert!(super::build_language_docs_model_with_prelude("fn broken(").is_err());
    }

    #[test]
    fn builtin_discriminator_docs_cover_all_registry_discriminators() {
        let docs = build_language_docs_model();

        let expected_discriminated: BTreeSet<String> = crate::registry::builtin_decls()
            .iter()
            .filter_map(|decl| decl.discriminator.map(|_| decl.name.to_string()))
            .collect();
        let actual_discriminated: BTreeSet<String> = docs
            .builtin_reference
            .iter()
            .filter(|builtin| builtin.discriminator.is_some())
            .map(|builtin| builtin.name.clone())
            .collect();

        assert_eq!(actual_discriminated, expected_discriminated);
    }

    #[test]
    fn builtin_discriminator_docs_resolve_enum_type_or_variants_consistently() {
        let docs = build_language_docs_model();

        for builtin in docs
            .builtin_reference
            .iter()
            .filter(|builtin| builtin.discriminator.is_some())
        {
            let discriminator = builtin.discriminator.as_ref().expect("checked above");

            assert!(
                !discriminator.variants.is_empty(),
                "builtin `{}` discriminator `{}` has no variants",
                builtin.name,
                discriminator.name
            );

            let discriminator_set = discriminator
                .variants
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();

            if let Some(enum_type) = discriminator.enum_type.as_deref() {
                let enum_variants = docs
                    .enum_variants_by_type
                    .get(enum_type)
                    .unwrap_or_else(|| {
                        panic!(
                            "builtin `{}` discriminator `{}` references missing enum `{}`",
                            builtin.name, discriminator.name, enum_type
                        )
                    })
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>();

                assert_eq!(
                    discriminator_set, enum_variants,
                    "builtin `{}` discriminator `{}` variants drifted from enum `{}`",
                    builtin.name, discriminator.name, enum_type
                );
            }
        }
    }

    #[test]
    fn pipeable_receiverless_builtins_render_pipe_form() {
        let docs = build_language_docs_model();
        let markdown = render_language_docs_markdown(&docs);

        assert!(
            markdown
                .contains("`color \\| gradient \\|> darken(by: scalar) -> color` | color | yes |"),
            "expected docs to render alternate pipe form for darken\nmarkdown:\n{markdown}"
        );
    }

    #[test]
    fn builtin_arg_kind_exports_expr_for_primitive_args() {
        let docs = build_language_docs_model();

        let mix = docs
            .builtin_reference
            .iter()
            .find(|builtin| builtin.name == "mix")
            .expect("mix builtin docs missing");
        let first_arg_kind = mix
            .signatures
            .iter()
            .find_map(|sig| sig.args.first())
            .map(|arg| arg.value_kind.as_str())
            .expect("mix builtin docs missing args");

        assert_eq!(
            first_arg_kind, "expr",
            "primitive builtin args should export as expression-valued docs entries"
        );
    }

    #[test]
    fn signal_builtin_args_export_explicit_viz_roles() {
        let docs = build_language_docs_model();

        let wave = docs
            .builtin_reference
            .iter()
            .find(|builtin| builtin.name == "wave")
            .expect("wave builtin docs missing");
        let wave_args = wave
            .signatures
            .iter()
            .flat_map(|sig| sig.args.iter())
            .collect::<Vec<_>>();

        assert!(
            wave_args
                .iter()
                .any(|arg| arg.name == "period" && arg.viz_role.as_deref() == Some("period")),
            "wave.period should be tagged with viz_role=period"
        );
        assert!(
            wave_args
                .iter()
                .any(|arg| arg.name == "range" && arg.viz_role.as_deref() == Some("range")),
            "wave.range should be tagged with viz_role=range"
        );

        let pulse = docs
            .builtin_reference
            .iter()
            .find(|builtin| builtin.name == "pulse")
            .expect("pulse builtin docs missing");
        assert!(
            pulse
                .signatures
                .iter()
                .flat_map(|sig| sig.args.iter())
                .any(|arg| arg.name == "every" && arg.viz_role.as_deref() == Some("period")),
            "pulse.every should be tagged with viz_role=period"
        );

        let ramp = docs
            .builtin_reference
            .iter()
            .find(|builtin| builtin.name == "ramp")
            .expect("ramp builtin docs missing");
        assert!(
            ramp.signatures
                .iter()
                .flat_map(|sig| sig.args.iter())
                .any(|arg| arg.name == "over" && arg.viz_role.as_deref() == Some("window")),
            "ramp.over should be tagged with viz_role=window"
        );

        let wrap = docs
            .builtin_reference
            .iter()
            .find(|builtin| builtin.name == "wrap")
            .expect("wrap builtin docs missing");
        assert!(
            wrap.signatures
                .iter()
                .flat_map(|sig| sig.args.iter())
                .any(|arg| arg.name == "range" && arg.viz_role.as_deref() == Some("range")),
            "wrap.range should be tagged with viz_role=range"
        );

        let rand = docs
            .builtin_reference
            .iter()
            .find(|builtin| builtin.name == "rand")
            .expect("rand builtin docs missing");
        assert!(
            rand.signatures
                .iter()
                .flat_map(|sig| sig.args.iter())
                .any(|arg| arg.name == "range" && arg.viz_role.as_deref() == Some("range")),
            "rand.range should be tagged with viz_role=range"
        );
    }

    #[test]
    fn space_transform_reference_exports_typed_args_from_registry() {
        let docs = build_language_docs_model();

        let rotate = docs
            .space_transform_reference
            .iter()
            .find(|xf| xf.name == "rotate")
            .expect("rotate transform docs missing");
        assert!(
            rotate
                .args
                .iter()
                .any(|arg| arg.name == "angle" && arg.ty == "scalar" && arg.required),
            "rotate.angle should export as required scalar arg"
        );
        assert!(
            rotate
                .args
                .iter()
                .any(|arg| arg.name == "around" && arg.ty == "vec2" && !arg.required),
            "rotate.around should export as optional vec2 arg"
        );

        let repeat = docs
            .space_transform_reference
            .iter()
            .find(|xf| xf.name == "repeat")
            .expect("repeat transform docs missing");
        assert!(
            repeat
                .args
                .iter()
                .any(|arg| arg.name == "cell" && arg.value_kind == "binding"),
            "repeat.cell should export as binding arg"
        );

        let centered = docs
            .space_transform_reference
            .iter()
            .find(|xf| xf.name == "centered")
            .expect("centered transform docs missing");
        assert!(
            centered.args.iter().any(|arg| {
                arg.name == "aspect"
                    && arg.value_kind == "enum"
                    && arg.enum_type.as_deref() == Some("CenteredMode")
            }),
            "centered.aspect should export enum metadata"
        );

        assert!(
            docs.space_transform_reference
                .iter()
                .any(|xf| xf.name == "warp" && !xf.args.is_empty()),
            "warp transform docs should be present with args"
        );
    }

    #[test]
    fn language_docs_generation_is_deterministic() {
        let first = build_language_docs_model();
        let first_json =
            serde_json::to_string_pretty(&first).expect("serialize first language docs model");
        let first_markdown = render_language_docs_markdown(&first);

        let second = build_language_docs_model();
        let second_json =
            serde_json::to_string_pretty(&second).expect("serialize second language docs model");
        let second_markdown = render_language_docs_markdown(&second);

        assert_eq!(
            first_json, second_json,
            "language docs JSON output should be deterministic across consecutive builds"
        );
        assert_eq!(
            first_markdown, second_markdown,
            "language docs markdown output should be deterministic across consecutive builds"
        );
    }

    #[test]
    fn prelude_program_parses_semantic_helpers() {
        let program = super::prelude_program(crate::test_support::PRELUDE)
            .expect("stdlib prelude should parse");
        let names = program
            .functions
            .iter()
            .map(|func| func.name.as_str())
            .collect::<Vec<_>>();

        assert!(program.material_properties.is_empty());
        assert!(program.schema_expressions.is_empty());
        assert!(program.schema_evaluators.is_empty());
        assert!(names.contains(&"mask"));
        assert!(names.contains(&"remap01"));
        assert!(names.contains(&"band"));
        assert!(names.contains(&"grounded"));
        assert!(names.contains(&"erode"));
        assert!(names.contains(&"coverage"));
        assert!(names.contains(&"sign_nonzero"));
        assert!(names.contains(&"clamp01"));
        assert!(names.contains(&"color_mix"));
        assert!(names.contains(&"rgb"));
        assert!(names.contains(&"grey"));
        assert!(names.contains(&"rgb24"));
        assert!(names.contains(&"rgba32"));
        let mask_docs = program
            .functions
            .iter()
            .find(|func| func.name == "mask")
            .and_then(|func| func.docs.as_deref())
            .expect("mask docs should be attached");
        assert!(mask_docs.contains("lo"));
        assert!(mask_docs.contains("hi"));
        assert!(mask_docs.contains("x"));
    }

    #[test]
    fn prelude_exports_include_authored_helpers() {
        let exports = super::build_language_docs_model_with_prelude(crate::test_support::PRELUDE)
            .expect("fixture docs")
            .stdlib_exports;
        assert!(exports.iter().any(|export| export.name == "mask"));
        assert!(exports.iter().any(|export| export.name == "remap01"));
        assert!(exports.iter().any(|export| export.name == "band"));
        assert!(exports.iter().any(|export| export.name == "clamp01"));
        assert!(exports.iter().any(|export| export.name == "color_mix"));
        assert!(exports.iter().any(|export| export.name == "rgb"));
        assert!(exports.iter().any(|export| export.name == "grey"));
        assert!(exports.iter().any(|export| export.name == "rgb24"));
        assert!(exports.iter().any(|export| export.name == "rgba32"));
        let remap_docs = exports
            .iter()
            .find(|export| export.name == "remap01")
            .and_then(|export| export.docs.as_deref())
            .expect("remap01 docs should be exported");
        assert!(remap_docs.contains("source range minimum"));
        let mask_lo_docs = exports
            .iter()
            .find(|export| export.name == "mask")
            .and_then(|export| export.params.iter().find(|param| param.name == "lo"))
            .and_then(|param| param.docs.as_deref())
            .expect("mask param docs should be exported");
        assert!(mask_lo_docs.contains("lower threshold"));
    }
}
