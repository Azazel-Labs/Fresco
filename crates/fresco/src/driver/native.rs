//! Complete direct-IR compute compilation for engine-selected native functions.
//! No source backend participates in this path.
mod expressions;
mod functions;
mod statements;
#[cfg(test)]
mod tests;
mod types;

use crate::ast::{self, SExpr};
use crate::native_type::{Access, Type};
pub use naga::ResourceBinding;
use naga::{Expression as Ex, Handle};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

const SP: naga::Span = naga::Span::UNDEFINED;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    #[default]
    Portable,
    NativeSpirV,
}

impl Target {
    fn capabilities(self) -> naga::valid::Capabilities {
        let mut caps = naga::valid::Capabilities::default();
        if self == Self::NativeSpirV {
            caps |= naga::valid::Capabilities::PHYSICAL_STORAGE_BUFFER_ADDRESSES;
        }
        caps
    }
}

/// Engine-owned entry registration. A sole `u32` parameter receives invocation ID.x.
#[derive(Clone, Debug)]
pub struct Options {
    pub target: Target,
    pub entry_point: String,
    pub workgroup_size: [u32; 3],
    /// Explicit host bindings for root records (read-only storage buffers).
    pub bindings: BTreeMap<String, ResourceBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub span: std::ops::Range<usize>,
    pub message: String,
}

impl Error {
    fn new(span: std::ops::Range<usize>, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}..{}: {}",
            self.span.start, self.span.end, self.message
        )
    }
}
impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize)]
pub struct FieldLayout {
    pub name: String,
    pub offset: u32,
    pub ty: Type,
}

#[derive(Clone, Debug, Serialize)]
pub struct TypeLayout {
    pub ty: Type,
    pub size: u32,
    pub alignment: u32,
    pub array_stride: Option<u32>,
    pub fields: Vec<FieldLayout>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Requirements {
    pub physical_storage_buffer_addresses: bool,
    pub reads_device_memory: bool,
    pub writes_device_memory: bool,
}

/// A complete validated module, including its entry point, helper functions and ABI.
pub struct Artifact {
    pub module: naga::Module,
    pub info: naga::valid::ModuleInfo,
    pub layouts: Vec<TypeLayout>,
    pub requirements: Requirements,
    target: Target,
}

impl Artifact {
    pub fn manifest_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "requirements": self.requirements,
            "layouts": self.layouts,
            "entry_points": self.module.entry_points.iter().map(|e| serde_json::json!({"name": e.name, "workgroup_size": e.workgroup_size})).collect::<Vec<_>>(),
            "roots": self.module.global_variables.iter().map(|(_, g)| serde_json::json!({"name": g.name, "group": g.binding.map(|b| b.group), "binding": g.binding.map(|b| b.binding), "type": self.module.types[g.ty].name})).collect::<Vec<_>>(),
        })).map_err(|e| Error::new(0..0, format!("native manifest serialization failed: {e}")))
    }
    pub fn spirv(&self) -> Result<Vec<u32>> {
        let options = naga::back::spv::Options {
            lang_version: (1, 3),
            // Restrict never manufactures a null pointer for pointer-valued loads.
            bounds_check_policies: naga::proc::BoundsCheckPolicies {
                index: naga::proc::BoundsCheckPolicy::Restrict,
                buffer: naga::proc::BoundsCheckPolicy::Restrict,
                ..Default::default()
            },
            ..Default::default()
        };
        naga::back::spv::write_vec(&self.module, &self.info, &options, None)
            .map_err(|e| Error::new(0..0, format!("SPIR-V emission failed: {e}")))
    }

    /// Revalidate after IR transformations using the original target permissions.
    pub fn compact(&mut self) -> Result<()> {
        naga::compact::compact(&mut self.module, naga::compact::KeepUnused::No);
        self.info = validate(&self.module, self.target)?;
        Ok(())
    }
}

fn validate(module: &naga::Module, target: Target) -> Result<naga::valid::ModuleInfo> {
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), target.capabilities())
        .validate(module)
        .map_err(|e| Error::new(0..0, format!("native IR validation failed: {e:?}")))
}

