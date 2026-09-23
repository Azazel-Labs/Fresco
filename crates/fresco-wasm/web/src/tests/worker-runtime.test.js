import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createWorkerRuntime } from "../worker-runtime";

let workers;
beforeEach(() => {
  workers = [];
  vi.stubGlobal("Worker", class {
    listeners = new Map();
    terminate = vi.fn();
    postMessage = vi.fn();
    constructor() { workers.push(this); }
    addEventListener(name, listener) { this.listeners.set(name, listener); }
    complete() {
      const { id } = this.postMessage.mock.lastCall[0];
      this.listeners.get("message")({ data: { type: "result", id, ok: true, result: { ok: true } } });
    }
  });
});
afterEach(() => vi.unstubAllGlobals());

function runtime() {
  return createWorkerRuntime({
    compileTimeoutMs: 1000, workerQueryTimeoutMs: 1000,
    workerRequestMaxAttempts: 1, transientWorkerRetryDelayMs: 0,
    workerAutoRecoveryDelayMs: 0, isRetryableWorkerFailure: () => false,
  });
}

describe("compiler worker lifetime", () => {
  it("reuses the loaded compiler when an edit arrives after compilation", async () => {
    const host = runtime();
    const first = host.compileInBackground("first");
    workers[0].complete();
    await first;
    const next = host.compileInBackground("replacement", { cancelInFlight: true });
    expect(workers).toHaveLength(1);
    expect(workers[0].terminate).not.toHaveBeenCalled();
    workers[0].complete();
    await next;
  });

  it("still replaces a worker that is actively compiling obsolete input", async () => {
    const host = runtime();
    const first = host.compileInBackground("first");
    const rejection = expect(first).rejects.toThrow("cancelled");
    const next = host.compileInBackground("replacement", { cancelInFlight: true });
    expect(workers).toHaveLength(2);
    expect(workers[0].terminate).toHaveBeenCalledOnce();
    workers[1].complete();
    await Promise.all([rejection, next]);
  });
});
