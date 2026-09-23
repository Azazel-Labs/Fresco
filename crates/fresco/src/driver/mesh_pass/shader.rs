//! General GPU stage programs declared by engine passes.
use super::*;
use fresco_artifact::{
    ManifestGpuBinding, ManifestGpuProgram, ManifestRasterEntry, ManifestStageOutput,
};

pub(in crate::driver) fn build(
    passes: &[PassDecl],
    structs: &[StructDecl],
    include_programs: bool,
    pipelines: &[crate::ast::PipelineDecl],
    library: &crate::check::compute::ComputeLibrary,
    materials: &[MaterialHir],
    prelude: &str,
) -> Result<Vec<(String, ManifestGpuProgram)>, String> {
    if !include_programs
        && !pipelines.iter().any(super::super::techniques::is_technique)
        && !passes.iter().any(|p| p.compute_invocation.is_some())
    {
        return Ok(Vec::new());
    }
    let selected = pipelines
        .iter()
        .find(|p| p.attrs.iter().any(|a| a.name == "selected_renderer"));
    passes
        .iter()
        .filter(|pass| {
            pass.compute_invocation.is_some()
                || (include_programs
                    && selected.is_none_or(|pipeline| {
                        pipeline.pass_refs.iter().any(|r| r.name == pass.name)
                    }))
                || pipelines
                    .iter()
                    .filter(|p| super::super::techniques::is_technique(p))
                    .any(|p| p.pass_refs.iter().any(|r| r.name == pass.name))
        })
        .filter(|pass| pass.attrs.iter().any(|a| a.name == "shader"))
        .flat_map(|pass| {
            if let Some(expected) = &pass.material_name {
                materials
                    .iter()
                    .filter(|m| m.material_schemas.contains_key(expected))
                    .map(|m| emit(pass, structs, library, Some(m), prelude))
                    .collect::<Vec<_>>()
            } else {
                vec![emit(pass, structs, library, None, prelude)]
            }
        })
        .collect()
}

