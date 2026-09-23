import { describe, expect, it } from "vitest";

import { isRetryableWorkerFailure } from "../worker-retry";

describe("worker retry classification", () => {
  it("retries transient worker bootstrap and reload failures", () => {
    expect(isRetryableWorkerFailure(new Error("compile worker crashed"))).toBe(true);
    expect(isRetryableWorkerFailure(new Error("query worker message decode failed"))).toBe(true);
    expect(isRetryableWorkerFailure(new Error("Failed to fetch dynamically imported module"))).toBe(true);
    expect(isRetryableWorkerFailure(new Error("Importing a module script failed."))).toBe(true);
  });

  it("does not retry ordinary compiler diagnostics", () => {
    expect(isRetryableWorkerFailure(new Error("unsupported visualizer kind: heatmap"))).toBe(false);
    expect(isRetryableWorkerFailure(new Error("expected ')' after expression"))).toBe(false);
    expect(isRetryableWorkerFailure("")).toBe(false);
  });
});