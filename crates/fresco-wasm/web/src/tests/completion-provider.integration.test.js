import { describe, expect, it, vi } from "vitest";

import { createWasmCompletionProvider } from "../completion-provider";

describe("wasm completion provider wiring", () => {
  it("keeps cell members separate from the generic fallback catalog", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => [{label: "inset_distance", kind: "function", detail: "cell member (method)", allowed: true}],
      utf16OffsetToUtf8Byte: (_source, offset) => offset,
      fallbackItemsFn: () => [{label: "abs", kind: "function"}]
    });
    const source="in space cells(cell: tile) { tile.";
    const items=await provider({getValue:()=>source,getOffsetAt:()=>source.length},{});
    expect(items.map(item=>item.label)).toEqual(["inset_distance"]);
  });
  it("keeps enum value suggestions constrained instead of merging the fallback catalog", async () => {
    const layouts = ["brick", "hex", "jittered", "square", "voronoi"];
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => layouts.map(label => ({ label, kind: "enum", allowed: true })),
      utf16OffsetToUtf8Byte: (_source, offset) => offset,
      fallbackItemsFn: () => [
        { label: "abs", kind: "function" },
        { label: "seed:", kind: "property" },
        { label: "compose", kind: "keyword" }
      ]
    });
    const source = ".cells(layout: ";
    const items = await provider({ getValue: () => source, getOffsetAt: () => source.length }, {});
    expect(items.map(item => item.label)).toEqual(layouts);
  });

  it("forwards utf8 cursor and maps rich wasm metadata", async () => {
    const completionItemsFn = vi.fn(() => [
      {
        label: "shadow",
        insert_text: "shadow()",
        kind: "function",
        detail: "Builtin (effects)",
        boost: 180,
        receiver_kinds: ["shape", "layer"],
        signature: "shadow(offset: vec2, soften: f32, color: color) -> layer",
        documentation: "Apply an analytic shadow effect.",
        snippet: "shadow(offset: ${1}, soften: ${2}, color: ${3})"
      }
    ]);
    const utf16OffsetToUtf8Byte = vi.fn(() => 42);

    const provider = createWasmCompletionProvider({
      completionItemsFn,
      utf16OffsetToUtf8Byte
    });

    const model = {
      getValue: () => "circle(at: center, radius: 0.2) |> sh",
      getOffsetAt: () => 33
    };

    const completions = await provider(model, { lineNumber: 1, column: 34 });

    expect(utf16OffsetToUtf8Byte).toHaveBeenCalledWith(model.getValue(), 33);
    expect(completionItemsFn).toHaveBeenCalledWith(model.getValue(), 42);
    expect(completions).toHaveLength(1);
    expect(completions[0]).toMatchObject({
      label: "shadow",
      type: "function",
      insertText: "shadow()",
      detail: "Builtin (effects)",
      receiverKinds: ["shape", "layer"],
      signature: "shadow(offset: vec2, soften: f32, color: color) -> layer",
      documentation: "Apply an analytic shadow effect.",
      snippet: "shadow(offset: ${1}, soften: ${2}, color: ${3})"
    });
    expect(completions[0].info).toContain("shadow(offset: vec2, soften: f32, color: color) -> layer");
    expect(completions[0].info).toContain("Apply an analytic shadow effect.");
  });

  it("fails closed to empty list when wasm completion call throws", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => {
        throw new Error("boom");
      },
      utf16OffsetToUtf8Byte: () => 0
    });

    const completions = await provider(
      {
        getValue: () => "",
        getOffsetAt: () => 0
      },
      { lineNumber: 1, column: 1 }
    );

    expect(completions).toEqual([]);
  });

  it("falls back to local items when wasm completion call throws", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => {
        throw new Error("boom");
      },
      utf16OffsetToUtf8Byte: () => 0,
      fallbackItemsFn: () => [
        { label: "compose", kind: "keyword" },
        { label: "color", kind: "type" }
      ]
    });

    const completions = await provider(
      {
        getValue: () => "co",
        getOffsetAt: () => 2
      },
      { lineNumber: 1, column: 3 }
    );

    expect(completions).toHaveLength(2);
    expect(completions.map((item) => item.label)).toContain("compose");
  });

  it("falls back to local items when wasm completion returns a non-array payload", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => ({ bad: true }),
      utf16OffsetToUtf8Byte: () => 0,
      fallbackItemsFn: () => [
        { label: "blend", kind: "keyword" },
        { label: "let", kind: "keyword" }
      ]
    });

    const completions = await provider(
      {
        getValue: () => "bl",
        getOffsetAt: () => 2
      },
      { lineNumber: 1, column: 3 }
    );

    expect(completions).toHaveLength(1);
    expect(completions[0].label).toBe("blend");
  });

  it("always filters disallowed results", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => [
        { label: "canvas", kind: "keyword", allowed: false },
        { label: "f32", kind: "type", allowed: true }
      ],
      utf16OffsetToUtf8Byte: () => 0
    });

    const completions = await provider(
      {
        getValue: () => "",
        getOffsetAt: () => 0
      },
      { lineNumber: 1, column: 1 }
    );

    expect(completions).toHaveLength(1);
    expect(completions[0].label).toBe("f32");
  });

  it.each(["", "shape |> "])("does not reintroduce rejected items through fallback for %j", async (source) => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => [
        { label: "shadow", kind: "function", receiver_kinds: ["shape"], allowed: false }
      ],
      utf16OffsetToUtf8Byte: () => source.length,
      fallbackItemsFn: () => [
        { label: "shadow", kind: "function", receiverKinds: ["shape"] },
        { label: "round", kind: "function", receiverKinds: ["shape"], allowed: true }
      ]
    });
    const items = await provider({ getValue: () => source, getOffsetAt: () => source.length }, {});
    expect(items.map((item) => item.label)).toEqual(["round"]);
  });

  it("filters disallowed fallback items when the worker is unavailable", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => { throw new Error("worker unavailable"); },
      utf16OffsetToUtf8Byte: () => 0,
      fallbackItemsFn: () => [
        { label: "canvas", kind: "keyword", allowed: false },
        { label: "f32", kind: "type", allowed: true }
      ]
    });
    const items = await provider({ getValue: () => "", getOffsetAt: () => 0 }, {});
    expect(items.map((item) => item.label)).toEqual(["f32"]);
  });

  it("augments pipe context with fallback functions when wasm only returns scope symbols", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => [{ label: "b", kind: "variable", detail: "Symbol in scope" }],
      utf16OffsetToUtf8Byte: () => 0,
      fallbackItemsFn: () => [
        {
          label: "round",
          kind: "function",
          detail: "Builtin",
          signature: "shape |> round(radius: scalar) -> shape",
          receiverKinds: ["shape"]
        },
        {
          label: "fill",
          kind: "function",
          detail: "Builtin",
          signature: "shape |> fill(color: color) -> layer",
          receiverKinds: ["shape"]
        },
        { label: "compose", kind: "keyword", detail: "Fresco keyword" }
      ]
    });

    const source = "let b = box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> ";
    const completions = await provider(
      {
        getValue: () => source,
        getOffsetAt: () => source.length
      },
      { lineNumber: 1, column: source.length + 1 }
    );

    const labels = completions.map((item) => item.label);
    expect(labels).toContain("round");
    expect(labels).toContain("fill");
    expect(labels).not.toContain("b");
    expect(labels).not.toContain("compose");
  });

  it("narrows pipe context using receiver metadata when wasm returns mixed functions", async () => {
    const provider = createWasmCompletionProvider({
      completionItemsFn: () => [
        {
          label: "abs",
          kind: "function",
          detail: "Builtin (scalar-math)",
          signature: "abs(x: scalar) -> scalar"
        },
        {
          label: "box",
          kind: "function",
          detail: "Builtin (shapes)",
          signature: "box(at: vec2, size: vec2) -> shape"
        },
        {
          label: "round",
          kind: "function",
          detail: "Builtin (shape-ops)",
          signature: "shape |> round(radius: scalar) -> shape",
          receiver_kinds: ["shape"]
        },
        {
          label: "bevel",
          kind: "function",
          detail: "Builtin (effects)",
          signature: "layer |> bevel(width?: scalar) -> layer",
          receiver_kinds: ["layer"]
        }
      ],
      utf16OffsetToUtf8Byte: () => 0,
      fallbackItemsFn: () => [
        {
          label: "round",
          kind: "function",
          detail: "Builtin",
          signature: "shape |> round(radius: scalar) -> shape",
          receiverKinds: ["shape"]
        }
      ]
    });

    const source = "let b = box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> ";
    const completions = await provider(
      {
        getValue: () => source,
        getOffsetAt: () => source.length
      },
      { lineNumber: 1, column: source.length + 1 }
    );

    const labels = completions.map((item) => item.label);
    expect(labels).toContain("round");
    expect(labels).toContain("bevel");
    expect(labels).not.toContain("box");
    expect(labels).not.toContain("abs");
  });
});
