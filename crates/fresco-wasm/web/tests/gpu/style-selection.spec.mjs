import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";

const toon = readFileSync(new URL("../../../../../examples/40) surface shaders/style_sample.fr", import.meta.url), "utf8");
const fur = readFileSync(new URL("../../../../../examples/40) surface shaders/style_sample_fur.fr", import.meta.url), "utf8");

for (const renderer of ["forward", "forward-plus", "deferred"]) {
  test(`imported styles rebuild resource hooks and preserve failed candidates on ${renderer}`, async ({page}) => {
    test.setTimeout(180_000);
    await page.goto("/tests/gpu/");
    await page.waitForFunction(() => window.gpuTest);
    await page.evaluate(() => window.gpuTest.init("mesh-default"));
    await page.evaluate(() => window.gpuTest.mesh("sphere"));
    await page.evaluate(() => window.gpuTest.frame("initial", 0));
    const split = toon.indexOf("surface style_sample");
    const external = toon.slice(0, split).replaceAll("Toon", "ExternalBands");
    const source = `import "external/bands.fr"\n${toon.slice(split).replaceAll("Toon", "ExternalBands")}`;
    const install = (main, overrides = {}, extra = {}) => page.evaluate(({files, renderer, entry}) =>
      window.gpuTest.installBundle(files, renderer, entry), {
        files: {"main.fr": main, "external/bands.fr": external, "fresco.config.json": JSON.stringify({property_overrides: overrides}), ...extra},
        renderer, entry: main.includes("chestnut_fur") ? "chestnut_fur" : "style_sample",
      });
    const bands = await install(source);
    expect(bands.ok).toBe(true);
    const selection = bands.manifest.surfaces.find(s => s.name === "style_sample").settings.implementations[0];
    expect(selection.symbol).toBe("ExternalBands");
    expect(selection.availability.find(c => c.symbol === "ExternalBands").supported).toBe(true);
    await page.evaluate(() => window.gpuTest.frame("bands", 0));
    expect((await page.evaluate(() => window.gpuTest.pixels("bands"))).some(v => v > 100)).toBe(true);
    const disabled = await install(source, {style_sample: {style: {symbol: "ExternalBands", settings: {outline_enabled: false}}}});
    expect(disabled.ok).toBe(true);
    expect(disabled.manifest.renderers.find(r => r.selected).steps.every(s => !s.invocation)).toBe(true);
    await page.evaluate(() => window.gpuTest.frame("without-outline", 0));
    expect((await page.evaluate(() => window.gpuTest.compare("bands", "without-outline"))).changedFraction).toBeGreaterThan(0.0001);
    const legacy = await install(`${source}
@contribute(StandardStyle, ExternalBands, after_opaque) pipeline(postprocess) obsolete { preview_mesh }`);
    expect(legacy.ok).toBe(false);
    expect(legacy.diagnostics.some(d => d.message.includes("style integration was removed"))).toBe(true);
    const failed = await install(source, {style_sample: {two_sided: true}});
    expect(failed.ok).toBe(false);
    expect(failed.diagnostics.some(d => /precondition/.test(d.message))).toBe(true);
    await page.evaluate(() => window.gpuTest.frame("after-failure", 0));
    expect(await page.evaluate(() => window.gpuTest.compare("without-outline", "after-failure"))).toEqual({changedFraction: 0, maxDifference: 0});

    // An imported compute-backed style shares the scene with the engine's ground material.
    const furStart = fur.indexOf("surface chestnut_fur");
    const furModules = {"external/fibers.fr": fur.slice(0, furStart).replaceAll("MeadowFur", "ExternalFibers")};
    const furMain = `import "external/fibers.fr"\n${fur.slice(furStart).replaceAll("MeadowFur", "ExternalFibers")}`;
    const fibers = await install(furMain, {}, furModules);
    expect(fibers.ok).toBe(true);
    expect(fibers.manifest.surfaces.some(s => s.name === "fresco_scene_ground")).toBe(true);
    expect(fibers.manifest.gpu_programs.filter(p => p.compute_invocation).length).toBe(4);
    await page.evaluate(() => window.gpuTest.frame("fibers", 0));
    const changed = await install(furMain, {chestnut_fur: {style: {symbol: "ExternalFibers", settings: {layers: 4, seed: 37}}}}, furModules);
    expect(changed.ok).toBe(true);
    const reflected = changed.manifest.surfaces.find(s => s.name === "chestnut_fur").settings.implementations[0];
    expect(reflected.static_parameters.find(p => p.name === "layers").default).toBe(4);
    expect(reflected.availability.find(c => c.symbol === "ExternalFibers").supported).toBe(true);
    await page.evaluate(() => window.gpuTest.frame("changed-fibers", 0));
    expect((await page.evaluate(() => window.gpuTest.compare("fibers", "changed-fibers"))).changedFraction).toBeGreaterThan(0.001);
  });
}

