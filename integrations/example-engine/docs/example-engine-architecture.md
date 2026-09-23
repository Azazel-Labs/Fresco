# A standalone example engine for Fresco

Status: implementation in progress, September 18, 2026.
The sample README now walks from profile selection through compilation, resource
preparation, and rendering. Local native/browser measurement scripts exercise all
three entry kinds; [release measurements](example-engine-measurements.md) record
artifact sizes, startup, preparation, and submission costs with explicit limits. Shared manifest types
have been extracted into `fresco-artifact` and are consumed by `fresco-wasm`.
Standalone and playground builds now use the same WASM build driver, producing
separate compiler and renderer modules. The playground site distributes the
renderer package under `example-engine-renderer/`; a base-URL-aware loader
initializes it independently. The built site package has been loaded in Chrome,
including device creation, camera setup, and resize. CPU package checks run during
both builds. The WASM response adapter preserves the browser API's empty resource
arrays even when shared manifest JSON omits unused fields. CI
checks the browser renderer target without running GPU tests. The playground now
uses the shared Rust engine in production and development. The TypeScript engine
renderers, backend selector, and session-storage handoff have been deleted.
Old `renderer=typescript` URLs cannot select another implementation.
Local Chrome tests cover canvas edits, source loading, mesh selection, particle
playback, paused stepping, and reset.
The adapter serializes
asynchronous frames and queues paused redraws requested during preparation.
Shader installation, texture replacement, and partial parameter edits share a
mutation queue so controls cannot cancel an in-flight installation. New source
aborts obsolete asset requests. Partial parameter edits are submitted in order
so edits to different controls do not cancel each other; queued edits and results from older shader revisions are
discarded. Source invalidation is tracked separately from mesh/texture replacement,
so a resource change preserves queued and in-flight parameter edits. Unit tests
cover both orderings, and a browser regression checks the final pixels after a
parameter edit and mesh selection in the same event-loop turn. Unit tests cover
deferred updates, and Chrome checks both same-turn
control edits reach the rendered output. Manifest JSON serialization preserves
WASM map-valued defaults both in the inspector and at the engine boundary.
Vector storage arrays translate between flat editor components and nested runtime
elements; Chrome checks defaults, component edits, growth, and shrink.
Texture replacement commits selector state only after installation, retains edited
parameters, and restores the selector on failure. Runtime diagnostics preserve
controls when a prior artifact remains installed. Chrome injects an HTTP failure,
checks unchanged pixels and controls, then verifies successful replacement and
exact restoration, including an edit while a texture response is deliberately
delayed. Unit tests cover stale request failures, shader/parameter ordering, and
abort of obsolete asset loads. Mesh selection commits only after GPU preparation;
failed and superseded candidates retain the prior selection. Rust geometry changes
reuse the compiled artifact rather than triggering a competing recompile. A Chrome
probe injects a geometry preparation failure, checks unchanged pixels and picker
state, and then recovers with a successful selection. Camera dragging stops
auto-spin, owns touch gestures, and releases pointer capture on completion or
cancellation. Browser checks verify visible orbit/zoom changes and that hover or
cancelled drags cannot keep moving the view.
The Rust adapter uses a GPU-free `PreviewController` for editor parameter
normalization, playback state, and asynchronous build reporting. GPU preparation
and rendering belong to Rust. Visualizer thumbnails retain their separately owned
TypeScript GPU path; they are editor tooling rather than an alternative engine.
The path comparison now passes under its documented exact-transport and bounded-raster checks. Particle reverse playback, seeking to
nonzero times, and looping currently report unsupported operations; reset to zero
and forward steps are available. Installing a particle entry resets an inherited
negative rate to 1x and clears unsupported looping; reverse rate choices and the
back-step button are disabled until a canvas or mesh entry is installed. The
local browser test switches from reverse canvas playback to particles and verifies
continued rendering and the control states. The Rust clock also supplies elapsed
time to editor visualizer cursors.
README image capture now uses the same standalone BrowserEngine API, with an explicit
embedded engine source bundle and encoded texture assets. The harness observes the
engine device for validation errors and GPU completion, without allocating renderer
resources itself. Its build produces both WASM modules, and its cache tracks shared
manifest types, Rust engine/host sources, engine assets, and both build scripts.
The local GPU suite runs every scenario in its Rust engine
project using the existing canary, cellular, contour, directional-AA, gradient,
variant, runtime-input, and material assertions. All 86 Rust engine checks
pass in Chrome on the local hardware adapter, including CPU geometry references,
live supersampled comparisons, and mesh rendering. The Rust test adapter only
supplies frame scheduling, parameter edits, and device observation. Mesh coverage now includes authored vertex/fragment edits, time/delta/resolution,
orbit/zoom/pause, blend modes, two-sided planes, and renamed engine vocabulary.
The same property-editing UI test uses the shared engine and checks
window errors across reload. This exposed a Rust ResizeObserver feedback loop;
canvas resizing now runs in a coalesced animation-frame callback, cancelled on
page disposal. Persisted page-hide events suspend drawing while preserving the
engine, camera, parameter edits, and editor visualizer device; restoration redraws
without adding the time spent away to simulation time. Unit and browser checks
exercise repeated persisted lifecycle events (actual browser cache admission is
not guaranteed for GPU pages). Particle compute/raster edits also pass. The existing
lit/unlit billboard normals and steep-angle billboard area tests now run on both
backends, with their pixel assertions preserved. All eight particle-state tests
also pass through Rust, including extended fields, explicit stepping, paused
redraws, reset, slot recycling, automatic/estimated growth, and particle editor UI.
The runtime queues an independently owned state copy for asynchronous browser
readback; scheduler metadata is exposed read-only. A concurrent-step regression
verifies that later simulation cannot change a pending snapshot.
Compile-known canvas variants now use exact binding selection in the shared Rust
runtime, native --variant options, and browser install_with_options. CPU tests
reject incomplete, unknown, malformed, and ambiguous selections. Native offscreen
readback and the existing browser variant test exercise both authored branches;
a browser regression checks invalid selections preserve the image and recover.
The existing variant test moved from runtime-inputs.spec.mjs to
engine-variants.spec.mjs so the Rust project run its unchanged assertions. The directly owned Vite test server disables hot reload and closes
without the shell-process teardown hang observed on Windows.
The shared particle renderer now owns material resources, managed scheduling,
state/slot buffers, authored compute and raster stages, and playback. Both native
and standalone browser hosts select it for surfaces with particle pipelines.
Fixed, estimated, and automatic allocation use bounded births, conservative
expiration, drop-new overflow, partial-frame ages, and checked GPU limits.
Growth prepares replacement buffers and bindings before committing simulation.

