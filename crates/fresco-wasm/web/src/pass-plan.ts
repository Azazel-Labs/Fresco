/**
 * Manifest pass-plan helpers.
 *
 * These are pure functions that operate on the pass_plan section of the
 * Rust-owned Fresco manifest object. They have no dependency on WebGPU or DOM APIs so
 * they can be tested in Node/Vitest without a browser.
 *
 * See LANGUAGE.md (Host integration obligations) for execution limits and the
 * artifact types for the current manifest contract.
 */

import type { ManifestRoot } from "./generated/wasm-contracts/ManifestRoot";
import type { ManifestPass } from "./generated/wasm-contracts/ManifestPass";
import type { ManifestEdge } from "./generated/wasm-contracts/ManifestEdge";
import type { ManifestIntermediateTarget } from "./generated/wasm-contracts/ManifestIntermediateTarget";

export type PassEntry = ManifestPass;

export type PassPlan = {
  passes: PassEntry[];
  edges: ManifestEdge[];
  targets: ManifestIntermediateTarget[];
};

function canvasNameFromEntryFunction(fnName: string | null | undefined): string {
  return String(fnName || "")
    .replace(/^fresco_/, "")
    .replace(/_pass\d+$/, "");
}

/**
 * Select the pass_plan section from a manifest object for a given canvas.
 *
 * @param manifest - The full manifest object from wasm compile result.
 * @param fnName - The WGSL entry function name (e.g. "fresco_foo").
 * @returns The pass plan object, or null if the manifest/canvas data is absent.
 */
export function parseManifestPassPlan(manifest: ManifestRoot | null | undefined, fnName: string | null | undefined): PassPlan | null {
  if (!manifest || typeof manifest !== "object") {
    return null;
  }

  const canvasName = canvasNameFromEntryFunction(fnName);
  const canvases = Array.isArray(manifest.canvases) ? manifest.canvases : [];
  const canvas = canvases.find((entry) => entry.name === canvasName) || canvases[0];
  const plan = canvas?.pass_plan;
  if (!plan || !Array.isArray(plan.passes)) {
    return null;
  }
  return {
    passes: plan.passes,
    edges: Array.isArray(plan.edges) ? plan.edges : [],
    targets: Array.isArray(plan.targets) ? plan.targets : [],
  };
}

/**
 * Return true when the pass plan requires more than one render pass.
 *
 * A pass plan is considered multi-pass when it has more than one pass entry
 * OR when any pass uses a non-fused kernel strategy (inline-taps is currently
 * folded into a single WGSL function; separable-hv and downsample-chain will
 * require separate entry points once the compiler emits them).
 */
export function isMultiPass(passPlan: { passes: PassEntry[] } | null): boolean {
  if (!passPlan || !Array.isArray(passPlan.passes) || passPlan.passes.length <= 1) {
    return false;
  }
  return passPlan.passes.some((pass) => pass.kernel_strategy !== "fused");
}

/**
 * Return true when the current web preview must ignore the manifest pass DAG
 * and render through the single-pass fallback path.
 *
 * Fallback remains necessary when multi-pass metadata is incomplete. The
 * preview requires an `entry_point` for every pass before it can build the
 * corresponding pipelines safely.
 */
export function shouldUsePreviewSinglePassFallback(passPlan: { passes: PassEntry[]; targets?: ManifestIntermediateTarget[] } | null): boolean {
  if (!passPlan) {
    return false;
  }
  if (!isMultiPass(passPlan)) {
    return false;
  }
  return !passPlan.passes.every((pass) => typeof pass?.entry_point === "string" && pass.entry_point.length > 0);
}

/**
 * Return a short human-readable summary of the pass plan suitable for
 * embedding in the preview status line.
 *
 * Examples:
 *   "1 pass (fused)"
 *   "2 passes: local/inline-taps(3px) → fused"
 *   "2 passes: local/separable-hv(20px) → fused"
 */
export function passPlanSummary(passPlan: { passes: PassEntry[] } | null): string {
  if (!passPlan || !Array.isArray(passPlan.passes) || passPlan.passes.length === 0) {
    return "";
  }

  const count = passPlan.passes.length;
  if (count === 1) {
    const pass = passPlan.passes[0];
    return `1 pass (${passLabel(pass)})`;
  }

  const sorted = [...passPlan.passes].sort((a, b) => (a.stage || 0) - (b.stage || 0));
  const labels = sorted.map((pass) => passLabel(pass));
  return `${count} passes: ${labels.join(" \u2192 ")}`;
}

/**
 * Return a concise label for a single pass, e.g. "fused", "inline-taps(3px)".
 */
export function passLabel(pass: PassEntry | null | undefined): string {
  const strategy = String(pass?.kernel_strategy || "fused");
  const radius = typeof pass?.kernel_radius_px === "number" ? pass.kernel_radius_px : null;
  const levels = typeof pass?.kernel_levels === "number" ? pass.kernel_levels : null;

  if (strategy === "fused") {
    return "fused";
  }
  if (strategy === "inline-taps" && radius !== null) {
    return `inline-taps(${radius.toFixed(0)}px)`;
  }
  if (strategy === "separable-hv" && radius !== null) {
    return `separable-hv(${radius.toFixed(0)}px)`;
  }
  if (strategy === "downsample-chain" && radius !== null) {
    const suffix = levels !== null ? ` ${levels}lvl` : "";
    return `downsample-chain(${radius.toFixed(0)}px${suffix})`;
  }
  if (strategy === "global-reduction") {
    return "global-reduction";
  }
  return strategy;
}