test("editor keeps symbolic style and static settings through renderer rebuilds", async ({page}) => {
  test.setTimeout(180_000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?example=" + encodeURIComponent("40) surface shaders/style_sample"));
  await expect(page.locator("#compile-status")).toContainText("Compiled", {timeout: 60_000});
  await expect(page.locator("#preview-freshness")).toBeHidden();
  const style = page.getByLabel("style_sample style", {exact: true});
  const outline = page.getByLabel("style_sample style.outline_enabled", {exact: true});
  await expect(style).toHaveValue("Toon");
  await outline.uncheck();
  await expect.poll(() => page.evaluate(() => JSON.parse(document.querySelector("#manifest").textContent)
    .surfaces.find(s => s.name === "style_sample").settings.implementations[0].static_parameters.find(p => p.name === "outline_enabled").default)).toBe(false);
  for (const mode of ["deferred", "forward-plus", "forward"]) {
    await page.locator("#preview-renderer").selectOption(mode);
    await expect.poll(() => page.evaluate(() => JSON.parse(document.querySelector("#manifest").textContent).renderers.find(r => r.selected).id)).toBe(mode);
    await expect(style).toHaveValue("Toon");
    await expect(outline).not.toBeChecked();
  }
  await style.selectOption("StandardGGX");
  await expect.poll(() => page.evaluate(() => JSON.parse(document.querySelector("#manifest").textContent)
    .surfaces.find(s => s.name === "style_sample").settings.implementations[0].symbol)).toBe("StandardGGX");
  await style.selectOption("Toon");
  await expect(outline).toBeChecked();
  await page.waitForFunction(() => Number(document.querySelector("#preview-canvas").dataset.engineFrames) > 3);
  const frames = Number(await page.locator("#preview-canvas").getAttribute("data-engine-frames"));
  await page.locator('[data-tab="inspector"]').click();
  await expect(page.getByText("Surface Profile", {exact: true})).toHaveCount(0);
  await page.getByRole("combobox", {name: /style_sample.*two_sided/}).selectOption("true");
  await expect(page.locator("#compile-status")).toContainText("Compile Failed", {timeout: 60_000});
  await expect(page.locator("#preview-freshness")).toHaveText("Preview out of date");
  await expect(page.locator("#preview-freshness")).toHaveAttribute("data-state", "error");
  await expect(page.locator("#preview-canvas")).toBeVisible();
  await expect.poll(async () => Number(await page.locator("#preview-canvas").getAttribute("data-engine-frames"))).toBeGreaterThan(frames);
  await expect(style).toHaveValue("Toon");
  await style.selectOption("StandardGGX");
  await expect(page.locator("#compile-status")).toContainText("Compiled", {timeout: 60_000});
  await expect(style).toHaveValue("StandardGGX");
  await expect(style.locator('option[value="Toon"]')).toContainText("unavailable");
  await style.selectOption("Toon");
  await expect(page.locator("#compile-status")).toContainText("Compile Failed", {timeout: 60_000});
  await outline.uncheck();
  await expect(page.locator("#compile-status")).toContainText("Compiled", {timeout: 60_000});
  await expect(style).toHaveValue("Toon");
  await expect(outline).not.toBeChecked();
  await expect(page.locator("#preview-freshness")).toBeHidden();
  expect(errors).toEqual([]);
});
