# Brick texture

`brick_tex.png` is a 512×512 opaque PNG generated with the built-in image
generation tool, then downsampled with bicubic filtering and finished for wrapping. It replaces the former
128×128 placeholder at the same asset path. Its four running-bond courses preserve
alternating rows across vertical repeats.

The generated tile initially had visible border discontinuities. The finishing
pass trimmed excess border mortar (4 pixels from each horizontal edge and 6 from
each vertical edge), resampled to 512×512, and blended opposing 24-pixel strips
with a quadratic falloff. Horizontal blending preceded vertical blending so the
corners are consistent too. The first and last pixel columns match exactly, as
do the first and last rows. Interior pixels beyond the strips retain their detail.
Both a 2×2 repeat and a half-tile-offset view with the joins centered were inspected.

Final generation prompt:

> Generate one square seamless tiling brick albedo texture. 512x512 desired. Photoreal warm reddish terracotta fired-clay brick, fine pores and small weathered chips, narrow warm gray mortar, flat even diffuse lighting. STRICT STRUCTURE: exactly FOUR horizontal brick courses, each exactly one quarter of image height. First row: two complete long bricks. Second row: half brick, complete brick, half brick. Third row: two complete bricks. Fourth row: half brick, complete brick, half brick. FOUR ROWS TOTAL, absolutely no fifth row. Each full brick approximately twice as wide as tall. Topmost and bottommost borders cut through midpoint of horizontal mortar joint. Left and right match seamlessly with half-width vertical mortar at edges in full-brick rows and continuing clay at edges in offset rows. This is a single periodic texture tile, no border or frame, no perspective, no text, no diagrams, no previews, no shadows or gradients of illumination. Clearly visible realistic surface detail and subtle natural color variation. Prioritize the four-row repeating structure.
