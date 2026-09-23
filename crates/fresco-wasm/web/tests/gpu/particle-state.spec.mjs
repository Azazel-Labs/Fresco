import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";

test("authored particle spawn, extended state, and explicit stepping survive GPU readback", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const source = readFileSync(new URL("../../../../../tests/fixtures/particle_contract.fr", import.meta.url), "utf8")
    .replace("@meta(capacity, 64)", "@meta(capacity, 129)")
    .replaceAll("@workgroup_size(64)", "@workgroup_size(32)")
    .replace("velocity: vec4\n", "velocity: vec4\n\theat: f32\n")
    .replace("cos(angle) * 0.12, 0.0, 0.0))", "cos(angle) * 0.12, 0.0, 0.0), heat: id / count)")
    .replace("return particle_integrate(p, dt)", "var next = particle_integrate(p, dt)\n next.heat = p.heat + dt\n return next")
    .replace("world_normal: vec3 in world\n", "world_normal: vec3 in world\n\t@location(3) heat: f32\n")
    .replace("vec3(0.0, 0.0, 1.0))", "vec3(0.0, 0.0, 1.0), p.heat)")
    .replace("return m.albedo", "return rgba(sp.heat, 0.2, 0.1, 1.0)");
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const details = await page.evaluate(source => window.gpuTest.init("particles", undefined, source), source);
  const contract = details.simulation;
  const state = contract.bindings.find(b => b.name === "particles");
  expect(Number(contract.metadata.capacity)).toBe(129);
  expect(state.element_stride).toBe(48);
  expect(state.fields.find(f => f.name === "heat").offset).toBe(32);
  await page.evaluate(() => window.gpuTest.frame("spawn", 0, 0));
  const initial = await page.evaluate(() => window.gpuTest.particleBytes());
  const floats = bytes => new DataView(Uint8Array.from(bytes).buffer);
  for (const id of [0, 1, 63, 64, 128]) {
    expect(floats(initial).getFloat32(id * 48 + 32, true)).toBeCloseTo(id / 129, 6);
  }
  await page.evaluate(() => window.gpuTest.particleStep(0.25, 3));
  const updated = await page.evaluate(() => window.gpuTest.particleBytes());
  for (const id of [0, 1, 63, 64, 128]) {
    expect(floats(updated).getFloat32(id * 48 + 32, true)).toBeCloseTo(id / 129 + 0.25, 6);
    const x = floats(initial).getFloat32(id * 48, true);
    const vx = floats(initial).getFloat32(id * 48 + 16, true);
    expect(floats(updated).getFloat32(id * 48, true)).toBeCloseTo(x + vx * 0.25, 6);
  }
  await page.evaluate(() => window.gpuTest.particleStep(0, 4));
  expect(await page.evaluate(() => window.gpuTest.particleBytes())).toEqual(updated);
  await page.evaluate(() => window.gpuTest.particleStep(0, 1, true));
  const reset = await page.evaluate(() => window.gpuTest.particleBytes());
  // Padding bytes are not authored state and need not have a portable value.
  for (let id = 0; id < 129; id++) {
    for (let component = 0; component < 9; component++) {
      const offset = id * 48 + component * 4;
      expect(floats(reset).getFloat32(offset, true)).toBe(floats(initial).getFloat32(offset, true));
    }
  }
  const snapshot = await page.evaluate(async () => {
    const pending = window.gpuTest.particleBytes();
    await window.gpuTest.particleStep(0.125);
    return await pending;
  });
  expect(snapshot).toEqual(reset);
  const afterSnapshot = await page.evaluate(() => window.gpuTest.particleBytes());
  expect(floats(afterSnapshot).getFloat32(32, true)).toBeCloseTo(0.125, 6);
  const png = await page.evaluate(() => window.gpuTest.png("spawn"));
  await testInfo.attach("authored-particle-state", { body: JSON.stringify(details), contentType: "application/json" });
  await testInfo.attach("particles", { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
});


test("typed particle modules preserve attributes and pass temporary records downstream", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const source = readFileSync(new URL("../../../../../tests/fixtures/particle_contract.fr", import.meta.url), "utf8")
    .replace("@meta(capacity, 64)", "@meta(capacity, 129)")
    .replaceAll("@workgroup_size(64)", "@workgroup_size(32)")
    .replace("velocity: vec4\n", "velocity: vec4\n\theat: f32\n")
    .replace("cos(angle) * 0.12, 0.0, 0.0))", "cos(angle) * 0.12, 0.0, 0.0), heat: id / count)")
    .replace("return particle_integrate(p, dt)", `
      let step = prepare_step(p)
      let falling = particle_gravity(step.particle, step.acceleration, dt)
      let heated = add_heat(falling, dt)
      let slowed = particle_drag(heated, 0.5, dt)
      var next = particle_integrate(slowed, dt)
      return collide(next)
    `)
    .replace("@pure fn main(p:", `@pure fn collide(p: PreviewParticle) -> PreviewParticle {
      var next = p
      if p.position.y < -2.0 {
        next.position = vec4(p.position.x, -2.0, p.position.z, p.position.w)
        next.velocity = vec4(p.velocity.x, abs(p.velocity.y) * 0.5, p.velocity.z, p.velocity.w)
      }
      return next
    }
    @pure fn main(p:`)
    + `
    struct ParticleStep { particle: PreviewParticle, acceleration: vec3 }
    fn prepare_step(p: PreviewParticle) -> ParticleStep {
      return ParticleStep(particle: p, acceleration: vec3(0.0, -2.0, 0.0))
    }
    fn add_heat<T>(p: T, dt: f32) -> T {
      var next = p
      next.heat = p.heat + dt
      return next
    }
    `;
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const details = await page.evaluate(source => window.gpuTest.init("particles", undefined, source), source);
  expect(details.simulation.bindings.find(b => b.name === "particles").element_stride).toBe(48);
  expect(details.simulation.bindings.find(b => b.name === "particles").fields.map(f => f.name)).toEqual(["position", "velocity", "heat"]);
  await page.evaluate(() => window.gpuTest.frame("modules", 0, 0));
  const view = bytes => new DataView(Uint8Array.from(bytes).buffer);
  const initial = view(await page.evaluate(() => window.gpuTest.particleBytes()));
  await page.evaluate(() => window.gpuTest.particleStep(0.25, 3));
  const updated = view(await page.evaluate(() => window.gpuTest.particleBytes()));
  const decay = Math.exp(-0.5 * 0.25);
  for (const id of [0, 1, 63, 64, 128]) {
    const offset = id * 48;
    const vx = initial.getFloat32(offset + 16, true) * decay;
    const vy = (initial.getFloat32(offset + 20, true) - 2 * 0.25) * decay;
    expect(updated.getFloat32(offset + 16, true)).toBeCloseTo(vx, 6);
    expect(updated.getFloat32(offset + 20, true)).toBeCloseTo(vy, 6);
    expect(updated.getFloat32(offset, true)).toBeCloseTo(initial.getFloat32(offset, true) + vx * 0.25, 6);
    expect(updated.getFloat32(offset + 4, true)).toBeCloseTo(initial.getFloat32(offset + 4, true) + vy * 0.25, 6);
    expect(updated.getFloat32(offset + 32, true)).toBeCloseTo(id / 129 + 0.25, 6);
  }
  await page.evaluate(() => window.gpuTest.particleStep(2.0));
  const collided = view(await page.evaluate(() => window.gpuTest.particleBytes()));
  expect(collided.getFloat32(4, true)).toBe(-2);
  expect(collided.getFloat32(20, true)).toBeGreaterThan(0);
  expect(collided.getFloat32(32, true)).toBeCloseTo(2.25, 6);
  await testInfo.attach("particle-module-contract", { body: JSON.stringify(details), contentType: "application/json" });
});


