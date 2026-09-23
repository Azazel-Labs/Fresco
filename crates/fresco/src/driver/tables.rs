//! Engine-declared, bundle-local tables of specialized surface records.
use crate::{
    ast::{Expr, StructDecl},
    material_hir::MaterialHir,
};
use fresco_artifact::{ManifestTable, ManifestTableRecord};
use std::collections::BTreeSet;

pub(super) fn build(
    structs: &[StructDecl],
    materials: &[MaterialHir],
) -> Result<Vec<ManifestTable>, String> {
    let mut tables = Vec::new();
    for record in structs {
        let attrs: Vec<_> = record.attrs.iter().filter(|a| a.name == "table").collect();
        if attrs.is_empty() {
            continue;
        }
        let [attr] = attrs.as_slice() else {
            return Err("duplicate @table declaration".into());
        };
        if attr.args.len() != 3
            || attr.args[0] != "surfaces"
            || !matches!(attr.args[1].as_str(), "ascending" | "descending" | "source")
        {
            return Err(
                "@table requires (surfaces, ascending|descending|source, first_index)".into(),
            );
        }
        let first_index = attr.args[2]
            .parse::<u32>()
            .map_err(|_| "table first index must be a u32")?;
        let indices: Vec<_> = record
            .fields
            .iter()
            .filter(|f| f.attrs.iter().any(|(a, _)| a == "table_index"))
            .collect();
        let [index_field] = indices.as_slice() else {
            return Err("table requires exactly one @table_index field".into());
        };
        let mut field_names = BTreeSet::new();
        for field in &record.fields {
            if !field_names.insert(&field.name) {
                return Err(format!("duplicate table field `{}`", field.name));
            }
            if field.ty_name != "u32" {
                return Err("table records currently support u32 fields".into());
            }
            let mut attributes = BTreeSet::new();
            for (name, args) in &field.attrs {
                if !attributes.insert(name) {
                    return Err(format!("duplicate table field attribute `@{name}`"));
                }
                match name.as_str() {
                    "table_index" if args.is_empty() => {}
                    "property_value" if args.len() == 1 => {}
                    "implementation_value" | "implementation_settings_value" if args.len() == 1 => {
                    }
                    "schema_value" if !args.is_empty() && args.len() % 2 == 0 => {
                        let mut schemas = BTreeSet::new();
                        for pair in args.as_chunks::<2>().0 {
                            if !schemas.insert(&pair[0]) {
                                return Err("duplicate table schema mapping".into());
                            }
                            pair[1]
                                .parse::<u32>()
                                .map_err(|_| "table schema value must be a u32")?;
                        }
                    }
                    _ => return Err(format!("invalid table field attribute `@{name}`")),
                }
            }
            if field.name == index_field.name {
                if attributes.len() != 1 || field.default.is_some() {
                    return Err("table index cannot also declare a value".into());
                }
            } else if attributes.contains(&"schema_value".to_string())
                || attributes.contains(&"property_value".to_string())
                || attributes.contains(&"implementation_value".to_string())
                || attributes.contains(&"implementation_settings_value".to_string())
            {
                if attributes.len() != 1 {
                    return Err("table field requires exactly one value source".into());
                }
                if field.default.is_some() {
                    return Err("table mapping cannot also declare a default".into());
                }
            } else {
                let Some(value) = &field.default else {
                    return Err(format!("table field `{}` requires a value", field.name));
                };
                let Expr::Num(number, _) = value.node else {
                    return Err("table defaults require a u32 literal".into());
                };
                if !number.is_finite()
                    || number.fract() != 0.0
                    || !(0.0..=f64::from(u32::MAX)).contains(&number)
                {
                    return Err("table default outside u32 range".into());
                }
            }
        }
        let mut selected: Vec<_> = materials.iter().collect();
        if attr.args[1] != "source" {
            selected.sort_by(|a, b| a.name.cmp(&b.name));
            if attr.args[1] == "descending" {
                selected.reverse();
            }
        }
        let mut keys = BTreeSet::new();
        let mut records = Vec::new();
        for (offset, material) in selected.into_iter().enumerate() {
            if !keys.insert(&material.name) {
                return Err(format!("duplicate table key `{}`", material.name));
            }
            let index = u32::try_from(offset)
                .ok()
                .and_then(|o| first_index.checked_add(o))
                .ok_or("table index exceeds u32")?;
            let mut values = Vec::new();
            for field in &record.fields {
                if field.ty_name != "u32" {
                    return Err("table records currently support u32 fields".into());
                }
                if field.name == index_field.name {
                    values.push(index);
                    continue;
                }
                if let Some((name, args)) = field.attrs.iter().find(|(name, _)| {
                    matches!(
                        name.as_str(),
                        "implementation_value" | "implementation_settings_value"
                    )
                }) {
                    if !material
                        .settings
                        .as_ref()
                        .is_some_and(|s| s.implementation_slots.contains(&args[0]))
                    {
                        return Err(format!(
                            "unknown implementation property `{}` for `{}`",
                            args[0], material.name
                        ));
                    }
                    let selection = material.settings.as_ref().and_then(|settings| {
                        settings.implementations.iter().find(|s| s.name == args[0])
                    });
                    // Zero denotes an entry to which this optional contract does not apply.
                    values.push(selection.map_or(0, |s| {
                        if name == "implementation_settings_value" {
                            s.settings_offset
                        } else {
                            s.id
                        }
                    }));
                    continue;
                }
                if let Some((_, args)) = field
                    .attrs
                    .iter()
                    .find(|(name, _)| name == "property_value")
                {
                    let value = material.settings.as_ref().and_then(|settings| settings.property_u32.get(&args[0]))
                        .ok_or_else(|| format!("table property `{}` for surface `{}` must be a checked u32 or nonnegative enum", args[0], material.name))?;
                    values.push(*value);
                    continue;
                }
                let cases: Vec<_> = field
                    .attrs
                    .iter()
                    .filter(|(a, _)| a == "schema_value")
                    .collect();
                if let [(_, args)] = cases.as_slice() {
                    if args.is_empty() || args.len() % 2 != 0 {
                        return Err("@schema_value requires (schema, value) pairs".into());
                    }
                    let mut seen = BTreeSet::new();
                    let mut matches = Vec::new();
                    for pair in args.as_chunks::<2>().0 {
                        if !seen.insert(&pair[0])
                            || !material.material_schemas.contains_key(&pair[0])
                        {
                            return Err(format!("duplicate or unknown table schema `{}`", pair[0]));
                        }
                        let value = pair[1]
                            .parse::<u32>()
                            .map_err(|_| "table schema value must be a u32")?;
                        let mut actual = material.material_properties_name.as_deref();
                        for distance in 0..=material.material_schemas.len() {
                            let Some(name) = actual else {
                                break;
                            };
                            if name == pair[0] {
                                matches.push((distance, value));
                                break;
                            }
                            actual = material
                                .material_schemas
                                .get(name)
                                .and_then(Option::as_deref);
                        }
                    }
                    matches.sort();
                    let Some((_, value)) = matches.first() else {
                        return Err(format!(
                            "no table value for surface `{}` field `{}`",
                            material.name, field.name
                        ));
                    };
                    values.push(*value);
                } else if cases.is_empty() {
                    let Some(value) = &field.default else {
                        return Err(format!("table field `{}` requires a value", field.name));
                    };
                    let Expr::Num(number, _) = value.node else {
                        return Err("table defaults require a u32 literal".into());
                    };
                    if number.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&number) {
                        return Err("table default outside u32 range".into());
                    }
                    values.push(number as u32);
                } else {
                    return Err("duplicate table field mapping".into());
                }
            }
            records.push(ManifestTableRecord {
                key: material.name.clone(),
                index,
                values,
            });
        }
        tables.push(ManifestTable {
            name: record.name.clone(),
            index_field: index_field.name.clone(),
            first_index,
            fields: record.fields.iter().map(|f| f.name.clone()).collect(),
            records,
        });
    }
    Ok(tables)
}
