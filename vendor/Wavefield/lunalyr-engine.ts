// Lunalyr's offline WebKit entry point. It uses Wavefield's original engine.
import { WebGLRenderer } from "three";
import { createHannWindow, FFT_SIZE } from "./src/audio/analyze";
import { AudioTemporalFeatureTracker, extractSpectrumFrame } from "./src/audio/featureAnalysis";
import { fftRadix2 } from "./src/audio/fft";
import { ModalFieldEngine, EMPTY_MODAL_FIELD_FRAME } from "./src/audio/ModalField";
import { createAmbientModalFieldFrame } from "./src/audio/fieldSources";
import { DEFAULT_SETTINGS } from "./src/config/settings";
import { createSettingsFromTemplate, loadWavefieldTemplates } from "./src/templateSettings";
import { SettingsTransitionController } from "./src/templates/settingsTransitionController";
import { DEFAULT_TEMPLATE_TRANSITION_CONFIG } from "./src/templateTransition";
import { ScreenViewController } from "./src/ui/screenViewController";
import { ModalFieldRenderer } from "./src/webgl/ModalFieldRenderer";

type Frame = {
  clock: number;
  width: number;
  height: number;
  preset: string;
  projection: "preset" | "screen" | "sphere";
  motion: number;
  sensitivity: number;
  reduced: boolean;
  /** Reduce Motion or Dim Flashing Lights: no beat flashes, slower morphs. */
  calm?: boolean;
  /** Lunalyr's Punch control: transient strength, where 1 is the shipped look. */
  punch?: number;
  paused: boolean;
  preview?: boolean;
  pcm?: string;
  sampleRate?: number;
};

// Lunalyr draws white lyrics over these presets. Each entry was chosen from
// renders in WebKit, with music and in the ambient field, at desktop sizes
// (WavefieldWebTests checks every preset): Black on Red, CRT, Grid, Knicks in
// 6 and Microsoft Excel filled the screen with a light or red background
// (mean luminance 0.17 to 0.89), so their background becomes a dark colour;
// Lightning and Matrix washed out the ambient field (0.6 and 0.47), so they
// draw less brightly with less bloom; Devil rendered almost black, so it
// draws its red pattern on black. The other presets already render dark with
// a visible pattern and keep upstream's settings. Skeleton Seizure is listed
// as Skeleton; Scripts/build-wavefield-engine.sh applies the same name to
// WavefieldPresets.json.
const desktopTuning: Record<string, { name?: string; settings: Record<string, string | number> }> = {
  "black-on-red": { settings: { backgroundColor: "#4a0000" } },
  "crt": { settings: { backgroundColor: "#0e0f12", cymaticBrightness: 1.1 } },
  "devil": { settings: { backgroundColor: "#000000", monoColor: "#ff1a1a", cymaticBrightness: 1.1,
    postAlphaDecayBlendMode: "screen" } },
  "grid": { settings: { backgroundColor: "#0e0f12", monoColor: "#ffffff", cymaticBrightness: 1.1 } },
  "knicks-in-6": { settings: { backgroundColor: "#000b7c", monoColor: "#ff5100" } },
  "lightning": { settings: { cymaticBrightness: 0.6, postBloomIntensity: 1 } },
  "matrix": { settings: { cymaticBrightness: 0.8, postBloomIntensity: 1.25 } },
  "microsoft-excel": { settings: { backgroundColor: "#0e0f12", cymaticBrightness: 1.1 } },
  "skeleton-seizure": { name: "Skeleton", settings: {} },
};

function tuneForDesktop<T extends { slug: string; name: string; settings: object }>(template: T): T {
  const tuning = desktopTuning[template.slug];
  if (!tuning) return template;
  const settings = { ...template.settings, ...tuning.settings };
  return { ...template, name: tuning.name ?? template.name, settings: settings as unknown as T["settings"] };
}

