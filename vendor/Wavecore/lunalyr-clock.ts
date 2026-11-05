import type { RootState } from "@react-three/fiber";
import { reactive } from "@/lib/audio/reactive";
import { reducedMotion } from "@/lib/motion";
import { updateAudio } from "./lunalyr-audio";

type Frame = {
  clock: number;
  sensitivity: number;
  motion: number;
  reduced: boolean;
  /** Reduce Motion or Dim Flashing Lights. */
  calm?: boolean;
  /** Lunalyr's Punch control: scales Wavecore's kick punch, 1 keeping its own strength. */
  punch?: number;
  paused: boolean;
  preview?: boolean;
  width: number;
  height: number;
  pcm?: string;
  sampleRate?: number;
};

let root: RootState | undefined;
let previous: Frame | undefined;
let waiting: Frame | undefined;
let elapsed = 0;
let initialDpr = 1;

// Wavecore's kick "punch" flashes the core, the bloom and the sparkles, and
// its Reduce Motion handling leaves those in place. An accessor on the
// reactive state scales it by Lunalyr's Punch control and reads it as zero
// for calm frames, so Wavecore's own source needs no change.
//
// Wavecore writes punch in two ways. A kick sets it (to at least 0.5; kicks
// are at least 0.34 s apart, when the 150 ms envelope is below a tenth), and
// every frame multiplies it by its decay, reading it back through this
// getter. A rise is stored as the kick it is; anything else is a decay of the
// scaled value, so the scale is undone and applies once instead of
// compounding every frame.
let punchScale = 1;
let punch = reactive.punch;
Object.defineProperty(reactive, "punch", {
  configurable: true,
  enumerable: true,
  get: () => punch * punchScale,
  set: (value: number) => {
    punch = punchScale > 0 && value <= punch * punchScale ? value / punchScale : value;
  },
});

export function connectWavecore(state: RootState) {
  root = state;
  previous = undefined;
  initialDpr = state.viewport.dpr;
  elapsed = 0;
  // Canvas mounts its children from onCreated. Wait one turn before the first
  // advance so the scene and the composer's render pass are subscribed.
  requestAnimationFrame(() => {
    if (waiting && root === state) draw(waiting);
  });
}

function draw(frame: Frame) {
  waiting = frame;
  if (!root || root.gl.getContext().isContextLost()) return { ready: false };

  const budget = frame.preview ? 220_000 : 1_000_000;
  const dpr = Math.min(initialDpr, Math.sqrt(budget / Math.max(1, frame.width * frame.height)));
  if (Math.abs(root.get().viewport.dpr - dpr) > 0.01) root.setDpr(dpr);
  const delta = !previous ? 1 / 60 : frame.paused ? 0 :
    previous.paused ? 1 / 60 : Math.max(0, Math.min(0.1, frame.clock - previous.clock));
  previous = frame;
  const punchSetting = typeof frame.punch === "number" && Number.isFinite(frame.punch) ? frame.punch : 1;
  punchScale = frame.calm === true ? 0 : Math.max(0, Math.min(2, punchSetting));
  // Only Reduce Motion calms Wavecore's motion. Dim Flashing Lights is about
  // flashes, which calm frames remove through the punch above.
  reducedMotion.value = frame.reduced || matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!frame.paused) updateAudio(frame.pcm, frame.sampleRate, frame.sensitivity, delta);
  elapsed += delta * frame.motion;
  root.advance(elapsed, false);
  return { ready: true, clock: reactive.clock, ambient: reactive.ambient, bass: reactive.bass,
    mid: reactive.mid, treble: reactive.treble, punch: reactive.punch, reduced: reducedMotion.value };
}

Object.assign(window, { lunalyrWavecore: { draw } });