test("sample emitter activates, advances and recycles its own state", async ({ page }, testInfo) => {
  const source = readFileSync(new URL("../../../../../examples/50) particles/drifting_sparks.fr", import.meta.url), "utf8");
  await page.goto("/tests/gpu/");
  await page.waitForFunction(() => window.gpuTest);
  const details = await page.evaluate(source => window.gpuTest.init("particles", source), source);
  expect(Number(details.simulation.metadata.capacity)).toBe(128);
  await page.evaluate(() => window.gpuTest.frame("fountain-start", 0));
  const view = bytes => new DataView(Uint8Array.from(bytes).buffer);
  const initial = view(await page.evaluate(() => window.gpuTest.particleBytes()));
  expect(initial.getFloat32(127 * 48 + 32, true)).toBe(0);
  const births = await page.evaluate(() => window.gpuTest.particleSlots());
  expect(births.words.filter((_, i) => i % 4 === 2).reduce((a,b) => a+b,0)).toBe(8);
  await page.evaluate(() => window.gpuTest.particleStep(0.25));
  const active = view(await page.evaluate(() => window.gpuTest.particleBytes()));
  expect(active.getFloat32(4, true)).toBeGreaterThan(initial.getFloat32(4, true));
  expect(active.getFloat32(12, true)).toBeGreaterThan(0);
  expect(active.getFloat32(127 * 48 + 12, true)).toBe(0);
  await page.evaluate(async () => { for (let i = 0; i < 40; i++) await window.gpuTest.particleStep(0.1); });
  const recycled = view(await page.evaluate(() => window.gpuTest.particleBytes()));
  const leases = await page.evaluate(() => window.gpuTest.particleSlots());
  expect(leases.dropped).toBe(0);
  expect(Math.max(...leases.words.filter((_, i) => i % 4 === 0))).toBeGreaterThan(128);
  for (let id = 0; id < 128; id++) {
    if (!leases.words[id * 4 + 2]) continue;
    const age = recycled.getFloat32(id * 48 + 32, true);
    const lifespan = recycled.getFloat32(id * 48 + 36, true);
    expect(age).toBeGreaterThanOrEqual(0);
    expect(age).toBeLessThanOrEqual(2.00001);
    if (age >= lifespan) expect(recycled.getFloat32(id * 48 + 12, true)).toBe(0);
    expect(recycled.getFloat32(id * 48 + 40, true)).toBe(leases.words[id * 4]);
  }
  await page.evaluate(() => window.gpuTest.frame("fountain", 0.75, 0.75));
  const png = await page.evaluate(() => window.gpuTest.png("fountain"));
  await testInfo.attach("fountain", { body: Buffer.from(png.split(",")[1], "base64"), contentType: "image/png" });
});

