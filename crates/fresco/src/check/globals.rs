//! Struct-typed global `param` declarations and their linkage to runtime
//! channels.
//!
//! Engine or stdlib `.fr` files declare a struct plus a global `param`
//! binding of that struct type, and plain FR accessor functions that read
//! from it:
//!
//! ```fresco
//! struct FrameGlobals {
//!     time: f32
//!     delta_time: f32
//!     resolution: vec2
//! }
//!
//! param frame: FrameGlobals
//!
//! fn time() -> f32 { return frame.time }
//! ```
//!
//! Scalar/array/texture-typed global `param` declarations don't go through
//! this module at all — they're declared via [`Checker::declare_global_params`],
//! which forwards straight to the same [`Checker::declare_param`] used for
//! canvas/surface-scoped params, with full `var<uniform>` lowering already in
//! place. Only a `param` whose type is a struct ends up here, since it
//! bundles multiple fields into one uniform buffer instead of one global per
//! field.
//!
//! The merged program (stdlib prelude + implicit engine modules + imports) is
//! checked as one library, so `frame.time` resolves generically through
//! ordinary struct field-access checking — no native semantics are attached
//! to the field name itself. Native compiler internals that need "the
//! current time" (animation builtins, unit conversions, entry-param
//! fallbacks, …) call the FR-authored `time()` / `delta_time()` /
//! `resolution()` functions via [`Checker::runtime_time`] and friends,
//! rather than referencing any field directly. Every field on a struct-typed
//! global param — including custom, non-channel fields — gets real
//! `var<uniform>` lowering via [`crate::hir::Sx::UniformField`] (see
//! `lower.rs`/`lower/surface.rs`); there is no restricted allowlist of
//! "known channels" anymore.

use super::*;

/// A global uniform declaration lowered into checker metadata: the binding
/// name, the struct type, and the channel fields in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalUniformDef {
    pub name: String,
    pub ty_name: String,
    /// `(field_name, ty_name)` pairs in struct declaration order.
    pub fields: Vec<(String, String)>,
}

impl GlobalUniformDef {
    /// Byte offsets and component counts for the compiler's f32-based uniform ABI.
    pub(crate) fn field_layouts(&self) -> Vec<(u32, u32)> {
        let mut offset = 0u32;
        self.fields
            .iter()
            .map(|(_, ty)| {
                let components = match field_shape(ty).expect("validated global uniform field type")
                {
                    FieldShape::Scalar => 1,
                    FieldShape::Vec2 => 2,
                    FieldShape::Vec3 => 3,
                    FieldShape::Vec4 => 4,
                };
                offset = offset.next_multiple_of(if components == 3 { 16 } else { components * 4 });
                let layout = (offset, components);
                offset += components * 4;
                layout
            })
            .collect()
    }

    pub(crate) fn byte_size(&self) -> u32 {
        self.field_layouts()
            .last()
            .map_or(16, |(offset, components)| {
                (offset + components * 4).next_multiple_of(16).max(16)
            })
    }
}

/// Registry of the `@global_uniform` declarations visible to one check pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct GlobalUniformRegistry {
    /// Declared globals in declaration order (carried into the HIR for
    /// lowering and manifest emission).
    pub defs: Vec<GlobalUniformDef>,
    /// Root-scope bindable value per declared global name.
    bindings: HashMap<String, Value>,
}

/// A struct-typed global uniform field's naga shape: how many scalar
/// components it decomposes into for [`crate::hir::Sx::UniformField`] reads.
#[derive(Clone, Copy)]
enum FieldShape {
    Scalar,
    Vec2,
    Vec3,
    Vec4,
}

/// Resolve the naga shape for a global uniform field's declared FR type, or
/// `None` if the type isn't supported as a global uniform field.
fn field_shape(ty_name: &str) -> Option<FieldShape> {
    let normalized = strip_spatial_type_suffix(ty_name);
    match normalized {
        "f32" | "f64" | "half" | "i32" | "u32" | "bool" | "signal" | "delta" | "angle"
        | "length" => Some(FieldShape::Scalar),
        "vec2" | "uvec2" | "coord" | "coord_like" | "resolution" => Some(FieldShape::Vec2),
        "vec3" => Some(FieldShape::Vec3),
        "vec4" => Some(FieldShape::Vec4),
        _ => None,
    }
}

