# Basic tiled deferred renderer

Choose **Deferred** in either browser host's **Renderer** selector. Selection
recompiles the engine configuration without editing the material. Native hosts use:

```powershell
cargo run -p fresco-example-engine-host -- --renderer deferred --source "examples/40) surface shaders/deferred.fr"
```

The same Rust/wgpu implementation runs in native and browser hosts. Opaque and
masked mesh surfaces render evaluated materials into a G-buffer, then a full-screen
lighting pass consumes tiled point-light lists. Translucent surfaces retain the
existing tiled forward path. Canvas and particle rendering are unchanged.

## G-buffer and authored contract

| Location | Format | Contents |
| --- | --- | --- |
| 0 | RGBA8 sRGB | Base color (hardware sRGB encode/decode), reserved alpha |
| 1 | RGBA16 float | Octahedral world normal (RG), perceptual roughness, metallic |
| 2 | RGBA16 float | Linear emissive RGB and ambient occlusion |
| 3 | R32 uint | Engine-table material ID |
| Depth | Depth32 float | Device depth for world-position reconstruction |

This uses 24 bytes of color data plus 4 bytes of depth per pixel. No world-position
attachment is stored. Targets are single-sampled, allocated at the viewport size,
and replaced on resize. The resolve reconstructs positions with the inverse
view-projection matrix and WebGPU's 0..1 depth convention. Singular/non-finite
camera matrices are rejected before frame submission.

`engine/core/05_mesh_contract.fr` declares explicit output locations and typed
`pack_pixels` overloads. The `geometry` fragment entry explicitly evaluates the
surface and invokes packing/coverage code. `engine/config/renderer.fr` chooses
attachment formats and scheduling; `engine/pipelines/30_deferred.fr` contains the
executable fullscreen lighting shader. The shared Rust executor consumes their
reflected entries, resources, and steps. The old separate payload layout and
embedded WGSL implementation are removed.

## Material identity

The engine declares `@table(surfaces, ascending, 1) struct DrawRecord`.
This chooses nonzero `u32` IDs by surface-name order within a bundle.
Zero denotes untouched/background pixels. Declaration reordering preserves IDs;
adding/removing/renaming a surface can change them, so IDs are **artifact-local**,
not persistent asset handles. Installation replaces the corresponding material
table together with the shaders.

IDs are written directly into an integer attachment, never encoded in a float or
interpolated. The lighting pass loads each pixel's ID and uses the manifest-built
material table to select standard PBR, unlit shading, or an authored custom response. The runtime validates
nonzero, unique, bounded identities. `MeshRenderer::material_id_texture()` exposes
the actual attachment for local readback tools.

## Lighting and limits

The resolve uses GGX distribution, height-correlated Smith visibility, Schlick
Fresnel, and energy-conserving metallic/diffuse weighting. Roughness is bounded
away from zero. Point lights use inverse-square attenuation with a smooth finite
radius window. AO modulates the small indirect ambient term; emissive is additive.
Custom schemas derived directly from `base` evaluate their own response during
geometry rendering. Their finished color is stored in the HDR emissive attachment;
shading-model value 0 passes it through without applying standard lighting again.
The custom toon example uses this route for three discrete bands. Its light and
band controls are authored in the sample; it does not use the standard PBR light
list. This preserves custom responses across renderer changes without introducing
style names or lighting rules into the compiler.

Unlit materials bypass direct lighting. Lighting remains linear until the chosen
output target; there is no HDR postprocessing or tone-mapping chain in this sample.

The existing 16px conservative tile-frustum cull supplies two 32-bit masks per
tile, supporting all 64 overlapping lights without list truncation. Deferred
lighting checks the mask before evaluating each light. The default Preview environment shares the shadowed directional light and ambient
fill with Forward and Forward+. Light upload validation and the debug colored
lights are also shared. Culling is not yet tightened with a
depth reduction, and there are no point-light shadows, IBL, motion vectors, TAA, MSAA, or
multi-object scene submission. Translucency uses the Forward+ path with the same GGX response. The sample host still previews
one selected surface; the supported recipe subset is described in [Engine recipe contracts](../../../LANGUAGE.md#passes-pipelines-and-annotations).

Design references: [NVIDIA's deferred material dispatch discussion](https://developer.nvidia.com/blog/?p=78888)
and [compact normal storage](https://aras-p.info/texts/CompactNormalStorage.html).

## Validation

The preview also exposes engine-authored [buffer inspection views](example-engine-buffer-views.md)
for the GBuffer, depth, shadow map, and light tiles.

Compiler stub tests cover typed MRT outputs and unsigned identities without
importing the example engine. Engine CPU tests validate generated WGSL, the ID
table, and inverse camera reconstruction. The local native offscreen executable
reads the integer attachment and checks ID/background coverage, light updates,
and invalid-camera isolation. Browser GPU tests cover material response, masked
coverage, unlit shading, resize, transparency, and selector recompilation. GPU
checks remain local-only.
