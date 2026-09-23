# Fresco versions

The authoritative release version is `workspace.package.version` in the root
`Cargo.toml`. All Rust packages inherit it. The CLI's `--version` and the
playground's version label report the version compiled into their binary; the
playground queries the loaded WASM compiler, so an older bundle reports its actual
version rather than the version of the page serving it.

Fresco is currently on the **0.2** development line. Version 1.0 is reserved for
a deliberate language/API stability commitment.

From the repository root, use:

```sh
node scripts/version.mjs patch   # fixes: 0.2.0 -> 0.2.1
node scripts/version.mjs minor   # features or pre-1.0 breaking changes: 0.2.x -> 0.3.0
node scripts/version.mjs major   # deliberate stable milestone: 0.x.y -> 1.0.0
node scripts/version.mjs check   # verify package/dependency versions and lockfiles
```

The bump command updates the workspace version, internal dependency requirements,
web package metadata, and both lockfiles. It requires Node and Cargo and may need
registry access when dependencies are not cached. It does not commit, tag, or
publish anything. If a refresh fails, resolve the reported error and run
`node scripts/version.mjs sync` to finish synchronizing the chosen version.

Choose a bump once per release, not once per build or commit. Before release:

1. Run the appropriate bump command and review its diff.
2. Run `cargo xtask ci-strict` and the web tests/build (`npm run test:unit` and
   `npm run build` in `crates/fresco-wasm/web`).
3. Commit the version changes with the release changes, then tag the release
   `v<version>` when ready. Rebuild distributable binaries from that commit.

CI checks version consistency. `npm run dev` watches the workspace manifest and
rebuilds WASM when the version changes. Existing binaries and deployed sites keep
their old version until rebuilt and deployed.
