import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

test("projected card stripes retain directional coverage at grazing angles", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const adapter = await page.evaluate(() => window.gpuTest.init("flip-card"));
  await testInfo.attach("adapter", { body: JSON.stringify(adapter), contentType: "application/json" });
  const measurements = [];
  const angles = [0, 65, 80, 85, 88];
  for (const angle of angles) {
    const time = 2 + angle / 90;
    await page.evaluate(async ({ angle, time }) => {
      await window.gpuTest.frame(`after-${angle}`, time, 0, 384, 512);
      await window.gpuTest.frame("high-resolution", time, 0, 1536, 2048);
      window.gpuTest.downsample(`reference-${angle}`, "high-resolution", 4);
    }, { angle, time });
    const error = await page.evaluate(angle => window.gpuTest.stripeError(`after-${angle}`, `reference-${angle}`), angle);
    measurements.push({ angle, ...error });
    for (const name of [`after-${angle}`, `reference-${angle}`]) {
      const png = await page.evaluate(name => window.gpuTest.png(name), name);
      await testInfo.attach(name, { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
    }
  }
  // Optional diagnostic capture of a pre-change shader. Never a saved baseline
  // or an input to the regression assertions; references are rendered live.
  if (process.env.FRESCO_AA_BEFORE) {
    const prefix = process.env.FRESCO_AA_BEFORE;
    const wgsl = await readFile(`${prefix}.wgsl`, "utf8");
    const manifest = JSON.parse(await readFile(`${prefix}.json`, "utf8"));
    await page.evaluate(({ wgsl, manifest }) => window.gpuTest.replaceShader(wgsl, manifest), { wgsl, manifest });
    for (const angle of angles) {
      await page.evaluate(angle => window.gpuTest.frame(`before-${angle}`, 2 + angle / 90, 0, 384, 512), angle);
      measurements.find(item => item.angle === angle).before = await page.evaluate(angle => window.gpuTest.stripeError(`before-${angle}`, `reference-${angle}`), angle);
      const png = await page.evaluate(angle => window.gpuTest.png(`before-${angle}`), angle);
      await testInfo.attach(`before-${angle}`, { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
    }
  }
  await testInfo.attach("coverage-measurements", { body: JSON.stringify(measurements, null, 2), contentType: "application/json" });
  for (const measurement of measurements) {
    expect(measurement.cardPixels).toBeGreaterThan(100);
    // Allow more quadrature error when the entire card is only a few pixels
    // wide. The former isotropic band exceeds these bounds at 80/85/88 degrees.
    expect(measurement.meanRedError, `coverage error at ${measurement.angle} degrees`)
      .toBeLessThan(measurement.angle >= 88 ? 6 : 2);
  }
  expect(errors).toEqual([]);
});