Particle frames use `begin_frame` / owned asynchronous `prepare` / synchronous
`render_prepared`. Browser preparation holds no state borrow across an await;
reset, cancellation, replacement, newer frames, and resize invalidate stale work
before queue writes. Native callers can use the serialized convenience method.
Zero-size frames suspend playback. Material parameters and textures use the same
resource preparation as mesh rendering.

Validation on the RTX 2080 Ti includes native GPU state readback, analytical
integration and partial-frame ages, buffer-prefix preservation during growth,
visible sprites, material edits, pause/reset replay, and rejection of stale frames.
Native fixed and automatic emitters presented ten frames, including a launch from
outside the repository. Interactive native keyboard controls remain unverified.
The local Chrome/WebGPU particle test passes automatic growth, visible authored
stages, simulation, pause/reset, parameter edits, GPU recreation, failed compilation,
and reset/cancellation/concurrent-frame/resize/replacement races during preparation.
Browser mesh GPU regressions remain a separate check. These GPU tests are local-only.

The generated compiler WASM regression compiles both the bundled canvas demo and
`drifting_sparks.fr`. The latter exposed excessive recursive evaluator stack use
in the unoptimized WASM build. Expression-specific evaluation now runs in separate
functions so unrelated temporaries do not enlarge every recursive dispatch frame.
All 202 core tests and the generated-WASM regression pass after this change.

