# Example engine completion audit

This checks the seven migration steps in
[the architecture plan](example-engine-architecture.md). It is not a completion
claim. Local execution evidence is from Windows, an NVIDIA GeForce RTX 2080 Ti,
and Chrome 153; other platforms have not been exercised.

| Plan step | Implementation and evidence | Remaining qualification |
| --- | --- | --- |
| 1. Shared artifact boundary | `crates/fresco-artifact` owns manifest types; compiler, WASM transport, and runtime consume them. Structural and malformed-input tests passed in `cargo xtask ci-strict`. | No broader cross-version wire compatibility is claimed. |
| 2. Package authored engine | Canonical sources are under `integrations/example-engine/engine`; native embedding and browser compiler use that bundle. CLI supports explicit engine selection. All 202 compiler tests passed in a separate temporary workspace without the sample engine. Particle and surface-property tests moved to the engine package; all 31 passed. | Core fixtures remain independently authored regression inputs; they must not be regenerated from the sample. |
| 3. Native canvas slice | The rebuilt release executable was copied alone outside the checkout. `--check` and `--hidden --frames 3` passed with embedded source, engine, and texture. The offscreen probe covers authored-stage changes and resource updates. | Interactive native keyboard handling has not been exercised end to end. Native evidence is Windows/Vulkan only. |
| 4. Minimal browser host | Separate compiler worker and renderer WASM modules run without the playground. The copied-package test passed canvas, mesh, and particle selection using only resources from its own server. Standalone scripts cover frame inputs, resize, failed updates, and recovery. | Path transport equality and bounded raster comparison now both pass; see the contract below. |
| 5. Mesh, material, texture, particles | Shared Rust renderers consume emitted stages and reflected resources. CPU and local GPU tests cover material parameters, texture bindings, camera geometry, particle scheduling/state, growth, overflow, partial steps, and reset. All 87 current Rust GPU scenarios passed. | This does not imply support for every external engine contract. |
| 6. Playground integration | Production preview uses the Rust adapter. The build rejects any legacy reference renderer in production chunks. README capture uses the shared renderer. Visualizers own their separate GPU device. TypeScript checking and the adapter/controller unit tests pass. | The TypeScript renderer and development switch are deleted. See [retirement coverage](example-engine-renderer-retirement.md); only CPU reference algorithms remain under `tests/reference`. |
| 7. Publish inspectable sample | Main README links both packages. Integration README walks from profile through compilation, preparation, resources, and rendering. Native/web build commands, examples, support limits, and measured package/startup/frame costs are documented. Both independently runnable demos have passed. | Measurements describe the recorded builds and machine, not universal performance guarantees. No release publication or push is implied. |

## Decisions and limits

The [implemented path comparison contract](example-engine-path-parity-decision.md)
requires exact matched-data constant/storage RGBA equality and limits differences
between the two/66-segment shader forms to one RGB level at 0.01% of pixels,
with exact dimensions and alpha. The full browser smoke passed after the
maintainer requested the fix. Six comparator boundary tests and all four existing
Rust path-packing tests pass. No compiler or renderer workaround was introduced.

Multipass declarations do not establish executable support. The current renderer
validates dependency graphs and target layouts, then rejects canvas execution
requiring intermediate targets. The plan permits this where emitted contracts
do not fully specify executable stages; this audit does not claim multipass
rendering has been implemented. Particle reverse playback, nonzero seeking, and
looping also report explicit unsupported operations.

## Current local evidence

- `target/compiler-isolation-current.log`: 202 compiler tests without sample sources.
- `target/example-engine-portable-check.json`: copied native executable hash and successful runs.
- `node integrations/example-engine-host/scripts/test-browser-package.mjs`: passed copied static-package checks.
- `cargo test -p fresco-example-engine --test uniforms`: six passed.
- `npm run test:unit`: 196 passed; `npm run typecheck`: passed.
- `target/example-engine-final-ci-strict.log`: strict workspace gate passed, exit 0.
- Browser-target renderer Clippy with `--locked -- -D warnings`: passed.
- `target/gpu-test-results/results.json`: 87 Rust-renderer GPU tests passed in 4.5 minutes.
- `test-playground-engine.mjs`: production playground parameters, arrays, textures, mesh selection, particle playback/reset, visualizers, and failure isolation passed.
- `cargo-deny` 0.20.2: advisories, bans, licenses, and sources passed against the freshly fetched advisory database. The initial check found RUSTSEC-2026-0192 through Winit's optional title-font parser. The host now uses `wayland-csd-adwaita-notitle`; Wayland client decorations omit title text, while retaining window controls. No advisory was ignored and no policy was relaxed.

Reports under `target` are local evidence, not committed baselines. Hardware GPU
tests remain local-only; the CI workflow builds native/WASM code and runs CPU and
web unit checks without installing browsers.
