import { describe, it, expect } from "vitest";
import { ParticlePool, type ParticleAllocation } from "../../tests/reference/particle-pool";
import { editEngineProperty, type EngineProperty } from "../app/engine-properties";
const policy: ParticleAllocation = { mode: "fixed", initial_capacity: 2, max_capacity: 8, growth_factor: 2, spawn_rate: 2, spawn_burst: 2, max_lifespan: 1, max_spawn_per_step: 100, overflow: "drop_new" };
const active = (p: ParticlePool) => Array.from(new Uint32Array(p.commands)).filter((_, i) => i % 4 === 2).reduce((a,b) => a+b,0);
describe("particle lifetime reservations", () => {
  it("bounds fixed storage and reuses expired slots with new birth identities", () => {
    const p = new ParticlePool(policy); p.advance(0); expect(active(p)).toBe(2);
    p.advance(.5); expect(p.dropped).toBe(1);
    p.advance(.5); expect(active(p)).toBe(1);
    expect(new Uint32Array(p.commands)[4]).toBe(3);
    expect(p.capacity).toBe(2);
  });
  it.each(["estimated", "automatic"] as const)("grows %s without losing existing commands and obeys the limit", mode => {
    const p = new ParticlePool({...policy, mode, spawn_burst: 10}); p.advance(0);
    expect(p.capacity).toBe(8); expect(active(p)).toBe(8); expect(p.dropped).toBe(2);
    expect(new Uint32Array(p.commands)[0]).toBe(0);
    p.reset(); expect(p.capacity).toBe(mode === "automatic" ? 8 : 2); expect(active(p)).toBe(0);
  });
  it("assigns partial frame time to new births", () => {
    const p = new ParticlePool({...policy, spawn_burst: 0, spawn_rate: 4}); p.advance(.375);
    expect(active(p)).toBe(1); expect(new Float32Array(p.commands)[3]).toBe(.125);
  });
  it("bounds per-step work and rejects invalid elapsed time", () => {
    const p = new ParticlePool({...policy, spawn_burst: 100, max_spawn_per_step: 1}); p.advance(0);
    expect(active(p)).toBe(1); expect(p.dropped).toBe(99);
    expect(() => p.advance(-1)).toThrow(); expect(() => p.advance(Infinity)).toThrow();
  });
});
describe("source-backed properties", () => {
  const property: EngineProperty = { entry: "effect", name: "rate", ty: "f32", value: 2, choices: [], permutation: false, value_span: null, insert_at: 0 };
  it("replaces UTF-8 byte ranges without changing surrounding source", () => {
    const source = "// ??\nrate: 2.0\n"; const start = new TextEncoder().encode(source.split("2.0")[0]).length;
    expect(editEngineProperty(source, {...property, value_span: {start, end: start+3}}, "4.0")).toBe(source.replace("2.0", "4.0"));
  });
  it("materializes engine defaults in the authored entry", () => {
    expect(editEngineProperty("spawn {}", property, "3")).toBe("rate: 3\n    spawn {}");
    expect(() => editEngineProperty("", {...property, insert_at: 1}, "3")).toThrow("stale");
  });
});

it("inserts a surface property block and separates new fields in existing blocks", () => {
  const property: EngineProperty = {entry: "leaf", name: "two_sided", ty: "bool", value: 0, choices: [], permutation: true, value_span: null, insert_at: 14, block: "properties", block_present: false};
  const source = "surface leaf { compose {} }";
  const edited = editEngineProperty(source, property, "true");
  expect(edited).toContain("properties {\n        two_sided: true");
  const existing = "properties { blend: Masked }";
  expect(editEngineProperty(existing, {...property, block_present: true, insert_at: existing.length-1}, "true")).toContain("Masked \n        two_sided: true");
});
