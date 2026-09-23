import { describe, it, expect } from "vitest";
import { buildCompileFilesBundle } from "../runtime-content-registry";

describe("renderer configuration", () => {
  it("replaces only the host configuration and leaves source/editor files intact", () => {
    const source = "surface example(sp: surf) -> material(standard) {}";
    const base = new Map([["main.fr", source], ["engine/core/02_functions.fr", "// authored edit"],
      ["engine/config/renderer.fr", "// previous renderer"]]);
    for (const mode of ["forward", "forward-plus", "deferred", "studio"] as const) {
      const bundle = buildCompileFilesBundle(base, "", mode);
      expect(bundle.get("main.fr")).toBe(source);
      expect(bundle.get("engine/core/02_functions.fr")).toBe("// authored edit");
      expect(bundle.get("engine/config/renderer.fr")).toBe("// previous renderer");
      expect(JSON.parse(bundle.get("fresco.config.json")!)).toEqual({ renderer: mode });
    }
    expect(base.get("engine/config/renderer.fr")).toBe("// previous renderer");
  });
});