const templates = loadWavefieldTemplates(import.meta.glob("./src/templates/*.json", {
  eager: true, import: "default",
})).map(tuneForDesktop);
const canvas = document.querySelector("canvas")!;
// The default preference lets dual-GPU Macs keep the integrated GPU.
const renderer = new WebGLRenderer({ canvas, alpha: true, antialias: true, powerPreference: "default" });
const fieldRenderer = new ModalFieldRenderer();
const modalEngine = new ModalFieldEngine();
const tracker = new AudioTemporalFeatureTracker();
const hann = createHannWindow(FFT_SIZE);
const real = new Float32Array(FFT_SIZE);
const imaginary = new Float32Array(FFT_SIZE);
const magnitudes = new Float32Array(FFT_SIZE / 2);
let transitions: SettingsTransitionController | undefined;
let selected = "";
let width = 0, height = 0, renderScale = 0;
let frameIndex = 0, lastClock: number | undefined, liveTime = 0, visualTime = 0;
let liveField = EMPTY_MODAL_FIELD_FRAME;
let hadAudio = false;
let quietSeconds = 0;
const HOP = FFT_SIZE / 2;
const MAX_SAMPLES = FFT_SIZE * 8;
type Features = ReturnType<AudioTemporalFeatureTracker["update"]>;
const idleMotion = 0.25;
const silenceHold = 0.6;
const audibleRms = 0.0012;
const quietRms = 0.0006;
let lastSampleRate: number | undefined;

const screenView = new ScreenViewController(canvas, () => transitions?.effectiveSettings ?? DEFAULT_SETTINGS);

function decodeAudio(pcm: string) {
  // Native supplies the same 2048-sample Float32 window Wavefield would
  // obtain from AnalyserNode, plus a 1024-sample hop for each hop of audio
  // since the last frame, so a slow wallpaper frame rate skips no audio.
  const bytes = atob(pcm);
  if (bytes.length < FFT_SIZE * 4 || bytes.length > MAX_SAMPLES * 4 || bytes.length % 4 !== 0) {
    throw new Error("Invalid Wavefield PCM window");
  }
  const buffer = new Uint8Array(bytes.length);
  for (let i = 0; i < bytes.length; i++) buffer[i] = bytes.charCodeAt(i);
  return new Float32Array(buffer.buffer);
}

function readAudioWindow(samples: Float32Array, end: number, gain: number) {
  // Retain Wavefield's window, FFT and temporal tracker for each window.
  let sum = 0;
  for (let i = 0; i < FFT_SIZE; i++) {
    const value = samples[end - FFT_SIZE + i];
    real[i] = value * hann[i] * gain;
    sum += value * value;
  }
  return Math.sqrt(sum / FFT_SIZE);
}

// A frame's features keep the strongest transient any of its hops found.
function strongest(previous: Features, next: Features): Features {
  return {
    ...next,
    onsets: {
      low: Math.max(previous.onsets.low, next.onsets.low),
      mid: Math.max(previous.onsets.mid, next.onsets.mid),
      high: Math.max(previous.onsets.high, next.onsets.high),
    },
    signals: {
      ...next.signals,
      beat: Math.max(previous.signals.beat, next.signals.beat),
      pulse: Math.max(previous.signals.pulse, next.signals.pulse),
      change: Math.max(previous.signals.change, next.signals.change),
    },
  };
}

function analyse(sampleRate: number, delta: number, rms: number) {
  imaginary.fill(0);
  fftRadix2(real, imaginary);
  for (let i = 0; i < magnitudes.length; i++) {
    magnitudes[i] = Math.hypot(real[i], imaginary[i]) / FFT_SIZE;
  }
  liveTime += delta;
  return tracker.update(extractSpectrumFrame({
    index: frameIndex++, time: liveTime, rms,
    magnitudes, sampleRate, fftSize: FFT_SIZE,
  }));
}

