import { test, expect } from "@playwright/test";

test("the host explicitly selects compile-known engine-pass variants", async ({ page }, testInfo) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const low = await page.evaluate(() => window.gpuTest.init("pass-variant-low"));
  expect(low.enginePass?.variants.map(variant => variant.key)).toEqual([
    "quality=low",
    "quality=high",
  ]);
  const lowFrame = await page.evaluate(() => window.gpuTest.frame("variant-low", 0));

  await page.reload();
  await page.waitForFunction(() => window.gpuTest);
  const high = await page.evaluate(() => window.gpuTest.init("pass-variant-high"));
  const highFrame = await page.evaluate(() => window.gpuTest.frame("variant-high", 0));

  expect(lowFrame.center[0]).toBeGreaterThanOrEqual(60);
  expect(lowFrame.center[0]).toBeLessThanOrEqual(68);
  expect(lowFrame.center[1]).toBeGreaterThanOrEqual(187);
  expect(lowFrame.center[1]).toBeLessThanOrEqual(195);
  expect(highFrame.center[0]).toBeGreaterThan(110);
  expect(highFrame.center[1]).toBeGreaterThan(110);
  await testInfo.attach("engine-pass-variants", {
    body: JSON.stringify({ low, high, lowFrame, highFrame }, null, 2),
    contentType: "application/json",
  });
  expect(errors).toEqual([]);
});

test("invalid variant installation preserves the image and a valid selection recovers", async ({ page }) => {
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(() => window.gpuTest.init("pass-variant-low"));
  await page.evaluate(() => window.gpuTest.frame("before", 0));
  for (const selection of [null, {}, { quality: "unknown" }, { quality: "low", extra: "on" }]) {
    const error = await page.evaluate(async selection => {
      try { await window.gpuTest.selectVariant(selection); return null; }
      catch (error) { return String(error); }
    }, selection);
    expect(error).toMatch(/variant/i);
    await page.evaluate(() => window.gpuTest.frame("after-error", 0));
    expect(await page.evaluate(() => window.gpuTest.compare("before", "after-error")))
      .toEqual({ changedFraction: 0, maxDifference: 0 });
  }
  await page.evaluate(() => window.gpuTest.selectVariant({ quality: "high" }));
  await page.evaluate(() => window.gpuTest.frame("recovered", 0));
  expect((await page.evaluate(() => window.gpuTest.compare("before", "recovered"))).changedFraction).toBeGreaterThan(0.1);
});

test("a sole emitted variant executes without host configuration", async ({ page }) => {
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const compiled = await page.evaluate(() => window.gpuTest.init("pass-variant-single"));
  expect(compiled.enginePass.variants.map(variant => variant.key)).toEqual(["quality=low"]);
  const frame = await page.evaluate(() => window.gpuTest.frame("sole-variant", 0));
  expect(frame.center[0]).toBeGreaterThanOrEqual(60);
  expect(frame.center[0]).toBeLessThanOrEqual(68);
  expect(frame.center[1]).toBeGreaterThanOrEqual(187);
  expect(frame.center[1]).toBeLessThanOrEqual(195);
});
