use super::*;

const SOURCE: &str = r#"
struct Roots {
    input: Ptr<array<u32, 64>, Access.Read>
    output: Ptr<array<u32, 64>>
}
param roots: Roots
fn update_one(index: u32) {
    if index < u32(64) {
        let output = roots.output
        (*output)[index] = (*roots.input)[index] * u32(3) + u32(7)
    }
}
"#;

fn options() -> Options {
    Options {
        target: Target::NativeSpirV,
        entry_point: "update_one".into(),
        workgroup_size: [64, 1, 1],
        bindings: BTreeMap::from([(
            "roots".into(),
            ResourceBinding {
                group: 0,
                binding: 0,
            },
        )]),
    }
}

#[test]
fn native_pointer_complete_ir_and_abi() {
    let mut artifact = compile(SOURCE, &options()).unwrap();
    assert_eq!(artifact.module.entry_points.len(), 1);
    let roots = artifact
        .layouts
        .iter()
        .find(|l| l.ty == Type::Record("Roots".into()))
        .unwrap();
    assert_eq!((roots.size, roots.alignment), (16, 8));
    assert_eq!(
        roots.fields.iter().map(|f| f.offset).collect::<Vec<_>>(),
        [0, 8]
    );
    assert!(artifact.requirements.physical_storage_buffer_addresses);
    assert!(artifact.requirements.reads_device_memory);
    assert!(artifact.requirements.writes_device_memory);
    assert!(
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default()
        )
        .validate(&artifact.module)
        .is_err()
    );
    assert_eq!(artifact.spirv().unwrap()[0], 0x0723_0203);
    artifact.compact().unwrap();
    assert_eq!(artifact.spirv().unwrap()[0], 0x0723_0203);
}

#[test]
fn native_pointer_permission_and_target_errors() {
    let readonly = SOURCE.replace(
        "output: Ptr<array<u32, 64>>",
        "output: Ptr<array<u32, 64>, Access.Read>",
    );
    let error = compile(&readonly, &options()).err().unwrap();
    assert!(error.message.contains("read-only"), "{error:?}");
    assert!(error.span.start < error.span.end);
    let mut portable = options();
    portable.target = Target::Portable;
    assert!(
        compile(SOURCE, &portable)
            .err()
            .unwrap()
            .message
            .contains("native SPIR-V target")
    );
    let raw = SOURCE.replace("(*output)[index]", "output[index]");
    let error = compile(&raw, &options()).err().unwrap();
    assert!(error.message.contains("raw pointer indexing"), "{error:?}");
}

#[test]
fn native_pointer_member_address_and_alias_effects() {
    let source = r#"
struct Cell { value: u32 }
struct Roots {
    first: Cell*
    second: Ptr<Cell>
}
param roots: Roots
fn update_one() {
    let p = &roots.first.value
    *p = u32(17)
    roots.second->value = roots.first.value + u32(1)
}
"#;
    let artifact = compile(source, &options()).unwrap();
    assert!(artifact.requirements.writes_device_memory);
    assert!(artifact.requirements.reads_device_memory);
    assert!(
        artifact.module.entry_points[0]
            .function
            .local_variables
            .is_empty()
    );
    artifact.spirv().unwrap();
}

#[test]
fn native_pointer_spelling_and_access_identity() {
    assert_eq!(
        Type::parse("u32*").unwrap(),
        Type::parse("Ptr<u32, Access.ReadWrite, AddressSpace.Device>").unwrap()
    );
    assert_ne!(
        Type::parse("Ptr<u32>").unwrap(),
        Type::parse("Ptr<u32, Access.Read>").unwrap()
    );
    assert!(Type::parse("Ptr<u32, Access.Read, AddressSpace.Function>").is_err());
    assert!(Type::parse("array<u32>").is_err());
}

#[test]
fn native_pointer_rejects_permission_changes_and_mutable_pointer_locals() {
    for (declaration, expected) in [
        (
            "let output: Ptr<array<u32, 64>, Access.Read> = roots.output",
            "not implicit",
        ),
        ("var output = roots.output", "mutable pointer locals"),
    ] {
        let source = SOURCE.replace("let output = roots.output", declaration);
        let error = compile(&source, &options()).err().unwrap();
        assert!(error.message.contains(expected), "{error:?}");
    }
}

#[test]
fn native_pointer_binding_and_pointee_mutability_are_distinct() {
    let source = r#"
struct Cell { value: u32 }
struct Roots { cell: Cell* }
param roots: Roots
fn update_one() {
    let p = roots.cell
    p.value = u32(4)
}
"#;
    compile(source, &options()).unwrap().spirv().unwrap();
    let illegal = source.replace("p.value = u32(4)", "p = roots.cell");
    assert!(
        compile(&illegal, &options())
            .err()
            .unwrap()
            .message
            .contains("assignable memory")
    );
    let readonly = source.replace("cell: Cell*", "cell: Ptr<Cell, Access.Read>");
    assert!(
        compile(&readonly, &options())
            .err()
            .unwrap()
            .message
            .contains("read-only")
    );
}

#[test]
fn native_pointer_projection_loads_are_narrow_and_ordered() {
    let source =
        include_str!("../../../../../integrations/example-engine/examples/native_pointers.fr");
    let artifact = compile(source, &options()).unwrap();
    let function = &artifact.module.entry_points[0].function;
    let body = function
        .body
        .iter()
        .find_map(|s| match s {
            naga::Statement::If { accept, .. } => Some(accept),
            _ => None,
        })
        .unwrap();
    let first_store = body
        .iter()
        .position(|s| matches!(s, naga::Statement::Store { .. }))
        .unwrap();
    let second_store = body
        .iter()
        .rposition(|s| matches!(s, naga::Statement::Store { .. }))
        .unwrap();
    assert!(first_store < second_store);
    assert!(
        body.iter()
            .skip(first_store + 1)
            .take(second_store - first_store - 1)
            .any(|s| {
                if let naga::Statement::Emit(range) = s {
                    range
                        .clone()
                        .any(|h| matches!(function.expressions[h], Ex::Load { .. }))
                } else {
                    false
                }
            }),
        "the aliased read must be emitted after the first store"
    );
}

#[test]
fn native_pointer_backend_gaps_have_operation_diagnostics() {
    let cases = [
        ("let output = roots.output + u32(1)", "pointer arithmetic"),
        ("let output = &index", "address-of requires"),
        ("let output = u32(roots.output)", "pointer conversions"),
    ];
    for (declaration, expected) in cases {
        let source = SOURCE.replace("let output = roots.output", declaration);
        let error = compile(&source, &options()).err().unwrap();
        assert!(error.message.contains(expected), "{error:?}");
    }
}

#[test]
fn native_void_helpers_preserve_calls_and_memory_effects() {
    let source = r#"
struct Roots { output: u32* }
param roots: Roots
fn store(value: u32) { *roots.output = value }
fn update_one() { store(u32(4294967295)) }
"#;
    let artifact = compile(source, &options()).unwrap();
    assert_eq!(artifact.module.functions.len(), 1);
    assert!(artifact.requirements.writes_device_memory);
    assert!(artifact.module.functions.iter().any(|(_, f)| {
        f.expressions
            .iter()
            .any(|(_, e)| matches!(e, Ex::FunctionArgument(0)))
    }));
    assert!(
        artifact.module.entry_points[0]
            .function
            .expressions
            .iter()
            .any(|(_, e)| matches!(e, Ex::Literal(naga::Literal::U32(u32::MAX))))
    );
    artifact.spirv().unwrap();
}