function draw(input: Frame) {
  const template = templates.find(({ slug }) => slug === input.preset) ?? templates.find(({ slug }) => slug === "default")!;
  if (!transitions || (input.paused && template.slug !== selected)) {
    transitions = new SettingsTransitionController(createSettingsFromTemplate(template.settings, DEFAULT_SETTINGS));
  } else if (template.slug !== selected) {
    transitions.startTemplateTransition(template, DEFAULT_TEMPLATE_TRANSITION_CONFIG);
  }
  selected = template.slug;

  const elapsed = lastClock === undefined ? 1 / 60 : input.clock - lastClock;
  const delta = input.paused ? 0 : Math.min(0.1, Math.max(1 / 240, elapsed));
  lastClock = input.paused ? undefined : input.clock;
  const motion = input.motion * (input.reduced ? 0.15 : 1);
  // Beat and pulse are softened to 85% for the desktop. Punch scales that;
  // calm frames (Reduce Motion or Dim Flashing Lights) remove every
  // transient and let modes morph at a quarter speed.
  const calm = input.calm === true;
  const transient = calm ? 0 : 0.85 * Math.max(0, Math.min(2, input.punch ?? 1));
  let analysedHops = 0;
  const effective = transitions.advance(delta).settings;
  const settings = {
    ...effective,
    projectionMode: input.projection === "preset" ? effective.projectionMode : input.projection,
    morphSeconds: effective.morphSeconds * 1.25,
  };

  // Keep WebKit's surface inside Lunalyr's desktop pixel budget, including
  // the extra render targets used by the stock post-processing stack.
  const scale = Math.min(window.devicePixelRatio || 1, 2,
    Math.sqrt((input.preview ? 220_000 : 950_000) / Math.max(1, input.width * input.height)));
  if (input.width !== width || input.height !== height || scale !== renderScale) {
    width = input.width;
    height = input.height;
    renderScale = scale;
    renderer.setPixelRatio(scale);
    renderer.setSize(width, height, false);
    fieldRenderer.setSize(width, height, scale);
  }

  if (!input.paused) {
    const samples = input.pcm && input.sampleRate ? decodeAudio(input.pcm) : undefined;
    const rms = samples ? readAudioWindow(samples, samples.length, input.sensitivity) : 0;
    // Capture can keep delivering zero-filled buffers with playback stopped.
    // Use signal level, with hysteresis and a short hold through musical rests.
    quietSeconds = rms >= (hadAudio ? quietRms : audibleRms) ? 0 : quietSeconds + delta;
    if (samples && input.sampleRate && (quietSeconds === 0 || (hadAudio && quietSeconds < silenceHold))) {
      if (!hadAudio || lastSampleRate !== input.sampleRate) {
        tracker.reset();
        modalEngine.reset(0);
        frameIndex = 0;
        liveTime = 0;
      }
      // Analyse every hop the host sent, oldest first, each for its share of
      // the frame, keeping the strongest transient any of them found.
      analysedHops = Math.floor((samples.length - FFT_SIZE) / HOP) + 1;
      const step = delta / analysedHops;
      let features = analyse(input.sampleRate, step,
        readAudioWindow(samples, samples.length - (analysedHops - 1) * HOP, input.sensitivity) * input.sensitivity);
      for (let hop = analysedHops - 2; hop >= 0; hop--) {
        const windowRms = readAudioWindow(samples, samples.length - hop * HOP, input.sensitivity);
        features = strongest(features, analyse(input.sampleRate, step, windowRms * input.sensitivity));
      }
      features.signals.pulse *= transient;
      features.signals.beat *= transient;
      if (calm) {
        features.signals.change = 0;
        features.onsets = { low: 0, mid: 0, high: 0 };
      }
      liveField = modalEngine.updateFromFeatureFrame(features, settings, calm ? delta * 0.25 : delta);
      hadAudio = true;
      lastSampleRate = input.sampleRate;
    } else {
      hadAudio = false;
      liveField = EMPTY_MODAL_FIELD_FRAME;
    }
  }
  // Use Wavefield's own ambient field until live modes are ready, including
  // the first few frames of audio, so neither handover needs a black frame.
  const ambient = liveField.modes.length === 0;
  // Mode selection is normalized upstream. Apply the gain to the selected
  // modes too, so changing the slider remains visible after normalization.
  // Calm frames keep the sensitivity off energy and remove the transients
  // the modal engine had already smoothed.
  const gain = Math.max(0.3, Math.min(3, input.sensitivity));
  const response = (value: number) => Math.min(1, value * gain);
  const field = ambient ? createAmbientModalFieldFrame(visualTime) : {
    ...liveField,
    flux: calm ? 0 : liveField.flux,
    onsets: calm ? { low: 0, mid: 0, high: 0 } : liveField.onsets,
    modes: liveField.modes.map(mode => ({
      ...mode, amplitude: response(mode.amplitude),
      excitation: response(mode.excitation), pulse: calm ? 0 : response(mode.pulse),
    })),
    signals: {
      ...liveField.signals,
      energy: calm ? Math.min(1, liveField.signals.energy) : response(liveField.signals.energy),
      pulse: calm ? 0 : response(liveField.signals.pulse),
      change: calm ? 0 : liveField.signals.change,
      beat: calm ? 0 : liveField.signals.beat,
    },
  };
  const motionDelta = delta * motion * (ambient ? idleMotion : 0.55 + 0.45 * gain);
  visualTime += motionDelta;
  screenView.update(motionDelta * 0.8);
  fieldRenderer.render(renderer, field, settings, screenView.view, motionDelta, false);
  return { preset: selected, projection: settings.projectionMode, modes: field.modes.length, ambient,
    calm, pulse: field.signals.pulse, hops: analysedHops };
}

Object.assign(window, { lunalyrWavefield: { draw } });
