import { afterEach, expect, it, vi } from "vitest";
import { createSourceStateController } from "../app/source-state";

afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

it("preserves renderer and surface choices made while source encoding is pending", async () => {
  vi.useFakeTimers();
  let complete!: (payload: string) => void;
  const encoded = new Promise<string>(resolve => { complete = resolve; });
  const location = { pathname: "/", search: "?renderer=forward&completion=old", hash: "#preview" };
  let committed = "";
  vi.stubGlobal("window", { location, history: { replaceState: (_state: unknown, _title: string, url: string) => { committed = url; } } });
  const controller = createSourceStateController({
    exampleSelectEl: { value: "custom" } as HTMLSelectElement,
    getSource: () => "surface example {}", setEditorSource: () => {},
    isSourceBlank: source => source.trim() === "", encodeSharedSource: () => encoded,
    encodeRawSource: source => "raw." + source, decodeSharedSource: async () => null,
    resolveExampleFromUrlParam: () => null, examplesById: new Map(), customExampleValue: "custom",
    maxShareParamLength: 12000, urlParamExample: "example", urlParamCode: "code", starterSource: "",
  });
  controller.scheduleUrlSync();
  await vi.advanceTimersByTimeAsync(700);
  location.search = "?renderer=forward-plus&surface=second&completion=old";
  complete("raw.encoded");
  await Promise.resolve();
  const url = new URL(committed, "http://localhost");
  expect(url.searchParams.get("renderer")).toBe("forward-plus");
  expect(url.searchParams.get("surface")).toBe("second");
  expect(url.searchParams.get("code")).toBe("raw.encoded");
  expect(url.searchParams.has("completion")).toBe(false);
  expect(url.hash).toBe("#preview");
});
