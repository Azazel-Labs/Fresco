import { defineConfig } from "vite";

export default defineConfig({
  plugins: [{
    name: "production-rust-preview",
    apply: "build",
    generateBundle(_options, bundle) {
      const legacy = /\/(?:tests\/reference\/|src\/preview\/(?:preview|surface)-renderer\.ts$)/;
      for (const output of Object.values(bundle)) {
        if (output.type !== "chunk") continue;
        for (const id of Object.keys(output.modules)) {
          if (legacy.test(id.replaceAll("\\", "/"))) {
            this.error(`Production preview must not bundle the legacy renderer: ${id}`);
          }
        }
      }
    },
  }],
  // GitHub Pages project sites are served under /<repo>/.
  base: process.env.FRESCO_BASE_PATH ?? "/",
  build: {
    // Monaco is intentionally split into its own chunk and remains large.
    // Vite's warning threshold is global, so set it above the expected Monaco size.
    chunkSizeWarningLimit: 3200,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("monaco-editor")) {
            return "monaco";
          }
          if (id.includes("node_modules")) {
            return "vendor";
          }
          return undefined;
        },
      },
    },
  },
  server: {
    host: "0.0.0.0",
    port: 5178
  }
});
