const PLAY_ICON_SVG =
  '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 3.8L12 8L5 12.2Z" fill="currentColor" stroke="none"/></svg>';
const PAUSE_ICON_SVG =
  '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 3.5v9"/><path d="M11 3.5v9"/></svg>';
const FULLSCREEN_ENTER_ICON_SVG =
  '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6V2.5H6"/><path d="M10 2.5h3.5V6"/><path d="M13.5 10v3.5H10"/><path d="M6 13.5H2.5V10"/></svg>';
const FULLSCREEN_EXIT_ICON_SVG =
  '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6.5 2.5H2.5v4"/><path d="M9.5 2.5h4v4"/><path d="M2.5 9.5v4h4"/><path d="M13.5 9.5v4h-4"/><path d="M6.5 6.5L2.5 2.5"/><path d="M9.5 6.5l4-4"/><path d="M6.5 9.5l-4 4"/><path d="M9.5 9.5l4 4"/></svg>';

type PreviewPlayback = {
  simTimeSeconds: number;
  fpsFrameCount: number;
  fpsSampleStartMs: number;
  setPlaybackPaused(paused: boolean): void;
  setPlaybackTime(seconds: number): void;
  stepPlaybackFrames(frames: number, fps: number): void;
  drawFrame(options: { advanceTime: boolean }): void;
  handleResize(): void;
};

type PreviewControlsOptions = {
  previewPanelEl: HTMLElement | null;
  previewPlayButtonEl: HTMLElement | null;
  previewFullscreenButtonEl: HTMLElement | null;
  previewStepBackButtonEl: HTMLElement | null;
  previewStepForwardButtonEl: HTMLElement | null;
  previewTimeInputEl: HTMLInputElement | null;
  previewFpsEl: HTMLElement | null;
  renderer: PreviewPlayback;
  getPreviewPaused(): boolean;
  setPreviewPausedState(nextPaused: boolean): void;
};

export function normalizePreviewLoopSeconds(seconds: unknown): number {
  const next = Number(seconds);
  if (!Number.isFinite(next) || next <= 0) {
    return 0;
  }
  return next;
}

export function wrapPreviewTime(seconds: unknown, loopSeconds: unknown): number {
  const time = Number(seconds);
  const loop = normalizePreviewLoopSeconds(loopSeconds);
  if (!Number.isFinite(time)) {
    return 0;
  }
  if (loop <= 0) {
    return time;
  }
  const wrapped = time % loop;
  return wrapped < 0 ? wrapped + loop : wrapped;
}

function isInteractiveTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  if (target.isContentEditable) {
    return true;
  }
  if (target.closest(".monaco-editor")) {
    return true;
  }
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

