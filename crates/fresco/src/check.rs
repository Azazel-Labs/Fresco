//! Semantic checking: AST -> typed HIR.
//!
//! This is where Fresco's "every value knows what it is" pillar lives.
//! Expressions evaluate to one of a small set of semantic values (shape,
//! layer, scalar, vec2, color); builtins have typed signatures with named or
//! positional arguments; and every mismatch is reported with a span and,
//! where possible, a fix-it hint.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::f64::consts::PI;
use std::time::Duration;

// Typed evaluation recursively visits expressions and authored function bodies.
// Grow native stacks before entering those large frames, independently of the
// embedding application's thread size. Semantic recursion limits still apply.
fn with_checker_stack<T>(evaluate: impl FnOnce() -> T) -> T {
    #[cfg(not(target_arch = "wasm32"))]
    {
        stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, evaluate)
    }
    #[cfg(target_arch = "wasm32")]
    {
        evaluate()
    }
}

use crate::ast::*;
use crate::diag::{Diag, Severity};
use crate::hir::{
    self, Blend, CenteredMode, ColorSource, GlowColorSpace, GlowFalloff, GlowReach, GradientAnchor,
    GradientKind, GradientStop, Hir, Layer, LayerId, Param, ParamDefault, Shape, ShapeId, Sx, V2,
    Xform,
};
use crate::pipeline_layout::canonical_layout_identity;

pub(crate) mod axes;
pub(crate) mod bindings;
mod blocks;
mod builtin_dispatch;
mod builtins;
mod cellular;
mod coercions;
pub(crate) mod compute;
mod declarations;
mod effect_locality;
mod entry_context;
mod expr;
mod functions;
mod globals;
mod params;
mod pass_hooks;
pub(crate) mod shader_services;
mod surface;

/// Semantic type of an expression at a given source span.  Used by the
/// workbook API to drive type-directed visualization (§21.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanValueKind {
    Layer,
    Shape,
    Color,
    ColorField,
    Scalar,
    Vec2,
    Vec3,
    Vec4,
    Space,
}

impl SpanValueKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SpanValueKind::Layer => "layer",
            SpanValueKind::Shape => "shape",
            SpanValueKind::Color => "color",
            SpanValueKind::ColorField => "color",
            SpanValueKind::Scalar => "scalar",
            SpanValueKind::Vec2 => "vec2",
            SpanValueKind::Vec3 => "vec3",
            SpanValueKind::Vec4 => "vec4",
            SpanValueKind::Space => "space",
        }
    }
}

/// The semantic value captured at a target span.  Carries enough information
/// to synthesize a variant canvas entry returning that value (§21.7 rung 1).
#[derive(Debug, Clone)]
pub enum SpanCapture {
    Layer(LayerId),
    Shape(ShapeId),
    Color([f32; 4]),
    ColorField([Sx; 4]),
    Scalar(Sx),
    Vec2(V2),
    Vec3((Sx, Sx, Sx)),
    Vec4((Sx, Sx, Sx, Sx)),
    Space(Vec<Xform>),
}

pub use declarations::{check, validate_vertex_contracts};
pub(crate) use declarations::{
    validate_conformance_method_bodies, validate_style_contract_signatures,
};
pub use functions::check_with_workbook;
pub(crate) use globals::GlobalUniformDef;
pub(crate) use surface::{
    check_surface, validate_material_models, validate_material_properties,
    validate_surface_model_bindings,
};

pub(crate) fn space_transform_names() -> &'static [&'static str] {
    blocks::SPACE_TRANSFORM_NAMES
}

fn normalize_pipeline_semantic_token(value: &str) -> String {
    canonical_layout_identity(value)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect()
}

fn validate_pipeline_attr_target(
    attr: &PipelineAttribute,
    target_kind: &str,
    allowed_attr_names: &[&str],
    diags: &mut Vec<Diag>,
) {
    if allowed_attr_names.contains(&attr.name.as_str()) {
        return;
    }

    // Generic @thing(...) metadata is parser-supported, but staged checker
    // only recognizes a small allow-list per target for now.
    diags.push(
        Diag::error(
            attr.name_span.clone(),
            format!(
                "attribute `@{}` is not currently allowed on {target_kind}",
                attr.name
            ),
        )
        .with_help(
            "attributes are parsed generically; checker allow-lists are staged and will expand with pipeline lowering",
        ),
    );
}

fn has_attr(pass_attrs: &[PipelineAttribute], attr_name: &str) -> bool {
    pass_attrs.iter().any(|attr| attr.name == attr_name)
}

fn attr_has_editor_config(attr: &PipelineAttribute) -> bool {
    if attr.name == "config_editor" {
        return true;
    }
    if attr.name != "config" {
        return false;
    }
    attr.args
        .first()
        .is_some_and(|arg| arg.eq_ignore_ascii_case("editor"))
}

#[derive(Debug, Clone)]
struct KnownAxisDecl {
    name: String,
}

#[derive(Debug, Clone)]
struct PipelineAxisDecl {
    pipeline_name: String,
    pass_name: String,
    signature_key: String,
    axis_span: Span,
}

fn pass_error(pass_decl: &PassDecl, span: Span, message: impl Into<String>) -> Diag {
    Diag::error(span, message).with_file(pass_decl.source_file.clone())
}

#[derive(Debug, Clone)]
enum PassValue {
    Number(f64),
    Symbol(String),
    Bool(bool),
    Set(Vec<PassValue>),
}

#[derive(Debug, Clone)]
struct DerivedLayoutDependencyEdge {
    pipeline_name: String,
    consumer_pass: String,
    producer_pass: String,
    consumer_decl_span: Span,
    producer_decl_span: Span,
    read_span: Span,
    signature: String,
}

#[derive(Debug, Clone, Default)]
struct DerivedLayoutDependencyGraph {
    axes_by_pass_and_signature: HashMap<String, HashMap<String, Vec<KnownAxisDecl>>>,
    edges: Vec<DerivedLayoutDependencyEdge>,
}

fn pass_value_from_text(text: &str) -> PassValue {
    if text == "true" {
        PassValue::Bool(true)
    } else if text == "false" {
        PassValue::Bool(false)
    } else if let Ok(value) = text.parse::<f64>() {
        PassValue::Number(value)
    } else {
        PassValue::Symbol(text.to_string())
    }
}

fn pass_value_truthy(value: &PassValue) -> Option<bool> {
    match value {
        PassValue::Bool(value) => Some(*value),
        PassValue::Number(value) => Some(*value != 0.0),
        PassValue::Symbol(symbol) if symbol == "true" => Some(true),
        PassValue::Symbol(symbol) if symbol == "false" => Some(false),
        PassValue::Symbol(_) | PassValue::Set(_) => None,
    }
}

fn pass_value_as_number(value: &PassValue) -> Option<f64> {
    match value {
        PassValue::Number(value) => Some(*value),
        PassValue::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        PassValue::Symbol(symbol) => symbol.parse::<f64>().ok(),
        PassValue::Set(_) => None,
    }
}

fn pass_value_set_items(value: PassValue) -> Vec<PassValue> {
    match value {
        PassValue::Set(values) => values,
        other => vec![other],
    }
}

fn pass_value_eq(lhs: &PassValue, rhs: &PassValue) -> bool {
    if let PassValue::Set(rhs_values) = rhs {
        return rhs_values
            .iter()
            .any(|candidate| pass_value_eq(lhs, candidate));
    }
    if let PassValue::Set(lhs_values) = lhs {
        return lhs_values
            .iter()
            .any(|candidate| pass_value_eq(candidate, rhs));
    }

    match (lhs, rhs) {
        (PassValue::Bool(a), PassValue::Bool(b)) => a == b,
        (PassValue::Number(a), PassValue::Number(b)) => a == b,
        (PassValue::Bool(a), other) | (other, PassValue::Bool(a)) => {
            pass_value_as_number(other).is_some_and(|b| f64::from(u8::from(*a)) == b)
        }
        (PassValue::Number(a), other) | (other, PassValue::Number(a)) => {
            pass_value_as_number(other).is_some_and(|b| *a == b)
        }
        (PassValue::Symbol(a), PassValue::Symbol(b)) => a == b,
        _ => false,
    }
}

fn pass_eval_expr(expr: &SExpr, env: &HashMap<String, String>) -> Option<PassValue> {
    match &expr.node {
        Expr::Num(value, _) => Some(PassValue::Number(*value)),
        Expr::Color(_) => None,
        Expr::Str(value) => Some(PassValue::Symbol(value.clone())),
        Expr::Var(name) => env
            .get(name)
            .map(|value| pass_value_from_text(value))
            .or_else(|| Some(pass_value_from_text(name))),
        Expr::Unary(UnOp::Neg, inner) => pass_eval_expr(inner, env)
            .and_then(|value| pass_value_as_number(&value))
            .map(|value| PassValue::Number(-value)),
        Expr::Binary(op, lhs, rhs) => {
            let left = pass_eval_expr(lhs, env)?;
            let right = pass_eval_expr(rhs, env)?;
            match op {
                BinOp::LogicalAnd => Some(PassValue::Bool(
                    pass_value_truthy(&left)? && pass_value_truthy(&right)?,
                )),
                BinOp::LogicalOr => Some(PassValue::Bool(
                    pass_value_truthy(&left)? || pass_value_truthy(&right)?,
                )),
                BinOp::Add => Some(PassValue::Number(
                    pass_value_as_number(&left)? + pass_value_as_number(&right)?,
                )),
                BinOp::Sub => Some(PassValue::Number(
                    pass_value_as_number(&left)? - pass_value_as_number(&right)?,
                )),
                BinOp::Mul => match (pass_value_truthy(&left), pass_value_truthy(&right)) {
                    (Some(a), Some(b)) => Some(PassValue::Bool(a && b)),
                    _ => Some(PassValue::Number(
                        pass_value_as_number(&left)? * pass_value_as_number(&right)?,
                    )),
                },
                BinOp::Div => Some(PassValue::Number(
                    pass_value_as_number(&left)? / pass_value_as_number(&right)?,
                )),
                BinOp::Mod => Some(PassValue::Number(
                    pass_value_as_number(&left)? % pass_value_as_number(&right)?,
                )),
                BinOp::Gt => Some(PassValue::Bool(
                    pass_value_as_number(&left)? > pass_value_as_number(&right)?,
                )),
                BinOp::Ge => Some(PassValue::Bool(
                    pass_value_as_number(&left)? >= pass_value_as_number(&right)?,
                )),
                BinOp::Lt => Some(PassValue::Bool(
                    pass_value_as_number(&left)? < pass_value_as_number(&right)?,
                )),
                BinOp::Le => Some(PassValue::Bool(
                    pass_value_as_number(&left)? <= pass_value_as_number(&right)?,
                )),
                BinOp::Eq => Some(PassValue::Bool(pass_value_eq(&left, &right))),
                BinOp::Ne => Some(PassValue::Bool(!pass_value_eq(&left, &right))),
                BinOp::Union => {
                    let mut values = pass_value_set_items(left);
                    values.extend(pass_value_set_items(right));
                    Some(PassValue::Set(values))
                }
                _ => None,
            }
        }
        Expr::Call { .. } => None,
        _ => None,
    }
}

fn permutation_signature_key(permutation: &PassPermutationDecl) -> String {
    let mut signature = permutation
        .values
        .iter()
        .map(|value| value.node.clone())
        .collect::<Vec<_>>()
        .join("|");
    if permutation.when_guard.is_some() {
        signature.push_str("|when");
    }
    if let Some(else_value) = &permutation.else_value {
        signature.push_str("|else:");
        signature.push_str(&else_value.node);
    }
    canonical_layout_identity(&signature)
}

fn collect_known_axes_by_permutation_signature(
    pass_decl: &PassDecl,
) -> HashMap<String, Vec<KnownAxisDecl>> {
    let mut grouped: HashMap<String, Vec<KnownAxisDecl>> = HashMap::new();

    for permutation in &pass_decl.permutations {
        let signature_key = permutation_signature_key(permutation);
        for attr in &permutation.attrs {
            if attr.name != "known" {
                continue;
            }
            grouped
                .entry(signature_key.clone())
                .or_default()
                .push(KnownAxisDecl {
                    name: permutation.name.clone(),
                });
        }
    }

    grouped
}

fn collect_pipeline_axis_decls(
    pipeline_decl: &PipelineDecl,
    pass_by_name: &HashMap<&str, &PassDecl>,
) -> HashMap<String, Vec<PipelineAxisDecl>> {
    let mut axes_by_name: HashMap<String, Vec<PipelineAxisDecl>> = HashMap::new();

    for pass_ref in &pipeline_decl.passes {
        let Some(pass_decl) = pass_by_name.get(pass_ref.node.as_str()).copied() else {
            continue;
        };

        for permutation in &pass_decl.permutations {
            if !permutation.attrs.iter().any(|attr| attr.name == "known") {
                continue;
            }

            axes_by_name
                .entry(permutation.name.clone())
                .or_default()
                .push(PipelineAxisDecl {
                    pipeline_name: pipeline_decl.name.clone(),
                    pass_name: pass_decl.name.clone(),
                    signature_key: permutation_signature_key(permutation),
                    axis_span: permutation.span.clone(),
                });
        }
    }

    axes_by_name
}

fn validate_pipeline_axis_agreement(program: &Program, diags: &mut Vec<Diag>) {
    let pass_by_name: HashMap<&str, &PassDecl> = program
        .passes
        .iter()
        .map(|pass| (pass.name.as_str(), pass))
        .collect();

    for pipeline_decl in &program.pipelines {
        let axes_by_name = collect_pipeline_axis_decls(pipeline_decl, &pass_by_name);

        for (axis_name, decls) in axes_by_name {
            if decls.len() < 2 {
                continue;
            }

            let mut signatures_by_pass: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for decl in &decls {
                signatures_by_pass
                    .entry(decl.signature_key.clone())
                    .or_default()
                    .push(decl.pass_name.clone());
            }

            if signatures_by_pass.len() <= 1 {
                continue;
            }

            let primary = &decls[0];
            let mut signature_chunks = Vec::new();
            for (signature, passes) in signatures_by_pass {
                signature_chunks.push(format!("{signature} in {}", passes.join(", ")));
            }

            diags.push(
                pass_error(
                    pass_by_name
                        .get(primary.pass_name.as_str())
                        .copied()
                        .unwrap_or_else(|| panic!("missing pass `{}` in pipeline axis agreement", primary.pass_name)),
                    primary.axis_span.clone(),
                    format!(
                        "pipeline `{}` axis `{}` must keep the same permutation domain across every pass that defines it; found {}",
                        primary.pipeline_name,
                        axis_name,
                        signature_chunks.join("; ")
                    ),
                )
                .with_label("shared pipeline axis")
                .with_help(
                    "shared axes in one pipeline need matching permutation shapes and value domains across all passes that define them",
                ),
            );
        }
    }
}

