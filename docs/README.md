# Fresco documentation

Start with the question you need answered. Current guidance, proposals, and
historical evidence have different roles here.

## Start here

| Question | Read |
| --- | --- |
| What is implemented, and how do Canvas, materials, styles, particles, and renderers work? | [Language and implementation reference](../LANGUAGE.md) |
| What should we work on next? | [Active roadmap](../TODO.md) |
| How do authored renderers work, and what remains? | [Renderer walkthrough](../LANGUAGE.md#bundled-renderers) and [engine roadmap](../TODO.md#engine-contracts-and-renderer-execution) |
| How do I consume compiler output today? | [Compilation and runtime](../LANGUAGE.md#compilation-and-runtime) and [host obligations](../LANGUAGE.md#host-integration-obligations) |
| What syntax and builtins does the compiler expose? | [Generated language reference](generated/language-reference.v1.md) and [examples](../examples) |

The generated reference describes registered language capabilities; neither a declaration nor a compiling
example alone proves that an arbitrary engine pass executes correctly.

## Visual explanations

The [visual guide](visual-guide.md) covers engine integration, fields, shape/layer
composition, nested spaces, source-to-shader evaluation, gradient anchors, cellular
ownership, effects, styles, and render dependencies. The SVG diagrams also appear
in the README and language reference.

## Focused guides and references

The [compiler/runtime source-of-truth checklist in TODO.md](../TODO.md#compiler-runtime-source-of-truth) tracks
confirmed mismatches, suspected defects, and consolidation work with acceptance checks.

Engine-specific guides and decisions live in the
[example-engine documentation directory](../integrations/example-engine/docs/README.md).

| Document | Purpose |
| --- | --- |
| [Example engine source package](../integrations/example-engine/README.md) | Inspect the authored engine, embedded source API, Rust canvas renderer, native host, and engine-specific tests; extraction is in progress |
| [Cellular spaces](../LANGUAGE.md#cellular-ownership-and-filtering) | Square, brick, hex, jittered, and Voronoi ownership, cell context, and filtering |
| [Gradients as colors](../LANGUAGE.md#gradient-coordinates-and-typed-texture-decoding) | Gradient inputs for paint, effects, color transforms, and materials |
| [Rewrite system](../TODO.md#rewrite-contracts-and-follow-up) | Authoring rules, safety constraints, and current limitations |
| [Visualizer options](visualizer-options.md) | Editor directives and supported options |
| [Compiler dataflow audit](compiler-dataflow-audit.md) | Lighting separation and remaining engine assumptions for review |
| [Manifest contract](contracts/manifest-pass-plan.contract.v1.json) | Machine-readable pass-plan schema |
| [Maintainer guardrails](maintainer-guardrails.md) | Module ownership and test placement; also see [AGENTS.md](../AGENTS.md) |
| [Testing canaries](testing-canaries.md) | Five broad non-snapshot test examples, execution commands, and guidance for extending coverage |
| [WASM contract generation](../TODO.md#wasm-contract-maintenance) | Updating the Rust/TypeScript boundary |
| [GPU regression checks](../crates/fresco-wasm/web/tests/gpu/README.md) | Running hardware-rendered animation, runtime-input, and orbit checks |
| [Contributor guidance](../AGENTS.md) | Toolchain, module layout, numeric behavior, and lint policy |

## Future proposals

[The proposal index](proposals/README.md) tracks potential future directions and
their status. These are not implementation commitments or supported syntax.

## Plans worth keeping

These specify desired behavior. Consult tests and implementation before treating
their task lists as current completion status.

| Plan | Scope |
| --- | --- |
| [Tiled deferred renderer](../integrations/example-engine/docs/example-engine-deferred.md) | G-buffer formats, engine material tables, GGX lighting, browser selection, and current limits |
| [Standalone example engine](../integrations/example-engine/docs/example-engine-architecture.md) | Shared Rust GPU runtime, bundled engine sources, native executable, browser host, and staged playground extraction |
| [Fresco Next language design](fresco-next-language-design.md) | Proposed standalone redesign with typed library extensions, scoped blocks, explicit sampling, and engine contracts |
| [Designing Fresco with Kotlin](kotlin-fresco-design.md) | Exploration of Kotlin DSL authoring, compiler targets, and extension ideas for standalone Fresco |
| [Surface engine-policy audit](surface-engine-policy-audit.md) | Hardcoded material semantics, ownership corrections, and migration acceptance criteria |
| [Engine follow-up](../TODO.md#engine-contracts-and-renderer-execution) | Executable engine-authored passes, shading, vertex contracts, permutations |
| [Canvas context design](canvas-context-design.md) | Accepted typed Canvas contract, scoped semantic inputs, and required migration away from draw adapters |
| [Lowering scalability tasks](../TODO.md#lowering-scalability) | Measured code-size budgets, scatter expansion, and temporal evaluation |
| [Path follow-up tasks](../TODO.md#path-geometry-and-arc-length) | Runtime geometry, arc-length accuracy, boundary semantics, and explain receipts |
| [Const-template follow-up](../TODO.md#const-templates-and-bounded-unrolling) | Compile-time value parameters and bounded specialization |
| [AA investigation](../TODO.md#anisotropic-aa-and-thin-feature-filtering) | Diagnosis and proposed handling of anisotropic footprints |
| [Editor design principles](bret-victor-principles-report.md) | Interaction goals and their implications for Fresco |

Superseded designs and task histories remain in Git history. Use the current
language reference, active plans, and proposal index for maintained documentation.

## Keeping this manageable

[The language and implementation reference](../LANGUAGE.md) is the
single current explanation of language semantics and engine composition. Update
it in the same change as language/engine behavior. Focused plans own task lists;
dated guides preserve evidence, not competing current specifications.

- Keep the active roadmap short; link to a focused plan for implementation detail.
- Give each document one role: guide, reference, plan, or dated assessment.
- Label proposed behavior and attach evidence to completion claims.
- Record decisions and acceptance criteria once, then link to their owner.
- Remove superseded discussion after preserving useful current details; Git retains history.
- Regenerate `generated/` through workspace tooling; do not edit it by hand.

Engine authoring: [Engine-registered entry contracts](../LANGUAGE.md#interfaces-and-registered-entries) describes custom declaration names and typed method blocks, including registered emitter methods and state composition.

- [Static surface properties](../LANGUAGE.md#surface-properties-and-defaults): engine-defined source options, coverage, sidedness, profiles, and usage variants.
