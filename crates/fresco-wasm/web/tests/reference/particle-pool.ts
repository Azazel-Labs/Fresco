// Independent CPU test reference; excluded from production.
export type ParticleAllocation = {
  mode: "fixed" | "estimated" | "automatic";
  initial_capacity: number; max_capacity: number; growth_factor: number;
  spawn_rate: number; spawn_burst: number; max_lifespan: number;
  max_spawn_per_step: number; overflow: "drop_new";
};

/** Host-side conservative lifetime reservations; no synchronous GPU readback. */
export class ParticlePool {
  capacity: number;
  commands: ArrayBuffer;
  dropped = 0;
  peak = 0;
  private time = 0;
  private fraction = 0;
  private nextId = 0;
  private first = true;
  private free: number[] = [];
  private leases: { slot: number; end: number }[] = [];
  private head = 0;
  constructor(readonly policy: ParticleAllocation) {
    this.capacity = policy.initial_capacity;
    this.commands = new ArrayBuffer(this.capacity * 16);
    this.reset();
  }
  reset() {
    if (this.policy.mode !== "automatic") this.capacity = this.policy.initial_capacity;
    this.time = 0; this.fraction = 0; this.nextId = 0; this.first = true; this.dropped = 0;
    this.leases = []; this.head = 0;
    this.free = Array.from({ length: this.capacity }, (_, i) => this.capacity - 1 - i);
    this.commands = new ArrayBuffer(this.capacity * 16);
  }
  private expire(time: number) {
    const words = new Uint32Array(this.commands);
    while (this.head < this.leases.length && this.leases[this.head].end <= time) {
      const { slot } = this.leases[this.head++];
      words[slot * 4 + 2] = 0;
      this.free.push(slot);
    }
    if (this.head > 4096 && this.head * 2 > this.leases.length) {
      this.leases = this.leases.slice(this.head); this.head = 0;
    }
  }
  advance(dt: number) {
    if (!Number.isFinite(dt) || dt < 0) throw new Error("Particle scheduling requires finite non-negative time");
    let words = new Uint32Array(this.commands);
    let floats = new Float32Array(this.commands);
    for (let i = 0; i < this.capacity; i++) { words[i * 4 + 1] = 0; floats[i * 4 + 3] = dt; }
    const end = this.time + dt;
    const accumulated = this.fraction + dt * this.policy.spawn_rate;
    const continuous = Math.floor(accumulated);
    const burst = this.first ? this.policy.spawn_burst : 0;
    const requested = continuous + burst;
    if (!Number.isSafeInteger(requested) || this.nextId + requested > 16777215) {
      throw new Error("Particle birth IDs exceed the current f32 ABI; reset the simulation");
    }
    const accepted = Math.min(requested, this.policy.max_spawn_per_step);
    for (let i = 0; i < accepted; i++) {
      const birth = i < burst ? this.time : this.time + (i - burst + 1 - this.fraction) / this.policy.spawn_rate;
      this.expire(birth);
      if (this.free.length === 0 && this.policy.mode !== "fixed" && this.capacity < this.policy.max_capacity) {
        const old = this.capacity;
        this.capacity = Math.min(this.policy.max_capacity, Math.max(old + 1, Math.ceil(old * this.policy.growth_factor)));
        const next = new ArrayBuffer(this.capacity * 16);
        new Uint8Array(next).set(new Uint8Array(this.commands)); this.commands = next;
        words = new Uint32Array(next); floats = new Float32Array(next);
        for (let slot = this.capacity - 1; slot >= old; slot--) this.free.push(slot);
      }
      const slot = this.free.pop();
      if (slot === undefined) { this.dropped++; continue; }
      words[slot * 4] = this.nextId + i;
      words[slot * 4 + 1] = 1; words[slot * 4 + 2] = 1;
      floats[slot * 4 + 3] = end - birth;
      this.leases.push({ slot, end: birth + this.policy.max_lifespan });
      this.peak = Math.max(this.peak, this.leases.length - this.head);
    }
    this.dropped += requested - accepted;
    this.nextId += requested; this.fraction = accumulated - continuous;
    this.first = false; this.time = end; this.expire(end);
    return this.commands;
  }
}
