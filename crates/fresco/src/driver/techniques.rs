//! Reusable executable pipelines independent of renderer and material selection.
use crate::ast::PipelineDecl;
use fresco_artifact::{
    ManifestDispatchExtent, ManifestDrawCount, ManifestGpuProgram, ManifestImageExtent,
    ManifestResourceSource, ManifestResourceType, ManifestTechnique, ManifestTechniqueOperation,
    ManifestTechniqueResource, ManifestTechniqueStep,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn is_technique(pipeline: &PipelineDecl) -> bool {
    pipeline.attrs.iter().any(|a| a.name == "technique")
}

pub(super) fn reflect(
    pipelines: &[PipelineDecl],
    programs: &[ManifestGpuProgram],
) -> Result<Vec<ManifestTechnique>, String> {
    let mut techniques = Vec::new();
    for pipeline in pipelines.iter().filter(|p| is_technique(p)) {
        let surfaces: BTreeSet<_> = programs
            .iter()
            .filter(|p| pipeline.pass_refs.iter().any(|r| r.name == p.pass))
            .filter_map(|p| p.surface.as_deref())
            .collect();
        if surfaces.is_empty() {
            techniques.push(build(pipeline, programs)?);
        } else {
            for surface in surfaces {
                let selected: Vec<_> = programs
                    .iter()
                    .filter(|p| p.surface.as_deref().is_none_or(|s| s == surface))
                    .cloned()
                    .collect();
                let mut technique = build(pipeline, &selected)?;
                technique.name = format!("{}:{surface}", technique.name);
                technique.surface = Some(surface.into());
                techniques.push(technique);
            }
        }
    }
    validate_imports(&techniques)?;
    Ok(techniques)
}

fn build(
    pipeline: &PipelineDecl,
    programs: &[ManifestGpuProgram],
) -> Result<ManifestTechnique, String> {
    let marker: Vec<_> = pipeline
        .attrs
        .iter()
        .filter(|a| a.name == "technique")
        .collect();
    if marker.len() != 1 || !marker[0].args.is_empty() {
        return Err("requires one argument-free @technique".into());
    }
    if pipeline.attrs.iter().any(|a| a.name == "renderer") {
        return Err("techniques are independent of material and renderer selection".into());
    }
    let mut recipe = pipeline.clone();
    let mut sources = BTreeMap::new();
    let mut pools = BTreeMap::new();
    let mut outputs = BTreeMap::new();
    let mut dimensions = BTreeMap::new();
    for attr in &mut recipe.attrs {
        match attr.name.as_str() {
            "asset" | "provider" if attr.args.len() == 2 => {
                if sources
                    .insert(
                        attr.args[0].clone(),
                        if attr.name == "asset" {
                            ManifestResourceSource::Asset {
                                asset: attr.args[1].clone(),
                            }
                        } else {
                            ManifestResourceSource::External {
                                provider: attr.args[1].clone(),
                            }
                        },
                    )
                    .is_some()
                {
                    return Err("duplicate asset requirement".into());
                }
            }
            "dimensions" if attr.args.len() == 3 => {
                let width = attr.args[1]
                    .parse::<u32>()
                    .map_err(|_| "image width must be u32")?;
                let height = attr.args[2]
                    .parse::<u32>()
                    .map_err(|_| "image height must be u32")?;
                if width == 0
                    || height == 0
                    || dimensions
                        .insert(
                            attr.args[0].clone(),
                            ManifestImageExtent::Fixed { width, height },
                        )
                        .is_some()
                {
                    return Err("image dimensions must be positive and declared once".into());
                }
            }
            "dimensions" => return Err("@dimensions requires (resource, width, height)".into()),
            "from" if attr.args.len() == 4 => {
                if sources
                    .insert(
                        attr.args[0].clone(),
                        ManifestResourceSource::TechniqueOutput {
                            instance: attr.args[1].clone(),
                            technique: attr.args[2].clone(),
                            output: attr.args[3].clone(),
                        },
                    )
                    .is_some()
                {
                    return Err("duplicate resource source".into());
                }
            }
            "from" => return Err("@from requires (resource, instance, technique, output)".into()),
            "pool" if attr.args.len() == 2 => {
                if pools
                    .insert(attr.args[0].clone(), attr.args[1].clone())
                    .is_some()
                {
                    return Err("duplicate pool requirement".into());
                }
            }
            "output" if attr.args.len() == 2 => {
                if outputs
                    .insert(attr.args[0].clone(), attr.args[1].clone())
                    .is_some()
                {
                    return Err("duplicate technique output".into());
                }
            }
            "asset" | "provider" | "pool" | "output" => {
                return Err(format!("@{} requires two arguments", attr.name));
            }
            "table_data" => {
                return Err(
                    "techniques import engine table buffers through typed external inputs".into(),
                );
            }
            "technique" | "image" | "buffer" | "external" | "meta" => {}
            _ => return Err(format!("unsupported technique attribute `@{}`", attr.name)),
        }
    }
    let mut extents = BTreeMap::new();
    let mut instances = BTreeMap::new();
    let mut enabled = BTreeMap::new();
    for reference in &mut recipe.pass_refs {
        let name = reference
            .attrs
            .iter()
            .find(|a| a.name == "node")
            .and_then(|a| a.args.first())
            .unwrap_or(&reference.name)
            .clone();
        if reference
            .attrs
            .iter()
            .any(|a| matches!(a.name.as_str(), "when" | "per_invocation"))
        {
            return Err(
                "standalone techniques require unconditional steps and explicit resource sizes"
                    .into(),
            );
        }
        for attr in &mut reference.attrs {
            match attr.name.as_str() {
                "enabled" if attr.args.len() == 1 && is_parameter(&attr.args[0]) => {
                    if enabled.insert(name.clone(), attr.args[0].clone()).is_some() {
                        return Err("duplicate step activation".into());
                    }
                }
                "dispatch" => {
                    if attr.args.len() == 2 {
                        if !is_parameter(&attr.args[1]) {
                            return Err(
                                "two-argument dispatch requires a named invocation extent".into()
                            );
                        }
                        extents.insert(
                            name.clone(),
                            ManifestDispatchExtent::Parameter {
                                parameter: attr.args[1].clone(),
                            },
                        );
                        attr.args = vec![attr.args[0].clone(), "1".into(), "1".into()];
                        continue;
                    }
                    if attr.args.len() != 4 {
                        return Err("technique dispatch requires (entry, x, y, z) logical invocation extent".into());
                    }
                    let mut extent = [0; 3];
                    for (axis, arg) in attr.args[1..].iter().enumerate() {
                        extent[axis] = arg.parse().map_err(|_| "dispatch extent must be u32")?;
                        if extent[axis] == 0 {
                            return Err("dispatch extent must be positive".into());
                        }
                    }
                    extents.insert(name.clone(), ManifestDispatchExtent::Fixed(extent));
                    // The shared recipe parser owns entry, binding, and graph syntax.
                    // Viewport scaling is replaced by an explicit extent below.
                    attr.args = vec![attr.args[0].clone(), "1".into(), "1".into()];
                }
                "instances" => {
                    if attr.args.len() != 1 || instances.contains_key(&name) {
                        return Err("requires one @instances(count)".into());
                    }
                    let count = match attr.args[0].parse::<u32>() {
                        Ok(0) => return Err("instance count must be positive".into()),
                        Ok(count) => ManifestDrawCount::Fixed(count),
                        Err(_) if is_parameter(&attr.args[0]) => ManifestDrawCount::Parameter {
                            parameter: attr.args[0].clone(),
                        },
                        Err(_) => {
                            return Err(
                                "instance count must be a positive u32 or named parameter".into()
                            );
                        }
                    };
                    instances.insert(name.clone(), count);
                }
                "node" | "draw" | "draw_depth" | "bind" | "color" | "depth" | "after"
                | "attachment" => {}
                _ => {
                    return Err(format!(
                        "unsupported technique step attribute `@{}`",
                        attr.name
                    ));
                }
            }
        }
        reference
            .attrs
            .retain(|a| a.name != "instances" && a.name != "enabled");
    }
    let (declarations, steps) = super::recipes::reflect(&recipe)?;
    let mut types = BTreeMap::new();
    let mut strides = BTreeMap::new();
    let mut result = Vec::new();
    for step in steps {
        let program = programs
            .iter()
            .find(|p| p.pass == step.pass)
            .ok_or_else(|| format!("step `{}` requires an explicit @shader program", step.name))?;
        if program.bindings.len() != step.bindings.len() {
            return Err(format!("incomplete resource wiring for `{}`", step.name));
        }
        let mut reads = BTreeSet::new();
        let mut writes = BTreeSet::new();
        for binding in &program.bindings {
            let resource = step
                .bindings
                .get(&binding.name)
                .ok_or_else(|| format!("missing binding `{}`", binding.name))?;
            if let Some(stride) = binding.element_stride {
                strides.insert(resource.clone(), stride);
                let declaration = declarations
                    .iter()
                    .find(|r| &r.name == resource)
                    .ok_or("undefined resource")?;
                if let Some(bytes) = declaration.bytes
                    && (bytes < stride || bytes % stride != 0)
                {
                    return Err(format!(
                        "resource `{resource}` size is incompatible with element stride {stride}"
                    ));
                }
            }
            let signature = (binding.kind.clone(), binding.ty.clone());
            if let Some(previous) = types.insert(resource.clone(), signature.clone())
                && previous != signature
            {
                return Err(format!(
                    "incompatible binding types for resource `{resource}`"
                ));
            }
            for entry in program.entries.iter().filter(|entry| {
                (entry.function == step.entry
                    && entry.stage
                        == if step.domain == "compute" {
                            "compute"
                        } else {
                            "fragment"
                        })
                    || (Some(&entry.function) == step.vertex.as_ref() && entry.stage == "vertex")
            }) {
                match binding.entry_access.get(&entry.entry).map(String::as_str) {
                    Some("read") => {
                        reads.insert(resource.clone());
                    }
                    Some("write") => {
                        writes.insert(resource.clone());
                    }
                    Some("read_write") => {
                        reads.insert(resource.clone());
                        writes.insert(resource.clone());
                    }
                    None => {}
                    Some(_) => return Err("unknown GPU resource access".into()),
                }
            }
        }
        let entry = |function: &str, stage: &str| {
            program
                .entries
                .iter()
                .find(|e| e.function == function && e.stage == stage)
                .ok_or_else(|| format!("unknown {stage} entry `{function}`"))
        };
        let operation = if step.domain == "compute" {
            if instances.contains_key(&step.name) {
                return Err("instances require a draw".into());
            }
            let extent = extents
                .get(&step.name)
                .ok_or("missing dispatch extent")?
                .clone();
            ManifestTechniqueOperation::Compute {
                entry: entry(&step.entry, "compute")?.entry.clone(),
                extent,
                workgroup_size: program.workgroup_size.ok_or("missing workgroup size")?,
            }
        } else {
            let fragment = if step.entry.is_empty() {
                if step.depth.is_none() || !step.colors.is_empty() {
                    return Err(
                        "depth-only draws require a depth attachment and no color attachments"
                            .into(),
                    );
                }
                None
            } else {
                Some(entry(&step.entry, "fragment")?)
            };
            if fragment.map_or(0, |f| f.outputs.len()) != step.colors.len() {
                return Err("fragment outputs and attachments disagree".into());
            }
            for output in fragment.into_iter().flat_map(|f| &f.outputs) {
                let name = step
                    .colors
                    .get(&output.location)
                    .ok_or("missing color attachment")?;
                let resource = declarations
                    .iter()
                    .find(|r| &r.name == name)
                    .ok_or("undefined color attachment")?;
                let compatible = resource
                    .format
                    .as_deref()
                    .and_then(fresco_artifact::types::ImageFormatRequirement::parse)
                    .is_some_and(|f| f.accepts_shader_output(&output.ty));
                if !compatible {
                    return Err("fragment output and attachment format are incompatible".into());
                }
            }
            for name in step.colors.values().chain(step.depth.iter()) {
                if step.bindings.values().any(|bound| bound == name) || !writes.insert(name.clone())
                {
                    return Err("a draw cannot bind or alias an active attachment".into());
                }
            }
            if let Some(name) = &step.depth
                && !declarations
                    .iter()
                    .any(|r| r.name == *name && r.format.as_deref() == Some("depth32float"))
            {
                return Err("depth attachment requires depth32float".into());
            }
            if step.vertex_count == 0 {
                return Err("draw vertex count must be positive".into());
            }
            ManifestTechniqueOperation::Draw {
                vertex: entry(
                    step.vertex.as_deref().ok_or("missing vertex entry")?,
                    "vertex",
                )?
                .entry
                .clone(),
                fragment: fragment.map(|f| f.entry.clone()),
                vertices: step.vertex_count,
                instances: instances
                    .get(&step.name)
                    .cloned()
                    .unwrap_or(ManifestDrawCount::Fixed(1)),
                colors: step.colors,
                depth: step.depth,
            }
        };
        result.push(ManifestTechniqueStep {
            attachments: step.attachments,
            enabled: enabled.remove(&step.name),
            name: step.name,
            pass: step.pass,
            operation,
            bindings: step.bindings,
            reads: reads.into_iter().collect(),
            writes: writes.into_iter().collect(),
            after: step.after,
        });
    }
    let mut resources = Vec::new();
    for declaration in declarations {
        let signature = types.get(&declaration.name);
        let descriptor = match declaration.kind.as_str() {
            "image" => {
                let format = declaration.format.ok_or("missing image format")?;
                if let Some((kind, ty)) = signature {
                    let expected = fresco_artifact::types::ImageFormatRequirement::parse(&format)
                        .and_then(
                            fresco_artifact::types::ImageFormatRequirement::sampled_shader_type,
                        )
                        .ok_or("image binding requires a sampleable format")?;
                    if kind != "texture" || ty != &expected {
                        return Err("image and binding types disagree".into());
                    }
                }
                ManifestResourceType::Image {
                    format,
                    extent: dimensions
                        .remove(&declaration.name)
                        .unwrap_or(ManifestImageExtent::Viewport),
                }
            }
            "buffer" => {
                let (kind, element) = signature.ok_or("allocated buffer has no typed binding")?;
                if kind != "storage" {
                    return Err("allocated buffers require storage bindings".into());
                }
                ManifestResourceType::Buffer {
                    element: element.clone(),
                    bytes: declaration.bytes.ok_or("missing buffer size")?,
                }
            }
            "external" => {
                let (kind, element) = signature.ok_or("external resource has no typed binding")?;
                match kind.as_str() {
                    "storage" => ManifestResourceType::Buffer {
                        element: element.clone(),
                        bytes: *strides
                            .get(&declaration.name)
                            .ok_or("external buffer layout missing")?,
                    },
                    "uniform" => ManifestResourceType::Uniform {
                        element: element.clone(),
                        bytes: *strides
                            .get(&declaration.name)
                            .ok_or("uniform layout missing")?,
                    },
                    "texture" => ManifestResourceType::Image {
                        extent: dimensions
                            .remove(&declaration.name)
                            .unwrap_or(ManifestImageExtent::Viewport),
                        format: match element.as_str() {
                            "texture_2d<u32>" => "r32uint",
                            "texture_2d<i32>" => "r32sint",
                            "texture_depth_2d" => "depth32float",
                            _ => {
                                return Err(
                                    "external float textures require an explicit image format"
                                        .into(),
                                );
                            }
                        }
                        .into(),
                    },
                    _ => return Err("unsupported external resource type".into()),
                }
            }
            _ => return Err("unsupported technique resource".into()),
        };
        let source = if let Some(source) = sources.remove(&declaration.name) {
            if pools.contains_key(&declaration.name) {
                return Err("borrowed resources cannot request a pool".into());
            }
            source
        } else if declaration.kind == "external" {
            if pools.contains_key(&declaration.name) {
                return Err("borrowed resources cannot request a pool".into());
            }
            sources
                .remove(&declaration.name)
                .unwrap_or(ManifestResourceSource::External {
                    provider: declaration.source.ok_or("missing provider")?,
                })
        } else {
            ManifestResourceSource::Allocate {
                pool: pools.remove(&declaration.name),
            }
        };
        if matches!(&descriptor, ManifestResourceType::Image { format, .. } if format == "float_color")
            && matches!(source, ManifestResourceSource::Allocate { .. })
        {
            return Err(
                "float_color images require an external source with a concrete GPU format".into(),
            );
        }
        resources.push(ManifestTechniqueResource {
            name: declaration.name,
            descriptor,
            source,
        });
    }
    if !dimensions.is_empty() {
        return Err("dimensions require a declared image resource".into());
    }
    if !sources.is_empty() {
        return Err("provider or asset references an undefined resource".into());
    }
    if !pools.is_empty() {
        return Err("pool references an undefined resource".into());
    }
    order(&resources, &mut result)?;
    for resource in outputs.values() {
        if !resources.iter().any(|r| &r.name == resource) {
            return Err("output references an undefined resource".into());
        }
        if !result.iter().any(|s| s.writes.contains(resource)) {
            return Err(format!("output `{resource}` has no producer"));
        }
    }
    Ok(ManifestTechnique {
        surface: None,
        metadata: metadata(&pipeline.attrs)?,
        name: pipeline.name.clone(),
        resources,
        steps: result,
        outputs,
    })
}

fn order(
    resources: &[ManifestTechniqueResource],
    steps: &mut Vec<ManifestTechniqueStep>,
) -> Result<(), String> {
    fn depends(
        steps: &[ManifestTechniqueStep],
        node: &str,
        predecessor: &str,
        visited: &mut BTreeSet<String>,
    ) -> bool {
        if !visited.insert(node.into()) {
            return false;
        }
        steps.iter().find(|s| s.name == node).is_some_and(|s| {
            s.after
                .iter()
                .any(|p| p == predecessor || depends(steps, p, predecessor, visited))
        })
    }
    let original = steps.clone();
    let mut producers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for step in &original {
        for resource in &step.writes {
            let prior = producers.entry(resource.clone()).or_default();
            if let Some(previous) = prior.last()
                && (!resources.iter().any(|r| {
                    r.name == *resource
                        && matches!(r.descriptor, ManifestResourceType::Buffer { .. })
                }) && !step.attachments.contains_key(resource)
                    || !depends(&original, &step.name, previous, &mut BTreeSet::new()))
            {
                return Err(format!(
                    "conflicting writes to `{resource}` require explicitly ordered buffer updates or attachment operations"
                ));
            }
            prior.push(step.name.clone());
        }
        if step.enabled.is_some()
            && step.writes.iter().any(|name| {
                resources.iter().any(|r| {
                    &r.name == name && matches!(r.source, ManifestResourceSource::Allocate { .. })
                })
            })
        {
            return Err(
                "conditionally executed writes require initialized external resources".into(),
            );
        }
    }
    for step in steps.iter_mut() {
        for resource in &step.reads {
            let producer = producers.get(resource).and_then(|writers| {
                writers.iter().rev().find(|writer| {
                    *writer != &step.name
                        && !depends(&original, writer, &step.name, &mut BTreeSet::new())
                })
            });
            if let Some(producer) = producer {
                if !step.after.contains(producer) {
                    step.after.push(producer.clone());
                }
            } else if resources.iter().any(|r| {
                &r.name == resource && matches!(r.source, ManifestResourceSource::Allocate { .. })
            }) {
                if producers.contains_key(resource) {
                    return Err(format!(
                        "dependency cycle: `{resource}` is read before its producer"
                    ));
                }
                return Err(format!("resource `{resource}` is read without a producer"));
            }
        }
        step.after.sort();
    }
    let mut done = BTreeSet::new();
    let mut ordered = Vec::new();
    while ordered.len() < steps.len() {
        let step = steps
            .iter()
            .find(|s| !done.contains(&s.name) && s.after.iter().all(|p| done.contains(p)))
            .ok_or("technique has a dependency cycle or missing predecessor")?;
        done.insert(step.name.clone());
        ordered.push(step.clone());
    }
    *steps = ordered;
    Ok(())
}

fn validate_imports(techniques: &[ManifestTechnique]) -> Result<(), String> {
    let mut dependencies = BTreeMap::new();
    for technique in techniques {
        let mut after = BTreeSet::new();
        let mut instances = BTreeMap::new();
        for resource in &technique.resources {
            if let ManifestResourceSource::TechniqueOutput {
                instance,
                technique: producer,
                output,
            } = &resource.source
            {
                if let Some(previous) = instances.insert(instance, producer)
                    && previous != producer
                {
                    return Err(format!(
                        "instance slot `{instance}` refers to different techniques"
                    ));
                }
                let producer = techniques
                    .iter()
                    .find(|t| &t.name == producer)
                    .ok_or_else(|| format!("unknown producer technique `{producer}`"))?;
                let name = producer.outputs.get(output).ok_or_else(|| {
                    format!("unknown technique output `{}.{output}`", producer.name)
                })?;
                let produced = producer
                    .resources
                    .iter()
                    .find(|r| &r.name == name)
                    .ok_or("producer output references an undefined resource")?;
                if resource.descriptor != produced.descriptor {
                    return Err(format!(
                        "incompatible imported output `{}.{output}`",
                        producer.name
                    ));
                }
                after.insert(producer.name.clone());
            }
        }
        if dependencies.insert(technique.name.clone(), after).is_some() {
            return Err("duplicate technique name".into());
        }
    }
    let mut done = BTreeSet::new();
    while done.len() < techniques.len() {
        let name = dependencies
            .iter()
            .find(|(name, after)| !done.contains(*name) && after.iter().all(|p| done.contains(p)))
            .map(|(name, _)| name.clone())
            .ok_or("cycle between technique output dependencies")?;
        done.insert(name);
    }
    Ok(())
}

pub(super) fn metadata(
    attrs: &[crate::ast::PipelineAttribute],
) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for attr in attrs.iter().filter(|a| a.name == "meta") {
        if attr.args.len() != 2
            || values
                .insert(attr.args[0].clone(), attr.args[1].clone())
                .is_some()
        {
            return Err("metadata requires a unique key and one scalar/string value".into());
        }
    }
    Ok(values)
}

fn is_parameter(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
