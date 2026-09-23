# Example engine hosts

A basic **Deferred** renderer is also available; see the [deferred guide](../example-engine/docs/example-engine-deferred.md) for G-buffer, material IDs, and limits.

Basic GPU forward+ point lighting is available for mesh surfaces; see the
[forward+ guide](../example-engine/docs/example-engine-forward-plus.md) for the sample,
light-upload API, and current limits. Both browser hosts have a **Renderer**
selector for **Forward** and **Forward+**; changing it recompiles without editing
the material. The native equivalent is `--renderer forward-plus`.

This package is the platform shell around [the example engine](../example-engine/README.md).
The native host opens a window and presents the engine's authored canvas, mesh,
or particle stages. The standalone browser page uses the same Rust renderers
through a separate WASM module, with compilation in a worker.

## Run locally

```sh
cargo run -p fresco-example-engine-host --features native
cargo run -p fresco-example-engine-host --features native -- --source artwork.fr
cargo run -p fresco-example-engine-host --features native -- --source artwork.fr --entry painting
cargo run -p fresco-example-engine-host --features native -- --source artwork.fr --engine-dir integrations/example-engine/engine --watch
```

The default animated [demo](demo.fr), checker texture, and complete engine source bundle are
embedded. The executable needs no checkout, current-directory assets, browser,
or network connection for that demo. Build it with `cargo build --release -p
fresco-example-engine-host`; the executable is `target/release/fresco-example-engine.exe`
on Windows.

The window dependency enables X11 and Wayland, but this sample has only been
run on Windows. On Wayland, client-side window controls use Winit's Adwaita
decoration without title text (`wayland-csd-adwaita-notitle`). This avoids the
optional unmaintained `ttf-parser` dependency; Windows and X11 window titles are
unaffected. Native keyboard controls are documented below.

The native host supports one fused canvas pass or a surface displayed on a
built-in mesh, including frame globals, parameter overrides, and PNG/JPEG textures.
Surfaces with a particle pipeline use the shared particle renderer automatically.
The standalone browser host supports the same three entry kinds. Multipass
execution remains unported. The playground uses this shared Rust engine in both
production and development. The TypeScript engine and backend selector are removed.

README media also consumes this browser API. Run the render-readme.mjs script in
crates/fresco-wasm/web/scripts locally to build both WASM modules and capture the
documented examples through the Rust engine. It requires Chrome with WebGPU and
Pillow with WebP support. Missing textures, compiler errors, and GPU validation
errors fail the capture; images are published only after every pending sample
succeeds. Source and output hashes skip unchanged captures. This is a local GPU
workflow, not a CI requirement.

Try the native material examples from the repository root:

```sh
cargo run -p fresco-example-engine-host -- --source integrations/example-engine/examples/material_parameters.fr
cargo run -p fresco-example-engine-host -- --source integrations/example-engine/examples/material_parameters.fr --mesh plane
cargo run -p fresco-example-engine-host -- --source integrations/example-engine/examples/material_parameters.fr --mesh box
cargo run -p fresco-example-engine-host -- --source integrations/example-engine/examples/material_texture.fr --texture paint=integrations/example-engine/assets/checker.png
cargo run -p fresco-example-engine-host -- --source "examples/50) particles/drifting_sparks.fr"
```

Mesh preview starts with a shared default camera. Native `--mesh sphere|plane|box` selects the
64-by-64 sphere, 16-by-16 subdivided plane, or box with separate face normals.
The browser's **Surface preview mesh** selector offers the same shapes and
preserves material edits and textures. The plane is tilted toward the camera.
In the native window, drag with the left mouse button to orbit, use the wheel to
zoom, and press C to reset the view. Camera state survives recompilation and GPU
recreation. In the browser, drag the preview to orbit, scroll to zoom, and use
**Reset camera** to restore the view. The CPU-only Rust camera object survives
renderer recreation; both hosts share the orbit math and limits.
Depth targets follow window dimensions and are recreated with the GPU. Material
parameters and texture files use the same override/watch options as canvases.

