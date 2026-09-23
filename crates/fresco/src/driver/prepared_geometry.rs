//! Engine-owned mesh preparation and resource aggregate contracts.
use crate::{ast::*, diag::Diag};
use std::collections::BTreeSet;

pub(super) fn collect(program: &mut Program) -> Result<(), Vec<Diag>> {
    let mut values = Vec::new();
    for declaration in std::mem::take(&mut program.structs) {
        if declaration.attrs.iter().any(|a| a.name == "resource") {
            program.resource_types.push(declaration);
        } else {
            values.push(declaration);
        }
    }
    program.structs = values;
    let mut names: BTreeSet<_> = program.structs.iter().map(|s| s.name.as_str()).collect();
    for resource in &program.resource_types {
        let fail = |message: String| vec![Diag::error(resource.span.clone(), message)];
        if !names.insert(&resource.name) {
            return Err(fail(format!("duplicate resource type `{}`", resource.name)));
        }
        let mut fields = BTreeSet::new();
        for field in &resource.fields {
            if !fields.insert(&field.name) || field.default.is_some() {
                return Err(fail(
                    "resource fields must be unique and have no value defaults".into(),
                ));
            }
        }
        let roles=resource.attrs.iter().find(|a|a.name=="geometry").ok_or_else(||fail("resource aggregates currently require @geometry(vertices, indices, vertex_count, index_count, bounds)".into()))?;
        if resource.fields.len() != 5
            || roles.args.len() != 5
            || roles.args.iter().collect::<BTreeSet<_>>().len() != 5
            || roles.args.iter().any(|name| !fields.contains(name))
        {
            return Err(fail(
                "geometry resource requires five distinct declared field roles".into(),
            ));
        }
        let field = |index: usize| {
            resource
                .fields
                .iter()
                .find(|f| f.name == roles.args[index])
                .expect("checked role")
        };
        let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let vertex = compact(&field(0).ty_name);
        let element = vertex
            .strip_prefix("buffer<")
            .and_then(|s| s.strip_suffix(",read>"));
        if !element.is_some_and(|element| program.structs.iter().any(|s| s.name == element)) {
            return Err(fail(
                "geometry vertices require buffer<Record, read>".into(),
            ));
        }
        if compact(&field(1).ty_name) != "buffer<u32,read>"
            || field(2).ty_name != "u32"
            || field(3).ty_name != "u32"
        {
            return Err(fail(
                "geometry requires read-only u32 indices and u32 vertex/index counts".into(),
            ));
        }
        if !program.structs.iter().any(|s| s.name == field(4).ty_name) {
            return Err(fail(
                "geometry bounds require a declared record type".into(),
            ));
        }
    }
    if !program.resource_types.is_empty() {
        if program
            .structs
            .iter()
            .any(|s| s.name == "__FrescoGeometryCounts")
        {
            return Err(vec![Diag::error(0..0, "reserved geometry metadata type")]);
        }
        program.structs.push(StructDecl {
            name: "__FrescoGeometryCounts".into(),
            name_span: 0..0,
            span: 0..0,
            attrs: vec![],
            fields: ["vertex_count", "index_count", "first", "indexed"]
                .into_iter()
                .map(|name| StructFieldDecl {
                    name: name.into(),
                    name_span: 0..0,
                    ty_name: "u32".into(),
                    ty_span: 0..0,
                    span: 0..0,
                    attrs: vec![],
                    semantic: None,
                    default: None,
                })
                .collect(),
        });
    }
    if !program.resource_types.is_empty() {
        if program
            .structs
            .iter()
            .any(|s| s.name == "__FrescoGeometryBounds")
        {
            return Err(vec![Diag::error(
                0..0,
                "reserved geometry bounds metadata type",
            )]);
        }
        program.structs.push(StructDecl {
            name: "__FrescoGeometryBounds".into(),
            name_span: 0..0,
            span: 0..0,
            attrs: vec![],
            fields: ["minimum", "maximum"]
                .into_iter()
                .map(|name| StructFieldDecl {
                    name: name.into(),
                    name_span: 0..0,
                    ty_name: "vec4".into(),
                    ty_span: 0..0,
                    span: 0..0,
                    attrs: vec![],
                    semantic: None,
                    default: None,
                })
                .collect(),
        });
    }
    for pass in &mut program.passes {
        if pass
            .bindings
            .iter()
            .any(|b| b.attrs.iter().any(|a| a.name == "geometry_resource"))
        {
            return Err(vec![Diag::error(
                pass.span.clone(),
                "geometry_resource is reserved for compiler-owned bindings",
            )]);
        }
        let preparations: Vec<_> = pass
            .hooks
            .iter()
            .filter(|h| h.attrs.iter().any(|a| a.name == "prepare"))
            .collect();
        if preparations.len() > 1 {
            return Err(vec![Diag::error(
                pass.span.clone(),
                "mesh pass accepts one preparation function",
            )]);
        }
        if let Some(hook) = preparations.first() {
            let resource = hook
                .attrs
                .iter()
                .find(|a| a.name == "prepare")
                .expect("preparation attribute");
            let [name] = resource.args.as_slice() else {
                return Err(vec![Diag::error(
                    resource.span.clone(),
                    "@prepare requires one geometry resource type",
                )]);
            };
            let Some(resource) = program.resource_types.iter().find(|r| r.name == *name) else {
                return Err(vec![Diag::error(
                    hook.span.clone(),
                    "unknown preparation resource type",
                )]);
            };
            if hook.params.len() != 1
                || !program
                    .vertex_interfaces
                    .iter()
                    .any(|i| i.name == hook.params[0].ty_name)
                || hook
                    .return_ty
                    .as_ref()
                    .is_none_or(|t| t.node != element(resource))
            {
                return Err(vec![Diag::error(
                    hook.span.clone(),
                    "preparation requires one declared vertex interface and the geometry vertex result",
                )]);
            }
            pass.vertex_interface = Some(Spanned {
                node: hook.params[0].ty_name.clone(),
                span: hook.params[0].name_span.clone(),
            });
        }
    }
    Ok(())
}

