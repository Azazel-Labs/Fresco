import { test, expect } from "@playwright/test";
const material = (roughness = 0.35, metallic = 0.5, properties = "", model = "standard") =>
`surface probe(sp: surf) -> material(${model}) {
 ${properties}
 compose { base(albedo: rgba(0.7, 0.35, 0.15, 1.0)${model === "standard" ? `, roughness: ${roughness}, metallic: ${metallic}` : ""}) }
}`;
async function init(page, source, mode = "deferred") {
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(({source, mode}) => window.gpuTest.init("surface", source, undefined, undefined, mode), {source, mode});
  await page.evaluate(() => window.gpuTest.setLightingEnvironment("three-lights"));
}
test("deferred lighting responds to material properties, lights, camera and resize", async ({page}) => {
  const errors = []; page.on("pageerror", e => errors.push(e.message));
  const pictures = [];
  for (const [roughness, metallic] of [[0.15, 0.0], [0.8, 0.0], [0.15, 1.0]]) {
    await init(page, material(roughness, metallic));
    await page.evaluate(() => window.gpuTest.frame("material", 0, 0, 257, 193));
    pictures.push(await page.evaluate(() => window.gpuTest.pixels("material")));
  }
  expect(pictures[0]).not.toEqual(pictures[1]); expect(pictures[0]).not.toEqual(pictures[2]);
  await page.evaluate(() => window.gpuTest.setPointLights([]));
  await page.evaluate(() => window.gpuTest.frame("empty", 0, 0, 257, 193));
  expect((await page.evaluate(() => window.gpuTest.compare("material", "empty"))).changedFraction).toBeGreaterThan(0.01);
  await page.evaluate(() => window.gpuTest.frame("small", 0, 0, 33, 17));
  await page.evaluate(() => window.gpuTest.frame("restored", 0, 0, 257, 193));
  expect((await page.evaluate(() => window.gpuTest.compare("empty", "restored"))).maxDifference).toBe(0);
  await page.evaluate(() => window.gpuTest.setPointLights([{position: [1, 0, 2], radius: 5, color: [1, 0.5, 0.1], intensity: 3}]));
  await page.evaluate(() => window.gpuTest.frame("camera-before", 0, 0, 257, 193));
  await page.evaluate(() => window.gpuTest.camera(0.6, 0.3));
  await page.evaluate(() => window.gpuTest.frame("camera-after", 0, 0, 257, 193));
  expect((await page.evaluate(() => window.gpuTest.compare("camera-before", "camera-after"))).changedFraction).toBeGreaterThan(0.01);
  expect(errors).toEqual([]);
});
test("masked deferred coverage and unlit resolve remain correct", async ({page}) => {
  await init(page, material(0.4, 0.0, "properties { blend: SurfaceBlend.Masked; mask_cutoff: 0.5 }").replace("0.15, 1.0", "0.15, 0.2"));
  await page.evaluate(() => window.gpuTest.frame("masked", 0));
  const masked = await page.evaluate(() => window.gpuTest.pixels("masked"));
  // The browser surface is opaque, so coverage must be checked in RGB, not canvas alpha.
  expect(masked.filter((_, i) => i % 4 !== 3).every(value => value === 0)).toBe(true);
  await init(page, material(0.4, 0.0));
  await page.evaluate(() => window.gpuTest.frame("opaque", 0));
  const opaque = await page.evaluate(() => window.gpuTest.pixels("opaque"));
  expect(opaque.filter((_, i) => i % 4 !== 3).some(value => value > 30)).toBe(true);
  await init(page, material(1, 0, "", "unlit"));
  await page.evaluate(() => window.gpuTest.frame("lit", 0));
  await page.evaluate(() => window.gpuTest.setPointLights([]));
  await page.evaluate(() => window.gpuTest.frame("empty", 0));
  expect((await page.evaluate(() => window.gpuTest.compare("lit", "empty"))).maxDifference).toBe(0);
});
test("transparent deferred selection uses the tiled forward path", async ({page}) => {
  const source = material(0.4, 0.2, "properties { blend: SurfaceBlend.Translucent }").replace("0.15, 1.0", "0.15, 0.4");
  const pictures = [];
  for (const mode of ["forward-plus", "deferred"]) {
    await init(page, source, mode); await page.evaluate(() => window.gpuTest.frame("result", 0));
    pictures.push(await page.evaluate(() => window.gpuTest.pixels("result")));
  }
  expect(pictures[0]).toEqual(pictures[1]);
});
test("browser renderer picker selects deferred without editing the material", async ({page}) => {
  const payload = "raw." + btoa(material());
  await page.goto("/?code=" + encodeURIComponent(payload));
  const selector = page.locator("#preview-renderer");
  await expect(selector).toBeEnabled({timeout: 30000});
  await selector.selectOption("deferred");
  await expect.poll(() => page.evaluate(() => {
    try { return JSON.parse(document.querySelector("#manifest").textContent).surfaces[0].settings.properties.find(p => p.name === "deferred").value; }
    catch { return null; }
  })).toBe(1);
  await page.waitForFunction(() => Number(document.querySelector("#preview-canvas").dataset.engineFrames) > 3);
  expect(new URL(page.url()).searchParams.get("code")).toBe(payload);
  await page.reload(); await expect(selector).toHaveValue("deferred");
  await expect(page.locator("#compile-status")).toContainText("Compiled", {timeout: 30000});
});
