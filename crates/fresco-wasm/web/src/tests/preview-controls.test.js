import { describe, expect, it } from "vitest";

import { normalizePreviewLoopSeconds, wrapPreviewTime } from "../preview/controls";

describe("preview loop helpers", () => {
  it("disables looping for non-positive or invalid durations", () => {
    expect(normalizePreviewLoopSeconds(0)).toBe(0);
    expect(normalizePreviewLoopSeconds(-4)).toBe(0);
    expect(normalizePreviewLoopSeconds(Number.NaN)).toBe(0);
  });

  it("keeps positive loop durations", () => {
    expect(normalizePreviewLoopSeconds(2.5)).toBe(2.5);
  });

  it("wraps positive playback time into the active loop window", () => {
    expect(wrapPreviewTime(5.75, 2)).toBeCloseTo(1.75, 6);
  });

  it("wraps negative playback time into the active loop window", () => {
    expect(wrapPreviewTime(-0.25, 2)).toBeCloseTo(1.75, 6);
  });

  it("leaves time untouched when looping is disabled", () => {
    expect(wrapPreviewTime(3.25, 0)).toBe(3.25);
  });
});
