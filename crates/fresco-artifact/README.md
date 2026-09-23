# Fresco artifact contracts

Shared, GPU-independent Rust types for producing and consuming Fresco's manifest JSON.
The native example engine and browser bindings should use these definitions
rather than maintaining separate representations of the wire format.

The default build depends on Serde and JSON support, without the compiler, a GPU
runtime, or WASM bindings. The optional `typescript` feature enables the existing
browser contract generator. Run that generator from the WASM crate:

```sh
cargo test -p fresco-wasm export_wasm_contract_types
```

These types describe serialized data. Deserialization does not establish that a
device supports an artifact, that resource bindings are valid, or that a pass plan
is executable. Those checks belong to runtime preparation.

The compiler now emits pass plans, engine-pass metadata, mesh variants, particle
pipelines and layout/allocation records, global-uniform layouts, storage-buffer parameters,
texture declarations and metadata, surface context/contract requirements,
custom material-channel mappings, surface settings, editable entry properties,
pipeline declarations and pass semantics, editor configuration axes, and vertex
factories with their attribute layouts and resource bindings using
these shared types.
Shared-layout signature uses are retained by browser consumers too.
Compiler tests verify pass-plan round trips preserve emitted resource bindings,
kernel values, and omitted fields. Buffer-record round trips also retain field
offsets, byte sizes, and dynamic-array type metadata using compiler-owned fixtures.
Texture records preserve omitted metadata, default assets, and named channel
layouts. Optional channel lists distinguish an absent declaration from an
explicitly present empty list; browser contracts expose this optionality too.
Particle and surface property checks round-trip the serialized JSON, including
source edit ranges, insertion positions, choices, and f32 values.
Pipeline round trips cover all/editor/runtime profiles, including omitted pass
lists after editor-only stages are stripped and their excluded configuration axes.
Vertex-factory round trips preserve byte offsets, GPU formats, logical and numeric
bindings, transform entries, and omitted interface/transform metadata using local
compiler fixtures.

Path buffers carry their geometry in `ManifestPathData`, using the explicit
`fresco-path-segment-v1` layout identifier. Each segment occupies 56 bytes:
four `vec2<f32>` control points at offsets 0, 8, 16, and 24; arc-length start
and length at 32 and 36; a `u32` kind at 40; and the midpoint arc-length
fraction at 44. Bytes 48 through 55 are padding. Kind 0 denotes quadratic
geometry (including lines); kind 1 denotes a cubic. The runtime packs these
typed records rather than depending on a Rust struct's memory layout.
Older manifests without geometry data still deserialize, but cannot prepare a
path buffer: recompile the source to obtain the payload. Runtime preparation
checks the layout version, stride, segment count, finite values, and device limits.

The compiler and runtime now share the complete manifest schema, including root,
canvas, and surface containers. `driver/emit.rs` assembles these public artifact
records directly; it no longer owns a parallel serialized manifest schema.
Complete canvas, mesh, and emitter artifacts are tested through serialization
and deserialization, including omission of unused optional resources.
Typed parameter defaults now use shared `ManifestParamDefault` and
`ManifestArrayDefault` records. The producer retains f32 scalar/color values until
serialization; readers still interpret the JSON default using the parameter's
declared type, since an untagged JSON number cannot identify its shader type.

Root, canvas, and surface containers carry the same producer/consumer type
parameters as their parameter records. Canvas and surface parameters use shared
`ManifestParam<D, N>` and
`ManifestSurfaceParam<D, N>` records. Producers instantiate them with
`ManifestParamDefault` and `f32`, preserving the compiler's decimal serialization.
Default consumer types remain `serde_json::Value` and `f64`, so runtime validation
continues to interpret defaults against the declared shader type. The optional
TypeScript generator exports concrete records with `unknown` defaults and numeric
ranges; Rust transport parameters do not appear in the browser API.

Manifest JSON omits unused optional resources. The WASM response adapter preserves
the browser API's materialized empty collections (for example,
`canvas.global_uniforms = []`) after serialization. This transport normalization
uses the shared manifest; it does not select engine resources or provide missing
shader bindings. Generated-WASM checks cover this distinction.
