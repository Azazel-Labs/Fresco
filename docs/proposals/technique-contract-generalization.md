# Generalizing technique allocation, lifetime, and composition

**Status: Exploring ? future potential, not committed.**

Revised 2026-09-23 from the earlier general technique design. This proposal retains
only unresolved extensions. It does not reopen the completed particle migration or
claim that standalone techniques have not shipped. See the
[proposal index](README.md) for status conventions.

## Existing contract and evidence

[LANGUAGE.md](../../LANGUAGE.md#standalone-techniques) owns implemented technique
semantics: independent compute/draw graphs, logical dispatch extents, typed
resources, explicit providers/assets/pools, instance-qualified imports/outputs,
dependencies, and validation. Particles use this machinery with engine-owned
allocation/playback policy. Those mechanisms are the baseline, not proposed work.

Current boundaries can be inspected in
[artifact technique types](../../crates/fresco-artifact/src/technique.rs),
[compiler reflection](../../crates/fresco/src/driver/techniques.rs), and
[technique regression tests](../../crates/fresco-cli/tests/techniques.rs).
Buffer descriptors carry concrete byte counts; image extents are fixed or viewport
based. Dispatch extents can be fixed or named invocation parameters. These are not
a general typed allocation-expression or persistent-resource version model.

[Style operations](../../LANGUAGE.md#styles-and-reusable-operations) already have
checked owned outputs, allocation expressions, and per-invocation identity. Reuse
that work where its semantics fit; it does not automatically extend the standalone
technique contract. Likewise particle persistence is a concrete engine contract,
not a general language-level history mechanism.

## Problem and motivating use

An engine may want a reusable deformation/culling technique that allocates outputs
from an invocation's mesh count, feeds a draw, and exposes those outputs to another
explicit technique instance. Another technique may retain simulation state between
submissions and consume a stable previous-state snapshot while producing next state.

Today the participating paths have different allocation, lifetime, and composition
contracts. Extending one path must not silently invent the missing semantics in
another, duplicate resource validation, or require hosts to interpret expression
strings differently.

## Candidate extensions

### Parameterized allocation descriptors

Allow standalone resource dimensions/capacities to depend on declared invocation
parameters through a bounded, typed expression representation. Define legal input
kinds and operations, overflow/conversion behavior, zero-work behavior, element
strides, image formats/sample counts, and when each check executes.

Compile-known expressions should be checked during compilation; invocation values
and actual device limits must be checked before allocation/submission. Pool choice
must not change descriptor, capacity, initialization, or lifetime semantics.

Open decisions include whether to reuse the style-operation host expression model
as-is, extract a shared subset, or retain distinct front ends with one checked
allocation representation. A dispatch parameter alone must not imply a buffer's
capacity or initialization.

### Persistent resource instances and versions

Distinguish invocation-local scratch, borrowed resources, and persistent state.
Persistent state needs explicit instance ownership, initialization, previous/current
versions, and rules for reset, resize, hot reload, failed submission, and GPU completion.

A previous-invocation read is an explicit version boundary, not permission to ignore
a dependency cycle. In-place read/write is valid only under declared access and
ordering; neighbor reads may require a snapshot. Define how conditional producers
leave versions initialized, and how skipped or failed invocations affect state.

Open decisions include whether persistence belongs entirely to engine providers
or also needs a language-visible resource/version contract. CPU readback and generic
simulation lifecycle policy are not implied by either choice.

### Composition across existing execution paths

Investigate a shared resolved representation for renderer recipes, style operations,
and standalone techniques. Preserve their distinct selection and scheduling scopes:
material/draw range, object, view, frame, and explicit host instance.

Imports must identify the actual producing instance/output and readiness, not just
a shader or style symbol. Sharing a provider must not instantiate duplicate shadow
renders or alias independent outputs. Resource dependencies can express a join;
they do not promise concurrent GPU queues or require a linear chain.

Preserve explicit attachment load/store/blend semantics, supported writer policies,
and both incoming and outgoing renderer completion edges. Generalization must not
remove the explicit placement guarantees used by current style contributions.

A shared executor already exists; the question is how far checked contracts and
reflection should converge, not whether to build another unrelated graph runner.

## Syntax and alternatives

No new grammar is accepted here. The earlier `technique prepared_mesh(...)` sketch
was illustrative and is not supported source syntax. Decide semantics before
choosing between extending `@technique` pipelines, composing existing typed
operations, or introducing a new authoring form.

Alternatives to compare:

- Keep external allocation/persistence entirely in hosts and improve typed provider
  requirements. This keeps the language smaller but leaves more integration work.
- Extend existing standalone techniques with shared checked allocation/version
  descriptors. This reduces duplication but adds lifecycle and compatibility rules.
- Unify resolved graph/resource contracts while retaining different authoring forms.
  This may share validation without forcing every domain into the same syntax.

Fresco owns types, graph contracts, and diagnostics. Engines own particle/shadow/
asset policy; hosts own resource resolution and device execution. Required backend
capabilities should be explicit, with target representability separate from actual
device support and optional portability restrictions.

## Non-goals

- Reimplementing shipped particle migration, providers, assets, or technique imports.
- Requiring every renderer to adopt speculative syntax before current fixes land.
- Adding compiler knowledge of particle lifetimes, pool growth, or shadow names.
- Implicit persistent history, automatic CPU readback, or guaranteed GPU concurrency.
- Treating a registered resource type as proof of every target/device capability.

## Investigation and acceptance checklist

These boxes track unresolved investigation and candidate acceptance requirements,
not an approved implementation schedule.

- [ ] Map standalone, style, recipe, and host allocation representations and identify
  which differences are semantic versus accidental duplication.
- [ ] Specify typed allocation inputs/operations and checks for overflow, conversion,
  strides, zero work, sample counts, and device limits.
- [ ] Decide host-only versus language-visible persistent ownership/versioning;
  specify initialization, conditional updates, reset/resize, and failed submissions.
- [ ] Specify cross-path instance identity, imported readiness, and resource sharing
  without losing current draw/view scope or renderer completion guarantees.
- [ ] Choose the authoring approach and artifact compatibility/migration strategy;
  do not add a silent fallback from an unsupported new contract.
- [ ] Prove parameterized independent compute producers joining a draw, alongside
  compute-only work and multiple differently sized instances.
- [ ] Prove persistent previous/next behavior, initialization, conditional producer
  availability, failure isolation, and reuse only after GPU completion.
- [ ] Reject missing providers/assets, incompatible layouts/usages, cycles, unordered
  writes, invalid sizes, and unsupported target/device requirements explicitly.
- [ ] Retain current particle and style behavior through native/browser consumers;
  run GPU behavior checks locally and update artifacts/contracts together.

Acceptance should use structural and behavioral regressions, not shader snapshots.
If a direction is accepted, link a concrete implementation plan and update
`LANGUAGE.md` as behavior ships. Until then, this file is potential future work.

## Decision history

- 2026-09-23: separated the remaining allocation/lifetime/composition questions from
  the shipped technique contract. Retired the mixed-status design document; its
  earlier draft remains in Git history. No new extension was accepted or implemented
  by this documentation change.
