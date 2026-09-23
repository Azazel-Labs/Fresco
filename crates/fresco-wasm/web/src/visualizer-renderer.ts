import {
  createPreviewUniformData,
  PREVIEW_UNIFORM_DT_INDEX,
  PREVIEW_UNIFORM_RES_INDEX,
  PREVIEW_UNIFORM_TIME_INDEX
} from "./preview/uniforms";

const VIZ_SERIES_SAMPLES = 256;
const gpu = globalThis as any;

type AxisMeta = {
  xAxisLabel?: string | null;
  yAxisLabel?: string | null;
  yMinHint?: number | null;
  yMaxHint?: number | null;
};

type TimeCursorRenderer = {
  elapsed?: number;
};

export class CompiledVisualizerRenderer {
  device: any;
  format: string;
  writeVizRuntimeParamUniforms: (uniform: Float32Array) => void;
  width: number = 300;
  height: number = 64;
  renderScale: number;
  _canvas: HTMLCanvasElement;
  _readback: HTMLCanvasElement;
  context: any;
  uniform: Float32Array;
  uniformBuffer: any;
  rangeBuffer: any;
  seriesBuffer: any;
  renderBindGroupLayout: any;
  computeBindGroupLayout: any;
  renderBindGroup: any;
  computeBindGroup: any;

