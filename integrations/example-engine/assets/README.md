# Embedded sample assets

`checker.png` is an original 8×8 RGBA8 checker created for this sample, licensed
under the same MIT-0 license as the example engine. It has opaque orange and blue
tiles and no color-profile metadata. Its public asset identity is
`example://checker`; both hosts receive the same embedded PNG bytes.

The runtime uploads decoded bytes to `rgba8unorm`, without implicit gamma
conversion or alpha premultiplication. PNG/JPEG decoding runs in shared Rust
code. The sample sampler repeats in both axes and uses linear filtering.