pub(super) fn producer<'a>(
    program: &'a Program,
    steps: &[fresco_artifact::ManifestRecipeStep],
    endpoint: &str,
    resource: &str,
    factory: Option<&str>,
) -> Result<(&'a PassDecl, &'a PassFnHookDecl), String> {
    let (node, hook) = endpoint
        .split_once('.')
        .ok_or("prepared geometry must name a recipe node's preparation function")?;
    let step = steps
        .iter()
        .find(|s| s.name == node)
        .ok_or_else(|| format!("unknown geometry producer node `{node}`"))?;
    if step.domain != "mesh" {
        return Err("geometry preparation requires a mesh producer".into());
    }
    let pass = program
        .passes
        .iter()
        .find(|p| p.name == step.pass)
        .ok_or("unknown geometry producer pass")?;
    let declaration = pass
        .hooks
        .iter()
        .find(|h| {
            h.name == hook
                && h.attrs
                    .iter()
                    .any(|a| a.name == "prepare" && a.args == [resource])
        })
        .ok_or_else(|| format!("`{endpoint}` does not prepare `{resource}`"))?;
    let resource = program
        .resource_types
        .iter()
        .find(|r| r.name == resource)
        .ok_or("unknown geometry resource type")?;
    let roles = resource
        .attrs
        .iter()
        .find(|a| a.name == "geometry")
        .expect("checked resource roles");
    let vertices = resource
        .fields
        .iter()
        .find(|f| f.name == roles.args[0])
        .expect("checked vertices");
    let ty: String = vertices
        .ty_name
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let element = ty
        .strip_prefix("buffer<")
        .and_then(|s| s.strip_suffix(",read>"))
        .expect("checked vertex stream");
    if declaration.params.len() != 1
        || declaration
            .return_ty
            .as_ref()
            .is_none_or(|t| t.node != element)
    {
        return Err("preparation function must accept one raw vertex and return the resource's vertex record".into());
    }
    let input = &declaration.params[0].ty_name;
    if !program.vertex_interfaces.iter().any(|i| i.name == *input) {
        return Err("preparation input must be a declared vertex interface".into());
    }
    let declared = pass
        .attrs
        .iter()
        .find(|a| a.name == "factory")
        .and_then(|a| a.args.first())
        .ok_or("preparation pass must declare its vertex factory")?;
    if factory.is_some_and(|f| f != declared) {
        return Err(format!(
            "preparation `{endpoint}` is unavailable for vertex factory `{}`",
            factory.expect("selected factory")
        ));
    }
    Ok((pass, declaration))
}

