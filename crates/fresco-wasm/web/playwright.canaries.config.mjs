import { defineConfig } from "@playwright/test";
import gpu from "./playwright.gpu.config.mjs";

export default defineConfig({
  ...gpu,
  testMatch: "**/canaries.spec.mjs",
  projects: gpu.projects.map(project => ({ ...project, testMatch: "**/canaries.spec.mjs" })),
  use: {
    ...gpu.use,
    channel: process.env.FRESCO_GPU_BROWSER || "chromium",
    launchOptions: {
      args: ["--enable-unsafe-webgpu", ...(process.env.FRESCO_GPU_SOFTWARE === "1" ? [
        "--enable-features=Vulkan",
        "--use-angle=vulkan",
        "--use-vulkan=swiftshader",
        "--use-webgpu-adapter=swiftshader",
        "--disable-vulkan-surface",
      ] : [])],
    },
  },
  webServer: undefined,
  globalSetup: "./tests/gpu/canary-server.mjs",
});
