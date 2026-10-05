# Embeddable manifest runtime

**Status: Exploring — future potential, not committed.**

Origin 2026-10-02. This grew out of R8 ("no host-language code generation")
in [One syntax for engine integration](engine-integration-syntax.md#runtime-direction-exploring).
See the [proposal index](README.md) for status conventions. API and bytecode
sketches below are illustrative. [LANGUAGE.md](../../LANGUAGE.md) owns current
behavior.

## Summary

Fresco's manifest tells an engine what to do: which variant to use, how to
pack material settings, how big a buffer is, what a pass reads and writes,
and what order things run in. Today every engine works that out again in its
own hand-written code. This proposal adds a small Rust library, with a C ABI
and a wasm build, that loads a manifest and does the engine-neutral part of
that work. It calls back into the engine for everything that touches memory,
resources, or the GPU.

The runtime is not a renderer, a frame graph, or a scripting language. It owns
no GPU objects, allocates no engine memory, and makes no scheduling decision
that the manifest does not state.

## Problem

### Every host re-derives the same rules

The example engine's runtime is about 11,700 lines across
`integrations/example-engine/src/runtime/`. Much of it is wgpu work that
belongs to that engine, such as pipeline creation, geometry upload, and
textures. Much of the rest is manifest interpretation that any engine would
need again:

| Module | Engine-neutral work it contains |
| --- | --- |
| `compute_expression.rs` | Checked evaluation of manifest size and dispatch expressions (`ManifestComputeExpression`) |
| `compute_plan.rs`, `pass_plan.rs` | Dispatch extents, allocation sizes, limit checks |
| `recipe.rs` | Step-order verification, read-before-write checks, capacity checks on resize |
| `compute_graph.rs` | Topological order of compute invocations |
| `parameters.rs`, `style_parameters.rs`, `uniforms.rs` | Packing settings and uniforms into byte layouts, and settings-buffer offsets |
| `tables.rs` | Draw-table column bytes |
| `mesh.rs`, `particle_contract.rs` | Provider-key allowlists and binding validation |

A second engine copies these rules or approximates them. Either way the rules
drift from the compiler, and the drift shows up as wrong rendering, not as an
error.

### Engines that own everything still need the neutral part

Lumina owns its renderer outright (see the
[Lumina stress test](engine-integration-syntax.md#lumina-as-a-stress-test)).
It would still need to:

- pack a Fresco material's settings into its own 24/24/24 `FMaterialUniforms`
  table, and request bindless slots for its textures;
- select the per-lane SPIR-V blob for a material's permutation key;
- fill Fresco pass-argument records in its transient memory;
- check that its hand-mirrored C++ structs still match what the shaders
  expect.

### Generated host code is ruled out

The integration proposal's R8 says Fresco never emits C, C++, or Rust for an
engine. Generated glue ties every engine to Fresco's codegen choices, and the
engine has to rebuild whenever a material changes. Running the same logic as
data avoids both.

## Current behavior vs proposed

| | Today | Proposed |
| --- | --- | --- |
| `fresco-artifact` | serde manifest types, plus TypeScript bindings | unchanged; it stays the schema crate |
| Manifest interpretation | hand-written per engine | a shared `fresco-runtime` crate; engines call it |
| Expressions | `ManifestComputeExpression` trees for compute extents and requirements, evaluated by the example engine | one shared expression form for sizes, conditions, table rows, and packing, evaluated by the runtime |
| Layout knowledge | spread across the host's packing code | queryable from the runtime; engine-owned layouts checked by the compiler (`extern struct`) |
| Non-Rust engines | must parse the JSON manifest themselves | link a C ABI |
| Playground | example engine compiled to wasm | the same, with the example engine calling the runtime |

## Non-goals

- Calling a graphics API, or owning GPU objects, memory, or queues.
- Its own frame graph that schedules work. The graph support below is an adapter that emits into the engine's scheduling.
- Compiling shaders at runtime. The compiler is not linked into the runtime.
- A user-facing scripting language. Programs come only from the compiler.
- Generating C or C++ per material or per engine. The runtime's own C header is a fixed library header, which is a different thing (see R8).

## Candidate design

### Crates

- **`fresco-artifact`**, unchanged: the manifest schema, with no runtime dependencies. The TypeScript bindings and tooling keep using it.
- **`fresco-runtime`**, new: loading, verification, and program evaluation. Pure Rust, `no_std`-friendly where practical, and it builds for wasm32.
- **`fresco-runtime-c`**, new: a `cdylib`/`staticlib` with the C ABI and one fixed header. Every entry point catches panics at the boundary and turns them into status codes. A panic is still a bug and gets reported, but it never unwinds into C.

### Program kinds

The compiler writes four kinds of program into the manifest. An engine can use
any subset.

| Program | When it runs | Inputs | Output |
| --- | --- | --- | --- |
| Material setup | material load or settings change | settings values, engine slot callbacks | variant key, plus a *material image*: bytes for the engine's parameter table and a patch list |
| Arguments | per pass, per frame, or per draw | engine-supplied addresses, slots, counts, instance data | a filled argument record, in engine-provided memory |
| Layout query | startup, tooling | record name, target | size, alignment, and every field's offset and type |
| Graph adapter | per frame | viewport, configuration, host-record bindings | callbacks: `add_pass`, `read`, `write`, `barrier_hint`, `resize` |

**Material setup** covers the integration proposal's per-row table
initializers (Q7), settings packing into an engine-declared table, `T?`
presence bits and sentinels, and variant selection. Its result is pre-baked
bytes plus a short patch list, for example "write the bindless slot of
`albedo` at offset 96". At draw time the engine copies bytes and applies a few
patches, so it does no interpretation.

**Arguments** fills Fresco-authored records, such as Lumina's pass arguments
or the example engine's draw records, using the per-target layout. That covers
16-byte spans on Vulkan, sentinels for absent resources, and padding. The
engine writes no struct mirror for these.

**Layout query** lets an engine assert its own mirror at startup:

```c
FrescoLayout scene_root;
fresco_layout_query(rt, "FSceneRoot", FRESCO_TARGET_VULKAN, &scene_root);
assert(scene_root.size == sizeof(FSceneRoot));
```

Engine-owned layouts are still checked by the compiler, through `extern
struct` with `@size`/`@offset`. The query covers the other direction, at the
engine's own startup.

**The graph adapter** walks an L4 description, meaning a `renderer` or
`technique`:

- It evaluates sizes and conditions.
- It uses the compiler's inferred hazards to order steps (integration Q1).
- It reports each step to the engine as callbacks.

An engine with a frame graph turns `add_pass`/`read`/`write` into its own graph
nodes. An engine without one (Lumina) records passes in callback order and
uses `barrier_hint` to choose its barriers. Either way, the engine schedules.

### Expressions first, bytecode only if needed

The manifest already carries expression trees (`ManifestComputeExpression`),
and the example engine evaluates them with checked arithmetic:

- Overflow, division by zero, non-finite floats, and out-of-range conversions are errors.
- None of them wrap silently (`runtime/compute_expression.rs`).

That semantics is the baseline, and it matches the workspace rule to keep
overflow explicit.

1. **Generalize the existing trees.** Turn `ManifestComputeExpression` into
   the one shared expression form. Add the operations that table rows,
   packing, and conditions need: field reads from settings and inputs, enum
   comparison, and `select`.
2. **Flatten to bytecode only if profiling demands it.** Material setup and
   graph walks run per load or per frame, not per draw, so a tree walk may be
   fast enough. If a flat form is needed, use a register bytecode with
   **no jumps**:
   - Conditions become `select`, so every program is straight-line.
   - Every program's length is fixed when it is loaded.
   - Each register's type and each store's offset are checked against the
     declared output size during verification.

`static if` and compile-realized permutations are resolved by the compiler and
never reach the runtime. Specialization-constant and dynamic permutations
reach it only as values to report or to write.

### Host interface (sketch)

```c
typedef struct FrescoRuntime FrescoRuntime;

typedef struct FrescoHost {
    uint32_t abi_version;
    void*    user;
    /* resolve a host-record field (integration Q9) to an engine resource:
       a binding, a heap slot, or a device address, per the target's access model */
    FrescoStatus (*resolve)(void* user, FrescoKey key, FrescoResource* out);
    /* request a bindless slot for a material texture (material setup) */
    FrescoStatus (*heap_slot)(void* user, FrescoTextureRef tex, uint32_t* out_slot);
    /* graph adapter only; may be NULL when unused */
    FrescoStatus (*add_pass)(void* user, const FrescoPassDesc* pass);
    FrescoStatus (*barrier_hint)(void* user, const FrescoHazard* hazard);
    FrescoStatus (*resize)(void* user, FrescoKey resource, uint64_t bytes);
} FrescoHost;

FrescoStatus fresco_runtime_load(const uint8_t* manifest, size_t len,
                                 const FrescoHost* host, FrescoRuntime** out);
FrescoStatus fresco_material_prepare(FrescoRuntime*, FrescoMaterialId,
                                     const FrescoSettings*, FrescoMaterialImage* out);
FrescoStatus fresco_args_fill(FrescoRuntime*, FrescoEntryId,
                              const FrescoArgInputs*, void* dst, size_t dst_len);
FrescoStatus fresco_layout_query(FrescoRuntime*, const char* record,
                                 FrescoTarget, FrescoLayout* out);
FrescoStatus fresco_graph_run(FrescoRuntime*, FrescoGraphId, const FrescoFrameInputs*);
void         fresco_runtime_destroy(FrescoRuntime*);
```

- **Errors.** Every call returns a status, plus a message the engine can fetch. A failed check is an error, never a default. That follows the workspace rule against silent fallbacks.
- **Threads.** A loaded runtime is immutable. `prepare` and `fill` are pure apart from their output buffers and host callbacks, so engines can prepare materials on worker threads.
- **Memory.** The runtime allocates only its own loaded state. All per-call outputs go into engine-provided memory.
- **Versions.** The manifest schema version and the C ABI version are checked separately at load, and a mismatch is a load error.

### Adoption path

1. **In-tree, in Rust.** Move the engine-neutral logic from the example engine
   into `fresco-runtime`, and have the example engine call it directly. Prove
   it with equivalence canaries: for every bundled renderer and material, the
   bytes, sizes, and orders are identical before and after the move. The
   playground gets this for free through its existing wasm build.
2. **C ABI.** Add `fresco-runtime-c` and a small C test harness, with no engine
   and no GPU, that loads bundled manifests and compares results with the Rust
   API.
3. **Lumina probe** in `.experiments/lumina-fresco`:
   - material setup into `FMaterialUniforms`;
   - argument filling for one Fresco compute pass;
   - layout queries checked against Lumina's existing `static_assert` values.
4. **Graph adapter.** Add it once the integration proposal's hazard inference
   (Q1) exists as a compiler service. The example engine's recipe executor
   becomes its first consumer.

## Alternatives considered

- **Status quo: every engine parses the JSON.** It costs nothing up front.
  Every engine re-derives packing, sizing, and ordering rules, and drift is
  silent. That is the problem this proposal addresses.
- **Generated host code.** Rejected by R8. It ties engines to Fresco's
  codegen, forces a rebuild per material, and needs one generator per host
  language.
- **Embedding a general VM** (Wasm, Lua). It is flexible, but the programs are
  unbounded and harder to verify, the dependency is heavy for engines, and the
  sandbox is overkill for straight-line arithmetic. It would also invite
  "scripts" that the compiler didn't write.
- **A runtime that drives the GPU** (wgpu-based). It is convenient for the
  playground, but it violates the premise that every engine owns its
  renderer. Engines with their own RHI (Lumina, Vulkan only) couldn't use it.

## Open questions

- **Q1. Runtime manifest encoding.** JSON is the canonical format for tooling
  and the playground. Should the runtime load JSON, a compact binary form the
  compiler emits next to it, or both? Binary avoids a JSON parser in C/C++
  engines. It also adds a second encoding that has to stay equivalent, which
  needs a round-trip canary.
- **Q2. Resize ownership.** When a size expression changes (a viewport-sized
  buffer, integration Q8), the runtime reports `resize`, and the engine
  reallocates. Should the engine also be able to refuse, or to cap
  allocations? Should a refusal be an error, or a declared fallback in the
  manifest?
- **Q3. Variant selection and blobs.** Does the runtime hand out shader blobs,
  or only the variant key, leaving the engine to look up its own cache?
  Lumina keys blobs by a 64-bit static-switch key plus a lane. The example
  engine compiles WGSL through wgpu.
- **Q4. C++ convenience layer.** A small header-only RAII wrapper over the C
  ABI is a library, not generated code, so R8 allows it. Is it worth
  maintaining, or should the C ABI stay the only supported surface?
- **Q5. Callback granularity for the graph adapter.** Are per-resource
  `read`/`write` callbacks enough for engines with real frame graphs, or do
  they need subresource ranges and access stages from day one?
- **Q6. Hot reload.** Can a new manifest replace an old one while material
  images from the old one are still in use? The likely answer is a new
  runtime instance, with the engine migrating materials explicitly.
- **Q7. Where the evaluator lives during migration.** The example engine's
  `compute_expression.rs` already evaluates manifest expressions. Move it as
  is into `fresco-runtime` (step 1), or redesign the shared expression form
  first?

## Investigation and acceptance checklist

These measure investigation, not delivery.

- [ ] Classify every example-engine runtime module, function by function, as engine-neutral (moves) or engine-specific (stays). Confirm the table above.
- [ ] Specify the shared expression form as a superset of `ManifestComputeExpression`. List the operations that table rows, packing, conditions, and sizing need.
- [ ] Prototype `fresco-runtime` with material setup only, used by the example engine. An equivalence canary must show byte-identical parameter and table uploads for every bundled material.
- [ ] Measure tree-walk cost for material setup and one graph walk at playground scale, before deciding on bytecode.
- [ ] Draft the C header. Check that every type crossing the boundary is plain C (no Rust layouts) and that every entry point returns a status.
- [ ] Fuzz the loader with malformed and truncated manifests. It must return errors, and never panic or read out of bounds.
- [ ] Confirm `fresco-runtime` builds for wasm32 within the playground's size budget.
- [ ] Lumina probe: pack one Fresco material into `FMaterialUniforms`, and check two Lumina structs' layouts through the query API.

## Decision history

- 2026-10-02: Proposal opened from the integration-syntax discussion. The premise is that every engine owns its renderer and Fresco generates no host code, so host-side manifest work runs in a shared runtime.