/// Build the checker-time [`Value`] for one global uniform field: a real
/// `var<uniform>` struct-member read, decomposed into one
/// [`crate::hir::Sx::UniformField`] leaf per scalar component.
fn field_value(
    binding_name: &std::rc::Rc<str>,
    field_name: &str,
    field_index: u32,
    shape: FieldShape,
) -> Value {
    let field_name: std::rc::Rc<str> = std::rc::Rc::from(field_name);
    let leaf = |component: Option<u8>| Sx::UniformField {
        binding_name: binding_name.clone(),
        field_name: field_name.clone(),
        field_index,
        component,
    };
    match shape {
        FieldShape::Scalar => Value::Scalar(leaf(None)),
        FieldShape::Vec2 => Value::Vec2((leaf(Some(0)), leaf(Some(1)))),
        FieldShape::Vec3 => Value::Vec3((leaf(Some(0)), leaf(Some(1)), leaf(Some(2)))),
        FieldShape::Vec4 => {
            Value::Vec4((leaf(Some(0)), leaf(Some(1)), leaf(Some(2)), leaf(Some(3))))
        }
    }
}

/// Validate struct-typed global `param` declarations (`param name: SomeStruct`)
/// against the merged program's structs and build the checker-time registry.
///
/// Scalar/array/texture-typed entries in `params` are not struct-typed and
/// are silently skipped here — they're declared through the ordinary
/// [`Checker::declare_param`] path instead (see
/// [`Checker::declare_global_params`]).
pub(crate) fn build_global_uniform_registry(
    params: &[GlobalParamDecl],
    structs: &[StructDecl],
    diags: &mut Vec<Diag>,
) -> GlobalUniformRegistry {
    let struct_decls: HashMap<&str, &StructDecl> = structs
        .iter()
        .map(|decl| (decl.name.as_str(), decl))
        .collect();

    let mut registry = GlobalUniformRegistry::default();
    for decl in params {
        let Some(struct_decl) = struct_decls.get(decl.ty_name.as_str()) else {
            // Not a struct type: an ordinary scalar/array/texture global
            // param, handled elsewhere.
            continue;
        };

        if registry.bindings.contains_key(decl.name.as_str()) {
            diags.push(
                Diag::error(
                    decl.name_span.clone(),
                    format!("duplicate global uniform `{}`", decl.name),
                )
                .with_label("global uniform name redefined")
                .with_help("global uniform names must be unique within a program"),
            );
            continue;
        }

        let binding_name: std::rc::Rc<str> = std::rc::Rc::from(decl.name.as_str());
        let mut fields: Vec<(String, String)> = Vec::new();
        let mut bind_fields: HashMap<String, Value> = HashMap::new();
        let mut bad_field = false;
        for (field_index, field) in struct_decl.fields.iter().enumerate() {
            let Some(shape) = field_shape(&field.ty_name) else {
                diags.push(
                    Diag::error(
                        field.ty_span.clone(),
                        format!(
                            "global uniform field `{}` has unsupported type `{}`",
                            field.name, field.ty_name
                        ),
                    )
                    .with_help(
                        "supported global uniform field types: f32-family scalars, vec2, vec3, vec4",
                    ),
                );
                bad_field = true;
                continue;
            };
            let value = field_value(&binding_name, &field.name, field_index as u32, shape);
            let kind = crate::typed_scalar::Kind::element(&field.ty_name)
                .unwrap_or(crate::typed_scalar::Kind::F32);
            let value = Checker::map_value_lanes(value, |lane| {
                crate::typed_scalar::Scalar::cast(lane, kind)
            });
            bind_fields.insert(field.name.clone(), value);
            fields.push((field.name.clone(), field.ty_name.clone()));
        }
        if bad_field {
            continue;
        }

        registry.defs.push(GlobalUniformDef {
            name: decl.name.clone(),
            ty_name: decl.ty_name.clone(),
            fields,
        });
        registry.bindings.insert(
            decl.name.clone(),
            Value::Struct {
                ty_name: decl.ty_name.clone(),
                fields: bind_fields,
            },
        );
    }

    registry
}

