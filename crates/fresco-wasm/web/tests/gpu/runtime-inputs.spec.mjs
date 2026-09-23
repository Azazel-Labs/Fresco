import { test, expect } from "@playwright/test";

async function setup(page, kind, testInfo) {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const result = await page.evaluate(kind => window.gpuTest.init(kind), kind);
  expect(result.uniforms).toEqual(expect.arrayContaining([
    expect.objectContaining({ name: "frame", group: 3, binding: 0 }),
  ]));
  await testInfo.attach("gpu-adapter", { body: JSON.stringify({ browserVersion: page.context().browser().version(), ...result }, null, 2), contentType: "application/json" });
  return errors;
}

async function frame(page, name, time, delta = 0, width = 256, height = 256, draws = 1) {
  return page.evaluate(args => window.gpuTest.frame(...args), [name, time, delta, width, height, draws]);
}

async function compare(page, a, b) {
  return page.evaluate(([a, b]) => window.gpuTest.compare(a, b), [a, b]);
}

async function attachFrames(page, testInfo, names) {
  for (const name of names) {
    const png = await page.evaluate(name => window.gpuTest.png(name), name);
    await testInfo.attach(name, { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
  }
}

test("badge animation changes rendered pixels and seeking reproduces the frame", async ({ page }, testInfo) => {
  const errors = await setup(page, "badge", testInfo);
  await frame(page, "badge-0", 0);
  await frame(page, "badge-2", 2);
  await frame(page, "badge-0-again", 0);
  const motion = await compare(page, "badge-0", "badge-2");
  const repeat = await compare(page, "badge-0", "badge-0-again");
  expect(motion.changedFraction).toBeGreaterThan(0.025);
  expect(repeat.maxDifference).toBeLessThanOrEqual(1);
  await testInfo.attach("pixel-measurements", { body: JSON.stringify({ motion, repeat }), contentType: "application/json" });
  await attachFrames(page, testInfo, ["badge-0", "badge-2", "badge-0-again"]);
  expect(errors).toEqual([]);
});

test("editing the engine-authored shade hook changes the executed fragment stage", async ({ page }, testInfo) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const original = await page.evaluate(() => window.gpuTest.init("pass-default"));
  expect(original.enginePass?.vertex_entry).toContain("fresco_pass_present_canvas");
  expect(original.enginePass?.fragment_entry).toContain("fresco_pass_present_canvas");
  const originalFrame = await page.evaluate(() => window.gpuTest.frame("authored-default", 0));

  await page.reload();
  await page.waitForFunction(() => window.gpuTest);
  const edited = await page.evaluate(() => window.gpuTest.init("pass-edited"));
  expect(edited.enginePass).toEqual(original.enginePass);
  const editedFrame = await page.evaluate(() => window.gpuTest.frame("authored-edited", 0));

  await page.reload();
  await page.waitForFunction(() => window.gpuTest);
  const vertexEdited = await page.evaluate(() => window.gpuTest.init("pass-vertex-edited"));
  expect(vertexEdited.enginePass).toEqual(original.enginePass);
  const vertexEditedFrame = await page.evaluate(() => window.gpuTest.frame("authored-vertex-edited", 0));

  expect(originalFrame.center[0]).toBeGreaterThan(110);
  expect(originalFrame.center[1]).toBeGreaterThan(110);
  expect(editedFrame.center[0]).toBeGreaterThanOrEqual(60);
  expect(editedFrame.center[0]).toBeLessThanOrEqual(68);
  expect(editedFrame.center[1]).toBeGreaterThanOrEqual(187);
  expect(editedFrame.center[1]).toBeLessThanOrEqual(195);
  expect(vertexEditedFrame.center[0]).toBeGreaterThanOrEqual(28);
  expect(vertexEditedFrame.center[0]).toBeLessThanOrEqual(36);
  expect(vertexEditedFrame.center[1]).toBeGreaterThanOrEqual(219);
  expect(vertexEditedFrame.center[1]).toBeLessThanOrEqual(227);
  await testInfo.attach("authored-pass-measurements", {
    body: JSON.stringify({ original, edited, vertexEdited, originalFrame, editedFrame, vertexEditedFrame }, null, 2),
    contentType: "application/json",
  });
  expect(errors).toEqual([]);
});

test("editing engine-authored mesh vertex and material hooks changes GPU output", async ({ page }, testInfo) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const original = await page.evaluate(() => window.gpuTest.init("mesh-default"));
  expect(original.meshPass?.entries.find(entry => entry.stage === "vertex")?.entry).toBe("fresco_mesh_preview_mesh_mesh_probe_preview_static_vertex");
  expect(original.meshPass?.entries.find(entry => entry.function === "raster")?.entry).toBe("fresco_mesh_preview_mesh_mesh_probe_preview_static_raster");
  await page.evaluate(() => window.gpuTest.frame("mesh-default", 0));

  const fragmentEdited = await page.evaluate(() => window.gpuTest.init("mesh-fragment-edited"));
  expect(fragmentEdited.meshPass).toEqual(original.meshPass);
  await page.evaluate(() => window.gpuTest.frame("mesh-fragment-edited", 0));

  const vertexEdited = await page.evaluate(() => window.gpuTest.init("mesh-vertex-edited"));
  expect(vertexEdited.meshPass).toEqual(original.meshPass);
  await page.evaluate(() => window.gpuTest.frame("mesh-vertex-edited", 0));

  const fragmentDifference = await compare(page, "mesh-default", "mesh-fragment-edited");
  const vertexDifference = await compare(page, "mesh-default", "mesh-vertex-edited");
  expect(fragmentDifference.changedFraction).toBeGreaterThan(0.05);
  expect(vertexDifference.changedFraction).toBeGreaterThan(0.02);
  await testInfo.attach("authored-mesh-measurements", {
    body: JSON.stringify({ original, fragmentEdited, vertexEdited, fragmentDifference, vertexDifference }, null, 2),
    contentType: "application/json",
  });
  expect(errors).toEqual([]);
});

