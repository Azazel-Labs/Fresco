import { test, expect } from "@playwright/test";
const source = (mode, sided = false) => `surface probe(sp: surf) -> material(unlit) {
  properties { blend: SurfaceBlend.${mode}
 two_sided: ${sided}
 mask_cutoff: 0.5 }
  compose { base(albedo: rgba(1.0, 0.0, 0.0, 0.25)) }
}`;
test("opaque, masked, and translucent surfaces change actual rendered coverage", async ({page}) => {
  const colors = {};
  for (const mode of ["Opaque", "Masked", "Translucent"]) {
    await page.goto("/tests/gpu/"); await page.waitForFunction(() => window.gpuTest);
    await page.evaluate(source => window.gpuTest.init("mesh-default",source), source(mode));
    const frame = await page.evaluate(() => window.gpuTest.frame("coverage",0));
    colors[mode] = frame.center;
  }
  expect(colors.Opaque[0]).toBeGreaterThan(240);
  expect(colors.Masked[0]).toBeLessThan(40);
  expect(colors.Translucent[0]).toBeGreaterThan(colors.Masked[0] + 20);
  expect(colors.Translucent[0]).toBeLessThan(colors.Opaque[0] - 40);
});
test("two-sided surfaces render the back of a plane", async ({page}) => {
  const areas = [];
  for (const sided of [false,true]) {
    await page.goto("/tests/gpu/"); await page.waitForFunction(() => window.gpuTest);
    await page.evaluate(source => window.gpuTest.init("mesh-default",source), source("Opaque",sided));
    await page.evaluate(() => { window.gpuTest.mesh("plane"); window.gpuTest.camera(0,-1.1); });
    await page.evaluate(() => window.gpuTest.frame("backface",0));
    const pixels = await page.evaluate(() => window.gpuTest.pixels("backface"));
    areas.push(pixels.filter((value,index) => index % 4 === 0 && value > 200).length);
  }
  expect(areas[0]).toBe(0); expect(areas[1]).toBeGreaterThan(1000);
});
test("surface controls create and update a source-backed property block", async ({page}, testInfo) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(() => {
    window.gpuRuntimeErrors = [];
    window.addEventListener("error", event => window.gpuRuntimeErrors.push(event.message));
  });
  await page.goto("/?example=" + encodeURIComponent("40) surface shaders/uv_material"));
  await expect(page.locator("#preview-backend")).toHaveCount(0);
  await expect(page.locator("#compile-status")).toContainText("Compiled",{timeout:30000});
  await page.locator('[data-tab="inspector"]').click();
  for (const stage of ["vertex", "fragment"]) {
    const row = page.locator("#manifest-inspector .manifest-inspector-row")
      .filter({ hasText: `Declared ${stage} stage` });
    const declaredEntry = await page.evaluate(stage => {
      const manifest = JSON.parse(document.querySelector("#manifest").textContent);
      return manifest.surfaces[0].mesh_passes[0].entries.find(entry => entry.stage === stage).entry;
    }, stage);
    expect(declaredEntry).toBeTruthy();
    await expect(row.locator(".manifest-inspector-value")).toHaveText(declaredEntry);
  }
  await expect(page.locator("#manifest-inspector .manifest-inspector-selected-badge"))
    .not.toHaveText("selected");

  for (const [name,value] of [["blend","SurfaceBlend.Translucent"],["two_sided","true"],["profile","SurfaceProfile.Unlit"]]) {
    const control = page.locator(`[data-engine-property="${name}"]`);
    await control.selectOption(value);
    await expect(control).toBeEnabled({timeout:30000}); await expect(control).toHaveValue(value);
  }
  await expect.poll(async () => page.evaluate(async () => {
    const {decodeSharedSource} = await import("/src/app/source-url-codec.ts");
    return decodeSharedSource({payload:new URL(location.href).searchParams.get("code"),urlCodePrefixGzip:"gz.",urlCodePrefixRaw:"raw."});
  })).toContain("profile: SurfaceProfile.Unlit");
  expect(await page.evaluate(() => window.gpuRuntimeErrors)).toEqual([]);
  await page.reload();
  await expect(page.locator("#compile-status")).toContainText("Compiled",{timeout:30000});
  await page.locator('[data-tab="inspector"]').click();
  await expect(page.locator('[data-engine-property="blend"]')).toHaveValue("SurfaceBlend.Translucent");
  await expect(page.locator('[data-engine-property="two_sided"]')).toHaveValue("true");
  await page.waitForFunction(() => Number(document.querySelector("#preview-canvas").dataset.engineFrames) > 3);
  expect(errors).toEqual([]);
  expect(await page.evaluate(() => window.gpuRuntimeErrors)).toEqual([]);
});

test("renaming engine channels, profile, and context preserves rendered pixels", async ({page}) => {
  const frames = [];
  for (const renames of [undefined, {albedo: "radiance", emissive: "glow", standard: "thermal", unlit: "flat", surf: "SampleContext"}]) {
    await page.goto("/tests/gpu/");
    await page.waitForFunction(() => window.gpuTest);
    await page.evaluate(({source, renames}) => window.gpuTest.init("mesh-default", source, undefined, renames), {
      source: "surface probe(sp: surf) -> material(standard) { compose { base(albedo: rgba(0.7, 0.2, 0.1, 1.0), emissive: vec3(0.1, 0.0, 0.0)) } }",
      renames,
    });
    await page.evaluate(() => window.gpuTest.frame("renamed", 0));
    frames.push(await page.evaluate(() => window.gpuTest.pixels("renamed")));
  }
  expect(frames[0].some((value, index) => index % 4 === 0 && value > 120)).toBe(true);
  expect(frames[1]).toEqual(frames[0]);
});