impl GlobalUniformRegistry {
    /// A cloneable snapshot of the root-scope bindings, so the checker can
    /// bind them without holding a borrow on itself.
    pub(crate) fn scope_bindings(&self) -> Vec<(String, Value)> {
        self.bindings
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}

impl Checker {
    /// Bind every declared global uniform into the root scope, so FR code can
    /// reference e.g. `frame.time` directly.
    pub(crate) fn bind_global_uniforms(&mut self) {
        for (name, value) in self.global_uniforms.scope_bindings() {
            self.bind(name, value);
        }
    }

    /// Declare every scalar/array/texture-typed global `param` into this
    /// checking context, exactly like a canvas/surface-scoped `param`
    /// statement (full `var<uniform>` lowering already applies uniformly).
    ///
    /// Struct-typed global params (e.g. `param frame: FrameGlobals`) are not
    /// declared here — they're bound via [`Checker::bind_global_uniforms`]
    /// instead, since the whole struct becomes one bundled uniform global
    /// rather than a per-param one.
    pub(crate) fn declare_global_params(
        &mut self,
        params: &[GlobalParamDecl],
        structs: &[StructDecl],
    ) {
        for decl in params {
            if structs.iter().any(|s| s.name == decl.ty_name) {
                continue;
            }
            let Some(default) = &decl.default else {
                self.diags.push(
                    Diag::error(
                        decl.span.clone(),
                        format!("global param `{}` requires a default value", decl.name),
                    )
                    .with_related_label(
                        decl.ty_span.clone(),
                        "this type requires an explicit default",
                    )
                    .with_help(format!(
                        "add a default, e.g. `param {}: {} = ...`",
                        decl.name, decl.ty_name
                    )),
                );
                continue;
            };
            self.declare_param(
                &decl.name,
                &decl.name_span,
                &decl.ty_name,
                default,
                decl.range.as_ref(),
            );
        }
    }

    /// Bind one root-entry/surface parameter to the runtime channel value,
    /// resolved through the FR-authored `time()`/`delta_time()`/`resolution()`
    /// accessor functions when the merged program defines them.
    pub(crate) fn bind_runtime_channel_param(&mut self, param_name: &str, ty_name: &str) -> bool {
        let value = match ty_name {
            "signal" => Value::Scalar(self.runtime_time()),
            "delta" => Value::Scalar(self.runtime_delta_time()),
            "resolution" => {
                let (x, y) = self.runtime_resolution();
                Value::Vec2((x, y))
            }
            _ => return false,
        };
        self.bind(param_name.to_string(), value);
        true
    }

    /// Evaluate a zero-argument function body already resolved by name, used
    /// to call FR-authored accessor functions (`time()`, `delta_time()`,
    /// `resolution()`) from native compiler internals instead of hardcoding
    /// the legacy semantic `Sx` leaves.
    fn eval_zero_arg_engine_fn(&mut self, name: &str) -> Option<Value> {
        let def = self
            .fn_defs(name)?
            .into_iter()
            .find(|d| d.params.is_empty() && d.const_params.is_empty())?;

        if self.fn_call_stack.contains(&def.declaration_identity(name))
            || self.fn_call_stack.len() > 32
        {
            self.diags.push(
                Diag::error(
                    def.span.clone(),
                    format!("recursive or excessively nested runtime accessor `{name}`"),
                )
                .with_file(def.source_file.clone()),
            );
            return None;
        }

        self.fn_call_stack.push(def.declaration_identity(name));
        self.scopes.push(HashMap::new());
        let diag_start = self.diags.len();
        let out = self.eval_fn_body(name, &def);
        for d in self.diags.iter_mut().skip(diag_start) {
            if d.file.is_none() {
                d.file = Some(def.source_file.clone());
            }
        }
        self.scopes.pop();
        self.fn_call_stack.pop();
        if let Some(value) = &out {
            let valid = if name == "resolution" {
                matches!(value, Value::Vec2(_))
            } else {
                matches!(
                    value,
                    Value::Scalar(_) | Value::Distance(_) | Value::Coverage(_) | Value::Mask(_)
                )
            };
            if !valid {
                let expected = if name == "resolution" {
                    "vec2"
                } else {
                    "scalar"
                };
                self.diags.push(
                    Diag::error(
                        def.span.clone(),
                        format!(
                            "runtime accessor `{name}` must return {expected}, found {}",
                            value.kind()
                        ),
                    )
                    .with_file(def.source_file.clone()),
                );
                return None;
            }
        }
        out
    }

