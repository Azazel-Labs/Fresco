type QueueCompileFn = (request?: {
  delayMs?: number;
  cancelInFlight?: boolean;
  deferPreviewBuild?: boolean;
}) => void;

type PreviewControls = {
  setPreviewPaused(nextPaused: boolean): void;
  stepPreviewFrames(delta: number): void;
  applyPreviewTimeInput(options?: { force?: boolean }): void;
  syncPreviewTimeReadout(): void;
  syncPreviewPlayButtonState(): void;
  syncPreviewFullscreenButtonState(): void;
  onPreviewHotkey(event: KeyboardEvent): void;
  onPreviewFullscreenChange(event: Event): void;
  togglePreviewFullscreen(): Promise<void>;
};

type RendererLike = {
  stagedShader: unknown;
  maybeBuildStagedShader(): Promise<void>;
  drawFrame(): boolean;
  setPlaybackRate(rate: number): void;
  setSurfaceOrbitAuto(enabled: boolean): void;
  handleResize(): void;
  reportRuntimeIssue(message: string, context: string): void;
};

type SourceEditorLike = {
  onDidChangeModelContent(listener: () => void): void;
};

export type BootstrapEventBindings = {
  scheduleParamsLayoutRefresh(): void;
  exampleSelectEl: HTMLElement;
  onExampleSelected(): void;
  docsToggleEl: HTMLElement | null;
  toggleDocsOverlay(): void;
  previewControls: PreviewControls;
  previewPlayButtonEl: HTMLElement | null;
  previewSpinButtonEl: HTMLElement | null;
  previewRateSelectEl: HTMLSelectElement | null;
  previewStepBackButtonEl: HTMLElement | null;
  previewStepForwardButtonEl: HTMLElement | null;
  previewTimeInputEl: HTMLElement | null;
  previewFullscreenButtonEl: HTMLElement | null;
  renderer: RendererLike;
  getPreviewPaused(): boolean;
  sourceEditor: SourceEditorLike;
  onSourceChanged(): void;
  queueCompile: QueueCompileFn;
};

export function bindBootstrapEventHandlers(deps: BootstrapEventBindings): void {
  window.addEventListener("resize", deps.scheduleParamsLayoutRefresh, { passive: true });
  deps.exampleSelectEl.addEventListener("change", deps.onExampleSelected);
  deps.docsToggleEl?.addEventListener("click", () => {
    deps.toggleDocsOverlay();
  });
  deps.previewPlayButtonEl?.addEventListener("click", () => {
    deps.previewControls.setPreviewPaused(!deps.getPreviewPaused());
  });
  deps.previewSpinButtonEl?.addEventListener("click", () => {
    deps.renderer.setSurfaceOrbitAuto(true);
    deps.renderer.drawFrame();
  });
  deps.previewRateSelectEl?.addEventListener("change", () => {
    const nextRate = Number(deps.previewRateSelectEl?.value);
    deps.renderer.setPlaybackRate(nextRate);
  });
  deps.previewStepBackButtonEl?.addEventListener("click", () => {
    deps.previewControls.stepPreviewFrames(-1);
  });
  deps.previewStepForwardButtonEl?.addEventListener("click", () => {
    deps.previewControls.stepPreviewFrames(1);
  });
  deps.previewTimeInputEl?.addEventListener("focus", () => {
    if (!deps.getPreviewPaused()) {
      deps.previewControls.setPreviewPaused(true);
    }
  });
  deps.previewTimeInputEl?.addEventListener("input", () => {
    deps.previewControls.applyPreviewTimeInput();
  });
  deps.previewTimeInputEl?.addEventListener("keydown", (event: KeyboardEvent) => {
    if (event.key === "Enter") {
      deps.previewControls.applyPreviewTimeInput({ force: true });
    }
    if (event.key === "Escape") {
      deps.previewControls.syncPreviewTimeReadout();
    }
  });
  deps.previewTimeInputEl?.addEventListener("blur", () => {
    deps.previewControls.applyPreviewTimeInput({ force: true });
  });
  document.addEventListener("keydown", deps.previewControls.onPreviewHotkey);
  document.addEventListener("fullscreenchange", deps.previewControls.onPreviewFullscreenChange);
  deps.previewFullscreenButtonEl?.addEventListener("click", async () => {
    try {
      await deps.previewControls.togglePreviewFullscreen();
      deps.renderer.handleResize();
    } catch (err: any) {
      deps.renderer.reportRuntimeIssue(err?.message || err, "fullscreen toggle");
    }
  });

  deps.sourceEditor.onDidChangeModelContent(() => {
    deps.onSourceChanged();
    deps.queueCompile({ cancelInFlight: true });
  });
}

export type PreviewFrameLoopDeps = {
  renderer: RendererLike;
  previewControls: PreviewControls;
  getPreviewPaused(): boolean;
  isPreviewPanelActive(): boolean;
  getPreviewBuildDeferred(): boolean;
  previewFpsEl: HTMLElement | null;
};

export function startPreviewFrameLoop(deps: PreviewFrameLoopDeps): void {
  const frame = () => {
    const previewActive = deps.isPreviewPanelActive();
    if (previewActive && deps.renderer.stagedShader && !deps.getPreviewBuildDeferred()) {
      void deps.renderer.maybeBuildStagedShader().catch((err: any) => {
        deps.renderer.reportRuntimeIssue(err?.message || err, "deferred preview shader build");
      });
    }
    if (previewActive && !deps.getPreviewPaused()) {
      const drewFrame = deps.renderer.drawFrame();
      if (!drewFrame && deps.previewFpsEl) {
        deps.previewFpsEl.textContent = "FPS --";
      }
    }
    deps.previewControls.syncPreviewTimeReadout();
    requestAnimationFrame(frame);
  };

  requestAnimationFrame(frame);
}
