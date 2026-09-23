---
title: AGENTS
description: Guidance for AI coding agents working in the Fresco Rust workspace.
ms.date: 2026-07-13
ms.topic: overview
---

## Purpose

This repository is a Rust workspace for the Fresco compiler, CLI, macros,
wasm playground, and build automation. Use this file as the first repo-specific
guide when making changes.

## Project Goals

Fresco is a compiler and tooling workspace for a layer-oriented shader
language. Changes should reinforce these goals.

* Keep the language coherent and ergonomic rather than patching over rough edges piecemeal.
* Fix compiler correctness issues at the owning layer when feasible.
* Preserve clear diagnostics, predictable lowering, and stable generated output.
* Improve the language and compiler in ways that help authored `.fr` programs stay expressive.
* Prefer real feature support over ad hoc special cases that only unblock one sample or test.

## Maintainer Preferences

These preferences are explicit and should guide tradeoffs.

* Prefer fixing the underlying compiler bug or missing feature over working around it in examples, tests, docs, or the CLI.
* Do not hide a semantic gap by rewriting inputs or weakening validation unless the task explicitly asks for a temporary compatibility path.
* If a workaround is unavoidable, say so clearly, keep it narrow, and describe the underlying issue that still remains.
* When behavior is wrong, move toward the owning parser, checker, rewrite, or lowering logic instead of patching symptoms farther downstream.
* Add or update regression coverage for the real failure mode so the root cause stays fixed.
* Avoid broad fallback behavior that makes unsupported language constructs appear to work when they are actually being miscompiled.

## Agent Anti-Patterns

These are explicitly forbidden. Do not do any of the following.

* Do not remove, skip, weaken, or `#[ignore]` existing tests to make CI green. Fix the defect instead.
* Do not lower a `panic!` or assertion to a silent fallback to hide a bug. If the invariant is wrong, fix it.
* Do not comment out, delete, or soften a diagnostic to suppress a compiler warning rather than addressing the root cause.
* Do not patch examples, test expectations, or CLI consumers to paper over a compiler defect — fix the compiler stage that owns the problem.
* Do not introduce broad `_ => { /* ignore */ }` match arms or `unwrap_or_default` calls as a workaround for unhandled language constructs.

## Workspace Map

* `crates/fresco`: core compiler pipeline, semantic checking, rewriting, and lowering
* `crates/fresco-cli`: command-line entrypoint and integration tests
* `crates/fresco-macros`: proc macros used by the workspace
* `crates/fresco-wasm`: wasm bindings for the browser playground
* `crates/fresco-wasm/web`: Vite-based web UI, Node-based tests, and preview app
* `xtask`: workspace automation for local and CI checks
* `examples`: `.fr` source examples grouped by difficulty and showcase tier
* `tests/canaries`: shared source fixtures for broad, non-snapshot regression tests
* `docs/generated/language-reference.v1.json`: generated language reference artifact

## Preferred Commands

Use the smallest command that validates the slice you changed.

* `cargo clippy -p fresco --lib -- -D warnings`: narrow Rust validation for the core crate
* `cargo test -p fresco-cli --test <name>`: narrow integration test validation when a specific test target is relevant
* `cargo xtask check`: quick workspace check
* `cargo xtask test`: workspace tests
* `cargo xtask ci`: local fast gate — blocking build/test, warning-only fmt/clippy
* `cargo xtask ci-strict`: full CI gate — blocking build/test/fmt/clippy (what GitHub Actions runs)
* `cargo test -p fresco-cli --test canaries`: semantic, equivalence, diagnostic, and mutation canaries
* `cargo xtask lang-docs`: regenerate the language reference artifact
* `npm ci` and `npm run test:unit`: web playground dependency install and unit tests in `crates/fresco-wasm/web`

## Working Norms

* Inherit edition, Rust version, and lint policy from the workspace manifest.
* Use `foo.rs` beside `foo/` for module roots; `mod.rs` files are forbidden.
* Keep text files LF; preserve binary assets. Run `cargo xtask repo-guard` for source hygiene.
* Keep integer overflow behavior explicit and use checked conversions for fallible numeric boundaries.
* Follow the workspace lint policy in `Cargo.toml`; do not suppress correctness findings.
* Start from the closest failing test, lint error, or owning module instead of broad repo exploration.
* Prefer minimal fixes that preserve existing APIs and output format unless the task requires a behavioral change.
* When a failure suggests a compiler defect, investigate the owning compiler stage before changing examples or consumer code.
* Keep Rust changes consistent with the current style. The workspace is clippy-clean under `-D warnings`.
* Run `cargo fmt --all` if `cargo xtask ci` reports formatting drift.
* Do not hand-edit generated artifacts unless the task is explicitly about generation output.
* When changing compiler output intentionally, validate behavior and check whether generated docs need an update.

## Generated Files And Tests

Treat these as derived outputs unless the task says otherwise.

* Do not create snapshot/golden baselines or update modes. Use behavioral, structural, relational, diagnostic, and mutation assertions; see `docs/testing-canaries.md`.
* `docs/generated/language-reference.v1.json`: regenerate with `cargo xtask lang-docs`
* `target/**`: build output, not source

## Good Starting Points

* Semantic checking issues usually live under `crates/fresco/src/check`.
* Parser and language-surface issues usually start in the compiler crate and then need coverage in `crates/fresco-cli/tests`.
* CLI behavior and regression tests usually live in `crates/fresco-cli`.
* Browser playground changes usually touch both `crates/fresco-wasm` and `crates/fresco-wasm/web`.
* Example-driven regressions should assert the behavior or compiler invariant that failed, without comparing saved compiler output.

## CI Expectations

GPU tests, including the behavior canary, are local-only. Do not install test browsers or run GPU tests in CI. Keep `npm run test:gpu` and the combined `npm run test:canaries` out of CI workflows; the native canaries remain part of workspace tests.

GitHub Actions currently enforces these main checks.

* `cargo xtask ci-strict` on the Rust workspace (fmt, clippy, build, and test all block the job)
* `npm run test:unit` and `npm run wasm:build` in `crates/fresco-wasm/web`
* `cargo-deny check` for license and dependency-source policy

There is also a non-blocking `cargo +nightly udeps --workspace --all-targets` job for unused dependency reporting.

Use `cargo xtask ci` locally for a faster feedback loop (fmt and clippy are warning-only). Use `cargo xtask ci-strict` to match what CI enforces before opening a PR.

## Change Hygiene

Before finishing a task:

1. Re-run the narrowest relevant validation.
2. Escalate to `cargo xtask ci` when the change touches shared compiler behavior or workspace-wide Rust code.
3. Update behavioral assertions and regenerate language docs when their contracts change.
   Keep `LANGUAGE.md` current in the same change when language
   or engine behavior changes; it owns the end-to-end explanation, while focused
   plans own implementation checklists. Distinguish implemented behavior from proposals.
4. Call out any intentionally unvalidated area in the final handoff.
