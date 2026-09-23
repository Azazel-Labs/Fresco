# Example engine documentation

These documents describe the bundled engine's architecture, rendering paths,
validation, and historical decisions. Language semantics belong in
[LANGUAGE.md](../../../LANGUAGE.md); runnable setup belongs in the
[engine README](../README.md) and [host guide](../../example-engine-host/README.md).
Dated audits and measurements describe their recorded checkpoint, not necessarily
current implementation status.

| Document | Scope |
| --- | --- |
| [Architecture](example-engine-architecture.md) | Engine structure and integration plan |
| [Forward+](example-engine-forward-plus.md) | Tiled light culling and forward shading |
| [Deferred](example-engine-deferred.md) | G-buffer and deferred lighting |
| [Buffer views](example-engine-buffer-views.md) | Renderer inspection views |
| [Completion audit](example-engine-completion-audit.md) | Recorded completion assessment |
| [Measurements](example-engine-measurements.md) | Recorded performance and measurement workflow |
| [Path parity decision](example-engine-path-parity-decision.md) | Scoped rendering comparison and acceptance criteria |
| [Renderer retirement](example-engine-renderer-retirement.md) | TypeScript retirement and regression ownership |
