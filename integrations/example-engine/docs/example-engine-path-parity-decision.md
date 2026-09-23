# Path comparison contract

Status: implemented after the maintainer requested the blocked comparison be fixed.

The standalone browser smoke test currently requires byte-identical PNGs for a
two-segment cubic path and the same geometry repeated to make 66 segments. The
compiler uses constants for the short path and storage for the larger path.

The local diagnostic (`integrations/example-engine-host/scripts/diagnose-path-parity.mjs`)
recorded the following at 514 by 514 pixels on Chrome 153.0.8010.48:

| Compared with storage, 66 segments | Changed pixels | Maximum channel difference |
| --- | ---: | ---: |
| Constants, same 66 segments | 0 | 0 |
| Constants, two segments | 7 | 1 |
| Private array, two segments | 7 | 1 |
| Storage, 66 rows but loop visits two | 0 | 0 |
| Constants, 66 rows but loop visits two | 7 | 1 |

This separates the observed difference from storage transport alone. It is
consistent with GPU specialization changing floating-point evaluation; it does
not establish which driver transformation caused it.

[WGSL section 15.7.5](https://www.w3.org/TR/WGSL/#reassociation-and-fusion)
permits reassociation and fusion. Mathematically equivalent shader forms are
therefore not generally promised bit-identical floating-point results.

## Implemented comparison

1. Keep exact CPU assertions for reflected path rows and their packed bytes.
2. Promote the matched 66-row constant-versus-storage comparison into the smoke
   test, requiring identical decoded RGBA pixels at the same resolution.
3. Retain the authored two-versus-66-segment comparison, requiring identical
   dimensions and alpha, at most one 8-bit RGB level of difference per channel,
   and at most 0.01% changed pixels (26 at the existing resolution).
4. Test the comparison helper with deliberate excessive channel differences,
   excessive changed-pixel counts, alpha differences, and dimension mismatches.
   Every such mutation must fail.
5. Keep all host lifecycle, resource, and shader validation assertions unchanged.

These bounds are a regression policy, not a tolerance derived from the
WGSL specification or evidence of portability to all GPUs. Failures on other
devices require investigation; the thresholds must not expand automatically.

The smoke test now enforces both comparisons. Six CPU helper tests reject
out-of-bound changes, and all four Rust path-packing tests remain unchanged and
pass. The full browser smoke passed with seven changed pixels at one RGB level
for the differently optimized forms, and zero differences for matched data.
No renderer or compiler workaround was introduced.
