use super::declarations::create_checker_for_surface;
use super::{CheckOptions, Checker, Value};
use crate::ast::{
    Arg, ComposeEntry, Expr, FnDecl, InterfaceDecl, MaterialPropertiesDecl, MaterialReturnTy,
    PassDecl, PipelineDecl, SchemaEvaluatorContractDecl, SchemaEvaluatorDecl,
    SchemaEvaluatorPermutationDecl, SchemaEvaluatorVariantDecl, SchemaExpressionDecl,
    SchemaProgramDecl, Span, Stmt, SurfaceDecl,
};
use crate::diag::{Diag, Severity};
use crate::hir::Sx;
use crate::material_hir::{
    EvaluationVariant, MaterialChannel, MaterialHir, MaterialLayer, MaterialValue,
    MaterialVertexProgram,
};
use std::collections::{BTreeSet, HashMap, HashSet};

struct CheckedMaterialLayer {
    is_base: bool,
    weight: Option<Sx>,
    channels: HashMap<String, Value>,
}

fn material_value(value: Value) -> MaterialValue {
    match value {
        Value::Scalar(v) | Value::Distance(v) | Value::Coverage(v) | Value::Mask(v) => {
            MaterialValue::Scalar(v)
        }
        Value::Vec2((x, y)) => MaterialValue::Vector(vec![x, y]),
        Value::Vec3((x, y, z)) => MaterialValue::Vector(vec![x, y, z]),
        Value::Vec4((x, y, z, w)) => MaterialValue::Vector(vec![x, y, z, w]),
        Value::Color { rgba, .. } => MaterialValue::Vector(rgba.map(Sx::Lit).to_vec()),
        Value::ColorField { rgba, .. } => MaterialValue::Vector(rgba.to_vec()),
        Value::Mat2((a, b)) => {
            MaterialValue::Matrix([a, b].into_iter().map(|(x, y)| vec![x, y]).collect())
        }
        Value::Mat3(v) => MaterialValue::Matrix(
            [v.0, v.1, v.2]
                .into_iter()
                .map(|(x, y, z)| vec![x, y, z])
                .collect(),
        ),
        Value::Mat4(v) => MaterialValue::Matrix(
            [v.0, v.1, v.2, v.3]
                .into_iter()
                .map(|(x, y, z, w)| vec![x, y, z, w])
                .collect(),
        ),
        Value::Array(values) => {
            MaterialValue::Array(values.into_iter().map(material_value).collect())
        }
        Value::Struct { fields, .. } => MaterialValue::Record(
            fields
                .into_iter()
                .map(|(k, v)| (k, material_value(v)))
                .collect(),
        ),
        _ => unreachable!("checked material value type"),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedMaterialChannel {
    pub name: String,
    pub ty_name: String,
    pub default_payload: Option<Value>,
    pub compose: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedMaterialModel {
    pub parent: Option<String>,
    pub context: Option<crate::context::EntryContext>,
    pub composition: Option<[String; 3]>,
    pub is_default: bool,
    pub evaluation: Option<String>,
    pub channels: Vec<ValidatedMaterialChannel>,
    pub schema_program_name: Option<String>,
    pub program: Option<crate::driver::schema_function::SchemaFunction>,
    pub schema_program_names: Vec<String>,
    pub schema_evaluator_name: Option<String>,
    pub schema_evaluator_names: Vec<String>,
    pub evaluation_contract: Option<SchemaEvaluatorContractDecl>,
    pub evaluation_permutations: Vec<SchemaEvaluatorPermutationDecl>,
    pub evaluation_specialize_pattern: Option<String>,
    pub evaluation_specialize_pattern_span: Option<Span>,
    pub evaluation_function: Option<crate::driver::schema_function::SchemaFunction>,
    pub evaluation_shader_source: Option<String>,
    pub evaluation_variants: Vec<SchemaEvaluatorVariantDecl>,
    pub evaluation_variant_bodies: Vec<EvaluationVariant>,
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub(crate) struct ValidatedSchemaExpression {
    pub name: String,
    pub properties_name: String,
    pub function: crate::driver::schema_function::SchemaFunction,
}

#[allow(
    dead_code,
    reason = "Retain compiler metadata and shared support APIs used by other compilation paths."
)]
#[derive(Debug, Clone)]
pub(crate) struct ResolvedMaterialProperties {
    pub context: Option<crate::context::EntryContext>,
    pub channels: Vec<ValidatedMaterialChannel>,
}

const ALLOWED_MATERIAL_CHANNEL_TYPES: &[&str] = &[
    "f32", "f64", "half", "i32", "u32", "bool", "angle", "length", "vec2", "vec3", "vec4", "color",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaterialPropertiesResolutionState {
    Resolving,
    Resolved,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SpatialQualifier {
    In(String),
    FromTo { from: String, to: String },
}

fn spatial_qualifier(ty_name: &str) -> Option<SpatialQualifier> {
    let trimmed = ty_name.trim();

    if let Some((base, suffix)) = trimmed.rsplit_once(" in ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
    {
        return Some(SpatialQualifier::In(suffix.trim().to_string()));
    }

    if let Some((base, suffix)) = trimmed.rsplit_once(" from ")
        && !base.trim().is_empty()
        && !suffix.trim().is_empty()
        && let Some((src, dst)) = suffix.rsplit_once(" to ")
        && !src.trim().is_empty()
        && !dst.trim().is_empty()
    {
        return Some(SpatialQualifier::FromTo {
            from: src.trim().to_string(),
            to: dst.trim().to_string(),
        });
    }

    None
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(crate) fn validate_material_properties(
    material_properties: &[MaterialPropertiesDecl],
    functions: &[FnDecl],
    consts: &[crate::ast::ConstDecl],
    enums: &[crate::ast::EnumDecl],
    structs: &[crate::ast::StructDecl],
    params: &[crate::ast::GlobalParamDecl],
    texture_types: &[crate::ast::TextureTypeDecl],
    interfaces: &[InterfaceDecl],
    conformances: &[crate::ast::ConformanceDecl],
    effects: &[crate::ast::EffectDecl],
    options: &CheckOptions,
) -> Result<HashMap<String, ResolvedMaterialProperties>, Vec<Diag>> {
    let defaults = material_properties
        .iter()
        .filter(|decl| decl.is_default)
        .collect::<Vec<_>>();
    if defaults.len() > 1 {
        return Err(vec![Diag::error(defaults[1].name_span.clone(), "multiple @default_material declarations are ambiguous")
            .with_help("select exactly one engine default profile; named profiles remain available explicitly")]);
    }
    let mut seen_names: HashMap<String, Span> = HashMap::new();
    let mut diags = Vec::new();
    let mut decls_by_name: HashMap<String, &MaterialPropertiesDecl> = HashMap::new();

    for decl in material_properties {
        if let Some(first_span) = seen_names.get(&decl.name) {
            diags.push(
                Diag::error(
                    decl.name_span.clone(),
                    format!("duplicate material_properties declaration `{}`", decl.name),
                )
                .with_help(format!(
                    "first declaration appears at bytes {}..{}",
                    first_span.start, first_span.end
                )),
            );
            continue;
        }
        seen_names.insert(decl.name.clone(), decl.name_span.clone());
        decls_by_name.insert(decl.name.clone(), decl);
    }

    let mut resolved = HashMap::new();
    let mut states = HashMap::<String, MaterialPropertiesResolutionState>::new();
    let mut stack = Vec::new();

    fn resolve_material_properties_decl(
        name: &str,
        decls_by_name: &HashMap<String, &MaterialPropertiesDecl>,
        resolved: &mut HashMap<String, ResolvedMaterialProperties>,
        states: &mut HashMap<String, MaterialPropertiesResolutionState>,
        stack: &mut Vec<String>,
        functions: &[FnDecl],
        consts: &[crate::ast::ConstDecl],
        enums: &[crate::ast::EnumDecl],
        structs: &[crate::ast::StructDecl],
        params: &[crate::ast::GlobalParamDecl],
        texture_types: &[crate::ast::TextureTypeDecl],
        interfaces: &[InterfaceDecl],
        conformances: &[crate::ast::ConformanceDecl],
        effects: &[crate::ast::EffectDecl],
        options: &CheckOptions,
        diags: &mut Vec<Diag>,
    ) -> Option<ResolvedMaterialProperties> {
        match states.get(name).copied() {
            Some(MaterialPropertiesResolutionState::Resolved) => {
                return resolved.get(name).cloned();
            }
            Some(MaterialPropertiesResolutionState::Failed) => return None,
            Some(MaterialPropertiesResolutionState::Resolving) => {
                if let Some(cycle_start) = stack.iter().position(|n| n == name) {
                    let mut chain = stack[cycle_start..].to_vec();
                    chain.push(name.to_string());
                    if let Some(decl) = decls_by_name.get(name) {
                        diags.push(
                            Diag::error(
                                decl.name_span.clone(),
                                format!(
                                    "cyclic material_properties inheritance involving `{}`",
                                    name
                                ),
                            )
                            .with_label("inheritance cycle")
                            .with_help(format!("inheritance chain: {}", chain.join(" -> "))),
                        );
                    }
                    for item in &stack[cycle_start..] {
                        states.insert(item.clone(), MaterialPropertiesResolutionState::Failed);
                    }
                }
                states.insert(name.to_string(), MaterialPropertiesResolutionState::Failed);
                return None;
            }
            None => {}
        }

        let Some(decl) = decls_by_name.get(name).copied() else {
            states.insert(name.to_string(), MaterialPropertiesResolutionState::Failed);
            return None;
        };

        states.insert(
            name.to_string(),
            MaterialPropertiesResolutionState::Resolving,
        );
        stack.push(name.to_string());
        let diag_start = diags.len();

        let mut channels = if let Some(parent_name) = &decl.extends_name {
            match resolve_material_properties_decl(
                parent_name,
                decls_by_name,
                resolved,
                states,
                stack,
                functions,
                consts,
                enums,
                structs,
                params,
                texture_types,
                interfaces,
                conformances,
                effects,
                options,
                diags,
            ) {
                Some(parent) => parent.channels,
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };

        let context_name = decl
            .context
            .clone()
            .or_else(|| {
                decl.extends_name
                    .as_ref()
                    .and_then(|name| resolved.get(name))
                    .and_then(|parent| parent.context.as_ref())
                    .map(|context| context.ty.name.clone())
            })
            .or_else(|| {
                decls_by_name
                    .values()
                    .find(|decl| decl.is_default)
                    .and_then(|decl| decl.context.clone())
            });
        let context_parameter = decl
            .context_parameter
            .clone()
            .or_else(|| {
                decl.extends_name
                    .as_ref()
                    .and_then(|name| resolved.get(name))
                    .and_then(|parent| parent.context.as_ref())
                    .map(|context| context.parameter.clone())
            })
            .or_else(|| {
                decls_by_name
                    .values()
                    .find(|decl| decl.is_default)
                    .and_then(|decl| decl.context_parameter.clone())
            });
        let context = if let Some(name) = context_name {
            match crate::context::struct_context(
                &name,
                context_parameter
                    .as_deref()
                    .expect("context declaration provides a parameter"),
                structs,
                &decl.name_span,
            ) {
                Ok(context) => Some(context),
                Err(errors) => {
                    diags.extend(errors);
                    None
                }
            }
        } else {
            None
        };
        let mut channel_index: HashMap<String, usize> = channels
            .iter()
            .enumerate()
            .map(|(idx, channel)| (channel.name.clone(), idx))
            .collect();
        let mut bound_channels = channels.clone();
        for channel in &decl.channels {
            if let Some(index) = bound_channels
                .iter()
                .position(|existing| existing.name == channel.name)
            {
                bound_channels[index].ty_name.clone_from(&channel.ty_name);
            } else {
                bound_channels.push(ValidatedMaterialChannel {
                    name: channel.name.clone(),
                    ty_name: channel.ty_name.clone(),
                    default_payload: None,
                    compose: channel.compose.clone(),
                });
            }
        }

        let (mut checker, prep_diags) = create_checker_for_surface(
            format!("material_properties_checker_{}", decl.name),
            functions,
            consts,
            enums,
            structs,
            params,
            texture_types,
            interfaces,
            conformances,
            effects,
            options,
        );
        diags.extend(prep_diags);
        bind_declared_channel_symbols(&mut checker, &bound_channels);
        if let Some(context) = &context {
            checker.bind_context_input(&context.parameter, context.clone(), &decl.span);
        }

        let mut seen_local_channels = HashSet::new();
        for channel in &decl.channels {
            if !seen_local_channels.insert(channel.name.clone()) {
                diags.push(
                    Diag::error(
                        channel.name_span.clone(),
                        format!(
                            "duplicate channel `{}` in material_properties `{}`",
                            channel.name, decl.name
                        ),
                    )
                    .with_help(
                        "channel names must be unique within a material_properties declaration",
                    ),
                );
                continue;
            }

            let normalized_channel_ty = super::strip_spatial_type_suffix(&channel.ty_name);
            if !material_value_type_supported(
                normalized_channel_ty,
                &checker.struct_defs,
                &mut HashSet::new(),
            ) {
                diags.push(
                    Diag::error(
                        channel.ty_span.clone(),
                        format!(
                            "unsupported material channel type `{}` for `{}.{}`",
                            channel.ty_name, decl.name, channel.name
                        ),
                    )
                    .with_help(
                        "use scalar, vector, color, matrix, fixed-size array, or nonrecursive record types (spatial qualifiers are allowed)",
                    ),
                );
                continue;
            }

            let default_payload = if let Some(default_expr) = &channel.default {
                let Some(default_value) = checker.eval_expected(default_expr, &channel.ty_name)
                else {
                    continue;
                };
                let type_param_names: &[&str] = &[];
                if !super::local_decl_type_matches(
                    &default_value,
                    &channel.ty_name,
                    &checker.enum_defs,
                    &checker.struct_defs,
                    type_param_names,
                ) {
                    diags.push(
                        Diag::error(
                            default_expr.span.clone(),
                            format!(
                                "default for material_properties channel `{}.{}` does not match declared type `{}`",
                                decl.name, channel.name, channel.ty_name
                            ),
                        )
                        .with_help("adjust the default expression to match the channel type"),
                    );
                }

                Some(default_value)
            } else {
                None
            };

            if let Some(index) = channel_index.get(&channel.name).copied() {
                let existing = &mut channels[index];
                if super::strip_spatial_type_suffix(&existing.ty_name)
                    != super::strip_spatial_type_suffix(&channel.ty_name)
                {
                    diags.push(
                        Diag::error(
                            channel.ty_span.clone(),
                            format!(
                                "material_properties `{}` overrides inherited channel `{}` with type `{}`, but the inherited channel uses `{}`",
                                decl.name, channel.name, channel.ty_name, existing.ty_name
                            ),
                        )
                        .with_help("match the inherited channel type or rename the channel"),
                    );
                    continue;
                }

                let inherited_qualifier = spatial_qualifier(&existing.ty_name);
                let override_qualifier = spatial_qualifier(&channel.ty_name);
                if let (Some(inherited), Some(override_q)) =
                    (&inherited_qualifier, &override_qualifier)
                    && inherited != override_q
                {
                    diags.push(
                        Diag::error(
                            channel.ty_span.clone(),
                            format!(
                                "material_properties `{}` overrides inherited channel `{}` with type `{}`, but the inherited channel uses `{}` and no implicit space transform is available",
                                decl.name, channel.name, channel.ty_name, existing.ty_name
                            ),
                        )
                        .with_help(
                            "use matching space qualifiers, keep one side unqualified, or apply an explicit transform before assigning across spaces",
                        ),
                    );
                    continue;
                }

                if let Some(default_payload) = default_payload {
                    existing.default_payload = Some(default_payload);
                }
                if channel.compose.is_some() {
                    existing.compose.clone_from(&channel.compose);
                }
            } else {
                channels.push(ValidatedMaterialChannel {
                    name: channel.name.clone(),
                    ty_name: channel.ty_name.clone(),
                    default_payload,
                    compose: channel.compose.clone(),
                });
                channel_index.insert(channel.name.clone(), channels.len() - 1);
            }
        }

        diags.extend(checker.diags);
        stack.pop();

        let had_error = diags[diag_start..]
            .iter()
            .any(|diag| diag.severity == Severity::Error);
        if had_error {
            states.insert(name.to_string(), MaterialPropertiesResolutionState::Failed);
            return None;
        }

        let resolved_decl = ResolvedMaterialProperties { channels, context };
        resolved.insert(name.to_string(), resolved_decl.clone());
        states.insert(
            name.to_string(),
            MaterialPropertiesResolutionState::Resolved,
        );
        Some(resolved_decl)
    }

    for decl in material_properties {
        let _ = resolve_material_properties_decl(
            &decl.name,
            &decls_by_name,
            &mut resolved,
            &mut states,
            &mut stack,
            functions,
            consts,
            enums,
            structs,
            params,
            texture_types,
            interfaces,
            conformances,
            effects,
            options,
            &mut diags,
        );
    }

    if diags.iter().any(|diag| diag.severity == Severity::Error) {
        return Err(diags);
    }

    Ok(resolved)
}

pub(crate) fn validate_material_models(
    resolved_properties: &HashMap<String, ResolvedMaterialProperties>,
    material_properties: &[MaterialPropertiesDecl],
    schema_evaluators: &[SchemaEvaluatorDecl],
    schema_programs: &[SchemaProgramDecl],
    functions: &[FnDecl],
    structs: &[crate::ast::StructDecl],
) -> Result<HashMap<String, ValidatedMaterialModel>, Vec<Diag>> {
    let mut validated_models: HashMap<String, ValidatedMaterialModel> = resolved_properties
        .iter()
        .map(|(name, properties)| {
            (
                name.clone(),
                ValidatedMaterialModel {
                    parent: material_properties
                        .iter()
                        .find(|decl| decl.name == *name)
                        .and_then(|decl| decl.extends_name.clone()),
                    context: properties.context.clone(),
                    composition: {
                        let mut decl = material_properties.iter().find(|decl| decl.name == *name);
                        let mut composition = None;
                        while let Some(current) = decl {
                            if current.composition.is_some() {
                                composition.clone_from(&current.composition);
                                break;
                            }
                            decl = current.extends_name.as_ref().and_then(|parent| {
                                material_properties
                                    .iter()
                                    .find(|candidate| candidate.name == *parent)
                            });
                        }
                        composition
                    },
                    is_default: material_properties
                        .iter()
                        .any(|decl| decl.name == *name && decl.is_default),
                    evaluation: material_properties
                        .iter()
                        .find(|decl| decl.name == *name)
                        .and_then(|decl| decl.evaluator.clone()),
                    channels: properties.channels.clone(),
                    schema_program_name: None,
                    program: None,
                    schema_program_names: Vec::new(),
                    schema_evaluator_name: None,
                    schema_evaluator_names: Vec::new(),
                    evaluation_contract: None,
                    evaluation_permutations: Vec::new(),
                    evaluation_specialize_pattern: None,
                    evaluation_specialize_pattern_span: None,
                    evaluation_function: None,
                    evaluation_shader_source: None,
                    evaluation_variants: Vec::new(),
                    evaluation_variant_bodies: Vec::new(),
                },
            )
        })
        .collect();
    let mut diags = Vec::new();
    let parent_by_material: HashMap<String, String> = material_properties
        .iter()
        .filter_map(|decl| {
            decl.extends_name
                .as_ref()
                .map(|parent| (decl.name.clone(), parent.clone()))
        })
        .collect();

    let mut seen_program_names: HashMap<String, Span> = HashMap::new();
    let mut program_decl_by_material: HashMap<String, &SchemaProgramDecl> = HashMap::new();
    for program in schema_programs {
        let [output] = program.output.as_slice() else {
            diags.push(Diag::error(
                program.name_span.clone(),
                if program.output.is_empty() {
                    "schema program requires an explicit output expression"
                } else {
                    "duplicate schema program output"
                },
            ));
            continue;
        };
        if let Some(first_span) = seen_program_names.get(&program.name) {
            diags.push(
                Diag::error(
                    program.name_span.clone(),
                    format!("duplicate schema_program declaration `{}`", program.name),
                )
                .with_help(format!(
                    "first declaration appears at bytes {}..{}",
                    first_span.start, first_span.end
                )),
            );
            continue;
        }
        seen_program_names.insert(program.name.clone(), program.name_span.clone());

        if let Some(existing) = program_decl_by_material.get(&program.material_model_name) {
            diags.push(
                Diag::error(
                    program.material_model_span.clone(),
                    format!(
                        "material_properties `{}` has multiple schema_program declarations (`{}` and `{}`)",
                        program.material_model_name, existing.name, program.name
                    ),
                )
                .with_help("declare at most one schema_program per material_properties type"),
            );
            continue;
        }

        let Some(model) = validated_models.get_mut(&program.material_model_name) else {
            diags.push(
                Diag::error(
                    program.material_model_span.clone(),
                    format!(
                        "schema_program `{}` references unknown material_properties `{}`",
                        program.name, program.material_model_name
                    ),
                )
                .with_help("declare the material_properties before binding a schema_program to it"),
            );
            continue;
        };

        model.schema_program_names.push(program.name.clone());
        model.schema_program_name = Some(program.name.clone());
        program_decl_by_material.insert(program.material_model_name.clone(), program);

        let mut typed = crate::driver::schema_function::SchemaFunction {
            functions: program.functions.clone(),
            imports: functions.to_vec(),
            structs: structs.to_vec(),
            output: output.clone(),
            parameters: Vec::new(),
            bindings: Vec::new(),
        };
        if let Expr::Var(name) = &output.node {
            let candidates = program
                .functions
                .iter()
                .filter(|f| f.name == *name)
                .collect::<Vec<_>>();
            if let [function] = candidates.as_slice() {
                typed.parameters = function
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty_name.clone()))
                    .collect();
                typed.output = crate::ast::SExpr {
                    node: Expr::Call {
                        name: name.clone(),
                        name_span: output.span.clone(),
                        const_args: Vec::new(),
                        args: function
                            .params
                            .iter()
                            .map(|p| Arg {
                                name: None,
                                value: crate::ast::SExpr {
                                    node: Expr::Var(p.name.clone()),
                                    span: p.name_span.clone(),
                                },
                            })
                            .collect(),
                    },
                    span: output.span.clone(),
                };
                model.evaluation_contract = Some(SchemaEvaluatorContractDecl {
                    inputs: function
                        .params
                        .iter()
                        .map(|p| crate::ast::EvaluationContractBindingDecl {
                            name: p.name.clone(),
                            name_span: p.name_span.clone(),
                            ty_name: Some(p.ty_name.clone()),
                            ty_span: Some(p.ty_span.clone()),
                            span: p.name_span.clone(),
                        })
                        .collect(),
                    runtime: Vec::new(),
                    span: output.span.clone(),
                });
            }
        }
        if let Some(context) = &model.context {
            let channels = model
                .channels
                .iter()
                .map(|c| MaterialChannel {
                    name: c.name.clone(),
                    ty_name: c.ty_name.clone(),
                })
                .collect::<Vec<_>>();
            if let Err(message) = typed.instantiate(
                &channels,
                context,
                "fresco_schema_check",
                "FrescoSchemaData",
            ) {
                diags.push(Diag::error(program.name_span.clone(), message));
            }
        }
        model.program = Some(typed);
    }

    struct InheritedProgram {
        contract: Option<SchemaEvaluatorContractDecl>,
        material_name: String,
        program_name: String,
        program: Option<crate::driver::schema_function::SchemaFunction>,
    }
    let mut inherited_program = Vec::new();
    for material_name in validated_models.keys() {
        let Some(model) = validated_models.get(material_name) else {
            continue;
        };
        if model.schema_program_name.is_some() {
            continue;
        }

        let mut cursor = parent_by_material.get(material_name).cloned();
        let mut visited = HashSet::new();
        while let Some(parent_name) = cursor {
            if !visited.insert(parent_name.clone()) {
                break;
            }
            if let Some(parent_model) = validated_models.get(&parent_name)
                && let Some(parent_program_name) = &parent_model.schema_program_name
            {
                inherited_program.push(InheritedProgram {
                    contract: parent_model.evaluation_contract.clone(),
                    material_name: material_name.clone(),
                    program_name: parent_program_name.clone(),
                    program: parent_model.program.clone(),
                });

                let parent_channels: HashSet<&str> = parent_model
                    .channels
                    .iter()
                    .map(|channel| channel.name.as_str())
                    .collect();
                let child_extra_channels = model
                    .channels
                    .iter()
                    .filter(|channel| !parent_channels.contains(channel.name.as_str()))
                    .map(|channel| channel.name.clone())
                    .collect::<Vec<_>>();
                if !child_extra_channels.is_empty() {
                    let span = material_properties
                        .iter()
                        .find(|decl| decl.name == *material_name)
                        .map(|decl| decl.name_span.clone())
                        .unwrap_or(0..0);
                    diags.push(
                        Diag::warning(
                            span,
                            format!(
                                "material_properties `{}` inherits schema_program `{}` from `{}`; child-only channels are currently unused: {}",
                                material_name,
                                parent_program_name,
                                parent_name,
                                child_extra_channels.join(", ")
                            ),
                        )
                        .with_help(
                            "declare a dedicated schema_program for this material_properties to consume child-specific channels",
                        ),
                    );
                }

                break;
            }
            cursor = parent_by_material.get(&parent_name).cloned();
        }
    }

    for InheritedProgram {
        contract,
        material_name,
        program_name,
        program,
    } in inherited_program
    {
        let Some(model) = validated_models.get_mut(&material_name) else {
            continue;
        };
        model.schema_program_name = Some(program_name);
        model.program = program;
        model.evaluation_contract = contract;
    }

    let mut seen_evaluation_names: HashMap<String, Span> = HashMap::new();
    for evaluation in schema_evaluators {
        if let Some(first_span) = seen_evaluation_names.get(&evaluation.name) {
            diags.push(
                Diag::error(
                    evaluation.name_span.clone(),
                    format!(
                        "duplicate schema_evaluator declaration `{}`",
                        evaluation.name
                    ),
                )
                .with_help(format!(
                    "first declaration appears at bytes {}..{}",
                    first_span.start, first_span.end
                )),
            );
            continue;
        }
        seen_evaluation_names.insert(evaluation.name.clone(), evaluation.name_span.clone());

        let Some(model) = validated_models.get_mut(&evaluation.material_model_name) else {
            diags.push(
                Diag::error(
                    evaluation.material_model_span.clone(),
                    format!(
                        "schema_evaluator `{}` references unknown material_properties `{}`",
                        evaluation.name, evaluation.material_model_name
                    ),
                )
                .with_help(
                    "declare the material_properties before binding a schema_evaluator to it",
                ),
            );
            continue;
        };

        model.schema_evaluator_names.push(evaluation.name.clone());
        let declared_selection = material_properties
            .iter()
            .find(|decl| decl.name == evaluation.material_model_name)
            .and_then(|decl| decl.evaluator.as_deref());
        let should_select_default = match declared_selection {
            Some(selected) => selected == evaluation.name,
            None => {
                if model.schema_evaluator_name.is_some() {
                    diags.push(Diag::error(evaluation.name_span.clone(), "ambiguous evaluation models; select one with @evaluator(name) on the material schema"));
                }
                model.schema_evaluator_name.is_none()
            }
        };
        if !should_select_default {
            continue;
        }

        let function =
            make_evaluation_function(evaluation, &evaluation.shade, functions, structs, &[]);

        let (evaluation_variant_bodies, variant_diags) =
            build_evaluation_variant_bodies(EvaluationVariantInputs {
                evaluation,
                functions,
                structs,
            });
        diags.extend(variant_diags);

        if evaluation.contract.is_none() && !evaluation.permutations.is_empty() {
            diags.push(
                Diag::error(
                    evaluation.name_span.clone(),
                    format!(
                        "schema_evaluator `{}` declares permutations but no contract block",
                        evaluation.name
                    ),
                )
                .with_help(
                    "add a `contract { ... }` block so the specialization strategy is explicit",
                ),
            );
        }

        if !evaluation.permutations.is_empty() && evaluation.specialize_pattern.is_none() {
            diags.push(
                Diag::error(
                    evaluation.name_span.clone(),
                    format!(
                        "schema_evaluator `{}` declares permutations but no `specialize as` pattern",
                        evaluation.name
                    ),
                )
                .with_help("add `specialize as \"...\"` so generated variant entry names are deterministic"),
            );
        }

        model.schema_evaluator_name = Some(evaluation.name.clone());
        model.evaluation_contract.clone_from(&evaluation.contract);
        model
            .evaluation_permutations
            .clone_from(&evaluation.permutations);
        model
            .evaluation_specialize_pattern
            .clone_from(&evaluation.specialize_pattern);
        model
            .evaluation_specialize_pattern_span
            .clone_from(&evaluation.specialize_pattern_span);
        model.evaluation_function = Some(function);
        model
            .evaluation_shader_source
            .clone_from(&evaluation.shader_source);
        model.evaluation_variants.clone_from(&evaluation.variants);
        model.evaluation_variant_bodies = evaluation_variant_bodies;
    }

    if diags.iter().any(|diag| diag.severity == Severity::Error) {
        return Err(diags);
    }

    Ok(validated_models)
}

struct EvaluationVariantInputs<'a> {
    evaluation: &'a SchemaEvaluatorDecl,
    functions: &'a [FnDecl],
    structs: &'a [crate::ast::StructDecl],
}

fn expand_type_bindings(ty: &str, bindings: &[(String, String)]) -> String {
    let expanded = expand_binding_pattern(ty, bindings);
    let mut output = String::new();
    let mut token = String::new();
    for c in expanded.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || c == '_' {
            token.push(c);
        } else {
            output.push_str(
                bindings
                    .iter()
                    .find(|(name, _)| name == &token)
                    .map_or(token.as_str(), |(_, value)| value.as_str()),
            );
            token.clear();
            output.push(c);
        }
    }
    output.trim().to_string()
}

fn make_evaluation_function(
    evaluation: &SchemaEvaluatorDecl,
    output: &crate::ast::SExpr,
    functions: &[FnDecl],
    structs: &[crate::ast::StructDecl],
    bindings: &[(String, String)],
) -> crate::driver::schema_function::SchemaFunction {
    crate::driver::schema_function::SchemaFunction {
        functions: Vec::new(),
        imports: functions.to_vec(),
        structs: structs.to_vec(),
        output: output.clone(),
        parameters: evaluation
            .contract
            .iter()
            .flat_map(|c| c.inputs.iter().chain(&c.runtime))
            .map(|p| {
                (
                    expand_binding_pattern(&p.name, bindings),
                    expand_type_bindings(p.ty_name.as_deref().unwrap_or("f32"), bindings),
                )
            })
            .collect(),
        bindings: bindings.to_vec(),
    }
}

fn build_evaluation_variant_bodies(
    input: EvaluationVariantInputs<'_>,
) -> (Vec<EvaluationVariant>, Vec<Diag>) {
    let EvaluationVariantInputs {
        evaluation,
        functions,
        structs,
    } = input;
    if evaluation.permutations.is_empty() && evaluation.variants.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if !evaluation.permutations.is_empty()
        && evaluation.variants.iter().any(|v| v.predicate.is_none())
    {
        return (
            Vec::new(),
            vec![Diag::error(
                evaluation.name_span.clone(),
                "specialized variant bodies require an explicit `when axis == value` predicate",
            )],
        );
    }
    let mut combinations = vec![Vec::new()];
    for axis in &evaluation.permutations {
        let values = permutation_values(&axis.spec);
        combinations = combinations
            .into_iter()
            .flat_map(|bindings| {
                values.iter().map(move |value| {
                    let mut result = bindings.clone();
                    result.push((axis.name.clone(), value.clone()));
                    result
                })
            })
            .collect();
    }
    let mut out = Vec::new();
    let mut diags = Vec::new();
    for bindings in combinations {
        let entry = apply_variant_pattern(
            evaluation
                .specialize_pattern
                .as_deref()
                .unwrap_or("fresco_evaluation_variant"),
            &bindings,
        );
        let mut selected = Vec::new();
        for variant in &evaluation.variants {
            match variant.predicate.as_ref().map_or(Ok(true), |p| {
                evaluate_variant_predicate(p, &bindings, &evaluation.permutations)
            }) {
                Ok(true) => selected.push(variant),
                Ok(false) => {}
                Err(message) => diags.push(Diag::error(variant.name_span.clone(), message)),
            }
        }
        if evaluation.variants.iter().any(|v| v.predicate.is_some()) && selected.len() != 1 {
            diags.push(Diag::error(evaluation.name_span.clone(), format!("evaluation predicates must select exactly one body for {bindings:?}; selected {}",selected.len())));
            continue;
        }
        if evaluation.variants.is_empty() {
            out.push(EvaluationVariant {
                result_type: None,
                entry: crate::material_hir::sanitize_wgsl_function_name(&entry),
                bindings: bindings
                    .iter()
                    .map(
                        |(axis, value)| crate::material_hir::EvaluationVariantBinding {
                            axis: axis.clone(),
                            value: value.clone(),
                        },
                    )
                    .collect(),
                function: Some(make_evaluation_function(
                    evaluation,
                    &evaluation.shade,
                    functions,
                    structs,
                    &bindings,
                )),
            });
        } else {
            let multiple = selected.len() > 1;
            for variant in selected {
                let entry = if multiple {
                    format!("{entry}_{}", variant.name)
                } else {
                    entry.clone()
                };
                out.push(EvaluationVariant {
                    result_type: None,
                    entry: crate::material_hir::sanitize_wgsl_function_name(&entry),
                    bindings: bindings
                        .iter()
                        .map(
                            |(axis, value)| crate::material_hir::EvaluationVariantBinding {
                                axis: axis.clone(),
                                value: value.clone(),
                            },
                        )
                        .collect(),
                    function: Some(make_evaluation_function(
                        evaluation,
                        &variant.shade,
                        functions,
                        structs,
                        &bindings,
                    )),
                });
            }
        }
    }
    out.sort_by(|a, b| a.entry.cmp(&b.entry));
    for pair in out.windows(2) {
        if pair[0].entry == pair[1].entry {
            diags.push(Diag::error(
                evaluation.name_span.clone(),
                format!(
                    "duplicate specialized entry `{}`; include all axes in the specialization name",
                    pair[0].entry
                ),
            ));
        }
    }
    (out, diags)
}

fn evaluate_variant_predicate(
    predicate: &crate::ast::SExpr,
    bindings: &[(String, String)],
    axes: &[SchemaEvaluatorPermutationDecl],
) -> Result<bool, String> {
    let Expr::Binary(op, left, right) = &predicate.node else {
        return Err("variant predicate requires an explicit axis comparison".into());
    };
    let Expr::Var(axis) = &left.node else {
        return Err("variant predicate must compare a declared axis".into());
    };
    let actual = bindings
        .iter()
        .find(|(name, _)| name == axis)
        .map(|(_, value)| value)
        .ok_or_else(|| format!("unknown evaluation axis `{axis}`"))?;
    let expected = match &right.node {
        Expr::Var(value) => value.clone(),
        Expr::Num(value, _) if value.fract() == 0.0 && *value >= 0.0 => format!("{value:.0}"),
        _ => return Err("variant predicate requires an axis value".into()),
    };
    let domain = axes
        .iter()
        .find(|a| a.name == *axis)
        .ok_or_else(|| format!("unknown evaluation axis `{axis}`"))?;
    if !permutation_values(&domain.spec).contains(&expected) {
        return Err(format!(
            "`{expected}` is not a value of evaluation axis `{axis}`"
        ));
    }
    match op {
        crate::ast::BinOp::Eq => Ok(actual == &expected),
        crate::ast::BinOp::Ne => Ok(actual != &expected),
        _ => Err("variant predicate supports == and !=".into()),
    }
}

fn apply_variant_pattern(pattern: &str, bindings: &[(String, String)]) -> String {
    expand_binding_pattern(pattern, bindings)
}

fn expand_binding_pattern(pattern: &str, bindings: &[(String, String)]) -> String {
    let mut output = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            let _ = chars.next();
            let mut expr = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                expr.push(next);
            }
            let (axis_name, format_hint) = expr
                .split_once(':')
                .map(|(name, fmt)| (name, Some(fmt)))
                .unwrap_or((expr.as_str(), None));
            if let Some((_, value)) = bindings.iter().find(|(axis, _)| axis == axis_name) {
                output.push_str(&format_variant_value(value, format_hint));
            } else {
                output.push_str(&expr);
            }
        } else if ch == '{' {
            let mut expr = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                expr.push(next);
            }
            let (axis_name, format_hint) = expr
                .split_once(':')
                .map(|(name, fmt)| (name, Some(fmt)))
                .unwrap_or((expr.as_str(), None));
            if let Some((_, value)) = bindings.iter().find(|(axis, _)| axis == axis_name) {
                output.push_str(&format_variant_value(value, format_hint));
            } else {
                output.push_str(&expr);
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn format_variant_value(value: &str, format_hint: Option<&str>) -> String {
    match format_hint {
        Some("x") => value
            .parse::<u32>()
            .map(|n| format!("{n:x}"))
            .unwrap_or_else(|_| value.to_string()),
        _ => value.to_string(),
    }
}

fn permutation_values(spec: &crate::ast::EvaluationPermutationSpec) -> Vec<String> {
    match spec {
        crate::ast::EvaluationPermutationSpec::Single(atom) => vec![atom_value(atom)],
        crate::ast::EvaluationPermutationSpec::Range { start, end } => {
            (*start..=*end).map(|value| value.to_string()).collect()
        }
        crate::ast::EvaluationPermutationSpec::Set(values) => {
            values.iter().map(atom_value).collect()
        }
    }
}

fn atom_value(atom: &crate::ast::EvaluationPermutationAtom) -> String {
    match atom {
        crate::ast::EvaluationPermutationAtom::Ident(name) => name.clone(),
        crate::ast::EvaluationPermutationAtom::Number(value) => value.to_string(),
    }
}

pub(crate) fn validate_surface_model_bindings(
    resolved_properties: &HashMap<String, ResolvedMaterialProperties>,
    schema_expressions: &[SchemaExpressionDecl],
    functions: &[FnDecl],
    structs: &[crate::ast::StructDecl],
) -> Result<HashMap<String, ValidatedSchemaExpression>, Vec<Diag>> {
    let mut diags = Vec::new();

    let mut surface_shader_by_name: HashMap<String, ValidatedSchemaExpression> = HashMap::new();
    let mut seen_shader_names: HashMap<String, Span> = HashMap::new();
    for shader in schema_expressions {
        if let Some(first_span) = seen_shader_names.get(&shader.name) {
            diags.push(
                Diag::error(
                    shader.name_span.clone(),
                    format!("duplicate schema_expression declaration `{}`", shader.name),
                )
                .with_help(format!(
                    "first declaration appears at bytes {}..{}",
                    first_span.start, first_span.end
                )),
            );
            continue;
        }
        seen_shader_names.insert(shader.name.clone(), shader.name_span.clone());

        let Some(properties) = resolved_properties.get(&shader.properties_name) else {
            diags.push(
                Diag::error(
                    shader.properties_span.clone(),
                    format!(
                        "schema_expression `{}` references unknown material_properties `{}`",
                        shader.name, shader.properties_name
                    ),
                )
                .with_help(
                    "declare the referenced material_properties before the schema_expression",
                ),
            );
            continue;
        };

        let function = crate::driver::schema_function::SchemaFunction {
            functions: Vec::new(),
            imports: functions.to_vec(),
            structs: structs.to_vec(),
            output: shader.shade.clone(),
            parameters: Vec::new(),
            bindings: Vec::new(),
        };
        if let Some(context) = &properties.context {
            let channels = properties
                .channels
                .iter()
                .map(|c| MaterialChannel {
                    name: c.name.clone(),
                    ty_name: c.ty_name.clone(),
                })
                .collect::<Vec<_>>();
            if let Err(message) = function.instantiate(
                &channels,
                context,
                "fresco_expression_check",
                "FrescoSchemaData",
            ) {
                diags.push(Diag::error(shader.name_span.clone(), message));
            }
        }

        surface_shader_by_name.insert(
            shader.name.clone(),
            ValidatedSchemaExpression {
                name: shader.name.clone(),
                properties_name: shader.properties_name.clone(),
                function,
            },
        );
    }

    if diags.iter().any(|diag| diag.severity == Severity::Error) {
        return Err(diags);
    }

    Ok(surface_shader_by_name)
}

#[expect(
    clippy::too_many_arguments,
    reason = "This compiler boundary explicitly threads independent checking or lowering inputs."
)]
pub(crate) fn check_surface(
    surface: &SurfaceDecl,
    _passes: &[PassDecl],
    _pipelines: &[PipelineDecl],
    validated_material_models: &HashMap<String, ValidatedMaterialModel>,
    validated_surface_models: &HashMap<String, ValidatedSchemaExpression>,
    functions: &[FnDecl],
    consts: &[crate::ast::ConstDecl],
    enums: &[crate::ast::EnumDecl],
    structs: &[crate::ast::StructDecl],
    params: &[crate::ast::GlobalParamDecl],
    texture_types: &[crate::ast::TextureTypeDecl],
    interfaces: &[InterfaceDecl],
    conformances: &[crate::ast::ConformanceDecl],
    effects: &[crate::ast::EffectDecl],
    options: &CheckOptions,
) -> Result<(MaterialHir, Vec<Diag>), Vec<Diag>> {
    let (mut checker, prep_diags) = create_checker_for_surface(
        surface.name.clone(),
        functions,
        consts,
        enums,
        structs,
        params,
        texture_types,
        interfaces,
        conformances,
        effects,
        options,
    );
    checker.diags.extend(prep_diags);

    let selected_surface_model = match &surface.material_ty {
        MaterialReturnTy::Named(name) => validated_surface_models.get(name),
        MaterialReturnTy::Default => None,
    };

    if let MaterialReturnTy::Named(model_name) = &surface.material_ty
        && !validated_material_models.contains_key(model_name)
        && selected_surface_model.is_none()
    {
        checker.diags.push(
            Diag::error(
                surface.name_span.clone(),
                format!(
                    "surface `{}` references unknown material_properties `{}`",
                    surface.name, model_name
                ),
            )
            .with_help(
                "declare `material_properties <name> { ... }` before using `material(<name>)`",
            ),
        );
    }

    let selected_model_name = match &surface.material_ty {
        MaterialReturnTy::Default => {
            let defaults: Vec<_> = validated_material_models
                .iter()
                .filter(|(_, model)| model.is_default)
                .map(|(name, _)| name.clone())
                .collect();
            match defaults.as_slice() {
                [name] => Some(name.clone()),
                _ => {
                    checker.diags.push(Diag::error(surface.name_span.clone(), "surface requires exactly one engine @default_material declaration or an explicit material profile"));
                    None
                }
            }
        }
        MaterialReturnTy::Named(name) => Some(
            selected_surface_model
                .map_or_else(|| name.clone(), |model| model.properties_name.clone()),
        ),
    };
    let selected_model = selected_model_name
        .as_ref()
        .and_then(|name| validated_material_models.get(name));
    if let Some(selection) = selected_model.and_then(|model| model.evaluation.as_deref())
        && !selected_model.is_some_and(|model| {
            model
                .schema_evaluator_names
                .iter()
                .any(|name| name == selection)
        })
    {
        checker.diags.push(Diag::error(
            surface.name_span.clone(),
            format!("material selects missing evaluator `{selection}`"),
        ));
    }
    let declared_channel_types: HashMap<String, String> = selected_model
        .map(|model| {
            model
                .channels
                .iter()
                .map(|channel| (channel.name.clone(), channel.ty_name.clone()))
                .collect()
        })
        .unwrap_or_default();

    let Some(context) = selected_model.and_then(|model| model.context.clone()) else {
        checker.diags.push(Diag::error(
            surface.name_span.clone(),
            "surface requires an engine @context(Type, parameter) material declaration",
        ));
        return Err(checker.diags);
    };
    checker.bind_entry_context(&surface.as_normalized_root_entry(), context.clone());
    checker.validate_registered_function_overloads();

    let mut compose_entries: Option<Vec<&ComposeEntry>> = None;
    let mut vertex_program: Option<MaterialVertexProgram> = None;
    for stmt in &surface.body {
        match stmt {
            Stmt::Param {
                name,
                name_span,
                ty_name,
                default,
                range,
                ..
            } => checker.declare_param(name, name_span, ty_name, default, range.as_ref()),
            Stmt::Let { name, value, .. } => {
                if let Some(v) = checker.eval(value) {
                    checker.bind(name.clone(), v);
                }
            }
            Stmt::Const {
                name,
                value,
                ty_name,
                ty_span,
                ..
            } => {
                if let Some(v) = checker.eval(value) {
                    if !super::local_decl_type_matches(
                        &v,
                        ty_name,
                        &checker.enum_defs,
                        &checker.struct_defs,
                        &[],
                    ) {
                        checker.diags.push(
                            Diag::error(
                                ty_span.clone(),
                                format!(
                                    "const `{name}` expected {}, but got {}",
                                    super::local_decl_expected_kind(
                                        ty_name,
                                        &checker.enum_defs,
                                        &checker.struct_defs,
                                        &[],
                                    ),
                                    v.kind()
                                ),
                            )
                            .with_label("declared type does not match initializer")
                            .with_help("adjust the declared type or const initializer expression"),
                        );
                        continue;
                    }

                    let what = format!("const `{name}` initializer");
                    if let Some(folded) =
                        checker.eval_compile_time_const_value(&v, &value.span, &what)
                    {
                        checker.bind(name.clone(), folded);
                    }
                }
            }
            Stmt::Compose { entries, .. } => {
                if compose_entries.is_some() {
                    checker.diags.push(
                        Diag::error(
                            stmt_span(stmt),
                            "surface body may only have one `compose` block",
                        )
                        .with_help("keep one `compose { ... }` block in the surface body"),
                    );
                } else {
                    compose_entries = Some(entries.iter().collect());
                }
            }
            Stmt::ComposePiped { entries, pipes, .. } => {
                if !pipes.is_empty() {
                    checker.diags.push(
                        Diag::error(stmt_span(stmt), "surface compose pipes are not yet supported")
                            .with_help(
                                "surface compose blocks currently support only the engine-declared initializer and layer operations",
                            ),
                    );
                }
                if compose_entries.is_some() {
                    checker.diags.push(
                        Diag::error(
                            stmt_span(stmt),
                            "surface body may only have one `compose` block",
                        )
                        .with_help("keep one `compose { ... }` block in the surface body"),
                    );
                } else {
                    compose_entries = Some(entries.iter().collect());
                }
            }
            Stmt::SurfaceVertex { fields, span } => {
                if vertex_program.is_some() {
                    checker.diags.push(
                        Diag::error(
                            span.clone(),
                            "surface body may only have one `vertex { ... }` block",
                        )
                        .with_help("keep one `vertex { ... }` block in the surface body"),
                    );
                    continue;
                }

                vertex_program = extract_vertex_program(&mut checker, fields, &context, span);
            }
            _ => checker.diags.push(
                Diag::error(
                    stmt_span(stmt),
                    "statement not yet supported in surface body",
                )
                .with_help(
                    "surface bodies currently support `param`, `let`, `const`, `vertex { ... }`, and `compose { ... }`",
                ),
            ),
        }
    }

    let composition = selected_model
        .and_then(|model| model.composition.as_ref())
        .or_else(|| {
            validated_material_models
                .values()
                .find(|model| model.is_default)
                .and_then(|model| model.composition.as_ref())
        });
    let Some(composition) = composition else {
        checker.diags.push(Diag::error(
            surface.name_span.clone(),
            "surface requires an engine @composition(initializer, layer, weight) declaration",
        ));
        return Err(checker.diags);
    };
    let mut layers = Vec::new();
    if let Some(entries) = compose_entries {
        for entry in entries {
            if let Some(layer) = process_compose_entry(
                &mut checker,
                entry,
                selected_model_name.as_deref(),
                &declared_channel_types,
                composition,
            ) {
                layers.push(layer);
            }
        }
    } else {
        checker.diags.push(
            Diag::error(surface.span.clone(), "surface body produces no material").with_help(
                format!(
                    "add a `compose {{ {}(...) }}` block to define the material",
                    composition[0]
                ),
            ),
        );
    }

    if let Some(first) = layers.first()
        && !first.is_base
    {
        checker.diags.push(
            Diag::error(
                surface.name_span.clone(),
                format!("surface compose must start with `{}(...)`", composition[0]),
            )
            .with_help(format!(
                "make the first compose entry `{}(...)`",
                composition[0]
            )),
        );
    }

    let mut authored_channels_set = BTreeSet::new();
    for layer in &layers {
        for name in layer.channels.keys() {
            authored_channels_set.insert(name.clone());
        }
    }

    let mut channel_defaults: HashMap<String, Value> = HashMap::new();
    if let Some(model) = selected_model {
        let declared_set: HashSet<String> = model.channels.iter().map(|c| c.name.clone()).collect();

        for authored_name in &authored_channels_set {
            if !declared_set.contains(authored_name) {
                checker.diags.push(
                    Diag::error(
                        surface.name_span.clone(),
                        format!(
                            "surface `{}` authors channel `{}` not declared by material_properties `{}`",
                            surface.name,
                            authored_name,
                            selected_model_name
                                .clone()
                                .unwrap_or_else(|| "<unknown>".to_string())
                        ),
                    )
                    .with_help("declare the channel in material_properties or remove the authored channel argument"),
                );
            }
        }

        for channel in &model.channels {
            if let Some(default_payload) = &channel.default_payload {
                channel_defaults.insert(channel.name.clone(), default_payload.clone());
            }

            if channel.default_payload.is_none() && !authored_channels_set.contains(&channel.name) {
                checker.diags.push(
                    Diag::error(
                        surface.name_span.clone(),
                        format!(
                            "surface `{}` is missing required material channel `{}` for model `{}`",
                            surface.name,
                            channel.name,
                            selected_model_name
                                .clone()
                                .unwrap_or_else(|| "<unknown>".to_string())
                        ),
                    )
                    .with_help("provide the channel in compose entries or declare a default in material_properties"),
                );
            }
        }
    }

    // Composition policy is an authored function, never an implicit per-type blend.
    let mut current = channel_defaults.clone();
    let mut checked_composers = HashSet::new();
    if let Some(model) = selected_model {
        for layer in &mut layers {
            for channel in &model.channels {
                let Some(next) = layer.channels.get(&channel.name).cloned() else {
                    continue;
                };
                let result = if let Some(weight) = &layer.weight {
                    let Some(previous) = current.get(&channel.name) else {
                        checker.diags.push(Diag::error(
                            surface.name_span.clone(),
                            format!(
                                "weighted field `{}` requires an initialized value",
                                channel.name
                            ),
                        ));
                        continue;
                    };
                    let Some(composer) = &channel.compose else {
                        checker.diags.push(Diag::error(surface.name_span.clone(),
                            format!("weighted field `{}` requires an explicit composition function", channel.name))
                            .with_help("declare channel @compose(function); the function takes previous, next, and weight and returns the channel type"));
                        continue;
                    };
                    checker.scopes.push(HashMap::new());
                    for (name, value) in [
                        ("__surface_previous", previous.clone()),
                        ("__surface_next", next.clone()),
                        ("__surface_weight", Value::Scalar(weight.clone())),
                    ] {
                        checker.bind(name.into(), value);
                    }
                    let args =
                        ["__surface_previous", "__surface_next", "__surface_weight"].map(|name| {
                            Arg {
                                name: None,
                                value: crate::ast::Spanned {
                                    node: Expr::Var(name.into()),
                                    span: surface.name_span.clone(),
                                },
                            }
                        });
                    let call = crate::ast::Spanned {
                        node: Expr::Call {
                            name: composer.clone(),
                            const_args: Vec::new(),
                            args: args.to_vec(),
                            name_span: surface.name_span.clone(),
                        },
                        span: surface.name_span.clone(),
                    };
                    if checked_composers.insert((composer.clone(), channel.ty_name.clone())) {
                        let program = crate::driver::schema_function::SchemaFunction {
                            functions: Vec::new(),
                            imports: functions.to_vec(),
                            structs: structs.to_vec(),
                            output: call.clone(),
                            parameters: vec![
                                ("__surface_previous".into(), channel.ty_name.clone()),
                                ("__surface_next".into(), channel.ty_name.clone()),
                                ("__surface_weight".into(), "f32".into()),
                            ],
                            bindings: Vec::new(),
                        };
                        let channels = model
                            .channels
                            .iter()
                            .map(|channel| MaterialChannel {
                                name: channel.name.clone(),
                                ty_name: channel.ty_name.clone(),
                            })
                            .collect::<Vec<_>>();
                        if let Err(message) = program.instantiate_with_result(
                            &channels,
                            &context,
                            "fresco_check_composition",
                            &format!("FrescoMaterial_{}", surface.name),
                            Some(&channel.ty_name),
                        ) {
                            checker.scopes.pop();
                            checker.diags.push(Diag::error(
                                surface.name_span.clone(),
                                format!(
                                    "invalid composition function `{composer}` for `{}`: {message}",
                                    channel.name
                                ),
                            ));
                            continue;
                        }
                    }
                    let evaluated = checker.eval(&call);
                    checker.scopes.pop();
                    let Some(value) = evaluated else {
                        continue;
                    };
                    let Some(value) = checked_material_value(
                        value,
                        &channel.ty_name,
                        &surface.name_span,
                        &mut checker,
                        &channel.name,
                    ) else {
                        continue;
                    };
                    value
                } else {
                    next
                };
                current.insert(channel.name.clone(), result.clone());
                layer.channels.insert(channel.name.clone(), result);
            }
            layer.weight = None;
        }
    }

    checker.emit_path_channel_demand_note();
    checker.materialize_pending_user_helpers();
    checker.emit_expr_hotspot_summary();

    if checker.diags.iter().any(|d| d.severity == Severity::Error) {
        return Err(checker.diags);
    }

    let material_channels = selected_model
        .map(|model| {
            model
                .channels
                .iter()
                .map(|channel| MaterialChannel {
                    name: channel.name.clone(),
                    ty_name: channel.ty_name.clone(),
                })
                .collect::<Vec<_>>()
        })
        .expect("checked surface material schema");

    let schema_program_name = selected_model.and_then(|model| model.schema_program_name.clone());
    let selected_function = selected_surface_model
        .map(|m| &m.function)
        .or_else(|| selected_model.and_then(|m| m.program.as_ref()))
        .or_else(|| selected_model.and_then(|m| m.evaluation_function.as_ref()));
    let evaluation_shader_entry =
        selected_function.map(|_| format!("fresco_evaluation_shader_{}", surface.name));
    let mut evaluation_variant_bodies = selected_model
        .map(|m| m.evaluation_variant_bodies.clone())
        .unwrap_or_default();
    if evaluation_variant_bodies.is_empty() {
        if let (Some(entry), Some(function)) = (&evaluation_shader_entry, selected_function) {
            evaluation_variant_bodies.push(EvaluationVariant {
                result_type: None,
                entry: entry.clone(),
                bindings: Vec::new(),
                function: Some(function.clone()),
            });
        }
    } else if selected_surface_model.is_some()
        || selected_model.is_some_and(|m| m.program.is_some())
    {
        for variant in &mut evaluation_variant_bodies {
            let mut function = selected_function.expect("selected schema function").clone();
            if let Some(specialization) = &variant.function {
                function.parameters.clone_from(&specialization.parameters);
                function.bindings.clone_from(&specialization.bindings);
            }
            variant.function = Some(function);
        }
    }
    let mut evaluation_typed_source = String::new();
    let mut evaluation_type_definitions = std::collections::BTreeMap::new();
    let mut evaluation_type_aliases = HashMap::new();
    fn record_dependencies(
        name: &str,
        structs: &[crate::ast::StructDecl],
        used: &mut HashSet<String>,
    ) {
        let name = super::strip_spatial_type_suffix(name);
        if let Some((element, _)) = crate::hir::parse_array_param_type(name) {
            record_dependencies(element, structs, used);
        } else if let Some(record) = structs.iter().find(|record| record.name == name)
            && used.insert(name.into())
        {
            for field in &record.fields {
                record_dependencies(&field.ty_name, structs, used);
            }
        }
    }
    let mut material_records = HashSet::new();
    for channel in &material_channels {
        record_dependencies(&channel.ty_name, structs, &mut material_records);
    }
    for (index, record) in structs.iter().enumerate() {
        if material_records.contains(&record.name) {
            evaluation_type_aliases.insert(
                record.name.clone(),
                if context.contains_record(&record.name) {
                    record.name.clone()
                } else {
                    format!("FrescoMaterial_{}_type_{index}", surface.name)
                },
            );
        }
    }
    let material_record_aliases: HashSet<_> = evaluation_type_aliases.values().cloned().collect();
    for variant in &mut evaluation_variant_bodies {
        if let Some(function) = &variant.function {
            match function.instantiate(
                &material_channels,
                &context,
                &variant.entry,
                &format!("FrescoMaterial_{}", surface.name),
            ) {
                Ok(instance) => {
                    evaluation_typed_source.push_str(&instance.source);
                    variant.result_type = Some(instance.result_type);
                    evaluation_type_definitions.extend(
                        instance
                            .type_definitions
                            .into_iter()
                            .filter(|(name, _)| !material_record_aliases.contains(name)),
                    );
                    evaluation_type_aliases.extend(instance.type_aliases);
                }
                Err(message) => checker
                    .diags
                    .push(Diag::error(surface.name_span.clone(), message)),
            }
        }
    }

    let evaluation_shader_entry = evaluation_shader_entry
        .filter(|entry| evaluation_variant_bodies.iter().any(|v| &v.entry == entry));
    let evaluation_shader_source = evaluation_shader_entry.as_ref().and_then(|_| {
        selected_surface_model
            .map(|model| format!("schema_expression:{}", model.name))
            .or_else(|| {
                schema_program_name
                    .as_ref()
                    .map(|name| format!("schema_program:{name}"))
            })
            .or_else(|| selected_model.and_then(|model| model.evaluation_shader_source.clone()))
    });

    if checker.diags.iter().any(|d| d.severity == Severity::Error) {
        return Err(checker.diags);
    }

    let hir = MaterialHir {
        context,
        settings: surface.settings.clone(),
        rendering_policy: checker.hir.rendering_policy,
        name: surface.name.clone(),
        material_properties_name: selected_model_name.clone(),
        material_schemas: validated_material_models
            .iter()
            .map(|(name, model)| (name.clone(), model.parent.clone()))
            .collect(),
        surface_shader_name: selected_surface_model
            .map_or_else(|| surface.name.clone(), |model| model.name.clone()),
        render_policy_name: match &surface.material_ty {
            MaterialReturnTy::Default => "default".to_string(),
            MaterialReturnTy::Named(name) => name.clone(),
        },
        material_ty: MaterialReturnTy::Named(
            selected_model_name
                .clone()
                .expect("checked material profile"),
        ),
        params: checker.hir.params.clone(),
        user_helpers: checker.hir.user_helpers.clone(),
        textures: checker.hir.textures.clone(),
        texture_index: checker.hir.texture_index.clone(),
        texture_metadata: checker.hir.texture_metadata.clone(),
        texture_type_defs: checker.hir.texture_type_defs.clone(),
        global_uniforms: checker.hir.global_uniforms.clone(),
        layers: layers
            .into_iter()
            .map(|layer| MaterialLayer {
                is_base: layer.is_base,
                weight: layer.weight,
                channels: layer
                    .channels
                    .into_iter()
                    .map(|(k, v)| (k, material_value(v)))
                    .collect(),
            })
            .collect(),
        material_channels,
        channel_defaults: channel_defaults
            .into_iter()
            .map(|(k, v)| (k, material_value(v)))
            .collect(),
        record_types: structs.to_vec(),
        schema_evaluator_name: selected_model.and_then(|model| model.schema_evaluator_name.clone()),
        schema_program_name,
        evaluation_contract: selected_model.and_then(|model| model.evaluation_contract.clone()),
        evaluation_permutations: selected_model
            .map(|model| model.evaluation_permutations.clone())
            .unwrap_or_default(),
        evaluation_specialize_pattern: selected_model
            .and_then(|model| model.evaluation_specialize_pattern.clone()),
        evaluation_typed_source: (!evaluation_variant_bodies.is_empty()).then(|| {
            evaluation_type_definitions
                .into_values()
                .collect::<String>()
                + &evaluation_typed_source
        }),
        evaluation_type_aliases,
        evaluation_shader_entry,
        evaluation_shader_source,
        evaluation_variant_decls: selected_model
            .map(|model| model.evaluation_variants.clone())
            .unwrap_or_default(),
        evaluation_variant_bodies,
        vertex_program,
        notes: checker.hir.notes.clone(),
    };
    Ok((hir, checker.diags))
}

fn material_payload_value(ty_name: &str, [x, y, z, w]: [Sx; 4]) -> Value {
    match super::strip_spatial_type_suffix(ty_name) {
        "color" => Value::ColorField {
            rgba: [x, y, z, w],
            space: super::ColorSpace::Srgb,
        },
        "vec4" => Value::Vec4((x, y, z, w)),
        "vec3" => Value::Vec3((x, y, z)),
        "vec2" => Value::Vec2((x, y)),
        "f32" | "f64" | "half" | "i32" | "u32" | "bool" | "angle" | "length" => Value::Scalar(x),
        _ => unreachable!("validated material field type"),
    }
}

fn material_channel_value(channel: &ValidatedMaterialChannel) -> Value {
    material_payload_value(
        &channel.ty_name,
        ["x", "y", "z", "w"].map(|component| Sx::Param(format!("{}.{component}", channel.name))),
    )
}

fn bind_declared_channel_symbols(checker: &mut Checker, channels: &[ValidatedMaterialChannel]) {
    for channel in channels {
        // The declaration validation loop reports unsupported types. Do not
        // construct symbolic values for those invalid declarations first.
        if ALLOWED_MATERIAL_CHANNEL_TYPES
            .contains(&super::strip_spatial_type_suffix(&channel.ty_name))
        {
            checker.bind(channel.name.clone(), material_channel_value(channel));
        }
    }
}

fn value_to_channel_vec4_payload(
    value: Option<Value>,
    span: &Span,
    checker: &mut Checker,
    context: &str,
) -> Option<[Sx; 4]> {
    let value = value?;
    let zero = Sx::Lit(0.0);
    match value {
        Value::Scalar(sx) | Value::Distance(sx) | Value::Coverage(sx) | Value::Mask(sx) => {
            Some([sx, zero.clone(), zero.clone(), zero])
        }
        Value::Vec2((x, y)) => Some([x, y, zero.clone(), zero]),
        Value::Vec3((x, y, z)) => Some([x, y, z, zero]),
        Value::Vec4((x, y, z, w)) => Some([x, y, z, w]),
        Value::Color { rgba, .. } => Some(rgba.map(Sx::Lit)),
        Value::ColorField { rgba, .. } => Some(rgba),
        other => {
            checker.diags.push(
                Diag::error(
                    span.clone(),
                    format!(
                        "expected scalar/vec/color value for {context}, found {}",
                        other.kind()
                    ),
                )
                .with_help("use scalar, vec2/vec3/vec4, or color values"),
            );
            None
        }
    }
}

fn extract_vertex_program(
    checker: &mut Checker,
    fields: &[(crate::ast::Spanned<String>, crate::ast::SExpr)],
    context: &crate::context::EntryContext,
    span: &Span,
) -> Option<MaterialVertexProgram> {
    let mut out = MaterialVertexProgram::default();
    for (name, expr) in fields {
        let Some(field) = context
            .ty
            .fields
            .iter()
            .find(|field| field.name == name.node)
        else {
            checker.diags.push(Diag::error(
                name.span.clone(),
                format!("unknown vertex context field `{}`", name.node),
            ));
            continue;
        };
        let (ty, width) = match field.ty {
            crate::context::ContextType::Float => ("f32", 1),
            crate::context::ContextType::Scalar(kind) => (kind.name(), 1),
            crate::context::ContextType::Vector(2) => ("vec2", 2),
            crate::context::ContextType::Vector(3) => ("vec3", 3),
            crate::context::ContextType::Vector(4) => ("vec4", 4),
            _ => {
                checker.diags.push(Diag::error(
                    name.span.clone(),
                    "vertex updates currently require scalar or vector context fields",
                ));
                continue;
            }
        };
        let Some(value) = checker.eval_expected(expr, ty) else {
            continue;
        };
        if !super::local_decl_type_matches(
            &value,
            ty,
            &checker.enum_defs,
            &checker.struct_defs,
            &[],
        ) {
            checker.diags.push(Diag::error(
                expr.span.clone(),
                format!(
                    "vertex field `{}` requires `{ty}`, found {}",
                    name.node,
                    value.kind()
                ),
            ));
            continue;
        }
        if let Some(payload) =
            value_to_channel_vec4_payload(Some(value), &expr.span, checker, "vertex context field")
        {
            let components = payload[..width].to_vec();
            if validate_surface_vertex_expr(checker, &components, &expr.span, &name.node) {
                out.fields.push((name.node.clone(), components));
            }
        }
    }
    if out.fields.is_empty() {
        checker.diags.push(Diag::error(
            span.clone(),
            "vertex block produced no valid outputs",
        ));
        return None;
    }
    Some(out)
}

fn validate_surface_vertex_expr(
    checker: &mut Checker,
    vec: &[Sx],
    span: &Span,
    field_name: &str,
) -> bool {
    for component in vec {
        let mut is_valid = true;
        component.walk_preorder(&mut |sx| {
            if matches!(
                sx,
                Sx::TexChannel { .. } | Sx::Ddx(_) | Sx::Ddy(_) | Sx::Fwidth(_)
            ) {
                is_valid = false;
            }
        });
        if !is_valid {
            checker.diags.push(
                Diag::error(
                    span.clone(),
                    format!(
                        "surface vertex `{field_name}:` cannot use texture sampling or screen-space derivatives"
                    ),
                )
                .with_help(
                    "vertex expressions can use surface fields, params, and scalar math but not texture sampling or ddx/ddy/fwidth",
                ),
            );
            return false;
        }
    }
    true
}

fn process_compose_entry(
    checker: &mut Checker,
    entry: &ComposeEntry,
    selected_model_name: Option<&str>,
    declared_channel_types: &HashMap<String, String>,
    composition: &[String; 3],
) -> Option<CheckedMaterialLayer> {
    match entry {
        ComposeEntry::Expr { expr, .. } => process_compose_expr(
            checker,
            expr,
            selected_model_name,
            declared_channel_types,
            composition,
        ),
        _ => {
            checker.diags.push(
                Diag::error(
                    compose_entry_span(entry),
                    "compose entry not yet supported in surface body",
                )
                .with_help(format!(
                    "surface compose blocks support `{}(...)` and `layer {}(...)`",
                    composition[0], composition[1]
                )),
            );
            None
        }
    }
}

fn process_compose_expr(
    checker: &mut Checker,
    expr: &crate::ast::SExpr,
    selected_model_name: Option<&str>,
    declared_channel_types: &HashMap<String, String>,
    composition: &[String; 3],
) -> Option<CheckedMaterialLayer> {
    match &expr.node {
        Expr::Call { name, args, .. } if name == &composition[0] => extract_material_args(
            checker,
            args,
            true,
            selected_model_name,
            declared_channel_types,
            composition,
        ),
        Expr::Layer(inner) => match &inner.node {
            Expr::Call { name, args, .. } if name == &composition[1] => extract_material_args(
                checker,
                args,
                false,
                selected_model_name,
                declared_channel_types,
                composition,
            ),
            _ => {
                checker.diags.push(
                    Diag::error(
                        expr.span.clone(),
                        "unsupported `layer` entry in surface compose block",
                    )
                    .with_help(format!(
                        "use `layer {}(...)` for surface override layers",
                        composition[1]
                    )),
                );
                None
            }
        },
        _ => {
            checker.diags.push(
                Diag::error(expr.span.clone(), "unsupported surface compose entry").with_help(
                    format!(
                        "use `{}(...)` or `layer {}(...)` entries",
                        composition[0], composition[1]
                    ),
                ),
            );
            None
        }
    }
}

fn extract_material_args(
    checker: &mut Checker,
    args: &[Arg],
    is_base: bool,
    selected_model_name: Option<&str>,
    declared_channel_types: &HashMap<String, String>,
    composition: &[String; 3],
) -> Option<CheckedMaterialLayer> {
    let mut channels = HashMap::new();
    let mut weight = None;
    for arg in args {
        let Some(name) = arg.name.as_deref() else {
            checker.diags.push(Diag::error(
                arg.value.span.clone(),
                "material arguments must be named",
            ));
            continue;
        };
        if name == composition[2] {
            weight = Some(value_to_scalar_sx(
                checker.eval(&arg.value)?,
                &arg.value.span,
                checker,
            )?);
            continue;
        }
        let Some(ty_name) = declared_channel_types.get(name) else {
            let mut known: Vec<_> = declared_channel_types.keys().cloned().collect();
            known.sort();
            let mut diag = Diag::error(
                arg.value.span.clone(),
                format!(
                    "unknown material argument `{name}` for model `{}`",
                    selected_model_name.unwrap_or("<unknown>")
                ),
            );
            if let Some(suggestion) = nearest_material_arg_name(name, &known) {
                diag = diag.with_help(format!("did you mean `{suggestion}`?"));
            }
            checker.diags.push(diag);
            continue;
        };
        let value = checker.eval_expected(&arg.value, ty_name)?;
        let payload = checked_material_value(value, ty_name, &arg.value.span, checker, name)?;
        if channels.insert(name.to_string(), payload).is_some() {
            checker.diags.push(Diag::error(
                arg.value.span.clone(),
                format!("duplicate material argument `{name}`"),
            ));
        }
    }
    Some(CheckedMaterialLayer {
        is_base,
        weight,
        channels,
    })
}

fn nearest_material_arg_name(arg_name: &str, candidates: &[String]) -> Option<String> {
    let needle = arg_name.to_ascii_lowercase();
    let mut best: Option<(&str, usize)> = None;

    for candidate in candidates {
        let distance = levenshtein_distance(&needle, &candidate.to_ascii_lowercase());
        let should_take = match best {
            None => true,
            Some((_, best_distance)) => distance < best_distance,
        };
        if should_take {
            best = Some((candidate.as_str(), distance));
        }
    }

    let (name, distance) = best?;

    // Keep suggestions conservative to avoid noisy hints.
    let threshold = if needle.len() <= 4 { 1 } else { 3 };
    if distance <= threshold {
        Some(name.to_string())
    } else {
        None
    }
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    if a.is_empty() {
        return b.chars().count();
    }
    if b.is_empty() {
        return a.chars().count();
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr: Vec<usize> = vec![0; b_chars.len() + 1];

    for (i, a_ch) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, b_ch) in b_chars.iter().enumerate() {
            let cost = if a_ch == b_ch { 0 } else { 1 };
            let del = prev[j + 1] + 1;
            let ins = curr[j] + 1;
            let sub = prev[j] + cost;
            curr[j + 1] = del.min(ins).min(sub);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_chars.len()]
}

fn checked_material_value(
    value: Value,
    ty_name: &str,
    span: &Span,
    checker: &mut Checker,
    channel: &str,
) -> Option<Value> {
    if super::strip_spatial_type_suffix(ty_name) == "color" {
        return value_to_color_sx(value, span, checker).map(|rgba| Value::ColorField {
            rgba,
            space: super::ColorSpace::Srgb,
        });
    }
    if super::local_decl_type_matches(
        &value,
        ty_name,
        &checker.enum_defs,
        &checker.struct_defs,
        &[],
    ) {
        Some(value)
    } else {
        checker.diags.push(Diag::error(
            span.clone(),
            format!(
                "material channel `{channel}` expects {ty_name}, found {}",
                value.kind()
            ),
        ));
        None
    }
}

fn material_value_type_supported(
    ty: &str,
    records: &HashMap<String, super::StructDef>,
    active: &mut HashSet<String>,
) -> bool {
    let ty = super::strip_spatial_type_suffix(ty);
    if ALLOWED_MATERIAL_CHANNEL_TYPES.contains(&ty) || matches!(ty, "mat2" | "mat3" | "mat4") {
        return true;
    }
    if let Some((element, count)) = crate::hir::parse_array_param_type(ty) {
        return count > 0 && material_value_type_supported(element, records, active);
    }
    if let Some(record) = records.get(ty) {
        if !active.insert(ty.into()) {
            return false;
        }
        let valid = record
            .fields
            .values()
            .all(|f| material_value_type_supported(&f.ty_name, records, active));
        active.remove(ty);
        return valid;
    }
    false
}

fn value_to_color_sx(v: Value, span: &Span, checker: &mut Checker) -> Option<[Sx; 4]> {
    match v {
        Value::Gradient { kind, stops } => {
            let rgba = crate::hir::GradientSample::channels(kind, stops);
            if rgba.iter().any(Sx::requires_shape_color_frame) {
                checker.diags.push(
                    Diag::error(span.clone(), "`anchor: shape` requires a shape receiver")
                        .with_help("material gradients use surface UVs; use `anchor: scene`"),
                );
                return None;
            }
            Some(rgba)
        }

        Value::Color { rgba, .. } => Some(rgba.map(Sx::Lit)),
        Value::ColorField { rgba, .. } => {
            if rgba.iter().any(Sx::requires_shape_color_frame) {
                checker.diags.push(
                    Diag::error(span.clone(), "`anchor: shape` requires a shape receiver")
                        .with_help("material gradients use surface UVs; use `anchor: scene`"),
                );
                None
            } else {
                Some(rgba)
            }
        }
        other => {
            checker.diags.push(
                Diag::error(
                    span.clone(),
                    format!("expected color or gradient, found {}", other.kind()),
                )
                .with_help("use a color literal, color param, color expression, or gradient here"),
            );
            None
        }
    }
}

fn value_to_scalar_sx(v: Value, span: &Span, checker: &mut Checker) -> Option<Sx> {
    match v {
        Value::Scalar(sx) | Value::Distance(sx) | Value::Coverage(sx) | Value::Mask(sx) => Some(sx),
        other => {
            checker.diags.push(
                Diag::error(
                    span.clone(),
                    format!("expected scalar, found {}", other.kind()),
                )
                .with_help("use a scalar expression here"),
            );
            None
        }
    }
}

fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::TextureBinding { span, .. }
        | Stmt::Param { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Store { span, .. }
        | Stmt::For { span, .. }
        | Stmt::If { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::SpaceDecl { span, .. }
        | Stmt::StyleDecl { span, .. }
        | Stmt::CanvasSpace { span, .. }
        | Stmt::InSpace { span, .. }
        | Stmt::InContext { span, .. }
        | Stmt::Block { span, .. }
        | Stmt::Seq { span, .. }
        | Stmt::Compose { span, .. }
        | Stmt::ComposePiped { span, .. }
        | Stmt::SurfaceVertex { span, .. }
        | Stmt::ReturnVoid { span }
        | Stmt::Return { span, .. }
        | Stmt::Break { span } => span.clone(),
        Stmt::Let { name_span, .. }
        | Stmt::Const { name_span, .. }
        | Stmt::LetScatter { name_span, .. } => name_span.clone(),
        Stmt::LocalFnDecl(f) => f.span.clone(),
        Stmt::Expr(expr) => expr.span.clone(),
    }
}

fn compose_entry_span(entry: &ComposeEntry) -> Span {
    match entry {
        ComposeEntry::Expr { expr, .. } => expr.span.clone(),
        ComposeEntry::Block { span, .. }
        | ComposeEntry::If { span, .. }
        | ComposeEntry::InSpace { span, .. }
        | ComposeEntry::For { span, .. } => span.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ResolvedMaterialProperties, validate_material_properties};
    use crate::ast::{Expr, MaterialChannelDecl, MaterialPropertiesDecl, Spanned, Unit};
    use crate::check::CheckOptions;
    use crate::check::Value;
    use crate::hir::Sx;

    fn span(start: usize, end: usize) -> std::ops::Range<usize> {
        start..end
    }

    fn num(value: f64) -> crate::ast::SExpr {
        Spanned {
            node: Expr::Num(value, Unit::None),
            span: span(0, 1),
        }
    }

    fn color(rgba: [f32; 4]) -> crate::ast::SExpr {
        Spanned {
            node: Expr::Color(rgba),
            span: span(0, 1),
        }
    }

    fn vec3(x: f64, y: f64, z: f64) -> crate::ast::SExpr {
        Spanned {
            node: Expr::Vec3(Box::new(num(x)), Box::new(num(y)), Box::new(num(z))),
            span: span(0, 1),
        }
    }

    fn channel(
        name: &str,
        ty_name: &str,
        default: Option<crate::ast::SExpr>,
    ) -> MaterialChannelDecl {
        MaterialChannelDecl {
            compose: None,
            name: name.to_string(),
            name_span: span(1, 2),
            ty_name: ty_name.to_string(),
            ty_span: span(3, 4),
            default,
            span: span(1, 4),
        }
    }

    fn properties_decl(
        name: &str,
        extends_name: Option<&str>,
        channels: Vec<MaterialChannelDecl>,
    ) -> MaterialPropertiesDecl {
        MaterialPropertiesDecl {
            context_parameter: None,
            context: None,
            composition: None,
            is_default: false,
            evaluator: None,
            name: name.to_string(),
            name_span: span(10, 11),
            extends_name: extends_name.map(str::to_string),
            extends_span: extends_name.map(|_| span(12, 13)),
            channels,
            span: span(10, 20),
        }
    }

    #[test]
    fn material_properties_inheritance_flattens_parent_channels() {
        let base = properties_decl(
            "base_surface",
            None,
            vec![
                channel("albedo", "color", Some(color([1.0, 0.0, 0.0, 1.0]))),
                channel("roughness", "f32", Some(num(0.5))),
            ],
        );
        let child = properties_decl(
            "pbr_surface",
            Some("base_surface"),
            vec![
                channel("roughness", "f32", Some(num(0.75))),
                channel("metallic", "f32", Some(num(0.1))),
            ],
        );

        let resolved = validate_material_properties(
            &[base, child],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        )
        .expect("valid inheritance should resolve");

        let resolved_child: &ResolvedMaterialProperties = resolved
            .get("pbr_surface")
            .expect("flattened child properties should be present");
        assert_eq!(resolved_child.channels.len(), 3);
        assert_eq!(resolved_child.channels[0].name, "albedo");
        assert_eq!(resolved_child.channels[1].name, "roughness");
        assert_eq!(resolved_child.channels[2].name, "metallic");
        assert!(
            matches!(resolved_child.channels[1].default_payload.as_ref().unwrap(), Value::Scalar(Sx::Lit(v)) if *v == 0.75)
        );
    }

    #[test]
    fn material_properties_accepts_spatial_vec3_channel_type() {
        let props = properties_decl(
            "standard_like",
            None,
            vec![channel(
                "normal",
                "vec3 in world",
                Some(vec3(0.0, 0.0, 1.0)),
            )],
        );

        let resolved = validate_material_properties(
            &[props],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        )
        .expect("spatial vec3 channel type should be accepted");

        let resolved_props = resolved
            .get("standard_like")
            .expect("resolved material_properties should be present");
        assert_eq!(resolved_props.channels.len(), 1);
        assert_eq!(resolved_props.channels[0].name, "normal");
        assert_eq!(resolved_props.channels[0].ty_name, "vec3 in world");
    }

    #[test]
    fn material_properties_inherited_spatial_vec3_override_requires_explicit_transform() {
        let base = properties_decl(
            "base_surface",
            None,
            vec![channel(
                "normal",
                "vec3 in world",
                Some(vec3(0.0, 0.0, 1.0)),
            )],
        );
        let derived = properties_decl(
            "derived_surface",
            Some("base_surface"),
            vec![channel(
                "normal",
                "vec3 in object",
                Some(vec3(1.0, 0.0, 0.0)),
            )],
        );

        let diags = validate_material_properties(
            &[base, derived],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        )
        .expect_err("override across labeled spaces should require an explicit transform");

        assert!(
            diags.iter().any(|d| d
                .message
                .contains("no implicit space transform is available")),
            "expected transform-required diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn material_properties_inherited_unlabeled_vec3_can_override_labeled_vec3() {
        let base = properties_decl(
            "base_surface",
            None,
            vec![channel("normal", "vec3", Some(vec3(0.0, 0.0, 1.0)))],
        );
        let derived = properties_decl(
            "derived_surface",
            Some("base_surface"),
            vec![channel(
                "normal",
                "vec3 in object",
                Some(vec3(1.0, 0.0, 0.0)),
            )],
        );

        let resolved = validate_material_properties(
            &[base, derived],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &CheckOptions::default(),
        )
        .expect("unlabeled vector channel should bridge to labeled vector channel");

        let resolved_child = resolved
            .get("derived_surface")
            .expect("resolved child material_properties should be present");
        assert_eq!(resolved_child.channels.len(), 1);
        assert_eq!(resolved_child.channels[0].name, "normal");
        assert!(
            matches!(resolved_child.channels[0].default_payload.as_ref().unwrap(), Value::Vec3((Sx::Lit(x), Sx::Lit(y), Sx::Lit(z))) if (*x, *y, *z) == (1.0, 0.0, 0.0))
        );
    }
}
