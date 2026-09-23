import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";

const source = profile => `
emitter light_probe {
    spawn_rate: 0.0
    burst_count: 1
    max_lifespan: 2.0
    spawn {
        particle_fountain(vec3(0.0), 0.0, 0.0, id)
        particle_fade_size(1.0)
    }
    update { particle_integrate(dt) }
}
surface sprite(sp: surf) -> material(${profile}) {
    properties { two_sided: true }
    compose { base(albedo: rgba(1.0, 0.0, 0.0, 1.0)) }
}`;

test("lit billboards use camera-facing normals while unlit billboards retain their color", async ({ page }, testInfo) => {
  const peaks = {};
  for (const profile of ["standard", "unlit"]) {
    await page.goto("/tests/gpu/");
    await page.waitForFunction(() => window.gpuTest);
    await page.evaluate(source => window.gpuTest.init("particles", source), source(profile));
    peaks[profile] = [];
    for (const [name, yaw] of [["front", 0], ["back", Math.PI]]) {
      await page.evaluate(yaw => window.gpuTest.camera(yaw, 0), yaw);
      await page.evaluate(name => window.gpuTest.frame(name, 0, 0), name);
      const pixels = await page.evaluate(name => window.gpuTest.pixels(name), name);
      peaks[profile].push(pixels.reduce((peak, value, i) => i % 4 === 0 ? Math.max(peak, value) : peak, 0));
      const png = await page.evaluate(name => window.gpuTest.png(name), name);
      await testInfo.attach(`${profile}-${name}`, { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
    }
  }
  expect(Math.abs(peaks.standard[0] - peaks.standard[1])).toBeGreaterThan(70);
  expect(peaks.unlit[0]).toBeGreaterThan(240);
  expect(peaks.unlit[1]).toBeGreaterThan(240);
  expect(Math.abs(peaks.unlit[0] - peaks.unlit[1])).toBeLessThan(3);
});

test("camera module preserves billboard area at steep orbit angles", async ({ page }, testInfo) => {
  const source = readFileSync(new URL("../../../../../examples/50) particles/drifting_sparks.fr", import.meta.url), "utf8")
    .replace("particle_fountain(vec3(0.0, -0.65, 0.0), 0.35, 1.0, id)", "billboard_probe()")
    + "\nfn billboard_probe<T>(p: T) -> T { var next = p; next.position = vec4(0.0, 0.0, 0.0, 0.4); return next }\n";
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  await page.evaluate(source => window.gpuTest.init("particles", source), source);
  const areas = [];
  for (const [yaw, pitch] of [[0, 0], [1.55, 0], [1.55, 1.2]]) {
    await page.evaluate(([yaw, pitch]) => window.gpuTest.camera(yaw, pitch), [yaw, pitch]);
    await page.evaluate(() => window.gpuTest.frame("billboard", 0));
    const pixels = await page.evaluate(() => window.gpuTest.pixels("billboard"));
    let area = 0;
    for (let i = 0; i < pixels.length; i += 4) if (pixels[i] > 200 && pixels[i + 2] < 200) area++;
    expect(area).toBeGreaterThan(100);
    areas.push(area);
  }
  expect(Math.max(...areas) / Math.min(...areas)).toBeLessThan(1.05);
  const png = await page.evaluate(() => window.gpuTest.png("billboard"));
  await testInfo.attach("steep-angle-billboard", { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
});
