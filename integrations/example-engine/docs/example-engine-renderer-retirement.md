# TypeScript engine retirement

The playground now uses the shared Rust example engine in development and
production. The canvas/surface TypeScript renderers, GPU texture pool and uniform
uploader, backend selector, reload handoff, and comparison-only browser script
have been deleted. Old `renderer=typescript` URLs have no effect.

Editor controls and visualizer thumbnails remain TypeScript. Thumbnails own their
GPU device separately; they do not provide an alternative engine implementation.
The two CPU algorithms under `tests/reference` support independent geometry and
particle-allocation tests and contain no renderer.

## Regression ownership

Renderer-specific mock tests have moved to the behavior's current owner. No GPU
spec was dropped or given wider image tolerances. Every spec now runs in the Rust
project, including editor completion and parameter forms previously assigned only
to the TypeScript project.

| Previous test responsibility | Current coverage |
| --- | --- |
| Canvas variant exact selection, unknown/extra axes, sole implicit specialization | `integrations/example-engine/tests/canvas_variant.rs`, GPU `engine-variants.spec.mjs` |
| Reject helper-only canvas output without losing the installed image | Native `examples/offscreen.rs` explicitly removes `engine_pass`, requires preparation failure, then verifies unchanged pixels |
| Default arrays, map-valued defaults, decimal normalization, mixed parameter types | Web `preview-manifest-defaults.test.js`, `preview-controller-defaults.test.js`, `example-engine-defaults.test.js` |
| Dynamic array GPU allocation, empty minimum stride, edits and replacement | Rust `tests/storage.rs`, `tests/parameters.rs`, native offscreen storage readback, production playground array edits; CPU `storageBufferByteSize` assertions remain |
| Wheel preserves spin and forwards both zoom directions; drag stops spin | Web `surface-orbit-controls.test.js`; Rust `tests/camera.rs` checks inverse zoom; GPU `runtime-inputs.spec.mjs` and playground drag/cancel checks |
| Automatic/explicit surface selection, lighting metadata ordering, unchanged compiler stages | Web `surface-renderer-texture-bindings.test.js` now tests the adapter and metadata helper; `example-engine-renderer.test.js`; production automatic-versus-explicit surface pixel checks |
| Mesh entries/layout, stage visibility, material textures, blend/depth/culling | Rust `tests/vertices.rs`, `tests/surface_parameters.rs`, native `examples/offscreen/mesh.rs`, GPU `surface-properties.spec.mjs` and `runtime-inputs.spec.mjs` |
| Particle compute/raster dispatch, reflected stride, state and frame uploads | Rust particle layout/playback tests, native offscreen particle modules, GPU `particle-state.spec.mjs` and `runtime-inputs.spec.mjs` |
| Required UV streams and invalid authored stages | Rust `tests/preview_requirements.rs`, native offscreen preparation rejection and standalone browser image-preservation checks |
| Arbitrary uniform fields, frame/custom sparse bindings, zero padding, missing-value diagnostics | Rust `tests/uniforms.rs`; GPU `runtime-inputs.spec.mjs` checks time, delta, resolution in canvas and mesh output |
| Actual sparse resource groups and replacement isolation | Native offscreen texture/storage and mesh readback checks |

Old assertions about JavaScript `destroy()` call counts and `storageParamsDirty`
flags described an implementation that no longer exists. Rust owns resource
lifetimes; replacement and failure isolation are checked by actual rendering and
readback. Backend-handoff tests are removed together with that deleted feature.

Multipass execution and particle reverse/seeking/looping limits are unchanged by
this removal; see the sample host README for supported operations.