pub(super) fn member<'a>(
    program: &'a Program,
    contract: &'a StyleContractDecl,
    name: &str,
) -> Result<Option<(&'a StyleCapabilityDecl, &'a StyleInputDecl)>, String> {
    let mut found = None;
    for reference in &contract.capabilities {
        let capability = program
            .style_capabilities
            .iter()
            .find(|c| c.name == reference.name)
            .ok_or("unknown capability")?;
        if let Some(member) = capability.members.iter().find(|m| m.name == name) {
            if found.is_some() {
                return Err(format!("ambiguous capability input `{name}`"));
            }
            found = Some((capability, member));
        }
    }
    Ok(found)
}

pub(super) fn roles(resource: &StructDecl) -> [&StructFieldDecl; 5] {
    let roles = resource
        .attrs
        .iter()
        .find(|a| a.name == "geometry")
        .expect("validated geometry resource");
    std::array::from_fn(|i| {
        resource
            .fields
            .iter()
            .find(|f| f.name == roles.args[i])
            .expect("validated geometry field")
    })
}
pub(super) fn element(resource: &StructDecl) -> String {
    let ty: String = roles(resource)[0]
        .ty_name
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    ty.strip_prefix("buffer<")
        .and_then(|s| s.strip_suffix(",read>"))
        .expect("validated geometry element")
        .into()
}

pub(super) fn select_draw(
    program: &Program,
    style: &StyleDecl,
    operation: &StyleOperation,
    args: &[Arg],
) -> Result<Option<Box<PreparedDraw>>, Vec<Diag>> {
    let parameters: Vec<_> = operation
        .inputs
        .iter()
        .filter(|p| program.resource_types.iter().any(|r| r.name == p.ty))
        .collect();
    let Some(parameter) = parameters.first() else {
        return Ok(None);
    };
    if parameters.len() != 1 {
        return Err(vec![Diag::error(
            parameter.span.clone(),
            "a draw requires exactly one geometry resource",
        )]);
    }
    select_resource(program, style, parameter, args).map(Some)
}

