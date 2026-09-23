# Hardware GPU regression checks

Run from `crates/fresco-wasm/web` on a machine with WebGPU and Google Chrome:

```sh
npm ci
npm run test:gpu
```

The suite runs every spec against the shared Rust engine, including editor,
parameter forms, canaries, cellular geometry, directional AA, gradients, variants,
mesh properties, runtime inputs, and particle state. Assertions and image tolerances
are unchanged. The adapter schedules frames and observes GPU validation errors;
rendering and resource preparation belong to Rust.

After building the WASM packages, run the suite without rebuilding:

```sh
npx playwright test --config=playwright.gpu.config.mjs
```

Set `FRESCO_GPU_BROWSER=msedge` to use installed Microsoft Edge instead.
The command syncs examples and rebuilds WASM to avoid validating stale artifacts.
The test starts its own localhost Vite server on port 5189 and headless browser,
then closes them. It requires a hardware adapter and fails rather than skipping
when WebGPU is unavailable or reports software rendering. Generic hosted CI
workers are not assumed to provide hardware graphics; run this suite on a
GPU-equipped workstation or runner.

The fixture uses the shipped engine bundle, WASM compiler, and production canvas
and surface renderers. There are no mocked GPU calls. Playback controls supply
deterministic time; real browser wheel events exercise the orbit handlers.
Canvas readback compares actual rendered RGB pixels, with small byte tolerances
for device rounding. The tests check:

- Badge rotation and exact repeatability after seeking back.
- Flipping-card stripe coverage at 0, 65, 80, 85 and 88 degrees against a live
  reference rendered at four times the width and height, then box-downsampled.
  This catches isotropic AA blur without storing image baselines.
- Square, brick, hex, jittered, and Voronoi ownership and seeded randomness against
  an independent CPU reference, including negative coordinates and cell seams;
  cellular shape coverage against a live higher-resolution render.
- Independent time, delta and resized-resolution values in both renderers.
- Linear and radial gradient effect colors against analytical color fields,
  including alpha, strokes, color transforms, and independent shape anchors.
- Visible surface rotation after wheel zoom in and out, plus a stationary paused
  control. The UV-colored box is framed so its edges stay visible during zoom.
- Engine-authored vertex and fragment edits change rendered pixels, and existing
  canvas output matches the compiler-owned fullscreen path.
- Engine-authored mesh-factory vertex and material-pass edits change rendered pixels;
  the surface host consumes compiler-emitted entries and vertex layouts.
- Engine-authored particle simulation and shade edits change rendered pixels through
  a real compute dispatch followed by instanced billboard rendering.
- Authored particle spawn, reflected state layout, and extra attributes across compute
  and rendering, including a partial workgroup, paused redraws, and reset; GPU
  storage readback checks the actual particle state.
- Generic gravity/drag/integration modules, preservation of custom attributes,
  temporary downstream records without storage growth, and conditional floor collision.
- Explicit selection of two compile-known fullscreen variants reaches the matching
  specialized stage entries and produces distinct pixels.
- Shader/device validation failures and runtime diagnostics.

`target/gpu-test-results/results.json` contains browser/adapter details, measured
pixel differences, and PNG attachments. Failed tests also save screenshots.
This validates raster output, renderer controls, and the current authored fullscreen
contract. It does not establish full playground UI behavior.

For an optional before/after diagnostic, set `FRESCO_AA_BEFORE` to the absolute
prefix of pre-change CLI outputs (`<prefix>.wgsl` and `<prefix>.json`, produced
with `--emit wgsl` and `--emit manifest`). The directional-AA test attaches these
additional renders and error measurements; assertions always use the freshly
compiled shader and live supersampled reference.

The native acceptance probe is also local-only:

```sh
cargo run -p fresco-example-engine --example offscreen
```

Its default invocation executes all checks. To investigate baseline canvas, mesh,
factory-variant, particle, and intermediate-target behavior independently of the
longer advanced-style checks, use `-- --baseline-only`. Existing focused options
such as `-- --fur-only` and `-- --mixed-styles-only` remain available. A selected
slice does not replace coverage of the other slices when validating a broad change.

`-- --placement-only` compares explicit integration points with inferred resource
ports in all three renderers. It checks exact pixel equivalence with multiple
materials, repeated ranges, and early compute whose returned data changes pixels.
