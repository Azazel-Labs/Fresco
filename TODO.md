# Fresco active roadmap

Updated 2026-09-23. This is the work queue; detailed contracts live in linked plans.
The [documentation index](docs/README.md) separates references, proposals, and history.

For execution validation, see the
[hardware GPU regression checks](crates/fresco-wasm/web/tests/gpu/README.md).

Style implementation tracking lives in the [phase checklist](docs/standard-shading-styles-design.md#phase-based-implementation-plan).
Phases 0-11 are complete, including retirement of the authoring adapters and browser
fallback, typed resource ports, and validated resource-driven placement. Ordinary interfaces, material
schema profiles, engine passes, and separately documented particle/canvas contracts
are retained, not pending deletion.

## 1. Continue focused compiler work

These are retained workstreams, not an assertion that every item is unimplemented.
Reproduce the gap and consult existing tests before resuming an older task.

| Workstream | Owning plan |
| --- | --- |
| Engine integration syntax, render-graph explanation, and optional runtime/C ABI | [Engine integration follow-up](#engine-integration-follow-up) |
| Compiler/runtime source-of-truth correctness and consolidation | [Active checklist](#compiler-runtime-source-of-truth) |
| Shader size, helper sharing, temporal evaluation | [Lowering scalability tasks](#lowering-scalability) |
| Runtime path geometry and arc-length accuracy | [Path follow-up tasks](#path-geometry-and-arc-length) |
| Compile-time templates and bounded unrolling | [Const-template follow-up](#const-templates-and-bounded-unrolling) |
| Anisotropic footprint handling | [AA investigation](#anisotropic-aa-and-thin-feature-filtering) |
| Pass partitioning, materialization, feedback, and sampled spaces | [Current execution limits](LANGUAGE.md#host-integration-obligations) |

<a id="engine-integration-follow-up"></a>

## Engine integration syntax and optional runtime

Committed follow-up after the completed styles work. The syntax and documentation
need improvement; the runtime packaging and VM architecture below require a design
decision. Keep current shipped behavior documented in `LANGUAGE.md` while this work
is designed and implemented.

### Structured engine authoring

- [ ] Rework engine-integration syntax into coherent, discoverable declarations and
  blocks. The current accumulation of `@` attributes obscures structure, ownership,
  and composition; avoid merely renaming attributes or adding another parallel DSL.
- [ ] Inventory current contracts, providers, resources, passes, pipelines, factories,
  bindings, render state, and ordering annotations. Define which are language
  constructs, which are engine-defined extension points, and how they compose.
- [ ] Demonstrate the proposed syntax with complete Canvas, surface/style, and particle
  engine definitions, including multiple renderers and compute-to-draw dependencies.
  Preserve typed validation, diagnostics, and the existing execution guarantees.
- [ ] Plan migration and retirement of replaced syntax; prove equivalent compiled
  graphs and rendering before updating examples and removing legacy forms.

### Explain the resulting render graph

- [ ] Add an end-to-end explanation in `LANGUAGE.md`: authored declarations to the
  emitted executable graph, then host execution. Explain nodes, resource versions,
  dependencies, scheduling, synchronization, lifetime, and per-view/object/material
  invocation identity, with concrete diagrams and corresponding artifact excerpts.
- [ ] Explain what the compiler proves, what remains a runtime decision, and what an
  engine must supply. Distinguish descriptive planning metadata from executable
  work; show early compute, opaque completion, contribution composition, sorted
  transparency, and presentation in one worked frame.

### Optional reusable runtime and C/C++ integration

- [ ] Design a runtime component an engine can choose to embed instead of implementing
  graph execution itself. Preserve direct consumption of compiler artifacts for
  engines that own their scheduler and resource management.
- [ ] Choose a clear package name and boundary. Reconsider the suggested
  `fresco-assets` name; the current `fresco-artifact` crate describes artifacts, and
  artifact schemas/assets and runtime execution have different responsibilities.
  Decide whether execution belongs alongside that crate or in a separate package.
- [ ] Evaluate a small VM/interpreter for the emitted execution graph against direct
  graph execution. Specify its instruction/operation model, validation, artifact
  versioning, scheduling, error reporting, and host/backend interface before choosing
  it. A VM is a candidate architecture, not an already committed implementation.
- [ ] Define the division of responsibility for GPU commands, resource allocation,
  synchronization, frame lifetimes, and device capabilities. Keep the optional
  runtime usable by external engines without requiring the example engine or a
  single graphics backend.
- [ ] Make C and C++ consumption a design requirement from the start: provide a stable
  C ABI with generated headers/bindings and, if useful, a thin C++ wrapper. Specify
  opaque handles, ownership/destruction, allocator and callback lifetimes, thread
  rules, error transport, version negotiation, and static/shared-library packaging;
  do not expose Rust layouts or let panics cross the ABI.
- [ ] Prove the optional runtime with standalone C and C++ integration examples that
  load a compiled artifact, supply host resources, update parameters, execute a
  frame, report failures, and release resources. Compare their behavior with the
  Rust integration using Canvas, styles with compute/draw work, and particles.

## Future work to retain

- Engine shading models, vertex stages, permutation validation, and resource capabilities.
- Broader rewrite matching and numeric self-verification.
- Stateful signals and general compute/grid interop.
- [Particle simulation and extensible module state](LANGUAGE.md#particles): effect-authored recycling emitters, camera-facing draw modules, reflected state layout, generic motion modules, and explicit temporary records are implemented; automatic record composition and general lifecycle allocation are next.
- Color/type refinements, derivatives, morphing, and vector asset import.
- Workbook probes, sweeps, comparisons, and compiler-cost inspection.

The [language design](LANGUAGE.md) links to each detailed proposal.
Older task histories remain in Git history; revalidate their status claims before
turning them into new work.

## Completion rule

Fix the owning compiler/runtime layer, add coverage for the actual failure, and
record relevant validation. Parsing a declaration or accepting an example does
not establish correct execution. Keep detailed implementation history in dated
assessments rather than growing this queue into another design document.

<a id="compiler-runtime-source-of-truth"></a>

## Compiler/runtime source-of-truth

Status: committed investigation and resolution work, 2026-09-23.
Commitment means resolving each finding with evidence, not implementing every
original suggested fix regardless of reproduction results.
Converted from the 2026-09-22 source inspection. This is a work plan, not a list
of ten reproduced bugs and not a future-language proposal. No fixes or GPU
reproductions are claimed by this conversion.

[LANGUAGE.md](LANGUAGE.md) owns shipped contracts; this section tracks the unresolved
investigations, consolidation work, and acceptance evidence.

### Classification and completion rules

- **Confirmed contract mismatch:** conflicting facts are visible in current source;
  this does not by itself prove a particular rendered failure.
- **Suspected defect:** establish a minimal reachable reproduction before changing
  behavior. If disproved or already fixed, record the evidence and close that claim.
- **Cleanup:** duplication is a maintenance concern, not automatically a bug.
  Preserve intentional differences between consumers.

`[x]` means the specific action has evidence; `[ ]` means it remains open.
Complete an issue only after its applicable regressions and validation pass, or
close it explicitly as disproved/superseded with evidence. Record commands/results,
remaining limits, and links to fixes in that issue's completion record. Do not
weaken tests, remove diagnostics, or replace invariant failures with fallbacks.

Conversion spot checks confirmed the 16-versus-64 visualizer capacities, duplicate
`layer_inputs` implementations, and differing Naga capability configurations.
Other findings below retain their original source-inspection status. Source line
numbers in inherited evidence are historical search hints, not current locations.
In particular, schema typing has changed since the audit and must be rechecked.

### Ownership constraints

Fresco owns language names, alias resolution, and semantics. Naga owns its IR
rules/layout/validation; selected backends own output-language representability
and version/shader-model restrictions. Hosts own actual device support. Artifacts
carry resolved facts across boundaries. Revalidating an external artifact is
useful; independently reimplementing its semantics is the problem.

Fresco is multi-target. WGSL is the frequently exercised output, not its capability
ceiling. Portability profiles are explicit policy, not an implicit WebGPU baseline.
Device restrictions are separate unless a device profile is requested. Prefer
shared typed IR followed by target-specific emission/validation; adding another
output enum does not remove upstream WGSL assumptions.

### Work order and tracker

Start by reproducing issues 1-3, then fix established correctness problems at their
owning layers. Coordinate capability work (4-5) around one explicit ownership
model. Tackle the smaller vocabularies/utilities after their current semantics are
known. This ordering is not a deadline or an assertion that all investigations
will require changes.

| ID | Workstream | Classification | Status |
| --- | --- | --- | --- |
| 1 | [Schema expression typing](#source-of-truth-issue-1) | Suspected defect; original failure needs revalidation | Open |
| 2 | [Vertex inheritance resolution](#source-of-truth-issue-2) | Suspected defect; inconsistent resolution needs a reachable reproduction | Open |
| 3 | [Visualizer ABI](#source-of-truth-issue-3) | Confirmed contract mismatch; rendered impact not yet reproduced | Open |
| 4 | [Runtime capability validation](#source-of-truth-issue-4) | Cleanup and suspected usage-policy defect | Open |
| 5 | [Shader validation profiles](#source-of-truth-issue-5) | Suspected defect; differing profiles are not inherently wrong | Open |
| 6 | [Vertex encoding vocabulary](#source-of-truth-issue-6) | Cleanup; no packing defect established | Open |
| 7 | [Typed entry reflection](#source-of-truth-issue-7) | Suspected defect; fragile parsing needs a failing input | Open |
| 8 | [HIR traversal and analyses](#source-of-truth-issue-8) | Cleanup; duplicate traversal confirmed | Open |
| 9 | [Raster state vocabulary](#source-of-truth-issue-9) | Cleanup; no vocabulary mismatch established | Open |
| 10 | [Primitive metadata and builtin catalog](#source-of-truth-issue-10) | Cleanup; catalog reachability remains unverified | Open |

<a id="source-of-truth-issue-1"></a>

### 1. Schema expression typing

**Classification:** Suspected defect; original failure needs revalidation.

#### Original evidence and proposed direction (2026-09-22)

Evidence: `crates/fresco/src/driver/schema_function.rs:196`, especially
lines 290-345 and the overloaded-call filtering at lines 400 onward.
`value_type` manually identifies scalar-returning functions and maintains a
list of functions whose result is assumed to have their first argument's
type. Native functions outside that list, such as `log2`, reach the
"cannot infer result" error when their result is needed for overload
selection. GPU emission otherwise passes ordinary call names through
(`driver/gpu_function.rs:320`). Later Naga validation cannot repair an
overload rejected before emission.

Prefer carrying checked expression types into this stage, or obtaining
native expression types through Naga IR before selecting Fresco overloads.
Do not require a WGSL source round-trip to type expressions for every target.
Keep Fresco alias resolution in one place. Moving the list into a table
alone would preserve the underlying duplicate type system.
Regression: overloads accepting scalar/vector arguments, called with
nested native builtin results, including a builtin absent from this list.
The failure path is evident in source; an end-to-end fixture is still needed.

#### Actions and acceptance

- [ ] Reproduce nested builtin-result overload selection (scalar/vector, including `log2`) on current code; record passing cases if the original failure is already fixed.
- [ ] Trace current type inference, including any Naga/text round trips added since the audit; choose checked-expression types or shared typed IR as the authority.
- [ ] Remove redundant builtin inference/alias decisions without making WGSL reparsing mandatory for other targets.
- [ ] Add positive and negative overload regressions covering nested calls and ambiguity; verify target-independent typing.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-2"></a>

### 2. Vertex inheritance resolution

**Classification:** Suspected defect; inconsistent resolution needs a reachable reproduction.

#### Original evidence and proposed direction (2026-09-22)

Evidence: `check.rs:757`, `check/declarations.rs:2353`,
`driver/emit.rs:86`, and `driver/mesh_pass.rs:434`, under
`crates/fresco/src`. The checker implementations append inherited and
local members; emission implementations replace inherited members with
matching names in place. Cycle handling also differs between helpers.

Resolve ordered members once in checked compiler metadata and use that
result for conformance, locations, manifest offsets, and shader declarations.
Whether duplicate inherited names can currently reach both behaviors needs
a targeted reproduction; the contradictory rules themselves are confirmed.
Regression: multi-level inheritance, same-name members, missing parents,
cycles, and agreement between shader locations and manifest attributes.

#### Actions and acceptance

- [ ] Compare current inheritance resolvers and reproduce multi-level and same-name member behavior, including missing parents and cycles.
- [ ] Define override/duplicate semantics explicitly, then produce one checked ordered member representation.
- [ ] Use it for conformance, shader locations, manifest attributes, offsets, and stride.
- [ ] Prove shader/manifest agreement and rejection behavior with inheritance mutation tests.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-3"></a>

### 3. Visualizer ABI

**Classification:** Confirmed contract mismatch; rendered impact not yet reproduced.

#### Original evidence and proposed direction (2026-09-22)

Evidence: `crates/fresco/src/driver/viz.rs:10` declares 16 runtime scalar
slots, while `crates/fresco-wasm/web/src/preview/uniforms.ts:1` declares 64.
Rust emits four parameter vec4s; the browser allocates sixteen.
`driver/viz.rs:799` generates argument slots without a corresponding
capacity check. `web/src/visualizer-renderer.ts:54` uses the browser buffer
for compiled visualizers. Series sample counts are separately hardcoded
to 256 in compiler and browser as well.

Export a visualizer ABI descriptor containing buffer size, offsets,
parameter capacity/layout, and series length. Both allocation and codegen
should consume it. An oversized host buffer alone is not a GPU error;
the concern is parameters beyond the shader's declared capacity and future
independent changes. Reproduce with more than 16 scalar slots and mixed
scalar/color parameters, checking generated accesses and rendered values.

#### Actions and acceptance

- [ ] Reproduce visualizers above 16 scalar slots, including mixed scalar/color parameters, and record emitted accesses and runtime behavior.
- [ ] Define one reflected visualizer ABI: byte size, offsets, capacity, parameter layout, and series length.
- [ ] Make compiler emission and browser allocation consume that contract; diagnose unsupported capacity before shader submission.
- [ ] Test capacity boundaries and series length agreement; run a local rendered probe for actual parameter values.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-4"></a>

### 4. Runtime capability validation

**Classification:** Cleanup and suspected usage-policy defect.

#### Original evidence and proposed direction (2026-09-22)

Evidence: native runtime `targets.rs:43`, `recipe.rs:685`, and
`owned_compute.rs:198` each combine required features and allowed usages.
These correctly consult wgpu, but independently. Intermediate targets
additionally require filtering; owned compute needs storage use. Recipe
image creation at `recipe.rs:679` requests attachment, sampling, and copy
usage together rather than deriving each image's usages from its consumers.

Introduce a runtime capability context plus a shared validator receiving
actual required usages, sample count, filter/blend requirements, and storage
access. Keep usage-specific requirements explicit rather than giving every
image the same union. In the wgpu host, retain adapter format queries and
honor device-enabled features. Offer a portable WebGPU profile only as an
explicit user/host policy; do not apply it to Fresco compilation generally.

`TextureFormat::guaranteed_format_features` describes WebGPU guarantees,
not every adapter-specific capability. Adapter-specific format features
require the corresponding enabled wgpu feature. Naga is not a device probe.
Sources: [wgpu format methods](https://docs.rs/wgpu/30.0.1/wgpu/enum.TextureFormat.html)
and [adapter format queries](https://docs.rs/wgpu/30.0.1/wgpu/struct.Adapter.html#method.get_texture_format_features).
Regression: portable versus adapter-specific support, disabled required
features, and the same format under different actual usages.

#### Actions and acceptance

- [ ] Inventory current allocation validators and derive required usages from actual consumers; establish a failing or unnecessarily rejected usage case.
- [ ] Introduce a shared host capability context/validator with explicit usage, sample count, filter/blend, and storage-access requirements.
- [ ] Keep mandatory backend constraints, optional portability policy, and device-enabled features separate.
- [ ] Test identical formats with different usages, disabled features, and portable versus adapter-specific capabilities.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-5"></a>

### 5. Shader validation profiles

**Classification:** Suspected defect; differing profiles are not inherently wrong.

#### Original evidence and proposed direction (2026-09-22)

Evidence: `crates/fresco/src/driver/mesh_pass/shader.rs:532` and
`lower/compute.rs:26` validate with `Capabilities::all()`, while combined
emission in `driver/emit.rs:546` uses `Capabilities::default()`.
Similar constructor calls occur across visualizer and runtime paths.

Pass an explicit target configuration through the pipeline. Separate valid
shared IR, representability in the selected output language/version, and
compatibility with an actual device. Do not mechanically replace all/default
with one blanket choice. Mandatory language constraints belong to backend
validation; optional portability restrictions belong to user-selected
profiles; device checks belong to hosts. Reflect required capabilities so
hosts can make that decision. Naga's flags are allowed IR capabilities:
[Naga validation API](https://docs.rs/naga/30.0.1/naga/valid/index.html).
Regression: a feature-bearing module validated alone and after composition.

#### Actions and acceptance

- [ ] Inventory validation sites and document why each uses its current capability set.
- [ ] Reproduce a feature-bearing module validated alone versus after composition; classify any mismatch against intended target policy.
- [ ] Thread explicit target configuration and reflect host-required capabilities instead of replacing all/default mechanically.
- [ ] Test composition and selected-target restrictions without imposing an implicit WebGPU portability ceiling.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-6"></a>

### 6. Vertex encoding vocabulary

**Classification:** Cleanup; no packing defect established.

#### Original evidence and proposed direction (2026-09-22)

Evidence: `crates/fresco/src/driver/emit.rs:114` maps types to format names
and bytes; native `runtime/vertices.rs:47` maps those names to scalar kinds
and component counts; `runtime/mesh_geometry.rs:23` maps names to wgpu enums.

Put the supported Fresco vertex encodings in a typed artifact vocabulary,
with one conversion to wgpu. Derive backend byte sizes from wgpu where
available. Keep host packing/conversion policy explicit: shader type alone
cannot choose between normalized, packed, and full-width vertex encodings.
Regression: every emitted encoding can be packed and produces matching
wgpu attribute size, offset, and stride.

#### Actions and acceptance

- [ ] Inventory emitted vertex encodings and their compiler/host packing mappings.
- [ ] Define one typed artifact encoding vocabulary with exhaustive backend conversion, retaining explicit packing/normalization policy.
- [ ] Use authoritative backend size/layout facts where applicable.
- [ ] Test every supported encoding for packability and agreement on size, offset, and stride; reject unknown encodings.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-7"></a>

### 7. Typed entry reflection

**Classification:** Suspected defect; fragile parsing needs a failing input.

#### Original evidence and proposed direction (2026-09-22)

Evidence: `crates/fresco/src/driver/viz.rs:870` finds parameter lists with
string searches and splits on commas. Browser `web/src/wgsl-entry.ts:22`
uses a regex and comma splitting, then guesses an entry from its signature
and generated name. Prefix recognition is repeated in Rust at `viz.rs:976`
and browser `preview/helpers.ts:39`.

Carry the selected entry and its typed parameters/context through artifact
metadata. For genuinely arbitrary WGSL, use Naga reflection on the Rust
side. Commas inside array/generic types and attributed parameters are
structural weaknesses of the current text parsers. The browser detector's
observed caller is annotation-sweep reporting, so its direct impact is
narrower than the compiler-side visualizer argument generation.
Regression: array parameters, struct contexts, multiple entries, and
generated helper functions preceding the requested entry.

#### Actions and acceptance

- [ ] Identify live text-parsing consumers and reproduce array/generic commas, attributed parameters, multiple entries, and preceding helper functions.
- [ ] Carry selected entry identity and typed parameters/context in artifact metadata.
- [ ] Replace generated-text inference; for arbitrary external shader input, use structural reflection appropriate to its target.
- [ ] Test exact entry selection and argument generation, including the annotation-sweep reporting path.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-8"></a>

### 8. HIR traversal and analyses

**Classification:** Cleanup; duplicate traversal confirmed.

#### Original evidence and proposed direction (2026-09-22)

Evidence: identical exhaustive `layer_inputs` matches in
`crates/fresco/src/check/declarations.rs:2180` and
`driver/pass_plan.rs:452`; duplicate `estimated_passes` in `rewrite.rs:180`
and `driver/pass_plan.rs:363`; duplicate synthetic-root-motion-blur checks.

Put direct input traversal on `Layer`, and share analysis results where
the semantics are identical. Keep traversal order/reachability policy at
each caller. New variants currently require parallel semantic edits.
Regression: branch, composition, scatter, optional user-effect input, and
motion-blur graph traversal across purity analysis and pass planning.

#### Actions and acceptance

- [ ] Compare traversal semantics and ordering at each caller before consolidating.
- [ ] Put direct-input traversal on a shared HIR API; share pass estimates/root checks only where semantics match.
- [ ] Keep reachability and traversal-order policy explicit at callers.
- [ ] Cover branches, compositions, scatter, optional effect inputs, and motion blur across purity and pass planning.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-9"></a>

### 9. Raster state vocabulary

**Classification:** Cleanup; no vocabulary mismatch established.

#### Original evidence and proposed direction (2026-09-22)

Evidence: `crates/fresco/src/driver/render_state.rs:62` declares valid
cull, compare, and blend choices; native `runtime/raster_state.rs:32`
repeats them as string matches; `crates/fresco-artifact/src/lib.rs:408`
carries strings in `ManifestRasterState`.

Use serde enums in the artifact layer for the shared vocabulary, preserving
existing wire spellings. Compiler constant evaluation resolves to these
enums; the host performs an exhaustive conversion to wgpu. Unknown external
values must still fail. Fresco's named blend recipes remain Fresco policy.
Regression: all vocabulary values round-trip and map, with unknown values
and duplicate aliases rejected.

#### Actions and acceptance

- [ ] Enumerate current cull/compare/blend vocabulary and existing wire spellings.
- [ ] Use typed artifact enums and exhaustive compiler/host conversions; preserve wire compatibility or version intentional changes.
- [ ] Keep authored blend recipes as Fresco policy and reject invalid external values.
- [ ] Test all values through serialization and backend mapping, including unknown values and duplicate aliases.

**Completion record:** open; no implementation or reproduction results recorded here yet.

<a id="source-of-truth-issue-10"></a>

### 10. Primitive metadata and builtin catalog

**Classification:** Cleanup; catalog reachability remains unverified.

#### Original evidence and proposed direction (2026-09-22)

Native `runtime/parameters.rs:110` and `runtime/storage.rs:85` separately
 parse scalar kinds and widths, with different aliases. Compiler
 `driver/gpu_function.rs:320` and `:388` repeat constructor/type mappings;
 `typed_scalar.rs:16` has another alias set. Share canonical primitive
 identity and shape, while keeping per-storage-class support explicit.
 Do not conflate logical component count with padded GPU byte layout.

 `crates/fresco/src/builtin_catalog.rs:152` contains a large separate
 `BUILTIN_SPECS` table. Source search found no callers of its three accessors
 outside that file; active builtin lookup uses `registry.rs:1703`.
 `BuiltinCaps` remains actively used. Check generated/macro references
 before deleting unused specs; move active capability flags to the registry
 rather than maintaining two catalogs.

#### Actions and acceptance

- [ ] Compare primitive identities/aliases across compiler and runtime while separating storage-class support from logical shape.
- [ ] Consolidate duplicated facts without equating component count with padded byte layout.
- [ ] Check macro/generated references and call sites for `BUILTIN_SPECS` and its accessors before removing anything.
- [ ] Remove only proven unused catalog data; preserve active `BuiltinCaps` and centralize active registry metadata.
- [ ] Test alias consistency, per-storage restrictions, and generated-reference coverage.

**Completion record:** open; no implementation or reproduction results recorded here yet.

### Existing authorities to preserve

Extend the shared `fresco-artifact/src/types.rs` resource vocabulary and exhaustive
Naga-format conversions, Naga layout/reflection and constant evaluation, and real
device-derived compute limits. Do not replace those with new handwritten tables.

### Validation and documentation handoff

Use focused behavioral/structural regressions for each issue, not shader snapshots.
Run owning compiler/runtime checks; run local GPU probes where claims concern
rendered values or actual device behavior. Update generated artifacts through their
generators when contracts change. Update `LANGUAGE.md` only when shipped semantics
or host obligations change; keep investigation history and completion evidence here.

## Lowering scalability

- [ ] Add WGSL byte-size and largest-function regression budgets for the fully
  enabled [reveal flourish stage](<examples/90) gallery/stages/reveal_flourish_stage6_plus_blur.fr>).
  On 2026-09-16, default CLI output including engine stages measured 95,571 bytes,
  10 functions, a largest function of 1,166 lines, and one temporal loop. Establish
  a rendered baseline too: the main gallery file disables major effects and is
  not an equivalent benchmark. The historical 48 KB / 300-line goal remains
  unproven; set regression limits from measured output without weakening coverage.
- [ ] Reduce measured compact-scatter expansion and repeated call-site setup.
  The full stage emits 130 instances; scatter accounts for 1,672 of 2,840 reported
  instructions. Existing scene, scatter, path, and vector/matrix helpers already
  share bodies. Identify remaining duplication, preserve instance semantics, and
  report before/after bytes, largest-function size, and rendered equivalence.
- [ ] Verify temporal resampling with engine-provided time and captured runtime
  inputs. Add regression coverage before dependency-based evaluation-rate hoisting;
  include signal-driven path `point_at`/`tangent_at` lookups and explain why an
  expression can or cannot be hoisted. Each shutter sample must evaluate at its
  own time. Consider CPU-side evaluation
  only if profiling justifies a defined host contract that preserves this behavior.

Path tables and shared forward `point_at`/`tangent_at` evaluation already exist.
Keep remaining path precision/runtime-geometry work in the
[path tasks below](#path-geometry-and-arc-length), and coordinate-space/derivative reuse in
the [AA investigation](#anisotropic-aa-and-thin-feature-filtering). For each lowering change,
validate generated WGSL and use GPU image comparisons where shader structure
alone cannot establish equivalence.

## Path geometry and arc length

Path values, length, demand-driven `dist`/`along`/`tangent` channels, shared nearest
searches, and forward `point_at`/`tangent_at` helpers are implemented. Retain these
specific follow-ups rather than restarting parser or lowering scaffolding:

- [ ] Support runtime-varying path geometry with explicit length/prefix-table
  update semantics and a matching host/resource contract where needed. Path
  construction currently requires compile-time coordinates. Test parameter and
  time changes through emitted execution, including temporal resampling.
- [ ] Establish numerical error bounds for arc-length and forward evaluation.
  Cover segment boundaries, degenerate curves, endpoints, and near-equidistant
  nearest-point choices; verify reveal continuity and approximately uniform
  arc-length motion against reference calculations and rendered output.
- [ ] Define and test tangent behavior at joins/cusps and `along` behavior at
  nearest-point ties. Any tangent smoothing must be explicit rather than silently
  changing geometry semantics; document unavoidable medial-axis discontinuities.
- [ ] Extend `--explain` with actual prefix-table provenance and evaluator
  precision policy. Add receipt regressions for channel demand and shared searches;
  report hoisting decisions through the evaluation-rate task above without claiming
  once-per-frame execution merely because an expression depends only on time.

## Engine contracts and renderer execution

Fullscreen, compile-time fullscreen variants, mesh factories/material passes,
and particle compute/draw slices already execute. Extend their resolved-contract
model rather than rebuilding those slices. Engine source owns interfaces, resource
types, hooks, passes, and pipelines; the compiler validates/specializes and emits
matching shaders/metadata; the host allocates resources and schedules execution.
Keep instance parameters separate from pass/view configuration, including editor
values. See [host integration](LANGUAGE.md#host-integration-obligations), [engine examples](examples/engine),
and the [current engine contract](LANGUAGE.md#language-engine-and-host).

- [ ] Finish the [Canvas context migration](docs/canvas-context-design.md).
  Registered entries now use explicit typed contexts and engine-selected methods;
  custom raster names and named method blocks share that binding path. Migrate
  remaining legacy unregistered canvas contracts before removing their argument
  synthesis adapter. Keep the shipped `Canvas`/`present_canvas`/`canvas_pipeline`
  contract and automatic engine discovery working throughout the migration.
- [ ] Finish [engine-registered authoring contracts](LANGUAGE.md#interfaces-and-registered-entries).
  Custom raster declaration names, named typed method blocks, explicit registered
  canvas context forwarding, engine-owned compile-time configuration, and ordered
  `@compose(state)` stacks and compute entry/pass binding are implemented. The
  particle sample now uses emitter sugar with engine-owned storage/dispatch/lifecycle.
  Next: inferred persistent/scratch records, general resource plans, and richer
  lifecycle scheduling.
- [ ] Generalize pass/hook lowering beyond the shipped subsets, including staged
  selection in lighting passes. Validate all required plugs, signatures, resources,
  and ambiguous implementations; interfaces alone do not establish execution.
- [ ] Define pipeline-known and draw-known axis resource/update contracts, distinct
  from compile-time specialization. Test constraint pruning, stage-specific variants,
  explicit host selection, material parameter attribution, and variant budgets.
- [ ] Complete shading-model hook resolution, inheritance/delegation, additive-pass
  compatibility, stable model IDs, and deferred dispatch/payloads. Resolve global
  versus model axes, indirect-light hook shape, unlit ownership, and pass-body reuse.
  Preserve diagnostics for unauthored lighting rather than inventing a lighting model.
- [ ] Generalize vertex-factory selection, required streams, interstage packing,
  and interpolation; connect declared render state to execution, including shadows,
  transparency, pass dependencies, and additional renderer families such as decals.
- [ ] Extend general engine struct-uniform layout beyond scalar/vector f32 storage:
  nested structs, matrices, arrays, and integer-preserving storage. Keep offsets,
  alignment, manifest types, uploads, and shader declarations in agreement.
- [ ] Define capabilities and diagnostics for bindless resources, non-uniform
  indexing, uniformity, and explicit texture sampling; unsupported constructs must
  fail at their owning layer rather than silently miscompile.
- [ ] Extend compile/validate and hardware GPU coverage for each new contract:
  authored hook edits change pixels, distinct instances retain their own parameters,
  repeated builds are deterministic, and shader/manifest layouts match. Check
  runtime variant combinations and separate particle compute/render access layouts.
  Keep missing executable stages as errors; validate each replacement compiler
  path through rendered output.

## Const templates and bounded unrolling

Const-template/unroll support already has tests in
`crates/fresco-cli/tests/interfaces_generics_progression.rs`; these include u32/i32
values, missing arguments, range errors, multiple specializations, and forwarding
const identifiers. The old literal-only proposal is not a current restriction.

- [ ] Audit remaining const-template semantics and coverage: mixed type/value
  parameters, bool values, uniqueness, arity, kind/range errors, unsupported kinds,
  and rejection of runtime-dependent `unroll` bounds. Preserve runtime parameters;
  recommend a const parameter or removing `unroll` when a bound is not known.
- [ ] Verify specialization keys include function identity and ordered type/value
  arguments, deterministic naming, per-compilation helper deduplication, and unroll
  expansion after specialization. Keep permutation axes separate from function
  specialization; do not implicitly convert axis values into const arguments.
- [ ] Audit and finish expansion budgets and boundary tests. Historical proposed
  defaults were 32 specializations/function, 256 iterations/loop, and 4,096 expanded
  statements/source loop; reconcile these with existing iteration limits before
  choosing policy. Errors should report measured cost, limit, and an actionable fix.
  Expose specialization count, unroll count, and largest expansion in `--explain`.
  Verify nested expansion cannot evade budgets and output remains valid WGSL.

## Anisotropic AA and thin-feature filtering

- [ ] Define coordinate boundary contracts and an explicit `with domain(...)`
  escape hatch while preserving authored shape syntax. Follow the
  [coordinate domains and coverage proposal](docs/proposals/coordinate-domain-coverage-design.md)
  for footprint requirements, reference filtering, limitations, and acceptance tests.

Investigate against current code before applying the old diagnosis: regressions in
`crates/fresco/src/lower/canvas.rs` already cover derivative-based shape bands and
mean-coverage saturation for soften/shadow, with glow kept separate.

- [ ] Reproduce excess blur near projective horizons and determine where scalar
  footprint width still smears edges under anisotropic stretch. Compare the edge
  width `w = ||F^T n||` (footprint matrix F, unit edge normal n) with current distance
  derivatives in the same coordinate space. Do not apply the largest footprint
  axis in every direction or double-apply a transform already captured by derivatives.
- [ ] Verify thin-feature filtering when the AA band approaches the feature width.
  The earlier proposal used `w > k * feature_width` with k around 0.5 as a validity
  threshold: prefer an analytic integral when available, otherwise bound coverage
  by the feature mean (for a stroke, `coverage <= stroke_width / w`). Validate units,
  geometry assumptions, existing saturation paths, and explanatory receipts.
- [ ] Keep projective denominator guards local to singularity handling. When the
  footprint becomes ill-conditioned, define fallback/culling and report it; avoid
  global horizon or primitive-specific crispness clamps that introduce aliasing.
  Genuine subpixel band-limiting is correct; any sharper artistic bias is explicit.
- [ ] Build supersampled-reference GPU comparisons: multiple FOVs, static edge-on
  views and animated horizon sweeps, thin stripes/strokes/fills, repeat and non-repeat
  spaces. Measure static over-blur and temporal aliasing separately before declaring
  the footprint correction or saturation policy complete.

## Rewrite contracts and follow-up

Rewrites transform typed HIR layer graphs. Built-in analytic rules and user effect
rules exist. Preserve the current user form
`rewrite outer(a, ...) compose inner(b, ...) => result(...)`, mandatory effect
locality (`point`, `local(radius)`, `global`), first-match ordering, and compile-time
`when` guards. Runtime-dependent guards currently skip conservatively. Validate
patterns/hole bindings before lowering and prohibit widening locality cost.

- [ ] Investigate broader composition-pattern matching and registry-driven rule
  expansion with deterministic ordering and accept/reject coverage. Preserve the
  cheapen-only locality rule and emitted rewrite/locality receipts.
- [ ] Design numeric self-verification (`within epsilon`) with explicit error
  metrics, sampling domains, and failure diagnostics before enabling it.
- [ ] Decide whether runtime-dependent guards merit support; if so, specify their
  semantic and execution-cost contract rather than applying them as compile-time facts.

Keep these runnable examples and their explain regressions as starting points:
[film grain](<examples/16) user-effects/film_grain.fr>),
[chromatic split](<examples/16) user-effects/chromatic_split.fr>),
[true guard](<examples/16) user-effects/rewrite_guard_true.fr>),
[false guard](<examples/16) user-effects/rewrite_guard_false.fr>),
[locality receipt](<examples/16) user-effects/rewrite_locality_receipt.fr>), and
[non-match](<examples/16) user-effects/rewrite_non_match.fr>).

## WASM contract maintenance

Rust structs and serde behavior in `crates/fresco-wasm/src/lib.rs` own the contract.
Generate TypeScript via ts-rs into
`crates/fresco-wasm/web/src/generated/wasm-contracts`; these are ignored local build
artifacts, not hand-maintained definitions. Update web consumers and worker/runtime
protocols for field names, types, and nullability changes.

- [ ] Audit clean-checkout contract generation and CI enforcement. A git-diff-only
  check cannot detect drift in ignored generated files; ensure generation happens
  before consumers are typechecked and serialization changes have runtime tests.
- [ ] When changing payloads, run `npm run types:contracts`, `npm run typecheck`,
  and `npm run test:unit` in `crates/fresco-wasm/web`, plus
  `cargo check -p fresco-wasm` at the root. Record changed Rust fields, affected
  worker/host consumers, and validation in the change description.