Space pauses/resumes time, Home resets time and particle playback, R recompiles, and Escape
closes the window. Failed recompilation or GPU preparation leaves the last valid
program displayed and reports the error in the console and window title.
Compilation runs on a worker thread.

`--source` resolves authored imports from the source file's directory. With no
engine override, engine imports resolve exclusively inside the embedded bundle.
`--engine-dir` replaces the whole profile and requires its `engine.fr`; missing
imports never fall back to embedded files.

`--watch` polls `.fr` files recursively under the source directory and the engine
override directory. It excludes symlink directories, `.git`, `target`, and
`node_modules`. Imports outside those watched trees still compile normally;
press R to reload after changing them. Explicit parameter and resolved texture
files are also watched, even outside those trees. Use `--entry` when a file declares more
than one canvas or surface; the host never silently chooses the first entry.

## Select compiled canvas variants

If an authored fullscreen pass emits multiple compile-known variants, select
every axis explicitly. A sole emitted specialization is selected automatically
when no selection is supplied; an explicit selection must still match it exactly. For example, use `--variant '{"quality":"low"}'` with the native host.
The same selection is available through the browser renderer:

```js
await engine.install_with_options(wgsl, manifestJson, entry, assets,
  JSON.stringify({ variant: { quality: "low" }, parameters: { gain: 0.5 } }));
```

The options object also accepts `mesh` for surface geometry. Unknown option fields,
incomplete selections, unknown values, and ambiguous variants fail installation;
the prior installed renderer remains usable. Omit `variant` for passes without
axes. Variant bindings currently apply to canvas passes; surface/particle entries
reject them explicitly. Native `--check` validates selections without a GPU.

## Run in a browser

Install Rust's `wasm32-unknown-unknown` target, `wasm-pack`, and Node.js, then run
from the repository root:

```sh
node integrations/example-engine-host/scripts/build-web.mjs
node integrations/example-engine-host/scripts/serve-web.mjs
```

Open <http://127.0.0.1:5182> in a WebGPU-capable browser. Use `--release` on the
build command for optimized WASM. The page has source and parameter editing,
compilation, pause/reset, and GPU recreation. Its sample selector loads packaged
canvas, mesh, and particle sources into the editor and compiles the selected
program. Selecting a sample replaces the editor contents. It needs neither Vite
nor the playground UI.
The static `web` directory is self-contained after building; generated compiler,
renderer, and demo files are ignored by Git. Serve it over localhost or HTTPS.
The build runs CPU-only checks of both generated modules: compiler builtin
registration, canvas/emitter compilation with the embedded engine bundle, and
the renderer API/camera lifecycle. The playground uses this same build driver:
`npm run wasm:build` produces its compiler package and distributes the separate
renderer under `public/example-engine-renderer`. Development watch builds use
`wasm-dev`; normal package builds use release mode. The playground defaults to
**Rust example engine**. The same engine serves development and production;
`renderer=typescript` no longer selects a backend. Source editing, mesh selection,
textures, and playback controls communicate with the shared Rust runtime.
Particle reverse playback, nonzero seeking, and looping still report explicit
unsupported-operation diagnostics.

To test the built playground locally, build the site, serve it with
`npm run preview -- --port 5183`, then run from the repository root:

```sh
node integrations/example-engine-host/scripts/test-playground-engine.mjs
```

This Chrome/WebGPU check is local-only. Set `FRESCO_PLAYGROUND_URL` to use a
different server URL. Full playground parity and retirement of the old runtime
are still in progress.

Development builds use the workspace's `wasm-dev` profile. It keeps the compiler
in one code-generation unit as a narrow workaround for a WASM archive-linking
issue: plain unoptimized builds on the tested Rust 1.98.1 toolchain drop some
constructor-only inventory members, leaving builtins such as `rgba` unregistered.
Calling the constructors again or enabling thin LTO did not correct that loss.
The generated-WASM check guards this requirement; native build settings and
language inputs are unchanged.

