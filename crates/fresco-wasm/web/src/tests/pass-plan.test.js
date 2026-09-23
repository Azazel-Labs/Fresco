import { describe, expect, it } from "vitest";

import {
  isMultiPass,
  parseManifestPassPlan,
  passLabel,
  passPlanSummary,
  shouldUsePreviewSinglePassFallback,
} from "../pass-plan";

// ---------------------------------------------------------------------------
// Fixture manifests
// ---------------------------------------------------------------------------

const SINGLE_PASS_MANIFEST = {
  canvases: [{
    name: "sunset",
    params: [],
    pass_plan: {
      passes: [
        { id: 0, stage: 0, locality: "point", start_layer: 0, end_layer: 6,
          count: 7, kernel_strategy: "fused", kernel_radius_px: null,
          entry_point: "fresco_sunset", inputs: [],
          kernel_levels: null, output_target: null }
      ],
      edges: [],
      targets: [],
    }
  }]
};

const INLINE_TAPS_MANIFEST = {
  canvases: [{
    name: "blur_badge",
    params: [],
    pass_plan: {
      passes: [
        { id: 0, stage: 0, locality: "local", start_layer: 0, end_layer: 1,
          count: 2, kernel_strategy: "inline-taps", kernel_radius_px: 3.0,
          entry_point: "fresco_blur_badge_pass0_", inputs: [],
          kernel_levels: null, output_target: 0 },
        { id: 1, stage: 1, locality: "point", start_layer: 2, end_layer: 3,
          count: 2, kernel_strategy: "fused", kernel_radius_px: null,
          entry_point: "fresco_blur_badge_pass1_", inputs: [{ from_pass: 0, target_id: 0, binding: 2 }],
          kernel_levels: null, output_target: null }
      ],
      edges: [{ from: 0, to: 1, reason: "locality-boundary" }],
      targets: [
        { id: 0, format: "rgba16float", scale: 1.0, lifetime: "transient" }
      ],
    }
  }]
};

const SEPARABLE_HV_MANIFEST = {
  canvases: [{
    name: "hazy",
    params: [],
    pass_plan: {
      passes: [
        { id: 0, stage: 0, locality: "local", start_layer: 0, end_layer: 1,
          count: 2, kernel_strategy: "separable-hv", kernel_radius_px: 20.0,
          entry_point: "fresco_hazy_pass0_", inputs: [],
          kernel_levels: null, output_target: 0 },
        { id: 1, stage: 1, locality: "point", start_layer: 2, end_layer: 2,
          count: 1, kernel_strategy: "fused", kernel_radius_px: null,
          entry_point: "fresco_hazy_pass1_", inputs: [{ from_pass: 0, target_id: 0, binding: 2 }],
          kernel_levels: null, output_target: null }
      ],
      edges: [{ from: 0, to: 1, reason: "locality-boundary" }],
      targets: [
        { id: 0, format: "rgba16float", scale: 1.0, lifetime: "transient" }
      ],
    }
  }]
};

const DOWNSAMPLE_MANIFEST = {
  canvases: [{
    name: "bloom",
    params: [],
    pass_plan: {
      passes: [
        { id: 0, stage: 0, locality: "local", start_layer: 0, end_layer: 0,
          count: 1, kernel_strategy: "downsample-chain", kernel_radius_px: 48.0,
          entry_point: "fresco_bloom_pass0_", inputs: [],
          kernel_levels: 3, output_target: 0 },
        { id: 1, stage: 1, locality: "point", start_layer: 1, end_layer: 1,
          count: 1, kernel_strategy: "fused", kernel_radius_px: null,
          entry_point: "fresco_bloom_pass1_", inputs: [{ from_pass: 0, target_id: 0, binding: 2 }],
          kernel_levels: null, output_target: null }
      ],
      edges: [{ from: 0, to: 1, reason: "locality-boundary" }],
      targets: [
        { id: 0, format: "rgba16float", scale: 1.0, lifetime: "transient" }
      ],
    }
  }]
};

// ---------------------------------------------------------------------------
// parseManifestPassPlan
// ---------------------------------------------------------------------------

describe("parseManifestPassPlan", () => {
  it("returns null for empty input", () => {
    expect(parseManifestPassPlan("", "fresco_foo")).toBeNull();
    expect(parseManifestPassPlan(null, "fresco_foo")).toBeNull();
    expect(parseManifestPassPlan("not-json", "fresco_foo")).toBeNull();
  });

  it("returns null when pass_plan is absent", () => {
    const manifest = { canvases: [{ name: "foo", params: [] }] };
    expect(parseManifestPassPlan(manifest, "fresco_foo")).toBeNull();
  });

  it("parses a single-pass fused plan by canvas name (strips fresco_ prefix)", () => {
    const plan = parseManifestPassPlan(SINGLE_PASS_MANIFEST, "fresco_sunset");
    expect(plan).not.toBeNull();
    expect(plan.passes).toHaveLength(1);
    expect(plan.passes[0].kernel_strategy).toBe("fused");
    expect(plan.edges).toHaveLength(0);
    expect(plan.targets).toHaveLength(0);
  });

  it("falls back to the first canvas when name does not match", () => {
    const plan = parseManifestPassPlan(SINGLE_PASS_MANIFEST, "fresco_unknown");
    expect(plan).not.toBeNull();
    expect(plan.passes).toHaveLength(1);
  });

  it("parses a two-pass inline-taps plan with target and edge", () => {
    const plan = parseManifestPassPlan(INLINE_TAPS_MANIFEST, "fresco_blur_badge");
    expect(plan.passes).toHaveLength(2);
    expect(plan.passes[0].kernel_strategy).toBe("inline-taps");
    expect(plan.passes[0].entry_point).toBe("fresco_blur_badge_pass0_");
    expect(plan.passes[0].kernel_radius_px).toBeCloseTo(3.0);
    expect(plan.passes[0].output_target).toBe(0);
    expect(plan.passes[1].kernel_strategy).toBe("fused");
    expect(plan.passes[1].inputs).toEqual([{ from_pass: 0, target_id: 0, binding: 2 }]);
    expect(plan.passes[1].output_target).toBeNull();
    expect(plan.edges).toHaveLength(1);
    expect(plan.edges[0]).toEqual({ from: 0, to: 1, reason: "locality-boundary" });
    expect(plan.targets).toHaveLength(1);
    expect(plan.targets[0].format).toBe("rgba16float");
    expect(plan.targets[0].lifetime).toBe("transient");
  });
});