test("emitter preview has no geometry picker and normal surfaces retain theirs", async ({ page }, testInfo) => {
  await page.goto("/?example=" + encodeURIComponent("50) particles/drifting_sparks"));
  await expect(page.locator("#compile-status")).toContainText("Compiled", { timeout: 30_000 });
  await expect(page.locator("#mesh-picker")).toBeHidden();
  await expect(page.locator('[data-render-mode="particles"]')).toHaveCount(0);
  const box = await page.locator("#preview-canvas").boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 60, box.y + box.height / 2 + 20);
  await page.mouse.up();
  await expect(page.locator("#preview-spin")).toBeHidden();
  await page.locator("#example-select").selectOption("40) surface shaders/uv_material", { force: true });
  await expect(page.locator("#mesh-picker")).toBeVisible();
  await page.locator("#example-select").selectOption("50) particles/drifting_sparks", { force: true });
  await expect(page.locator("#mesh-picker")).toBeHidden();
  await expect(page.locator("#preview-spin")).toBeHidden();
});

for (const mode of ["Estimated", "Automatic"]) {
  test(`${mode} allocation grows GPU storage preserving live state`, async ({page}) => {
    const source = readFileSync(new URL("../../../../../examples/50) particles/drifting_sparks.fr", import.meta.url), "utf8")
      .replace("spawn_rate: 60.0", `allocation: ParticleAllocationMode.${mode}\n    allocation_hint: 8\n    max_particles: 32\n    simulate_motion: false\n    spawn_rate: 60.0`);
    await page.goto("/tests/gpu/"); await page.waitForFunction(() => window.gpuTest);
    await page.evaluate(source => window.gpuTest.init("particles", source), source);
    await page.evaluate(() => window.gpuTest.particleStep(0));
    const initial = await page.evaluate(() => window.gpuTest.particleBytes());
    await page.evaluate(() => window.gpuTest.particleStep(.25));
    const grown = await page.evaluate(() => window.gpuTest.particleBytes());
    expect(grown.length).toBe(32 * 48);
    expect(grown.slice(0,32)).toEqual(initial.slice(0,32));
    expect(new DataView(Uint8Array.from(grown).buffer).getFloat32(32,true)).toBeCloseTo(.25,6);
    await page.evaluate(() => window.gpuTest.particleStep(.25));
    expect((await page.evaluate(() => window.gpuTest.particleSlots())).dropped).toBeGreaterThan(0);
    await page.evaluate(() => window.gpuTest.particleStep(0,1,true));
    expect((await page.evaluate(() => window.gpuTest.particleSlots())).capacity).toBe(mode === "Automatic" ? 32 : 8);
  });
}

