# Coordinate domains and coverage filtering

**Status: Exploring ? future potential, not committed.**

Origin: 2026-09-16. See the [proposal index](README.md) for status conventions.

The syntax below
is illustrative and is not currently supported. This document records the
direction discussed after the `polar_dial` AA fix; it is not a claim of general
antialiasing support or an accepted final grammar.

## Goal

Keep authored shapes such as the existing progress bar:

```fresco
let bar = box(
    at: (progress / 2, 0.32),
    size: (progress, 0.045)
)
```

Allow authors to describe coordinate-boundary behavior when the compiler cannot
establish it, without reconstructing the shape as a separate coverage mask or
adding progress-dependent epsilon checks.

Built-in mappings should supply their known boundary semantics automatically.
Explicit declarations are primarily an escape hatch for custom mappings.

## Proposed declaration

```fresco
space dial_space = polar(
    center: center,
    from: -90deg,
    direction: clockwise
) with domain(x: periodic(0 .. 1))
```

The explicit declaration would be redundant for built-in `polar`. It illustrates
the information that a custom mapping could supply. The bar remains filled
normally inside the space: `bar |> fill(#4facfe)`.

`domain(x: periodic(0 .. 1))` describes a canonical coordinate cell `[0, 1)`
whose endpoints connect. A filter footprint crossing one boundary continues at
the other. A pixel near `x = 0.999` can cover both `0.997 .. 1` and `0 .. 0.001`.

This annotation describes the mapping; it does not execute a wrap, resize the
shape, or supply a pixel footprint. It is not merely a numeric range constraint.
For periodic filtering, evaluate the content defined within the canonical cell
and continue that content across the seam. Do not indiscriminately repeat shapes
that lie outside the cell: clip to the canonical domain before periodic extension.

The bar already describes the occupied angular interval `[0, progress]`:

- Zero width has zero coverage through finite-interval filtering.
- A full-domain interval has full angular coverage through periodic filtering.
- Partial intervals retain filtered endpoints, including across the seam.

## Other uses and candidate boundary modes

| Use | Boundary information |
| --- | --- |
| Repeating stripes | One periodic coordinate |
| Tiled patterns | Periodic X and Y, including corner crossings |
| Cylindrical labels or gauges | Periodic angular coordinate; nonperiodic height |
| Custom angular conventions | A different cell, such as `periodic(-0.5 .. 0.5)` |

Potential later modes include mirrored continuation, empty exterior, and
clamping to the boundary value. These need separate definitions for shape
occupancy and sampled fields before choosing syntax. A domain declaration must
not silently imply a boundary mode for unspecified axes.

## Compiler direction and limits

Use a shared coverage definition: average the resulting shape's inside/outside
value over a screen-pixel footprint, with coordinate mappings evaluated at each
sample. Preserve boolean shape semantics before filtering; independently
filtered primitive opacities are not generally equivalent to filtering their
union, intersection, or subtraction.

Analytic formulas are optimizations with explicit applicability conditions.
Finite interval filtering for sharp boxes is one such path. The current
implementation recognizes eligible sharp boxes and records polar domain
metadata, including seam-safe derivatives. It is bounded support, not a general
analysis of arbitrary shader math.

Track domain information and footprints through operations only where the
transformation is established. Scaling changes the period; rotation can change
the direction of the periodic boundary, so an axis-aligned representation alone
is insufficient. Nested mappings and nonlinear warps require deliberate handling.

Do not assume that finding `fract` anywhere makes an entire shader periodic.
`fract`, modulo, `floor`, branches, and nonlinear mappings can introduce different
discontinuities or distortions. Dependencies and mapping semantics matter.
An explicit domain cannot repair discontinuities inside the cell or an incorrect
footprint. Custom mappings need a reliable footprint contract or a sampled path
that reevaluates the mapping at each screen-space sample.

A sampled reference implementation should establish expected behavior and test
analytic optimizations. A production sampling option, if provided, needs explicit
quality/cost controls. Finite sampling can miss small features; neither sampling
nor domain metadata guarantees correct AA for arbitrary math.

## Implementation plan and acceptance criteria

1. Define the domain contract independently of syntax: canonical bounds, boundary
   continuation, unspecified axes, and interaction with existing space semantics.
2. Define scoped domain/footprint representation and propagation rules. Specify
   what happens when a declaration conflicts with a known mapping, or when a
   later operation cannot preserve the representation. Do not silently retain
   invalid metadata or claim filtering support that is unavailable.
3. Build a screen-space reference coverage evaluator with documented sampling
   limits. Compare analytic paths against it before broadening specialization.
4. Integrate built-in mappings and the explicit author escape hatch. Keep shape
   syntax intact; avoid example-name checks or fixed progress thresholds.
5. Validate empty, subpixel, partial, nearly full, and full intervals; both sides
   of seams; non-unit periods; clipping outside the canonical cell; two-axis
   corner crossings; reflection; nested transforms; domain scope isolation; and
   singularities. Exercise multiple resolutions and zoom levels.
6. Add GPU image comparisons and performance/code-size budgets alongside numeric
   and lowering tests. Rebuild the WASM playground for visual validation.

Open decisions include final grammar, author-supplied footprint syntax, handling
of unsupported mappings, sampling controls, and the semantics of additional
boundary modes. Coordinate-field access should follow the
[current context semantics](../../LANGUAGE.md#canvas-contracts-and-context), rather than depending on an
identifier being named `uv`.