    /// The `time` runtime value, resolved by calling the FR-authored `time()`
    /// function when the merged program defines one. Standalone canvases use
    /// their existing entry ABI arguments; absence of an engine is not a
    /// request to replace dynamic inputs with constants.
    pub(crate) fn runtime_time(&mut self) -> Sx {
        if self.hir.entry_context.is_some() || self.evaluation_context.is_some() {
            return match self.context_role("time", &(0..0)) {
                Some(Value::Scalar(value)) => value,
                None => Sx::Lit(0.0), // Poison after a missing-role diagnostic.
                Some(_) => unreachable!("validated context role"),
            };
        }
        if let Some(sx) = &self.runtime_channel_cache.time {
            return sx.clone();
        }
        let sx = match self.eval_zero_arg_engine_fn("time") {
            Some(
                Value::Scalar(sx) | Value::Distance(sx) | Value::Coverage(sx) | Value::Mask(sx),
            ) => sx,
            _ => Sx::EntryInput(hir::EntryInput::Time),
        };
        self.runtime_channel_cache.time = Some(sx.clone());
        sx
    }

    /// The `delta_time` runtime value, resolved through the FR-authored
    /// `delta_time()` function; see [`Checker::runtime_time`].
    pub(crate) fn runtime_delta_time(&mut self) -> Sx {
        if self.hir.entry_context.is_some() || self.evaluation_context.is_some() {
            return match self.context_role("delta_time", &(0..0)) {
                Some(Value::Scalar(value)) => value,
                None => Sx::Lit(0.0), // Poison after a missing-role diagnostic.
                Some(_) => unreachable!("validated context role"),
            };
        }
        if let Some(sx) = &self.runtime_channel_cache.delta_time {
            return sx.clone();
        }
        let sx = match self.eval_zero_arg_engine_fn("delta_time") {
            Some(
                Value::Scalar(sx) | Value::Distance(sx) | Value::Coverage(sx) | Value::Mask(sx),
            ) => sx,
            _ => Sx::EntryInput(hir::EntryInput::Delta),
        };
        self.runtime_channel_cache.delta_time = Some(sx.clone());
        sx
    }

    /// The `resolution` runtime value, resolved through the FR-authored
    /// `resolution()` function; see [`Checker::runtime_time`].
    pub(crate) fn runtime_resolution(&mut self) -> (Sx, Sx) {
        if self.hir.entry_context.is_some() || self.evaluation_context.is_some() {
            return match self.context_role("resolution", &(0..0)) {
                Some(Value::Vec2(value)) => value,
                None => (Sx::Lit(0.0), Sx::Lit(0.0)), // Poison after a missing-role diagnostic.
                Some(_) => unreachable!("validated context role"),
            };
        }
        if let Some(xy) = &self.runtime_channel_cache.resolution {
            return xy.clone();
        }
        let xy = match self.eval_zero_arg_engine_fn("resolution") {
            Some(Value::Vec2(xy)) => xy,
            _ => (
                Sx::EntryInput(hir::EntryInput::ResolutionX),
                Sx::EntryInput(hir::EntryInput::ResolutionY),
            ),
        };
        self.runtime_channel_cache.resolution = Some(xy.clone());
        xy
    }
}

/// Memoizes the resolved runtime-channel `Sx` expressions per check pass, so
/// repeated native call sites (animation builtins, unit conversions, …) don't
/// re-evaluate the FR-authored accessor function bodies on every use.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeChannelCache {
    time: Option<Sx>,
    delta_time: Option<Sx>,
    resolution: Option<(Sx, Sx)>,
}
