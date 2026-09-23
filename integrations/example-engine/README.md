# Fresco example engine

See the [engine documentation index](docs/README.md) for architecture, renderer
guides, audits, and decisions.

A basic **Deferred** renderer is also available; see the [deferred guide](docs/example-engine-deferred.md) for G-buffer, material IDs, and limits.

Basic GPU forward+ point lighting is available for mesh surfaces; see the
[forward+ guide](docs/example-engine-forward-plus.md) for the sample,
light-upload API, and current limits.

This package owns the authored engine profile used by the Fresco playground.
Start with [engine/engine.fr](engine/engine.fr), which imports its policy,
contracts, shader functions, particle modules, and pipeline declarations.

This package provides the embedded source bundle, real-engine contract tests,
reflected uniform packing, and shared Rust/wgpu canvas, mesh, and particle renderers. They support a fused authored
fullscreen pass, authored mesh stages, and particle compute/draw stages, with
frame globals, validated parameter updates, and PNG/JPEG textures. Hosts can
preview mesh surfaces on a sphere, plane, or box. Mesh renderer recipes execute authored compute, MRT, and fullscreen passes; see
[engine recipe contracts](../../LANGUAGE.md#passes-pipelines-and-annotations) for the supported subset. The playground now uses this Rust engine in both development and production.
The TypeScript engine has been deleted; see the
[retirement coverage map](docs/example-engine-renderer-retirement.md).
[Native and standalone browser hosts](../example-engine-host/README.md) now wrap
this path. The browser host runs the compiler in a separate worker and loads the
shared Rust renderer as its own WASM module.

## Read the integration from source to frame

1. **Choose the profile.** [engine/engine.fr](engine/engine.fr) is the authored
   entrypoint. [src/lib.rs](src/lib.rs) exposes its complete embedded source map;
   compiler tests use independent stubs instead of importing this package.
2. **Compile the bundle.** Combine that map with `main.fr` and call
   `compile_bundle_virtual`, as in the snippet below. Keep the WGSL and manifest
   together. The [native host](../example-engine-host/src/native.rs) also shows
   explicit filesystem profiles and compilation on a worker thread.
3. **Validate and prepare.** [CanvasRenderer](src/runtime/canvas.rs),
   [MeshRenderer](src/runtime/mesh.rs), and [ParticleRenderer](src/runtime/particles.rs)
   consume the reflected contracts and prepare independent GPU candidates.
   Unsupported execution plans and malformed resources fail with diagnostics.
4. **Provide host inputs.** [profile.rs](src/profile.rs) and
   [profile/mesh.rs](src/profile/mesh.rs) adapt the sample's frame and camera data.
   [assets.rs](src/assets.rs) decodes named textures. Resource IO belongs to the
   host; shader and material policy remain in the `.fr` profile.
5. **Render and update.** The [native event loop](../example-engine-host/src/native/window.rs)
   and [browser adapter](../example-engine-host/src/browser.rs) supply a target,
   dimensions, time, and delta time. Keep the installed renderer until a new
   candidate succeeds; reset particles explicitly when their state is replaced.

To try the flow, start with the [native or minimal browser demo](../example-engine-host/README.md).
Neither standalone host imports the playground application. The browser compiler
runs in a worker, while the renderer is a separate WASM module built from this
same Rust engine. [Local measurement scripts](../example-engine-host/README.md#measure-the-integration)
report startup, preparation, submission costs, and package sizes.

## Inspect the integration

- `engine/core/04_canvas_contract.fr`: canvas context and fullscreen stages.
- `engine/core/05_mesh_contract.fr`: vertex factory and mesh material execution.
- `engine/core/06_particle_contract.fr`: particle entry and simulation contract.
- `engine/core/07_surface_properties.fr`: engine-defined surface policy.
- `engine/particles/`: particle behavior and presentation helpers.
- `engine/pipelines/`: authored pipeline declarations; declaration alone does not
  establish that the current host supports execution.
- `src/lib.rs`: embedded source-bundle API, independent of the compiler and GPU.
- `src/profile.rs`: maps explicit host frame inputs to the authored frame contract.
- `src/runtime/uniforms.rs`: validates reflected layouts and stages buffer updates.
- `src/runtime/parameters.rs`: validates defaults and atomic parameter edits for
  the fullscreen instance ABI, shared by both hosts.
- `src/runtime/storage.rs`: validates and packs dynamic arrays, including padded
  vec3 elements, device limits, binding collisions, and atomic CPU edits.
- `src/runtime/textures.rs`: validates named RGBA8 inputs and reflected bindings.
- `src/runtime/pass_plan.rs`: validates dependency graphs and intermediate targets,
  orders sparse pass IDs, and checks scaled dimensions before GPU allocation.
- `src/runtime/targets.rs`: prepares complete intermediate texture/view sets;
  callers keep the installed set until a resize or plan replacement succeeds.
- `src/assets.rs` and `assets/`: shared PNG/JPEG decoding and the embedded demo texture.
- `src/runtime/canvas.rs`: prepares authored GPU stages and renders to a supplied target.
- `examples/offscreen.rs`: local GPU probe with readback assertions, outside CI.
- `tests/`: behavioral checks against this real engine profile.

## Compile with the embedded engine

In a Rust host that depends on both `fresco` and `fresco-example-engine`:

```rust,ignore
let mut files = fresco_example_engine::source_files();
files.insert("main.fr".into(), authored_source);
let artifact = fresco::driver::compile_bundle_virtual(&files, "main.fr", false)?;
// Keep artifact.wgsl and artifact.manifest together for runtime preparation.
```

The bundle contains stable `engine/...` virtual paths and embedded source bytes;
it needs no runtime filesystem lookup. The build script discovers `.fr` files
recursively in sorted order and rebuilds the bundle when the engine tree changes.
An explicit engine override should replace the entire bundle, rather than filling
missing override files from this profile.

Run the contract tests with:

```sh
cargo test -p fresco-example-engine
```

The particle and surface-property suites were moved from `fresco-cli`; their
assertions are retained here. Compiler-only tests use their own minimal fixtures
and do not depend on this package.

For filesystem compilation, select this profile explicitly from the repository root:

```sh
cargo run -p fresco-cli -- "examples/10) fundamentals/engine_canvas.fr" --engine-dir integrations/example-engine/engine
```

Rust tools can use `fresco::driver::compile_source_bundle_with_engine_dir` with
that directory. To pair filesystem user imports with this embedded profile, use
`compile_source_bundle_with_engine_files` and pass `source_files()` plus `ENTRYPOINT`;
[the native host](../example-engine-host/src/native.rs) demonstrates both modes. The selected directory must contain `engine.fr`; imports resolve
relative to their importing module. Missing entries or imports fail compilation
without falling back to an ancestor engine. Omitting the selection retains the
existing discovery behavior.

Repository examples select this profile explicitly; the old forwarding file
under `examples/engine` has been removed. The browser runtime registry bundles this
package's canonical source tree directly under stable virtual paths, independently
of generated example catalog copies. CLI regression tests,
visualizer tests, and README validation also supply their selected engine explicitly.

See the [architecture and migration plan](docs/example-engine-architecture.md)
for the remaining native/browser runtime work.

## First Rust runtime slice

The default `runtime` feature enables `wgpu`. Compiler workers and other
source-bundle consumers use `default-features = false` to exclude GPU dependencies.
The runtime has no compiler dependency; only the example executable and contract
tests invoke the compiler.

The local GPU probe has been verified on Windows with an NVIDIA GeForce RTX
2080 Ti, including running the built executable outside the repository. It
checks readback pixels after frame updates and an authored vertex change, plus
zero-size suspension and invalid-preparation recovery. Run it outside CI:

```sh
cargo run -p fresco-example-engine --example offscreen
# Optionally write a PPM image to a path you choose:
cargo run -p fresco-example-engine --example offscreen -- target/example-engine.ppm
```

`CanvasRenderer::prepare` consumes matching WGSL and manifest data plus an explicit
canvas name. Unsupported resources or pass plans produce errors. Prepare a new
candidate before replacing the active renderer; failed preparation preserves the
old instance. `render` receives time, delta time, physical size, and a host-owned
target view. Zero width or height suspends drawing. The target must match the
prepared format and supplied dimensions.

The current compiler canvas output is opaque: layer coverage and opacity are
flattened against black before presentation, including a standalone layer root.
The runtime writes those pixels without additional blending. Transparent canvas
targets will require an explicit compiler/runtime contract; selecting a browser
alpha mode alone cannot restore alpha already flattened by the compiler.

Pass-plan validation runs before pipeline preparation. It checks resource
producers, dependency/input agreement, cycles, a single presentation sink, and
supported transient target formats and scales. This is the foundation for
multipass execution; the renderer still explicitly rejects plans requiring
intermediate targets until per-pass authored stages and GPU execution are wired up.

`IntermediateTargets::prepare` allocates validated `rgba8unorm` or `rgba16float`
transient targets with render, sample, and readback usage. Target IDs need not be
contiguous. Every size is checked before allocation, and GPU validation/allocation
errors reject the candidate. Preparing a zero-size viewport returns an empty set.
Prepare a replacement even when only format or scale changes; reusing an old set
based on viewport dimensions alone is incorrect. Native GPU readback verifies
these allocation and replacement behaviors. WASM compilation has been checked;
the multipass allocation path has not yet been exercised on WebGPU.

The generic uniform packer accepts reflected `f32`, `i32`, and `u32` scalar/vector
fields with 1 to 4 components. It checks bounds, overlap, alignment, duplicate bindings,
and supplied device limits, and preserves integer bits. Buffer size follows the
compiler's 16-byte-rounded global-uniform ABI; field alignment follows
[WGSL layout rules](https://www.w3.org/TR/WGSL/#alignment-and-size).
All fields must receive typed values. A failed update leaves every previously
committed buffer unchanged. No GPU writes occur until packing succeeds.

The first canvas path supplies the bundled `frame: FrameGlobals` through the
profile adapter. Extending hosts can use the generic uniform packer with their
own resolver. Arrays and matrices in global uniform records and additional GPU
execution paths require further runtime implementation.

The mesh port has a separate profile adapter in `profile/mesh.rs`.
`MeshSceneInputs::pack` produces the bundled `PreviewScene`'s 240-byte scene
buffer from column-major model/view/projection matrices, camera position, frame
inputs, and displacement controls. It validates finite inputs and zeroes padding
before returning owned upload bytes. This is an explicit adapter for the bundled
engine, not general matrix reflection support. Its integration test compiles the
real mesh contract and checks the emitted scene binding's layout against the
packer. The initial renderer below consumes this profile adapter.

`runtime/vertices.rs` packs named, typed host vertex streams using the factory's
reflected stride and attribute offsets. It supports the compiler's current
32-bit float, signed integer, and unsigned integer vertex formats, with one to
four lanes. Vertex vec3 values occupy 12 bytes; storage-array vec3 padding does
not apply to this layout. Integer lanes retain their bits. Padding is zeroed,
and attribute order in the manifest does not determine packing order.
The caller supplies device limits and the vertex count; malformed layouts,
overlapping attributes, missing or unknown streams, mismatched lane counts,
non-finite floats, and excessive buffer sizes are errors. Optional/defaulted
attributes also require explicit host streams because the current manifest does
not include their default values. Tests exercise the shipped mesh factory as well
as custom layouts with padding and integer attributes. These CPU preparation
pieces are independent of GPU pipeline creation.

`runtime/mesh_geometry.rs` prepares those streams as GPU vertex buffers and
optional u32 index buffers. `MeshGeometry::prepare` validates indices, checks
allocation/validation errors, and returns an independent candidate. Keep the
installed geometry until preparation succeeds. Its `vertex_layout()` supplies
pipeline attributes and `draw()` issues the indexed or non-indexed draw after
the renderer binds a compatible pipeline and resources. Empty geometry issues
no draw. The local offscreen probe verifies reflected offsets/locations, exact
integer attributes, both draw paths, empty geometry, and failed/replacement
isolation on native GPU. Both hosts use the shared geometry. The standalone
browser GPU check verifies sphere, plane, and box selection with distinct visible
silhouettes, preserved material edits, and recovery after GPU recreation.

The initial `runtime/mesh.rs` renderer now consumes `MeshGeometry` and executes
the selected surface's authored vertex/fragment entries. It supplies the bundled
scene and frame uniforms, resolves their bindings from the manifest, and uses
material blend, two-sided, and depth policy. The host supplies matching color and
`Depth32Float` targets; zero-size frames suspend drawing. Keep the installed
renderer while an independent replacement is prepared. A local native GPU probe
checks actual material pixels, time updates, an edited engine vertex hook,
incompatible geometry metadata, and failed-preparation/invalid-input recovery.
The standalone browser GPU check also verifies material and texture edits,
asynchronous replacement, cancellation, and depth-target resizing.
Material texture bindings use the same validation, RGBA8 uploads, and repeat/linear
sampler policy as canvas textures, through `runtime/resources.rs`.
Use `MeshRenderer::prepare_with_resources` with a `MeshResources` value containing
geometry, named decoded images, and optional parameter overrides. The overrides
are validated and installed with the candidate, preserving edited parameters
across texture replacement or device recreation. Missing images, invalid bytes,
and binding collisions reject preparation without modifying the installed renderer.
The native GPU probe exercises
[`material_texture.fr`](examples/material_texture.fr), including orientation,
repeat/linear sampling, replacement recovery, and textures sharing a bind group
with parameter uniforms. The native host now uses this preparation path; browser
mesh execution remains pending.

`runtime/depth.rs` owns asynchronous depth-target preparation for both host
platforms. It validates dimensions, checks GPU allocation and validation results,
and returns a complete candidate before the host replaces its installed target.
Zero dimensions return no allocation. The native host now consumes this shared
path, and GPU checks cover resized and rejected candidates without invalidating
the old target. Browser wiring must await preparation without holding its mutable
render state and discard obsolete resize requests.

Material parameters use `runtime/surface_parameters.rs`: one reflected group-0
uniform per scalar or color parameter. `MeshRenderer::update_parameters` accepts
a JSON object of edits, preserves unspecified values, validates the entire batch,
and then writes fixed-size buffers without replacing the pipeline. Read current
values with `parameter_values()`. Booleans require JSON booleans, and integers
must be exactly representable by the compiler's f32 uniform transport. Declared
ranges and binding collisions are checked. This ABI supports `f32`, `i32`, `u32`,
`bool`, and `color`; unsupported types fail explicitly. Unlike canvas instance
parameters, independent material buffers do not share a 64-component capacity.
CPU and native GPU tests exercise the authored
[`material_parameters.fr`](examples/material_parameters.fr) sample, including
defaults, color/scalar edits, integer/boolean branches, and failed-batch recovery.

Canvas dynamic-array defaults now use reflected read-only storage bindings.
Supported elements match the compiler's current storage lowering: `f32`, `i32`,
`u32`, `bool`, and `vec2`/`vec3`/`vec4`. Scalar lanes use the compiler's f32
transport; integers must be exactly representable. Vec3 elements occupy 16 bytes,
including padding. Empty arrays retain an empty host value and one zeroed backing
element, satisfying the GPU binding's minimum size. The compiler allocates canvas
storage in group 2, separately from group 0 instance uniforms, without host shader
rewrites. Native GPU readback covers array defaults mixed with instance edits.
The standalone browser GPU probe verifies padded defaults and device recreation.

Live edits can grow, shrink, or empty these arrays without recompiling the shader.
`CanvasParameters` validates instance and storage values as one atomic batch.
For GPU updates, call `CanvasRenderer::begin_parameter_update`, await the returned
candidate's `prepare`, then call `apply_parameter_update`. Preparation owns its
resources, allowing frames to continue on the installed renderer. Failed batches
preserve values and pixels; commits reject candidates from another renderer or
an input state that has since changed. GPU layouts use the element stride as the
minimum binding size, so resized buffers reuse the pipeline.

`CanvasRenderer::update_parameters` remains a synchronous instance-only helper.
Both hosts use the candidate API for complete parameter batches. Native JSON
overrides include arrays; browser updates are asynchronous and cancellable.
Installing a browser artifact can include parameter JSON so texture replacement
and GPU recreation install the artifact and its edited values together.
Native and browser GPU checks cover resized arrays, mixed texture bindings,
failed-batch preservation, and stale updates.

Path buffers now use compiler-provided segment data from the manifest.
`runtime/paths.rs` validates the versioned layout and packs immutable geometry
into reflected storage bindings. It checks collisions with other resources and
counts paths and editable arrays together against the device's storage limits.
Array updates retain the installed path resources. Rendering needs neither the
original source files nor a compiler instance to reconstruct the geometry.

Try the curve example from the repository root:

```sh
cargo run -p fresco-example-engine-host --features native -- --source integrations/example-engine/examples/path_canvas.fr
```

Its `widths` array controls the stroke width. CPU preparation tests and native
GPU checks cover geometry packing, missing payloads, relocated bindings, and
array replacement. Browser verification remains incomplete: the strict comparison
between equivalent two-segment and repeated 66-segment cubic shaders differs at
seven antialiased pixels by one color level. A controlled browser comparison using
the same 66 rows as constants and as storage data produced identical pixels.
The broader numerical discrepancy remains under investigation.

## Particle execution details

`runtime/particle_pool.rs` supplies CPU lifetime reservations for the particle
allocation manifest. It emits 16-byte little-endian slot commands, bounds growth
by explicit host limits, and rejects invalid steps without changing commands.
`runtime/particles.rs` connects scheduling to GPU resources, authored spawn/update
stages, and particle drawing. `ParticleRenderer::prepare` accepts a compiled
manifest, WGSL, textures, and optional material overrides. Its asynchronous
`render` method takes scene inputs and color/depth views; hosts must serialize
calls because growth can await GPU preparation. `reset` restarts playback.
Event-driven hosts can instead call `begin_frame`, await the owned frame's
`prepare` future, then call `render_prepared` synchronously. No renderer borrow
spans that await; reset, replacement, or another committed frame makes obsolete
work fail before queue writes. The existing asynchronous `render` is a convenience
wrapper for hosts that serialize whole frames.
The local offscreen probe verifies automatic growth, simulation, visible output,
pause/reset, material edits, and failed replacement preservation. Both native and
standalone browser hosts select it for surfaces with particle pipelines. Local
Chrome/WebGPU checks cover visible particles, growth, playback controls, material
edits, GPU recreation, and invalidation of pending frames.


### Surface preview scene

The native and browser hosts submit a normal scene: the selected mesh/material,
plus a ground mesh using a separately authored standard PBR material. Scene
content is opt-in through `preview_source_files`; engine-only compiler clients
receive no sample materials. `MeshRenderer::add_object` assembles mesh draws from
one compiled artifact into shared frame attachments. Shadow and geometry stages
draw all applicable objects; frame compute, deferred resolve, and presentation
execute once. Opaque draws precede blended draws within a mesh stage.

The floor participates in depth, shadow maps, GBuffer channels and generated
material IDs. The host chooses its transform; no rendering code knows a floor
height. A reusable `scene_background` pass supplies a procedural sky behind the
scene, without altering depth or debug buffer inspection.

Lighting presets live in `profile/lighting.rs`. They create ordinary directional,
point and hemispherical ambient light data; shaders have no preset-name branches.
`set_scene_lighting` also accepts independently authored values. Unlit is an
explicit shading override. The initial lighting contract supports one shadowed
directional light and up to 64 point lights. Scene objects must share a compiled
artifact so deferred material/style dispatch and resource contracts agree.

### Standard shading styles

`engine/styles/contract.fr` declares `StandardStyle for standard`. PBR and the
external Toon sample use `style Name for standard : StandardStyle` with structured
`StandardSurface`, `ShadingContext`, and light inputs. The renderer sums complete
`direct` results unchanged; `indirect` excludes emission; `finish` runs once and
adds emission by default. Light radiance, attenuation, and visibility are separate
inputs, so a style controls how it uses them. Coverage remains renderer-owned.
Forward, Forward+, and Deferred use this same contract. No UV or per-draw resource
capability is promised by the current Deferred GBuffer.

The Toon outline still uses the existing mesh-contribution API. Settings and
operation-based techniques are later steps in the style design.

### Preview UV requirements

`PreviewShape::geometry_for_surface` validates the selected surface before
allocating built-in geometry. The sample supplies two-component UV streams 0 and
1. A required channel outside those streams, or with a different width, fails
with a diagnostic naming its selector and `surface_requirements.uv_channels`.
Optional unavailable channels do not make the preview incompatible. Both hosts
use this profile check; custom integrations supplying their own geometry remain
responsible for their own surface-to-stream policy.

Runtime style settings use reflected per-material records (`f32`, vectors, and
colors). `properties { style: Toon(band_threshold: 0.35) }` sets a material's
default; hosts update `style.band_threshold` through the ordinary parameter API.
Forward, Forward+, and Deferred share the record layout and runtime updates do not
recompile shaders. The sample's outline constants remain separate until style
operations support checked captures. Artifact schema 7 requires rebuilding both
compiler and renderer WASM packages together.

Alternate vertex factories used with the bundled mesh recipes must expose the
shared `style_settings: buffer<vec4>` draw binding, alongside the other engine
bindings. Their DrawRecord source includes `style_settings_offset` automatically.
