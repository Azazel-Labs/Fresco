// Shared by the standalone host and the playground distribution.
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const host = fileURLToPath(new URL("../", import.meta.url));
const root = path.resolve(host, "../..");

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", windowsHide: true });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status ?? result.signal})`);
}

// Cargo resolves inherited workspace fields. wasm-pack's package writer still
// expects some of those fields (notably repository) to be literal strings.
function workspacePackages() {
  const result = spawnSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
    cwd: root, encoding: "utf8", windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`cargo metadata failed: ${result.stderr}`);
  return JSON.parse(result.stdout).packages;
}

function writePackageMetadata(output, metadata) {
  const target = metadata.targets.find(target => target.crate_types.includes("cdylib"));
  if (!target) throw new Error(`Missing WASM library target for ${metadata.name}`);
  const stem = target.name;
  const packageJson = {
    name: metadata.name, type: "module", version: metadata.version,
    description: metadata.description, license: metadata.license, repository: metadata.repository,
    files: [`${stem}_bg.wasm`, `${stem}.js`, `${stem}.d.ts`, "snippets"],
    main: `${stem}.js`, types: `${stem}.d.ts`, sideEffects: ["./snippets/*"],
  };
  writeFileSync(path.join(output, "package.json"), JSON.stringify(packageJson, null, 2) + "\n");
}

export function buildWasm({ compilerOutput, rendererOutput, release = false }) {
  const packages = workspacePackages();
  const mode = release ? ["--release"] : ["--profile", "wasm-dev", "--no-opt"];
  for (const [crate, output, features] of [
    [path.join(root, "crates/fresco-wasm"), compilerOutput, []],
    [host, rendererOutput, ["--no-default-features", "--features", "browser"]],
  ]) {
    mkdirSync(output, { recursive: true });
    run(process.platform === "win32" ? "wasm-pack.exe" : "wasm-pack", [
      "build", path.relative(root, crate).replaceAll("\\", "/"), "--target", "web", "--no-pack", ...mode,
      "--out-dir", path.relative(crate, output).replaceAll("\\", "/"), ...features,
    ]);
    const metadata = packages.find(pkg => path.resolve(pkg.manifest_path) === path.join(crate, "Cargo.toml"));
    if (!metadata) throw new Error(`Missing Cargo metadata for ${crate}`);
    writePackageMetadata(output, metadata);
    run(process.execPath, [path.join(host, "scripts", output === compilerOutput ? "test-compiler.mjs" : "test-renderer.mjs"), output,
      ...(output === compilerOutput ? [release ? "release" : "dev"] : []),
    ]);
  }
}
