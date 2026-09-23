//! Resolve an explicit compile-known selection without rewriting shader artifacts.
use std::collections::{BTreeMap, BTreeSet};

use fresco_artifact::ManifestEnginePass;

use super::RuntimeError;

pub type VariantSelection = BTreeMap<String, String>;

#[derive(Debug)]
pub struct SelectedStages<'a> {
    pub vertex: &'a str,
    pub fragment: &'a str,
}

pub fn select<'a>(
    entry: &str,
    pass: &'a ManifestEnginePass,
    selection: Option<&VariantSelection>,
) -> Result<SelectedStages<'a>, RuntimeError> {
    let invalid = |reason: &str| RuntimeError::CanvasContract {
        entry: entry.into(),
        reason: reason.into(),
    };
    for variant in &pass.variants {
        let mut axes = BTreeSet::new();
        for binding in &variant.bindings {
            if !axes.insert(&binding.axis) {
                return Err(invalid("pass variant contains duplicate axis bindings"));
            }
        }
    }
    // A sole specialization is unambiguous, but an explicit selection must
    // still match all of its bindings exactly.
    let implicit = match pass.variants.as_slice() {
        [variant] if selection.is_none() && !variant.bindings.is_empty() => Some(
            variant
                .bindings
                .iter()
                .map(|binding| (binding.axis.clone(), binding.value.clone()))
                .collect::<VariantSelection>(),
        ),
        _ => None,
    };
    let selection = selection.or(implicit.as_ref());
    let has_axes = pass
        .variants
        .iter()
        .any(|variant| !variant.bindings.is_empty());
    if !has_axes && pass.variants.len() > 1 {
        return Err(invalid("pass variant selection is ambiguous"));
    }
    let stages = if !has_axes && selection.is_none_or(BTreeMap::is_empty) {
        SelectedStages {
            vertex: &pass.vertex_entry,
            fragment: &pass.fragment_entry,
        }
    } else {
        let selection =
            selection.ok_or_else(|| invalid("an explicit pass variant selection is required"))?;
        let mut matched = None;
        for variant in &pass.variants {
            if variant.bindings.len() == selection.len()
                && variant
                    .bindings
                    .iter()
                    .all(|binding| selection.get(&binding.axis) == Some(&binding.value))
                && matched.replace(variant).is_some()
            {
                return Err(invalid("pass variant selection is ambiguous"));
            }
        }
        let variant =
            matched.ok_or_else(|| invalid("no pass variant matches the complete selection"))?;
        SelectedStages {
            vertex: &variant.vertex_entry,
            fragment: &variant.fragment_entry,
        }
    };
    if stages.vertex.is_empty() || stages.fragment.is_empty() {
        return Err(invalid("selected pass variant has an empty stage entry"));
    }
    Ok(stages)
}
