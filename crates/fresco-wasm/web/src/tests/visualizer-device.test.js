import { describe, expect, it, vi } from "vitest";
import { createVisualizerDevice } from "../preview/visualizer-device";

describe("editor visualizer device ownership", () => {
  it("reports live errors and disposes once without reporting intentional loss", async () => {
    let lose;
    const device = { lost: new Promise(resolve => { lose = resolve; }),
      addEventListener: vi.fn(), removeEventListener: vi.fn(), destroy: vi.fn() };
    const requestDevice = vi.fn().mockResolvedValue(device);
    const report = vi.fn();
    const resources = await createVisualizerDevice({
      requestAdapter: async () => ({ requestDevice }), getPreferredCanvasFormat: () => "bgra8unorm",
    }, report);
    expect(requestDevice).toHaveBeenCalledWith({ label: "Fresco editor visualizers" });
    const onError = device.addEventListener.mock.calls[0][1];
    const preventDefault = vi.fn();
    onError({ error: Error("bad visualizer"), preventDefault });
    expect(report).toHaveBeenCalledWith("bad visualizer");
    expect(preventDefault).toHaveBeenCalledOnce();
    resources.dispose(); resources.dispose();
    lose({ reason: "destroyed" }); await Promise.resolve();
    expect(device.destroy).toHaveBeenCalledOnce();
    expect(device.removeEventListener).toHaveBeenCalledWith("uncapturederror", onError);
    expect(report).toHaveBeenCalledOnce();
  });
  it("returns setup failures to the visualizer caller", async () => {
    await expect(createVisualizerDevice(null, vi.fn())).rejects.toThrow("unavailable");
    await expect(createVisualizerDevice({ requestAdapter: async () => null }, vi.fn()))
      .rejects.toThrow("No GPU adapter");
  });
});
