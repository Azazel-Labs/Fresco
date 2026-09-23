# Compiler engine fixtures

These authored Fresco inputs belong to compiler tests. They are independent of
the example engine and must not be refreshed by copying its source tree.

- `policy/engine/engine.fr` supplies the explicit rendering policy and minimal
  surface vocabulary used by existing compiler regressions. Its initial contents
  were extracted from the repository's compiler regression fixture.
- `particle.fr` exercises executable particle contracts and reflected state.
- `fullscreen.fr` exercises structured vertex/fragment hook parsing.
- `staged.fr` exercises staged hook token retention, nested branches, and loops.
- `prelude.fr` retains the authored helper declarations needed for documentation,
  overload resolution, and helper-lowering regressions, without host frame data.

`src/test_support.rs` supplies the policy through an explicit virtual source map.
Ordinary compiler tests must not discover the sample engine from a source path.
Loader-specific tests may create their own isolated on-disk engine directories.

Keep assertions about the compiler here. Assertions about the shipped engine's
particular shading, material defaults, or rendering belong to the sample engine.
The fixtures are deliberately not a runnable renderer profile.