Compiler emission now shares pass plans, engine-pass metadata, mesh variants,
particle pipelines and layout/allocation records, global-uniform layouts, storage-buffer
parameters, texture declarations/metadata, surface requirements and custom-channel
mappings, surface settings, editable entry properties, pipeline declarations/pass
semantics, editor configuration axes, and vertex factories with their attribute
layouts and resource bindings.
Typed scalar, integer, boolean, color, and array default serialization now uses
shared artifact types, retaining f32 precision through emission. Dedicated wire
tests cover decimal precision, integer boundaries, and array shapes.
Canvas and surface parameter records are now shared by emission and readers.
Their generic producer representation retains typed defaults and f32 ranges;
the default reader representation keeps JSON values and f64 ranges. TypeScript
exports stay concrete. Root, canvas, and surface containers also use these shared
records; the compiler no longer duplicates the serialized manifest schema. Full
canvas, mesh, and emitter round trips verify resource omission and field retention.
The mesh profile's scene-input packing is now extracted into Rust, with a CPU
integration test against the actual emitted uniform binding and matrix layout.
It retains the current renderer's 240-byte scene ABI.
Reflected vertex preparation is also available in the shared runtime: named host
streams pack into the authored stride/offsets with checked device limits and
exact integer encoding. Tests cover all current compiler vertex formats, layout
mutations, and the real bundled mesh factory.
The shared GPU geometry component now allocates validated vertex/index buffers,
provides reflected pipeline attributes, and issues indexed or non-indexed draws.
Native pixel readback verifies integer transport, both draw paths, empty geometry,
and replacement isolation; its WASM build passes. Both hosts now use these resources.
The first shared mesh renderer
now executes the authored material stages with the bundled scene/frame bindings,
reflected geometry, and material blend/cull/depth state. Native readback verifies
frame changes and an edited engine vertex hook, plus failed preparation and
invalid-input isolation. Broader render-state
coverage remains to be implemented.
Material scalar, integer, boolean, and color parameters now use reflected group-0
uniforms and atomic CPU validation before live GPU edits. CPU tests check their
ABI against actual compiled materials; native readback verifies defaults, authored
branches, edits without pipeline replacement, and invalid-batch isolation.
Material textures now reuse canvas image validation and a shared GPU binding/upload
implementation. Mesh preparation accepts decoded images and optional edited
parameter values as one replacement candidate. Native readback verifies sampling,
missing-image recovery, preserved edits, and reflected mixed uniform/texture groups;
the WASM build passes. The native host now selects canvas or surface entries,
previews surfaces with the bundled sphere/camera, and resizes depth targets.
Native material and textured-material launches passed, including outside the
repository; a 600-frame watch probe recovered from invalid source and switched
surface → canvas → surface. The browser host supports the same preview meshes;
both hosts now use the shared orbit camera.
Shared `profile/camera.rs` now owns bounded orbit and zoom state, matching the
existing drag/zoom sensitivity while retaining the standalone host's initial
view. CPU tests verify orthonormal views, camera-to-origin mapping, centered
targets, finite extreme inputs, and atomic rejection of invalid input. Native
event wiring now handles left-button drag, logical-pixel wheel zoom, focus-loss
cleanup, and C to reset. Camera state belongs to the native application, surviving
shader reload and GPU recreation. The browser retains a separate CPU-only Rust
camera across renderer recreation and forwards captured pointer drags and wheel
input. Chrome GPU tests verify changed views, preserved camera state after GPU
recreation, and exact restoration after reset. Native interactive mouse behavior
has not yet been manually verified.
The shared profile now also generates the subdivided XZ plane and hard-normal
box. CPU tests verify their stream lengths, UV ranges, tangent frames, outward
winding, and total surface areas. Native `--mesh sphere|plane|box` selects these
meshes and gives the plane a visible tilted view. CLI tests cover every selection
and invalid names; plane and box each presented three frames on an NVIDIA RTX
2080 Ti through Vulkan. This launch check does not establish pixel parity.
The browser shape selector installs an independent candidate while preserving
material edits and textures. Local Chrome GPU checks verify distinct sphere,
plane, and box silhouettes and restore each selected shape after GPU recreation.
Depth-target preparation has moved from the native host into the shared runtime,
with dimension validation and owned asynchronous candidates. Native GPU checks
cover successful resizing, zero-size suspension, and rejected replacements while
the old depth target remains usable. Browser integration sequences these candidates
with asynchronous installation/cancellation, committing only the latest requested
size. A local Chrome GPU test passes for visible sphere rendering, material and
texture edits, GPU recreation, failed replacement preservation, cancellation,
concurrent resizing, zero-size recovery, and overlapping resize/install requests.
Compiler tests now have crate-owned contract fixtures; ordinary driver, variant,
and canvas-lowering tests supply explicit virtual engine bundles. Language metadata
accepts authored prelude source explicitly; the compiler no longer embeds the
example prelude. The browser host and documentation generator supply their chosen
prelude, while compiler tests use a local fixture. The canonical engine sources
now live in `integrations/example-engine`, with an embedded source API and the
relocated particle and surface-property test suites. The forwarding entry under
`examples/engine` has been removed; repository example consumers now select the
engine explicitly. The CLI accepts `--engine-dir`, and Rust tools can select an engine
with `compile_source_bundle_with_engine_dir`. The new
`compile_source_bundle_with_engine_files` combines filesystem user imports with
an isolated embedded engine namespace. Missing selected entries or imports fail
without discovery fallback.
Compiler emission unification, the rest of the test ownership audit,
the remaining renderer paths, and
the playground migration remain to be implemented. The first Rust canvas path and
reflected uniform packer now exist. Its offscreen probe passed on Windows
with an NVIDIA GeForce RTX 2080 Ti, including a launch outside the repository;
it verifies frame-driven pixels, authored vertex changes, rejected invalid
preparation, and zero-size suspension.

The native window host now supports embedded startup, source/engine selection,
background reloads, directory watching, pause/reset, resizing, and surface/device
recreation. Local presentation and failed-reload recovery have been verified on
Windows. The standalone browser adapter now builds as a separate rendering WASM
module and uses the same runtime. Its plain HTML/JavaScript host compiles in a
worker using the compiler WASM and embedded engine source export; no playground
UI or Vite code is imported. Development and optimized builds passed a local Chrome/WebGPU
probe covering visible pixels, failed/valid reloads, frame continuity, pause/reset,
GPU recreation, resize, cancellation during preparation, invalid shader rejection,
and zero-size suspension. A generated-WASM CPU check covers builtin registration,
embedded source transport, and demo compilation. Development builds use the
documented `wasm-dev` profile to retain constructor-only compiler registrations.
The shared canvas runtime now packs authored scalar, boolean, color, and fixed
array parameters. Native hosts accept JSON overrides; the browser exposes atomic
parameter editing without recompilation. Integer values that cannot be represented
by the existing fullscreen f32 transport are rejected explicitly. Local GPU
readback verifies scalar, boolean, color-array, and matrix defaults and edits.
Texture assets now share a Rust PNG/JPEG decoder, explicit named inputs, and
reflected GPU bindings. The default demo embeds its checker PNG. Native hosts
support asset roots and watched overrides; browser hosts accept uploaded bytes
and retain them across GPU recreation. CPU checks cover decoding and binding
validation. Native GPU readback verifies orientation, repeat/linear sampling,
missing-input recovery, and relocated bindings; the existing image-texture sample
also renders through the native host. Browser GPU checks cover upload, failed
replacement, and restoration after GPU recreation.
The opacity probe exposed a compiler bug: standalone canvas layers discarded
coverage while top-level composition applied it. Both paths now flatten against
black consistently; compiler regression coverage, native pixel readback, and
the browser texture-parameter check pass. The current canvas target remains
opaque, as documented in the sample runtime README.
The runtime now validates pass resource graphs and derives a deterministic
dependency order independently of vector positions. CPU tests cover malformed
graphs, scaled target limits, and an actual compiler-produced blur plan. This
does not yet enable multipass drawing: the compiler's authored stage mapping
and runtime intermediate-target execution still need implementation. Inspection
also confirmed that the existing browser currently builds one authored stage
pair, so its multipass metadata alone is not an execution reference.
The shared runtime also prepares intermediate texture/view sets with validated
dimensions and formats, checked GPU allocation, and replacement ownership that
preserves the installed set on failure. Native GPU checks render into the targets
and verify readback, resize/format changes, failed-resize preservation, and
zero-size suspension. Browser-target compilation passes; WebGPU allocation is
not yet exercised. These resources are not yet connected to multipass drawing.

