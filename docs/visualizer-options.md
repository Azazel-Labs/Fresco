---
title: Fresco Visualizer Options
description: Reference for inline visualizer directives, option syntax, rendering modes, and sweep behavior in the Fresco web playground.
author: Copilot
ms.date: 2026-07-15
ms.topic: reference
keywords:
  - fresco
  - visualizer
  - sparkline
  - swatch
  - thumbnail
  - @viz
estimated_reading_time: 8
---

## Overview

Fresco inline visualizers are comment directives attached to let and param lines.
They render previews directly in the editor so you can inspect expressions without
switching context.

Two directive names are supported:

* `@viz`
* `@visualizer`

Both names use the same option parser.

## Directive Forms

Trailing form on the same line:

```text
let speed = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)  // @viz
```

Leading form on its own line:

```text
// @visualizer(kind=timeseries, domain=time, title="speed")
let speed = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)
```

## Supported Targets

Visualizers currently attach to:

* `let` bindings
* `param` declarations

For params, the binding span is the param identifier so the visualizer can still
render even when expression span capture is limited.

## Options Reference

Options can be positional tokens or key value pairs.

### Domain

Domain controls the sweep axis.

Accepted values:

* `time`
* `x`
* `thumb`
* `auto` (default)

Examples:

```text
// @viz(time)
// @viz(domain=x)
// @visualizer(domain=thumb)
```

### Kind

Kind controls the renderer selection.

Accepted aliases:

* `timeseries`, `chart`, `sparkline` -> timeseries renderer
* `swatch`, `color` -> color swatch renderer
* `thumbnail`, `preview`, `thumb` -> thumbnail renderer

Examples:

```text
// @viz(kind=timeseries)
// @visualizer(mode=swatch)
// @viz(thumbnail)
```

### Title

Title overrides the visualizer label.

```text
// @viz(title="carrier wave")
```

### Controls

Controls are parsed and displayed in metadata.

Separators accepted in one string:

* `|`
* `+`
* `/`
* whitespace

```text
// @visualizer(controls=time|range)
```

### Sweep Window Override

`window` and `span` explicitly set the sweep max value.

```text
// @viz(time, window=8s)
// @visualizer(domain=x, span=2.0)
```

## Auto Domain Behavior

When domain is omitted, the visualizer chooses automatically.

Auto picks `time` if the expression contains:

* literal `time`
* temporal named args like `period`, `cycle`, `frequency`, `freq`, `hz`,
  `rate`, `every`, `over`, `window`, `duration`, `span`, `horizon`

Otherwise auto defaults to `x`.

## Sweep Resolution Rules

Sweep is resolved in this order:

1. Explicit window or span option.
2. Builtin metadata and expression hints for temporal args.
3. Range span inference for x domain.
4. Annotation fallback value.
5. Domain default fallback.

Time-domain period behavior uses one period window.
If period is 2s, the time span is 2s.

## Renderer Selection Rules

Renderer path is selected using annotation kind and semantic type.

* Thumbnail renderer:
  * Domain `thumb`
  * Kind `thumbnail`
  * Fallback for non-scalar and non-color outputs
* Swatch renderer:
  * Color semantic type
  * Kind `swatch`
* Sparkline renderer:
  * Scalar and vec2 semantic types
  * Kind `timeseries` or `chart`

## Shader Inspector

Each visualizer zone can expose a shader button.
The dialog includes:

* Generated WGSL source
* Sweep provenance and resolved value
* Called builtin and metadata roles
* Render path and semantic type
* Source expression context

## Practical Examples

Time series:

```text
let speed = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)  // @viz
```

Fast waveform with explicit window:

```text
let carrier = wave(period: 120ms, shape: square, range: 0.0 .. 1.0)  // @viz(time, window=480ms)
```

Spatial sweep:

```text
let striped = wrap(x: uv.x * 14.0 + time * 0.4, range: 0.0 .. 1.0)  // @viz(x)
```

Color swatch:

```text
let tint = rgb(speed, 0.3, 1.0 - speed)  // @viz(kind=swatch, domain=time)
```

Shape thumbnail:

```text
let blob = circle(at: (speed, 0.5), radius: 0.1)  // @viz(thumb)
```

## Known Limits

* Visualizers are editor-only previews and do not change program semantics.
* Some spans can fail capture depending on expression shape; params include a
  fallback preview path.
* Auto domain is heuristic and can be overridden explicitly when needed.

## Demo File

For a richer sample set, see the showcase demo file at
examples/90) gallery/viz_demo.fr.
