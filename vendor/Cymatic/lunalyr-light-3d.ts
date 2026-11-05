import { createRendererWithFallback, Director, type AudioFeatureFrame } from "@cymatic/core";
import { light3dPreset } from "./packages/presets/src/particle/light-3d";

type Audio = { bass: number; mid: number; treble: number; level: number; beat: number; flux: number; centroid: number; bands: number[] };
// `calm` is Reduce Motion or Dim Flashing Lights; `punch` is Lunalyr's
// transient-strength control, where 1 is the shipped look.
type Frame = { width: number; height: number; clock: number; sensitivity: number; motion: number;
  paused: boolean; reduced: boolean; calm?: boolean; punch?: number; preview?: boolean; audio?: Audio };

const canvas = document.querySelector("canvas")!;
const init = createRendererWithFallback(canvas).then(async renderer => {
  const preset = light3dPreset.create();
  await preset.init({ renderer, width: 2, height: 2, dpr: 1 });
  return { renderer, preset };
});
const director = new Director();
let lastClock: number | undefined;
let elapsed = 0;
let width = 0;
let height = 0;
let beat = 0;
let lastState = { backend: "", clock: 0, bass: 0, mid: 0, treble: 0, onset: false };

async function draw(frame: Frame) {
  const { renderer, preset } = await init;
  const scale = Math.min(devicePixelRatio || 1, Math.sqrt((frame.preview ? 220_000 : 1_000_000) / (frame.width * frame.height)));
  const w = Math.max(2, Math.round(frame.width * scale));
  const h = Math.max(2, Math.round(frame.height * scale));
  const resized = w !== width || h !== height;
  if (resized) {
    width = w;
    height = h;
    renderer.resize(width, height, 1);
    preset.resize(width, height, 1);
  }
  if (frame.paused && !resized && lastState.backend) {
    lastClock = undefined;
    return lastState;
  }

  const dt = lastClock === undefined ? 1 / 60 : Math.max(1 / 240, Math.min(0.1, frame.clock - lastClock));
  lastClock = frame.paused ? undefined : frame.clock;
  const audio = frame.audio;
  const gain = Math.max(0.3, Math.min(3, frame.sensitivity));
  const response = (value = 0) => Math.min(1, value * gain);
  const bass = response(audio?.bass);
  const mid = response(audio?.mid);
  const treble = response(audio?.treble);
  const rms = response(audio?.level);
  // Calm frames have no onsets or flux, so the director never flashes.
  const transient = frame.calm === true || frame.reduced ? 0 : Math.max(0, Math.min(2, frame.punch ?? 1));
  const onset = transient > 0.05 && beat < 0.45 && (audio?.beat ?? 0) >= 0.45;
  beat = audio?.beat ?? 0;
  elapsed += dt;
  const features: AudioFeatureFrame = {
    bands: audio?.bands.map(response) ?? new Array(16).fill(0), bass, mid, treble, rms, onset,
    spectralCentroid: audio?.centroid ?? 0, spectralRolloff: 0, spectralFlux: response((audio?.flux ?? 0) * transient),
    loudnessShort: rms, loudnessLong: rms, dynamics: 0, tempo: 0, beatPhase: 0, onsetDensity: 0,
    mood: { energy: Math.min(1, rms * 0.6 + mid * 0.4), brightness: treble, busyness: response((audio?.flux ?? 0) * transient),
      valence: 0.5, dynamics: 0 },
    time: elapsed,
  };
  const direction = director.update(features, dt);
  preset.update(features, elapsed, dt * frame.motion * (frame.reduced ? 0.2 : 1), { director: direction });
  lastState = { backend: renderer.backend, clock: elapsed, bass, mid, treble, onset };
  return lastState;
}

Object.assign(window, { lunalyrLight3D: { draw } });