Canvas dynamic-array defaults now prepare reflected read-only storage buffers,
with vec3 padding, checked bindings/sizes, and exact integer transport validation.
A compiler regression exposed the old group-0 collision with instance uniforms;
storage now shares a collision-free group-2 allocator with path buffers across
all canvases, in both emitted WGSL and manifests, and the browser's
shader-text remapping workaround has been removed. CPU tests cover atomic packing
and rejected malformed inputs; native GPU readback covers defaults alongside
instance parameter edits. The rebuilt standalone browser GPU probe also verifies
padded array defaults and restoration after device recreation.
Live storage updates now use owned GPU candidates and an atomic commit alongside
instance values. Arrays can grow, shrink, or become empty without rebuilding the
pipeline. Native parameter JSON and browser edits share this path; browser
installation can include edited values for atomic texture/device replacement.
Native GPU tests cover mixed resources and stale candidates; browser GPU checks
cover overlapping requests, cancellation, failed batches, and state restoration.
Those concurrency checks exposed a nested error-scope lifetime bug: all nested
GPU scopes now pop synchronously before awaiting results, including target
allocation.

Path-buffer transport and runtime preparation are now implemented. The manifest
carries versioned segment rows, and the shared runtime validates and packs their
56-byte GPU layout before installing reflected storage bindings. Static path data
survives dynamic-array replacement. The standalone `path_canvas.fr` example
combines a buffered curve with an editable stroke-width array. Core regressions
also ensure length-only paths do not advertise unused geometry buffers.
CPU tests and native presentation pass; native 64-by-64 readback matches equivalent
constant and buffered line/cubic paths exactly. Browser verification now uses
exact decoded RGBA equality for matching 66-row constant/storage shader forms,
plus a bounded comparison for equivalent two/66-segment forms. The latter allows
at most one RGB level at 0.01% of pixels and requires exact dimensions and alpha.
The observed difference is seven pixels at 514 by 514; matched data differs at
zero pixels. The full browser smoke passes, including storage, texture, rebuild,
resize, cancellation, and installation lifecycle checks. Six CPU mutation tests
cover the comparator's rejection boundaries. Rust packing assertions are unchanged.
The [path diagnostic](../../example-engine-host/scripts/diagnose-path-parity.mjs)
reproduces the constant/storage/private loop matrix; its `--verify` mode enforces
the exact matched-data check and is invoked by the smoke test. GPU specialization
is consistent with the evidence, but the exact driver transformation is unproven.
See the [comparison contract](example-engine-path-parity-decision.md).
Standalone mesh and particle browser checks also pass. The native release
executable compiles and presents its embedded demo from a temporary directory
outside the checkout (three hidden frames on the local Vulkan adapter).

Per-pass stage emission must preserve the authored hook boundary: replaying an
entire `shade` hook for every intermediate would repeat its coordinate or color
transformations. Resolve how internal canvas materialization receives context
and how the final authored presentation hook consumes it before emitting those
stages. The existing single authored stage pair remains unchanged meanwhile.
Paths, APIs, features, and
commands under “proposed” describe the intended implementation.

Compiler isolation reverified September 19: all 202 current compiler library
tests passed in a temporary workspace outside the checkout containing only
`fresco`, its macro crate, shared artifact types, crate-owned fixtures, and the
manifest schema. The workspace used the repository's pinned Rust toolchain;
neither example sources nor the sample integration were copied. The command was
`cargo test --offline -p fresco --lib --quiet` with a separate build directory.
The local result is recorded in `target/compiler-isolation-current.log`.

Native portability was also reverified from the current release build: only
`fresco-example-engine.exe` was copied into another temporary directory.
Both `--check` and `--hidden --frames 3` succeeded there using the embedded demo,
engine bundle, and texture. The local report, including the executable hash,
is `target/example-engine-portable-check.json`. This verifies Windows/Vulkan on
the local NVIDIA adapter, not other platforms.

The browser path comparison now passes its implemented
[test contract](example-engine-path-parity-decision.md), with exact transport
checks retained and bounded raster differences checked separately.

The standalone page now includes the planned sample selector for canvas, mesh,
and particles. The build copies their authored sources into the static package;
selection exposes the source and compiles through the existing worker. The local
`test-browser-package.mjs` check copies only that built directory outside the
checkout, serves it independently, and verifies all three selections render
without browser errors or resource requests outside the copied package. This
check passed on the local Chrome/WebGPU configuration.

