# One syntax for engine integration

**Status: Exploring — future potential, not committed.**

Origin 2026-10-02, from a full review of every `.fr` file in
`integrations/example-engine` (engine, examples, test fixtures). Revised the
same day against Lumina, a Vulkan-only, bindless, pointer-based engine (see
[Lumina as a stress test](#lumina-as-a-stress-test)).
See the [proposal index](README.md) for status conventions. All syntax below is
illustrative. [LANGUAGE.md](../../LANGUAGE.md) owns current behavior.

This is a design candidate for the committed
[engine integration syntax workstream](../../TODO.md#engine-integration-follow-up).
It must stay consistent with the
[native device pointer design](../native-pointers-design.md).

## Summary

Fresco has two dialects for describing the engine integration layer:

1. **The attribute dialect.** Renderer recipes, techniques, passes, and
   interfaces are configured by stacking `@name(args)` attributes in front
   of a bare identifier: `@node`, `@draw`, `@bind`, `@after`, `@external`,
   `@meta`, `@configure`, and about 40 more.
2. **The typed dialect.** Newer features use ordinary typed declarations
   and expressions: `draw Shell(geometry: PreparedMesh, …) { … }`,
   `compute ShellVertices(…) -> buffer<…>`, and
   `style … for self { at transparent as target { … } }`.

The typed dialect is better on every axis that matters. It has named
arguments, type checking, real identities, data-flow dependencies, and reuse
through functions. The attribute dialect is where the eyesore, the
duplication, and the per-feature special cases live. Every new engine need
has added another attribute with its own positional argument convention and
its own consumer branch in the compiler.

**Premise: every engine owns its renderer.** The example engine exists to
show how a real-ish engine could integrate with Fresco, and to host the
playground. Fresco must not assume it owns the frame, the barriers, the
resource model, or the host code anywhere. The integration language has to fit
whatever shape an engine already has. Lumina is one of the stranger shapes
seen so far, and stranger ones will come.

This proposal standardizes on the typed dialect and retires the attribute
dialect for anything that has semantics. It is built on eight rules (see
[Design rules](#design-rules)):

1. A declaration body is a list of `key: value` descriptor fields and members. There is no other configuration syntax.
2. Every descriptor is a value of a type declared in `.fr`, so extending the language means adding a field, not a parser branch.
3. Reuse comes from records, spread (`..`), and functions, including graph functions that create steps.
4. Graph identity comes from `let` bindings, and dependencies come from resource data flow.
5. `@` is reserved for declared, typed markers on fields, parameters, and functions. An `@` stack never forms a program.
6. Authored code uses logical resource types. Whether a resource is reached through a binding slot, a bindless heap index, a device address, or a lookup table is decided by the engine's target profile.
7. Integration is layered. An engine adopts only the layers it needs: shared layouts, function libraries, entries in engine-owned slots, standalone passes, or a graph description. A graph description is something the engine executes, never something Fresco owns, and no lower layer depends on it.
8. Fresco never generates host-language code. Layouts, setup, and graph descriptions reach the engine as manifest data, which the engine reads directly or through the optional embeddable runtime (see [Runtime direction](#runtime-direction-exploring)).

Rules 6–8 come from testing the design against Lumina. In practice, the
example engine had been treated as the shape every engine has. It isn't.

---

## Review findings

The findings below come from the current sources. Line references are to
`integrations/example-engine/` unless otherwise noted.

### F1. Recipe steps are attribute soup

A single renderer step in [renderer.fr](../../integrations/example-engine/engine/config/renderer.fr)
looks like this:

```fresco
@when(blend != SurfaceBlend.Translucent) @after(clear_opaque)
@after(sun_shadow) @draw(vertex, raster, mesh) @color(0, output) @depth(forward_depth)
@bind(shadow_camera, sun_camera) @bind(shadow_map, sun_depth) @bind(environment, environment_input) @bind(style_settings, style_values) @bind(scene, frame) @bind(point_lights, lamps) @bind(light_tiles, bins)
@attachment(output, load, store) @attachment(forward_depth, load, store)
preview_mesh
```

- The thing being invoked (`preview_mesh`) comes **last**, after 14 modifiers.
- Nothing marks where one step ends and the next begins, except that a bare identifier terminates the step.
- Ordering, attachments, bindings, and selection are all the same syntactic kind, so neither a reader nor the parser can group them.
- The renderer header has the same problem. Each renderer's body is preceded by 10–17 resource attributes (`@external`, `@image`, `@buffer`, `@table_data`).

### F2. Step identity is ambiguous

Steps without `@node(name)` are referenced by their **pass** name.
`renderer_forward` uses `preview_mesh` three times: as the `sun_shadow`
node, the anonymous opaque step, and the `transparent` node. So
`@after(preview_mesh)`, `all(preview_mesh)`, and `preview_mesh.vertex.uv`
silently mean "the one unnamed `preview_mesh` step". Adding a fourth use
changes what those references mean.

### F3. Massive duplication

| Duplicated thing | Copies | Where |
| --- | --- | --- |
| 7-entry mesh lighting `@bind` list | 7 | every mesh step in `renderer.fr` |
| Renderer resource header (`@external` ×9, `@image`, `@buffer`) | 3 | `renderer.fr` |
| `provide StandardStyle` block | 3 | `renderer.fr` (differs in 6 values) |
| `transparent` vs `after_opaque` point fields inside one `provide` | 5 shared fields | `renderer.fr` |
| `factories: preview_static` | 4 per `provide` | `renderer.fr` |
| Fullscreen-triangle vertex entry | 6 | `30_deferred.fr`, `50_buffer_views.fr`, `60_scene_background.fr` |
| `preview_visibility` shadow filter | 2 | `05_mesh_contract.fr`, `30_deferred.fr` |
| Style dispatch hook stubs (`style_direct/indirect/finish`) | 2 | same two files |
| Octahedral normal encode/decode | 4 | `02_functions.fr`, `05_mesh_contract.fr` ×2, `30_deferred.fr`, `50_buffer_views.fr` |
| 10-positional-argument `surf(...)` construction | 4 | `05_mesh_contract.fr`, `02_camera_billboard.fr` |
| GGX/Smith/Schlick BRDF | 2 | `02_functions.fr` (`brdf`), `styles/pbr.fr` (`pbr_direct_response`, with inlined `3.14159265`) |
| `sign_nz` / `sign_nonzero` | 2 | `02_functions.fr`, `00_prelude.fr` |
| `param @config(editor)` prefix | 7 + 8 | `06_particle_contract.fr`, `07_surface_properties.fr` |
| `channel @compose(mix)` prefix | every channel | `01_core.fr`, fixtures |

There is no mechanism to fix most of these today. Recipes have no records,
spread, or callable sub-graphs, and pass-local helpers can only read
pass-scoped bindings implicitly. So copy-paste is the only reuse tool.

### F4. One name, several meanings

| Spelling | Meanings |
| --- | --- |
| `@dispatch` | (a) renderer step with viewport-scaled extent `@dispatch(cull, 16, 16)`; (b) technique step with fixed or named extent `@dispatch(initialize, capacity)`; (c) **not scheduling at all**: generate a style-hook dispatcher `@dispatch(StandardStyle, direct, style_settings, draw)` |
| `@bind` | (a) step: shader name ← graph resource; (b) interface method: hook alias `@bind(initialize_particle)` |
| `@compose` | (a) material channel composition `@compose(mix)`; (b) entry-method state threading `@compose(p)` |
| `fullscreen` | vertex entry name **and** draw mode in `@draw(fullscreen, clear, fullscreen, 3)` |
| `pipeline(...)` | `postprocess` (LANGUAGE.md concedes the name is wrong), `lighting`, `compute`, or none; meaning depends on the other attributes present |

[LANGUAGE.md](../../LANGUAGE.md#passes-pipelines-and-annotations) currently
documents this as "the enclosing declaration is part of an annotation's
meaning" and "not one universal annotation algebra". That accurately
describes the problem rather than justifying it.

### F5. Several spellings for one concept

| Concept | Spellings in use |
| --- | --- |
| Host-provided resource | `@external(local, source)` (recipes), `@provider(resource, name)` (techniques) |
| Attachment preservation | `@attachment(x, load, store)` (recipes), `attachment<fmt, preserve_update>` + `attachments { color: load_store }` (draws) |
| Storage access | `@access(read_write) buffer<T>`, `buffer<T> readwrite`, `buffer<T, write>` |
| Textures | `texture_2d<f32>`, `texture2d<rg32float, read>`, `texture_depth_2d`, `texture<Decoder>` |
| Bounded parameter | `@range(0, 1)` (interfaces), `= 0.5 in 0.0 .. 1.0` (surfaces, styles) |
| Constants | `const f32 PI = …` (`02_functions.fr`), `const name: bool = …` (`renderer.fr`) |
| Field separators | newline, `;` (`struct FogData { color: vec3; density: f32 }`), `,` (`struct PreviewLighting { settings: vec4, sky: vec4 }`) |
| Unsigned literal | `0u`, `u32(0)` |
| Inverse sqrt | `inverse_sqrt`, `inverseSqrt` (`08_lighting.fr`), `rsqrt` |
| Specialization | `param @config(engine) @permutation` + global `const` + `@configure` on renderer; `permutations { @known(compile) x: "a"\|"b" }` + `select`; `@known(pipeline)`; `@known(draw)` |
| Engine settings | `#pragma check.x = v`, `budget { … }`, `@meta(k, v)` |
| `provide` body | `Name { k: v; k: v }`, `name { k: v }`, and `frame = frame`, all in one block |

### F6. Placeholder bodies for compiler-generated functions

```fresco
@dispatch(StandardStyle, direct, style_settings, draw)
fn style_direct(response: u32, settings_offset: u32, surface: StandardSurface, context: ShadingContext, light: DirectLight) -> vec3 { return vec3(0.0) }

@evaluate(surface) fn evaluate_surface(context: surf) -> base {}
```

The bodies are fake. The signatures repeat information the contract already
owns, and they are pasted into each pass that needs them.

### F7. Stringly-typed engine contracts

The particle technique carries its allocation contract as nine `@meta`
entries:

```fresco
@meta(capacity, allocation == ParticleAllocationMode.Fixed ? max(1, u32(ceil(spawn_rate * max_lifespan)) + burst_count) : allocation_hint)
@meta(overflow, "drop_new")
```

Keys are unchecked strings, `"drop_new"` is an untyped string, and the host
interprets the bag by convention.

### F8. The compiler mirrors the surface

In `crates/fresco/src`, `PipelineAttribute` stores arguments as
`Vec<String>`. There are 162 attribute lookups by name across 21 files, and
40 sites construct `PipelineAttribute::synthetic(...)`. For example, style
compute operations fabricate a `@shader` attribute to reach the GPU program
emitter (`driver/style_operations/compute.rs`). The attribute bag is
effectively the IR. Each new attribute therefore costs a parser case, a
placement check (`validate_pipeline_attr_target`), positional string
re-parsing, and an ad hoc consumer. That is the mechanism behind
"constantly unfixing old designs".

### F9. Bugs, dead code, and inverted dependencies found in review

- `@external(depth, depth)` is declared in `renderer_forward_plus` and `renderer_deferred` and never bound.
- `renderer_forward` allocates `@buffer(bins, 8)` only because `preview_mesh`'s factory unconditionally declares `light_tiles`. The permutation that reads it (`forward_plus`) is off. A `static if forward_plus` binding would remove the dummy resource.
- Redundant transitive edges: `@after(opaque) @after(clear_geometry) @after(clear_lighting)`.
- Deferred lighting is `@draw(fullscreen, lighting, instance, 3)`, a fullscreen draw repeated per shading instance with a fragment-side identity discard. The mode name `instance` doesn't say that.
- `core/07_surface_properties.fr` imports `../config/renderer.fr` only to read `preview_deferred` / `preview_forward_plus`. Core depends on config because renderer selection is threaded through global constants.
- `clear_opaque` and `clear_gbuffer` exist only to establish attachment contents when no draws select. That is a load-op concern expressed as two extra passes and steps.
- `core/03_payload.fr` is a comment-only file.
- `pipelines/10_forward.fr` and `20_forward_plus.fr` are self-described non-executable demonstrations. They carry a third dialect (`permutations`, `require`, `@known`, `@expect`, `select`, `dispatch host { $viewport }`, `reads:`) used nowhere else.

---

## Design rules

These are the standard. Any new engine-facing feature should be expressible
under them without a parser change. If it isn't, that is a signal to extend
a rule deliberately, not to add an attribute.

### R1. One declaration shape

```
<kind> Name[<generics>][(params)] [-> Result] [for Schema] [: Contracts] {
    descriptor: value        // fields, typed by the kind's descriptor record
    member                   // let, fn, step, nested declaration
}
```

`pass`, `draw`, `compute`, `renderer`, `technique`, `style`, `interface`,
`contract`, `vertex_factory`, and `material_properties` all use this shape.
Passes already mostly do (`stage: raster`). The rule extends it to everything
and removes the declaration-level `@` prefix (`@shader`, `@factory`,
`@service`, `@technique`, `@renderer`, `@default`, `@entry`,
`@surface_defaults`, `@surface_properties`, `@context`, `@composition`).

### R2. Descriptors are typed records declared in `.fr`

Each kind's descriptor fields are a record type declared in a compiler-owned
prelude module (for example `fresco/graph.fr`). Engines can extend those
records with their own:

```fresco
// fresco/graph.fr (compiler prelude, readable source)
struct DrawStep {
    vertex:    entry<vertex>
    fragment:  entry<fragment>?
    geometry:  Geometry
    color:     Attachments = {}
    depth:     Attachment? = none
    bind:      Bindings    = {}
    after:     [Step]      = []
    queue:     Queue?      = none
}
enum Geometry { meshes(where: predicate<SurfaceOptions>?), fullscreen(per: DrawScope?), procedural(vertices: u32, instances: u32) }
```

The consequences:

- **Adding a capability means adding a field with a default.** Old sources keep compiling. Nothing is parsed positionally.
- An unknown key, a wrong type, or a misplaced field is an ordinary type error with "did you mean". It is not a per-attribute placement table.
- The generated language reference can be produced from the prelude types, so the vocabulary is documented by construction.
- The compiler lowers descriptor values into typed HIR, so `Vec<String>` attribute arguments disappear (F8).

### R3. Reuse means records, spread, and functions

- **Record literals.** `Type { k: v, … }` or `{ k: v, … }` when the expected type is known. Fields are separated by `,` or a newline. `;` is retired as a field separator and remains a statement separator.
- **Field shorthand.** `{ tiles }` means `{ tiles: tiles }`.
- **Spread.** `{ ..base, k: v }` copies the fields of `base`, and later fields override. Spread is **same-type only**, with no exceptions. Resource binding composes as a list of records (`bind: [lit, g.textures, { … }]`), not by name-keyed spread (Q3).
- **Graph functions.** `graph fn name(…) -> T { … }` may create steps and resources. Calling one inlines its steps into the caller's graph. It is the recipe counterpart of `style … for self`. One concept should serve both (see open questions).

### R4. Identity from `let`, ordering from data flow

- A step is an expression (`draw <pass> { … }` or `dispatch <pass>.<entry> { … }`) whose value is a step handle. `let name = …` gives it identity. Unnamed steps cannot be referenced, which fixes F2.
- Dependencies are **inferred from resource hazards in program order**: a step that reads or writes a resource follows earlier writers of it. `after:` is only for ordering that no resource expresses. This is ordinary render-graph semantics. It replaces nearly every `@after` in the engine, but it changes meaning and has to be decided explicitly (see Q1).
- Attachments bind **by output-field name**, not slot index. `color: g` binds a record of images to `PreviewGBuffer`'s fields. `color: { paint, tag: stamps }` binds `Pixels`.
- Load operations are methods: `output.load` (today's `@attachment(output, load, store)`), `output.clear(0.0)`, and bare `output` (today's default). A `.clear` must take effect even when the step selects zero draws, which removes `clear_opaque` and `clear_gbuffer`.

### R5. `@` is for declared, typed leaf markers only

An annotation:

- is **declared** (`annotation semantic(role: SemanticRole) on field`), by the compiler prelude or by the engine;
- applies to a **field, parameter, function, or type** (`@semantic`, `@location`, `@builtin`, `@interpolate`, `@group`/`@binding`, stage markers, and layout assertions such as `@size`/`@offset` on engine-owned structs);
- has typed arguments, and one name has one meaning.

If an annotation would need to be ordered against its neighbors or would
form a step, it belongs in a descriptor. Engine-declared annotations replace
`@meta` as the reflected-metadata escape hatch, and they are checked.

### R6. Logical resources, target-selected access

Authored code uses `buffer<T>`, `span<T>`, textures, samplers, and `T?`.
The `engine` record chooses, per target, whether each kind of resource is
reached through a binding, a descriptor-table index, or a device address. The compiler owns the consequences: non-uniform decoration,
sentinels, span layout, and how pointer-bearing records are loaded.
`Ptr<T>` is available only where the profile provides device addresses.
See [R6 in practice](#r6-in-practice-logical-resources-target-selected-access).

### R7. Integration is layered

Shared layout, function libraries, slot entries, standalone passes, and graph
descriptions are separate layers. Every engine owns its renderer, so each
layer describes work the engine runs. An engine adopts the layers it needs. See
[R7 in practice](#r7-in-practice-integration-layers).

### R8. No host-language code generation

Fresco emits shader binaries and the manifest. It does not emit C++, C, or
Rust for the host. Layouts are reflected as data and checked against
engine-declared `extern struct` layouts. Host-side work that would otherwise
need generated code runs as manifest programs in the embeddable runtime.

---

## Core constructs

### Host sources

The engine names its host protocol once. A field name **is** the provider
key, so there are no `local, source` pairs. Renaming is ordinary `let`.

```fresco
host PreviewHost {
    scene:                uniform<PreviewScene>
    point_lights:         buffer<vec4>
    lighting_environment: uniform<PreviewLighting>
    style_parameters:     buffer<vec4>
    shadow_camera:        uniform<PreviewShadow>
    resolve_camera:       uniform<ResolveInputs>
    scene_background:     uniform<SceneBackground>
    buffer_view:          uniform<BufferViewInputs>
    presentation:         target
}
```

This replaces `@external` (recipes) and `@provider` (techniques). Renderers
and techniques take the host record as a parameter, so a test can supply a
different one.

### Binding records

A pass declares its resource interface as one or more records. The same
record type is what a step passes to `bind:`.

```fresco
struct MeshLighting {
    scene:          uniform<PreviewScene>
    environment:    uniform<PreviewLighting>
    shadow_camera:  uniform<PreviewShadow>
    shadow_map:     depth2d
    point_lights:   buffer<vec4>
    style_settings: buffer<vec4>

    static if forward_plus {
        light_tiles: buffer<u32>
    }
}

vertex_factory preview_static for PreviewMesh {
    binding draw: [MeshLighting, DrawIdentity]
    fn transform(v: PreviewMesh) -> mat4 from object to world { return scene.model }
}

struct DrawIdentity {
    draw_record:         uniform<DrawRecord> from table(DrawRecord)
    shading_instance_id: uniform<u32> from draw_data(instance_id)
}
```

A pass's binding interface is a list of records, and a step's `bind:` is a
list of values of those records (see [Q3](#decisions)). `lighting_resolve`
declares `MeshLighting` too. That removes today's renames: the same host
buffers are currently bound as `point_lights`/`light_tiles` in mesh steps and
as `lights`/`tiles` in `tile_culling` and `lighting_resolve`. With this
change, `preview_visibility` can live once as
`fn preview_visibility(l: MeshLighting, position: vec3, n: vec3) -> f32`.

### Generated only when: `static if`

Today the forward renderer allocates an 8-byte `bins` buffer that nothing
writes, only because `preview_static` declares `light_tiles` unconditionally.
The read in the shader is already guarded by `if forward_plus`, but that `if`
is constant-folded silently. Nothing at the declaration says the binding only
matters in some variants, nothing ties the read to the declaration, and in a
variant where the branch is folded away it is not type-checked at all.

Anything that exists only in some variants is wrapped in a `static if` block.
That is the existing style-graph keyword, with the same meaning: every branch
is checked, and only the selected branch is generated.

```fresco
// declaration: the field exists only in forward_plus variants
struct MeshLighting {
    …
    static if forward_plus {
        light_tiles: buffer<u32>
    }
}

// construction mirrors the declaration
fn mesh_lighting(host: PreviewHost, shadow: depth2d,
                 static if forward_plus { tiles: buffer<u32> }) -> MeshLighting {
    return MeshLighting {
        scene: host.scene, environment: host.lighting_environment,
        shadow_camera: host.shadow_camera, shadow_map: shadow,
        point_lights: host.point_lights, style_settings: host.style_parameters,
        static if forward_plus { light_tiles: tiles }
    }
}

// use: the read lives in the same kind of block
for i in 0 .. 64 {
    var visible = true
    static if forward_plus {
        let mask = light_tiles[tile * 2u + i / 32u]
        visible = (mask & (1u << (i % 32u))) != 0u
    }
    …
}
```

The rules:

- **One spelling, everywhere.** `static if P { … }` may wrap fields, binding records, parameters, record-literal fields, statements, steps, and `provide` entries. No item has its own guard suffix.
- **The condition must be visible where the item is used.** An item introduced under `static if P` can only be named inside `static if P` (or a stronger condition), or in a context where the configuration fixes `P` to true, such as a renderer that sets `configure: { forward_plus: true }`. Otherwise it's a diagnostic naming the guard.
- **Supplying it in the wrong variant is an error.** `renderer forward` configures `forward_plus: false`, so passing `tiles:` to `mesh_lighting`, or binding `light_tiles`, is a diagnostic. Nothing is silently ignored, so the dummy `bins` simply disappears.
- **`P` is a permutation realized as a compiled variant** (see [Specialization realization](#specialization-realization)). Something realized as a specialization constant or runtime value still exists in the binary, so it branches with a plain `if`. `static if` on such a permutation is a diagnostic. When you read the code, `static if` means "this is generated or not", and `if` means "this branches in the shader".
- **`static if` vs `T?`.** `static if` is compile-time absence: the field does not exist in that variant. `T?` is run-time absence: the field exists, and its value may be empty (Lumina's `0xFFFFFFFF` slots). Use `T?` when one binary must handle both cases.
- **Migration.** `fresco fmt --migrate` rewrites a plain `if` whose condition folds to a compiled permutation into `static if`. A folded `if` that guards a variant-only binding becomes a hard error after the migration edition.

This replaces the staged dialect's `when`/`else` axis guards, `require { … => disable(…) }`, and `@expect` (Q4), and it fixes F9's dummy `bins`.

### Intrinsic functions

Compiler-supplied bodies are spelled as intrinsics. They never use
placeholder bodies, and their signatures come from the contract.

```fresco
fn style_direct   = dispatch StandardStyle.direct   { settings: style_settings, scope: draw }
fn style_indirect = dispatch StandardStyle.indirect { settings: style_settings, scope: draw }
fn style_finish   = dispatch StandardStyle.finish   { settings: style_settings, scope: draw }
fn evaluate_surface = evaluate surface(context: surf) -> base
fn evaluate_vertex  = evaluate vertex(context: surf) -> surf
```

With this, `@dispatch` keeps a single meaning: scheduling compute work, as a
step verb, not an annotation.

### Settings sections

Repeated parameter prefixes become sections:

```fresco
interface ParticleEmitter {
    entry:            emitter(update)
    surface_defaults: { blend: SurfaceBlend.Translucent }

    settings editor {
        allocation:      ParticleAllocationMode = Fixed
        spawn_rate:      f32 = 60.0
        burst_count:     u32 = 8
        max_lifespan:    f32 = 2.0
        allocation_hint: u32 = 32
        max_particles:   u32 = 4096
        permutation simulate_motion: bool = true
    }

    fn spawn(p: Particle, id: f32, count: f32) -> Particle  = hook initialize_particle { threads: p }
    fn update(p: Particle, dt: f32) -> Particle             = hook advance_particle    { threads: p }
}
```

`settings engine { … }` holds engine-config parameters. Bounds use the
existing `= v in lo .. hi` form everywhere, and `@range` is retired. Enum
values are written as bare variants (`Fixed`, `Translucent`) wherever the
expected type is known. That extends the existing contextual-enum rule
rather than adding a new `.Fixed` form (Q6).

### Specialization

There is one mechanism. A `permutation` is declared on a settings field with
a typed (enum or bool) domain. A renderer sets values in `configure:`, which
names the permutation directly, so no global `const` hop is needed:

```fresco
interface SurfaceOptions {
    surface_properties: properties
    settings engine {
        permutation deferred:     bool = false
        permutation forward_plus: bool = false
    }
    settings editor { … }
}
```

String-union permutations (`"flat"|"sh2"`), `@known(...)`, `@expect(...)`,
and `require { … => disable(...) }` either port onto typed enum permutations
plus `static if`, or go away with the demonstration files. The recommendation is to delete them (Q4).

---

## Before and after: the renderers

### Shared graph functions

`mesh_lighting` is shown under [`static if`](#generated-only-when-static-if).

```fresco
graph fn sun_shadow(host: PreviewHost) -> image<depth32float> {
    let depth = image<depth32float>()
    draw preview_mesh {
        vertex: shadow_vertex, fragment: shadow_fragment
        geometry: meshes
        depth
        bind: { scene: host.scene, shadow_camera: host.shadow_camera }
    }
    return depth
}

graph fn cull_lights(host: PreviewHost) -> buffer<u32> {
    let grid  = viewport.tiles(16)
    let tiles = buffer<u32>(count: grid.count * 2)
    dispatch tile_culling.cull {
        extent: grid
        bind: { scene: host.scene, point_lights: host.point_lights, light_tiles: tiles }
    }
    return tiles
}

// Opaque then transparent forward shading into one color/depth pair.
graph fn forward_mesh(lit: MeshLighting, color: image<rgba16float>, depth: image<depth32float>) -> MeshSteps {
    let shade = { vertex: vertex, fragment: raster, bind: lit }
    let opaque = draw preview_mesh {
        ..shade
        geometry: meshes(where: blend != SurfaceBlend.Translucent)
        color: color.clear(0.0), depth: depth.clear(1.0)
    }
    let transparent = draw preview_mesh {
        ..shade
        geometry: meshes(where: blend == SurfaceBlend.Translucent)
        queue: transparency
        color: color.load, depth: depth.load
    }
    return MeshSteps { opaque, transparent }
}

graph fn present(host: PreviewHost, shaded: image<rgba16float>) -> Step {
    return draw scene_background {
        vertex: fullscreen_triangle, fragment: present
        geometry: fullscreen
        color: host.presentation
        bind: { shaded, settings: host.scene_background, resolve: host.resolve_camera, view_mode: host.buffer_view }
    }
}

// The single mapping from renderer steps onto StandardStyle's contract points.
fn standard_points(host: PreviewHost, complete_opaque: Step, transparent: Step, inspection: Step,
                   presentation: Step, color: image<rgba16float>, depth: image<depth32float>,
                   opaque: Step) -> provide StandardStyle {
    let boundary = { complete_opaque: [complete_opaque], inspection: [inspection],
                     presentation: [presentation], color, depth }
    return {
        factories: [preview_static]
        frame: host.scene
        ForwardLighting:      { lights: transparent.service(LightingService) }
        SurfaceUV:            { surface.uv: opaque.output(vertex.uv) }
        DrawShadingResources: { shading_instance_id: opaque.draw_data(shading_instance_id) }
        PreparedGeometry:     { mesh.prepared: opaque.prepare }
        after_opaque: { ..boundary, transparency: [transparent], order: stable_draw_order }
        transparent:  { ..boundary, queue: transparency }
    }
}
```

### Forward: 30 lines of attributes and a 24-line `provide` become this

```fresco
renderer forward(host: PreviewHost) {
    id: "forward", label: "Forward", default: true
    configure: { forward_plus: false, deferred: false }

    let color = image<rgba16float>()
    let depth = image<depth32float>()
    let mesh  = forward_mesh(mesh_lighting(host, sun_shadow(host)), color, depth)
    let shown = present(host, color)

    provide StandardStyle = standard_points(host, mesh.opaque, mesh.transparent,
        inspection: shown, presentation: shown, color, depth, opaque: mesh.opaque)
}
```

There are no `@after` edges. Shadow-before-opaque follows from `shadow_map`,
opaque-before-transparent from `color`/`depth`, and transparent-before-present
from `shaded`. There is no light-tile buffer at all. `forward_plus` is false,
so `MeshLighting` has no `light_tiles` field and `mesh_lighting` takes no
`tiles:` argument.

### Forward+

```fresco
renderer forward_plus(host: PreviewHost) {
    id: "forward-plus", label: "Forward+"
    configure: { forward_plus: true, deferred: false }

    let color  = image<rgba16float>()
    let depth  = image<depth32float>()
    let output = image<rgba16float>()
    let sun    = sun_shadow(host)
    let tiles  = cull_lights(host)
    let mesh   = forward_mesh(mesh_lighting(host, sun, tiles: tiles), color, depth)
    let view   = draw preview_buffer_view {
        vertex: fullscreen_triangle, fragment: visualize, geometry: fullscreen
        color: output
        bind: { view_mode: host.buffer_view, shaded: color, shadow_map: sun, light_tiles: tiles, depth_texture: depth }
    }
    let shown = present(host, output)

    provide StandardStyle = standard_points(host, mesh.opaque, mesh.transparent,
        inspection: view, presentation: shown, color, depth, opaque: mesh.opaque)
}
```

### Deferred

```fresco
struct GBufferImages {
    albedo:             image<rgba8unorm_srgb>
    normal_material:    image<rgba16float>
    emissive_occlusion: image<rgba16float>
    identity:           image<rg32uint>
    surface_uv:         image<rg32float>
}

renderer deferred(host: PreviewHost) {
    id: "deferred", label: "Deferred"
    configure: { forward_plus: true, deferred: true }

    let g      = GBufferImages::new()            // field names match PreviewGBuffer outputs
    let depth  = image<depth32float>()
    let color  = image<rgba16float>()
    let output = image<rgba16float>()
    let sun    = sun_shadow(host)
    let tiles  = cull_lights(host)
    let lit    = mesh_lighting(host, sun, tiles: tiles)

    let opaque = draw preview_mesh {
        vertex: vertex, fragment: geometry
        geometry: meshes(where: blend != SurfaceBlend.Translucent)
        color: g.clear(0), depth: depth.clear(1.0)
        bind: lit
    }
    let resolve = draw lighting_resolve {
        vertex: fullscreen_triangle, fragment: lighting
        geometry: fullscreen(per: shading_instance)
        color: color.clear(0.0)
        bind: [lit, g.textures, {
            depth_texture: depth, resolve: host.resolve_camera
            materials:     table(DrawRecord).shading_model
            styles:        table(DrawRecord).style_id
            style_offsets: table(DrawRecord).style_settings_offset
        }]
    }
    let transparent = draw preview_mesh {
        vertex: vertex, fragment: raster
        geometry: meshes(where: blend == SurfaceBlend.Translucent)
        queue: transparency
        color: color.load, depth: depth.load
        bind: lit
    }
    let view = draw deferred_buffer_view {
        vertex: fullscreen_triangle, fragment: visualize, geometry: fullscreen
        color: output
        bind: [g.textures, { view_mode: host.buffer_view, shaded: color, shadow_map: sun,
                             light_tiles: tiles, depth_texture: depth }]
    }
    let shown = present(host, output)

    provide StandardStyle = {
        ..standard_points(host, resolve, transparent, inspection: view, presentation: shown,
                          color, depth, opaque: opaque)
        SurfaceUV:            { surface.uv: opaque.output(geometry.surface_uv) }
        DrawShadingResources: { shading_instance_id: resolve.draw_data(shading_instance_id) }
    }
}
```

Compared with today:

- All three renderers share the shadow, cull, mesh, presentation, and contract-point code.
- The 7-entry binding list exists once, as `mesh_lighting`.
- The two clear passes are gone, and MRT slots are bound by name.
- Every step has a name or is unreferenced.

## Before and after: passes and techniques

### Shared fullscreen entry

There is one top-level vertex entry. Any step may select it.

```fresco
@vertex fn fullscreen_triangle(@builtin(vertex_index) index: u32) -> ScreenPosition {
    let p = vec2(f32((index << 1u) & 2u), f32(index & 2u))
    return ScreenPosition(vec4(p * 2.0 - 1.0, 0.0, 1.0))
}

pass scene_background {
    binding {
        shaded:    texture2d<f32>
        settings:  uniform<SceneBackground>
        resolve:   uniform<ResolveInputs>
        view_mode: uniform<BufferViewInputs>
    }
    @fragment fn present(@builtin(position) pixel: vec4) -> vec4 { … }
}
```

`@shader pass` becomes plain `pass`. Whether a pass is emitted as a GPU
program follows from being referenced by a step (Q5).

### Mesh pass header

```fresco
// before
@service(LightingService)
@factory(preview_static) pass preview_mesh for base {
    stage: raster
    draw: per_object
    …

// after
pass preview_mesh for base {
    factory:  preview_static
    provides: [LightingService]
    stage:    raster
    draw:     per_object
    cull:     choose_state(back, none, two_sided)
    depth:    { write: blend != SurfaceBlend.Translucent, compare: less }
    blend:    { all: choose_state(replace, premultiplied, blend == SurfaceBlend.Translucent) }
    …
```

`depth { … }` and `blend { … }` become ordinary record-valued fields. That
is the same syntax as everywhere else and it can be spread from a shared
state.

### Particle technique

```fresco
struct ParticleAllocation {                        // engine-declared, host-reflected
    mode:               ParticleAllocationMode
    capacity:           u32
    max_capacity:       u32
    growth_factor:      f32 = 2.0
    spawn_rate:         f32
    spawn_burst:        u32
    max_lifespan:       f32
    max_spawn_per_step: u32 = 4096
    overflow:           OverflowPolicy = drop_new
}

technique particle_system(host: ParticleHost) for ParticleEmitter {
    allocation: ParticleAllocation {
        mode: allocation
        capacity: allocation == Fixed
            ? max(1, u32(ceil(spawn_rate * max_lifespan)) + burst_count)
            : allocation_hint
        max_capacity: max_particles
        spawn_rate, spawn_burst: burst_count, max_lifespan
    }

    let output = image<float_color>() from host.presentation
    let depth  = image<depth32float>() from host.depth
    let sim    = { particles: host.state, particle_config: host.config, particle_slots: host.slots }

    let spawn  = dispatch particle_simulation.initialize { extent: capacity, enabled: spawn, bind: sim }
    let update = dispatch particle_simulation.simulate   { extent: capacity, enabled: update, bind: sim }
    draw camera_billboard {
        vertex: vertex, fragment: fragment
        geometry: procedural(vertices: 6, instances: capacity)
        color: output, depth
        bind: { scene: host.scene, particle_render: host.state, particle_slots: host.slots }
    }
}
```

Nine `@meta` strings become one typed record that the host reads as
reflected data. `spawn` → `update` ordering is not inferred, because both
write `particles` and program order already places `update` second.
`@enabled`, `@instances`, and `@provider` become descriptor fields.

### Material properties

```fresco
material_properties standard extends base {
    compose: mix                                     // default for every channel
    roughness:  f32 = 1.0
    metallic:   f32 = 0.0
    normal:     vec3 in world = sp.normal
    normal_map: vec3 in tangent = vec3(0.0, 0.0, 1.0)
    occlusion:  f32 = 1.0
}
```

`@context(surf, sp)` and `@composition(base, material, weight)` on `base`
become the `context:` and `composition:` descriptor fields. `channel` is
implied inside `material_properties`.

### Engine settings

```fresco
engine {
    check:  { shape_aa_min_px: 1.5, shape_aa_max_px: 3.0, shape_aa_style: gradient,
              projective_footprint_max_px: 64.0 }
    budget: { warn_above: 256, fail_above: 2048 }
}
```

This replaces `#pragma check.*` and the bare `budget { }` with one typed
engine record. The same record also carries the target and resource profile
described in the next section. There is one `engine` record, not a second
configuration surface.

---

## Lumina as a stress test

Every engine owns its renderer (see the premise in the [Summary](#summary)).
The example engine happens to describe its frame in Fresco, because the
playground needed a renderer. Lumina (`.experiments/LuminaEngine`) shares
almost none of the example engine's habits, which makes it a good test of
whether the rules are engine-neutral. The goal is not to design the Lumina
integration now. The goal is to confirm that the syntax above leaves room for
it.

### What Lumina looks like

| Area | Lumina today | Example-engine habit it breaks |
| --- | --- | --- |
| Frame structure | Hand-ordered passes in `DefaultSceneRenderer::RenderView`. No render graph. | The frame is described in `.fr` and executed by a graph runtime. |
| Synchronization | Manual global `CmdBarrier(stage, access, stage, access)`. Every image stays in `GENERAL` layout. Dynamic rendering. No transient aliasing. | The runtime derives barriers from the recipe. |
| Buffers | Never descriptors. Always 64-bit device addresses (`GPUPtr`), plus `GPUSpan<T>{ T* Data; uint Count; uint _Pad }`. | Buffers are bind-group bindings. |
| Textures and samplers | One bindless set. Binding 0 holds samplers. Binding 1 holds sampled images, aliased as 2D/Cube/3D/2DArray/CubeArray/`uint`. Binding 2 holds storage images, included only by writers. Shaders index the heap with `uint` slots under `NonUniformResourceIndex`. | Each resource has its own binding slot. |
| Root | A 40-byte push constant, `FRHIRoot{ Args, SceneRoot, SceneData, Instances, Lights }`. Per-pass args are a transient-copied struct reached through `GetPassArgs<T>()`. | Bind groups per pass. |
| CPU/GPU layout | C++ and Slang structs mirrored by hand, with byte-offset comments in shaders. Only `static_assert(sizeof)` checks them (`FSceneRoot` 352, `FMaterialUniforms` 592, and others). `0xFFFFFFFF` marks absent resources. | Fresco reflects the host layout. |
| Materials | A material graph is substituted into Slang templates per *lane*: forward pixel, deferred compute, VisBuffer mesh, masked shadow, moment generation, decal, and others. One SPIR-V blob is produced per `EMaterialShaderStage`. | A material is one surface evaluation inside a Fresco pass. |
| Derivatives | The deferred lane is compute, so `#define SHADING_COMPUTE` switches texture sampling to analytic gradients. | Fragment derivatives are always available. |
| Variants | `#define` variants, specialization constants 1–8 (`SPEC_SKINNED` is tri-state: off, on, or decided at runtime), static switches packed into a 64-bit key, and passes compiled twice (`MOMENT_GENERATION`). | One `permutation` mechanism realized as compiled variants. |
| Material parameters | `FMaterialUniforms{ Vectors[24], Scalars[24], Textures[24], … }`, indexed by `MaterialIndex` from the instance. | Typed per-material settings with an engine table schema. |

The realistic Lumina integration consists of:

- Fresco materials compiled into Lumina's lanes.
- Fresco compute kernels that Lumina records itself.
- Struct layouts that Lumina owns and Fresco checks.
- Possibly the runtime's material-setup and argument programs.

That integration already matches the finding in
`.experiments/lumina-fresco/README.md`: a callable material library is the
first integration point.

### R7 in practice: integration layers

An engine adopts the layers it needs. Each layer only depends on the layers
below it.

| Layer | Fresco provides | Engine owns | Example engine | Lumina |
| --- | --- | --- | --- | --- |
| L0 Shared layout | Typed records lowered to shader layout, checked against `extern struct` declarations and reflected in the manifest | Host structs, allocation, upload | yes | yes, checks the hand mirrors |
| L1 Function library | Callable functions (BRDFs, codecs, noise) | Calling them | yes | yes |
| L2 Slot entries | Materials and hooks that fill engine-declared `interface` slots, per lane | Templates, pipelines, and draw submission | yes | yes, the main target |
| L3 Standalone passes | Compute and draw entries with reflected resource access | Ordering, barriers, dispatch | yes | yes, culling and post-processing |
| L4 Graph description | `renderer`, `technique`, steps, and inferred dependencies | Execution: its own graph, its own sequence, or the runtime's adapter | yes | no |

The renderer and technique syntax earlier in this document is L4. L0–L3 use
only records, `interface`, functions, and the `engine` record, so an
integration that doesn't use a graph description never touches
`draw`/`dispatch` steps. This is a
constraint on the L4 design. Anything a renderer step does that an L3 pass
needs must be a property of the pass, not the step. Examples are resource
access, workgroup size, and indirect arguments.

### R6 in practice: logical resources, target-selected access

Authored code names **what** a resource is. The engine profile decides **how**
a shader reaches it. Most engines that want bindless everywhere need two or
more access models at once:

- device addresses on Vulkan;
- descriptor-heap indices on D3D12 SM 6.6;
- argument buffers on Metal;
- plain per-resource bindings on WebGPU, which has no portable bindless model today.

If the authored code changes per platform, every material ends up with
`#ifdef`s.

**Authored types (portable):**

| Type | Meaning |
| --- | --- |
| `buffer<T, access>` | A whole buffer |
| `span<T, access>` | A bounded view: base and count. This is the "bounded span" from the native pointer design, distinct from a pointer. |
| `texture2d<T>`, `texture_cube<T>`, `storage2d<F, access>`, … | Images |
| `sampler` | A sampler |
| `T?` | An optional resource. Absent is a typed state, not a magic number. |
| `Ptr<T, access>` | A raw device pointer, with arithmetic. **Target-gated**: only profiles whose buffer model is `device_address` accept it. Elsewhere it is a diagnostic. |

**Access models (chosen per target in the `engine` record):**

| Model | Buffers | Textures and samplers | Typical target |
| --- | --- | --- | --- |
| `binding` | one binding per buffer | one binding per resource | WebGPU, baseline |
| `table(heap)` | index into a buffer descriptor array | index into a descriptor array | Vulkan descriptor indexing, D3D12 SM 6.6, Metal argument buffers |
| `device_address` | 64-bit address; `span` = `{ addr, count, pad }` | (n/a, so combine with `table`) | Vulkan BDA (Lumina), Metal 3, CUDA-style |

```fresco
engine {
    check:  { … }
    budget: { … }

    target vulkan: {
        api: Vulkan { version: 1.3, spirv: 1.6 }
        buffers:  device_address
        textures: table(LuminaHeap)
        samplers: table(LuminaHeap)
        root:     push_constant(LuminaRoot)
        layout:   { blocks: scalar, matrices: column_major }
    }
    target webgpu: {
        api: WebGPU
        buffers:  binding
        textures: binding
        samplers: binding
        root:     bind_group(0)
        layout:   { blocks: std430 }
    }
}
```

Lumina declares only `target vulkan`. A portable engine declares both, and the
same authored source compiles for each. The compiler owns these lowering
details, so authors and engines do not hand-code them:

- **Non-uniform indexing.** Fresco's uniformity analysis already knows when a
  heap index is not dynamically uniform. On `table` targets it inserts the
  non-uniform decoration (`NonUniformResourceIndex` in Lumina's Slang).
  Authors never write it.
- **Pointer-bearing records.** Lumina's rule "never load a struct containing
  pointers by value" exists because of invalid SPIR-V and GPU-AV failures.
  In Fresco this is a lowering invariant. Records with `Ptr`/`span` fields
  are loaded field by field. It is not author discipline.
- **Optional resources.** `T?` lowers to a sentinel (`0xFFFFFFFF` slot, null
  address) on `table`/`device_address`. On `binding` the host binds a
  placeholder resource, and the argument record carries a presence bit. Code
  must test it (`if let v = velocity { … }`) before use. This is run-time
  absence. Compile-time absence is [`static if`](#generated-only-when-static-if).
- **Alignment.** Vector loads from spans know their alignment from the type
  and layout, which replaces hand-written `loadAligned<16>`.
- **Unsupported combinations are diagnostics.** Examples are `Ptr<T>` on
  `binding`, `storage2d` aliasing a sampled view on a heap that forbids it,
  and atomics on a layout the target lacks. Nothing silently falls back,
  per the maintainer preferences.

This keeps the
[native pointer design](../native-pointers-design.md#accepted-authoring-direction)
boundary: pointer, span, and `DescriptorHandle<T>` stay distinct concepts,
and target selection lives in compiler options and the host profile, not in
pointer-specific `@` attributes. `DescriptorHandle<T>` from that design is
what `table` lowers handle-typed fields to. Authors only see it when they
need a raw slot, for example when storing one in a GPU-written list.

### Root and argument ABI (L0)

The engine declares its root once. Per-entry arguments are an ordinary record
that the root reaches:

```fresco
extern struct LuminaRoot @size(40) {   // FRHIRoot push constant; engine-owned layout
    args:       Ptr<Args>              // retyped per entry: GetPassArgs<T>()
    scene:      Ptr<SceneRoot>
    scene_data: Ptr<SceneData>
    instances:  Ptr<GpuInstance>
    lights:     Ptr<LightData>
}

struct CullInstancesArgs {
    instances: span<GpuInstance>
    visible:   span<u32, write>
    hzb:       texture2d<f32>
    hzb_point: sampler = LuminaHeap.point_clamp
    velocity:  storage2d<rg16float, write>?
}

@compute @workgroup_size(64)
fn cull_instances(args: CullInstancesArgs, @builtin(global_invocation_id) id: vec3<u32>) { … }
```

Fresco never writes the host side (R8). The same records still connect both
sides:

- **Engine-owned layouts (the default).** `extern struct FSceneRoot { … }`
  declares an existing host layout. A declared annotation (`@size(352)`,
  `@offset(n)` on fields) asserts it. Fresco checks its own lowered layout
  against those numbers and reports drift as a Fresco compile error. That
  catches the bug on the shader side, where today only the C++
  `static_assert` sees it.
- **Fresco-authored records**, such as a Fresco pass's arguments. The
  manifest reflects the per-target layout: size, alignment, and every field's
  offset and type. One record's layout differs by target model (a 16-byte span
  on Vulkan, a binding plus a count on WebGPU), so the reflection is per
  target. The engine can use that layout in three ways:
  - read it and write the bytes itself;
  - let the runtime's argument program fill the record;
  - assert its own mirror at startup through the runtime's layout query.

Sentinels, padding, and span shape are profile facts. They are not spelled in
each record.

Lumina's root fields are `uint64_t` today. Typing them as `Ptr<T>` means
either adding integer/pointer conversions or agreeing on a typed root ABI
with the engine. The
[pointer design](../native-pointers-design.md#bounded-spans-and-the-lumina-abi)
requires that choice to be recorded rather than made by quietly changing
engine layouts. The same section lists the remaining prerequisites: span
support in the Naga fork, scalar layout, pointer helper parameters, atomics,
and descriptor handles. This proposal only fixes how they are **spelled** in
integration code.

### Lanes: one material, many shader contexts (L2)

Lumina's templates are not a Fresco concern. Their **contracts** are. A lane
is a typed shader context with capabilities, declared in the `engine` record
and used by an `interface`:

```fresco
engine {
    lanes: {
        forward:          Lane { stage: fragment, derivatives: hardware }
        deferred:         Lane { stage: compute,  derivatives: supplied([uv0, uv1, world_position, vertex_color, normal]) }
        decal:            Lane { stage: fragment, derivatives: supplied([uv0]) }
        visbuffer:        Lane { stage: mesh,     derivatives: none, evaluates: [vertex] }
        visbuffer_masked: Lane { stage: fragment, derivatives: hardware, outputs: coverage_only }
        shadow_masked:    Lane { stage: fragment, derivatives: hardware, outputs: coverage_only }
        moment:           Lane { stage: fragment, derivatives: hardware }
    }
}

interface LuminaMaterial {
    lanes:      [forward, deferred, decal, visbuffer, visbuffer_masked, shadow_masked, moment]
    parameters: LuminaMaterialTable
    fn pixel(ctx: MaterialContext) -> MaterialPixelInputs = default_material_inputs(ctx)
    fn vertex(ctx: VertexContext) -> MaterialVertexInputs = { world_position_offset: vec3(0.0) }
}

material brushed_metal for LuminaMaterial {
    settings editor {
        tint:      vec3 = vec3(0.8)
        roughness: f32 = 0.35 in 0.0 .. 1.0
        albedo:    texture2d<vec4>
        mask:      texture2d<f32>?
    }
    fn pixel(ctx: MaterialContext) -> MaterialPixelInputs {
        let base = sample(albedo, ctx.uv)
        return { ..default_material_inputs(ctx), diffuse: base.rgb * tint, metallic: 1.0, roughness }
    }
}
```

- `interface` is the same construct as `ParticleEmitter`. A lane list is one
  more descriptor field, not a new kind of declaration.
- **Derivatives are a checked capability** (Q13). `sample(albedo, ctx.uv)`
  lowers to implicit-derivative sampling in `hardware` lanes. In `supplied`
  lanes it lowers to explicit-gradient sampling: the lane's template provides
  the gradients of the listed inputs, and Fresco carries them through the
  material by the chain rule. In a `none` lane, implicit sampling or `ddx` is
  a diagnostic that names the lane, and explicit-LOD sampling is still
  allowed. This replaces `#define SHADING_COMPUTE` and
  `SHADING_EXPLICIT_GRADIENTS` with something the checker can see.
- **Coverage-only lanes** (`visbuffer_masked`, `shadow_masked`) compile only the code that
  opacity and masking depend on. Dead-code elimination from the material's
  data flow replaces hand-written `#ifdef MASKED` templates.
- The output is one SPIR-V blob per lane and variant, plus reflected
  parameter layout, matching Lumina's per-`EMaterialShaderStage` blobs. The
  example engine is the same construct with one lane.

### Parameter packing policy (L0/L2)

The engine decides how typed settings reach the GPU. A material does not.

```fresco
struct LuminaMaterialTable {
    vectors:  [vec4; 24]
    scalars:  [f32; 24]
    textures: [texture2d<vec4>; 24]
    flags:    u32
}
// engine record: materials: { table: LuminaMaterialTable, index: instance.material_index }
```

The compiler packs each material's `settings editor` fields into the table by
type. It reports overflow as a diagnostic that names the parameter, and it
reflects a name-to-slot map for the editor and runtime. The example engine's
`@table` draw-record schema is the same policy with a different table. Both
use the per-row initializers from Q7.

### Specialization realization

Lumina has four variant mechanisms. Fresco should keep one authored concept,
`permutation`, and let the profile choose how each one is **realized**:

| Realization | Lumina mechanism | Cost |
| --- | --- | --- |
| `compile` | `#define` variants and material static switches (64-bit permutation key), each a separate blob | binary size |
| `specialization(id)` | SPIR-V specialization constant in the pipeline key | pipeline count |
| `dynamic` | a uniform or push-constant branch | runtime branch |
| `lane` | a second compile of one lane (`MOMENT_GENERATION`) | binary size |

```fresco
settings engine {
    permutation skinned: bool = false, realize: specialization(1), runtime: allowed
}
```

`runtime: allowed` covers `SPEC_SKINNED`'s third state, where the value is
decided at runtime. The specialization constant gets a "dynamic" value, and
the compiler emits both paths behind it. A bool with a tri-state realization
remains a bool in authored code.

### Scheduling without a render graph (L3)

Lumina records passes itself and places global barriers by hand. Fresco
cannot take over that order, but it can make it safe:

- **Reflection.** Every L3 entry reflects its resource accesses (read, write,
  atomic, per stage) from its argument record. The engine, or a debug
  validation layer, derives barriers from them.
- **Engine barrier intrinsic.** When a `graph fn` is used only to sequence
  L3 passes (Lumina's cull → prefix → meshlet-cull chain), and the profile
  declares `scheduling: host_barriers(barrier_fn)`, the hazards inferred
  under R4 lower to calls to the engine-declared barrier, not to a graph
  runtime.

This affects **Q1**. Lumina shows that hazard inference is valuable without a
render graph runtime, so it should be a compiler service that L3 and L4 share,
not an L4-only feature. Lumina also needs `indirect:` dispatch and
draw-count arguments (`CmdDrawMeshTasksIndirectCount`) to be pass/step
descriptor fields.

### Heaps and stock samplers

```fresco
struct LuminaHeap {
    samplers: { linear_clamp: 0, linear_wrap: 1, point_clamp: 2, … }   // fixed slots
    sampled:  [texture2d, texture_cube, texture3d, texture2d_array, texture_cube_array, texture2d<u32>]
    storage:  [storage2d]                 // declared only in entries that write
}
```

- Stock samplers are named, typed constants. The manifest reflects the
  table, and the runtime can check at load time that the engine registered the
  same sampler in each slot. A lockstep break between `RHICore.cpp` and the
  shaders then fails at startup, instead of silently sampling with the wrong
  sampler.
- View aliasing is a heap property. Fresco declares only the views an entry
  uses, which replaces "include `GlobalRHIStorage.slang` only in writers".

### What Lumina does not change

The Lumina pass adds no new `@` forms. The only new declaration form is
`extern struct`, which marks a layout the engine owns:

- **Profiles, lanes, packing, and realization** are fields of the proposed
  `engine` record (see [Engine settings](#engine-settings)).
- **Heaps and roots** are ordinary records. Their layout is checked through
  declared annotations (`@size`, `@offset`).
- **Materials** are `interface` slots.
- **Handles** are types.

That is the extensibility test from R2. A second engine with a different
philosophy fits into prelude records and owning lowering stages, with no
parser branches. Engine-specific `.fr` files (Lumina's heap, root, and
material contract) belong to that integration under `.experiments` (or a
future `integrations/lumina`), not in the language prelude.

## Runtime direction (exploring)

This direction is less settled than the rest of the document. It is recorded
here because R8 depends on it. The full design lives in its own proposal,
[Embeddable manifest runtime](embeddable-runtime.md). This section only
summarizes how it relates to integration syntax.

**Problem.** The manifest is declarative data, and every host reinterprets it
by hand. In the example engine, that interpretation is spread across six
runtime modules:

- `runtime/recipe.rs`: order verification, capacity formulas, resize checks;
- `runtime/mesh.rs`: provider-key allowlists, binding wiring;
- `runtime/tables.rs`: table columns;
- `runtime/technique.rs`;
- `runtime/compute_graph.rs`;
- `runtime/particle_contract.rs`.

A second engine would have to rewrite all of it, and Lumina would need the
same kinds of work: packing material settings into `FMaterialUniforms`,
filling pass arguments, and checking layouts. Generating host code for this
is ruled out (R8). The alternative is to run it.

**Direction.** `fresco-artifact` is currently only the serde manifest types.
It (or a sibling crate) could grow into a small, embeddable Rust runtime with a
C ABI. The runtime would:

- load a manifest;
- run the short programs the compiler put in it;
- call back into the engine for anything that touches memory or the GPU.

It never calls a graphics API and never owns GPU objects.

| Program | Runs | Does | Lumina | Example engine |
| --- | --- | --- | --- | --- |
| Material setup | Material load or settings change | Evaluates per-row table initializers (Q7). Packs settings into the engine's table layout, as bytes plus heap-slot requests. Computes the variant key. | yes | yes |
| Arguments | Per pass, per frame, or per draw | Fills a Fresco-authored argument record from engine-supplied addresses, slots, and counts, into engine-provided memory | yes | yes |
| Layout query | Startup | Returns size, alignment, and field offsets, so the engine can assert its own mirrors | yes | optional |
| Graph adapter | Per frame | Walks an L4 description. It evaluates sizes (Q8), conditions (`where:`), and inferred hazards (Q1), and emits `add_pass` / `read` / `write` / `barrier` callbacks into the engine's own frame graph or command recording | no | yes |

**Shape:**

- **Bounded programs, not a general VM.** Programs are a register bytecode for small pure expressions: arithmetic, comparisons, field reads, and table lookups. They have no loops, recursion, or allocation, so every program has a statically known maximum length, and it is verified at load.
- **Speed comes from doing little at runtime.** The compiler pre-bakes what it can into byte images with patch lists. The common per-draw case then becomes "copy N bytes, write K slots".
- **The C ABI** uses opaque handles and one engine callback table: transient allocation, resource lookup by host key, heap slot, barrier, dispatch, and draw. Manifest version and ABI version are checked independently at load.
- **The same runtime compiles to wasm.** The example engine and the playground become its first consumers and replace their hand-written interpretation. That gives dogfooding before any outside engine depends on it.

**Graph adapter, not graph executor.** Engines that already have a frame graph
want Fresco work to appear *inside* their graph. They don't want a second
graph next to it. Engines without one (Lumina) want ordered calls and barrier
hints. Both are served by an adapter that emits into engine callbacks. A
standalone "tiny frame graph" that schedules on its own would quietly make
Fresco own the frame again, which the premise rules out.

**Risks:**

- **Scope creep into an engine.** Rule: the runtime owns no GPU objects, no memory, and no scheduling decisions beyond what a program states.
- **Debuggability.** It needs a disassembler and a trace callback from day one.
- **ABI permanence.** A C ABI is a long-term commitment. Keep the surface minimal and versioned.

**Recommendation.** If this becomes its own proposal, start with the material
setup, argument, and layout-query programs. Every engine needs them, Lumina
included, and they replace real hand-written code in the example engine. Add
the graph adapter once Q1's hazard inference exists as a compiler service.

---

## Attribute disposition

Every attribute that appears in the example engine, with its proposed home.

| Today | Proposed |
| --- | --- |
| `@renderer(id, label)`, `@default` | `renderer` declaration; `id:`, `label:`, `default:` fields |
| `@configure(name, v)` | `configure: { name: v }` |
| `@external(local, src)`, `@provider(res, name)` | `host` record field; `from host.x` |
| `@image(n, fmt)`, `@buffer(n, bytes)`, `@dimensions` | `let n = image<fmt>(…)`, `buffer<T>(count: …)` |
| `@table_data(r, T, f)` | `table(T).f` expression |
| `@per_invocation(r, bytes)` | `buffer<T>(count: extent.count * k)` |
| `@node(n)` | `let n = draw …` |
| `@draw(v, f, mode[, n])`, `@draw_depth` | `draw pass { vertex:, fragment:, geometry: }` (no `fragment:` means depth-only) |
| `@dispatch(entry, …)` (recipe/technique) | `dispatch pass.entry { extent: … }` |
| `@dispatch(Contract, hook, settings, scope)` | `fn x = dispatch Contract.hook { settings:, scope: }` |
| `@bind(shader, res)` (step) | `bind: { shader: res }` |
| `@bind(hook)`, `@compose(p)` (interface method) | `= hook name { threads: p }` |
| `@color(i, r)`, `@depth(r)`, `@attachment(r, load, store)` | `color: { field: r.load }`, `depth: r.clear(1.0)` |
| `@after(n)` | inferred from resources; `after: [n]` for the rest |
| `@when(pred)` | `geometry: meshes(where: pred)` |
| `@transparent_queue(q)` | `queue: q` |
| `@enabled(n)`, `@instances(n)` | `enabled: n`, `procedural(instances: n)` |
| `@technique`, `pipeline(kind)` | `technique` / `renderer` declarations; `pipeline(...)` categories retired |
| `@shader`, `@factory(f)`, `@service(S)` | `@shader` is derived from use (Q5); `factory: f`; `provides: [S]` |
| `@meta(k, v)` | typed descriptor record, or a declared annotation |
| `@entry(n, m)`, `@surface_defaults`, `@surface_properties(n)` | `entry:`, `surface_defaults:`, `surface_properties:` fields |
| `@context`, `@composition`, `@default_material` | `context:`, `composition:`, `default: true` on `material_properties` |
| `@compose(mix)` (channel) | `compose:` default plus per-channel override |
| `@config(editor\|engine)`, `@permutation`, `@range` | `settings editor { … }`, `permutation` modifier, `in lo .. hi` |
| `@known`, `@expect`, `require {}` | deleted with the staged dialect; `static if` covers variant-only declarations (Q4) |
| `@evaluate(x)` | `fn f = evaluate x(…)` |
| `@prepare(R)`, `@pure` | stay as function markers (declared annotations) |
| `@vertex`, `@fragment`, `@compute`, `@workgroup_size` | stay as function markers |
| `@semantic`, `@location`, `@builtin`, `@interpolate`, `@group`, `@binding`, `@access` | stay as field/parameter markers; `@access` folds into `buffer<T, access>` |
| `@table`, `@table_index`, `@schema_value`, `@implementation_value`, `@implementation_settings_value`, `@property_value` | `@table({ … })` on the struct, plus per-row field initializers (`row.index`, `row.schema({ … })`, `row.implementation(s).id`) (Q7) |
| `@draw_data`, `@source` | `from draw_data(…)` / `from table(T)` on the binding field |
| `@geometry(…)` on `resource` | `geometry: { vertices, indices, … }` descriptor field |
| `@usage(x)`, `@profiles(…)`, `@schema(x)` | settings-field markers; declared annotations |

## Extensibility and evolution

To add an engine feature under these rules:

1. Add a field (with a default) to the relevant descriptor record in the prelude, or declare a new record or annotation in the engine.
2. Lower the typed value in the owning compiler stage.
3. Document it. The language reference is generated from the prelude.

No parser change, placement table, or string re-parsing is needed.

To change a design later:

- **Editions.** A file-level `edition 2` line (or a workspace setting) selects the surface. The old and new surfaces lower to the same typed HIR during migration, so the regression check can be **equivalence canaries**: same manifest and pass plan from both spellings. That is relational, not a snapshot baseline, as `docs/testing-canaries.md` requires.
- **Mechanical migration.** `fresco fmt --migrate` rewrites edition-1 attribute recipes to edition 2. The formatter already owns the AST round trip.
- **Deprecation.** Retired fields stay parseable for one edition and produce a diagnostic that names the replacement. They don't silently keep working.

## Non-goals

- Changing shader-body syntax (expressions, statements, WGSL-style builtins). This proposal is about the integration layer.
- Changing runtime semantics of existing renderers, beyond the explicit decisions below (dependency inference, clear-on-zero-draws, `static if` bindings, viewport-sized `bins`).
- A general macro system. Graph functions and records are the reuse mechanism. Macros would reintroduce untyped surface area.
- Keeping attribute and descriptor spellings both first-class indefinitely.
- Owning any engine's frame structure. Every engine owns its renderer, and L4 is a description the engine executes.
- Generating host-language code (C, C++, Rust headers or glue). See R8.
- Designing the Lumina integration itself, or the pointer, span, atomics, or descriptor-handle compiler work. This proposal only ensures the syntax leaves room for them. The [native pointer design](../native-pointers-design.md) owns that work.

## Decisions

These are recommended answers, drawn from how the example engine, the
compiler, and Lumina work today. They are not accepted yet. The proposal stays
**Exploring** until the maintainer signs off, and a prototype or canary must
confirm each answer before it is relied on. Citations are to the code as of
2026-10-02. Paths without a crate prefix are under `crates/fresco/src` or
`integrations/example-engine/src`.

- **Q1. Dependency inference: infer from resource hazards, in program order.**
  - **Recipes today:** `@after` is the only source of edges. `order_by_after` (`driver/recipes.rs:274`) is a stable sort in declaration order, so reordering lines already changes meaning today. Reads and writes are only *validated* (`recipes.rs:510-574`).
  - **Elsewhere, edges are already inferred:** techniques add read-after-write edges to the latest writer (`driver/techniques.rs:578-603`), and `for self` compute operations derive dependencies from kernel reads (`style_operations/compute.rs:158-166`).
  - **The authored edges are almost all redundant.** Of the 27 authored `@after` edges in the three renderers, 26 are implied by a resource hazard. The exception, `clear_lighting` after `clear_geometry`, orders two clear passes that `.clear` removes anyway. Two more (`lighting_resolve` after `clear_geometry`, and `deferred_buffer_view` after `lighting_resolve`) are redundant with a later writer.
  - **Rule:**
    - A read follows the latest earlier writer of that resource.
    - A write follows earlier readers and writers.
    - Writes to one resource happen in program order. Today recipes reject them as unordered, and techniques require an explicit `@after`.
    - When the latest writer is conditional (`where:`), a reader also follows each earlier writer, up to the nearest unconditional one. This fixes a fragility in today's deferred renderer: `lighting_resolve` reaches `sun_shadow` only transitively through the conditional `opaque` step.
    - `after:` remains for ordering that no resource expresses.
    - Techniques and recipes share the one rule.
  - **Scheduling:** this is a compiler service shared by L3 and L4. The runtime's existing "unsatisfied dependency" check (`runtime/recipe.rs:155`) stays as the verifier.
- **Q2. `graph fn` and `style … for self` are one construct.**
  - `for self` already lowers to ordinary recipe steps through generic splicing: prefix the step names, rewire ports to renderer resources, add boundary edges, and insert (`driver/operation_composition.rs:57-432`).
  - `graph fn` is the base form. `style … for self` is a `graph fn` that runs once per material, with an implicit `self` and a per-material invocation identity.
  - Contract-point placement (`at P as target`) and the mesh-only restriction remain specific to `for self`.
- **Q3. No name-keyed spread into `bind:`. Binding composes as a list of records.**
  - Passes need different subsets of the same resources:
    - Shadow steps bind 2 of the factory's 9 bindings.
    - Opaque and transparent steps bind 7.
    - `lighting_resolve` binds 16.
  - The same buffers are also renamed between passes: `lamps` is `point_lights` or `lights`, and `bins` is `light_tiles` or `tiles`.
  - A pass therefore declares `binding: [RecordA, RecordB]`, and a step supplies `bind: [a, b, { … }]`, where each element matches one record by type. Field names are canonical, so the renames go away.
  - **Owning-layer fix:** an unknown binding on a mesh/factory step is currently caught only at runtime (`runtime/mesh.rs:588-596`), while GPU-program steps are checked by the compiler (`driver/recipes.rs:380-387`). The compiler should check both.
- **Q4. Delete the staged dialect** (`10_forward.fr`, `20_forward_plus.fr`), and the features that exist only for it, as a separate change.
  - The files are imported and parsed, but their pass bodies are kept as raw tokens and never statement-checked. Constructs like `select`, `unroll for`, and `cascade_for<…>` therefore *appear* to compile. That is the "unsupported constructs appear to work" pattern that AGENTS.md forbids.
  - Features only they use:
    - `permutations {}` with `@known` and `when`/`else` guards;
    - `require {}` and `disable()`;
    - `@expect` (allowlisted with no semantics);
    - `axis` declarations;
    - `dispatch host`;
    - `reads:`/`writes:` axis agreement;
    - the `config_axes` manifest;
    - the `stage:`/`draw:`/`blend:` pass scalars;
    - the vocabulary in `00_prelude`/`01_core`/`02_functions` used only by them.
  - Executable forward+ already lives in `core/05_mesh_contract.fr` and `runtime/forward_plus.rs`. `static if` covers the one real need, variant-only declarations.
  - **This is a feature removal.** Tests that exercise those features go with them. Tests that only use these files as convenient input (the web manifest inspector and diagnostics UI tests, among others) must be re-pointed at executable sources, not deleted.
- **Q5. Drop `@shader`, and derive emission from use.**
  - Every `@shader` pass in the engine is used by a step, and every step-used pass without `@shader` is a factory mesh pass. Emission is already filtered by use (`driver/mesh_pass/shader.rs:26-36`).
  - The step verb determines the program kind:
    - `dispatch` and fullscreen `draw` produce a GPU program;
    - `meshes`/`instances` geometry produces a factory mesh pass.
  - Compute lowering already adds `@shader` synthetically (`style_operations/compute.rs:107`).
  - An L3 pass with no step is emitted when the `engine` record exports it. Otherwise it gets an "unused pass" warning.
- **Q6. No `.Fixed`. Use bare variants resolved by expected type.**
  - Fresco already resolves bare variants. User enums are searched globally, with an "ambiguous, qualify it" diagnostic (`check/expr.rs:328-360`). Built-in `contextual` enums resolve from the expected argument type (`registry.rs:262`).
  - Extend expected-type resolution to descriptor fields, settings defaults, and comparisons against a typed operand. Then `blend: Translucent` and `allocation == Fixed` work without a new token.
- **Q7. Table fields become per-row initializers.**
  - Today's markers are positional, and one of them, `@property_value`, is undocumented (`driver/tables.rs`). Every one of them is really "compute this column for each surface". The proposed form:

    ```fresco
    @table({ rows: surfaces, order: ascending, first_index: 1 })
    struct DrawRecord {
        material_id:           u32 = row.index
        shading_model:         u32 = row.schema({ base: 0, standard: 1, unlit: 2 })
        style_id:              u32 = row.implementation(style).id
        style_settings_offset: u32 = row.implementation(style).settings_offset
    }
    ```

  - Lumina's material table is the same mechanism (`vectors: [vec4; 24] = row.pack(settings, vec4)`). The runtime's material-setup program evaluates these initializers, so one evaluator serves both engines.
- **Q8. Size `bins` from the viewport, and reallocate on resize.**
  - Today `bins` is a fixed 2 MiB. On resize, the runtime computes the real need (`ceil(w/16)·ceil(h/16)·8` bytes) and *errors* when it is exceeded, at roughly 8192×8192 (`runtime/recipe.rs:742-758`). Images are already recreated on resize, but buffers are not.
  - `buffer<u32>(count: viewport.tiles(16).count * 2)` removes the hard-coded size and the resolution cap.
  - **Behavior change:** memory now scales with the viewport. The forward renderer's dummy 8-byte `bins` disappears through `static if`.
- **Q9. One `host` record per engine.**
  - The provider namespace is already shared: particles read the same `scene` buffer as the mesh renderer (`runtime/particles.rs:210`). `presentation` and `depth` overlap too.
  - Today each consumer validates keys with its own hard-coded allowlist (`runtime/mesh.rs:598-624`, `runtime/particle_contract.rs:110-127`).
  - With one engine-declared `host` record, the compiler checks key existence and type. The runtime resolves keys through one engine callback in place of per-consumer allowlists.
- **Q10. Choose the access model per target and per resource kind (buffers, textures, samplers), with no per-resource overrides.**
  - Lumina is uniform within each kind:
    - one descriptor set, holding three bindless arrays;
    - every shader buffer is a device address;
    - the only exception, index and indirect buffers, is resolved on the host (`VulkanRHI.cpp:861-872`).
  - Fresco's WebGPU path uses per-resource bindings throughout (`lower.rs`). There are no binding arrays anywhere.
  - A per-resource override can be added later as a field, if an engine needs one.
  - Bindless is still an open item in `TODO.md` ("capabilities and diagnostics for bindless resources, non-uniform indexing"). This proposal only fixes its spelling.
- **Q11. The engine owns the layout. Fresco checks it and reflects it, and never generates it (R8).**
  - Lumina mirrors about 111 structs by hand, checked only by `static_assert(sizeof)`, and they have already drifted in element type names and comments. `extern struct` with `@size`/`@offset` makes Fresco check its side against Lumina's numbers. Fresco-authored records are reflected in the manifest and filled or queried through the runtime.
  - **The root needs no layout change.** `FRHIRoot` has no C++ struct. It is five 8-byte push-constant slots that the Slang side declares as `uint64_t` and casts. A root of five `Ptr<T>` fields has the identical 40-byte layout, which is the "genuinely compatible typed root ABI" the pointer design allows. The `lumina-fresco` probe already used typed pointer roots.
  - The root still needs push-constant pointer support in the native route. Nested raw addresses would need the same treatment, or integer/pointer conversions.
- **Q12. No arena emulation.**
  - The native pointer design forbids rewriting pointers into integer or buffer indices (`native-pointers-design.md:144-145, 166-169`).
  - `Ptr<T>` on a target without device addresses is a diagnostic.
  - `span<T>` is not a pointer, so on `binding` targets it lowers to a binding plus a count.
  - Fresco's existing settings-buffer offsets (`driver/implementations.rs:137-149`) are data indexing, not pointer emulation, and they stay.
- **Q13. Lanes declare one of three derivative capabilities, matching Lumina's three cases.**
  - **Lumina's three cases:**
    - **`hardware`:** forward, masked VisBuffer, masked shadow, particle, UI, terrain, and post-process pixel templates.
    - **`supplied([inputs])`:** the deferred compute lane reconstructs perspective-correct barycentric gradients from the VisBuffer triangle (`VisBufferSurface.slang:177-288`). Decals derive UV gradients from world-position gradients.
    - **`none`:** vertex and mesh stages sample at LOD 0. Compute lighting such as `DeferredLighting` and SSR has no derivatives.
  - **Propagation:** Lumina's material compiler tracks each value's derivative state as zero, valid, or unknown (`MaterialCompiler.h:67-97`). For unknown values it falls back to UV0 gradients with a warning. Fresco should do the same propagation, but make "unknown" a diagnostic, not a fallback.
  - **What this catches:** a raw `ddx`/`ddy` in a lane without derivatives is caught at check time. Lumina's `NormalFromHeight` appears to emit an unguarded `ddx` that would reach the deferred lane (`MaterialCompiler.cpp:3558-3566`). That is unverified, but it is exactly this kind of bug.
- **Q14. Realization follows ownership.**
  - Material authors own `settings editor { permutation … }`. In Lumina these are static switches and blend-mode defines, always realized as `compile`, so they can guard `static if`.
  - The engine owns `settings engine { permutation … }`. Its profile chooses the realization. In Lumina, the pipeline key sets specialization constants 1–8 (`DefaultSceneRenderer.cpp:2011-2026`) from scene and geometry state, not from the material.
  - Fresco already makes engine permutations non-overridable by materials (`driver/surface_properties.rs:342-348`).
  - A material cannot choose a realization.
- **Q15. Ship L3 with access reflection first. Add the runtime's graph adapter later.**
  - Lumina has about 138 hand-placed global barriers spread across its pass functions, no resource-state tracking, and every image in `GENERAL` layout. It will insert Fresco passes into sequences it owns.
  - Per-entry reflected access (read or write, per stage) is what lets the engine pick the right barrier helper.
  - Emitting barrier callbacks belongs to the runtime's graph adapter. It only matters for engines that hand Fresco a sequence to describe.

### Still needs a maintainer call

- **Q4:** sign off on removing the staged dialect and its features. This is a deliberate feature removal, not cleanup.
- **Q8:** accept viewport-scaled `bins` memory in place of the fixed 2 MiB cap.
- **Q11:** a typed `Ptr<T>` root is binary-compatible with Lumina. Using it in Lumina's shaders is still a choice for whoever owns Lumina's shaders. Record it in the pointer design when it's made.
- **Runtime:** now its own proposal, [Embeddable manifest runtime](embeddable-runtime.md). It recommends a sibling `fresco-runtime` crate, with `fresco-artifact` staying the schema crate.

## Investigation and acceptance checklist

These measure investigation, not delivery.

- [ ] Inventory every attribute consumer (`has_attr`, `PipelineAttribute::synthetic`) and map each one onto a descriptor field or declared annotation. Flag any that don't fit R1–R7.
- [ ] Draft `fresco/graph.fr` prelude records for `DrawStep`, `DispatchStep`, `Renderer`, `Technique`, `PassState`, and `ProvidePoint`.
- [ ] Prototype record literals, same-type spread, and field shorthand in the parser/checker. Confirm no conflict with struct-literal or block grammar.
- [ ] Prototype `let`-bound steps and graph functions lowering to the existing recipe HIR for `renderer_forward` only. Prove equivalence with an equivalence canary.
- [ ] Confirm Q1 with an equivalence canary: the inferred order for each bundled renderer is a valid order under today's `@after` edges, and the `@after` edges that are dropped are exactly the ones listed under Q1.
- [ ] Decide clear-on-zero-draws load-op semantics with the runtime and confirm `clear_opaque` / `clear_gbuffer` can be removed.
- [ ] Prototype `static if` on fields, parameters, literals, and statements. Confirm that the forward renderer's dummy `bins` disappears, that naming a guarded item outside its guard is a diagnostic, and that the guarded branch is checked in every variant.
- [ ] Move the unknown-binding check for mesh/factory steps from the runtime (`runtime/mesh.rs`) into the compiler (Q3).
- [ ] Specify `fmt --migrate` coverage for every row of the disposition table.
- [ ] Check every L4 step property against L3. Anything an engine without a render graph needs (resource access, workgroup size, indirect arguments) must live on the pass, not the step.
- [ ] Write the Lumina profile (heap, root, lanes, material table) as `.fr` under `.experiments/lumina-fresco`, using only records and declared annotations. List anything that does not fit R1–R7.
- [ ] Compile one material into Lumina's forward and deferred lanes from a single source. Show implicit vs explicit-gradient sampling, and show a diagnostic for implicit derivatives in a `none` lane.
- [ ] Declare `FSceneRoot` and one pass-argument record as `extern struct` with `@size`/`@offset`. Assert that Fresco's lowered layout matches Lumina's `static_assert` values, and that a one-field drift is a diagnostic (mutation canary).
- [ ] Lower one argument record under `binding` and `table`/`device_address` from identical source. Show that `T?`, `span<T>`, and non-uniform indexing lower correctly on each.
- [ ] Map Lumina's `#define`, specialization-constant, static-switch, and dual-compile variants onto `permutation` realizations. Flag anything left over.
- [ ] Independent of syntax, clean up F3/F5/F9 duplicates in the example engine (fullscreen entry, octahedral codec, BRDF, `sign_nz`, `inverseSqrt`, `const` form, separators, dead `@external(depth, depth)`).

## Decision history

- 2026-10-02: Proposal opened from a review of the example engine `.fr` sources.
- 2026-10-02: Revised against Lumina (Vulkan-only, device-address buffers, bindless heap, no render graph). Added R6 (logical resources, target-selected access), R7 (integration layers), the Lumina section, and Q10–Q15.
- 2026-10-02: Made "every engine owns its renderer" the premise, and added R8 (no host-language code generation) and the runtime direction. Replaced guard suffixes with `static if` blocks. Answered Q1–Q15 with recommendations from the engine, compiler, and Lumina sources. Dropped arena pointer emulation (forbidden by the native pointer design).
