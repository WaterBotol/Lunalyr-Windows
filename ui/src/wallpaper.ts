// wallpaper.html — one per monitor, parented behind the desktop icons by the
// Rust platform layer. Outside Tauri (a plain browser) it plays the built-in
// demo feed instead, so the page can be opened directly while developing.

import { createBridge } from "./bridge";
import { preloadFonts } from "./render/preload";
import { WallpaperRenderer } from "./render/renderer";
import { connectStore, Store } from "./store";

async function main(): Promise<void> {
  const store = new Store();
  const renderer = new WallpaperRenderer(document.body, store);
  await preloadFonts();
  renderer.start();
  const bridge = await createBridge(async () => {
    const { DemoHost } = await import("./demo/host");
    return new DemoHost({ scenario: "word" });
  });
  await connectStore(bridge, store, { onAudio: (frame) => renderer.pushAudio(frame) });
}

main().catch((error) => console.error("Lunalyr wallpaper failed to start", error));
