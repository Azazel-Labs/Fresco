import { test } from "node:test";
import assert from "node:assert/strict";
import { nextVersion, run } from "./version.mjs";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

test("release increments reset lower components", () => {
  assert.equal(nextVersion("0.2.9", "patch"), "0.2.10");
  assert.equal(nextVersion("0.2.9", "minor"), "0.3.0");
  assert.equal(nextVersion("0.2.9", "major"), "1.0.0");
  assert.equal(nextVersion("1.9.99", "minor"), "1.10.0");
});

test("invalid release input is rejected", () => {
  for (const version of ["01.2.3", "1.2", "1.2.3-dev", "-1.2.3"]) {
    assert.throws(() => nextVersion(version, "patch"));
  }
  assert.throws(() => nextVersion("0.2.0", "typo"));
  assert.throws(() => run("typo"));
});

test("all workspace and web versions agree", () => {
  run("check");
});

test("bumping updates manifests and lockfiles, and checks reject drift", (t) => {
  const root = mkdtempSync(join(tmpdir(), "fresco-version-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const write = (name, text) => writeFileSync(join(root, name), text);
  const readJson = (name) => JSON.parse(readFileSync(join(root, name), "utf8"));
  for (const dir of ["scripts", "crates/fresco/src", "crates/fresco-macros/src", "crates/fresco-wasm/web"]) {
    mkdirSync(join(root, dir), { recursive: true });
  }
  copyFileSync(new URL("./version.mjs", import.meta.url), join(root, "scripts/version.mjs"));
  write("Cargo.toml", `[workspace]
members = ["crates/fresco", "crates/fresco-macros"]
resolver = "2"
[workspace.package]
repository = "https://example.com/fresco"

# The release version need not be the first field.
version  =  "0.2.9" # Keep this comment when bumping.
[workspace.dependencies]
fresco-macros = { version = "=0.2.9", path = "crates/fresco-macros" }
`);
  for (const name of ["fresco", "fresco-macros"]) {
    write(`crates/${name}/Cargo.toml`, `[package]\nname = "${name}"\nversion.workspace = true\nedition = "2021"\n${name === "fresco" ? "[dependencies]\nfresco-macros.workspace = true\n" : ""}`);
    write(`crates/${name}/src/lib.rs`, "");
  }
  const web = "crates/fresco-wasm/web/";
  write(`${web}package.json`, JSON.stringify({ version: "0.2.9", scripts: { untouched: "keep me" } }));
  write(`${web}package-lock.json`, JSON.stringify({ version: "0.2.9", packages: { "": { version: "0.2.9" } } }));
  const command = (arg) => spawnSync(process.execPath, ["scripts/version.mjs", arg], { cwd: root, encoding: "utf8" });
  for (const [increment, expected] of [["patch", "0.2.10"], ["minor", "0.3.0"], ["major", "1.0.0"]]) {
    const result = command(increment);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(readJson(`${web}package.json`).version, expected);
    assert.equal(readJson(`${web}package.json`).scripts.untouched, "keep me");
    assert.equal(readJson(`${web}package-lock.json`).packages[""].version, expected);
    assert.ok(readFileSync(join(root, "Cargo.lock"), "utf8").includes(`version = "${expected}"`));
    assert.ok(readFileSync(join(root, "Cargo.toml"), "utf8").includes(`version  =  "${expected}" # Keep this comment when bumping.`));
    assert.equal(command("check").status, 0);
  }
  // Cargo forwards package metadata verbatim. Exceed spawnSync's default 1 MiB
  // buffer without depending on the size of the real workspace's dependency graph.
  const crateManifest = readFileSync(join(root, "crates/fresco/Cargo.toml"), "utf8");
  write("crates/fresco/Cargo.toml", `${crateManifest}\n[package.metadata.version-test]\npayload = "${"x".repeat(2 * 1024 * 1024)}"\n`);
  const largeMetadataCheck = command("check");
  assert.equal(largeMetadataCheck.status, 0, largeMetadataCheck.stderr);
  write(`${web}package.json`, JSON.stringify({ version: "0.1.0" }));
  assert.notEqual(command("check").status, 0);
  assert.equal(command("sync").status, 0);
  assert.equal(command("check").status, 0);
  const cargoLock = readFileSync(join(root, "Cargo.lock"), "utf8");
  write("Cargo.lock", cargoLock.replaceAll('version = "1.0.0"', 'version = "0.1.0"'));
  assert.notEqual(command("check").status, 0, "a stale Cargo lockfile must fail without being rewritten");
  assert.equal(readFileSync(join(root, "Cargo.lock"), "utf8").includes('version = "1.0.0"'), false);
  assert.equal(command("sync").status, 0);
  write("crates/fresco/Cargo.toml", '[package]\nname = "fresco"\nversion = "1.0.0"\nedition = "2021"\n');
  assert.notEqual(command("check").status, 0, "independent crate versions must be rejected even if equal");
  write("Cargo.toml", '[workspace]\nmembers = []\n[workspace.package]\nrepository = "https://example.com/fresco"\n[workspace.metadata]\nversion = "1.0.0"\n');
  const missingVersion = command("check");
  assert.notEqual(missingVersion.status, 0);
  assert.match(missingVersion.stderr, /Missing workspace\.package\.version/);
});
