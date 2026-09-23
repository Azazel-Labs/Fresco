import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/gpu",
  testMatch: "**/*.spec.mjs",
  projects: [{ name: "rust-engine" }],
  timeout: 60_000,
  workers: 1,
  retries: 0,
  outputDir: "../../../target/gpu-test-results",
  reporter: [["list"], ["json", { outputFile: "../../../target/gpu-test-results/results.json" }]],
  use: {
    channel: process.env.FRESCO_GPU_BROWSER || "chrome",
    headless: true,
    viewport: { width: 800, height: 600 },
    deviceScaleFactor: 1,
    baseURL: "http://127.0.0.1:5189",
    screenshot: "only-on-failure",
  },
  globalSetup: "./tests/gpu/canary-server.mjs",
});
