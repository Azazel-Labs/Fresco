import { describe, expect, it, vi } from "vitest";

import { createBootstrapCompileWatchdog } from "../app/bootstrap-watchdog";

describe("bootstrap compile watchdog integration", () => {
  it("surfaces watchdog diagnostics when bootstrap queries are delayed", async () => {
    vi.useFakeTimers();
    const diagnostics = [];

    const watchdog = createBootstrapCompileWatchdog({
      timeoutMs: 100,
      onTimeout: (elapsedMs) => {
        diagnostics.push({
          file: "bootstrap",
          severity: "warning",
          message: `startup exceeded 100ms before first compile kickoff (${elapsedMs}ms elapsed)`,
        });
      },
    });

    const delayedBootstrapQuery = new Promise((resolve) => {
      setTimeout(() => resolve({ profile: "slow" }), 1000);
    });

    watchdog.arm();
    await vi.advanceTimersByTimeAsync(120);

    expect(diagnostics).toHaveLength(1);
    expect(diagnostics[0].file).toBe("bootstrap");
    expect(diagnostics[0].severity).toBe("warning");
    expect(diagnostics[0].message).toContain("before first compile kickoff");

    await vi.advanceTimersByTimeAsync(1000);
    await delayedBootstrapQuery;
    watchdog.clear();
    vi.useRealTimers();
  });
});
