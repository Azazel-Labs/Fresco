//! Validate the compiler's resource DAG before creating GPU resources.
use std::collections::{BTreeMap, BTreeSet};

use fresco_artifact::{ManifestIntermediateTarget, ManifestPassPlan};

use super::RuntimeError;

/// An owned, validated plan. Pass IDs are identities, not vector indices.
#[derive(Debug)]
pub struct ValidatedPassPlan {
    plan: ManifestPassPlan,
    order: Vec<usize>,
    final_pass: usize,
}

impl ValidatedPassPlan {
    pub fn new(plan: &ManifestPassPlan) -> Result<Self, RuntimeError> {
        let invalid = |message: String| RuntimeError::PassPlan(message);
        if plan.passes.is_empty() {
            return Err(invalid("no passes".into()));
        }
        let mut passes = BTreeMap::new();
        for (index, pass) in plan.passes.iter().enumerate() {
            if passes.insert(pass.id, index).is_some() {
                return Err(invalid(format!("duplicate pass {}", pass.id)));
            }
            if pass.entry_point.is_empty() {
                return Err(invalid(format!("pass {} has no entry point", pass.id)));
            }
        }
        let mut targets = BTreeMap::new();
        for target in &plan.targets {
            if targets.insert(target.id, target).is_some() {
                return Err(invalid(format!("duplicate target {}", target.id)));
            }
            if !target.scale.is_finite() || target.scale <= 0.0 {
                return Err(invalid(format!("target {} has invalid scale", target.id)));
            }
            if target.lifetime != "transient" {
                return Err(invalid(format!(
                    "target {} requires unsupported lifetime {}",
                    target.id, target.lifetime
                )));
            }
            // This pass path produces floating-point color and samples its
            // intermediates. Device-specific format support is checked at allocation.
            if !fresco_artifact::types::ImageFormat::parse(&target.format).is_some_and(|format| {
                let info = format.info();
                info.scalar == fresco_artifact::types::ImageScalar::F32
                    && info.supports(fresco_artifact::types::ImageUse::ColorAttachment)
            }) {
                return Err(invalid(format!(
                    "target {} requires unsupported format {}",
                    target.id, target.format
                )));
            }
        }
        let mut writers = BTreeMap::new();
        let mut final_pass = None;
        for pass in &plan.passes {
            if let Some(target) = pass.output_target {
                if !targets.contains_key(&target) {
                    return Err(invalid(format!(
                        "pass {} writes missing target {target}",
                        pass.id
                    )));
                }
                if writers.insert(target, pass.id).is_some() {
                    return Err(invalid(format!("target {target} has multiple writers")));
                }
            } else if final_pass.replace(pass.id).is_some() {
                return Err(invalid("multiple presentation passes".into()));
            }
        }
        let final_pass = final_pass.ok_or_else(|| invalid("no presentation pass".into()))?;
        if writers.len() != targets.len() {
            return Err(invalid("intermediate target has no writer".into()));
        }
        let mut edges = BTreeSet::new();
        let mut incoming: BTreeMap<_, usize> = passes.keys().map(|id| (*id, 0)).collect();
        let mut outgoing: BTreeMap<_, Vec<usize>> = BTreeMap::new();
        for edge in &plan.edges {
            if !passes.contains_key(&edge.from) || !passes.contains_key(&edge.to) {
                return Err(invalid(format!(
                    "edge {} -> {} references missing pass",
                    edge.from, edge.to
                )));
            }
            if !edges.insert((edge.from, edge.to)) {
                return Err(invalid(format!(
                    "duplicate edge {} -> {}",
                    edge.from, edge.to
                )));
            }
            if edge.from == final_pass {
                return Err(invalid(
                    "presentation pass cannot supply an intermediate input".into(),
                ));
            }
            *incoming.get_mut(&edge.to).expect("validated pass") += 1;
            outgoing.entry(edge.from).or_default().push(edge.to);
        }
        let mut input_edges = BTreeSet::new();
        for pass in &plan.passes {
            let mut bindings = BTreeSet::new();
            for input in &pass.inputs {
                if writers.get(&input.target_id) != Some(&input.from_pass) {
                    return Err(invalid(format!(
                        "pass {} input target {} does not belong to producer {}",
                        pass.id, input.target_id, input.from_pass
                    )));
                }
                if !bindings.insert(input.binding) {
                    return Err(invalid(format!(
                        "pass {} has duplicate input binding {}",
                        pass.id, input.binding
                    )));
                }
                if !input_edges.insert((input.from_pass, pass.id)) {
                    return Err(invalid(format!(
                        "pass {} repeats producer {}",
                        pass.id, input.from_pass
                    )));
                }
            }
            if pass.id != final_pass && !outgoing.contains_key(&pass.id) {
                return Err(invalid(format!(
                    "pass {} does not contribute to presentation",
                    pass.id
                )));
            }
        }
        if edges != input_edges {
            return Err(invalid(
                "dependency edges and resource inputs disagree".into(),
            ));
        }
        let mut ready: BTreeSet<_> = incoming
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(*id))
            .collect();
        let mut order = Vec::with_capacity(passes.len());
        while let Some(id) = ready.pop_first() {
            order.push(passes[&id]);
            if let Some(consumers) = outgoing.get(&id) {
                for consumer in consumers {
                    let count = incoming.get_mut(consumer).expect("validated consumer");
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(*consumer);
                    }
                }
            }
        }
        if order.len() != passes.len() {
            return Err(invalid("dependency cycle".into()));
        }
        Ok(Self {
            plan: plan.clone(),
            order,
            final_pass,
        })
    }

    pub fn passes(&self) -> impl Iterator<Item = &fresco_artifact::ManifestPass> {
        self.order.iter().map(|index| &self.plan.passes[*index])
    }

    pub fn final_pass(&self) -> usize {
        self.final_pass
    }

    /// Resolve all sizes before allocation, so invalid resizes cannot partially
    /// replace an installed target set. Zero-sized hosts suspend rendering.
    pub fn target_sizes(
        &self,
        size: [u32; 2],
        max_dimension: u32,
    ) -> Result<Vec<(&ManifestIntermediateTarget, [u32; 2])>, RuntimeError> {
        if size.contains(&0) {
            return Ok(Vec::new());
        }
        self.plan
            .targets
            .iter()
            .map(|target| {
                let mut dimensions = [0; 2];
                for (out, source) in dimensions.iter_mut().zip(size) {
                    let scaled = (f64::from(source) * f64::from(target.scale))
                        .round()
                        .max(1.0);
                    if scaled > f64::from(max_dimension) {
                        return Err(RuntimeError::PassPlan(format!(
                            "target {} exceeds device dimension limit",
                            target.id
                        )));
                    }
                    *out = scaled as u32;
                }
                Ok((target, dimensions))
            })
            .collect()
    }
}
