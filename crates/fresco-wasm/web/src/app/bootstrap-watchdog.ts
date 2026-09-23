type BootstrapWatchdogDeps = {
  timeoutMs: number;
  onTimeout: (elapsedMs: number) => void;
  now?: () => number;
  setTimeoutFn?: (callback: () => void, timeoutMs: number) => ReturnType<typeof setTimeout>;
  clearTimeoutFn?: (handle: ReturnType<typeof setTimeout>) => void;
};

export function createBootstrapCompileWatchdog(deps: BootstrapWatchdogDeps) {
  const {
    timeoutMs,
    onTimeout,
    now = () => performance.now(),
    setTimeoutFn = (callback, delayMs) => setTimeout(callback, delayMs),
    clearTimeoutFn = (handle) => clearTimeout(handle),
  } = deps;

  let timeoutHandle: ReturnType<typeof setTimeout> | null = null;
  let startedAtMs = 0;

  function clear(): void {
    if (timeoutHandle !== null) {
      clearTimeoutFn(timeoutHandle);
      timeoutHandle = null;
    }
  }

  function arm(): void {
    clear();
    startedAtMs = now();
    timeoutHandle = setTimeoutFn(() => {
      timeoutHandle = null;
      const elapsedMs = Math.max(0, Math.round(now() - startedAtMs));
      onTimeout(elapsedMs);
    }, timeoutMs);
  }

  return {
    arm,
    clear,
  };
}