  constructor(device: any, format: string, writeVizRuntimeParamUniforms: (uniform: Float32Array) => void) {
    this.device = device;
    this.format = format;
    this.writeVizRuntimeParamUniforms = typeof writeVizRuntimeParamUniforms === "function"
      ? writeVizRuntimeParamUniforms
      : () => {};
    this.renderScale = Math.max(1, Math.min(2, Number(window?.devicePixelRatio) || 1));

    this._canvas = document.createElement("canvas");
    this._readback = document.createElement("canvas");
    this.context = this._canvas.getContext("webgpu");
    this.context.configure({ device, format, alphaMode: "premultiplied" });

    this.uniform = createPreviewUniformData();
    this.uniformBuffer = device.createBuffer({
      size: this.uniform.byteLength,
      usage: gpu.GPUBufferUsage.UNIFORM | gpu.GPUBufferUsage.COPY_DST
    });
    this.rangeBuffer = device.createBuffer({
      size: 8,
      usage: gpu.GPUBufferUsage.STORAGE | gpu.GPUBufferUsage.COPY_DST
    });
    this.seriesBuffer = device.createBuffer({
      size: VIZ_SERIES_SAMPLES * 4,
      usage: gpu.GPUBufferUsage.STORAGE | gpu.GPUBufferUsage.COPY_DST
    });
    this.renderBindGroupLayout = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: gpu.GPUShaderStage.FRAGMENT, buffer: { type: "uniform" } },
        { binding: 1, visibility: gpu.GPUShaderStage.FRAGMENT, buffer: { type: "read-only-storage" } },
        { binding: 2, visibility: gpu.GPUShaderStage.FRAGMENT, buffer: { type: "read-only-storage" } }
      ]
    });
    this.computeBindGroupLayout = device.createBindGroupLayout({
      entries: [
        { binding: 0, visibility: gpu.GPUShaderStage.COMPUTE, buffer: { type: "uniform" } },
        { binding: 1, visibility: gpu.GPUShaderStage.COMPUTE, buffer: { type: "storage" } },
        { binding: 2, visibility: gpu.GPUShaderStage.COMPUTE, buffer: { type: "storage" } }
      ]
    });
    this.renderBindGroup = device.createBindGroup({
      layout: this.renderBindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: this.uniformBuffer } },
        { binding: 1, resource: { buffer: this.rangeBuffer } },
        { binding: 2, resource: { buffer: this.seriesBuffer } }
      ]
    });
    this.computeBindGroup = device.createBindGroup({
      layout: this.computeBindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: this.uniformBuffer } },
        { binding: 1, resource: { buffer: this.rangeBuffer } },
        { binding: 2, resource: { buffer: this.seriesBuffer } }
      ]
    });
    this.device.queue.writeBuffer(this.rangeBuffer, 0, new Float32Array([0, 1]));
    this.device.queue.writeBuffer(this.seriesBuffer, 0, new Float32Array(VIZ_SERIES_SAMPLES));
    this.ensureSize(this.width, this.height);
  }

  ensureSize(width: unknown, height: unknown) {
    const nextWidth = Math.max(120, Math.floor(Number(width) || this.width));
    const nextHeight = Math.max(24, Math.floor(Number(height) || this.height));
    if (nextWidth === this.width && nextHeight === this.height) {
      return;
    }

    this.width = nextWidth;
    this.height = nextHeight;
    const renderWidth = Math.max(1, Math.floor(this.width * this.renderScale));
    const renderHeight = Math.max(1, Math.floor(this.height * this.renderScale));
    this._canvas.width = renderWidth;
    this._canvas.height = renderHeight;
    this._readback.width = this.width;
    this._readback.height = this.height;
    this.context.configure({ device: this.device, format: this.format, alphaMode: "premultiplied" });
  }

  async renderShader(
    shaderCode: string | { drawWgsl?: unknown; reduceWgsl?: unknown; wgsl?: unknown },
    options: { width?: unknown; height?: unknown; clearColor?: unknown[] } = {}
  ) {
    this.ensureSize(options.width, options.height);
    const drawShaderCode = typeof shaderCode === "string"
      ? shaderCode
      : String(shaderCode?.drawWgsl || shaderCode?.wgsl || "");
    const reduceShaderCode = typeof shaderCode === "string"
      ? ""
      : String(shaderCode?.reduceWgsl || "");
    const module = this.device.createShaderModule({ code: drawShaderCode });
    const compilation = await module.getCompilationInfo();
    const errors = compilation.messages.filter((m: { type?: string; message?: string }) => m.type === "error");
    if (errors.length > 0) {
      throw new Error(`Visualizer shader compilation failed: ${errors[0].message}`);
    }

    const pipeline = await this.device.createRenderPipelineAsync({
      layout: this.device.createPipelineLayout({ bindGroupLayouts: [this.renderBindGroupLayout] }),
      vertex: { module, entryPoint: "vs" },
      fragment: { module, entryPoint: "fs", targets: [{ format: this.format }] },
      primitive: { topology: "triangle-list" }
    });

    let computePipeline = null;
    if (reduceShaderCode) {
      const reduceModule = this.device.createShaderModule({ code: reduceShaderCode });
      const reduceInfo = await reduceModule.getCompilationInfo();
      const reduceErrors = reduceInfo.messages.filter((m: { type?: string; message?: string }) => m.type === "error");
      if (reduceErrors.length > 0) {
        throw new Error(`Visualizer reduce shader compilation failed: ${reduceErrors[0].message}`);
      }
      computePipeline = await this.device.createComputePipelineAsync({
        layout: this.device.createPipelineLayout({ bindGroupLayouts: [this.computeBindGroupLayout] }),
        compute: { module: reduceModule, entryPoint: "cs_reduce" }
      });
    }

    this.uniform[PREVIEW_UNIFORM_TIME_INDEX] = 0;
    this.uniform[PREVIEW_UNIFORM_DT_INDEX] = 1 / 60;
    this.uniform[PREVIEW_UNIFORM_RES_INDEX] = this._canvas.width;
    this.uniform[PREVIEW_UNIFORM_RES_INDEX + 1] = this._canvas.height;
    this.writeVizRuntimeParamUniforms(this.uniform);
    this.device.queue.writeBuffer(this.uniformBuffer, 0, this.uniform);

    const clear = Array.isArray(options.clearColor) ? options.clearColor : [0.04, 0.08, 0.12, 1.0];
    const encoder = this.device.createCommandEncoder();
    if (computePipeline) {
      const computePass = encoder.beginComputePass();
      computePass.setPipeline(computePipeline);
      computePass.setBindGroup(0, this.computeBindGroup);
      computePass.dispatchWorkgroups(1, 1, 1);
      computePass.end();
    }
    const pass = encoder.beginRenderPass({
      colorAttachments: [{
        view: this.context.getCurrentTexture().createView(),
        clearValue: { r: clear[0] ?? 0, g: clear[1] ?? 0, b: clear[2] ?? 0, a: clear[3] ?? 1 },
        loadOp: "clear",
        storeOp: "store"
      }]
    });
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, this.renderBindGroup);
    pass.draw(3, 1, 0, 0);
    pass.end();
    this.device.queue.submit([encoder.finish()]);
    await this.device.queue.onSubmittedWorkDone();

    const bitmap = await createImageBitmap(this._canvas);
    const readbackCtx = this._readback.getContext("2d");
    if (!readbackCtx) {
      throw new Error("Visualizer readback context unavailable");
    }
    readbackCtx.drawImage(bitmap, 0, 0);
    return this._readback.toDataURL();
  }
}

