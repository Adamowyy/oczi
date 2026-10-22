import { defineConfig } from "vite";
import { resolve } from "node:path";

export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "chrome110",
    minify: "esbuild",
    sourcemap: false,
    emptyOutDir: true,
    rollupOptions: {
      input: {
        island: resolve(__dirname, "index.html"),
        settings: resolve(__dirname, "settings.html"),
        // The screen-selection overlay: its own page, its own styles, so the island
        // stylesheet can never affect it.
        snip: resolve(__dirname, "snip.html"),
        // A pinned crop: its own page for the same reason as the overlay, and because it is
        // drawn by a window of its own.
        pin: resolve(__dirname, "pin.html"),
        // The two PC windows — the health report and the debloat list — each a page of its own.
        health: resolve(__dirname, "health.html"),
        debloat: resolve(__dirname, "debloat.html"),
      },
    },
  },
});