`compiler-worker.js` loads the compiler WASM and obtains the embedded engine
source bundle through `example_engine_sources_json()`. `main.js` sends the
resulting WGSL and manifest to `BrowserEngine`. Failed compilation or GPU
preparation preserves the current program, and obsolete preparation requests
cannot replace newer ones. GPU loss is reported; **Rebuild GPU** recreates the
adapter and reinstalls the last valid artifact.

For diagnostics, `await engine.particle_state_bytes()` returns a copied
`Uint8Array` of the submitted particle storage, using the manifest's state layout.
It does not advance simulation. The copy is queued when called; later frames,
resets, and shader replacements cannot modify it. Await any in-flight
`render_async` first to include that frame. `particle_slots_json()` returns the
committed managed scheduler's capacity, dropped count, and four ABI words per
slot. Both APIs reject non-particle entries; slot metadata also requires managed
allocation. Readback is opt-in and is not part of the normal frame loop.

The browser calls `await render_async(time, delta_time)` for all entry kinds.
Particle buffer growth can await GPU preparation; a canceled or superseded frame
resolves to `false`. Call `reset_playback()` to restart particle simulation. The
page serializes animation frames so simulation steps cannot overtake each other.

For the local-only GPU smoke check, install the existing playground's Node
dependencies and Chrome, start the server above, then run:

```sh
node integrations/example-engine-host/scripts/test-browser.mjs
node integrations/example-engine-host/scripts/test-browser-mesh.mjs
node integrations/example-engine-host/scripts/test-browser-particles.mjs
node integrations/example-engine-host/scripts/test-browser-package.mjs
```

This uses Playwright solely as a test driver and is excluded from CI. Set
`FRESCO_GPU_BROWSER` to select another installed Playwright browser channel.
The package check copies only the built static `web` directory into a temporary
directory, owns its local server, and exercises canvas, mesh, and particle sample
selection there. It requires no separately running server and rejects browser
resource requests outside that copied package.
The mesh check passes in Chrome on Windows, covering visible sphere rendering,
material edits, texture upload, GPU recreation, failed replacement preservation,
cancellation, concurrent resizing, and zero-size recovery. The particle check
passes automatic growth, simulation, pause/reset, material edits, GPU recreation,
failed compilation, and cancellation/reset/resize/replacement races during frame
preparation. With the refreshed release packages, the canvas suite passes its
storage, texture, rebuild, resize, cancellation, and installation lifecycle
assertions and both path comparisons. Matched 66-row constant/storage data must
produce identical decoded RGBA pixels. Equivalent two/66-segment shader forms
may differ by at most one RGB level at 0.01% of pixels, with exact alpha and
dimensions. The verified run differs at seven pixels at 514 by 514. See the
[comparison contract](../example-engine/docs/example-engine-path-parity-decision.md).

Reproduce the controlled matrix against freshly compiled artifacts with:

```sh
node integrations/example-engine-host/scripts/diagnose-path-parity.mjs --output target/path-parity-diagnostic.json
```

It compares constant and storage tables with 66 and two active iterations, plus
an initialized private-array variant. The private-array variant also retains the
seven-pixel difference on the verified Chrome configuration. This diagnostic
reports differences; add `--verify` to enforce exact matched-data equality. The
canvas smoke invokes that mode automatically. CPU-only comparator boundary tests
run with `node --test integrations/example-engine-host/scripts/path-pixels.test.mjs`
from the repository root and are also included in CI.

## Parameter editing

The demo exposes `speed`. In the browser, edit the **Parameters (JSON)** object
and apply it without recompiling. Updates may name a subset of the parameters;
unspecified values retain their current values. Invalid batches change nothing.
GPU recreation restores edited values. Compiling a new artifact starts from its
authored defaults.

For native use, put `{"speed": 0.25}` in `parameters.json`, then run:

```sh
cargo run -p fresco-example-engine-host -- --params-file parameters.json --watch
```

