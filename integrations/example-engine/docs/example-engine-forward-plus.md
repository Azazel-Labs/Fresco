# Basic forward+ in the example engine

The shared Rust mesh renderer supports an opt-in tiled point-light path on native
`wgpu` and browser WebGPU. Try
[`examples/40) surface shaders/forward_plus.fr`](../../../examples/40%29%20surface%20shaders/forward_plus.fr)
in the playground, or run:

```powershell
cargo run -p fresco-example-engine-host -- --renderer forward-plus --source "examples/40) surface shaders/forward_plus.fr"
```

The standalone browser sample selector also includes **Mesh: forward+ point
lights** after running `node integrations/example-engine-host/scripts/build-web.mjs`.
Choose **Forward**, **Forward+**, or **Deferred** in the **Renderer** selector in either browser
host. Switching recompiles the current source bundle with the selected engine
configuration and installs the resulting shader and GPU resources. The material
source stays unchanged. The playground preserves the choice in its URL; the preview blanks immediately on a source or renderer change and remains blank
if compilation fails. Compilation currently processes the
whole bundle; this is not an incremental compiler cache.

The host replaces `engine/config/renderer.fr` with the chosen renderer?s
configuration constant. It supplies the engine-owned
`forward_plus` permutation. This property appears as noneditable in the manifest
and cannot be overridden by a material's `properties` block. Other engine files
remain editable. Canvas and particle paths continue to work in either mode;
the selection changes mesh lighting.

## Lighting environments and normal maps

**Preview** is the default in every host and renderer. It is an adjustable studio
look, currently a warm shadow-casting directional light with a cool sky/ground
ambient fill. Shadowed regions retain ambient light. The name does not promise a
fixed number or arrangement of lights.

The playground's sun menu separates Preview from the debug modes **Unlit**,
**One directional light**, and **Three colored lights**. Selection updates a
uniform without recompilation and survives renderer/example changes. The menu
is hidden when the installed engine has no active lighting-environment input;
canvas and independent engines do not need a special example-name check.
Forward's debug directional mode retains the authored schema evaluator;
Forward+ and deferred use GGX. All three use the same Preview response.

The engine recipe renders a depth-only caster into a `depth32float` image before
shading. Both passes share mesh deformation; masked materials preserve alpha
coverage, and translucent surfaces do not cast opaque shadows. The shaders use
a bilinearly interpolated comparison filter and a resolution-dependent slope
bias. The light projection follows the preview object's model transform. This
is a single-object preview shadow map at viewport resolution, not a cascaded
scene-wide shadow system or image-based lighting. Ambient colors and direct-light
intensity are engine-authored artistic defaults.

The compiler supports sampled factory textures, multiple vertex entries with a
shared interface, and fragment entries without color outputs. It has no shadow
concept. The shared runtime binds only resources used by each entry and rebuilds
image bind groups on resize. The engine owns light direction, projection, bias,
filtering, material coverage, and ambient response.

The standard material's `normal` channel is a world-space normal. Tangent-space
texture normals belong in `normal_map`; its default is `(0, 0, 1)`. Mesh shading
uses interpolated mesh tangents with UV-derived handedness, supports mirrored
UVs, and falls back to the world
normal for degenerate UVs. Deferred stores the resulting world normal, not the
texture's tangent vector. GGX rejects light/view directions below the shading
normal's hemisphere. The crate-side example demonstrates this contract.

## Execution and ownership

1. The host supplies world-space point-light positions, radii, linear RGB colors,
   and intensities. Three colored demo lights are installed for the debug preset.
2. A Rust-owned WGSL compute pass tests each light sphere against the six planes
   of each 16-by-16-pixel tile frustum. It uses the current view/projection and
   viewport every frame. No depth texture or depth prepass is required.
3. Each tile stores two `u32` masks. All 64 supported lights can overlap a tile;
   there is no per-tile truncation or atomic append counter.
4. The compute pass precedes the mesh draw in the same command encoder. The
   Fresco-authored raster hook indexes the tile using fragment pixel coordinates,
   then shades the lights selected by its masks.

Standard materials use the same linear-light GGX BRDF and smooth finite-radius
inverse-square attenuation as deferred. Both paths use the authored material
normal, bounded roughness/metallic values, occlusion on indirect ambient light,
and additive emissive. Unlit materials bypass point-light shading. Transparent
meshes use the same conservative light lists and BRDF, with the existing alpha,
mask, and two-sided policies. Particles retain their existing rendering path.

The helpers remain local to each executable pass because imported top-level GPU
helpers are not yet emitted by both backends. The native GPU regression compares
Forward+ and deferred pixels, allowing only G-buffer encoding error.

The active source locations are:

- [Mesh contract](../engine/core/05_mesh_contract.fr):
  reflected light buffers and the authored per-fragment light loop.
- [Surface properties](../engine/core/07_surface_properties.fr):
  the engine-owned static switch.
- [Runtime](../src/runtime/forward_plus.rs) and
  [authored culling shader](../engine/pipelines/40_gpu_programs.fr):
  validated uploads, tile storage, and compute dispatch.

`engine/pipelines/20_forward_plus.fr` remains the richer staged clustered-pipeline
vocabulary (depth slices, spot lights, shadows, fog). This implementation does not
claim to execute that general pipeline graph. The basic path uses the existing
executable mesh contract and a shared engine compute pass.

## Supplying lights

Rust integrations call `MeshRenderer::set_point_lights(&[PointLight])`. Browser
integrations call `BrowserEngine.set_point_lights(JSON.stringify(lights))`:

```js
engine.set_lighting_environment("three-lights");
engine.set_point_lights(JSON.stringify([{
  position: [0, 1, 2], radius: 4,
  color: [1, 0.25, 0.1], intensity: 2
}]));
```

An empty array clears all point lights. The entire update is validated before any
GPU write: more than 64 lights, non-finite values, non-positive radii, or negative
color/intensity are rejected, preserving the installed lights. A new shader or
mesh installation restores the demo light set; hosts should reapply their own
lights after installation. GPU/device reconstruction follows the same rule.

The maximum viewport is 8192 pixels per dimension. Enabled meshes reserve 2 MiB
for tile masks, avoiding allocation during resize; disabled meshes use a tiny
placeholder mask buffer and never dispatch the cull. Zero-sized frames suspend
rendering. `MeshRenderer::light_tile_masks()` exposes the last grid for readback:
row-major tiles, two words each, least-significant bit first.

Point-light shadows, spot/directional lights in the tiled list, depth-restricted culling,
cluster depth slices, shared scene-wide culling across multiple draws, and image-based
lighting are outside this implementation.

## Validation

CPU tests validate light packing, invalid inputs, viewport bounds, generated WGSL,
and reflected bindings. The compiler's own stub-based regression covers read-only
factory storage, integer casts, indexing, and range loops without depending on
the example engine. Typed executable mesh hooks preserve scalar conversions;
the signal-expression parser's historical cast behavior is unchanged.

The local native `offscreen` example compares tiled output exactly with an
all-lights reference, checks actual GPU masks for accepted/rejected lights,
exercises all 64 overlapping lights, camera motion, clearing, and rejected edits.
The browser GPU regression covers live updates, failed-update isolation, partial
tiles, and resize restoration. GPU checks remain local-only.
