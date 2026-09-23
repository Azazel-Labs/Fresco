//! Resolve authored group symbols independently of engine vocabulary.
use crate::{ast::Program, diag::Diag};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn resolve(program: &mut Program) -> Result<(), Vec<Diag>> {
    for pass in &program.passes {
        if !pass.attrs.iter().any(|a| a.name == "factory")
            && pass
                .bindings
                .iter()
                .any(|b| b.attrs.iter().any(|a| a.name == "sampler"))
        {
            return Err(vec![Diag::error(
                pass.span.clone(),
                "sampler presets require a factory execution scope",
            )]);
        }
        if !pass.attrs.iter().any(|a| a.name == "factory")
            && pass
                .bindings
                .iter()
                .any(|b| b.attrs.iter().any(|a| a.name == "draw_data"))
        {
            return Err(vec![Diag::error(
                pass.span.clone(),
                "draw data requires a mesh factory execution scope",
            )]);
        }
    }
    let mut declarations = BTreeMap::new();
    let mut occupied = BTreeSet::new();
    for group in &program.groups {
        let fail = |message| vec![Diag::error(group.span.clone(), message)];
        if declarations.contains_key(&group.name) {
            return Err(fail(format!("duplicate resource group `{}`", group.name)));
        }
        for attr in &group.attrs {
            if !matches!(attr.name.as_str(), "group" | "allocate") {
                return Err(fail(format!(
                    "unsupported group attribute `@{}`",
                    attr.name
                )));
            }
        }
        let indices: Vec<_> = group.attrs.iter().filter(|a| a.name == "group").collect();
        let index = match indices.as_slice() {
            [] => None,
            [attr] if attr.name == "group" && attr.args.len() == 1 => {
                Some(attr.args[0].parse::<u32>().map_err(|_| {
                    fail("resource group index must be a nonnegative u32 integer".into())
                })?)
            }
            _ => return Err(fail("resource group accepts only @group(index)".into())),
        };
        if let Some(index) = index
            && !occupied.insert(index)
        {
            return Err(fail(format!(
                "resource group index {index} is assigned more than once"
            )));
        }
        declarations.insert(group.name.clone(), index);
    }
    // Assign unnumbered declarations by symbol name, independent of import order.
    let mut next = 0u32;
    for index in declarations.values_mut() {
        if index.is_none() {
            while occupied.contains(&next) {
                next = next
                    .checked_add(1)
                    .ok_or_else(|| vec![Diag::error(0..0, "resource group index overflow")])?;
            }
            *index = Some(next);
            occupied.insert(next);
        }
    }
    let mut roles = BTreeSet::new();
    let role_names = ["parameters", "textures", "storage", "globals"];
    for group in &program.groups {
        for attr in group.attrs.iter().filter(|a| a.name == "allocate") {
            let [role] = attr.args.as_slice() else {
                return Err(vec![Diag::error(
                    attr.span.clone(),
                    "@allocate requires one resource class",
                )]);
            };
            let index = role_names
                .iter()
                .position(|name| *name == role)
                .ok_or_else(|| {
                    vec![Diag::error(
                        attr.span.clone(),
                        format!("unknown resource allocation class `{role}`"),
                    )]
                })?;
            if !roles.insert(index) {
                return Err(vec![Diag::error(
                    attr.span.clone(),
                    format!("duplicate allocation for `{role}`"),
                )]);
            }
            program.resource_layout.0[index] = declarations[&group.name].expect("resolved group");
        }
    }
    if program
        .resource_layout
        .0
        .into_iter()
        .collect::<BTreeSet<_>>()
        .len()
        != 4
    {
        return Err(vec![Diag::error(
            0..0,
            "compiler-managed resource classes require distinct groups",
        )]);
    }
    for binding in program
        .vertex_factories
        .iter_mut()
        .flat_map(|f| &mut f.bindings)
        .chain(program.passes.iter_mut().flat_map(|p| &mut p.bindings))
    {
        let samplers: Vec<_> = binding
            .attrs
            .iter()
            .filter(|a| a.name == "sampler")
            .collect();
        if !samplers.is_empty()
            && (samplers.len() != 1
                || samplers[0].args.len() != 1
                || samplers[0]
                    .args
                    .first()
                    .and_then(|s| fresco_artifact::types::SamplerPreset::parse(s))
                    .is_none()
                || binding.value_signature.as_deref() != Some("sampler")
                || binding.attrs.iter().any(|a| {
                    matches!(
                        a.name.as_str(),
                        "draw_data" | "source" | "geometry_resource" | "access"
                    )
                }))
        {
            return Err(vec![Diag::error(
                binding.span.clone(),
                "@sampler requires one standard sampler preset, a sampler type, and no other binding source",
            )]);
        }
        let draw_data: Vec<_> = binding
            .attrs
            .iter()
            .filter(|a| a.name == "draw_data")
            .collect();
        if !draw_data.is_empty()
            && (draw_data.len() != 1
                || draw_data[0].args.as_slice() != ["instance_id"]
                || binding.value_signature.as_deref() != Some("uniform<u32>")
                || binding
                    .attrs
                    .iter()
                    .any(|a| matches!(a.name.as_str(), "source" | "geometry_resource")))
        {
            return Err(vec![Diag::error(
                binding.span.clone(),
                "@draw_data(instance_id) requires uniform<u32> and no other binding source",
            )]);
        }
        let groups: Vec<_> = binding.attrs.iter().filter(|a| a.name == "group").collect();
        let attr = match groups.as_slice() {
            [] => {
                binding.group_index = None;
                continue;
            }
            [attr] if attr.args.len() == 1 => *attr,
            _ => {
                return Err(vec![Diag::error(
                    binding.span.clone(),
                    "resource binding requires exactly one @group(name or index)",
                )]);
            }
        };
        let name = &attr.args[0];
        binding.group_index = Some(match name.parse::<u32>() {
            Ok(index) => index,
            Err(_) => declarations.get(name).copied().flatten().ok_or_else(|| vec![Diag::error(attr.span.clone(), format!("unknown resource group `{name}`; declare the group or use an explicit numeric index"))])?,
        });
    }
    for bindings in program
        .vertex_factories
        .iter_mut()
        .map(|f| &mut f.bindings)
        .chain(program.passes.iter_mut().map(|p| &mut p.bindings))
    {
        let mut used = BTreeSet::new();
        for binding in bindings.iter_mut() {
            let attrs: Vec<_> = binding
                .attrs
                .iter()
                .filter(|a| a.name == "binding")
                .collect();
            binding.binding_index = match attrs.as_slice() {
                [] => None,
                [attr] if attr.args.len() == 1 => {
                    Some(attr.args[0].parse::<u32>().map_err(|_| {
                        vec![Diag::error(
                            attr.span.clone(),
                            "binding index must be a nonnegative u32",
                        )]
                    })?)
                }
                _ => {
                    return Err(vec![Diag::error(
                        binding.span.clone(),
                        "requires one @binding(index)",
                    )]);
                }
            };
            if let Some(index) = binding.binding_index {
                let group = binding.group_index.ok_or_else(|| {
                    vec![Diag::error(
                        binding.span.clone(),
                        "explicit binding requires a group",
                    )]
                })?;
                if !used.insert((group, index)) {
                    return Err(vec![Diag::error(
                        binding.span.clone(),
                        "duplicate resource binding",
                    )]);
                }
            }
        }
        for binding in bindings {
            if binding.binding_index.is_none()
                && let Some(group) = binding.group_index
            {
                let mut index = 0u32;
                while used.contains(&(group, index)) {
                    index = index.checked_add(1).ok_or_else(|| {
                        vec![Diag::error(binding.span.clone(), "binding index overflow")]
                    })?;
                }
                binding.binding_index = Some(index);
                used.insert((group, index));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chumsky::Parser;
    fn parse(source: &str) -> Program {
        let tokens = crate::lexer::lex_spanned(source);
        crate::parser::program()
            .parse(crate::parser::input(&tokens, source.len()..source.len()))
            .into_result()
            .unwrap()
    }
    #[test]
    fn names_do_not_choose_slots_and_implicit_allocation_is_order_independent() {
        for declarations in ["group zebra, alpha", "group alpha, zebra"] {
            let mut program = parse(&format!(
                "@group(2) group object_data\n{declarations}\nvertex_format V {{ required {{ position: vec3 }} }}\nvertex_factory f for V {{ binding {{ @group(object_data) scene: uniform<S>\n @group(zebra) other: buffer<u32>\n @group(alpha) first: buffer<u32> }} }}"
            ));
            resolve(&mut program).unwrap();
            let groups: Vec<_> = program.vertex_factories[0]
                .bindings
                .iter()
                .map(|b| b.group_index.unwrap())
                .collect();
            assert_eq!(groups, [2, 1, 0]);
        }
    }
    #[test]
    fn explicit_slots_are_reserved_before_implicit_slots() {
        let mut program = parse(
            "@group(5) group objects\npass p { binding { @group(objects) first: uniform<S>\n@group(objects) @binding(0) reserved: uniform<S>\n@group(objects) last: uniform<S> } }",
        );
        resolve(&mut program).unwrap();
        assert_eq!(
            program.passes[0]
                .bindings
                .iter()
                .map(|b| (b.group_index, b.binding_index))
                .collect::<Vec<_>>(),
            [(Some(5), Some(1)), (Some(5), Some(0)), (Some(5), Some(2))]
        );
        for attrs in ["@binding(0)", "@binding(-1)", "@binding(2) @binding(3)"] {
            let mut program = parse(&format!(
                "pass p {{ binding {{ @group(5) @binding(0) a: uniform<S>\n@group(5) {attrs} b: uniform<S> }} }}"
            ));
            assert!(resolve(&mut program).is_err());
        }
    }
    #[test]
    fn collisions_and_unknown_groups_are_diagnostics() {
        for source in [
            "@group(2) group a\n@group(2) group b",
            "group a\ngroup a",
            "@group(-1) group a",
            "vertex_format V { required { position: vec3 } }\nvertex_factory f for V { binding { @group(missing) scene: uniform<S> } }",
        ] {
            assert!(resolve(&mut parse(source)).is_err(), "{source}");
        }
    }
}
