import { detectTier, QUALITY } from "@/lib/quality";
import { engine } from "./lunalyr-audio";

// The host sets the frame cadence. A 30 FPS wallpaper setting, for instance,
// must not trip the browser demo's low-FPS watchdog.
const quality = QUALITY[detectTier()];
const reportSlowFrameBudget = () => {};

export function useAudio() {
  return { engine, quality, reportSlowFrameBudget };
}
