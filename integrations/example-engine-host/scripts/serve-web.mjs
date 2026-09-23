import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../web/", import.meta.url));
const port = Number(process.env.PORT ?? 5182);
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".fr": "text/plain", ".json": "application/json" };
createServer(async (request, response) => {
  try {
    const url = new URL(request.url, "http://localhost");
    const file = path.resolve(root, `.${decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname)}`);
    const relative = path.relative(root, file);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      response.writeHead(403).end("Forbidden");
      return;
    }
    const content = await readFile(file);
    response.writeHead(200, { "Content-Type": types[path.extname(file)] ?? "application/octet-stream", "Cache-Control": "no-store" });
    response.end(content);
  } catch (error) {
    response.writeHead(error.code === "ENOENT" ? 404 : 400).end("Unable to serve this path");
  }
}).listen(port, "127.0.0.1", () => console.log(`Fresco example engine: http://127.0.0.1:${port}`));
