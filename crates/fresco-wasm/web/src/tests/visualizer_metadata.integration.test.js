import { beforeAll, describe, expect, it } from "vitest";
import { readFile } from "node:fs/promises";

import initWasm, { fresco_compile_visualizer } from "../../pkg/fresco_wasm.js";

import enginePolicy from "../../../../../tests/render-policy/engine/engine.fr?raw";

const SPACE_SOURCE = `canvas badge(uv: coord, time: signal) -> color {
    space stage = centered(aspect: preserve)  // @viz

    compose {
        in space stage {
            circle(at: center, radius: 0.2) |> fill(#ffffff)
        }
    }
}
`;

const SHAPE_SOURCE = `canvas badge(uv: coord, time: signal) -> color {
    let b = box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> round(0.04uv)  // @viz

    compose {
        b |> fill(#ff2d78)
    }
}
`;

beforeAll(async () => {
    const wasmPath = new URL("../../pkg/fresco_wasm_bg.wasm", import.meta.url);
    const wasmBytes = await readFile(wasmPath);
    await initWasm({ module_or_path: wasmBytes });
});

describe("browser visualizer metadata", () => {
    it("requires an explicit engine for visualizer compilation", () => {
        const marker = "centered(aspect: preserve)";
        const start = SPACE_SOURCE.indexOf(marker);
        const result = fresco_compile_visualizer(SPACE_SOURCE, start, start + marker.length, "thumbnail", "thumb", 0);
        expect(result.ok).toBe(false);
        expect(result.diagnostics.filter(diag => diag.message.includes("missing required engine setting"))).toHaveLength(4);
    });

    it("reports malformed visualizer bundles", () => {
        const result = fresco_compile_visualizer(SPACE_SOURCE, 0, 1, "thumbnail", "thumb", 0, 42);
        expect(result.ok).toBe(false);
        expect(result.diagnostics[0].message).toContain("invalid files argument");
    });

    it("returns space distortion metadata for thumbnail previews", () => {
        const marker = "centered(aspect: preserve)";
        const start = SPACE_SOURCE.indexOf(marker);
        expect(start).toBeGreaterThanOrEqual(0);

        const result = fresco_compile_visualizer(
            SPACE_SOURCE,
            start,
            start + marker.length,
            "thumbnail",
            "thumb",
            0,
            { "engine/engine.fr": enginePolicy }
        );

        expect(result.ok).toBe(true);
        expect(result.semanticType).toBe("space");
        expect(result.metadata).toMatchObject({
            kind: "thumbnail",
            domain: "thumb",
            fitMode: "space-probe"
        });
        expect(String(result.metadata.previewLabel || "")).toContain("space centered(preserve)");
        expect(String(result.metadata.previewDetail || "")).toContain("distortion-field");
        expect(String(result.metadata.previewDetail || "")).toContain("warp-grid");
    });

    it("returns shape metadata labels for thumbnail previews", () => {
        const marker = "box(at: (0.5, 0.5), size: (0.30uv, 0.20uv)) |> round(0.04uv)";
        const start = SHAPE_SOURCE.indexOf(marker);
        expect(start).toBeGreaterThanOrEqual(0);

        const result = fresco_compile_visualizer(
            SHAPE_SOURCE,
            start,
            start + marker.length,
            "thumbnail",
            "thumb",
            0,
            { "engine/engine.fr": enginePolicy }
        );

        expect(result.ok).toBe(true);
        expect(result.semanticType).toBe("shape");
        expect(result.metadata).toMatchObject({
            kind: "thumbnail",
            domain: "thumb",
            fitMode: "shape-probe"
        });
        expect(result.metadata.previewLabel).toBe("shape box -> round");
        expect(String(result.metadata.previewDetail || "")).toContain("rounded");
    });
});
