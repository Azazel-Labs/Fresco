import { afterEach, describe, expect, it, vi } from "vitest";
import { ExampleEngineRenderer } from "../preview/example-engine-renderer";

vi.mock("../preview/example-engine-module", () => ({
  loadExampleEngine: async () => ({
    BrowserCamera: class { drag = vi.fn(); zoom = vi.fn(); },
    BrowserEngine: { create: async () => ({ set_camera: vi.fn() }) },
  }),
}));
afterEach(() => vi.unstubAllGlobals());

async function createRenderer() {
  vi.stubGlobal("location", { search: "" });
  vi.stubGlobal("ResizeObserver", class { observe() {} });
  vi.stubGlobal("addEventListener", vi.fn());
  const listeners = new Map();
  const canvas = {
    style: {}, clientHeight: 600,
    setPointerCapture: vi.fn(),
    addEventListener: (type, listener) => listeners.set(type, listener),
  };
  const renderer = new ExampleEngineRenderer(canvas);
  renderer.handleResize = vi.fn();
  renderer.drawFrame = vi.fn();
  renderer.onOrbitAutoChanged = vi.fn();
  await renderer.init();
  return { renderer, dispatch: (type, event) => listeners.get(type)(event) };
}

describe("shared engine orbit controls", () => {
  it.each([true, false])("wheel zoom preserves auto spin = %s", async (autoSpin) => {
    const { renderer, dispatch } = await createRenderer();
    renderer.setOrbitAuto(autoSpin);
    renderer.onOrbitAutoChanged.mockClear();
    const preventDefault = vi.fn();
    dispatch("wheel", { deltaY: -120, preventDefault });
    expect(renderer.camera.zoom).toHaveBeenLastCalledWith(-120);
    expect(renderer.getOrbitAuto()).toBe(autoSpin);
    dispatch("wheel", { deltaY: 120, preventDefault });
    expect(renderer.camera.zoom).toHaveBeenLastCalledWith(120);
    expect(renderer.getOrbitAuto()).toBe(autoSpin);
    expect(renderer.onOrbitAutoChanged).not.toHaveBeenCalled();
    expect(preventDefault).toHaveBeenCalledTimes(2);
    expect(renderer.drawFrame).toHaveBeenCalledTimes(2);
  });
  it("manual orbit dragging disables auto spin and forwards movement to Rust", async () => {
    const { renderer, dispatch } = await createRenderer();
    renderer.setOrbitAuto(true);
    renderer.onOrbitAutoChanged.mockClear();
    dispatch("pointerdown", { button: 0, pointerId: 1, clientX: 0, clientY: 0 });
    dispatch("pointermove", { pointerId: 1, clientX: 10, clientY: 0 });
    expect(renderer.camera.drag).toHaveBeenCalledWith(10, 0);
    expect(renderer.getOrbitAuto()).toBe(false);
    expect(renderer.onOrbitAutoChanged).toHaveBeenCalledOnce();
    expect(renderer.engine.set_camera).toHaveBeenLastCalledWith(renderer.camera);
  });
});
