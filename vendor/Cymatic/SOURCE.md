# Cymatic Light 3D

Lunalyr bundles the [Light 3D preset](https://github.com/coreyepstein/cymatic/blob/217f285645914866df7ca685a7194308a42fd430/packages/presets/src/particle/light-3d.ts) and the Cymatic renderer and director used by it, from commit `217f285645914866df7ca685a7194308a42fd430`. The preset and the imported renderer source are covered by the repository's [MIT licence](https://github.com/coreyepstein/cymatic/blob/217f285645914866df7ca685a7194308a42fd430/LICENSE), copyright (c) 2026 Corey Epstein. The notice is bundled as `Sources/Resources/Cymatic-LICENSE.txt`.

Lunalyr supplies system-audio features and a frame clock. The original preset retains its depth-based point glows, palette changes, and WebGPU bloom and trails. The upstream WebGL fallback renders the points without the WebGPU post effects.

Run `Scripts/build-light-3d-engine.sh /path/to/cymatic` with Node.js and Python 3 to rebuild the offline asset.

## Desktop safety

`lunalyr-light-3d.ts` gives the director no onsets and no flux for calm frames (Reduce Motion or Dim Flashing Lights) or when Lunalyr's Punch control is off, and scales the flux it receives by Punch otherwise. Lunalyr's host limits how fast the page's mean brightness may rise while calm (`WebVisualizerView.drawScript`). No Node toolchain was available in November 2026, so the same edit was applied by hand to the entry point at the end of `Sources/Resources/Light3DEngine.html` (`lunalyrTransient` in `draw`); rebuilding with `Scripts/build-light-3d-engine.sh` reproduces it.

