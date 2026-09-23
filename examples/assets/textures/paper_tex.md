# Paper texture

`paper_tex.png` replaces the existing paper asset with a 512×512 warm off-white
paper texture generated using the built-in image-generation tool.

The output was downsampled using bicubic filtering with mirrored sampling outside
the source bounds to avoid resizing halos. Opposite 24-pixel edge strips were
blended with quadratic falloff, first horizontally and then vertically. Opposite
edge pixels match exactly, including corners. The final 2×2 repeat was visually
inspected and the playground copy synced from this asset.

Generation prompt:

> Generate a single seamless repeating paper texture for use as a shader background. Square 512x512 desired. Natural warm off-white uncoated fine drawing paper, very subtle short cellulose fibers and fine tooth, understated irregular microscopic grain, faint ivory tonal variation. Clean paper stock, no dramatic cloudy patches or prominent flecks. Flat orthographic closeup of material only, edge-to-edge texture with perfectly uniform diffuse lighting. All four edges must wrap continuously: identical average tone and fine texture continuity left to right and top to bottom. Low contrast suitable behind readable text. No sheet edges, folds, creases, stains, writing, objects, border, vignette, cast shadows, highlights, or directional lighting. Single tile only, not a repeating preview grid.
