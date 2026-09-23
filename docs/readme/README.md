# README samples

The root README and Fresco Lab share the `.fr` sources under `examples/`.
Edit an example once, then run `cargo xtask readme-sync` to refresh its README
code block. Lab's `npm run sync:examples` copies the same sources into its
generated catalog; do not edit generated code blocks or catalog copies.

[`samples.json`](samples.json) selects each snippet by a stable `id` and a
repository-relative `source` path. It also owns preview settings, so code and
images use the same program. For example:

```json
{
  "id": "badge",
  "source": "examples/10) fundamentals/badge.fr",
  "duration": 18,
  "alt": "Rounded pink badge rotating with a shadow and white outline"
}
```

To add a snippet, choose or create an example, add its manifest entry, and place
these markers in the root README:

```markdown
<!-- readme:sample badge -->
<!-- readme:end -->
```

The sync command inserts the entire source file as a fenced Fresco block and
compiles each selected program. Paths may contain spaces. Renaming an example
only requires updating its manifest path; README markers retain their IDs.
Unknown or duplicate IDs, missing sources, unused manifest entries, malformed
markers, unmanaged Fresco fences, and compilation errors fail the update.

For an engine contract excerpt, set `entrypoint` to the example that imports it
and `preview: false` to omit a separate image. The `canvas_contract` entry uses
`examples/10) fundamentals/engine_canvas.fr` to compile the shipped contract with its dependencies.
Other snippets compile and render their `source` directly.

Both workspace CI gates refresh snippets automatically. `cargo xtask readme-sync
--check` is a read-only drift check. Commit regenerated README changes with the
source; CI updates its checkout but does not commit it. Compilation validates
semantic checking, Naga IR, WGSL emission, and manifest emission. Use rendered
previews and hardware GPU tests to check appearance and execution.

## Render README previews

From `crates/fresco-wasm/web`, with Node dependencies, Rust, wasm-pack, Python,
and Chrome installed:

```sh
python -m pip install -r scripts/readme-media-requirements.txt
npm run readme:render
```

This first syncs/compiles the README snippets, then rebuilds WASM and renders only
stale or missing previews using isolated release packages under
`target/readme-media/packages` (separate from a running playground build) and the production compiler, engine bundle, and
shared Rust/wgpu host. WebGPU must work; failed compilation, missing textures, or
GPU diagnostics fail rather than generating a substitute image. Use
`FRESCO_GPU_BROWSER=msedge` for Edge, or `chromium` for Playwright's installed
Chromium. `FRESCO_README_SOFTWARE_GPU=1` requests software rendering for machines
without hardware graphics; this does not replace hardware GPU regression tests.

`samples.json` owns source paths, dimensions, frame rates, durations, texture
fixtures, and alt text. `mesh` selects `box` (default) or `sphere`; `warmup`
advances simulation for up to ten seconds before capture. Animated captures pass
a fixed `1 / fps` delta to the host so particle state advances. A captured simulation
repeats in the WebP player but is not necessarily a seamless periodic loop. Add an entry when adding a runnable README snippet.
Still samples generate one WebP frame; animated samples default to 60 frames per
second and use deterministic times over one cycle, excluding the duplicate
endpoint, and loop forever. WebP uses alternating 16/17 ms frame durations to
preserve the cycle length at 60 fps. No motion is invented for static samples.
The texture example explicitly receives the shipped
brick texture from the host. Intermediate PNGs remain in `target/readme-media`
for inspection. Still images use lossless WebP. Animations use mixed lossy/lossless
encoding at quality 85 with size minimization, keeping the 60 fps capture timeline
and full loop duration. Consecutive frames that compress to identical pixels may
be combined with their durations added together. Animated pixels may differ
slightly from the original captures. Each preview's encoding settings are recorded
in the output manifest.

Commit `media/*.webp` and `media/manifest.json` with source and README changes.
The manifest holds SHA-256 input/output hashes and capture provenance, including
browser, adapter, encoder, and frame counts. Inputs include sample/import sources,
engine files, compiler and renderer sources, lockfiles, capture code, settings,
and textures. Text is normalized to LF for hashing. Missing or modified images
invalidate the cache too. `npm run readme:render -- --force` refreshes everything,
including after a browser/GPU update; device versions are recorded as provenance,
not used to invalidate otherwise valid previews across machines.

The Previews workflow checks README snippet drift and compiles the sample programs
on relevant pushes/PRs. It does not render images or require WebGPU, WASM, Node,
or Python. Generate previews locally with the render command above and commit
the images and manifest to update the repository's displayed README images.
