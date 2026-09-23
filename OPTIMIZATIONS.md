Grouped by how strong the safety guarantee is, since "no side effects" has a crisp version (bit-identical output) and a weaker one (identical given a precondition you can check).

# Tier 1 — Unconditionally safe, bit-exact

**1. Constant folding over the whole literal subgraph.**
`max(abs(0.08), 1e-6)`, `1.84 * 0.5`, `(1.5 * 0.5)`. Any node whose operands are all literals folds. Run to fixpoint — this is the enabling pass for almost everything below, so schedule it first and re-run after each rewrite. Status: implemented in checker scalar folding.

**2. Algebraic identity elimination.**
`x - 0`, `x * 1`, `x / 1`, `min(x, x)`, `max(x, x)`, `clamp(x, x, x)`. One caveat worth encoding: `x + 0.0` is *not* an identity when `x` is `-0.0`, while `x - 0.0` is. If you want strict bit-exactness, rewrite `x + 0.0` → `x` only when you can't prove the sign, or just accept it (no shader cares about signed zero coverage). Status: safe scalar and same-operand cases implemented; broader zero/one simplifications still open.

**3. CSE after operand normalization.**
`1.5 * _e180` appears six times; `length(p_space_1 - vec2(0,0))` twice. Sort commutative operands into a canonical order before structural hashing, or you'll miss `a*b` vs `b*a`. Safe unconditionally in straight-line code. The one rule: don't CSE *across* a derivative op if you ever introduce non-uniform control flow, since derivative validity depends on quad uniformity. Status: implemented for scalar `+` and `*` normalization.

**4. Dead code elimination on the emitted source.**
`outline_half_w_s15_`, `d_s15_`, and the same pair for s11/s13/s16/s17 — ten statements never read. The driver DCEs these anyway, so this is purely about emitted-source size: compile time, cache size, log noise, and any per-shader source limits you might hit.
Status: partially implemented by trimming single-use outline temps, AA width labels, and one outline fade temp from emitted WGSL; broader source-level DCE remains open.

**5. Unused parameter and struct-field elimination.**
`ctx.aa`, `ctx.delta`, `time`/`t`. Interprocedural, safe once all call sites are visible. Also lets you shrink the uniform upload.

**6. Function deduplication.**
`fresco_gridlines` and `fresco_gridlines_pass0_` are byte-identical. Structural hash modulo parameter renaming, emit once. Status: implemented for reusable lowered helper functions through the module-builder dedup cache.

**7. Monotone-function hoisting.**
For `f` monotonically increasing on the operand range: `max(f(x), f(y)) → f(max(x, y))`. `sqrt` qualifies on `[0, ∞)`, so `max(length(a), length(b))` → `sqrt(max(dot(a,a), dot(b,b)))`. Exact — same `sqrt`, same argument, one instead of two. Generalizes to `min`, to `exp`/`log`, and to n-ary reductions.

# Tier 2 — Safe given a precondition you can check

**8. Affine derivative propagation.**
Precondition: `q = A·p + b` where `A`, `b` are uniform-rank. Rewrite `dpdx(q)` → `A * dpdx(p)`. Exact because `dpdx` is linear and hardware finite-differencing across a quad is exact for affine functions. Compose `A` through nested spaces. Do **not** apply to nonlinear spaces (polar, fisheye) — the analytic Jacobian is a different value than the hardware difference there, so you'd be changing output.

**9. Symbolic scalar factorization.**
This is the general form of the px-stroke win, and probably your highest-value pass. Represent values as `(compile-time coefficient, runtime symbol)` pairs. Then, given `s ≥ 0`:

```
max(a·s, b·s) → max(a,b)·s        // ordering preserved only if s ≥ 0
(a·s) / (b·s) → a/b               // and b ≠ 0
(a·s) - (b·s) → (a-b)·s
```

The sign precondition holds for anything rooted in `length()`, `abs()`, or `max(_, positive_literal)` — which covers your `fw`. This is what collapses `thin_fade`, `locked_w`, and the whole `max`/ratio chain to literals whenever the widths are px-unit.
Status: partially implemented in checker scalar factorization for shared positive symbolic terms through `max`/`min`, subtraction, and division.

**10. Interval analysis to kill saturating ops.**
Propagate `[lo, hi]` bounds through the DAG. Then drop `clamp(x, 0, 1)` when `x`'s interval ⊆ [0,1], drop `min(x, k)` when `hi ≤ k`, drop `max(x, k)` when `lo ≥ k`. Fires on chained coverage multiplies where a clamp already ran upstream. Sound as long as your interval rules are conservative on `fract`/`sin`/division.

**11. Group-alpha liveness.**
Precondition: a compose group has no group opacity, no non-`over` blend mode, no mask, no backdrop read. Then the accumulated alpha channel is dead and layers composite straight onto the parent — deleting the whole `compose_over_a_*` chain. Separately: if the backdrop is provably opaque and full-coverage, the final `mix(bg, group, group_a)` collapses to the premultiplied form.

**12. Full-coverage shape elision.**
Precondition: the shape's SDF is `≤ -aa_max` over the *entire* sampled domain — note the margin. A rect that exactly matches the domain has `d = 0` on the boundary, giving coverage 0.5 in the outermost half-pixel, so eliding it without the margin check changes edge pixels. Your background box is exactly this case, which is why the check matters. When it fires, it kills the coverage math *and* any derivatives feeding only it.

**13. Occlusion DCE.**
If a layer is opaque and provably full-coverage over the domain, every layer beneath it is dead. Rarely fires, cheap to test, and worth having because generated content hits it more often than hand-written.

**14. Uniform-rank promotion to the CPU.**
Anything whose inputs are all uniforms/literals — `px`, the aspect ratios, `space_centered_inv_t0_` — moves into the uniform buffer. Precondition: no derivative op in the subtree. Bit-exactness caveat: compute in `f32` on the host, not `f64`-then-narrow, or you'll get last-ulp divergence from the GPU path. Usually irrelevant, occasionally not if you're comparing renders across backends.

# Tier 3 — Changes float results; gate behind a flag

**15. Common-denominator reciprocal hoisting.**
n divides by the same value → one `1.0/d` plus n multiplies. Off by up to ~1 ulp per site, so it's a fast-math transform, not a free one. Your six coverage divides reduce to one reciprocal.

**16. Affine reassociation inside clamp.**
`clamp(0.5 - (d - h)/aa, 0, 1)` → `clamp(K - d·inv_aa, 0, 1)` with `K = 0.5 + h/aa` folded. Requires #9 to prove `h/aa` is constant, and reassociates the subtraction. Same flag as #15.

# Notes on scheduling

Passes 1, 9, and 10 are mutually enabling: folding literals exposes shared symbolic factors, which collapse `max`es into constants, which tightens intervals, which kills clamps, which exposes more literals. Run the trio to fixpoint rather than once each. Passes 8 and 12 should run before general DCE so the newly-dead derivative pairs get collected.

Two that are code-size-only, worth separating in your own mind from the speed work: extracting the outline-coverage chain into a `fn` (drivers inline it, so it's ~60 lines → ~10 with no perf delta), and packing the x/y line families into `vec2` lanes with a component-wise `min`. The latter halves the emitted statements but buys little on scalar GPU ISAs.
