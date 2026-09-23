import { describe, expect, it } from "vitest";
import { digest, inputDigest, isCurrent, embedPreviews } from "../../scripts/readme-media-lib.mjs";

describe("README preview cache", () => {
  it("invalidates source, renderer, settings, and binary asset changes", () => {
    const source = new Map([["sample.fr", "source\n"], ["renderer.ts", "renderer"], ["texture.png", Buffer.from([1, 2])]]);
    const original = inputDigest(source, { fps: 12 });
    expect(inputDigest(new Map([...source].reverse()), { fps: 12 })).toBe(original);
    expect(inputDigest(new Map([...source, ["sample.fr", "source\r\n"]]), { fps: 12 })).toBe(original);
    for (const key of source.keys()) {
      const changed = new Map(source);
      changed.set(key, "changed");
      expect(inputDigest(changed, { fps: 12 })).not.toBe(original);
    }
    expect(inputDigest(source, { fps: 24 })).not.toBe(original);
  });

  it("rerenders missing or corrupted outputs even when source is unchanged", () => {
    const bytes = Buffer.from("preview");
    const record = { inputSha256: "source", outputSha256: digest(bytes) };
    expect(isCurrent(record, "source", bytes)).toBe(true);
    expect(isCurrent(record, "changed", bytes)).toBe(false);
    expect(isCurrent(record, "source", null)).toBe(false);
    expect(isCurrent(record, "source", Buffer.from("corrupt"))).toBe(false);
    expect(isCurrent(undefined, "source", bytes)).toBe(false);
  });

  it("embeds previews idempotently and preserves source blocks and prose", () => {
    const block = "<!-- readme:sample a -->\n```fresco\nsource\n```\n<!-- readme:end -->";
    const markdown = `before\n${block}\n\nafter\n`;
    const samples = [{ id: "a", source: "examples/a.fr", alt: "Preview" }];
    const rendered = embedPreviews(markdown, samples);
    expect(rendered).toContain(block);
    expect(rendered).toContain("![Preview](docs/readme/media/a.webp)");
    expect(rendered.startsWith("before\n")).toBe(true);
    expect(rendered.endsWith("\n\nafter\n")).toBe(true);
    expect(embedPreviews(rendered, samples)).toBe(rendered);
    expect(embedPreviews(rendered, [{ ...samples[0], preview: false }])).toBe(markdown);
    expect(() => embedPreviews(`${block}\n${block}`, samples)).toThrow("Duplicate README sample");
    expect(() => embedPreviews(markdown, [])).toThrow("Unknown README sample");
    expect(() => embedPreviews("no source", samples)).toThrow("not embedded");
  });
});
