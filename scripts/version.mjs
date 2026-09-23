import { closeSync, mkdtempSync, openSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";

const root = fileURLToPath(new URL("../", import.meta.url));
const web = "crates/fresco-wasm/web/";
const read = (name) => readFileSync(resolve(root, name), "utf8");
const write = (name, text) => writeFileSync(resolve(root, name), text.replaceAll("\r\n", "\n"));

export function nextVersion(current, increment) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(current)) {
    throw new Error(`Expected a stable major.minor.patch version, got ${current}`);
  }
  const parts = current.split(".").map(BigInt);
  const index = ["major", "minor", "patch"].indexOf(increment);
  if (index < 0) throw new Error("Usage: node scripts/version.mjs [check|sync|patch|minor|major]");
  parts[index] += 1n;
  for (let i = index + 1; i < parts.length; i++) parts[i] = 0n;
  return parts.join(".");
}

function cargoMetadata(locked) {
  // Full metadata preserves Cargo's lockfile validation. Capture to files so
  // growing dependency graphs (or Cargo diagnostics) cannot overflow maxBuffer.
  const capture = mkdtempSync(join(tmpdir(), "fresco-cargo-metadata-"));
  const stdoutPath = join(capture, "stdout.json");
  const stderrPath = join(capture, "stderr.txt");
  const descriptors = [];
  try {
    descriptors.push(openSync(stdoutPath, "w"));
    descriptors.push(openSync(stderrPath, "w"));
    const result = spawnSync("cargo", ["metadata", "--format-version", "1", ...(locked ? ["--locked"] : [])], {
      cwd: root, stdio: ["ignore", ...descriptors]
    });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(readFileSync(stderrPath, "utf8"));
    return JSON.parse(readFileSync(stdoutPath, "utf8"));
  } finally {
    for (const descriptor of descriptors) closeSync(descriptor);
    rmSync(capture, { recursive: true, force: true });
  }
}

export function run(command = "check") {
  if (!["check", "sync", "patch", "minor", "major"].includes(command)) {
    throw new Error("Usage: node scripts/version.mjs [check|sync|patch|minor|major]");
  }
  let manifest = read("Cargo.toml");
  // Fields and comments may precede version, but never search another table.
  const pattern = /^(\[workspace\.package\][ \t]*(?:#[^\r\n]*)?\r?\n(?:(?![ \t]*\[)[^\r\n]*\r?\n)*?[ \t]*version[ \t]*=[ \t]*")([^"\r\n]+)(")/m;
  const match = manifest.match(pattern);
  if (!match) throw new Error("Missing workspace.package.version in Cargo.toml");
  const current = match[2];
  const version = ["check", "sync"].includes(command) ? current : nextVersion(current, command);
  const pkg = JSON.parse(read(`${web}package.json`));
  const lock = JSON.parse(read(`${web}package-lock.json`));

  if (command !== "check") {
    manifest = manifest.replace(pattern, (_match, prefix, _old, suffix) => `${prefix}${version}${suffix}`);
    // Internal dependency requirements stay exact and match the workspace release.
    manifest = manifest.replace(/^(fresco(?:-macros)? = \{ version = ")[^"]+(".*)$/gm,
      (_match, prefix, suffix) => `${prefix}=${version}${suffix}`);
    write("Cargo.toml", manifest);
    pkg.version = version;
    lock.version = version;
    lock.packages[""].version = version;
    write(`${web}package.json`, `${JSON.stringify(pkg, null, 2)}\n`);
    write(`${web}package-lock.json`, `${JSON.stringify(lock, null, 2)}\n`);
  }

  const metadata = cargoMetadata(command === "check");
  const members = metadata.packages.filter((p) => metadata.workspace_members.includes(p.id));
  for (const member of members) {
    const source = readFileSync(member.manifest_path, "utf8");
    if (!/^version\.workspace = true$/m.test(source.replaceAll("\r\n", "\n")) || member.version !== version) {
      throw new Error(`${member.name} must inherit workspace version ${version}`);
    }
    for (const dependency of member.dependencies) {
      if (members.some((p) => p.name === dependency.name) && dependency.req !== `=${version}`) {
        throw new Error(`${member.name}: ${dependency.name} requirement must be =${version}`);
      }
    }
  }
  if ([pkg.version, lock.version, lock.packages[""].version].some((v) => v !== version)) {
    throw new Error("Web package version drift; run node scripts/version.mjs sync");
  }
  console.log(command === "check" ? `Fresco ${version}: versions and lockfiles agree` : `Fresco ${current} -> ${version}; rebuild CLI and WASM before release`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length > 3) throw new Error("Expected one version command");
    run(process.argv[2]);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
