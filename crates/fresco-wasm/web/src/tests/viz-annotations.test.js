import { describe, expect, it } from "vitest";
import { VizAnnotationScanner, removeVizDirectiveFromLine } from "../viz-annotations";

function modelFromSource(source) {
  return {
    getValue() {
      return source;
    }
  };
}

function utf8SliceByByteRange(source, start, end) {
  const bytes = new TextEncoder().encode(source);
  return new TextDecoder().decode(bytes.slice(start, end));
}

describe("VizAnnotationScanner", () => {
  it("parses leading visualizer directives and trailing legacy annotations", () => {
    const source = [
      "// @visualizer(kind=timeseries, domain=time, title=\"speed\")",
      "let speed = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)",
      "let pulse = wave(period: 3s, shape: square, range: 0.0 .. 1.0)  // @viz(time)",
      "// @visualizer(kind=timeseries, domain=time, title=\"threshold\", controls=time|range)",
      "param threshold: f32 = 0.5 in 0 .. 1"
    ].join("\n");

    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(3);
    expect(annotations[0]).toMatchObject({
      name: "speed",
      title: "speed",
      kind: "timeseries",
      domain: "time",
      directivePlacement: "leading"
    });
    expect(annotations[1]).toMatchObject({
      name: "pulse",
      domain: "time",
      directivePlacement: "trailing"
    });
    expect(annotations[2]).toMatchObject({
      name: "threshold",
      kind: "timeseries",
      domain: "time",
      controls: ["time", "range"]
    });
  });

  it("parses directives placed on the line below a statement", () => {
    const source = [
      "let speed = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)",
      "// @viz(time, title=\"speed_below\")"
    ].join("\n");

    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0]).toMatchObject({
      name: "speed_below",
      title: "speed_below",
      domain: "time",
      directivePlacement: "below",
      lineNumber: 1,
      directiveLineNumber: 2
    });
  });

  it("keeps stable annotation keys when unrelated lines are inserted above", () => {
    const base = [
      "let speed = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)",
      "// @viz(time, title=\"speed\")",
      "let pulse = wave(period: 3s, shape: square, range: 0.0 .. 1.0)",
      "// @viz(time, title=\"pulse\")"
    ].join("\n");

    const shifted = [
      "let unrelated = 42",
      "let another = unrelated + 1",
      base
    ].join("\n");

    const scanner = new VizAnnotationScanner();
    const before = scanner.scan(modelFromSource(base));
    const after = scanner.scan(modelFromSource(shifted));

    expect(before).toHaveLength(2);
    expect(after).toHaveLength(2);
    expect(before.map((ann) => ann.stableKey)).toEqual(after.map((ann) => ann.stableKey));
  });

  it("removes trailing visualizer directives without disturbing code", () => {
    expect(removeVizDirectiveFromLine("let speed = wave()  // @visualizer(kind=timeseries)")).toBe("let speed = wave()");
  });

  it("targets param identifier span for param annotations", () => {
    const source = "param threshold: f32 = 0.5 in 0 .. 1  // @viz(time)";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    const [annotation] = annotations;
    expect(annotation.name).toBe("threshold");
    expect(utf8SliceByByteRange(source, annotation.spanStart, annotation.spanEnd)).toBe("threshold");
  });

  it("targets space declaration expression spans", () => {
    const source = "space stage = centered(aspect: preserve)  // @viz()";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    const [annotation] = annotations;
    expect(annotation.name).toBe("stage");
    expect(utf8SliceByByteRange(source, annotation.spanStart, annotation.spanEnd)).toBe("centered(aspect: preserve)");
  });

  it("infers time sweep window from wave period", () => {
    const source = "let pulse = wave(period: 1.5s, shape: sine, range: 0.0 .. 1.0)  // @viz(time)";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0].domain).toBe("time");
    expect(annotations[0].sweepMax).toBeCloseTo(1.5, 5);
  });

  it("infers time sweep window from generic period labels in non-wave calls", () => {
    const source = "let lfo = oscillator(period: 0.25s, phase: time)  // @viz(time)";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0].sweepMax).toBeCloseTo(0.25, 5);
  });

  it("infers time sweep window from frequency labels", () => {
    const source = "let lfo = oscillator(freq: 2hz, phase: time)  // @viz(time)";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0].sweepMax).toBeCloseTo(0.5, 5);
  });

  it("infers x sweep window from range literals", () => {
    const source = "let ramp = remap(x: uv.x, range: -2.0 .. 3.0)  // @viz(x)";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0].domain).toBe("x");
    expect(annotations[0].sweepMax).toBeCloseTo(5.0, 5);
  });

  it("captures sweep hint expressions for runtime resolution", () => {
    const source = "let lfo = oscillator(period: tempo * 0.5, range: -width .. width)  // @viz(time)";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0].sweepHints).toMatchObject({
      periodExpr: "tempo * 0.5",
      rangeStartExpr: "-width",
      rangeEndExpr: "width"
    });
  });

  it("honors explicit viz window override", () => {
    const source = "let pulse = wave(period: 1.5s, shape: sine, range: 0.0 .. 1.0)  // @viz(time, window=8s)";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0].sweepMax).toBeCloseTo(8.0, 5);
  });

  it("auto-detects time domain for wave period annotations without explicit time token", () => {
    const source = "let speed = wave(period: 2s, shape: sine, range: 0.2 .. 0.8)  // @viz";
    const scanner = new VizAnnotationScanner();
    const annotations = scanner.scan(modelFromSource(source));

    expect(annotations).toHaveLength(1);
    expect(annotations[0].domain).toBe("time");
    expect(annotations[0].sweepMax).toBeCloseTo(2.0, 5);
  });
});