// ---------------------------------------------------------------------------
// isMultiPass
// ---------------------------------------------------------------------------

describe("isMultiPass", () => {
  it("returns false for null / missing plan", () => {
    expect(isMultiPass(null)).toBe(false);
    expect(isMultiPass(undefined)).toBe(false);
    expect(isMultiPass({ passes: [] })).toBe(false);
  });

  it("returns false for a single-pass fused plan", () => {
    const plan = parseManifestPassPlan(SINGLE_PASS_MANIFEST, "fresco_sunset");
    expect(isMultiPass(plan)).toBe(false);
  });

  it("returns true for a two-pass plan with inline-taps", () => {
    const plan = parseManifestPassPlan(INLINE_TAPS_MANIFEST, "fresco_blur_badge");
    expect(isMultiPass(plan)).toBe(true);
  });

  it("returns true for separable-hv plan", () => {
    const plan = parseManifestPassPlan(SEPARABLE_HV_MANIFEST, "fresco_hazy");
    expect(isMultiPass(plan)).toBe(true);
  });

  it("returns true for downsample-chain plan", () => {
    const plan = parseManifestPassPlan(DOWNSAMPLE_MANIFEST, "fresco_bloom");
    expect(isMultiPass(plan)).toBe(true);
  });
});

describe("shouldUsePreviewSinglePassFallback", () => {
  it("returns false for null plan", () => {
    expect(shouldUsePreviewSinglePassFallback(null)).toBe(false);
  });

  it("returns false for a fused single-pass plan", () => {
    const plan = parseManifestPassPlan(SINGLE_PASS_MANIFEST, "fresco_sunset");
    expect(shouldUsePreviewSinglePassFallback(plan)).toBe(false);
  });

  it("returns false for multi-pass plans when compiler emitted pass entry points", () => {
    const plan = parseManifestPassPlan(INLINE_TAPS_MANIFEST, "fresco_blur_badge");
    expect(shouldUsePreviewSinglePassFallback(plan)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// passLabel
// ---------------------------------------------------------------------------

describe("passLabel", () => {
  it("labels fused passes", () => {
    expect(passLabel({ kernel_strategy: "fused" })).toBe("fused");
  });

  it("labels inline-taps with radius", () => {
    expect(passLabel({ kernel_strategy: "inline-taps", kernel_radius_px: 3.0 }))
      .toBe("inline-taps(3px)");
  });

  it("labels separable-hv with radius", () => {
    expect(passLabel({ kernel_strategy: "separable-hv", kernel_radius_px: 20.5 }))
      .toBe("separable-hv(21px)");
  });

  it("labels downsample-chain with radius and levels", () => {
    expect(passLabel({ kernel_strategy: "downsample-chain", kernel_radius_px: 48.0, kernel_levels: 3 }))
      .toBe("downsample-chain(48px 3lvl)");
  });

  it("labels global-reduction", () => {
    expect(passLabel({ kernel_strategy: "global-reduction" })).toBe("global-reduction");
  });

  it("falls back gracefully for unknown strategies", () => {
    expect(passLabel({ kernel_strategy: "future-strategy" })).toBe("future-strategy");
  });
});

// ---------------------------------------------------------------------------
// passPlanSummary
// ---------------------------------------------------------------------------

describe("passPlanSummary", () => {
  it("returns empty string for null plan", () => {
    expect(passPlanSummary(null)).toBe("");
    expect(passPlanSummary({ passes: [] })).toBe("");
  });

  it("formats a single-pass plan", () => {
    const plan = parseManifestPassPlan(SINGLE_PASS_MANIFEST, "fresco_sunset");
    expect(passPlanSummary(plan)).toBe("1 pass (fused)");
  });

  it("formats a two-pass inline-taps plan", () => {
    const plan = parseManifestPassPlan(INLINE_TAPS_MANIFEST, "fresco_blur_badge");
    // Passes sorted by stage: inline-taps(3px) → fused
    expect(passPlanSummary(plan)).toBe("2 passes: inline-taps(3px) \u2192 fused");
  });

  it("formats a two-pass separable-hv plan", () => {
    const plan = parseManifestPassPlan(SEPARABLE_HV_MANIFEST, "fresco_hazy");
    expect(passPlanSummary(plan)).toBe("2 passes: separable-hv(20px) \u2192 fused");
  });

  it("formats a two-pass downsample-chain plan", () => {
    const plan = parseManifestPassPlan(DOWNSAMPLE_MANIFEST, "fresco_bloom");
    expect(passPlanSummary(plan)).toBe("2 passes: downsample-chain(48px 3lvl) \u2192 fused");
  });
});
