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
  paused: boolean;
  preview?: boolean;
  pcm?: string;
  sampleRate?: number;
};

const templates = loadWavefieldTemplates(import.meta.glob("./src/templates/*.json", {
  eager: true, import: "default",
}));
const canvas = document.querySelector("canvas")!;
const renderer = new WebGLRenderer({ canvas, alpha: true, antialias: true, powerPreference: "high-performance" });
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
const idleMotion = 0.25;
const silenceHold = 0.6;
const audibleRms = 0.0012;
const quietRms = 0.0006;
let lastSampleRate: number | undefined;

const screenView = new ScreenViewController(canvas, () => transitions?.effectiveSettings ?? DEFAULT_SETTINGS);

function readAudioWindow(pcm: string, gain: number) {
  // Native supplies the same 2048-sample Float32 window Wavefield would
  // obtain from AnalyserNode. Retain its window, FFT and temporal tracker.
  const bytes = atob(pcm);
  if (bytes.length !== FFT_SIZE * 4) throw new Error("Invalid Wavefield PCM window");
  const buffer = new Uint8Array(bytes.length);
  for (let i = 0; i < bytes.length; i++) buffer[i] = bytes.charCodeAt(i);
  const samples = new Float32Array(buffer.buffer);
  let sum = 0;
  for (let i = 0; i < FFT_SIZE; i++) {
    const value = samples[i];
    real[i] = value * hann[i] * gain;
    sum += value * value;
  }
  return Math.sqrt(sum / FFT_SIZE);
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
    const rms = input.pcm && input.sampleRate ? readAudioWindow(input.pcm, input.sensitivity) : 0;
    // Capture can keep delivering zero-filled buffers with playback stopped.
    // Use signal level, with hysteresis and a short hold through musical rests.
    quietSeconds = rms >= (hadAudio ? quietRms : audibleRms) ? 0 : quietSeconds + delta;
    if (input.pcm && input.sampleRate && (quietSeconds === 0 || (hadAudio && quietSeconds < silenceHold))) {
      if (!hadAudio || lastSampleRate !== input.sampleRate) {
        tracker.reset();
        modalEngine.reset(0);
        frameIndex = 0;
        liveTime = 0;
      }
      const features = analyse(input.sampleRate, delta, rms * input.sensitivity);
      features.signals.pulse *= 0.85;
      features.signals.beat *= 0.85;
      liveField = modalEngine.updateFromFeatureFrame(features, settings, delta);
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
  const gain = Math.max(0.3, Math.min(3, input.sensitivity));
  const response = (value: number) => Math.min(1, value * gain);
  const field = ambient ? createAmbientModalFieldFrame(visualTime) : {
    ...liveField,
    modes: liveField.modes.map(mode => ({
      ...mode, amplitude: response(mode.amplitude),
      excitation: response(mode.excitation), pulse: response(mode.pulse),
    })),
    signals: { ...liveField.signals, energy: response(liveField.signals.energy), pulse: response(liveField.signals.pulse) },
  };
  const motionDelta = delta * motion * (ambient ? idleMotion : 0.55 + 0.45 * gain);
  visualTime += motionDelta;
  screenView.update(motionDelta * 0.8);
  fieldRenderer.render(renderer, field, settings, screenView.view, motionDelta, false);
  return { preset: selected, projection: settings.projectionMode, modes: field.modes.length, ambient };
}

Object.assign(window, { lunalyrWavefield: { draw } });