test("engine-authored particle compute and raster hooks execute on the GPU", async ({ page }, testInfo) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const original = await page.evaluate(() => window.gpuTest.init("particles"));
  expect(original.technique.steps.find(s => s.name === "update").operation.entry).toContain("_simulate");
  expect(original.technique.steps.find(s => s.name === "draw").operation.vertex).toContain("_vertex");
  await frame(page, "particles-default", 1, 1 / 60, 256, 256, 24);

  const simEdited = await page.evaluate(() => window.gpuTest.init("particle-sim-edited"));
  expect(simEdited.technique).toEqual(original.technique);
  await frame(page, "particles-sim-edited", 1, 1 / 60, 256, 256, 24);

  const shadeEdited = await page.evaluate(() => window.gpuTest.init("particle-shade-edited"));
  expect(shadeEdited.technique).toEqual(original.technique);
  await frame(page, "particles-shade-edited", 1, 1 / 60, 256, 256, 24);

  const simulationDifference = await compare(page, "particles-default", "particles-sim-edited");
  const shadeDifference = await compare(page, "particles-default", "particles-shade-edited");
  expect(simulationDifference.changedFraction).toBeGreaterThan(0.005);
  expect(shadeDifference.changedFraction).toBeGreaterThan(0.005);
  await testInfo.attach("particle-pipeline-measurements", {
    body: JSON.stringify({ original, simEdited, shadeEdited, simulationDifference, shadeDifference }, null, 2),
    contentType: "application/json",
  });
  expect(errors).toEqual([]);
});

for (const kind of ["canvas", "surface"]) {
  test(`${kind}: time, delta and resized resolution independently reach rendered pixels`, async ({ page }, testInfo) => {
    const errors = await setup(page, kind, testInfo);
    const base = await frame(page, "base", 1, 0.125);
    const time = await frame(page, "time", 2, 0.125);
    const delta = await frame(page, "delta", 2, 0.5);
    const resized = await frame(page, "resolution", 2, 0.5, 512, 256);
    for (const [actual, expected] of [[base, [64, 32, 64]], [time, [128, 32, 64]],
      [delta, [128, 128, 64]], [resized, [128, 128, 128]]]) {
      for (let c = 0; c < 3; c++) expect(Math.abs(actual.center[c] - expected[c])).toBeLessThanOrEqual(2);
    }
    for (const [a, b, channel] of [[base, time, 0], [time, delta, 1], [delta, resized, 2]]) {
      expect(b.center[channel] - a.center[channel]).toBeGreaterThan(20);
      for (const other of [0, 1, 2].filter(c => c !== channel)) {
        expect(Math.abs(b.center[other] - a.center[other])).toBeLessThanOrEqual(2);
      }
      expect(b.center[3]).toBe(255);
    }
    expect([resized.width, resized.height]).toEqual([512, 256]);
    await testInfo.attach("pixel-measurements", { body: JSON.stringify({ base, time, delta, resized }, null, 2), contentType: "application/json" });
    await attachFrames(page, testInfo, ["base", "time", "delta", "resolution"]);
    expect(errors).toEqual([]);
  });
}

test("wheel zoom in and out preserves rendered surface spin; pause stops it", async ({ page }, testInfo) => {
  const errors = await setup(page, "orbit", testInfo);
  const before = await frame(page, "before", 0);
  await page.locator("#preview").hover();
  await page.mouse.wheel(0, -120);
  await page.waitForFunction(distance => window.gpuTest.distance() < distance, before.distance);
  const zoomIn = await frame(page, "zoom-in", 0);
  expect(zoomIn.distance).toBeLessThan(before.distance);
  expect(zoomIn.orbitAuto).toBe(true);
  const zoomChange = await compare(page, "before", "zoom-in");
  expect(zoomChange.changedFraction).toBeGreaterThan(0.01);
  await frame(page, "spin-in", 0, 0, 256, 256, 60);
  const spinIn = await compare(page, "zoom-in", "spin-in");
  expect(spinIn.changedFraction).toBeGreaterThan(0.01);
  await page.mouse.wheel(0, 120);
  await page.waitForFunction(distance => window.gpuTest.distance() > distance, zoomIn.distance);
  const zoomOut = await frame(page, "zoom-out", 0);
  expect(zoomOut.distance).toBeCloseTo(before.distance, 4);
  expect(zoomOut.orbitAuto).toBe(true);
  await frame(page, "spin-out", 0, 0, 256, 256, 60);
  const spinOut = await compare(page, "zoom-out", "spin-out");
  expect(spinOut.changedFraction).toBeGreaterThan(0.01);
  await page.evaluate(() => window.gpuTest.pause());
  await frame(page, "paused", 0);
  await frame(page, "paused-again", 0, 0, 256, 256, 60);
  const paused = await compare(page, "paused", "paused-again");
  expect(paused.maxDifference).toBeLessThanOrEqual(1);
  await testInfo.attach("pixel-measurements", { body: JSON.stringify({ before, zoomIn, zoomOut, zoomChange, spinIn, spinOut, paused }, null, 2), contentType: "application/json" });
  await attachFrames(page, testInfo, ["before", "zoom-in", "spin-in", "zoom-out", "spin-out", "paused"]);
  expect(errors).toEqual([]);
});
