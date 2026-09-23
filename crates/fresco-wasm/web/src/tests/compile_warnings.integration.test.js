import { beforeAll, describe, expect, it } from "vitest";
import { readFile } from "node:fs/promises";

import initWasm, { compile_fresco_bundle } from "../../pkg/fresco_wasm.js";

import enginePolicy from "../../../../../tests/render-policy/engine/engine.fr?raw";

const WARNING_SOURCE = `canvas t(uv: coord, time: signal) -> color {
    let a = circle(at: (0.45, 0.5), radius: 0.16)
    let b = box(at: (0.55, 0.5), size: (0.26, 0.18)) |> round(radius: 0.05)
    let joined = (a | b) |> smooth(radius: 0.04)

    compose {
        joined |> shadow(offset: (0.01, -0.01), soften: 0.03, color: #00000088)
        joined |> glow(reach: 0.06, strength: 0.7, color: #7dd3fc66) |> blend(add)
        joined |> fill(#93c5fd)
    }
}
`;

const CLEAN_SOURCE = `canvas t(uv: coord, time: signal) -> color {
    compose {
        circle(at: center, radius: 0.2) |> fill(#ffffff)
    }
}
`;

const COMPARISON_IF_SOURCE = `canvas t(uv: coord, time: signal) -> color {
    let a = circle(at: center, radius: 0.3)
    let cond = cos(time) >= 0.0
    compose {
        if cond {
            a |> fill(#ffffff)
        } else {
            a |> fill(#000000)
        }
    }
}
`;

const ELSE_IF_SOURCE = `canvas t(uv: coord, time: signal) -> color {
    let a = circle(at: center, radius: 0.3)
    let c1 = cos(time) > 0.5
    let c2 = cos(time) >= 0.0
    compose {
        if c1 {
            a |> fill(#ff0000)
        } else if c2 {
            a |> fill(#00ff00)
        } else {
            a |> fill(#0000ff)
        }
    }
}
`;

beforeAll(async () => {
    const wasmPath = new URL("../../pkg/fresco_wasm_bg.wasm", import.meta.url);
    const wasmBytes = await readFile(wasmPath);
    await initWasm({ module_or_path: wasmBytes });
});

describe("browser compile response diagnostics", () => {
    it("keeps ok=true and includes warning diagnostics for wide effects", () => {
        const result = compile_fresco_bundle({ "main.fr": WARNING_SOURCE, "engine/engine.fr": enginePolicy }, "main.fr", false);

        expect(result.ok).toBe(true);
        expect(Array.isArray(result.diagnostics)).toBe(true);

        const warnings = result.diagnostics.filter(
            (diag) => String(diag?.severity || "").toLowerCase() === "warning"
        );

        expect(warnings.length).toBeGreaterThan(0);
        expect(
            warnings.some((diag) =>
                String(diag?.message || "").includes("approximation artifacts")
            )
        ).toBe(true);
    });

    it("returns no diagnostics for a clean compile", () => {
        const result = compile_fresco_bundle({ "main.fr": CLEAN_SOURCE, "engine/engine.fr": enginePolicy }, "main.fr", false);

        expect(result.ok).toBe(true);
        expect(Array.isArray(result.diagnostics)).toBe(true);
        expect(result.diagnostics.length).toBe(0);
    });

    it("supports comparison operators in compose if conditions", () => {
        const result = compile_fresco_bundle({ "main.fr": COMPARISON_IF_SOURCE, "engine/engine.fr": enginePolicy }, "main.fr", false);

        expect(result.ok).toBe(true);
        expect(Array.isArray(result.diagnostics)).toBe(true);
        expect(result.diagnostics.length).toBe(0);
    });

    it("supports else-if chains in compose blocks", () => {
        const result = compile_fresco_bundle({ "main.fr": ELSE_IF_SOURCE, "engine/engine.fr": enginePolicy }, "main.fr", false);

        expect(result.ok).toBe(true);
        expect(Array.isArray(result.diagnostics)).toBe(true);
        expect(result.diagnostics.length).toBe(0);
    });
});
