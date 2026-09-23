# Example engine buffer inspection

The browser preview's **View** selector lists the views supported by the installed
mesh recipe. Changing views updates an engine uniform and redraws; it does not
recompile the material or reload the page. Compatible selections survive shader,
texture, and mesh replacement. An unavailable selection resets to **Shaded**.
Canvas and particle previews, and the basic Forward recipe, have no buffer selector.

Forward+ exposes Shaded, Sun shadow depth, Tile light count, and Depth.
Deferred additionally exposes Albedo, World normals, Roughness, Metallic,
Emissive / custom response, Occlusion, and Material IDs. Translucent surfaces use
the Forward+ path and expose only its views, even with Deferred selected.
Forward+ does not allocate a GBuffer just for inspection.

Deferred also offers **View all buffers**: a live 4×3 grid containing Shaded and
all ten individual inspection views, with the final cell unused. Each tile
preserves the scene aspect ratio and uses the same decoding as its individual
view. The browser overlays labels supplied by the engine catalog. Switching to
the overview requires no recompilation or additional geometry passes.

The dedicated fullscreen passes in
`integrations/example-engine/engine/pipelines/50_buffer_views.fr` read the actual
attachments and tile storage produced by the renderer. The renderer recipes wire
these resources explicitly. Shading writes an HDR intermediate image; the final
fullscreen pass either presents that image or visualizes a selected buffer.
These are ordinary authored raster programs, using the same fullscreen rendering
mechanism as a canvas. No GBuffer decoding, visualization WGSL, or renderer policy
is added to the compiler or browser.

Display conventions:

| View | Display |
| --- | --- |
| Albedo | sRGB attachment decoded to linear, then presented normally |
| World normals | Octahedral decoding, then XYZ mapped from −1…1 to RGB |
| Roughness / metallic / occlusion | Scalar mapped to grayscale |
| Emissive / custom response | HDR color compressed with `c / (1 + c)` |
| Material IDs | Stable hash of the integer ID; zero is transparent background |
| Depth | Device depth raised to power 32 for contrast; not linear distance |
| Sun shadow depth | Raw shadow depth, black near and white far |
| Tile light count | Point-light candidates per 16×16 tile, blue at zero to yellow at 64, using a logarithmic scale |

The shared Rust engine owns the view catalog and validates selections against the
active recipe steps. Both native hosts (`MeshRenderer::buffer_views` and
`set_buffer_view`) and the WASM host use this API. The browser only groups and
labels the returned choices. Adding a view means authoring its decode in the
engine shader, wiring any new inputs in the recipe, and updating the engine
catalog.

Tile counts describe the point-light culling data, even when the current lighting
environment uses the sun instead. They are not counts of lights evaluated by
every shading mode. GBuffer views show stored data: custom responses already
shaded into the emissive attachment will not have standard material channels.

The native offscreen test checks actual channel pixels, empty light-tile output,
selection rejection without mutation, and restoration of the shaded image.
Browser adapter tests cover live selection and capability changes. GPU checks
remain local-only.
