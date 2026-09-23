// Editor visualizers own a separate device from the Rust example engine.
// No preview surface, material resources, or engine pipelines are created here.
export async function createVisualizerDevice(gpu: any, reportError: (message: string) => void) {
  if (!gpu) throw new Error("WebGPU is unavailable for editor visualizers.");
  const adapter = await gpu.requestAdapter();
  if (!adapter) throw new Error("No GPU adapter is available for editor visualizers.");
  const device = await adapter.requestDevice({ label: "Fresco editor visualizers" });
  let disposed = false;
  const onError = (event: any) => {
    if (!disposed) reportError(String(event.error?.message || event.error));
    event.preventDefault?.();
  };
  device.addEventListener("uncapturederror", onError);
  void device.lost.then(info => {
    if (!disposed) reportError(`Editor visualizer GPU device lost: ${info.message || info.reason}`);
  });
  return {
    device,
    format: gpu.getPreferredCanvasFormat(),
    dispose() {
      if (disposed) return;
      disposed = true;
      device.removeEventListener("uncapturederror", onError);
      device.destroy();
    },
  };
}