pub(in crate::driver) fn emit(
    pass: &PassDecl,
    structs: &[StructDecl],
    library: &crate::check::compute::ComputeLibrary,
    material: Option<&MaterialHir>,
    prelude: &str,
) -> Result<(String, ManifestGpuProgram), String> {
    if pass
        .hooks
        .iter()
        .filter_map(|hook| hook.dispatch.as_ref())
        .any(|plan| {
            !plan.draw_scoped
                && plan
                    .cases
                    .iter()
                    .any(|case| !case.shading_inputs.is_empty())
        })
    {
        return Err(
            "resource-capturing dynamic dispatch requires shading-instance lowering".into(),
        );
    }
    let prefix = format!(
        "fresco_gpu_{}{}",
        sanitize(&pass.name),
        material
            .map(|m| format!("_{}", sanitize(&m.name)))
            .unwrap_or_default()
    );
    let mut aliases: HashMap<String, String> = structs
        .iter()
        .map(|s| (s.name.clone(), format!("{prefix}_{}", s.name)))
        .collect();
    if let Some(material) = material {
        aliases.insert(
            material.context.ty.name.clone(),
            material.context.ty.name.clone(),
        );
        for name in material.material_schemas.keys() {
            aliases.insert(name.clone(), format!("FrescoMaterial_{}", material.name));
        }
        aliases.extend(material.evaluation_type_aliases.clone());
    }
    // Imported concrete functions retain library scope, separate from pass hooks.
    for (index, function) in library.functions().iter().enumerate() {
        if !function.is_builtin && !function.is_internal {
            aliases.insert(
                function.name.clone(),
                format!("{prefix}_import_{}", function.name),
            );
            aliases.insert(
                super::super::schema_function::library_symbol(index),
                if library
                    .functions()
                    .iter()
                    .filter(|f| f.name == function.name)
                    .count()
                    > 1
                {
                    format!("{prefix}_import_{}_{index}", function.name)
                } else {
                    format!("{prefix}_import_{}", function.name)
                },
            );
        }
    }
    let imported_aliases = aliases.clone();
    let mut names = BTreeSet::new();
    for hook in &pass.hooks {
        if !names.insert(&hook.name) {
            return Err("standalone GPU functions require unique names".into());
        }
        aliases.insert(hook.name.clone(), format!("{prefix}_{}", hook.name));
    }
    aliases.insert("clip_position".into(), "vec4<f32>".into());
    let type_name = |name: &str| {
        aliases
            .get(name)
            .cloned()
            .map_or_else(|| wgsl_type(name), Ok)
    };
    let mut needed = BTreeSet::new();
    let mut variables = HashMap::new();
    let mut binding_slots = BTreeSet::new();
    let mut bindings = Vec::new();
    let mut code = String::new();
    for binding in &pass.bindings {
        let signature = binding
            .value_signature
            .as_deref()
            .ok_or("GPU resource requires a signature")?;
        let group = binding.group_index.ok_or("GPU resource requires a group")?;
        let index = binding
            .binding_index
            .ok_or("GPU resource requires a binding")?;
        if !binding_slots.insert((group, index)) {
            return Err("GPU resource bindings occupy the same slot".into());
        }
        let mut access = "read";
        for attr in &binding.attrs {
            match attr.name.as_str() {
                "group" | "binding" | "geometry_resource" => {}
                "source" | "draw_data" | "sampler"
                    if pass.attrs.iter().any(|a| a.name == "factory") => {}
                "access" if attr.args.as_slice() == ["read_write"] => access = "read_write",
                "access" if attr.args.as_slice() == ["write"] => access = "write",
                _ => {
                    return Err(format!(
                        "unsupported GPU resource attribute `@{}`",
                        attr.name
                    ));
                }
            }
        }
        let (kind, ty, address) = if let Some(ty) = uniform_type(signature) {
            if access != "read" {
                return Err("uniform resources are read-only".into());
            }
            if aliases.contains_key(ty) {
                needed.insert(ty.to_string());
            }
            ("uniform", ty.to_string(), "<uniform>".to_string())
        } else if let Some(ty) = signature
            .strip_prefix("buffer<")
            .and_then(|s| s.strip_suffix('>'))
        {
            if aliases.contains_key(ty) {
                needed.insert(ty.to_string());
            }
            (
                "storage",
                ty.to_string(),
                format!(
                    "<storage, {}>",
                    if access == "write" {
                        "read_write"
                    } else {
                        access
                    }
                ),
            )
        } else if signature == "sampler" {
            if access != "read" {
                return Err("samplers are read-only".into());
            }
            ("sampler", signature.into(), String::new())
        } else if signature.starts_with("texture_storage_2d<") {
            let inner = signature
                .strip_prefix("texture_storage_2d<")
                .and_then(|s| s.strip_suffix('>'))
                .ok_or("invalid storage image signature")?;
            let (_, qualifier) = inner
                .rsplit_once(',')
                .ok_or("storage image requires explicit access")?;
            if qualifier.trim() != access || access != "write" {
                return Err("owned storage images require write access".into());
            }
            ("storage_texture", signature.into(), String::new())
        } else if matches!(
            signature,
            "texture_2d<f32>" | "texture_2d<u32>" | "texture_2d<i32>" | "texture_depth_2d"
        ) {
            if access != "read" {
                return Err("sampled textures are read-only".into());
            }
            ("texture", signature.into(), String::new())
        } else {
            return Err(format!("unsupported GPU resource `{signature}`"));
        };
        let variable = format!("{prefix}_resource_{}", binding.name);
        let wgsl = match kind {
            "storage" => format!("array<{}>", type_name(&ty)?),
            "uniform" => type_name(&ty)?,
            _ => ty.clone(),
        };
        code.push_str(&format!(
            "@group({group}) @binding({index}) var{address} {variable}: {wgsl};\n"
        ));
        if let Some(alias) = &binding.operation_alias
            && variables.insert(alias.as_str(), variable.clone()).is_some()
        {
            return Err("operation binding alias is ambiguous".into());
        }
        if variables.insert(binding.name.as_str(), variable).is_some() {
            return Err("GPU resource binding names must be unique".into());
        }
        bindings.push(ManifestGpuBinding {
            query_only_entries: BTreeSet::new(),
            fields: Vec::new(),
            entry_access: Default::default(),
            element_stride: None,
            name: binding.name.clone(),
            group,
            binding: index,
            kind: kind.into(),
            ty,
            access: access.into(),
        });
    }
    let referenced_calls = std::cell::RefCell::new(BTreeSet::new());
    let resources = ResourceSpecializations::default();
    let mut imports = library.functions().to_vec();
    for function in &mut imports {
        if names.contains(&function.name) {
            function.is_internal = true;
        }
    }
    let context = ExprContext {
        resource_specializations: Some(&resources),
        imported_functions: Some(&imports),
        imported_records: None,
        referenced_calls: Some(&referenced_calls),
        material,
        input_var: "",
        varying_source_ty: "",
        varying_ty: "",
        factory_entry: "",
        binding_vars: &variables,
        type_aliases: Some(&aliases),
    };
    let pure_pass = PassDecl {
        compute_invocation: None,
        prepared_draw: None,
        preparation: None,
        operation: None,
        hooks: pass
            .hooks
            .iter()
            .filter(|h| h.attrs.iter().any(|a| a.name == "pure"))
            .cloned()
            .collect(),
        ..pass.clone()
    };
    let mut entries = Vec::new();
    let mut workgroup = None;
    for hook in &pass.hooks {
        if hook
            .return_ty
            .as_ref()
            .is_some_and(|ty| super::super::shader_iterators::element(&ty.node).is_some())
        {
            continue;
        }
        if hook.attrs.iter().any(|a| a.name == "pure") {
            if hook.attrs.len() != 1 || !hook.attrs[0].args.is_empty() {
                return Err("@pure must be the only attribute on a typed helper".into());
            }
            let result = hook
                .return_ty
                .as_ref()
                .ok_or("typed helper requires a record result")?;
            let record = find_struct(structs, &result.node)
                .ok_or("typed helper requires a declared record result")?;
            let checked = library.check_hook(&pure_pass, hook, record)?;
            let state_name = format!("{prefix}_pure_{}_result", hook.name);
            let state = format!(
                "struct {state_name} {{ {} }}",
                record
                    .fields
                    .iter()
                    .map(|f| Ok(format!("{}: {},", f.name, wgsl_type(&f.ty_name)?)))
                    .collect::<Result<Vec<_>, String>>()?
                    .join("\n")
            );
            let internal = format!("{prefix}_pure_{}", hook.name);
            code.push_str(&crate::lower::compute::hooks_wgsl(
                &state,
                &state_name,
                &format!("{internal}_ctx"),
                &[(&internal, &checked)],
            )?);
            let params = hook
                .params
                .iter()
                .map(|p| Ok(format!("{}: {}", p.name, type_name(&p.ty_name)?)))
                .collect::<Result<Vec<_>, String>>()?
                .join(", ");
            for param in &hook.params {
                if aliases.contains_key(&param.ty_name) {
                    needed.insert(param.ty_name.clone());
                }
            }
            needed.insert(record.name.clone());
            let args = checked
                .inputs
                .iter()
                .map(|(_, access)| access.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let mut temporary = "fresco_result".to_string();
            while hook.params.iter().any(|p| p.name == temporary) {
                temporary.push('_');
            }
            code.push_str(&format!("fn {}({params}) -> {} {{ let {temporary} = {internal}({args}); return {}({}); }}\n",
                aliases[&hook.name], type_name(&record.name)?, type_name(&record.name)?,
                record.fields.iter().map(|f| format!("{temporary}.{}", f.name)).collect::<Vec<_>>().join(", ")));
            continue;
        }
        if hook.attrs.iter().any(|a| a.name == "evaluate") {
            let material =
                material.ok_or("evaluator requires a material-specialized shader pass")?;
            if hook.attrs.len() != 1
                || hook.attrs[0].args.as_slice() != ["surface"]
                || hook.params.len() != 1
                || hook.params[0].ty_name != material.context.ty.name
                || !hook
                    .return_ty
                    .as_ref()
                    .is_some_and(|t| material.material_schemas.contains_key(&t.node))
                || !crate::parser::executable_pass_hook_body(hook)
                    .map_err(|e| format!("{e:?}"))?
                    .is_empty()
            {
                return Err("@evaluate(surface) requires an empty function with the material context and schema result".into());
            }
            code.push_str(&format!(
                "fn {}({}: {}) -> {} {{ return fresco_{}({}); }}\n",
                aliases[&hook.name],
                hook.params[0].name,
                material.context.ty.name,
                type_name(&hook.return_ty.as_ref().expect("checked result").node)?,
                sanitize(&material.name),
                hook.params[0].name
            ));
            continue;
        }
        let mut attrs = String::new();
        let mut stage = None;
        for attr in &hook.attrs {
            match attr.name.as_str() {
                "vertex" | "fragment" | "compute" if attr.args.is_empty() => {
                    if stage.replace(attr.name.as_str()).is_some() {
                        return Err("GPU function has multiple stages".into());
                    }
                    attrs.push_str(&format!("@{} ", attr.name));
                }
                "workgroup_size" => {
                    if attr.args.is_empty() || attr.args.len() > 3 {
                        return Err("compute program requires one workgroup size of one to three dimensions".into());
                    }
                    let mut size = [1; 3];
                    for (axis, arg) in attr.args.iter().enumerate() {
                        size[axis] = arg.parse().map_err(|_| "workgroup size must be u32")?;
                        if size[axis] == 0 {
                            return Err("workgroup dimensions must be positive".into());
                        }
                    }
                    if workgroup.is_some_and(|previous| previous != size) {
                        return Err(
                            "entries in one program require matching workgroup sizes".into()
                        );
                    }
                    workgroup = Some(size);
                    attrs.push_str(&format!(
                        "@workgroup_size({}, {}, {}) ",
                        size[0], size[1], size[2]
                    ));
                }
                _ => {
                    return Err(format!(
                        "unsupported GPU function attribute `@{}`",
                        attr.name
                    ));
                }
            }
        }
        if hook.attrs.iter().any(|a| a.name == "workgroup_size") && stage != Some("compute") {
            return Err("workgroup size requires a compute entry".into());
        }
        if stage == Some("compute")
            && (!hook.attrs.iter().any(|a| a.name == "workgroup_size") || hook.return_ty.is_some())
        {
            return Err("compute entry requires a workgroup size and no return value".into());
        }
        let mut params = Vec::new();
        for param in &hook.params {
            let mut attribute = String::new();
            for attr in &param.attrs {
                if stage.is_none()
                    || attr.name != "builtin"
                    || attr.args.len() != 1
                    || !attribute.is_empty()
                {
                    return Err("GPU entry parameter accepts one @builtin(name)".into());
                }
                attribute = format!("@builtin({}) ", attr.args[0]);
            }
            if aliases.contains_key(&param.ty_name) {
                needed.insert(param.ty_name.clone());
            }
            params.push(format!(
                "{attribute}{}: {}",
                shader_identifier(&param.name),
                type_name(&param.ty_name)?
            ));
        }
        let mut outputs = Vec::new();
        let result = if let Some(result) = &hook.return_ty {
            let attribute = if stage == Some("fragment") && !aliases.contains_key(&result.node) {
                outputs.push(ManifestStageOutput {
                    location: 0,
                    name: "color".into(),
                    ty: result.node.clone(),
                });
                "@location(0) "
            } else if stage == Some("vertex") && result.node == "clip_position" {
                "@builtin(position) "
            } else {
                ""
            };
            if let Some(record) = find_struct(structs, &result.node) {
                needed.insert(record.name.clone());
                if stage == Some("fragment") {
                    outputs = program::output_fields(record)?;
                }
            }
            format!(" -> {attribute}{}", type_name(&result.node)?)
        } else {
            String::new()
        };
        let mut body = crate::parser::executable_pass_hook_body(hook)
            .map_err(|e| format!("invalid GPU function `{}`: {e:?}", hook.name))?;
        if pass.hooks.iter().any(|hook| {
            hook.return_ty
                .as_ref()
                .is_some_and(|ty| super::super::shader_iterators::element(&ty.node).is_some())
        }) {
            super::super::shader_iterators::expand_body(pass, &mut body)?;
        }
        let mut value_types: HashMap<_, _> = bindings
            .iter()
            .map(|binding| {
                (
                    binding.name.clone(),
                    if binding.kind == "storage" {
                        format!("buffer<{}>", binding.ty)
                    } else {
                        binding.ty.clone()
                    },
                )
            })
            .collect();
        for binding in &pass.bindings {
            if let Some(alias) = &binding.operation_alias {
                let ty = value_types[&binding.name].clone();
                if value_types.insert(alias.clone(), ty).is_some() {
                    return Err("operation binding alias is ambiguous".into());
                }
            }
        }
        value_types.extend(
            hook.params
                .iter()
                .map(|p| (p.name.clone(), p.ty_name.clone())),
        );
        super::super::schema_function::resolve_library_body(
            &mut body,
            value_types,
            library.functions(),
            structs,
            &names.iter().map(|name| (*name).clone()).collect(),
        )?;
        code.push_str(&format!(
            "{attrs}fn {}({}){result} {{\n{}\n}}\n",
            aliases[&hook.name],
            params.join(", "),
            emit_statements(&body, &context)?
        ));
        if let Some(stage) = stage {
            if pass.stage.as_ref().map(|s| s.node.as_str())
                != Some(if stage == "compute" {
                    "compute"
                } else {
                    "raster"
                })
            {
                return Err("GPU entry does not match its pass stage".into());
            }
            entries.push(ManifestRasterEntry {
                function: hook.name.clone(),
                entry: aliases[&hook.name].clone(),
                stage: stage.into(),
                outputs,
            });
        }
    }
    let initial: BTreeSet<_> = referenced_calls
        .into_inner()
        .into_iter()
        .filter(|name| !names.contains(name))
        .collect();
    let (imported_code, imported_types) = link_imports(
        library.functions(),
        structs,
        &imported_aliases,
        initial,
        &resources,
    )?;
    code.push_str(&imported_code);
    needed.extend(imported_types);
    if entries.is_empty() {
        return Err("GPU program has no stage entries".into());
    }
    let varying_types: BTreeSet<_> = pass
        .hooks
        .iter()
        .filter(|hook| {
            hook.attrs
                .iter()
                .any(|attribute| attribute.name == "vertex")
        })
        .filter_map(|hook| hook.return_ty.as_ref().map(|ty| ty.node.as_str()))
        .collect();
    let mut done = BTreeSet::new();
    while let Some(name) = needed.iter().find(|name| !done.contains(*name)).cloned() {
        done.insert(name.clone());
        if material
            .is_some_and(|m| m.material_schemas.contains_key(&name) || m.context.ty.name == name)
        {
            continue;
        }
        let record =
            find_struct(structs, &name).ok_or_else(|| format!("unknown GPU record `{name}`"))?;
        let mut occupied_locations = BTreeSet::new();
        if varying_types.contains(name.as_str()) {
            for field in &record.fields {
                for (_, args) in field.attrs.iter().filter(|(name, _)| name == "location") {
                    let [location] = args.as_slice() else {
                        return Err("varying location requires one index".into());
                    };
                    let location = location
                        .parse::<u32>()
                        .map_err(|_| "varying location requires a u32 index")?;
                    if !occupied_locations.insert(location) {
                        return Err("duplicate varying location".into());
                    }
                }
            }
        }
        let mut next_location = 0u32;
        code.push_str(&format!("struct {} {{\n", aliases[&name]));
        for field in &record.fields {
            if aliases.contains_key(&field.ty_name) {
                needed.insert(field.ty_name.clone());
            }
            let mut attrs = String::new();
            for (name, args) in &field.attrs {
                if record.attrs.iter().any(|a| a.name == "table")
                    && matches!(
                        name.as_str(),
                        "table_index"
                            | "property_value"
                            | "schema_value"
                            | "implementation_value"
                            | "implementation_settings_value"
                    )
                {
                    // Validated table columns describe CPU population, not GPU
                    // interface decorations. Their field layout stays intact.
                    continue;
                }
                if name == "semantic" && args.as_slice() == ["position"] {
                    attrs.push_str("@builtin(position) ");
                } else if matches!(name.as_str(), "location" | "builtin" | "interpolate")
                    && !args.is_empty()
                {
                    attrs.push_str(&format!("@{name}({}) ", args.join(", ")));
                } else {
                    return Err(format!("unsupported GPU record field attribute `@{name}`"));
                }
            }
            if varying_types.contains(record.name.as_str())
                && !field
                    .attrs
                    .iter()
                    .any(|(name, _)| matches!(name.as_str(), "location" | "builtin" | "semantic"))
            {
                while occupied_locations.contains(&next_location) {
                    next_location = next_location
                        .checked_add(1)
                        .ok_or("varying location overflow")?;
                }
                attrs.push_str(&format!("@location({next_location}) "));
                occupied_locations.insert(next_location);
            }
            code.push_str(&format!(
                "{attrs}{}: {},\n",
                shader_identifier(&field.name),
                type_name(&field.ty_name)?
            ));
        }
        code.push_str("}\n");
    }
    let combined = format!("{prelude}\n{code}");
    let module =
        naga::front::wgsl::parse_str(&combined).map_err(|e| e.emit_to_string(&combined))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| {
        format!(
            "invalid GPU program `{}`: {}",
            pass.name,
            e.emit_to_string(&combined)
        )
    })?;
    let mut layouts = naga::proc::Layouter::default();
    layouts.update(module.to_ctx()).map_err(|e| e.to_string())?;
    for (handle, global) in module.global_variables.iter() {
        if !global
            .name
            .as_ref()
            .is_some_and(|name| name.starts_with(&format!("{prefix}_resource_")))
        {
            continue;
        }
        let Some(slot) = &global.binding else {
            continue;
        };
        let Some(binding) = bindings
            .iter_mut()
            .find(|b| b.group == slot.group && b.binding == slot.binding)
        else {
            return Err("GPU resource reflection is incomplete".into());
        };
        for (index, entry) in module.entry_points.iter().enumerate() {
            let usage = info.get_entry_point(index)[handle];
            let query = usage.contains(naga::valid::GlobalUse::QUERY);
            if query
                && !usage.intersects(naga::valid::GlobalUse::READ | naga::valid::GlobalUse::WRITE)
            {
                binding.query_only_entries.insert(entry.name.clone());
            }
            let access = match (
                usage.contains(naga::valid::GlobalUse::READ)
                    || query && !usage.contains(naga::valid::GlobalUse::WRITE),
                usage.contains(naga::valid::GlobalUse::WRITE),
            ) {
                (true, true) => Some("read_write"),
                (true, false) => Some("read"),
                (false, true) => Some("write"),
                (false, false) => None,
            };
            if let Some(access) = access {
                binding
                    .entry_access
                    .insert(entry.name.clone(), access.into());
            }
        }
        let element = match &module.types[global.ty].inner {
            naga::TypeInner::Array { base, .. } => *base,
            _ => global.ty,
        };
        if let naga::TypeInner::Struct { members, .. } = &module.types[element].inner {
            let record =
                find_struct(structs, &binding.ty).ok_or("GPU record layout source missing")?;
            binding.fields = members
                .iter()
                .zip(&record.fields)
                .map(|(member, field)| fresco_artifact::ManifestBufferField {
                    name: field.name.clone(),
                    offset: member.offset,
                    ty: field.ty_name.clone(),
                })
                .collect();
        }
        binding.element_stride = match &module.types[global.ty].inner {
            naga::TypeInner::Array { stride, .. } => Some(*stride),
            _ if binding.kind == "uniform" => Some(layouts[global.ty].size),
            _ => None,
        };
        if binding.access == "write" {
            if !(0..module.entry_points.len()).any(|index| {
                info.get_entry_point(index)[handle].contains(naga::valid::GlobalUse::WRITE)
            }) {
                return Err(format!(
                    "write-only resource `{}` is never written",
                    binding.name
                ));
            }

            for index in 0..module.entry_points.len() {
                if info.get_entry_point(index)[handle].contains(naga::valid::GlobalUse::READ) {
                    return Err(format!(
                        "write-only resource `{}` is read by a shader entry",
                        binding.name
                    ));
                }
            }
        }
    }
    Ok((
        code,
        ManifestGpuProgram {
            compute_invocation: pass.compute_invocation.as_deref().cloned(),
            compute_bindings: pass
                .compute_invocation
                .as_ref()
                .map(|i| i.bindings.clone())
                .unwrap_or_default(),
            metadata: super::super::techniques::metadata(&pass.attrs)?,
            properties: pass
                .entry_properties
                .iter()
                .cloned()
                .map(super::super::emit::manifest_entry_property)
                .collect(),
            surface: material
                .map(|m| m.name.clone())
                .or_else(|| pass.compute_invocation.as_ref().map(|i| i.material.clone())),
            pass: pass.name.clone(),
            entries,
            bindings,
            workgroup_size: workgroup,
        },
    ))
}
