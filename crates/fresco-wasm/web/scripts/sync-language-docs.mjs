import { mkdir, readFile, writeFile, copyFile } from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { marked } from "marked";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const webDir = path.resolve(__dirname, "..");
const workspaceRoot = path.resolve(webDir, "../../..");

const jsonSourcePath = path.join(workspaceRoot, "docs", "generated", "language-reference.v1.json");
const markdownSourcePath = path.join(workspaceRoot, "docs", "generated", "language-reference.v1.md");
const publicGeneratedDir = path.join(webDir, "public", "generated");

const skipGenerate = process.argv.includes("--skip-generate");

function runCommand(command, args, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      stdio: "inherit",
      shell: process.platform === "win32"
    });

    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`${command} exited with code ${code ?? 1}`));
      }
    });
  });
}

marked.setOptions({
  gfm: true,
  breaks: false,
  headerIds: false,
  mangle: false
});

function markdownToHtml(markdown) {
  return marked.parse(String(markdown || ""));
}

function renderDocsHtml(markdown) {
  const body = markdownToHtml(markdown);
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Fresco Language API Reference</title>
  <style>
    :root {
      color-scheme: dark;
      --bg: #07131d;
      --bg-soft: #0b1b28;
      --ink: #dcecf8;
      --muted: #9bb4c4;
      --line: rgba(154, 200, 222, 0.28);
      --accent: #2dd4bf;
      --mono: "IBM Plex Mono", Consolas, monospace;
      --sans: "Chivo", "Segoe UI", sans-serif;
    }

    * { box-sizing: border-box; }
    html, body { margin: 0; padding: 0; }
    body {
      font-family: var(--sans);
      line-height: 1.45;
      color: var(--ink);
      background: linear-gradient(165deg, #06111a, #0a1f2b 52%, #0b1520);
    }

    .page {
      max-width: 1120px;
      margin: 0 auto;
      padding: 22px 20px 60px;
    }

    h1, h2, h3, h4 {
      margin: 1.15em 0 0.45em;
      line-height: 1.2;
      letter-spacing: 0.015em;
    }

    h1 {
      margin-top: 0.15em;
      font-size: clamp(1.5rem, 1.1rem + 1.2vw, 2.25rem);
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    h2 {
      font-size: 1.18rem;
      border-bottom: 1px solid var(--line);
      padding-bottom: 0.25em;
      margin-top: 1.6em;
    }

    h3 { font-size: 1.03rem; color: #bde9f2; }
    h4 { font-size: 0.95rem; color: #bde9f2; }

    p, li { color: #d3e8f6; }
    ul { padding-left: 1.2rem; }

    code {
      font-family: var(--mono);
      font-size: 0.9em;
      background: rgba(11, 27, 39, 0.95);
      border: 1px solid var(--line);
      padding: 0.12em 0.34em;
      border-radius: 6px;
    }

    table {
      width: fit-content;
      border-collapse: collapse;
      margin: 0.62rem 0 1.05rem;
      background: rgba(6, 20, 31, 0.72);
      border: 1px solid var(--line);
      border-radius: 8px;
      overflow: hidden;
      display: block;
      overflow-x: auto;
    }

    thead th {
      position: sticky;
      top: 0;
      z-index: 1;
      background: rgba(11, 28, 40, 0.96);
      color: var(--accent);
      text-transform: uppercase;
      letter-spacing: 0.04em;
      font-size: 0.72rem;
    }

    th, td {
      text-align: left;
      padding: 8px 10px;
      border-bottom: 1px solid rgba(154, 200, 222, 0.14);
      white-space: nowrap;
      font-size: 0.84rem;
    }

    tr:last-child td { border-bottom: none; }

    @media (max-width: 720px) {
      .page { padding: 14px 10px 40px; }
      th, td { padding: 7px 8px; font-size: 0.78rem; }
      h2 { margin-top: 1.3em; }
    }
  </style>
</head>
<body>
  <main class="page">
${body}
  </main>
</body>
</html>
`;
}

async function main() {
  if (!skipGenerate) {
    await runCommand("cargo", ["xtask", "lang-docs"], workspaceRoot);
  }

  await mkdir(publicGeneratedDir, { recursive: true });

  const markdown = await readFile(markdownSourcePath, "utf8");
  await copyFile(jsonSourcePath, path.join(publicGeneratedDir, "language-reference.v1.json"));
  await writeFile(path.join(publicGeneratedDir, "language-reference.v1.md"), markdown, "utf8");
  await writeFile(
    path.join(publicGeneratedDir, "language-reference.v1.html"),
    renderDocsHtml(markdown),
    "utf8"
  );

  console.log("[docs] synced language reference artifacts to web/public/generated");
}

main().catch((err) => {
  console.error(`[docs] sync failed: ${err?.message ?? err}`);
  process.exit(1);
});
