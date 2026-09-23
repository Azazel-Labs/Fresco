import { watch } from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const webDir = path.resolve(__dirname, "..");
const wasmDir = path.resolve(webDir, "..");
const workspaceRoot = path.resolve(webDir, "../../..");
const examplesDir = path.join(workspaceRoot, "examples");
const exampleEngineDir = path.join(workspaceRoot, "integrations/example-engine");
const wasmSrcDir = path.join(wasmDir, "src");
const wasmCargoToml = path.join(wasmDir, "Cargo.toml");

const isWindows = process.platform === "win32";
const npmCmd = process.platform === "win32" ? "npm.cmd" : "npm";

function runCommand(command, args, cwd) {
  return new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd,
      stdio: "inherit",
    });

    child.on("exit", (code) => {
      resolve(code === 0);
    });

    child.on("error", () => {
      resolve(false);
    });
  });
}

function runNpmScript(script, cwd) {
  if (isWindows) {
    const comspec = process.env.ComSpec || "cmd.exe";
    return runCommand(comspec, ["/d", "/s", "/c", `${npmCmd} run ${script}`], cwd);
  }

  return runCommand(npmCmd, ["run", script], cwd);
}

async function runSyncExamples() {
  return runNpmScript("sync:examples", webDir);
}

async function runWasmBuild() {
  return runCommand(process.execPath, [path.join(webDir, "scripts/build-wasm.mjs"), "--dev"], webDir);
}

async function runDocsSync() {
  return runNpmScript("docs:sync", webDir);
}

let running = false;
let pendingExamplesSync = false;
let pendingWasmBuild = false;
let debounce = null;

function queueWork({ syncExamples = false, wasmBuild = false, reason = "" }) {
  pendingExamplesSync = pendingExamplesSync || syncExamples;
  pendingWasmBuild = pendingWasmBuild || wasmBuild;

  if (debounce) {
    clearTimeout(debounce);
  }

  debounce = setTimeout(() => {
    debounce = null;
    void processQueue(reason);
  }, 150);
}

async function processQueue(reason) {
  if (running) {
    return;
  }

  const doSync = pendingExamplesSync;
  const doBuild = pendingWasmBuild;
  pendingExamplesSync = false;
  pendingWasmBuild = false;

  if (!doSync && !doBuild) {
    return;
  }

  running = true;
  console.log(`\n[dev:auto] Change detected (${reason || "update"}). Rebuilding...`);

  if (doSync) {
    const ok = await runSyncExamples();
    if (!ok) {
      console.error("[dev:auto] Example sync failed.");
    }
  }

  if (doBuild) {
    const ok = await runWasmBuild();
    if (!ok) {
      console.error("[dev:auto] Wasm build failed.");
    }
  }

  running = false;

  if (pendingExamplesSync || pendingWasmBuild) {
    void processQueue("queued changes");
  } else {
    console.log("[dev:auto] Rebuild complete.");
  }
}

function watchPath(targetPath, opts, onChange) {
  const watcher = watch(targetPath, opts, (_eventType, filename) => {
    const rel = filename ? filename.toString() : path.basename(targetPath);
    onChange(rel);
  });

  watcher.on("error", (err) => {
    console.error(`[dev:auto] Watcher error for ${targetPath}:`, err.message);
  });

  return watcher;
}

console.log("[dev:auto] Initial sync + wasm build...");
const syncOk = await runSyncExamples();
const wasmOk = await runWasmBuild();
const docsOk = await runDocsSync();
if (!syncOk || !wasmOk || !docsOk) {
  console.warn("[dev:auto] Initial setup failed. Vite will still start; subsequent changes will retrigger.");
}

console.log("[dev:auto] Starting Vite dev server...");
const viteCommand = isWindows ? process.env.ComSpec || "cmd.exe" : npmCmd;
const viteArgs = isWindows ? ["/d", "/s", "/c", `${npmCmd} run dev:vite`] : ["run", "dev:vite"];
const vite = spawn(viteCommand, viteArgs, {
  cwd: webDir,
  stdio: "inherit",
});

const watchers = [];
for (const relative of ["integrations/example-engine-host/src", "crates/fresco-artifact/src", "crates/fresco/src"]) {
  watchers.push(watchPath(path.join(workspaceRoot, relative), { recursive: true }, (filename) => {
    if (filename.endsWith(".rs")) queueWork({ wasmBuild: true, reason: `${relative}/${filename}` });
  }));
}
for (const relative of ["integrations/example-engine-host/Cargo.toml", "crates/fresco-artifact/Cargo.toml", "crates/fresco/Cargo.toml", "Cargo.lock"]) {
  watchers.push(watchPath(path.join(workspaceRoot, relative), {}, () => {
    queueWork({ wasmBuild: true, reason: relative });
  }));
}

watchers.push(
  watchPath(exampleEngineDir, { recursive: true }, (filename) => {
    if (!/\.(fr|rs|toml)$/.test(filename)) return;
    queueWork({ syncExamples: true, wasmBuild: true, reason: `example-engine/${filename}` });
  }),
);
watchers.push(
  watchPath(path.join(workspaceRoot, "Cargo.toml"), {}, () => {
    queueWork({ wasmBuild: true, reason: "workspace version or dependencies" });
  })
);
watchers.push(
  watchPath(examplesDir, { recursive: true }, (filename) => {
    queueWork({ syncExamples: true, reason: `examples/${filename}` });
  })
);
watchers.push(
  watchPath(wasmSrcDir, { recursive: true }, (filename) => {
    queueWork({ wasmBuild: true, reason: `wasm/src/${filename}` });
  })
);
watchers.push(
  watchPath(wasmCargoToml, {}, () => {
    queueWork({ wasmBuild: true, reason: "fresco-wasm/Cargo.toml" });
  })
);

function cleanupAndExit(code = 0) {
  for (const w of watchers) {
    try {
      w.close();
    } catch {
      // ignore cleanup errors
    }
  }

  if (!vite.killed) {
    vite.kill();
  }

  process.exit(code);
}

vite.on("exit", (code) => {
  cleanupAndExit(code ?? 0);
});

process.on("SIGINT", () => cleanupAndExit(0));
process.on("SIGTERM", () => cleanupAndExit(0));