export function addAxisOverlay(
  container: HTMLElement | null,
  canvas: HTMLCanvasElement,
  domain: string,
  sweepMax: number,
  showTimeCursor = false,
  axisMeta: AxisMeta | null = null,
  renderer: TimeCursorRenderer | null = null
) {
  const AXIS_FOOTER_PX = 20;

  const overlay = document.createElement("canvas");
  overlay.width = canvas.width;
  overlay.height = canvas.height + AXIS_FOOTER_PX;
  overlay.style.position = "absolute";
  overlay.style.top = "0";
  overlay.style.left = "0";
  overlay.style.width = canvas.style.width || `${canvas.width}px`;
  overlay.style.height = `calc(${canvas.style.height || `${canvas.height}px`} + ${AXIS_FOOTER_PX}px)`;
  overlay.style.pointerEvents = "none";

  const ctx = overlay.getContext("2d");
  if (!ctx) {
    return overlay;
  }
  const plotHeight = canvas.height;

  const drawAxis = () => {
    ctx.clearRect(0, 0, overlay.width, overlay.height);
    ctx.strokeStyle = "rgba(154, 200, 222, 0.3)";
    ctx.fillStyle = "rgba(154, 200, 222, 0.6)";
    ctx.font = '9px "IBM Plex Mono", monospace';
    ctx.textBaseline = "bottom";
    ctx.lineWidth = 1;

    const sweepLimit = Math.max(0.0001, Number(sweepMax) || (domain === "time" ? 4.0 : 1.0));
    const domainMax = domain === "time"
      ? `${sweepLimit.toFixed(sweepLimit < 1 ? 2 : 1)}s`
      : `${sweepLimit.toFixed(sweepLimit < 2 ? 2 : 1)}`;
    ctx.textAlign = "left";
    ctx.fillText("0", 2, overlay.height - 1);
    ctx.textAlign = "right";
    ctx.fillText(domainMax, overlay.width - 2, overlay.height - 1);

    ctx.beginPath();
    ctx.moveTo(0.5, 0);
    ctx.lineTo(0.5, plotHeight);
    ctx.stroke();

    ctx.beginPath();
    ctx.moveTo(0, plotHeight - 0.5);
    ctx.lineTo(overlay.width, plotHeight - 0.5);
    ctx.stroke();

    ctx.save();
    ctx.fillStyle = "rgba(154, 200, 222, 0.72)";
    ctx.font = '9px "IBM Plex Mono", monospace';
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    ctx.fillText(axisMeta?.yAxisLabel || "value", 4, 3);
    ctx.textAlign = "right";
    ctx.fillText(axisMeta?.xAxisLabel || domain, overlay.width - 4, 3);

    const minHint = Number(axisMeta?.yMinHint);
    const maxHint = Number(axisMeta?.yMaxHint);
    if (Number.isFinite(maxHint)) {
      ctx.textAlign = "left";
      ctx.textBaseline = "top";
      ctx.fillText(maxHint.toFixed(2), 4, 14);
    }
    if (Number.isFinite(minHint)) {
      ctx.textAlign = "left";
      ctx.textBaseline = "bottom";
      ctx.fillText(minHint.toFixed(2), 4, plotHeight - 2);
    }
    ctx.restore();
  };

  drawAxis();

  if (showTimeCursor && domain === "time" && renderer) {
    const drawTimeCursor = () => {
      drawAxis();
      const loopWindow = Math.max(0.0001, Number(sweepMax) || 4.0);
      const t = (renderer.elapsed || 0) % loopWindow;
      const x = (t / loopWindow) * overlay.width;

      ctx.strokeStyle = "rgba(45, 212, 191, 0.8)";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, plotHeight);
      ctx.stroke();

      requestAnimationFrame(drawTimeCursor);
    };
    requestAnimationFrame(drawTimeCursor);
  }

  if (container) {
    const wrapper = document.createElement("div");
    wrapper.style.position = "relative";
    wrapper.style.display = "inline-block";
    wrapper.style.paddingBottom = `${AXIS_FOOTER_PX}px`;
    wrapper.appendChild(canvas);
    wrapper.appendChild(overlay);
    return wrapper;
  }
  return overlay;
}