fn expand_pass_variants(pass_decl: &PassDecl) -> Vec<HashMap<String, String>> {
    fn recurse(
        permutations: &[PassPermutationDecl],
        index: usize,
        current: &mut HashMap<String, String>,
        out: &mut Vec<HashMap<String, String>>,
    ) {
        if index == permutations.len() {
            out.push(current.clone());
            return;
        }

        let permutation = &permutations[index];
        let condition_matches = permutation
            .when_guard
            .as_ref()
            .and_then(|expr| pass_eval_expr(expr, current))
            .and_then(|value| pass_value_truthy(&value))
            .unwrap_or(true);

        if condition_matches {
            for value in &permutation.values {
                current.insert(permutation.name.clone(), value.node.clone());
                recurse(permutations, index + 1, current, out);
                current.remove(&permutation.name);
            }
        } else if let Some(else_value) = &permutation.else_value {
            current.insert(permutation.name.clone(), else_value.node.clone());
            recurse(permutations, index + 1, current, out);
            current.remove(&permutation.name);
        } else {
            recurse(permutations, index + 1, current, out);
        }
    }

    let mut out = Vec::new();
    let mut current = HashMap::new();
    recurse(&pass_decl.permutations, 0, &mut current, &mut out);
    out
}

pub(crate) fn active_pass_variants(pass_decl: &PassDecl) -> Vec<HashMap<String, String>> {
    let mut variants = expand_pass_variants(pass_decl);
    for requirement in &pass_decl.requirements {
        let is_disable_clause = matches!(
            &requirement.clause.constraint_expr().node,
            Expr::Call { name, args, .. } if name == "disable" && !args.is_empty()
        );
        variants.retain(|variant| {
            let guard_matches = requirement
                .clause
                .guard_expr()
                .and_then(|guard| pass_eval_expr(guard, variant))
                .and_then(|value| pass_value_truthy(&value))
                .unwrap_or(true);
            if !guard_matches {
                return true;
            }
            if is_disable_clause {
                return false;
            }
            pass_eval_expr(requirement.clause.constraint_expr(), variant)
                .and_then(|value| pass_value_truthy(&value))
                .unwrap_or(false)
        });
    }
    variants
}

fn validate_pass_requirements(pass_decl: &PassDecl, diags: &mut Vec<Diag>) {
    if pass_decl.requirements.is_empty() || pass_decl.permutations.is_empty() {
        return;
    }

    let variants = expand_pass_variants(pass_decl);
    if variants.is_empty() {
        return;
    }

    let active_variants = active_pass_variants(pass_decl);

    if active_variants.is_empty() {
        let prune_kind = if pass_decl.requirements.iter().any(|requirement| {
            matches!(
                &requirement.clause.constraint_expr().node,
                Expr::Call { name, args, .. } if name == "disable" && !args.is_empty()
            )
        }) {
            "`disable(...)` clause"
        } else {
            "`require` clause"
        };

        diags.push(
            pass_error(
                pass_decl,
                pass_decl.span.clone(),
                format!(
                    "pass `{}` has no surviving permutation variants after a matching {prune_kind} pruned the last one",
                    pass_decl.name,
                ),
            )
            .with_help(
                "relax the matching `require { ... }` / `disable(...)` condition, or add more permutation values so at least one variant survives",
            ),
        );
    }
}

fn build_derived_layout_dependency_graph(program: &Program) -> DerivedLayoutDependencyGraph {
    let pass_by_name: HashMap<&str, &PassDecl> = program
        .passes
        .iter()
        .map(|pass| (pass.name.as_str(), pass))
        .collect();

    let mut graph = DerivedLayoutDependencyGraph::default();
    for pass in &program.passes {
        graph.axes_by_pass_and_signature.insert(
            pass.name.clone(),
            collect_known_axes_by_permutation_signature(pass),
        );
    }

    for pipeline_decl in &program.pipelines {
        for pass_ref in &pipeline_decl.passes {
            let Some(consumer_pass) = pass_by_name.get(pass_ref.node.as_str()).copied() else {
                continue;
            };

            let Some(consumer_signatures) = graph
                .axes_by_pass_and_signature
                .get(consumer_pass.name.as_str())
            else {
                continue;
            };
            if consumer_signatures.is_empty() {
                continue;
            }

            for read_ref in &consumer_pass.reads {
                let Some(producer_pass) = pass_by_name.get(read_ref.node.as_str()).copied() else {
                    continue;
                };
                let Some(producer_signatures) = graph
                    .axes_by_pass_and_signature
                    .get(producer_pass.name.as_str())
                else {
                    continue;
                };
                if producer_signatures.is_empty() {
                    continue;
                }

                for signature in producer_signatures.keys() {
                    if !consumer_signatures.contains_key(signature) {
                        continue;
                    }
                    graph.edges.push(DerivedLayoutDependencyEdge {
                        pipeline_name: pipeline_decl.name.clone(),
                        consumer_pass: consumer_pass.name.clone(),
                        producer_pass: producer_pass.name.clone(),
                        consumer_decl_span: consumer_pass.name_span.clone(),
                        producer_decl_span: producer_pass.name_span.clone(),
                        read_span: read_ref.span.clone(),
                        signature: signature.clone(),
                    });
                }
            }
        }
    }

    graph
}

