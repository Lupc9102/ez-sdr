import { defineConfig } from "vite";

// Lean, framework-free build: plain TS + Vite bundling only. No UI framework
// runtime (React/Vue/etc.) — matches the daemon's low-overhead philosophy and
// keeps the shipped bundle small for a control surface that's mostly canvas
// rendering and WebSocket plumbing.
export default defineConfig({
  root: "src",
  publicDir: "../public",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:7891",
      "/ws": {
        target: "ws://127.0.0.1:7891",
        ws: true,
      },
    },
  },
});