## Decision

Build one small Rust example engine, with its engine `.fr` modules beside its
host implementation. Run that engine through two thin hosts: a native executable
and a browser canvas adapter. The playground becomes one consumer of the browser
adapter, rather than the place where the engine is defined.

The result should teach an integrator the whole path:

```text
Authored .fr + engine .fr modules
               |
         Fresco compiler
               |
       WGSL + typed manifest
               |
        Example engine
               |
     native window / web canvas
```

Use Rust's `wgpu` for the shared GPU implementation. It supports native graphics
backends and browser execution through WebAssembly; the browser target for this
engine should explicitly require WebGPU. Native rendering does not require a
browser or a JavaScript runtime. [wgpu overview](https://wgpu.rs/)

This is a sample integration, not a new mandatory runtime for Fresco. Other
engines remain free to consume compiler artifacts directly.

## What exists today

The important correction to the starting assumption is that the current web
renderer was **TypeScript using WebGPU**, before extraction into Rust `wgpu`. The Rust WASM
crate exposes the compiler and editor services. Producing a shared Rust engine
therefore requires both extraction and a renderer port.

| Current implementation | Current responsibility | Proposed destination |
| --- | --- | --- |
| [fresco compiler](../../../crates/fresco/src/driver.rs) | Compile source bundles to WGSL, manifest JSON, and diagnostics | Remains the compiler |
| [WASM bindings](../../../crates/fresco-wasm/src/lib.rs) | Compiler/editor bindings and manifest transport types | Retains compiler/editor bindings; shared artifact types move out |
| [Engine sources](../engine/engine.fr) | Rendering policy, contracts, material types, shader helpers, passes | Canonical sources beside the example engine |
| Former TypeScript canvas renderer | GPU resources, canvas rendering, pass execution, parameter handling | Port engine behavior to Rust |
| Former TypeScript surface renderer | Mesh rendering, material settings, particle compute and drawing | Port engine behavior to Rust |
| Former frame adapter (`playground-engine.ts`, retired) | Supply the shipped engine's time and resolution fields | Example engine profile |
| Former uniform bindings (`engine-uniforms.ts`, retired) | Validate reflected layouts and upload host values | Shared resource binding code |
| [Particle pool](../../../crates/fresco-wasm/web/tests/reference/particle-pool.ts) | Spawn scheduling, lifetime reservations, capacity policy | Shared particle runtime |
| [Mesh generation](../../../crates/fresco-wasm/web/tests/reference/surface-mesh.ts) | Plane, sphere, box geometry | Example assets/geometry module |
| [Pass helpers](../../../crates/fresco-wasm/web/src/pass-plan.ts) and former TypeScript texture pool | Pass selection and intermediate target allocation | Validated execution plan and resource pool |
| [Example synchronization](../../../crates/fresco-wasm/web/scripts/sync-examples.mjs) | Copy the sample catalog into the web build | Engine registry bundles canonical integration sources directly, independently of catalog synchronization |

The port must inspect behavior rather than copy comments or assume every declared
pipeline is executable. The current pass helpers include a fallback for incomplete
multi-pass metadata. The existing uniform uploader also has concrete restrictions
on group indices and field types. These are limitations to identify explicitly,
not universal rules to bake into a new public API.

## Proposed package boundaries

```text
crates/
  fresco/                         compiler, checker, lowering
  fresco-artifact/                 shared manifest and artifact contract
  fresco-wasm/                     browser compiler and editor services

integrations/
  example-engine/
    Cargo.toml                    fresco-example-engine: Rust library
    README.md                     integration walkthrough and support table
    engine/                       canonical authored .fr engine sources
      engine.fr
      core/
      particles/
      pipelines/
    assets/                       small, explicitly licensed demo assets
    src/
      lib.rs
      bundle.rs                   embedded engine source inventory
      profile.rs                  engine-specific runtime values and policy
      runtime.rs                  shared rendering API
      runtime/                    binding, pass, mesh, particle modules
    tests/                        contract and scheduling tests

  example-engine-host/
    Cargo.toml                    native binary + browser cdylib adapter
    src/
      main.rs                     native window and command-line handling
      lib.rs                      browser-facing exports
      native.rs
      browser.rs
    web/                          minimal canvas page, no editor
```

Use normal `foo.rs` module roots, consistent with the repository convention.
Package boundaries matter more than these particular filenames.

### Compiler artifact contract

Create `fresco-artifact` as a small, GPU-independent contract crate. Move the
manifest DTO definitions currently owned by `fresco-wasm` into it, separating
compile diagnostics and editor protocol types from render artifacts. Make the
compiler's manifest emission and the runtime's deserialization use this shared
contract. Merely copying the WASM DTOs into a second crate would preserve drift.

Preserve the existing serialized contract during this extraction. Keep CLI JSON
output compatible; add schema/version changes deliberately when needed. Generate
TypeScript contracts from the shared definitions through the existing generation
workflow. Do not hand-edit generated files or force a GPU dependency into the
compiler, CLI, or language server.

### Example engine library

`fresco-example-engine` owns the renderer and its authored profile. Its default
runtime feature enables `wgpu`; a source-bundle-only build excludes GPU
dependencies. The compiler worker can embed engine sources through that lightweight
feature combination without loading the renderer.

The runtime consumes compiled artifacts and asset data. It does not depend on
`fresco`, Monaco, DOM events, filesystem paths, or an application event loop.
The engine profile may know that `FrameGlobals.time` receives the frame clock;
generic resource packing should only know reflected types, offsets, and values.

### Native and browser hosts

The native host depends on the compiler, the example engine, and a window/event
library such as `winit`. `winit` provides window creation and event-loop management;
its exact version and platform configuration should be selected during the first
buildable slice. [winit documentation](https://docs.rs/winit/latest/winit/)

The browser host is a small `wasm-bindgen` adapter around the same engine library.
It receives a canvas, compiled artifacts, asset bytes, and frame commands. JavaScript
owns animation scheduling, DOM input, CSS sizing, and UI. Keep the existing compiler
worker: send compile results to the renderer, not GPU handles to the worker.

Build the renderer adapter as a separate WASM module initially. This avoids making
the compiler worker instantiate renderer code, or making the rendering instance
carry editor/compiler services. Both modules are included in the web distribution;
“included with the WASM build” need not mean one `.wasm` file. Measure download,
startup, and memory costs before considering consolidation.

## Bundle the engine once

The canonical sources have moved from `examples/engine` to
`integrations/example-engine/engine`. The temporary forwarding import is removed.
The CLI example compile and WGSL validation suites and example-driven regression
tests select the profile with `--engine-dir` or the corresponding Rust API.
Visualizer and README checks inject an explicit virtual source bundle.
Keep one authoritative copy. The source
bundle exposes stable virtual paths such as `engine/core/04_canvas_contract.fr`
regardless of its physical directory.

A deterministic build step embeds the complete engine source inventory, including
relative paths, and reports changes to Cargo for rebuilds. Native compilation and
the browser compiler worker consume this same inventory. Engine sources and the
Rust host carry a common profile identity/revision in packaged builds, with a
content hash for reproducibility. That identity is distinct from the compiler
version and manifest schema version.

Provide two source-loading modes:

- **Embedded default:** the executable runs the bundled demo without a checkout or
  loose engine files. The minimal browser demo gets the same profile.
- **Explicit engine directory:** development can replace the entire engine source
  tree and recompile. Missing imports fail; do not quietly complete a partial
  override from the embedded profile.

User-authored shader imports still resolve against the user's project. Engine
policy imports resolve against the selected engine root. Define this separation
in the compiler's filesystem and virtual-source entry points before moving files;
do not require a duplicate `engine` folder beside every shader sample.

Ship small demo assets alongside the profile and include all required bytes in
the default executable or its documented release archive. Asset identity and
decoding are explicit. Do not rely on the current working directory, repository
relative paths, or a network fetch for the default demo.

## Runtime contract

The following is a conceptual API, not code expected to compile yet:

```rust
// Platform-specific setup supplies the GPU device, queue, and output format.
let mut engine = ExampleEngine::new(gpu, output_format, profile)?;

// A candidate is validated before replacing the currently rendered program.
let candidate = engine.prepare(artifact, entry_selection, assets).await?;
engine.install(candidate);

engine.set_parameter(parameter_id, typed_value)?;
engine.resize(physical_size)?;
engine.render(frame_inputs, target_view)?;
```

`artifact` contains WGSL and its matching manifest from one compiler invocation.
The runtime checks schema compatibility, entry selection, resource layouts, stage
entry points, required device capabilities, and available host operations before
installing it. Missing entries, resources, or unsupported plans produce structured
errors; they do not select an unrelated canvas or drop passes.

`frame_inputs` explicitly contains time, delta time, physical resolution, camera
state, and an execution action such as advance versus redraw. Redrawing while
paused must not advance particle simulation. Reset is an explicit operation;
seeking a simulation requires reset/replay or a documented unsupported result.
The renderer must not obtain time or resolution from global browser state.

The host creates and owns the presentation surface, acquires a frame, and presents
it. The shared runtime owns its pipelines, buffers, textures, bindings, and
simulation state and encodes work against a supplied target view. This also allows
offscreen rendering without a window.

Resizing to zero suspends rendering. Surface loss/reconfiguration is handled by
the platform host; device loss invalidates engine resources and triggers an
explicit rebuild. Asset uploads and shader preparation are asynchronous where
required. Avoid blocking waits in the browser adapter.

Live edits are transactional: failed compilation or preparation leaves the last
valid program visible with a diagnostic. Successful installation replaces the
whole artifact consistently. Resource reuse is allowed only after compatibility
checks; changes to particle storage layout require a declared reset rather than
reinterpreting old state.

## What belongs in Rust and what belongs in `.fr`

| Authored engine `.fr` | Rust engine integration | Platform host / UI |
| --- | --- | --- |
| Surface records and context contracts | Supply frame, camera, and scene data | Window/canvas and input events |
| Vertex and fragment functions | Upload vertices and bind resources | File picker and editor |
| Particle spawn/update and billboard shaders | Schedule births, allocate state, dispatch compute | Play/pause/reset controls |
| Declared pass and pipeline contracts | Validate support, schedule passes, allocate intermediate targets | Animation scheduling and presentation |
| Rendering and material policy declarations | Translate reflected policy into GPU state | Display diagnostics and parameter controls |

Changing a shader contract must change execution through emitted stages and
metadata. The new Rust runtime must not quietly recreate the shader's material
logic, lighting, or alpha rules in hardcoded host-generated WGSL.

Likewise, the example engine's choices must not move into the compiler. A sphere
generator, sample camera, particle capacity strategy, or `FrameGlobals` adapter
belongs to this integration, even if the playground happens to use it everywhere.

## Scope of the tiny engine

“Tiny” means a small, inspectable host and a clear supported profile. It does not
mean duplicating only a fullscreen triangle and declaring the existing surface
and particle paths migrated.

The complete initial integration should support the current executable canvas,
mesh surface, and compute-particle paths, runtime parameters, the profile's texture
inputs, resize, reset, and actionable diagnostics. Port intermediate target and
pass execution only where emitted contracts fully specify the work; report
unsupported contracts explicitly. A declared forward-plus or deferred pipeline
does not establish executable support by itself.

The initial migration used one canvas as a vertical slice and retained the
TypeScript runtime for comparison. That runtime and its selector are now removed.
Do not make silent per-frame fallbacks part of the final API. Remove the old
renderers only after their supported behavior is covered by the Rust backend.

Out of scope: a scene editor, general entity/component system, material authoring
UI, arbitrary engine plugins, WebGL fallback, and automatic support for every
pipeline an external engine could declare.

## How people would run and study it

Native commands now available (the standalone browser host is documented in
[`integrations/example-engine-host`](../../example-engine-host/README.md)):

```sh
# Opens a window using the embedded engine, demo, and assets.
cargo run -p fresco-example-engine-host --features native

# Compiles a user file using the embedded engine profile.
cargo run -p fresco-example-engine-host --features native -- --source artwork.fr

# Iterates on engine sources too.
cargo run -p fresco-example-engine-host --features native -- \
  --source artwork.fr --engine-dir integrations/example-engine/engine --watch
```

Release builds provide a native executable with the same default demo. Start with
Windows as the locally verified platform; build portability is not evidence that
other operating systems have been exercised.

The minimal browser page should contain a canvas, sample selector, and error
display. It uses the compiler worker and renderer adapter without importing the
playground application. It demonstrates the same integration in a browser while
remaining small enough to read.

The integration README should explain, in order: choose an engine profile, compile
a source bundle, validate the artifact, provide resources, render a frame, then
handle updates and errors. Link each step to actual implementation code. Keep
the broader [engine integration reference](../../../LANGUAGE.md#host-integration-obligations) as the contract
reference rather than duplicating it.

## Migration sequence and acceptance gates

### 1. Establish the shared artifact boundary

Extract manifest types and route compiler output and WASM transport through them.
Preserve JSON compatibility and regenerate TypeScript definitions. Add structural
tests that cover the actual fields the runtime consumes and reject malformed
layouts or unsupported schema versions. Existing compiler and web unit tests pass.

### 2. Package the authored engine

First establish the test ownership and stub boundary described below. Compiler
tests must not acquire a dependency on the extracted sample engine to stay green.

Add explicit engine-root selection and the source-bundle API. Update native tests,
CLI example compilation, editor services, browser compilation, and example sync
before removing the old source location. A native bundle and browser bundle must
compile the same source with the same engine contents and semantics.

### 3. Build the native canvas slice

Implement device setup, one authored fullscreen pass, reflected runtime inputs,
and a native window. Include a deterministic offscreen render path for tests.
The built executable runs outside the repository using its embedded demo.
Authored vertex/fragment changes demonstrably change the rendered result.

### 4. Run that same slice in a minimal browser host

Build the renderer library for WASM and attach it to an existing canvas. Keep the
compiler in its worker. Validate time, delta time, physical resolution, parameter
updates, resize, bad shaders, and runtime errors on both hosts. Verify compilation
does not stall animation through accidental work on the rendering thread.

### 5. Port mesh, material, texture, and particle execution

Port algorithms and policy from the current runtime with focused tests. Preserve
reflected vertex/storage layouts, pipeline variants, blending and depth behavior,
particle birth scheduling, extra state fields, capacity growth, overflow behavior,
paused redraws, and reset. Exercise partial compute workgroups and device limits.

### 6. Replace the playground's engine implementation

Adapt the existing preview controller to the new API. Keep editor parameter forms,
camera input, diagnostic display, and preview controls in TypeScript. Inventory
visualizers, thumbnails, and README capture too: migrate their runtime dependencies
before deleting code they still need. Remove obsolete shader-wrapper generation
only when its replacement executes the corresponding compiler-authored stages.

The TypeScript engine has been removed. See the
[retirement coverage map](example-engine-renderer-retirement.md) for the shared
runtime, adapter, and GPU checks replacing renderer-specific mocked tests.
Every GPU spec now runs in the Rust project, including editor and parameter-form
scenarios. CPU geometry and particle-allocation reference algorithms remain under
`tests/reference`; no renderer or GPU resource manager remains there.

The adapter and inspector share automatic surface selection: prefer a matching
material pipeline, otherwise use the first surface. Explicit inspector and URL
selections override this default. Browser tests compare automatic and explicit
selection, and retain engine-file editing and reload checks.

### 7. Publish the sample integration

Document supported execution contracts and platform limits, add native packaging
and minimal-web build commands, and link the sample from the main README. Measure
binary/WASM size, startup, shader preparation, and frame cost. Claims of completion
require an executable native demo and an independently runnable browser demo.

## Validation strategy

### Compiler stubs and test ownership

Tests inside `crates/fresco` must use small, crate-owned `.fr` engine stubs wherever
an engine contract is needed. Extract the relevant contract shapes from the current
engine files, then reduce them to the declarations and executable bodies needed
to exercise compiler behavior. These are real Fresco inputs, not mocked compiler
results: parsing, checking, lowering, and WGSL validation still run normally.

Proposed fixture location: `crates/fresco/tests/fixtures/engines/`, with separate
minimal policy, canvas, surface, and particle fixtures. Keep the fixtures private
to testing. Do not copy the complete example engine, load it indirectly through
a helper, or regenerate stubs from it during tests. Changes to sample lighting,
materials, or scheduling must not change compiler unit-test inputs implicitly.

Provide explicit virtual source bundles to ordinary compiler tests. Reserve
filesystem discovery for dedicated loader tests, which create isolated temporary
engine directories from the same local stubs. Those tests must not depend on the
working directory or find an engine by walking into the repository's examples.
Move applicable uses of the shared `tests/render-policy` and particle fixtures
into this crate-owned setup as part of the audit.

Choose ownership by what a test proves, including tests currently outside the
compiler crate:

| Assertion | Owner after extraction |
| --- | --- |
| Parsing, generic checking, mutability, diagnostics, reflection, or lowering of arbitrary engine contracts | `fresco`, using minimal stubs |
| Correctness of the shipped engine's shaders, material defaults, particle modules, or sample rendering policy | Example engine tests, using its real `.fr` bundle |
| CLI arguments, exit codes, file loading, or output transport | CLI tests, with explicit minimal fixtures where sufficient |
| WASM serialization, editor services, worker messages, or UI interactions | WASM/web tests, with explicit fixtures where sufficient |
| End-to-end rendering of the sample profile in native and browser hosts | Example engine/host integration tests |

Do not move every test outside `fresco` indiscriminately. Move tests whose subject
is the sample engine; retain CLI and browser boundary tests with their owning code.
For a mixed test, preserve the compiler invariant as a focused stub regression
and move the real-engine assertion to the sample integration. Record old and new
test locations so relocation does not silently remove coverage or weaken checks.

Acceptance gates for this separation:

- `fresco` tests run without the sample engine source directory present, using
  an isolated test checkout or fixture environment rather than deleting sources.
- No compiler test includes or reads `.fr` files from `examples/engine` or the
  new sample integration, directly or through a test helper.
- Stub contracts use arbitrary names where appropriate to catch accidental
  coupling to sample material or entry names.
- The moved integration tests execute the real bundled sources and remain part
  of the relevant CPU test gates; GPU execution stays local-only.
- Audit compile-time source inclusions as well as runtime discovery. If production
  compiler code embeds an engine helper, decide whether it is a genuine language
  standard-library component or sample-engine code. Stubs must not conceal that
  production dependency.

### Runtime and cross-host validation

Reuse the existing [hardware GPU regression scenarios](../../../crates/fresco-wasm/web/tests/gpu/README.md).
They already exercise authored pass edits, material policy, runtime inputs, and
particle state. Port the assertions, not just screenshots.

- CPU tests check layout packing, pass dependency order, resource compatibility,
  parameter validation, particle scheduling, and artifact replacement behavior.
- Compiler tests establish that both hosts receive artifacts from the same source
  bundle, and engine imports keep their policy ownership after relocation.
- Local GPU tests render identical deterministic inputs on native and browser
  backends, comparing with tolerances appropriate to format and device rounding.
  Compare state/readback and relational properties; do not add golden baselines.
- Compare TypeScript and Rust paths during migration, supplemented by analytical
  or independent references so shared defects are not treated as correctness.
- Regular CI builds native and WASM targets and runs CPU/compiler/web unit checks.
  Keep hardware GPU tests local-only, as required by repository policy; do not
  install browsers or add the GPU suite to CI.

Record which hardware and platforms actually ran. Passing a native Rust build
does not validate WebGPU execution or prove browser/native visual parity.

## Risks and choices to settle in the first slice

The largest risk is porting accumulated behavior embedded in the TypeScript
renderers. The source map above starts that inventory; tests and an explicit support
table decide when a path is complete. Avoid a simultaneous language redesign.

Pin compatible `wgpu`, windowing, and WASM toolchain versions after checking the
workspace Rust version and transitive dependencies. Match browser WebGPU limits
when validating the portable profile rather than developing only against stronger
native hardware. Desktop-only capabilities can be separate explicit profiles later.

Settle output color format, transfer function, alpha convention, image decoding,
and coordinate orientation early. Otherwise two correct-looking backends can
still disagree on composition and screenshots.

The key boundary is stable: the compiler produces artifacts; the example engine
and its `.fr` sources define one executable integration; native and browser shells
provide the platform. The playground then demonstrates that integration without
owning it.
