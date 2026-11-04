import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// The frontend lives in ui/. Three pages:
//  - wallpaper.html: the per-monitor lyric wallpaper (loaded by Tauri)
//  - settings.html:  the settings window (loaded by Tauri)
//  - demo.html:      a browser-only harness that feeds sample lyrics, for
//                    developing/verifying the renderer on any OS
const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "ui");

export default defineConfig({
  root,
  publicDir: resolve(root, "public"),
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: resolve(here, "dist"),
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: {
        wallpaper: resolve(root, "wallpaper.html"),
        settings: resolve(root, "settings.html"),
        demo: resolve(root, "demo.html"),
      },
    },
  },
  test: {
    root: here,
    include: ["ui/**/*.test.ts"],
    environment: "node",
  },
});
