# Particles

`torch.fr` is a continuously simulated torch flame with three particle families:
short-lived curling flame tongues, slowly rising and expanding smoke puffs, and
small ballistic embers. Birth IDs select stable variation; each particle has its
own position, velocity, age, and lifespan. The surface fades particles over their
lifetime and uses soft alpha blending with the three individual 512?512 sprites
in [assets/particles](../assets/particles/README.md).

Open `torch.fr` in the playground. Adjust `spawn_rate`, `flame_intensity`,
`smoke_opacity`, and `ember_intensity` to tune the effect. The default emitter
births 95 particles per second, starts with a 12-particle burst, and bounds its
pool using a 2.5-second maximum lifetime. The demo is the emitted effect; it does
not include a torch handle, scene-light injection, or inter-particle collisions.

`drifting_sparks.fr` is the smaller procedural-material introduction to emitter
`spawn` and `update` stacks.

The engine supplies reusable motion modules and the `camera_billboard` draw
module. Effects do not import engine files. `position.xyz` is world center and
`position.w` is sprite size. The draw contract exposes `sp.particle_age`,
`sp.particle_lifespan`, `sp.particle_id`, and `sp.particle_velocity` to materials.
Billboards face the camera, including camera roll. The host schedules births,
retains simulation state, and owns bounded allocation and playback; rendering
shares the preview environment and ground occlusion.

Local GPU verification and a 512?512 rendered preview:

```sh
cargo run -p fresco-example-engine --example offscreen -- --torch-only
```

The preview is written to `target/torch-preview.png`. This GPU probe is local-only.