Editing that file reloads the artifact and its parameter values. Invalid JSON,
unknown names, type errors, and out-of-range values preserve the last valid
program. `--params` also accepts an inline JSON object; it cannot be combined
with `--params-file`. Both forms are validated by `--check` without a GPU.

The fullscreen ABI supports scalar `f32`, `i32`, `u32`, `bool`, and `color`
parameters, plus fixed arrays of scalar, boolean, color, vector, and matrix
elements, up to 64 total components. Colors are four numeric components; vectors
are component arrays; matrices are arrays of columns. Array values are ordinary
JSON arrays (the compiler's default-value envelope is handled internally).
Declared ranges are enforced rather than clamped. Because the current compiler
ABI transports all these components as `f32`, integer inputs that are not exactly
representable are rejected. Dynamic storage arrays support defaults and live
overrides, including growth, shrinkage, empty arrays, and padded vector elements.
Mixed instance/storage batches either apply completely or preserve every old
value. Native `--params`, `--params-file`, and watched reloads use this same path.

The Rust runtime uses `begin_parameter_update`, then the candidate's asynchronous
`prepare`, followed by `apply_parameter_update`. Rendering may continue during
preparation. Stale candidates cannot overwrite a newer committed input state.
The browser's `await update_parameters(json)` resolves to `true` when installed,
or `false` when canceled/superseded; invalid updates reject the Promise.
`parameter_values_json()` includes both instance and storage values.
`install_with_assets` accepts optional parameter JSON as its fifth argument, so
GPU recreation and texture replacement preserve edited values atomically.

## Texture assets

The demo's `example://checker` PNG is embedded in the renderer package and native
executable. The shared Rust decoder accepts PNG and JPEG, preserving raw RGBA8
channel values and straight alpha; GPU textures use `rgba8unorm` with repeat
addressing and linear filtering. Texture and sampler locations come from the
manifest, including sparse groups and binding numbers. There is no placeholder
substitution when an image is missing or invalid.

Native hosts resolve authored asset paths under `--asset-root`, or under the
source file's directory when that option is omitted. Override a shader texture
by name with `--texture name=path`; repeat the flag for multiple textures:

```sh
cargo run -p fresco-example-engine-host -- --texture checker=picture.png --watch
cargo run -p fresco-example-engine-host -- --source "examples/20) techniques/image_texture.fr" --asset-root examples
```

The native compiler worker reads and decodes assets before the GPU candidate is
installed. `--watch` detects changes to the resolved files; invalid images preserve
the current program. `--check` validates asset availability and decoding too.
The default demo needs neither an asset root nor loose files.

In the browser, select a texture name and upload a PNG or JPEG to replace it.
Uploaded bytes stay in memory and survive **Rebuild GPU**. Uploads prepare a new
candidate while preserving parameter edits. A newly compiled source starts from
its authored defaults. External asset identities are not fetched automatically:
a missing image is reported, and the upload control can supply it. Invalid image
bytes leave the last valid texture visible.

Rust hosts pass a `TextureInputs` map of named `TextureImage` values to
`CanvasRenderer::prepare_with_textures`. Image decoding and the sample's embedded
asset resolver live in `fresco_example_engine::assets`; the renderer performs no
file or network IO. Browser hosts pass a `Map<string, Uint8Array>` of encoded
images to `BrowserEngine.install_with_assets`. Decoding is limited to 256 MiB of
RGBA8 data and 16,384 pixels per dimension; GPU preparation additionally enforces
the actual device limits.

## Inspect the integration

- [native.rs](src/native.rs): CLI options and compilation/source selection.
- [native/window.rs](src/native/window.rs): window lifecycle, surface presentation,
  frame timing, reloads, resizing, and device/surface recreation.
- [browser.rs](src/browser.rs): browser canvas, GPU lifecycle, and asynchronous
  artifact installation without borrowing the renderer across awaits.
- [web/main.js](web/main.js): browser clock, source editing, and worker transport.
- [runtime/canvas.rs](../example-engine/src/runtime/canvas.rs): shared rendering;
  no window, filesystem, or application event loop.
- [runtime/uniforms.rs](../example-engine/src/runtime/uniforms.rs): reflected buffer packing.
- [assets.rs](../example-engine/src/assets.rs) and [textures.rs](../example-engine/src/runtime/textures.rs):
  shared decoding, embedded assets, and reflected texture binding validation.
- [profile.rs](../example-engine/src/profile.rs): example-specific frame values.

The host owns the window, surface, and clock. It passes physical dimensions,
time, delta time, and a target view into the runtime. Zero-size windows suspend
drawing; surface/device loss rebuilds GPU resources from the current artifact.

## Checks

`--check` compiles and validates entry selection without a window or GPU:

```sh
cargo run -p fresco-example-engine-host -- --check
cargo test -p fresco-example-engine-host --test compiler
```

On Windows with an RTX 2080 Ti (Vulkan), the native executable has presented
frames from outside the repository. A local watch probe also kept rendering
through a syntax error, installed the corrected source, and completed 600 frames.
The parameter-file watch probe likewise rejected an out-of-range edit, installed
the corrected values, and completed 600 frames. Native GPU readback checks cover
matrix, color-array, boolean, and scalar updates; the browser GPU probe verifies
pixel changes from parameter edits, atomic rejection, and restoration after GPU
recreation.

Native mesh checks have also presented the material and textured-material samples,
including a launch from outside the repository. A 600-frame watch probe recovered
from a syntax error, switched from a surface to a canvas, and then installed the
surface again. Offscreen readback verifies that the bundled sphere is visible
with the preview camera, back-face culling, and depth testing.

The compiler tests above are CPU-only and can run in CI. They do not establish
GPU support for an artifact. For a **local-only** presentation smoke test:

```sh
cargo run -p fresco-example-engine-host -- --hidden --frames 3
```

That command must actually present the requested frames; it fails after 30 seconds
if presentation cannot proceed. GPU smoke tests are never part of CI. The separate
`fresco-example-engine` offscreen example checks rendered pixel values.

## Measure the integration

See the [recorded Windows release measurements](../example-engine/docs/example-engine-measurements.md)
for one local run and its limits.

These scripts are local-only; they do not run GPU work in CI. Build release
artifacts first, and keep the standalone web server running for the browser run:

```sh
cargo build --release -p fresco-example-engine-host
node integrations/example-engine-host/scripts/build-web.mjs --release
node integrations/example-engine-host/scripts/serve-web.mjs
```

In another terminal:

```sh
node integrations/example-engine-host/scripts/benchmark-native.mjs --output target/example-engine-native-benchmark.json
node integrations/example-engine-host/scripts/benchmark-browser.mjs --output target/example-engine-browser-benchmark.json
```

The native script reports executable size and whole-process timings for compile
checks, one presented frame, and 120 presented frames. It launches from the system
temporary directory: the default canvas demonstrates the executable's embedded
profile, source, and asset independence. `--exe PATH` measures another build.

The browser script runs an empty host at 512 x 512, loads the compiler worker and
renderer separately, and measures canvas, mesh, and particle samples. It reports
module/device startup, compiler round trips, first and repeated preparation,
first frame submission, and 120 warmed submission samples. `FRESCO_ENGINE_URL`
and `FRESCO_GPU_BROWSER` select the server and installed browser, as in the GPU
checks. Install the playground's Node dependencies to provide Playwright.

Frame submission measurements include host preparation and queue submission;
they do **not** wait for GPU completion or measure display FPS. Native process
timings include startup and shutdown. WASM sizes are uncompressed response bytes.
Browser/driver caches, build profile, hardware, and system load affect all results;
compare like configurations and retain the JSON report with each measurement.

The production playground regression checks that legacy renderer selection is
unavailable, including when an old URL contains `renderer=typescript`.
