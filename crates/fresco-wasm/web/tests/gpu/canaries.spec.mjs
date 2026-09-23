import { test, expect } from "@playwright/test";

// Calculated pixel expectations, not saved images. Keep probes away from AA edges.
function near(actual, expected, label) {
  expect(actual, label).toHaveLength(4);
  for (let channel = 0; channel < 4; channel++) {
    expect(Math.abs(actual[channel] - expected[channel]), `${label}, channel ${channel}`).toBeLessThanOrEqual(2);
  }
}

test("perspective flip canary: typed context, conditional faces and derivative AA compile and render", async ({ page }) => {
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(() => window.gpuTest.init("flip-card"));
  const front = await page.evaluate(() => window.gpuTest.frame("front", 0, 0, 256, 256));
  const back = await page.evaluate(() => window.gpuTest.frame("back", 2, 0, 256, 256));
  near(front.center, [248, 250, 252, 255], "front face");
  near(back.center, [15, 118, 110, 255], "back face");
  await page.evaluate(() => window.gpuTest.frame("front-again", 4, 0, 256, 256));
  const repeated = await page.evaluate(() => window.gpuTest.compare("front", "front-again"));
  expect(repeated.maxDifference).toBeLessThanOrEqual(1);
});

test("behavior canary: imports, context, controls, arrays, composition, time, resize and repeatability", async ({ page }, testInfo) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const manifest = await page.evaluate(() => window.gpuTest.initCanary());
  expect(manifest.canvases[0].params.map(param => param.name)).toEqual(["gain", "levels"]);
  expect(manifest.canvases[0].engine_pass).toBeTruthy();
  const render = (name, time, delta = 0, size = 256) => page.evaluate(
    args => window.gpuTest.frame(...args), [name, time, delta, size, size],
  );
  const corner = (name, size = 256) => page.evaluate(
    ([name, size]) => window.gpuTest.pixel(name, Math.floor(size / 8), Math.floor(size / 8), size), [name, size],
  );

  const initial = await render("initial", 2, 0.125);
  near(initial.center, [191, 64, 32, 255], "array-selected imported color on circle");
  near(await corner("initial"), [16, 96, 128, 255], "coordinate, gain, time, delta and resolution");
  for (const [x, red] of [[64, 64], [128, 128], [192, 191]]) {
    const pixel = await page.evaluate(x => window.gpuTest.pixel("initial", x, 38, 256), x);
    near(pixel, [red, 64, 32, 255], "unrolled loop and indexed array color");
  }
  await page.evaluate(() => window.gpuTest.setCanaryParam("gain", 1));
  await render("gain", 2, 0.125);
  near(await corner("gain"), [32, 96, 128, 255], "scalar control reaches GPU");
  await page.evaluate(() => window.gpuTest.setCanaryParam("levels", [0.1, 0.3, 0.5]));
  const array = await render("array", 2, 0.125);
  near(array.center, [128, 64, 32, 255], "array control reaches imported helper");
  const resized = await render("resized", 0, 0, 128);
  expect([resized.width, resized.height]).toEqual([128, 128]);
  near(await corner("resized", 128), [33, 0, 64, 255], "resize and seek update context");
  await page.evaluate(() => {
    window.gpuTest.setCanaryParam("gain", 0.5);
    window.gpuTest.setCanaryParam("levels", [0.25, 0.5, 0.75]);
  });
  await render("restored", 2, 0.125);
  const repeated = await page.evaluate(() => window.gpuTest.compare("initial", "restored"));
  expect(repeated.maxDifference).toBeLessThanOrEqual(1);
  expect(errors).toEqual([]);
  await testInfo.attach("measurements", {
    body: JSON.stringify({ initial, array, resized, repeated }), contentType: "application/json",
  });
});