pub(super) fn select_resource(
    program: &Program,
    style: &StyleDecl,
    parameter: &StyleInputDecl,
    args: &[Arg],
) -> Result<Box<PreparedDraw>, Vec<Diag>> {
    let check = || -> Result<Box<PreparedDraw>, String> {
        let resource = program
            .resource_types
            .iter()
            .find(|r| r.name == parameter.ty)
            .expect("resource input");
        let value = &args
            .iter()
            .find(|a| a.name.as_ref() == Some(&parameter.name))
            .ok_or("missing geometry argument")?
            .value;
        let name = super::style_graph::path(value)?;
        let contract = program
            .style_contracts
            .iter()
            .find(|c| c.name == style.contract)
            .expect("style contract");
        let (capability, _) = member(program, contract, &name)?
            .ok_or("prepared geometry must be supplied by an engine capability")?;
        let renderer = program
            .pipelines
            .iter()
            .find(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"))
            .ok_or("prepared geometry requires a selected renderer")?;
        let provider = program
            .style_providers
            .iter()
            .find(|p| p.contract == style.contract && p.renderer == renderer.name)
            .ok_or("missing geometry provider")?;
        let binding = provider
            .blocks
            .iter()
            .find(|b| b.name == capability.name)
            .and_then(|b| b.fields.iter().find(|f| f.name == name))
            .ok_or("missing geometry capability binding")?;
        let [value] = binding.values.as_slice() else {
            return Err("geometry capability requires one producer".into());
        };
        let endpoint = super::style_graph::path(value)?;
        let (_, steps) = super::recipes::reflect(renderer)?;
        let (pass, hook) = producer(program, &steps, &endpoint, &resource.name, None)?;
        Ok(Box::new(PreparedDraw {
            producer_pass: pass.name.clone(),
            producer_hook: hook.name.clone(),
            producer_node: endpoint
                .split_once('.')
                .expect("validated endpoint")
                .0
                .into(),
            resource: resource.clone(),
            parameter: parameter.name.clone(),
        }))
    };
    check().map_err(|m| vec![Diag::error(parameter.span.clone(), m)])
}

fn geometry_binding(
    program: &Program,
    pass: &mut PassDecl,
    name: &str,
    ty: &str,
    producer: &str,
    role: &str,
) -> Result<(), Vec<Diag>> {
    super::style_operations::ensure_binding(pass, program, name, ty)?;
    let binding = pass
        .bindings
        .iter_mut()
        .find(|b| b.name == name)
        .ok_or_else(|| {
            vec![Diag::error(
                pass.span.clone(),
                "geometry binding conflicts with a factory resource",
            )]
        })?;
    binding.attrs.push(PipelineAttribute {
        name: "geometry_resource".into(),
        args: vec![producer.into(), role.into()],
        expressions: vec![],
        name_span: pass.span.clone(),
        args_span: None,
        span: pass.span.clone(),
    });
    Ok(())
}

pub(super) fn bind_draw(program: &Program, pass: &mut PassDecl) -> Result<String, Vec<Diag>> {
    let Some(draw) = pass.prepared_draw.clone() else {
        return Ok(String::new());
    };
    let fields = roles(&draw.resource);
    let vertices = format!("__geometry_{}_vertices", draw.parameter);
    let indices = format!("__geometry_{}_indices", draw.parameter);
    let counts = format!("__geometry_{}_counts", draw.parameter);
    let bounds = format!("fresco_geometry_{}_bounds", draw.parameter);
    geometry_binding(
        program,
        pass,
        &vertices,
        &format!("buffer<{}>", element(&draw.resource)),
        &draw.producer_pass,
        "vertices",
    )?;
    geometry_binding(
        program,
        pass,
        &indices,
        "buffer<u32>",
        &draw.producer_pass,
        "indices",
    )?;
    geometry_binding(
        program,
        pass,
        &counts,
        "uniform<__FrescoGeometryCounts>",
        &draw.producer_pass,
        "counts",
    )?;
    let replacements = [
        vertices,
        indices,
        format!("{counts}.vertex_count"),
        format!("{counts}.index_count"),
        bounds.clone(),
    ];
    let mut uses_bounds = false;
    for hook in &mut pass.hooks {
        let mut rewritten = Vec::new();
        let mut cursor = 0;
        while cursor < hook.body.len() {
            let tail = &hook.body[cursor..];
            if let [
                (crate::lexer::Token::Ident(root), _),
                (crate::lexer::Token::Dot, _),
                (crate::lexer::Token::Ident(member), span),
                ..,
            ] = tail
                && root == &draw.parameter
            {
                let role = fields
                    .iter()
                    .position(|f| f.name == *member)
                    .ok_or_else(|| {
                        vec![Diag::error(
                            span.clone(),
                            "unknown geometry resource member",
                        )]
                    })?;
                uses_bounds |= role == 4;
                rewritten.extend(
                    crate::lexer::lex_spanned(&replacements[role])
                        .into_iter()
                        .map(|(token, _)| (token, span.clone())),
                );
                cursor += 3;
            } else {
                rewritten.push(hook.body[cursor].clone());
                cursor += 1;
            }
        }
        hook.body = rewritten;
        if hook.attrs.iter().any(|a| a.name == "vertex") {
            for parameter in &mut hook.params {
                if parameter.attrs.is_empty() {
                    parameter.attrs.push(PipelineAttribute {
                        name: "builtin".into(),
                        args: vec!["vertex_index".into()],
                        expressions: vec![],
                        name_span: parameter.name_span.clone(),
                        args_span: None,
                        span: parameter.name_span.clone(),
                    });
                }
            }
        }
    }
    if uses_bounds {
        let record = program
            .structs
            .iter()
            .find(|s| s.name == fields[4].ty_name)
            .expect("geometry bounds type");
        if record.fields.len() != 3
            || record.fields[0].ty_name != "vec3"
            || record.fields[1].ty_name != "vec3"
            || record.fields[2].ty_name != "bool"
        {
            return Err(vec![Diag::error(
                record.span.clone(),
                "geometry bounds require minimum/maximum vec3 followed by a valid bool",
            )]);
        }
        let binding = format!("__geometry_{}_bounds", draw.parameter);
        geometry_binding(
            program,
            pass,
            &binding,
            "uniform<__FrescoGeometryBounds>",
            &draw.producer_pass,
            "bounds",
        )?;
        Ok(format!(
            "let {bounds}: {} = {}({binding}.minimum.xyz,{binding}.maximum.xyz,{binding}.minimum.w != 0.0);\n",
            record.name, record.name
        ))
    } else {
        Ok(String::new())
    }
}

pub(super) fn install(program: &mut Program) -> Result<(), Vec<Diag>> {
    let mut draws: Vec<_> = program
        .passes
        .iter()
        .filter_map(|p| p.prepared_draw.clone())
        .collect();
    for invocation in program
        .passes
        .iter()
        .filter_map(|p| p.compute_invocation.as_ref())
    {
        for (parameter, argument) in &invocation.arguments {
            if let fresco_artifact::ManifestComputeArgument::Geometry {
                producer,
                hook,
                node,
                resource_type,
            } = argument
            {
                let resource = program
                    .resource_types
                    .iter()
                    .find(|r| &r.name == resource_type)
                    .expect("validated compute geometry");
                draws.push(Box::new(PreparedDraw {
                    producer_pass: producer.clone(),
                    producer_hook: hook.clone(),
                    producer_node: node.clone(),
                    resource: resource.clone(),
                    parameter: parameter.clone(),
                }));
            }
        }
    }
    for draw in draws {
        let index = program
            .passes
            .iter()
            .position(|p| p.name == draw.producer_pass)
            .expect("validated preparation pass");
        if let Some(existing) = &program.passes[index].preparation {
            if existing.producer_hook != draw.producer_hook
                || existing.resource.name != draw.resource.name
            {
                return Err(vec![Diag::error(
                    program.passes[index].span.clone(),
                    "one mesh pass cannot supply incompatible geometry preparations",
                )]);
            }
            continue;
        }
        let mut pass = program.passes[index].clone();
        for (role, ty) in [
            ("raw_vertices", "buffer<u32>".to_string()),
            ("raw_indices", "buffer<u32>".to_string()),
            ("counts", "uniform<__FrescoGeometryCounts>".to_string()),
            ("vertices", format!("buffer<{}>", element(&draw.resource))),
            ("output", format!("buffer<{}>", element(&draw.resource))),
        ] {
            geometry_binding(
                program,
                &mut pass,
                &format!("__prepare_{role}"),
                &ty,
                &draw.producer_pass,
                role,
            )?;
        }
        pass.preparation = Some(draw);
        program.passes[index] = pass;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    const RESOURCE: &str = r#"
struct PreparedVertex { world_position: vec3; geometric_normal: vec3 }
struct Bounds { minimum: vec3; maximum: vec3; valid: bool }
@geometry(vertices, indices, vertex_count, index_count, bounds)
resource Geometry {
 vertices: buffer<PreparedVertex, read>
 indices: buffer<u32, read>
 vertex_count: u32
 index_count: u32
 bounds: Bounds
}
draw Outline(geometry: Geometry, ink: color, target: attachment<rgba16float,preserve_update>) {
 raster geometry
 visibility: uncullable
 attachments { target: load_store }
 @vertex fn vertex(index: u32) -> clip_position { return vec4(geometry.vertices[index].world_position,1.0) }
 @fragment fn fragment() -> vec4 { if geometry.bounds.valid { return ink * 0.5 }; return ink }
}
"#;
    #[test]
    fn prepared_resource_draw_resolves_engine_producer() {
        let original = include_str!("../../tests/fixtures/engines/operation-engine.fr");
        let engine = original
            .replace("interface Action {", "contract Action for Value { optional capability GeometryAccess; point finish: Target { scope: view; accepts: raster_draws; composition: ordered_draws(engine.stable_draw_order); after: complete; before: presentation }; ")
            .replace("@implementation(Action) struct Plain {}\nconform Plain : Action", "style Plain for Value : Action")
            .replace("@implementation(Action) struct Extra {}\nconform Extra : Action", "style Extra for Value : Action")
            .replace("style Extra for Value : Action {", "style Extra for Value : Action { for self { at finish as target { Outline(geometry: mesh.prepared, ink: #000, target: target.target) } }; ")
            .replace("@external(result, presentation)", "@image(result, rgba16float)")
            .replace("@draw(project, paint, mesh) @color(0, result) first", "@draw(project, paint, mesh) @color(0, result) first\n @node(display) @after(first) @draw(project, paint, mesh) @color(0, result) @attachment(result, load, store) first")
            .replace("@vertex fn project(v: Corner) -> Projected { return Projected(vec4(v.point, 1.0)) }", "@prepare(Geometry) fn prepare(v: Corner) -> PreparedVertex { return PreparedVertex(v.point, vec3(0.0, 0.0, 1.0)) }\n @vertex fn project(v: PreparedVertex) -> Projected { return Projected(vec4(v.world_position, 1.0)) }");
        let engine = format!(
            "{engine}\nstruct Target {{ target: attachment<rgba16float, preserve_update> }}\ncapability GeometryAccess {{ mesh.prepared: Geometry }}\nprovide Action for main_plan {{ GeometryAccess {{ mesh.prepared: first.prepare; factories: plain }}; finish {{ complete: all(first); presentation: all(display); target: result; order: stable_draw_order }} }}"
        );
        let files = std::collections::HashMap::from([
            ("engine/engine.fr".into(), engine),
            (
                "main.fr".into(),
                format!(
                    "{RESOURCE}\nsurface item(sp: SamplePoint) -> material(Value) {{ properties {{ action: Extra }}; compose {{ Value(tint:#fff) }} }}"
                ),
            ),
        ]);
        let compiled = super::super::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let module = naga::front::wgsl::parse_str(&compiled.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&compiled.manifest).unwrap();
        let surface = &manifest.surfaces[0];
        let producer = surface
            .mesh_passes
            .iter()
            .find(|p| p.pass == "first")
            .unwrap();
        let preparation = producer.preparation.as_ref().unwrap();
        assert_eq!(preparation.node, "first");
        assert!(
            module
                .entry_points
                .iter()
                .any(|e| e.stage == naga::ShaderStage::Compute && e.name == preparation.entry)
        );
        for entry in &producer.entries {
            if entry.stage == "vertex" {
                let entry = module
                    .entry_points
                    .iter()
                    .find(|e| e.name == entry.entry)
                    .unwrap();
                assert!(
                    entry
                        .function
                        .arguments
                        .iter()
                        .all(|a| a.binding
                            == Some(naga::Binding::BuiltIn(naga::BuiltIn::VertexIndex)))
                );
            }
        }
        let consumer = surface
            .mesh_passes
            .iter()
            .find(|p| p.prepared_source.as_deref() == Some("first"))
            .unwrap();
        assert!(consumer.bindings.iter().any(|b| {
            b.geometry
                .as_ref()
                .is_some_and(|g| g.producer == "first" && g.role == "vertices")
        }));
        let mut disabled = files.clone();
        let source = disabled.get_mut("main.fr").unwrap();
        *source = source.replace("action: Extra", "action: Plain");
        let output = super::super::compile_bundle_virtual(&disabled, "main.fr", false).unwrap();
        let manifest: fresco_artifact::ManifestRoot =
            serde_json::from_str(&output.manifest).unwrap();
        assert!(
            manifest.surfaces[0]
                .mesh_passes
                .iter()
                .all(|p| p.preparation.is_none() && p.prepared_source.is_none())
        );
        for (from, to, expected) in [
            ("first.prepare", "first.absent", "does not prepare"),
            (
                "first.prepare",
                "absent.prepare",
                "unknown geometry producer",
            ),
            (
                "factories: plain",
                "factories: absent",
                "unknown capability factory",
            ),
        ] {
            let mut broken = files.clone();
            let engine = broken.get_mut("engine/engine.fr").unwrap();
            *engine = engine.replace(from, to);
            let errors =
                super::super::compile_bundle_virtual(&broken, "main.fr", false).unwrap_err();
            assert!(format!("{errors:?}").contains(expected), "{errors:?}");
        }
    }
    #[test]
    fn preparation_hook_is_inlined_without_geometry_demand() {
        let engine = include_str!("../../tests/fixtures/engines/operation-engine.fr")
            .replace("@vertex fn project(v: Corner) -> Projected { return Projected(vec4(v.point, 1.0)) }", "@prepare(Geometry) fn prepare(v: Corner) -> PreparedVertex { return PreparedVertex(v.point, vec3(0.0, 0.0, 1.0)) }\n @vertex fn project(v: PreparedVertex) -> Projected { return Projected(vec4(v.world_position, 1.0)) }");
        let files = std::collections::HashMap::from([
            ("engine/engine.fr".into(), engine),
            (
                "main.fr".into(),
                format!(
                    "{RESOURCE}\nsurface item(sp: SamplePoint) -> material(Value) {{ compose {{ Value(tint:#fff) }} }}"
                ),
            ),
        ]);
        let compiled = super::super::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let module = naga::front::wgsl::parse_str(&compiled.wgsl).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .unwrap();
        assert!(
            !module
                .entry_points
                .iter()
                .any(|e| e.stage == naga::ShaderStage::Compute)
        );
        assert!(compiled.wgsl.contains("_prepare("));
    }
    #[test]
    fn prepared_resource_and_unused_indexed_draw_are_typed_without_work() {
        let files = std::collections::HashMap::from([
            (
                "engine/engine.fr".into(),
                include_str!("../../tests/fixtures/engines/operation-engine.fr").to_string(),
            ),
            (
                "main.fr".into(),
                format!(
                    "{RESOURCE}\nsurface item(sp: SamplePoint) -> material(Value) {{ compose {{ Value(tint:#fff) }} }}"
                ),
            ),
        ]);
        let compiled = super::super::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        assert!(!compiled.wgsl.contains("Outline"));
        let mut rejected = files;
        let source = rejected.get_mut("main.fr").unwrap();
        *source = source.replace("buffer<u32, read>", "buffer<i32, read>");
        let error = super::super::compile_bundle_virtual(&rejected, "main.fr", false).unwrap_err();
        assert!(format!("{error:?}").contains("u32 indices"), "{error:?}");
    }
}
