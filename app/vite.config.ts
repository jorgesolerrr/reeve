import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  // Fixed port, matching `devUrl` in src-tauri/tauri.conf.json: Tauri starts
  // this server and then points a webview at it, so a port fallback would
  // silently open an empty window.
  server: { port: 1420, strictPort: true },
  build: { target: "es2022" },
  test: {
    // The risk lives in lib/, and lib/ is pure logic — no DOM (10-lld-frontend).
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