fn validate_pipeline_known_axis_agreement(program: &Program, diags: &mut Vec<Diag>) {
    let graph = build_derived_layout_dependency_graph(program);

    for edge in &graph.edges {
        let Some(producer_axes_by_signature) = graph
            .axes_by_pass_and_signature
            .get(edge.producer_pass.as_str())
        else {
            continue;
        };
        let Some(consumer_axes_by_signature) = graph
            .axes_by_pass_and_signature
            .get(edge.consumer_pass.as_str())
        else {
            continue;
        };
        let Some(producer_axes) = producer_axes_by_signature.get(edge.signature.as_str()) else {
            continue;
        };
        let Some(consumer_axes) = consumer_axes_by_signature.get(edge.signature.as_str()) else {
            continue;
        };

        let producer_set: HashSet<&str> = producer_axes
            .iter()
            .map(|axis| axis.name.as_str())
            .collect();
        let consumer_set: HashSet<&str> = consumer_axes
            .iter()
            .map(|axis| axis.name.as_str())
            .collect();

        let missing_in_consumer = producer_set
            .difference(&consumer_set)
            .map(|axis| (*axis).to_string())
            .collect::<BTreeSet<_>>();
        let missing_in_producer = consumer_set
            .difference(&producer_set)
            .map(|axis| (*axis).to_string())
            .collect::<BTreeSet<_>>();

        if missing_in_consumer.is_empty() && missing_in_producer.is_empty() {
            continue;
        }

        let mut mismatch_chunks = Vec::new();
        if !missing_in_consumer.is_empty() {
            mismatch_chunks.push(format!(
                "missing in `{}`: {}",
                edge.consumer_pass,
                missing_in_consumer
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !missing_in_producer.is_empty() {
            mismatch_chunks.push(format!(
                "missing in `{}`: {}",
                edge.producer_pass,
                missing_in_producer
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        let producer_binding_names = producer_axes
            .iter()
            .map(|axis| axis.name.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        let consumer_binding_names = consumer_axes
            .iter()
            .map(|axis| axis.name.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");

        diags.push(
            Diag::error(
                edge.read_span.clone(),
                format!(
                    "pipeline `{}` layout dependency mismatch: `{}` reads `{}` with shared layout signature `{}` but declared @known axes diverge ({}) [producer bindings: {}; consumer bindings: {}]",
                    edge.pipeline_name,
                    edge.consumer_pass,
                    edge.producer_pass,
                    edge.signature,
                    mismatch_chunks.join("; "),
                    producer_binding_names,
                    consumer_binding_names
                ),
            )
            .with_label("shared layout read edge")
            .with_related_label(
                edge.consumer_decl_span.clone(),
                format!("consumer pass `{}` declaration", edge.consumer_pass),
            )
            .with_related_label(
                edge.producer_decl_span.clone(),
                format!("producer pass `{}` declaration", edge.producer_pass),
            )
            .with_help(format!(
                "cause chain: `{} -> reads: {}` plus shared binding signature `{}` requires matching @known axes; pair format matches manifest debug output: [signature_id: {}, producer_pass: {}, consumer_pass: {}]",
                edge.consumer_pass,
                edge.producer_pass,
                edge.signature,
                producer_binding_names,
                edge.consumer_pass,
                edge.producer_pass
            )),
        );
    }
}

fn resolve_vertex_format_members_for_pass(
    format_name: &str,
    formats_by_name: &HashMap<&str, &VertexFormatDecl>,
    visiting: &mut HashSet<String>,
) -> Option<Vec<VertexInterfaceMemberDecl>> {
    if !visiting.insert(format_name.to_string()) {
        return None;
    }

    let format_decl = formats_by_name.get(format_name)?;
    let mut members = Vec::new();
    if let Some(parent) = &format_decl.parent {
        {
            let mut inherited =
                resolve_vertex_format_members_for_pass(parent, formats_by_name, visiting)?;
            members.append(&mut inherited);
        }
    }

    members.extend(format_decl.members.iter().cloned());
    visiting.remove(format_name);
    Some(members)
}

fn interface_member_satisfied_by_format_member(
    iface_member: &VertexInterfaceMemberDecl,
    format_member: Option<&VertexInterfaceMemberDecl>,
) -> bool {
    let Some(format_member) = format_member else {
        return iface_member.optional && iface_member.default.is_some();
    };

    if type_name_to_key(&iface_member.ty_name) != type_name_to_key(&format_member.ty_name) {
        return false;
    }

    // Optional members only satisfy contracts when the fallback value is explicit.
    if format_member.optional && format_member.default.is_none() {
        return false;
    }

    true
}

fn validate_pass_vertex_factory_contracts(program: &Program, diags: &mut Vec<Diag>) {
    let interfaces_by_name: HashMap<&str, &VertexInterfaceDecl> = program
        .vertex_interfaces
        .iter()
        .map(|iface| (iface.name.as_str(), iface))
        .collect();
    let formats_by_name: HashMap<&str, &VertexFormatDecl> = program
        .vertex_formats
        .iter()
        .map(|format_decl| (format_decl.name.as_str(), format_decl))
        .collect();

    for pass_decl in &program.passes {
        let Some(vertex_interface) = &pass_decl.vertex_interface else {
            continue;
        };

        let Some(interface_decl) = interfaces_by_name.get(vertex_interface.node.as_str()) else {
            continue;
        };

        let mut eligible_factories = Vec::new();
        for factory in &program.vertex_factories {
            let mut visiting = HashSet::new();
            let Some(resolved_members) = resolve_vertex_format_members_for_pass(
                &factory.target_format,
                &formats_by_name,
                &mut visiting,
            ) else {
                continue;
            };

            let mut satisfies = true;
            for iface_member in &interface_decl.members {
                let format_member = resolved_members
                    .iter()
                    .find(|member| member.name == iface_member.name);
                if !interface_member_satisfied_by_format_member(iface_member, format_member) {
                    satisfies = false;
                    break;
                }
            }

            if satisfies {
                eligible_factories.push(factory.name.clone());
            }
        }

        if eligible_factories.is_empty() {
            diags.push(
                pass_error(
                    pass_decl,
                    vertex_interface.span.clone(),
                    format!(
                        "pass `{}` requires vertex_interface `{}`, but no vertex_factory target format satisfies that contract",
                        pass_decl.name, vertex_interface.node
                    ),
                )
                .with_help(
                    "add or update a vertex_factory whose vertex_format provides all required interface members (optional members must declare defaults)",
                ),
            );
        }
    }
}

fn validate_vertex_factory_bodies(program: &Program, diags: &mut Vec<Diag>) {
    for factory in &program.vertex_factories {
        let mut binding_names = HashSet::new();
        for binding in &factory.bindings {
            if !binding_names.insert(binding.name.as_str()) {
                diags.push(Diag::error(
                    binding.name_span.clone(),
                    format!(
                        "duplicate binding `{}` in vertex factory `{}`",
                        binding.name, factory.name
                    ),
                ));
            }
        }

        if factory.hooks.is_empty() {
            continue;
        }
        let Some(transform) = factory.hooks.iter().find(|hook| hook.name == "transform") else {
            diags.push(
                Diag::error(
                    factory.span.clone(),
                    format!(
                        "executable vertex factory `{}` is missing required `transform` hook",
                        factory.name
                    ),
                )
                .with_help(format!(
                    "add `fn transform(v: {}) -> mat4 {{ ... }}`",
                    factory.target_format
                )),
            );
            continue;
        };
        let valid_signature = transform.params.len() == 1
            && transform.params[0].ty_name == factory.target_format
            && transform
                .return_ty
                .as_ref()
                .is_some_and(|ty| type_name_to_key(&ty.node) == "mat4");
        if !valid_signature {
            diags.push(Diag::error(
                transform.span.clone(),
                format!(
                    "vertex factory `{}` transform hook must have signature `fn transform(v: {}) -> mat4`",
                    factory.name, factory.target_format
                ),
            ));
        }
        if let Err(errors) = crate::parser::pass_hook_body(transform) {
            for error in errors {
                diags.push(Diag::error(
                    error.span().clone(),
                    format!(
                        "invalid body of vertex factory `{}` hook `transform`: {:?}",
                        factory.name,
                        error.reason()
                    ),
                ));
            }
        }
    }
}

pub fn validate_pipeline_skeleton_decls(program: &Program) -> Result<(), Vec<Diag>> {
    let mut diags = Vec::new();
    let known_builtin_pipeline_types: HashSet<&'static str> =
        HashSet::from(["postprocess", "compute"]);
    let declared_pipeline_types: HashSet<String> = program
        .tags
        .iter()
        .filter(|decl| decl.domain == "pipeline")
        .flat_map(|decl| decl.values.iter().map(|value| value.node.clone()))
        .collect();
    let declared_pass_names: HashSet<String> = program
        .passes
        .iter()
        .map(|pass_decl| pass_decl.name.clone())
        .collect();
    let declared_vertex_interfaces: HashSet<String> = program
        .vertex_interfaces
        .iter()
        .map(|iface| iface.name.clone())
        .collect();

    for pass_decl in &program.passes {
        pass_hooks::validate(pass_decl, program, &mut diags);
        shader_services::validate(pass_decl, program, &mut diags);
        for attr in &pass_decl.attrs {
            validate_pipeline_attr_target(
                attr,
                "pass declarations",
                &[
                    "editor_only",
                    "shader",
                    "meta",
                    "factory",
                    "workgroup_size",
                    "service",
                ],
                &mut diags,
            );
        }

        let stage_name = pass_decl
            .stage
            .as_ref()
            .map(|stage| normalize_pipeline_semantic_token(&stage.node));

        if let Some(stage) = &pass_decl.stage {
            if stage.node != "raster" && stage.node != "compute" {
                diags.push(
                    pass_error(
                        pass_decl,
                        stage.span.clone(),
                        format!(
                            "unknown pass stage `{}` in pass `{}`",
                            stage.node, pass_decl.name
                        ),
                    )
                    .with_help("use `stage: raster` or `stage: compute`"),
                );
            }

            if stage.node == "compute" {
                if pass_decl.blend.is_some() {
                    diags.push(
                        pass_error(
                            pass_decl,
                            pass_decl
                                .blend
                                .as_ref()
                                .map(|blend| blend.span.clone())
                                .unwrap_or_else(|| stage.span.clone()),
                            format!(
                                "pass `{}` declares `blend`, but compute passes do not support blending",
                                pass_decl.name
                            ),
                        )
                        .with_help("remove `blend:` or switch the pass stage to `raster`"),
                    );
                }

                if pass_decl.draw.is_some() {
                    diags.push(
                        pass_error(
                            pass_decl,
                            pass_decl
                                .draw
                                .as_ref()
                                .map(|draw| draw.span.clone())
                                .unwrap_or_else(|| stage.span.clone()),
                            format!(
                                "pass `{}` declares `draw`, but compute passes cannot use draw semantics",
                                pass_decl.name
                            ),
                        )
                        .with_help("remove `draw:` or switch the pass stage to `raster`"),
                    );
                }
            }
        }

        if let Some(draw) = &pass_decl.draw {
            let draw_name = normalize_pipeline_semantic_token(&draw.node);
            let repeated_binding = draw
                .node
                .strip_prefix("per_binding(")
                .and_then(|value| value.strip_suffix(')'))
                .is_some_and(|name| {
                    !name.is_empty()
                        && name
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                });
            if stage_name.as_deref() == Some("raster")
                && !repeated_binding
                && !matches!(
                    draw_name.as_str(),
                    "per_object" | "per_binding" | "fullscreen" | "geometry"
                )
            {
                diags.push(
                    pass_error(
                        pass_decl,
                        draw.span.clone(),
                        format!(
                            "unknown draw semantic `{}` in pass `{}`",
                            draw.node, pass_decl.name
                        ),
                    )
                    .with_help(
                        "supported raster draw semantics: `per_object`, `per_binding(name)`, `fullscreen`, `geometry`",
                    ),
                );
            }
        }

        if let Some(blend) = &pass_decl.blend {
            let blend_name = normalize_pipeline_semantic_token(&blend.node);
            if stage_name.as_deref() == Some("raster")
                && !matches!(blend_name.as_str(), "opaque" | "additive" | "over")
            {
                diags.push(
                    pass_error(
                        pass_decl,
                        blend.span.clone(),
                        format!(
                            "unknown blend semantic `{}` in pass `{}`",
                            blend.node, pass_decl.name
                        ),
                    )
                    .with_help("supported raster blend semantics: `opaque`, `additive`, `over`"),
                );
            }
        }

        if let Some(vertex_interface) = &pass_decl.vertex_interface
            && !declared_vertex_interfaces.contains(&vertex_interface.node)
        {
            diags.push(
                    pass_error(
                        pass_decl,
                        vertex_interface.span.clone(),
                        format!(
                            "pass `{}` references unknown vertex_interface `{}` in `fn vertex(...)`",
                            pass_decl.name, vertex_interface.node
                        ),
                    )
                    .with_help("declare the vertex interface with `vertex_interface <Name> { ... }` before the pass"),
                );
        }

        for binding in &pass_decl.bindings {
            for attr in &binding.attrs {
                if attr.name == "geometry_resource"
                    && (pass_decl.prepared_draw.is_some() || pass_decl.preparation.is_some())
                {
                    continue;
                }
                validate_pipeline_attr_target(
                    attr,
                    "pass binding entries",
                    &["group", "binding", "expect", "known", "access", "sampler"],
                    &mut diags,
                );
            }
        }

        for permutation in &pass_decl.permutations {
            let known_attr = permutation.attrs.iter().find(|attr| attr.name == "known");
            let Some(known_attr) = known_attr else {
                diags.push(
                    pass_error(
                        pass_decl,
                        permutation.span.clone(),
                        format!(
                            "pass `{}` axis `{}` must declare `@known(...)` explicitly",
                            pass_decl.name, permutation.name
                        ),
                    )
                    .with_help(
                        "add `@known(compile)`, `@known(pipeline)`, or `@known(draw)` so the binding time is explicit",
                    ),
                );
                continue;
            };

            let known_mode = known_attr
                .args
                .first()
                .map(|value| value.trim().to_ascii_lowercase());

            if known_mode.is_none() {
                diags.push(
                    pass_error(
                        pass_decl,
                        known_attr.name_span.clone(),
                        format!(
                            "pass `{}` axis `{}` uses `@known` without a binding-time mode",
                            pass_decl.name, permutation.name
                        ),
                    )
                    .with_help("spell the mode explicitly as `@known(compile)`, `@known(pipeline)`, or `@known(draw)`"),
                );
            } else if !matches!(known_mode.as_deref(), Some("compile" | "pipeline" | "draw")) {
                diags.push(
                    pass_error(
                        pass_decl,
                        known_attr.name_span.clone(),
                        format!(
                            "pass `{}` axis `{}` uses unknown @known mode `{}`",
                            pass_decl.name,
                            permutation.name,
                            known_mode.unwrap_or_default(),
                        ),
                    )
                    .with_help(
                        "supported binding-time modes are `compile`, `pipeline`, and `draw`",
                    ),
                );
            }

            if permutation.values.is_empty() {
                diags.push(
                    pass_error(
                        pass_decl,
                        permutation.span.clone(),
                        format!(
                            "pass `{}` axis `{}` must declare at least one permutation value",
                            pass_decl.name, permutation.name
                        ),
                    )
                    .with_help("add one or more literal values in the pass `permutations` block"),
                );
            }

            for attr in &permutation.attrs {
                validate_pipeline_attr_target(
                    attr,
                    "pass permutation entries",
                    &[
                        "known",
                        "known_compile",
                        "known_pipeline",
                        "known_draw",
                        "config_editor",
                        "config",
                    ],
                    &mut diags,
                );
            }
        }

        let has_editor_config_axis = pass_decl
            .permutations
            .iter()
            .any(|permutation| permutation.attrs.iter().any(attr_has_editor_config));
        let is_editor_only_pass = has_attr(&pass_decl.attrs, "editor_only");
        if has_editor_config_axis && !is_editor_only_pass {
            diags.push(
                pass_error(
                    pass_decl,
                    pass_decl.span.clone(),
                    format!(
                        "pass `{}` declares `@config(editor)` permutations but is not marked `@editor_only`",
                        pass_decl.name
                    ),
                )
                .with_help(
                    "mark the pass `@editor_only`, or remove `@config_editor` from shipping-required axes",
                ),
            );
        }

        validate_pass_requirements(pass_decl, &mut diags);
    }

    validate_pipeline_axis_agreement(program, &mut diags);

    for pipeline_decl in &program.pipelines {
        let pipeline_kind = pipeline_decl.pipeline_type.as_str();
        let pipeline_kind_known = known_builtin_pipeline_types.contains(pipeline_kind)
            || declared_pipeline_types.contains(pipeline_kind);
        if !pipeline_kind_known {
            diags.push(
                Diag::error(
                    pipeline_decl.pipeline_type_span.clone(),
                    format!(
                        "unknown pipeline kind `{}` for pipeline `{}`",
                        pipeline_decl.pipeline_type, pipeline_decl.name
                    ),
                )
                .with_help(
                    "use a built-in kind (`postprocess`, `compute`) or declare project kinds with `tags pipeline { ... }`",
                ),
            );
        }

        for attr in &pipeline_decl.attrs {
            if matches!(
                attr.name.as_str(),
                "pool" | "asset" | "provider" | "from" | "output" | "dimensions"
            ) && !pipeline_decl.attrs.iter().any(|a| a.name == "technique")
            {
                diags.push(Diag::error(
                    attr.span.clone(),
                    format!("@{} requires an executable @technique pipeline", attr.name),
                ));
            }
            validate_pipeline_attr_target(
                attr,
                "pipeline declarations",
                &[
                    "technique",
                    "meta",
                    "pool",
                    "asset",
                    "provider",
                    "from",
                    "dimensions",
                    "output",
                    "renderer",
                    "default",
                    "configure",
                    "selected_renderer",
                    "image",
                    "buffer",
                    "external",
                    "table_data",
                ],
                &mut diags,
            );
        }

        for pass_ref in &pipeline_decl.pass_refs {
            if !declared_pass_names.contains(&pass_ref.name) {
                diags.push(
                    Diag::error(
                        pass_ref.name_span.clone(),
                        format!(
                            "pipeline `{}` references unknown pass `{}`",
                            pipeline_decl.name, pass_ref.name
                        ),
                    )
                    .with_help(
                        "declare the pass before the pipeline or fix the pass reference name",
                    ),
                );
            }

            for attr in &pass_ref.attrs {
                validate_pipeline_attr_target(
                    attr,
                    "pipeline pass references",
                    &[
                        "node",
                        "draw",
                        "dispatch",
                        "bind",
                        "color",
                        "depth",
                        "after",
                        "when",
                        "per_invocation",
                        "transparent_queue",
                        "instances",
                        "draw_depth",
                        "enabled",
                        "attachment",
                        "implementation_when",
                        "implementation_require",
                    ],
                    &mut diags,
                );
            }
        }
    }

    validate_pass_vertex_factory_contracts(program, &mut diags);
    validate_vertex_factory_bodies(program, &mut diags);
    validate_pipeline_known_axis_agreement(program, &mut diags);

    if diags.is_empty() { Ok(()) } else { Err(diags) }
}

pub(super) fn is_stdlib_source(path: &str) -> bool {
    path.starts_with("<stdlib:")
}

/// Returns the canonical UV coordinates for a named anchor variant, or `None`
/// if the name is not a known anchor. Both `anchor_variant_value` (in `expr`)
/// and `anchor_variant_vec2` (in `coercions`) derive their values from this
/// single source of truth.
pub(super) fn anchor_variant_coords(name: &str) -> Option<(f32, f32)> {
    match name {
        "center" => Some((0.5, 0.5)),
        "top_center" => Some((0.5, 0.0)),
        "bottom_center" => Some((0.5, 1.0)),
        "left_center" => Some((0.0, 0.5)),
        "right_center" => Some((1.0, 0.5)),
        "top_left" => Some((0.0, 0.0)),
        "top_right" => Some((1.0, 0.0)),
        "bottom_left" => Some((0.0, 1.0)),
        "bottom_right" => Some((1.0, 1.0)),
        _ => None,
    }
}

pub(super) fn validate_normalized_root_entry_params(
    entry: &NormalizedRootEntry,
    prep_diags: &mut Vec<Diag>,
) {
    use crate::ast::RootEntryParamRole;

    let mut seen_coord: Option<(String, Span)> = None;
    let mut seen_surf: Option<(String, Span)> = None;
    let mut seen_signal: Option<(String, Span)> = None;
    let mut seen_delta: Option<(String, Span)> = None;
    let mut seen_resolution: Option<(String, Span)> = None;

    for param in &entry.params {
        // `param.role` drives which kind of parameter this is; `ty_name` is
        // consulted only to tell `coord` and `surf` apart within `Primary`
        // and to word diagnostics — never to redetermine the classification.
        match param.role {
            RootEntryParamRole::Primary if param.ty_name == "coord" => {
                if let Some((first_name, first_span)) = &seen_coord {
                    prep_diags.push(
                        Diag::error(
                            param.name_span.clone(),
                            "duplicate `coord` canvas parameter",
                        )
                        .with_label("`coord` parameter already declared")
                        .with_help(format!(
                            "first `coord` parameter is `{first_name}` at bytes {}..{}",
                            first_span.start, first_span.end
                        )),
                    );
                } else {
                    seen_coord = Some((param.name.clone(), param.name_span.clone()));
                }
            }
            RootEntryParamRole::Primary => {
                if let Some((first_name, first_span)) = &seen_surf {
                    prep_diags.push(
                        Diag::error(param.name_span.clone(), "duplicate `surf` entry parameter")
                            .with_label("`surf` parameter already declared")
                            .with_help(format!(
                                "first `surf` parameter is `{first_name}` at bytes {}..{}",
                                first_span.start, first_span.end
                            )),
                    );
                } else {
                    seen_surf = Some((param.name.clone(), param.name_span.clone()));
                }
            }
            RootEntryParamRole::Signal => {
                if let Some((first_name, first_span)) = &seen_signal {
                    prep_diags.push(
                        Diag::error(
                            param.name_span.clone(),
                            "duplicate `signal` canvas parameter",
                        )
                        .with_label("`signal` parameter already declared")
                        .with_help(format!(
                            "first `signal` parameter is `{first_name}` at bytes {}..{}",
                            first_span.start, first_span.end
                        )),
                    );
                } else {
                    seen_signal = Some((param.name.clone(), param.name_span.clone()));
                }
            }
            RootEntryParamRole::Delta => {
                if let Some((first_name, first_span)) = &seen_delta {
                    prep_diags.push(
                        Diag::error(
                            param.name_span.clone(),
                            "duplicate `delta` canvas parameter",
                        )
                        .with_label("`delta` parameter already declared")
                        .with_help(format!(
                            "first `delta` parameter is `{first_name}` at bytes {}..{}",
                            first_span.start, first_span.end
                        )),
                    );
                } else {
                    seen_delta = Some((param.name.clone(), param.name_span.clone()));
                }
            }
            RootEntryParamRole::Resolution => {
                if let Some((first_name, first_span)) = &seen_resolution {
                    prep_diags.push(
                        Diag::error(
                            param.name_span.clone(),
                            "duplicate `resolution` canvas parameter",
                        )
                        .with_label("`resolution` parameter already declared")
                        .with_help(format!(
                            "first `resolution` parameter is `{first_name}` at bytes {}..{}",
                            first_span.start, first_span.end
                        )),
                    );
                } else {
                    seen_resolution = Some((param.name.clone(), param.name_span.clone()));
                }
            }
            RootEntryParamRole::Other => {
                let is_likely_reversed = (param.name == "coord" && param.ty_name == "uv")
                    || (param.name == "signal" && param.ty_name == "time")
                    || (param.name == "surf" && param.ty_name == "sp");

                let mut diag = Diag::error(
                    param.name_span.clone(),
                    format!(
                        "unsupported entry parameter `{}: {}`",
                        param.name, param.ty_name
                    ),
                )
                .with_label("entry parameters use `name: type`");

                if is_likely_reversed {
                    diag = diag
                        .with_help(format!("did you mean `{}: {}`?", param.ty_name, param.name));
                } else {
                    diag = diag.with_help(
                        "supported entry parameter types: exactly one primary `: coord` or `: surf`, plus optional `: signal`, `: delta`, and `: resolution` (names are user-defined)",
                    );
                }

                prep_diags.push(diag);
            }
        }
    }

    if seen_coord.is_some() && seen_surf.is_some() {
        prep_diags.push(
            Diag::error(
                entry.name_span.clone(),
                format!(
                    "entry `{}` cannot mix `coord` and `surf` parameters",
                    entry.name
                ),
            )
            .with_help("declare either a canvas-style `name: coord` input or a surface-style `name: surf` input, but not both"),
        );
    }

    if seen_coord.is_none() && seen_surf.is_none() {
        prep_diags.push(
            Diag::error(
                entry.name_span.clone(),
                format!(
                    "entry `{}` is missing a primary input parameter",
                    entry.name
                ),
            )
            .with_help(
                "declare one primary entry parameter as `name: coord` (for example `uv: coord`) or `name: surf` (for example `sp: surf`)",
            )
            .with_label(
                "entry parameters define which name refers to the primary canvas or surface input",
            ),
        );
    }

    let primary_count = entry
        .params
        .iter()
        .filter(|param| matches!(param.role, crate::ast::RootEntryParamRole::Primary))
        .count();
    if primary_count != 1 {
        prep_diags.push(
            Diag::error(
                entry.name_span.clone(),
                format!(
                    "normalized entry `{}` must contain exactly one primary input parameter",
                    entry.name
                ),
            )
            .with_help(
                "the normalizer should assign one primary parameter role for each root entry",
            ),
        );
    }
}

pub(super) fn validate_interface_conformance_methods(
    interfaces: &[InterfaceDecl],
    conformances: &[ConformanceDecl],
    prep_diags: &mut Vec<Diag>,
) {
    let interface_map: HashMap<&str, &InterfaceDecl> = interfaces
        .iter()
        .map(|iface| (iface.name.as_str(), iface))
        .collect();

    for conform in conformances {
        let Some(interface_decl) = interface_map.get(conform.interface_name.as_str()).copied()
        else {
            eprintln!(
                "[debug validate_interface_conformance_methods] interfaces={:?}, looking for {:?}",
                interface_map.keys().collect::<Vec<_>>(),
                conform.interface_name
            );
            prep_diags.push(
                Diag::error(
                    conform.interface_name_span.clone(),
                    format!(
                        "unknown interface `{}` in conformance",
                        conform.interface_name
                    ),
                )
                .with_help("declare the interface before using it in a `conform` block"),
            );
            continue;
        };

        let mut impl_methods: HashMap<&str, &FnDecl> = HashMap::new();
        for method in &conform.methods {
            if let Some(existing) = impl_methods.insert(method.name.as_str(), method) {
                prep_diags.push(
                    Diag::error(
                        method.name_span.clone(),
                        format!(
                            "duplicate conformance method `{}` for interface `{}`",
                            method.name, conform.interface_name
                        ),
                    )
                    .with_help(format!(
                        "keep one `{}` implementation in this conformance block",
                        existing.name
                    )),
                );
            }
        }

        for iface_method in &interface_decl.methods {
            let Some(impl_method) = impl_methods.get(iface_method.name.as_str()).copied() else {
                prep_diags.push(
                    Diag::error(
                        conform.span.clone(),
                        format!(
                            "conformance `{}` for `{}` is missing method `{}`",
                            conform.type_name, conform.interface_name, iface_method.name
                        ),
                    )
                    .with_help("add the missing method implementation to the conformance block"),
                );
                continue;
            };

            if iface_method.params.len() != impl_method.params.len() {
                prep_diags.push(
                    Diag::error(
                        impl_method.name_span.clone(),
                        format!(
                            "method `{}` in conformance `{}` has {} parameter(s), but interface `{}` requires {}",
                            impl_method.name,
                            conform.type_name,
                            impl_method.params.len(),
                            conform.interface_name,
                            iface_method.params.len()
                        ),
                    )
                    .with_help("match the interface method arity exactly"),
                );
                continue;
            }

            for (index, (iface_param, impl_param)) in iface_method
                .params
                .iter()
                .zip(impl_method.params.iter())
                .enumerate()
            {
                if !declared_type_text_compatible(&iface_param.ty_name, &impl_param.ty_name) {
                    prep_diags.push(
                        Diag::error(
                            impl_param.ty_span.clone(),
                            format!(
                                "method `{}` parameter {} in conformance `{}` has type `{}`, expected `{}` from interface `{}`",
                                impl_method.name,
                                index + 1,
                                conform.type_name,
                                impl_param.ty_name,
                                iface_param.ty_name,
                                conform.interface_name
                            ),
                        )
                        .with_help(
                            "match interface method parameter types by position; unlabeled types may bridge to labeled ones, but two labeled spaces must match exactly unless transformed explicitly",
                        ),
                    );
                }

                if iface_param.keyword_only != impl_param.keyword_only {
                    let expected = if iface_param.keyword_only {
                        "keyword-only"
                    } else {
                        "positional-or-named"
                    };
                    prep_diags.push(
                        Diag::error(
                            impl_param.name_span.clone(),
                            format!(
                                "method `{}` parameter `{}` keyword-only boundary mismatch in conformance `{}`: interface `{}` expects {} parameter",
                                impl_method.name,
                                impl_param.name,
                                conform.type_name,
                                conform.interface_name,
                                expected
                            ),
                        )
                        .with_help(
                            "align `*` placement with the interface method so keyword-only parameters match",
                        ),
                    );
                }
            }

            match (&iface_method.ret_ty, &impl_method.ret_ty) {
                (Some((iface_ret, _)), Some((impl_ret, _)))
                    if declared_type_text_compatible(iface_ret, impl_ret) => {}
                (Some((iface_ret, _)), Some((impl_ret, impl_span))) => {
                    prep_diags.push(
                        Diag::error(
                            impl_span.clone(),
                            format!(
                                "method `{}` in conformance `{}` returns `{}`, expected `{}` from interface `{}`",
                                impl_method.name,
                                conform.type_name,
                                impl_ret,
                                iface_ret,
                                conform.interface_name
                            ),
                        )
                        .with_help(
                            "match interface return type semantics; unlabeled types may bridge to labeled ones, but two labeled spaces must match exactly unless transformed explicitly",
                        ),
                    );
                }
                (Some((iface_ret, _)), None) => {
                    prep_diags.push(
                        Diag::error(
                            impl_method.name_span.clone(),
                            format!(
                                "method `{}` in conformance `{}` is missing return type `{}` required by interface `{}`",
                                impl_method.name,
                                conform.type_name,
                                iface_ret,
                                conform.interface_name
                            ),
                        )
                        .with_help("add the declared return type to the conformance method"),
                    );
                }
                (None, Some((_, impl_span))) => {
                    prep_diags.push(
                        Diag::error(
                            impl_span.clone(),
                            format!(
                                "method `{}` in conformance `{}` declares a return type, but interface `{}` method does not",
                                impl_method.name, conform.type_name, conform.interface_name
                            ),
                        )
                        .with_help("remove the return type or update the interface declaration"),
                    );
                }
                (None, None) => {}
            }
        }

        for impl_method in &conform.methods {
            if !interface_decl
                .methods
                .iter()
                .any(|iface_method| iface_method.name == impl_method.name)
            {
                prep_diags.push(
                    Diag::error(
                        impl_method.name_span.clone(),
                        format!(
                            "method `{}` is not declared by interface `{}`",
                            impl_method.name, conform.interface_name
                        ),
                    )
                    .with_help("remove the extra method or add it to the interface"),
                );
            }
        }
    }
}

fn declared_type_text_compatible(expected: &str, actual: &str) -> bool {
    if strip_spatial_type_suffix(expected) != strip_spatial_type_suffix(actual) {
        return false;
    }

    match (
        spatial_qualifier_text(expected),
        spatial_qualifier_text(actual),
    ) {
        (Some(expected_q), Some(actual_q)) => expected_q == actual_q,
        _ => true,
    }
}

fn spatial_qualifier_text(name: &str) -> Option<String> {
    let trimmed = name.trim();

    if let Some((base, suffix)) = trimmed.rsplit_once(" in ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
    {
        return Some(format!("in {}", suffix.trim()));
    }

    if let Some((base, suffix)) = trimmed.rsplit_once(" from ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
        && let Some((src, dst)) = suffix.rsplit_once(" to ")
        && !src.trim().is_empty()
        && !dst.trim().is_empty()
    {
        return Some(format!("from {} to {}", src.trim(), dst.trim()));
    }

    None
}

#[derive(Debug, Clone, Copy)]
pub struct CheckOptions {
    pub wide_effect_warn_ratio: f32,
    pub wide_effect_note_ratio: f32,
    pub projective_footprint_max_px: Option<f32>,
    pub shape_aa_min_px: Option<f32>,
    pub shape_aa_max_px: Option<f32>,
    pub shape_aa_style: Option<hir::ShapeAaStyle>,
    pub expr_timeout_ms: Option<u64>,
    pub allow_implicit_texture_uv: bool,
    pub warn_implicit_texture_uv: bool,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            wide_effect_warn_ratio: 0.12,
            wide_effect_note_ratio: 0.04,
            projective_footprint_max_px: None,
            shape_aa_min_px: None,
            shape_aa_max_px: None,
            shape_aa_style: None,
            expr_timeout_ms: None,
            allow_implicit_texture_uv: false,
            warn_implicit_texture_uv: false,
        }
    }
}

impl CheckOptions {
    pub(crate) fn rendering_policy(&self) -> Option<hir::RenderingPolicy> {
        Some(hir::RenderingPolicy {
            shape_aa_min_px: self.shape_aa_min_px?,
            shape_aa_max_px: self.shape_aa_max_px?,
            shape_aa_style: self.shape_aa_style?,
            projective_footprint_max_px: self.projective_footprint_max_px?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSpace {
    Srgb,
    Linear,
}

type Mat3Value = ((Sx, Sx, Sx), (Sx, Sx, Sx), (Sx, Sx, Sx));
type Mat4Value = (
    (Sx, Sx, Sx, Sx),
    (Sx, Sx, Sx, Sx),
    (Sx, Sx, Sx, Sx),
    (Sx, Sx, Sx, Sx),
);

#[derive(Debug, Clone)]
pub(crate) struct PathValue {
    profile_id: usize,
    segment_count: usize,
    total_length: f32,
    storage: PathStorageDecision,
}

pub(crate) const PATH_CONST_SEGMENT_THRESHOLD: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathStorageDecision {
    ConstModuleEmbedded,
    BufferExceedsConstThreshold,
}

impl PathValue {
    pub(crate) fn from_points(profile_id: usize, points: Vec<(f32, f32)>) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        let segment_count = points.len().saturating_sub(1);
        let mut total = 0.0_f32;
        for i in 1..points.len() {
            let a = points[i - 1];
            let b = points[i];
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            let len = (dx * dx + dy * dy).sqrt();
            total += len;
        }
        if total <= f32::EPSILON {
            return None;
        }
        let storage = if segment_count <= PATH_CONST_SEGMENT_THRESHOLD {
            PathStorageDecision::ConstModuleEmbedded
        } else {
            PathStorageDecision::BufferExceedsConstThreshold
        };
        Some(Self {
            profile_id,
            segment_count,
            total_length: total,
            storage,
        })
    }

    pub(crate) fn from_derived(
        profile_id: usize,
        segment_count: usize,
        total_length: f32,
        storage: PathStorageDecision,
    ) -> Option<Self> {
        if segment_count == 0 || total_length <= f32::EPSILON {
            return None;
        }
        Some(Self {
            profile_id,
            segment_count,
            total_length,
            storage,
        })
    }

    pub(crate) fn profile_id(&self) -> usize {
        self.profile_id
    }

    pub(crate) fn with_profile_id(mut self, profile_id: usize) -> Self {
        self.profile_id = profile_id;
        self
    }

    pub(crate) fn total_length_sx(&self) -> Sx {
        Sx::Lit(self.total_length)
    }

    pub(crate) fn segment_count(&self) -> usize {
        self.segment_count
    }

    pub(crate) fn storage_decision(&self) -> PathStorageDecision {
        self.storage
    }
}

/// Keep cell metadata off the recursive checker's stack as members expand.
#[derive(Debug, Clone)]
pub(crate) struct RepeatCellValue {
    scope_id: u32,
    every: Option<[f32; 2]>,
    id: V2,
    center: V2,
    uv: V2,
    rand: Sx,
}

#[derive(Debug, Clone)]
pub(crate) enum Value {
    Error,
    Scalar(Sx),
    Distance(Sx),
    Coverage(Sx),
    Mask(Sx),
    Slot {
        index: Sx,
        count: Sx,
        start: Sx,
        end: Sx,
    },
    Vec2(V2),
    Vec3((Sx, Sx, Sx)),
    Vec4((Sx, Sx, Sx, Sx)),
    Mat2((V2, V2)),
    Mat3(Box<Mat3Value>),
    Mat4(Box<Mat4Value>),
    Array(Vec<Value>),
    ScatterInstance {
        pos: V2,
        id: Sx,
        index01: Sx,
        age_norm: Sx,
    },
    RepeatCell(Box<RepeatCellValue>),
    CellContour {
        scope_id: u32,
        inset: Sx,
    },
    Color {
        rgba: [f32; 4],
        space: ColorSpace,
    },
    ColorField {
        rgba: [Sx; 4],
        space: ColorSpace,
    },
    Gradient {
        kind: GradientKind,
        stops: Vec<GradientStop>,
    },
    Shape(ShapeId),
    Layer(LayerId),
    Space(Vec<Xform>),
    Lambda {
        params: Vec<String>,
        body: LambdaBody,
    },
    /// A reference to a named top-level or local `fn` declaration.
    /// Used to pass function references as callable arguments (§16.5).
    FnRef(String),
    /// A typed texture that has been resolved to a layer but also carries
    /// its texture name for semantic channel access (`.roughness`, `.metallic`, etc.).
    TypedTextureSample {
        layer_id: LayerId,
        tex_name: String,
        sample_at: Option<V2>,
    },
    /// A reference to a dynamic array parameter (`array<T>` with no size).
    /// Indexing this value with a runtime scalar produces the element type.
    DynamicArray {
        param_name: String,
        elem_type: hir::ArrayElemType,
    },
    Struct {
        ty_name: String,
        fields: HashMap<String, Value>,
    },
    /// Path value with segment geometry lowered to a sampled polyline profile.
    PathFuture(PathValue),
}

enum FillStyle {
    Solid([f32; 4]),
    Dynamic([Sx; 4]),
    Gradient {
        kind: GradientKind,
        stops: Vec<GradientStop>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum NumberSource {
    Range { lo: Sx, hi: Sx, span: Span },
    List { items: Vec<Sx>, span: Span },
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Error => "error",
            Value::Scalar(_) => "scalar",
            Value::Distance(_) => "distance",
            Value::Coverage(_) => "coverage",
            Value::Mask(_) => "mask",
            Value::Slot { .. } => "slot",
            Value::Vec2(_) => "vec2",
            Value::Vec3(_) => "vec3",
            Value::Vec4(_) => "vec4",
            Value::Mat2(_) => "mat2",
            Value::Mat3(_) => "mat3",
            Value::Mat4(_) => "mat4",
            Value::Array(_) => "array",
            Value::ScatterInstance { .. } => "scatter instance",
            Value::RepeatCell(_) => "repeat cell",
            Value::CellContour { .. } => "contour",
            Value::Color { .. } => "color",
            Value::ColorField { .. } => "color",
            Value::Gradient { .. } => "gradient",
            Value::Shape(_) => "shape",
            Value::Layer(_) => "layer",
            Value::Space(_) => "space",
            Value::Lambda { .. } => "lambda",
            Value::FnRef(_) => "fn reference",
            Value::TypedTextureSample { .. } => "typed texture",
            Value::Struct { .. } => "struct",
            Value::PathFuture(_) => "path",
            Value::DynamicArray { .. } => "dynamic array",
        }
    }
}

pub(super) fn local_decl_type_matches(
    value: &Value,
    ty_name: &str,
    enum_defs: &HashMap<String, EnumDef>,
    struct_defs: &HashMap<String, StructDef>,
    type_param_names: &[&str],
) -> bool {
    let ty_name = strip_spatial_type_suffix(ty_name);

    if let Some((base, args)) = try_parse_type_application(ty_name)
        && base == "field"
        && args.len() == 1
    {
        return local_decl_type_matches(value, &args[0], enum_defs, struct_defs, type_param_names);
    }

    if let Some((element, length)) = hir::parse_array_param_type(ty_name) {
        return matches!(value, Value::Array(values) if values.len() == length && values.iter().all(|value|
            local_decl_type_matches(value, element, enum_defs, struct_defs, type_param_names)));
    }

    if let Some(kind) = crate::typed_scalar::Kind::element(ty_name) {
        let actual = Checker::value_element_kind(value);
        if actual.is_some_and(|actual| actual != kind) {
            return false;
        }
    }

    match ty_name {
        "f32" | "float" | "f64" | "half" | "i32" | "u32" | "bool" | "angle" | "length" => matches!(
            value,
            Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_)
        ),
        "slot" => matches!(value, Value::Slot { .. }),
        "vec2" | "uvec2" | "ivec2" | "bvec2" | "coord" | "coord_like" | "resolution" => {
            matches!(value, Value::Vec2(_))
        }
        "cell" => matches!(value, Value::RepeatCell(_)),
        "contour" => matches!(value, Value::CellContour { .. }),
        "vec3" | "uvec3" | "ivec3" | "bvec3" => matches!(value, Value::Vec3(_)),
        "vec4" | "uvec4" | "ivec4" | "bvec4" => matches!(value, Value::Vec4(_)),
        "mat2" => matches!(value, Value::Mat2(_)),
        "mat3" => matches!(value, Value::Mat3(_)),
        "mat4" => matches!(value, Value::Mat4(_)),
        "color" => matches!(value, Value::Color { .. } | Value::ColorField { .. }),
        "shape" => matches!(value, Value::Shape(_)),
        "layer" => matches!(value, Value::Layer(_)),
        "space" => matches!(value, Value::Space(_)),
        "signal" | "delta" => matches!(
            value,
            Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_)
        ),
        other => {
            // Generic type parameter: any value matches (type is inferred at the call site).
            if type_param_names.contains(&other) {
                return true;
            }
            // User-defined struct type.
            if struct_defs.contains_key(other) {
                return matches!(value, Value::Struct { ty_name, .. } if ty_name == other);
            }
            // User-defined enum: enum values are represented as scalars in the IR.
            if enum_defs.contains_key(other) {
                return matches!(
                    value,
                    Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_)
                );
            }
            // Callable type annotation (`fn(T)->R`): value must be a fn reference or lambda.
            if other.starts_with("fn(") {
                return matches!(value, Value::FnRef(_) | Value::Lambda { .. });
            }
            false
        }
    }
}

pub(super) fn local_decl_expected_kind(
    ty_name: &str,
    enum_defs: &HashMap<String, EnumDef>,
    struct_defs: &HashMap<String, StructDef>,
    type_param_names: &[&str],
) -> &'static str {
    let ty_name = strip_spatial_type_suffix(ty_name);

    if let Some((base, args)) = try_parse_type_application(ty_name)
        && base == "field"
        && args.len() == 1
    {
        return local_decl_expected_kind(&args[0], enum_defs, struct_defs, type_param_names);
    }

    match ty_name {
        "f32" | "float" | "f64" | "half" | "i32" | "u32" | "bool" | "signal" | "delta"
        | "angle" | "length" => "scalar",
        "vec2" | "coord" | "coord_like" | "resolution" => "vec2",
        "slot" => "slot",
        "cell" => "repeat cell",
        "vec3" => "vec3",
        "vec4" => "vec4",
        "mat2" => "mat2",
        "mat3" => "mat3",
        "mat4" => "mat4",
        "color" => "color",
        "shape" => "shape",
        "layer" => "layer",
        "space" => "space",
        other => {
            if type_param_names.contains(&other) {
                "any value"
            } else if struct_defs.contains_key(other) {
                "struct"
            } else if enum_defs.contains_key(other) {
                "scalar (enum value)"
            } else if other.starts_with("fn(") {
                "fn reference"
            } else {
                "unknown"
            }
        }
    }
}

pub(crate) fn strip_spatial_type_suffix(name: &str) -> &str {
    let trimmed = name.trim();

    if let Some((base, suffix)) = trimmed.rsplit_once(" in ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
    {
        return base.trim();
    }

    if let Some((base, suffix)) = trimmed.rsplit_once(" from ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
        && let Some((_src, dst)) = suffix.rsplit_once(" to ")
        && !dst.trim().is_empty()
    {
        return base.trim();
    }

    trimmed
}

fn scalar_like_to_sx(value: &Value) -> Option<Sx> {
    match value {
        Value::Scalar(sx) | Value::Distance(sx) | Value::Coverage(sx) | Value::Mask(sx) => {
            Some(sx.clone())
        }
        _ => None,
    }
}

fn swizzle_components(value: &Value) -> Option<(usize, Vec<Sx>)> {
    match value {
        Value::Vec2((x, y)) => Some((2, vec![x.clone(), y.clone()])),
        Value::Vec3((x, y, z)) => Some((3, vec![x.clone(), y.clone(), z.clone()])),
        Value::Vec4((x, y, z, w)) => Some((4, vec![x.clone(), y.clone(), z.clone(), w.clone()])),
        _ => None,
    }
}

fn value_kind_for_swizzle_len(len: usize) -> &'static str {
    match len {
        1 => "scalar",
        2 => "vec2",
        3 => "vec3",
        4 => "vec4",
        _ => "value",
    }
}

pub(super) fn apply_swizzle_assignment(
    base: Value,
    swizzle: &str,
    rhs: Value,
) -> Result<Value, String> {
    let (dim, mut base_components) = swizzle_components(&base).ok_or_else(|| {
        format!(
            "swizzle assignment base must be vec2/vec3/vec4, got {}",
            base.kind()
        )
    })?;

    if swizzle.is_empty() {
        return Err("empty swizzle in assignment target".to_string());
    }

    let rhs_components = if swizzle.len() == 1 {
        vec![scalar_like_to_sx(&rhs).ok_or_else(|| {
            format!(
                "swizzle assignment expects scalar right-hand side for single-component target, got {}",
                rhs.kind()
            )
        })?]
    } else {
        let (_, comps) = swizzle_components(&rhs).ok_or_else(|| {
            format!(
                "swizzle assignment expects {} right-hand side, got {}",
                value_kind_for_swizzle_len(swizzle.len()),
                rhs.kind()
            )
        })?;
        comps
    };

    if rhs_components.len() != swizzle.len() {
        return Err(format!(
            "swizzle assignment expects {} right-hand side, got {}",
            value_kind_for_swizzle_len(swizzle.len()),
            value_kind_for_swizzle_len(rhs_components.len())
        ));
    }

    let mut assigned = [false; 4];
    for (idx, ch) in swizzle.chars().enumerate() {
        let target = match ch {
            'x' => 0,
            'y' => 1,
            'z' => 2,
            'w' => 3,
            _ => {
                return Err(format!(
                    "invalid swizzle component `{ch}` in assignment target"
                ));
            }
        };

        if target >= dim {
            return Err(format!(
                "swizzle component `{ch}` is invalid for vec{dim} assignment target"
            ));
        }

        if assigned[target] {
            return Err(format!(
                "duplicate swizzle component `{ch}` in assignment target"
            ));
        }
        assigned[target] = true;
        base_components[target] = rhs_components[idx].clone();
    }

    match dim {
        2 => Ok(Value::Vec2((
            base_components[0].clone(),
            base_components[1].clone(),
        ))),
        3 => Ok(Value::Vec3((
            base_components[0].clone(),
            base_components[1].clone(),
            base_components[2].clone(),
        ))),
        4 => Ok(Value::Vec4((
            base_components[0].clone(),
            base_components[1].clone(),
            base_components[2].clone(),
            base_components[3].clone(),
        ))),
        _ => unreachable!(),
    }
}

/// Named/positional argument matcher for builtin signatures.
pub(crate) struct ArgBag<'a> {
    args: &'a [Arg],
    taken: Vec<bool>,
    call_span: Span,
    callee: String,
    path_recv: Option<PathValue>,
}

impl<'a> ArgBag<'a> {
    fn new(callee: &str, args: &'a [Arg], call_span: Span) -> Self {
        ArgBag {
            args,
            taken: vec![false; args.len()],
            call_span,
            callee: callee.to_string(),
            path_recv: None,
        }
    }

    pub(crate) fn set_path_receiver(&mut self, path: PathValue) {
        self.path_recv = Some(path);
    }

    pub(crate) fn path_receiver(&self) -> Option<&PathValue> {
        self.path_recv.as_ref()
    }

    /// Take the argument named `name`, or the next unused positional one.
    fn take(&mut self, name: &str) -> Option<&'a SExpr> {
        // named match first
        for (i, a) in self.args.iter().enumerate() {
            if !self.taken[i] && a.name.as_deref() == Some(name) {
                self.taken[i] = true;
                return Some(&a.value);
            }
        }
        // then first unused positional
        for (i, a) in self.args.iter().enumerate() {
            if !self.taken[i] && a.name.is_none() {
                self.taken[i] = true;
                return Some(&a.value);
            }
        }
        None
    }

    pub(crate) fn take_named(&mut self, name: &str) -> Option<&'a SExpr> {
        self.take(name)
    }

    pub(crate) fn take_exact_named(&mut self, name: &str) -> Option<&'a SExpr> {
        for (i, a) in self.args.iter().enumerate() {
            if !self.taken[i] && a.name.as_deref() == Some(name) {
                self.taken[i] = true;
                return Some(&a.value);
            }
        }
        None
    }

    fn require(&mut self, name: &str, diags: &mut Vec<Diag>) -> Option<&'a SExpr> {
        let got = self.take(name);
        if got.is_none() {
            diags.push(
                Diag::error(
                    self.call_span.clone(),
                    format!("`{}` is missing its `{name}` argument", self.callee),
                )
                .with_label(format!("expected `{name}: ...` here")),
            );
        }
        got
    }

    fn finish(self, diags: &mut Vec<Diag>) {
        for (i, a) in self.args.iter().enumerate() {
            if !self.taken[i] {
                let what = a
                    .name
                    .as_deref()
                    .map(|n| format!("unknown argument `{n}`"))
                    .unwrap_or_else(|| "unexpected extra argument".to_string());
                diags.push(
                    Diag::error(a.value.span.clone(), format!("{what} to `{}`", self.callee))
                        .with_label("not part of this builtin's signature"),
                );
            }
        }
    }

    /// Access call span for error reporting.
    pub(crate) fn call_span(&self) -> &Span {
        &self.call_span
    }
}

impl Checker {
    pub(super) fn implicit_texture_uv_allowed(
        &mut self,
        span: Span,
        texture_name: &str,
        action: &str,
        help: impl Into<String>,
    ) -> bool {
        if self.check_options.allow_implicit_texture_uv {
            return true;
        }

        let help = help.into();
        if self.check_options.warn_implicit_texture_uv {
            self.diags.push(
                Diag::warning(
                    span,
                    format!("implicit texture sampling is deprecated for `{texture_name}`; provide explicit coordinates"),
                )
                .with_help(help),
            );
            true
        } else {
            self.diags.push(
                Diag::error(
                    span,
                    format!("implicit texture sampling is disabled for `{texture_name}`; {action}"),
                )
                .with_help(help),
            );
            false
        }
    }

    pub(crate) fn with_piped_arg<R>(
        &mut self,
        recv: Value,
        args: &[Arg],
        call_span: &Span,
        f: impl FnOnce(&mut Self, &[Arg]) -> R,
    ) -> R {
        let pipe_name = "__pipe_arg0".to_string();
        self.scopes.push(HashMap::new());
        self.bind(pipe_name.clone(), recv);

        let mut piped_args = Vec::with_capacity(args.len() + 1);
        piped_args.push(Arg {
            name: None,
            value: Spanned {
                node: Expr::Var(pipe_name),
                span: call_span.clone(),
            },
        });
        piped_args.extend(args.iter().cloned());

        let out = f(self, &piped_args);
        self.scopes.pop();
        out
    }
}

pub struct Checker {
    hir: Hir,
    diags: Vec<Diag>,
    scopes: Vec<HashMap<String, Value>>,
    // Nested inline evaluations record writes to enclosing lexical scopes.
    assignment_scopes: Vec<BTreeSet<(usize, String)>>,
    style_scopes: Vec<HashMap<String, StyleDef>>,
    scatter_rand_scopes: Vec<ScatterRandScope>,
    next_repeat_cell_scope_id: u32,
    enum_defs: HashMap<String, EnumDef>,
    struct_defs: HashMap<String, StructDef>,
    fn_defs: HashMap<String, Vec<FnDef>>,
    #[allow(
        dead_code,
        reason = "Interface definitions are staged for fuller conformance checking"
    )]
    /// Registered interface definitions.
    interface_defs: HashMap<String, InterfaceDef>,
    /// Set of (concrete_type_name, interface_name) pairs for registered conformances.
    /// type_name uses the FnValueTy string representation: "scalar", "vec2", "shape", etc.
    conformance_set: HashSet<(String, String)>,
    /// Names of all registered interfaces (for dyn-dispatch rejection).
    interface_names: HashSet<String>,
    fn_eval_cache: HashMap<String, Value>,
    pending_user_helpers: HashMap<String, (String, FnDef)>,
    emit_user_helper_calls: bool,
    fn_call_stack: Vec<(String, String, Span)>,
    fn_source_stack: Vec<String>,
    /// Monotonically-increasing counter used to mint checker-unique
    /// `Sx::Let` placeholder names when hoisting call-argument expressions
    /// so they aren't duplicated on every reference inside a callee's body.
    let_counter: u64,
    /// Specialization notes accumulated during generic function evaluation.
    specialization_notes: Vec<String>,
    check_profile_timing: CheckProfileTiming,
    check_options: CheckOptions,
    workbook_enabled: bool,
    expr_watchdog_every: u64,
    expr_watchdog_counter: u64,
    expr_timeout: Option<Duration>,
    expr_timeout_reported: bool,
    check_started_at: web_time::Instant,
    expr_hotspots: HashMap<ExprHotspotKey, ExprHotspotStats>,
    expr_hotspot_summary_emitted: bool,
    pub span_log: Vec<(Span, SpanValueKind)>,
    capture_target: Option<(usize, usize)>,
    pub captured: Option<(usize, SpanCapture)>,
    /// Current filtering state for procedural patterns.
    /// Defaults to `Auto`, which enables filtering when footprint is available.
    current_filtering_state: hir::FilteringState,
    /// Registered user-defined effects, keyed by effect name.
    effect_defs: HashMap<String, EffectRegistered>,
    /// `@global_uniform` declarations visible to this check pass, resolved
    /// against the merged program's structs.
    global_uniforms: globals::GlobalUniformRegistry,
    /// Memoized `time`/`delta_time`/`resolution` runtime channel values,
    /// resolved by calling the FR-authored accessor functions.
    runtime_channel_cache: globals::RuntimeChannelCache,
    evaluation_context: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ExprHotspotKey {
    span_start: usize,
    span_end: usize,
    kind: &'static str,
}

#[derive(Debug, Clone, Copy, Default)]
struct ExprHotspotStats {
    total: Duration,
    max: Duration,
    count: u64,
}

#[derive(Debug, Clone, Copy)]
struct ScatterRandScope {
    seed: u32,
}

#[derive(Debug, Default, Clone, Copy)]
struct CheckProfileTiming {
    eval_expr_total: Duration,
    eval_expr_num_total: Duration,
    eval_expr_color_total: Duration,
    eval_expr_var_total: Duration,
    eval_expr_vec2_total: Duration,
    eval_expr_range_total: Duration,
    eval_expr_array_total: Duration,
    eval_expr_field_total: Duration,
    eval_expr_layer_total: Duration,
    eval_expr_unary_total: Duration,
    eval_expr_binary_total: Duration,
    eval_expr_call_total: Duration,
    eval_expr_pipe_total: Duration,
    eval_binary_total: Duration,
    as_scalar_total: Duration,
    as_vec2_total: Duration,
    as_vec3_total: Duration,
    as_vec4_total: Duration,
    as_color_expr_total: Duration,
    as_fill_style_total: Duration,
    builtin_dispatch_total: Duration,
    user_fn_call_total: Duration,
    fn_body_total: Duration,
    eval_block_total: Duration,
    eval_compose_total: Duration,
}

impl CheckProfileTiming {
    fn record(&mut self, key: &str, elapsed: Duration) {
        match key {
            "eval_expr" => self.eval_expr_total += elapsed,
            "eval_expr_num" => self.eval_expr_num_total += elapsed,
            "eval_expr_color" => self.eval_expr_color_total += elapsed,
            "eval_expr_var" => self.eval_expr_var_total += elapsed,
            "eval_expr_vec2" => self.eval_expr_vec2_total += elapsed,
            "eval_expr_range" => self.eval_expr_range_total += elapsed,
            "eval_expr_array" => self.eval_expr_array_total += elapsed,
            "eval_expr_field" => self.eval_expr_field_total += elapsed,
            "eval_expr_layer" => self.eval_expr_layer_total += elapsed,
            "eval_expr_unary" => self.eval_expr_unary_total += elapsed,
            "eval_expr_binary" => self.eval_expr_binary_total += elapsed,
            "eval_expr_call" => self.eval_expr_call_total += elapsed,
            "eval_expr_pipe" => self.eval_expr_pipe_total += elapsed,
            "eval_binary" => self.eval_binary_total += elapsed,
            "as_scalar" => self.as_scalar_total += elapsed,
            "as_vec2" => self.as_vec2_total += elapsed,
            "as_vec3" => self.as_vec3_total += elapsed,
            "as_vec4" => self.as_vec4_total += elapsed,
            "as_color_expr" => self.as_color_expr_total += elapsed,
            "as_fill_style" => self.as_fill_style_total += elapsed,
            "builtin_dispatch" => self.builtin_dispatch_total += elapsed,
            "user_fn_call" => self.user_fn_call_total += elapsed,
            "fn_body" => self.fn_body_total += elapsed,
            "eval_block" => self.eval_block_total += elapsed,
            "eval_compose" => self.eval_compose_total += elapsed,
            _ => {}
        }
    }
}

impl Checker {
    pub(super) fn expr_watchdog_every_default() -> u64 {
        std::env::var("FRESCO_CHECK_EXPR_WATCHDOG_EVERY")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(10_000)
    }

    /// Fallback timeout applied when neither `--expr-timeout-ms` nor
    /// `FRESCO_CHECK_EXPR_TIMEOUT_MS` is set. Without this, pathological
    /// expressions (e.g. a ternary-based accumulator chained many times,
    /// which duplicates its own subtree on every step since `Sx` has no
    /// structural sharing) hang the compiler forever with zero diagnostic
    /// instead of failing fast with an actionable error.
    ///
    /// 45s (not 30s) to match the existing precedent for legitimately heavy
    /// scenes: `codegen_size_regression.rs`'s scatter compile-timing guard
    /// already budgets 45s on Windows for large-but-finite compiles. A large
    /// raymarched scene (many sequential helper calls per pixel-equivalent
    /// evaluation) is the same class of legitimately-slow-but-not-broken
    /// workload, not a runaway loop.
    const DEFAULT_EXPR_TIMEOUT_MS: u64 = 45_000;

    pub(super) fn expr_timeout_default(options: &CheckOptions) -> Option<Duration> {
        options
            .expr_timeout_ms
            .or_else(|| {
                std::env::var("FRESCO_CHECK_EXPR_TIMEOUT_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .or(Some(Self::DEFAULT_EXPR_TIMEOUT_MS))
            .map(Duration::from_millis)
    }

    pub(super) fn record_check_profile_timing(&mut self, key: &str, elapsed: Duration) {
        self.check_profile_timing.record(key, elapsed);
    }

    /// Get the current filtering state for procedural patterns.
    pub(super) fn get_filtering_state(&self) -> hir::FilteringState {
        self.current_filtering_state
    }

    /// Set the current filtering state for procedural patterns.
    /// Used by the `filtering(on/off)` builtin effect.
    pub(super) fn set_filtering_state(&mut self, state: hir::FilteringState) {
        self.current_filtering_state = state;
    }

    pub(super) fn record_path_channel_demand(
        &mut self,
        path_profile_id: usize,
        channel: &'static str,
    ) {
        if let Some(profile) = self.hir.path_profiles.get_mut(path_profile_id) {
            profile.demand.record_channel(channel);
        }
    }

    pub(super) fn record_path_eval_demand(&mut self, path_profile_id: usize, eval: &'static str) {
        if let Some(profile) = self.hir.path_profiles.get_mut(path_profile_id) {
            profile.demand.record_eval(eval);
        }
    }

    pub(super) fn emit_path_channel_demand_note(&mut self) {
        let mut demanded = BTreeSet::new();
        let mut shared_nearest_search = false;

        for profile in &self.hir.path_profiles {
            let channels = profile.demand.nearest_channels();
            if channels.len() > 1 {
                shared_nearest_search = true;
            }
            demanded.extend(channels);
        }

        if demanded.is_empty() {
            return;
        }

        let channels = demanded.iter().copied().collect::<Vec<_>>().join(", ");
        self.hir
            .notes
            .push(format!("path channels: {{{channels}}}"));

        if shared_nearest_search {
            self.hir
                .notes
                .push("path nearest-search shared=true (demanded channels > 1)".to_string());
        }
    }

    pub(crate) fn register_path_profile_from_primitives(
        &mut self,
        primitives: Vec<hir::PathPrimitive>,
        span: &Span,
        what: &str,
    ) -> Option<PathValue> {
        self.register_path_profile_from_primitives_with_sampling(
            primitives,
            hir::PathSamplingPolicy::default(),
            span,
            what,
        )
    }

    pub(crate) fn register_path_profile_from_primitives_with_sampling(
        &mut self,
        primitives: Vec<hir::PathPrimitive>,
        sampling: hir::PathSamplingPolicy,
        span: &Span,
        what: &str,
    ) -> Option<PathValue> {
        let cubic_count_before = primitives
            .iter()
            .filter(|p| matches!(p, hir::PathPrimitive::Cubic { .. }))
            .count();
        let primitives = hir::preprocess_path_primitives(&primitives, sampling);
        let cubic_count_after = primitives
            .iter()
            .filter(|p| matches!(p, hir::PathPrimitive::Cubic { .. }))
            .count();
        let quadratic_count_after = primitives
            .iter()
            .filter(|p| matches!(p, hir::PathPrimitive::Quadratic { .. }))
            .count();

        let provisional = hir::PathProfile {
            primitives: primitives.clone(),
            sampling,
            flattened_segment_count: 0,
            total_length: 0.0,
            storage: hir::PathStorageDecision::ConstModuleEmbedded,
            demand: hir::PathChannelDemand::default(),
        };
        let rows = provisional.flattened_rows();
        let segment_count = rows.len();
        let total_length = rows.last().map(|seg| seg.s0 + seg.len).unwrap_or(0.0);

        let Some(path) = PathValue::from_derived(
            0,
            segment_count,
            total_length,
            if segment_count <= PATH_CONST_SEGMENT_THRESHOLD {
                PathStorageDecision::ConstModuleEmbedded
            } else {
                PathStorageDecision::BufferExceedsConstThreshold
            },
        ) else {
            self.diags.push(
                Diag::error(
                    span.clone(),
                    format!("{what} must yield at least two distinct sampled points"),
                )
                .with_help("provide a path with visible extent"),
            );
            return None;
        };

        let storage = match path.storage_decision() {
            PathStorageDecision::ConstModuleEmbedded => {
                hir::PathStorageDecision::ConstModuleEmbedded
            }
            PathStorageDecision::BufferExceedsConstThreshold => {
                hir::PathStorageDecision::BufferExceedsConstThreshold
            }
        };
        let profile_id = self.hir.register_path_profile(hir::PathProfile {
            primitives,
            sampling,
            flattened_segment_count: segment_count,
            total_length,
            storage,
            demand: hir::PathChannelDemand::default(),
        });
        let path = path.with_profile_id(profile_id);

        if cubic_count_before > 0 {
            if sampling.preserve_cubics {
                self.hir.notes.push(format!(
                    "path preprocess: cubics retained={} (quadratics={})",
                    cubic_count_after, quadratic_count_after
                ));
            } else {
                self.hir.notes.push(format!(
                    "path preprocess: cubics={} -> quadratics={}",
                    cubic_count_before, quadratic_count_after
                ));
            }
        }

        match path.storage_decision() {
            PathStorageDecision::ConstModuleEmbedded => {
                self.hir.notes.push(format!(
                    "path: {} segs (const, module-embedded)",
                    path.segment_count()
                ));
            }
            PathStorageDecision::BufferExceedsConstThreshold => {
                self.hir.notes.push(format!(
                    "path: {} segs (buffer: exceeds const threshold {PATH_CONST_SEGMENT_THRESHOLD})",
                    path.segment_count()
                ));
            }
        }

        Some(path)
    }

    /// Check if footprint is available for pattern filtering.
    /// Footprint terms are now always expressible in lowered code and resolve
    /// to either composed in-space Jacobians or identity.
    pub(super) fn has_footprint(&self) -> bool {
        true
    }

    /// Get symbolic footprint Jacobian terms for footprint computation.
    pub(super) fn get_canvas_jacobian(&self) -> (Sx, Sx, Sx, Sx) {
        (
            Sx::FootprintJ11,
            Sx::FootprintJ12,
            Sx::FootprintJ21,
            Sx::FootprintJ22,
        )
    }

    pub(super) fn value_cache_key(value: &Value) -> String {
        format!("{value:?}")
    }

    pub(super) fn record_expr_hotspot(
        &mut self,
        span_start: usize,
        span_end: usize,
        kind: &'static str,
        elapsed: Duration,
    ) {
        let key = ExprHotspotKey {
            span_start,
            span_end,
            kind,
        };
        let entry = self.expr_hotspots.entry(key).or_default();
        entry.total += elapsed;
        entry.max = entry.max.max(elapsed);
        entry.count = entry.count.saturating_add(1);
    }

    pub(super) fn emit_expr_hotspot_summary(&mut self) {
        if self.expr_hotspot_summary_emitted || self.expr_hotspots.is_empty() {
            return;
        }

        let top_n = std::env::var("FRESCO_CHECK_EXPR_HOTSPOT_TOP")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(8);
        if top_n == 0 {
            return;
        }

        let min_ms = std::env::var("FRESCO_CHECK_EXPR_HOTSPOT_MIN_MS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.5);

        let mut ranked = self
            .expr_hotspots
            .iter()
            .map(|(k, s)| (k, s, s.total.as_secs_f64() * 1000.0))
            .filter(|(_, _, total_ms)| *total_ms >= min_ms)
            .collect::<Vec<_>>();

        ranked.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        if ranked.is_empty() {
            return;
        }

        self.expr_hotspot_summary_emitted = true;

        let shown = ranked.len().min(top_n);
        tracing::info!(
            unique_spans = self.expr_hotspots.len(),
            shown = shown,
            min_ms = min_ms,
            "checker expr hotspot summary"
        );

        for (rank, (key, stats, total_ms)) in ranked.into_iter().take(top_n).enumerate() {
            let max_ms = stats.max.as_secs_f64() * 1000.0;
            let avg_ms = if stats.count == 0 {
                0.0
            } else {
                total_ms / stats.count as f64
            };
            tracing::info!(
                rank = rank + 1,
                total_ms = total_ms,
                avg_ms = avg_ms,
                max_ms = max_ms,
                count = stats.count,
                span_start = key.span_start,
                span_end = key.span_end,
                kind = key.kind,
                "checker expr hotspot"
            );
        }
    }

    pub(super) fn timeout_exceeded(&mut self, e: &SExpr) -> bool {
        if self.expr_timeout_reported {
            return true;
        }

        let Some(limit) = self.expr_timeout else {
            return false;
        };

        let elapsed = self.check_started_at.elapsed();
        if elapsed < limit {
            return false;
        }

        self.expr_timeout_reported = true;
        tracing::error!(
            timeout_ms = limit.as_millis() as u64,
            elapsed_ms = elapsed.as_secs_f64() * 1000.0,
            evals = self.expr_watchdog_counter,
            span_start = e.span.start,
            span_end = e.span.end,
            expr_kind = ?e.node,
            scope_depth = self.scopes.len(),
            diags = self.diags.len(),
            "checker expr timeout checkpoint"
        );

        self.diags.push(
            Diag::error(
                e.span.clone(),
                format!(
                    "checker expression evaluation timed out after {}ms",
                    limit.as_millis()
                ),
            )
            .with_help(
                "increase --expr-timeout-ms (or FRESCO_CHECK_EXPR_TIMEOUT_MS) to continue, then inspect timeout checkpoint and hotspot logs",
            ),
        );
        self.emit_expr_hotspot_summary();
        true
    }

    pub(super) fn push_scatter_rand_scope(&mut self, seed: u32) {
        self.scatter_rand_scopes.push(ScatterRandScope { seed });
    }

    pub(super) fn pop_scatter_rand_scope(&mut self) {
        self.scatter_rand_scopes.pop();
    }

    pub(super) fn scatter_rand_unit(&self, call_span: &Span) -> Option<Sx> {
        let scope = self.scatter_rand_scopes.last()?;
        let id = Sx::Add(Box::new(Sx::ScatterInstanceId), Box::new(Sx::Lit(1.0)));
        let seed_term = Sx::Lit(scope.seed as f32 * 78.233);
        let salt_term = Sx::Lit(Self::scatter_rand_call_salt(call_span) as f32 * 37.719);
        let phase = Sx::Add(
            Box::new(Sx::Add(
                Box::new(Sx::Mul(Box::new(id), Box::new(Sx::Lit(12.9898)))),
                Box::new(seed_term),
            )),
            Box::new(salt_term),
        );
        Some(Sx::Fract(Box::new(Sx::Mul(
            Box::new(Sx::Sin(Box::new(phase))),
            Box::new(Sx::Lit(43_758.547)),
        ))))
    }

    fn scatter_rand_call_salt(call_span: &Span) -> u32 {
        let mut x = (call_span.start as u32) ^ (call_span.end as u32).rotate_left(16) ^ 0x9E37_79B9;
        x ^= x >> 16;
        x = x.wrapping_mul(0x7FEB_352D);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846C_A68B);
        x ^ (x >> 16)
    }
}

#[derive(Debug, Clone)]
enum FnValueTy {
    ShaderResource(crate::resource_type::ShaderResourceType),
    Scalar,
    Vec2,
    Vec3,
    Vec4,
    Mat2,
    Mat3,
    Mat4,
    CoordLike,
    Texture,
    Color,
    Shape,
    Layer,
    Struct(String),
    Array(String),
    /// User-defined enum type (e.g., "WaveShape", "Easing")
    Enum(String),
    /// First-class callable reference: `fn(T1, T2) -> T`.
    /// Stores the parameter types and return type of the callable signature.
    Callable {
        params: Vec<FnValueTy>,
        ret: Box<FnValueTy>,
    },
    /// A type variable introduced by a generic function, e.g. `T` in `fn foo<T>(x: T) -> T`.
    TypeVar(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarSpecialization {
    Any,
    F32,
    I32,
    U32,
}

fn scalar_specialization_from_type_name(name: &str) -> ScalarSpecialization {
    match name {
        "i32" => ScalarSpecialization::I32,
        "u32" => ScalarSpecialization::U32,
        "f32" | "f64" | "half" | "float" => ScalarSpecialization::F32,
        _ => ScalarSpecialization::Any,
    }
}

fn split_decl_type_union(type_name: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut angle_depth = 0i32;
    let mut paren_depth = 0i32;
    for (idx, ch) in type_name.char_indices() {
        match ch {
            '<' => angle_depth += 1,
            '>' => angle_depth = (angle_depth - 1).max(0),
            '(' => paren_depth += 1,
            ')' => paren_depth = (paren_depth - 1).max(0),
            '|' if angle_depth == 0 && paren_depth == 0 => {
                let branch = type_name[start..idx].trim();
                if !branch.is_empty() {
                    parts.push(branch.to_string());
                }
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    let tail = type_name[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }

    if parts.is_empty() {
        vec![type_name.to_string()]
    } else {
        parts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FnTypeFamily {
    ShaderResource,
    Scalar,
    Vector,
    Matrix,
    CoordLike,
    Texture,
    Color,
    Shape,
    Layer,
    Struct,
    Array,
    Enum,
    Callable,
    TypeVar,
}

impl FnValueTy {
    fn family(&self) -> FnTypeFamily {
        match self {
            FnValueTy::ShaderResource(_) => FnTypeFamily::ShaderResource,
            FnValueTy::Scalar => FnTypeFamily::Scalar,
            FnValueTy::Vec2 | FnValueTy::Vec3 | FnValueTy::Vec4 => FnTypeFamily::Vector,
            FnValueTy::Mat2 | FnValueTy::Mat3 | FnValueTy::Mat4 => FnTypeFamily::Matrix,
            FnValueTy::CoordLike => FnTypeFamily::CoordLike,
            FnValueTy::Texture => FnTypeFamily::Texture,
            FnValueTy::Color => FnTypeFamily::Color,
            FnValueTy::Shape => FnTypeFamily::Shape,
            FnValueTy::Layer => FnTypeFamily::Layer,
            FnValueTy::Struct(_) => FnTypeFamily::Struct,
            FnValueTy::Array(_) => FnTypeFamily::Array,
            FnValueTy::Enum(_) => FnTypeFamily::Enum,
            FnValueTy::Callable { .. } => FnTypeFamily::Callable,
            FnValueTy::TypeVar(_) => FnTypeFamily::TypeVar,
        }
    }

    fn vector_width(&self) -> Option<u8> {
        match self {
            FnValueTy::Vec2 | FnValueTy::CoordLike => Some(2),
            FnValueTy::Vec3 => Some(3),
            FnValueTy::Vec4 => Some(4),
            _ => None,
        }
    }

    fn matrix_dims(&self) -> Option<(u8, u8)> {
        match self {
            FnValueTy::Mat2 => Some((2, 2)),
            FnValueTy::Mat3 => Some((3, 3)),
            FnValueTy::Mat4 => Some((4, 4)),
            _ => None,
        }
    }

    fn canonical_type_name(&self) -> &'static str {
        match self {
            FnValueTy::ShaderResource(crate::resource_type::ShaderResourceType::Sampler) => {
                "sampler"
            }
            FnValueTy::ShaderResource(crate::resource_type::ShaderResourceType::Data(
                crate::resource_type::ResourceType::Buffer(_),
            )) => "buffer",
            FnValueTy::ShaderResource(crate::resource_type::ShaderResourceType::Data(
                crate::resource_type::ResourceType::Image(_),
            )) => "texture2d",
            FnValueTy::Scalar => "scalar",
            FnValueTy::Vec2 => "vec2",
            FnValueTy::Vec3 => "vec3",
            FnValueTy::Vec4 => "vec4",
            FnValueTy::Mat2 => "mat2",
            FnValueTy::Mat3 => "mat3",
            FnValueTy::Mat4 => "mat4",
            FnValueTy::CoordLike => "coord_like",
            FnValueTy::Texture => "texture",
            FnValueTy::Color => "color",
            FnValueTy::Shape => "shape",
            FnValueTy::Layer => "layer",
            FnValueTy::Struct(_) => "struct",
            FnValueTy::Array(_) => "array",
            FnValueTy::Enum(_) => "enum",
            FnValueTy::Callable { .. } => "fn reference",
            FnValueTy::TypeVar(_) => "type variable",
        }
    }
}

#[derive(Debug, Clone)]
struct FnParamDef {
    scalar_kind: crate::typed_scalar::Kind,
    is_context: bool,
    name: String,
    ty: FnValueTy,
    scalar_specialization: ScalarSpecialization,
    keyword_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstTemplateValue {
    U32(u32),
    I32(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstTemplateParamTy {
    U32,
    I32,
}

#[derive(Debug, Clone)]
struct ConstTemplateParamDef {
    name: String,
    ty: ConstTemplateParamTy,
}

#[derive(Debug, Clone)]
struct FnDef {
    infer_return: bool,
    ret_kind: crate::typed_scalar::Kind,
    params: Vec<FnParamDef>,
    ret: FnValueTy,
    is_internal: bool,
    is_builtin: bool,
    source_file: String,
    body: Vec<Stmt>,
    span: Span,
    /// Generic type parameters: (var_name, bound_names).
    type_params: Vec<(String, Vec<String>)>,
    const_params: Vec<ConstTemplateParamDef>,
    const_bindings: Vec<(String, ConstTemplateValue)>,
}

impl FnDef {
    fn declaration_identity(&self, name: &str) -> (String, String, Span) {
        (
            name.to_string(),
            self.source_file.clone(),
            self.span.clone(),
        )
    }

    fn same_overload_signature(&self, other: &FnDef) -> bool {
        self.params.len() == other.params.len()
            && self.const_params.len() == other.const_params.len()
            && self
                .const_params
                .iter()
                .zip(other.const_params.iter())
                .all(|(a, b)| a.ty == b.ty)
            && self.params.iter().zip(other.params.iter()).all(|(a, b)| {
                if let (FnValueTy::ShaderResource(a), FnValueTy::ShaderResource(b)) = (&a.ty, &b.ty)
                    && a != b
                {
                    return false;
                }
                a.ty.family() == b.ty.family()
                    && a.ty.vector_width() == b.ty.vector_width()
                    && a.scalar_specialization == b.scalar_specialization
                    && a.scalar_kind == b.scalar_kind
                    && a.keyword_only == b.keyword_only
            })
    }
}

/// Registered user-defined effect, captured at effect-check time and indexed
/// by name in `Checker::effect_defs` for call-site lookup.
#[allow(
    dead_code,
    reason = "Effect registration fields are staged for locality soundness checks and diagnostics"
)]
#[derive(Debug, Clone)]
struct EffectRegistered {
    /// The declared name of this effect (used in diagnostics).
    name: String,
    /// Index into `hir.effects` (set after `check_effect_decl` completes).
    def_idx: usize,
    /// Number of declared parameters (for arity checking at call sites).
    param_count: usize,
    /// The resolved locality of this effect.
    locality: hir::Locality,
    /// Parameter names in declaration order.
    param_names: Vec<String>,
    /// Span of the effect declaration for diagnostics.
    span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct EnumDef {
    pub(super) variants: HashMap<String, Span>,
}

#[derive(Debug, Clone)]
pub(super) struct StructFieldDef {
    pub(super) semantic: Option<String>,
    pub(super) ty_name: String,
    #[allow(dead_code, reason = "field type span retained for richer diagnostics")]
    pub(super) span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct StructDef {
    pub(super) fields: HashMap<String, StructFieldDef>,
}

#[allow(
    dead_code,
    reason = "Interface metadata is staged for fuller method-level conformance checks"
)]
#[derive(Debug, Clone)]
struct InterfaceDef {
    /// Method names declared in the interface (just the names, for v1 bound checking).
    method_names: Vec<String>,
}

#[derive(Debug, Clone)]
struct VertexContractMemberDef {
    name: String,
    ty_name: String,
    optional: bool,
    default: Option<SExpr>,
    span: Span,
}

#[derive(Debug, Clone)]
struct VertexInterfaceDef {
    members: Vec<VertexContractMemberDef>,
}

#[derive(Debug, Clone)]
struct VertexFormatDef {
    parent: Option<String>,
    members: Vec<VertexContractMemberDef>,
}

#[allow(
    dead_code,
    reason = "Vertex factory metadata is staged for future lowering support"
)]
#[derive(Debug, Clone)]
struct VertexFactoryDef {
    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    target_format: String,
    #[allow(
        dead_code,
        reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
    )]
    target_format_span: Span,
}

#[derive(Debug, Clone)]
struct StyleDef {
    params: Vec<StyleParam>,
    stages: Vec<StyleStage>,
    span: Span,
}

#[derive(Debug, Default)]
struct ReturnPathAnalysis {
    fallthrough_value: Option<(Value, Span)>,
    guaranteed_return: Option<(Value, Span)>,
    guaranteed_return_span: Option<Span>,
    break_span: Option<Span>,
    conditional_returns: Vec<(Sx, Value, Span)>,
    conditional_break: Option<Sx>,
}

/// Parse a canonical callable type string produced by the parser.
/// Format: `fn(T1,T2)->T` or `fn(T1,T2)` (no return type means `f32` fallback).
/// Returns `Some((param_type_names, return_type_name))` on success, `None` if not a callable string.
pub(super) fn try_parse_callable_ty(name: &str) -> Option<(Vec<String>, String)> {
    let rest = name.strip_prefix("fn(")?;
    let (params_str, after_paren) = rest.rsplit_once(')')?;
    let params: Vec<String> = if params_str.is_empty() {
        Vec::new()
    } else {
        params_str
            .split(',')
            .map(|s| s.trim().to_string())
            .collect()
    };
    let ret = if let Some(ret_str) = after_paren.strip_prefix("->") {
        ret_str.trim().to_string()
    } else {
        "f32".to_string()
    };
    Some((params, ret))
}

/// Parse `base<arg1,arg2,...>` type applications with nesting support.
pub(super) fn try_parse_type_application(name: &str) -> Option<(String, Vec<String>)> {
    let name = name.trim();
    let lt = name.find('<')?;
    if !name.ends_with('>') {
        return None;
    }
    let base = name[..lt].trim();
    if base.is_empty() {
        return None;
    }

    let mut args = Vec::new();
    let mut current = String::new();
    let mut angle_depth = 0i32;
    let mut paren_depth = 0i32;
    for ch in name[lt + 1..name.len() - 1].chars() {
        match ch {
            '<' => {
                angle_depth += 1;
                current.push(ch);
            }
            '>' => {
                if angle_depth == 0 {
                    return None;
                }
                angle_depth -= 1;
                current.push(ch);
            }
            '(' => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' => {
                if paren_depth == 0 {
                    return None;
                }
                paren_depth -= 1;
                current.push(ch);
            }
            ',' if angle_depth == 0 && paren_depth == 0 => {
                let arg = current.trim();
                if arg.is_empty() {
                    return None;
                }
                args.push(arg.to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if angle_depth != 0 || paren_depth != 0 {
        return None;
    }
    let arg = current.trim();
    if arg.is_empty() {
        return None;
    }
    args.push(arg.to_string());
    Some((base.to_string(), args))
}

pub(super) fn type_name_to_key(name: &str) -> String {
    let name = strip_spatial_type_suffix(name);

    if let Some((base, args)) = try_parse_type_application(name)
        && base == "field"
        && args.len() == 1
    {
        return type_name_to_key(&args[0]);
    }
    if let Some((base, _args)) = try_parse_type_application(name)
        && base == "texture"
    {
        return "typed_texture".to_string();
    }

    match name {
        "f32" | "float" | "signal" | "mask" | "coverage" | "delta" => "scalar".to_string(),
        "vec2" | "uvec2" | "resolution" | "coord" | "coord_like" => "vec2".to_string(),
        "vec3" | "uvec3" => "vec3".to_string(),
        "vec4" | "uvec4" => "vec4".to_string(),
        "mat2" => "mat2".to_string(),
        "mat3" => "mat3".to_string(),
        "mat4" => "mat4".to_string(),
        "color" => "color".to_string(),
        "shape" => "shape".to_string(),
        "layer" => "layer".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn value_to_type_key(value: &Value) -> &'static str {
    match value {
        Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_) => "scalar",
        Value::Slot { .. } => "slot",
        Value::Vec2(_) => "vec2",
        Value::Vec3(_) => "vec3",
        Value::Vec4(_) => "vec4",
        Value::Mat2(_) => "mat2",
        Value::Mat3(_) => "mat3",
        Value::Mat4(_) => "mat4",
        Value::Color { .. } | Value::ColorField { .. } | Value::Gradient { .. } => "color",
        Value::Shape(_) => "shape",
        Value::Layer(_) => "layer",
        Value::Array(_) => "array",
        Value::FnRef(_) => "fn_ref",
        Value::Lambda { .. } => "lambda",
        Value::Space(_) => "space",
        Value::TypedTextureSample { .. } => "typed_texture",
        Value::ScatterInstance { .. } => "scatter_instance",
        Value::RepeatCell(_) => "repeat_cell",
        Value::CellContour { .. } => "contour",
        Value::PathFuture(_) => "path",
        Value::DynamicArray { .. } => "dynamic_array",
        Value::Struct { .. } => "struct",
        Value::Error => "error",
    }
}

/// Unit resolution (§7):
/// - deg -> radians (constant-folded)
/// - px -> runtime pixel-scaled literal (via `PxLit`)
/// - vw/vh/vmin/vmax -> CSS-like viewport percentages
/// - uv/unitless/s -> plain scalar literals
fn scalar_lit(v: f64, unit: Unit) -> Sx {
    let lit = |x: f32| Sx::Lit(x);
    // No checker context here to resolve the real `resolution()` runtime
    // channel (see `Checker::scalar_lit_runtime`, which intercepts `vw`/
    // `vmin`/`vmax` before falling back to this function) — treat the
    // viewport as square for this const-only path.
    let viewport_ratio = || Sx::Lit(1.0);
    let viewport_scale =
        |percent: f32, axis_factor: Sx| Sx::Mul(Box::new(lit(percent)), Box::new(axis_factor));

    match unit {
        Unit::Deg => lit((v * PI / 180.0) as f32),
        Unit::Turn => lit((v * 2.0 * PI) as f32),
        Unit::Px => Sx::PxLit(v as f32),
        Unit::MilliSec => lit((v * 0.001) as f32),
        Unit::Vh => lit((v * 0.01) as f32),
        Unit::Vw => viewport_scale((v * 0.01) as f32, viewport_ratio()),
        Unit::Vmin => viewport_scale(
            (v * 0.01) as f32,
            Sx::Min(Box::new(viewport_ratio()), Box::new(lit(1.0))),
        ),
        Unit::Vmax => viewport_scale(
            (v * 0.01) as f32,
            Sx::Max(Box::new(viewport_ratio()), Box::new(lit(1.0))),
        ),
        Unit::Uv | Unit::None | Unit::Sec => lit(v as f32),
    }
}

impl Checker {
    /// Like [`scalar_lit`], but resolves viewport-relative units (`vw`,
    /// `vmin`, `vmax`) against the `resolution()` runtime channel — see
    /// [`Checker::runtime_resolution`] — instead of the legacy `Sx::ResX` /
    /// `Sx::ResY` leaves directly.
    pub(super) fn scalar_lit_runtime(&mut self, v: f64, unit: Unit) -> Sx {
        if !matches!(unit, Unit::Vw | Unit::Vmin | Unit::Vmax) {
            return scalar_lit(v, unit);
        }

        let lit = |x: f32| Sx::Lit(x);
        let (res_x, res_y) = self.runtime_resolution();
        let viewport_ratio = Sx::Div(Box::new(res_x), Box::new(res_y));
        let viewport_scale =
            |percent: f32, axis_factor: Sx| Sx::Mul(Box::new(lit(percent)), Box::new(axis_factor));

        match unit {
            Unit::Vw => viewport_scale((v * 0.01) as f32, viewport_ratio),
            Unit::Vmin => viewport_scale(
                (v * 0.01) as f32,
                Sx::Min(Box::new(viewport_ratio), Box::new(lit(1.0))),
            ),
            Unit::Vmax => viewport_scale(
                (v * 0.01) as f32,
                Sx::Max(Box::new(viewport_ratio), Box::new(lit(1.0))),
            ),
            _ => unreachable!("guarded above"),
        }
    }
}

/// Bridges the native (Rust `registry`-declared) enums — `BlendMode` and
/// friends — into the parser's `EnumDecl` shape so the checker can register
/// them as known enum types alongside user- and engine-authored enums.
pub(super) fn native_enums_program() -> Program {
    let mut enums: Vec<EnumDecl> = crate::registry::enum_decls()
        .iter()
        .copied()
        .map(|decl| EnumDecl {
            name: decl.id.0.to_string(),
            name_span: 0..0,
            variants: decl
                .variants
                .iter()
                .map(|variant| EnumVariant {
                    name: variant.name.to_string(),
                    span: 0..0,
                })
                .collect(),
            span: 0..0,
        })
        .collect();

    enums.sort_by(|a, b| a.name.cmp(&b.name));

    Program {
        enums,
        ..Program::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{declared_type_text_compatible, validate_pipeline_skeleton_decls};
    use crate::ast::{
        BinOp, Expr, PassBindingDecl, PassDecl, PassPermutationDecl, PassRequirementClause,
        PassRequirementDecl, PipelineAttribute, PipelineDecl, PipelinePassRef, Program, SExpr,
        Spanned,
    };

    fn known_attr() -> PipelineAttribute {
        PipelineAttribute {
            expressions: Vec::new(),
            name: "known".to_string(),
            name_span: 0..0,
            args: vec!["compile".to_string()],
            args_span: Some(0..0),
            span: 0..0,
        }
    }

    fn editor_only_attr() -> PipelineAttribute {
        PipelineAttribute {
            expressions: Vec::new(),
            name: "editor_only".to_string(),
            name_span: 0..0,
            args: Vec::new(),
            args_span: None,
            span: 0..0,
        }
    }

    fn config_editor_attr() -> PipelineAttribute {
        PipelineAttribute {
            expressions: Vec::new(),
            name: "config".to_string(),
            name_span: 0..0,
            args: vec!["editor".to_string()],
            args_span: Some(0..0),
            span: 0..0,
        }
    }

    fn pass_binding(name: &str, signature: &str) -> PassBindingDecl {
        PassBindingDecl {
            operation_alias: None,
            group_index: None,
            binding_index: None,
            name: name.to_string(),
            name_span: 0..0,
            attrs: vec![known_attr()],
            value_signature: Some(signature.to_string()),
            span: 0..0,
        }
    }

    fn expr_var(name: &str) -> SExpr {
        Spanned {
            node: Expr::Var(name.to_string()),
            span: 0..0,
        }
    }

    fn expr_eq(lhs: &str, rhs: &str) -> SExpr {
        Spanned {
            node: Expr::Binary(BinOp::Eq, Box::new(expr_var(lhs)), Box::new(expr_var(rhs))),
            span: 0..0,
        }
    }

    fn pipeline_pass_ref(name: &str) -> PipelinePassRef {
        PipelinePassRef {
            invocation: None,
            name: name.to_string(),
            name_span: 0..0,
            attrs: Vec::new(),
            span: 0..0,
        }
    }

    fn pass_permutation(name: &str, signature: &str) -> PassPermutationDecl {
        PassPermutationDecl {
            name: name.to_string(),
            name_span: 0..0,
            attrs: vec![known_attr()],
            values: vec![Spanned {
                node: signature.to_string(),
                span: 0..0,
            }],
            when_guard: None,
            else_value: None,
            value_signature: Some(signature.to_string()),
            span: 0..0,
        }
    }

    fn pass_requirement() -> PassRequirementDecl {
        PassRequirementDecl {
            clause: PassRequirementClause::Implication {
                guard: expr_var("true"),
                constraint: expr_var("false"),
            },
            span: 0..0,
        }
    }

    fn disable_requirement(guard: SExpr, axes: &[&str]) -> PassRequirementDecl {
        PassRequirementDecl {
            clause: PassRequirementClause::Implication {
                guard,
                constraint: Spanned {
                    node: Expr::Call {
                        name: "disable".to_string(),
                        name_span: 0..0,
                        const_args: Vec::new(),
                        args: axes
                            .iter()
                            .map(|axis| crate::ast::Arg {
                                name: None,
                                value: expr_var(axis),
                            })
                            .collect(),
                    },
                    span: 0..0,
                },
            },
            span: 0..0,
        }
    }

    fn pass_decl(
        name: &str,
        reads: &[&str],
        permutations: Vec<PassPermutationDecl>,
        requirements: Vec<PassRequirementDecl>,
        bindings: Vec<PassBindingDecl>,
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
            attrs: Vec::new(),
            stage: None,
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
            requirements,
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
            pipeline_type: "compute".to_string(),
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
                .map(|name| pipeline_pass_ref(name))
                .collect(),
            span: 0..0,
        }
    }

    #[test]
    fn validates_shared_known_axes_for_read_linked_passes() {
        let producer = pass_decl(
            "cluster_cull",
            &[],
            vec![pass_permutation("tile_size", "buffer_u32_cluster_lights")],
            Vec::new(),
            vec![
                pass_binding("tile_size", "buffer_u32_cluster_lights"),
                pass_binding("max_per_cluster", "buffer_u32_cluster_lights"),
            ],
        );
        let consumer = pass_decl(
            "fp_shade",
            &["cluster_cull"],
            vec![pass_permutation(
                "max_per_cluster",
                "buffer_u32_cluster_lights",
            )],
            Vec::new(),
            vec![pass_binding("tile_size", "buffer_u32_cluster_lights")],
        );
        let pipeline = pipeline_decl("forward_plus", &["cluster_cull", "fp_shade"]);

        let program = Program {
            passes: vec![producer, consumer],
            pipelines: vec![pipeline],
            ..Program::default()
        };

        let result = validate_pipeline_skeleton_decls(&program);
        let diags = result.expect_err("missing shared axes should fail validation");
        assert!(
            diags.iter().any(|diag| {
                diag.message.contains("layout dependency mismatch")
                    && diag.message.contains("max_per_cluster")
            }),
            "expected shared-axis mismatch diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn accepts_matching_shared_known_axes_for_read_linked_passes() {
        let producer = pass_decl(
            "cluster_cull",
            &[],
            vec![pass_permutation("tile_size", "buffer_u32_cluster_lights")],
            Vec::new(),
            vec![
                pass_binding("tile_size", "buffer_u32_cluster_lights"),
                pass_binding("max_per_cluster", "buffer_u32_cluster_lights"),
            ],
        );
        let consumer = pass_decl(
            "fp_shade",
            &["cluster_cull"],
            vec![pass_permutation("tile_size", "buffer_u32_cluster_lights")],
            Vec::new(),
            vec![
                pass_binding("tile_size", "buffer_u32_cluster_lights"),
                pass_binding("max_per_cluster", "buffer_u32_cluster_lights"),
            ],
        );
        let pipeline = pipeline_decl("forward_plus", &["cluster_cull", "fp_shade"]);

        let program = Program {
            passes: vec![producer, consumer],
            pipelines: vec![pipeline],
            ..Program::default()
        };

        let result = validate_pipeline_skeleton_decls(&program);
        assert!(
            result.is_ok(),
            "matching shared axes should pass validation"
        );
    }

    #[test]
    fn accepts_matching_shared_known_axes_within_one_pipeline() {
        let base = pass_decl(
            "fwd_base",
            &[],
            vec![pass_permutation("fog", "FogMode")],
            Vec::new(),
            Vec::new(),
        );
        let ambient = pass_decl(
            "deferred_ambient",
            &[],
            vec![pass_permutation("fog", "FogMode")],
            Vec::new(),
            Vec::new(),
        );
        let pipeline = pipeline_decl("deferred", &["fwd_base", "deferred_ambient"]);

        let program = Program {
            passes: vec![base, ambient],
            pipelines: vec![pipeline],
            ..Program::default()
        };

        assert!(
            validate_pipeline_skeleton_decls(&program).is_ok(),
            "matching shared axis domains should pass validation"
        );
    }

    #[test]
    fn rejects_shared_known_axis_with_mismatched_domain_within_one_pipeline() {
        let base = pass_decl(
            "fwd_base",
            &[],
            vec![pass_permutation("fog", "FogMode")],
            Vec::new(),
            Vec::new(),
        );
        let ambient = pass_decl(
            "deferred_ambient",
            &[],
            vec![pass_permutation("fog", "off|linear|exp2")],
            Vec::new(),
            Vec::new(),
        );
        let pipeline = pipeline_decl("deferred", &["fwd_base", "deferred_ambient"]);

        let program = Program {
            passes: vec![base, ambient],
            pipelines: vec![pipeline],
            ..Program::default()
        };

        let diags = validate_pipeline_skeleton_decls(&program)
            .expect_err("mismatched shared axis domain should fail validation");
        assert!(
            diags.iter().any(|diag| {
                diag.message.contains("pipeline `deferred` axis `fog`")
                    && diag.message.contains("fogmode")
                    && diag.message.contains("off|linear|exp2")
            }),
            "expected shared-axis mismatch diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn rejects_require_that_prunes_all_variants() {
        let pass = pass_decl(
            "tone_map",
            &[],
            vec![pass_permutation("mode", "linear")],
            vec![pass_requirement()],
            Vec::new(),
        );

        let program = Program {
            passes: vec![pass],
            pipelines: Vec::new(),
            ..Program::default()
        };

        let result = validate_pipeline_skeleton_decls(&program);
        let diags = result.expect_err("pruning away every variant should fail validation");
        assert!(
            diags.iter().any(|diag| {
                diag.message.contains("no surviving permutation variants")
                    && diag
                        .message
                        .contains("matching `require` clause pruned the last one")
            }),
            "expected pruning diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn declared_type_compatibility_allows_unlabeled_to_labeled_bridge() {
        assert!(declared_type_text_compatible("vec3", "vec3 in world"));
        assert!(declared_type_text_compatible("vec3 in world", "vec3"));
    }

    #[test]
    fn declared_type_compatibility_rejects_mismatched_labeled_spaces() {
        assert!(!declared_type_text_compatible(
            "vec3 in world",
            "vec3 in object"
        ));
        assert!(!declared_type_text_compatible(
            "mat4 from world to clip",
            "mat4 from object to clip"
        ));
    }

    #[test]
    fn validates_disable_clause_with_multiple_axes() {
        let pass = pass_decl(
            "fwd_base",
            &[],
            vec![
                PassPermutationDecl {
                    name: "main".to_string(),
                    name_span: 0..0,
                    attrs: vec![known_attr()],
                    values: vec![
                        Spanned {
                            node: "shadowed".to_string(),
                            span: 0..0,
                        },
                        Spanned {
                            node: "lit".to_string(),
                            span: 0..0,
                        },
                    ],
                    when_guard: None,
                    else_value: None,
                    value_signature: Some("shadowed|lit".to_string()),
                    span: 0..0,
                },
                pass_permutation("cascades", "4"),
                pass_permutation("cascade_taps", "5"),
            ],
            vec![disable_requirement(
                expr_eq("main", "shadowed"),
                &["cascades", "cascade_taps"],
            )],
            Vec::new(),
        );

        let program = Program {
            passes: vec![pass],
            pipelines: Vec::new(),
            ..Program::default()
        };

        assert!(validate_pipeline_skeleton_decls(&program).is_ok());
    }

    #[test]
    fn rejects_permutation_without_known_attribute() {
        let pass = pass_decl(
            "fwd_base",
            &[],
            vec![PassPermutationDecl {
                name: "ambient".to_string(),
                name_span: 0..0,
                attrs: Vec::new(),
                values: vec![Spanned {
                    node: "flat".to_string(),
                    span: 0..0,
                }],
                when_guard: None,
                else_value: None,
                value_signature: Some("flat".to_string()),
                span: 0..0,
            }],
            Vec::new(),
            Vec::new(),
        );

        let program = Program {
            passes: vec![pass],
            pipelines: Vec::new(),
            ..Program::default()
        };

        let diags = validate_pipeline_skeleton_decls(&program)
            .expect_err("missing @known should fail validation");
        assert!(
            diags.iter().any(|diag| diag
                .message
                .contains("must declare `@known(...)` explicitly")),
            "expected explicit-known diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn rejects_editor_config_axis_on_non_editor_pass() {
        let pass = PassDecl {
            service_captures: Vec::new(),
            compute_invocation: None,
            prepared_draw: None,
            preparation: None,
            operation: None,
            state: Vec::new(),
            entry_bindings: Vec::new(),
            entry_properties: Vec::new(),
            name: "preview_tone".to_string(),
            name_span: 0..0,
            source_file: "main.fr".to_string(),
            material_name: None,
            material_span: None,
            attrs: Vec::new(),
            stage: None,
            draw: None,
            blend: None,
            reads: Vec::new(),
            writes: Vec::new(),
            permutations: vec![PassPermutationDecl {
                name: "zoom_mode".to_string(),
                name_span: 0..0,
                attrs: vec![known_attr(), config_editor_attr()],
                values: vec![Spanned {
                    node: "identity".to_string(),
                    span: 0..0,
                }],
                when_guard: None,
                else_value: None,
                value_signature: Some("identity".to_string()),
                span: 0..0,
            }],
            requirements: Vec::new(),
            bindings: Vec::new(),
            hooks: Vec::new(),
            vertex_interface: None,
            span: 0..0,
        };

        let program = Program {
            passes: vec![pass],
            pipelines: Vec::new(),
            ..Program::default()
        };

        let diags = validate_pipeline_skeleton_decls(&program)
            .expect_err("editor config without @editor_only should fail");
        assert!(
            diags.iter().any(|diag| {
                diag.message.contains("@config(editor)") && diag.message.contains("@editor_only")
            }),
            "expected editor-config leak diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn accepts_editor_config_axis_on_editor_only_pass() {
        let pass = PassDecl {
            service_captures: Vec::new(),
            compute_invocation: None,
            prepared_draw: None,
            preparation: None,
            operation: None,
            state: Vec::new(),
            entry_bindings: Vec::new(),
            entry_properties: Vec::new(),
            name: "preview_tone".to_string(),
            name_span: 0..0,
            source_file: "main.fr".to_string(),
            material_name: None,
            material_span: None,
            attrs: vec![editor_only_attr()],
            stage: None,
            draw: None,
            blend: None,
            reads: Vec::new(),
            writes: Vec::new(),
            permutations: vec![PassPermutationDecl {
                name: "zoom_mode".to_string(),
                name_span: 0..0,
                attrs: vec![known_attr(), config_editor_attr()],
                values: vec![Spanned {
                    node: "identity".to_string(),
                    span: 0..0,
                }],
                when_guard: None,
                else_value: None,
                value_signature: Some("identity".to_string()),
                span: 0..0,
            }],
            requirements: Vec::new(),
            bindings: Vec::new(),
            hooks: Vec::new(),
            vertex_interface: None,
            span: 0..0,
        };

        let program = Program {
            passes: vec![pass],
            pipelines: Vec::new(),
            ..Program::default()
        };

        assert!(
            validate_pipeline_skeleton_decls(&program).is_ok(),
            "editor-only pass with editor config axis should validate"
        );
    }
}