/// Compile authored Fresco declarations into a complete compute module.
/// Engine registration, target permissions and descriptor bindings are explicit inputs.
pub fn compile(source: &str, options: &Options) -> Result<Artifact> {
    let map_error = |diags: Vec<crate::diag::Diag>| {
        let first = diags
            .into_iter()
            .next()
            .expect("failed parsing produces a diagnostic");
        Error::new(first.span, first.message)
    };
    let tokens = super::pipeline::lex(source).map_err(map_error)?;
    let program = super::pipeline::parse_syntax(source, &tokens, "native.fr").map_err(map_error)?;
    let mut names = HashSet::new();
    for function in &program.functions {
        if !names.insert(&function.name) {
            return Err(Error::new(
                function.name_span.clone(),
                "native function overload resolution is not implemented; function names must be unique",
            ));
        }
    }
    let mut names = HashSet::new();
    for record in &program.structs {
        if !names.insert(&record.name) {
            return Err(Error::new(
                record.name_span.clone(),
                "duplicate native record declaration",
            ));
        }
    }
    if !program.imports.is_empty() {
        return Err(Error::new(
            program.imports[0].span.clone(),
            "native compilation requires resolved source; module imports are not supported yet",
        ));
    }
    let entry = program
        .functions
        .iter()
        .find(|f| f.name == options.entry_point)
        .ok_or_else(|| {
            Error::new(
                0..0,
                format!("unknown native entry `{}`", options.entry_point),
            )
        })?;
    if entry.ret_ty.is_some()
        || entry.params.len() > 1
        || entry.params.first().is_some_and(|p| p.ty_name != "u32")
    {
        return Err(Error::new(
            entry.span.clone(),
            "compute entry must return void and accept zero parameters or one u32 invocation index",
        ));
    }
    if options.workgroup_size.contains(&0) {
        return Err(Error::new(
            entry.span.clone(),
            "compute workgroup dimensions must be positive",
        ));
    }
    if entry.is_builtin
        || !entry.type_params.is_empty()
        || !entry.const_params.is_empty()
        || entry
            .params
            .iter()
            .any(|p| p.default.is_some() || p.is_context || p.keyword_only)
    {
        return Err(Error::new(
            entry.span.clone(),
            "native entries must be concrete functions without default or context parameters",
        ));
    }
    for name in options.bindings.keys() {
        if !program.params.iter().any(|p| &p.name == name) {
            return Err(Error::new(
                0..0,
                format!("binding supplied for unknown native root `{name}`"),
            ));
        }
    }
    let mut compiler = Compiler {
        program: &program,
        target: options.target,
        module: naga::Module::default(),
        types: HashMap::new(),
        resolving: HashSet::new(),
        globals: HashMap::new(),
        requirements: Requirements::default(),
        functions: HashMap::new(),
        compiling_functions: HashSet::new(),
    };
    let mut used_bindings = HashSet::new();
    for param in &program.params {
        let binding = options.bindings.get(&param.name).ok_or_else(|| {
            Error::new(
                param.span.clone(),
                format!("missing native host binding for `{}`", param.name),
            )
        })?;
        if !used_bindings.insert((binding.group, binding.binding)) {
            return Err(Error::new(
                param.span.clone(),
                "duplicate native host binding",
            ));
        }
        let ty = Type::parse(&param.ty_name).map_err(|e| Error::new(param.ty_span.clone(), e))?;
        if !matches!(ty, Type::Record(_)) || param.default.is_some() || param.range.is_some() {
            return Err(Error::new(
                param.span.clone(),
                "native roots must be host-supplied records without parameter defaults or ranges",
            ));
        }
        let resolved = compiler.resolve(&ty, &param.ty_span)?;
        let handle = compiler.module.global_variables.append(
            naga::GlobalVariable {
                name: Some(param.name.clone()),
                space: naga::AddressSpace::Storage {
                    access: naga::StorageAccess::LOAD,
                },
                binding: Some(*binding),
                ty: resolved.handle,
                init: None,
                memory_decorations: Default::default(),
            },
            SP,
        );
        compiler.globals.insert(param.name.clone(), (handle, ty));
    }
    let mut function = Function::default();
    if let Some(param) = entry.params.first() {
        let vector = compiler.module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Vector {
                    size: naga::VectorSize::Tri,
                    scalar: naga::Scalar::U32,
                },
            },
            SP,
        );
        function.ir.arguments.push(naga::FunctionArgument {
            name: Some("invocation_id".into()),
            ty: vector,
            binding: Some(naga::Binding::BuiltIn(naga::BuiltIn::GlobalInvocationId)),
        });
        let id = function.expr(Ex::FunctionArgument(0), false);
        let index = function.expr(Ex::AccessIndex { base: id, index: 0 }, true);
        function.variables.insert(
            param.name.clone(),
            Value {
                expression: index,
                ty: Type::U32,
                place: None,
            },
        );
    }
    compiler.statements(
        &mut function,
        entry.typed_body.as_ref().unwrap_or(&entry.body),
    )?;
    compiler.module.entry_points.push(naga::EntryPoint {
        name: entry.name.clone(),
        stage: naga::ShaderStage::Compute,
        early_depth_test: None,
        workgroup_size: options.workgroup_size,
        workgroup_size_overrides: None,
        function: function.finish(),
        mesh_info: None,
        task_payload: None,
        incoming_ray_payload: None,
    });
    let info = validate(&compiler.module, options.target)?;
    let mut layouts: Vec<_> = compiler.types.into_values().map(|t| t.layout).collect();
    layouts.sort_by_key(|t| format!("{:?}", t.ty));
    Ok(Artifact {
        module: compiler.module,
        info,
        layouts,
        requirements: compiler.requirements,
        target: options.target,
    })
}

#[derive(Clone)]
struct Resolved {
    handle: Handle<naga::Type>,
    layout: TypeLayout,
}

struct Compiler<'a> {
    program: &'a ast::Program,
    target: Target,
    module: naga::Module,
    types: HashMap<Type, Resolved>,
    resolving: HashSet<Type>,
    globals: HashMap<String, (Handle<naga::GlobalVariable>, Type)>,
    requirements: Requirements,
    functions: HashMap<String, functions::Signature>,
    compiling_functions: HashSet<String>,
}

#[derive(Clone)]
struct Place {
    writable: bool,
    physical: bool,
}

#[derive(Clone)]
struct Value {
    expression: Handle<Ex>,
    ty: Type,
    place: Option<Place>,
}

#[derive(Default)]
struct Function {
    ir: naga::Function,
    body: naga::Block,
    variables: HashMap<String, Value>,
    result: Option<Type>,
}

impl Function {
    fn expr(&mut self, expr: Ex, emit: bool) -> Handle<Ex> {
        let handle = self.ir.expressions.append(expr, SP);
        if emit {
            self.body.push(
                naga::Statement::Emit(naga::Range::new_from_bounds(handle, handle)),
                SP,
            );
        }
        handle
    }
    fn finish(mut self) -> naga::Function {
        self.ir.body = self.body;
        self.ir
    }
}
