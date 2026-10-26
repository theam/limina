import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const runtime = process.env.LIMINA_RUNTIME_URL ?? "http://127.0.0.1:7433";

export default defineConfig({
  plugins: [react()],
  server: {
    host: "127.0.0.1",
    port: 5173,
    // Keep the browser's Host so session cookies, the Origin check, and the WorkOS redirect URI
    // all see the dev origin (http://127.0.0.1:5173).
    proxy: {
      "/v1": { target: runtime, ws: true, changeOrigin: false },
      "/auth": { target: runtime, changeOrigin: false },
      "/healthz": { target: runtime, changeOrigin: false },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: true,
    chunkSizeWarningLimit: 900,
  },
  test: {
    environment: "jsdom",
  },
});
