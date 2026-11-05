import { defineConfig } from "vite";
import { resolve } from "node:path";

export default defineConfig({
  publicDir: false,
  build: {
    target: "safari17",
    lib: {
      entry: resolve("lunalyr-engine.ts"),
      name: "LunalyrWavefield",
      formats: ["iife"],
      fileName: () => "wavefield.js",
    },
    minify: true,
    sourcemap: false,
  },
});
