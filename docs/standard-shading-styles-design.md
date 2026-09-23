# Authored material styles and rendering contributions

Status: phases 0-11 are implemented, including typed contracts/providers,
range-scoped reusable draws, owned compute outputs, shading-resource captures,
global transparency, shader lighting services, the executable Toon/MeadowFur
samples, capability reflection, recoverable editor selection, and legacy adapter
retirement, and typed completion ports with inferred placement. The phase sections
below record validation and precise limits.

This document owns the implementation checklist and detailed design history.
[The language and implementation reference](../LANGUAGE.md#styles-and-reusable-operations)
owns the current end-to-end explanation. Worked specification excerpts below may
retain historical alternatives; the linked executable samples and implemented
phase sections establish current syntax.

For illustrated context, see the [style and frame walkthrough](visual-guide.md#styles-and-the-engine-frame).

![Engine inputs connect Toon shading and its outline contribution to the frame between opaque completion and transparency.](graphics/toon-frame.svg)

## Phase-based implementation plan

This is the authoritative implementation and cleanup tracker. `[x]` means the
named deliverable exists and has supporting checks; a proposed syntax block does
not count as implementation. `[ ]` means outstanding, including validation that
has not run. Complete each phase's acceptance items before retiring its predecessor.
Update this checklist, the implementation notes below, and `LANGUAGE.md` when a
phase lands. Compiler/runtime consolidation is tracked in `TODO.md`. Preserve
regression coverage when migrating an old path.

Current position: **phases 0-11 are complete**, including resource-driven placement.
Its implementation and acceptance evidence are recorded below.

### Phase 0 - Specification and working baseline

- [x] Specify complete Toon and MeadowFur examples, including engine obligations,
  operation definitions/calls, resource ownership, and bounded limitations.
- [x] Implement a baseline of symbolic implementation selection, GPU dispatch,
  and per-material mesh contributions through the existing attributes.
- [x] Establish renderer-parity and independent-invocation regression coverage
  for that baseline. These tests are migration evidence, not completion of the new API.

### Phase 1 - Shading contracts and styles

Depends on phase 0. Implemented for shading hooks; graph members belong to phase 3.

- [x] Parse and retain `contract C for schema` and `style S for schema : C` as
  distinct declarations, including imports, source spans, and schema relationships.
- [x] Check exact hook signatures, required/default hooks, duplicate/unknown hooks,
  and unused declarations. Check defaults in the contract's defining scope.
- [x] Lower selected styles into ordinary implementation dispatch without a
  compiler-known StandardStyle, Toon, or renderer name.
- [x] Use structured `direct`, `indirect`, and `finish` in the example engine's
  executable Forward, Forward+, and Deferred paths; migrate authored PBR and Toon hooks.
- [x] Verify direct/indirect/emission accounting and finish overrides across those
  renderers, plus a domain-neutral custom-schema contract.

### Phase 2 - Material settings and shader specialization

Depends on phase 1. Graph specialization is a separate phase 4 deliverable.

- [x] Support runtime scalar/vector/color settings with typed constant defaults,
  ranges, named material assignments, and symbolic JSON overrides.
- [x] Allocate independent per-material runtime records and wire them through
  Forward, Forward+, and Deferred; reject invalid updates atomically.
- [x] Preserve every bit of u32/i32 settings and boolean values through checking,
  reflection, browser defaults, host updates, and GPU transport.
- [x] Implement `static param`, specialized dispatch identities, typed constant
  hook bindings, and separate `static_parameters` reflection without runtime lanes.
- [x] Share code for equal static choices; distinguish different choices while
  retaining independent runtime values and contract-default scope.
- [x] Provide live runtime controls and static rebuild controls. Persist symbols
  and named values; preserve compatible edits without transferring settings to another style.
- [x] Add compiler, CPU, browser unit, and native GPU regression coverage for settings,
  exact integer ranges, specialization, rejected batches, and renderer parity.
- [x] Update schema 8, generated TypeScript contracts, language reference, and design notes.
- [x] Rebuild and smoke-test both compiler and renderer WASM packages with schema 8.

### Phase 3 - Typed engine contracts and renderer providers

Depends on phases 1-2. Implemented; executable syntax and limits are recorded below.

- [x] Add typed contract inputs, capability declarations/members, integration
  points, optionality, and renderer `provide` declarations to AST, imports, and checking.
- [x] Validate provider bindings against real resource types, producer nodes,
  attachment formats/access/sample counts, and downstream consumers; names alone
  must not establish compatibility.
- [x] Implement provider selection and capability/precondition diagnostics for
  the selected renderer, material, and vertex factory. Check unused declarations too.
- [x] Declare integration scope, accepted operation kinds, and writer composition
  policies. Reject unsupported policies rather than treating them as ordinary draws.
- [x] Publish genuine view-wide opaque-completion boundaries in all three renderers,
  with contributions finishing before transparency, inspection, and presentation.
- [x] Keep `at after_opaque` explicit. Prove the incoming and outgoing graph edges
  with a domain-neutral engine fixture and renderer integration tests. Phase 4 exposes `at`;
  phase 10 removes the temporary contribution adapter.

### Phase 3 implementation contract

The executable provider spelling uses the **renderer pipeline identifier**, not
its UI label. Logical phase roles in the contract are explicitly bound to real
recipe nodes by the provider. This closes a gap in the earlier `after`/`next`
sketch: a single successor cannot prove every required downstream phase waits.
For example, the current engine defines:

```fresco
struct OpaqueTarget {
    color: attachment<rgba16float, preserve_update>
    depth: attachment<depth32float, test_only>
}
contract StandardStyle for standard {
    input frame: PreviewScene
    point after_opaque: OpaqueTarget {
        scope: view
        accepts: raster_draws
        composition: ordered_draws(engine.stable_draw_order)
        after: complete_opaque
        before: transparency, inspection, presentation
    }
    // direct, indirect, and finish signatures/defaults follow.
}
provide StandardStyle for renderer_forward {
    frame = frame
    after_opaque {
        complete_opaque: all(preview_mesh)
        transparency: all(transparent)
        inspection: all(scene_background)
        presentation: all(scene_background)
        color: output
        depth: forward_depth
        order: stable_draw_order
    }
}
```

`all(a, b)` denotes completion of every invocation of each named recipe node in
the view. The compiler checks real graph reachability from every incoming node to
every declared downstream node. The existing scene executor runs a recipe step
across all scene objects before advancing to its next step. Forward and Forward+
now split opaque and transparent draws and clear attachments unconditionally;
Deferred uses its existing G-buffer clear and lighting resolve. A view containing
only transparent objects therefore still has initialized opaque targets.

Resource input bindings may name a recipe resource (`frame`) or a concrete binding
endpoint (`preview_mesh.scene`). Their types are checked against actual pass and
factory binding signatures. Internal buffers need a real ordered producer;
allocation is insufficient. Integration attachments must match the declared
format, access, and sample count, have an unconditional stored initializer, and
have downstream consumers. Samples other than one, unsupported scopes/operations,
and composition policies such as `global_transparent_queue` fail explicitly.

A resource capability is declared with typed members and listed in a contract:

```fresco
capability SceneData { frame: PreviewScene }
// Within a contract:
optional capability SceneData
// Within its renderer provider:
SceneData { frame: preview_mesh.scene; factories: preview_static }
// Within a style:
requires SceneData
requires after_opaque
requires material.blend == SurfaceBlend.Opaque && material.two_sided == false
```

Optional means the provider may omit the capability/point. A selected style that
requires it must find it on the selected renderer and every requested vertex
factory (including the default factory). Required capability declarations are
also checked against factory restrictions. All declarations/providers are checked
even when unused; material predicates are typechecked as booleans and evaluated
per selected material. Resource capabilities here expose existing recipe bindings;
prepared geometry, shading services, owned outputs, and shader resource captures
remain the deliverables of phases 5-8, not implicit services provided by this phase.

Point selection stays explicit through `at after_opaque`. Styles call reusable
operations with explicit attachments; the temporary contribution pipeline adapter
has been retired. See LANGUAGE.md's migration guidance and the runnable Toon sample.
Every operation waits for all incoming nodes, and every declared downstream phase
waits for the operation's terminals. Transparent work uses the declared global
queue policy.

Validation includes compiler rejection tests for unused providers/contributions,
resource types, factory restrictions, optional requirements, material predicates,
missing initialization, sample/access mismatches, unordered overlapping points,
and unsupported policies; an independent engine verifies multiple contributors.
All three renderer integrations verify explicit incoming/outgoing edges. Native
GPU readback verifies overlapping opaque/Toon/transparent objects under reordered
primary ownership and transparent-only view initialization. This uncovered and
fixed primary-object-dependent shadow fitting: the engine now fits shared shadows
to an order-independent scene bound. Workspace CI, strict compiler/engine clippy,
browser unit tests, and rebuilt compiler/renderer WASM smoke checks passed; browser
GPU interaction remains part of phase 9.

The final Toon/Fur appendix records the complete design; its MeadowFur operation
and style blocks now match the executable Phase 8 example. This intermediate
adapter alone did not supply those capabilities. Phase 11 still follows the style work: completed opaque
color/depth resource versions may eventually imply placement, but must retain both
the incoming completion edges and outgoing transparency/inspection/presentation
obligations demonstrated here.

### Phase 4 - Style invocation graph and reusable draw operations

Depends on phase 3. Complete this infrastructure before rewriting the Toon sample.

- [x] Implement `draw` definitions, typed named operation calls, and explicit
  attachment arguments. Definition alone must not instantiate any work.
- [x] Implement `for self` identity as frame/view/object/material draw range;
  retain that identity through batching and repeated uses of the same material or mesh.
- [x] Implement `at point as target` and operation `requires` checks against
  immutable invocation metadata, including blend and sidedness restrictions.
- [x] Capture declared settings explicitly in operation invocations; prevent
  shader hooks from capturing arbitrary graph locals or implicit engine resources.
- [x] Implement `static if` and bounded `static for` graph construction. Typecheck
  all branches, activate requirements only for selected branches, and reject runtime
  shader control flow used to construct graph nodes.
- [x] Compose preserving attachment writes in the engine's stable order; validate
  initialization, access, conflicts, discarded outputs, missing bindings, and cycles.
- [x] Prove operation reuse, per-range selection, deterministic composition, and
  independent invocation bindings without recognizing a particular style in the host.

#### Phase 4 executable baseline

Reusable draws currently rasterize a `DrawRange` through an explicitly selected
vertex factory. `self` denotes this view's occurrence of an object/material range;
it is not a mesh asset, material ID, or view-wide fullscreen invocation. The host
retains the submitted vertex/index range, including empty ranges, for every call.
Phase 5 adds the prepared resource input described below; `DrawRange` remains available.

```fresco
@factory(plain)
draw Paint(geometry: DrawRange, tint: color,
           target: attachment<rgba16float, preserve_update>) {
    raster geometry
    visibility: uncullable
    cull: none
    attachments { target: load_store }
    @vertex fn project(v: Corner) -> Projected {
        return Projected(vec4(v.point, 1.0))
    }
    @fragment fn paint(v: Projected) -> vec4 { return tint }
}

style Extra for Value : Action {
    param ink: color = #00ff00
    static param repetitions: u32 = 2u
    fn value(x: f32) -> f32 { return x * 2.0 }
    for self {
        at finish as target {
            static for ordinal in 0u..repetitions {
                Paint(geometry: self, tint: ink, target: target.target)
            }
        }
    }
}
```

Here `Action.finish` is an engine-provided typed point. `Corner`, `Projected`,
`plain`, and `Value` are the independent engine's ordinary declarations. Draw
inputs are named and typechecked, and defining `Paint` adds no renderer work.
Draw defaults are no culling, replacement color, disabled depth writes, and
less-equal depth comparison. Draw bodies see their declared scalar/resource inputs and shader parameters;
they cannot capture the style's graph locals or undeclared factory resources.
A factory's own transform function retains its engine-defined bindings, which
must be explicitly supplied when used. Resource arguments name contract inputs
or target members; scalar arguments are constants/static expressions or direct
runtime setting captures. Runtime arithmetic belongs inside the draw shader.

`requires` on a draw checks immutable material metadata at the call, e.g.
`requires material.blend == SurfaceBlend.Opaque && material.two_sided == false`.
`static if` checks both branches but activates requirements/work only in the
selected branch. `static for` accepts ascending integer `start..end` ranges with
at most 1024 iterations; the whole graph also has a 1024-node construction budget.
Zero-trip bodies are still checked. Runtime `if`/`for` cannot construct graph work.

Every invocation reflects its contract point, material entry, operation, and local
ordinal. Within a point the host executes submitted object/ranges in stable order,
then each occurrence's operations in authored order. All opaque producers finish
before that sequence and all outgoing consumers follow it. Frame/view identity is
bounded by each scene invocation; repeating a material or mesh does not merge its
occurrences. Settings remain material-owned; conflicting edits to the same
material row remain errors. Operation bindings are reflected per mesh pass rather
than added to the vertex factory's public resource namespace.

Attachment arguments require explicit `load_store`; aliases, incompatible formats
or access, and writes through test-only depth fail validation. Provider and graph
validation continue to enforce initialization, preservation, and acyclic incoming
and outgoing dependencies. The artifact schema is version 9 so an older host cannot
silently execute the new per-occurrence ordering as ordinary contribution draws.

### Phase 5 - Prepared geometry and complete Toon migration

Depends on phase 4. Land the direct draw example before user-authored compute operations.

- [x] Implement demand-driven PreparedGeometry producers for supported factories,
  after authored deformation and object transformation. Unsupported factories must fail clearly.
- [x] Use the same prepared stream for the base draw and contributed draw, with
  material-range-local indices, transformed normals, validated counts, and explicit bounds.
- [x] Implement indexed raster inputs, view inputs, conservative visibility/bounds
  policy, culling, depth test/write rules, and attachment preservation for reusable draws.
- [x] Define the entire `InvertedHull` draw operation locally in
  `style_sample.fr`; invoke it inside Toon through the contract point.
- [x] Wire outline width/color runtime settings and static outline activation.
  Disabling the outline must remove its work and geometry requirements.
- [x] Remove the sample's separate `@contribute` pipeline and old `toon_shell`
  authoring path after the new operation matches its behavior.
- [x] Verify mixed-material ranges, two objects sharing a material/mesh, deformation,
  nonuniform transforms, depth occlusion, multiple views, and all three renderers.

#### Phase 5 executable contract

The example now defines the complete `InvertedHull` operation in
`examples/40) surface shaders/style_sample.fr`. Its style graph is:

```fresco
param outline_width: f32 in [0.0, 10.0] = 2.5
param outline_color: color = #2e1938
static param outline_enabled: bool = true
for self {
    static if outline_enabled {
        at after_opaque as target {
            InvertedHull(geometry: mesh.prepared, view: frame,
                         width: outline_width, ink: outline_color,
                         color: target.color, depth: target.depth)
        }
    }
}
```

The engine declares the aggregate and maps its roles explicitly. None of these
resource, capability, field, or function names are compiler-known:

```fresco
struct PreparedVertex {
    world_position: vec3
    geometric_normal: vec3
    world_tangent: vec3
    uv: vec2
    uv2: vec2
}
struct WorldBounds { minimum: vec3; maximum: vec3; valid: bool }
@geometry(vertices, indices, vertex_count, index_count, bounds)
resource PreparedMesh {
    vertices: buffer<PreparedVertex, read>
    indices: buffer<u32, read>
    vertex_count: u32
    index_count: u32
    bounds: WorldBounds
}
capability PreparedGeometry { mesh.prepared: PreparedMesh }
```

`StandardStyle` declares `optional capability PreparedGeometry`. Each renderer's
provider binds `mesh.prepared` to a real mesh node's `prepare` function and declares
`factories: preview_static`. The Forward paths use `preview_mesh.prepare`; Deferred
uses `opaque.prepare`. The pass declares `@prepare(PreparedMesh)` on a function
from its raw vertex interface to `PreparedVertex`. Its vertex entries consume
`PreparedVertex`, including the shadow entry. The preparation function applies
authored vertex changes and the engine's displacement before transforming positions
and normals into world space. The explicit view input remains `PreviewScene`.

An active resource argument demands preparation. The compiler emits a guarded
compute entry and makes both the base pass and contribution read its output.
Without demand, the base entry calls the same preparation function inline and no
preparation dispatch or allocation exists. Declaring an operation alone does not
create demand; a false static branch removes both the call and capability demand.

The current producer expands the selected index/vertex range into a local stream
with identity indices. Counts are range-local, raw indices are validated at upload,
and allocation sizes, storage strides, dispatch counts, and device limits are
checked. Repeated uses of a material or mesh retain separate object/range resources.
Each render refreshes preparation with that view's inputs before shadows and opaque
work; contributions retain explicit `at after_opaque`. This is engine-owned
preparation, not the phase 6 general compute scheduler. Its inputs must be available
before renderer work; dependencies on intermediate recipe outputs are rejected.

Phase 5 initially exposed unavailable bounds (`valid == false`) and required
`visibility: uncullable`. Phase 8 adds engine-provided conservative bounds and
host spatial policies, described below. Prepared draw vertex functions take
one `u32` vertex index and can return `clip_position`; indexed submission supplies
range-local indices. Test-only depth cannot enable depth writes, including in unused
definitions. Explicit `load_store` policies preserve both color and depth.

Artifact schema 10 reflects preparation programs, source pass identity, and typed
geometry binding roles. The host uses this metadata without recognizing Toon,
StandardStyle, or a renderer name. The legacy sample pipeline and `toon_shell` are
removed; phase 10 migrates its adapter regression coverage to operation calls.

### Phase 6 - Owned compute outputs and dependency scheduling

Depends on phase 4; use phase 5 as the completed draw acceptance baseline.

Status: complete. Host arithmetic is implemented in
`integrations/example-engine/src/runtime/compute_plan.rs` and used by prepared
geometry allocation. Compute declarations and graph result bindings now parse,
and definition contracts, kernel bodies, and graph handle scope are checked.
Kernel validation lowers bindings and inserts logical-dispatch bounds guards.
Selected calls now emit material-owned kernels and schema-v11 invocation metadata,
including captured settings, engine resources, geometry providers, output layouts,
host expressions, and data versus allocation dependencies. The host can evaluate
these plans and reject invalid work before allocation. A one-shot GPU executor
now allocates fresh outputs and executes reflected kernels; a dependency planner
separates allocation order from GPU data dependencies. The local `owned_compute`
example verifies real buffer/image chains, query-before-producer execution,
rounded/empty work, and independent allocations. Renderer integration creates
fresh outputs per frame and object, resolves
captured settings and prepared geometry, and binds returned resources into draws.
A local GPU probe verifies buffer/image consumption and live settings in all
three renderers, including isolation across views, queued frames, shared-material
objects and ranges, and empty work. The acceptance checks below are complete.

Draw calls now lower returned compute handles to read-only buffer/image bindings,
with producer identities retained on each draw invocation. Logical `count`,
`width`, and `height` members use typed compiler-owned bindings, including when
the resource expression is parenthesized. Reusing a handle in multiple draws
does not turn it into a renderer-global resource. The renderer resolves these
bindings from the current invocation output map,
including logical dimensions for empty allocations.

Image-format semantics belong to the shared resource-type registry in
`fresco-artifact::types`, not to compute-specific string allowlists or WGSL's
supported subset. The registry describes scalar types, channel counts, known
texel sizes, and language-level resource uses. Its storage formats map
exhaustively to Naga IR; compiler resource checking and engine format conversion
use that same source. Backend lowering support and the selected device's enabled
features, usages, and allocation limits are separate checks: a registered type
does not promise that every target device can execute every use of it. New
resource features should extend these shared type facts rather than add local
format lists.

- [x] Add shared host checks for storage stride/byte size, buffer and binding
  limits, workgroup dimensions/product, rounded dispatch counts and invocation-ID
  overflow. Preserve logical extents separately from empty-allocation sentinels.
- [x] Apply these checks to geometry preparation before GPU allocation, including
  fallible host allocation of its index data.
- [x] Parse `compute`, typed `output`, `workgroup_size`, `dispatch threads`, resource
  returns, and graph `let` calls without instantiating unused definitions.
- [x] Check allocation/return type agreement, access qualifiers, host-expression
  inputs, extent dimensionality, constant workgroup sizes, and precondition types.
  Check graph result types and lexical scope even in inactive static branches.
- [x] Typecheck unused kernel bodies and generate logical-dispatch bounds guards.
- [x] Share image-format metadata and Naga storage-format mappings across compute
  checking, contract attachments, renderer resource declarations, and engine conversion.
- [x] Retain the producing call identity on graph resource handles across static
  expansion and repeated operation calls; inactive branches allocate no identities.
- [x] Lower explicit compute binding provenance and classify actual shader reads
  separately as resource data, logical dimensions, or captured values. Include
  reads through helpers; unused inputs do not create shader-read dependencies.
  Bound output handles establish allocation dependencies even for unused bindings;
  only actual shader data reads establish compute-producer execution dependencies.
- [x] Preserve Naga resource-query uses separately from data reads in invocation
  dependencies, including image dimension queries and buffer length queries.
- [x] Lower selected calls and reflect resource producers and invocation metadata
  into artifacts.
- [x] Evaluate typed host preconditions, allocation extents, and logical dispatch
  expressions with checked arithmetic. Preserve integer precision, require explicit
  conversion of fractional extents, and check reflected layouts and a supplied
  remaining transient-byte budget before allocation.
- [x] Activate prepared geometry for compute-only consumers, scoped to the material
  that owns the invocation, without adding the opaque draw's completion edge.
- [x] Execute resolved compute kernels with fresh owned buffers/images, reflected
  bindings, logical dispatch uniforms, device format checks, and one-shot encoding.
  Retained handles and command buffers keep resources alive; there is no allocation
  pool or premature reuse. Verify real results with the local-only command
  `cargo run -p fresco-example-engine --example owned_compute`.
- [x] Derive compute allocation order and GPU readiness separately, validate
  declared edges against shader data reads, reject missing/foreign handles and
  cycles, and retain explicit prepared-geometry and engine-node prerequisites.
  Verify query-only consumers can execute before their producer's GPU writes.
- [x] Plan all owned allocations against one remaining byte budget and derive
  consumer dimensions from producer allocation metadata. Preserve logical zero
  dimensions even when the physical allocation uses a sentinel.
- [x] Lower compute-to-draw resource arguments and logical dimensions to typed
  bindings, retaining their producer identity per draw invocation. Validate
  repeated handle use and emitted shaders in Forward, Forward+, and Deferred;
  check resource types and capture shadowing in unused draw definitions too.
- [x] Connect allocation plans to authored compute outputs and per-frame invocation
  resources, and validate image formats/access and allocation budgets there.
- [x] Resolve returned buffers/images and logical dimensions into scene draw bindings.
  Verify live settings, empty resources/ranges, view changes, queued frames, reversed
  object/range order, and atomic budget rejection in all three renderers with
  `cargo run -p fresco-example-engine --example offscreen -- --compute-only`.
- [x] Use the same resource-readiness scheduler in the scene renderer and CPU
  tests. Prove prepared work and its consumers run before opaque completion;
  completion identities remain object-local and reset for every frame/view.

- [x] Implement `compute` definitions/calls, typed `output` allocation, resource
  return handles, buffer/image access qualifiers, and logical dispatch extents.
- [x] Derive producer/consumer dependencies and synchronization from actual
  resource uses, including compute-to-compute and compute-to-draw connections.
- [x] Keep compute that only needs prepared geometry/view/settings free of an
  opaque-completion edge even when its eventual draw uses `at after_opaque`.
- [x] Isolate transient outputs per invocation, frame, and view; reuse allocations
  only after GPU completion, never merely because materials or style symbols match.
- [x] Check extent/byte arithmetic, capacity and device limits, rounded dispatches,
  zero-work cases, and invalid allocations on the host; require shader bounds guards.
- [x] Validate initialization and access hazards. Keep persistent history and host
  readback explicitly outside this initial operation model.
- [x] Add an independent compute-assisted fixture proving returned data is consumed,
  scheduling is early enough, and different invocations cannot alias outputs.

Validation: `cargo xtask ci` passed with no Clippy warnings; the focused suite
includes 11 owned-compute integration tests, kernel/type/access tests, and provider
initialization regressions. Both local GPU probes passed, including the scene
fixture in all three renderers. All 216 browser unit tests and rebuilt compiler/
renderer Wasm smoke tests passed. Language-reference artifacts were regenerated.
The complete native offscreen suite also passed during renderer integration.

The provider check requires an unconditional initializer for internal resources,
even when a style compute call is their first consumer outside the recipe. Captured
settings must match the owning material's declared name, type, and offset. Persistent
history, host readback, and shading-hook captures remain outside this phase; the
next phase adds the typed shading slots and their dependencies.

### Phase 7 - Resources captured by shading hooks

Depends on phases 3 and 6.

Phase 7 is complete. Capture lowering and all three consuming renderer paths,
typed capability providers, and sampler provisioning are implemented. The
acceptance audit, local GPU probes, strict workspace gate, and browser builds
and tests pass.

Completed infrastructure:

- [x] Parse typed `shading_input` declarations and explicit graph bindings;
  validate draw scope, read access, names, resource types, and active binding
  uniqueness. Inactive branches retain lexical/type checking without producing work.
- [x] Resolve bindings to their compute producer ordinals and diagnose missing
  bindings for selected styles. Keep shading-slot names distinct from graph locals,
  loop variables, and integration targets.
- [x] Lower typed image `sample_level(sampler, uv, lod)` and
  `sample_grad(sampler, uv, dx, dy)` in the typed GPU function linker. Resolve image formats
  through the shared resource registry and validate argument types. Naga tests
  verify explicit level/gradient operations and reject invalid access and arguments.
- [x] Register resource helper signatures through the shared resource type parser
  and check their GPU bodies even when unused. Explicit buffer arguments specialize
  helpers by bound resource; they do not rely on unsupported storage-pointer function
  arguments. Public-pipeline tests verify independent bindings, actual shader reads,
  explicit sampling, and rejection of implicit resource captures.
- [x] Check declared shading-slot types and hook bodies using that resource-aware
  path, including unselected styles. Preserve read access through helper calls.
- [x] Enforce derivative-free hook effects through resolved helper calls, including
  defaults, unused styles, and helpers previously emitted for unrestricted callers.
  Explicit LOD and supplied gradients remain permitted.
- [x] Preserve resolved dispatch cases and the authored fallback as a rebuildable
  plan. Rebinding a case replaces generated branches rather than accumulating them.

Runtime wiring: mesh-pass artifacts carry `shading_inputs`
separately from contributed-operation inputs. The mesh runtime routes both through
the same owned-output type checks, reflected read dependencies, and per-object
frame bindings, rejecting overlapping bindings. Draw-scoped compiler lowering
now populates the field from selected style graph bindings and appends explicit
resource arguments to the selected hook implementation. Each generated
capture binding is namespaced by contract, implementation ID, and slot name;
contract-local IDs and identical slot names cannot alias another contract's
owned output or sampler. An owning-stage regression verifies both kinds of
binding remain independent across two contracts.
A Forward/Forward+
compiler fixture verifies a fragment entry reads the captured compute output and
that its producer has no opaque-completion dependency. The local
`offscreen --shading-only` GPU probe also verifies buffer/image reads in base
shading, live settings, view changes, deterministic and queued frames, reordered
draw ranges, and independent objects sharing a mesh/material in Forward,
Forward+, and Deferred. It also queues two independently targeted views with different view
inputs and settings in both submission orders, comparing each result to its
isolated reference. The views share material and producer symbols; captured
outputs remain independent.

Draw identity infrastructure now includes the typed engine binding
`@draw_data(instance_id) name: uniform<u32>`. The runtime assigns a nonzero index
to each object/material-range invocation within a rendered view and allocates
fresh uniform storage per frame. It does not reuse the material-table ID.
The local GPU probe verifies two draws of one material get distinct IDs and two
queued views retain independent identity uniforms. Deferred now stores material
and draw-instance IDs in separate `rg32uint` channels, and surface UVs in
`rg32float`. The `offscreen --deferred-only` GPU probe verifies the identities
remain distinct and a high-frequency UV lookup agrees across all three renderers
within one output byte. A deliberate `rg16float` mutation fails that precision
check. The G-buffer requires 40 color-attachment bytes per sample; hosts request
the adapter's supported budget and recipe validation rejects insufficient limits
before pipeline creation. Deferred now resolves each draw instance separately,
binding its actual owned outputs and discarding pixels with a different stored
instance ID. This portable implementation requires no resource descriptor arrays.

Engine dispatch integration: `@dispatch(Contract, method,
settings_buffer, draw)` explicitly limits a mesh dispatcher to the current
material's selected implementation. Reachable calls require one matching selection;
unused dispatchers in a shared engine pass impose no selection requirement. It keeps
the authored fallback for other selector values. Without `draw`, dispatch remains
dynamic across the complete implementation table. This distinction permits
per-draw resource binding without giving unrelated implementations dummy resources;
specialization is explicit for per-instance procedural draws as well. This engine ABI
does not replace the per-object/view shading-instance identity required below.

The bundled mesh hooks and Deferred resolve now use draw-scoped dispatch.
`@draw(vertex, fragment, instance, count)` runs a procedural draw for each range,
with factory resources and captured outputs but without its mesh vertex stream.
The compiler and host reject mismatched mesh/instance vertex sources. Deferred
clears lighting once, waits for all opaque geometry, and preserves lighting across
instance resolves; its completion boundary waits for every instance before
contributions and transparency. Compute dependencies attach to each consuming
resolve without delaying independent producers until opaque completion.
Unrestricted dynamic dispatch with captures still diagnoses unsupported
shading-instance lowering.

Sampler captures use `shading_input filtering: sampler scope draw` and an explicit
`bind shading.filtering = nearest_repeat` inside `for self`. The immutable standard
values are `nearest_repeat`, `nearest_clamp`, `linear_repeat`, and `linear_clamp`.
Shader hooks receive the declared slot as an ordinary typed sampler argument;
they do not capture an implicit engine sampler. Factory bindings can also declare
`@sampler(nearest_repeat) filtering: sampler`. Filtering and addressing are typed
artifact metadata, and image filterability comes from the actual format and
enabled device features. Invalid presets, conflicting sources, shadowed values,
missing bindings, and non-sampler declarations are rejected. GPU probes verify
all four behaviors with both explicit LOD and explicit gradients in every renderer.
A combined GPU probe samples the captured compute image with varying surface UVs,
compares all renderer outputs within one display-output byte, and proves that
reducing Deferred UV storage to `rg16float` breaks that tolerance.

All three renderers provide `SurfaceUV` and `DrawShadingResources` using checked
sources. `shader_output(node.entry.member)` must name an invoked raster entry's
actual typed output; fragment sources must have a compatible stored attachment.
`draw_data(node.binding)` must name an exclusive `@draw_data(instance_id)` binding
on a mesh or instance node. Its capability type is `draw_instance_id`, distinct
from an ordinary `u32` even though its GPU representation is `uniform<u32>`.
Material-table bindings cannot satisfy this type. Requirements support comma
lists, such as `requires SurfaceUV, DrawShadingResources`; missing selected
capabilities and invalid unused provider declarations are diagnosed.

- [x] Implement typed `shading_input` slots and `bind shading.name = value`, with
  producer dependencies attached to every consuming shading path.
- [x] Add draw-scoped shading-instance records distinct from material settings
  records, preserving object/range/view identity when materials are shared.
- [x] Provide SurfaceUV and DrawShadingResources where the renderer can honor them;
  reject missing capabilities rather than substituting material IDs or dummy data.
- [x] Extend Deferred to preserve shading-instance identity and lookup coordinates;
  define coordinate precision and validate equivalent Forward/Forward+/Deferred lookup.
- [x] Enforce supported explicit-LOD/gradient sampling and derivative restrictions
  for dynamically dispatched hooks.
- [x] Verify different compute outputs affect different objects sharing a material,
  including multiple views and invalid/missing resource bindings.

Acceptance evidence:

| Requirement | Executable coverage |
| --- | --- |
| Typed slots, explicit bindings, and producer identity | Core `style_operations` and `style_tests` tests; example-engine `owned_compute::draw_scoped_shading_captures_owned_compute_output_in_all_renderers` checks actual Naga resource reads and early compute dependencies. |
| Independent contract bindings | Core `implementations::captures_with_equal_local_ids_and_slots_remain_independent_across_contracts` checks separate producer handles, sampler presets, and hook arguments. |
| Draw/range/view identity | `offscreen --shading-only` checks shared-material objects, reordered ranges, and independently queued views against isolated references. |
| Typed renderer capabilities | Example-engine `owned_compute` tests reject missing capabilities, invalid shader outputs, and material-table substitutions for draw-instance identity. |
| Deferred lookup fidelity | `offscreen --shading-only` compares captured-image UV lookups across all three renderers within one display-output byte; a half-precision UV mutation must fail that tolerance. `offscreen --deferred-only` also checks integer IDs and background behavior. |
| Explicit sampling and effects | Core resource-linker and shading-hook tests check explicit levels/gradients, invalid arguments, implicit captures, and derivative effects through helpers/defaults/unused styles; `offscreen --shading-only` exercises all four sampler presets in all renderers. |

These GPU probes are local acceptance checks, not CI jobs. Browser interaction
and mixed-style GPU acceptance remain in phases 8-9.

Completion validation (2026-09-22): `cargo xtask ci-strict`, local
`offscreen --shading-only` and `offscreen --deferred-only`, release
`npm run wasm:build` (compiler and renderer smoke tests), `npm run test:unit`
(216 tests), and `npm run typecheck` passed. The generated language reference
was regenerated with `cargo xtask lang-docs`.

### Phase 8 - Global transparency and MeadowFur acceptance

Depends on phases 5-7.

Phase 8 is implemented and verified. Draw operations accept explicit `sampler`
parameters supplied by immutable standard sampler values. They reuse the typed
sampler provisioning from Phase 7 without inventing compute producers for
samplers. Regression coverage checks separate presets per invocation and rejects
unknown, non-sampler, and shadowed arguments. The full shell pipeline, lighting
service, and mixed-scene GPU acceptance are implemented. Validation passed with
`cargo xtask ci-strict`, the release compiler/renderer WASM build and smoke tests,
web typechecking, and all 216 web unit tests. Local offscreen GPU probes passed in
Forward, Forward+, and Deferred for generated geometry/bounds, global transparency,
lighting/shadows, full MeadowFur mutations, and mixed Toon/fur/opaque/glass scenes.

The engine recipe now has an explicit `@transparent_queue(name)` draw marker.
All three bundled renderers mark ordinary transparency with it, emitting
premultiplied color and disabling depth writes. Compiler and host share queue
validation: members must form one contiguous boundary, preserve the same color
and depth attachments, and have no internal node-order dependencies. Runtime
ordering merges member nodes by view depth, draw identity, and local ordinal.
Behavioral tests exercise ordinary and contributed nodes together and reject
incompatible queue declarations and raster state. Typed `global_transparent_queue`
providers bind a named renderer queue and explicit incoming/outgoing completion
phases. Contributions inherit the queue's external dependencies, including opaque
style draws, and all downstream consumers wait for the complete queue. Tests
reverse style declaration order to ensure this does not depend on installation
order. A material may contribute alongside its ordinary transparent base draw.
Compatible integration points can bind the same actual queue: its global sort
provides their shared writer order, independent of point names.
The local `offscreen --transparency-only` GPU probe interleaves ordinary and
contributed transparency at swapped view depths, reverses primary ownership, and
compares all three renderers. The full MeadowFur and mixed-scene probes below also
pass in all three renderers.
Queue semantics use artifact schema version 12, so older hosts reject these
manifests instead of silently ignoring cross-node ordering.

Generated vertex substitution is implemented as
`raster geometry using vertices base_vertex expression`. The source must be
prepared geometry and the replacement must be an owned read-only buffer output.
Each frame retains its own generated draw geometry, sharing the prepared local
indices. Host evaluation checks arithmetic, logical buffer capacity, and the
signed base-vertex limit before submission. The `generated-shells.fr` fixture
creates three compute-written shells; the local `offscreen --generated-only`
probe verifies distinct coverage, repeatable fresh frames, and rejected offsets
in all three renderers. The fixture also uses
`visibility: geometry.bounds.expand_world(expansion)` and
`sort_position: geometry.bounds.center`. Current settings are captured before
visibility decisions; negative/nonfinite expansion and unavailable explicit bounds
are errors. Ordinary transparency uses engine range-center depth when available;
contributions may select their prepared bounds center explicitly.

The bundled `preview_static` factory derives a source AABB for the selected
index/vertex range, expands it for the engine's sinusoidal displacement, and
transforms it conservatively into world space. Authored vertex stages and custom
factories do not inherit that bound: their host must call `set_prepared_bounds`
with conservative world bounds before rendering, or use uncullable contributions
without an explicit bounds-center sort position. The engine trusts supplied bounds;
it cannot infer arbitrary shader displacement. Unavailable bounds remain marked
`valid == false`. Compute kernels, raster shaders, visibility, and sorting receive
the same bounds for the current object/view. Source vec3 streams and indices are
retained on the CPU to compute bounds for independently selected material ranges.

The GPU probe places a source range outside the frustum and moves generated shells
into view. Zero expansion culls them; updated expansion admits their pixels. A
second material range sharing the upload does not enlarge the selected bound.
Another view recomputes visibility. Culled preserving nodes retain their incoming
graph dependencies and cannot release consumers before those inputs complete.

Draw `requires` expressions can also refer to captured scalar inputs and logical
resource dimensions. These are checked on the host for each active invocation,
before GPU submission, using the same checked arithmetic as compute allocation.
Material/capability requirements remain compile-time checks. Shader data cannot
be read to decide host preconditions; unused declarations are typechecked too.

- [x] Implement the ForwardLighting service using the engine's actual lights,
  visibility, and indirect inputs; expose its typed bindings through providers.
  - [x] Check explicit `@service(Interface)` pass exports against nominal interface
    method signatures, including unused exports and ambiguous implementations.
  - [x] Check reachable local/imported helper bodies, explicit binding captures,
    return paths, and recursive call rejection before service selection.
  - [x] Link callable service methods and their resource dependencies through
    typed provider bindings and operation arguments.
  - [x] Supply the real engine lighting service and allocation-free applicable-light
    iteration through all three renderer providers; compile and Naga-validate consumers.
  - [x] Verify light/shadow mutations
    affect contributed shell shading in every renderer.
- [x] Implement the transparent integration point as one global queue combining
  ordinary transparent draws and all style contributions.
- [x] Sort by view depth, stable draw identity, and local ordinal; validate
  premultiplied blending, preserved color, depth testing, and contribution bounds.
- [x] Implement generated vertex stream substitution, checked per-shell offsets,
  and bounds updates before visibility decisions.
- [x] Make the full MeadowFur example executable: density compute, shell-vertex
  compute, density used by base shading, and multiple transparent shell draws.
  The executable source is `examples/40) surface shaders/style_sample_fur.fr`.
  `offscreen --fur-only` verifies time, density, tint, shell opacity, and fresh-frame
  mutations across forward, Forward+, and deferred rendering.
  Explicit field arguments such as `time: frame.time` and `tint: fur_tint.rgb`
  capture the complete typed uniform/setting and select the field in shader code.
  Unknown fields, invalid swizzles, and projected values used for host allocations,
  dispatch extents, bounds, or preconditions are rejected. No host readback or
  unaligned scalar uniform binding is implied. Graph `let` also accepts compile-time
  values derived from static settings and loop variables, preserving lexical scope
  and integer types; runtime settings and shader values cannot construct the graph.
- [x] Verify Toon, fur, ordinary opaque materials, and ordinary transparency together,
  including interleaved styles, shared materials, multiple views, and all renderers.
  `offscreen --mixed-styles-only` compares shared fur instances to isolated draws,
  interleaves Toon/fur/opaque/glass submissions, splits one mesh between materials,
  and checks independent views and restored frames in all three renderers.
  Material and explicit pass resources are assembled per entry-point layout:
  Deferred resolve textures may reuse slots occupied by material uniforms in
  other entries. Compiler validation rejects an unused pass declaration that
  shadows a live material/external resource in the same entry; it uses Naga source
  ownership and actual entry uses rather than naming conventions.
- [x] Document tested limits for shell intersections, sorting, shadow participation,
  and supported factories; do not present procedural shell fur as strand simulation.

Phase 8 acceptance limits:

- Shell fur is a decorative surface approximation, not strand simulation. Each
  frame generates fresh shell vertices; there is no persistent simulation history.
- Transparency sorts whole draw ranges by their view-space center, then stable
  identity and local ordinal. This provides deterministic composition, not exact
  per-pixel ordering for intersecting shells or intersecting transparent objects.
  The transparency probe checks ordering and stability; it does not establish
  order-independent transparency.
- Contributed shells receive the engine's actual light visibility and shadows.
  The lighting-service probe checks shadow mutations and preserved ambient light.
  Generated shells do not add shadow-caster passes or fur self-shadowing; the base
  material retains its ordinary shadow participation.
- The bundled providers support `preview_static`. Other factories need compatible
  provider declarations and valid prepared geometry. Authored vertex displacement
  requires conservative host-supplied bounds, as described above. The generated
  geometry probe checks expanded bounds, selected ranges, and changed views.
- The sample restricts the base surface to opaque, one-sided materials, shell
  counts to 1–32, and density dimensions to 1–512. Its conservative world-space
  expansion is twice the fur length. Light iteration scans the engine's 64 point
  light slots, including in Forward+; it does not reuse tiled light acceleration.
- Projected arguments are shader values and cannot control host allocation,
  dispatch, bounds, or invocation preconditions. Raster iterators are pass-local,
  expand at direct `for` uses, and finish by falling through; generator early
  return/break and stored or first-class iterator values are unsupported.

### Phase 9 - Capability reflection and editor completion

Completed 2026-09-23 against both Toon and MeadowFur acceptance cases.

- [x] Reflect provider/capability availability and unsupported-selection reasons
  for the selected renderer, material schema, factory, and settings specialization.
- [x] Keep unsupported selections visible and recoverable in the editor; never
  silently replace a requested style with a default.
- [x] Preserve valid symbolic assignments through renderer changes and static
  rebuilds; failed candidates must leave the previous working render installed.
- [x] Verify imported external styles require neither a central registry entry nor
  a style-specific compiler, native host, or browser branch.
- [x] Run browser interaction/GPU coverage for style choice, static rebuilds,
  resource-backed hooks, and mixed scenes, as well as native coverage.

Implementation and validation:

- `ManifestImplementationSelection.available` remains the declaration catalog.
  Its additive `availability` records identify the renderer pipeline, schema,
  factory domain, provider presence, capability/integration-point support,
  static specialization, and diagnostic reasons. Provider/factory checks and
  candidate graph expansion share the actual compilation paths; probes cannot
  add work to the installed graph.
- The selected symbol is checked with its selected static settings. Alternatives
  are checked with their declared defaults. Optional unavailable capabilities do
  not reject a style unless its active graph requires them. This describes typed
  graph compatibility, not device allocation limits or runtime preparation.
- The editor retains unsupported symbols, displays compiler reasons, and exposes
  alternative static settings for recovery. Overrides are keyed by document,
  material, and property, using symbols rather than dispatch IDs. Turning an
  example into custom source no longer resets its style settings. Failed compile
  and preparation candidates leave the working preview visible; explicit document
  switches still clear it.
- Native `bundle` regressions prove default versus selected specialization,
  inactive requirements, unused capability factory restrictions, diagnostic parity
  with compilation, imported renamed styles, and absence of probe-generated work.
- `web/tests/gpu/style-selection.spec.mjs` passes all four browser cases: imported
  renamed Toon and compute-backed fur in Forward, Forward+, and Deferred, plus
  actual editor selection/static/renderer changes and recovery. Pixel checks prove
  outline removal, returned compute data consumption, mixed scenes, and unchanged
  output after a failed candidate. No style-specific host registry was added.
- Fresh native `offscreen --fur-only` and `--mixed-styles-only` probes pass on all
  three renderers, including shared materials, mesh ranges, independent views,
  time/density/tint changes, and ordinary/contributed transparency.
- Validation passed: `cargo xtask ci-strict`, generated TypeScript contract export,
  `npm run typecheck`, 223 browser unit tests, both development WASM builds and
  their smoke tests, and the browser/native GPU acceptance above. GPU execution
  remains local-only. The WASM build wrapper now gets resolved package metadata
  from Cargo, avoiding wasm-pack's rejection of inherited repository metadata.

### Phase 10 - Retire legacy style paths and close the migration

Retire each path only after its replacement and equivalent regression coverage land.
General-purpose compiler/engine mechanisms are not legacy merely because styles use them.

- [x] Replace `@stage(mesh_color, ...)` / `@stage_input(...)` style integration in
  `engine/config/renderer.fr` with the checked contract/provider path from phase 3.
- [x] Migrate all style uses of `@contribute`, including independent-engine fixtures
  and tests, to operation calls. Preserve scope, attachment, diagnostics, and ordering assertions.
- [x] Remove obsolete style-contribution parsing/lowering in
  `driver/contributions.rs`, or extract genuinely shared graph logic before removal;
  do not leave a second supported style composition path by accident.
- [x] Audit staged `fwd_base` / `fp_shade` examples and duplicated shading helpers.
  Migrate supported standard-material consumers; remove obsolete duplication or
  clearly identify intentionally separate staged demonstrations.
- [x] Audit particle, unlit, and custom-schema response paths. Migrate compatible
  consumers or explicitly document their separate contracts; do not force every schema
  into StandardStyle or claim compatibility they do not implement.
- [x] Audit legacy style-profile configuration and browser fallback controls;
  remove overlapping selection paths after migrating their remaining consumers.
- [x] Retain ordinary interfaces, conformances, generic implementation dispatch,
  engine passes, and renderer pipelines where they remain useful lower-level mechanisms.
  Record intentional retention so cleanup does not become wholesale API deletion.
- [x] Publish migration diagnostics/guidance for removed authoring syntax and
  persisted configuration; remove obsolete examples and contradictory documentation.
- [x] Regenerate language/reference artifacts, update `LANGUAGE.md` and relevant TODOs, rebuild
  both WASM packages, and verify native/browser package schema compatibility.
- [x] Pass the relevant compiler/host tests, workspace CI, strict lint, native GPU
  acceptance, and browser acceptance. Keep GPU execution local-only, outside CI.

Phase 10 completed (2026-09-23). Authoring/fixture migration and acceptance are
complete. Evidence and the deliberately retained mechanisms are recorded below.

- All three executable renderer recipes already publish checked typed providers.
- `driver/operation_composition.rs` retains shared resource remapping, identity,
  attachment lifetime checks, and incoming/outgoing ordering. Generated operation
  metadata is private; authored legacy adapters and private metadata are rejected
  before graph expansion, including unused imported declarations.
- Independent-engine and unused-operation mutation tests preserve scope, ordering,
  stored-producer, attachment, and binding validation through operation calls.
- The no-op browser profile fallback is removed. Reflected implementation settings
  remain the style selection path; schema profiles remain a distinct engine API.
- Staged `fwd_base`/`fp_shade` demos, their BRDF examples, and particle/unlit/custom
  contracts are intentionally retained and identified in LANGUAGE.md. Ordinary
  interfaces, conformances, dispatch, passes, and renderer recipes remain useful.
- Migration guidance lives in LANGUAGE.md; malformed persisted configuration points
  to symbolic `property_overrides` rather than silently accepting obsolete fields.

Validation: `cargo xtask ci-strict` passed, including workspace tests, all-target
checking, strict Clippy, formatting, and rustdoc. The final strengthened migration
and unused-operation diagnostics passed targeted tests; the final native probe
changes passed strict example Clippy. TypeScript and all 223 browser unit tests
passed. `cargo xtask lang-docs` regenerated both references; both shared-contract
export tests and rebuilt compiler/renderer WASM package smoke tests passed.
All four `style-selection.spec.mjs` browser GPU/editor tests passed, including
legacy-syntax rejection and retained pixels after rejected candidates.

Local RTX 2080 Ti acceptance covered compute/shading captures, operation range
identity, transparent ordering, lighting services, Fur, mixed styles, generated
and prepared geometry, environment, independent operation reuse, Toon, settings,
and renderer parity in the broad native run. That run exposed two stale probes:
environment readback still used one word for the two-word identity texture, and
the alternate factory omitted the now-required draw-instance input. Both fixtures
were updated without relaxing assertions. The subsequent `--baseline-only` run
passed all baseline checks through factory variants, particles, and intermediate
targets; the default invocation still runs every probe. GPU execution remains
local-only, outside CI.

### Phase 11 - Resource-driven placement after styles

Depends on phases 3-10. This is a concrete follow-up, not a shortcut around their contracts.

Implemented: a point may expose its typed attachment members with
`port: opaque`. A draw inside `for self` can pass `opaque.color` and `opaque.depth`
without an `at` block; those arguments select one checked engine boundary. The
provider must prove the position of every reader and writer (including storage
bindings) and order conflicting accesses, in addition to the
existing initialization, format, preservation, and downstream-consumer checks.
Explicit points without exposed ports remain supported. Compute dependencies
continue to come from their actual inputs, independently of a later draw's port.

Renderer manifests now expose `resource_ports`: each attachment has an incoming
version, preserving operation versions, and the direct downstream consumers of
the exported version. Test-only depth retains its incoming version. Conditional
operations preserve the preceding version when skipped; queued writers form one
runtime-sorted version group. The redundant single-anchor metadata has been
removed; complete incoming and outgoing boundary sets own ordering. Adversarial,
all-renderer pixel, and migration validation passed.

- [x] Define semantic ports/versions for completed opaque color and depth, including
  how operation outputs become the versions consumed downstream.
- [x] Infer placement only when both incoming dependencies and outgoing ordering
  before transparency, inspection, and presentation are proven.
- [x] Generalize writer composition and conflict diagnostics without relying on
  source/import order or compiler-known renderer names.
- [x] Prove equivalent graphs/pixels for explicit `at after_opaque` and inferred
  placement across all renderers, with multiple contributors and independent early compute.
- [x] Test missing downstream connections, ambiguous writers, and cycles; keep
  explicit placement where resource flow cannot prove the required ordering.
- [x] Simplify examples only after equivalence is established; then retire any
  redundant placement machinery and update the language/migration documentation.

Phase 11 acceptance coverage:

- `driver::style_graph` and `driver::resource_ports` exercise domain-neutral
  contracts, missing consumers, unordered readers/storage writers, ambiguous
  boundaries, cycles, port shadowing, attachment aliasing, and import-order
  independence. Explicit placement remains available without an exposed port.
- `integrations/example-engine/tests/resource_placement.rs` compares shaders,
  executable graphs, resource versions, and early compute readiness in all three
  renderers. It also checks transparent queue versions and the migrated Toon sample.
- `offscreen --placement-only` passed exact explicit/inferred pixel comparisons
  in Forward, Forward+, and Deferred on an NVIDIA GeForce RTX 2080 Ti. The probe
  includes multiple contributors, repeated material draw ranges, and changes to
  compute-returned data that visibly change the rendered image.
- The compiler and renderer WASM packages built successfully; browser typechecking
  and all 223 unit/integration tests passed against the rebuilt compiler.
- `cargo xtask ci-strict` passed on 2026-09-23: repository hygiene, README
  samples, formatting, workspace checking/Clippy, rustdoc, and workspace tests
  (including generated WASM contract exports).

## Proposed syntax: a complete style using compute and draw operations

This is the proposed author-facing shape, not executable Fresco today. The
lighting is simplified to show the hooks. `StandardSurface`, `DirectLight`,
`LightingResult`, prepared geometry, and the integration point are supplied by
the engine contract; they are not compiler-defined rendering vocabulary.

```text
style Toon for standard : StandardStyle {
    param outline_width: f32 = 2.5
    param outline_color: color = #2e1938

    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 {
        let nl = max(dot(surface.normal, light.direction), 0.0)
        let band = step(0.5, nl)
        return surface.albedo * band
             * light.radiance * light.attenuation * light.visibility
    }

    fn finish(surface: StandardSurface, context: ShadingContext, lighting: LightingResult) -> vec3 {
        return lighting.direct + lighting.indirect + surface.emissive
    }

    for self {
        let offsets = OutlineOffsets(
            geometry: mesh.prepared,
            view: view,
            width: outline_width
        )

        at after_opaque as target {
            OutlineShell(
                geometry: mesh.prepared,
                view: view,
                offsets: offsets,
                ink: outline_color,
                color: target.color,
                depth: target.depth
            )
        }
    }
}

surface style_sample(sp: surf) -> material(standard) {
    properties { style: Toon }

    param tint: color = #f5ad69

    compose {
        base(albedo: tint, roughness: 0.45, metallic: 0.0)
    }
}
```

`OutlineOffsets(...)` and `OutlineShell(...)` are operation invocations. Their
separate `compute OutlineOffsets(...) { ... }` and `draw OutlineShell(...) { ... }`
declarations are shown under [Defining operations versus invoking
them](#defining-operations-versus-invoking-them). For the demo, definitions and
their shader bodies belong in the same `.fr` file as the style. The outline draw
definition must declare its opaque, single-sided material requirement. Omitted
shading hooks use only defaults explicitly supplied by the contract.

The compute call is outside `at after_opaque` deliberately: its inputs determine
when it can run. Only the draw needs completed opaque color and depth. See
[Scheduling follows dependencies](#scheduling-follows-dependencies) for resource
ownership and ordering, and its follow-up subsection for replacing explicit
placement with typed resource connections after the style work lands.

## What a style is

A style is a named declaration in an ordinary imported `.fr` module. It targets a
material schema and implements an engine-defined rendering contract. It can
provide both shading functions and a technique containing additional rendering
work. Toon is one user-authored example of this concept, not a language feature
or a member of an engine-maintained enum.

An existing `material(standard)` still computes standard material data. Someone
explicitly assigns a compatible style to that material. The style determines how
that data participates in shading and, when needed, adds work such as an
inverted-hull outline draw for each mesh using the material.

The important distinctions are:

| Concept | Owns |
| --- | --- |
| Material schema | The meaning and types of the composed values |
| Surface/material | Layers, textures, parameters, deformation, and the selected style instance |
| Style declaration | An implementation of a rendering contract, its settings, and optional technique contributions |
| Renderer | Pass scheduling, shared scene resources, light enumeration, and integration points |
| Lighting environment | Actual scene lights, ambient illumination, and shadow data |

A style can change rendering organization. It is therefore more than a BRDF
selector, although a BRDF-only style is a valid simple case.

## Ownership and extensibility

The engine defines contracts describing what an extension must supply and what
resources/insertion points the engine provides. A style author implements those
contracts in a separate file. Importing that file makes its declaration available;
it does not change existing material assignments.

The engine assigns its authored PBR style as the default for standard
materials. Selecting Toon is an explicit material/preview operation. Adding
`my_studio/ink.fr` must not require editing a central enum, Rust response switch,
browser option list, or compiler table of shading models.

The compiler knows typed implementations, symbolic references, specialization,
resource access, tables, and graph composition. It does not know what Toon, PBR,
a silhouette, a light, or a GBuffer is. Contract member names and graph insertion
point names are authored engine vocabulary, not reserved compiler hooks.

The shared runtime executes the compiled plan. It must not contain
`if style == Toon { draw_outline() }` or an equivalent check of a manifest label.

## First-class style declarations

A style owns its target schema, engine contract, settings, shading implementations,
resources, and additional rendering work in one checked declaration. Implementing
the contract must reveal both its shading hooks and available graph integration
points. Authors should not have to discover disconnected `@contribute` pipelines
to understand what completing a style entails.

The following sketches establish the proposed shape; the worked specification at
the bottom supplies concrete proposed operation bodies and contracts. Build on the existing implementation dispatch
and technique/recipe machinery rather than creating another executor.

```text
style Toon for standard : StandardStyle {
    param outline_width: f32 = 2.5
    param outline_color: color = #2e1938

    direct = toon_direct
    finish = toon_finish

    for self {
        at after_opaque as target {
            InvertedHull(
                geometry: mesh.prepared,
                view: view,
                width: outline_width,
                ink: outline_color,
                color: target.color,
                depth: target.depth
            )
        }
    }
}

surface style_sample(sp: surf) -> material(standard) {
    properties { style: Toon }
    compose {
        base(albedo: #f5ad69, roughness: 0.45, metallic: 0.0)
    }
}
```

`toon_direct` and `toon_finish` are ordinary authored shader functions with
contract-checked signatures; a style may also define its hooks inline. Omitted
hooks are legal only where the engine contract explicitly supplies defaults.
`InvertedHull` is an authored draw operation. For the demonstration, its definition
and shader bodies belong in `style_sample.fr` alongside Toon, so
the example shows the actual algorithm. It may later be imported and reused.
No compiler-provided outline implementation is implied.

Keep `properties { style: Toon }` as the simple assignment form. Assignment refers
to a declaration, not an enum or replacement material schema. Configurable
instances have checked named settings, spelled `Toon(outline_width: 3.0)` in the
worked specification below. There is one selected style instance per material initially. Layering
independently scheduling styles is a separate composition feature.

The compiler must diagnose unresolved declarations, incompatible schemas or
contracts, ambiguous implementations, invalid settings, and missing renderer
integration points. A selected style must not silently fall back to PBR or lose
its extra passes when the renderer changes.

## Defining operations versus invoking them

Definitions are declarations; uses are calls. Do not use `compute Name { ... }`
for both. Illustrative declarations, with operation bodies abbreviated:

```text
compute OutlineOffsets(
    geometry: PreparedMesh,
    view: View,
    width: f32
) -> buffer<vec4> {
    // Declare output allocation, dispatch sizing, and the compute shader body.
}

draw OutlineShell(
    geometry: PreparedMesh,
    view: View,
    offsets: buffer<vec4>,
    ink: color,
    color: attachment<rgba16float, preserve_update>,
    depth: attachment<depth32float, test_only>
) {
    // Declare attachment requirements and raster state, including front culling,
    // depth testing without depth writes, and replace blending.
    // Define vertex and fragment shaders; the vertex shader reads offsets.
}
```

Operation definitions own typed input/output and access contracts. Compute
definitions describe output extent, dispatch sizing, workgroup size, and bounds
handling. Draw definitions describe geometry indexing, resource reads, attachment
requirements, raster state, and shader entry points. A reusable technique may
compose operations and encapsulate intermediate resources behind typed inputs
and outputs. The worked specification below supplies their proposed declaration
syntax; compatibility with the existing grammar must be checked during implementation.

Calls in graph-building blocks instantiate GPU work. They are not shader function
calls, CPU execution, or synchronous GPU readback. Shader function calls remain
inside shader bodies; the checker distinguishes these contexts. A returned
`buffer<vec4>` represents a typed graph resource with a producer. Passing it to a
consumer establishes bindings and dependencies. Definitions declare access modes
once; callers should not repeat the operation's reads, writes, or dispatch math.

## Scheduling follows dependencies

Only work that needs an engine integration point belongs at that point. Computing
outline offsets need not wait for opaque lighting just because the consuming draw
must wait. The compute-assisted variant of Toon would contain:

```text
for self {
    let offsets = OutlineOffsets(
        geometry: mesh.prepared,
        view: view,
        width: outline_width
    )

    at after_opaque as target {
        OutlineShell(
            geometry: mesh.prepared,
            view: view,
            offsets: offsets,
            ink: outline_color,
            color: target.color,
            depth: target.depth
        )
    }
}
```

This is an optional example of compute-to-draw composition, not a requirement to
move the simple inverted-hull algorithm into compute.

```text
prepared geometry --> offset computation ---------+
                                                  +--> outline draw
completed opaque color and depth -----------------+
```

Source order does not impose a total command sequence. The compute operation is
eligible once its inputs are ready. The outline draw additionally waits for the
engine boundary and must complete before its downstream consumers. Independent
work may overlap where supported and beneficial; this does not promise an async
compute queue or require earliest-possible execution at the expense of resource
lifetime and memory usage.

The compiler composes producer/consumer edges, engine ordering constraints, and
resource hazards into one graph. It validates initialization, conditional producer
availability, resource extents, conflicting writers, and cycles. Attachment
updates need explicit version/order semantics; a shared resource name alone is
not enough to order multiple writers. The backend supplies required barriers and,
if multiple queues are used, queue synchronization.

`for self` scopes work and logical resources to each selected material draw range
of an object instance within a view and frame. Two meshes sharing a style must have separate geometry,
transforms, and intermediate contents. Physical allocations may be reused only
when lifetimes permit. Other scopes, such as per-view work, need explicit identity
and sharing rules and must not be inferred from the operation name.

`mesh.prepared` is an engine capability with an explicit producer, availability,
indexing, and deformation contract. Vertex-stage inputs are not automatically
compute-readable. The engine must expose compatible storage or a declared
preparation path, and preserve base-draw deformation and transforms. Unsupported
geometry factories fail with a capability diagnostic.

Compute outputs used by base shading use typed `shading_input` slots and explicit
`bind` declarations in the worked specification. An `after_opaque` producer cannot
feed an earlier shading consumer. Host readback
is a separate asynchronous host-facing operation, outside this initial design.

### Follow-up after styles: placement through typed resource access

The initial style implementation kept placement explicit until the typed contract,
resource, and writer-composition rules were established. Phase 11 now exposes a
point's attachment members through an engine-authored binding (`port: opaque`).
A draw's `opaque.color` and `opaque.depth` arguments select the checked boundary.
The compiler does not recognize opaque rendering, Toon, or renderer names.

These are semantic versions with declared incoming completion and outgoing
consumers. They are not inferred from texture names or formats. Preserving color
updates form an ordered version chain; test-only depth remains the same version.
The final color version flows to the downstream access frontier, after which
ordinary renderer passes produce their own results. A sorted transparent queue
is one runtime-composed version group, including ordinary and contributed draws.
The manifest records version producers/readers and their common physical resource.

Provider checks reject missing downstream consumers, unordered readers, ambiguous
writers, and cyclic flow. No source/import-order fallback supplies missing edges.
Explicit `at` remains valid, including for points without an exposed port. Do not
remove a point's obligations when removing its call-site `at` block: the engine
must still prove completion before transparency, inspection, and presentation.
Independent compute retains only its own resource dependencies.

The Phase 11 CPU fixture compares explicit/inferred graphs and shaders across all
three renderers, including multiple contributors and ready-before-opaque compute.
The local `offscreen --placement-only` probe compares exact pixels and verifies
that returned compute data changes the image. See the Phase 11 checklist for
acceptance status and the language reference for current author-facing syntax.

## Engine contracts, not a universal lighting interface

The example engine can define a `StandardStyle` contract with prepared
standard values, shading context, incident-light inputs, and direct/ambient
response functions. These are ordinary engine records and methods. Another
engine can declare a completely different contract.

The revised contract should expose structured prepared-surface, direct-light, and
accumulated-lighting inputs. Its declared sequence is prepared surface, per-light
direct response, indirect response, lighting accumulation, then final surface
color. The engine owns light enumeration and shadow sampling, while the contract
specifies exactly where radiance, attenuation, and visibility are applied. This
allows a style to distinguish banding angular response, shadowed illumination,
and accumulated lighting without accidentally applying factors twice.

A `finish` hook chooses final linear HDR surface color before transparency
composition and scene tone mapping. It does not grant arbitrary access to final
scene pixels. Defaults, required hooks, available fields, and alpha policy must
be explicit. Every renderer claiming compatibility must supply the promised data;
Deferred must preserve it in its encoding or provide a checked alternate path.

The implemented baseline is narrower and currently behaves as follows:

- Material composition and normal mapping produce the prepared standard values.
- Forward and Forward+ supply actual scene light samples to the selected response.
- Deferred reconstructs those values and invokes the same response implementation.
- The engine owns light enumeration, attenuation, filtered shadow visibility,
  ambient inputs, emission accumulation, and final alpha policy. The contract
  explicitly says which factors are already applied so they are never doubled.
- Toon bands quantize angular response; changing a light's distance or intensity
  still changes its contribution. Ambient fill remains visible in shadow.
- Style settings control bands and stylized highlights; existing roughness,
  metallic, normal, occlusion, and emissive data remain meaningful.

Do not retain three independently maintained copies of the response math. Share
ordinary typed helper functions and implementations across the calling passes.
Do not let the legacy particle response silently stand in for this contract:
particle techniques must explicitly provide the required inputs before they can
advertise support. Canvas is likewise compatible only with a declared contract,
not because the compiler recognizes its entry name.

## Symbolic identities and generated IDs

Separate three identities:

1. **Style declaration identity:** a module-qualified symbol. This is what source
   assignments and saved preview choices reference.
2. **Style instance identity:** the selected declaration plus specialized settings
   and implementations. Different static settings may produce different instances.
3. **Material identity:** the existing generated material ID, identifying material
   data and its assigned style instance.

The compiler assigns dense artifact-local IDs to the used style instances, with
reproducible ordering. Numeric IDs are not authored enum values, stable asset IDs,
or values the browser persists. Unused imports do not become active draw work.
Runtime-editable settings live in reflected data; they do not require a new ID for
every slider movement. Static settings that change code or graph structure do
require specialization/recompilation.

The manifest should describe:

- Available style declarations: symbol, label, source location, target schema,
  required engine contracts, and settings metadata.
- Used style instances: generated ID, declaration reference, specialized response
  implementations, settings layout, and contribution/technique references.
- Material assignments: material ID to style instance ID and settings record.
- The resolved executable graph: contribution instances, draw scope, resource
  bindings, dependencies, and explicit attachment operations.

Exact artifact field names are an implementation detail to settle with the shared
Rust/TypeScript contracts. This is an artifact-version change if new required
records are added; compiler, native host, renderer WASM, and compiler WASM must
agree. The catalog comes from imported declarations, not a hardcoded Rust list.

## Response dispatch in each renderer

### Forward and Forward+

Specialize each material's mesh shading entry to its selected implementation.
Forward and Forward+ differ in light enumeration, not in response selection.
Normal maps, material layers, opacity, and deformation are prepared consistently.

### Deferred

Keep material IDs in the GBuffer. Resolve looks up the material's style instance
and dispatches to the corresponding linked response implementation. Its settings
are supplied through the declared layout.

There must be no handwritten `switch` that lists PBR and Toon. The compiler can
generate a dispatch function from a typed implementation table, just as it can
generate table IDs. That operation must be generic: selector plus compatible
implementation signatures, independent of rendering vocabulary. Only referenced
implementations are linked; GPU recursion and incompatible signatures fail.
A generated switch is an acceptable first implementation. Function pointers are
not required.

The engine determines the standard GBuffer encoding. A style that implements the
standard style contract can reuse it. If a later style requires information
that cannot survive that encoding, it must declare a compatible alternate path or
additional resources. Neither the compiler nor host may silently pre-shade it or
pretend missing data exists.

A global style uniform is insufficient: different material IDs must be able to
select different styles in one artifact. Packing a style number into arbitrary
material-ID bits is also unnecessary.

## Composing style techniques with renderer stages

A renderer publishes **typed integration points** as part of its authored
contract. For example, the example engine can expose an opaque-color stage after
lighting and before presentation, with access to readable scene depth and a
loadable scene-color attachment. The compiler assigns no special meaning to the
stage's name.

The contract must collect these declarations with the shading hooks so authors
can discover the whole extension API. `after_opaque` is proposed engine vocabulary,
not a reserved language stage. Avoid an inverted-hull-shaped extension point such
as `geometry_overlay`: an integration point specifies timing, resources, and
capabilities, while the style chooses draws, compute, or composed techniques.

For this engine, `after_opaque` guarantees completed opaque scene color and depth
for all relevant objects and completion of contributed work before transparency,
inspection, and presentation. Each renderer maps this guarantee to its own graph;
an arbitrary node anchor alone does not establish the scene-wide guarantee.
Attachment access and preservation are checked against the operation requirements
and integration contract; shorthand must not silently change global clear/load
defaults. The worked specification passes attachments explicitly through typed
parameters supplied by `at point as target`.

Mesh invocation scope does not imply pixel-local effects. A shell writes pixels
covered by selected geometry, including expanded silhouettes. A fullscreen effect
that should affect only selected objects needs an explicit selection mask or
identity resource and a contract that supplies it.

A style contribution names a compatible point and declares:

- Whether it runs per view, per material instance, or per mesh instance.
- Required geometry/factory capabilities and prepared vertex/material inputs.
- Its resource reads and writes, explicit bindings, and attachment operations.
- Ordering dependencies and any static activation condition.

Graph composition instantiates the contribution in the declared scope, binds the
provided resources, and validates the combined dependencies. It namespaces nodes
and owned resources per instance, so multiple materials using one style do not
collide. Scope is explicit; it must not be inferred from a style name.

For an outline, two mesh instances using the same Toon material cause two outline
draw invocations. Their pipelines can be shared, but their transforms, geometry,
and per-draw inputs cannot. An ordinary PBR mesh gets no outline invocation.
Per-view contributions should not be duplicated once per material merely because
several materials select the same style; their sharing/instance key must be
explicit.

Do not rely on a single selected preview mesh for validation. Exercise scoped
invocation data with multiple synthetic draw instances and views in runtime tests.
A full scene editor is not required to prove that the plan supports multiple
meshes or that the second pass belongs to the selected style.

## Inverted-hull Toon outline: the first stage-extension test

The Toon module declares an outline technique in addition to its lighting
response. Start with closed opaque meshes and explicitly reject unsupported
coverage modes for the outline-enabled configuration. Do not imply correct masked
cutout or translucent outlines merely because their base shading works.

The outline vertex path:

1. Uses the same vertex-factory preparation, object transform, and authored
   deformation as the base mesh, so animated/deformed geometry stays aligned.
2. Computes an expansion using geometric/smoothed mesh normals, not a normal-map
   perturbation. The engine contract must expose the required position/normal
   data; incompatible factories produce a capability diagnostic.
3. Produces a clip-space offset for a requested pixel width using viewport size
   and clip `w`, with defined handling for degenerate projected directions and
   near-plane cases. Perspective/distance and nonuniform transforms must be tested.

The outline raster pass:

- Culls front faces and draws the expanded back-face shell.
- Loads the existing HDR scene color and depth.
- Uses a declared depth comparison against completed scene geometry.
- Disables depth writes in the initial implementation.
- Writes the authored outline color using explicit replace/blend state.

Schedule it after opaque color/lighting and before presentation and transparent
composition, rather than accidentally applying lighting to the outline color.
All opaque depth must be available before outlines, so other opaque meshes can
occlude them. Depth-write-disabled shell overlaps have ordinary ordering
limitations; the first version must document them rather than claim exact
outline-to-outline occlusion. A later outline depth/composite technique can refine
that behavior without adding compiler outline rules.

For Forward/Forward+, the contribution loads their completed color and mesh depth.
For Deferred, it loads the lit-color target after resolve and tests against the
geometry depth target. The deferred shell draw must not write fake material IDs or
GBuffer values. GBuffer inspection continues to show geometry data; the final
shaded view includes the outline. Existing buffer inspection and presentation
must be ordered after the relevant style work.

Use an outline-friendly closed mesh in the example. Hard edges and split normals
can separate inverted shells; a polished result may need dedicated outline
normals or different mesh preparation. That requirement belongs in the style's
factory contract and assets, not a compiler normal-repair heuristic.

## Real gaps this must exercise

Implementation selection, generated dispatch, mesh contributions, and explicit
attachment operations provide a baseline. The revised design still needs:

1. A first-class style declaration unifying schema, settings, hooks, and graph work.
2. Discoverable engine contracts covering shading and typed integration points,
   including structured lighting inputs and final surface-color selection.
3. Separate operation declarations and graph calls with checked inputs, outputs,
   resource access, dispatch sizing, and reusable technique composition.
4. Owned intermediate resources, invocation identity, and dependencies across
   engine boundaries without unnecessarily serializing independent compute.
5. Prepared geometry/deformation producers with checked compute and draw access.
6. Reflected settings and capability-aware catalogs, consumed by native/browser
   hosts through the compiled graph without recognizing individual styles.

Preserve explicit clear/load and store/discard declarations and validation of
initialization and dependency order. A load requires an
initialized prior version or an explicit imported initialized attachment.
Reject unordered writers, cycles, incompatible formats/sample counts, and illegal
same-pass sampling/attachment use. Do not relax the current conflicting-write
check globally or silently change every pass from clear to load.

Do not evade the graph work by adding a Rust-only outline pass, a hidden second
renderer, or a renderer name for every style combination. The purpose of this
example is to demonstrate that authored contributions work end to end.

## Browser selection and persistence

Populate a material's style selector from imported declarations compatible with
its schema and the selected renderer contract. Show the engine's default assignment
and allow explicit selection of a declared style. Importing Toon alone changes
nothing. The sample imports it and can explicitly assign it to demonstrate use;
other standard materials can select it when that module is in their bundle.

A preview override refers to the style symbol and settings without rewriting
material-layer source. An authored assignment persists in `.fr` or the material
asset. Preview overrides are separate, scoped to the current document/material,
and clearable. Persist symbols and settings, never artifact-local numeric IDs.

Switching styles recompiles the affected implementations and composed graph,
including adding/removing outline draws and their bindings. Use the existing
candidate installation and cancellation mechanism: cancel obsolete candidates,
keep the previous working preview visible, install atomically, and reject stale
results. Explicit document switches and blank documents still clear the preview.
Preserve compatible material parameters, textures, camera, geometry, lighting,
and buffer inspection state. Runtime settings such as outline color/width can be
uniform updates; changing contribution topology requires recompilation.

Changing renderer rechecks the selected style's requirements. Unsupported styles
remain visible with an actionable incompatibility message; never silently strip
the outline or substitute PBR. The renderer and style are independently selected
but their contracts must be compatible.

## Implementation sequence

The following decisions resolve the adversarial review. The worked specification
at the bottom exercises them; these rules take precedence over abbreviated sketches.

### Decisions required before implementation

1. **Engine contract and provider.** A contract declares hooks, defaults, input
   capabilities, and integration points together. A renderer provides it through
   explicit graph connections. Compiler checks cover types and graph relationships;
   engine tests establish the meaning of supplied data. Names alone prove nothing.
2. **Attachments.** `at point as target` obtains a typed integration context.
   Alternatively, a point's declared resource port supplies members such as
   `opaque.color` and `opaque.depth`, from which placement is inferred. Draw calls
   pass attachments explicitly; signatures declare preserve/update or test-only
   access. No ambient attachment lookup occurs.
3. **Composition.** An integration point declares a writer policy for both explicit
   and inferred placement. `ordered_draws` serializes preserving writes in the engine's
   stable draw order. Transparent work joins the engine's global transparency
   queue, with declared sort keys. Arbitrary compute/fullscreen updates cannot
   join these policies merely by sharing attachments. Unsupported conflicts fail.
4. **Invocation identity.** `self` is `(frame, view, object instance, material draw
   range)`, not a geometry asset or style symbol. Instancing and batching preserve
   this logical identity. A mixed-material mesh contributes only selected ranges.
   A prepared stream maps local indices to exactly that range's vertices and
   triangles. View-dependent outputs cannot be shared between views.
5. **Compute-to-shading.** Typed resource slots explicitly bind graph outputs to
   hook inputs. Material-scoped resources use material records; draw-scoped outputs
   require a shading-instance record distinguishing objects and ranges. Deferred
   must preserve an engine-provided shading-instance ID and reconstruct lookup
   coordinates, or reject the requested capability. Material ID alone is inadequate.
6. **Preparation.** A demand-driven engine producer materializes prepared geometry
   when required. Base and contributed draws use that same prepared stream. Ordinary
   materials need not take this path. Coordinate spaces, indexing, deformation,
   normal transformation, and view dependence are part of the capability. Engines
   may implement it for several factories without a compiler-specific factory branch.
7. **Settings.** Runtime records belong to material instances independently of
   shared compiled style code. Named assignment overrides are type/range checked.
   `static param` controls topology and specialization; `param` controls runtime
   values. `static if` builds graph branches and checks requirements only for the
   selected branch (all branches remain syntactically/type valid). Shader `if` never
   constructs graph nodes. Runtime disabling of nodes is outside the initial model.
8. **Lighting accounting.** `direct` returns a complete per-light contribution,
   including whatever use it makes of separate radiance/attenuation/visibility.
   The engine sums it without reweighting. `indirect` excludes emission. `finish`
   returns linear HDR color; its default adds direct, indirect, and emission once.
   Coverage/alpha stays separate. Common dynamically dispatched hooks initially
   disallow implicit derivatives; resource sampling uses explicit LOD or supplied
   gradients under a capability that defines them.
9. **Validation and lifetime.** The compiler checks types, declared shape relations,
   producers, static activation, and graph hazards. The host checks dynamic extents,
   checked size arithmetic, allocation/device limits, and dispatch limits. Shader
   authors guard dynamic indexing; arbitrary shader math is not statically proven
   safe or fully initializing. Intermediates are transient for the frame and logical
   invocation; allocation reuse waits for GPU completion. Persistent history,
   cross-frame feedback, and readback are excluded from the initial implementation.

Preparation and extra draws need a visibility policy independent of base-draw
culling. A contribution declares conservative bounds or requests uncullable work.
The initial pixel-width Toon outline uses the latter rather than claiming a fixed
world-space bound. World-space fur length supplies a conservative expansion.
Runtime settings affecting bounds update bounds before visibility decisions.

Engine-provided stable ordering is a rendering policy, not source/import order.
Noncommutative writes cannot be arbitrarily reordered. Multiple independent
transparent contributions must sort together with ordinary engine transparency;
sorting once per style is incorrect. The initial sorted transparency capability
documents intersecting-surface limitations and rejects renderers that cannot
provide its promised queue integration.

The decisive pre-implementation deliverable is a closed example covering the
contract, providers, operation bodies, assignments, and instantiated graph. The
appendix below is that proposed specification, including bounded limitations.

### Delivery and status

Follow the [phase-based implementation plan](#phase-based-implementation-plan)
above. It is the single completion/cleanup checklist. The implementation notes
below explain completed behavior. The worked appendix labels its executable
MeadowFur blocks separately from remaining illustrative engine declarations.

### First implementation task: shading-only contracts and styles

Implemented: the shading-only slice below. The full Toon/Fur appendix remains
a proposal. This slice uses an engine-declared `implementation<C>` property and
explicit `@dispatch(C, hook)` wiring; it does not create an implicit `style`
property or renderer provider.

This vertical slice implements `contract C for schema` and
`style S for schema : C`, with typed shading hooks, explicit contract defaults,
and symbolic material assignment. Start with a tiny custom schema and contract,
not `standard` or Toon. This establishes the ownership and conformance rules
without baking engine vocabulary into the compiler.

The slice includes:

- AST/parser nodes retaining contract, target schema, style, hook bodies, and
  source spans. These remain distinct semantic declarations even when lowering
  reuses interfaces and named implementation dispatch.
- Name resolution and checking for imported declarations, schema compatibility,
  exact hook signatures, missing required hooks, duplicate/unknown hooks, and
  default bodies checked in their contract's defining scope. A default becomes
  part of the selected style's implementation; it is not an engine fallback for
  an invalid style.
- Lowering of stateless shading-only styles into the existing implementation
  selection and GPU dispatch path. An engine-authored fixture invokes its hooks
  explicitly, so declaring a contract never pretends to install renderer wiring.
  Two materials can select different imported styles in the same artifact.
- Focused diagnostics for not-yet-supported contract capabilities, integration
  points, static graph branches, and technique bodies. Never accept these constructs
  and silently discard their semantics. Their final ownership remains in the
  unified contract/style declarations specified above.

Acceptance: compile and validate WGSL for the custom-schema fixture with a
required hook, a defaulted hook, and a style overriding that default. Check
selected hook behavior, stable symbolic selection after import reordering, and
rejection of a wrong schema, wrong signature, missing hook, and unknown hook.
Keep the existing named-implementation and standard renderer tests passing.
This is a compiler feature slice, not a claim that the full appendix compiles.

Executable declaration/selection shape (the engine must also supply its schema,
material composition, and pass):

```fresco
contract Response for base {
    fn apply(value: vec4) -> vec4
    fn finish(value: vec4) -> vec4 { return value }
}

style Plain for base : Response {
    fn apply(value: vec4) -> vec4 { return value }
}

style Half for base : Response {
    fn apply(value: vec4) -> vec4 { return value * 0.5 }
    fn finish(value: vec4) -> vec4 { return value * 0.75 }
}

@surface_properties(properties) interface Options {
    param @config(editor) style: implementation<Response> = Plain
}
```

A `material(base)` surface selects `properties { style: Half }`. Required/default
hooks are checked even on unused declarations; defaults use ordinary library
scope, with no implicit access to style hooks or material/pass locals. Contract
and style schema names must agree. Material selection permits the target schema
or a schema extending it, following existing material compatibility rules.

Completed follow-up: the example engine now uses structured `direct`, `indirect`,
and `finish` hooks across Forward, Forward+, and Deferred. PBR preserves the
previous BRDF and indirect-fill math; emission moved to the default finish hook.
The sample Toon declaration retains its locally authored outline using the
existing contribution API. No operation syntax or provider capabilities are implied.

Next, add the contract/provider graph features needed for the direct inverted
hull, followed by owned compute outputs. Do not migrate the outline to
operation calls until scoping, attachment access, and integration ordering are
enforced.

### Implemented follow-up: per-material runtime style settings

A style may declare `param` settings captured by its authored shading hooks:

```fresco
style Toon for standard : StandardStyle {
    param band_threshold: f32 in [0.05, 0.55] = 0.25
    param highlight_strength: f32 in [0.0, 2.0] = 1.0
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 {
        let band = smoothstep(band_threshold - 0.02, band_threshold + 0.02,
                              dot(surface.normal, light.direction))
        return surface.albedo * band * light.radiance * light.attenuation * light.visibility
    }
}
```

Assign defaults with `properties { style: Toon }`, or named overrides with
`properties { style: Toon(band_threshold: 0.35, highlight_strength: 0.8) }`.
The existing spelling `param gain: f32 = 0.5 in 0.0 .. 1.0` is also accepted.
Defaults and overrides must be typed compile-time constants. Defaults are required;
duplicate/unknown/positional assignments, unsupported types, reversed ranges, and
out-of-range components are errors. Supported types are `f32`, `vec2`, `vec3`, `vec4`, `color`, `u32`, `i32`, and
`bool`. Integer settings preserve all 32 bits; booleans reject numeric ranges.
Resource captures and operation-body captures remain follow-ups. An authored
hook's parameter cannot shadow a setting. Contract defaults do not capture settings.

**Engine integration.** Each material selection reflects `parameters` (name, type,
resolved default, min/max) and `settings_offset`. Offsets address a bundle-wide
`buffer<vec4>` with one 16-byte lane per parameter, in declaration order; float scalars
and short vectors use the leading components. Integers use two exactly representable
f32 components containing the low and high 16-bit halves, reconstructed with integer
operations; signed values use their two's-complement representation. Boolean values
use 0 or 1 in the first component. This avoids rounding integer seeds through f32. Offsets are assigned in material-name
order independently of the shared style dispatch ID. A material table exposes its
row through `@implementation_settings_value(style)`. An engine wires the resource
and offset explicitly:

```fresco
@dispatch(StandardStyle, direct, style_settings)
fn style_direct(style_id: u32, settings_offset: u32,
                surface: StandardSurface, context: ShadingContext,
                light: DirectLight) -> vec3 { return vec3(0.0) }
```

The compiler loads the selected style's captures and calls its shared implementation.
A selected style with runtime parameters requires the three-argument dispatch form;
static-only styles can use the two-argument form. The
example engine supplies `style_settings` from `@external(style_values,
style_parameters)`. Forward/Forward+ obtain the offset from `DrawRecord`; Deferred
looks it up by GBuffer material ID. Each renderer binds the same per-material data.
This is explicit engine resource wiring, not a renderer name hardcoded in the compiler.
The artifact schema is version 9; compiler, native runtime, renderer WASM, compiler
WASM, and generated TypeScript contracts must ship together.

**Live editing.** Host keys are `<property>.<parameter>`, e.g.
`style.band_threshold`. Updates validate the complete material/style batch before
committing CPU state; the next frame uploads the changed values without compilation
or pipeline replacement. Scene objects share the material table and settings buffer.
Different materials selecting Toon have independent records. Repeated occurrences
of the same material share its record; contradictory per-object settings are rejected.
Per-object style-instance overrides are not part of this slice.

The browser derives controls from reflection and retains compatible edited values
across renderer changes. Switching style symbols resets style-specific edits.
Compile configuration can persist a symbolic selection and named values:

```json
{"property_overrides":{"my_material":{"style":{
  "symbol":"Toon", "settings":{"band_threshold":0.35}
}}}}
```

Never persist numeric IDs or offsets. The sample's band threshold and highlight
strength now use this path. Outline width/color remain authored constants until
operation/contribution settings captures have a checked ownership model.

### Implemented follow-up: static setting specialization

`static param` accepts the same checked types, defaults, ranges, and named
assignment syntax as `param`. For example:

```fresco
style Banded for standard : StandardStyle {
    static param bands: u32 in [1, 16] = u32(4)
    param seed: u32 = u32(4294967295)
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 {
        let nl = max(dot(surface.normal, light.direction), 0.0)
        let response = floor(nl * f32(bands)) / f32(bands)
        return surface.albedo * response * light.radiance * light.attenuation * light.visibility
    }
}
```

A material can select `Banded(bands: u32(8), seed: u32(16777217))`.
Static values become typed constants in the generated hook body. Equal static
choices share a dispatch identity; different choices receive distinct identities.
Runtime values remain separate per material and do not change code identity.
Contract defaults retain their own scope and cannot capture either kind of setting.

Reflection separates `static_parameters` from live `parameters`; static values
consume no settings-buffer lanes. Browser controls mark them as rebuilds and
persist the style symbol and named values. They go through the existing candidate
compilation/replacement flow. Live host updates cannot change static parameters;
an invalid mixed update leaves previous values intact.

This implements shader specialization, not graph construction. `static if`,
`static for`, `for self`, typed capability/provider declarations, and operation
calls remain unsupported. The next integration milestone must establish those
contracts and per-invocation resource ownership before migrating the outline or
claiming that the MeadowFur appendix executes.

## Acceptance tests

- Importing a third externally authored style adds a selectable declaration with
  no central enum, hand-coded response branch, or browser option change.
- Existing standard material layers remain unchanged when assigning a style.
- Two material IDs in one artifact can use different style instances; IDs remain
  correct after declaration/import ordering changes and unused styles are omitted
  from executable work.
- Shared shading produces matching Forward/Forward+/Deferred results within
  documented buffer precision, using actual scene lights and shadows.
- Selecting Toon with outlines adds its declared mesh draw; selecting PBR removes
  it. Two meshes sharing the style produce distinct invocations with correct inputs.
- The extra draw loads and preserves prior color/depth. Other opaque geometry
  occludes the shell, deformation stays aligned, and pixel width is checked across
  viewport sizes, distance, camera angles, and nonuniform transforms.
- Missing capabilities, invalid assignments, unsupported outline coverage modes,
  dependency cycles, uninitialized loads, and conflicting writes fail explicitly.
- A domain-neutral fixture proves contributed draws and implementation dispatch
  without any names or concepts related to Toon, lighting, or GBuffer layout.
- The browser reflects imported styles, preserves source text for preview changes,
  hides the control where no contract applies, and never installs stale results.
- Native and browser hosts execute the same compiled contribution graph. GPU
  checks remain local-only; no screenshots or golden shader output replace
  behavioral assertions.
- Operation definitions and calls are checked in their respective contexts;
  incompatible output types/access, missing inputs, and shader/graph misuse fail.
- A compute-to-outline fixture has preparation-to-compute, compute-to-draw, and
  opaque-completion-to-draw edges, without an unnecessary opaque-to-compute edge.
  Assert graph relationships rather than one incidental execution order.
- Two Toon meshes and one PBR mesh preserve distinct geometry and intermediate
  contents; only the Toon meshes receive outline work, and all opaque depth can
  occlude it. Unsupported compute access to prepared geometry fails explicitly.
- Missing or conditionally absent producers, incompatible buffer extents/indexing,
  unordered writes, and a late producer feeding an earlier consumer fail.
- The engine contract makes required/default shading hooks and available integration
  points discoverable together. Missing renderer data or capabilities cannot silently
  omit work or substitute another style.
- Two materials using one style retain independent runtime settings. Static disabled
  branches contribute no resources or requirements; topology cannot change through
  an ordinary runtime parameter update.
- Mixed-material ranges, instanced geometry, two cameras, and expanded bounds retain
  correct invocation identity and visibility. Uncullable contributions remain
  eligible when their base draw is culled.
- Contributions from multiple styles participate in the declared common attachment
  ordering and global transparency queue, independent of import order.
- Compute output bound to a base shading hook resolves through the correct instance
  record in Deferred; unsupported binding models fail before artifact installation.
- Runtime extent overflow, allocation/dispatch limits, and resource shape mismatches
  produce actionable errors; tests do not assume the compiler proves arbitrary
  shader writes or indices.
- Fur output count/indexing, bounded wind displacement, nonuniform transforms,
  conservative bounds, alpha handling, and emission accounting have behavioral
  coverage. Frame-transient data cannot leak between views or survive as history.

## Implemented syntax and behavior

The engine now declares `contract StandardStyle for standard` with these hooks:

```fresco
fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3
fn indirect(surface: StandardSurface, context: ShadingContext, light: IndirectLight) -> vec3
fn finish(surface: StandardSurface, context: ShadingContext, lighting: LightingResult) -> vec3
```

`direct` is required. The contract's default `indirect` supplies occluded diffuse
environment lighting; default `finish` adds direct, indirect, and emission once.
PBR and the local Toon example are ordinary `style ... for standard : StandardStyle`
declarations. Both override indirect lighting to retain the engine's existing
specular-fill approximation. They use the default finish hook.

`StandardSurface` contains prepared world-space normal, linear albedo, roughness,
metallic, occlusion, and emissive. `ShadingContext` contains world position and
normalized view direction. `DirectLight` separates direction, radiance,
attenuation, and visibility. `IndirectLight` supplies irradiance and the engine's
`specular_fill` scalar. `LightingResult` contains the accumulated direct and
indirect contributions. UVs and draw-scoped resources are not promised by this
contract yet; Deferred does not preserve them.

For example, an imported style can implement just direct lighting:

```fresco
style Ink for standard : StandardStyle {
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 {
        return surface.albedo * step(0.5, dot(surface.normal, light.direction))
             * light.radiance * light.attenuation * light.visibility
    }
}
```

The lower-level stateless `@implementation`/`conform` API remains available for
ordinary interfaces. Styles lower into the same dispatch machinery. Registration,
signature, and default-body errors are diagnosed even for unused declarations.
Names follow existing imported symbol resolution; styles add no module namespace.

The example engine's existing property contract declares:

```fresco
param @config(editor) @schema(standard) style: implementation<StandardStyle> = StandardGGX
```

`@schema` restricts this property to that schema and its descendants. Incompatible
explicit assignments fail. PBR is an ordinary registered implementation in
`engine/styles/pbr.fr`. The sample defines Toon in its own source file and selects
it with `properties { style: Toon }`, retaining `material(standard)` and its
normal material layers. Importing another implementation only extends the catalog.

Passes bind generated dispatch using an ordinary typed hook:

```fresco
@dispatch(StandardStyle, direct)
fn style_direct(response: u32, surface: StandardSurface,
                context: ShadingContext, light: DirectLight) -> vec3 {
    return vec3(0.0)
}
```

The signature must prepend a `u32` selector to the exact interface signature.
The compiler inserts branches for used implementations and links their ordinary
functions. The authored body handles IDs outside the table; there is no implicit
PBR fallback. Dispatch IDs start at one, are dense over used selections, and use
sorted contract/symbol ordering. Unassigned optional slots project to zero.
`@implementation_value(style)` projects a checked property slot into an
ordinary table column, rejecting unknown slot names.

Forward and Forward+ pass the material selection to these functions. Deferred
retains the GBuffer material ID and uses an engine-declared table column and buffer
to retrieve the same selection. Rust has no PBR/Toon branch or style registry.
Both paths prepare the same structured inputs. `direct` returns a complete
contribution: the renderer sums it without multiplying by light energy, distance
attenuation, visibility, or cosine again. It dispatches only active directional
lights and point samples inside their finite influence volume; empty/zero-energy
slots are not lights. Point visibility is currently 1; directional visibility
comes from the existing shadow map. `indirect` runs once and excludes emission.
`finish` runs once after all lighting, before coverage, blending, and presentation.
Normal mapping, material composition, and opacity remain renderer responsibilities.

The explicit unlit inspection mode continues to bypass lighting styles and show
albedo plus emission. Unlit/custom material schemas and the older particle response
path are separate contracts. The staged `fwd_base`/`fp_shade` sketches are not the
executable renderer providers; their existing BRDF examples remain staged.

`settings.implementations` reflects the property name, contract, selected symbol,
artifact-local ID, declared imported symbols, editability, and contextual
`availability` records. Unsupported symbols remain in the catalog; capability
requirements and static graph specialization determine their support status.
These are optional additive manifest fields. Native/browser consumers use the same generated
resource bindings. The browser builds controls from these declarations, stores
symbolic compile overrides, preserves the material source, retains selection
across renderer changes, and resets overrides when changing documents. The style
dropdown remains usable after a failed compile. Unrelated canvas/unlit previews do
not expose a style selector.

## Remaining implementation work

Track the remaining resource-driven placement work in
[phase 11](#phase-based-implementation-plan). Shading hooks, settings, graph
capabilities, operations, owned resources, complete Toon/Fur samples, and editor
selection and legacy retirement are implemented. Typed completion ports and
inferred placement are implemented and validated; Phase 11 records their acceptance evidence.

## Operation composition and migration

The supported path is a typed contract/provider plus operation calls in the style.
Legacy `@stage`, `@stage_input`, and `@contribute` authoring is rejected; migration
steps and intentionally retained lower-level mechanisms are documented in
[LANGUAGE.md](../LANGUAGE.md#migrating-legacy-style-integration).

The Toon sample captures width and color settings in its inverted-hull invocation,
uses prepared geometry shared with the base surface, preserves color/depth, and
rejects incompatible blend/sidedness. Its silhouette limitations on open meshes
and split hard-edge normals still require an authored coverage/normal policy.

## Worked proposed specification: Toon and MeadowFur

This final section specifies the new authored pieces end to end. Sections E and F
match the runnable `examples/40) surface shaders/style_sample_fur.fr` accepted in Phase 8.
Other blocks retain illustrative design spellings; the executable engine contract,
providers, and Toon source remain authoritative for their current syntax.
Engine services are an explicit boundary, specified
below rather than assumed to be language intrinsics. This is not a replacement
implementation of the entire engine's geometry, light culling, or shadow system.

Toon changes lighting and adds one inverted-hull draw. MeadowFur keeps an opaque
standard-material root, computes a density field and displaced shell vertices,
uses the density field in base shading, and submits multiple translucent shell
draws. It is procedural shell fur, not strand simulation or a production hair BSDF.

### A. Declaration and execution rules used by the examples

- `contract` declares an engine API; `provide` maps one renderer graph to that API.
  Capabilities have typed members. A style requests optional capabilities explicitly.
- Hook methods see their style's settings and typed `shading_input` bindings.
  Ordinary external helper functions receive all their arguments explicitly.
- `draw` and `compute` declare reusable operations. Calls instantiate graph work.
  `output` allocates a typed transient resource; returning it exports the resource
  handle, not its contents to the CPU. Read/write qualifiers declare access.
- `dispatch threads(...)` specifies logical extents. The backend rounds up to
  workgroups; shaders must guard padded invocations. Extent multiplication and
  ceiling division use checked host arithmetic. Zero logical extent skips dispatch.
- `raster geometry` supplies triangle indices, range, and indexed draw counts from
  a prepared mesh. `vertices` substitutes a compatible stream; `base_vertex`
  selects a shell's region. No indirect draw-count generation is used here.
- `vertex_index` indexes that stream including `base_vertex`. Raster varyings
  interpolate smoothly unless stated otherwise; `clip` is builtin clip position.
- `sample_level` is explicit-LOD texture sampling. Density fields below use
  nearest/repeat at level zero. They have one mip level and r32float storage.
- Attachments are explicit parameters. `preserve_update` means load/store plus
  an ordered color write. `test_only` means depth testing without writes, preserving
  the attachment. Both validate initialization, formats, and matching sample counts.
- `static for` instantiates a bounded set of graph nodes; it does not establish
  draw ordering. The integration point's queue determines execution order.
- Named settings assignment uses `properties { style: Toon(outline_width: 3.0) }`.
  Ranges are validated on initial assignment and every runtime update; updates are
  published as one frame-consistent record across graph and shader uses.
- Floating inputs must be finite. `length_of` denotes vector length and explicitly
  wrapping integer operations have modulo-2^32 semantics. The authored standard
  library supplies `normalize_or_zero(v)` and `normalize_or(v, fallback)` for
  vec2/vec3/vec4: lengths at or below 0.0001 return zero or the fallback unchanged;
  longer finite vectors normalize to unit length. Scaling before squaring avoids
  overflow for large finite vectors. These are numeric helpers, not engine services.
  Use `inverse_sqrt` for reciprocal square root in new source; `inverseSqrt`,
  `inversesqrt`, and `rsqrt` remain compatible spellings. Generated WGSL uses
  its native `inverseSqrt` spelling.
- Operation `requires material...` reads immutable invocation metadata: the selected
  material's blend and sidedness properties. Nested operation calls retain that
  metadata. Reusable operations cannot inspect undeclared material shader values.
- Style hook captures are restricted to declared settings and shading inputs;
  lowering supplies their records explicitly. Graph locals are not implicitly
  visible inside shader functions. Function aliases must match the same signatures.
- `requires` is a checked precondition, not a filter that silently drops work.
  `visibility: uncullable` disables contribution culling for the selected range.
  `bounds.expand_world(x)` expands the prepared world-space bounds conservatively.
- `bind shading.name = value` exports a graph resource to the current style's
  hook invocation record and makes every hook consumer depend on its producer.
  Per-draw bindings are indexed by shading-instance identity, not material ID.
- The initial worked provider supports single-sample rgba16float scene color and
  depth32float depth. MSAA requires a separate matching provider, not a hidden resolve.

### B. Shared engine types, hooks, and capabilities

The engine defines these records and service signatures. Their members are part
of the contract, not compiler-known material or rendering concepts.

```text
struct StandardSurface {
    albedo: vec3
    roughness: f32
    metallic: f32
    occlusion: f32
    emissive: vec3
    normal: vec3                 // Prepared world-space shading normal.
    uv: vec2                    // Only promised by SurfaceUV capability.
}

struct DirectLight {
    direction: vec3              // Unit world-space direction toward light.
    radiance: vec3               // Linear incident light color/energy.
    attenuation: f32             // Distance/cone factor, not yet applied.
    visibility: f32              // Shadow factor, not yet applied.
}

struct IndirectLight { irradiance: vec3; specular_fill: f32 }
struct LightingResult { direct: vec3; indirect: vec3 }
struct ShadingContext { world_position: vec3; view_direction: vec3 }

struct PreparedVertex {
    world_position: vec3
    geometric_normal: vec3       // Unit normal after inverse-transpose transform.
    uv: vec2
}

resource PreparedMesh {
    vertices: buffer<PreparedVertex, read>
    indices: buffer<u32, read>   // Local indices, all less than vertex_count.
    vertex_count: u32
    index_count: u32
    bounds: WorldBounds
}

struct View {
    view_matrix: mat4
    projection_matrix: mat4
    viewport_size: vec2          // Strictly positive physical pixel dimensions.
    camera_position: vec3
    time: f32
}

struct OpaqueTarget {
    color: attachment<rgba16float, preserve_update>
    depth: attachment<depth32float, test_only>
}

capability PreparedGeometry {
    mesh.prepared: PreparedMesh
}

capability SurfaceUV {
    surface.uv: vec2
}

capability DrawShadingResources {
    shading_instance_id: draw_instance_id
    // A typed per-instance table supplies explicitly bound hook resources.
}

capability ForwardLighting {
    lights: LightingService
}

contract StandardStyle for standard {
    input view: View

    fn direct(surface: StandardSurface, context: ShadingContext,
              light: DirectLight) -> vec3

    fn indirect(surface: StandardSurface, context: ShadingContext,
                light: IndirectLight) -> vec3 {
        return surface.albedo * (1.0 - surface.metallic)
             * light.irradiance * surface.occlusion
    }

    fn finish(surface: StandardSurface, context: ShadingContext,
              lighting: LightingResult) -> vec3 {
        return lighting.direct + lighting.indirect + surface.emissive
    }

    optional capability PreparedGeometry
    optional capability SurfaceUV
    optional capability DrawShadingResources
    optional capability ForwardLighting

    point after_opaque: OpaqueTarget {
        scope: view
        accepts: raster_draws
        composition: ordered_draws(engine.stable_draw_order)
        after: complete_opaque_lighting_and_depth
        before: transparency, inspection, presentation
    }

    optional point transparent: OpaqueTarget {
        scope: view
        accepts: raster_draws
        composition: global_transparent_queue
        after: after_opaque
        before: inspection, presentation
    }
}
```

`context.view_direction` is safe-normalized toward the camera. The engine invokes
`direct` once per applicable light, sums its returned values unchanged, invokes
`indirect`, then `finish` once. Emission is excluded from `IndirectLight` and its
default hook. Coverage is applied outside these hooks. The revised engine PBR
implementation must conform to this accounting; old angular-only functions cannot
be substituted without adapting their weighting.

`PreparedGeometry` means a material-range-local indexed stream produced after
authored deformation and object transformation. A compute preparation producer
materializes it on demand, and the base draw uses the same stream. The host
validates indices and ranges. A factory that cannot materialize equivalent data
does not provide this capability. Prepared bounds include deformation; if those
bounds are unavailable, contributions use uncullable visibility.

`LightingService` has exactly two operations: `direct_samples(position, normal)`
enumerates the engine's applicable `DirectLight` values, including shadow sampling;
`indirect(position, normal)` returns `IndirectLight`. It is shader-callable from
forward raster draws and uses the same scene lights as base shading. It is not a
CPU callback or implicit GPU resource discovery. Bindings and shader helper code
are linked through the capability. Extra geometry shadow casting is not implied.

The declaration-level export syntax is `@service(LightingService)` on the engine
pass that owns the helper methods and bindings. The interface declares ordinary
explicit method arguments; stage entries, evaluators, and dispatch hooks cannot
serve as exported methods. Export checking does not schedule the pass. It checks
unused exports too, and rejects missing or ambiguous signatures. This declaration
checking and reachable helper-body checking are implemented. Imported helpers keep
their module scope; they cannot capture a caller's pass bindings or pass-local
functions. Invalid returns, missing captures, and recursive closures are rejected
even for unused exports. Providers bind services with `shader_service(node)` and
draw operations receive an explicit interface-typed parameter, called as
`lights.method(arguments)`. The linker copies only reachable helpers and their
declared captures into each invocation, preserving independent capture bindings.
The node supplies explicit binding provenance,
not a dependency on executing that node. Actual resource producers must satisfy
the consuming operation's graph dependencies: internal captures must already be
initialized before the selected integration boundary. Unused providers are checked
against all exported methods; an active invocation binds only the methods it calls.
Transparent boundaries include the queue's external prerequisites. Typed boundary
composition rewires explicit outgoing consumers, preserving prerequisite producers
between opaque completion and transparency; only legacy anchor adapters infer all
direct successors. Tests reject a late resource at `after_opaque`, accept it at a
queue that waits for it, and verify unused operation helpers add no captures.
This mechanism is implemented and tested with environment and shadow-resource
methods across all three renderers. The concrete `ForwardLighting` provider exports
`LightingService` from the engine's transparent draw. `direct_samples(position,
normal)` enumerates applicable point lights and the sun, sharing sampling helpers
with base shading; `indirect(position, normal)` shares its hemisphere inputs.
Contributed draws currently enumerate all 64 point-light slots even in Forward+;
inactive slots and samples outside a light's volume produce no yielded sample.
The local `offscreen --lighting-service-only` GPU probe verifies independent
indirect, sun, point-light, radius, and shadow mutations in contributed draws,
including a separate opaque shadow caster and preserved ambient illumination.
All three renderers agree. Full MeadowFur acceptance remains pending.

Pass-local shader helpers may return `iterator<T>` and produce elements with
`yield(value)`. Consumers use `for element in helper(arguments) { ... }`, including
typed service method calls. Lowering expands the helper at the loop, evaluates
arguments once, and renames helper locals to preserve lexical isolation. No array
of light samples is allocated. Every yield is typechecked, including unused service
exports. Iterators must be consumed directly by `for`; they cannot be stored or
passed as shader values. This initial implementation supports conditional yields,
range loops, and falling through to completion, but rejects iterator early returns,
`break`, and imported/global iterator functions. Expansion is limited to 256 calls;
recursive operation/service helpers are rejected.

### C. Renderer providers and graph obligations

This proposed provider syntax names graph nodes/resources from an engine-owned
recipe. `all(...)` denotes a view-wide completion join, not a per-object anchor.
Point outputs flow into the next named consumers; they do not update an unused
copy of scene color. Integration-owned attachment sequencing supplies those edges.

```text
provide StandardStyle for Forward {
    PreparedGeometry = prepare_selected_material_ranges
    SurfaceUV = interpolated_prepared_uv
    DrawShadingResources = draw_instance_binding_table
    ForwardLighting = forward_light_service
    view = current_view

    after_opaque {
        after: all(opaque_forward_draws)
        color: opaque_hdr
        depth: opaque_depth
        order: stable_draw_order
        next: transparent_queue
    }
    transparent {
        queue: transparent_queue
        color: after_opaque.color_out
        depth: opaque_depth
        order: back_to_front_view_depth_then_stable_draw_key
        next: inspection, presentation
    }
}

provide StandardStyle for Deferred {
    PreparedGeometry = prepare_selected_material_ranges
    SurfaceUV = gbuffer_surface_uv
    DrawShadingResources = gbuffer_shading_instance_id_to_binding_table
    ForwardLighting = forward_light_service
    view = current_view

    after_opaque {
        after: all(opaque_gbuffer_draws), lighting_resolve
        color: resolved_hdr
        depth: gbuffer_depth
        order: stable_draw_order
        next: transparent_queue
    }
    transparent {
        queue: transparent_queue
        color: after_opaque.color_out
        depth: gbuffer_depth
        order: back_to_front_view_depth_then_stable_draw_key
        next: inspection, presentation
    }
}
```

These are target providers, not claims that today's example engine already
exposes these resources. Deferred now preserves f32 surface UVs and a distinct
draw-instance ID. Its per-instance resolve uses that identity to select pixels and
bind the draw's captured resources; material and style settings retain their
separate table indices. `SurfaceUV` and `DrawShadingResources` are implemented;
`ForwardLighting` and the transparent queue remain Phase 8 work. UV
lookup equivalence is tested within one display-output byte, including a mutation
that demonstrates half-float UV storage is insufficient for the precision probe.
Forward+ uses the Forward provider structure with its tiled light enumeration.

An engine stable draw key orders object instances and material ranges independently
of source/import order, with an operation-local ordinal as a final tie-breaker.
Transparent submissions supply a world-space bounds-center sort position. All
ordinary and contributed transparent draws sort together back to front in view
space; ties use that stable key. Sorting draw centers is approximate for intersecting
or enclosing shells. The engine must advertise this limitation, not claim exact
order-independent transparency. A renderer with no compatible transparent queue
rejects MeadowFur while remaining compatible with Toon.

### D. Toon: local draw implementation, style, and assignments

These declarations belong together in the proposed version of
`style_sample.fr`. The outline is implemented locally, not supplied
by the compiler. The standard-library `normalize_or_zero` and `normalize_or`
helpers supply explicit degenerate-vector behavior; no local normalization helper
is needed. Normal and camera preparation remain engine responsibilities.

```text
draw InvertedHull(
    geometry: PreparedMesh,
    view: View,
    width: f32,
    ink: color,
    color: attachment<rgba16float, preserve_update>,
    depth: attachment<depth32float, test_only>
) {
    requires material.blend == SurfaceBlend.Opaque && !material.two_sided
    raster geometry
    visibility: uncullable
    cull: front
    depth { compare: less_equal; write: false }
    blend { all: replace }
    attachments { color: load_store; depth: load_store }

    @vertex fn vertex(vertex_index: u32) -> clip_position {
        let v = geometry.vertices[vertex_index]
        var clip = view.projection_matrix * view.view_matrix
                 * vec4(v.world_position, 1.0)
        let normal = (view.view_matrix * vec4(v.geometric_normal, 0.0)).xyz
        let direction = (view.projection_matrix * vec4(normal, 0.0)).xy
                      * view.viewport_size
        let length_squared = dot(direction, direction)
        if length_squared > 0.000001 && clip.w > 0.000001 {
            let offset = direction * inverse_sqrt(length_squared)
                       * (2.0 * width / view.viewport_size)
            clip.x += offset.x * clip.w
            clip.y += offset.y * clip.w
        }
        return clip
    }

    @fragment fn fragment() -> vec4 {
        return vec4(ink.rgb, 1.0)
    }
}

style Toon for standard : StandardStyle {
    static param outline_enabled: bool = true
    param outline_width: f32 in [0.0, 16.0] = 2.5
    param outline_color: color = #2e1938
    param highlight_strength: f32 in [0.0, 1.0] = 0.3

    fn direct(s: StandardSurface, c: ShadingContext, light: DirectLight) -> vec3 {
        let nl = dot(s.normal, light.direction)
        if nl <= 0.0 { return vec3(0.0) }
        let bands = smoothstep(0.0, 0.04, nl)
                  * (0.15 + 0.35 * smoothstep(0.23, 0.27, nl)
                          + 0.50 * smoothstep(0.63, 0.67, nl))
        let h = normalize_or_zero(c.view_direction + light.direction)
        let spec = smoothstep(0.55, 0.65,
            pow(max(dot(s.normal, h), 0.0), mix(96.0, 4.0, s.roughness)))
        let response = s.albedo * (1.0 - s.metallic) * bands / 3.14159265
                     + mix(vec3(0.04), s.albedo, s.metallic)
                       * spec * highlight_strength
        return response * light.radiance * light.attenuation * light.visibility
    }

    fn finish(s: StandardSurface, c: ShadingContext, light: LightingResult) -> vec3 {
        return light.direct + light.indirect + s.emissive
    }

    static if outline_enabled {
        requires PreparedGeometry
        for self {
            at after_opaque as target {
                InvertedHull(
                    geometry: mesh.prepared, view: view,
                    width: outline_width, ink: outline_color,
                    color: target.color, depth: target.depth
                )
            }
        }
    }
}

surface peach_toon(sp: surf) -> material(standard) {
    properties { style: Toon(outline_width: 2.5) }
    compose { base(albedo: #f5ad69, roughness: 0.45, metallic: 0.0) }
}

surface blue_toon(sp: surf) -> material(standard) {
    properties { style: Toon(outline_width: 4.0, outline_color: #102840) }
    compose { base(albedo: #608cce, roughness: 0.7, metallic: 0.0) }
}
```

Both materials may share pipelines and static specialization while retaining
independent settings records. With `outline_enabled: false`, the style requires
no prepared-geometry contribution capability and does not impose the outline's
opaque/single-sided requirement. The shader hooks still obey the selected engine
coverage contract. With outlines enabled, missing or incompatible inputs fail.

The degeneracy guard avoids division by zero; it is not a claim of perfect
near-plane silhouettes. The example assumes closed smooth meshes with suitable
geometric normals. It intentionally does not repair topology or hard edges.

### E. MeadowFur: density compute and generated shell geometry

Fur has an opaque root plus translucent generated shells. Density also darkens
the root response, exercising a compute output read by base shading in both
Forward and Deferred. All preparation uses the same deformed material range.
There is no persistent simulation state: wind is a bounded function of time.

```text
// Procedural shell fur: opaque roots, per-draw density, and lit translucent shells.
// Shells receive scene shadows but do not cast additional shell shadows.
struct FurVertex { world_position: vec3; normal: vec3; uv: vec2; height: f32 }
struct FurVarying {
    @semantic(position) clip: vec4
    world_position: vec3
    normal: vec3
    uv: vec2
    height: f32
}

fn density_hash(cell: uvec2, seed: u32) -> f32 {
    var x = wrapping_add(wrapping_mul(cell.x, 1664525u), cell.y)
    x = wrapping_add(x, seed)
    x = x ^ (x >> 16u)
    x = wrapping_mul(x, 2246822519u)
    x = x ^ (x >> 13u)
    return f32(x & 65535u) / 65535.0
}

compute FurDensity(resolution: u32, seed: u32) -> texture2d<r32float, read> {
    requires resolution >= 1u && resolution <= 512u
    output field: texture2d<r32float, write>(resolution, resolution)
    workgroup_size: (8, 8, 1)
    dispatch threads(resolution, resolution, 1u)
    @compute fn main(id: uvec3) {
        if id.x >= resolution || id.y >= resolution { return }
        field.store(id.xy, density_hash(id.xy, seed))
    }
    return field
}

compute FurShellVertices(geometry: PreparedMesh, layers: u32, length: f32, wind: vec3, time: f32) -> buffer<FurVertex, read> {
    requires layers >= 1u && layers <= 32u
    requires length >= 0.0
    output vertices: buffer<FurVertex, write>(checked_mul(geometry.vertex_count, layers))
    workgroup_size: (64, 1, 1)
    dispatch threads(checked_mul(geometry.vertex_count, layers), 1u, 1u)
    @compute fn main(id: uvec3) {
        let count = geometry.vertex_count * layers
        if id.x >= count { return }
        let layer = id.x / geometry.vertex_count
        let index = id.x % geometry.vertex_count
        let source = geometry.vertices[index]
        let height = f32(layer + 1u) / f32(layers)
        let bounded_wind = wind / max(1.0, sqrt(dot(wind, wind)))
        let bend = bounded_wind * sin(time + source.world_position.x) * length * height * height
        vertices[id.x] = FurVertex(source.world_position + source.geometric_normal * length * height + bend,
                                  source.geometric_normal, source.uv, height)
    }
    return vertices
}

fn fur_response(albedo: vec3, normal: vec3, light: DirectLight) -> vec3 {
    let wrapped = clamp((dot(normal, light.direction) + 0.4) / 1.4, 0.0, 1.0)
    return albedo * wrapped / 3.14159265 * light.radiance * light.attenuation * light.visibility
}

draw FurShell(geometry: PreparedMesh, vertices: buffer<FurVertex, read>, layer: u32, layers: u32, length: f32,
              density: texture2d<r32float, read>, density_sampler: sampler, tint: vec3, opacity: f32,
              view: PreviewScene, lights: LightingService,
              color: attachment<rgba16float, preserve_update>, depth: attachment<depth32float, test_only>) {
    requires vertices.count == checked_mul(geometry.vertex_count, layers)
    requires layer < layers
    raster geometry using vertices base_vertex checked_mul(layer, geometry.vertex_count)
    visibility: geometry.bounds.expand_world(2.0 * length)
    sort_position: geometry.bounds.center
    cull: back
    depth { compare: less_equal; write: false }
    blend { all: premultiplied }
    attachments { color: load_store; depth: load_store }
    @vertex fn vertex(index: u32) -> FurVarying {
        let v = vertices[index]
        return FurVarying(view.proj * view.view * vec4(v.world_position, 1.0), v.world_position, v.normal, v.uv, v.height)
    }
    @fragment fn fragment(input: FurVarying) -> vec4 {
        let density_value = density.sample_level(density_sampler, input.uv, 0.0).r
        if density_value < input.height { discard_fragment() }
        let alpha = clamp(opacity * (1.0 - 0.8 * input.height), 0.0, 1.0)
        let normal = normalize_or(input.normal, vec3(0.0, 1.0, 0.0))
        var rgb = tint * lights.indirect(input.world_position, normal).irradiance
        for light in lights.direct_samples(input.world_position, normal) {
            rgb += fur_response(tint, normal, light)
        }
        return vec4(rgb * alpha, alpha)
    }
}
```

`checked_mul` on graph extents is host-validated before dispatch, including u32
representability, byte size, and device limits. Therefore the bounded shader
product is valid. Empty geometry dispatches zero threads and submits no draws,
so division by zero is never executed. This proof applies to this operation;
the compiler does not generally prove arbitrary shader indexing.

The maximum root-to-shell displacement is at most `2 * length`: normal extrusion
is bounded by length and wind bend by length. Normals remain the root normals,
an explicit approximation for this decorative shell model. The technique receives
scene lighting and existing shadows but does not cast extra shell shadows or
simulate inter-strand/self shadowing. Shell color uses its own tint parameter;
it does not silently claim to replay arbitrary surface layers on displaced geometry.

### F. MeadowFur style and material use

```text
style MeadowFur for standard : StandardStyle {
    requires PreparedGeometry, SurfaceUV, DrawShadingResources, ForwardLighting
    requires transparent
    requires blend == SurfaceBlend.Opaque && !two_sided
    static param layers: u32 in [1, 32] = 12u
    static param density_resolution: u32 in [1, 512] = 128u
    param seed: u32 = 7u
    param fur_length: f32 in [0.0, 0.25] = 0.025
    param fur_tint: color = #8c6239
    param shell_opacity: f32 in [0.0, 1.0] = 0.22
    param wind: vec3 = vec3(0.3, 0.0, 0.1)
    shading_input density: texture2d<r32float, read> scope draw
    shading_input density_sampler: sampler scope draw
    fn direct(surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 {
        let density_value = density.sample_level(density_sampler, surface.uv, 0.0).r
        return fur_response(surface.albedo * mix(0.6, 1.0, density_value), surface.normal, light)
    }
    for self {
        let field = FurDensity(resolution: density_resolution, seed: seed)
        bind shading.density = field
        bind shading.density_sampler = nearest_repeat
        let vertices = FurShellVertices(geometry: mesh.prepared, layers: layers, length: fur_length, wind: wind, time: frame.time)
        at transparent as target {
            static for ordinal in 0u..layers {
                let layer = layers - 1u - ordinal
                FurShell(geometry: mesh.prepared, vertices: vertices, layer: layer, layers: layers, length: fur_length,
                         density: field, density_sampler: nearest_repeat, tint: fur_tint.rgb, opacity: shell_opacity,
                         view: frame, lights: lights, color: target.color, depth: target.depth)
            }
        }
    }
}

surface chestnut_fur(sp: surf) -> material(standard) {
    properties { style: MeadowFur(layers: 12u, fur_length: 0.025, fur_tint: #8c6239) }
    compose { base(albedo: #69482f, roughness: 0.9, metallic: 0.0) }
}
surface pale_fur(sp: surf) -> material(standard) {
    properties { style: MeadowFur(layers: 20u, fur_length: 0.06, fur_tint: #ddd3b9) }
    compose { base(albedo: #b5ac93, roughness: 1.0, metallic: 0.0) }
}
```

Density is deliberately draw-scoped to exercise per-draw resource binding. A later
explicit material-scoped declaration could share this particular field across
objects because its inputs are material settings only. The compiler must not infer
sharing of mutable outputs merely from equal settings. Each view/frame invocation
initially computes its own transient field; caching is not promised.

Fur shell operations use the explicitly bound lighting service rather than
recursively invoking the style's base shading. Recursive graph/style invocation
is not part of this proposal. Both root and shells use the same `fur_response`
helper, but their inputs and coverage behavior differ intentionally.

### G. Concrete scene expansion and acceptance cases

Consider two views, a peach Toon object, a second Toon object using blue settings,
one chestnut fur object, and one PBR object. A mixed-material object additionally
has one Toon range and one PBR range. For each view:

1. The engine creates shading-instance records for each object/material range and
   binds independent material settings. It prepares ranges whose selected styles
   require prepared geometry; base and contribution consumers share each result.
2. Fur density compute can begin when its settings and allocation are ready. Fur
   shell vertex compute waits for prepared geometry and time/settings, not opaque
   completion. The two computes have no dependency on each other.
3. Base Forward shading waits for density where the Fur hook reads it. Deferred
   geometry emits UV and shading-instance identity; its resolve waits for density
   and looks up the correct record. Geometry itself need not wait for density.
4. All opaque depth and lighting complete before the ordered Toon shell draws.
   The PBR object and PBR range have no outline draws. Toon outputs become the
   color input to the global transparency queue.
5. Fur shell submissions join ordinary transparency. They wait for generated
   vertices, density, scene lighting inputs, opaque depth, and preceding Toon work.
   The queue supplies one total order for preserving color writes.
6. Inspection and presentation consume the color version after all relevant work.
   Resources for different views/frames are logically distinct; allocation reuse
   obeys completion and lifetime analysis.

```text
fur settings --------> density --------------------------> fur base shading
                          |                                     |
prepared fur mesh ---> shell vertices                           |
                          |                                     v
                          |                       complete opaque depth + lighting
                          |                                     |
                          |                                Toon draws
                          |                                     |
                          +--> globally ordered transparent fur + ordinary draws
                                      ^                         |
                                      |                         v
                                   density             inspection / presentation
```

Structural tests assert edges, absence of unnecessary edges, attachment versions,
and instance bindings, not a saved command list. Render tests cover independent
settings, mixed material ranges, transforms, two cameras, ordinary transparent
geometry, and ordinary opaque occluders. Provider tests reject missing UV, missing
instance-resource lookup, incompatible sample counts, and missing transparency.

### H. What the second style exposed, and deliberate limits

| Pressure from MeadowFur | Resolution or explicit limit |
| --- | --- |
| Compute influences original shading as well as extra draws | Typed shading slots and producer edges; Deferred needs UV plus shading-instance lookup |
| Generated vertex streams keep original topology | Checked stream length, local indexed range, explicit base vertex per shell |
| Several translucent draws share scene color | Global engine transparency queue, not a private queue per style |
| Expansion affects visibility | Declared conservative world bounds; Toon instead requests uncullable contribution work |
| Extra geometry needs lights without a renderer-specific shader | Shader-callable ForwardLighting capability with explicit bindings |
| Shell appearance differs from root material | Explicit shell tint and shared response helper; no automatic replay of surface layers |
| Temporal-looking wind could imply persistent simulation | Stateless bounded displacement only; no history or integration across frames |
| Multiple material/view instances could alias resources | Explicit invocation keys and frame-transient ownership |
| More compute does not imply it must run late | Producers scheduled from their own inputs; only shell draws join the late queue |
| Sorted transparency is insufficient for all hair | Approximate shell sorting is documented; OIT/strand rendering needs another capability |

Before compiler implementation, map proposed buffer/texture/resource slot types to
the existing IR and settle their backend binding representation. In particular,
Deferred per-instance texture lookup may require an engine atlas/array scheme or
another supported binding strategy; arbitrary bindless indexing is not assumed.
The provider must expose and validate its supported capacity. If this cannot be
implemented on a target, MeadowFur is unsupported there until a compatible provider
exists; the compiler must not substitute a material-global texture.

The examples replace empty operation bodies with actual algorithms and identify
the engine services they require. They are a proposed specification to validate,
not evidence that the current compiler can lower every declaration. Remaining
implementation choices include lowering/binding representation, syntax integration
with existing declarations, and platform capacity. New feature work
such as persistent simulation, exact hair transparency, extra-geometry shadow
casting or automatic layer replay is explicitly outside this initial specification
and must not be implied by the demos. Phase 11 adds the bounded resource-port
placement described above; it does not infer arbitrary engine phase semantics.
