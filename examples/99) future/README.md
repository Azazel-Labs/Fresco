# `examples/0) north-star/` — the aspirational gallery

Programs we *wish* compiled. Every file here is allowed — expected — to fail
compilation today. Each is the acceptance test for the language features it
names, per the method in the extensions doc (§22):

1. CI compiles every file here anyway and records the error list.
2. A feature's Definition of Done is stated as *"moves `north-star/<file>` from
   N errors to M"* — ideally to zero.
3. When a file compiles clean AND its `--explain` receipt matches the
   expectations written in its header, it graduates: move it to the numbered
   examples, keep a tombstone line here.

## Header convention

Every file carries:

```
// north-star: <name> — <one line>
// status: aspirational
// rung: <staging ladder position, ext §31>
// deltas: <the SEMANTIC deltas this file exists to force — not syntax gaps>
// done when: <observable acceptance criteria, including --explain receipt lines>
```

If you add a file without naming its deltas, you haven't finished designing it.

## The gallery

| # | File | Rung | Forces | Audience/demo value |
|---|------|------|--------|---------------------|
| ~~01~~ | ~~`cloudscape.fr`~~ | ~~1~~ | **GRADUATED** → `examples/90) gallery/cloudscape.fr` | `fn -> layer` helpers + `field expr at coord` bump lighting; validates deck ergonomics |
| ~~02~~ | ~~`polka-field.fr`~~ | ~~2~~ | **GRADUATED** → `examples/01) basic/repeat_cell_polka.fr` | Cheap named repeat-cell bindings with stable per-cell variation |
| 03 | `kaleido-bloom.fr` | 2 | `kaleido`; bleed rings from effect footprints | Seam-free mirrors; the bleed-policy proof |
| 04 | `progress-dial.fr` | 2/7 | polar wrap; `anchor: path` gradients | Workbook's best scrub demo; consumer UI |
| ~~05~~ | ~~`star-to-circle.fr`~~ | ~~2.5~~ | **GRADUATED** → [star_to_circle.fr](../20%29%20techniques/star_to_circle.fr) | Unlocked by `mix(shape, shape, t)` + `ease(t, curve)` standalone builtin |
| 06 | `newsprint.fr` | 3 | image params; `at` operator; **cell-rate evaluation** | Halftone; tuition for ray-rate hoisting later |
| 07 | `comet-trails.fr` | 4 | `previous_frame` minimal loop | Feedback regression anchor |
| 08 | `smear.fr` | 4 | host-driven params; simplest `through` | Pointer paint |
| 09 | `gray-scott.fr` | 4 | runnable `grid`; grid→canvas cut | Reaction–diffusion; §14 proof |
| 10 | `settling-snow.fr` | 4+ | scatter × feedback composed; `remember` layers | Composition-bug hunting ground |
| 11 | `bar-chart.fr` | 5 | `param` arrays; `each` unrolling; `anchor: shape` | **GRADUATED** → `examples/20) techniques/bar_chart.fr` |
| 12 | `photo-lab.fr` | 5 | samplemap category; non-separable kernels; `global` | `--explain` is the star; threshold tuning |
| 13 | `crt.fr` | 6 | the `through` contract; piecewise warps | Beloved target; closes the warp spec hole |
| 14 | `glitch-kit.fr` | 6 | **user-defined samplemap spaces**; datamosh via feedback | Community ask (XtyXzW-style); shippable stdlib module |
| 15 | `hex-glitch.fr` | 2+3+6 | hex cells; cell-rate branching; `through` in cells | Community ask (lfscD7); stress test as eye candy |
| 16 | `route-reveal.fr` | 7 | `path` primitive; trim/dash; arc-length honesty | Draw-on animation; prerequisite for text |
| 17 | `fluid-paint.fr` | 4-ext | grid→grid reads; `advect` with declared bound | Community ask (multi-buffer); buffers as *output* |
| 18 | `crate-side.fr` | 8 | `surface`/`material`; channel fold; `normal_from` | The audience move begins |
| 19 | `hull-panel.fr` | 8 | `inset_bevel`; emissive channel effects | Greebles without geometry |
| 20 | `terrain-splat.fr` | 8 | world unit `m`; position/normal-driven masks | The classic splat, five entries |
| 21 | `relief-wall.fr` | 8+ | **parallax as samplemap space**; relief self-shadow | Community ask (parallax + animated clouds) |
| 22 | `holo-card.fr` | 8 | `material(unlit)`; view-dependent scatter emits | Community ask (holo foil); doubles as canvas demo |
| 23 | `flip-card-3d.fr` | 9 | volume v0; camera space; two-sided `shade`; **surfaces shading shape3 faces** | Retires the 2.5D flip-card workaround |
| 24 | `sky.fr` | 10–11 | density; transmittance fold; `lit_by`; march rewrites; temporal | The demo that sells Part VI |
| 25 | `terrain-under-sky.fr` | 11 | volume↔surface interop; cached transmittance; host depth | Cloud shadows in one line |

## Reading order

By rung if you're implementing; by cluster if you're designing:

* **Spaces**: 02 → 03 → 15 → 13 → 21 (repeat → bleed → hex+through → warp contract → iterating samplemaps)
* **Rates**: 06 → 15 → 24 (cell-rate → cell-rate branching → ray-rate)
* **State**: 07 → 08 → 09 → 10 → 17 → 24 (feedback → grids → composition → multi-buffer → temporal)
* **Painter's model in new clothes**: 18 → 23 → 24 → 25 (material fold → surface-on-shape3 → transmittance fold → cross-entry interop)

## Second wave (named, not yet written)

Parallax starfield (stochastic scatter fallback), confetti burst (instance-local
spaces; footprints from transformed bounds), fireflies (footprint growth from
glow), magnifier lens + water-ripple-refraction (piecewise `through`; grid
normals feeding warp), page curl (discontinuous Jacobian honesty note),
cross-hatching + Ben-Day + dither/palette (print-look kit), sparkline +
skeleton-shimmer + toggle (consumer kit round 2), `text_outline` reviving the
aspirational neon sign (after 16).

## Current graduation target (planning cycle)

Target file: `07-comet-trails.fr`

Dependency checklist for graduation:

- [ ] `previous_frame` MVP in compose with explicit ping-pong manifest contract
- [ ] `remember`/feedback cycle diagnostics (static cycle detection + actionable error)
- [ ] Explain receipts for feedback edges and lifecycle of persisted targets
- [ ] One focused regression test that compiles the file and asserts expected
      feedback-related explain lines

Graduation target:

- Move to `/home/runner/work/fresco/fresco/examples/20) techniques/` after zero
  compile errors and stable explain receipts matching its header criteria.

## The graduation bar

Compiling is necessary, not sufficient. A file graduates when:

1. Zero errors, zero warnings it doesn't explicitly acknowledge in a comment.
2. `--explain` contains every receipt line promised in the header's `done when`.
3. The workbook interactions named in the header actually work.
4. Someone who didn't write it can read the source and predict the picture.

Criterion 4 is the language's whole thesis. If a north-star file compiles but
reads like soup, the feature design failed even though the implementation
succeeded — send it back to §22 step 1.
