# Maintainer guardrails

This is the contributor-facing map for where changes belong and where to add coverage.

## Ownership map

- Parser and syntax surface: `/home/runner/work/fresco/fresco/crates/fresco/src/parser/`
- Semantic checking and typing: `/home/runner/work/fresco/fresco/crates/fresco/src/check/`
- Rewrites: `/home/runner/work/fresco/fresco/crates/fresco/src/rewrite.rs`
- Lowering to Naga IR: `/home/runner/work/fresco/fresco/crates/fresco/src/lower/`
- Driver orchestration and emit/manifest: `/home/runner/work/fresco/fresco/crates/fresco/src/driver/`
- CLI wiring and end-to-end UX: `/home/runner/work/fresco/fresco/crates/fresco-cli/`
- WASM API: `/home/runner/work/fresco/fresco/crates/fresco-wasm/src/`
- Web playground runtime/UI: `/home/runner/work/fresco/fresco/crates/fresco-wasm/web/src/`

## Test placement guardrails

- Prefer core crate unit tests for semantic/lowering invariants that do not require CLI behavior.
- Use CLI integration tests for end-to-end `.fr` compile behavior and diagnostics.
- Keep web unit tests focused on pure helpers and manifest/runtime contract handling.
- Use behavioral and semantic assertions for compiler changes; never regenerate saved expected output to accept a change.

## Validation guardrails

- Core Rust baseline: `cargo xtask ci`
- Strict Rust gate: `cargo xtask ci-strict`
- Web baseline: in `/home/runner/work/fresco/fresco/crates/fresco-wasm/web` run:
  - `npm ci`
  - `npm run test:unit`

## Change hygiene checklist

1. Add/adjust the narrowest relevant tests at the owning layer.
2. Re-run narrow validation first, then broader gate if shared behavior changed.
3. If this changes manifest/runtime contracts, update:
   - `/home/runner/work/fresco/fresco/docs/contracts/manifest-pass-plan.contract.v1.json`
   - Rust + web contract tests.
