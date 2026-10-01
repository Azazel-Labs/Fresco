# Native device pointers in Fresco

Status: fixed-size native compute slice implemented and validated, 2026-09-28.
The broader integration plan remains in progress.

Implemented in the main compiler: a complete direct-IR compute API and CLI,
structural pointer access types, fixed-array/record projections, SSA pointer
bindings, access checks, ABI reflection, and explicit native SPIR-V permissions.
Portable engine-stage composition has not migrated. Naga helper pointers,
spans, integer/address conversions and the real Lumina material path remain open.
See [LANGUAGE.md](../LANGUAGE.md#native-device-pointers) for the implemented surface.
The workspace uses the Naga/wgpu fork at `5a19e8777`; that dependency change
supplies the backend for the explicit native compiler; portable permissions remain unchanged.

## Implemented route and execution evidence

`fresco::driver::native::compile` takes an explicit target, entry name, workgroup
size and root bindings. The `fresco native` command exposes the same route. It
uses the shared lexer/parser, structural native types and checked memory places,
then constructs Naga expressions, statements, helper functions and a compute
entry directly. No WGSL writer/parser participates. See `LANGUAGE.md` for its
current scalar/record/fixed-array language subset and unsupported constructs.
The regular engine pass compiler is still a separate migration item.

The local `native_pointers` example compiled its authored `.fr` file and executed
on an NVIDIA GeForce RTX 2080 Ti, driver 596.49. Separate and aliased addresses,
before and after compaction, passed: 512 values, 16 guard values, and 256 guarded-out
invocations. `spirv-val` accepted every module, the safe wgpu API rejected them,
and the Khronos validation canary remained clear through device destruction.
This is evidence for typed fixed-size pointer access, not Lumina's integer-root,
GPUSpan, scalar-layout, or material ABI.

Tracked compiler tests cover layouts/access identity, permissions through aliases,
SSA bindings, narrow projections, load ordering across stores, target rejection,
unsupported operations, ordinary helper calls, SPIR-V validation and CLI outputs.
The hardware example is opt-in and stays out of CI. There are no Slang compatibility
tests or Slang build dependencies.

## Objective and ownership

Let engine authors describe native pointer-based data layouts and access them
through ordinary typed Fresco code. Compile complete shader modules directly
to Naga IR, then emit the selected backend. No WGSL text escape hatch or
WGSL parser roundtrip may sit on the native-pointer path.

Implement the language and compiler in the main Fresco workspace. Keep the
Lumina checkout and its integration `.fr` files in ignored `.experiments/`.
Keep durable, engine-independent regression tests in the tracked test suites.
Coordinate with the [engine syntax redesign](../TODO.md#engine-integration-follow-up)
and [target validation profiles](../TODO.md#source-of-truth-issue-5).

## Evidence and current limits

The fork supports eight-byte physical pointers in host-shareable data, fixed-size
pointees, pointer chasing, field/array access, and aligned SPIR-V loads/stores.
It rejects pointer function arguments/returns/local storage, runtime-sized
pointees, atomics, cooperative operations, and integer/address conversions.
Non-SPIR-V writers reject physical pointers. Pointer-valued zero-fill bounds
checks are rejected because the emitter cannot manufacture a valid null value.
See the [fork's prototype notes](https://github.com/nickdarnell/wgpu/blob/5a19e8777f21fbc1f88096dc9bc9d905304636d7/naga/PHYSICAL_STORAGE.md).

Local Lumina evidence, under `Engine/Resources/Shaders/Includes/`:

- `GPUSpan.slang`: `T* Data`, `uint Count`, padding; guarded reads/writes and
  element-address access. This is a 16-byte engine ABI, not just a raw pointer.
- `GlobalRHI.slang`: root addresses stored as `uint64_t`, converted to `T*`.
- `SceneGlobals.slang`: pointer-returning accessors, generic `Ptr<T>` helpers,
  pointer fields/spans, and separately stored bindless texture indices.
- Local comments identify scalar layout and pointer-containing aggregate
  materialization concerns. Treat these as integration requirements to verify
  with reflected offsets, `spirv-val`, and execution, not as universal ABI rules.

The current fullscreen Lumina probe does not prove these data paths work.

## Accepted authoring direction

Use Slang's pointer syntax and semantics wherever the target permits them.
Keep Fresco's surrounding declaration grammar (`name: Type`, `fn`, existing
array types). Do not introduce a competing pointer model to fit temporary Naga
limitations. Track unsupported operations as explicit implementation gaps.

Keep three concepts distinct:

| Concept | Meaning | Initial direction |
| --- | --- | --- |
| `T*` / `Ptr<T>` | Native device address of a typed object | Equivalent compiler types; explicit native target requirement |
| Bounded span | Address plus element count | Later view with a specified ABI; Lumina's `GPUSpan<T>` is an engine abstraction |
| `DescriptorHandle<T>` direction | Descriptor/table reference | Separate resource work; opaque resources are not ordinary pointer pointees |

Adopt `T*` and `Ptr<T>` as equivalent to
`Ptr<T, Access.ReadWrite, AddressSpace.Device>`. Select read-only access with
`Ptr<T, Access.Read>`. Preserve access/address-space traits in type identity.
Do not add implicit read-write to read-only conversion; follow Slang's explicit
conversion rules as each conversion is implemented. Do not add C-style `const`
pointer syntax. Treat Slang's `ImmutablePtr<T>` as a separate follow-up with a
stronger memory contract, not an alias for ordinary read-only access.
[Slang pointer reference](https://docs.shader-slang.org/en/stable/external/slang/docs/language-reference/types-pointer.html).

Fresco operations to implement:

- `*p` loads/stores the pointee according to expression/assignment context.
- `p.field` and `p->field` project through a pointer to a record, then load only the selected
  field when a value is required. Do not copy the entire record to access a field.
- `&p.field` obtains a physical subobject address. Taking the address of a local
  value does not produce a device pointer; reject it in the initial feature.
- `p[i]` indexes consecutive pointees of `p: T*`, with the same element-address
  meaning as `*(p + i)`. Addition/subtraction use pointee stride, not byte offsets.
  Preserve the same-array and one-past-end rules; one-past pointers cannot be
  dereferenced. Never reinterpret raw pointer indexing as a bounded span.
- The first fixed-array slice uses `(*p)[i]` for
  `p: Ptr<array<T, N>>`. This preserves the distinction between indexing the
  pointee array and indexing consecutive array objects through the pointer.
  Raw element-pointer arithmetic waits for the necessary Naga support.
- Never widen access or make nested mutable pointer types covariant. Specify
  conversions, pointer comparisons/differences, null handling, and operator
  precedence in Fresco's semantics as those operations are implemented.
- `let p = roots.values` can bind the loaded pointer as an SSA expression.
  This must not introduce a Naga pointer local variable. Reassignable pointer
  variables and pointer-valued control-flow merges require separate support.

Implemented fixed-size slice (select `update_one` and bind `roots` through native options):

```fresco
struct NativeRoots {
    input: Ptr<array<u32, 64>, Access.Read>
    output: Ptr<array<u32, 64>>
}
param roots: NativeRoots

fn update_one(index: u32) {
    if index < u32(64) {
        (*roots.output)[index] = (*roots.input)[index] * u32(3) + u32(7)
    }
}
```

Entry registration and resource binding remain engine-authored. The native
host supplies address fields; they are not editable material parameters.
Target selection belongs in compiler options and host integration, not another
collection of pointer-specific `@` attributes.

The desired helper API includes `fn shade(p: Ptr<Material, Access.Read>) -> vec4`,
`fn update(p: Material*)`, and
pointer-returning accessors. Those require Naga extensions; do not silently
inline every helper or rewrite pointers into integer/buffer indices to fake support.

## Type, memory, and target semantics

Represent pointer types structurally: pointee, access permission, and address
space. Initially support device physical pointers; diagnose other address spaces
until the selected backend implements them. Preserve traits through AST/type resolution, checked HIR,
specialization, manifests, and lowering. Audit existing string-backed record and
array types; do not spread ad hoc pointer-string parsing across consumers.

Distinguish values from assignable memory locations in the checker and lowering.
Carry access permissions through every field/index projection and reject writes
through read-only pointers, including generic helpers and aliases. Pointer
permission says what this reference may do; it does not promise unique ownership,
immutability through other aliases, or `restrict`/`noalias` semantics.

Loads must observe intervening stores that may alias. Do not hoist, cache,
deduplicate, or constant-fold memory operations as though they were pure values.
Propagate load/store effects through helpers and preserve non-uniformity.
No automatic barriers or cross-invocation race guarantees are implied.

Separate three checks: allowed source/IR features, output-backend support, and
actual device/host support. Physical pointers initially require a native SPIR-V
profile and the fork capability. WGSL/WebGPU, GLSL, HLSL, and MSL must diagnose
reachable physical-pointer use rather than erase it or silently emulate it.
Typecheck declarations normally; collect target feature requirements from the
selected specialized program, including host-visible pointer-bearing layouts.

Reflect required features and memory effects in the artifact. The safe wgpu
shader API must continue rejecting these modules. Execution uses a deliberately
native host path, including unsafe passthrough where wgpu is used. The host owns
address validity, allocation lifetime, alignment, usage, synchronization, and
device feature enablement. A compiler bounds check proves none of those by itself.

Reflect pointer width/alignment separately from pointee layout, member offsets,
array strides, and required load/store alignment. Preserve address bits as bytes
or an exact integer representation; never route them through floating-point
parameters or JavaScript `Number`. Reject unrepresentable layouts rather than
assuming ordinary Naga buffer layout matches Lumina's scalar layout.

Initially diagnose unsupported pointer literals/null, numeric casts, arithmetic, comparisons,
runtime-sized pointees, atomics, pointer stage IO, and unsupported pointer-bearing
aggregate operations. Give operation-specific diagnostics. Fixed-array zero-read /
skip-write policies may apply to ordinary values; never synthesize zero/null for
pointer-valued or pointer-containing results. Keep those operations guarded or
reject them until their semantics and backend support exist. These are staged
backend limitations, not intended permanent departures from Slang.

Slang is the design reference, not a test oracle or build dependency. No pinned
Slang version, compatibility matrix, or cross-compiler comparison tests are
required. Test Fresco's specified behavior, generated IR/SPIR-V, and execution
directly. Track aligned load/store
helpers, immutable pointers, address spaces, and descriptor handles separately.
Document any necessary Fresco deviation with its target restriction and evidence;
do not copy Slang bugs or promise support merely because the grammar accepts it.

## Bounded spans and the Lumina ABI

Plan spans before general engine adoption, but do not claim the fork already
supports them. A runtime-length span of `T` needs checked element-address
formation beyond indexing a fixed-size Naga array. Extend Naga deliberately;
do not fabricate a huge array type or cast addresses through floating point.

Specify address/count offsets, count width, stride, padding, and empty-span
behavior. Lumina's current `{ pointer, u32 count, u32 pad }` is an integration
target to verify. Bounds checks must happen before dereference; address arithmetic
must detect overflow, and a zero-length span must never dereference its address.
Zero-filled reads are only meaningful for supported value types. Define checked
element-address results without relying on the currently unsupported null pointer.

For Lumina's integer root addresses, add explicit integer/pointer conversions
to Naga and Fresco's native boundary, or agree on a genuinely compatible typed
root ABI with the engine. Record that choice; do not quietly change engine layouts.
Pointer helper parameters/returns and scalar-layout compatibility are additional
requirements for the real material path. Atomics and descriptor handles remain
independent milestones, even when a Lumina shader requires both.

## Implementation sequence and acceptance

1. **Target policy and complete IR boundary.** Inventory all validation sites;
   thread one explicit target configuration through compilation and composition.
   Provide a complete Naga module/artifact route for a native entry point with no
   WGSL assembly dependency. Keep existing portable paths working while migrating
   their remaining text lowering; do not force a native module through a WGSL writer.
2. **Fixed-size pointer language slice.** Add structural types, pointer projections,
   access checks, precise spans in diagnostics, reflected ABI, and direct IR lowering.
   Compile a host-supplied fixed-array compute example end to end. Support narrow
   member access and valid SSA pointer bindings; diagnose unsupported contexts.
3. **Real helper and span support in Naga.** Add pointer arguments/returns and any
   needed local/merge representation with correct aliasing rules. Add checked
   element-address formation and explicit integer conversions required by Lumina.
   Validate pointer-containing aggregate handling and required scalar layouts.
   Add matching Fresco features only when these representations are supported.
4. **Lumina integration trial.** Match the actual root/pass/span layouts in
   `.experiments/lumina-fresco`; first execute a pointer-fed compute workload,
   then a material/mesh path. Inventory descriptor-indexing and atomic gaps
   explicitly. Add transferable regressions to the main workspace.

Likely owning code:

- Syntax and spans: `ast.rs`, `parser.rs`, `parser/expr.rs`, `parser/stmt.rs`,
  `parser/top.rs`; inspect shared type parsing before adding a second parser.
- Semantics: `check.rs`, `check/`, `hir.rs`, `resource_type.rs`, specialization.
- Lowering/composition: `lower/ir.rs`, `lower/compute.rs`, `lower/`,
  `driver/emit.rs`, `driver/schema_function.rs`, and engine pass lowering.
- ABI/host contracts: `crates/fresco-artifact/src/`, CLI outputs, and the optional
  engine runtime. Keep raw address provision outside normal safe parameter upload.

Acceptance coverage, without new golden snapshots:

- Parser precedence for `*`, `&`, `.`, `->`, indexing, and nested pointer types;
  `T*` and `Ptr<T>` resolve identically. Test multiplication and return arrows
  remain unambiguous in Fresco's surrounding grammar.
- Fresco semantic tests for access defaults, explicit qualifiers, rejection of
  implicit access conversion, pointer-to-array versus element-pointer indexing,
  helper signatures, and later arithmetic/conversion operations.
- Permission and target rejection through aliases, generics, helper boundaries,
  and composition; native capabilities remain disabled for portable compilation.
- Offset/stride/alignment agreement, bit-exact host address serialization,
  nested records, pointer chasing, and narrow field loads.
- Aliasing regression: store through one pointer and read through another that
  addresses the same object; optimized and uncompacted paths agree.
- Structural IR and `spirv-val` checks for every supported native entry point;
  exercise compaction and specialization. No hidden WGSL frontend dependency.
- Native execution: two independently addressed buffers, visible readback,
  guards, checked boundary indices, and Vulkan validation. Retain the safe-wgpu
  rejection test. Never execute deliberately invalid device addresses.
- Portable example suites remain green. Add equivalent bound-buffer/native
  pointer computations for differential results where both are supported.
- Native GPU tests stay local-only in Fresco CI. CI runs compiler/SPIR-V checks;
  report hardware and driver evidence separately from software validation.

Pointer spellings and read-write device defaults are decided in favor of Slang.
Before implementing spans, settle the bounded
span library API/ABI; do not invent a Slang span primitive that does not exist.
The first milestone must not be
described as full Lumina support. Update `LANGUAGE.md` and regenerate registered
language docs as features become implemented, keeping proposals clearly marked.
