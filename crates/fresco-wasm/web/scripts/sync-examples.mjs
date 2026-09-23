import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const workspaceRoot = path.resolve(__dirname, "../../../..");
const sourceDir = path.join(workspaceRoot, "examples");
const targetDir = path.resolve(__dirname, "../src/examples");
const SYNC_EXTENSIONS = new Set([".fr", ".png", ".jpg", ".jpeg", ".webp", ".avif", ".json"]);

async function copyExamplesRecursive(fromDir, toDir) {
  await mkdir(toDir, { recursive: true });
  const entries = await readdir(fromDir, { withFileTypes: true });

  for (const entry of entries) {
    const fromPath = path.join(fromDir, entry.name);
    const toPath = path.join(toDir, entry.name);

    if (entry.isDirectory()) {
      if (fromDir === sourceDir && entry.name === "engine") continue;
      await copyExamplesRecursive(fromPath, toPath);
      continue;
    }

    if (!entry.isFile() || !SYNC_EXTENSIONS.has(path.extname(entry.name).toLowerCase())) {
      continue;
    }
    await mkdir(path.dirname(toPath), { recursive: true });

    if (path.extname(entry.name).toLowerCase() === ".fr") {
      const content = await readFile(fromPath, "utf8");
      await writeFile(toPath, content, "utf8");
      continue;
    }

    const bytes = await readFile(fromPath);
    await writeFile(toPath, bytes);
  }
}

await rm(targetDir, { recursive: true, force: true });
await copyExamplesRecursive(sourceDir, targetDir);

// runtime-content-registry.ts bundles the canonical engine independently.
