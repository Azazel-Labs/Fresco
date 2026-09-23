//! Reflected executable subsets of the existing pass/pipeline language.
use crate::ast::PipelineDecl;
use fresco_artifact::{ManifestRecipeResource, ManifestRecipeStep};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn reflect(
    pipeline: &PipelineDecl,
) -> Result<(Vec<ManifestRecipeResource>, Vec<ManifestRecipeStep>), String> {
    let mut resources = Vec::new();
    let mut names = BTreeSet::new();
    for attr in &pipeline.attrs {
        if !matches!(
            attr.name.as_str(),
            "image" | "buffer" | "external" | "table_data"
        ) {
            continue;
        }
        let args = &attr.args;
        let expected = if attr.name == "table_data" { 3 } else { 2 };
        if args.len() != expected {
            return Err(format!("@{} requires {expected} arguments", attr.name));
        }
        if !names.insert(args[0].clone()) {
            return Err(format!("duplicate recipe resource `{}`", args[0]));
        }
        let mut resource = ManifestRecipeResource {
            name: args[0].clone(),
            kind: attr.name.clone(),
            format: None,
            bytes: None,
            source: None,
            table: None,
            column: None,
        };
        match attr.name.as_str() {
            "image" => {
                if fresco_artifact::types::ImageFormatRequirement::parse(&args[1]).is_none() {
                    return Err(format!("unknown attachment format `{}`", args[1]));
                }
                resource.format = Some(args[1].clone());
            }
            "buffer" => {
                let bytes = args[1]
                    .parse::<u32>()
                    .map_err(|_| "buffer size must be a u32")?;
                if bytes == 0 || bytes % 4 != 0 {
                    return Err("buffer size must be a positive multiple of four".into());
                }
                resource.bytes = Some(bytes);
            }
            "external" => resource.source = Some(args[1].clone()),
            "table_data" => {
                resource.table = Some(args[1].clone());
                resource.column = Some(args[2].clone());
            }
            _ => unreachable!("selected resource kind"),
        }
        resources.push(resource);
    }
    let mut steps = Vec::new();
    let mut nodes = BTreeSet::new();
    for (index, reference) in pipeline.pass_refs.iter().enumerate() {
        let mut step = ManifestRecipeStep {
            transparent_queue: None,
            attachments: BTreeMap::new(),
            name: reference.name.clone(),
            pass: reference.name.clone(),
            invocation: reference.invocation.as_deref().cloned(),
            domain: String::new(),
            vertex: None,
            entry: String::new(),
            bindings: BTreeMap::new(),
            colors: BTreeMap::new(),
            depth: None,
            dispatch_scale: [1; 2],
            capacity: BTreeMap::new(),
            vertex_count: 3,
            after: Vec::new(),
            condition: None,
        };
        let mut seen = BTreeSet::new();
        for attr in &reference.attrs {
            let args = &attr.args;
            if !matches!(
                attr.name.as_str(),
                "bind"
                    | "color"
                    | "after"
                    | "per_invocation"
                    | "attachment"
                    | "implementation_require"
            ) && !seen.insert(&attr.name)
            {
                return Err(format!("duplicate step attribute `@{}`", attr.name));
            }
            match attr.name.as_str() {
                "node" if args.len() == 1 => step.name.clone_from(&args[0]),
                "transparent_queue" if args.len() == 1 => {
                    step.transparent_queue = Some(args[0].clone());
                }
                "draw" if args.len() == 3 || args.len() == 4 => {
                    if !step.domain.is_empty()
                        || !matches!(args[2].as_str(), "mesh" | "instance" | "fullscreen")
                    {
                        return Err(
                            "draw requires (vertex,fragment,mesh|instance|fullscreen[,vertex_count])".into(),
                        );
                    }
                    step.vertex = Some(args[0].clone());
                    step.entry.clone_from(&args[1]);
                    step.domain.clone_from(&args[2]);
                    if args.len() == 4 {
                        step.vertex_count =
                            args[3].parse().map_err(|_| "vertex count must be u32")?;
                    }
                }
                "draw_depth" if args.len() == 2 || args.len() == 3 => {
                    if !step.domain.is_empty() {
                        return Err("step cannot both draw and dispatch".into());
                    }
                    step.vertex = Some(args[0].clone());
                    let (domain, count) = if args.len() == 3 {
                        (args[1].as_str(), &args[2])
                    } else {
                        ("fullscreen", &args[1])
                    };
                    if !matches!(domain, "mesh" | "instance" | "fullscreen") {
                        return Err("unknown depth draw domain".into());
                    }
                    step.domain = domain.into();
                    step.vertex_count = count.parse().map_err(|_| "vertex count must be u32")?;
                }
                "dispatch" if args.len() == 3 => {
                    if !step.domain.is_empty() {
                        return Err("step cannot both draw and dispatch".into());
                    }
                    step.domain = "compute".into();
                    step.entry.clone_from(&args[0]);
                    for (axis, arg) in args[1..].iter().enumerate() {
                        step.dispatch_scale[axis] =
                            arg.parse().map_err(|_| "dispatch scale must be u32")?;
                        if step.dispatch_scale[axis] == 0 {
                            return Err("dispatch scale must be positive".into());
                        }
                    }
                }
                "bind" if args.len() == 2 => {
                    if step
                        .bindings
                        .insert(args[0].clone(), args[1].clone())
                        .is_some()
                    {
                        return Err("duplicate step resource binding".into());
                    }
                }
                "color" if args.len() == 2 => {
                    let location = args[0].parse().map_err(|_| "color location must be u32")?;
                    if step.colors.insert(location, args[1].clone()).is_some() {
                        return Err("duplicate attachment location".into());
                    }
                }
                "per_invocation" if args.len() == 2 => {
                    let bytes = args[1]
                        .parse::<u32>()
                        .map_err(|_| "capacity stride must be u32")?;
                    if bytes == 0 || step.capacity.insert(args[0].clone(), bytes).is_some() {
                        return Err("invalid or duplicate capacity declaration".into());
                    }
                }
                "attachment" if args.len() == 3 => {
                    if !matches!(args[1].as_str(), "load" | "clear")
                        || !matches!(args[2].as_str(), "store" | "discard")
                    {
                        return Err(
                            "@attachment requires (resource, load|clear, store|discard)".into()
                        );
                    }
                    if step
                        .attachments
                        .insert(
                            args[0].clone(),
                            fresco_artifact::ManifestAttachmentOps {
                                load: args[1] == "load",
                                store: args[2] == "store",
                            },
                        )
                        .is_some()
                    {
                        return Err("duplicate attachment operations".into());
                    }
                }
                "implementation_require" if attr.expressions.len() == 1 => {}
                "implementation_when" if args.len() == 2 => {
                    step.condition = Some(format!("{}:{index}", pipeline.name));
                }
                "depth" if args.len() == 1 => step.depth = Some(args[0].clone()),
                "after" if args.len() == 1 => step.after.push(args[0].clone()),
                "when" if attr.expressions.len() == 1 => {
                    step.condition = Some(format!("{}:{index}", pipeline.name));
                }
                _ => return Err(format!("invalid recipe step attribute `@{}`", attr.name)),
            }
        }
        if step.domain.is_empty() {
            return Err(format!(
                "recipe step `{}` has no draw or dispatch implementation",
                step.name
            ));
        }
        if !nodes.insert(step.name.clone()) {
            return Err(format!(
                "duplicate recipe step `{}`; declare a distinct @node name",
                step.name
            ));
        }
        for name in step
            .bindings
            .values()
            .chain(step.colors.values())
            .chain(step.depth.iter())
        {
            if !names.contains(name) {
                return Err(format!(
                    "step `{}` references undefined resource `{name}`",
                    step.name
                ));
            }
        }
        for name in step.capacity.keys() {
            if step.domain != "compute"
                || !resources
                    .iter()
                    .any(|r| r.name == *name && r.kind == "buffer")
                || !step.bindings.values().any(|r| r == name)
            {
                return Err("capacity requires a bound compute buffer".into());
            }
        }
        if step.domain != "compute"
            && step.entry.is_empty()
            && (step.depth.is_none() || !step.colors.is_empty())
        {
            return Err("depth-only draw requires a depth attachment and no colors".into());
        }
        if step.domain == "compute" && (!step.colors.is_empty() || step.depth.is_some()) {
            return Err("compute steps cannot have attachments".into());
        }
        for name in step.attachments.keys() {
            if !step
                .colors
                .values()
                .chain(step.depth.iter())
                .any(|attachment| attachment == name)
            {
                return Err(format!(
                    "attachment operations reference non-attachment `{name}`"
                ));
            }
        }
        steps.push(step);
    }
    if steps.is_empty() {
        return Err("renderer recipe has no executable steps".into());
    }
    let mut ordered = Vec::new();
    let mut done = BTreeSet::new();
    while ordered.len() < steps.len() {
        let Some(step) = steps.iter().find(|step| {
            !done.contains(&step.name) && step.after.iter().all(|name| done.contains(name))
        }) else {
            return Err("recipe has a cycle or an undefined dependency".into());
        };
        done.insert(step.name.clone());
        ordered.push(step.clone());
    }
    fresco_artifact::validate_transparent_queues(&ordered)?;
    Ok((resources, ordered))
}

