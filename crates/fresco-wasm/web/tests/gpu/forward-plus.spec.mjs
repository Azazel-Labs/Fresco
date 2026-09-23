import { test, expect } from "@playwright/test";

const source = `surface point_lit(sp: surf) -> material(standard) {
  compose { base(albedo: rgba(0.8, 0.8, 0.8, 1.0)) }
}`;

test("forward+ light updates are atomic and survive partial-tile resize", async ({ page }) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(source => window.gpuTest.init("surface", source, undefined, undefined, "forward-plus"), source);
  await page.evaluate(() => window.gpuTest.setLightingEnvironment("three-lights"));
  await page.evaluate(() => window.gpuTest.frame("demo", 0, 0, 257, 193));
  const light = { position: [0, 0, 1.8], radius: 5, color: [1, 0, 0], intensity: 1.5 };
  await page.evaluate(light => window.gpuTest.setPointLights([light]), light);
  await page.evaluate(() => window.gpuTest.frame("red", 0, 0, 257, 193));
  expect((await page.evaluate(() => window.gpuTest.compare("demo", "red"))).changedFraction).toBeGreaterThan(0.02);
  const failure = await page.evaluate(light => {
    try { window.gpuTest.setPointLights(Array(65).fill(light)); return ""; }
    catch (error) { return String(error); }
  }, light);
  expect(failure).toContain("64");
  await page.evaluate(() => window.gpuTest.frame("unchanged", 0, 0, 257, 193));
  expect((await page.evaluate(() => window.gpuTest.compare("red", "unchanged"))).maxDifference).toBe(0);
  await page.evaluate(() => window.gpuTest.setPointLights([]));
  await page.evaluate(() => window.gpuTest.frame("ambient", 0, 0, 257, 193));
  expect((await page.evaluate(() => window.gpuTest.compare("red", "ambient"))).changedFraction).toBeGreaterThan(0.02);
  await page.evaluate(light => window.gpuTest.setPointLights([light]), light);
  await page.evaluate(() => window.gpuTest.frame("small", 0, 0, 33, 17));
  await page.evaluate(() => window.gpuTest.frame("restored", 0, 0, 257, 193));
  expect((await page.evaluate(() => window.gpuTest.compare("red", "restored"))).maxDifference).toBe(0);
  expect(errors).toEqual([]);
});

test("playground renderer selection recompiles unchanged material and survives reload", async ({ page }) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  const payload = "raw." + btoa(source);
  await page.goto("/?code=" + encodeURIComponent(payload));
  const selector = page.locator("#preview-renderer");
  await expect(selector).toBeEnabled({ timeout: 30000 });
  const selectedProperty = () => page.evaluate(() => {
    try {
      const manifest = JSON.parse(document.querySelector("#manifest").textContent);
      return manifest.surfaces[0].settings.properties.find(p => p.name === "forward_plus");
    } catch { return null; }
  });
  await expect.poll(async () => (await selectedProperty())?.value).toBe(0);
  await page.waitForFunction(() => Number(document.querySelector("#preview-canvas").dataset.engineFrames) > 3);
  await page.locator("#preview-lighting-menu summary").click();
  await page.locator('input[name="preview-lighting"][value="directional"]').check();
  const before = await page.locator("#preview-canvas").screenshot();
  await selector.selectOption("forward-plus");
  await expect.poll(async () => (await selectedProperty())?.value).toBe(1);
  expect((await selectedProperty()).editable).toBe(false);
  await expect(page.locator('[data-engine-property="forward_plus"]')).toHaveCount(0);
  await expect.poll(async () => (await page.locator("#preview-canvas").screenshot()).equals(before)).toBe(false);
  expect(new URL(page.url()).searchParams.get("code")).toBe(payload);
  expect(new URL(page.url()).searchParams.get("renderer")).toBe("forward-plus");
  await page.reload();
  await expect(selector).toHaveValue("forward-plus");
  await expect.poll(async () => (await selectedProperty())?.value).toBe(1);
  await selector.selectOption("forward");
  await expect.poll(async () => (await selectedProperty())?.value).toBe(0);
  expect(new URL(page.url()).searchParams.get("code")).toBe(payload);
  expect(errors).toEqual([]);
});

test("unknown persisted renderer choices show the engine default and a stable diagnostic", async ({ page }) => {
  const payload = "raw." + btoa(source);
  await page.goto("/?renderer=removed-engine-choice&code=" + encodeURIComponent(payload));
  const selector = page.locator("#preview-renderer");
  await expect(selector).toBeEnabled({ timeout: 30000 });
  await expect(page.locator("#compile-status")).toContainText("Compiled", { timeout: 30000 });
  await expect(selector).toHaveValue("forward");
  const warning = page.locator('#preview-renderer + [role="status"]');
  await expect(warning).toContainText('Renderer "removed-engine-choice" is unavailable; using Forward.');
  await selector.selectOption("forward-plus");
  await expect(warning).toHaveText("");
  await expect.poll(() => new URL(page.url()).searchParams.get("renderer")).toBe("forward-plus");
  expect(new URL(page.url()).searchParams.get("code")).toBe(payload);
});