export function createPreviewControls({
  previewPanelEl,
  previewPlayButtonEl,
  previewFullscreenButtonEl,
  previewStepBackButtonEl,
  previewStepForwardButtonEl,
  previewTimeInputEl,
  previewFpsEl,
  renderer,
  getPreviewPaused,
  setPreviewPausedState
}: PreviewControlsOptions) {
  function isPreviewFullscreen(): boolean {
    return document.fullscreenElement === previewPanelEl;
  }

  function syncPreviewStepButtonsVisibility(): void {
    const paused = Boolean(getPreviewPaused());
    for (const el of [previewStepBackButtonEl, previewStepForwardButtonEl]) {
      if (!el) {
        continue;
      }
      el.hidden = !paused;
      el.setAttribute("aria-hidden", paused ? "false" : "true");
    }
  }

  function syncPreviewPlayButtonState(): void {
    if (!previewPlayButtonEl) {
      return;
    }
    const previewPaused = Boolean(getPreviewPaused());
    previewPlayButtonEl.innerHTML = previewPaused ? PLAY_ICON_SVG : PAUSE_ICON_SVG;
    previewPlayButtonEl.setAttribute("aria-label", previewPaused ? "Play preview" : "Pause preview");
    previewPlayButtonEl.setAttribute("title", previewPaused ? "Play preview" : "Pause preview");
    previewPlayButtonEl.setAttribute("aria-pressed", previewPaused ? "true" : "false");
    syncPreviewStepButtonsVisibility();
  }

  function syncPreviewTimeReadout(): void {
    if (previewTimeInputEl && document.activeElement !== previewTimeInputEl) {
      previewTimeInputEl.value = Number(renderer.simTimeSeconds).toFixed(3);
    }
  }

  function setPreviewPaused(nextPaused: boolean): void {
    const previewPaused = Boolean(nextPaused);
    setPreviewPausedState(previewPaused);
    renderer.setPlaybackPaused(previewPaused);
    syncPreviewPlayButtonState();
    syncPreviewTimeReadout();
    if (previewPaused) {
      if (previewFpsEl) {
        previewFpsEl.textContent = "FPS Paused";
      }
      return;
    }
    renderer.fpsFrameCount = 0;
    renderer.fpsSampleStartMs = performance.now();
  }

  function applyPreviewTimeInput({ force = false }: { force?: boolean } = {}): void {
    if (!previewTimeInputEl) {
      return;
    }
    const next = Number(previewTimeInputEl.value);
    if (!Number.isFinite(next)) {
      if (force) {
        previewTimeInputEl.value = Number(renderer.simTimeSeconds).toFixed(3);
      }
      return;
    }
    renderer.setPlaybackTime(next);
    renderer.drawFrame({ advanceTime: false });
    syncPreviewTimeReadout();
  }

  function stepPreviewFrames(frames: number): void {
    if (!Number.isFinite(frames) || frames === 0) {
      return;
    }
    if (!getPreviewPaused()) {
      setPreviewPaused(true);
    }
    renderer.stepPlaybackFrames(frames, 60);
    renderer.drawFrame({ advanceTime: false });
    syncPreviewTimeReadout();
  }

  function onPreviewHotkey(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.repeat || isInteractiveTypingTarget(event.target)) {
      return;
    }

    if (event.code === "Space") {
      event.preventDefault();
      setPreviewPaused(!getPreviewPaused());
      return;
    }

    if (event.altKey && event.code === "ArrowLeft") {
      event.preventDefault();
      stepPreviewFrames(event.shiftKey ? -10 : -1);
      return;
    }

    if (event.altKey && event.code === "ArrowRight") {
      event.preventDefault();
      stepPreviewFrames(event.shiftKey ? 10 : 1);
      return;
    }

    if (event.altKey && event.code === "Digit0") {
      event.preventDefault();
      if (!getPreviewPaused()) {
        setPreviewPaused(true);
      }
      renderer.setPlaybackTime(0);
      renderer.drawFrame({ advanceTime: false });
      syncPreviewTimeReadout();
    }
  }

  function syncPreviewFullscreenButtonState(): void {
    if (!previewFullscreenButtonEl) {
      return;
    }
    const active = isPreviewFullscreen();
    previewFullscreenButtonEl.innerHTML = active ? FULLSCREEN_EXIT_ICON_SVG : FULLSCREEN_ENTER_ICON_SVG;
    previewFullscreenButtonEl.setAttribute("aria-label", active ? "Exit fullscreen" : "Enter fullscreen");
    previewFullscreenButtonEl.setAttribute("title", active ? "Exit fullscreen" : "Enter fullscreen");
    previewFullscreenButtonEl.setAttribute("aria-pressed", active ? "true" : "false");
  }

  function onPreviewFullscreenChange(): void {
    syncPreviewFullscreenButtonState();
    // Fullscreen transitions can settle over multiple layout ticks.
    // Resize after two RAFs so canvas/backbuffer dimensions stay in sync.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        renderer.handleResize();
      });
    });
  }

  async function togglePreviewFullscreen(): Promise<void> {
    if (!previewPanelEl) {
      return;
    }
    if (isPreviewFullscreen()) {
      await document.exitFullscreen();
      return;
    }
    await previewPanelEl.requestFullscreen();
  }

  return {
    syncPreviewPlayButtonState,
    syncPreviewTimeReadout,
    setPreviewPaused,
    applyPreviewTimeInput,
    stepPreviewFrames,
    onPreviewHotkey,
    syncPreviewFullscreenButtonState,
    onPreviewFullscreenChange,
    togglePreviewFullscreen
  };
}