/// Check executable references against the same entries used to emit WGSL.
pub(super) fn validate<D, N>(root: &fresco_artifact::ManifestRoot<D, N>) -> Result<(), String> {
    if !root.canvases.is_empty() && root.surfaces.is_empty() {
        return Ok(());
    }
    for recipe in root.renderers.iter().filter(|r| r.selected) {
        fresco_artifact::validate_transparent_queues(&recipe.steps)?;
        let resources: BTreeMap<_, _> = recipe
            .resources
            .iter()
            .map(|r| (r.name.as_str(), r))
            .collect();
        for resource in &recipe.resources {
            if resource.kind == "table_data" {
                let table = root
                    .tables
                    .iter()
                    .find(|t| Some(&t.name) == resource.table.as_ref())
                    .ok_or("recipe references an undefined table")?;
                if !table
                    .fields
                    .iter()
                    .any(|f| Some(f) == resource.column.as_ref())
                {
                    return Err("recipe references an undefined table column".into());
                }
            }
        }
        for step in &recipe.steps {
            let mut programs = Vec::new();
            if step.is_draw_scoped() {
                for surface in &root.surfaces {
                    if let Some(condition) = &step.condition
                        && !surface
                            .settings
                            .as_ref()
                            .and_then(|s| s.recipe_conditions.get(condition))
                            .copied()
                            .ok_or("unresolved recipe condition")?
                    {
                        continue;
                    }
                    if !surface.mesh_passes.is_empty() {
                        if step.transparent_queue.is_some() {
                            surface
                                .settings
                                .as_ref()
                                .and_then(|settings| settings.pass_states.get(&step.pass))
                                .ok_or("transparent queue requires explicit raster state")?
                                .validate_transparent_queue()?;
                        }
                        let Some(pass) = surface.mesh_passes.iter().find(|p| p.pass == step.pass)
                        else {
                            return Err(format!(
                                "recipe references unavailable mesh pass `{}`",
                                step.pass
                            ));
                        };
                        if pass.procedural != (step.domain == "instance") {
                            return Err(format!(
                                "recipe draw domain `{}` does not match pass `{}` vertex source",
                                step.domain, step.pass
                            ));
                        }
                        programs.push(pass.entries.as_slice());
                        programs.extend(pass.variants.iter().map(|v| v.entries.as_slice()));
                    }
                }
            } else {
                let program = root
                    .gpu_programs
                    .iter()
                    .find(|p| p.pass == step.pass)
                    .ok_or_else(|| {
                        format!("recipe step `{}` has no GPU implementation", step.name)
                    })?;
                programs.push(program.entries.as_slice());
                if program.bindings.len() != step.bindings.len() {
                    return Err(format!("incomplete resource wiring for `{}`", step.name));
                }
                for binding in &program.bindings {
                    let name = step
                        .bindings
                        .get(&binding.name)
                        .ok_or_else(|| format!("missing resource binding `{}`", binding.name))?;
                    let resource = resources
                        .get(name.as_str())
                        .ok_or("undefined recipe resource")?;
                    if step.colors.values().any(|output| output == name)
                        || step.depth.as_ref() == Some(name)
                    {
                        return Err("a step cannot sample its active attachment".into());
                    }
                    if binding.kind == "texture" && resource.kind != "image" {
                        return Err("texture binding requires an image resource".into());
                    }
                    if binding.kind == "texture" {
                        let expected = resource
                            .format
                            .as_deref()
                            .and_then(fresco_artifact::types::ImageFormatRequirement::parse)
                            .and_then(
                                fresco_artifact::types::ImageFormatRequirement::sampled_shader_type,
                            )
                            .ok_or("texture binding requires a sampleable image format")?;
                        if binding.ty != expected {
                            return Err("texture type and image format are incompatible".into());
                        }
                    }
                    if matches!(resource.source.as_deref(), Some("presentation" | "depth")) {
                        return Err("GPU buffer binding cannot use an external attachment".into());
                    }
                    if binding.kind != "texture" && resource.kind == "image" {
                        return Err("buffer binding cannot use an image resource".into());
                    }
                }
            }
            for entries in programs {
                let stage = if step.domain == "compute" {
                    "compute"
                } else {
                    "fragment"
                };
                if step.entry.is_empty() && step.domain != "compute" {
                    if !entries
                        .iter()
                        .any(|e| Some(&e.function) == step.vertex.as_ref() && e.stage == "vertex")
                    {
                        return Err("recipe vertex function missing".into());
                    }
                    let depth = resources
                        .get(step.depth.as_deref().ok_or("depth attachment missing")?)
                        .ok_or("undefined depth attachment")?;
                    if depth.format.as_deref() != Some("depth32float")
                        && depth.source.as_deref() != Some("depth")
                    {
                        return Err("depth attachment requires depth32float".into());
                    }
                    continue;
                }
                let entry = entries
                    .iter()
                    .find(|e| e.function == step.entry && e.stage == stage)
                    .ok_or_else(|| {
                        format!(
                            "recipe references unknown {stage} function `{}`",
                            step.entry
                        )
                    })?;
                if stage == "fragment" {
                    if !entries
                        .iter()
                        .any(|e| Some(&e.function) == step.vertex.as_ref() && e.stage == "vertex")
                    {
                        return Err("recipe vertex function missing".into());
                    }
                    if entry.outputs.len() != step.colors.len() {
                        return Err("fragment outputs and recipe attachments disagree".into());
                    }
                    for output in &entry.outputs {
                        let name = step.colors.get(&output.location).ok_or_else(|| {
                            format!(
                                "fragment output location {} has no recipe attachment",
                                output.location
                            )
                        })?;
                        let resource =
                            resources.get(name.as_str()).ok_or("undefined attachment")?;
                        let format = resource
                            .format
                            .as_deref()
                            .or_else(|| {
                                (resource.source.as_deref() == Some("presentation"))
                                    .then_some("presentation")
                            })
                            .ok_or("color attachment is not an image")?;
                        let matches = if format == "presentation" {
                            fresco_artifact::types::ImageScalar::F32
                                .accepts_shader_output(&output.ty)
                        } else {
                            fresco_artifact::types::ImageFormatRequirement::parse(format)
                                .is_some_and(|f| f.accepts_shader_output(&output.ty))
                        };
                        if !matches {
                            return Err(
                                "fragment output and attachment format are incompatible".into()
                            );
                        }
                    }
                }
            }
        }
        fn depends(
            steps: &[ManifestRecipeStep],
            current: &str,
            prior: &str,
            seen: &mut BTreeSet<String>,
        ) -> bool {
            if !seen.insert(current.into()) {
                return false;
            }
            steps.iter().find(|s| s.name == current).is_some_and(|s| {
                s.after
                    .iter()
                    .any(|name| name == prior || depends(steps, name, prior, seen))
            })
        }
        // Validate dataflow independently for each set of compile-time route conditions.
        for surface in &root.surfaces {
            let mut written = BTreeSet::new();
            let mut writers: BTreeMap<String, &ManifestRecipeStep> = BTreeMap::new();
            for step in &recipe.steps {
                if let Some(condition) = &step.condition
                    && !surface
                        .settings
                        .as_ref()
                        .and_then(|s| s.recipe_conditions.get(condition))
                        .copied()
                        .ok_or("unresolved recipe condition")?
                {
                    continue;
                }
                if let Some(program) = root.gpu_programs.iter().find(|p| p.pass == step.pass) {
                    for binding in &program.bindings {
                        let name = &step.bindings[&binding.name];
                        let resource = resources[name.as_str()];
                        if binding.access == "read"
                            && matches!(resource.kind.as_str(), "image" | "buffer")
                            && !written.contains(name)
                        {
                            return Err(format!("recipe reads undefined resource `{name}`"));
                        }
                    }
                    for binding in program
                        .bindings
                        .iter()
                        .filter(|b| matches!(b.access.as_str(), "write" | "read_write"))
                    {
                        written.insert(step.bindings[&binding.name].clone());
                    }
                }
                for name in step.colors.values().chain(step.depth.iter()) {
                    let ops = step.attachments.get(name);
                    if let Some(previous) = writers.get(name)
                        && !(step.transparent_queue.is_some()
                            && step.transparent_queue == previous.transparent_queue)
                        && !depends(
                            &recipe.steps,
                            &step.name,
                            &previous.name,
                            &mut BTreeSet::new(),
                        )
                    {
                        return Err(format!("attachment `{name}` has unordered writes"));
                    }
                    writers.insert(name.clone(), step);
                    if ops.is_some_and(|ops| ops.load) && !written.contains(name) {
                        return Err(format!("attachment `{name}` loads undefined contents"));
                    }
                    if written.contains(name) && ops.is_none() {
                        return Err(format!(
                            "conflicting writes to `{name}` require explicit attachment operations"
                        ));
                    }
                    if ops.is_none_or(|ops| ops.store) {
                        written.insert(name.clone());
                    } else {
                        written.remove(name);
                    }
                }
            }
        }
    }
    Ok(())
}
