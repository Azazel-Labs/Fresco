import { createServer } from "vite";

// Own Vite in this process: closing it must not depend on shell process-tree
// termination (which can hang on Windows). Override the deployment-only base.
export default async function setup() {
  const server = await createServer({
    base: "/",
    server: { host: "127.0.0.1", port: 5189, strictPort: true, hmr: false },
  });
  try {
    await server.listen();
  } catch (error) {
    await server.close();
    throw error;
  }
  return () => server.close();
}
