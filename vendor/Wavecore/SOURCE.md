# Wavecore

Lunalyr bundles the original [Wavecore](https://github.com/dineshkadavakuduru-cmd/wavecore/tree/5ef5a252deef71290096b1c5dd1f0edc0dc93acd) WebGL scene by Dinesh Kadavakuduru. The core and particle GLSL, camera, backdrop, sparkles, effects and spectral mapping come from this pinned revision. The [original demo](https://wavecore-gilt.vercel.app/) also includes a browser player and interface.

The host replaces the demo's audio player with a local PCM bridge and supplies the frame clock. The PCM bridge matches the demo's analyser window, FFT size and decibel range. The canvas uses Wavecore's hardware-based quality selection with Lunalyr's frame cadence. Lunalyr softens the core highlights and reduces bloom and punch flashes for desktop use.

- Wavecore's MIT notice is in `Sources/Resources/Wavecore-LICENSE.txt`.
- The [Ashima Arts and Stefan Gustavson simplex noise notice](https://github.com/ashima/webgl-noise/blob/6abed1e77ed1e18b181627c35f688eb30c9fe75e/LICENSE) is in `Sources/Resources/Wavecore-Simplex-LICENSE.txt`.
- The dependencies in the generated bundle are listed in `Sources/Resources/Wavecore-Web-Dependencies-LICENSE.txt`. React Three Fiber's npm package omits the text, so its notice is taken from the [v8.18.0 source tag](https://github.com/pmndrs/react-three-fiber/blob/v8.18.0/LICENSE).

Run `Scripts/build-wavecore-engine.sh /path/to/wavecore` with a clone of the upstream repository, Node.js 22 or later, and Python 3 to regenerate the offline bundle from the pinned revision and lockfile.

## Desktop safety

Wavecore's Reduce Motion handling still lets its kick "punch" flash the core, bloom and sparkles. `lunalyr-clock.ts` puts an accessor on the reactive state's `punch`, so Wavecore's own source is unchanged: it reads as zero for calm frames (Reduce Motion or Dim Flashing Lights) and is otherwise scaled by Lunalyr's Punch control (0 to 2, where 1 keeps Wavecore's own strength). Wavecore multiplies `punch` by its decay through the same accessor every frame, so the setter stores a rise as a new kick and undoes the scale on a decay; the scale then applies once instead of compounding. Only Reduce Motion switches on Wavecore's reduced-motion mode. Lunalyr's host limits how fast the page's mean brightness may rise while calm (`WebVisualizerView.drawScript`).

### Manual bundle patch (November 2026)

No Node toolchain was available, so the same edits were applied by hand to the clock section of `Sources/Resources/WavecoreEngine.html` (`lunalyrPunchScale` and the accessor before `connectWavecore`, and the `calm`/`punch`/`reduced` handling in `draw`). Rebuilding with `Scripts/build-wavecore-engine.sh` reproduces them from the TypeScript.

## Frame-rate independence

The demo's AnalyserNode smoothed the spectrum by 0.55 on every 60 fps frame, and the clock capped each step at 1/20 s, so a 15 fps wallpaper ran at 75% speed with different smoothing. `lunalyr-audio.ts` now smooths with the equivalent time constant (about 28 ms) and analyses every hop the host sends since the last frame; `lunalyr-clock.ts` caps a step at 0.1 s. The same edits were applied by hand to the bundle (`mI`, `lunalyrAnalyseWindow` and the clock's `draw`).

Wavecore's own camera and core also capped their per-frame step at 50 ms, and its audio driver (which advances the reactive clock and envelopes) at 1/20 s, so below 20 fps the scene still ran slower than at full rate. All three now allow 0.1 s, matching the clock. The upstream source of those components is not vendored here, so instead of a `.patch` file `Scripts/build-wavecore-engine.sh` rewrites the two `Math.min(…,.05)` caps and the `=1/20` driver constant in the minified output, and stops if it does not find exactly those three. The current bundle was edited by hand the same way (the camera rig's and the core's `useFrame` callbacks and `H5`).

The host also asks WebKit for the default GPU when Wavecore's canvas requests the high-performance one (`WebVisualizerView.contextScript`), so dual-GPU Macs can stay on the integrated GPU.

