# Testing canaries

There are no saved-output baselines, refresh commands, or update modes. The five
canaries are broad alarms: a failure means some part of the compiler-to-renderer
path broke, not necessarily the component named by the first failed assertion.
Existing focused non-snapshot tests remain useful and are not removed.

## Run

`cargo xtask test`, `cargo xtask ci`, and `cargo xtask ci-strict` default to
half the available logical CPUs (rounded up, minimum one) for build jobs and test workers.
Set `CARGO_BUILD_JOBS` and `RUST_TEST_THREADS` to override them independently.
The example compilation and WGSL validation suites run one test per example,
so examples can run concurrently and failures remain individually named.

Native canaries (also run by workspace tests and CI):

```sh
cargo test -p fresco-cli --test canaries
```

All five locally, from `crates/fresco-wasm/web`, after installing dependencies:

```sh
npx playwright install chromium
npm run test:canaries
```

To run only the behavior canary after building the current WASM:

```sh
npx playwright test --config playwright.canaries.config.mjs
```

GPU tests are local-only. CI runs the four native canaries, not the behavior
canary, and must not install test browsers or invoke `test:gpu` or the combined
`test:canaries` command. Set `FRESCO_GPU_BROWSER=chrome` locally to use installed
Chrome. No adapter, invalid shaders, and runtime errors fail the test rather
than skipping it.

Software mode explicitly selects Chromium's SwiftShader adapter, rather than
silently substituting a renderer when hardware is unavailable. The Vulkan switch
is documented in [Chromium's SwiftShader guide](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/gpu/swiftshader.md).
Optional local software rendering uses `FRESCO_GPU_SOFTWARE=1`; it tests execution,
not hardware portability or performance. Initial validation used Windows Chrome
with hardware. Forcing software mode in that installation did not provide an adapter.

## Examples to extend

The shared authored inputs are in `tests/canaries/`. They exercise transitive
engine imports, typed context, scalar and array controls, imported functions,
loops, shapes, composition, and runtime frame inputs.

| Category | Example | Oracle |
| --- | --- | --- |
| Behavior | `web/tests/gpu/canaries.spec.mjs` | Calculated RGBA probes, parameter changes, time/delta, resize, and repeat-frame equality |
| Semantics | `fresco-cli/tests/canaries.rs` | Naga validation, typed entry ABI, executable calls, and manifest/resource agreement |
| Equivalence | Same native file | Repeat compilation, formatter idempotence and meaning preservation, helper renaming |
| Diagnostics | Same native file | Valid control plus independent broken imports, names, types, and semantic-role declarations |
| Mutation | Same native file | Corrupted stage metadata, resources, controls, executable stages, and ABI must fail the shared semantic oracle |

To add coverage, extend the shared program and assert an observable property.
Keep numeric probes away from antialiased boundaries and explain tolerances.
Compare two live executions for equivalence instead of checking in the output
of either execution. The native equivalence check removes function debug names
but preserves instructions, types, operands, and resources; it is intentionally
stricter than proving arbitrary mathematical equivalence.

Every negative or mutation case needs a passing positive control. The mutation
canary is a small deterministic demonstration, not an exhaustive mutation-testing
framework: a surviving mutant means the chosen checks do not detect that defect.
Failure screenshots and measurements are debugging artifacts, never baselines.

These five examples replace the testing approach, not the full historical breadth
of the removed output comparisons. Grow coverage from real failures.
