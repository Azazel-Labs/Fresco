# Example engine local measurements

Measured September 19, 2026 UTC on Windows x64 with an NVIDIA Turing GPU
and headless Chrome 153.0.8010.48. Native and WASM packages used release builds.
The native and browser scripts ran sequentially. These observations describe one
local run, not performance guarantees or CI thresholds.

Reproduce with the [measurement commands](../../example-engine-host/README.md#measure-the-integration).
The scripts emit JSON including sample counts, timestamps, environment, and timing
limits. Keep the raw report when comparing builds.

## Artifact sizes

| Artifact | Uncompressed bytes |
| --- | ---: |
| Native executable (excluding separate debug symbols) | 21,678,080 |
| Compiler WASM | 6,870,605 |
| Renderer WASM | 1,307,636 |

The native executable embeds its engine profile, default source, and demo texture.
The browser keeps compiler and renderer in separate packages; hosting compression
can reduce transfer sizes but is not included here.

## Native process timings

All launches used the system temporary directory as their working directory.
The canvas used only embedded inputs; mesh and particle probes supplied explicit
absolute source paths. Each requested frame count was checked in process output.

| Scene | Compile/check process (ms) | One-frame process median, 3 runs (ms) | 120-frame process (ms) |
| --- | ---: | ---: | ---: |
| embedded_canvas | 104.5 | 1011.6 | 2930.6 |
| mesh | 100.6 | 1003.2 | 2945.3 |
| particles | 133.2 | 1076.8 | 3012.5 |

These include process startup and shutdown; rendering runs also include compilation,
window/device setup, and presentation. They are not isolated GPU execution times.

## Browser timings

The probe uses an empty 512 x 512 host, with no playground UI or concurrent demo
animation. The compiler runs in a worker. Module startup includes fetch and WASM
initialization; the device figure includes surface/camera setup and resize.

- Compiler worker startup: 853.2 ms.
- Renderer module startup: 41.5 ms.
- Renderer device startup: 172.4 ms.

| Scene | Compiler round trip (ms) | First preparation (ms) | Repeated preparation median, 4 runs (ms) | Warm frame submission p95, 120 frames (ms) |
| --- | ---: | ---: | ---: | ---: |
| canvas | 211.9 | 74.6 | 1.20 | 0.1 |
| mesh | 87.1 | 57.1 | 3.65 | 0.1 |
| particles | 100.3 | 134.9 | 3.60 | 0.1 |

Frame timings measure host preparation and queue submission, without waiting for
GPU completion or vsync. Twenty warm-up frames precede the submission samples.
Submissions approach the browser timer resolution; a recorded zero means the
duration fell below measurement resolution, not that rendering costs nothing.

Browser/driver caches and system load remain uncontrolled. In another release
run on this machine, compiler-worker startup measured 55.2 ms instead of 853.2 ms.
Use repeated runs for comparisons; these measurements do not establish GPU
throughput, cold-driver startup, or performance on other platforms.