test("engine property choices edit saved effect source", async ({page}, testInfo) => {
  await page.goto("/?example=" + encodeURIComponent("50) particles/drifting_sparks"));
  await expect(page.locator("#compile-status")).toContainText("Compiled", {timeout:30000});
  await page.locator('[data-tab="inspector"]').click();
  await page.locator('[data-engine-property="simulate_motion"]').selectOption("false");
  await expect(page.locator('[data-engine-property="simulate_motion"]')).toBeEnabled({timeout:30000});
  await expect(page.locator('[data-engine-property="simulate_motion"]')).toHaveValue("false");
  await page.locator('[data-engine-property="allocation"]').selectOption("ParticleAllocationMode.Automatic");
  await expect(page.locator('[data-engine-property="allocation"]')).toBeEnabled({timeout:30000});
  await expect(page.locator('[data-engine-property="allocation"]')).toHaveValue("ParticleAllocationMode.Automatic");
  await expect.poll(() => new URL(page.url()).searchParams.has("code")).toBe(true);
  // URL synchronization is debounced; confirm the shared source includes both edits.
  await expect.poll(async () => {
    const url = new URL(page.url());
    return page.evaluate(async payload => {
      const { decodeSharedSource } = await import("/src/app/source-url-codec.ts");
      return decodeSharedSource({payload, urlCodePrefixGzip: "gz.", urlCodePrefixRaw: "raw."});
    }, url.searchParams.get("code"));
  }).toContain("allocation: ParticleAllocationMode.Automatic");
  await page.reload();
  await expect(page.locator("#compile-status")).toContainText("Compiled", {timeout:30000});
  await page.locator('[data-tab="inspector"]').click();
  await expect(page.locator('[data-engine-property="simulate_motion"]')).toHaveValue("false");
  await expect(page.locator('[data-engine-property="allocation"]')).toHaveValue("ParticleAllocationMode.Automatic");
});


test("engine-defined entry and module blocks highlight as contextual keywords", async ({page}, testInfo) => {
  await page.goto("/?example=" + encodeURIComponent("50) particles/drifting_sparks"));
  await expect(page.locator("#compile-status")).toContainText("Compiled", {timeout:30000});
  for (const word of ["emitter", "spawn", "update"]) {
    await expect(page.locator("#editor .cm-fresco-keyword").filter({hasText: new RegExp(`^${word}$`)}).first()).toBeVisible();
  }
});
