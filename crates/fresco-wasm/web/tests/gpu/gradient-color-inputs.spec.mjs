import { test, expect } from "@playwright/test";

const cases = [
  ["tint", "disk |> fill(#fff) |> tint(ink)"],
  ["shadow", "disk |> shadow(offset: (0.04, 0.02), soften: 0.04, color: ink)"],
  ["soften", "disk |> soften(radius: 0.04, color: ink)"],
  ["bevel", "disk |> bevel(width: 0.06, highlight: ink, shadow: ink)"],
  ["stroke", "disk |> stroke(width: 0.04, color: ink)"],
  ["path stroke", 'let curve = path_svg("M 0.1 0.2 L 0.9 0.8"); curve |> stroke(width: 0.05, color: ink)'],
  ["color transforms", "fill(lighten(desaturate(ink, by: 0.3), by: 0.2))"],
];

function program(body, analytic, radial = false) {
  const coordinate = radial ? "clamp(length(uv - (0.5, 0.5)) / 0.5, 0.0, 1.0)" : "uv.x";
  const gradient = radial ? "kind: radial, center: (0.5, 0.5), radius: 0.5" : "along: x";
  const ink = analytic
    ? "rgba(1.0 - blend_t, 0.0, blend_t, 0.25 + 0.5 * blend_t)"
    : `gradient(${gradient}, stops: [stop(at: 0.0, color: rgba(1, 0, 0, 0.25)), stop(at: 1.0, color: rgba(0, 0, 1, 0.75))])`;
  return `canvas gradient_probe(ctx: CanvasContext) -> color {
    let uv = context(coord)
    let blend_t = ${coordinate}
    let ink = ${ink}
    let disk = circle(at: (0.5, 0.5), radius: 0.32)
    ${body}
  }`;
}

async function render(page, name, source) {
  await page.evaluate(source => window.gpuTest.init("flip-card", source), source);
  await page.evaluate(name => window.gpuTest.frame(name, 0, 0, 160, 144), name);
  return page.evaluate(name => window.gpuTest.pixels(name), name);
}

for (const [name, body] of cases) {
  test(`${name} samples gradient color and alpha like an analytical color field`, async ({ page }) => {
    test.setTimeout(120_000);
    await page.goto("/tests/gpu/");
    await page.waitForFunction(() => window.gpuTest);
    for (const radial of [false, true]) {
      const actual = await render(page, "actual", program(body, false, radial));
      const reference = await render(page, "reference", program(body, true, radial));
      let total = 0, max = 0, visible = 0;
      for (let i = 0; i < actual.length; i += 4) {
        for (let c = 0; c < 3; c++) {
          const error = Math.abs(actual[i + c] - reference[i + c]);
          total += error;
          max = Math.max(max, error);
          if (actual[i + c] > 10) visible++;
        }
      }
      // Gradient dithering intentionally differs from a plain color field by
      // at most a small quantization error. Require visible output as well.
      expect(visible).toBeGreaterThan(100);
      expect(total / (actual.length / 4 * 3)).toBeLessThan(0.7);
      expect(max).toBeLessThanOrEqual(2);
    }
  });
}

test("shape-anchored gradient tint follows each receiver independently", async ({ page }) => {
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const source = tint => `canvas anchors(ctx: CanvasContext) -> color {
    let ink = gradient(along: x, anchor: shape, stops: [stop(at: 0.0, color: #f00), stop(at: 1.0, color: #00f)])
    compose {
      box(at: (0.25, 0.5), size: (0.35, 0.7)) |> ${tint ? "fill(#fff) |> tint(ink)" : "fill(ink)"}
      box(at: (0.75, 0.5), size: (0.2, 0.4)) |> ${tint ? "fill(#fff) |> tint(ink)" : "fill(ink)"}
    }
  }`;
  await render(page, "actual", source(true));
  await render(page, "reference", source(false));
  expect(await page.evaluate(() => window.gpuTest.compare("actual", "reference")))
    .toEqual({ changedFraction: 0, maxDifference: 0 });
});
